# CLI reference

```text
docray extract <FILE> [OPTIONS]

Options:
  --granularity <element|word|char>  Output detail. Omit for byte-identical
                                     lossless (schema 1.1) output.
  --max-pages <N>                    Refuse documents with more pages.
  --pretty                           Pretty-print the JSON.
```

The JSON document is written to **stdout**; nothing else ever is. The CLI is
also the isolation worker the server spawns per document, so its contract is
deliberately strict and machine-parseable.

## Errors

Failures print a single JSON object to **stderr**:

```json
{"error": {"code": "encrypted_pdf", "message": "PDF is encrypted / password-protected"}}
```

with a stable exit code:

| Exit | Code | Meaning |
|---:|---|---|
| 0 | — | success (warnings, if any, are inside the JSON) |
| 2 | `unsupported_format` | input is not a PDF |
| 3 | `encrypted_pdf` | password-protected |
| 4 | `parse_failure` | document could not be opened |
| 5 | `io_error` | file unreadable / missing |
| 6 | `too_many_pages` | over the `--max-pages` cap |

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
```
