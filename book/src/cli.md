# CLI reference

```text
docray extract <FILE> [OPTIONS]

Options:
  --granularity <element|word|char>  Output detail. Omit for byte-identical
                                     lossless (schema 1.1) output.
  --format <json|lean>               Output encoding. Default: json. Lean
                                     implies element granularity.
  --max-pages <N>                    Refuse documents over the page/flow cap.
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
| 0 | — | success (warnings, if any, are inside the JSON / `#warning` lines in lean) |
| 2 | `unsupported_format` | input is not supported PDF/PPTX/DOCX/DOCM, or is legacy/encrypted Office |
| 3 | `encrypted_pdf` | password-protected |
| 4 | `parse_failure` | document could not be opened |
| 5 | `io_error` | file unreadable / missing |
| 6 | `too_many_pages` | over the `--max-pages` cap |
| 7 | `bad_format` | invalid format, or lean requested with `char` granularity |
| 8 | `granularity_unavailable` | the requested granularity is finer than this source provides |

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

# Pages that need OCR
docray extract scan.pdf --granularity element \
  | jq '[.pages[] | select(.scanned) | .page_number]'

# Fail a CI step if extraction produced warnings
docray extract input.pdf | jq -e '.warnings | length == 0'

# Token-lean element output for an LLM
docray extract report.pdf --format lean
```

`--format lean --granularity word` emits word boxes. Lean with no explicit
granularity implies `element`; `--format lean --granularity char` fails with
exit 7 and code `bad_format`. `--pretty` affects JSON only. See
[output formats](output-formats.md) for the line format and its deliberate
lossless-JSON deltas.

PPTX supports element granularity. An omitted `--granularity` defaults to
`element` for PPTX (so `docray extract deck.pptx` just works), and lean also
defaults to element; asking for finer detail (`word` or `char`) returns exit 8
with `granularity_unavailable`. See [PowerPoint extraction](pptx.md).

DOCX and DOCM also default to element and support lean. They emit schema 1.7
flow sections/blocks; `word` and `char` return exit 8. With pagination hints,
`--max-pages` caps the approximate page count. Without hints it caps blocks at
`N * 200` and records the approximation warning. See [Word extraction](docx.md).
