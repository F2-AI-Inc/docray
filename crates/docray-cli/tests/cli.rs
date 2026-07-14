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
fn explicit_char_has_v1_2_envelope_and_lossless_hierarchy() {
    dps()
        .arg("extract")
        .arg(testdata("simple.pdf"))
        .args(["--granularity", "char"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"schema_version\":\"1.2\""))
        .stdout(predicate::str::contains("\"granularity\":\"char\""))
        .stdout(predicate::str::contains("\"chars\""));
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
