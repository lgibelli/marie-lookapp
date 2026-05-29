use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use rusqlite::{params, Connection};
use serde::Serialize;
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    Manager,
};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

// CONTRACT: this exact error string is matched in dist/lookup.js (search for
// "accessibility_required"). If you rename it, update lookup.js too — there is
// no bundler / shared module across the Rust↔JS boundary to keep them in sync.
// Only the macOS paste path produces it (elsewhere `accessibility_ok` is a
// no-op), so the const is macOS-only.
#[cfg(target_os = "macos")]
const ERR_ACCESSIBILITY: &str = "accessibility_required";

pub struct Db(pub Mutex<Connection>);

/// Process-wide flags shared across commands, the tray, and window events.
struct AppState {
    /// True while `paste_text` is mid-flight (window hidden, focus handed back
    /// to the previous app, keystroke pending). Guards the global-hotkey and
    /// focus-loss handlers from re-showing/re-hiding the lookup window during
    /// that gap — otherwise the synthesized ⌘V can land in our own search box.
    is_pasting: AtomicBool,
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

#[derive(Serialize, Clone)]
pub struct Entry {
    pub id: i64,
    pub title: String,
    pub body: String,
}

/// Single source of truth for mapping a `SELECT id, title, body` row into an
/// `Entry` — shared by `list_entries` and `search_entries`.
fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<Entry> {
    Ok(Entry {
        id: row.get(0)?,
        title: row.get(1)?,
        body: row.get(2)?,
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
    Ok(conn)
}

#[tauri::command]
fn list_entries(db: tauri::State<Db>) -> Result<Vec<Entry>, AppError> {
    let conn = db.0.lock()?;
    let mut stmt = conn.prepare("SELECT id, title, body FROM entries ORDER BY updated_at DESC")?;
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
        "SELECT id, title, body FROM entries \
         WHERE title LIKE ?1 ESCAPE '\\' OR body LIKE ?1 ESCAPE '\\' \
         ORDER BY (title LIKE ?1 ESCAPE '\\') DESC, updated_at DESC \
         LIMIT 20",
    )?;
    let entries = stmt
        .query_map([&like], row_to_entry)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(entries)
}

#[tauri::command]
fn add_entry(db: tauri::State<Db>, title: String, body: String) -> Result<i64, AppError> {
    let conn = db.0.lock()?;
    conn.execute(
        "INSERT INTO entries (title, body) VALUES (?1, ?2)",
        params![title, body],
    )?;
    Ok(conn.last_insert_rowid())
}

#[tauri::command]
fn update_entry(
    db: tauri::State<Db>,
    id: i64,
    title: String,
    body: String,
) -> Result<(), AppError> {
    let conn = db.0.lock()?;
    conn.execute(
        "UPDATE entries SET title=?1, body=?2, updated_at=strftime('%s','now') WHERE id=?3",
        params![title, body, id],
    )?;
    Ok(())
}

#[tauri::command]
fn delete_entry(db: tauri::State<Db>, id: i64) -> Result<(), AppError> {
    let conn = db.0.lock()?;
    conn.execute("DELETE FROM entries WHERE id=?1", params![id])?;
    Ok(())
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
async fn paste_text(app: tauri::AppHandle, text: String) -> Result<(), AppError> {
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
    cb.set_text(text)?;
    drop(cb);

    // Mark the paste in-flight so the global hotkey / focus-loss handler won't
    // re-show or re-hide the lookup window during the focus hand-off + keystroke.
    app.state::<AppState>()
        .is_pasting
        .store(true, Ordering::SeqCst);

    hide_lookup_window(&app);

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
    outcome
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

/// Show + focus the manage window. Shared by `open_manage` and the tray menu so
/// both restore a minimized window identically.
fn show_manage(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("manage") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn show_lookup(app: &tauri::AppHandle) {
    // Don't pop the lookup window back up while a paste is mid-flight — that
    // would steal focus from the target app and the ⌘V would land in our own
    // search box instead of the intended destination.
    if app.state::<AppState>().is_pasting.load(Ordering::SeqCst) {
        return;
    }
    if let Some(w) = app.get_webview_window("lookup") {
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
        .setup(|app| {
            // DB + shared state. Manage AppState before the tray/hotkey are wired
            // up so any handler that reads it always finds it.
            let path = db_path(app.handle());
            let conn = open_db(&path).expect("open db");
            app.manage(Db(Mutex::new(conn)));
            app.manage(AppState {
                is_pasting: AtomicBool::new(false),
            });

            // Tray menu. The accelerator on "Lookup…" is purely a visual hint —
            // the real global hotkey is registered below via the plugin.
            let show_item = MenuItemBuilder::with_id("show", "Lookup…")
                .accelerator("CmdOrCtrl+Alt+M")
                .build(app)?;
            let manage_item = MenuItemBuilder::with_id("manage", "Manage entries…").build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "Quit")
                .accelerator("CmdOrCtrl+Q")
                .build(app)?;
            let menu = MenuBuilder::new(app)
                .items(&[&show_item, &manage_item])
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
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            // Register the global hotkey: Cmd+Option+M (macOS) / Ctrl+Alt+M (win/linux).
            // M = Marie. Avoids Cmd+Space (Spotlight) and Cmd+Shift+Space (Character Viewer).
            #[cfg(desktop)]
            {
                #[cfg(target_os = "macos")]
                let shortcut = Shortcut::new(Some(Modifiers::META | Modifiers::ALT), Code::KeyM);
                #[cfg(not(target_os = "macos"))]
                let shortcut =
                    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyM);
                app.global_shortcut().register(shortcut)?;
            }

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
        ])
        .on_window_event(|window, event| {
            // Keep the app running when the manage window is closed:
            // just hide it instead of letting close terminate the process.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
