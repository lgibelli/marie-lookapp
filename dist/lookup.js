const { invoke } = window.__TAURI__.core;
const { getCurrentWindow } = window.__TAURI__.window;

const $search = document.getElementById("search");
const $matches = document.getElementById("matches");
const $body = document.getElementById("body");

let results = [];
let activeIdx = 0;
let searchTimer = null;

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

// Move the selection by `delta` (wrapping). No-op with 0 or 1 results.
function move(delta) {
  if (results.length <= 1) return;
  activeIdx = (activeIdx + delta + results.length) % results.length;
  render();
}

async function doSearch() {
  const q = $search.value.trim();
  if (!q) {
    results = [];
    activeIdx = 0;
    render();
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

async function paste() {
  if (!results.length) return;
  const sel = window.getSelection().toString();
  const text = sel || results[activeIdx].body;
  if (!text) return;
  try {
    await invoke("paste_text", { text });
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

function showAccessibilityHint() {
  $body.classList.remove("empty");
  $body.replaceChildren();
  const title = document.createElement("h3");
  title.style.margin = "0 0 8px";
  title.textContent = "Accessibility permission needed";
  const p1 = document.createElement("p");
  p1.style.margin = "0 0 12px";
  p1.textContent =
    "Marie pastes by simulating ⌘V, which macOS only allows for apps in Accessibility. " +
    "System Settings just opened to the right pane — enable marie-lookup, then come back here and press Enter again.";
  const btn = document.createElement("button");
  btn.textContent = "Re-open Accessibility settings";
  btn.style.cssText =
    "padding:6px 12px;border-radius:6px;border:1px solid var(--border);" +
    "background:var(--accent);color:white;cursor:pointer;font-size:13px;";
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
  results = [];
  activeIdx = 0;
  render();
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
  setTimeout(() => $search.focus(), 0);
});

setTimeout(() => $search.focus(), 0);
