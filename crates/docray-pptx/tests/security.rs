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
