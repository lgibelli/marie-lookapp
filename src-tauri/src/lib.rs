use std::sync::Mutex;

use rusqlite::{params, Connection};
use serde::Serialize;
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    Manager,
};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

pub struct Db(pub Mutex<Connection>);

#[derive(Serialize, Clone)]
pub struct Entry {
    pub id: i64,
    pub title: String,
    pub body: String,
}

fn db_path(app: &tauri::AppHandle) -> std::path::PathBuf {
    let dir = app.path().app_data_dir().expect("app_data_dir");
    std::fs::create_dir_all(&dir).ok();
    dir.join("marie-lookup.db")
}

fn open_db(path: &std::path::Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS entries (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            title TEXT NOT NULL,
            body TEXT NOT NULL,
            created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
            updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
        );
        CREATE INDEX IF NOT EXISTS idx_entries_title ON entries(title);",
    )?;
    Ok(conn)
}

#[tauri::command]
fn list_entries(db: tauri::State<Db>) -> Result<Vec<Entry>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT id, title, body FROM entries ORDER BY updated_at DESC")
        .map_err(|e| e.to_string())?;
    let iter = stmt
        .query_map([], |row| {
            Ok(Entry {
                id: row.get(0)?,
                title: row.get(1)?,
                body: row.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in iter {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
fn search_entries(db: tauri::State<Db>, query: String) -> Result<Vec<Entry>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let like = format!("%{}%", q);
    let mut stmt = conn
        .prepare(
            "SELECT id, title, body FROM entries \
             WHERE LOWER(title) LIKE ?1 OR LOWER(body) LIKE ?1 \
             ORDER BY (LOWER(title) LIKE ?1) DESC, updated_at DESC \
             LIMIT 20",
        )
        .map_err(|e| e.to_string())?;
    let iter = stmt
        .query_map([&like], |row| {
            Ok(Entry {
                id: row.get(0)?,
                title: row.get(1)?,
                body: row.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in iter {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
fn add_entry(db: tauri::State<Db>, title: String, body: String) -> Result<i64, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO entries (title, body) VALUES (?1, ?2)",
        params![title, body],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

#[tauri::command]
fn update_entry(
    db: tauri::State<Db>,
    id: i64,
    title: String,
    body: String,
) -> Result<(), String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE entries SET title=?1, body=?2, updated_at=strftime('%s','now') WHERE id=?3",
        params![title, body, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn delete_entry(db: tauri::State<Db>, id: i64) -> Result<(), String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM entries WHERE id=?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn paste_text(app: tauri::AppHandle, text: String) -> Result<(), String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    cb.set_text(text).map_err(|e| e.to_string())?;
    drop(cb);

    if let Some(w) = app.get_webview_window("lookup") {
        let _ = w.hide();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = app.hide();
    }

    std::thread::sleep(std::time::Duration::from_millis(180));

    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
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
}

#[tauri::command]
fn hide_lookup(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("lookup") {
        let _ = w.hide();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = app.hide();
    }
    Ok(())
}

#[tauri::command]
fn open_manage(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("manage") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
    Ok(())
}

fn show_lookup(app: &tauri::AppHandle) {
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
        .setup(|app| {
            // DB
            let path = db_path(app.handle());
            let conn = open_db(&path).expect("open db");
            app.manage(Db(Mutex::new(conn)));

            // Tray menu
            let show_item = MenuItemBuilder::with_id("show", "Lookup…").build(app)?;
            let manage_item = MenuItemBuilder::with_id("manage", "Manage entries…").build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
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
                    "manage" => {
                        if let Some(w) = app.get_webview_window("manage") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            // Register the global shortcut
            #[cfg(desktop)]
            {
                #[cfg(target_os = "macos")]
                let shortcut = Shortcut::new(Some(Modifiers::META | Modifiers::SHIFT), Code::Space);
                #[cfg(not(target_os = "macos"))]
                let shortcut =
                    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::Space);
                app.global_shortcut().register(shortcut)?;
            }

            // Hide lookup when it loses focus
            if let Some(w) = app.get_webview_window("lookup") {
                let app_handle = app.handle().clone();
                w.on_window_event(move |event| {
                    if let tauri::WindowEvent::Focused(false) = event {
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
