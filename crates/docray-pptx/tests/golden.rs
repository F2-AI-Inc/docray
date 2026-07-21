use docray_core::{check_granularity, Extractor};
use docray_model::{Element, GranularExtraction, Granularity, HiddenItem};
use docray_pptx::PptxExtractor;
use std::fs;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixtures() -> Vec<PathBuf> {
    let mut fixtures: Vec<_> = fs::read_dir(root().join("testdata/pptx"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("pptx"))
        .collect();
    fixtures.sort();
    fixtures
}

#[test]
fn element_and_lean_goldens_are_byte_exact_on_every_platform() {
    let golden_dir = root().join("testdata/golden/pptx");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        fs::create_dir_all(&golden_dir).unwrap();
    }
    for fixture in fixtures() {
        let name = fixture.file_stem().unwrap().to_str().unwrap();
        let bytes = fs::read(&fixture).unwrap();
        let extraction = PptxExtractor.extract(&bytes, None).unwrap();
        let json = serde_json::to_string_pretty(&extraction.with_granularity(Granularity::Element))
            .unwrap();
        let lean = match extraction.with_granularity(Granularity::Element) {
            GranularExtraction::Compact(compact) => compact.to_lean(),
            GranularExtraction::Char(_) => unreachable!(),
        };
        for (suffix, actual) in [("element.json", json), ("lean.txt", lean)] {
            let path = golden_dir.join(format!("{name}.{suffix}"));
            if std::env::var_os("UPDATE_GOLDEN").is_some() {
                fs::write(&path, &actual).unwrap();
            } else {
                let expected = fs::read_to_string(&path).unwrap_or_else(|_| {
                    panic!("missing golden {path:?}; run with UPDATE_GOLDEN=1")
                });
                assert_eq!(
                    actual, expected,
                    "byte-exact golden mismatch for {name}.{suffix}"
                );
            }
        }
    }
}

#[test]
fn group_fixture_geometry_matches_hand_math() {
    let bytes = fs::read(root().join("testdata/pptx/groups.pptx")).unwrap();
    let extraction = PptxExtractor.extract(&bytes, None).unwrap();
    let Element::Text(text) = &extraction.pages[0].elements[0] else {
        panic!("group fixture must emit text");
    };
    // Child 10,10..50,30 flips and rotates 90 degrees about 30,20 ->
    // AABB 20,0..40,40. Inner maps this to 40,20..60,60. Outer maps
    // x'=10+(x-10)*2 and y'=20+(y-10)*2 -> 70,40..110,120 pt.
    assert_eq!(text.bbox.x0, 70.0);
    assert_eq!(text.bbox.y0, 40.0);
    assert_eq!(text.bbox.x1, 110.0);
    assert_eq!(text.bbox.y1, 120.0);
}

#[test]
fn table_fixture_geometry_matches_prefix_sum_math() {
    let bytes = fs::read(root().join("testdata/pptx/table.pptx")).unwrap();
    let extraction = PptxExtractor.extract(&bytes, None).unwrap();
    assert_eq!(extraction.pages[0].elements.len(), 1);
    let Element::Table(table) = &extraction.pages[0].elements[0] else {
        panic!("table fixture must emit one first-class table");
    };
    // Frame origin is (72,90). Column widths are 80,120; row heights
    // are 30,50. The first anchor gridSpan=2 covers x=72..272.
    assert_eq!(table.rows, 2);
    assert_eq!(table.cols, 2);
    assert_eq!(
        (table.bbox.x0, table.bbox.y0, table.bbox.x1, table.bbox.y1),
        (72.0, 90.0, 272.0, 170.0)
    );
    assert_eq!(
        table
            .cells
            .iter()
            .map(|cell| (cell.row, cell.col, cell.row_span, cell.col_span))
            .collect::<Vec<_>>(),
        vec![(0, 0, 1, 2), (1, 0, 1, 1), (1, 1, 1, 1)],
        "the hMerge continuation is not emitted"
    );
    assert_eq!(
        (
            table.cells[0].bbox.x0,
            table.cells[0].bbox.y0,
            table.cells[0].bbox.x1,
            table.cells[0].bbox.y1
        ),
        (72.0, 90.0, 272.0, 120.0)
    );
    assert_eq!(
        (
            table.cells[1].bbox.x0,
            table.cells[1].bbox.y0,
            table.cells[1].bbox.x1,
            table.cells[1].bbox.y1
        ),
        (72.0, 120.0, 152.0, 170.0)
    );
    assert_eq!(
        (
            table.cells[2].bbox.x0,
            table.cells[2].bbox.y0,
            table.cells[2].bbox.x1,
            table.cells[2].bbox.y1
        ),
        (152.0, 120.0, 272.0, 170.0)
    );
    assert_eq!(
        table
            .cells
            .iter()
            .map(|cell| cell.content.as_str())
            .collect::<Vec<_>>(),
        vec!["Merged", "Left", "Right"]
    );
    assert!(table
        .cells
        .iter()
        .all(|cell| cell.runs.as_ref().is_some_and(|runs| runs.len() == 1)));
}

#[test]
fn graphic_frame_fixtures_preserve_chart_smartart_and_picture_content() {
    let bytes = fs::read(root().join("testdata/pptx/chart.pptx")).unwrap();
    let extraction = PptxExtractor.extract(&bytes, None).unwrap();
    assert!(extraction.warnings.is_empty());
    let Element::Text(chart) = &extraction.pages[0].elements[0] else {
        panic!("chart fixture must emit synthesized text");
    };
    // Frame (72,72) + (360,216) points; chart internals have no authored bbox.
    assert_eq!(
        (chart.bbox.x0, chart.bbox.y0, chart.bbox.x1, chart.bbox.y1),
        (72.0, 72.0, 432.0, 288.0)
    );
    assert_eq!(
        chart.content,
        "Quarterly revenue\nQuarter\nUSD millions\nRevenue\nQ1: 10.5\nQ2: 12\nCosts\nQ1: 7\nQ2: 8.25"
    );
    assert_eq!(chart.runs, None);

    let bytes = fs::read(root().join("testdata/pptx/smartart.pptx")).unwrap();
    let extraction = PptxExtractor.extract(&bytes, None).unwrap();
    assert!(extraction.warnings.is_empty());
    let Element::Text(diagram) = &extraction.pages[0].elements[0] else {
        panic!("SmartArt fixture must emit synthesized text");
    };
    // Frame (100,80) + (400,200) points.
    assert_eq!(
        (
            diagram.bbox.x0,
            diagram.bbox.y0,
            diagram.bbox.x1,
            diagram.bbox.y1
        ),
        (100.0, 80.0, 500.0, 280.0)
    );
    assert_eq!(diagram.content, "Discover\nBuild\nDeliver");
    assert_eq!(diagram.runs, None);

    let bytes = fs::read(root().join("testdata/pptx/graphic-picture.pptx")).unwrap();
    let extraction = PptxExtractor.extract(&bytes, None).unwrap();
    assert!(extraction.warnings.is_empty());
    let Element::Image(image) = &extraction.pages[0].elements[0] else {
        panic!("picture graphicFrame must emit an image");
    };
    // Frame (200,100) + (160,80) points.
    assert_eq!(
        (image.bbox.x0, image.bbox.y0, image.bbox.x1, image.bbox.y1),
        (200.0, 100.0, 360.0, 180.0)
    );
    assert!(image.content_hash.is_some());
    assert_eq!(
        extraction.pages[0].hidden,
        vec![HiddenItem {
            kind: "alt".into(),
            element: Some("p1-e0".into()),
            content: "Graphic-frame picture alternative".into(),
        }]
    );
}

#[test]
fn missing_chart_part_warns_and_the_rest_of_the_slide_survives() {
    let bytes = fs::read(root().join("testdata/pptx/missing-chart.pptx")).unwrap();
    let extraction = PptxExtractor.extract(&bytes, None).unwrap();
    assert_eq!(extraction.pages[0].elements.len(), 1);
    let Element::Text(text) = &extraction.pages[0].elements[0] else {
        panic!("shape after missing chart must still extract");
    };
    assert_eq!(text.content, "Slide still extracts");
    assert_eq!(extraction.warnings.len(), 1);
    assert!(extraction.warnings[0].contains("chart graphicFrame part is missing or unreadable"));
    assert!(extraction.warnings[0].contains("ppt/charts/missing.xml"));
}

#[test]
fn styled_text_fixture_preserves_each_run_style_and_external_href() {
    let bytes = fs::read(root().join("testdata/pptx/styled-text.pptx")).unwrap();
    let extraction = PptxExtractor.extract(&bytes, None).unwrap();
    let Element::Text(text) = &extraction.pages[0].elements[0] else {
        panic!("styled-text fixture must begin with text");
    };
    let runs = text.runs.as_ref().expect("PPTX text must carry runs");
    assert_eq!(runs.len(), 3);
    assert_eq!(runs[0].content, "Hello");
    assert_eq!(runs[0].font.name, "Fixture Serif");
    assert_eq!(runs[0].font.size, 24.0);
    assert!(runs[0].font.bold);
    assert_eq!(runs[0].color.fill, Some([64, 77, 89]));
    assert_eq!(runs[1].content, " theme");
    assert_eq!(runs[1].font.name, "Fixture Sans");
    assert_eq!(runs[1].font.size, 14.4);
    assert_eq!(runs[2].content, "Second paragraph");
    assert!(runs[2].font.italic);
    assert_eq!(
        runs[2].href.as_deref(),
        Some("https://example.com/styled-run")
    );

    let Element::Annotation(annotation) = &extraction.pages[0].elements[1] else {
        panic!("whole-shape hyperlink annotation must remain emitted");
    };
    assert_eq!(
        annotation.uri.as_deref(),
        Some("https://example.com/styled-run")
    );
}

#[test]
fn pptx_is_element_only_and_text_has_runs_but_no_lines() {
    let capabilities = PptxExtractor.capabilities();
    assert_eq!(capabilities.finest_granularity, Granularity::Element);
    assert!(check_granularity(&capabilities, None).is_err());
    assert!(check_granularity(&capabilities, Some(Granularity::Word)).is_err());
    assert_eq!(
        check_granularity(&capabilities, Some(Granularity::Element)),
        Ok(())
    );

    let bytes = fs::read(root().join("testdata/pptx/basic.pptx")).unwrap();
    let extraction = PptxExtractor.extract(&bytes, None).unwrap();
    for element in &extraction.pages[0].elements {
        if let Element::Text(text) = element {
            assert!(text.lines.is_none());
            assert!(text.runs.is_some());
        }
    }
}

#[test]
fn repeated_extraction_is_byte_identical() {
    let bytes = fs::read(root().join("testdata/pptx/styled-text.pptx")).unwrap();
    let first = serde_json::to_vec(&PptxExtractor.extract(&bytes, None).unwrap()).unwrap();
    let second = serde_json::to_vec(&PptxExtractor.extract(&bytes, None).unwrap()).unwrap();
    assert_eq!(first, second);
}

#[test]
fn pptx_emits_notes_alt_text_roles_and_hidden_slide_markers() {
    let hidden_context = fs::read(root().join("testdata/pptx/hidden-context.pptx")).unwrap();
    let extraction = PptxExtractor.extract(&hidden_context, None).unwrap();
    assert_eq!(
        extraction.pages[0].hidden,
        vec![
            HiddenItem {
                kind: "role".into(),
                element: Some("p1-e0".into()),
                content: "body".into(),
            },
            HiddenItem {
                kind: "alt".into(),
                element: Some("p1-e0".into()),
                content: "Shape alternative text".into(),
            },
            HiddenItem {
                kind: "alt".into(),
                element: Some("p1-e1".into()),
                content: "Chart showing Q3 revenue".into(),
            },
            HiddenItem {
                kind: "notes".into(),
                element: None,
                content: "Presenter script line one\nline two".into(),
            },
            HiddenItem {
                kind: "hidden-slide".into(),
                element: None,
                content: "true".into(),
            },
        ]
    );
    assert!(!extraction.pages[0]
        .hidden
        .iter()
        .any(|item| item.content.contains("IGNORE")));

    let placeholders = fs::read(root().join("testdata/pptx/placeholders.pptx")).unwrap();
    let extraction = PptxExtractor.extract(&placeholders, None).unwrap();
    assert_eq!(
        extraction.pages[0].hidden,
        vec![
            HiddenItem {
                kind: "role".into(),
                element: Some("p1-e0".into()),
                content: "ctrTitle".into(),
            },
            HiddenItem {
                kind: "role".into(),
                element: Some("p1-e1".into()),
                content: "body".into(),
            },
        ]
    );
}
