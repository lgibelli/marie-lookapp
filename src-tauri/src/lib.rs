use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    Emitter, Manager,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

mod backup;
mod db;
mod error;

use backup::{
    backup_dir, backup_maintenance, checkpoint_wal, do_backup, list_backup_files, schedule_backup,
    BackupFile, BackupInfo,
};
use db::{Db, Entry, EntryVersion, TrashedEntry};
use error::AppError;

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

/// Handle to the tray's "Lookup…" item so `set_hotkey` can keep its
/// accelerator hint in sync with the live combo — it's set at build time
/// from whatever registered at startup and would otherwise go stale the
/// moment the user records a different hotkey.
struct TrayMenu {
    show_item: tauri::menu::MenuItem<tauri::Wry>,
}

/// Process-wide flags shared across commands, the tray, and window events.
pub(crate) struct AppState {
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
    pub(crate) backup_gen: AtomicU64,
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

fn db_path(app: &tauri::AppHandle) -> std::path::PathBuf {
    let dir = app.path().app_data_dir().expect("app_data_dir");
    std::fs::create_dir_all(&dir).ok();
    dir.join("marie-lookup.db")
}

#[tauri::command]
fn list_entries(db: tauri::State<Db>) -> Result<Vec<Entry>, AppError> {
    let conn = db.0.lock()?;
    Ok(db::list_entries(&conn)?)
}

#[tauri::command]
fn search_entries(db: tauri::State<Db>, query: String) -> Result<Vec<Entry>, AppError> {
    let conn = db.0.lock()?;
    Ok(db::search_entries(&conn, &query)?)
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
        db::add_entry(&conn, &title, &body)?
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
        db::update_entry(&conn, id, &title, &body)?;
    }
    schedule_backup(&app);
    Ok(())
}

#[tauri::command]
fn delete_entry(app: tauri::AppHandle, db: tauri::State<Db>, id: i64) -> Result<(), AppError> {
    {
        let conn = db.0.lock()?;
        db::delete_entry(&conn, id)?;
    }
    schedule_backup(&app);
    Ok(())
}

/// Soft-deleted entries, newest deletion first. Excluded from lookup search
/// and the live list; hard-purged with their history after 90 days.
#[tauri::command]
fn list_trash(db: tauri::State<Db>) -> Result<Vec<TrashedEntry>, AppError> {
    let conn = db.0.lock()?;
    Ok(db::list_trash(&conn)?)
}

#[tauri::command]
fn restore_entry(app: tauri::AppHandle, db: tauri::State<Db>, id: i64) -> Result<(), AppError> {
    {
        let conn = db.0.lock()?;
        db::restore_entry(&conn, id)?;
    }
    schedule_backup(&app);
    Ok(())
}

/// Previous versions of an entry, newest first. Powers the Manage window's
/// view-only time machine.
#[tauri::command]
fn list_versions(db: tauri::State<Db>, entry_id: i64) -> Result<Vec<EntryVersion>, AppError> {
    let conn = db.0.lock()?;
    Ok(db::list_versions(&conn, entry_id)?)
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
        db::set_setting(&conn, "backup_dir", &dir)?;
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
        db::restore_from_backup(&conn, &path)?
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
            let _ = db::record_paste(&conn, entry_id, &text);
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
    Ok(db::recent_topics(&conn)?)
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
    db::set_setting(&conn, "hotkey", &hotkey)?;
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

/// Open this app's license (AGPL-3.0) on the public source repo. Hand-rolled to
/// match `open_releases_url` rather than pulling in tauri-plugin-opener.
fn open_license_url() -> std::io::Result<()> {
    let url = "https://github.com/lgibelli/marie-lookapp/blob/main/LICENSE";
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
fn open_license_page() -> Result<(), AppError> {
    open_license_url()?;
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

/// Tray "About Marie Lookup…": a tiny native dialog — name + version + license +
/// a button to view the license. Deliberately NOT the startup window (that's the
/// richer status panel "Check for updates…" uses); About is at-a-glance.
fn show_about(app: &tauri::AppHandle) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
    let version = app.package_info().version.to_string();
    app.dialog()
        .message(format!(
            "Marie Lookup v{version}\n\nLicensed under the GNU Affero General Public License v3.0 (AGPL-3.0-or-later)."
        ))
        .title("About Marie Lookup")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "View license".into(),
            "Close".into(),
        ))
        .show(|view_license| {
            if view_license {
                let _ = open_license_url();
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
        // MUST be the first plugin. A tray app with autostart can otherwise
        // end up with two processes (autostart + a manual/updater launch),
        // each with its own tray and SQLite connection — nondeterministic
        // behaviour (the "list sometimes shows" Windows symptom). The second
        // launch hands off to the first (summon its lookup) and exits.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            show_lookup(app);
        }))
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
            let conn = db::open_db(&path).expect("open db");
            // The user-chosen hotkey must be known before any webview JS runs,
            // so read it straight off the fresh connection here.
            let stored_hotkey = db::setting(&conn, "hotkey");
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
            open_license_page,
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
