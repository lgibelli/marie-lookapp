# Marie LookApp

Tray-resident text snippet lookup with paste-at-cursor. Tauri 2 (Rust +
native webview, no JS bundler). Replaces the legacy .NET / WPF v1.x
(branch `legacy-abandoned-20260522`).

## Commands

```bash
npm run dev               # tauri dev (cargo tauri dev also fine)
npm run build             # local platform release bundle
scripts/build-windows.sh release all   # cross-compile x64 + arm64 from macOS
```

No test suite yet.

## Architecture

- `src-tauri/` — Rust backend. `lib.rs` has all the commands, tray, hotkey,
  DB. `main.rs` is a one-liner.
- `dist/` — static HTML/CSS/JS frontend, served directly to WKWebView /
  WebView2. No Vite, no React, no bundler.
- `scripts/` — `make_icons.py` (PIL+iconutil), `build-windows.sh` (cross
  compile via cargo-xwin).
- `package.json` exists **only** to host `@tauri-apps/cli` for the `tauri`
  command. Don't add runtime JS deps.

Two windows, both declared in `tauri.conf.json`:
- `lookup` — frameless, always-on-top, hidden by default, summoned by
  global hotkey, auto-hides on focus loss.
- `manage` — regular window for CRUD on entries, opened via tray menu.

SQLite DB at `app_data_dir()/marie-lookup.db`. Schema: single `entries`
table (id, title, body, created_at, updated_at).

## Non-obvious patterns (the dragons)

- **`enigo` MUST be called on the main thread on macOS.** It calls
  `TSMGetInputSourceProperty`, which `dispatch_assert_queue`s and traps
  if you're on a tokio worker. `paste_text` dispatches via
  `app.run_on_main_thread`. **Do not** move that call back inline.
- **macOS Accessibility permission gates `enigo`.** Without it, paste
  silently fails (no crash now that #1 is fixed). `paste_text` checks via
  `macos-accessibility-client` and triggers the system prompt + opens
  Settings if missing. Frontend handles the `accessibility_required` error.
- **`withGlobalTauri: true`** in `tauri.conf.json` — frontend uses
  `window.__TAURI__.core.invoke` directly. Keep it. Adding a bundler is
  out of scope.
- **macOS-only deps go in `[target.'cfg(target_os = "macos")']`** in
  `Cargo.toml` (currently `macos-accessibility-client`). Don't accidentally
  pull them in for Windows.
- **App is a tray-only background app on macOS** —
  `ActivationPolicy::Accessory` set in `setup`. No Dock icon. If you can't
  find the running app, look at the menu bar.
- **Hotkey is Cmd+Alt+M / Ctrl+Alt+M** (deliberately not Cmd+Space —
  Spotlight, not Cmd+Shift+Space — Character Viewer). `Code::KeyM`,
  `Modifiers::META|ALT` on mac, `CONTROL|ALT` elsewhere.

## Cross-compile from macOS to Windows

`scripts/build-windows.sh` handles a lot of toolchain quirks specific to
this Mac. **Read it before tweaking.** Key things:
- This box has both MacPorts Rust (`/opt/local/bin`) and rustup-managed
  Rust (`~/.cargo/bin`). Only rustup has cross-targets. The script forces
  rustup's bins ahead of MacPorts in PATH.
- `tauri-winres` needs `llvm-rc` to embed the icon. `llvm-tools-preview`
  rustup component does NOT ship it. The script discovers it from Android
  NDK / MacPorts / Homebrew. If none of those are present, the build will
  print a clear install hint.
- Bare `cargo xwin build` produces standalone `.exe`. Add
  `BUILD_INSTALLER=1` (with `makensis` on PATH) to also produce an NSIS
  installer that bundles the WebView2 bootstrapper — see
  `scripts/installer.nsi`. We deliberately did NOT take the Tauri-CLI
  bundle path because it can't be driven through `cargo-xwin`.
- `.msi` bundling needs WiX, which only runs on Windows. We don't ship MSI
  from Mac.

## Autostart

Implemented via `tauri-plugin-autostart` (LaunchAgent on macOS,
`HKCU\...\Run` on Windows). UI: checkbox in the Manage window's sidebar
footer. The frontend calls the plugin commands directly
(`window.__TAURI__.core.invoke('plugin:autostart|enable'` etc.); the
capability `autostart:default` is granted in `capabilities/default.json`.
No wrapper Rust commands needed.

## Release & repo layout

- Source: `lgibelli/marie-lookapp` — `main` is the Tauri rewrite,
  `legacy-abandoned-20260522` keeps the original .NET code.
- Binaries: `lgibelli/marie-lookapp-releases` — GitHub Releases hold the
  built `.exe`s / `.dmg`s. The repo's own tree is just a README.
- **Builds are local. No GitHub Actions** (cost reason). Don't add
  `.github/workflows/*` for build/release pipelines without asking.
- Release flow:
  ```bash
  scripts/build-windows.sh release all
  gh release create vX.Y.Z --repo lgibelli/marie-lookapp-releases \
    --title "..." --notes-file NOTES.md --prerelease \
    src-tauri/target/x86_64-pc-windows-msvc/release/marie-lookup.exe#marie-lookup-windows-x64.exe \
    src-tauri/target/aarch64-pc-windows-msvc/release/marie-lookup.exe#marie-lookup-windows-arm64.exe
  ```

## Don't

- Add a JS bundler / framework — it's deliberately bundler-free.
- Mock the DB in any tests — use a temp SQLite file.
- Set `innerHTML` in `dist/*.js` — a project hook blocks it. Use
  `replaceChildren()` + `createElement` / `textContent`.
- Use blocking `std::thread::sleep` inside an async command — use
  `tokio::time::sleep` (the `time` feature is enabled).
