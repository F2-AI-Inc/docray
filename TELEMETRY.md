# Docray telemetry

`docray-server` exposes vendor-neutral operational telemetry for both synchronous
extractions (`POST /v1/extract`) and asynchronous jobs. Telemetry is disabled by
default. Operators can enable OpenTelemetry metrics, one-line structured JSON
events, or both without changing extraction responses.

## Choose an output

| Need | Configuration | Notes |
|---|---|---|
| Metrics through an OpenTelemetry Collector | `OTEL_METRICS_EXPORTER=otlp` | OTLP over HTTP/protobuf; recommended for production |
| One event per completed extraction in the existing log pipeline | `DOCRAY_TELEMETRY_LOGS=json` | JSON is written to stdout |
| Both | Set both variables | Useful when metrics drive dashboards and logs support individual-event investigation |
| Neither | Defaults (`none` and `off`) | No telemetry exporter and no telemetry network traffic |

An invalid, explicitly configured telemetry mode fails server startup instead
of silently disabling the requested output.

## OpenTelemetry setup

Point Docray at an OTLP-capable collector rather than installing a
vendor-specific SDK in the service:

```bash
OTEL_METRICS_EXPORTER=otlp \
OTEL_EXPORTER_OTLP_ENDPOINT=http://otel-collector:4318 \
OTEL_SERVICE_NAME=docray-server \
OTEL_RESOURCE_ATTRIBUTES=deployment.environment.name=production \
docray-server
```

The exporter uses OTLP/HTTP protobuf and honors these standard variables:

| Variable | Purpose |
|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Base OTLP endpoint; the metrics path is derived by the exporter |
| `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` | Metrics-specific endpoint; takes precedence over the base endpoint |
| `OTEL_EXPORTER_OTLP_HEADERS` | Headers shared by OTLP exporters, such as collector authentication |
| `OTEL_EXPORTER_OTLP_METRICS_HEADERS` | Metrics-specific headers |
| `OTEL_EXPORTER_OTLP_TIMEOUT` | Shared export timeout |
| `OTEL_EXPORTER_OTLP_METRICS_TIMEOUT` | Metrics-specific export timeout |
| `OTEL_METRIC_EXPORT_INTERVAL` | Periodic metric export interval |
| `OTEL_SERVICE_NAME` | Service resource name; defaults to `docray-server` |
| `OTEL_RESOURCE_ATTRIBUTES` | Deployment-level dimensions, such as environment, region, or cluster |

The resource also includes `service.version`, populated from the Docray server
package version. An OpenTelemetry Collector can route the same metrics to one
or several systems, such as Prometheus/Grafana, CloudWatch, Datadog, Honeycomb,
or New Relic, without adding those vendors to Docray.

## Metric catalog

All duration metric values are seconds. Byte histograms use the OpenTelemetry
unit `By`.

| Metric | Type | Unit | Recorded when |
|---|---|---|---|
| `docray.extraction.requests` | Counter | `{request}` | An extraction attempt completes, including validation errors handled by the sync endpoint |
| `docray.extraction.errors` | Counter | `{error}` | An attempt has any non-success outcome, including timeouts and crashes |
| `docray.extraction.timeouts` | Counter | `{timeout}` | The service terminates an extraction at `DOCRAY_TIMEOUT_SECS` |
| `docray.extraction.request.duration` | Histogram | `s` | End-to-end sync handling, or claimed-job processing, completes |
| `docray.extraction.queue.duration` | Histogram | `s` | A sync request acquires an extraction slot |
| `docray.extraction.worker.duration` | Histogram | `s` | The child extraction process completes or is terminated |
| `docray.extraction.active` | Up-down counter | `{request}` | A worker extraction starts or stops |
| `docray.extraction.input.size` | Histogram | `By` | An input upload or queued job file has a measurable size |
| `docray.extraction.output.size` | Histogram | `By` | An extraction succeeds |
| `docray.extraction.pages` | Counter | `{page}` | A successful LEAN response can be summarized |
| `docray.extraction.records` | Counter | `{record}` | A successful LEAN response emits counted records; split by `docray.record.type` |
| `docray.extraction.warnings` | Counter | `{warning}` | A successful LEAN response contains warnings |
| `docray.extraction.textless_pages` | Counter | `{page}` | A successful LEAN response contains a page without readable text payloads |

`request.duration` has slightly different boundaries by route. For `sync`, it
starts when the HTTP handler begins and includes request validation, upload
handling, queueing, extraction, and response construction. For `job`, it starts
after a queued job has been claimed and therefore excludes time spent waiting in
the persistent job queue. `queue.duration` is currently sync-only.

Content-quality counters are currently LEAN-only. JSON and Markdown outputs are
not reparsed for telemetry because doing so would require another allocation
proportional to the response size; their request, outcome, duration, and byte-size
metrics are still emitted.

## Metric attributes

The following bounded attributes can be used safely for aggregation:

| Attribute | Example values | Applied to |
|---|---|---|
| `docray.route` | `sync`, `job` | All metrics |
| `docray.format` | `lean`, `json`, `markdown`, `unknown` | Completed extraction metrics |
| `docray.granularity` | `element`, `word`, `char`, `default`, `unknown` | Completed extraction metrics |
| `docray.classify` | `true`, `false` | Completed extraction metrics |
| `docray.outcome` | `success`, `error`, `timeout`, `crash` | Completed extraction metrics |
| `error.type` | Stable service error code | Failed extraction metrics |
| `http.response.status_code` | HTTP status integer | Sync requests that reach the handler |
| `docray.schema.version` | LEAN schema version | Successfully summarized LEAN output |
| `docray.record.type` | `text`, `image`, `path`, `chart`, `table`, `table_cell`, `annotation`, `word` | `docray.extraction.records` only |

Use OpenTelemetry resource attributes—not metric attributes—for deployment
identity such as environment, region, cluster, or task family. This keeps metric
cardinality predictable while still allowing the collector or backend to split
production from demo and development.

## Structured JSON event

Set `DOCRAY_TELEMETRY_LOGS=json` to write one
`docray.extraction.completed` object to stdout for every recorded extraction.
The event schema is versioned independently from the Docray output schema:

```json
{
  "event": "docray.extraction.completed",
  "event_schema_version": 1,
  "timestamp_unix_ms": 1788911397953,
  "service_name": "docray-server",
  "service_version": "0.5.0",
  "route": "sync",
  "format": "lean",
  "granularity": "element",
  "classify": false,
  "outcome": "success",
  "status_code": 200,
  "request_duration_ms": 368.66,
  "queue_duration_ms": 0.002,
  "extraction_duration_ms": 368.30,
  "input_bytes": 725,
  "output_bytes": 278,
  "in_flight": 1,
  "page_count": 1,
  "text_record_count": 2,
  "textless_page_count": 0,
  "warning_count": 0,
  "schema_version": "1.9"
}
```

Fields that are not available for a particular route, outcome, or output format
are omitted. LEAN events can additionally include counts for image, path, chart,
table, table-cell, annotation, and word records. Failed events include
`error_code` when a stable code is available.

## Privacy and cardinality contract

Telemetry is intentionally operational and content-free. Docray does not emit:

- filenames or filesystem paths,
- job IDs or source hashes,
- document text or extracted content,
- tenant, user, or customer identifiers,
- raw worker error messages.

The service emits counts, byte sizes, timings, controlled modes, stable outcome
codes, and version identifiers. If a deployment needs tenant-level allocation,
join or enrich metrics outside Docray at a trusted boundary rather than adding
tenant IDs as high-cardinality metric labels.

## Recommended dashboards and alerts

Start with four views:

1. **Reliability:** request rate, success rate, errors by `error.type`, timeout
   rate, and crashes by route.
2. **Latency:** p50/p95/p99 `request.duration`, `queue.duration`, and
   `worker.duration`, split by route and output settings.
3. **Capacity:** `active`, queue latency, input size, output size, and timeout
   rate. Rising queue latency with high active concurrency is the clearest
   saturation signal.
4. **Extraction quality:** warnings per page, textless-page ratio, and record mix
   for LEAN output. These are screening signals, not document correctness scores.

Useful initial alert candidates are a sustained success-rate drop, any sustained
crash rate, timeout-rate growth, and p95 queue latency above the caller's latency
budget. Establish normal baselines before choosing production thresholds.

## Known coverage gaps

- Requests rejected by Axum's route-level body-size limit never enter the
  extraction handler and are absent from application telemetry. Retain 413 counts
  from the load balancer, reverse proxy, or access logs for complete ingress
  accounting.
- Async job queue age is not currently measured. The job route's
  `request.duration` begins only after the job is claimed.
- Job persistence failures that happen after successful extraction are logged by
  the job system but are not represented as extraction errors.
- Telemetry currently exports metrics and structured events, not distributed
  traces. Trace context propagation and spans can be added later without changing
  this metric contract.

## Verification

For a local JSON smoke test:

```bash
DOCRAY_TELEMETRY_LOGS=json docray-server
curl -sS -F file=@testdata/simple.pdf \
  'http://localhost:41619/v1/extract?format=lean' >/dev/null
```

The server should write exactly one `docray.extraction.completed` JSON line for
the request. Check that the line has operational fields only and contains no
document content. For OTLP, point the exporter at a development collector with a
debug exporter and verify that all expected metric names and resource attributes
arrive before enabling a production destination.
