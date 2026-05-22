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

## Cross-compile from macOS to Windows

For testing the Windows build without a Windows machine. Produces just the
`.exe` (the `.msi` installer step needs WiX, which runs only on Windows).

One-time setup:

```bash
~/.cargo/bin/rustup target add x86_64-pc-windows-msvc
~/.cargo/bin/rustup target add aarch64-pc-windows-msvc   # only if you want native ARM64
~/.cargo/bin/rustup component add llvm-tools-preview
~/.cargo/bin/cargo install --locked cargo-xwin
```

You also need `llvm-rc` (the Windows resource compiler) somewhere on disk
— `rustup`'s `llvm-tools-preview` doesn't ship it. Any of these work:

- An installed Android NDK (`~/Library/Android/sdk/ndk/.../llvm-rc`)
- `brew install llvm` (`/opt/homebrew/opt/llvm/bin/llvm-rc`)
- `sudo port install llvm-22` (or any recent llvm-*)

The script auto-discovers any of those.

Then:

```bash
scripts/build-windows.sh                 # release, x86_64
scripts/build-windows.sh release arm64   # release, aarch64
scripts/build-windows.sh release all     # both archs
scripts/build-windows.sh debug           # debug build, x86_64
```

Output:
- `src-tauri/target/x86_64-pc-windows-msvc/release/marie-lookup.exe`
- `src-tauri/target/aarch64-pc-windows-msvc/release/marie-lookup.exe`

Each `.exe` is self-contained (frontend + icon embedded). WebView2 must be
present on the target Windows machine — it ships pre-installed on Windows
11 and on most Windows 10 installs.

A x86_64 build runs on ARM64 Windows under Microsoft's built-in emulator
("Prism") with negligible overhead for a tray app. Build the `aarch64`
target only if you want fully native ARM performance.

First run downloads ~700 MB of MSVC headers via `cargo-xwin` into
`~/.cache/cargo-xwin/` and recompiles every dependency for the chosen
target (5–15 min). Subsequent builds are fast.

## Build a Windows installer (NSIS)

Wraps the x64 `.exe` in an NSIS installer that runs the WebView2 bootstrapper
at install time if the runtime isn't already present — so the installer is
small (~6 MB) and the end user needs no external downloads.

One-time setup:

```bash
sudo port install nsis      # MacPorts (preferred on this machine)
# or:
brew install makensis       # Homebrew
```

Then:

```bash
BUILD_INSTALLER=1 scripts/build-windows.sh release x64
```

Output: `src-tauri/target/marie-lookup-setup-<version>.exe`. Per-user install
(no UAC prompt). Adds a Start Menu shortcut and an entry under Settings →
Apps for clean uninstall. The first run also downloads the WebView2
bootstrapper (~2 MB) from Microsoft into `~/.cache/marie-lookup/`.

## Launch at login

Toggleable inside the app: open the *Manage entries* window → the sidebar
footer has a **Launch at login** checkbox. On macOS this writes a LaunchAgent
plist; on Windows it writes an `HKCU\...\Run` entry. Backed by Tauri's
`autostart` plugin — no installer step required.

## Default hotkey

- macOS: **⌘⌥M** (Cmd+Option+M)
- Windows / Linux: **Ctrl+Alt+M**

Chosen to avoid ⌘Space (Spotlight), ⌘⇧Space (Character Viewer), and the
usual launcher-app bindings (Alfred, Raycast).

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
