# CLI reference

```text
docray extract <FILE> [OPTIONS]

Options:
  --granularity <element|word|char>  Output detail. Omit for byte-identical
                                     lossless (schema 1.1) output.
  --format <json|lean|md>            Output encoding. Default: json. Lean and
                                     Markdown imply element granularity.
  --classify                         Add per-page classification (schema 1.8;
                                     JSON only).
  --max-pages <N>                    Refuse documents over the page/flow cap.
  --pages <spec>                     Extract only a sub-range of pages, e.g.
                                     "7" or "1-200". PDF only.
  --pretty                           Pretty-print the JSON.
```

The selected document representation is written to **stdout**; nothing else
ever is. The CLI is also the isolation worker the server spawns per document,
so its contract is deliberately strict and machine-parseable.

## Errors

Failures print a single JSON object to **stderr**:

```json
{"error": {"code": "encrypted_pdf", "message": "PDF is encrypted / password-protected"}}
```

with a stable exit code:

| Exit | Code | Meaning |
|---:|---|---|
| 0 | — | success (warnings are inside JSON, `#warning` lean lines, or Markdown callouts) |
| 2 | `unsupported_format` | input is not supported PDF/PPTX/DOCX/DOCM, or is legacy/encrypted Office |
| 3 | `encrypted_pdf` | password-protected |
| 4 | `parse_failure` | document could not be opened |
| 5 | `io_error` | file unreadable / missing |
| 6 | `too_many_pages` | over the `--max-pages` cap |
| 7 | `bad_format` | invalid format, or lean/Markdown requested with `char` granularity |
| 7 | `bad_pages` | `--pages` value is unparseable, reversed (start > end), zero, or negative |
| 8 | `granularity_unavailable` | the requested granularity is finer than this source provides |
| 9 | `page_out_of_range` | `--pages` range extends beyond the document's last page |
| 10 | `page_selection_unsupported` | `--pages` was given for a non-PDF format (PPTX, DOCX, DOCM) |

Anything else (e.g. 101, or death by signal) means the parser crashed —
treat it as `crash`. The server does exactly this mapping.

## Environment

| Variable | Purpose |
|---|---|
| `DOCRAY_PDFIUM_DIR` | Directory containing the PDFium dynamic library. Falls back to `./.pdfium/lib`, then the system library. |

## Pipeline examples

```bash
# All text of a document, one line per element
docray extract report.pdf --granularity element \
  | jq -r '.pages[].elements[] | select(.type=="text") | .text'

# Pages that need OCR, including mixed/garbled cases
docray extract scan.pdf --classify --granularity element \
  | jq '[.pages[] | select(.classification.needs_ocr) | .page_number]'

# Fail a CI step if extraction produced warnings
docray extract input.pdf | jq -e '.warnings | length == 0'

# Token-lean element output for an LLM
docray extract report.pdf --format lean

# Reading-order Markdown with inferred PDF headings
docray extract report.pdf --format md
```

`--format lean --granularity word` emits word boxes. Lean with no explicit
granularity implies `element`; `--format lean --granularity char` fails with
exit 7 and code `bad_format`. `--pretty` affects JSON only. See
[output formats](output-formats.md) for the line format and its deliberate
lossless-JSON deltas.

`--format md` follows the same granularity defaults and `char` rejection.
Markdown emits semantic prose rather than coordinate detail; `element` is the
recommended setting.

`--classify` is opt-in and available only for JSON. Paged PDF/PPTX responses
use schema `1.8` and add a `classification` object to each page. With no flag,
PDF output remains the byte-identical schema `1.1` contract and the legacy
`scanned` field is unchanged.

`--pages` selects a 1-based sub-range of a PDF: `--pages 7` for a single
page, `--pages 1-200` for an inclusive range. Page numbers stay absolute
over the whole document — `--pages 201-287` on a 287-page PDF emits pages
numbered `201` through `287`, not renumbered from `1`. `--max-pages`
compares against the *selected* page count, so `--pages 1-200 --max-pages
200` succeeds on a document with far more than 200 pages. Omitting
`--pages` extracts the whole document, unchanged. `--pages` is PDF-only;
requesting it on PPTX or DOCX/DOCM fails with exit 10 and
`page_selection_unsupported`.

PPTX supports element granularity. An omitted `--granularity` defaults to
`element` for PPTX (so `docray extract deck.pptx` just works), and lean also
defaults to element; asking for finer detail (`word` or `char`) returns exit 8
with `granularity_unavailable`. See [PowerPoint extraction](pptx.md).

DOCX and DOCM also default to element and support lean and Markdown. They emit schema 1.7
flow sections/blocks; `word` and `char` return exit 8. With pagination hints,
`--max-pages` caps the approximate page count. Without hints it caps blocks at
`N * 200` and records the approximation warning. See [Word extraction](docx.md).
