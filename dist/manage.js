const { invoke } = window.__TAURI__.core;

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
  for (const e of filtered) {
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
  if (!isEditing() || inHistory()) {
    $saveBtn.disabled = true;
    return;
  }
  if (selectedId == null) {
    $saveBtn.disabled = !($title.value.trim() || $body.value.trim());
  } else {
    $saveBtn.disabled = $title.value === loadedTitle && $body.value === loadedBody;
  }
}

function selectEntry(id) {
  const e = entries.find((x) => x.id === id);
  if (!e) return;
  exitHistory();
  selectedId = id;
  $title.value = e.title;
  $body.value = e.body;
  loadedTitle = e.title;
  loadedBody = e.body;
  document.body.classList.add("editing", "has-selection");
  renderList();
  updateSaveState();
}

function newEntry() {
  exitHistory();
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

async function save() {
  if ($saveBtn.disabled || inHistory()) return;
  const title = $title.value.trim();
  const body = $body.value;
  if (!title && !body.trim()) return;
  try {
    if (selectedId == null) {
      const id = await invoke("add_entry", { title, body });
      selectedId = id;
    } else {
      await invoke("update_entry", { id: selectedId, title, body });
    }
    await refresh();
    if (selectedId != null) selectEntry(selectedId);
  } catch (e) {
    console.error("save failed", e);
    alert("Save failed: " + e);
  }
}

async function del() {
  if (selectedId == null) return;
  if (!confirm("Delete this entry?")) return;
  try {
    await invoke("delete_entry", { id: selectedId });
    selectedId = null;
    document.body.classList.remove("editing", "has-selection");
    await refresh();
  } catch (e) {
    console.error("delete failed", e);
    alert("Delete failed: " + e);
  }
}

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
  let list;
  try {
    list = await invoke("list_versions", { entryId: selectedId });
  } catch (e) {
    alert("Couldn't load history: " + e);
    return;
  }
  if (!list.length) {
    alert("No previous versions of this entry yet — history starts with the next save.");
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

document.addEventListener("keydown", (e) => {
  if ((e.metaKey || e.ctrlKey) && e.key === "s") {
    e.preventDefault();
    if (isEditing()) save();
  } else if ((e.metaKey || e.ctrlKey) && e.key === "n") {
    e.preventDefault();
    newEntry();
  } else if (e.key === "Escape" && inHistory()) {
    exitHistory();
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
    alert("Couldn't update launch-at-login setting: " + e);
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
    backupDir = info.dir;
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

$backupNow.addEventListener("click", async () => {
  $backupNow.disabled = true;
  try {
    await invoke("backup_now");
    await refreshBackups();
  } catch (e) {
    alert("Backup failed: " + e);
  } finally {
    $backupNow.disabled = false;
  }
});

$backupFolder.addEventListener("click", async () => {
  try {
    const dir = await window.__TAURI__.dialog.open({
      directory: true,
      defaultPath: backupDir || undefined,
      title: "Choose backup folder",
    });
    if (!dir) return;
    await invoke("set_backup_dir", { dir });
    await refreshBackups();
  } catch (e) {
    alert("Couldn't set backup folder: " + e);
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
    if (
      !confirm(
        "Replace ALL current entries with this backup?\n\n" +
          file +
          "\n\n(A snapshot of the current state is taken first, so this can be undone.)"
      )
    ) {
      return;
    }
    // Snapshot current state before overwriting it, so a wrong pick is undoable.
    await invoke("backup_now");
    const n = await invoke("restore_backup", { path: file });
    alert(`Restored ${n} entr${n === 1 ? "y" : "ies"}.`);
    selectedId = null;
    document.body.classList.remove("editing", "has-selection");
    await refresh();
    await refreshBackups();
  } catch (e) {
    alert("Restore failed: " + e);
  }
});

refresh();
refreshAutostart();
refreshBackups();
