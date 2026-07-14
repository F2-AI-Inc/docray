# HTTP API

All endpoints accept/return JSON; every error — at any layer — uses the same
envelope:

```json
{"error": {"code": "…", "message": "…"}}
```

## Sync extraction

```text
POST /v1/extract[?granularity=element|word|char]
Content-Type: multipart/form-data   (field name: file)
```

Returns `200` with the extraction JSON. Bounded for interactive use:
**25 MB / 200 pages** by default (configurable). Oversized requests get
`413` pointing you to the jobs API.

```bash
curl -sf -F file=@report.pdf 'http://localhost:41619/v1/extract?granularity=element'
```

## Async jobs

For large documents (default cap 1 GiB):

```text
POST /v1/jobs[?granularity=…]     → 202 {"job_id": "…"}
GET  /v1/jobs/{id}                → {"job_id", "status", "error"}
GET  /v1/jobs/{id}/result         → 200 extraction JSON
```

`status` walks `queued → running → succeeded | failed`. The result endpoint
returns `404` with code `not_ready` until the job succeeds and `not_found`
for unknown ids. Jobs and results are retained for 24 h (configurable), then
swept. Job state is instance-local — see
[architecture & guarantees](architecture.md).

## Error code map

| HTTP | Code | Meaning |
|---:|---|---|
| 400 | `bad_granularity` | invalid `granularity` value |
| 400 | `bad_multipart` / `missing_file` | malformed upload |
| 413 | `too_large` / `too_many_pages` | over sync caps — use jobs |
| 415 | `unsupported_format` | not a PDF |
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
