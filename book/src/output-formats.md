# Output formats

docray has three output encodings: `json`, the default machine contract;
`lean`, a line-oriented reading format for token-conscious LLM consumers; and
`md`, deterministic GitHub-flavored Markdown for human review and semantic
consumption. Choose granularity separately: lean and Markdown accept `element`
and `word`; requesting either without a granularity implies `element`.

```bash
docray extract report.pdf --format lean
docray extract report.pdf --format lean --granularity word
curl -F file=@report.pdf 'http://localhost:41619/v1/extract?format=lean'
docray extract report.pdf --format md
curl -F file=@report.pdf 'http://localhost:41619/v1/extract?format=md'
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

## Classified JSON

Paged JSON can opt into deterministic page routing metadata with
`--classify` (CLI), `classify=true` (sync and async HTTP), or the WASM
`extract_classified` export. The response uses schema `1.8` and adds
`classification { kind, confidence, needs_ocr, reasons }` to every page.
It composes with `element`, `word`, and `char` granularity. Classification is
JSON-only; lean and Markdown remain reading projections without routing fields.

Without the option, PDF JSON remains byte-identical schema `1.1`. The legacy
`scanned` field and its existing heuristic are not changed.

## Markdown

Markdown is an additive semantic projection over the existing extraction. It
does not change the frozen schema 1.1 JSON response, compact JSON, or lean
bytes. It ends with a newline and is deterministic for identical input bytes
on a given platform and PDFium build.

For PDF, docray geometrically clusters text into supported columns, guards
against treating short inline font fragments as columns, orders each column
top-to-bottom, and emits columns left-to-right. It joins nearby lines into
paragraphs; recognizes bullet, numeric, and letter list markers; preserves
bold, italic, and hyperlink runs; and separates pages with `---`. Heading
levels H1-H4 are inferred from deterministic font-size ranks relative to the
document's weighted median body size; body-size bold text is promoted only
when it also reads like a heading (a short line), and a bold heading is
promoted one tier. A heading is never additionally wrapped in `**bold**`.
URI annotations are associated with the overlapping or nearest text. On
the PDF path, a strict ruled-table detector reconstructs grids with at least
two columns and two rows from thin vector rulings and renders their cells as
GFM pipe tables. Cell text
is assigned by geometric center and removed from ordinary reading-order output,
so it appears exactly once. Incomplete grids and path-dense chart-like regions
are rejected; table-like candidates that cannot be reconstructed are reported
as Markdown warning callouts. After ruled detection, a borderless
(alignment-based) detector runs only on the text no ruled table already claimed:
it groups visual lines into rows and finds columns from stable whitespace
gutters. The gate combines geometry (at least two columns and three rows, tight
column edges, and a dense fill) with content discriminators — a candidate whose
cells are predominantly sentence-like is rejected as multi-column running prose,
and a two-column candidate whose first column is colon-terminated labels is
rejected as a key/value form — so prose, code, and forms stay reading-order
text. Merged cells are recovered as `colspan`/`rowspan` — from missing interior
separators in ruled grids, and from cells straddling a gutter in borderless
ones. A table with any merged cell is emitted as a raw HTML `<table>` (which
carries the spans), while simple all-single-span tables stay GFM pipe tables;
every cell string is HTML-escaped so untrusted document text cannot break out of
the table, and link targets are dropped unless their scheme is `http`, `https`,
or `mailto` (scheme-less internal links are kept). This Markdown-only
reconstruction does not change schema 1.1 JSON, compact JSON, or lean output.

DOCX Markdown renders authored flow structure directly: `h1`-`h9` and `title`
roles become headings (levels above six use H6), quotes become block quotes,
lists retain nesting, and logical tables become GFM pipe tables. PPTX uses its
paged element model: placeholder roles inform headings, geometric sequencing
sets reading order, and existing first-class PowerPoint tables become GFM pipe
tables. Images without an exported asset URL are represented by an HTML
comment. Page and section breaks render as `---`.

Warnings remain visible as GFM warning callouts. Non-visible hidden-channel
items are retained as trailing `docray-hidden` HTML comments, so they do not
appear as document prose. Markdown escapes document-controlled Markdown/HTML
syntax, percent-encodes unsafe link-target characters, and drops any link whose
scheme is not `http`, `https`, or `mailto` (rendering its text only) across all
formats.

Markdown is a reading format, not a lossless replacement for JSON. It omits
geometry, source hashes, paint, and reconstruction metadata. Use `element` for
Markdown unless the same request plumbing also needs `word`; Markdown itself
does not emit coordinate detail. Use JSON `char` for the full archival model.

Native CLI and HTTP use `--format md` / `?format=md`; HTTP returns
`Content-Type: text/markdown; charset=utf-8`. The WASM API exports
`extract_markdown(bytes, granularity, max_input_bytes, max_output_bytes)`.

## Lean format specification

Lean is deterministic, line-oriented UTF-8 with `\n` separators and a final
newline. The first two lines are always:

```text
#docray <granularity> v<schema_version> pages=<N>[ warnings=<K>]
#legend <the fixed legend for the selected granularity>
```

When the response contains run, table, or chart detail, the element legend is:

```text
#legend T x0 y0 x1 y1 font size style text | r font size style [href#<uri>] text | TB x0 y0 x1 y1 rows cols | c row col rowspan colspan x0 y0 x1 y1 font size style text | CH x0 y0 x1 y1 type [title] | s series-name | p [category] value | I/P x0 y0 x1 y1 | A x0 y0 x1 y1 subtype uri | pt, top-left origin
```

and the word legend is:

```text
#legend T x0 y0 x1 y1 font size style | w x0 y0 x1 y1 word | r font size style [href#<uri>] text | TB x0 y0 x1 y1 rows cols | c row col rowspan colspan x0 y0 x1 y1 font size style text | CH x0 y0 x1 y1 type [title] | s series-name | p [category] value | I/P x0 y0 x1 y1 | A x0 y0 x1 y1 subtype uri | pt, top-left origin
```

Responses without run, table, or chart detail retain the preceding schema-1.3
legend shape (without `r`, `TB`, `c`, `CH`, `s`, or `p`). In particular, PDF
lean output has no such detail. Lean deliberately keeps path records bbox-only,
so the schema-1.6 bump changes PDF lean bytes only in the header version token;
compact JSON paths additionally carry their authored paint.

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

Schema 1.7 flow output uses a different fixed header and records because it
has no resolved pages or coordinates:

```text
#docray element v1.7 sections=<N>[ warnings=<K>]
#legend #section width height | H1..H9/TI/Q/P text | LI level o|b label text | r font size style [href#<uri>] text | TB cols col-width... | c row col rowspan colspan text | I [width height] | BR page|column|section | ~page N | pt, authored flow; no resolved coordinates
#section 612 792
H1 Heading text
LI 1 o 1.a) Nested item
TB 2 72 144
c 0 0 1 2 Merged cell
I 100 50
BR section
```

`~page N` comes only from `lastRenderedPageBreak`. Headers and footers are
written around their section body in story order. Textbox and nested-cell
blocks recurse into the same grammar. Flow hidden items use the same bounded
`<hidden>` block and escaping rules as paged output.

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

CH x0 y0 x1 y1 <chart-type> [<title to end of line>]
s <series name to end of line>
p [<category>] <formatted value to end of line>

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
| `source-layer` | element | `master` or `layout` for inherited visible shapes | not emitted |
| `field` | block | DOCX field instruction | not emitted |
| `comment` | block | DOCX comment body | not emitted |
| `tracked-insert` | block | DOCX accepted insertion | not emitted |
| `tracked-delete` | block | DOCX rejected deletion | not emitted |
| `footnote` | block | DOCX note body linked to its reference | not emitted |
| `endnote` | block | DOCX endnote body linked to its reference | not emitted |

New hidden semantics receive new documented kind strings; these six strings
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

`CH` introduces a first-class chart. Its title is optional. Each named series
emits an `s` record before its points; an unnamed series omits that record.
Each `p` carries its category followed by the already-formatted value, or only
the value when the source point has no category. Series and points remain in
deterministic source/index order.

The style token concatenates `b` for bold and `i` for italic, or uses `-` when
neither applies. A non-default text fill is appended as lowercase RGB hex,
for example `b#231f20` or `-#ff0000`.

Text, word, run text, run hyperlink URI, table-cell text, chart title, series
name, chart category, chart value, annotation URI, and hidden content use the
same escaping. Text-bearing fields run to end of line. Backslash becomes `\\`,
LF becomes `\n`, and CR becomes `\r`. Every other Unicode
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
- WASM exposes `extract`, `extract_classified`, `extract_lean`, and
  `extract_markdown`; all four
  share the same input/output caps and stable error envelope.

Lean HTTP successes use `Content-Type: text/plain; charset=utf-8`. Async jobs
persist their requested format with the job, so the result endpoint returns
the stored bytes with the same content type. JSON behavior and bytes are
unchanged when `format` is omitted or set to `json`.
