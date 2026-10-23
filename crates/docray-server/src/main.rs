mod config;
mod disk;
mod http;
mod jobs;
mod telemetry;
mod worker;

use config::Config;
use futures::FutureExt;
use http::AppState;
use jobs::{ClaimedJob, JobStore};
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::Arc;
use telemetry::{ExtractionMetric, Telemetry};
use worker::{run_extraction, WorkerOutcome};

#[tokio::main]
async fn main() {
    // `docray-server --healthcheck` probes a running server on this host and
    // exits 0/1. It exists so the container image needs neither a shell nor
    // curl: Docker HEALTHCHECK and ECS `CMD` health checks exec this binary.
    if std::env::args().nth(1).as_deref() == Some("--healthcheck") {
        let port = Config::from_env().port;
        std::process::exit(if healthcheck(port) { 0 } else { 1 });
    }

    let cfg = Arc::new(Config::from_env());
    let telemetry = Telemetry::from_env().unwrap_or_else(|error| {
        eprintln!("cannot initialize telemetry: {error}");
        std::process::exit(1);
    });
    std::fs::create_dir_all(cfg.data_dir.join("uploads")).expect("cannot create data dir");
    std::fs::create_dir_all(cfg.data_dir.join("results")).expect("cannot create data dir");
    let store = Arc::new(JobStore::new(&cfg.data_dir.join("jobs.sqlite")));

    // Job workers.
    for _ in 0..cfg.workers {
        let cfg = cfg.clone();
        let store = store.clone();
        let telemetry = telemetry.clone();
        tokio::spawn(async move {
            // The worker loop must never exit: a claim error backs off (no tight
            // error spin) and a panic in the per-job work is caught so the job is
            // marked failed rather than stranded 'running' (which, with 1 worker,
            // would starve the queue forever).
            loop {
                let ClaimedJob {
                    id,
                    input_path,
                    granularity,
                    format,
                    classify,
                    pages,
                } = match store.claim_next() {
                    Ok(Some(job)) => job,
                    Ok(None) => {
                        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                        continue;
                    }
                    Err(e) => {
                        eprintln!("worker: claim_next failed: {e}");
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        continue;
                    }
                };
                let work = AssertUnwindSafe(process_job(
                    &cfg,
                    &store,
                    &id,
                    &input_path,
                    granularity,
                    format,
                    classify,
                    pages,
                    &telemetry,
                ));
                if work.catch_unwind().await.is_err() {
                    match store.mark_failed(&id, "crash", "worker task panicked") {
                        Ok(_) => release_input(&input_path),
                        Err(e) => eprintln!("worker: mark_failed after panic for {id} failed: {e}"),
                    }
                }
            }
        });
    }

    // TTL sweeper.
    {
        let cfg = cfg.clone();
        let store = store.clone();
        tokio::spawn(async move {
            // A running job is never expired before its extraction could have
            // timed out, and an unreferenced upload is never swept while it
            // could still be arriving.
            let min_running_secs = cfg.timeout_secs + 60;
            let min_orphan_age_secs = cfg.result_ttl_secs.max(cfg.upload_timeout_secs + 60);
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(600)).await;
                match store.expire_stale(cfg.result_ttl_secs, min_running_secs) {
                    Ok(n) if n > 0 => println!("expired {n} stuck jobs"),
                    Ok(_) => {}
                    Err(e) => eprintln!("sweeper: expire_stale failed: {e}"),
                }
                match store.sweep_expired(cfg.result_ttl_secs) {
                    Ok(n) if n > 0 => println!("swept {n} expired jobs"),
                    Ok(_) => {}
                    Err(e) => eprintln!("sweeper: sweep_expired failed: {e}"),
                }
                match store.sweep_orphan_uploads(&cfg.data_dir.join("uploads"), min_orphan_age_secs)
                {
                    Ok(n) if n > 0 => println!("swept {n} orphaned uploads"),
                    Ok(_) => {}
                    Err(e) => eprintln!("sweeper: sweep_orphan_uploads failed: {e}"),
                }
            }
        });
    }

    let state = AppState::new(cfg.clone(), store, telemetry.clone());
    let app = http::router(state);
    let addr = format!("0.0.0.0:{}", cfg.port);
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot listen on {addr}: {e}");
            eprintln!("another process may be using the port - set DOCRAY_PORT to change it");
            std::process::exit(1);
        }
    };
    println!("docray-server listening on http://localhost:{}", cfg.port);
    println!(
        "playground UI:        http://localhost:{}/playground",
        cfg.port
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");
    if let Err(error) = telemetry.shutdown() {
        eprintln!("cannot flush telemetry during shutdown: {error}");
    }
}

/// GET `/healthz` on 127.0.0.1:`port` over plain std networking and report
/// whether the server answered 200. Deliberately dependency-free (no reqwest,
/// no tokio) so the probe stays a few hundred KiB of already-loaded code and
/// cannot hang: every socket operation is bounded by a 3s timeout.
fn healthcheck(port: u16) -> bool {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;

    let timeout = Duration::from_secs(3);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, timeout) else {
        return false;
    };
    if stream.set_read_timeout(Some(timeout)).is_err()
        || stream.set_write_timeout(Some(timeout)).is_err()
    {
        return false;
    }
    if stream
        .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }
    // `Connection: close` makes the server end the stream after one response,
    // so read_to_end terminates; the status line is all we need.
    let mut response = Vec::new();
    let _ = stream.take(4096).read_to_end(&mut response);
    response.starts_with(b"HTTP/1.1 200 ")
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("cannot install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("cannot install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

/// Delete a job's upload once its outcome is recorded: the input is never read
/// again, and holding it for the result TTL would let finished jobs pin disk.
fn release_input(input_path: &str) {
    if !jobs::remove_ok(input_path) {
        eprintln!("worker: cannot delete upload {input_path}; the TTL sweep retries");
    }
}

/// Run one claimed job to completion and record its outcome. Store errors while
/// marking the result are logged (worst case the job is re-queued by startup
/// running->queued recovery, so its upload is kept); they must not abort the
/// worker loop.
#[allow(clippy::too_many_arguments)]
async fn process_job(
    cfg: &Config,
    store: &JobStore,
    id: &str,
    input_path: &str,
    granularity: Option<docray_model::Granularity>,
    format: docray_model::OutputFormat,
    classify: bool,
    pages: Option<docray_core::PageSelection>,
    telemetry: &Telemetry,
) {
    let started = std::time::Instant::now();
    let input_bytes = std::fs::metadata(input_path).map(|meta| meta.len()).ok();
    let active = telemetry.begin_extraction("job");
    let outcome = run_extraction(
        cfg,
        Path::new(input_path),
        None,
        granularity,
        format,
        classify,
        pages,
    )
    .await;
    let mut metric = ExtractionMetric::new("job");
    metric.format = format.as_str();
    metric.granularity = granularity.map(|value| value.as_str()).unwrap_or("default");
    metric.classify = classify;
    metric.input_bytes = input_bytes;
    metric.extraction_duration = Some(started.elapsed());
    metric.request_duration = started.elapsed();
    metric.in_flight = Some(active.current());
    metric.observe_outcome(&outcome, format);
    telemetry.record(&metric);
    drop(active);
    let marked = match outcome {
        WorkerOutcome::Success(bytes) => {
            let extension = match format {
                docray_model::OutputFormat::Json => "json",
                docray_model::OutputFormat::Lean => "lean.txt",
                docray_model::OutputFormat::Markdown => "md",
            };
            let results_dir = cfg.data_dir.join("results");
            let result_path = results_dir.join(format!("{id}.{extension}"));
            match disk::ensure_room(&results_dir, cfg.min_free_bytes, bytes.len() as u64) {
                Err(disk::RoomError::Full) => store.mark_failed(
                    id,
                    "insufficient_storage",
                    "data volume is below its free-space floor; result not stored",
                ),
                Err(disk::RoomError::Io(e)) => {
                    store.mark_failed(id, "io_error", &format!("cannot determine free space: {e}"))
                }
                Ok(()) => match std::fs::write(&result_path, &bytes) {
                    Ok(()) => {
                        let marked = store.mark_succeeded(id, result_path.to_str().unwrap());
                        if let Ok(false) = marked {
                            // Expired by the sweeper meanwhile: nothing
                            // references this result.
                            let _ = std::fs::remove_file(&result_path);
                        }
                        marked
                    }
                    Err(e) => {
                        let _ = std::fs::remove_file(&result_path);
                        store.mark_failed(id, "io_error", &e.to_string())
                    }
                },
            }
        }
        WorkerOutcome::Failed { code, message } => store.mark_failed(id, &code, &message),
        WorkerOutcome::Timeout => store.mark_failed(id, "timeout", "extraction timed out"),
        WorkerOutcome::Crashed => store.mark_failed(
            id,
            "crash",
            "worker crashed (signal; possibly memory limit)",
        ),
        WorkerOutcome::OutputTooLarge => {
            store.mark_failed(id, "output_too_large", "output exceeded cap")
        }
    };
    match marked {
        Ok(_) => release_input(input_path),
        Err(e) => eprintln!("worker: recording outcome for job {id} failed: {e}"),
    }
}
