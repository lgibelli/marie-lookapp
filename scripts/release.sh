#!/usr/bin/env bash
# Local release pipeline for Marie LookApp — deliberately no CI.
#
# Builds the Windows binaries (cargo-xwin), signs them (Authenticode via
# sign-windows.sh + Tauri-updater minisign), generates the updater manifest
# (latest.json) and publishes everything as a GitHub Release on the PUBLIC
# releases repo — the app's auto-updater fetches
#   https://github.com/<releases-repo>/releases/latest/download/latest.json
# anonymously, so this must NOT be the (private) source repo.
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
RELEASES_REPO="lgibelli/marie-lookapp-releases"
UPDATER_KEY="${HOME}/.tauri/marie-lookup-updater.key"
NOTES="${1:-Marie LookApp release}"

VERSION="$(node -p "require('${ROOT}/src-tauri/tauri.conf.json').version")"
TAG="v${VERSION}"

X64_EXE="${ROOT}/src-tauri/target/x86_64-pc-windows-msvc/release/marie-lookup.exe"
ARM64_EXE="${ROOT}/src-tauri/target/aarch64-pc-windows-msvc/release/marie-lookup.exe"
SETUP_EXE="${ROOT}/src-tauri/target/marie-lookup-setup-${VERSION}.exe"

[ -f "${UPDATER_KEY}" ] || { echo "error: updater key missing: ${UPDATER_KEY}" >&2; exit 1; }
command -v makensis >/dev/null 2>&1 || { echo "error: makensis not on PATH (port/brew install nsis)" >&2; exit 1; }
gh release view "${TAG}" --repo "${RELEASES_REPO}" >/dev/null 2>&1 \
  && { echo "error: ${TAG} already exists on ${RELEASES_REPO} — bump the version first" >&2; exit 1; }

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
      "url": "https://github.com/${RELEASES_REPO}/releases/download/${TAG}/marie-lookup-setup-${VERSION}.exe"
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
cp "${LATEST}" "${STAGE}/latest.json"

echo ">> publishing ${TAG} to ${RELEASES_REPO}"
gh release create "${TAG}" --repo "${RELEASES_REPO}" \
  --title "Marie LookApp ${VERSION}" --notes "${NOTES}" \
  "${STAGE}/marie-lookup-setup-${VERSION}.exe" \
  "${STAGE}/marie-lookup-windows-x64.exe" \
  "${STAGE}/marie-lookup-windows-arm64.exe" \
  "${STAGE}/latest.json"

echo ">> done: https://github.com/${RELEASES_REPO}/releases/tag/${TAG}"
