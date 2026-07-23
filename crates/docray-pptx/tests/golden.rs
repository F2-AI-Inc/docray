use docray_core::{check_granularity, Extractor, GeometryKind};
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
            GranularExtraction::Flow(_) => unreachable!(),
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
fn inherited_shapes_use_template_order_relationships_and_provenance() {
    let bytes = fs::read(root().join("testdata/pptx/inherited-shapes.pptx")).unwrap();
    let extraction = PptxExtractor.extract(&bytes, None).unwrap();
    assert!(extraction.warnings.is_empty());
    let page = &extraction.pages[0];
    assert_eq!(page.elements.len(), 5);

    let Element::Text(master) = &page.elements[0] else {
        panic!("master shape must be first in z-order");
    };
    assert_eq!(
        (master.id.as_str(), master.content.as_str()),
        ("p1-e0", "Master brand")
    );

    let Element::Image(layout_picture) = &page.elements[1] else {
        panic!("layout picture must follow master shapes");
    };
    assert_eq!(layout_picture.id, "p1-e1");
    assert!(
        layout_picture.content_hash.is_some(),
        "the image relationship must resolve relative to the layout part"
    );

    let Element::Path(layout_background) = &page.elements[2] else {
        panic!("layout background must use the ordinary shape path");
    };
    assert_eq!(layout_background.id, "p1-e2");
    assert_eq!(layout_background.fill, Some([230, 230, 230]));

    let Element::Path(layout_rule) = &page.elements[3] else {
        panic!("layout connector must use the ordinary connector path");
    };
    assert_eq!(layout_rule.id, "p1-e3");
    assert_eq!(layout_rule.stroke, Some([51, 102, 153]));

    let Element::Text(slide_title) = &page.elements[4] else {
        panic!("the slide's own title must follow inherited shapes");
    };
    assert_eq!(
        (slide_title.id.as_str(), slide_title.content.as_str()),
        ("p1-e4", "Slide title")
    );
    assert!(page.elements.iter().all(|element| match element {
        Element::Text(text) => !text.content.contains("Click to edit"),
        _ => true,
    }));
    assert_eq!(
        page.hidden,
        vec![
            HiddenItem {
                kind: "source-layer".into(),
                element: Some("p1-e0".into()),
                content: "master".into(),
            },
            HiddenItem {
                kind: "source-layer".into(),
                element: Some("p1-e1".into()),
                content: "layout".into(),
            },
            HiddenItem {
                kind: "source-layer".into(),
                element: Some("p1-e2".into()),
                content: "layout".into(),
            },
            HiddenItem {
                kind: "source-layer".into(),
                element: Some("p1-e3".into()),
                content: "layout".into(),
            },
            HiddenItem {
                kind: "role".into(),
                element: Some("p1-e4".into()),
                content: "title".into(),
            },
        ]
    );
}

#[test]
fn show_master_sp_gates_master_and_layout_shapes_independently() {
    let bytes = fs::read(root().join("testdata/pptx/layout-hides-master-shapes.pptx")).unwrap();
    let extraction = PptxExtractor.extract(&bytes, None).unwrap();
    let page = &extraction.pages[0];
    let contents = page
        .elements
        .iter()
        .filter_map(|element| match element {
            Element::Text(text) => Some(text.content.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(contents, vec!["Layout gated", "Slide own"]);
    assert_eq!(
        page.hidden,
        vec![HiddenItem {
            kind: "source-layer".into(),
            element: Some("p1-e0".into()),
            content: "layout".into(),
        }]
    );

    let bytes = fs::read(root().join("testdata/pptx/slide-hides-template-shapes.pptx")).unwrap();
    let extraction = PptxExtractor.extract(&bytes, None).unwrap();
    let page = &extraction.pages[0];
    let Element::Text(slide) = &page.elements[0] else {
        panic!("slide-owned shape must remain visible");
    };
    assert_eq!(page.elements.len(), 1);
    assert_eq!(slide.content, "Slide own");
    assert!(page.hidden.is_empty());
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
fn chart_values_render_with_their_percent_format() {
    // Regression: values stored as fractions (0.41) with a "0%" format must
    // render as percentages, not raw/scientific-notation floats.
    let bytes = fs::read(root().join("testdata/pptx/percent-chart.pptx")).unwrap();
    let extraction = PptxExtractor.extract(&bytes, None).unwrap();
    let Element::Chart(chart) = &extraction.pages[0].elements[0] else {
        panic!("percent chart must emit a first-class chart");
    };
    assert_eq!(chart.chart_type, "doughnut");
    assert_eq!(chart.title, None);
    assert_eq!(chart.series.len(), 1);
    assert_eq!(chart.series[0].name, None);
    assert_eq!(
        chart.series[0].points,
        vec![
            docray_model::ChartPoint {
                category: Some("Direct".into()),
                value: "41%".into(),
            },
            docray_model::ChartPoint {
                category: Some("Reseller".into()),
                value: "59%".into(),
            },
        ]
    );
}

#[test]
fn hidden_shapes_are_skipped_without_warnings() {
    // Regression: hidden shapes (cNvPr hidden="1") — including think-cell's
    // hidden OLE data objects — must be skipped silently, not extracted and not
    // warned about (this deck class produced a warning per slide before).
    let bytes = fs::read(root().join("testdata/pptx/hidden-shapes.pptx")).unwrap();
    let extraction = PptxExtractor.extract(&bytes, None).unwrap();
    assert!(
        extraction.warnings.is_empty(),
        "hidden shapes must not warn: {:?}",
        extraction.warnings
    );
    let texts: Vec<&str> = extraction.pages[0]
        .elements
        .iter()
        .filter_map(|element| match element {
            Element::Text(text) => Some(text.content.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, vec!["Visible"], "hidden text must not be extracted");
    assert!(
        !extraction.pages[0]
            .elements
            .iter()
            .any(|element| matches!(element, Element::Table(_) | Element::Image(_))),
        "the hidden OLE frame must not produce an element"
    );
}

#[test]
fn autoheight_rows_are_extracted_not_dropped() {
    // Regression: PowerPoint writes h="0" for auto-height rows. The extractor
    // must derive heights from the frame extent and emit the table, not skip it.
    let bytes = fs::read(root().join("testdata/pptx/autoheight-table.pptx")).unwrap();
    let extraction = PptxExtractor.extract(&bytes, None).unwrap();
    assert!(
        extraction.warnings.is_empty(),
        "a sized frame with auto-height rows needs no warning: {:?}",
        extraction.warnings
    );
    let Element::Table(table) = &extraction.pages[0].elements[0] else {
        panic!("auto-height table must still be extracted as a first-class table");
    };
    assert_eq!((table.rows, table.cols), (2, 2));
    assert_eq!(
        table
            .cells
            .iter()
            .map(|cell| cell.content.as_str())
            .collect::<Vec<_>>(),
        vec!["A1", "B1", "A2", "B2"]
    );
    // Frame is 1016000 EMU tall (80pt); two auto rows split it -> 40pt each.
    assert_eq!(table.cells[0].bbox.y0, 72.0);
    assert_eq!(table.cells[0].bbox.y1, 112.0);
    assert_eq!(table.cells[2].bbox.y0, 112.0);
    assert_eq!(table.cells[2].bbox.y1, 152.0);
}

#[test]
fn graphic_frame_fixtures_preserve_chart_smartart_and_picture_content() {
    let bytes = fs::read(root().join("testdata/pptx/chart.pptx")).unwrap();
    let extraction = PptxExtractor.extract(&bytes, None).unwrap();
    assert!(extraction.warnings.is_empty());
    let Element::Chart(chart) = &extraction.pages[0].elements[0] else {
        panic!("chart fixture must emit a first-class chart");
    };
    // Frame (72,72) + (360,216) points; chart internals have no authored bbox.
    assert_eq!(
        (chart.bbox.x0, chart.bbox.y0, chart.bbox.x1, chart.bbox.y1),
        (72.0, 72.0, 432.0, 288.0)
    );
    assert_eq!(chart.chart_type, "bar");
    assert_eq!(chart.title.as_deref(), Some("Quarterly revenue"));
    assert_eq!(
        chart
            .series
            .iter()
            .map(|series| series.name.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("Revenue"), Some("Costs")]
    );
    assert_eq!(
        chart
            .series
            .iter()
            .flat_map(|series| series.points.iter())
            .map(|point| (point.category.as_deref(), point.value.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (Some("Q1"), "10.5"),
            (Some("Q2"), "12"),
            (Some("Q1"), "7"),
            (Some("Q2"), "8.25"),
        ]
    );

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
    // Missing part -> warn; valid-but-empty chart part -> warn (rule 3: an
    // extracted graphic that yields no text must not vanish silently).
    assert_eq!(extraction.warnings.len(), 2);
    assert!(extraction.warnings.iter().any(|w| w
        .contains("chart graphicFrame part is missing or unreadable")
        && w.contains("ppt/charts/missing.xml")));
    assert!(extraction
        .warnings
        .iter()
        .any(|w| w == "page 1: chart graphicFrame has no extractable text"));
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
    assert_eq!(capabilities.geometry, GeometryKind::Container);
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
