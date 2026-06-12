#!/usr/bin/env bash
# SIGNED release pipeline for Marie LookApp. Runs locally, or in CI via
# .github/workflows/release-signed.yml (which provisions the keys + toolchain
# and then calls this same script). The release/** CI only builds unsigned test
# artifacts — see .github/workflows/release.yml.
#
# Builds the Windows binaries (cargo-xwin) and the macOS bundle (tauri
# bundler: .dmg + .app.tar.gz updater artifact), signs them (Authenticode via
# sign-windows.sh + Tauri-updater minisign), generates the updater manifest
# (latest.json, windows-x86_64 + darwin-aarch64), then publishes the signed
# binaries AND latest.json to ONE public repo (BINARIES_REPO) as assets on a
# single release. Installed clients fetch latest.json anonymously from the
# endpoint baked into the app
# (https://github.com/<repo>/releases/latest/download/latest.json, see
# src-tauri/tauri.conf.json); its url fields point at the binaries on the same
# release.
#
# The macOS bundle is unsigned/un-notarized (no Apple Developer cert):
# first-time installs need right-click → Open; self-updates are fine.
#
# Usage:
#   scripts/release.sh ["release notes…"]
#
# Version is read from src-tauri/tauri.conf.json — bump it there (plus
# package.json + src-tauri/Cargo.toml) before releasing. Prereqs:
#   - gh logged in with push access to ${RELEASES_REPO}
#   - az login session for Authenticode (see sign-windows.sh), OR
#     SKIP_AUTHENTICODE=1 to release unsigned (SmartScreen warns; the
#     updater itself only verifies the minisign signature)
#   - makensis on PATH (NSIS installer)
#   - ~/.tauri/marie-lookup-updater.key (updater signing key; generated once
#     via `npx tauri signer generate`)
#
# NB: the release is deliberately NOT marked --prerelease even for -preN
# versions — GitHub's releases/latest endpoint (through which latest.json is
# fetched) ignores prereleases entirely, so a prerelease would be invisible
# to the updater.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# Org/signing identifiers are NEVER hardcoded in this (public) repo. Locally they
# come from a gitignored scripts/release.env; in CI from repo Variables/Secrets
# exported as env. See scripts/release.env.example.
# shellcheck source=/dev/null
[ -f "${ROOT}/scripts/release.env" ] && . "${ROOT}/scripts/release.env"
# One public repo now hosts BOTH the signed binaries and the updater manifest
# (latest.json) — the latter rides along as an asset on the same release, so
# /releases/latest/download/latest.json (the endpoint baked into installed
# clients, see src-tauri/tauri.conf.json) resolves to it. This lets CI publish
# with just the built-in GITHUB_TOKEN (no cross-repo PAT).
BINARIES_REPO="lgibelli/marie-lookapp"
UPDATER_KEY="${HOME}/.tauri/marie-lookup-updater.key"
NOTES="${1:-Marie LookApp release}"

VERSION="$(node -p "require('${ROOT}/src-tauri/tauri.conf.json').version")"
TAG="v${VERSION}"

X64_EXE="${ROOT}/src-tauri/target/x86_64-pc-windows-msvc/release/marie-lookup.exe"
ARM64_EXE="${ROOT}/src-tauri/target/aarch64-pc-windows-msvc/release/marie-lookup.exe"
SETUP_EXE="${ROOT}/src-tauri/target/marie-lookup-setup-${VERSION}.exe"

[ -f "${UPDATER_KEY}" ] || { echo "error: updater key missing: ${UPDATER_KEY}" >&2; exit 1; }
command -v makensis >/dev/null 2>&1 || { echo "error: makensis not on PATH (port/brew install nsis)" >&2; exit 1; }
# Abort if this version is already PUBLISHED (a non-prerelease release). A
# leftover unsigned prerelease from release/** CI is fine — the publish step
# below clobbers + promotes it.
if [ "$(gh release view "${TAG}" --repo "${BINARIES_REPO}" --json isPrerelease -q .isPrerelease 2>/dev/null)" = "false" ]; then
  echo "error: ${TAG} already published to ${BINARIES_REPO} — bump the version first" >&2
  exit 1
fi

echo ">> building Windows binaries (x64 + arm64)"
"${ROOT}/scripts/build-windows.sh" release all

if [ "${SKIP_AUTHENTICODE:-0}" != "1" ]; then
  echo ">> Authenticode-signing standalone exes"
  "${ROOT}/scripts/sign-windows.sh" "${X64_EXE}" "${ARM64_EXE}"
else
  echo ">> SKIP_AUTHENTICODE=1 — exes stay unsigned (SmartScreen will warn)"
fi

# Re-running the build with BUILD_INSTALLER=1 only re-runs makensis: cargo's
# freshness check is source-based, so it does NOT relink (and unsign) the
# exe we just signed — the installer embeds the signed binary.
echo ">> building NSIS installer (wraps the x64 exe)"
BUILD_INSTALLER=1 "${ROOT}/scripts/build-windows.sh" release x64

if [ "${SKIP_AUTHENTICODE:-0}" != "1" ]; then
  echo ">> Authenticode-signing installer"
  "${ROOT}/scripts/sign-windows.sh" "${SETUP_EXE}"
fi

# Updater (minisign) signature — what tauri-plugin-updater actually verifies.
# Must be generated AFTER any Authenticode signing (it hashes the final file).
echo ">> generating updater signature"
(cd "${ROOT}" && npx tauri signer sign -f "${UPDATER_KEY}" --password "" "${SETUP_EXE}")
SIG="$(cat "${SETUP_EXE}.sig")"

# macOS: the Tauri bundler builds the .app, signs it with the StarData
# Developer ID identity under the hardened runtime, notarizes + staples it (when
# notary creds are present), then writes the .app.tar.gz the updater consumes
# (minisigned via TAURI_SIGNING_PRIVATE_KEY). The signing identity must be in
# the keychain — CI imports it into a throwaway keychain; locally it's already
# there. Notarization uses an App Store Connect API key when
# APPLE_API_KEY_PATH/_ISSUER/_KEY_ID are exported (the CI default) or an
# app-specific password via APPLE_ID/APPLE_PASSWORD; with neither the app is
# signed but NOT notarized and a downloaded DMG still warns.
#
# This replaces the old self-signed "Marie Lookup Signing" cert. Developer ID is
# also a *stable* cert, so the Accessibility/TCC grant survives auto-updates
# exactly as before (TCC keys on the cert's designated requirement) — and now
# Gatekeeper passes with no right-click → Open. The DMG is rolled by hand
# (tauri's dmg bundler drives Finder via AppleScript and fails headless) and
# notarized + stapled on its own so the downloaded image itself is clean.
DEVID="${MACOS_SIGNING_IDENTITY:-}"
MAC_SIGN_ENV=()
NOTARIZE=0
if [ -n "${DEVID}" ] && security find-identity -v -p codesigning 2>/dev/null | grep -qF "${DEVID}"; then
  MAC_SIGN_ENV+=("APPLE_SIGNING_IDENTITY=${DEVID}")
  if [ -n "${APPLE_API_KEY_PATH:-}" ] && [ -n "${APPLE_API_ISSUER:-}" ] && [ -n "${APPLE_API_KEY_ID:-}" ]; then
    MAC_SIGN_ENV+=("APPLE_API_ISSUER=${APPLE_API_ISSUER}" "APPLE_API_KEY=${APPLE_API_KEY_ID}" "APPLE_API_KEY_PATH=${APPLE_API_KEY_PATH}")
    NOTARIZE=1
    echo ">> macOS: Developer ID sign + notarize (App Store Connect API key)"
  elif [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_PASSWORD:-}" ]; then
    MAC_SIGN_ENV+=("APPLE_ID=${APPLE_ID}" "APPLE_PASSWORD=${APPLE_PASSWORD}" "APPLE_TEAM_ID=${APPLE_TEAM_ID}")
    NOTARIZE=1
    echo ">> macOS: Developer ID sign + notarize (app-specific password)"
  else
    echo ">> macOS: Developer ID sign only (no notary creds — a downloaded DMG will warn)"
  fi
else
  echo ">> WARNING: Developer ID cert not in keychain — building AD-HOC (Gatekeeper will warn)" >&2
fi

echo ">> building macOS bundle (app + updater artifact)"
# `env` is required: the MAC_SIGN_ENV array holds VAR=value strings, and bash
# only treats VAR=value as an assignment when it's a literal at parse time — an
# array element would instead be run as a command ("…: command not found").
# ${arr[@]+"${arr[@]}"} expands safely when the array is EMPTY under `set -u`
# on the runner's bash 3.2 (a bare "${arr[@]}" there is an "unbound variable").
(cd "${ROOT}" && env TAURI_SIGNING_PRIVATE_KEY="${UPDATER_KEY}" \
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" \
  ${MAC_SIGN_ENV[@]+"${MAC_SIGN_ENV[@]}"} npx tauri build --bundles app)
BUNDLE_DIR="${ROOT}/src-tauri/target/release/bundle"
MAC_APP="${BUNDLE_DIR}/macos/Marie Lookup.app"

# The bundler tars + minisigns the already-signed/notarized/stapled app, so use
# its artifact directly (no manual re-tar now that signing happens in-build).
MAC_TARGZ="${BUNDLE_DIR}/macos/Marie Lookup.app.tar.gz"
[ -f "${MAC_TARGZ}.sig" ] || { echo "error: updater artifact sig missing — is createUpdaterArtifacts on?" >&2; exit 1; }
MAC_SIG="$(cat "${MAC_TARGZ}.sig")"

MAC_DMG="${BUNDLE_DIR}/macos/marie-lookup-${VERSION}-macos-arm64.dmg"
hdiutil create -quiet -volname "Marie Lookup" \
  -srcfolder "${MAC_APP}" \
  -ov -format UDZO "${MAC_DMG}"

# Notarize + staple the DMG itself. The .app inside is already notarized, but a
# downloaded .dmg is Gatekeeper-checked too; stapling lets the image pass offline.
if [ "${NOTARIZE}" = "1" ]; then
  echo ">> notarizing + stapling the DMG"
  if [ -n "${APPLE_API_KEY_PATH:-}" ]; then
    xcrun notarytool submit "${MAC_DMG}" \
      --key "${APPLE_API_KEY_PATH}" --key-id "${APPLE_API_KEY_ID}" --issuer "${APPLE_API_ISSUER}" --wait
  else
    xcrun notarytool submit "${MAC_DMG}" \
      --apple-id "${APPLE_ID}" --password "${APPLE_PASSWORD}" --team-id "${APPLE_TEAM_ID}" --wait
  fi
  xcrun stapler staple "${MAC_DMG}"
fi

echo ">> generating latest.json"
LATEST="${ROOT}/src-tauri/target/latest.json"
NOTES_JSON="$(node -p 'JSON.stringify(process.argv[1])' "${NOTES}")"
cat > "${LATEST}" <<EOF
{
  "version": "${VERSION}",
  "notes": ${NOTES_JSON},
  "pub_date": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "platforms": {
    "windows-x86_64": {
      "signature": "${SIG}",
      "url": "https://github.com/${BINARIES_REPO}/releases/download/${TAG}/marie-lookup-setup-${VERSION}.exe"
    },
    "darwin-aarch64": {
      "signature": "${MAC_SIG}",
      "url": "https://github.com/${BINARIES_REPO}/releases/download/${TAG}/marie-lookup-macos-arm64.app.tar.gz"
    }
  }
}
EOF

# Asset NAMES (the download URLs) come from the file basename — gh's
# `file#label` syntax only sets a display label. Both arch builds are named
# marie-lookup.exe, so stage renamed copies before uploading.
STAGE="$(mktemp -d)"
trap 'rm -rf "${STAGE}"' EXIT
cp "${SETUP_EXE}" "${STAGE}/marie-lookup-setup-${VERSION}.exe"
cp "${X64_EXE}" "${STAGE}/marie-lookup-windows-x64.exe"
cp "${ARM64_EXE}" "${STAGE}/marie-lookup-windows-arm64.exe"
cp "${MAC_DMG}" "${STAGE}/marie-lookup-${VERSION}-macos-arm64.dmg"
cp "${MAC_TARGZ}" "${STAGE}/marie-lookup-macos-arm64.app.tar.gz"
cp "${LATEST}" "${STAGE}/latest.json"

# Signed binaries -> source repo. The release/** CI may have already created
# this tag as an unsigned prerelease; if so, clobber those assets with the
# signed ones and promote it to a full release, otherwise create it.
echo ">> publishing signed binaries + latest.json to ${BINARIES_REPO} (${TAG})"
# latest.json rides on the SAME release so /releases/latest/download/latest.json
# resolves to it. The release must NOT be a prerelease — the updater's
# /releases/latest endpoint skips prereleases (the clobber path forces it off).
BIN_ASSETS=(
  "${STAGE}/marie-lookup-setup-${VERSION}.exe"
  "${STAGE}/marie-lookup-windows-x64.exe"
  "${STAGE}/marie-lookup-windows-arm64.exe"
  "${STAGE}/marie-lookup-${VERSION}-macos-arm64.dmg"
  "${STAGE}/marie-lookup-macos-arm64.app.tar.gz"
  "${STAGE}/latest.json"
)
if gh release view "${TAG}" --repo "${BINARIES_REPO}" >/dev/null 2>&1; then
  gh release upload "${TAG}" --repo "${BINARIES_REPO}" --clobber "${BIN_ASSETS[@]}"
  gh release edit "${TAG}" --repo "${BINARIES_REPO}" --prerelease=false
else
  gh release create "${TAG}" --repo "${BINARIES_REPO}" \
    --title "Marie LookApp ${VERSION}" --notes "${NOTES}" \
    --target "$(git -C "${ROOT}" rev-parse HEAD)" \
    "${BIN_ASSETS[@]}"
fi

echo ">> done: https://github.com/${BINARIES_REPO}/releases/tag/${TAG}"
echo "   (signed binaries + latest.json on the one release)"
