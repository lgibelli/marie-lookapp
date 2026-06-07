const { invoke } = window.__TAURI__.core;
// Native dialogs — window.confirm()/alert() are unreliable in WKWebView
// (silently return undefined on macOS), so never use them here.
const { ask: dialogAsk, message: dialogMessage } = window.__TAURI__.dialog;
const { getCurrentWindow } = window.__TAURI__.window;

const $autostart = document.getElementById("autostart");
const $list = document.getElementById("list");
const $filter = document.getElementById("filter");
const $newBtn = document.getElementById("new-btn");
const $title = document.getElementById("title");
const $body = document.getElementById("body");
const $saveBtn = document.getElementById("save-btn");
const $deleteBtn = document.getElementById("delete-btn");

let entries = [];
let selectedId = null;

async function refresh() {
  try {
    entries = await invoke("list_entries");
  } catch (e) {
    console.error("list_entries failed", e);
    entries = [];
  }
  renderList();
  await refreshTrash();
}

// Resolves true when it's safe to leave the current entry. With unsaved
// changes (Save enabled ⇔ fields differ from what was loaded; history/trash
// views always have it disabled), asks Save / Discard via a native dialog.
// Choosing Save saves first and only proceeds if the save succeeded.
async function confirmLeave() {
  // Nothing open ⇒ nothing to lose (also guards the pristine just-launched
  // state, where the button's disabled state hasn't been synced yet).
  if (!isEditing() || $saveBtn.disabled) return true;
  const wantSave = await dialogAsk("Save changes before leaving this entry?", {
    title: "Unsaved changes",
    okLabel: "Save",
    cancelLabel: "Discard",
  });
  if (wantSave) return await save(); // proceed only if the save succeeded
  return true; // Discard
}

// Sidebar sort order. Default A–Z so the list is stable (saving an entry no
// longer makes it jump around); "recent" = most-recently-modified first.
// Persisted in localStorage (pure UI preference, device-local).
let sortMode = localStorage.getItem("sort-mode") === "recent" ? "recent" : "az";

function sortedEntries(list) {
  const copy = list.slice();
  if (sortMode === "recent") {
    copy.sort((a, b) => b.updated_at - a.updated_at);
  } else {
    copy.sort((a, b) =>
      (a.title || "").localeCompare(b.title || "", undefined, { sensitivity: "base" })
    );
  }
  return copy;
}

function renderList() {
  const q = $filter.value.trim().toLowerCase();
  const filtered = q
    ? entries.filter(
        (e) =>
          e.title.toLowerCase().includes(q) ||
          e.body.toLowerCase().includes(q),
      )
    : entries;
  const sorted = sortedEntries(filtered);
  $list.replaceChildren();
  if (!filtered.length) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = entries.length
      ? "No entries match."
      : "No entries yet. Click + New to add one.";
    $list.appendChild(empty);
    return;
  }
  for (const e of sorted) {
    const div = document.createElement("div");
    div.className = "item" + (e.id === selectedId ? " active" : "");
    const title = document.createElement("span");
    title.className = "item-title";
    title.textContent = e.title || "(untitled)";
    const date = document.createElement("span");
    date.className = "item-date";
    const d = new Date(e.updated_at * 1000);
    date.textContent = d.toLocaleDateString();
    date.title = "Last modified " + d.toLocaleString();
    div.append(title, date);
    div.addEventListener("click", () => selectEntry(e.id));
    $list.appendChild(div);
  }
}

// Editor visibility is driven by CSS classes on <body> (see style.css):
//   editing       — fields shown instead of the placeholder
//   has-selection — an existing entry is loaded (reveals the Delete button)
function isEditing() {
  return document.body.classList.contains("editing");
}

// Last-loaded values — the Save button stays disabled until the fields
// actually differ from them (and is re-disabled right after a save).
let loadedTitle = "";
let loadedBody = "";

function updateSaveState() {
  if (!isEditing() || inHistory() || viewingTrash()) {
    $saveBtn.disabled = true;
    return;
  }
  if (selectedId == null) {
    $saveBtn.disabled = !($title.value.trim() || $body.value.trim());
  } else {
    $saveBtn.disabled = $title.value === loadedTitle && $body.value === loadedBody;
  }
}

async function selectEntry(id) {
  // Clicking the entry that's already open must not reload (= wipe) edits.
  if (id === selectedId && !inHistory() && !viewingTrash()) return;
  if (!(await confirmLeave())) return;
  exitHistory();
  leaveTrashView();
  // Re-find AFTER confirmLeave — a Save in there reloads `entries`, so an
  // object captured before would be stale.
  const e = entries.find((x) => x.id === id);
  if (!e) return;
  selectedId = id;
  $title.value = e.title;
  $body.value = e.body;
  loadedTitle = e.title;
  loadedBody = e.body;
  document.body.classList.add("editing", "has-selection");
  renderList();
  updateSaveState();
}

async function newEntry() {
  if (!(await confirmLeave())) return;
  exitHistory();
  leaveTrashView();
  selectedId = null;
  $title.value = "";
  $body.value = "";
  loadedTitle = "";
  loadedBody = "";
  document.body.classList.add("editing");
  document.body.classList.remove("has-selection");
  renderList();
  updateSaveState();
  setTimeout(() => $title.focus(), 0);
}

// Returns true when the entry is safely persisted (so callers like
// confirmLeave can proceed), false on failure or nothing-to-save edge cases.
async function save() {
  if (inHistory()) return false;
  if ($saveBtn.disabled) return true; // nothing changed — already safe
  const title = $title.value.trim();
  const body = $body.value;
  if (!title && !body.trim()) return false;
  try {
    if (selectedId == null) {
      selectedId = await invoke("add_entry", { title, body });
    } else {
      await invoke("update_entry", { id: selectedId, title, body });
    }
    await refresh();
    // Reset the dirty baseline in place — selectEntry() deliberately
    // no-ops for the already-selected entry, so it can't do this for us.
    $title.value = title; // reflect the trimmed title that was saved
    loadedTitle = title;
    loadedBody = body;
    document.body.classList.add("editing", "has-selection");
    updateSaveState();
    return true;
  } catch (e) {
    console.error("save failed", e);
    await dialogMessage("Save failed: " + e, { title: "Marie Lookup", kind: "error" });
    return false;
  }
}

async function del() {
  if (selectedId == null) return;
  const ok = await dialogAsk("Move this entry to Deleted? It can be restored for 90 days.", {
    title: "Delete entry",
    okLabel: "Move to Deleted",
    cancelLabel: "Cancel",
  });
  if (!ok) return;
  try {
    await invoke("delete_entry", { id: selectedId });
    selectedId = null;
    document.body.classList.remove("editing", "has-selection");
    updateSaveState();
    await refresh();
  } catch (e) {
    console.error("delete failed", e);
    await dialogMessage("Delete failed: " + e, { title: "Marie Lookup", kind: "error" });
  }
}

// ---- Deleted (trash) section -------------------------------------------------
// Soft-deleted entries live here for 90 days (then the backend purges them,
// history included). Clicking one opens it read-only with a Restore button.

const $trashSection = document.getElementById("trash-section");
const $trashToggle = document.getElementById("trash-toggle");
const $trashList = document.getElementById("trash-list");
const $restoreBtn = document.getElementById("restore-btn");

let trash = [];
let trashOpen = false;
let trashViewId = null;

function viewingTrash() {
  return trashViewId != null;
}

function leaveTrashView() {
  if (!viewingTrash()) return;
  trashViewId = null;
  document.body.classList.remove("trash-view");
  $title.readOnly = false;
  $body.readOnly = false;
  renderTrashList();
}

async function refreshTrash() {
  try {
    trash = await invoke("list_trash");
  } catch (e) {
    console.error("list_trash failed", e);
    trash = [];
  }
  $trashSection.classList.toggle("hidden", trash.length === 0);
  $trashToggle.textContent = `Deleted (${trash.length}) ${trashOpen ? "▾" : "▸"}`;
  renderTrashList();
}

function renderTrashList() {
  $trashList.classList.toggle("hidden", !trashOpen);
  $trashList.replaceChildren();
  if (!trashOpen) return;
  for (const t of trash) {
    const div = document.createElement("div");
    div.className = "item" + (t.id === trashViewId ? " active" : "");
    const title = document.createElement("span");
    title.className = "item-title";
    title.textContent = t.title || "(untitled)";
    const date = document.createElement("span");
    date.className = "item-date";
    const d = new Date(t.deleted_at * 1000);
    date.textContent = d.toLocaleDateString();
    date.title = "Deleted " + d.toLocaleString();
    div.append(title, date);
    div.addEventListener("click", () => selectTrashed(t.id));
    $trashList.appendChild(div);
  }
}

async function selectTrashed(id) {
  const t = trash.find((x) => x.id === id);
  if (!t) return;
  if (id === trashViewId) return;
  if (!(await confirmLeave())) return;
  exitHistory();
  selectedId = null;
  trashViewId = id;
  $title.value = t.title;
  $body.value = t.body;
  $title.readOnly = true;
  $body.readOnly = true;
  document.body.classList.add("editing", "trash-view");
  document.body.classList.remove("has-selection");
  renderList();
  renderTrashList();
  updateSaveState();
}

$trashToggle.addEventListener("click", () => {
  trashOpen = !trashOpen;
  $trashToggle.textContent = `Deleted (${trash.length}) ${trashOpen ? "▾" : "▸"}`;
  renderTrashList();
});

$restoreBtn.addEventListener("click", async () => {
  if (!viewingTrash()) return;
  const id = trashViewId;
  try {
    await invoke("restore_entry", { id });
    leaveTrashView();
    document.body.classList.remove("editing", "trash-view");
    await refresh();
    selectEntry(id);
  } catch (e) {
    await dialogMessage("Restore failed: " + e, { title: "Marie Lookup", kind: "error" });
  }
});

// ---- Time machine (view-only version history) -------------------------------
// ↺ next to the title steps into the entry's saved versions (newest previous
// first); ‹ Older / Newer › navigate, Exit returns to the live, editable
// entry. Fields are read-only while viewing history (copying text out is
// fine), Save/Delete are disabled.

const $historyBtn = document.getElementById("history-btn");
const $historyBar = document.getElementById("history-bar");
const $historyInfo = document.getElementById("history-info");
const $historyOlder = document.getElementById("history-older");
const $historyNewer = document.getElementById("history-newer");
const $historyExit = document.getElementById("history-exit");

let versions = [];
let versionIdx = -1; // -1 = live entry, 0 = newest previous version

function inHistory() {
  return versionIdx >= 0;
}

function renderHistory() {
  if (!inHistory()) {
    $historyBar.classList.add("hidden");
    $title.readOnly = false;
    $body.readOnly = false;
    $deleteBtn.disabled = false;
    $historyBtn.disabled = false;
    updateSaveState();
    return;
  }
  const v = versions[versionIdx];
  $title.value = v.title;
  $body.value = v.body;
  $title.readOnly = true;
  $body.readOnly = true;
  $deleteBtn.disabled = true;
  $historyBtn.disabled = true;
  $historyInfo.textContent =
    `Viewing version ${versions.length - versionIdx} of ${versions.length}` +
    ` — saved ${new Date(v.saved_at * 1000).toLocaleString()} (read-only)`;
  $historyOlder.disabled = versionIdx >= versions.length - 1;
  $historyBar.classList.remove("hidden");
  $saveBtn.disabled = true;
}

function exitHistory() {
  if (!inHistory()) return;
  versionIdx = -1;
  versions = [];
  const e = entries.find((x) => x.id === selectedId);
  if (e) {
    $title.value = e.title;
    $body.value = e.body;
  }
  renderHistory();
}

$historyBtn.addEventListener("click", async () => {
  if (selectedId == null || inHistory()) return;
  // Entering history overwrites the fields with old versions.
  if (!(await confirmLeave())) return;
  let list;
  try {
    list = await invoke("list_versions", { entryId: selectedId });
  } catch (e) {
    await dialogMessage("Couldn't load history: " + e, { title: "Marie Lookup", kind: "error" });
    return;
  }
  if (!list.length) {
    await dialogMessage(
      "No previous versions of this entry yet — history starts with the next save.",
      { title: "Marie Lookup" }
    );
    return;
  }
  versions = list;
  versionIdx = 0;
  renderHistory();
});

$historyOlder.addEventListener("click", () => {
  if (versionIdx < versions.length - 1) {
    versionIdx += 1;
    renderHistory();
  }
});

$historyNewer.addEventListener("click", () => {
  if (versionIdx > 0) {
    versionIdx -= 1;
    renderHistory();
  } else {
    exitHistory();
  }
});

$historyExit.addEventListener("click", exitHistory);

$newBtn.addEventListener("click", newEntry);
$saveBtn.addEventListener("click", save);
$deleteBtn.addEventListener("click", del);
$filter.addEventListener("input", renderList);
$title.addEventListener("input", updateSaveState);
$body.addEventListener("input", updateSaveState);

const $sortAz = document.getElementById("sort-az");
const $sortRecent = document.getElementById("sort-recent");
function setSort(mode) {
  sortMode = mode;
  localStorage.setItem("sort-mode", mode);
  $sortAz.classList.toggle("active", mode === "az");
  $sortRecent.classList.toggle("active", mode === "recent");
  renderList();
}
$sortAz.addEventListener("click", () => setSort("az"));
$sortRecent.addEventListener("click", () => setSort("recent"));
// Reflect the persisted choice in the toggle on load.
setSort(sortMode);

document.addEventListener("keydown", (e) => {
  if ((e.metaKey || e.ctrlKey) && e.key === "s") {
    e.preventDefault();
    if (isEditing()) save();
  } else if ((e.metaKey || e.ctrlKey) && e.key === "n") {
    e.preventDefault();
    newEntry();
  } else if (e.key === "Escape" && inHistory()) {
    exitHistory();
  } else if (e.key === "Escape" && viewingTrash()) {
    leaveTrashView();
    document.body.classList.remove("editing", "trash-view");
  }
});

async function refreshAutostart() {
  try {
    $autostart.checked = await invoke("plugin:autostart|is_enabled");
  } catch (e) {
    console.error("autostart check failed", e);
    $autostart.disabled = true;
  }
}

$autostart.addEventListener("change", async () => {
  try {
    if ($autostart.checked) {
      await invoke("plugin:autostart|enable");
    } else {
      await invoke("plugin:autostart|disable");
    }
  } catch (e) {
    console.error("autostart toggle failed", e);
    $autostart.checked = !$autostart.checked;
    await dialogMessage("Couldn't update launch-at-login setting: " + e, {
      title: "Marie Lookup",
      kind: "error",
    });
  }
});

// ---- Backups ---------------------------------------------------------------
// Rust takes versioned snapshots automatically (debounced after every change
// + a daily safety net); this block is status display, manual backup, backup
// folder choice and restore. Point the folder at OneDrive/iCloud for off-site
// copies.

const $backupStatus = document.getElementById("backup-status");
const $backupNow = document.getElementById("backup-now");
const $backupFolder = document.getElementById("backup-folder");
const $backupRestore = document.getElementById("backup-restore");

let backupDir = null;

async function refreshBackups() {
  try {
    const info = await invoke("backup_info");
    backupDir = info.dir; // null until the user picks a folder
    if (!info.dir) {
      $backupStatus.textContent = "No backup folder chosen — click Folder…";
      $backupStatus.title = "";
      return;
    }
    if (info.backups.length) {
      const latest = new Date(info.backups[0].modified * 1000);
      $backupStatus.textContent =
        `${info.backups.length} backup${info.backups.length === 1 ? "" : "s"}` +
        ` · latest ${latest.toLocaleString()}`;
    } else {
      $backupStatus.textContent = "No backups yet";
    }
    $backupStatus.title = info.dir;
  } catch (e) {
    console.error("backup_info failed", e);
    $backupStatus.textContent = "Backups unavailable";
  }
}

// Folder choice is always explicit — there is no automatic default path.
async function chooseBackupFolder() {
  const dir = await window.__TAURI__.dialog.open({
    directory: true,
    defaultPath: backupDir || undefined,
    title: "Choose backup folder",
  });
  if (!dir) return false;
  await invoke("set_backup_dir", { dir });
  await refreshBackups();
  return true;
}

$backupNow.addEventListener("click", async () => {
  $backupNow.disabled = true;
  try {
    await invoke("backup_now");
    await refreshBackups();
  } catch (e) {
    // CONTRACT: matches the error string in do_backup (lib.rs).
    if (String(e) === "no_backup_dir") {
      try {
        await chooseBackupFolder(); // set_backup_dir does an immediate backup
      } catch (e2) {
        await dialogMessage("Couldn't set backup folder: " + e2, {
          title: "Marie Lookup",
          kind: "error",
        });
      }
    } else {
      await dialogMessage("Backup failed: " + e, { title: "Marie Lookup", kind: "error" });
    }
  } finally {
    $backupNow.disabled = false;
  }
});

$backupFolder.addEventListener("click", async () => {
  try {
    await chooseBackupFolder();
  } catch (e) {
    await dialogMessage("Couldn't set backup folder: " + e, {
      title: "Marie Lookup",
      kind: "error",
    });
  }
});

$backupRestore.addEventListener("click", async () => {
  try {
    const file = await window.__TAURI__.dialog.open({
      defaultPath: backupDir || undefined,
      title: "Choose a backup to restore",
      filters: [{ name: "Marie Lookup backup", extensions: ["db"] }],
    });
    if (!file) return;
    const ok = await dialogAsk(
      "Replace ALL current entries with this backup?\n\n" +
        file +
        "\n\n(A snapshot of the current state is taken first, so this can be undone.)",
      { title: "Restore backup", okLabel: "Restore", cancelLabel: "Cancel" }
    );
    if (!ok) return;
    // Snapshot current state before overwriting it, so a wrong pick is undoable.
    await invoke("backup_now");
    const n = await invoke("restore_backup", { path: file });
    await dialogMessage(`Restored ${n} entr${n === 1 ? "y" : "ies"}.`, {
      title: "Marie Lookup",
    });
    exitHistory();
    leaveTrashView();
    selectedId = null;
    document.body.classList.remove("editing", "has-selection", "trash-view");
    updateSaveState();
    await refresh();
    await refreshBackups();
  } catch (e) {
    await dialogMessage("Restore failed: " + e, { title: "Marie Lookup", kind: "error" });
  }
});

// Re-read the DB every time this window is shown/focused. The one-time
// startup read below can lose the race between WebView2 init and the backend
// DB state being ready on Windows, leaving a stale empty list that persists
// because opening "Manage entries" only *shows* the already-loaded window —
// it never reloads. (WKWebView on macOS always won that race, which is why it
// only misbehaved on Windows.) refresh() re-renders the list/trash only; it
// never touches the editor fields, so an in-progress edit is safe.
getCurrentWindow().listen("tauri://focus", () => {
  refresh();
  refreshBackups();
});

updateSaveState();
refresh();
refreshAutostart();
refreshBackups();
