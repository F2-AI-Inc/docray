use docray_model::*;

fn sample() -> Extraction {
    Extraction {
        schema_version: "1.1".into(),
        source: Source {
            format: "pdf".into(),
            sha256: "abc".into(),
            size_bytes: 10,
        },
        document: DocumentInfo {
            page_count: 1,
            metadata: DocMetadata {
                title: Some("T".into()),
                author: None,
            },
        },
        warnings: vec![],
        pages: vec![Page {
            page_number: 1,
            width: 612.0,
            height: 792.0,
            rotation: 0,
            scanned: false,
            elements: vec![Element::Text(TextElement {
                id: "p1-e0".into(),
                bbox: BBox {
                    x0: 1.0,
                    y0: 2.0,
                    x1: 3.0,
                    y1: 4.0,
                },
                content: "Hi".into(),
                font: Font {
                    name: "Helvetica".into(),
                    size: 12.0,
                    bold: false,
                    italic: false,
                },
                color: TextColor {
                    fill: Some([0, 0, 0]),
                    stroke: None,
                },
                lines: vec![Line {
                    bbox: BBox {
                        x0: 1.0,
                        y0: 2.0,
                        x1: 3.0,
                        y1: 4.0,
                    },
                    baseline_y: 3.5,
                    words: vec![Word {
                        content: "Hi".into(),
                        bbox: BBox {
                            x0: 1.0,
                            y0: 2.0,
                            x1: 3.0,
                            y1: 4.0,
                        },
                        chars: vec![Char {
                            content: "H".into(),
                            bbox: BBox {
                                x0: 1.0,
                                y0: 2.0,
                                x1: 2.0,
                                y1: 4.0,
                            },
                            unicode: 72,
                        }],
                    }],
                }],
            })],
        }],
    }
}

#[test]
fn serializes_with_type_tag_and_exact_field_names() {
    let v: serde_json::Value = serde_json::to_value(sample()).unwrap();
    assert_eq!(v["schema_version"], "1.1");
    assert_eq!(v["pages"][0]["scanned"], false);
    assert_eq!(v["pages"][0]["elements"][0]["type"], "text");
    assert_eq!(v["pages"][0]["elements"][0]["bbox"]["x0"], 1.0);
    assert_eq!(
        v["pages"][0]["elements"][0]["lines"][0]["words"][0]["chars"][0]["unicode"],
        72
    );
    assert_eq!(v["document"]["metadata"]["author"], serde_json::Value::Null);
}

#[test]
fn roundtrips() {
    let json = serde_json::to_string(&sample()).unwrap();
    let back: Extraction = serde_json::from_str(&json).unwrap();
    assert_eq!(serde_json::to_string(&back).unwrap(), json);
}

#[test]
fn explicit_granularities_keep_nonempty_warnings() {
    let mut extraction = sample();
    extraction.warnings = vec!["page 1 recovered with omissions".into()];
    for level in [Granularity::Char, Granularity::Word, Granularity::Element] {
        let value = serde_json::to_value(extraction.with_granularity(level)).unwrap();
        assert_eq!(value["schema_version"], "1.2");
        assert_eq!(value["granularity"], level.as_str());
        assert_eq!(
            value["warnings"],
            serde_json::json!(["page 1 recovered with omissions"])
        );
    }
}

#[test]
fn bbox_union_and_round3() {
    let a = BBox {
        x0: 1.0,
        y0: 5.0,
        x1: 3.0,
        y1: 8.0,
    };
    let b = BBox {
        x0: 2.0,
        y0: 4.0,
        x1: 9.0,
        y1: 6.0,
    };
    let u = a.union(&b);
    assert_eq!(
        u,
        BBox {
            x0: 1.0,
            y0: 4.0,
            x1: 9.0,
            y1: 8.0
        }
    );
    assert_eq!(round3(1.23456), 1.235);
    assert_eq!(round3(1.0), 1.0);
}
