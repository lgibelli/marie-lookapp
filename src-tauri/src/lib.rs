use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use rusqlite::{params, Connection};
use serde::Serialize;
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    Emitter, Manager,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

// CONTRACT: this exact error string is matched in dist/lookup.js (search for
// "accessibility_required"). If you rename it, update lookup.js too — there is
// no bundler / shared module across the Rust↔JS boundary to keep them in sync.
// Only the macOS paste path produces it (elsewhere `accessibility_ok` is a
// no-op), so the const is macOS-only.
#[cfg(target_os = "macos")]
const ERR_ACCESSIBILITY: &str = "accessibility_required";

/// Default global lookup hotkey in global_hotkey accelerator syntax. M = Marie.
/// Deliberately not Cmd+Space (Spotlight) or Cmd+Shift+Space (Character
/// Viewer). The user can override it from the startup window; the override is
/// persisted in the `settings` table and wins over this at launch.
/// NB: the Cmd/Win modifier token must be "Super" — that's the only spelling
/// both global_hotkey (registration) and muda (tray accelerator hint) parse;
/// "Meta" is rejected by both. dist/startup.js emits the same syntax.
#[cfg(target_os = "macos")]
const DEFAULT_HOTKEY: &str = "Super+Alt+M";
#[cfg(not(target_os = "macos"))]
const DEFAULT_HOTKEY: &str = "Control+Alt+M";

pub struct Db(pub Mutex<Connection>);

/// Handle to the tray's "Lookup…" item so `set_hotkey` can keep its
/// accelerator hint in sync with the live combo — it's set at build time
/// from whatever registered at startup and would otherwise go stale the
/// moment the user records a different hotkey.
struct TrayMenu {
    show_item: tauri::menu::MenuItem<tauri::Wry>,
}

/// Process-wide flags shared across commands, the tray, and window events.
struct AppState {
    /// True while `paste_text` is mid-flight (window hidden, focus handed back
    /// to the previous app, keystroke pending). Guards the global-hotkey and
    /// focus-loss handlers from re-showing/re-hiding the lookup window during
    /// that gap — otherwise the synthesized ⌘V can land in our own search box.
    is_pasting: AtomicBool,
    /// The currently registered global hotkey in global_hotkey accelerator
    /// syntax (e.g. "Control+Alt+M"), or None if registration failed and the
    /// user hasn't recorded a free combo yet. Written at setup and by
    /// `set_hotkey`; read by `get_hotkey` (the startup window's recorder UI).
    hotkey: Mutex<Option<String>>,
    /// Debounce generation for auto-backups: every mutation bumps it and
    /// schedules a snapshot 60s out; the snapshot only runs if no newer
    /// mutation has bumped the counter since (bursts → one backup).
    backup_gen: AtomicU64,
    /// HWND of the window that was foreground before the lookup window stole
    /// focus (0 = none captured yet). `paste_text` hands focus back to it via
    /// SetForegroundWindow before synthesizing Ctrl+V — merely hiding our
    /// window leaves activation to Z-order, which is not guaranteed to land
    /// on the paste target in time (or at all). macOS doesn't need this:
    /// `app.hide()` in `hide_lookup_window` restores focus properly there.
    #[cfg(target_os = "windows")]
    prev_foreground: std::sync::atomic::AtomicIsize,
}

/// Minimal hand-rolled user32 bindings for the foreground-window bookkeeping
/// described on `AppState::prev_foreground`. Two stable functions aren't
/// worth a direct `windows` crate dependency.
#[cfg(target_os = "windows")]
mod win_focus {
    use std::ffi::c_void;
    #[link(name = "user32")]
    extern "system" {
        pub fn GetForegroundWindow() -> *mut c_void;
        pub fn SetForegroundWindow(hwnd: *mut c_void) -> i32;
    }
}

/// Error wrapper so commands can use `?` instead of `.map_err(|e| e.to_string())`
/// on every fallible line. Serializes to a plain string, preserving the existing
/// JS error contract (e.g. lookup.js compares the rejection against
/// ERR_ACCESSIBILITY, and manage.js concatenates it into an alert).
#[derive(Debug)]
struct AppError(String);

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl From<String> for AppError {
    fn from(s: String) -> Self {
        AppError(s)
    }
}

impl From<&str> for AppError {
    fn from(s: &str) -> Self {
        AppError(s.to_string())
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError(e.to_string())
    }
}

impl<T> From<std::sync::PoisonError<T>> for AppError {
    fn from(e: std::sync::PoisonError<T>) -> Self {
        AppError(e.to_string())
    }
}

impl From<arboard::Error> for AppError {
    fn from(e: arboard::Error) -> Self {
        AppError(e.to_string())
    }
}

impl From<tauri::Error> for AppError {
    fn from(e: tauri::Error) -> Self {
        AppError(e.to_string())
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError(e.to_string())
    }
}

#[derive(Serialize, Clone)]
pub struct Entry {
    pub id: i64,
    pub title: String,
    pub body: String,
    /// Unix epoch seconds; shown next to each entry in the Manage sidebar.
    pub updated_at: i64,
}

/// Single source of truth for mapping a `SELECT id, title, body, updated_at`
/// row into an `Entry` — shared by `list_entries` and `search_entries`.
fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<Entry> {
    Ok(Entry {
        id: row.get(0)?,
        title: row.get(1)?,
        body: row.get(2)?,
        updated_at: row.get(3)?,
    })
}

fn db_path(app: &tauri::AppHandle) -> std::path::PathBuf {
    let dir = app.path().app_data_dir().expect("app_data_dir");
    std::fs::create_dir_all(&dir).ok();
    dir.join("marie-lookup.db")
}

fn open_db(path: &std::path::Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;

    // Concurrency hardening: WAL lets a reader and a writer coexist, and a
    // non-zero busy_timeout makes a contended lock retry instead of failing
    // immediately with SQLITE_BUSY. Harmless today (a single Mutex<Connection>
    // already serializes access), but removes the latent trap if a second
    // connection is ever opened.
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    // Auto-checkpoint defaults to 1000 WAL pages (~4 MB) — a snippet DB never
    // gets that big, so the main .db would stay perpetually stale and a naive
    // file copy (or hand-off of the DB) would lose everything still in the WAL.
    // Lower the threshold and also checkpoint explicitly (periodically, after
    // edits, and on exit — see checkpoint_wal) so the .db stays close to current.
    conn.pragma_update(None, "wal_autocheckpoint", 64)?;
    // Fold any WAL left over from a previous (possibly force-killed) run into
    // the main file right away.
    let _ = conn.pragma_update(None, "wal_checkpoint", "TRUNCATE");

    // Schema migrations keyed on PRAGMA user_version. Pre-versioning DBs report
    // version 0 and are stamped to 1 here (their schema already matches v1).
    // Future schema changes append `if version < 2 { ... }` blocks rather than
    // relying on CREATE TABLE IF NOT EXISTS, which silently no-ops on an
    // existing table and would never add a new column.
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < 1 {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS entries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                body TEXT NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            );
            -- idx_entries_title was never usable by any query (search wraps the
            -- column in LOWER()/uses a leading-wildcard LIKE, neither of which
            -- a plain B-tree index can serve); drop it on upgraded DBs.
            DROP INDEX IF EXISTS idx_entries_title;",
        )?;
        conn.pragma_update(None, "user_version", 1)?;
    }
    if version < 2 {
        // Key/value app settings: "hotkey" (global hotkey accelerator string)
        // and "backup_dir" (user-chosen backup folder override).
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );",
        )?;
        conn.pragma_update(None, "user_version", 2)?;
    }
    if version < 3 {
        // Per-entry version history for the Manage window's view-only "time
        // machine". `update_entry` snapshots the previous state here before
        // overwriting (capped at 50 per entry); `saved_at` is the moment
        // that version was originally saved, not when it was superseded.
        // Cleanup on entry deletion is done explicitly in `delete_entry` —
        // SQLite foreign keys are off by default and we don't turn them on.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS entry_versions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                entry_id INTEGER NOT NULL,
                title TEXT NOT NULL,
                body TEXT NOT NULL,
                saved_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_entry_versions_entry
                ON entry_versions(entry_id, id);",
        )?;
        conn.pragma_update(None, "user_version", 3)?;
    }
    if version < 4 {
        // Soft deletion: `delete_entry` stamps deleted_at instead of removing
        // the row; trashed entries are restorable from the Manage window and
        // hard-purged (history included) after 90 days by the sweep below.
        conn.execute_batch("ALTER TABLE entries ADD COLUMN deleted_at INTEGER;")?;
        conn.pragma_update(None, "user_version", 4)?;
    }
    if version < 5 {
        // Paste log powering the lookup window's empty-search "recents" view
        // (last pasted snippets + the entries they came from). Only successful
        // pastes are recorded; capped at 100 rows in paste_text.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS paste_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                entry_id INTEGER,
                text TEXT NOT NULL,
                pasted_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );",
        )?;
        conn.pragma_update(None, "user_version", 5)?;
    }

    // Empty the trash for real after 90 days, history included. Runs on
    // every launch (open_db is called once at setup).
    conn.execute_batch(
        "DELETE FROM entry_versions WHERE entry_id IN
           (SELECT id FROM entries WHERE deleted_at IS NOT NULL
              AND deleted_at < strftime('%s','now') - 90*86400);
         DELETE FROM entries WHERE deleted_at IS NOT NULL
           AND deleted_at < strftime('%s','now') - 90*86400;",
    )?;
    Ok(conn)
}

fn setting(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
        r.get(0)
    })
    .ok()
}

fn set_setting(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

// ---- Backups ---------------------------------------------------------------
// Versioned snapshots of the DB so the entries can't be lost to a dead disk,
// a botched reinstall or a fat-fingered delete. Snapshots are taken with
// VACUUM INTO — safe while the app runs (WAL included), produces a compact
// standalone .db. Triggered debounced after every mutation, on startup when
// the newest snapshot is older than a day, and manually from the Manage
// window. Pointing backup_dir at a OneDrive/iCloud-synced folder makes the
// backups off-site for free.

const BACKUP_KEEP: usize = 30;

/// User-chosen backup folder. Deliberately NO automatic default: the user
/// must pick a location (ideally a cloud-synced one) before any backup is
/// written — see the 72h reminder in `backup_maintenance`. Created on demand.
fn backup_dir(conn: &Connection) -> Option<std::path::PathBuf> {
    let dir = setting(conn, "backup_dir").map(std::path::PathBuf::from)?;
    std::fs::create_dir_all(&dir).ok();
    Some(dir)
}

fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Backups in `dir`, newest first (the timestamped names sort lexically).
fn list_backup_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
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
fn do_backup(app: &tauri::AppHandle) -> Result<std::path::PathBuf, AppError> {
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
fn checkpoint_wal(app: &tauri::AppHandle) {
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
fn schedule_backup(app: &tauri::AppHandle) {
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
    let gen = state.backup_gen.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        if app.state::<AppState>().backup_gen.load(Ordering::SeqCst) != gen {
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
fn backup_maintenance(app: &tauri::AppHandle) {
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

#[tauri::command]
fn list_entries(db: tauri::State<Db>) -> Result<Vec<Entry>, AppError> {
    let conn = db.0.lock()?;
    let mut stmt = conn.prepare(
        "SELECT id, title, body, updated_at FROM entries
         WHERE deleted_at IS NULL ORDER BY updated_at DESC",
    )?;
    let entries = stmt
        .query_map([], row_to_entry)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(entries)
}

#[tauri::command]
fn search_entries(db: tauri::State<Db>, query: String) -> Result<Vec<Entry>, AppError> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    // SQLite's LIKE is ASCII case-insensitive by default. Compare the column
    // against the needle so both sides fold identically. (The previous code
    // lowercased the needle in Rust — full Unicode — but wrapped the column in
    // SQLite LOWER() — ASCII only — so accented text matched inconsistently.)
    //
    // Escape LIKE metacharacters (\ % _) so a user typing them searches for the
    // literal character instead of a wildcard; pair with an explicit ESCAPE
    // clause. Backslash is escaped first so we don't double-escape the rest.
    let escaped = q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    let like = format!("%{}%", escaped);
    let conn = db.0.lock()?;
    let mut stmt = conn.prepare(
        "SELECT id, title, body, updated_at FROM entries \
         WHERE deleted_at IS NULL \
           AND (title LIKE ?1 ESCAPE '\\' OR body LIKE ?1 ESCAPE '\\') \
         ORDER BY (title LIKE ?1 ESCAPE '\\') DESC, updated_at DESC \
         LIMIT 20",
    )?;
    let entries = stmt
        .query_map([&like], row_to_entry)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(entries)
}

#[tauri::command]
fn add_entry(
    app: tauri::AppHandle,
    db: tauri::State<Db>,
    title: String,
    body: String,
) -> Result<i64, AppError> {
    let id = {
        let conn = db.0.lock()?;
        conn.execute(
            "INSERT INTO entries (title, body) VALUES (?1, ?2)",
            params![title, body],
        )?;
        conn.last_insert_rowid()
    };
    schedule_backup(&app);
    Ok(id)
}

#[tauri::command]
fn update_entry(
    app: tauri::AppHandle,
    db: tauri::State<Db>,
    id: i64,
    title: String,
    body: String,
) -> Result<(), AppError> {
    {
        let conn = db.0.lock()?;
        // Snapshot the outgoing state for the time machine — only when the
        // content actually changed, so no-op saves don't spam history.
        conn.execute(
            "INSERT INTO entry_versions (entry_id, title, body, saved_at)
             SELECT id, title, body, updated_at FROM entries
             WHERE id = ?1 AND (title <> ?2 OR body <> ?3)",
            params![id, title, body],
        )?;
        conn.execute(
            "UPDATE entries SET title=?1, body=?2, updated_at=strftime('%s','now') WHERE id=?3",
            params![title, body, id],
        )?;
        // Cap history per entry.
        conn.execute(
            "DELETE FROM entry_versions WHERE entry_id = ?1 AND id NOT IN (
                SELECT id FROM entry_versions WHERE entry_id = ?1
                ORDER BY id DESC LIMIT 50)",
            params![id],
        )?;
    }
    schedule_backup(&app);
    Ok(())
}

#[tauri::command]
fn delete_entry(app: tauri::AppHandle, db: tauri::State<Db>, id: i64) -> Result<(), AppError> {
    {
        let conn = db.0.lock()?;
        // Soft delete: restorable from the Manage window's "Deleted" section.
        // History stays (needed if the entry is restored); the 90-day purge in
        // open_db removes both for real.
        conn.execute(
            "UPDATE entries SET deleted_at=strftime('%s','now') WHERE id=?1",
            params![id],
        )?;
    }
    schedule_backup(&app);
    Ok(())
}

#[derive(Serialize)]
struct TrashedEntry {
    id: i64,
    title: String,
    body: String,
    /// When the entry was moved to the trash (epoch seconds).
    deleted_at: i64,
}

/// Soft-deleted entries, newest deletion first. Excluded from lookup search
/// and the live list; hard-purged with their history after 90 days.
#[tauri::command]
fn list_trash(db: tauri::State<Db>) -> Result<Vec<TrashedEntry>, AppError> {
    let conn = db.0.lock()?;
    let mut stmt = conn.prepare(
        "SELECT id, title, body, deleted_at FROM entries
         WHERE deleted_at IS NOT NULL ORDER BY deleted_at DESC",
    )?;
    let items = stmt
        .query_map([], |r| {
            Ok(TrashedEntry {
                id: r.get(0)?,
                title: r.get(1)?,
                body: r.get(2)?,
                deleted_at: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(items)
}

#[tauri::command]
fn restore_entry(app: tauri::AppHandle, db: tauri::State<Db>, id: i64) -> Result<(), AppError> {
    {
        let conn = db.0.lock()?;
        conn.execute("UPDATE entries SET deleted_at=NULL WHERE id=?1", params![id])?;
    }
    schedule_backup(&app);
    Ok(())
}

#[derive(Serialize)]
struct EntryVersion {
    id: i64,
    title: String,
    body: String,
    /// When this version was originally saved (epoch seconds).
    saved_at: i64,
}

/// Previous versions of an entry, newest first. Powers the Manage window's
/// view-only time machine.
#[tauri::command]
fn list_versions(db: tauri::State<Db>, entry_id: i64) -> Result<Vec<EntryVersion>, AppError> {
    let conn = db.0.lock()?;
    let mut stmt = conn.prepare(
        "SELECT id, title, body, saved_at FROM entry_versions
         WHERE entry_id = ?1 ORDER BY id DESC",
    )?;
    let versions = stmt
        .query_map([entry_id], |r| {
            Ok(EntryVersion {
                id: r.get(0)?,
                title: r.get(1)?,
                body: r.get(2)?,
                saved_at: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(versions)
}

#[derive(Serialize)]
struct BackupFile {
    name: String,
    path: String,
    size: u64,
    /// Unix epoch seconds of the file's mtime.
    modified: i64,
}

#[derive(Serialize)]
struct BackupInfo {
    /// None until the user has chosen a backup folder.
    dir: Option<String>,
    backups: Vec<BackupFile>,
}

#[tauri::command]
fn backup_info(db: tauri::State<Db>) -> Result<BackupInfo, AppError> {
    let dir = {
        let conn = db.0.lock()?;
        backup_dir(&conn)
    };
    let Some(dir) = dir else {
        return Ok(BackupInfo {
            dir: None,
            backups: Vec::new(),
        });
    };
    let backups = list_backup_files(&dir)
        .into_iter()
        .filter_map(|p| {
            let meta = std::fs::metadata(&p).ok()?;
            let modified = meta
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_secs() as i64;
            Some(BackupFile {
                name: p.file_name()?.to_string_lossy().into_owned(),
                path: p.display().to_string(),
                size: meta.len(),
                modified,
            })
        })
        .collect();
    Ok(BackupInfo {
        dir: Some(dir.display().to_string()),
        backups,
    })
}

#[tauri::command]
fn backup_now(app: tauri::AppHandle) -> Result<String, AppError> {
    Ok(do_backup(&app)?.display().to_string())
}

/// Persist a new backup folder and immediately snapshot into it, so an
/// unwritable location fails loudly here instead of silently at 3am.
#[tauri::command]
fn set_backup_dir(
    app: tauri::AppHandle,
    db: tauri::State<Db>,
    dir: String,
) -> Result<(), AppError> {
    {
        let conn = db.0.lock()?;
        set_setting(&conn, "backup_dir", &dir)?;
    }
    do_backup(&app)?;
    Ok(())
}

/// Replace all entries with the ones in the given backup file. Settings are
/// deliberately untouched (hotkey/backup dir are device-local). Done via
/// ATTACH inside one transaction so the live connection never closes.
#[tauri::command]
fn restore_backup(
    app: tauri::AppHandle,
    db: tauri::State<Db>,
    path: String,
) -> Result<i64, AppError> {
    let count = {
        let conn = db.0.lock()?;
        conn.execute("ATTACH DATABASE ?1 AS bak", params![path])?;
        let result = (|| -> Result<i64, AppError> {
            let has_entries: i64 = conn.query_row(
                "SELECT count(*) FROM bak.sqlite_master WHERE type='table' AND name='entries'",
                [],
                |r| r.get(0),
            )?;
            if has_entries == 0 {
                return Err("not a Marie Lookup backup (no entries table)".into());
            }
            // Pre-v3 backups have no entry_versions table; in that case the
            // local history is cleared (it would describe entries that no
            // longer match) instead of copied. Pre-v4 backups have no
            // deleted_at column — their entries restore as live.
            let bak_has_versions: i64 = conn.query_row(
                "SELECT count(*) FROM bak.sqlite_master WHERE type='table' AND name='entry_versions'",
                [],
                |r| r.get(0),
            )?;
            let copy_versions = if bak_has_versions > 0 {
                "INSERT INTO main.entry_versions (id, entry_id, title, body, saved_at)
                   SELECT id, entry_id, title, body, saved_at FROM bak.entry_versions;"
            } else {
                ""
            };
            let bak_has_deleted: i64 = conn.query_row(
                "SELECT count(*) FROM bak.pragma_table_info('entries') WHERE name='deleted_at'",
                [],
                |r| r.get(0),
            )?;
            let bak_has_pastes: i64 = conn.query_row(
                "SELECT count(*) FROM bak.sqlite_master WHERE type='table' AND name='paste_history'",
                [],
                |r| r.get(0),
            )?;
            let copy_pastes = if bak_has_pastes > 0 {
                "INSERT INTO main.paste_history (id, entry_id, text, pasted_at)
                   SELECT id, entry_id, text, pasted_at FROM bak.paste_history;"
            } else {
                ""
            };
            let copy_entries = if bak_has_deleted > 0 {
                "INSERT INTO main.entries (id, title, body, created_at, updated_at, deleted_at)
                   SELECT id, title, body, created_at, updated_at, deleted_at FROM bak.entries;"
            } else {
                "INSERT INTO main.entries (id, title, body, created_at, updated_at)
                   SELECT id, title, body, created_at, updated_at FROM bak.entries;"
            };
            conn.execute_batch(&format!(
                "BEGIN;
                 DELETE FROM main.entries;
                 {copy_entries}
                 DELETE FROM main.entry_versions;
                 {copy_versions}
                 DELETE FROM main.paste_history;
                 {copy_pastes}
                 COMMIT;"
            ))?;
            Ok(conn.query_row("SELECT count(*) FROM main.entries", [], |r| r.get(0))?)
        })();
        if result.is_err() {
            let _ = conn.execute_batch("ROLLBACK;");
        }
        let _ = conn.execute("DETACH DATABASE bak", []);
        result?
    };
    // Snapshot the restored state too — a restore is a mutation like any other.
    schedule_backup(&app);
    Ok(count)
}

#[cfg(target_os = "macos")]
fn accessibility_ok(prompt: bool) -> bool {
    use macos_accessibility_client::accessibility::{
        application_is_trusted, application_is_trusted_with_prompt,
    };
    if prompt {
        application_is_trusted_with_prompt()
    } else {
        application_is_trusted()
    }
}

// Non-macOS stub: there's no Accessibility gate elsewhere, so nothing calls this
// today, but keep it so any future cross-platform call site compiles.
#[cfg(not(target_os = "macos"))]
#[allow(dead_code)]
fn accessibility_ok(_prompt: bool) -> bool {
    true
}

/// Open the macOS Accessibility settings pane. Best-effort (errors ignored).
/// Single source of the settings-pane deep-link URL.
#[cfg(target_os = "macos")]
fn open_accessibility_pane() {
    let _ = std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        .spawn();
}

#[tauri::command]
fn open_accessibility_settings() -> Result<(), AppError> {
    #[cfg(target_os = "macos")]
    open_accessibility_pane();
    Ok(())
}

#[tauri::command]
async fn paste_text(
    app: tauri::AppHandle,
    db: tauri::State<'_, Db>,
    text: String,
    entry_id: Option<i64>,
) -> Result<(), AppError> {
    // Check Accessibility permission BEFORE hiding our window, so the user
    // can read the inline hint we'll show if it's missing.
    #[cfg(target_os = "macos")]
    if !accessibility_ok(true) {
        // application_is_trusted_with_prompt has just shown the system dialog.
        // Also open the Accessibility settings pane directly for a clear path.
        open_accessibility_pane();
        return Err(ERR_ACCESSIBILITY.into());
    }

    let mut cb = arboard::Clipboard::new()?;
    cb.set_text(text.clone())?;
    drop(cb);

    // Mark the paste in-flight so the global hotkey / focus-loss handler won't
    // re-show or re-hide the lookup window during the focus hand-off + keystroke.
    app.state::<AppState>()
        .is_pasting
        .store(true, Ordering::SeqCst);

    hide_lookup_window(&app);

    // On Windows, hiding alone leaves activation to Z-order and the
    // synthesized Ctrl+V can land in the wrong window (or nowhere) — hand
    // focus back to the window captured in show_lookup explicitly. macOS
    // gets the equivalent via app.hide() inside hide_lookup_window.
    #[cfg(target_os = "windows")]
    {
        let prev = app
            .state::<AppState>()
            .prev_foreground
            .load(Ordering::SeqCst);
        if prev != 0 && unsafe { win_focus::SetForegroundWindow(prev as *mut _) } == 0 {
            eprintln!("SetForegroundWindow failed; paste may miss its target");
        }
    }

    // Give the OS time to transfer focus back to the previous app.
    tokio::time::sleep(std::time::Duration::from_millis(180)).await;

    // enigo on macOS calls TSMGetInputSourceProperty which asserts main-thread.
    // Dispatch the keystroke to the main thread so it doesn't trap. Pipe the
    // result back through a channel so a failed paste surfaces as an Err to the
    // frontend instead of silently returning Ok (the whole point of the
    // Accessibility handling — a paste that does nothing must not look like
    // success).
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let dispatch = app.run_on_main_thread(move || {
        use enigo::{Direction, Enigo, Key, Keyboard, Settings};
        let result = (|| -> Result<(), String> {
            let mut enigo =
                Enigo::new(&Settings::default()).map_err(|e| format!("enigo init failed: {e}"))?;
            #[cfg(target_os = "macos")]
            let modifier = Key::Meta;
            #[cfg(not(target_os = "macos"))]
            let modifier = Key::Control;
            enigo
                .key(modifier, Direction::Press)
                .map_err(|e| e.to_string())?;
            enigo
                .key(Key::Unicode('v'), Direction::Click)
                .map_err(|e| e.to_string())?;
            enigo
                .key(modifier, Direction::Release)
                .map_err(|e| e.to_string())?;
            Ok(())
        })();
        let _ = tx.send(result);
    });

    let outcome = match dispatch {
        Err(e) => Err(AppError::from(e)),
        Ok(()) => match rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(inner) => inner.map_err(AppError::from),
            Err(_) => Err(AppError::from("paste keystroke dispatch timed out")),
        },
    };

    app.state::<AppState>()
        .is_pasting
        .store(false, Ordering::SeqCst);

    // Log successful pastes for the lookup window's "recents" view. Best
    // effort — a failed log must not fail the paste.
    if outcome.is_ok() {
        if let Ok(conn) = db.0.lock() {
            let _ = conn.execute(
                "INSERT INTO paste_history (entry_id, text) VALUES (?1, ?2)",
                params![entry_id, text],
            );
            let _ = conn.execute(
                "DELETE FROM paste_history WHERE id NOT IN
                   (SELECT id FROM paste_history ORDER BY id DESC LIMIT 100)",
                [],
            );
        }
    }
    outcome
}

/// Empty-search state of the lookup window: the last 3 distinct (live)
/// entries something was pasted from, most recently used first — the lookup
/// opens the first one by default so more text can be selected from it
/// without typing. Trashed entries never appear.
#[tauri::command]
fn recent_topics(db: tauri::State<Db>) -> Result<Vec<Entry>, AppError> {
    let conn = db.0.lock()?;
    let mut stmt = conn.prepare(
        "SELECT e.id, e.title, e.body, e.updated_at, MAX(p.pasted_at) AS t
         FROM paste_history p JOIN entries e ON e.id = p.entry_id
         WHERE e.deleted_at IS NULL
         GROUP BY e.id ORDER BY t DESC LIMIT 3",
    )?;
    let topics = stmt
        .query_map([], row_to_entry)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(topics)
}

/// Hide the lookup window (and, on macOS, the whole app so focus returns to the
/// previous app). Shared by `hide_lookup` and `paste_text`.
fn hide_lookup_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("lookup") {
        let _ = w.hide();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = app.hide();
    }
}

#[tauri::command]
fn hide_lookup(app: tauri::AppHandle) -> Result<(), AppError> {
    hide_lookup_window(&app);
    Ok(())
}

#[tauri::command]
fn open_manage(app: tauri::AppHandle) -> Result<(), AppError> {
    show_manage(&app);
    Ok(())
}

/// The active global hotkey (accelerator syntax, e.g. "Control+Alt+M"), or
/// None if nothing could be registered. The startup window (dist/startup.js)
/// displays it and auto-opens its recorder UI when this is None.
#[tauri::command]
fn get_hotkey(state: tauri::State<AppState>) -> Result<Option<String>, AppError> {
    Ok(state.hotkey.lock()?.clone())
}

/// Register + persist a user-chosen global hotkey (accelerator syntax). The
/// new combo is registered BEFORE the old one is dropped: if another app owns
/// it (or the string doesn't parse) this errors and the previous hotkey keeps
/// working — the startup window's recorder loops until a free combo is found.
#[tauri::command]
fn set_hotkey(
    app: tauri::AppHandle,
    db: tauri::State<Db>,
    state: tauri::State<AppState>,
    hotkey: String,
) -> Result<(), AppError> {
    let mut current = state.hotkey.lock()?;
    if current.as_deref() == Some(hotkey.as_str()) {
        return Ok(()); // already active — re-registering would fail as a dup
    }
    app.global_shortcut()
        .register(hotkey.as_str())
        .map_err(|e| format!("could not register {hotkey}: {e}"))?;
    if let Some(old) = current.take() {
        let _ = app.global_shortcut().unregister(old.as_str());
    }
    *current = Some(hotkey.clone());
    // Refresh the tray hint. Best-effort: muda may reject a combo that
    // global_hotkey accepted, and the hint is cosmetic.
    if let Some(tray) = app.try_state::<TrayMenu>() {
        let _ = tray.show_item.set_accelerator(Some(hotkey.as_str()));
    }
    let conn = db.0.lock()?;
    set_setting(&conn, "hotkey", &hotkey)?;
    Ok(())
}

/// Show + focus the manage window. Shared by `open_manage` and the tray menu so
/// both restore a minimized window identically.
fn show_manage(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("manage") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Open the public releases page in the default browser. Hand-rolled rather
/// than pulling in tauri-plugin-opener for one fixed URL.
fn open_releases_url() -> std::io::Result<()> {
    let url = "https://github.com/lgibelli/marie-lookapp-releases/releases";
    #[cfg(target_os = "macos")]
    std::process::Command::new("open").arg(url).spawn()?;
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()?;
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    std::process::Command::new("xdg-open").arg(url).spawn()?;
    Ok(())
}

#[tauri::command]
fn open_releases_page() -> Result<(), AppError> {
    open_releases_url()?;
    Ok(())
}

/// Force a WAL checkpoint now. Called from the frontend right before an update
/// install: the Windows updater's installer `taskkill /F`s us (the exe is
/// locked while running and our WM_CLOSE-to-hide makes a polite close a no-op),
/// which skips the on-exit checkpoint — so flush first, deterministically.
#[tauri::command]
fn checkpoint_now(app: tauri::AppHandle) -> Result<(), AppError> {
    checkpoint_wal(&app);
    Ok(())
}

/// Tray "About Marie Lookup…": a tiny native dialog — name + version + a
/// button to the releases page. Deliberately NOT the startup window (that's
/// the richer status panel "Check for updates…" uses); About is at-a-glance.
fn show_about(app: &tauri::AppHandle) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
    let version = app.package_info().version.to_string();
    app.dialog()
        .message(format!("Marie Lookup v{version}"))
        .title("About Marie Lookup")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Open releases page".into(),
            "Close".into(),
        ))
        .show(|open_releases| {
            if open_releases {
                let _ = open_releases_url();
            }
        });
}

fn show_lookup(app: &tauri::AppHandle) {
    // Don't pop the lookup window back up while a paste is mid-flight — that
    // would steal focus from the target app and the ⌘V would land in our own
    // search box instead of the intended destination.
    if app.state::<AppState>().is_pasting.load(Ordering::SeqCst) {
        return;
    }
    if let Some(w) = app.get_webview_window("lookup") {
        // Remember who has focus before we steal it, so paste_text can hand
        // it straight back. Skip when the lookup window itself is foreground
        // (hotkey pressed while already open) — saving our own handle would
        // make the paste target ourselves.
        #[cfg(target_os = "windows")]
        {
            let prev = unsafe { win_focus::GetForegroundWindow() } as isize;
            let ours = w.hwnd().map(|h| h.0 as isize).unwrap_or(0);
            if prev != 0 && prev != ours {
                app.state::<AppState>()
                    .prev_foreground
                    .store(prev, Ordering::SeqCst);
            }
        }
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        let _ = w.center();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        show_lookup(app);
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        // Auto-updates: checks latest.json on the public releases repo; the
        // startup window drives the ask-then-install flow (dist/startup.js).
        // Only Windows artifacts are published for now — on macOS check()
        // errors with "no matching platform" and the frontend ignores it.
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Native folder/file pickers for the backup UI in the Manage window.
        .plugin(tauri_plugin_dialog::init())
        // relaunch() after a macOS update — the updater swaps the .app on
        // disk but the old process keeps running (Windows restarts via the
        // installer instead and never reaches the relaunch call).
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            // DB + shared state. Manage AppState before the tray/hotkey are wired
            // up so any handler that reads it always finds it.
            let path = db_path(app.handle());
            let conn = open_db(&path).expect("open db");
            // The user-chosen hotkey must be known before any webview JS runs,
            // so read it straight off the fresh connection here.
            let stored_hotkey = setting(&conn, "hotkey");
            app.manage(Db(Mutex::new(conn)));
            app.manage(AppState {
                is_pasting: AtomicBool::new(false),
                hotkey: Mutex::new(None),
                backup_gen: AtomicU64::new(0),
                #[cfg(target_os = "windows")]
                prev_foreground: std::sync::atomic::AtomicIsize::new(0),
            });

            // Backup housekeeping: safety-net backups + the 72h "please pick
            // a backup folder" reminder. First pass shortly after launch
            // (delayed so it never competes with launch work), then hourly —
            // the app is tray-resident and can run for weeks.
            {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    loop {
                        backup_maintenance(&app_handle);
                        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                    }
                });
            }

            // Periodic WAL checkpoint so the main .db never drifts far behind
            // the WAL (see checkpoint_wal). Independent of edits/backups — even
            // an idle-then-killed session leaves a near-current .db.
            {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    loop {
                        tokio::time::sleep(std::time::Duration::from_secs(300)).await;
                        checkpoint_wal(&app_handle);
                    }
                });
            }

            // Register the global hotkey: the stored custom combo if any, else
            // DEFAULT_HOTKEY. Non-fatal — if every candidate is owned by another
            // app, keep running (tray + windows still work); the startup window
            // sees None via `get_hotkey` and opens its recorder UI so the user
            // can pick a free combo. Propagating with `?` here used to kill the
            // whole app silently.
            let mut active_hotkey: Option<String> = None;
            #[cfg(desktop)]
            {
                let mut candidates = Vec::new();
                if let Some(stored) = stored_hotkey {
                    candidates.push(stored);
                }
                if candidates.first().map(String::as_str) != Some(DEFAULT_HOTKEY) {
                    candidates.push(DEFAULT_HOTKEY.to_string());
                }
                for cand in candidates {
                    match app.global_shortcut().register(cand.as_str()) {
                        Ok(()) => {
                            active_hotkey = Some(cand);
                            break;
                        }
                        Err(e) => eprintln!("global hotkey {cand}: registration failed: {e}"),
                    }
                }
                *app.state::<AppState>().hotkey.lock().unwrap() = active_hotkey.clone();
            }

            // Tray menu. The accelerator on "Lookup…" is purely a visual hint —
            // it mirrors whatever combo actually registered above (none if
            // registration failed entirely). muda's accelerator parser is not
            // identical to global_hotkey's, so if it rejects the combo, fall
            // back to a hint-less item rather than killing the app.
            let show_item = {
                let mut builder = MenuItemBuilder::with_id("show", "Lookup…");
                if let Some(hk) = &active_hotkey {
                    builder = builder.accelerator(hk);
                }
                builder
                    .build(app)
                    .or_else(|_| MenuItemBuilder::with_id("show", "Lookup…").build(app))?
            };
            app.manage(TrayMenu {
                show_item: show_item.clone(),
            });
            let manage_item = MenuItemBuilder::with_id("manage", "Manage entries…").build(app)?;
            let check_item =
                MenuItemBuilder::with_id("check-updates", "Check for updates…").build(app)?;
            let about_item =
                MenuItemBuilder::with_id("about", "About Marie Lookup…").build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "Quit")
                .accelerator("CmdOrCtrl+Q")
                .build(app)?;
            let menu = MenuBuilder::new(app)
                .items(&[&show_item, &manage_item, &check_item, &about_item])
                .separator()
                .item(&quit_item)
                .build()?;

            let _tray = TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_lookup(app),
                    "manage" => show_manage(app),
                    // The update flow lives in the startup window's JS
                    // (dist/startup.js) — poke it to check right now,
                    // bypassing the hourly cadence and the daily prompt cap.
                    "check-updates" => {
                        let _ = app.emit_to("startup", "check-updates", ());
                    }
                    "about" => show_about(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            // Hide lookup when it loses focus
            if let Some(w) = app.get_webview_window("lookup") {
                let app_handle = app.handle().clone();
                w.on_window_event(move |event| {
                    if let tauri::WindowEvent::Focused(false) = event {
                        // Don't fight paste_text's own hide while a paste is in
                        // flight — it manages the window itself.
                        if app_handle.state::<AppState>().is_pasting.load(Ordering::SeqCst) {
                            return;
                        }
                        if let Some(win) = app_handle.get_webview_window("lookup") {
                            let _ = win.hide();
                        }
                    }
                });
            }

            // On macOS, make the app a background accessory (no Dock icon)
            #[cfg(target_os = "macos")]
            {
                let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);

                // Trigger the macOS Accessibility-permission prompt on first launch.
                // If already trusted, this is a silent no-op. If not, the standard
                // system dialog appears with an "Open System Settings" button.
                let _ = accessibility_ok(true);
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_entries,
            search_entries,
            add_entry,
            update_entry,
            delete_entry,
            paste_text,
            hide_lookup,
            open_manage,
            open_accessibility_settings,
            get_hotkey,
            set_hotkey,
            backup_info,
            backup_now,
            set_backup_dir,
            restore_backup,
            list_versions,
            list_trash,
            restore_entry,
            recent_topics,
            open_releases_page,
            checkpoint_now,
        ])
        .on_window_event(|window, event| {
            // Keep the app running when the manage window is closed:
            // just hide it instead of letting close terminate the process.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Final WAL checkpoint on a clean exit (tray Quit → app.exit, etc.)
            // so the main .db is complete on disk. A force-kill (e.g. the
            // Windows updater's taskkill) skips this, but the WAL is crash-safe
            // and the next launch checkpoints it in open_db.
            if let tauri::RunEvent::Exit = event {
                checkpoint_wal(app);
            }
        });
}
