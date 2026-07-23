# Word extraction

DOCX and DOCM are extracted as authored **flow**, not as fabricated pages.
WordprocessingML stores paragraphs, runs, tables, sections, and positioned
objects, but a layout engine computes line breaks, y positions, and pages.
DOCX output therefore uses schema `1.7`, `layout: "flow"`, and
`sections[].blocks[]`; it never invents y coordinates or page assignments.

DOCX is element-only and defaults to that finest level:

```bash
docray extract report.docx
docray extract report.docx --format lean
```

`word` and `char` return `granularity_unavailable`. DOCM is parsed identically,
reports `source.format: "docm"`, and warns `macro project ignored`; VBA bytes
are never read.

## Preserved structure

- Paragraph reading order; resolved `h1`–`h9`, `title`, `quote`, or `body`
  roles; resolved fonts, sizes, emphasis, colors, and hyperlinks.
- Numbering labels after list counters, overrides, and restarts are applied.
- Authored section page size, margins, and column count. These describe an
  intended page frame, not resolved pagination.
- Logical tables, authored grid widths, merges, nested blocks, and floating
  table placement constraints.
- Text inside `w:sdt` content controls, smart tags, and custom XML wrappers;
  these wrappers are transparent and do not create extra flow blocks.
- Inline image extents and anchored image/textbox placement constraints.
  `frame` records page-, margin-, column-, paragraph-, or line-relative
  offsets. Image bytes are represented by SHA-256 content hashes.
- Section-scoped default/first/even header and footer stories. Referenced
  footnote and endnote paragraphs are appended to the section blocks.

Adjacent identically formatted and linked runs are merged. RTL and
complex-script text remains in stored logical order; docray does not reverse
or visually reorder it.

## Pagination hints and limits

`lastRenderedPageBreak` is a producer cache, not authored layout. When present,
docray derives optional `approx_page` block hints and top-level `approx_pages`
from it. Hints in body table cells, nested tables, and textboxes advance the
same body counter; header, footer, footnote, and endnote hints do not. A table
is traversed in row/cell flow order, so its first cell paragraph retains the
page where the table starts while later cell paragraphs can advance as hints
pass. Cached `PAGE` field results never count as pagination. When no cache
markers exist, `approx_page` is omitted and the response contains exactly one
`no pagination hints; approx_page omitted` warning.

For flow documents, `max_pages` caps `approx_pages` when hints exist. Without
hints it is approximated as `max_pages * 200` total blocks and the response
warns `max_pages approximated as block cap for flow documents`.

## Non-visible context

Each section can carry stable hidden items targeted by block ID:

| Kind | Meaning |
|---|---|
| `field` | Field instruction; only its cached result is visible |
| `comment` | Comment body only; author and date are excluded |
| `tracked-insert` | Inserted text, also visible in the accepted projection |
| `tracked-delete` | Deleted text, excluded from visible content |
| `alt` | Drawing alternative text |
| `footnote` | Note body linked to its reference block |

Tracked moves follow the same accepted projection: `moveTo` is insertion and
`moveFrom` is deletion. External hyperlinks remain literal strings and are
never fetched. Missing media keeps its image block with a null hash and a
warning.
