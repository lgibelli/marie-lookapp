const { invoke } = window.__TAURI__.core;
const { getCurrentWindow } = window.__TAURI__.window;

// localStorage persists in the webview profile under the app data dir, so the
// dismissal survives restarts without touching the SQLite DB.
const DISMISS_KEY = "startup-notice-dismissed";

const win = getCurrentWindow();
const isMac = navigator.userAgent.includes("Mac");

const $hotkey = document.getElementById("hotkey");
const $status = document.getElementById("hotkey-status");
const $change = document.getElementById("change-hotkey");
const $recorder = document.getElementById("recorder");
const $cancel = document.getElementById("cancel-record");
const $ok = document.getElementById("ok");

document.getElementById("tray-name").textContent = isMac
  ? "menu bar (top right)"
  : "system tray (bottom right — check the ^ overflow)";

// Currently active hotkey in canonical accelerator syntax ("Control+Alt+M"),
// shared with the Rust side (get_hotkey / set_hotkey). null = none registered.
let active = null;

// "Control+Alt+M" → "Ctrl+Alt+M" (Windows/Linux) or "⌃⌥M" (macOS) for display.
// "Super" is the canonical Cmd/Win token — the only spelling the Rust-side
// parsers accept (see DEFAULT_HOTKEY in lib.rs).
function fmt(combo) {
  if (!combo) return "—";
  if (isMac) {
    return combo
      .replace("Control", "⌃")
      .replace("Alt", "⌥")
      .replace("Shift", "⇧")
      .replace("Super", "⌘")
      .replaceAll("+", "");
  }
  return combo.replace("Control", "Ctrl").replace("Super", "Win");
}

function setStatus(text, ok) {
  $status.textContent = text;
  $status.classList.toggle("ok", !!ok);
}

function render() {
  $hotkey.textContent = fmt(active);
  // Without a working hotkey the app would be tray-only — keep OK disabled
  // (and the notice shown) until the user records a free combo.
  $ok.disabled = !active;
}

function startRecording() {
  $change.classList.add("hidden");
  $recorder.classList.remove("hidden");
  $cancel.classList.remove("hidden");
  $recorder.value = "";
  $recorder.focus();
}

function stopRecording() {
  $recorder.classList.add("hidden");
  $cancel.classList.add("hidden");
  $change.classList.remove("hidden");
}

// Map a keydown to canonical accelerator syntax, or null while only modifiers
// are held. Letters / digits / F-keys only — those parse reliably on the Rust
// side (global_hotkey), and anything more exotic is a footgun as a global key.
function comboFromEvent(e) {
  let key = null;
  if (/^Key[A-Z]$/.test(e.code)) key = e.code.slice(3);
  else if (/^Digit[0-9]$/.test(e.code)) key = e.code.slice(5);
  else if (/^F([1-9]|1[0-9]|2[0-4])$/.test(e.code)) key = e.code;
  if (!key) return null;
  const mods = [];
  if (e.ctrlKey) mods.push("Control");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Super");
  return {
    combo: [...mods, key].join("+"),
    // Shift alone can't gate a global key (it would eat capital letters
    // everywhere); F-keys are fine bare.
    usable: e.ctrlKey || e.altKey || e.metaKey || key.length > 1,
  };
}

$recorder.addEventListener("keydown", async (e) => {
  e.preventDefault();
  e.stopPropagation();
  if (e.code === "Escape") {
    stopRecording();
    render();
    return;
  }
  const rec = comboFromEvent(e);
  if (!rec) {
    // Modifiers only so far — live preview.
    const mods = [];
    if (e.ctrlKey) mods.push("Control");
    if (e.altKey) mods.push("Alt");
    if (e.shiftKey) mods.push("Shift");
    if (e.metaKey) mods.push("Super");
    $recorder.value = mods.length ? fmt(mods.join("+")) + "+…" : "";
    return;
  }
  if (!rec.usable) {
    setStatus(
      `Add ${isMac ? "⌃, ⌥ or ⌘" : "Ctrl, Alt or Win"} — a bare key would fire while you type.`,
      false
    );
    return;
  }
  $recorder.value = fmt(rec.combo);
  try {
    await invoke("set_hotkey", { hotkey: rec.combo });
    active = rec.combo;
    stopRecording();
    setStatus(`Hotkey saved: ${fmt(rec.combo)}`, true);
    render();
  } catch (_err) {
    setStatus(
      `${fmt(rec.combo)} is taken by another app — try a different combo.`,
      false
    );
  }
});

// Clicking away cancels recording; Cancel is just a visible way to do it.
$recorder.addEventListener("blur", () => {
  stopRecording();
  render();
});
$cancel.addEventListener("click", stopRecording);
$change.addEventListener("click", startRecording);

document.getElementById("releases-link").addEventListener("click", () => {
  invoke("open_releases_page").catch(() => {});
});

$ok.addEventListener("click", async () => {
  if (document.getElementById("dont-show").checked) {
    localStorage.setItem(DISMISS_KEY, "1");
  }
  await win.hide();
});

// ---- Auto-update (tauri-plugin-updater) -----------------------------------
// Checks latest.json on the public releases repo at launch and then hourly
// (the app is tray-resident and can run for weeks). A found update pops the
// window with an Install/Later prompt — overriding the "don't show again"
// dismissal — but at most once per day PER VERSION: a brand-new release
// prompts immediately, declining it buys 24h of silence. Install downloads
// the signed NSIS installer and runs it silently; on Windows the app exits
// while the installer runs and the installer relaunches it (installer.nsi).

const UPDATE_PROMPT_KEY = "update-prompt-last-shown";
const CHECK_INTERVAL_MS = 60 * 60 * 1000; // hourly
const PROMPT_INTERVAL_MS = 24 * 60 * 60 * 1000; // once per day per version

const $updateRow = document.getElementById("update-row");
const $updateText = document.getElementById("update-text");
const $updateInstall = document.getElementById("update-install");
const $updateLater = document.getElementById("update-later");

let pendingUpdate = null;

async function checkForUpdate() {
  const updater = window.__TAURI__.updater;
  if (!updater) return false;
  // Drop the previous check's plugin-side resource before re-checking.
  if (pendingUpdate) {
    try {
      await pendingUpdate.close();
    } catch (_err) {
      /* already closed */
    }
    pendingUpdate = null;
  }
  try {
    pendingUpdate = await updater.check();
  } catch (_err) {
    // Offline, endpoint unreachable, or no artifact for this platform
    // (macOS for now) — stay quiet, this is a background check.
    return false;
  }
  if (!pendingUpdate) return false;
  $updateText.textContent =
    `Version ${pendingUpdate.version} is available ` +
    `(you have ${pendingUpdate.currentVersion}).`;
  $updateRow.classList.remove("hidden");
  return true;
}

function updatePromptAllowed(version) {
  try {
    const last = JSON.parse(localStorage.getItem(UPDATE_PROMPT_KEY) || "{}");
    if (last.version !== version) return true; // new release → prompt now
    return Date.now() - last.ts > PROMPT_INTERVAL_MS;
  } catch (_err) {
    return true;
  }
}

async function maybePromptUpdate() {
  if (!(await checkForUpdate())) return;
  if (!updatePromptAllowed(pendingUpdate.version)) return;
  localStorage.setItem(
    UPDATE_PROMPT_KEY,
    JSON.stringify({ version: pendingUpdate.version, ts: Date.now() })
  );
  await win.show();
}

// Backup reminder, emitted by backup_maintenance (lib.rs) when entries have
// changed but no backup happened for 72h — i.e. no backup folder is chosen
// yet, or backups keep failing. The folder is ALWAYS picked by the user;
// there is no automatic default path.
window.__TAURI__.event.listen("backup-nag", async () => {
  const dialog = window.__TAURI__.dialog;
  const choose = await dialog.ask(
    "Your entries have changed, but no backup has been made in the last 3 days.\n\n" +
      "Choose a backup folder now? Tip: pick a OneDrive/iCloud-synced folder " +
      "so backups are stored off this machine too.",
    { title: "Backup reminder", okLabel: "Choose folder…", cancelLabel: "Later" }
  );
  if (!choose) return;
  const dir = await dialog.open({ directory: true, title: "Choose backup folder" });
  if (!dir) return;
  try {
    await invoke("set_backup_dir", { dir }); // takes an immediate backup
    setStatus("Backup folder set — first backup done.", true);
  } catch (err) {
    await dialog.message("Couldn't set backup folder: " + err, {
      title: "Marie Lookup",
      kind: "error",
    });
  }
});

// Tray menu "Check for updates…" — an explicit user action, so it bypasses
// both the hourly cadence and the daily prompt cap, and always reports the
// outcome (including "up to date", which background checks never show).
window.__TAURI__.event.listen("check-updates", async () => {
  await win.show();
  setStatus("Checking for updates…", true);
  const found = await checkForUpdate();
  if (found) {
    setStatus("", true);
    localStorage.setItem(
      UPDATE_PROMPT_KEY,
      JSON.stringify({ version: pendingUpdate.version, ts: Date.now() })
    );
  } else {
    setStatus("You're running the latest version.", true);
  }
});

$updateLater.addEventListener("click", () => {
  $updateRow.classList.add("hidden");
});

$updateInstall.addEventListener("click", async () => {
  if (!pendingUpdate) return;
  $updateInstall.disabled = true;
  $updateLater.disabled = true;
  let total = 0;
  let got = 0;
  try {
    // Flush the WAL into the main .db before the installer force-kills us
    // (Windows: the running exe is locked and our WM_CLOSE-to-hide makes a
    // polite close a no-op, so the installer taskkill /F's us, skipping the
    // on-exit checkpoint). Belt-and-suspenders with the new version's
    // on-open checkpoint. Best-effort — don't block the update if it fails.
    try {
      await invoke("checkpoint_now");
    } catch (_err) {
      /* non-fatal */
    }
    await pendingUpdate.downloadAndInstall((ev) => {
      if (ev.event === "Started") {
        total = ev.data.contentLength || 0;
        setStatus("Downloading update…", true);
      } else if (ev.event === "Progress") {
        got += ev.data.chunkLength;
        const pct = total ? ` ${Math.round((got / total) * 100)}%` : "";
        setStatus(`Downloading update…${pct}`, true);
      } else if (ev.event === "Finished") {
        setStatus("Installing update — the app will restart…", true);
      }
    });
    // Windows never reaches this line — the installer exits the app and
    // relaunches it itself. On macOS the new .app is on disk but this old
    // process keeps running: relaunch into the new version ourselves.
    setStatus("Restarting…", true);
    await window.__TAURI__.process.relaunch();
  } catch (err) {
    setStatus(`Update failed: ${err}`, false);
    $updateInstall.disabled = false;
    $updateLater.disabled = false;
  }
});

async function init() {
  try {
    document.getElementById("app-version").textContent =
      await window.__TAURI__.app.getVersion();
  } catch (_err) {
    // cosmetic only
  }
  try {
    active = await invoke("get_hotkey");
  } catch (_err) {
    active = null;
  }
  render();
  if (!active || localStorage.getItem(DISMISS_KEY) !== "1") {
    await win.show();
  }
  if (!active) {
    // The default combo is owned by another app and the user hasn't picked a
    // replacement yet — warn and drop straight into recording. After show(),
    // so the recorder actually receives focus.
    setStatus(
      `The default hotkey ${fmt(isMac ? "Super+Alt+M" : "Control+Alt+M")} is ` +
        "taken by another app. Press a free combo to use instead.",
      false
    );
    startRecording();
  }
  // Background update check last — network-bound, must not delay the notice.
  // Repeats hourly for as long as the app stays resident.
  await maybePromptUpdate();
  setInterval(maybePromptUpdate, CHECK_INTERVAL_MS);
}

init();
