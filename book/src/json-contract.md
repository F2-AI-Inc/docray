# The JSON contract

This page documents the **lossless `char`-level contract** (schema `1.1`,
the default when no granularity is requested). The compact shapes are
documented in [choosing a granularity](granularity.md).

DOCX/DOCM uses the separate schema `1.7` flow contract. Its envelope contains
`layout: "flow"`, optional `approx_pages`, and `sections` instead of `pages`:

```json
{
  "granularity": "element", "schema_version": "1.7", "layout": "flow",
  "source": {"format": "docx", "sha256": "…", "size_bytes": 1234},
  "document": {"metadata": {"title": "…", "author": "…"}},
  "warnings": [], "approx_pages": null,
  "sections": [{
    "page_width": 612.0, "page_height": 792.0,
    "margins": {"top": 72.0, "right": 72.0, "bottom": 72.0, "left": 72.0},
    "headers": [], "footers": [], "blocks": []
  }]
}
```

Flow block types are `paragraph`, `table`, `image`, `textbox`, and `break`.
Paragraphs contain stable block IDs, semantic roles, resolved runs, optional
list labels and approximate-page hints, and authored breaks. Tables use
authored column widths and merge-anchor cells. Positioned tables, images, and
textboxes carry tagged placement constraints, never resolved bounding boxes.
See [Word extraction](docx.md) for provenance and limits.

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

Granularity-shaped schema `1.6` pages can also carry a `hidden` array. The
field is omitted when empty and is copied unchanged across granularities:

```json
"hidden": [
  { "kind": "role", "element": "p1-e0", "content": "title" },
  { "kind": "notes", "content": "Presenter script" }
]
```

`element` is omitted for page-targeted items. Hidden content is supplemental,
non-visible document context and must not be treated as text rendered on the
page. The kind namespace is stable:

| Kind | Target | PPTX | PDF |
|---|---|---|---|
| `role` | element | Placeholder type, defaulting to `body` | not emitted |
| `notes` | page | Speaker-notes body text | not emitted |
| `alt` | element | Shape/picture alternative text | not emitted |
| `hidden-slide` | page | `true` for a slide with `show="0"` | not emitted |
| `source-layer` | element | `master` or `layout` for inherited visible shapes | not emitted |
| `field` | block | DOCX field instruction | not emitted |
| `comment` | block | DOCX comment body | not emitted |
| `tracked-insert` | block | DOCX accepted insertion | not emitted |
| `tracked-delete` | block | DOCX rejected deletion | not emitted |
| `footnote` | block | DOCX note linked to its reference | not emitted |

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

Schema `1.6` granularity-shaped text elements can additionally carry `runs`.
Each run preserves its own content, resolved font, color, and optional external
hyperlink target:

```json
"runs": [
  {
    "content": "linked text",
    "font": { "name": "Aptos", "size": 18.0, "bold": true },
    "color": { "fill": [31, 78, 121] },
    "href": "https://example.com"
  }
]
```

`href` is omitted for an unlinked run. Element/word compact output applies the
same compact font and color rules to runs as to their parent: false emphasis
flags, black fill, and empty color objects are omitted. PDF text has no
separate native shape run layer and omits `runs`, preserving the frozen
no-parameter schema `1.1` bytes. PPTX text keeps `content`, `font`, and `color`
as the concatenated and dominant summary while using `runs` for the per-run
detail.

### table

Schema `1.6` carries first-class table elements for PPTX:

```json
{
  "id": "p1-e2", "type": "table",
  "bbox": {"x0": 72.0, "y0": 90.0, "x1": 272.0, "y1": 170.0},
  "rows": 2, "cols": 2,
  "cells": [
    {
      "bbox": {"x0": 72.0, "y0": 90.0, "x1": 272.0, "y1": 120.0},
      "row": 0, "col": 0, "row_span": 1, "col_span": 2,
      "content": "Merged heading",
      "runs": []
    }
  ]
}
```

`rows` and `cols` are the source grid dimensions. Only merge-anchor cells are
emitted; continuation cells are omitted and the anchor carries the clamped
span and merged bounding box. Cell paragraphs are joined with `\n`, and cell
`runs` use the same shape as text-element runs. PDF emits no table elements.

### chart

Schema `1.6` carries first-class chart elements for PPTX:

```json
{
  "id": "p1-e3", "type": "chart",
  "bbox": {"x0": 72.0, "y0": 72.0, "x1": 432.0, "y1": 288.0},
  "chart_type": "doughnut",
  "title": "Channel mix",
  "series": [
    {
      "name": "Share",
      "points": [
        {"category": "Direct", "value": "41%"},
        {"category": "Reseller", "value": "59%"}
      ]
    }
  ]
}
```

`chart_type` is `bar`, `pie`, `doughnut`, `line`, `area`, `scatter`, or
`other`, derived from the chart node in `plotArea`. Combo charts use the first
chart node in document order while retaining every series in document order.
The optional chart title and series name are omitted when absent. Points pair
categories and finite values by their source index; unmatched values remain as
points without `category`. Values are strings formatted with the series'
OOXML `formatCode`, so a stored `0.41` displayed as `0%` is returned as
`"41%"`. PDF emits no chart elements.

### image

Bounding box plus a `quad` (four corner points — meaningful when the image is
placed with rotation or skew), pixel dimensions, colorspace, and a
`content_hash` (sha256 of the raw image data) for deduplication. Pixel data
itself is never embedded.

### path

Bounding box plus paint: fill/stroke colors and stroke width. Path operator
lists are not included. Schema `1.6` compact element/word paths retain the
same optional `fill`, `stroke`, and `stroke_width` fields while rounding the
bbox and stroke width to one decimal. An absent paint field is omitted;
compact images remain bbox-only.

### annotation

Subtype (`link`, `highlight`, `widget`, …), bounding box, and `uri` for
links.

## Stability

- The no-parameter response is frozen at schema `1.1` — new fields are only
  ever additive, and granularity-shaped responses carry their own version
  (`1.6`) and a `granularity` discriminator. Flow responses use schema `1.7`.
  PDF emits no hidden items, runs,
  tables, or charts, so its
  no-parameter `1.1` bytes remain unchanged.
- Element IDs, field names, and the coordinate system are load-bearing
  contract; they do not change within a major schema version. Hidden kind
  strings are equally stable and are never renamed.
