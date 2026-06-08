//! Backups & WAL maintenance — the AppHandle-coupled orchestration layer.
//!
//! Versioned `VACUUM INTO` snapshots (safe under WAL, always a complete
//! standalone file) into a **user-chosen** folder (deliberately no default).
//! Triggered debounced after every mutation, hourly as a safety net, and
//! manually from the Manage window. Plus the WAL checkpointing that keeps the
//! main .db from drifting behind the -wal. The pure SQL helpers these lean on
//! (`setting`, `set_setting`) live in `db.rs`.

use std::sync::atomic::Ordering;

use rusqlite::{params, Connection};
use serde::Serialize;
use tauri::{Emitter, Manager};

use crate::db::{set_setting, setting, Db};
use crate::error::AppError;
use crate::AppState;

const BACKUP_KEEP: usize = 30;

#[derive(Serialize)]
pub struct BackupFile {
    pub name: String,
    pub path: String,
    pub size: u64,
    /// Unix epoch seconds of the file's mtime.
    pub modified: i64,
}

#[derive(Serialize)]
pub struct BackupInfo {
    /// None until the user has chosen a backup folder.
    pub dir: Option<String>,
    pub backups: Vec<BackupFile>,
}

fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// User-chosen backup folder. Deliberately NO automatic default: the user
/// must pick a location (ideally a cloud-synced one) before any backup is
/// written — see the 72h reminder in `backup_maintenance`. Created on demand.
pub fn backup_dir(conn: &Connection) -> Option<std::path::PathBuf> {
    let dir = setting(conn, "backup_dir").map(std::path::PathBuf::from)?;
    std::fs::create_dir_all(&dir).ok();
    Some(dir)
}

/// Backups in `dir`, newest first (the timestamped names sort lexically).
pub fn list_backup_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("marie-lookup-") && n.ends_with(".db"))
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files.reverse();
    files
}

/// Snapshot the live DB into the backup dir, then prune to BACKUP_KEEP.
/// Errors with the literal "no_backup_dir" (matched in dist/manage.js) when
/// the user hasn't chosen a folder yet.
pub fn do_backup(app: &tauri::AppHandle) -> Result<std::path::PathBuf, AppError> {
    let db = app.state::<Db>();
    let conn = db.0.lock()?;
    let Some(dir) = backup_dir(&conn) else {
        return Err("no_backup_dir".into());
    };
    let stamp: String =
        conn.query_row("SELECT strftime('%Y%m%d-%H%M%S','now')", [], |r| r.get(0))?;
    let path = dir.join(format!("marie-lookup-{stamp}.db"));
    // VACUUM INTO refuses to overwrite; drop a same-second leftover first.
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    conn.execute("VACUUM INTO ?1", params![path.to_string_lossy()])?;
    // Bookkeeping for the 72h reminder: backup done, nothing dirty anymore.
    set_setting(&conn, "last_backup_at", &now_epoch().to_string())?;
    let _ = conn.execute("DELETE FROM settings WHERE key='dirty_since'", []);
    drop(conn);
    for old in list_backup_files(&dir).into_iter().skip(BACKUP_KEEP) {
        let _ = std::fs::remove_file(old);
    }
    Ok(path)
}

/// Fold the WAL into the main .db (TRUNCATE so the -wal file shrinks back).
/// Keeps the on-disk marie-lookup.db close to current state — otherwise the
/// data lives only in the WAL and a raw file copy / hand-off loses it. Cheap
/// for this DB's size; best-effort.
pub fn checkpoint_wal(app: &tauri::AppHandle) {
    if let Some(db) = app.try_state::<Db>() {
        if let Ok(conn) = db.0.lock() {
            let _ = conn.pragma_update(None, "wal_checkpoint", "TRUNCATE");
        }
    }
}

/// Debounced auto-backup: runs 60s after the latest mutation unless a newer
/// mutation superseded it. See AppState::backup_gen. Also stamps
/// `dirty_since` (cleared by a successful backup) so `backup_maintenance`
/// can remind the user after 72h without one.
pub fn schedule_backup(app: &tauri::AppHandle) {
    if let Some(db) = app.try_state::<Db>() {
        if let Ok(conn) = db.0.lock() {
            // Insert-if-absent: dirty_since marks the FIRST un-backed-up
            // change, so continuous editing can't postpone the reminder.
            let _ = conn.execute(
                "INSERT INTO settings (key, value)
                 SELECT 'dirty_since', strftime('%s','now')
                 WHERE NOT EXISTS (SELECT 1 FROM settings WHERE key='dirty_since')",
                [],
            );
        }
    }
    let state = app.state::<AppState>();
    let generation = state.backup_gen.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        if app.state::<AppState>().backup_gen.load(Ordering::SeqCst) != generation {
            return;
        }
        // Consolidate the just-made edits into the main .db, then snapshot.
        checkpoint_wal(&app);
        if let Err(e) = do_backup(&app) {
            // Quietly skip when no folder is chosen — the 72h reminder in
            // backup_maintenance owns that situation.
            if e.0 != "no_backup_dir" {
                eprintln!("auto-backup failed: {e}");
            }
        }
    });
}

/// Hourly housekeeping (first run shortly after launch):
/// - backup folder chosen → safety-net backup when there are changes and the
///   last backup is >24h old;
/// - no folder (or backups keep failing) → after 72h with un-backed-up
///   changes, emit `backup-nag` so the startup window asks the user to
///   choose a folder (at most once per day). The path is never auto-picked.
pub fn backup_maintenance(app: &tauri::AppHandle) {
    const DAY: i64 = 86400;
    let now = now_epoch();
    let snapshot = {
        let db = app.state::<Db>();
        let guard = db.0.lock();
        match guard {
            Ok(conn) => Some((
                setting(&conn, "backup_dir").is_some(),
                setting(&conn, "dirty_since").and_then(|s| s.parse::<i64>().ok()),
                setting(&conn, "last_backup_at").and_then(|s| s.parse::<i64>().ok()),
                setting(&conn, "last_nag_at").and_then(|s| s.parse::<i64>().ok()),
            )),
            Err(_) => None,
        }
    };
    let Some((has_dir, dirty_since, last_backup, last_nag)) = snapshot else {
        return;
    };
    let Some(dirty) = dirty_since else { return }; // nothing to protect
    let mut nag = false;
    if has_dir {
        if last_backup.is_none_or(|b| now - b > DAY) {
            if let Err(e) = do_backup(app) {
                eprintln!("safety-net backup failed: {e}");
                nag = now - dirty >= 3 * DAY; // folder broken for days → ask again
            }
        }
    } else {
        nag = now - dirty >= 3 * DAY;
    }
    if nag && last_nag.is_none_or(|n| now - n >= DAY) {
        {
            let db = app.state::<Db>();
            let guard = db.0.lock();
            if let Ok(conn) = guard {
                let _ = set_setting(&conn, "last_nag_at", &now.to_string());
            }
        }
        let _ = app.emit_to("startup", "backup-nag", ());
    }
}
