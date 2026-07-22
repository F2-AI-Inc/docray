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
fn lean_defaults_to_element_and_emits_fixed_header_lines() {
    let assert = dps()
        .arg("extract")
        .arg(testdata("simple.pdf"))
        .args(["--format", "lean"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let mut lines = stdout.lines();
    assert_eq!(lines.next(), Some("#docray element v1.6 pages=1"));
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
fn pptx_requires_element_granularity_and_supports_lean() {
    for args in [
        Vec::<&str>::new(),
        vec!["--granularity", "word"],
        vec!["--granularity", "char"],
    ] {
        dps()
            .arg("extract")
            .arg(testdata("pptx/basic.pptx"))
            .args(args)
            .assert()
            .code(8)
            .stderr(predicate::str::contains("\"granularity_unavailable\""))
            .stderr(predicate::str::contains("retry with granularity=element"));
    }

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
        .stdout(predicate::str::starts_with("#docray element v1.6 pages=1"));
}
