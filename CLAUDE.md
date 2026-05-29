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
  compile via cargo-xwin), `sign-windows.sh` (Authenticode sign via Azure
  Trusted Signing).
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
- **CRT is statically linked** (`src-tauri/.cargo/config.toml` sets
  `target-feature=+crt-static` for both Windows targets). Without it the
  `.exe` would import `VCRUNTIME140.dll` / `VCRUNTIME140_1.dll` and fail
  silently on machines without the VC++ Redistributable — no Event Viewer
  entry, no stderr. Don't remove this without also bundling vc_redist in
  the installer.

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
- Binaries: GitHub Releases on `lgibelli/marie-lookapp` itself hold the built
  `.exe`s / `.dmg`s. (Releases previously lived in a separate
  `marie-lookapp-releases` repo — retired in favour of same-repo releases.)
- **Local builds are the default** (full control; the only path that signs and
  the only way to get the macOS `.dmg`). **Windows-only CI also exists**:
  `.github/workflows/release-windows.yml` builds the **unsigned** x64 + arm64
  `.exe` on a `v*` tag push and publishes them as a Release on this repo.
  - No secret needed: the built-in `GITHUB_TOKEN` (granted `contents: write`)
    creates the release in its own repo.
  - This repo is **private**, so Windows runners bill at 2x minutes (the
    publish job runs on Linux, 1x). Don't add macOS/Linux *build* runners
    (10x / cost) or expand CI without asking.
  - CI output is **unsigned** (SmartScreen warns). Sign locally for trusted
    releases; the installer is not built in CI.
- Release flow:
  ```bash
  scripts/build-windows.sh release all
  scripts/sign-windows.sh \
    src-tauri/target/x86_64-pc-windows-msvc/release/marie-lookup.exe \
    src-tauri/target/aarch64-pc-windows-msvc/release/marie-lookup.exe
  BUILD_INSTALLER=1 scripts/build-windows.sh release x64   # wraps signed x64
  scripts/sign-windows.sh src-tauri/target/marie-lookup-setup-X.Y.Z.exe
  gh release create vX.Y.Z --repo lgibelli/marie-lookapp \
    --title "..." --notes-file NOTES.md --prerelease \
    src-tauri/target/x86_64-pc-windows-msvc/release/marie-lookup.exe#marie-lookup-windows-x64.exe \
    src-tauri/target/aarch64-pc-windows-msvc/release/marie-lookup.exe#marie-lookup-windows-arm64.exe \
    src-tauri/target/marie-lookup-setup-X.Y.Z.exe#marie-lookup-setup-X.Y.Z.exe
  ```

## Code signing (Windows)

Windows binaries are Authenticode-signed with **Azure Trusted Signing**
(individual cert, subject "Luca Gibelli") via `jsign`. All signing happens
on this Mac — no hardware token, no Windows box required.

**One-time setup**:

1. Azure Trusted Signing account + Public Trust certificate profile,
   created in the Azure portal. Identity validation (passport + selfie)
   takes a few business days to a couple of weeks; Microsoft reviews
   manually.
2. Install tooling: `sudo port install jsign azure-cli osslsigncode`
   (or the Homebrew equivalents). `osslsigncode` is optional — only used
   for `verify`.
3. `az login` once interactively. Azure CLI caches the session in
   `~/.azure/`.
4. Stash the non-secret endpoint/account/profile config in the login
   keychain:
   ```bash
   security add-generic-password -U \
     -a "$USER" -s "marie-lookup-signing" \
     -w '{"endpoint":"https://<region>.codesigning.azure.net","account":"<account>","profile":"<profile>"}'
   ```

`scripts/sign-windows.sh` reads the keychain item and fetches a fresh
access token via `az account get-access-token` on each run (tokens expire
in ~1h, no caching). No secrets in the repo, no secrets in env.

**Order matters**: sign the inner `marie-lookup.exe` *before* re-running
`makensis`, so the installer embeds an already-signed binary. Then sign
the installer itself. See the release flow above.

The NSIS uninstaller (`uninstall.exe`) is generated at install time on
the user's machine and is intentionally left unsigned — Windows doesn't
SmartScreen-warn on uninstall flows, and the two-pass signing dance isn't
worth the complexity.

## Don't

- Add a JS bundler / framework — it's deliberately bundler-free.
- Mock the DB in any tests — use a temp SQLite file.
- Set `innerHTML` in `dist/*.js` — a project hook blocks it. Use
  `replaceChildren()` + `createElement` / `textContent`.
- Use blocking `std::thread::sleep` inside an async command — use
  `tokio::time::sleep` (the `time` feature is enabled).
