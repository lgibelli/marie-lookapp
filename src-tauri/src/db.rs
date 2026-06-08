//! Data layer. Every function here takes a borrowed `&Connection` and contains
//! no Tauri types, so it's unit-testable against a real temp SQLite file (see
//! the `tests` module — never a mock, per CLAUDE.md). The `#[tauri::command]`
//! wrappers in `lib.rs` are thin: lock the `Db` mutex, call one of these, map
//! the error. This is where the risky logic lives — the migration ladder, the
//! version-tolerant restore, the soft-delete invariant, the history/paste caps.

use rusqlite::{params, Connection};
use serde::Serialize;
use std::sync::Mutex;

use crate::error::AppError;

/// The single SQLite connection, behind a mutex. Tauri-managed state.
pub struct Db(pub Mutex<Connection>);

#[derive(Serialize, Clone)]
pub struct Entry {
    pub id: i64,
    pub title: String,
    pub body: String,
    /// Unix epoch seconds; shown next to each entry in the Manage sidebar.
    pub updated_at: i64,
}

#[derive(Serialize)]
pub struct TrashedEntry {
    pub id: i64,
    pub title: String,
    pub body: String,
    /// When the entry was moved to the trash (epoch seconds).
    pub deleted_at: i64,
}

#[derive(Serialize)]
pub struct EntryVersion {
    pub id: i64,
    pub title: String,
    pub body: String,
    /// When this version was originally saved (epoch seconds).
    pub saved_at: i64,
}

/// Single source of truth for mapping a `SELECT id, title, body, updated_at`
/// row into an `Entry` — shared by `list_entries`, `search_entries` and
/// `recent_topics`.
fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<Entry> {
    Ok(Entry {
        id: row.get(0)?,
        title: row.get(1)?,
        body: row.get(2)?,
        updated_at: row.get(3)?,
    })
}

pub fn open_db(path: &std::path::Path) -> rusqlite::Result<Connection> {
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

pub fn setting(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
        r.get(0)
    })
    .ok()
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

// ---- Entry queries & mutations --------------------------------------------
// The soft-delete invariant lives here: live queries filter `deleted_at IS
// NULL`, the trash query filters `deleted_at IS NOT NULL`. Keep it that way.

/// Live entries, most-recently-updated first. Excludes trashed rows.
pub fn list_entries(conn: &Connection) -> rusqlite::Result<Vec<Entry>> {
    let mut stmt = conn.prepare(
        "SELECT id, title, body, updated_at FROM entries
         WHERE deleted_at IS NULL ORDER BY updated_at DESC",
    )?;
    let rows = stmt
        .query_map([], row_to_entry)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Search live entries by title/body. Empty/whitespace query → no results.
pub fn search_entries(conn: &Connection, query: &str) -> rusqlite::Result<Vec<Entry>> {
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
    let mut stmt = conn.prepare(
        "SELECT id, title, body, updated_at FROM entries \
         WHERE deleted_at IS NULL \
           AND (title LIKE ?1 ESCAPE '\\' OR body LIKE ?1 ESCAPE '\\') \
         ORDER BY (title LIKE ?1 ESCAPE '\\') DESC, updated_at DESC \
         LIMIT 20",
    )?;
    let rows = stmt
        .query_map([&like], row_to_entry)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Insert a new entry, returning its id.
pub fn add_entry(conn: &Connection, title: &str, body: &str) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO entries (title, body) VALUES (?1, ?2)",
        params![title, body],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Update an entry, snapshotting the outgoing version (only on a real change)
/// and capping per-entry history at 50.
pub fn update_entry(conn: &Connection, id: i64, title: &str, body: &str) -> rusqlite::Result<()> {
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
    Ok(())
}

/// Soft-delete: stamp `deleted_at`. History stays (needed on restore); the
/// 90-day purge in `open_db` removes both for real.
pub fn delete_entry(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE entries SET deleted_at=strftime('%s','now') WHERE id=?1",
        params![id],
    )?;
    Ok(())
}

/// Soft-deleted entries, newest deletion first.
pub fn list_trash(conn: &Connection) -> rusqlite::Result<Vec<TrashedEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, title, body, deleted_at FROM entries
         WHERE deleted_at IS NOT NULL ORDER BY deleted_at DESC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(TrashedEntry {
                id: r.get(0)?,
                title: r.get(1)?,
                body: r.get(2)?,
                deleted_at: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Bring a trashed entry back to life.
pub fn restore_entry(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute("UPDATE entries SET deleted_at=NULL WHERE id=?1", params![id])?;
    Ok(())
}

/// Previous versions of an entry, newest first (the time machine).
pub fn list_versions(conn: &Connection, entry_id: i64) -> rusqlite::Result<Vec<EntryVersion>> {
    let mut stmt = conn.prepare(
        "SELECT id, title, body, saved_at FROM entry_versions
         WHERE entry_id = ?1 ORDER BY id DESC",
    )?;
    let rows = stmt
        .query_map([entry_id], |r| {
            Ok(EntryVersion {
                id: r.get(0)?,
                title: r.get(1)?,
                body: r.get(2)?,
                saved_at: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Empty-search state of the lookup window: the last 3 distinct (live)
/// entries something was pasted from, most recently used first. Trashed
/// entries never appear.
pub fn recent_topics(conn: &Connection) -> rusqlite::Result<Vec<Entry>> {
    let mut stmt = conn.prepare(
        "SELECT e.id, e.title, e.body, e.updated_at, MAX(p.pasted_at) AS t
         FROM paste_history p JOIN entries e ON e.id = p.entry_id
         WHERE e.deleted_at IS NULL
         GROUP BY e.id ORDER BY t DESC LIMIT 3",
    )?;
    let rows = stmt
        .query_map([], row_to_entry)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Log a successful paste for the "recents" view, capped at the 100 most
/// recent rows. Best-effort at the call site (the paste already happened).
pub fn record_paste(conn: &Connection, entry_id: Option<i64>, text: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO paste_history (entry_id, text) VALUES (?1, ?2)",
        params![entry_id, text],
    )?;
    conn.execute(
        "DELETE FROM paste_history WHERE id NOT IN
           (SELECT id FROM paste_history ORDER BY id DESC LIMIT 100)",
        [],
    )?;
    Ok(())
}

/// Replace all entries (and history/pastes) with the ones in `source_path`,
/// via ATTACH inside one transaction so the live connection never closes.
/// Version-tolerant: cherry-picks the columns/tables that exist in the source
/// so an older backup imports without a separate migration step.
/// - pre-v3 source (no `entry_versions`): local history is cleared, not copied
///   (it would describe entries that no longer match).
/// - pre-v4 source (no `deleted_at`): entries restore as live.
/// - pre-v5 source (no `paste_history`): pastes are cleared, not copied.
///
/// Settings (hotkey/backup_dir) are deliberately untouched — device-local.
/// Returns the resulting entry count.
pub fn restore_from_backup(conn: &Connection, source_path: &str) -> Result<i64, AppError> {
    conn.execute("ATTACH DATABASE ?1 AS bak", params![source_path])?;
    let result = (|| -> Result<i64, AppError> {
        let has_entries: i64 = conn.query_row(
            "SELECT count(*) FROM bak.sqlite_master WHERE type='table' AND name='entries'",
            [],
            |r| r.get(0),
        )?;
        if has_entries == 0 {
            return Err("not a Marie Lookup backup (no entries table)".into());
        }
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
        // NB: must be the two-argument `pragma_table_info('entries','bak')`
        // form. The schema-prefixed `bak.pragma_table_info('entries')` form
        // silently reads `main`'s table instead of the attached backup's, so
        // it always reports deleted_at present (main always has it) and a
        // pre-v4 backup would wrongly take the with-deleted_at branch and fail
        // with "no such column: deleted_at". (Covered by the restore tests.)
        let bak_has_deleted: i64 = conn.query_row(
            "SELECT count(*) FROM pragma_table_info('entries','bak') WHERE name='deleted_at'",
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
    result
}

#[cfg(test)]
mod tests {
    //! DB-layer tests against a real temp SQLite file (never a mock — see
    //! CLAUDE.md). They cover what breaks silently on refactor: the migration
    //! ladder, the 90-day trash sweep, the soft-delete invariant, the
    //! history/paste caps, and the version-tolerant restore.
    use super::*;

    /// A fresh temp DB opened through the real `open_db`. Returns the TempDir
    /// too so the caller keeps it alive (dropping it deletes the file).
    fn fresh() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_db(&dir.path().join("marie-lookup.db")).unwrap();
        (dir, conn)
    }

    fn user_version(conn: &Connection) -> i64 {
        conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap()
    }

    fn table_exists(conn: &Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [name],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
            > 0
    }

    fn column_exists(conn: &Connection, table: &str, col: &str) -> bool {
        conn.prepare(&format!("PRAGMA table_info({table})"))
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .any(|c| c == col)
    }

    // ---- migration ladder --------------------------------------------------

    #[test]
    fn fresh_db_is_current_version_with_all_tables() {
        let (_dir, conn) = fresh();
        assert_eq!(user_version(&conn), 5, "fresh DB must be at the current schema version");
        for t in ["entries", "settings", "entry_versions", "paste_history"] {
            assert!(table_exists(&conn, t), "missing table {t}");
        }
        assert!(column_exists(&conn, "entries", "deleted_at"));
    }

    /// Hand-build a pre-versioning (user_version=0) DB with only the v1 schema
    /// and real data, then prove `open_db` migrates it all the way to v5
    /// without dropping the row.
    #[test]
    fn migrates_pre_versioning_db_to_current_preserving_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("marie-lookup.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE entries (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    title TEXT NOT NULL,
                    body TEXT NOT NULL,
                    created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
                    updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
                );
                INSERT INTO entries (title, body) VALUES ('legacy', 'kept');",
            )
            .unwrap();
            // user_version stays 0 — simulates a DB created before versioning.
        }
        let conn = open_db(&path).unwrap();
        assert_eq!(user_version(&conn), 5);
        for t in ["settings", "entry_versions", "paste_history"] {
            assert!(table_exists(&conn, t), "migration should have created {t}");
        }
        assert!(column_exists(&conn, "entries", "deleted_at"));
        let (title, body): (String, String) = conn
            .query_row("SELECT title, body FROM entries WHERE id=1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!((title.as_str(), body.as_str()), ("legacy", "kept"));
    }

    /// A v3 DB (entries+settings+entry_versions, but no deleted_at and no
    /// paste_history) must gain the v4 column and v5 table with data intact.
    #[test]
    fn migrates_v3_db_adding_deleted_at_and_paste_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("marie-lookup.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE entries (id INTEGER PRIMARY KEY AUTOINCREMENT,
                    title TEXT NOT NULL, body TEXT NOT NULL,
                    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
                 CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE entry_versions (id INTEGER PRIMARY KEY AUTOINCREMENT,
                    entry_id INTEGER NOT NULL, title TEXT NOT NULL,
                    body TEXT NOT NULL, saved_at INTEGER NOT NULL);
                 INSERT INTO entries (title, body, created_at, updated_at)
                    VALUES ('v3', 'survives', 1000, 1000);
                 PRAGMA user_version = 3;",
            )
            .unwrap();
        }
        let conn = open_db(&path).unwrap();
        assert_eq!(user_version(&conn), 5);
        assert!(column_exists(&conn, "entries", "deleted_at"));
        assert!(table_exists(&conn, "paste_history"));
        let deleted_at: Option<i64> = conn
            .query_row("SELECT deleted_at FROM entries WHERE id=1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(deleted_at, None);
    }

    /// The 90-day purge in `open_db` removes trashed entries (and their
    /// version history) older than the cutoff, and only those.
    #[test]
    fn sweep_purges_trash_older_than_90_days() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("marie-lookup.db");
        {
            let conn = open_db(&path).unwrap();
            conn.execute(
                "INSERT INTO entries (id, title, body, deleted_at)
                 VALUES (1, 'old', 'x', strftime('%s','now') - 91*86400)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO entry_versions (entry_id, title, body, saved_at)
                 VALUES (1, 'old', 'x', 1000)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO entries (id, title, body, deleted_at)
                 VALUES (2, 'recent', 'y', strftime('%s','now') - 89*86400)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO entries (id, title, body) VALUES (3, 'live', 'z')",
                [],
            )
            .unwrap();
        }
        let conn = open_db(&path).unwrap();
        let surviving: Vec<i64> = conn
            .prepare("SELECT id FROM entries ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert_eq!(surviving, vec![2, 3], "only the 91d-old trash should be purged");
        let orphan_versions: i64 = conn
            .query_row("SELECT count(*) FROM entry_versions WHERE entry_id=1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(orphan_versions, 0, "purged entry's history must be gone too");
    }

    // ---- soft-delete invariant --------------------------------------------

    #[test]
    fn soft_delete_excludes_from_list_search_and_recents_but_shows_in_trash() {
        let (_dir, conn) = fresh();
        let keep = add_entry(&conn, "keep", "alpha").unwrap();
        let gone = add_entry(&conn, "gone", "alpha").unwrap();
        // A paste from each, so recent_topics would surface both if not filtered.
        record_paste(&conn, Some(keep), "alpha").unwrap();
        record_paste(&conn, Some(gone), "alpha").unwrap();
        delete_entry(&conn, gone).unwrap();

        let live: Vec<i64> = list_entries(&conn).unwrap().iter().map(|e| e.id).collect();
        assert_eq!(live, vec![keep], "trashed entry must not be in the live list");

        let found: Vec<i64> = search_entries(&conn, "alpha").unwrap().iter().map(|e| e.id).collect();
        assert_eq!(found, vec![keep], "trashed entry must not be searchable");

        let recents: Vec<i64> = recent_topics(&conn).unwrap().iter().map(|e| e.id).collect();
        assert_eq!(recents, vec![keep], "trashed entry must not appear in recents");

        let trash: Vec<i64> = list_trash(&conn).unwrap().iter().map(|e| e.id).collect();
        assert_eq!(trash, vec![gone], "trashed entry must show in the trash view");

        // ...and restoring brings it back to the live list.
        restore_entry(&conn, gone).unwrap();
        assert_eq!(list_entries(&conn).unwrap().len(), 2);
        assert!(list_trash(&conn).unwrap().is_empty());
    }

    #[test]
    fn search_escapes_like_metacharacters() {
        let (_dir, conn) = fresh();
        let literal = add_entry(&conn, "100%", "discount").unwrap();
        add_entry(&conn, "fifty", "off").unwrap();
        // '%' must be treated literally, not as a wildcard — so this matches
        // only the entry that actually contains "100%".
        let hits: Vec<i64> = search_entries(&conn, "100%").unwrap().iter().map(|e| e.id).collect();
        assert_eq!(hits, vec![literal]);
        // A bare '%' would match *every* row if treated as a wildcard. Escaped,
        // it matches only the row that literally contains '%' — proving the
        // escaping works ("fifty" must be excluded).
        let pct: Vec<i64> = search_entries(&conn, "%").unwrap().iter().map(|e| e.id).collect();
        assert_eq!(pct, vec![literal], "'%' must be literal, not a match-all wildcard");
    }

    // ---- history & paste caps ---------------------------------------------

    #[test]
    fn update_snapshots_only_real_changes_and_caps_history_at_50() {
        let (_dir, conn) = fresh();
        let id = add_entry(&conn, "t", "v0").unwrap();
        // A no-op save must not create a version.
        update_entry(&conn, id, "t", "v0").unwrap();
        assert_eq!(list_versions(&conn, id).unwrap().len(), 0, "no-op save must not snapshot");
        // 60 real edits → history capped at 50, keeping the newest.
        for i in 1..=60 {
            update_entry(&conn, id, "t", &format!("v{i}")).unwrap();
        }
        let versions = list_versions(&conn, id).unwrap();
        assert_eq!(versions.len(), 50, "history must be capped at 50 per entry");
        // Newest snapshot is the body that was current just before the last edit.
        assert_eq!(versions.first().unwrap().body, "v59");
    }

    #[test]
    fn record_paste_caps_history_at_100() {
        let (_dir, conn) = fresh();
        let id = add_entry(&conn, "t", "b").unwrap();
        for i in 0..130 {
            record_paste(&conn, Some(id), &format!("p{i}")).unwrap();
        }
        let count: i64 = conn
            .query_row("SELECT count(*) FROM paste_history", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 100, "paste_history must be capped at 100 rows");
    }

    // ---- version-tolerant restore -----------------------------------------

    /// Build a standalone source DB at `version` with the period-correct
    /// schema, seed it, and return its path (kept alive by the TempDir).
    fn make_source(dir: &std::path::Path, name: &str, version: i64) -> String {
        let path = dir.join(name);
        let conn = Connection::open(&path).unwrap();
        // entries table: deleted_at only exists from v4.
        if version >= 4 {
            conn.execute_batch(
                "CREATE TABLE entries (id INTEGER PRIMARY KEY, title TEXT NOT NULL,
                   body TEXT NOT NULL, created_at INTEGER NOT NULL,
                   updated_at INTEGER NOT NULL, deleted_at INTEGER);",
            )
            .unwrap();
        } else {
            conn.execute_batch(
                "CREATE TABLE entries (id INTEGER PRIMARY KEY, title TEXT NOT NULL,
                   body TEXT NOT NULL, created_at INTEGER NOT NULL,
                   updated_at INTEGER NOT NULL);",
            )
            .unwrap();
        }
        if version >= 3 {
            conn.execute_batch(
                "CREATE TABLE entry_versions (id INTEGER PRIMARY KEY, entry_id INTEGER NOT NULL,
                   title TEXT NOT NULL, body TEXT NOT NULL, saved_at INTEGER NOT NULL);",
            )
            .unwrap();
        }
        if version >= 5 {
            conn.execute_batch(
                "CREATE TABLE paste_history (id INTEGER PRIMARY KEY, entry_id INTEGER,
                   text TEXT NOT NULL, pasted_at INTEGER NOT NULL);",
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO entries (id, title, body, created_at, updated_at)
             VALUES (1, 'from_backup', 'b', 1000, 1000)",
            [],
        )
        .unwrap();
        conn.pragma_update(None, "user_version", version).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn restore_replaces_entries_and_returns_count() {
        let (_dir, conn) = fresh();
        // Destination starts with its own (different) data.
        add_entry(&conn, "local", "x").unwrap();
        let src = make_source(_dir.path(), "v5.db", 5);
        let count = restore_from_backup(&conn, &src).unwrap();
        assert_eq!(count, 1);
        let titles: Vec<String> = conn
            .prepare("SELECT title FROM entries")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert_eq!(titles, vec!["from_backup".to_string()], "restore replaces, not merges");
    }

    #[test]
    fn restore_from_pre_v4_backup_brings_entries_back_as_live() {
        let (_dir, conn) = fresh();
        let src = make_source(_dir.path(), "v3.db", 3);
        restore_from_backup(&conn, &src).unwrap();
        let deleted_at: Option<i64> = conn
            .query_row("SELECT deleted_at FROM entries WHERE id=1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(deleted_at, None, "pre-v4 backup has no deleted_at → restores live");
        // And it's visible in the live list.
        assert_eq!(list_entries(&conn).unwrap().len(), 1);
    }

    #[test]
    fn restore_clears_local_history_when_backup_predates_versions() {
        let (_dir, conn) = fresh();
        // Local entry with history that the (pre-v3) backup can't describe.
        let id = add_entry(&conn, "local", "v0").unwrap();
        update_entry(&conn, id, "local", "v1").unwrap();
        assert_eq!(list_versions(&conn, id).unwrap().len(), 1);
        let src = make_source(_dir.path(), "pre_v3.db", 2);
        restore_from_backup(&conn, &src).unwrap();
        let total_versions: i64 = conn
            .query_row("SELECT count(*) FROM entry_versions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total_versions, 0, "stale local history must be cleared, not kept");
    }

    #[test]
    fn restore_rejects_non_backup_and_rolls_back() {
        let (_dir, conn) = fresh();
        add_entry(&conn, "precious", "keep me").unwrap();
        // A valid SQLite file with no `entries` table.
        let bad = _dir.path().join("not_marie.db");
        Connection::open(&bad)
            .unwrap()
            .execute_batch("CREATE TABLE notes (x TEXT);")
            .unwrap();
        let err = restore_from_backup(&conn, &bad.to_string_lossy());
        assert!(err.is_err(), "a non-Marie DB must be rejected");
        // Destination data must be untouched (rollback + DETACH succeeded).
        let titles: Vec<String> = conn
            .prepare("SELECT title FROM entries")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert_eq!(titles, vec!["precious".to_string()], "failed restore must not destroy data");
    }
}
