#!/usr/bin/env bash
# Fetches the pinned PDFium build from bblanchon/pdfium-binaries into .pdfium/,
# verifying the archive against the SHA-256 pinned below before extracting it.
#
# PDFium is the native code that parses untrusted documents and it ships in
# every release archive and container image, so the release tag alone is not a
# pin (release assets are mutable). The verified archive is kept under
# .pdfium/.archive/ so CI can cache it; every run re-verifies that archive and
# re-extracts lib/ and include/ from it, so a restored cache is never trusted
# as-is.
#
# Environment:
#   PDFIUM_VERSION  optional; if set it must equal VERSION below (CI sets it so
#                   the workflow pins and this script cannot drift apart)
#   PDFIUM_ASSET    optional; archive name to fetch instead of the host's
#                   (release cross-compiles, e.g. pdfium-mac-x64.tgz on arm64)
#   PDFIUM_DIR      optional; install directory (default: .pdfium)
set -euo pipefail

# Pinned to chromium/7934: pdfium-render 0.8.37 requires symbols (e.g.
# FPDFFormObj_RemoveObject) absent from older builds such as chromium/7047.
# Bumping VERSION means replacing every checksum below.
VERSION="chromium/7934"

if [[ -n "${PDFIUM_VERSION:-}" && "$PDFIUM_VERSION" != "$VERSION" ]]; then
  echo "PDFIUM_VERSION=${PDFIUM_VERSION} does not match the pin in scripts/fetch-pdfium.sh (${VERSION})" >&2
  exit 1
fi

if [[ -n "${PDFIUM_ASSET:-}" ]]; then
  ASSET="$PDFIUM_ASSET"
else
  case "$(uname -s)-$(uname -m)" in
    Darwin-arm64)  ASSET="pdfium-mac-arm64.tgz" ;;
    Darwin-x86_64) ASSET="pdfium-mac-x64.tgz" ;;
    Linux-x86_64)  ASSET="pdfium-linux-x64.tgz" ;;
    Linux-aarch64) ASSET="pdfium-linux-arm64.tgz" ;;
    *) echo "unsupported platform" >&2; exit 1 ;;
  esac
fi

case "$ASSET" in
  pdfium-mac-arm64.tgz)   SHA256="24815e8986a85e1ce5553027fc2e038c21d61310665ffdf849625994cc43819c" ;;
  pdfium-mac-x64.tgz)     SHA256="7aace004650a5933467c100956bd39feeb36c28d92660dbf0326ad10022f48e9" ;;
  pdfium-linux-x64.tgz)   SHA256="60975aea26c0c6e2f0e0ca3e3c55d184085362f78ac689e4cba2d7d7997920bb" ;;
  pdfium-linux-arm64.tgz) SHA256="a5ea0edd14b9e352a96f47c9484ebbd24af8dfcadc0f0cb9a199b832d815e75e" ;;
  *) echo "no pinned checksum for ${ASSET}" >&2; exit 1 ;;
esac

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

DEST="${PDFIUM_DIR:-.pdfium}"
ARCHIVE_DIR="${DEST}/.archive"
ARCHIVE="${ARCHIVE_DIR}/${VERSION//\//-}-${ASSET}"
mkdir -p "$ARCHIVE_DIR"

# Fail closed: drop any previously extracted (possibly cache-restored) code
# first, so nothing unverified survives an early exit below.
rm -rf "${DEST:?}/lib" "${DEST:?}/include"

if [[ ! -f "$ARCHIVE" ]]; then
  URL="https://github.com/bblanchon/pdfium-binaries/releases/download/${VERSION}/${ASSET}"
  temp_archive="$(mktemp "${ARCHIVE_DIR}/download.XXXXXX")"
  trap 'rm -f "$temp_archive"' EXIT
  echo "fetching ${URL}"
  curl -fL "$URL" -o "$temp_archive"
  actual="$(sha256 "$temp_archive")"
  if [[ "$actual" != "$SHA256" ]]; then
    echo "PDFium checksum mismatch for downloaded ${ASSET}" >&2
    echo "expected: ${SHA256}" >&2
    echo "actual:   ${actual}" >&2
    exit 1
  fi
  mv "$temp_archive" "$ARCHIVE"
  trap - EXIT
fi

# Re-verify on every run: the archive may have come from a cache restore.
actual="$(sha256 "$ARCHIVE")"
if [[ "$actual" != "$SHA256" ]]; then
  echo "PDFium checksum mismatch for cached ${ARCHIVE}" >&2
  echo "expected: ${SHA256}" >&2
  echo "actual:   ${actual}" >&2
  echo "refusing to use it; delete ${ARCHIVE} to re-download" >&2
  exit 1
fi

tar -xzf "$ARCHIVE" -C "$DEST"
echo "pdfium ${VERSION} (${ASSET}, sha256 ${SHA256}) installed under ${DEST}/lib"
