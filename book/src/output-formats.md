# Output formats

docray has two output encodings: `json`, the default machine contract, and
`lean`, a line-oriented reading format for token-conscious LLM consumers.
Choose granularity separately: lean supports `element` and `word`; requesting
lean without a granularity implies `element`.

```bash
docray extract report.pdf --format lean
docray extract report.pdf --format lean --granularity word
curl -F file=@report.pdf 'http://localhost:41619/v1/extract?format=lean'
```

Lean was selected by measuring real tokenizer counts on a real-document
corpus, not by estimating from byte size:

| Granularity | Lean reduction vs compact JSON |
|---|---:|
| `element` | 26–39% |
| `word` | 14.6% |

TOON was also measured and declined. Faithful TOON was worse than compact
JSON for this data shape, while a type-grouped TOON variant still trailed lean
by 7–10 percentage points.

## Format specification

Lean is deterministic, line-oriented UTF-8 with `\n` separators and a final
newline. The first two lines are always:

```text
#docray <granularity> v<schema_version> pages=<N>[ warnings=<K>]
#legend <the fixed legend for the selected granularity>
```

When the response contains per-run or table detail, the element legend is:

```text
#legend T x0 y0 x1 y1 font size style text | r font size style [href#<uri>] text | TB x0 y0 x1 y1 rows cols | c row col rowspan colspan x0 y0 x1 y1 font size style text | I/P x0 y0 x1 y1 | A x0 y0 x1 y1 subtype uri | pt, top-left origin
```

and the word legend is:

```text
#legend T x0 y0 x1 y1 font size style | w x0 y0 x1 y1 word | r font size style [href#<uri>] text | TB x0 y0 x1 y1 rows cols | c row col rowspan colspan x0 y0 x1 y1 font size style text | I/P x0 y0 x1 y1 | A x0 y0 x1 y1 subtype uri | pt, top-left origin
```

Responses without run or table detail retain the preceding schema-1.3 legend
shape (without `r`, `TB`, or `c`). In particular, PDF lean output has no such
detail. Lean deliberately keeps path records bbox-only, so the schema-1.5
change to PDF lean bytes is limited to the header version token; compact JSON
paths additionally carry their authored paint.

When any page contains non-visible context, one additional legend line follows
the element/word legend:

```text
#legend <hidden> kind [element-id] content | non-visible document context
```

When warnings exist, each follows the legend immediately. Newlines and tabs
inside a warning are collapsed to one space:

```text
#warning <warning text>
```

Each page then starts with:

```text
#page <n> <W>x<H>[ rot=<degrees>][ scanned]
```

Elements follow in extraction/content-stream z-order:

```text
# element granularity
T x0 y0 x1 y1 <font> <size> <style> <text to end of line>

# word granularity: word records are nested immediately under their T record
T x0 y0 x1 y1 <font> <size> <style>
w x0 y0 x1 y1 <word text to end of line>

# emitted directly after T when the text element has multiple runs or a linked run
r <font> <size> <style> <text to end of line>
r <font> <size> <style> href#<external-uri> <text to end of line>

TB x0 y0 x1 y1 <rows> <cols>
c <row> <col> <rowspan> <colspan> x0 y0 x1 y1 <font> <size> <style> <cell text to end of line>
# multi-run or linked cells use the same r records directly after their c record

I x0 y0 x1 y1
P x0 y0 x1 y1
A x0 y0 x1 y1 <subtype> <uri or ->
```

After a page's element records, its non-visible context is explicitly bounded:

```text
<hidden>
<kind> [<element-id>] <content to end of line>
</hidden>
```

The element ID is present only when the item annotates a visible element. The
block appears after that page's elements and before the next `#page`. Documents
without hidden items omit both the block and its legend line.

Hidden content uses the same escapes as visible text and annotation URIs:
backslash becomes `\\`, LF becomes `\n`, CR becomes `\r`, and every other
control character plus U+2028/U+2029 becomes `\u{hex}` with lowercase,
unpadded hexadecimal digits. An item's content therefore occupies exactly one
physical line and can never produce a line equal to `</hidden>` or forge a
visible element record.

Hidden kinds are stable contract strings:

| Kind | Target | PPTX | PDF |
|---|---|---|---|
| `role` | element | Placeholder `type` (`body` when omitted) | not emitted |
| `notes` | page | Speaker-notes body text | not emitted |
| `alt` | element | Shape/picture `descr`, falling back to `title` | not emitted |
| `hidden-slide` | page | `true` when the slide has `show="0"` | not emitted |

New hidden semantics receive new documented kind strings; these four strings
are never repurposed or renamed.

All coordinates use PDF points with a top-left origin after page rotation.
Numbers, including font and page sizes, round to one decimal and omit a
trailing `.0` (`72`, not `72.0`; `61.1` remains `61.1`). Every whitespace
character in a font name becomes `_`; a missing font name is `-`.

`TB` introduces a first-class table and is followed by one `c` record per
merge-anchor cell in row-major order. Row and column indices are zero-based;
spans are at least one. Each `c` carries the font, size, and style of its first
run as its cell summary; an empty cell uses `-` for all three. A plain single
run adds no information beyond its parent `T` or `c` record, so it has no `r`
record. Multiple runs emit every `r`, and a linked single run emits its one
`r` so the hyperlink is not lost. A linked run inserts the literal token
`href#<`, the escaped external URI, and `>` before its text.

The style token concatenates `b` for bold and `i` for italic, or uses `-` when
neither applies. A non-default text fill is appended as lowercase RGB hex,
for example `b#231f20` or `-#ff0000`.

Text, word, run text, run hyperlink URI, table-cell text, annotation URI, and
hidden content use the same escaping. Text-bearing fields run to end of line. Backslash
becomes `\\`, LF becomes `\n`, and CR becomes `\r`. Every other Unicode
control character, U+2028, and U+2029 becomes `\u{hex}` with lowercase,
unpadded hexadecimal digits (for example, tab is `\u{9}`). All other
characters are literal. A fixed-position optional value that is absent is `-`.

## JSON versus lean

Lean is a reading format, not a lossless replacement for JSON:

- It omits the JSON envelope, including source format, SHA-256, byte size, and
  document metadata. The header carries only granularity, schema version, page
  count, and warning count.
- It includes non-default text fill color but deliberately omits stroke color
  and path paint. Use compact JSON when a path's fill, stroke, or stroke width
  is required for reconstruction.
- It supports only `element` and `word`; use JSON for the lossless `char`
  hierarchy and reconstruction metadata.
- The Rust/Wasm API emits JSON only. Lean is available from the native CLI and
  HTTP server.

Lean HTTP successes use `Content-Type: text/plain; charset=utf-8`. Async jobs
persist their requested format with the job, so the result endpoint returns
the stored bytes with the same content type. JSON behavior and bytes are
unchanged when `format` is omitted or set to `json`.
