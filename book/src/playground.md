# The playground

There are two ways to use the playground:

- **Hosted, fully in-browser** at [`/try`](try/index.html) on this site — extraction
  runs as WebAssembly inside a Web Worker; documents never leave your
  machine. Inputs cap at 100 MB.
- **Embedded in `docray-server`** at `/playground` — extraction uses the
  server's native engine (both engines appear in a selector when available).

`docray-server` embeds a browser workbench at **`/playground`** — the fastest
way to understand what docray extracts and to debug a specific document.

Drop a PDF, PPTX, DOCX, or DOCM on it. PDF pages and PPTX slides use paged
navigation. Word documents use their authored sections; a one-section document
hides the rail rather than inventing pages.

The workbench provides:

- **A thumbnail rail** for navigation (arrow keys work; scanned pages carry a
  badge). PPTX slides start with clean extraction schematics and the current
  slide upgrades to an isolated visual render. Word sections use clean
  reading-order schematic thumbnails.
- **Two independent panels**, each switchable between six lenses:
  - **source** — the rendered PDF page, or an offline visual PPTX/Word render
    inside a locked-down, null-origin browser sandbox. Word source is one
    scrollable flow; page breaks appear only where the renderer can honor them.
  - **boxes** — the page with filled, color-coded bounding boxes
    (<span style="color:#5cc8ff">text</span>,
    <span style="color:#ff7ac2">image</span>,
    <span style="color:#9dff70">path</span>,
    <span style="color:#ffd166">annotation</span>,
    <span style="color:#c79cff">table</span>)
  - **x-ray** — the page dimmed with wireframe boxes over it
  - **text** — the extracted content, element by element
  - **json** — the page's JSON, syntax-highlighted, with a copy button
  - **lean** — the whole document in the token-lean format (canonical bytes
    from the same renderer as `--format lean`), with a copy button
- **Cross-panel sync** — click any box and the opposite panel jumps to and
  flashes that exact element's JSON; click a JSON element and its box flashes
  on the page.
- **Filter chips** to isolate element types, zoom controls, a granularity
  selector that re-extracts live, and per-page/document JSON views.

For Word flow output, **boxes** shows the authored page frame and margins plus
only containers with numeric page/margin-relative placement and extents. It
states that ordinary flow content has no resolved boxes. **x-ray** becomes a
prominently labeled reading-order schematic whose vertical stack and lanes are
synthetic, not positions. It cross-links blocks to text and JSON by stable ID.
Extraction geometry is never overlaid on the separate docx-preview render.

Hover a PDF/PPTX box for its content, font, and coordinates. Hover a Word block
for its content and authored placement constraint, when one exists.

## Notes

- The page loads pdf.js and fonts from CDNs, so the **browser** needs
  internet access for PDF rendering and web fonts — the extraction API itself
  does not. The PPTX and Word renderers are vendored and make no network
  requests.
- Hostile PPTX and Word visual rendering runs in an iframe with only
  `sandbox="allow-scripts"` and a `default-src 'none'` Content Security Policy.
  The parent transfers document bytes in, never reads the iframe DOM, and
  accepts only exact, bounded status messages. Errors and timeouts fall back to
  extraction-derived structure or reading-order schematics.
- Uploads go to the server's own `/v1/extract`; nothing leaves your
  deployment.
- The UI is a single self-contained HTML file compiled into the server
  binary — there is no build step and no separate deployment.
