use assert_cmd::Command;
use predicates::prelude::*;

fn testdata(name: &str) -> String {
    format!("{}/../../testdata/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn dps() -> Command {
    let mut cmd = Command::cargo_bin("docray").unwrap();
    // Tests run with crate CWD; point at the workspace-root pdfium dir.
    cmd.env(
        "DOCRAY_PDFIUM_DIR",
        format!("{}/../../.pdfium/lib", env!("CARGO_MANIFEST_DIR")),
    );
    cmd
}

#[test]
fn extracts_pdf_to_json_stdout() {
    dps()
        .arg("extract")
        .arg(testdata("simple.pdf"))
        .assert()
        .success()
        .stdout(predicate::str::contains("\"schema_version\":\"1.1\""))
        .stdout(predicate::str::contains("Hello"));
}

#[test]
fn explicit_char_has_v1_6_envelope_and_lossless_hierarchy() {
    dps()
        .arg("extract")
        .arg(testdata("simple.pdf"))
        .args(["--granularity", "char"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"schema_version\":\"1.6\""))
        .stdout(predicate::str::contains("\"granularity\":\"char\""))
        .stdout(predicate::str::contains("\"chars\""));
}

#[test]
fn classify_is_opt_in_schema_1_8_and_composes_with_granularity() {
    dps()
        .arg("extract")
        .arg(testdata("simple.pdf"))
        .arg("--classify")
        .assert()
        .success()
        .stdout(predicate::str::contains("\"schema_version\":\"1.8\""))
        .stdout(predicate::str::contains(
            "\"classification\":{\"kind\":\"text\"",
        ))
        .stdout(predicate::str::contains("\"needs_ocr\":false"));

    dps()
        .arg("extract")
        .arg(testdata("mixed.pdf"))
        .args(["--classify", "--granularity", "element"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"schema_version\":\"1.8\""))
        .stdout(predicate::str::contains("\"granularity\":\"element\""))
        .stdout(predicate::str::contains("\"kind\":\"mixed\""));
}

#[test]
fn classify_rejects_non_json_formats() {
    dps()
        .arg("extract")
        .arg(testdata("simple.pdf"))
        .args(["--classify", "--format", "lean"])
        .assert()
        .code(7)
        .stderr(predicate::str::contains("\"code\":\"bad_format\""));
}

#[test]
fn lean_defaults_to_element_and_emits_fixed_header_lines() {
    let assert = dps()
        .arg("extract")
        .arg(testdata("simple.pdf"))
        .args(["--format", "lean"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let mut lines = stdout.lines();
    assert_eq!(lines.next(), Some("#docray element v1.9 pages=1"));
    assert_eq!(
        lines.next(),
        Some(
            "#legend T x0 y0 x1 y1 font size style text | I/P x0 y0 x1 y1 | A x0 y0 x1 y1 subtype uri | pt, top-left origin"
        )
    );
}

#[test]
fn lean_char_exits_7_with_bad_format_envelope() {
    dps()
        .arg("extract")
        .arg(testdata("simple.pdf"))
        .args(["--format", "lean", "--granularity", "char"])
        .assert()
        .code(7)
        .stderr(predicate::str::contains("\"code\":\"bad_format\""));
}

#[test]
fn markdown_defaults_to_element_and_char_is_rejected() {
    dps()
        .arg("extract")
        .arg(testdata("simple.pdf"))
        .args(["--format", "md"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hello World"))
        .stdout(predicate::str::contains("# Bold Title"));

    dps()
        .arg("extract")
        .arg(testdata("simple.pdf"))
        .args(["--format", "md", "--granularity", "char"])
        .assert()
        .code(7)
        .stderr(predicate::str::contains("\"code\":\"bad_format\""));
}

#[test]
fn unknown_output_format_exits_7_with_bad_format_envelope() {
    dps()
        .arg("extract")
        .arg(testdata("simple.pdf"))
        .args(["--format", "toon"])
        .assert()
        .code(7)
        .stderr(predicate::str::contains("\"code\":\"bad_format\""));
}

#[test]
fn unsupported_format_exits_2_with_error_json() {
    dps()
        .arg("extract")
        .arg(testdata("malformed/garbage.bin"))
        .assert()
        .code(2)
        .stderr(predicate::str::contains("\"unsupported_format\""));
}

#[test]
fn missing_file_exits_5() {
    dps()
        .arg("extract")
        .arg("no-such-file.pdf")
        .assert()
        .code(5)
        .stderr(predicate::str::contains("\"io_error\""));
}

#[test]
fn page_cap_exits_6() {
    dps()
        .arg("extract")
        .arg(testdata("simple.pdf"))
        .args(["--max-pages", "0"])
        .assert()
        .code(6)
        .stderr(predicate::str::contains("\"too_many_pages\""));
}

#[test]
fn pptx_finer_than_element_exits_8() {
    // PPTX only supports element granularity; asking for a finer level errors.
    for args in [vec!["--granularity", "word"], vec!["--granularity", "char"]] {
        dps()
            .arg("extract")
            .arg(testdata("pptx/basic.pptx"))
            .args(args)
            .assert()
            .code(8)
            .stderr(predicate::str::contains("\"granularity_unavailable\""))
            .stderr(predicate::str::contains("retry with granularity=element"));
    }
}

#[test]
fn pptx_defaults_to_element_when_granularity_omitted() {
    // `docray extract deck.pptx` with no flags should just work: default to the
    // finest granularity the format supports (element), not error.
    dps()
        .arg("extract")
        .arg(testdata("pptx/basic.pptx"))
        .assert()
        .success()
        .stdout(predicate::str::contains("\"format\":\"pptx\""))
        .stdout(predicate::str::contains("\"granularity\":\"element\""))
        .stdout(predicate::str::contains("\"text\":\"First shape\""));
}

#[test]
fn pptx_explicit_element_and_lean_work() {
    dps()
        .arg("extract")
        .arg(testdata("pptx/basic.pptx"))
        .args(["--granularity", "element"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"format\":\"pptx\""))
        .stdout(predicate::str::contains("\"text\":\"First shape\""));

    dps()
        .arg("extract")
        .arg(testdata("pptx/basic.pptx"))
        .args(["--format", "lean"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("#docray element v1.9 pages=1"));

    dps()
        .arg("extract")
        .arg(testdata("pptx/table.pptx"))
        .args(["--format", "md"])
        .assert()
        .success()
        .stdout(predicate::str::contains("| Merged |  |"));
}

#[test]
fn docx_defaults_to_element_rejects_finer_and_supports_lean() {
    dps()
        .arg("extract")
        .arg(testdata("docx/fields.docx"))
        .assert()
        .success()
        .stdout(predicate::str::contains("\"schema_version\":\"1.7\""))
        .stdout(predicate::str::contains("\"layout\":\"flow\""))
        .stdout(predicate::str::contains("Cached heading"))
        .stdout(predicate::str::contains("instrText").not());

    for granularity in ["word", "char"] {
        dps()
            .arg("extract")
            .arg(testdata("docx/fields.docx"))
            .args(["--granularity", granularity])
            .assert()
            .code(8)
            .stderr(predicate::str::contains("\"granularity_unavailable\""));
    }

    dps()
        .arg("extract")
        .arg(testdata("docx/fields.docx"))
        .args(["--format", "lean"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with(
            "#docray element v1.7 sections=1",
        ))
        .stdout(predicate::str::contains("Cached heading"));

    dps()
        .arg("extract")
        .arg(testdata("docx/roles.docx"))
        .args(["--format", "md"])
        .assert()
        .success()
        .stdout(predicate::str::contains("# H1"))
        .stdout(predicate::str::contains("## H2"));
}

#[test]
fn closed_stdout_pipe_is_quiet_success_not_a_panic() {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_docray"))
        .env(
            "DOCRAY_PDFIUM_DIR",
            format!("{}/../../.pdfium/lib", env!("CARGO_MANIFEST_DIR")),
        )
        .arg("extract")
        .arg(testdata("simple.pdf"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // Close the pipe's read end now, long before extraction finishes, so the
    // CLI's eventual stdout write hits EPIPE.
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panicked"),
        "CLI panicked on closed stdout: {stderr}"
    );
    assert!(
        output.status.success(),
        "expected quiet success on closed stdout, got {:?} (stderr: {stderr})",
        output.status
    );
}

#[test]
fn pages_selects_absolute_range_and_lean_header_counts_selected_blocks() {
    let assert = dps()
        .arg("extract")
        .arg(testdata("multipage.pdf"))
        .args(["--pages", "2-4", "--format", "lean"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let header = stdout
        .lines()
        .next()
        .expect("lean output has a header line");
    assert!(
        header.contains("pages=3"),
        "expected header to report 3 emitted blocks for a 2-4 selection, got: {header:?}"
    );
    assert!(
        stdout.contains("#page 2 "),
        "expected an absolute #page 2 block, got:\n{stdout}"
    );
    assert!(
        stdout.contains("#page 3 "),
        "expected an absolute #page 3 block, got:\n{stdout}"
    );
    assert!(
        stdout.contains("#page 4 "),
        "expected an absolute #page 4 block, got:\n{stdout}"
    );
    assert!(
        !stdout.contains("#page 1 ")
            && !stdout.contains("#page 5 ")
            && !stdout.contains("#page 6 "),
        "did not expect pages outside the 2-4 selection, got:\n{stdout}"
    );
}

#[test]
fn pages_single_page_selection() {
    let assert = dps()
        .arg("extract")
        .arg(testdata("multipage.pdf"))
        .args(["--pages", "3-3", "--format", "lean"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let header = stdout
        .lines()
        .next()
        .expect("lean output has a header line");
    assert!(
        header.contains("pages=1"),
        "expected header to report 1 emitted block for a 3-3 selection, got: {header:?}"
    );
    assert!(
        stdout.contains("#page 3 "),
        "expected an absolute #page 3 block, got:\n{stdout}"
    );
}

#[test]
fn pages_unparseable_value_exits_7_with_bad_pages_envelope() {
    dps()
        .arg("extract")
        .arg(testdata("multipage.pdf"))
        .args(["--pages", "abc"])
        .assert()
        .code(7)
        .stderr(predicate::str::contains("\"code\":\"bad_pages\""));
}

#[test]
fn pages_out_of_range_exits_9_with_page_out_of_range_envelope() {
    dps()
        .arg("extract")
        .arg(testdata("multipage.pdf"))
        .args(["--pages", "99-100"])
        .assert()
        .code(9)
        .stderr(predicate::str::contains("\"code\":\"page_out_of_range\""));
}

#[test]
fn version_flag_reports_the_crate_version() {
    dps()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}
