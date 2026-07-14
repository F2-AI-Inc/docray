# The JSON contract

This page documents the **lossless `char`-level contract** (schema `1.1`,
the default when no granularity is requested). The compact shapes are
documented in [choosing a granularity](granularity.md).

## Coordinate system

Everything you need to place a box on a rendered page:

- Origin **top-left**, y increases **downward** — what viewers and CV
  pipelines expect.
- Units are **PDF points** (1/72 inch).
- Coordinates are reported **after page rotation** — a 612×792 page with
  `/Rotate 90` reports `width: 792, height: 612` and boxes in that rotated,
  visible space. What you see is what the coordinates mean.
- All values are rounded to 3 decimals; output is deterministic —
  byte-identical for identical input on a given platform and PDFium build.
  (Documents using non-embedded fonts pick up the platform's substitute
  font metrics, so coordinates can differ by fractions of a point across
  operating systems.)
- Bounding boxes are objects — `{"x0", "y0", "x1", "y1"}` — never bare
  arrays at this level.

## Envelope

```json
{
  "schema_version": "1.1",
  "source": { "format": "pdf", "sha256": "…", "size_bytes": 123456 },
  "document": { "page_count": 12, "metadata": { "title": "…", "author": "…" } },
  "warnings": [],
  "pages": [
    { "page_number": 1, "width": 612.0, "height": 792.0,
      "rotation": 0, "scanned": false, "elements": [] }
  ]
}
```

`warnings` is the no-silent-failure channel: skipped object kinds, per-page
parse problems, geometry that couldn't be read. Empty means a fully clean
extraction.

`scanned` is true when a page has **no text elements and a single image
covering ≥ 85% of the page area** — the signal that a page's text
is not machine-readable and needs OCR to recover. It also flags pre-rendered
(rasterized-slide) pages, which have the same property.

## Elements

One element per native PDF page object, in z-order, discriminated by
`"type"`. IDs are stable within a response: `p{page}-e{index}`.
Content inside Form XObjects (containers PowerPoint exports wrap everything
in) is recursively extracted and flattened into the page's element stream
with correct page-space coordinates.

### text

```json
{
  "id": "p1-e4", "type": "text",
  "bbox": {"x0": 294.9, "y0": 48.0, "x1": 300.3, "y1": 61.2},
  "content": "Introduction to parsing",
  "font": { "name": "NimbusRomNo9L-Regu", "size": 10.909, "bold": false, "italic": false },
  "color": { "fill": [0, 0, 0], "stroke": null },
  "lines": [
    { "bbox": {}, "baseline_y": 61.2,
      "words": [
        { "content": "Introduction", "bbox": {},
          "chars": [ { "content": "I", "bbox": {}, "unicode": 73 } ] }
      ] }
  ]
}
```

One text element per native text run. Lines and words are grouped
geometrically and deterministically; whitespace characters separate words and
are not emitted as `chars`. Word order is content-stream order — reading
order is not inferred.

### image

Bounding box plus a `quad` (four corner points — meaningful when the image is
placed with rotation or skew), pixel dimensions, colorspace, and a
`content_hash` (sha256 of the raw image data) for deduplication. Pixel data
itself is never embedded.

### path

Bounding box plus paint: fill/stroke colors and stroke width. Path operator
lists are not included.

### annotation

Subtype (`link`, `highlight`, `widget`, …), bounding box, and `uri` for
links.

## Stability

- The no-parameter response is frozen at schema `1.1` — new fields are only
  ever additive, and granularity-shaped responses carry their own version
  (`1.2`) and a `granularity` discriminator.
- Element IDs, field names, and the coordinate system are load-bearing
  contract; they do not change within a major schema version.
