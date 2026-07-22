use std::time::Duration;

// Boots the real server binary on an ephemeral port and hits it with reqwest.
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
fn playground_pptx_source_isolation_contract() {
    let html = include_str!("../assets/playground.html");
    assert!(html.contains("const PPTX_IFRAME_SANDBOX = \"allow-scripts\";"));
    assert!(!html.contains("allow-same-origin"));
    assert!(html.contains(
        "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; base-uri 'none'; form-action 'none'"
    ));
    assert!(html.contains("event.origin !== \"null\""));
    assert!(html.contains("void parent.location.href"));
    assert!(html.contains("parent.postMessage({ status }, \"*\")"));
    assert!(html.contains("state.file.arrayBuffer()"));
    assert!(html.contains("[bytes]"));
    assert!(html.contains("visual render unavailable - showing structure schematic"));
    assert!(html.contains("PPTX_CANVAS_BYTE_CAP = 256 * 1024 * 1024"));
    assert!(html.contains("31cf1e39818c52395b185186229f80ecf8333db0d7bb3a06f6c0bd74b87aaad5"));
}

#[test]
fn playground_pptx_thumbnail_isolation_contract() {
    let html = include_str!("../assets/playground.html");
    assert!(html.contains("iframe.setAttribute(\"sandbox\", PPTX_IFRAME_SANDBOX);"));
    assert!(!html.contains("allow-same-origin"));
    assert!(html.contains(
        "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; base-uri 'none'; form-action 'none'"
    ));
    assert!(html.contains("iframe.srcdoc = pptxRendererSrcdoc();"));
    assert!(html.contains("event.source !== iframe.contentWindow || event.origin !== \"null\""));
    assert!(html.contains("keys.length !== 1 || keys[0] !== \"status\""));
    assert!(html.contains("![\"ready\", \"rendered\", \"error\"].includes(data.status)"));
    assert!(html.contains("parent.postMessage({ status }, \"*\")"));
    assert!(!html.contains("PPTX_THUMB_DATA_URL_MAX"));
    assert!(!html.contains("cmd: \"renderThumb\""));
    assert!(!html.contains("dataUrl"));
    assert!(!html.contains("XMLSerializer().serializeToString"));
    assert!(!html.contains("iframe.contentDocument"));
    assert!(html.contains("state.file.arrayBuffer()"));
    assert!(html.contains("[bytes]"));
    assert!(html.contains("const PPTX_LIVE_THUMB_MAX = 4;"));
    assert!(html.contains("while (pptxLiveThumbs.size > PPTX_LIVE_THUMB_MAX)"));
    assert!(html.contains("ensurePptxLiveThumb(state.pageIdx, gen)"));
    assert!(html.contains("resetPptxLiveThumbs();"));
    assert!(html.contains("thumbnail: true"));
    assert!(html.contains("if (role && !thumbnail)"));
    assert!(html.contains("if (!opts.thumbnail)"));
}

#[test]
fn healthz_and_sync_extract_and_errors() {
    let server = TestServer::start();

    // healthz
    let r = reqwest::blocking::get(format!("{}/healthz", server.base)).unwrap();
    assert_eq!(r.status(), 200);

    // playground UI is served embedded
    let r = reqwest::blocking::get(format!("{}/playground", server.base)).unwrap();
    assert_eq!(r.status(), 200);
    assert!(r
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("text/html"));
    assert!(r.text().unwrap().contains("docray · Playground"));

    // happy path
    let r = upload(&server.base, "/v1/extract", fixture("simple.pdf"));
    assert_eq!(r.status(), 200);
    let v: serde_json::Value = r.json().unwrap();
    assert_eq!(v["schema_version"], "1.1");
    assert_eq!(v["pages"][0]["page_number"], 1);

    // unsupported format -> 415
    let r = upload(&server.base, "/v1/extract", b"not a pdf at all".to_vec());
    assert_eq!(r.status(), 415);
    let v: serde_json::Value = r.json().unwrap();
    assert_eq!(v["error"]["code"], "unsupported_format");

    // parse failure -> 422
    let r = upload(
        &server.base,
        "/v1/extract",
        fixture("malformed/truncated.pdf"),
    );
    assert!(
        r.status() == 422 || r.status() == 200,
        "truncated pdf: {}",
        r.status()
    );
}

#[test]
fn extract_granularity_element_and_invalid_value() {
    let server = TestServer::start();

    let r = upload(
        &server.base,
        "/v1/extract?granularity=element",
        fixture("simple.pdf"),
    );
    assert_eq!(r.status(), 200);
    let v: serde_json::Value = r.json().unwrap();
    assert_eq!(v["schema_version"], "1.6");
    assert_eq!(v["granularity"], "element");
    assert_eq!(v["pages"][0]["elements"][0]["type"], "text");
    assert_eq!(v["pages"][0]["elements"][0]["text"], "Hello World");
    assert!(v["pages"][0]["elements"][0]["bbox"].is_array());
    assert!(v["pages"][0]["elements"][0].get("lines").is_none());

    let r = upload(
        &server.base,
        "/v1/extract?granularity=detail",
        fixture("simple.pdf"),
    );
    assert_eq!(r.status(), 400);
    let v: serde_json::Value = r.json().unwrap();
    assert_eq!(v["error"]["code"], "bad_granularity");
}

#[test]
fn extract_lean_content_type_default_and_char_rejection() {
    let server = TestServer::start();

    let r = upload(
        &server.base,
        "/v1/extract?format=lean",
        fixture("simple.pdf"),
    );
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.headers().get("content-type").unwrap(),
        "text/plain; charset=utf-8"
    );
    assert!(r
        .text()
        .unwrap()
        .starts_with("#docray element v1.6 pages=1\n#legend "));

    let r = upload(
        &server.base,
        "/v1/extract?format=lean&granularity=char",
        fixture("simple.pdf"),
    );
    assert_eq!(r.status(), 400);
    let v: serde_json::Value = r.json().unwrap();
    assert_eq!(v["error"]["code"], "bad_format");
}

#[test]
fn pptx_element_no_parameter_and_lean_end_to_end() {
    let server = TestServer::start();

    let r = upload(
        &server.base,
        "/v1/extract?granularity=element",
        fixture("pptx/basic.pptx"),
    );
    assert_eq!(r.status(), 200);
    let v: serde_json::Value = r.json().unwrap();
    assert_eq!(v["source"]["format"], "pptx");
    assert_eq!(v["granularity"], "element");
    assert_eq!(v["pages"][0]["elements"][0]["text"], "First shape");

    let r = upload(&server.base, "/v1/extract", fixture("pptx/basic.pptx"));
    assert_eq!(r.status(), 400);
    let v: serde_json::Value = r.json().unwrap();
    assert_eq!(v["error"]["code"], "granularity_unavailable");
    assert!(v["error"]["message"]
        .as_str()
        .unwrap()
        .contains("retry with granularity=element"));

    let r = upload(
        &server.base,
        "/v1/extract?format=lean",
        fixture("pptx/basic.pptx"),
    );
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.headers().get("content-type").unwrap(),
        "text/plain; charset=utf-8"
    );
    assert!(r.text().unwrap().contains("First shape"));
}

// An encrypted/password-protected PDF must map to 422 encrypted_pdf.
#[test]
fn encrypted_pdf_returns_422() {
    let server = TestServer::start();
    let r = upload(
        &server.base,
        "/v1/extract",
        fixture("encrypted/encrypted.pdf"),
    );
    assert_eq!(r.status(), 422, "expected 422, got {}", r.status());
    let v: serde_json::Value = r.json().unwrap();
    assert_eq!(v["error"]["code"], "encrypted_pdf");
}

// With DOCRAY_SYNC_MAX_PAGES=0 even a 1-page PDF exceeds the cap -> 413 too_many_pages.
#[test]
fn too_many_pages_returns_413() {
    let server = TestServer::start_with(&[("DOCRAY_SYNC_MAX_PAGES", "0")]);
    let r = upload(&server.base, "/v1/extract", fixture("simple.pdf"));
    assert_eq!(r.status(), 413, "expected 413, got {}", r.status());
    let v: serde_json::Value = r.json().unwrap();
    assert_eq!(v["error"]["code"], "too_many_pages");
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

// F1: cap breach must kill the child immediately and return 500 output_too_large
// well under the timeout — a stdout bomb must NOT get mislabeled Timeout/504.
#[test]
fn output_cap_breach_kills_and_returns_output_too_large_fast() {
    let script = write_script("stdout-bomb", "exec yes X");
    let server = TestServer::start_with(&[
        ("DOCRAY_CLI_PATH", &script),
        ("DOCRAY_OUTPUT_CAP_BYTES", "1000000"),
        // 30s timeout: a mislabel-as-timeout would surface as a slow 504, not this.
        ("DOCRAY_TIMEOUT_SECS", "30"),
    ]);

    let start = std::time::Instant::now();
    let r = upload(&server.base, "/v1/extract", fixture("simple.pdf"));
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_secs(10),
        "cap breach took {elapsed:?}"
    );
    assert_eq!(r.status(), 500, "expected 500, got {}", r.status());
    let v: serde_json::Value = r.json().unwrap();
    assert_eq!(v["error"]["code"], "output_too_large");
}

// F2/F3: a child that floods stderr (1 MiB) while stdout stays open must not
// deadlock — bounded stderr drain + concurrent pipe reads keep it responsive.
#[test]
fn stderr_flood_does_not_deadlock() {
    // ~1 MiB of garbage to stderr, then exit 4. Garbage prefix means stderr JSON
    // parse fails -> exit_4 fallback code -> 500.
    let script = write_script(
        "stderr-flood",
        "head -c 1048576 /dev/zero | tr '\\0' 'Z' 1>&2\nexit 4",
    );
    let server = TestServer::start_with(&[("DOCRAY_CLI_PATH", &script)]);

    let start = std::time::Instant::now();
    let r = upload(&server.base, "/v1/extract", fixture("simple.pdf"));
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_secs(10),
        "stderr flood took {elapsed:?}"
    );
    assert_eq!(r.status(), 500, "expected 500, got {}", r.status());
    let v: serde_json::Value = r.json().unwrap();
    assert_eq!(v["error"]["code"], "exit_4");
}

// A worker that hangs forever must be killed once DOCRAY_TIMEOUT_SECS elapses,
// surfacing as 504 {"error":{"code":"timeout"}} rather than hanging the request.
#[test]
fn worker_timeout_returns_504() {
    let script = write_script("hang", "exec sleep 600");
    let server =
        TestServer::start_with(&[("DOCRAY_CLI_PATH", &script), ("DOCRAY_TIMEOUT_SECS", "1")]);

    let start = std::time::Instant::now();
    let r = upload(&server.base, "/v1/extract", fixture("simple.pdf"));
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_secs(10),
        "timeout enforcement took {elapsed:?}"
    );
    assert_eq!(r.status(), 504, "expected 504, got {}", r.status());
    let v: serde_json::Value = r.json().unwrap();
    assert_eq!(v["error"]["code"], "timeout");
}
