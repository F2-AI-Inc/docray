# docray — Document Parsing Service

docray extracts structured, deterministic JSON (text with font/color/hierarchy,
images, vector paths, and annotations, all with page-space coordinates) from
documents — PDF in v1, with the extractor trait designed so new formats are
new crates rather than schema changes. It ships as a CLI (`docray`) for
one-shot extraction and a server (`docray-server`) exposing a sync endpoint for
small/fast documents and an async job queue for everything else. The full
JSON contract is documented in the API and granularity sections below.

## Prerequisites

- Rust stable (see [Docker](#docker) below for the exact version this repo
  is verified against — the toolchain's `rustup` default `stable` channel
  works for local development).
- Run all cargo commands from the workspace root.
- Fetch the pinned PDFium binary **before** building or testing anything
  that touches `docray-pdf`, `docray-cli`, or `docray-server`:

  ```bash
  ./scripts/fetch-pdfium.sh
  ```

  This downloads a pinned `bblanchon/pdfium-binaries` release into
  `.pdfium/lib`. `docray-pdf` looks for the library in this order: the
  `DOCRAY_PDFIUM_DIR` env var, then `./.pdfium/lib` (relative to the process's
  current directory), then the system library. The integration tests under
  `crates/docray-pdf/tests`, `crates/docray-cli/tests`, and
  `crates/docray-server/tests` **self-set `DOCRAY_PDFIUM_DIR`** to
  `<repo-root>/.pdfium/lib`, so a bare `cargo test` works as long as you ran
  `fetch-pdfium.sh` once — you do not need to export the variable yourself
  for local development.

## Quickstart

Build the `docray` CLI first, then run the server:

```bash
cargo build -p docray-cli        # or: cargo build --workspace
cargo run -p docray-server
```

The server spawns the `docray` CLI binary as a subprocess for every document
(see `DOCRAY_CLI_PATH` below), so that binary must already exist on disk —
`cargo run -p docray-server` builds only the server crate, not the CLI. If you
skip the build step every extraction fails with `io_error` because the
worker binary can't be found.

By default it listens on `0.0.0.0:41619` and stores job data under `./data`.

### API

```bash
# Health check
curl -sf http://localhost:41619/healthz
# {"status":"ok"}

# Synchronous extraction (caps: 25 MB / 200 pages by default; over-cap
# requests get 413 and should use the async jobs API instead)
curl -sf -F file=@testdata/simple.pdf http://localhost:41619/v1/extract

# Async: submit a job, returns 202 + job_id
curl -sf -F file=@testdata/simple.pdf http://localhost:41619/v1/jobs
# {"job_id":"..."}

# Poll job status
curl -sf http://localhost:41619/v1/jobs/<job_id>
# {"job_id":"...","status":"queued|running|succeeded|failed","error":null}

# Fetch the result once status is "succeeded" (404 until then)
curl -sf http://localhost:41619/v1/jobs/<job_id>/result
```

### CLI

```bash
cargo run -p docray-cli -- extract file.pdf --pretty
# or, once built:
docray extract file.pdf [--max-pages N] [--pretty] [--granularity element|word|char]
```

### Granularity

By default, docray emits the lossless char-level v1.1 response exactly as before.
Passing `--granularity element|word|char` to the CLI, or
`?granularity=element|word|char` to `POST /v1/extract` or `POST /v1/jobs`,
selects an explicit v1.2 response with a top-level `granularity` field. Jobs
persist the requested level and use it when their worker runs. Invalid query
values return `400 {"error":{"code":"bad_granularity",...}}`.

```bash
docray extract file.pdf --granularity word
curl -sf -F file=@testdata/simple.pdf \
  'http://localhost:41619/v1/extract?granularity=element'
```

`char` retains the complete lossless hierarchy. `word` and `element` are
LLM/RAG-oriented, one-decimal-point representations: their bbox is
`[x0,y0,x1,y1]`, and their words remain in extraction/content-stream order;
docray does not perform semantic reordering.

| Level | Four-file measured total | Estimated tokens* | Reduction vs current char |
|---|---:|---:|---:|
| char (default) | 22.01 MB | 6.29 M | — |
| word (W9) | 2.39 MB | 0.68 M | 89.15% |
| element (E8) | 1.57 MB | 0.45 M | 92.85% |

\*Estimated as bytes / 3.5 because `tiktoken` was unavailable during the
measurement run; use it for comparison, not a model-specific token count.

Word records are positional tuples `[text,x0,y0,x1,y1]` so text and its
click-to-source bbox stay adjacent:

```json
{
  "type": "text",
  "bbox": [72.0, 61.1, 134.0, 74.5],
  "words": [["Hello", 72.0, 61.1, 99.3, 74.5], ["World", 102.7, 61.1, 134.0, 74.5]],
  "font": {"name": "Helvetica", "size": 12.0}
}
```

Element output keeps one text string and its source bbox:

```json
{
  "type": "text",
  "bbox": [72.0, 61.1, 134.0, 74.5],
  "text": "Hello World",
  "font": {"name": "Helvetica", "size": 12.0}
}
```

At word and element levels, `id`, chars, line/baseline data, and non-reading
image/path reconstruction data are omitted. `font.name` and `font.size` are
always present for text; `bold` and `italic` are omitted when false. Text
`fill` is omitted when it is `[0,0,0]`; text `stroke` is omitted when it is
null or `[0,0,0]`. Empty `warnings` arrays are omitted, but every non-empty
warnings array is retained at every explicit granularity level.

Exit codes:

| Code | Meaning            |
|------|--------------------|
| 0    | ok                 |
| 2    | unsupported_format |
| 3    | encrypted_pdf      |
| 4    | parse_failure      |
| 5    | io_error           |
| 6    | too_many_pages     |

On failure, `docray` prints `{"error": {"code": ..., "message": ...}}` to
stderr and exits with the corresponding code above.

### Schema notes

An absent granularity parameter emits the byte-identical
`"schema_version":"1.1"` response. Every explicit granularity request emits
`"schema_version":"1.2"` plus its `granularity` discriminator. Each page includes a plain
boolean `scanned` field. A page is `scanned: true` when it has ZERO text
elements AND at least one image element whose bbox covers ≥ 85% of the page
area. This intentionally also flags pre-rendered (rasterized-slide) pages —
the flag means "no machine-readable text; a full-page raster carries the
content; OCR required", not strictly "came from a scanner". A scan with
an invisible OCR text layer has text elements and is therefore
`scanned: false`; this is correct because text is extractable.

## Config

`docray-server` is configured entirely via environment variables (see
`crates/docray-server/src/config.rs`):

| Variable               | Default                                | Meaning |
|-------------------------|-----------------------------------------|---------|
| `DOCRAY_PORT`              | `41619`                                  | HTTP listen port |
| `DOCRAY_CLI_PATH`          | `docray` next to the running binary, else `docray` on `PATH` | Path to the `docray` CLI binary the server spawns per document |
| `DOCRAY_PDFIUM_DIR`        | unset                                    | Directory containing the PDFium shared library (passed through to the spawned CLI) |
| `DOCRAY_DATA_DIR`          | `./data`                                | Root for uploads, job results, and the jobs SQLite database |
| `DOCRAY_SYNC_MAX_BYTES`    | `26214400` (25 MiB)                     | Max upload size accepted by `POST /v1/extract` |
| `DOCRAY_JOBS_MAX_BYTES`    | `1073741824` (1 GiB)                    | Max upload size accepted by `POST /v1/jobs` (enforced as both the route body limit and the streaming-to-disk byte cap) |
| `DOCRAY_SYNC_MAX_PAGES`    | `200`                                    | Max page count accepted by `POST /v1/extract` |
| `DOCRAY_TIMEOUT_SECS`      | `300`                                    | Wall-clock timeout per extraction before the worker is killed |
| `DOCRAY_OUTPUT_CAP_BYTES`  | `536870912` (512 MiB)                   | Max stdout size read from a worker before it's killed as `output_too_large` |
| `DOCRAY_MEM_LIMIT_BYTES`   | `2147483648` (2 GiB)                    | Per-worker memory rlimit (Linux only) |
| `DOCRAY_WORKERS`           | number of CPU cores (min 1)             | Size of the async job worker pool; also bounds concurrent `/v1/extract` extractions. The sync path and the job pool share this knob but keep independent concurrency counts |
| `DOCRAY_RESULT_TTL_SECS`   | `86400` (24 h)                          | Age at which succeeded/failed jobs and their results are swept |

## Known limitations (v1)

- **Scanned pages are detected, but not OCRed.** Raster-only pages are no
  longer silent: `scanned: true` identifies pages that require OCR, but
  docray does not itself recover text from the raster.
- **Corrupt-but-recoverable pages surface as empty pages without warnings.**
  When PDFium silently recovers from a damaged content stream (e.g. it drops
  unreadable operators but still returns a valid page object), docray emits that
  page with zero elements and no warning, because the recovery happens inside
  PDFium with no signal back to us. A page that fails to parse outright *does*
  get a `page N failed to parse` warning.
- **Shading objects (gradient fills) are skipped with a warning.** Each
  skipped object adds a `p{page}-e-skipped: unsupported object type ...`
  warning. (Form XObjects *are* recursively extracted with correct
  page-space coordinates.)
- **Job state is instance-local and ephemeral.** The jobs SQLite database and
  result files live on the task's local disk, so job IDs are only meaningful
  to the instance that created them and do not survive a task replacement.
  See the Deploy section for why v1 is single-task only.

## Testing

```bash
# Run the full workspace test suite (requires ./scripts/fetch-pdfium.sh to
# have been run at least once; see Prerequisites)
cargo test

# Update golden fixtures after an intentional output change — review the
# resulting diff in git before committing
UPDATE_GOLDEN=1 cargo test -p docray-pdf

# Regenerate the generated PDF corpus under testdata/ (rotation, CJK,
# ligatures, multi-column, etc.) — run after changing the generator, then
# regenerate goldens with UPDATE_GOLDEN=1 above
cargo run -p docray-pdf --example gen_fixtures
```

Also run `cargo clippy --all-targets -- -D warnings` before committing;
CI-equivalent gate for this repo is `cargo test --workspace && cargo clippy
--all-targets -- -D warnings`.

## Docker

The image is a two-stage build: `rust:1.88-slim` compiles `docray` and
`docray-server` (release), and `debian:bookworm-slim` runs them alongside the
fetched PDFium shared library. (`rust:1.79-slim`, the initial target, cannot
build this workspace's dependency graph — `clap_lex` requires the
`edition2024` Cargo feature, stabilized in 1.85, and `image`/`libloading`
require rustc 1.88; 1.88 is the lowest verified-working tag.)

```bash
docker build -t docray:dev .
docker run -d --rm -p 41619:41619 --name docray-smoke docray:dev
sleep 2
curl -sf http://localhost:41619/healthz
curl -sf -F file=@testdata/simple.pdf http://localhost:41619/v1/extract | head -c 300
docker stop docray-smoke
```

Expected: `healthz` returns `{"status":"ok"}`; `extract` returns JSON
starting with `{"schema_version":"1.1"`.

## Deploy

v1 targets a single ECS Fargate task (see
[`deploy/ecs-task-def.example.json`](deploy/ecs-task-def.example.json)):

1. Build and push the image to ECR:

   ```bash
   aws ecr get-login-password --region <region> | docker login --username AWS --password-stdin <account>.dkr.ecr.<region>.amazonaws.com
   docker build -t <account>.dkr.ecr.<region>.amazonaws.com/docray:latest .
   docker push <account>.dkr.ecr.<region>.amazonaws.com/docray:latest
   ```

2. Before registering, create the CloudWatch Logs group `/ecs/docray` and an
   ECS task execution role (`ecsTaskExecutionRole`, with the managed
   `AmazonECSTaskExecutionRolePolicy`); the task definition references both
   (`logConfiguration.options.awslogs-group` and `executionRoleArn`) and
   registration/launch fails without them.

3. Register the task definition (fill in `<account>`/`<region>`) and run it
   as an ECS service with a single task behind an ALB target group (health
   check `GET /healthz`) or plain service discovery — either way, route all
   traffic to that one task/service. Note the Fargate `cpu`/`memory` pair
   must be valid: the example uses `cpu: "1024"` (1 vCPU) with
   `memory: "5120"` (5 GiB); `cpu: "512"` does not permit 5 GiB.

4. Job state (SQLite + result files) lives on the task's local disk, so v1
   is **single-task only**: scaling the service to N tasks would split job
   state across N independent, inconsistent stores. Horizontal scaling
   requires swapping the job store for a shared backend (S3/DynamoDB behind
   the existing store trait) with no API changes — see "Job state" in the
   design spec — before running more than one task.

5. Task memory must exceed `DOCRAY_WORKERS` × `DOCRAY_MEM_LIMIT_BYTES` with
   headroom left over for the server process itself — e.g. 2 workers × 2 GiB
   + ~1 GiB headroom = 5 GiB, hence `memory: "5120"` in the example task
   definition. The per-worker `RLIMIT_AS` caps each extraction's address
   space before the task-level OOM killer would otherwise trigger, but that
   only holds if the task itself has enough memory above the workers'
   combined limit to avoid being killed first.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option. Unless you explicitly state
otherwise, any contribution intentionally submitted for inclusion in docray
by you, as defined in the Apache-2.0 license, shall be dual licensed as
above, without any additional terms or conditions.
