use std::time::Duration;

// Boots the real server binary on an ephemeral port and hits it with reqwest.
// Helpers (TestServer, free_port, upload, fixture) are copied verbatim from
// sync_api.rs: integration test files cannot share modules without extra setup,
// so this duplication is deliberate.
struct TestServer {
    child: std::process::Child,
    base: String,
    port: u16,
    data_dir: std::path::PathBuf,
}

impl TestServer {
    fn start() -> TestServer {
        TestServer::start_with(&[])
    }

    // Boots the server with extra env vars and (optionally) an override CLI path,
    // so tests can point DOCRAY_CLI_PATH at a fake shell script.
    //
    // clippy::zombie_processes fires because the readiness loop can return
    // `TestServer` without ever calling `.wait()` on `child`. That's
    // intentional: the process is meant to keep running for the test's
    // duration and is killed (not waited on) in `Drop` below; the child is a
    // short-lived test server reaped by the OS when the test binary exits.
    #[allow(clippy::zombie_processes)]
    fn start_with(extra_env: &[(&str, &str)]) -> TestServer {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
        let port = free_port();
        let data_dir = std::env::temp_dir().join(format!("docray-test-{port}"));
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_docray-server"));
        cmd.env("DOCRAY_PORT", port.to_string())
            .env("DOCRAY_CLI_PATH", format!("{root}/target/debug/docray"))
            .env("DOCRAY_PDFIUM_DIR", format!("{root}/.pdfium/lib"))
            .env("DOCRAY_DATA_DIR", &data_dir);
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        let child = cmd.spawn().unwrap();
        let base = format!("http://127.0.0.1:{port}");
        // Wait for readiness.
        for _ in 0..50 {
            if reqwest::blocking::get(format!("{base}/healthz")).is_ok() {
                return TestServer {
                    child,
                    base,
                    port,
                    data_dir,
                };
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("server did not become ready");
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn upload(base: &str, path: &str, bytes: Vec<u8>) -> reqwest::blocking::Response {
    let part = reqwest::blocking::multipart::Part::bytes(bytes).file_name("in.pdf");
    let form = reqwest::blocking::multipart::Form::new().part("file", part);
    reqwest::blocking::Client::new()
        .post(format!("{base}{path}"))
        .multipart(form)
        .send()
        .unwrap()
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../../testdata/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

#[test]
fn job_lifecycle_success_and_failure() {
    let server = TestServer::start();
    let client = reqwest::blocking::Client::new();

    // Submit a good job.
    let r = upload(&server.base, "/v1/jobs", fixture("simple.pdf"));
    assert_eq!(r.status(), 202);
    let v: serde_json::Value = r.json().unwrap();
    let id = v["job_id"].as_str().unwrap().to_string();

    // Poll until terminal.
    let mut status = String::new();
    for _ in 0..100 {
        let v: serde_json::Value = client
            .get(format!("{}/v1/jobs/{id}", server.base))
            .send()
            .unwrap()
            .json()
            .unwrap();
        status = v["status"].as_str().unwrap().to_string();
        if status == "succeeded" || status == "failed" {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(status, "succeeded");

    // Fetch result.
    let r = client
        .get(format!("{}/v1/jobs/{id}/result", server.base))
        .send()
        .unwrap();
    assert_eq!(r.status(), 200);
    let v: serde_json::Value = r.json().unwrap();
    assert_eq!(v["schema_version"], "1.1");

    // Failing job (garbage input).
    let r = upload(&server.base, "/v1/jobs", b"garbage".to_vec());
    assert_eq!(r.status(), 202);
    let id = r.json::<serde_json::Value>().unwrap()["job_id"]
        .as_str()
        .unwrap()
        .to_string();
    let mut last = serde_json::Value::Null;
    for _ in 0..100 {
        last = client
            .get(format!("{}/v1/jobs/{id}", server.base))
            .send()
            .unwrap()
            .json()
            .unwrap();
        if last["status"] == "failed" {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(last["status"], "failed");
    assert_eq!(last["error"]["code"], "unsupported_format");

    // Result of failed job -> 404, job exists but has no result: not_ready.
    let r = client
        .get(format!("{}/v1/jobs/{id}/result", server.base))
        .send()
        .unwrap();
    assert_eq!(r.status(), 404);
    let v: serde_json::Value = r.json().unwrap();
    assert_eq!(v["error"]["code"], "not_ready");

    // Unknown job status -> 404.
    let r = client
        .get(format!("{}/v1/jobs/does-not-exist", server.base))
        .send()
        .unwrap();
    assert_eq!(r.status(), 404);

    // Unknown job result -> 404, no such job: not_found.
    let r = client
        .get(format!("{}/v1/jobs/does-not-exist/result", server.base))
        .send()
        .unwrap();
    assert_eq!(r.status(), 404);
    let v: serde_json::Value = r.json().unwrap();
    assert_eq!(v["error"]["code"], "not_found");
}

#[test]
fn job_output_options_are_stored_and_forwarded_to_the_worker_cli() {
    let server = TestServer::start();
    let client = reqwest::blocking::Client::new();
    let r = upload(
        &server.base,
        "/v1/jobs?granularity=word&classify=true",
        fixture("simple.pdf"),
    );
    assert_eq!(r.status(), 202);
    let id = r.json::<serde_json::Value>().unwrap()["job_id"]
        .as_str()
        .unwrap()
        .to_string();

    let mut status = String::new();
    for _ in 0..100 {
        let v: serde_json::Value = client
            .get(format!("{}/v1/jobs/{id}", server.base))
            .send()
            .unwrap()
            .json()
            .unwrap();
        status = v["status"].as_str().unwrap().to_string();
        if status == "succeeded" || status == "failed" {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(status, "succeeded");

    let r = client
        .get(format!("{}/v1/jobs/{id}/result", server.base))
        .send()
        .unwrap();
    assert_eq!(r.status(), 200);
    let v: serde_json::Value = r.json().unwrap();
    assert_eq!(v["schema_version"], "1.8");
    assert_eq!(v["granularity"], "word");
    assert_eq!(v["pages"][0]["classification"]["kind"], "text");
    assert_eq!(v["pages"][0]["elements"][0]["words"][0][0], "Hello");
}

#[test]
fn lean_job_roundtrips_stored_format_and_content_type() {
    let server = TestServer::start();
    let client = reqwest::blocking::Client::new();
    let r = upload(
        &server.base,
        "/v1/jobs?format=lean&granularity=word",
        fixture("simple.pdf"),
    );
    assert_eq!(r.status(), 202);
    let id = r.json::<serde_json::Value>().unwrap()["job_id"]
        .as_str()
        .unwrap()
        .to_string();

    let mut status = String::new();
    for _ in 0..100 {
        let v: serde_json::Value = client
            .get(format!("{}/v1/jobs/{id}", server.base))
            .send()
            .unwrap()
            .json()
            .unwrap();
        status = v["status"].as_str().unwrap().to_string();
        if status == "succeeded" || status == "failed" {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(status, "succeeded");

    let r = client
        .get(format!("{}/v1/jobs/{id}/result", server.base))
        .send()
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.headers().get("content-type").unwrap(),
        "text/plain; charset=utf-8"
    );
    assert!(r
        .text()
        .unwrap()
        .starts_with("#docray word v1.9 pages=1\n#legend "));
}

#[test]
fn markdown_job_roundtrips_stored_format_and_content_type() {
    let server = TestServer::start();
    let client = reqwest::blocking::Client::new();
    let r = upload(&server.base, "/v1/jobs?format=md", fixture("simple.pdf"));
    assert_eq!(r.status(), 202);
    let id = r.json::<serde_json::Value>().unwrap()["job_id"]
        .as_str()
        .unwrap()
        .to_string();

    let mut status = String::new();
    for _ in 0..100 {
        let v: serde_json::Value = client
            .get(format!("{}/v1/jobs/{id}", server.base))
            .send()
            .unwrap()
            .json()
            .unwrap();
        status = v["status"].as_str().unwrap().to_string();
        if status == "succeeded" || status == "failed" {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(status, "succeeded");

    let r = client
        .get(format!("{}/v1/jobs/{id}/result", server.base))
        .send()
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.headers().get("content-type").unwrap(),
        "text/markdown; charset=utf-8"
    );
    assert!(r.text().unwrap().contains("# Bold Title"));
}

// `pages=2-3` on a job over the 6-page multipage.pdf must persist through the
// job store and be forwarded to the worker's CLI invocation the same way sync
// does: the result has exactly 2 pages, keeping their ORIGINAL absolute page
// numbers (not renumbered to 1,2), and the LEAN header's `pages=` count
// reflects the number of page blocks returned.
#[test]
fn job_pages_selects_sub_range_with_absolute_numbering() {
    let server = TestServer::start();
    let client = reqwest::blocking::Client::new();

    let r = upload(&server.base, "/v1/jobs?pages=2-3", fixture("multipage.pdf"));
    assert_eq!(r.status(), 202);
    let id = r.json::<serde_json::Value>().unwrap()["job_id"]
        .as_str()
        .unwrap()
        .to_string();

    let mut status = String::new();
    for _ in 0..100 {
        let v: serde_json::Value = client
            .get(format!("{}/v1/jobs/{id}", server.base))
            .send()
            .unwrap()
            .json()
            .unwrap();
        status = v["status"].as_str().unwrap().to_string();
        if status == "succeeded" || status == "failed" {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(status, "succeeded");

    let r = client
        .get(format!("{}/v1/jobs/{id}/result", server.base))
        .send()
        .unwrap();
    assert_eq!(r.status(), 200);
    let v: serde_json::Value = r.json().unwrap();
    assert_eq!(v["pages"].as_array().unwrap().len(), 2);
    assert_eq!(v["pages"][0]["page_number"], 2);
    assert_eq!(v["pages"][1]["page_number"], 3);

    // Same selection, LEAN format: header `pages=` count is 2 (selected page
    // blocks), and only pages 2 and 3 appear as `#page` markers.
    let r = upload(
        &server.base,
        "/v1/jobs?pages=2-3&format=lean",
        fixture("multipage.pdf"),
    );
    assert_eq!(r.status(), 202);
    let id = r.json::<serde_json::Value>().unwrap()["job_id"]
        .as_str()
        .unwrap()
        .to_string();

    let mut status = String::new();
    for _ in 0..100 {
        let v: serde_json::Value = client
            .get(format!("{}/v1/jobs/{id}", server.base))
            .send()
            .unwrap()
            .json()
            .unwrap();
        status = v["status"].as_str().unwrap().to_string();
        if status == "succeeded" || status == "failed" {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(status, "succeeded");

    let r = client
        .get(format!("{}/v1/jobs/{id}/result", server.base))
        .send()
        .unwrap();
    assert_eq!(r.status(), 200);
    let body = r.text().unwrap();
    assert!(body.contains("pages=2\n"), "body: {body}");
    assert!(body.contains("#page 2 "), "body: {body}");
    assert!(body.contains("#page 3 "), "body: {body}");
    assert!(!body.contains("#page 1 "), "body: {body}");
    assert!(!body.contains("#page 4 "), "body: {body}");
}

// A malformed `pages=` value is rejected at submit time (400 bad_pages,
// shared `requested_output` validation), before a job is ever created — no
// job_id is issued and nothing reaches the queue or the worker.
#[test]
fn job_pages_bad_selection_returns_400_bad_pages_at_submit() {
    let server = TestServer::start();

    let r = upload(&server.base, "/v1/jobs?pages=abc", fixture("multipage.pdf"));
    assert_eq!(r.status(), 400, "expected 400, got {}", r.status());
    let v: serde_json::Value = r.json().unwrap();
    assert_eq!(v["error"]["code"], "bad_pages");

    let r = upload(
        &server.base,
        "/v1/jobs?pages=200-1",
        fixture("multipage.pdf"),
    );
    assert_eq!(r.status(), 400, "expected 400, got {}", r.status());
    let v: serde_json::Value = r.json().unwrap();
    assert_eq!(v["error"]["code"], "bad_pages");
}

// Sanity: a full-document job (no `pages=` at all) is unaffected by the new
// wiring — every page of the multi-page fixture comes back, in order.
#[test]
fn job_without_pages_returns_full_document_unchanged() {
    let server = TestServer::start();
    let client = reqwest::blocking::Client::new();

    let r = upload(&server.base, "/v1/jobs", fixture("multipage.pdf"));
    assert_eq!(r.status(), 202);
    let id = r.json::<serde_json::Value>().unwrap()["job_id"]
        .as_str()
        .unwrap()
        .to_string();

    let mut status = String::new();
    for _ in 0..100 {
        let v: serde_json::Value = client
            .get(format!("{}/v1/jobs/{id}", server.base))
            .send()
            .unwrap()
            .json()
            .unwrap();
        status = v["status"].as_str().unwrap().to_string();
        if status == "succeeded" || status == "failed" {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(status, "succeeded");

    let r = client
        .get(format!("{}/v1/jobs/{id}/result", server.base))
        .send()
        .unwrap();
    assert_eq!(r.status(), 200);
    let v: serde_json::Value = r.json().unwrap();
    let pages = v["pages"].as_array().unwrap();
    assert_eq!(pages.len(), 6);
    for (i, page) in pages.iter().enumerate() {
        assert_eq!(page["page_number"], (i + 1) as i64);
    }
}

// Writes a `#!/bin/sh` script to a unique temp path, chmod 755, returns the path.
fn write_script(tag: &str, body: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::temp_dir().join(format!(
        "docray-fake-cli-{tag}-{}-{}.sh",
        std::process::id(),
        free_port()
    ));
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.to_str().unwrap().to_string()
}

fn submit(base: &str, bytes: Vec<u8>) -> String {
    let r = upload(base, "/v1/jobs", bytes);
    assert_eq!(r.status(), 202);
    r.json::<serde_json::Value>().unwrap()["job_id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn wait_terminal(base: &str, id: &str) -> serde_json::Value {
    let client = reqwest::blocking::Client::new();
    for _ in 0..200 {
        let v: serde_json::Value = client
            .get(format!("{base}/v1/jobs/{id}"))
            .send()
            .unwrap()
            .json()
            .unwrap();
        if v["status"] == "succeeded" || v["status"] == "failed" {
            return v;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("job {id} did not finish");
}

fn upload_files(server: &TestServer) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(server.data_dir.join("uploads"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn error_code(r: reqwest::blocking::Response) -> String {
    r.json::<serde_json::Value>().unwrap()["error"]["code"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn job_upload_is_deleted_as_soon_as_extraction_finishes() {
    let server = TestServer::start();
    let ok = submit(&server.base, fixture("simple.pdf"));
    let bad = submit(&server.base, b"garbage".to_vec());
    assert_eq!(wait_terminal(&server.base, &ok)["status"], "succeeded");
    assert_eq!(wait_terminal(&server.base, &bad)["status"], "failed");

    // Retaining inputs for the result TTL let finished jobs pin disk.
    assert_eq!(upload_files(&server), Vec::<String>::new());
    assert!(server.data_dir.join(format!("results/{ok}.json")).exists());
    let r = reqwest::blocking::get(format!("{}/v1/jobs/{ok}/result", server.base)).unwrap();
    assert_eq!(r.status(), 200);
}

#[test]
fn job_submission_over_pending_cap_is_rejected_until_a_slot_frees() {
    let script = write_script("slow", "sleep 2");
    let server = TestServer::start_with(&[
        ("DOCRAY_CLI_PATH", &script),
        ("DOCRAY_MAX_PENDING_JOBS", "1"),
        ("DOCRAY_WORKERS", "1"),
    ]);
    let first = submit(&server.base, b"one".to_vec());

    let r = upload(&server.base, "/v1/jobs", b"two".to_vec());
    assert_eq!(r.status(), 503);
    assert_eq!(error_code(r), "queue_full");
    assert_eq!(
        upload_files(&server).len(),
        1,
        "rejected upload left no file"
    );

    // The rejected request released its reservation and the finished job
    // freed its slot, so the cap admits a new job again.
    wait_terminal(&server.base, &first);
    submit(&server.base, b"three".to_vec());
}

#[test]
fn job_submission_below_free_space_floor_is_rejected_without_writing() {
    let server = TestServer::start_with(&[("DOCRAY_MIN_FREE_BYTES", &u64::MAX.to_string())]);
    let r = upload(&server.base, "/v1/jobs", fixture("simple.pdf"));
    assert_eq!(r.status(), 507);
    assert_eq!(error_code(r), "insufficient_storage");
    assert_eq!(upload_files(&server), Vec::<String>::new());
}

// Sends the headers and the start of a 1 MB multipart body, then stalls.
// Returns the status line the server answers with.
fn stalled_upload(server: &TestServer, path: &str) -> String {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\
         Content-Type: multipart/form-data; boundary=X\r\nContent-Length: 1000000\r\n\r\n\
         --X\r\nContent-Disposition: form-data; name=\"file\"; filename=\"in.pdf\"\r\n\r\n%PDF-1.7 partial"
    )
    .unwrap();
    stream.flush().unwrap();
    let mut response = [0u8; 64];
    let n = stream
        .read(&mut response)
        .expect("server must answer a stalled upload before the client gives up");
    String::from_utf8_lossy(&response[..n])
        .lines()
        .next()
        .unwrap_or_default()
        .to_string()
}

#[test]
fn stalled_uploads_time_out_and_leave_no_file() {
    let server = TestServer::start_with(&[("DOCRAY_UPLOAD_TIMEOUT_SECS", "1")]);
    assert!(
        stalled_upload(&server, "/v1/jobs").starts_with("HTTP/1.1 408"),
        "jobs route"
    );
    assert_eq!(upload_files(&server), Vec::<String>::new());
    assert!(
        stalled_upload(&server, "/v1/extract").starts_with("HTTP/1.1 408"),
        "sync route"
    );
}
