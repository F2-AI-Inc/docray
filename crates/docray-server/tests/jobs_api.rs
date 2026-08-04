use std::time::Duration;

// Boots the real server binary on an ephemeral port and hits it with reqwest.
// Helpers (TestServer, free_port, upload, fixture) are copied verbatim from
// sync_api.rs: integration test files cannot share modules without extra setup,
// so this duplication is deliberate.
struct TestServer {
    child: std::process::Child,
    base: String,
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
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_docray-server"));
        cmd.env("DOCRAY_PORT", port.to_string())
            .env("DOCRAY_CLI_PATH", format!("{root}/target/debug/docray"))
            .env("DOCRAY_PDFIUM_DIR", format!("{root}/.pdfium/lib"))
            .env(
                "DOCRAY_DATA_DIR",
                std::env::temp_dir().join(format!("docray-test-{port}")),
            );
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        let child = cmd.spawn().unwrap();
        let base = format!("http://127.0.0.1:{port}");
        // Wait for readiness.
        for _ in 0..50 {
            if reqwest::blocking::get(format!("{base}/healthz")).is_ok() {
                return TestServer { child, base };
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
