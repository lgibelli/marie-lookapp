# Marie LookApp

Tray-resident text snippet lookup with paste-at-cursor.

A small desktop app that lives in the menu bar / system tray. Press a global
hotkey, search your saved entries, optionally select a portion of the body,
and the text is pasted wherever your cursor is.

## Stack

- **Tauri 2** (Rust backend + native webview UI)
- **SQLite** via `rusqlite` (bundled) for storage
- **enigo** for cross-platform keystroke simulation (the paste)
- **arboard** for clipboard access

## Layout

```
src-tauri/   Rust backend, Tauri config
dist/        Static frontend (HTML/CSS/JS, no bundler)
scripts/     Icon generator
```

## Develop

```bash
npm install
npm run dev
```

The first `cargo` build downloads & compiles a lot of dependencies — expect
a few minutes initially. Subsequent builds are fast.

## Build a release bundle

```bash
npm run build
```

Produces a `.app` (macOS), `.msi` / `.exe` (Windows), or `.AppImage` / `.deb`
(Linux) in `src-tauri/target/release/bundle/`.

## Default hotkey

- macOS: **⌘⇧Space**
- Windows / Linux: **Ctrl+Shift+Space**

## macOS first-launch note

Auto-paste simulates the Cmd+V keystroke, which requires Accessibility
permission. The first paste attempt will trigger a system prompt — grant
Marie LookApp permission, then it just works.

## Manage entries

Click the tray icon → **Manage entries…** to add / edit / delete the
snippets that the lookup popup searches.

## History

This codebase replaces the original .NET / WPF implementation in this repo.
That old code lives on as the branch `legacy-abandoned-20260522`.
