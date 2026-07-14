# Limitations

docray is honest about what it does not do. Everything here is by design or
known, not hidden.

- **PDF only.** The extractor trait and format sniffing are built to support
  multiple formats, but PDF is the only implemented input today.
- **No OCR.** Raster-only pages are *flagged* (`"scanned": true`) but their
  text is not recovered — recovering it requires OCR downstream.
- **No semantic layer.** docray reports physical structure — it does not
  classify headings, paragraphs, tables, or lists, and does not infer reading
  order. Text and words appear in content-stream order, which usually but not
  always matches reading order.
- **Silently recovered corruption.** When the underlying parser encounters a
  corrupt page it sometimes recovers by rendering an empty page without
  reporting an error; such pages are indistinguishable from genuinely blank
  ones.
- **Shading objects** (gradient fills) are skipped, with a warning per
  skipped object.
- **Job state is instance-local** — single-instance deployments are the
  supported topology.
- **Rotated-page text grouping**: geometry on rotated pages is correct, but
  text that renders vertically groups one character per line.
- **No HTTP response compression** — front docray with a reverse proxy and
  enable gzip/brotli there if you transport large `char`-level responses.
