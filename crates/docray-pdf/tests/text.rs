use docray_core::{check_granularity, Extractor, GeometryKind};
use docray_model::{Element, Granularity};
use docray_pdf::PdfExtractor;

/// cargo runs test binaries with CWD = crate dir, so bind()'s relative
/// `./.pdfium/lib` candidate never resolves against the workspace-root lib.
/// Point the documented env var at the workspace-root .pdfium/lib.
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
    PdfExtractor.extract(&bytes, None).unwrap()
}

#[test]
fn extracts_text_hierarchy_from_simple_pdf() {
    let out = extract("simple.pdf");
    assert_eq!(out.schema_version, "1.1");
    assert_eq!(out.source.format, "pdf");
    assert_eq!(out.document.page_count, 1);
    let page = &out.pages[0];
    assert_eq!((page.width, page.height), (612.0, 792.0));

    let texts: Vec<_> = page
        .elements
        .iter()
        .filter_map(|e| match e {
            Element::Text(t) => Some(t),
            _ => None,
        })
        .collect();
    let all: String = texts
        .iter()
        .map(|t| t.content.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(all.contains("Hello World"), "got: {all}");
    assert!(all.contains("Bold Title"));

    // "Hello World" -> one line, two words, chars have boxes.
    let hello = texts.iter().find(|t| t.content.contains("Hello")).unwrap();
    assert!(
        texts.iter().all(|text| text.runs.is_none()),
        "PDF must not populate granularity-shaped run detail"
    );
    let lines = hello
        .lines
        .as_ref()
        .expect("PDF text must include the full hierarchy");
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].words.len(), 2);
    assert_eq!(lines[0].words[0].content, "Hello");
    let c0 = &lines[0].words[0].chars[0];
    assert_eq!(c0.content, "H");
    assert!(c0.bbox.x1 > c0.bbox.x0 && c0.bbox.y1 > c0.bbox.y0);

    // Coordinates are top-left: text drawn near y=720 in PDF space (page height 792)
    // must have y0 around 792-732=60, definitely < 100.
    assert!(hello.bbox.y0 < 100.0, "y not flipped: {:?}", hello.bbox);

    // Baseline derives from each char's origin y (flipped to top-left space),
    // so it must sit within the line's glyph box — near the bottom for this
    // ascender-only text. Pins the F4 origin-based baseline against regression.
    let baseline = lines[0].baseline_y;
    assert!(
        baseline >= hello.bbox.y0 && baseline <= hello.bbox.y1 + 1.0,
        "baseline {baseline} outside bbox {:?}",
        hello.bbox
    );

    // Font metadata.
    assert!(hello.font.name.to_lowercase().contains("helvetica"));
    assert_eq!(hello.font.size, 12.0);
    let bold = texts.iter().find(|t| t.content.contains("Bold")).unwrap();
    assert!(bold.font.bold);
    assert_eq!(hello.color.fill, Some([0, 0, 0]));

    // IDs in z-order.
    assert!(page.elements.iter().enumerate().all(|(i, e)| {
        let id = match e {
            Element::Text(t) => &t.id,
            Element::Table(t) => &t.id,
            Element::Chart(t) => &t.id,
            Element::Image(t) => &t.id,
            Element::Path(t) => &t.id,
            Element::Annotation(t) => &t.id,
        };
        id == &format!("p1-e{i}")
    }));
}

#[test]
fn pdf_capabilities_accept_every_granularity() {
    let capabilities = PdfExtractor.capabilities();
    assert_eq!(capabilities.geometry, GeometryKind::Exact);
    for requested in [
        None,
        Some(Granularity::Char),
        Some(Granularity::Word),
        Some(Granularity::Element),
    ] {
        assert_eq!(check_granularity(&capabilities, requested), Ok(()));
    }
}

#[test]
fn rejects_garbage_and_respects_page_cap() {
    use docray_core::ExtractError;
    ensure_pdfium_dir();
    let garbage = std::fs::read(format!(
        "{}/../../testdata/malformed/garbage.bin",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    assert!(matches!(
        PdfExtractor.extract(&garbage, None),
        Err(ExtractError::UnsupportedFormat)
    ));

    let bytes = std::fs::read(format!(
        "{}/../../testdata/simple.pdf",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    assert!(matches!(
        PdfExtractor.extract(&bytes, Some(0)),
        Err(ExtractError::TooManyPages {
            limit: 0,
            actual: 1
        })
    ));
}
