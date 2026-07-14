# docray

**X-ray for documents.** docray takes a PDF and returns JSON describing every
physical element on every page — text with a full character → word → line
hierarchy, images, vector paths, and annotations — each with a bounding box,
font, and color information. It sees through the rendered page to the
skeleton underneath.

```bash
docray extract report.pdf --granularity element
```

```json
{
  "type": "text",
  "bbox": [72.0, 61.1, 143.4, 74.5],
  "text": "Quarterly results",
  "font": { "name": "Helvetica-Bold", "size": 18.0, "bold": true }
}
```

## What it's for

- **LLM / RAG pipelines** — token-efficient page content with coordinates for
  click-to-source citations. Start with the
  [granularity guide](granularity.md); if you are token-conscious, you want
  `element`.
- **Document viewers** — overlay highlights and selections on the original
  page using the same coordinates the extraction reports.
- **ML and data extraction** — deterministic, lossless physical structure at
  `char` granularity for training data and downstream parsing.

## Design principles

**Lossless and unopinionated.** At full granularity docray records what the
PDF physically contains — it does not guess at headings, tables, or reading
order. Consumers filter down; the extractor does not interpret.

**Deterministic.** The same input bytes always produce byte-identical JSON.
The test suite enforces this with golden files and double-extraction checks.

**No silent failures.** Anything skipped, unsupported, or partially parsed is
recorded in a `warnings` array. Scanned (raster-only) pages are flagged
`"scanned": true` so you know exactly which pages need OCR.

**Hardened against hostile input.** PDFs are untrusted. The HTTP server never
parses them in-process — every document runs in an isolated worker subprocess
with a wall-clock timeout, memory limit, and output cap. A malformed file that
crashes the parser kills one job, never the service.

## The pieces

| Piece | What it is |
|---|---|
| `docray` | CLI: PDF in, JSON on stdout |
| `docray-server` | HTTP API: sync extraction + async job queue |
| `/playground` | Browser workbench: pages beside their bounding boxes, live JSON |
| `docray-{model,core,pdf}` | Rust crates: the schema, extraction traits, and PDFium-backed extractor |
