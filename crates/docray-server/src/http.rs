use crate::config::Config;
use crate::jobs::JobStore;
use crate::worker::{run_extraction, WorkerOutcome};
use axum::extract::multipart::MultipartRejection;
use axum::extract::rejection::QueryRejection;
use axum::extract::{DefaultBodyLimit, Multipart, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use docray_model::{Granularity, OutputFormat};
use serde::Deserialize;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
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
}

impl AppState {
    pub fn new(cfg: Arc<Config>, jobs: Arc<JobStore>) -> AppState {
        let sync_slots = Arc::new(Semaphore::new(cfg.workers));
        AppState {
            cfg,
            jobs,
            sync_slots,
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

/// Interactive testing UI: upload a PDF, see each page beside its extracted
/// bounding boxes, and inspect the JSON. Single self-contained file embedded
/// at compile time; pdf.js and fonts load from CDNs, so the page (not the
/// API) needs outbound network access in the viewer's browser.
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
}

#[derive(Debug)]
struct OutputQueryError {
    code: &'static str,
    message: String,
}

fn requested_output(
    query: Result<Query<OutputQuery>, QueryRejection>,
) -> Result<(Option<Granularity>, OutputFormat), OutputQueryError> {
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
    match (format, granularity) {
        (OutputFormat::Lean, None) => Ok((Some(Granularity::Element), format)),
        (OutputFormat::Lean, Some(Granularity::Char)) => Err(OutputQueryError {
            code: "bad_format",
            message: "lean format requires element or word granularity".to_string(),
        }),
        _ => Ok((granularity, format)),
    }
}

pub async fn read_upload(multipart: &mut Multipart, max_bytes: u64) -> Result<Vec<u8>, Response> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| error_response(StatusCode::BAD_REQUEST, "bad_multipart", &e.to_string()))?
    {
        if field.name() == Some("file") {
            let bytes = field.bytes().await.map_err(|e| {
                error_response(StatusCode::PAYLOAD_TOO_LARGE, "too_large", &e.to_string())
            })?;
            if bytes.len() as u64 > max_bytes {
                return Err(error_response(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "too_large",
                    "request exceeds sync size cap; use POST /v1/jobs",
                ));
            }
            return Ok(bytes.to_vec());
        }
    }
    Err(error_response(
        StatusCode::BAD_REQUEST,
        "missing_file",
        "multipart field 'file' required",
    ))
}

fn content_type(format: OutputFormat) -> &'static str {
    match format {
        OutputFormat::Json => "application/json",
        OutputFormat::Lean => "text/plain; charset=utf-8",
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
    let (granularity, format) = match requested_output(query) {
        Ok(value) => value,
        Err(error) => return error_response(StatusCode::BAD_REQUEST, error.code, &error.message),
    };
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
            return if rej.status() == StatusCode::PAYLOAD_TOO_LARGE {
                error_response(StatusCode::PAYLOAD_TOO_LARGE, "too_large", &rej.body_text())
            } else {
                error_response(StatusCode::BAD_REQUEST, "bad_multipart", &rej.body_text())
            };
        }
    };
    let bytes = match read_upload(&mut multipart, state.cfg.sync_max_bytes).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let tmp = match tempfile::NamedTempFile::new() {
        Ok(t) => t,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "io_error",
                &e.to_string(),
            )
        }
    };
    if let Err(e) = std::fs::write(tmp.path(), &bytes) {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "io_error",
            &e.to_string(),
        );
    }
    // Bound concurrent sync extractions. We await the permit (bounded queueing)
    // rather than 503-ing on contention: a brief queue is preferable to shedding
    // load, and the request already has the client waiting synchronously. The
    // semaphore is never closed, so acquire() cannot error.
    let _permit = state
        .sync_slots
        .acquire()
        .await
        .expect("semaphore not closed");
    let outcome = run_extraction(
        &state.cfg,
        tmp.path(),
        Some(state.cfg.sync_max_pages),
        granularity,
        format,
    )
    .await;
    outcome_to_response(outcome, format)
}

/// Stream the multipart `file` field straight to `path`, chunk by chunk, so a
/// 1 GiB upload is never buffered whole in RAM. Bytes are counted against
/// `max_bytes` (413 too_large on breach). Read/write errors are surfaced as
/// io_error 500; the caller deletes any partial file on error.
async fn stream_upload_to_file(
    multipart: &mut Multipart,
    path: &Path,
    max_bytes: u64,
) -> Result<(), Response> {
    use std::io::Write;
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| error_response(StatusCode::BAD_REQUEST, "bad_multipart", &e.to_string()))?
    {
        if field.name() != Some("file") {
            continue;
        }
        let mut file = std::fs::File::create(path).map_err(|e| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "io_error",
                &e.to_string(),
            )
        })?;
        let mut written: u64 = 0;
        while let Some(chunk) = field.chunk().await.map_err(|e| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "io_error",
                &e.to_string(),
            )
        })? {
            written += chunk.len() as u64;
            if written > max_bytes {
                return Err(error_response(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "too_large",
                    "request exceeds job size cap",
                ));
            }
            file.write_all(&chunk).map_err(|e| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "io_error",
                    &e.to_string(),
                )
            })?;
        }
        file.flush().map_err(|e| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "io_error",
                &e.to_string(),
            )
        })?;
        return Ok(());
    }
    Err(error_response(
        StatusCode::BAD_REQUEST,
        "missing_file",
        "multipart field 'file' required",
    ))
}

async fn create_job(
    State(state): State<AppState>,
    query: Result<Query<OutputQuery>, QueryRejection>,
    multipart: Result<Multipart, MultipartRejection>,
) -> Response {
    let (granularity, format) = match requested_output(query) {
        Ok(value) => value,
        Err(error) => return error_response(StatusCode::BAD_REQUEST, error.code, &error.message),
    };
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

    let id = uuid::Uuid::new_v4().to_string();
    let input_path = state.cfg.data_dir.join("uploads").join(&id);
    if let Err(e) = std::fs::create_dir_all(input_path.parent().unwrap()) {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "io_error",
            &e.to_string(),
        );
    }

    // Jobs accept larger inputs than sync: cap by the jobs body limit, streaming
    // to disk. On any failure delete the partial file so we never leave orphans.
    if let Err(resp) =
        stream_upload_to_file(&mut multipart, &input_path, state.cfg.jobs_max_bytes).await
    {
        let _ = std::fs::remove_file(&input_path);
        return resp;
    }

    if let Err(e) = state
        .jobs
        .create(&id, input_path.to_str().unwrap(), granularity, format)
    {
        let _ = std::fs::remove_file(&input_path);
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "store_error",
            &e.to_string(),
        );
    }
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
                        if job.format == OutputFormat::Lean.as_str() {
                            content_type(OutputFormat::Lean)
                        } else {
                            content_type(OutputFormat::Json)
                        },
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
        })
    }

    #[test]
    fn lean_query_implies_element_and_rejects_char() {
        assert_eq!(
            requested_output(Ok(query(None, Some("lean")))).unwrap(),
            (Some(Granularity::Element), OutputFormat::Lean)
        );

        let error = requested_output(Ok(query(Some("char"), Some("lean")))).unwrap_err();
        assert_eq!(error.code, "bad_format");
    }

    #[test]
    fn invalid_format_query_has_stable_error_code() {
        let error = requested_output(Ok(query(None, Some("toon")))).unwrap_err();
        assert_eq!(error.code, "bad_format");
    }
}
