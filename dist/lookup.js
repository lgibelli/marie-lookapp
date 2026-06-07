const { invoke } = window.__TAURI__.core;
const { getCurrentWindow } = window.__TAURI__.window;

const $search = document.getElementById("search");
const $matches = document.getElementById("matches");
const $body = document.getElementById("body");

let results = [];
let activeIdx = 0;
let searchTimer = null;

// Empty-search state: recently pasted snippets + the entries they came from
// ("topics"). Fetched on every window focus; replaced by normal results the
// moment the user types.
let recents = { pastes: [], topics: [] };

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

// Topic pills in the matches row, recently-pasted snippets as clickable rows
// in the body area. Clicking a topic shows it like a single search result;
// clicking a snippet pastes it straight away.
function renderRecents() {
  $matches.replaceChildren();
  for (const t of recents.topics) {
    const el = document.createElement("div");
    el.className = "pill";
    el.textContent = t.title;
    el.addEventListener("mousedown", (e) => {
      e.preventDefault();
      results = [t];
      activeIdx = 0;
      render();
    });
    $matches.appendChild(el);
  }
  $body.replaceChildren();
  if (!recents.pastes.length) {
    $body.classList.add("empty");
    $body.textContent = "Type to search.";
    return;
  }
  $body.classList.remove("empty");
  const hint = document.createElement("div");
  hint.className = "recent-hint";
  hint.textContent = "Recently pasted — click to paste again:";
  $body.appendChild(hint);
  for (const p of recents.pastes) {
    const row = document.createElement("div");
    row.className = "recent-row";
    row.textContent = p.text;
    row.title = p.text;
    row.addEventListener("mousedown", (e) => {
      e.preventDefault();
      pasteText(p.text, p.entry_id);
    });
    $body.appendChild(row);
  }
}

function render() {
  if (!$search.value.trim() && !results.length) {
    renderRecents();
    return;
  }
  renderMatches();
  renderBody();
}

async function refreshRecents() {
  try {
    recents = await invoke("recent_lookups");
  } catch (e) {
    console.error("recent_lookups failed", e);
    recents = { pastes: [], topics: [] };
  }
  if (!$search.value.trim() && !results.length) render();
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
  refreshRecents();
  setTimeout(() => $search.focus(), 0);
});

refreshRecents();
setTimeout(() => $search.focus(), 0);
