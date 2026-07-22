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
- Charts are emitted as first-class chart elements at the graphic-frame bbox,
  with chart type, optional title, and ordered series containing optional names
  and category/value points. Category/value points are paired by source index,
  and values reuse the chart series' number format (for example, `0.41` with
  `0%` becomes `41%`). Combo charts use the first chart node for `chart_type`.
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
- Visible non-placeholder shapes inherited from the slide master and layout
  are extracted through the same shape, picture, connector, group, table, and
  graphic-frame paths as slide-owned content. Their element-targeted
  `source-layer` hidden item contains `master` or `layout`.
- Template placeholder shapes are not emitted, so authoring prompts such as
  "Click to edit ..." do not appear as page content. They still participate in
  placeholder geometry and style inheritance for slide-owned placeholders.
- A slide with `showMasterSp="0"` suppresses both layout and master shapes. A
  layout with `showMasterSp="0"` suppresses master shapes while retaining its
  own shapes. An absent attribute defaults to showing inherited shapes.
- Placeholder geometry is inherited from the slide layout and master. Group,
  shape, picture, and frame rotation/flip transforms are flattened into slide
  coordinates.

Elements follow PowerPoint z-order: master shapes, layout shapes, then the
slide's own `p:spTree`, with document order preserved inside each layer. docray
does not infer a semantic reading order. Hidden items are explicitly marked as
non-visible context in JSON and lean so consumers do not mistake notes or
accessibility metadata for slide-visible text.

## Deliberate limits

PPTX extraction does not provide character or word geometry, render slides, or
reconstruct chart and SmartArt visual geometry: OOXML stores those layouts for
the application to render. Charts report their structure at the containing
frame bbox; SmartArt remains a text element at that bbox. OLE and unknown
graphic frames with no extractable text produce a warning. Legacy `.ppt`,
encrypted Office documents, and other ZIP-based Office formats are rejected.

## Container safety

PPTX files are OPC ZIP containers. Before parsing XML, docray enforces caps on
entry count, per-entry and total inflated size, and compression ratio. Unsafe
entry names are rejected, parts are read only by exact name, and nothing is
extracted to disk. DTD declarations and external entities are never resolved;
external relationship targets are treated as literal metadata only. The HTTP
server retains its normal worker-process timeout, memory limit, and output cap.
