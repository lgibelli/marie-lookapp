#!/usr/bin/env bash
# Cross-compile marie-lookup from macOS to Windows (x86_64-pc-windows-msvc)
# using cargo-xwin. Produces a single self-contained .exe; the frontend
# assets in dist/ and the icon are embedded by tauri-build at compile time.
#
# Prereqs (one-time):
#   ~/.cargo/bin/rustup target add x86_64-pc-windows-msvc
#   ~/.cargo/bin/rustup component add llvm-tools-preview
#   ~/.cargo/bin/cargo install --locked cargo-xwin
#
# On first run, cargo-xwin will download ~700 MB of MSVC headers/CRT into
# ~/.cache/cargo-xwin/ — subsequent runs reuse the cache. Initial Windows
# build then takes 5–15 minutes; incremental builds are much faster.

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Usage: build-windows.sh [release|debug] [x64|arm64|all]
#   default profile: release
#   default arch:    x64
PROFILE="${1:-release}"
ARCH="${2:-x64}"

case "${ARCH}" in
  x64)   TARGETS=("x86_64-pc-windows-msvc") ;;
  arm64) TARGETS=("aarch64-pc-windows-msvc") ;;
  all)   TARGETS=("x86_64-pc-windows-msvc" "aarch64-pc-windows-msvc") ;;
  *) echo "error: unknown arch '${ARCH}' (expected x64, arm64, or all)" >&2; exit 1 ;;
esac

# This box has both MacPorts Rust (in /opt/local/bin) and rustup-managed Rust
# (in ~/.cargo/bin + ~/.rustup/toolchains/...). Only rustup knows about
# cross-targets, so force its toolchain ahead of MacPorts in PATH for every
# subprocess that cargo-xwin spawns.
RUSTUP_TOOLCHAIN_BIN="${HOME}/.rustup/toolchains/stable-aarch64-apple-darwin/bin"
export PATH="${HOME}/.cargo/bin:${RUSTUP_TOOLCHAIN_BIN}:${PATH}"
CARGO="${HOME}/.cargo/bin/cargo"

# tauri-winres needs llvm-rc to embed the icon + version info into the .exe.
# rustup's llvm-tools-preview component doesn't ship llvm-rc, so probe other
# common locations and append the first match to PATH.
find_llvm_rc() {
  if command -v llvm-rc >/dev/null 2>&1; then
    command -v llvm-rc
    return
  fi
  local c
  for c in "${HOME}"/Library/Android/sdk/ndk/*/toolchains/llvm/prebuilt/*/bin/llvm-rc \
           /opt/local/libexec/llvm-*/bin/llvm-rc \
           /opt/homebrew/opt/llvm/bin/llvm-rc \
           /usr/local/opt/llvm/bin/llvm-rc; do
    if [ -x "${c}" ]; then echo "${c}"; return; fi
  done
}
LLVM_RC="$(find_llvm_rc || true)"
if [ -z "${LLVM_RC}" ]; then
  echo "error: llvm-rc not found. install one of:" >&2
  echo "       brew install llvm" >&2
  echo "       sudo port install llvm-22" >&2
  echo "       (or any Android NDK install includes it)" >&2
  exit 1
fi
# Append to PATH (after rustup tools so we don't accidentally pick up the
# NDK's clang for normal cross-compile — we just need its llvm-rc).
export PATH="${PATH}:$(dirname "${LLVM_RC}")"
echo "using llvm-rc: ${LLVM_RC}"

if [ ! -x "${CARGO}" ]; then
  echo "error: rustup-managed cargo not found at ${CARGO}" >&2
  echo "       install rustup from https://rustup.rs/" >&2
  exit 1
fi

if ! "${CARGO}" xwin --help >/dev/null 2>&1; then
  echo "error: cargo-xwin not installed. run:" >&2
  echo "       ${CARGO} install --locked cargo-xwin" >&2
  exit 1
fi

INSTALLED_TARGETS="$("${HOME}/.cargo/bin/rustup" target list --installed 2>/dev/null || true)"
for t in "${TARGETS[@]}"; do
  if ! echo "${INSTALLED_TARGETS}" | grep -q "^${t}$"; then
    echo "error: ${t} not installed. run:" >&2
    echo "       ${HOME}/.cargo/bin/rustup target add ${t}" >&2
    exit 1
  fi
done

cd "${PROJECT_ROOT}/src-tauri"

if [ "${PROFILE}" = "debug" ]; then
  BUILD_FLAGS=""
  PROFILE_DIR="debug"
elif [ "${PROFILE}" = "release" ]; then
  BUILD_FLAGS="--release"
  PROFILE_DIR="release"
else
  echo "error: unknown profile '${PROFILE}' (expected release or debug)" >&2; exit 1
fi

echo ">> cross-compiling marie-lookup (profile: ${PROFILE}, targets: ${TARGETS[*]})"
echo ">> first run pulls MSVC headers via cargo-xwin (~700 MB) — be patient"
echo

declare -a BUILT_PATHS=()
for t in "${TARGETS[@]}"; do
  echo "---- ${t} ----"
  "${CARGO}" xwin build ${BUILD_FLAGS} --target "${t}"
  OUT="${PROJECT_ROOT}/src-tauri/target/${t}/${PROFILE_DIR}/marie-lookup.exe"
  if [ ! -f "${OUT}" ]; then
    echo "error: expected output not found at ${OUT}" >&2
    exit 1
  fi
  BUILT_PATHS+=("${OUT}")
done

echo
echo "================================================================"
for p in "${BUILT_PATHS[@]}"; do
  echo " built: ${p}  ($(du -h "${p}" | cut -f1))"
done
echo "================================================================"
echo
echo "Each .exe is self-contained (frontend + icon embedded)."
echo "Note: this produces just .exe(s), not .msi installer(s)."
echo "MSI bundling needs WiX, which only runs on Windows."
