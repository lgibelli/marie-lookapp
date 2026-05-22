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
      renderMatches();
      renderBody();
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

async function doSearch() {
  const q = $search.value.trim();
  if (!q) {
    results = [];
    activeIdx = 0;
    renderMatches();
    renderBody();
    return;
  }
  try {
    results = await invoke("search_entries", { query: q });
  } catch (e) {
    console.error("search failed", e);
    results = [];
  }
  activeIdx = 0;
  renderMatches();
  renderBody();
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
    console.error("paste failed", e);
  }
  resetUI();
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
  renderMatches();
  renderBody();
}

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    e.preventDefault();
    cancel();
  } else if (e.key === "Enter") {
    e.preventDefault();
    paste();
  } else if (e.key === "ArrowDown" || (e.key === "Tab" && !e.shiftKey)) {
    if (results.length > 1) {
      e.preventDefault();
      activeIdx = (activeIdx + 1) % results.length;
      renderMatches();
      renderBody();
    }
  } else if (e.key === "ArrowUp" || (e.key === "Tab" && e.shiftKey)) {
    if (results.length > 1) {
      e.preventDefault();
      activeIdx = (activeIdx - 1 + results.length) % results.length;
      renderMatches();
      renderBody();
    }
  }
});

getCurrentWindow().listen("tauri://focus", () => {
  resetUI();
  setTimeout(() => $search.focus(), 0);
});

setTimeout(() => $search.focus(), 0);
