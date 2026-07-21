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
  newlines. Run font, size, emphasis, theme color, and normal-autofit scaling
  are resolved where present.
- Table cells are emitted as text elements. Grid and row dimensions determine
  their boxes; merged-cell continuations are omitted and the anchor spans the
  merged box.
- Pictures are emitted as image elements. Their content hash covers the exact
  referenced media-part bytes.
- Geometry-only shapes and connectors are emitted as path elements with the
  fill, stroke, and stroke width represented by the existing JSON model.
- External click hyperlinks are emitted as link annotations. Their targets are
  returned literally and are never fetched.
- Placeholder geometry is inherited from the slide layout and master. Group,
  shape, picture, and frame rotation/flip transforms are flattened into slide
  coordinates.

Elements follow `p:spTree` document order, which is PowerPoint z-order. docray
does not infer a semantic reading order.

## Deliberate limits

PPTX extraction does not provide character or word geometry, render slides, or
reconstruct chart and SmartArt geometry. Unsupported graphic frames produce a
warning. Legacy `.ppt`, encrypted Office documents, and other ZIP-based Office
formats are rejected.

## Container safety

PPTX files are OPC ZIP containers. Before parsing XML, docray enforces caps on
entry count, per-entry and total inflated size, and compression ratio. Unsafe
entry names are rejected, parts are read only by exact name, and nothing is
extracted to disk. DTD declarations and external entities are never resolved;
external relationship targets are treated as literal metadata only. The HTTP
server retains its normal worker-process timeout, memory limit, and output cap.
