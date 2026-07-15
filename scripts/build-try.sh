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
python3 - "$OUT/index.html" "${ROOT}/book/src/SUMMARY.md" <<'PYEOF'
import re, sys
page, summary = sys.argv[1], sys.argv[2]
s = open(page).read()

# Build the chapter list from SUMMARY.md so the rail IS the docs sidebar:
# part headers (# Title) and continuously-numbered chapters, mdBook-style.
items, n = [], 0
for line in open(summary):
    line = line.strip()
    m = re.match(r"^# (.+)$", line)
    if m and m.group(1) != "Summary":
        items.append(("part", m.group(1)))
        continue
    m = re.match(r"^- \[(.+?)\]\((.+?)\)$", line)
    if m:
        n += 1
        items.append(("ch", f"{n}.", m.group(1), m.group(2).replace(".md", ".html")))
        continue
    m = re.match(r"^\[(.+?)\]\((.+?)\)$", line)
    if m:
        items.append(("ch", "", m.group(1), m.group(2).replace(".md", ".html")))

rows = []
for it in items:
    if it[0] == "part":
        rows.append(f'<div class="rail-part">{it[1]}</div>')
    else:
        _, num, title, href = it
        numhtml = f'<strong>{num}</strong> ' if num else ""
        rows.append(f'<a class="rail-link" href="../{href}">{numhtml}{title}</a>')
chapters = "\n  ".join(rows)

rail_css = """
<style>
  /* hosted docs rail (generated from SUMMARY.md by build-try.sh; absent in
     docray-server). Sized and styled to match the mdBook sidebar. */
  body { margin-left: 300px; }
  #docs-rail {
    position: fixed; left: 0; top: 0; bottom: 0; width: 300px;
    background: var(--ink-2); border-right: 1px solid var(--hair);
    padding: 0 0 24px; z-index: 40; overflow-y: auto;
    font-size: 13px; line-height: 2.2;
  }
  #docs-rail .rail-try {
    display: block; margin: 12px 12px 16px; padding: 10px 12px;
    background: var(--amber); color: #241a08; border-radius: 3px;
    font-weight: 600; text-decoration: none; line-height: 1.3;
  }
  #docs-rail .rail-try span { display: block; font-weight: 400; font-size: 0.78em; opacity: 0.85; }
  #docs-rail .rail-part {
    font-weight: 600; color: var(--fg); margin: 10px 0 0; padding: 0 20px;
  }
  #docs-rail a.rail-link {
    display: block; color: var(--fg-dim); text-decoration: none; padding: 0 20px 0 20px;
  }
  #docs-rail a.rail-link strong { color: var(--fg); font-weight: 600; }
  #docs-rail a.rail-link:hover { color: var(--amber); }
  #docs-rail .rail-note {
    margin: 22px 20px 0; padding-top: 14px; border-top: 1px solid var(--hair);
    color: var(--fg-dim); font-size: 11px; line-height: 1.5;
  }
  #docs-rail .rail-note b { color: var(--fg); font-weight: 500; }
  @media (max-width: 1100px) { body { margin-left: 0; } #docs-rail { display: none; } }
</style>
"""

rail_html = f"""
<nav id="docs-rail">
  <a class="rail-try" href="index.html">Try docray <span>in your browser — no install, documents never leave your machine</span></a>
  {chapters}
  <div class="rail-note"><b>This page is the live demo.</b>
  Extraction runs as WebAssembly in this tab — your documents never
  leave your machine.</div>
</nav>
"""

assert "</head>" in s and "<body>" in s
s = s.replace("</head>", rail_css + "</head>", 1)
s = s.replace("<body>", "<body>" + rail_html, 1)
open(page, "w").write(s)
print("SUMMARY-driven docs rail injected")
PYEOF
cp "${ROOT}/web/worker.js" "$OUT/"
cp "$SRC"/{docray_wasm.js,docray_wasm_bg.wasm,pdfium.js,pdfium.wasm} "$OUT/wasm/"
echo "static playground assembled in $OUT"
