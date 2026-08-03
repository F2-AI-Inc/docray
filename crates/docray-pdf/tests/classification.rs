use docray_core::Extractor;
use docray_model::{PageClassification, PageKind};
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

fn classification(fixture: &str) -> PageClassification {
    ensure_pdfium_dir();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata")
        .join(fixture);
    let extraction = PdfExtractor
        .extract(&std::fs::read(path).unwrap(), None)
        .unwrap();
    let value = serde_json::to_value(extraction.with_classification(None)).unwrap();
    serde_json::from_value(value["pages"][0]["classification"].clone()).unwrap()
}

#[test]
fn hand_verified_fixture_classifications_cover_all_page_kinds_and_garble() {
    let cases = [
        ("simple.pdf", PageKind::Text, false),
        ("scan.pdf", PageKind::Scanned, true),
        ("image.pdf", PageKind::Image, true),
        ("mixed.pdf", PageKind::Mixed, true),
    ];
    let mut confidences = Vec::new();
    for (fixture, kind, needs_ocr) in cases {
        let actual = classification(fixture);
        assert_eq!(actual.kind, kind, "{fixture}: {actual:?}");
        assert_eq!(actual.needs_ocr, needs_ocr, "{fixture}: {actual:?}");
        assert!((0.0..=1.0).contains(&actual.confidence));
        confidences.push(actual.confidence);
    }
    confidences.sort_by(f64::total_cmp);
    confidences.dedup();
    assert!(
        confidences.len() > 1,
        "confidence must be a decision margin, not a branch constant"
    );

    let garbled = classification("broken-encoding.pdf");
    assert!(garbled.needs_ocr, "{garbled:?}");
    assert!(
        garbled
            .reasons
            .iter()
            .any(|reason| reason == "suspected_garbled_text"),
        "{garbled:?}"
    );
}

#[test]
fn classification_is_schema_1_8_and_legacy_scanned_is_unchanged() {
    ensure_pdfium_dir();
    let bytes =
        std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata/scan.pdf"))
            .unwrap();
    let extraction = PdfExtractor.extract(&bytes, None).unwrap();
    let default = serde_json::to_value(&extraction).unwrap();
    let classified = serde_json::to_value(extraction.with_classification(None)).unwrap();

    assert_eq!(default["schema_version"], "1.1");
    assert!(default["pages"][0].get("classification").is_none());
    assert_eq!(classified["schema_version"], "1.8");
    assert_eq!(
        classified["pages"][0]["scanned"],
        default["pages"][0]["scanned"]
    );
}
