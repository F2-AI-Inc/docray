use docray_pdf::bind::pdfium;

#[test]
fn binds_and_opens_fixture() {
    // cargo runs test binaries with CWD = crate dir, so bind()'s relative
    // `./.pdfium/lib` candidate never resolves against the workspace-root lib.
    // Point the documented env var at the workspace-root .pdfium/lib.
    if std::env::var_os("DOCRAY_PDFIUM_DIR").is_none() {
        std::env::set_var(
            "DOCRAY_PDFIUM_DIR",
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../.pdfium/lib"),
        );
    }
    let p = pdfium().expect("pdfium binding failed - did you run scripts/fetch-pdfium.sh?");
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../testdata/simple.pdf"
    ))
    .unwrap();
    let doc = p.load_pdf_from_byte_slice(&bytes, None).unwrap();
    assert_eq!(doc.pages().len(), 1);
}
