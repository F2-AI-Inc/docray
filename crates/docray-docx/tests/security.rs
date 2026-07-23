use docray_core::Extractor;
use docray_docx::DocxExtractor;
use docray_model::Block;
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
fn hostile_fields_are_bounded_and_instructions_are_never_visible() {
    let extraction = DocxExtractor
        .extract(&fixture("docx-field-nesting.docx"), None)
        .unwrap();
    let visible = extraction.sections[0]
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Paragraph { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert!(!visible.contains("SECRET"));
    assert!(extraction
        .warnings
        .iter()
        .any(|warning| warning.contains("field nesting depth limit")));
    assert!(extraction.sections[0]
        .hidden
        .iter()
        .any(|item| item.kind == "field" && item.content == "SECRET"));
}

#[test]
fn style_numbering_and_mc_attacks_warn_without_panicking_or_hanging() {
    for (name, expected) in [
        ("docx-style-cycle.docx", "style basedOn cycle"),
        ("docx-numbering-cycle.docx", "abstract numbering definition"),
        (
            "docx-mc-bomb.docx",
            "markup-compatibility nesting depth limit",
        ),
    ] {
        let extraction = DocxExtractor.extract(&fixture(name), None).unwrap();
        assert!(
            extraction
                .warnings
                .iter()
                .any(|warning| warning.contains(expected)),
            "{name}: {:?}",
            extraction.warnings
        );
    }
}

#[test]
fn huge_story_stops_at_the_documented_block_cap() {
    let extraction = DocxExtractor
        .extract(&fixture("docx-huge-story.docx"), None)
        .unwrap();
    assert!(extraction.sections[0].blocks.len() <= 250_000);
    assert!(extraction
        .warnings
        .iter()
        .any(|warning| warning.contains("story block limit 250000 exceeded")));
}

#[test]
fn pagination_hints_do_not_bypass_the_max_pages_block_cap() {
    assert!(matches!(
        DocxExtractor.extract(&fixture("docx-hinted-block-cap.docx"), Some(2)),
        Err(docray_core::ExtractError::TooManyPages {
            limit: 2,
            actual: 3
        })
    ));
}

#[test]
fn malformed_image_target_keeps_the_block_and_warns() {
    let extraction = DocxExtractor
        .extract(&fixture("docx-bad-image-target.docx"), None)
        .unwrap();
    assert!(matches!(
        extraction.sections[0].blocks.first(),
        Some(Block::Image {
            content_hash: None,
            ..
        })
    ));
    assert!(extraction.warnings.iter().any(|warning| warning
        .contains("image relationship target")
        && warning.contains("invalid")));
}

#[test]
fn corrupt_optional_parts_and_header_target_degrade_to_warnings() {
    let extraction = DocxExtractor
        .extract(&fixture("docx-corrupt-optional-parts.docx"), None)
        .unwrap();
    let Block::Paragraph { runs, content, .. } = &extraction.sections[0].blocks[0] else {
        panic!("valid body content must survive optional-part failures");
    };
    assert_eq!(content, "defaults survive");
    assert_eq!(runs[0].font.name, "Calibri");
    assert_eq!(runs[0].font.size, 11.0);
    assert!(!runs[0].font.bold);
    assert!(!runs[0].font.italic);
    for part in ["word/styles.xml", "docProps/core.xml"] {
        assert!(
            extraction
                .warnings
                .iter()
                .any(|warning| warning.contains(part) && warning.contains("failed to parse")),
            "missing warning for {part}: {:?}",
            extraction.warnings
        );
    }
    assert!(extraction.warnings.iter().any(|warning| {
        warning.contains("headerReference target") && warning.contains("invalid")
    }));
}
