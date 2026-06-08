const { invoke } = window.__TAURI__.core;
const { getCurrentWindow } = window.__TAURI__.window;

const $search = document.getElementById("search");
const $matches = document.getElementById("matches");
const $body = document.getElementById("body");

// Minimum query length before we search. 1–2 characters match almost
// everything, so we wait for the third keystroke (this also gates the body
// highlighting and the "keep typing" hint, so they stay in sync).
const MIN_SEARCH = 3;

let results = [];
let activeIdx = 0;
let searchTimer = null;

// Empty-search state: the last 3 entries something was pasted from, most
// recent first. They're loaded as ordinary results — pills + body — with the
// most recent one open, so more text can be selected from it immediately and
// the other recent entries are one click away. Fetched on every window focus;
// typing replaces them with real search hits.
let topics = [];
// True while showing the recent-topics state (vs. live search results).
let inTopics = false;

function renderMatches() {
  $matches.replaceChildren();
  // In topics mode always show the pills — even a single one — so the user
  // can see which recent entry the body belongs to and switch between them.
  // In search mode a lone result needs no pill.
  if (!inTopics && results.length <= 1) return;
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

// Render `text` into $body, bolding every case-insensitive occurrence of
// `q` (treated as a literal substring). Built from text nodes + <b> so the
// innerHTML-blocking project hook is never tripped.
function renderHighlighted(text, q) {
  $body.replaceChildren();
  const lowText = text.toLowerCase();
  const lowQ = q.toLowerCase();
  let i = 0;
  while (i <= text.length) {
    const idx = lowText.indexOf(lowQ, i);
    if (idx === -1) {
      if (i < text.length) $body.appendChild(document.createTextNode(text.slice(i)));
      break;
    }
    if (idx > i) $body.appendChild(document.createTextNode(text.slice(i, idx)));
    const b = document.createElement("b");
    b.className = "match";
    b.textContent = text.slice(idx, idx + q.length);
    $body.appendChild(b);
    i = idx + q.length;
  }
}

// Scroll $body so the first highlighted match sits ~2 lines below the top
// edge — a little context above it — instead of leaving the user to scroll a
// long entry by hand. The browser clamps scrollTop to [0, max].
function scrollToFirstMatch() {
  const first = $body.querySelector("b.match");
  if (!first) {
    $body.scrollTop = 0;
    return;
  }
  const lineHeight = parseFloat(getComputedStyle($body).lineHeight) || 20;
  const delta =
    first.getBoundingClientRect().top - $body.getBoundingClientRect().top - lineHeight * 2;
  $body.scrollTop += delta;
}

function renderBody() {
  if (!results.length) {
    $body.classList.add("empty");
    const q = $search.value.trim();
    $body.textContent = !q
      ? "Type to search."
      : q.length < MIN_SEARCH
      ? "Keep typing…"
      : "No matches.";
    return;
  }
  $body.classList.remove("empty");
  const result = results[activeIdx];
  const text = result.body;
  const q = $search.value.trim();
  // Bold the matched term inside the body, but only once the query is long
  // enough to be meaningful (≥ MIN_SEARCH) — short queries would highlight
  // noise. Topics mode has an empty query, so it stays plain.
  if (q.length >= MIN_SEARCH) {
    renderHighlighted(text, q);
    // Jump to the first body hit — but only when the match is in the content.
    // A title hit means the entry is relevant by name, so keep the body at the
    // top; diving into it would be disorienting. (If the title matches, the
    // body may not contain the term at all.)
    if (result.title.toLowerCase().includes(q.toLowerCase())) {
      $body.scrollTop = 0;
    } else {
      scrollToFirstMatch();
    }
  } else {
    $body.textContent = text;
    $body.scrollTop = 0;
  }
}

function render() {
  renderMatches();
  renderBody();
}

// Load the recent topics as the current result set (most recent one open).
function showTopics() {
  inTopics = topics.length > 0;
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
  inTopics = false;
  // Hold off until MIN_SEARCH chars: show no pills and a "keep typing" body
  // (via renderBody) rather than a flood of 1–2 char matches.
  if (q.length < MIN_SEARCH) {
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
