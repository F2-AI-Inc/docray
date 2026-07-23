use docray_core::{check_granularity, Extractor, GeometryKind};
use docray_docx::DocxExtractor;
use docray_model::{Block, GranularExtraction, Granularity, ListKind};
use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixtures() -> Vec<PathBuf> {
    let mut fixtures: Vec<_> = fs::read_dir(root().join("testdata/docx"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("docx" | "docm")
            )
        })
        .collect();
    fixtures.sort();
    fixtures
}

fn extract(path: impl AsRef<Path>) -> docray_model::FlowExtraction {
    DocxExtractor
        .extract(&fs::read(path).unwrap(), None)
        .unwrap()
}

fn fixture(name: &str) -> docray_model::FlowExtraction {
    extract(root().join("testdata/docx").join(name))
}

#[test]
fn element_and_lean_goldens_are_byte_exact_on_every_platform() {
    let golden_dir = root().join("testdata/golden/docx");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        fs::create_dir_all(&golden_dir).unwrap();
    }
    for fixture in fixtures() {
        let name = fixture.file_stem().unwrap().to_str().unwrap();
        let extraction = extract(&fixture);
        let projected = extraction.with_granularity(Granularity::Element).unwrap();
        let json = serde_json::to_string_pretty(&projected).unwrap();
        let lean = match extraction.with_granularity(Granularity::Element).unwrap() {
            GranularExtraction::Flow(flow) => flow.to_lean(),
            _ => unreachable!(),
        };
        for (suffix, actual) in [("element.json", json), ("lean.txt", lean)] {
            let path = golden_dir.join(format!("{name}.{suffix}"));
            if std::env::var_os("UPDATE_GOLDEN").is_some() {
                fs::write(&path, &actual).unwrap();
            } else {
                let expected = fs::read_to_string(&path).unwrap_or_else(|_| {
                    panic!("missing golden {path:?}; run with UPDATE_GOLDEN=1")
                });
                assert_eq!(actual, expected, "golden mismatch for {name}.{suffix}");
            }
        }
    }
}

#[test]
fn capabilities_are_element_only_flow() {
    let capabilities = DocxExtractor.capabilities();
    assert_eq!(capabilities.finest_granularity, Granularity::Element);
    assert_eq!(capabilities.geometry, GeometryKind::Flow);
    assert!(check_granularity(&capabilities, None).is_err());
    assert!(check_granularity(&capabilities, Some(Granularity::Word)).is_err());
    assert_eq!(
        check_granularity(&capabilities, Some(Granularity::Element)),
        Ok(())
    );
}

#[test]
fn styles_toggle_theme_defaults_and_complex_script_resolve() {
    let extraction = fixture("styles.docx");
    let Block::Paragraph { runs, .. } = &extraction.sections[0].blocks[0] else {
        panic!("styles fixture must be a paragraph")
    };
    assert_eq!(runs[0].font.name, "Fixture Serif");
    assert_eq!(runs[0].font.size, 14.0);
    assert!(!runs[0].font.bold, "style bold + direct bold toggles off");
    assert!(runs[0].font.italic);
    assert!(runs[1].font.bold);

    let rtl = fixture("rtl.docx");
    let Block::Paragraph { content, runs, .. } = &rtl.sections[0].blocks[0] else {
        panic!("rtl fixture must be a paragraph")
    };
    assert_eq!(content, "שלום logical", "logical order is preserved");
    assert_eq!(runs[0].font.name, "Fixture Arabic");
    assert_eq!(runs[0].font.size, 15.0);
    assert!(runs[0].font.bold);
}

#[test]
fn numbering_roles_and_restarts_are_resolved() {
    let extraction = fixture("numbering.docx");
    let lists: Vec<_> = extraction.sections[0]
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Paragraph { list, .. } => list.as_ref(),
            _ => None,
        })
        .collect();
    assert_eq!(
        lists
            .iter()
            .map(|list| list.label.as_str())
            .collect::<Vec<_>>(),
        ["1.", "1.a)", "1.a.I.", "3.", "•"]
    );
    assert!(lists[..4].iter().all(|list| list.kind == ListKind::Ordered));
    assert_eq!(lists[4].kind, ListKind::Bullet);
    assert!(extraction
        .warnings
        .iter()
        .any(|warning| warning.contains("picture bullet")));

    let roles = fixture("roles.docx");
    let roles: Vec<_> = roles.sections[0]
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Paragraph { role, .. } => Some(role.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(roles, ["h1", "h2", "title", "quote"]);
}

#[test]
fn fields_emit_cached_results_and_hide_instructions() {
    let extraction = fixture("fields.docx");
    let visible = extraction.sections[0]
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Paragraph { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(visible, "3 Cached heading");
    assert!(!visible.contains("PAGE"));
    assert!(!visible.contains("TOC"));
    let fields: Vec<_> = extraction.sections[0]
        .hidden
        .iter()
        .filter(|item| item.kind == "field")
        .map(|item| item.content.as_str())
        .collect();
    assert_eq!(fields, ["PAGE", "TOC \\o \"1-3\""]);
}

#[test]
fn mc_revisions_comments_and_run_merging_never_duplicate_visible_text() {
    let mc = fixture("mc.docx");
    let serialized = serde_json::to_string(&mc).unwrap();
    assert_eq!(serialized.matches("Choice text").count(), 2); // content + run
    assert_eq!(serialized.matches("VML fallback selected").count(), 2);
    assert!(!serialized.contains("Unselected fallback"));
    assert!(!serialized.contains("Unselected choice"));

    let revisions = fixture("revisions-comments.docx");
    let serialized = serde_json::to_string(&revisions.sections[0].blocks).unwrap();
    assert!(serialized.contains("inserted"));
    assert!(!serialized.contains("deleted"));
    assert_eq!(
        revisions.sections[0]
            .hidden
            .iter()
            .filter(|item| item.kind == "comment")
            .count(),
        1
    );
    assert!(revisions.sections[0]
        .hidden
        .iter()
        .any(|item| item.kind == "tracked-delete" && item.content == "deleted"));

    let merged = fixture("run-merge.docx");
    let Block::Paragraph { runs, .. } = &merged.sections[0].blocks[0] else {
        panic!("run fixture must be a paragraph")
    };
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].content, "ABC");
}

#[test]
fn tables_images_hyperlinks_and_authored_placement_are_preserved() {
    let tables = fixture("tables.docx");
    let Block::Table {
        cells, placement, ..
    } = &tables.sections[0].blocks[0]
    else {
        panic!("table fixture must be a table")
    };
    assert_eq!(cells[0].row_span, 2);
    assert!(
        cells[1].blocks.is_some(),
        "nested table lives in cells.blocks"
    );
    assert_eq!(placement.as_ref().unwrap().x, Some(36.0));

    let images = fixture("images.docx");
    assert!(matches!(
        &images.sections[0].blocks[0],
        Block::Image {
            content_hash: Some(_),
            placement: None,
            ..
        }
    ));
    assert!(matches!(
        &images.sections[0].blocks[1],
        Block::Image {
            placement: Some(_),
            ..
        }
    ));
    assert!(matches!(
        &images.sections[0].blocks[2],
        Block::Image {
            content_hash: None,
            ..
        }
    ));
    assert!(matches!(
        &images.sections[0].blocks[3],
        Block::Image {
            content_hash: None,
            ..
        }
    ));
    assert!(images
        .warnings
        .iter()
        .any(|warning| warning.contains("relationship is missing or broken")));
    assert!(images
        .warnings
        .iter()
        .any(|warning| warning.contains("external image target was not fetched")));
    assert_eq!(
        images.sections[0]
            .hidden
            .iter()
            .filter(|item| item.kind == "alt")
            .count(),
        3
    );

    let hyperlinks = fixture("hyperlinks.docx");
    let Block::Paragraph { runs, .. } = &hyperlinks.sections[0].blocks[0] else {
        panic!("link fixture must be a paragraph")
    };
    assert_eq!(
        runs[0].href.as_deref(),
        Some("https://example.test/literal?x=1&y=2")
    );
    assert_eq!(runs[2].href.as_deref(), Some("#bookmark"));
}

#[test]
fn sections_stories_notes_breaks_and_docm_policy_are_explicit() {
    let stories = fixture("stories.docx");
    assert_eq!(stories.sections[0].headers.len(), 2);
    assert_eq!(stories.sections[0].footers.len(), 1);
    assert!(stories.sections[0]
        .hidden
        .iter()
        .any(|item| item.kind == "footnote"));
    assert!(stories.sections[0].blocks.iter().any(
        |block| matches!(block, Block::Paragraph { content, .. } if content == "Footnote body")
    ));
    assert!(stories.sections[0].blocks.iter().any(
        |block| matches!(block, Block::Paragraph { content, .. } if content == "Endnote body")
    ));

    let sections = fixture("sections.docx");
    assert_eq!(sections.sections.len(), 2);
    assert_eq!(sections.sections[0].columns, Some(2));
    assert!(matches!(
        sections.sections[0].blocks.last(),
        Some(Block::Break {
            kind: docray_model::BreakKind::Section
        })
    ));

    let breaks = fixture("breaks.docx");
    assert_eq!(
        breaks.approx_pages,
        Some(3),
        "a trailing rendered-page marker still advances top-level provenance"
    );
    assert!(
        matches!(&breaks.sections[0].blocks[1], Block::Paragraph { breaks_before, .. } if breaks_before.contains(&docray_model::BreakKind::Page))
    );
    let zero = fixture("zero-hints.docx");
    assert_eq!(zero.approx_pages, None);
    assert_eq!(
        zero.warnings
            .iter()
            .filter(|warning| warning.as_str() == "no pagination hints; approx_page omitted")
            .count(),
        1
    );

    let macro_doc = fixture("macro.docm");
    assert_eq!(macro_doc.source.format, "docm");
    assert!(macro_doc
        .warnings
        .iter()
        .any(|warning| warning == "macro project ignored"));
}

#[test]
fn repeated_extraction_and_max_pages_policy_are_deterministic() {
    let bytes = fs::read(root().join("testdata/docx/breaks.docx")).unwrap();
    let first = serde_json::to_vec(&DocxExtractor.extract(&bytes, None).unwrap()).unwrap();
    let second = serde_json::to_vec(&DocxExtractor.extract(&bytes, None).unwrap()).unwrap();
    assert_eq!(first, second);
    assert!(matches!(
        DocxExtractor.extract(&bytes, Some(1)),
        Err(docray_core::ExtractError::TooManyPages {
            limit: 1,
            actual: 3
        })
    ));

    let zero = fs::read(root().join("testdata/docx/zero-hints.docx")).unwrap();
    let extraction = DocxExtractor.extract(&zero, Some(1)).unwrap();
    assert!(extraction
        .warnings
        .iter()
        .any(|warning| warning == "max_pages approximated as block cap for flow documents"));
}
