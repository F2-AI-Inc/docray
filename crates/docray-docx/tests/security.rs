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
