# Architecture & guarantees

## The shape of the system

```text
                   ┌──────────────────────────────┐
 document upload ─►│ docray-server (axum)         │
                   │  sync: bounded semaphore     │
                   │  jobs: SQLite queue + pool   │
                   └──────────────┬───────────────┘
                                  │ spawns per document
                   ┌──────────────▼───────────────┐
                   │ docray CLI (subprocess)      │
                   │  docray-pdf / docray-pptx    │
                   │  timeout · memory rlimit ·   │
                   │  output cap                  │
                   └──────────────┬───────────────┘
                                  │ JSON or lean text on stdout
                                  ▼
                        response / stored result
```

Crates: `docray-model` (the serde schema — the contract everything shares),
`docray-core` (extractor trait, format sniffing, geometric char→word→line
grouping), `docray-pdf` (PDFium-backed extractor), `docray-pptx` (pure-Rust
OOXML extractor), `docray-cli`,
`docray-server`.

## Why a subprocess per document

Documents are hostile input, and PDF parsers in particular are large C++
codebases. docray treats parser compromise/crash as *expected*: the server
never parses PDF or PPTX input in its own process. Each extraction runs in a
worker with:

- a **wall-clock timeout** (killed, reported as `timeout`),
- a **memory rlimit** (Linux; the worker dies before the container OOMs),
- an **output cap** enforced while streaming (a JSON bomb is killed
  mid-stream, reported as `output_too_large`),
- concurrent pipe draining and bounded stderr capture (a worker that floods
  stderr cannot deadlock the server).

A segfault in the parser costs one request, never the service.

## Extraction guarantees

- **Deterministic**: identical input bytes → byte-identical JSON. PDF goldens
  are Linux-canonical because font metrics can vary; PPTX goldens are
  byte-exact on every platform.
- **Lossless at `char` level**: every text run, glyph box, image, path, and
  annotation the PDF physically contains, including content nested inside
  (possibly deeply nested) Form XObjects, with ancestor transforms composed
  into page space.
- **No silent failures**: unsupported object kinds, unreadable geometry, and
  per-page parse problems all land in `warnings`. Raster-only pages are
  flagged `scanned`.
- **Rotation-correct**: page dimensions and all coordinates are reported in
  the rotated, visible page space.

## Engine choice

Extraction geometry comes from **PDFium** (the renderer inside Chrome) via
the `pdfium-render` crate, pinned to an exact version. Character-level
bounding boxes require interpreting content streams, font metrics, CMaps, and
transformation matrices — the hardest part of PDF — and PDFium has been
hardened by billions of real-world documents. docray's own code owns the
schema, the grouping, the hierarchy, and everything above the glyph level.
