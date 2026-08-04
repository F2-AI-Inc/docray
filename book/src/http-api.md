# HTTP API

Endpoints accept PDF, PPTX, DOCX, or DOCM multipart uploads and return JSON by default. Successful lean
and Markdown extractions return UTF-8 text; every error — at any layer — still uses the
same JSON envelope:

```json
{"error": {"code": "…", "message": "…"}}
```

## Sync extraction

```text
POST /v1/extract[?granularity=element|word|char][&format=json|lean|md][&classify=true][&pages=<spec>]
Content-Type: multipart/form-data   (field name: file)
```

Returns `200` with extraction JSON, or `text/plain; charset=utf-8` for lean.
Markdown uses `text/markdown; charset=utf-8`. Lean and Markdown with no
granularity imply `element`; either with `char` returns
`400 bad_format`. The endpoint is bounded for interactive use: **25 MB / 200
pages** by default (configurable). Oversized requests get `413` pointing you
to the jobs API.

```bash
curl -sf -F file=@report.pdf 'http://localhost:41619/v1/extract?granularity=element'
# PPTX and DOCX default to element granularity
curl -sf -F file=@deck.pptx 'http://localhost:41619/v1/extract?granularity=element'
curl -sf -F file=@report.docx 'http://localhost:41619/v1/extract'
curl -sf -F file=@report.pdf 'http://localhost:41619/v1/extract?format=md'
curl -sf -F file=@report.pdf 'http://localhost:41619/v1/extract?classify=true'
curl -sf -F file=@report.pdf 'http://localhost:41619/v1/extract?pages=1-200'
```

## Page selection

`pages=<spec>` restricts extraction to a sub-range of a PDF, on both
`POST /v1/extract` and `POST /v1/jobs`. The spec is 1-based:

- `pages=7` — a single page.
- `pages=1-200` — an inclusive range.

Page numbers are absolute over the whole document, not relative to the
selection: `pages=201-287` on a 287-page PDF emits pages numbered `#page
201` through `#page 287` (lean) / `page_number: 201..287` (JSON), not a
renumbered `1..87`. The response's page count — the JSON `pages` array
length, and the LEAN header's `pages=` value — always equals the number of
pages actually selected, not the document's total page count.

Any configured page cap (`too_many_pages`, `413`) is evaluated against the
**selected** page count, not the document total: a 287-page PDF with
`pages=1-200` succeeds against a 200-page cap because only 200 pages were
requested.

Omitting `pages` extracts the whole document, unchanged from prior
behavior. `pages` is PDF-only; sending it with a PPTX or DOCX/DOCM upload
returns `400 page_selection_unsupported`.

## Async jobs

For large documents (default cap 1 GiB):

```text
POST /v1/jobs[?granularity=…][&format=json|lean|md][&classify=true][&pages=<spec>] → 202 {"job_id": "…"}
GET  /v1/jobs/{id}                → {"job_id", "status", "error"}
GET  /v1/jobs/{id}/result         → 200 stored JSON, lean, or Markdown bytes
```

`status` walks `queued → running → succeeded | failed`. The result endpoint
returns `404` with code `not_ready` until the job succeeds and `not_found`
for unknown ids. Jobs and results are retained for 24 h (configurable), then
swept. The requested format, classification option, and page selection are
persisted on the job, and the result endpoint uses them to return the
corresponding JSON, lean, or Markdown content type against the same page
range that was requested at submit time. Job
state is instance-local — see
[architecture & guarantees](architecture.md).

## Error code map

| HTTP | Code | Meaning |
|---:|---|---|
| 400 | `bad_granularity` | invalid `granularity` value |
| 400 | `bad_format` | invalid `format`, or lean/Markdown combined with `char` |
| 400 | `granularity_unavailable` | the requested granularity is finer than this source provides |
| 400 | `bad_multipart` / `missing_file` | malformed upload |
| 400 | `bad_pages` | `pages` value is unparseable, reversed (start > end), zero, or negative |
| 400 | `page_out_of_range` | `pages` range extends beyond the document's last page |
| 400 | `page_selection_unsupported` | `pages` was given for a non-PDF format (PPTX, DOCX, DOCM) |
| 413 | `too_large` / `too_many_pages` | over sync caps — use jobs; `too_many_pages` is evaluated against the selected page count when `pages` is set |
| 415 | `unsupported_format` | not supported PDF/PPTX/DOCX/DOCM, or legacy/encrypted Office |
| 422 | `encrypted_pdf` / `parse_failure` | unprocessable document |
| 500 | `crash` | worker died (hostile/malformed input — contained) |
| 500 | `output_too_large` | extraction JSON exceeded the output cap |
| 500 | `store_error` / `io_error` | server-side storage trouble |
| 504 | `timeout` | extraction exceeded the wall-clock limit |

## Health

```text
GET /healthz    → 200 {"status": "ok"}
```

## Concurrency behavior

Sync extractions are bounded by a semaphore sized to the worker count —
excess requests queue rather than spawning unbounded subprocesses. The job
queue runs on its own bounded pool. Both pools spawn one isolated worker
subprocess per document.
