use docray_core::{ExtractError, Extractor};
use docray_model::Element;
use docray_pptx::PptxExtractor;
use std::fs;
use std::path::PathBuf;

fn fixture(name: &str) -> Vec<u8> {
    fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/malformed")
            .join(name),
    )
    .unwrap()
}

#[test]
fn hostile_containers_return_structured_errors() {
    for (name, expected_fragment) in [
        ("zip-bomb.pptx", "compression-ratio limit"),
        ("path-traversal.pptx", "unsafe OPC entry name"),
        ("truncated.pptx", "invalid ZIP container"),
    ] {
        let error = PptxExtractor.extract(&fixture(name), None).unwrap_err();
        assert_eq!(error.code(), "parse_failure", "{name}: {error}");
        assert!(
            error.to_string().contains(expected_fragment),
            "{name}: {error}"
        );
    }
}

#[test]
fn format_disambiguation_and_cfb_have_specific_unsupported_messages() {
    let error = PptxExtractor
        .extract(&fixture("not-pptx.zip"), None)
        .unwrap_err();
    assert_eq!(error.code(), "unsupported_format");
    assert!(error
        .to_string()
        .contains("zip archive is not a PowerPoint file"));

    let error = PptxExtractor
        .extract(&fixture("legacy-office.cfb"), None)
        .unwrap_err();
    assert_eq!(
        error,
        ExtractError::UnsupportedFormatMessage(
            "legacy or encrypted Office documents are not supported".into()
        )
    );
}

#[test]
fn dtd_entities_are_never_expanded_or_fetched() {
    for name in ["xxe.pptx", "external-entity.pptx"] {
        let extraction = PptxExtractor.extract(&fixture(name), None).unwrap();
        let text = extraction.pages[0]
            .elements
            .iter()
            .find_map(|element| match element {
                Element::Text(text) => Some(text.content.as_str()),
                _ => None,
            })
            .unwrap();
        assert_eq!(text, "&xxe;");
        assert!(!text.contains("EXPANSION_MUST_NOT_APPEAR"));
        assert!(!text.contains("root:"));
    }
}

#[test]
fn escaping_picture_relationships_warn_without_discarding_the_slide() {
    // Both picture paths (p:pic and a picture graphicFrame) reference media
    // through targets that escape the package root. Each must degrade to a
    // per-picture warning while the picture geometry and every other element
    // on the slide survive — one hostile relationship must not blank the page.
    let extraction = PptxExtractor
        .extract(&fixture("escaping-picture.pptx"), None)
        .unwrap();
    let page = &extraction.pages[0];
    assert_eq!(
        page.elements.len(),
        3,
        "both pictures and the survivor shape must be emitted: {:?}",
        extraction.warnings
    );
    let (Element::Image(picture), Element::Image(framed)) = (&page.elements[0], &page.elements[1])
    else {
        panic!("both hostile pictures must still be emitted as images");
    };
    assert_eq!(picture.content_hash, None);
    assert_eq!(framed.content_hash, None);
    let Element::Text(survivor) = &page.elements[2] else {
        panic!("the shape after the hostile pictures must still extract");
    };
    assert_eq!(survivor.content, "Slide still extracts");
    assert!(
        !extraction
            .warnings
            .iter()
            .any(|warning| warning.contains("page 1 failed to parse")),
        "the slide must not be reported as failed: {:?}",
        extraction.warnings
    );
    assert_eq!(
        extraction
            .warnings
            .iter()
            .filter(
                |warning| warning.contains("picture media relationship is invalid")
                    && warning.contains("escapes the package root")
            )
            .count(),
        2,
        "each hostile relationship warns individually: {:?}",
        extraction.warnings
    );
}

#[test]
fn max_pages_caps_slides_before_extraction() {
    let bytes =
        fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata/pptx/basic.pptx"))
            .unwrap();
    assert_eq!(
        PptxExtractor.extract(&bytes, Some(0)),
        Err(ExtractError::TooManyPages {
            limit: 0,
            actual: 1
        })
    );
}
