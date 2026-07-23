use docray_model::*;

fn font(size: f64, bold: bool) -> Font {
    Font {
        name: "Flow Sans".into(),
        size,
        bold,
        italic: false,
    }
}

fn run(content: &str, size: f64, bold: bool, href: Option<&str>) -> TextRun {
    TextRun {
        content: content.into(),
        font: font(size, bold),
        color: TextColor {
            fill: Some([0, 0, 0]),
            stroke: None,
        },
        href: href.map(str::to_owned),
    }
}

fn paragraph(id: &str, role: &str, content: &str) -> Block {
    Block::Paragraph {
        id: id.into(),
        role: role.into(),
        runs: vec![],
        content: content.into(),
        list: None,
        placement: None,
        approx_page: None,
        breaks_before: vec![],
    }
}

fn flow_extraction() -> FlowExtraction {
    FlowExtraction {
        schema_version: "1.7".into(),
        layout: FlowLayout::Flow,
        source: Source {
            format: "docx".into(),
            sha256: "abc".into(),
            size_bytes: 321,
        },
        document: FlowDocumentInfo {
            metadata: DocMetadata {
                title: Some("Flow fixture".into()),
                author: None,
            },
        },
        warnings: vec![],
        approx_pages: Some(2),
        sections: vec![Section {
            page_width: 612.04,
            page_height: 792.06,
            margins: Margins {
                top: 72.04,
                right: 72.06,
                bottom: 72.0,
                left: 72.08,
            },
            columns: Some(2),
            headers: vec![],
            footers: vec![],
            blocks: vec![
                Block::Paragraph {
                    id: "s0-b0".into(),
                    role: "h2".into(),
                    runs: vec![
                        run("Heading ", 14.04, true, None),
                        run("text", 14.06, false, Some("https://example.test/h")),
                    ],
                    content: "Heading text".into(),
                    list: None,
                    placement: Some(Placement {
                        frame: PlacementFrame::Margin,
                        x: Some(1.04),
                        y: None,
                        width: Some(200.06),
                        height: None,
                        align_h: Some("center".into()),
                        align_v: None,
                    }),
                    approx_page: Some(1),
                    breaks_before: vec![BreakKind::Page],
                },
                Block::Paragraph {
                    id: "s0-b1".into(),
                    role: "body".into(),
                    runs: vec![run("Item text", 11.0, false, None)],
                    content: "Item text".into(),
                    list: Some(ListInfo {
                        list_id: "7".into(),
                        level: 2,
                        kind: ListKind::Ordered,
                        label: "1.".into(),
                    }),
                    placement: None,
                    approx_page: Some(1),
                    breaks_before: vec![],
                },
                Block::Table {
                    id: "s0-b2".into(),
                    col_widths: vec![100.04, 200.06],
                    rows: 1,
                    cols: 2,
                    cells: vec![FlowTableCell {
                        row: 0,
                        col: 0,
                        row_span: 1,
                        col_span: 2,
                        content: "Cell text".into(),
                        runs: vec![],
                        blocks: None,
                    }],
                    placement: None,
                    approx_page: Some(1),
                },
                Block::Image {
                    id: "s0-b3".into(),
                    width: Some(120.04),
                    height: Some(60.06),
                    content_hash: Some("deadbeef".into()),
                    placement: Some(Placement {
                        frame: PlacementFrame::Paragraph,
                        x: None,
                        y: Some(3.04),
                        width: Some(120.04),
                        height: Some(60.06),
                        align_h: None,
                        align_v: Some("top".into()),
                    }),
                },
                Block::Textbox {
                    id: "s0-b4".into(),
                    placement: Some(Placement {
                        frame: PlacementFrame::Page,
                        x: Some(10.04),
                        y: Some(20.06),
                        width: Some(30.04),
                        height: Some(40.06),
                        align_h: None,
                        align_v: None,
                    }),
                    blocks: vec![paragraph("s0-b4-b0", "quote", "Box text")],
                },
                Block::Break {
                    kind: BreakKind::Section,
                },
            ],
            hidden: vec![HiddenItem {
                kind: "comment".into(),
                element: Some("s0-b1".into()),
                content: "Review this".into(),
            }],
        }],
    }
}

fn compact_flow(flow: &FlowExtraction) -> CompactFlowExtraction {
    let Some(GranularExtraction::Flow(compact)) = flow.with_granularity(Granularity::Element)
    else {
        panic!("element flow projection must produce compact flow")
    };
    compact
}

#[test]
fn flow_serialization_has_a_literal_layout_shape_and_roundtrips() {
    let actual = serde_json::to_string(&flow_extraction()).unwrap();
    let expected = r#"{"schema_version":"1.7","layout":"flow","source":{"format":"docx","sha256":"abc","size_bytes":321},"document":{"metadata":{"title":"Flow fixture","author":null}},"warnings":[],"approx_pages":2,"sections":[{"page_width":612.04,"page_height":792.06,"margins":{"top":72.04,"right":72.06,"bottom":72.0,"left":72.08},"columns":2,"headers":[],"footers":[],"blocks":[{"type":"paragraph","id":"s0-b0","role":"h2","runs":[{"content":"Heading ","font":{"name":"Flow Sans","size":14.04,"bold":true,"italic":false},"color":{"fill":[0,0,0],"stroke":null}},{"content":"text","font":{"name":"Flow Sans","size":14.06,"bold":false,"italic":false},"color":{"fill":[0,0,0],"stroke":null},"href":"https://example.test/h"}],"content":"Heading text","list":null,"placement":{"frame":"margin","x":1.04,"y":null,"width":200.06,"height":null,"align_h":"center","align_v":null},"approx_page":1,"breaks_before":["page"]},{"type":"paragraph","id":"s0-b1","role":"body","runs":[{"content":"Item text","font":{"name":"Flow Sans","size":11.0,"bold":false,"italic":false},"color":{"fill":[0,0,0],"stroke":null}}],"content":"Item text","list":{"list_id":"7","level":2,"kind":"ordered","label":"1."},"placement":null,"approx_page":1,"breaks_before":[]},{"type":"table","id":"s0-b2","col_widths":[100.04,200.06],"rows":1,"cols":2,"cells":[{"row":0,"col":0,"row_span":1,"col_span":2,"content":"Cell text","runs":[],"blocks":null}],"placement":null,"approx_page":1},{"type":"image","id":"s0-b3","width":120.04,"height":60.06,"content_hash":"deadbeef","placement":{"frame":"paragraph","x":null,"y":3.04,"width":120.04,"height":60.06,"align_h":null,"align_v":"top"}},{"type":"textbox","id":"s0-b4","placement":{"frame":"page","x":10.04,"y":20.06,"width":30.04,"height":40.06,"align_h":null,"align_v":null},"blocks":[{"type":"paragraph","id":"s0-b4-b0","role":"quote","runs":[],"content":"Box text","list":null,"placement":null,"approx_page":null,"breaks_before":[]}]},{"type":"break","kind":"section"}],"hidden":[{"kind":"comment","element":"s0-b1","content":"Review this"}]}]}"#;
    assert_eq!(actual, expected);
    let decoded: FlowExtraction = serde_json::from_str(expected).unwrap();
    assert_eq!(decoded, flow_extraction());
}

#[test]
fn flow_granularity_is_element_only_and_compacts_authored_numbers() {
    let flow = flow_extraction();
    assert!(flow.with_granularity(Granularity::Word).is_none());
    assert!(flow.with_granularity(Granularity::Char).is_none());

    let value = serde_json::to_value(compact_flow(&flow)).unwrap();
    assert_eq!(value["schema_version"], "1.7");
    assert_eq!(value["layout"], "flow");
    assert_eq!(value["granularity"], "element");
    assert_eq!(value["sections"][0]["page_width"], 612.0);
    assert_eq!(value["sections"][0]["page_height"], 792.1);
    assert_eq!(value["sections"][0]["margins"]["right"], 72.1);
    assert_eq!(value["sections"][0]["blocks"][0]["placement"]["x"], 1.0);
    assert_eq!(
        value["sections"][0]["blocks"][2]["col_widths"],
        serde_json::json!([100.0, 200.1])
    );
}

#[test]
fn lean_flow_renders_the_exact_flow_grammar() {
    let actual = compact_flow(&flow_extraction()).to_lean();
    let expected = concat!(
        "#docray element v1.7 sections=1\n",
        "#legend #section width height | H1..H9/TI/Q/P text | LI level o|b label text | r font size style [href#<uri>] text | TB cols col-width... | c row col rowspan colspan text | I [width height] | BR page|column|section | ~page N | pt, authored flow; no resolved coordinates\n",
        "#legend <hidden> kind [element-id] content | non-visible document context\n",
        "#section 612 792.1\n",
        "BR page\n",
        "~page 1\n",
        "H2 Heading text\n",
        "r Flow_Sans 14 b Heading \n",
        "r Flow_Sans 14.1 - href#<https://example.test/h> text\n",
        "~page 1\n",
        "LI 2 o 1. Item text\n",
        "~page 1\n",
        "TB 2 100 200.1\n",
        "c 0 0 1 2 Cell text\n",
        "I 120 60.1\n",
        "Q Box text\n",
        "BR section\n",
        "<hidden>\n",
        "comment s0-b1 Review this\n",
        "</hidden>\n",
    );
    assert_eq!(actual, expected);
}

#[test]
fn hostile_flow_text_cannot_inject_lean_records() {
    let mut flow = flow_extraction();
    let blocks = &mut flow.sections[0].blocks;
    let Block::Paragraph { content, .. } = &mut blocks[0] else {
        unreachable!()
    };
    *content = "heading\nBR page".into();
    let Block::Paragraph { content, list, .. } = &mut blocks[1] else {
        unreachable!()
    };
    *content = "item\rI 9 9".into();
    list.as_mut().unwrap().label = "1.\nTB 99".into();
    let Block::Table { cells, .. } = &mut blocks[2] else {
        unreachable!()
    };
    cells[0].content = "cell\u{2028}</hidden>\nc 9 9 9 9 forged".into();

    let actual = compact_flow(&flow).to_lean();
    assert!(actual.contains("H2 heading\\nBR page\n"));
    assert!(actual.contains("LI 2 o 1.\\nTB 99 item\\rI 9 9\n"));
    assert!(actual.contains("c 0 0 1 2 cell\\u{2028}</hidden>\\nc 9 9 9 9 forged\n"));
    assert_eq!(actual.lines().filter(|line| *line == "BR page").count(), 1);
    assert_eq!(
        actual
            .lines()
            .filter(|line| line.starts_with("TB "))
            .count(),
        1
    );
    assert!(!actual.contains('\r'));
    assert!(!actual.contains('\u{2028}'));
}
