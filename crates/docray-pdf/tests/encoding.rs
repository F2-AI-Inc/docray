use docray_core::Extractor;
use docray_model::Element;
use docray_pdf::PdfExtractor;

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
    let path = format!("{}/../../testdata/{fixture}", env!("CARGO_MANIFEST_DIR"));
    let bytes = std::fs::read(path).unwrap();
    PdfExtractor.extract(&bytes, None, None).unwrap()
}

#[test]
fn warns_when_a_page_is_dominated_by_unmapped_glyphs() {
    let out = extract("broken-encoding.pdf");
    assert_eq!(
        out.warnings.len(),
        1,
        "unexpected warnings: {:?}",
        out.warnings
    );
    assert!(
        out.warnings[0].starts_with("page 1: suspected_garbled_text:"),
        "warning must identify the affected page and stable warning kind: {:?}",
        out.warnings
    );
    assert!(
        out.warnings[0].contains("12 of 12 glyphs"),
        "warning must report the evidence: {:?}",
        out.warnings
    );
}

#[test]
fn clean_and_cjk_text_do_not_trigger_garbled_text_warning() {
    let clean = extract("simple.pdf");
    assert!(
        clean.warnings.is_empty(),
        "clean warnings: {:?}",
        clean.warnings
    );

    let cjk = extract("cjk.pdf");
    assert!(cjk.warnings.is_empty(), "CJK warnings: {:?}", cjk.warnings);
    let text = cjk.pages[0]
        .elements
        .iter()
        .filter_map(|element| match element {
            Element::Text(text) => Some(text.content.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert_eq!(text, "你好世界漢字文本");
}
