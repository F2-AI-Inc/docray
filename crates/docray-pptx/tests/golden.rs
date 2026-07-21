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
    let boxes: Vec<_> = extraction.pages[0]
        .elements
        .iter()
        .map(|element| match element {
            Element::Text(text) => text.bbox,
            _ => panic!("table fixture must emit only text cells"),
        })
        .collect();
    // Frame origin is (72,90). Column widths are 80,120; row heights
    // are 30,50. The first anchor gridSpan=2 covers x=72..272.
    assert_eq!(boxes.len(), 3, "the hMerge continuation is not emitted");
    assert_eq!(
        (boxes[0].x0, boxes[0].y0, boxes[0].x1, boxes[0].y1),
        (72.0, 90.0, 272.0, 120.0)
    );
    assert_eq!(
        (boxes[1].x0, boxes[1].y0, boxes[1].x1, boxes[1].y1),
        (72.0, 120.0, 152.0, 170.0)
    );
    assert_eq!(
        (boxes[2].x0, boxes[2].y0, boxes[2].x1, boxes[2].y1),
        (152.0, 120.0, 272.0, 170.0)
    );
}

#[test]
fn pptx_is_element_only_and_text_has_no_lines() {
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
