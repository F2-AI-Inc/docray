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

# Hosted-only docs rail: the static /try page gets a slim sidebar linking back
# into the documentation site, so the hosted demo feels like one experience.
# The server-embedded playground (docker / docray-server users) is untouched.
python3 - "$OUT/index.html" <<'PYEOF'
import sys
p = sys.argv[1]
s = open(p).read()

rail_css = """
<style>
  /* hosted docs rail (injected by build-try.sh; absent in docray-server) */
  body { margin-left: 196px; }
  #docs-rail {
    position: fixed; left: 0; top: 0; bottom: 0; width: 196px;
    background: var(--ink-2); border-right: 1px solid var(--hair);
    padding: 18px 16px; z-index: 40; overflow-y: auto;
    font-size: 12px;
  }
  #docs-rail .rail-wordmark {
    font-family: var(--serif); font-style: italic; font-size: 20px;
    color: var(--film); display: block; text-decoration: none; margin-bottom: 2px;
  }
  #docs-rail .rail-sub {
    color: var(--amber); font-size: 9.5px; letter-spacing: 0.2em;
    text-transform: uppercase; margin-bottom: 18px;
  }
  #docs-rail a.rail-link {
    display: block; color: var(--fg-dim); text-decoration: none;
    padding: 5px 0; border-bottom: 1px solid transparent;
  }
  #docs-rail a.rail-link:hover { color: var(--amber); }
  #docs-rail .rail-note {
    margin-top: 22px; padding-top: 14px; border-top: 1px solid var(--hair);
    color: var(--fg-dim); font-size: 10.5px; line-height: 1.5;
  }
  #docs-rail .rail-note b { color: var(--fg); font-weight: 500; }
  @media (max-width: 900px) { body { margin-left: 0; } #docs-rail { display: none; } }
</style>
"""

rail_html = """
<nav id="docs-rail">
  <a class="rail-wordmark" href="../index.html">docray</a>
  <div class="rail-sub">documentation</div>
  <a class="rail-link" href="../quickstart.html">Quickstart</a>
  <a class="rail-link" href="../granularity.html">Choosing a granularity</a>
  <a class="rail-link" href="../json-contract.html">The JSON contract</a>
  <a class="rail-link" href="../cli.html">CLI reference</a>
  <a class="rail-link" href="../http-api.html">HTTP API</a>
  <a class="rail-link" href="../playground.html">The playground</a>
  <a class="rail-link" href="https://github.com/F2-AI-Inc/docray">GitHub &nearr;</a>
  <div class="rail-note"><b>This page is the live demo.</b>
  Extraction runs as WebAssembly in this tab — your documents never
  leave your machine.</div>
</nav>
"""

assert "</head>" in s and "<header>" in s
s = s.replace("</head>", rail_css + "</head>", 1)
s = s.replace("<body>", "<body>" + rail_html, 1)
open(p, "w").write(s)
print("docs rail injected into", p)
PYEOF
cp "${ROOT}/web/worker.js" "$OUT/"
cp "$SRC"/{docray_wasm.js,docray_wasm_bg.wasm,pdfium.js,pdfium.wasm} "$OUT/wasm/"
echo "static playground assembled in $OUT"
