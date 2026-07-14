use crate::config::Config;
use docray_model::Granularity;
use serde_json::Value;
use std::path::Path;
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

pub enum WorkerOutcome {
    Success(Vec<u8>),
    Failed { code: String, message: String },
    Timeout,
    Crashed,
    OutputTooLarge,
}

pub async fn run_extraction(
    cfg: &Config,
    input: &Path,
    max_pages: Option<u32>,
    granularity: Option<Granularity>,
) -> WorkerOutcome {
    let mut cmd = Command::new(&cfg.cli_path);
    cmd.arg("extract").arg(input);
    if let Some(n) = max_pages {
        cmd.args(["--max-pages", &n.to_string()]);
    }
    if let Some(level) = granularity {
        cmd.args(["--granularity", level.as_str()]);
    }
    if let Some(dir) = &cfg.pdfium_dir {
        cmd.env("DOCRAY_PDFIUM_DIR", dir);
    }
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    // Memory rlimit (Linux; best-effort elsewhere — macOS ignores RLIMIT_AS in practice).
    #[cfg(target_os = "linux")]
    {
        let limit = cfg.mem_limit_bytes;
        unsafe {
            cmd.pre_exec(move || {
                let rl = libc::rlimit {
                    rlim_cur: limit,
                    rlim_max: limit,
                };
                // If setrlimit fails we must NOT exec: an unlimited child could
                // exhaust the machine's memory. Returning Err aborts the fork so
                // spawn() surfaces the failure (mapped to io_error) rather than
                // silently running the worker without a memory cap.
                if libc::setrlimit(libc::RLIMIT_AS, &rl) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return WorkerOutcome::Failed {
                code: "io_error".into(),
                message: e.to_string(),
            }
        }
    };

    let cap = cfg.output_cap_bytes;
    let stdout = child.stdout.take().expect("piped");
    let stderr = child.stderr.take().expect("piped");

    // Drain stdout with a streaming cap check. `Err(())` signals a cap breach.
    let read_stdout = async move {
        let mut stdout = stdout;
        let mut out = Vec::new();
        let mut buf = [0u8; 64 * 1024];
        loop {
            match stdout.read(&mut buf).await {
                Ok(0) => break Ok::<Vec<u8>, ()>(out),
                Ok(n) => {
                    if out.len() as u64 + n as u64 > cap {
                        break Err(()); // over cap
                    }
                    out.extend_from_slice(&buf[..n]);
                }
                Err(_) => break Ok(out),
            }
        }
    };

    // Drain stderr concurrently, capped at 64 KiB. Overflow is truncated (not an
    // error); we keep reading past the cap so the child can't block on a full
    // stderr pipe while we're still draining stdout.
    const STDERR_CAP: usize = 64 * 1024;
    let read_stderr = async move {
        let mut stderr = stderr;
        let mut err = Vec::new();
        let mut buf = [0u8; 16 * 1024];
        loop {
            match stderr.read(&mut buf).await {
                Ok(0) => break err,
                Ok(n) => {
                    if err.len() < STDERR_CAP {
                        let take = n.min(STDERR_CAP - err.len());
                        err.extend_from_slice(&buf[..take]);
                    }
                    // else: discard, but keep draining the pipe.
                }
                Err(_) => break err,
            }
        }
    };

    enum Drained {
        TooLarge,
        Done(Vec<u8>, Vec<u8>, std::io::Result<std::process::ExitStatus>),
    }

    let timeout = std::time::Duration::from_secs(cfg.timeout_secs);
    let result = tokio::time::timeout(timeout, async {
        // Concurrently drain both pipes: a stderr flood while stdout is still
        // open must not deadlock (F2). stderr is drained in a background task.
        let stderr_task = tokio::spawn(read_stderr);
        match read_stdout.await {
            Err(()) => {
                // Cap breached: kill the child immediately rather than awaiting
                // stderr EOF + child exit, which would let a stdout bomb block
                // until the timeout and get mislabeled Timeout (F1).
                let _ = child.kill().await;
                stderr_task.abort();
                Drained::TooLarge
            }
            Ok(out) => {
                let err = stderr_task.await.unwrap_or_default();
                let status = child.wait().await;
                Drained::Done(out, err, status)
            }
        }
    })
    .await;

    let (out, err, status) = match result {
        Ok(Drained::Done(out, err, status)) => (out, err, status),
        Ok(Drained::TooLarge) => return WorkerOutcome::OutputTooLarge,
        Err(_) => {
            let _ = child.kill().await;
            return WorkerOutcome::Timeout;
        }
    };

    match status {
        Ok(s) if s.success() => WorkerOutcome::Success(out),
        Ok(s) => match s.code() {
            Some(code @ 2..=6) => {
                let parsed: Option<Value> = serde_json::from_slice(&err).ok();
                let (c, m) = parsed
                    .as_ref()
                    .and_then(|v| {
                        Some((
                            v["error"]["code"].as_str()?.to_string(),
                            v["error"]["message"].as_str().unwrap_or("").to_string(),
                        ))
                    })
                    .unwrap_or((format!("exit_{code}"), String::new()));
                WorkerOutcome::Failed {
                    code: c,
                    message: m,
                }
            }
            // Killed by signal (segfault, RLIMIT_AS abort, or OOM-kill) or an
            // unknown exit. We deliberately do NOT add an OOM-specific outcome:
            // a memory-limit death (SIGSEGV from a failed allocation, SIGABRT, or
            // SIGKILL from the kernel OOM killer) is not reliably distinguishable
            // from an ordinary crash by signal number alone, so all signal deaths
            // collapse to Crashed and the message hints at the memory-limit case.
            _ => WorkerOutcome::Crashed,
        },
        Err(_) => WorkerOutcome::Crashed,
    }
}
