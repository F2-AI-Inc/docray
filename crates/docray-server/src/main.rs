mod config;
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
                    if let Err(e) = store.mark_failed(&id, "crash", "worker task panicked") {
                        eprintln!("worker: mark_failed after panic for {id} failed: {e}");
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
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(600)).await;
                match store.sweep_expired(cfg.result_ttl_secs) {
                    Ok(n) if n > 0 => println!("swept {n} expired jobs"),
                    Ok(_) => {}
                    Err(e) => eprintln!("sweeper: sweep_expired failed: {e}"),
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

/// Run one claimed job to completion and record its outcome. Store errors while
/// marking the result are logged (worst case the job is re-queued by startup
/// running->queued recovery); they must not abort the worker loop.
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
            let result_path = cfg
                .data_dir
                .join("results")
                .join(format!("{id}.{extension}"));
            match std::fs::write(&result_path, &bytes) {
                Ok(()) => store.mark_succeeded(id, result_path.to_str().unwrap()),
                Err(e) => store.mark_failed(id, "io_error", &e.to_string()),
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
    if let Err(e) = marked {
        eprintln!("worker: recording outcome for job {id} failed: {e}");
    }
}
