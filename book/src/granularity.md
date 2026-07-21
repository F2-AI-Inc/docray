# Choosing a granularity

docray emits three output shapes. **This is the most important decision you
make as a consumer** — it changes payload size by more than an order of
magnitude.

> **The short version: if an LLM reads the output, use `element`, then select
> the token-lean [`lean` output format](output-formats.md) when you do not need
> the JSON provenance envelope.** It carries the text, position, and style of
> every element at ~7% of the lossless payload. Only move to `word` when you
> need word-level highlighting, and to `char` when you need the full archival
> hierarchy.

## The three levels

| Level | Shape | Measured size¹ | Use when |
|---|---|---:|---|
| `element` | one `text` string + bbox per element | **−92.9%** | LLM/RAG consumption, semantic processing, citations |
| `word` | flat `[text, x0, y0, x1, y1]` tuples | **−89.2%** | word-precise highlighting, search-hit boxes |
| `char` *(default)* | full char → word → line hierarchy | baseline | archival, ML training data, anything lossless |

¹ Measured across a mixed corpus of real documents (bank statements, pitch
decks, a 49-page signed contract) totalling 22 MB of `char` output.

## element

```bash
docray extract file.pdf --granularity element
# or: POST /v1/extract?granularity=element
```

```json
{
  "type": "text",
  "bbox": [399.6, 90.7, 531.5, 99.4],
  "text": "Customer service information",
  "font": { "name": "ConnectionsBold_CZEX0AA0", "size": 9.5, "bold": true },
  "color": { "fill": [35, 31, 32] }
}
```

Non-text elements reduce to `{"type": "image|path", "bbox": [...]}`;
annotations keep their `subtype` and `uri`. The bbox is still precise enough
for click-to-source highlighting.

## word

```json
{
  "type": "text",
  "bbox": [399.6, 90.7, 531.5, 99.4],
  "font": { "name": "ConnectionsBold_CZEX0AA0", "size": 9.5, "bold": true },
  "words": [
    ["Customer", 399.6, 90.8, 442.0, 99.4],
    ["service", 444.8, 90.8, 476.3, 99.4],
    ["information", 479.1, 90.7, 531.5, 99.4]
  ]
}
```

Each word is a positional tuple: `[text, x0, y0, x1, y1]`. Words appear in
content-stream (extraction) order — docray does not infer reading
order.

## char (the default)

Omit the parameter and you get the lossless v1.1 contract, byte-identical
across versions: every text run with nested lines, words, and per-character
boxes, full font/color detail on every element, image quads and content
hashes, path stroke properties. This is the archival shape — see
[the JSON contract](json-contract.md).

## Rules shared by the compact levels

- Coordinates round to **1 decimal** (0.05 pt max displacement — chosen over
  integers, which produced degenerate zero-area boxes on thin paths).
- **Omitted when default:** `bold`/`italic` when false, `fill` when black,
  `stroke` when absent or black. If you see `"bold": true`, it is always
  informative.
- **Never omitted:** page dimensions, element type, bbox, `scanned` flags, and
  any *non-empty* `warnings` array. Silent-failure freedom survives every
  granularity.
- Compact responses report `"schema_version": "1.4"` and echo the
  `"granularity"` you asked for.
- Every compact granularity carries each page's non-visible `hidden` items
  verbatim; granularity changes visible text detail, not supplemental context.

Granularity controls which information is retained; output format controls
how that information is encoded. See [output formats](output-formats.md) for
the measured JSON-versus-lean tradeoff and complete lean specification.

## Deliberate non-optimizations

Two further size levers were measured and rejected — documented here so you
know they were choices, not oversights:

- **One-letter type tags** (`"t"` vs `"text"`) save only 0.3% more while
  making the payload harder for models and humans to read.
- **Document-level font/color tables** save ~13% more but break
  self-containment: a single page or element retrieved into a RAG context
  would no longer describe itself.
