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

Invalid values fall back to the default rather than failing startup.

## Sizing guidance

Task/container memory should exceed
`DOCRAY_WORKERS × DOCRAY_MEM_LIMIT_BYTES` **plus headroom** for the server
process itself (e.g. 2 workers × 2 GiB + ~1 GiB ≈ 5 GiB). The per-worker
rlimit caps each extraction before container-level OOM would trigger.
