use docray_model::*;

fn extraction() -> Extraction {
    Extraction {
        schema_version: "1.1".into(),
        source: Source {
            format: "pdf".into(),
            sha256: "not-rendered".into(),
            size_bytes: 123,
        },
        document: DocumentInfo {
            page_count: 1,
            metadata: DocMetadata {
                title: None,
                author: None,
            },
        },
        warnings: vec!["recovered\n\twith omissions".into()],
        pages: vec![Page {
            page_number: 1,
            width: 612.04,
            height: 792.06,
            rotation: 90,
            scanned: true,
            elements: vec![
                Element::Text(TextElement {
                    id: "p1-e0".into(),
                    bbox: BBox {
                        x0: 1.04,
                        y0: 2.06,
                        x1: 30.04,
                        y1: 40.06,
                    },
                    content: "a\\b\nc\td".into(),
                    font: Font {
                        name: "A B\tC".into(),
                        size: 12.06,
                        bold: true,
                        italic: true,
                    },
                    color: TextColor {
                        fill: Some([35, 31, 32]),
                        stroke: Some([255, 0, 0]),
                    },
                    lines: vec![Line {
                        bbox: BBox {
                            x0: 1.04,
                            y0: 2.06,
                            x1: 30.04,
                            y1: 40.06,
                        },
                        baseline_y: 30.0,
                        words: vec![Word {
                            content: "a\\b\nc\td".into(),
                            bbox: BBox {
                                x0: 1.04,
                                y0: 2.06,
                                x1: 30.04,
                                y1: 40.06,
                            },
                            chars: vec![],
                        }],
                    }],
                }),
                Element::Text(TextElement {
                    id: "p1-e1".into(),
                    bbox: BBox {
                        x0: 41.0,
                        y0: 42.0,
                        x1: 43.0,
                        y1: 44.0,
                    },
                    content: String::new(),
                    font: Font {
                        name: String::new(),
                        size: 9.0,
                        bold: false,
                        italic: false,
                    },
                    color: TextColor {
                        fill: Some([0, 0, 0]),
                        stroke: Some([255, 0, 0]),
                    },
                    lines: vec![],
                }),
                Element::Image(ImageElement {
                    id: "p1-e2".into(),
                    bbox: BBox {
                        x0: 5.0,
                        y0: 6.0,
                        x1: 7.0,
                        y1: 8.0,
                    },
                    quad: [[0.0; 2]; 4],
                    pixel_width: None,
                    pixel_height: None,
                    colorspace: None,
                    content_hash: None,
                }),
                Element::Path(PathElement {
                    id: "p1-e3".into(),
                    bbox: BBox {
                        x0: 9.0,
                        y0: 10.0,
                        x1: 11.0,
                        y1: 12.0,
                    },
                    fill: None,
                    stroke: None,
                    stroke_width: None,
                }),
                Element::Annotation(AnnotationElement {
                    id: "p1-e4".into(),
                    bbox: BBox {
                        x0: 13.0,
                        y0: 14.0,
                        x1: 15.0,
                        y1: 16.0,
                    },
                    subtype: "link".into(),
                    uri: None,
                }),
            ],
        }],
    }
}

fn lean(extraction: &Extraction, granularity: Granularity) -> String {
    match extraction.with_granularity(granularity) {
        GranularExtraction::Compact(compact) => compact.to_lean(),
        GranularExtraction::Char(_) => panic!("test only renders compact granularities"),
    }
}

#[test]
fn element_lean_renders_all_edge_rules_exactly() {
    let actual = lean(&extraction(), Granularity::Element);
    let expected = concat!(
        "#docray element v1.2 pages=1 warnings=1\n",
        "#legend T x0 y0 x1 y1 font size style text | I/P x0 y0 x1 y1 | A x0 y0 x1 y1 subtype uri | pt, top-left origin\n",
        "#warning recovered with omissions\n",
        "#page 1 612x792.1 rot=90 scanned\n",
        "T 1 2.1 30 40.1 A_B_C 12.1 bi#231f20 a\\\\b\\nc\td\n",
        "T 41 42 43 44 - 9 - \n",
        "I 5 6 7 8\n",
        "P 9 10 11 12\n",
        "A 13 14 15 16 link -\n",
    );
    assert_eq!(actual, expected);
    assert!(
        !actual.contains("not-rendered"),
        "source envelope must be absent"
    );
    assert!(!actual.contains("#ff0000"), "stroke color must be absent");
}

#[test]
fn word_lean_nests_escaped_words_under_text_elements() {
    let actual = lean(&extraction(), Granularity::Word);
    let lines: Vec<_> = actual.lines().collect();
    assert_eq!(
        lines[1],
        "#legend T x0 y0 x1 y1 font size style | w x0 y0 x1 y1 word | I/P x0 y0 x1 y1 | A x0 y0 x1 y1 subtype uri | pt, top-left origin"
    );
    assert_eq!(lines[4], "T 1 2.1 30 40.1 A_B_C 12.1 bi#231f20");
    assert_eq!(lines[5], "w 1 2.1 30 40.1 a\\\\b\\nc\td");
    assert_eq!(lines[6], "T 41 42 43 44 - 9 -");
}
