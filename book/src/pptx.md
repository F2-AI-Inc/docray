# PowerPoint extraction

docray extracts `.pptx` presentations natively in pure Rust. Each slide is a
page. Slide dimensions are read from the presentation in EMUs, converted at
12,700 EMUs per point, and reported in the same top-left, y-down point space as
PDF output.

PPTX is available at `element` granularity and in lean output:

```bash
docray extract deck.pptx --granularity element
docray extract deck.pptx --format lean
```

An omitted granularity, `char`, or `word` requests detail that PPTX cannot
provide and returns `granularity_unavailable`, with guidance to retry using
`granularity=element`.

## Extracted content

- Text shapes are emitted as one text element, with paragraphs separated by
  newlines and explicit DrawingML hard breaks preserved at their document
  position. The dominant summary remains on the text element, while `runs`
  preserves each ordinary run and field's resolved font, size, emphasis,
  theme color, normal-autofit scaling, and external hyperlink target. A hard
  break contributes `\n` to aggregate content but is not a styled `TextRun`.
- Tables are emitted as first-class table elements with grid dimensions and
  row-major anchor cells. Grid and row dimensions determine cell boxes;
  merged-cell continuations are omitted and the anchor carries the row/column
  span and merged box. Cell text also preserves per-run styles.
- Chart titles, axis titles, series names, category labels, and finite values
  are read from the related chart part and emitted as one text element at the
  graphic-frame bbox. Category/value points are paired by source index.
- SmartArt text is read from the related diagram-data part in document order
  and emitted as one text element at the graphic-frame bbox.
- Pictures are emitted as image elements. Their content hash covers the exact
  referenced media-part bytes, including pictures carried by graphic frames.
- Geometry-only shapes and connectors are emitted as path elements with the
  fill, stroke, and stroke width represented by the existing JSON model.
- External click hyperlinks continue to be emitted as link annotations. A
  text run also carries its external target in `href`; targets are returned
  literally and are never fetched.
- Placeholder roles are emitted in the non-visible `hidden` channel using the
  placeholder `type` verbatim; the ECMA default is `body` when `type` is absent.
- Speaker notes are emitted as page-targeted `notes`, using only the notes
  slide's body placeholder. Slide-image and slide-number placeholders are
  ignored.
- Shape and picture alternative text is emitted as element-targeted `alt`,
  preferring `descr` and falling back to `title`.
- Slides with `show="0"` remain ordinary extracted pages and carry a
  page-targeted `hidden-slide` item with content `true`.
- Placeholder geometry is inherited from the slide layout and master. Group,
  shape, picture, and frame rotation/flip transforms are flattened into slide
  coordinates.

Elements follow `p:spTree` document order, which is PowerPoint z-order. docray
does not infer a semantic reading order. Hidden items are explicitly marked as
non-visible context in JSON and lean so consumers do not mistake notes or
accessibility metadata for slide-visible text.

## Deliberate limits

PPTX extraction does not provide character or word geometry, render slides, or
reconstruct chart and SmartArt visual geometry: OOXML stores those layouts for
the application to render, so docray reports their text at the containing
frame bbox. OLE and unknown graphic frames with no extractable text produce a
warning. Legacy `.ppt`, encrypted Office documents, and other ZIP-based Office
formats are rejected.

## Container safety

PPTX files are OPC ZIP containers. Before parsing XML, docray enforces caps on
entry count, per-entry and total inflated size, and compression ratio. Unsafe
entry names are rejected, parts are read only by exact name, and nothing is
extracted to disk. DTD declarations and external entities are never resolved;
external relationship targets are treated as literal metadata only. The HTTP
server retains its normal worker-process timeout, memory limit, and output cap.
