# Configuration

Everything is environment variables with sensible defaults. All limits exist
to keep hostile or pathological documents from taking the service down.

| Variable | Default | Purpose |
|---|---|---|
| `DOCRAY_PORT` | `41619` | HTTP listen port |
| `DOCRAY_CLI_PATH` | `docray` beside the server binary, else on `PATH` | Worker binary the server spawns per document |
| `DOCRAY_PDFIUM_DIR` | `./.pdfium/lib` | Directory of the PDFium dynamic library |
| `DOCRAY_DATA_DIR` | `./data` | Job uploads, results, and the SQLite job store |
| `DOCRAY_SYNC_MAX_BYTES` | `26214400` (25 MB) | Sync upload cap |
| `DOCRAY_SYNC_MAX_PAGES` | `200` | Sync page cap |
| `DOCRAY_JOBS_MAX_BYTES` | `1073741824` (1 GiB) | Jobs upload cap |
| `DOCRAY_TIMEOUT_SECS` | `300` | Wall-clock limit per extraction |
| `DOCRAY_OUTPUT_CAP_BYTES` | `536870912` (512 MB) | Max JSON a worker may produce |
| `DOCRAY_MEM_LIMIT_BYTES` | `2147483648` (2 GiB) | Per-worker memory rlimit (enforced on Linux) |
| `DOCRAY_WORKERS` | CPU cores (min 1) | Job worker pool size; also bounds concurrent sync extractions |
| `DOCRAY_RESULT_TTL_SECS` | `86400` (24 h) | How long finished jobs and results are kept |
| `DOCRAY_TELEMETRY_LOGS` | `off` | `json` emits a bounded extraction-completed event to stdout |
| `OTEL_METRICS_EXPORTER` | `none` | `otlp` enables OpenTelemetry metrics over OTLP/HTTP protobuf |

## Telemetry

Structured events and OTLP metrics are independently opt-in. A typical
collector configuration is:

```bash
DOCRAY_TELEMETRY_LOGS=json \
OTEL_METRICS_EXPORTER=otlp \
OTEL_EXPORTER_OTLP_ENDPOINT=http://otel-collector:4318 \
OTEL_SERVICE_NAME=docray-server \
OTEL_RESOURCE_ATTRIBUTES=deployment.environment.name=production \
docray-server
```

The exporter honors standard OpenTelemetry endpoint, metric-specific
endpoint, header, timeout, export interval, service-name, and resource
attribute variables. Metrics cover request outcomes, latency, queueing,
concurrency, and byte sizes. LEAN responses also report pages, records,
warnings, and textless pages. JSON and Markdown responses are not reparsed for
telemetry, avoiding an additional allocation proportional to a potentially
large response.

| Metric | Instrument | Unit |
|---|---|---|
| `docray.extraction.requests` | Counter | `{request}` |
| `docray.extraction.errors` | Counter | `{error}` |
| `docray.extraction.timeouts` | Counter | `{timeout}` |
| `docray.extraction.request.duration` | Histogram | `s` |
| `docray.extraction.queue.duration` | Histogram | `s` |
| `docray.extraction.worker.duration` | Histogram | `s` |
| `docray.extraction.active` | Up-down counter | `{request}` |
| `docray.extraction.input.size` | Histogram | `By` |
| `docray.extraction.output.size` | Histogram | `By` |
| `docray.extraction.pages` | Counter | `{page}` |
| `docray.extraction.records` | Counter | `{record}` |
| `docray.extraction.warnings` | Counter | `{warning}` |
| `docray.extraction.textless_pages` | Counter | `{page}` |

Only bounded operational attributes are emitted. Filenames, job IDs, source
hashes, document text, tenant identifiers, and extracted content are never
telemetry fields. Requests rejected by the route's body-limit layer before
the extraction handler must be counted at the proxy or load balancer.

Invalid numeric limit values fall back to their defaults. Unknown telemetry
modes fail startup so an explicitly requested exporter is never silently
disabled.

## Sizing guidance

Task/container memory should exceed
`DOCRAY_WORKERS × DOCRAY_MEM_LIMIT_BYTES` **plus headroom** for the server
process itself (e.g. 2 workers × 2 GiB + ~1 GiB ≈ 5 GiB). The per-worker
rlimit caps each extraction before container-level OOM would trigger.
