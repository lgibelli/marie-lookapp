const { invoke } = window.__TAURI__.core;

const $list = document.getElementById("list");
const $filter = document.getElementById("filter");
const $newBtn = document.getElementById("new-btn");
const $title = document.getElementById("title");
const $body = document.getElementById("body");
const $placeholder = document.getElementById("placeholder");
const $actions = document.getElementById("actions");
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

function showEditor() {
  $placeholder.style.display = "none";
  $title.style.display = "";
  $body.style.display = "";
  $actions.style.display = "";
}

function hideEditor() {
  $placeholder.style.display = "";
  $title.style.display = "none";
  $body.style.display = "none";
  $actions.style.display = "none";
}

function selectEntry(id) {
  const e = entries.find((x) => x.id === id);
  if (!e) return;
  selectedId = id;
  $title.value = e.title;
  $body.value = e.body;
  $deleteBtn.style.display = "";
  showEditor();
  renderList();
}

function newEntry() {
  selectedId = null;
  $title.value = "";
  $body.value = "";
  $deleteBtn.style.display = "none";
  showEditor();
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
    selectedId = undefined;
    hideEditor();
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
    if ($title.style.display !== "none") save();
  } else if ((e.metaKey || e.ctrlKey) && e.key === "n") {
    e.preventDefault();
    newEntry();
  }
});

refresh();
