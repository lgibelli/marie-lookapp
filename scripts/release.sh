#!/usr/bin/env bash
# Local SIGNED release pipeline for Marie LookApp (the release/** CI only builds
# unsigned test artifacts — see .github/workflows/release.yml).
#
# Builds the Windows binaries (cargo-xwin) and the macOS bundle (tauri
# bundler: .dmg + .app.tar.gz updater artifact), signs them (Authenticode via
# sign-windows.sh + Tauri-updater minisign), generates the updater manifest
# (latest.json, windows-x86_64 + darwin-aarch64), then publishes:
#   - the signed binaries -> the source repo (BINARIES_REPO, now public)
#   - latest.json         -> the releases repo (MANIFEST_REPO)
# Installed clients fetch latest.json anonymously from the endpoint baked into
# the app (https://github.com/<manifest-repo>/releases/latest/download/latest.json,
# see src-tauri/tauri.conf.json); its url fields point at the signed binaries on
# the source repo.
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
# Signed binaries are hosted on the (now public) source repo; the updater
# manifest stays on the releases repo, which is the endpoint baked into
# installed clients (see src-tauri/tauri.conf.json).
BINARIES_REPO="lgibelli/marie-lookapp"
MANIFEST_REPO="lgibelli/marie-lookapp-releases"
UPDATER_KEY="${HOME}/.tauri/marie-lookup-updater.key"
NOTES="${1:-Marie LookApp release}"

VERSION="$(node -p "require('${ROOT}/src-tauri/tauri.conf.json').version")"
TAG="v${VERSION}"

X64_EXE="${ROOT}/src-tauri/target/x86_64-pc-windows-msvc/release/marie-lookup.exe"
ARM64_EXE="${ROOT}/src-tauri/target/aarch64-pc-windows-msvc/release/marie-lookup.exe"
SETUP_EXE="${ROOT}/src-tauri/target/marie-lookup-setup-${VERSION}.exe"

[ -f "${UPDATER_KEY}" ] || { echo "error: updater key missing: ${UPDATER_KEY}" >&2; exit 1; }
command -v makensis >/dev/null 2>&1 || { echo "error: makensis not on PATH (port/brew install nsis)" >&2; exit 1; }
gh release view "${TAG}" --repo "${MANIFEST_REPO}" >/dev/null 2>&1 \
  && { echo "error: ${TAG} already published to ${MANIFEST_REPO} — bump the version first" >&2; exit 1; }

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

# macOS: bundle the .app plus — thanks to createUpdaterArtifacts — the
# .app.tar.gz the updater consumes, minisigned during the build (the bundler
# reads TAURI_SIGNING_PRIVATE_KEY, which takes a path or the key itself; the
# _PATH variant is only understood by `tauri signer sign`). The .dmg is then
# rolled by hand with hdiutil: tauri's dmg bundler drives Finder via
# AppleScript and fails in non-interactive shells.
echo ">> building macOS bundle (app + updater artifact + dmg)"
(cd "${ROOT}" && TAURI_SIGNING_PRIVATE_KEY="${UPDATER_KEY}" \
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" npx tauri build --bundles app)
BUNDLE_DIR="${ROOT}/src-tauri/target/release/bundle"
MAC_APP="${BUNDLE_DIR}/macos/Marie Lookup.app"

# Sign with the stable self-signed identity ("Marie Lookup Signing" in the
# login keychain — one-time setup, see CLAUDE.md). Without this the app is
# ad-hoc signed and its signature changes EVERY build, which silently
# invalidates the macOS Accessibility (TCC) grant on each auto-update: the
# permission is keyed on the certificate, not the path.
echo ">> codesigning macOS app (stable identity)"
codesign --force --deep --sign "Marie Lookup Signing" "${MAC_APP}"

# Re-create the updater artifact from the SIGNED app — the bundler tars it
# before we sign — and minisign the new archive.
MAC_TARGZ="${BUNDLE_DIR}/macos/Marie Lookup.app.tar.gz"
tar -czf "${MAC_TARGZ}" -C "${BUNDLE_DIR}/macos" "Marie Lookup.app"
(cd "${ROOT}" && npx tauri signer sign -f "${UPDATER_KEY}" --password "" "${MAC_TARGZ}")
MAC_SIG="$(cat "${MAC_TARGZ}.sig")"
MAC_DMG="${BUNDLE_DIR}/macos/marie-lookup-${VERSION}-macos-arm64.dmg"
hdiutil create -quiet -volname "Marie Lookup" \
  -srcfolder "${MAC_APP}" \
  -ov -format UDZO "${MAC_DMG}"

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
echo ">> publishing signed binaries to ${BINARIES_REPO} (${TAG})"
BIN_ASSETS=(
  "${STAGE}/marie-lookup-setup-${VERSION}.exe"
  "${STAGE}/marie-lookup-windows-x64.exe"
  "${STAGE}/marie-lookup-windows-arm64.exe"
  "${STAGE}/marie-lookup-${VERSION}-macos-arm64.dmg"
  "${STAGE}/marie-lookup-macos-arm64.app.tar.gz"
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

# latest.json -> manifest repo. Deliberately NOT a prerelease: the updater reads
# it through /releases/latest/download/latest.json, which skips prereleases.
echo ">> publishing latest.json to ${MANIFEST_REPO} (${TAG})"
gh release create "${TAG}" --repo "${MANIFEST_REPO}" \
  --title "Marie LookApp ${VERSION}" --notes "${NOTES}" \
  "${STAGE}/latest.json"

echo ">> done:"
echo "   binaries: https://github.com/${BINARIES_REPO}/releases/tag/${TAG}"
echo "   manifest: https://github.com/${MANIFEST_REPO}/releases/tag/${TAG}"
