use docray_core::Extractor;
use docray_pdf::PdfExtractor;
use std::path::PathBuf;

fn ensure_pdfium_dir() {
    if std::env::var_os("DOCRAY_PDFIUM_DIR").is_none() {
        std::env::set_var(
            "DOCRAY_PDFIUM_DIR",
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../.pdfium/lib"),
        );
    }
}

fn extract(fixture: &str) -> docray_model::Extraction {
    ensure_pdfium_dir();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata")
        .join(fixture);
    PdfExtractor
        .extract(&std::fs::read(path).unwrap(), None)
        .unwrap()
}

#[test]
fn full_page_raster_without_text_is_scanned() {
    let out = extract("scan.pdf");
    assert_eq!(out.pages.len(), 1);
    assert!(out.pages[0].scanned);
}

#[test]
fn text_or_small_image_is_not_scanned() {
    assert!(!extract("simple.pdf").pages[0].scanned);
    assert!(!extract("image.pdf").pages[0].scanned);
}
