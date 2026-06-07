const { invoke } = window.__TAURI__.core;
const { getCurrentWindow } = window.__TAURI__.window;

const $search = document.getElementById("search");
const $matches = document.getElementById("matches");
const $body = document.getElementById("body");

let results = [];
let activeIdx = 0;
let searchTimer = null;

// Empty-search state: the last 3 entries something was pasted from, most
// recent first. They're loaded as ordinary results — pills + body — with the
// most recent one open, so more text can be selected from it immediately.
// Fetched on every window focus; typing replaces them with real search hits.
let topics = [];

function renderMatches() {
  $matches.replaceChildren();
  if (results.length <= 1) return;
  results.forEach((r, i) => {
    const el = document.createElement("div");
    el.className = "pill" + (i === activeIdx ? " active" : "");
    el.textContent = r.title;
    el.addEventListener("mousedown", (e) => {
      e.preventDefault();
      activeIdx = i;
      render();
    });
    $matches.appendChild(el);
  });
}

function renderBody() {
  if (!results.length) {
    $body.classList.add("empty");
    $body.textContent = $search.value.trim()
      ? "No matches."
      : "Type to search.";
    return;
  }
  $body.classList.remove("empty");
  $body.textContent = results[activeIdx].body;
}

function render() {
  renderMatches();
  renderBody();
}

// Load the recent topics as the current result set (most recent one open).
function showTopics() {
  results = topics.slice();
  activeIdx = 0;
  render();
}

async function refreshTopics() {
  try {
    topics = await invoke("recent_topics");
  } catch (e) {
    console.error("recent_topics failed", e);
    topics = [];
  }
  if (!$search.value.trim()) showTopics();
}

// Move the selection by `delta` (wrapping). No-op with 0 or 1 results.
function move(delta) {
  if (results.length <= 1) return;
  activeIdx = (activeIdx + delta + results.length) % results.length;
  render();
}

async function doSearch() {
  const q = $search.value.trim();
  if (!q) {
    // Box cleared — fall back to the recent topics.
    showTopics();
    return;
  }
  try {
    results = await invoke("search_entries", { query: q });
  } catch (e) {
    console.error("search failed", e);
    results = [];
  }
  activeIdx = 0;
  render();
}

$search.addEventListener("input", () => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(doSearch, 120);
});

async function pasteText(text, entryId) {
  if (!text) return;
  try {
    await invoke("paste_text", { text, entryId: entryId ?? null });
  } catch (e) {
    // CONTRACT: this string matches ERR_ACCESSIBILITY returned by paste_text in
    // src-tauri/src/lib.rs. Keep both sides in sync — there's no shared module
    // across the Rust↔JS boundary.
    if (e === "accessibility_required") {
      showAccessibilityHint();
      return;
    }
    console.error("paste failed", e);
  }
  resetUI();
}

async function paste() {
  if (!results.length) return;
  const sel = window.getSelection().toString();
  const text = sel || results[activeIdx].body;
  await pasteText(text, results[activeIdx].id);
}

function showAccessibilityHint() {
  $body.classList.remove("empty");
  $body.replaceChildren();
  const title = document.createElement("h3");
  title.className = "ax-hint-title";
  title.textContent = "Accessibility permission needed";
  const p1 = document.createElement("p");
  p1.className = "ax-hint-text";
  p1.textContent =
    "Marie pastes by simulating ⌘V, which macOS only allows for apps in Accessibility. " +
    "System Settings just opened to the right pane — enable marie-lookup, then come back here and press Enter again.";
  const btn = document.createElement("button");
  btn.className = "ax-hint-btn";
  btn.textContent = "Re-open Accessibility settings";
  btn.addEventListener("click", () => invoke("open_accessibility_settings"));
  $body.append(title, p1, btn);
  $matches.replaceChildren();
}

async function cancel() {
  try {
    await invoke("hide_lookup");
  } catch (e) {
    console.error("hide failed", e);
  }
  resetUI();
}

function resetUI() {
  $search.value = "";
  showTopics();
}

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    e.preventDefault();
    cancel();
  } else if (e.key === "Enter") {
    e.preventDefault();
    paste();
  } else if (e.key === "ArrowDown" || (e.key === "Tab" && !e.shiftKey)) {
    if (results.length <= 1) return;
    e.preventDefault();
    move(1);
  } else if (e.key === "ArrowUp" || (e.key === "Tab" && e.shiftKey)) {
    if (results.length <= 1) return;
    e.preventDefault();
    move(-1);
  }
});

getCurrentWindow().listen("tauri://focus", () => {
  resetUI();
  refreshTopics();
  setTimeout(() => $search.focus(), 0);
});

refreshTopics();
setTimeout(() => $search.focus(), 0);
