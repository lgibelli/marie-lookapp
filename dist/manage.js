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
    div.textContent = e.title || "(untitled)";
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

function selectEntry(id) {
  const e = entries.find((x) => x.id === id);
  if (!e) return;
  selectedId = id;
  $title.value = e.title;
  $body.value = e.body;
  document.body.classList.add("editing", "has-selection");
  renderList();
}

function newEntry() {
  selectedId = null;
  $title.value = "";
  $body.value = "";
  document.body.classList.add("editing");
  document.body.classList.remove("has-selection");
  renderList();
  setTimeout(() => $title.focus(), 0);
}

async function save() {
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

$newBtn.addEventListener("click", newEntry);
$saveBtn.addEventListener("click", save);
$deleteBtn.addEventListener("click", del);
$filter.addEventListener("input", renderList);

document.addEventListener("keydown", (e) => {
  if ((e.metaKey || e.ctrlKey) && e.key === "s") {
    e.preventDefault();
    if (isEditing()) save();
  } else if ((e.metaKey || e.ctrlKey) && e.key === "n") {
    e.preventDefault();
    newEntry();
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

refresh();
refreshAutostart();
