# The playground

There are two ways to use the playground:

- **Hosted, fully in-browser** at [`/try`](try/index.html) on this site — extraction
  runs as WebAssembly inside a Web Worker; documents never leave your
  machine. Inputs cap at 100 MB.
- **Embedded in `docray-server`** at `/playground` — extraction uses the
  server's native engine (both engines appear in a selector when available).

`docray-server` embeds a browser workbench at **`/playground`** — the fastest
way to understand what docray extracts and to debug a specific document.

Drop a PDF or PPTX on it and every page or slide appears with:

- **A thumbnail rail** for navigation (arrow keys work; scanned pages carry a
  badge). PPTX thumbnails render progressively as they scroll into view and
  fall back individually to the extraction schematic if visual rendering fails.
- **Two independent panels**, each switchable between six lenses:
  - **source** — the rendered PDF page, or an offline visual PPTX render inside
    a locked-down, null-origin browser sandbox
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

Hover any box for its content, font, and coordinates.

## Notes

- The page loads pdf.js and fonts from CDNs, so the **browser** needs
  internet access for PDF rendering and web fonts — the extraction API itself
  does not. The PPTX renderer is vendored and makes no network requests.
- Hostile PPTX visual rendering runs in an iframe with only
  `sandbox="allow-scripts"` and a `default-src 'none'` Content Security Policy.
  The parent transfers document bytes in, never reads the iframe DOM, and
  accepts only small status messages plus a size-bounded PNG/WebP data URL for
  each thumbnail; that inert URL is used only as an image source. Errors and
  timeouts fall back to the structure schematic built from docray's extraction.
- Uploads go to the server's own `/v1/extract`; nothing leaves your
  deployment.
- The UI is a single self-contained HTML file compiled into the server
  binary — there is no build step and no separate deployment.
