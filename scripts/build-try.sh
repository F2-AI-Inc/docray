#!/usr/bin/env bash
# Assembles the static, fully client-side playground ("try it" page):
#   web/try/ = playground.html + extraction worker + web-target wasm artifacts.
# Prerequisite: scripts/build-wasm.sh (web target) has produced web/wasm-web/.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="${ROOT}/web/wasm-web"
OUT="${ROOT}/web/try"

for f in docray_wasm.js docray_wasm_bg.wasm pdfium.js pdfium.wasm; do
  [[ -f "$SRC/$f" ]] || { echo "missing $SRC/$f — run scripts/build-wasm.sh first" >&2; exit 1; }
done

rm -rf "$OUT"
mkdir -p "$OUT/wasm"
cp "${ROOT}/crates/docray-server/assets/playground.html" "$OUT/index.html"
cp "${ROOT}/web/worker.js" "$OUT/"
cp "$SRC"/{docray_wasm.js,docray_wasm_bg.wasm,pdfium.js,pdfium.wasm} "$OUT/wasm/"
echo "static playground assembled in $OUT"
