#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="web"
PDFIUM_RELEASE="7902"
PDFIUM_SHA256="153871da7e958a9440c84648eb45ddd9ad603efda9fcd8f021766dba5a9157a2"
PDFIUM_URL="https://github.com/paulocoutinhox/pdfium-lib/releases/download/${PDFIUM_RELEASE}/wasm.tgz"
CACHE_DIR="${ROOT}/.pdfium-wasm"
ARCHIVE="${CACHE_DIR}/wasm-${PDFIUM_RELEASE}.tgz"
EXTRACT_DIR="" # per-target, computed after arg parsing (parallel-build safe)
OUT_DIR="" # computed after arg parsing: web and nodejs artifacts are
           # different module formats and must never overwrite each other

usage() {
  echo "usage: scripts/build-wasm.sh [--target web|nodejs]" >&2
}

while (($#)); do
  case "$1" in
    --target)
      [[ $# -ge 2 ]] || { usage; exit 2; }
      TARGET="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage
      exit 2
      ;;
  esac
done

if [[ "$TARGET" != "web" && "$TARGET" != "nodejs" ]]; then
  usage
  exit 2
fi

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

mkdir -p "$CACHE_DIR"
if [[ ! -f "$ARCHIVE" ]]; then
  temp_archive="${ARCHIVE}.download"
  trap 'rm -f "$temp_archive"' EXIT
  echo "downloading Pdfium WASM release ${PDFIUM_RELEASE}" >&2
  curl -fL "$PDFIUM_URL" -o "$temp_archive"
  mv "$temp_archive" "$ARCHIVE"
  trap - EXIT
fi

actual_sha256="$(sha256 "$ARCHIVE")"
if [[ "$actual_sha256" != "$PDFIUM_SHA256" ]]; then
  echo "Pdfium WASM checksum mismatch" >&2
  echo "expected: ${PDFIUM_SHA256}" >&2
  echo "actual:   ${actual_sha256}" >&2
  exit 1
fi

EXTRACT_DIR="${CACHE_DIR}/${PDFIUM_RELEASE}-${TARGET}"
rm -rf "$EXTRACT_DIR"
mkdir -p "$EXTRACT_DIR"
tar -xzf "$ARCHIVE" -C "$EXTRACT_DIR"

OUT_DIR="${ROOT}/web/wasm-${TARGET}"
rm -rf "$OUT_DIR"
WASM_PACK_CACHE="${WASM_PACK_CACHE:-${CACHE_DIR}/wasm-pack-cache}" \
  wasm-pack build "${ROOT}/crates/docray-wasm" \
  --release \
  --target "$TARGET" \
  --out-dir "$OUT_DIR" \
  --no-opt

wasm_file="${OUT_DIR}/docray_wasm_bg.wasm"
if command -v wasm-opt >/dev/null 2>&1; then
  echo "optimizing $(basename "$wasm_file") with wasm-opt -Oz" >&2
  wasm-opt -Oz "$wasm_file" -o "${wasm_file}.optimized"
  mv "${wasm_file}.optimized" "$wasm_file"
else
  echo "warning: wasm-opt not found; install binaryen for the production size optimization" >&2
fi

# Pdfium's JS and WASM files are one pinned runtime artifact. Keep them beside
# the generated package so both browser workers and the Node parity gate load
# the exact pair verified above.
cp "${EXTRACT_DIR}/release/node/pdfium.js" "${OUT_DIR}/pdfium.js"
cp "${EXTRACT_DIR}/release/node/pdfium.wasm" "${OUT_DIR}/pdfium.wasm"

echo "built docray-wasm target=${TARGET} in ${OUT_DIR}" >&2
