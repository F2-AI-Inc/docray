use crate::config::Config;
use crate::disk::{ensure_room, RoomError};
use crate::jobs::JobStore;
use crate::telemetry::{ExtractionMetric, Telemetry};
use crate::worker::{run_extraction, WorkerOutcome};
use axum::extract::multipart::MultipartRejection;
use axum::extract::rejection::QueryRejection;
use axum::extract::{DefaultBodyLimit, Multipart, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use docray_core::PageSelection;
use docray_model::{Granularity, OutputFormat};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub jobs: Arc<JobStore>,
    /// Bounds the number of concurrent *sync* extractions so `/v1/extract` can't
    /// spawn one unbounded subprocess per in-flight request. Sized by
    /// `cfg.workers` — the sync path and the async job pool share the machine but
    /// keep independent concurrency counts, which is acceptable for v1.
    pub sync_slots: Arc<Semaphore>,
    /// Job uploads being received. They hold disk before their row exists, so
    /// they count against `max_pending_jobs` alongside queued/running rows.
    pub uploads_in_flight: Arc<AtomicUsize>,
    pub telemetry: Telemetry,
}

impl AppState {
    pub fn new(cfg: Arc<Config>, jobs: Arc<JobStore>, telemetry: Telemetry) -> AppState {
        let sync_slots = Arc::new(Semaphore::new(cfg.workers));
        AppState {
            cfg,
            jobs,
            sync_slots,
            uploads_in_flight: Arc::new(AtomicUsize::new(0)),
            telemetry,
        }
    }
}

pub fn router(state: AppState) -> Router {
    let sync_body_limit = state.cfg.sync_max_bytes as usize + 1024 * 1024;
    let jobs_body_limit = state.cfg.jobs_max_bytes as usize;
    // DefaultBodyLimit is scoped per route by layering the individual MethodRouter
    // (`post(handler).layer(...)`) instead of the whole Router, so the sync route
    // keeps its small cap while the jobs route gets the (configurable) jobs cap.
    Router::new()
        .route("/healthz", get(healthz))
        .route("/playground", get(playground))
        .route(
            "/v1/extract",
            post(sync_extract).layer(DefaultBodyLimit::max(sync_body_limit)),
        )
        .route(
            "/v1/jobs",
            post(create_job).layer(DefaultBodyLimit::max(jobs_body_limit)),
        )
        .route("/v1/jobs/{id}", get(job_status))
        .route("/v1/jobs/{id}/result", get(job_result))
        .with_state(state)
}

async fn healthz() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

/// Interactive testing UI: upload PDF, PPTX, or DOCX and inspect the rendered
/// source beside extracted paged elements or flow blocks and JSON. Single
/// self-contained file embedded at compile time; pdf.js and fonts load from
/// CDNs, so the page (not the API) needs outbound network access in the
/// viewer's browser.
async fn playground() -> Response {
    (
        StatusCode::OK,
        [("content-type", "text/html; charset=utf-8")],
        include_str!("../assets/playground.html"),
    )
        .into_response()
}

fn error_response(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(serde_json::json!({ "error": { "code": code, "message": message } })),
    )
        .into_response()
}

#[derive(Deserialize)]
struct OutputQuery {
    granularity: Option<String>,
    format: Option<String>,
    #[serde(default)]
    classify: bool,
    pages: Option<String>,
}

#[derive(Debug)]
struct OutputQueryError {
    code: &'static str,
    message: String,
}

fn requested_output(
    query: Result<Query<OutputQuery>, QueryRejection>,
) -> Result<
    (
        Option<Granularity>,
        OutputFormat,
        bool,
        Option<PageSelection>,
    ),
    OutputQueryError,
> {
    let query = query.map_err(|error| OutputQueryError {
        code: "bad_granularity",
        message: error.to_string(),
    })?;
    let granularity = match query.0.granularity {
        Some(value) => {
            Granularity::from_str(&value)
                .map(Some)
                .map_err(|message| OutputQueryError {
                    code: "bad_granularity",
                    message,
                })?
        }
        None => None,
    };
    let format = match query.0.format {
        Some(value) => OutputFormat::from_str(&value).map_err(|message| OutputQueryError {
            code: "bad_format",
            message,
        })?,
        None => OutputFormat::Json,
    };
    if query.0.classify && format != OutputFormat::Json {
        return Err(OutputQueryError {
            code: "bad_format",
            message: "classify=true is available only with JSON output".into(),
        });
    }
    let pages = match query.0.pages {
        Some(value) => {
            Some(
                PageSelection::from_str(&value).map_err(|message| OutputQueryError {
                    code: "bad_pages",
                    message,
                })?,
            )
        }
        None => None,
    };
    match (format, granularity) {
        (OutputFormat::Lean | OutputFormat::Markdown, None) => {
            Ok((Some(Granularity::Element), format, false, pages))
        }
        (OutputFormat::Lean | OutputFormat::Markdown, Some(Granularity::Char)) => {
            Err(OutputQueryError {
                code: "bad_format",
                message: format!("{format} format requires element or word granularity"),
            })
        }
        _ => Ok((granularity, format, query.0.classify, pages)),
    }
}

async fn read_upload(
    multipart: &mut Multipart,
    max_bytes: u64,
) -> Result<Vec<u8>, (&'static str, Box<Response>)> {
    while let Some(field) = multipart.next_field().await.map_err(|e| {
        (
            "bad_multipart",
            Box::new(error_response(
                StatusCode::BAD_REQUEST,
                "bad_multipart",
                &e.to_string(),
            )),
        )
    })? {
        if field.name() == Some("file") {
            let bytes = field.bytes().await.map_err(|e| {
                (
                    "too_large",
                    Box::new(error_response(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "too_large",
                        &e.to_string(),
                    )),
                )
            })?;
            if bytes.len() as u64 > max_bytes {
                return Err((
                    "too_large",
                    Box::new(error_response(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "too_large",
                        "request exceeds sync size cap; use POST /v1/jobs",
                    )),
                ));
            }
            return Ok(bytes.to_vec());
        }
    }
    Err((
        "missing_file",
        Box::new(error_response(
            StatusCode::BAD_REQUEST,
            "missing_file",
            "multipart field 'file' required",
        )),
    ))
}

fn content_type(format: OutputFormat) -> &'static str {
    match format {
        OutputFormat::Json => "application/json",
        OutputFormat::Lean => "text/plain; charset=utf-8",
        OutputFormat::Markdown => "text/markdown; charset=utf-8",
    }
}

pub fn outcome_to_response(outcome: WorkerOutcome, format: OutputFormat) -> Response {
    match outcome {
        WorkerOutcome::Success(bytes) => (
            StatusCode::OK,
            [("content-type", content_type(format))],
            bytes,
        )
            .into_response(),
        WorkerOutcome::Failed { code, message } => {
            let status = match code.as_str() {
                "granularity_unavailable" => StatusCode::BAD_REQUEST,
                "page_out_of_range" => StatusCode::BAD_REQUEST,
                "page_selection_unsupported" => StatusCode::BAD_REQUEST,
                "unsupported_format" => StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "encrypted_pdf" | "parse_failure" => StatusCode::UNPROCESSABLE_ENTITY,
                "too_many_pages" => StatusCode::PAYLOAD_TOO_LARGE,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            error_response(status, &code, &message)
        }
        WorkerOutcome::Timeout => error_response(
            StatusCode::GATEWAY_TIMEOUT,
            "timeout",
            "extraction timed out",
        ),
        WorkerOutcome::Crashed => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "crash",
            "worker crashed (signal; possibly memory limit)",
        ),
        WorkerOutcome::OutputTooLarge => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "output_too_large",
            "output exceeded cap",
        ),
    }
}

async fn sync_extract(
    State(state): State<AppState>,
    query: Result<Query<OutputQuery>, QueryRejection>,
    multipart: Result<Multipart, MultipartRejection>,
) -> Response {
    let started = Instant::now();
    let mut metric = ExtractionMetric::new("sync");
    let (granularity, format, classify, pages) = match requested_output(query) {
        Ok(value) => value,
        Err(error) => {
            metric.fail(error.code);
            return finish_response(
                &state.telemetry,
                started,
                metric,
                error_response(StatusCode::BAD_REQUEST, error.code, &error.message),
            );
        }
    };
    metric.format = format.as_str();
    metric.granularity = granularity.map(|value| value.as_str()).unwrap_or("default");
    metric.classify = classify;
    // axum rejects a malformed multipart request (e.g. bad/missing boundary)
    // before the handler body runs, with a plaintext body. Taking the extractor
    // as a `Result` lets us re-map that rejection into the always-JSON error
    // envelope. `MultipartRejection` maps by its own status: length/limit
    // rejections -> 413 too_large, anything else (invalid boundary) -> 400.
    //
    // NOTE: this does NOT cover bodies larger than the `DefaultBodyLimit` layer
    // ceiling (sync_max_bytes + 1 MiB overhead). That limit is enforced by a
    // tower layer that short-circuits with axum's own plaintext 413 *before* any
    // handler/extractor runs, so it cannot be intercepted with the `Result`
    // extractor here. Those specific over-ceiling requests remain plaintext 413;
    // uploads between sync_max_bytes and the ceiling still get the JSON envelope
    // from `read_upload`. Per the brief we do not add middleware to rewrite it.
    let mut multipart = match multipart {
        Ok(m) => m,
        Err(rej) => {
            let (code, response) = if rej.status() == StatusCode::PAYLOAD_TOO_LARGE {
                (
                    "too_large",
                    error_response(StatusCode::PAYLOAD_TOO_LARGE, "too_large", &rej.body_text()),
                )
            } else {
                (
                    "bad_multipart",
                    error_response(StatusCode::BAD_REQUEST, "bad_multipart", &rej.body_text()),
                )
            };
            metric.fail(code);
            return finish_response(&state.telemetry, started, metric, response);
        }
    };
    let deadline = Duration::from_secs(state.cfg.upload_timeout_secs);
    let bytes = match tokio::time::timeout(
        deadline,
        read_upload(&mut multipart, state.cfg.sync_max_bytes),
    )
    .await
    {
        Ok(Ok(b)) => b,
        Ok(Err((code, resp))) => {
            metric.fail(code);
            return finish_response(&state.telemetry, started, metric, *resp);
        }
        Err(_) => {
            metric.fail("upload_timeout");
            return finish_response(&state.telemetry, started, metric, upload_timeout_response());
        }
    };
    metric.input_bytes = Some(bytes.len() as u64);
    let tmp = match tempfile::NamedTempFile::new() {
        Ok(t) => t,
        Err(e) => {
            metric.fail("io_error");
            return finish_response(
                &state.telemetry,
                started,
                metric,
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "io_error",
                    &e.to_string(),
                ),
            );
        }
    };
    if let Err(e) = std::fs::write(tmp.path(), &bytes) {
        metric.fail("io_error");
        return finish_response(
            &state.telemetry,
            started,
            metric,
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "io_error",
                &e.to_string(),
            ),
        );
    }
    // Bound concurrent sync extractions. We await the permit (bounded queueing)
    // rather than 503-ing on contention: a brief queue is preferable to shedding
    // load, and the request already has the client waiting synchronously. The
    // semaphore is never closed, so acquire() cannot error.
    let queued = Instant::now();
    let _permit = state
        .sync_slots
        .acquire()
        .await
        .expect("semaphore not closed");
    metric.queue_duration = Some(queued.elapsed());
    let active = state.telemetry.begin_extraction("sync");
    metric.in_flight = Some(active.current());
    let extraction_started = Instant::now();
    let outcome = run_extraction(
        &state.cfg,
        tmp.path(),
        Some(state.cfg.sync_max_pages),
        granularity,
        format,
        classify,
        pages,
    )
    .await;
    metric.extraction_duration = Some(extraction_started.elapsed());
    metric.observe_outcome(&outcome, format);
    let response = outcome_to_response(outcome, format);
    drop(active);
    finish_response(&state.telemetry, started, metric, response)
}

fn finish_response(
    telemetry: &Telemetry,
    started: Instant,
    mut metric: ExtractionMetric,
    response: Response,
) -> Response {
    metric.request_duration = started.elapsed();
    metric.status_code = Some(response.status().as_u16());
    telemetry.record(&metric);
    response
}

fn upload_timeout_response() -> Response {
    error_response(
        StatusCode::REQUEST_TIMEOUT,
        "upload_timeout",
        "upload was not received within the upload deadline",
    )
}

fn room_error_response(error: RoomError) -> Response {
    match error {
        RoomError::Full => error_response(
            StatusCode::INSUFFICIENT_STORAGE,
            "insufficient_storage",
            "data volume is below its free-space floor; retry later",
        ),
        RoomError::Io(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "io_error",
            &format!("cannot determine free space: {e}"),
        ),
    }
}

/// Free space is re-checked after every this many bytes written, bounding how
/// far one upload can overshoot the floor between checks.
const ROOM_CHECK_INTERVAL: u64 = 1024 * 1024;

/// Stream the multipart `file` field straight to `path`, chunk by chunk, so a
/// 1 GiB upload is never buffered whole in RAM. Bytes are counted against
/// `max_bytes` (413 too_large on breach), and the volume's free space is kept
/// above `min_free` (507 insufficient_storage). Read/write errors are surfaced
/// as io_error 500; the caller deletes any partial file on error.
async fn stream_upload_to_file(
    multipart: &mut Multipart,
    path: &Path,
    max_bytes: u64,
    min_free: u64,
) -> Result<(), Box<Response>> {
    use std::io::Write;
    while let Some(mut field) = multipart.next_field().await.map_err(|e| {
        Box::new(error_response(
            StatusCode::BAD_REQUEST,
            "bad_multipart",
            &e.to_string(),
        ))
    })? {
        if field.name() != Some("file") {
            continue;
        }
        let mut file = std::fs::File::create(path).map_err(|e| {
            Box::new(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "io_error",
                &e.to_string(),
            ))
        })?;
        let dir = path.parent().unwrap_or(Path::new("."));
        let mut written: u64 = 0;
        let mut next_room_check: u64 = 0;
        while let Some(chunk) = field.chunk().await.map_err(|e| {
            Box::new(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "io_error",
                &e.to_string(),
            ))
        })? {
            written += chunk.len() as u64;
            if written > max_bytes {
                return Err(Box::new(error_response(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "too_large",
                    "request exceeds job size cap",
                )));
            }
            if written >= next_room_check {
                ensure_room(dir, min_free, chunk.len() as u64)
                    .map_err(|e| Box::new(room_error_response(e)))?;
                next_room_check = written + ROOM_CHECK_INTERVAL;
            }
            file.write_all(&chunk).map_err(|e| {
                Box::new(error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "io_error",
                    &e.to_string(),
                ))
            })?;
        }
        file.flush().map_err(|e| {
            Box::new(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "io_error",
                &e.to_string(),
            ))
        })?;
        return Ok(());
    }
    Err(Box::new(error_response(
        StatusCode::BAD_REQUEST,
        "missing_file",
        "multipart field 'file' required",
    )))
}

/// Holds a slot against `max_pending_jobs` for an upload in progress and
/// owns its partial file. Dropping it on any exit path — including the handler
/// future being dropped when a client disconnects — releases the slot and
/// deletes the file unless `keep` was called.
struct UploadReservation {
    in_flight: Arc<AtomicUsize>,
    path: Option<PathBuf>,
}

impl UploadReservation {
    fn keep(mut self) {
        self.path = None;
    }
}

impl Drop for UploadReservation {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = std::fs::remove_file(path);
        }
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

async fn create_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    query: Result<Query<OutputQuery>, QueryRejection>,
    multipart: Result<Multipart, MultipartRejection>,
) -> Response {
    let (granularity, format, classify, pages) = match requested_output(query) {
        Ok(value) => value,
        Err(error) => return error_response(StatusCode::BAD_REQUEST, error.code, &error.message),
    };
    // `pages` is validated above (via `requested_output`, same as sync) so a
    // bad `pages=` value fails fast at submit time with `bad_pages`, before any
    // upload streaming or job-store write happens.
    // Same rejection-to-JSON mapping the sync route uses (see `sync_extract`):
    // length/limit rejections -> 413 too_large, anything else -> 400.
    let mut multipart = match multipart {
        Ok(m) => m,
        Err(rej) => {
            return if rej.status() == StatusCode::PAYLOAD_TOO_LARGE {
                error_response(StatusCode::PAYLOAD_TOO_LARGE, "too_large", &rej.body_text())
            } else {
                error_response(StatusCode::BAD_REQUEST, "bad_multipart", &rej.body_text())
            };
        }
    };

    // Admission happens before any body byte is read. The in-flight count is
    // taken first so concurrent submissions cannot all pass the check.
    let in_flight = state.uploads_in_flight.fetch_add(1, Ordering::SeqCst) + 1;
    let mut reservation = UploadReservation {
        in_flight: state.uploads_in_flight.clone(),
        path: None,
    };
    match state.jobs.count_pending() {
        Ok(pending) if pending + in_flight > state.cfg.max_pending_jobs => {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "queue_full",
                "too many pending jobs; retry later",
            );
        }
        Ok(_) => {}
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "store_error",
                &e.to_string(),
            );
        }
    }

    let id = uuid::Uuid::new_v4().to_string();
    let uploads_dir = state.cfg.data_dir.join("uploads");
    if let Err(e) = std::fs::create_dir_all(&uploads_dir) {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "io_error",
            &e.to_string(),
        );
    }
    // A declared length is checked up front; the streaming checks below
    // cover chunked bodies and a length that under-declares.
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    if let Err(e) = ensure_room(&uploads_dir, state.cfg.min_free_bytes, declared) {
        return room_error_response(e);
    }

    // Jobs accept larger inputs than sync: cap by the jobs body limit, streaming
    // to disk under the upload deadline. The reservation deletes the partial
    // file on every failure path so we never leave orphans.
    let input_path = uploads_dir.join(&id);
    reservation.path = Some(input_path.clone());
    let deadline = Duration::from_secs(state.cfg.upload_timeout_secs);
    match tokio::time::timeout(
        deadline,
        stream_upload_to_file(
            &mut multipart,
            &input_path,
            state.cfg.jobs_max_bytes,
            state.cfg.min_free_bytes,
        ),
    )
    .await
    {
        Ok(Ok(())) => {}
        Ok(Err(resp)) => return *resp,
        Err(_) => return upload_timeout_response(),
    }

    if let Err(e) = state.jobs.create(
        &id,
        input_path.to_str().unwrap(),
        granularity,
        format,
        classify,
        pages,
    ) {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "store_error",
            &e.to_string(),
        );
    }
    // The row now counts the job as pending and owns the upload.
    reservation.keep();
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "job_id": id })),
    )
        .into_response()
}

async fn job_status(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    match state.jobs.get(&id) {
        Ok(None) => error_response(StatusCode::NOT_FOUND, "not_found", "no such job"),
        Ok(Some(job)) => {
            let error = job.error_code.as_ref().map(|c| {
                serde_json::json!({ "code": c, "message": job.error_message.clone().unwrap_or_default() })
            });
            Json(serde_json::json!({
                "job_id": job.id, "status": job.status,
                "error": error,
            }))
            .into_response()
        }
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "store_error",
            &e.to_string(),
        ),
    }
}

async fn job_result(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    match state.jobs.get(&id) {
        Ok(None) => error_response(StatusCode::NOT_FOUND, "not_found", "no such job"),
        Ok(Some(job)) if job.status == "succeeded" => {
            match std::fs::read(job.result_path.as_deref().unwrap_or("")) {
                Ok(bytes) => (
                    StatusCode::OK,
                    [(
                        "content-type",
                        OutputFormat::from_str(&job.format)
                            .map(content_type)
                            .unwrap_or_else(|_| content_type(OutputFormat::Json)),
                    )],
                    bytes,
                )
                    .into_response(),
                Err(e) => error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "io_error",
                    &e.to_string(),
                ),
            }
        }
        Ok(Some(_)) => error_response(StatusCode::NOT_FOUND, "not_ready", "job has no result"),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "store_error",
            &e.to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(granularity: Option<&str>, format: Option<&str>) -> Query<OutputQuery> {
        Query(OutputQuery {
            granularity: granularity.map(str::to_string),
            format: format.map(str::to_string),
            classify: false,
            pages: None,
        })
    }

    #[test]
    fn lean_query_implies_element_and_rejects_char() {
        assert_eq!(
            requested_output(Ok(query(None, Some("lean")))).unwrap(),
            (Some(Granularity::Element), OutputFormat::Lean, false, None)
        );

        let error = requested_output(Ok(query(Some("char"), Some("lean")))).unwrap_err();
        assert_eq!(error.code, "bad_format");

        assert_eq!(
            requested_output(Ok(query(None, Some("md")))).unwrap(),
            (
                Some(Granularity::Element),
                OutputFormat::Markdown,
                false,
                None
            )
        );
        let error = requested_output(Ok(query(Some("char"), Some("md")))).unwrap_err();
        assert_eq!(error.code, "bad_format");
    }

    #[test]
    fn pages_query_parses_and_rejects_bad_selection() {
        let mut q = query(None, None);
        q.0.pages = Some("2-3".to_string());
        assert_eq!(
            requested_output(Ok(q)).unwrap(),
            (
                None,
                OutputFormat::Json,
                false,
                Some(PageSelection { start: 2, end: 3 })
            )
        );

        let mut q = query(None, None);
        q.0.pages = Some("abc".to_string());
        let error = requested_output(Ok(q)).unwrap_err();
        assert_eq!(error.code, "bad_pages");

        let mut q = query(None, None);
        q.0.pages = Some("200-1".to_string());
        let error = requested_output(Ok(q)).unwrap_err();
        assert_eq!(error.code, "bad_pages");
    }

    #[test]
    fn invalid_format_query_has_stable_error_code() {
        let error = requested_output(Ok(query(None, Some("toon")))).unwrap_err();
        assert_eq!(error.code, "bad_format");
    }

    #[test]
    fn unavailable_granularity_is_a_bad_request() {
        let response = outcome_to_response(
            WorkerOutcome::Failed {
                code: "granularity_unavailable".into(),
                message: "requested word granularity is unavailable".into(),
            },
            OutputFormat::Json,
        );
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
