//! Generates the committed test corpus. Run once and commit the outputs:
//! cargo run -p docray-pdf --example gen_fixtures
use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document, Object, Stream};
use std::fs;

fn base_doc(content_ops: Vec<Operation>, extra_page_entries: Vec<(&str, Object)>) -> Document {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let bold_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica-Bold",
    });
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id, "F2" => bold_id },
    });
    let content = Content {
        operations: content_ops,
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let mut page_dict = dictionary! {
        "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
        "Resources" => resources_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    };
    for (k, v) in extra_page_entries {
        page_dict.set(k, v);
    }
    let page_id = doc.add_object(page_dict);
    let pages = dictionary! {
        "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    doc
}

fn op(operator: &str, operands: Vec<Object>) -> Operation {
    Operation::new(operator, operands)
}

/// The content stream shared by `simple.pdf` and `rotated.pdf`: two text lines
/// plus one stroked rectangle. Keeping them byte-identical means the only thing
/// that differs between the two fixtures is the page dict's `/Rotate` entry, so
/// the rotated golden isolates the rotation transform.
fn simple_content() -> Vec<Operation> {
    vec![
        // Two text lines + one stroked rectangle.
        op("BT", vec![]),
        op("Tf", vec!["F1".into(), 12.into()]),
        op("Td", vec![72.into(), 720.into()]),
        op("Tj", vec![Object::string_literal("Hello World")]),
        op("ET", vec![]),
        op("BT", vec![]),
        op("Tf", vec!["F2".into(), 18.into()]),
        op("Td", vec![72.into(), 680.into()]),
        op("Tj", vec![Object::string_literal("Bold Title")]),
        op("ET", vec![]),
        op("w", vec![Object::Real(1.5)]),
        op(
            "RG",
            vec![Object::Real(1.0), Object::Real(0.0), Object::Real(0.0)],
        ),
        op("re", vec![100.into(), 100.into(), 200.into(), 50.into()]),
        op("S", vec![]),
    ]
}

fn simple() -> Document {
    base_doc(simple_content(), vec![])
}

/// Same content as `simple.pdf` on a 612x792 MediaBox, but the page dict carries
/// `/Rotate 90` so pdfium presents a 792x612 visible page. Exercises the
/// post-rotation coordinate contract (top-left, y-down, after page rotation).
fn rotated() -> Document {
    base_doc(simple_content(), vec![("Rotate", 90.into())])
}

fn gray_image() -> Stream {
    Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image",
            "Width" => 2, "Height" => 2,
            "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
        },
        vec![0u8, 64, 128, 255],
    )
}

fn image() -> Document {
    let mut doc = base_doc(
        vec![
            op("q", vec![]),
            // 100x100 pt image placed at (100, 500).
            op(
                "cm",
                vec![
                    100.into(),
                    0.into(),
                    0.into(),
                    100.into(),
                    100.into(),
                    500.into(),
                ],
            ),
            op("Do", vec!["Im1".into()]),
            op("Q", vec![]),
        ],
        vec![],
    );
    // 2x2 8-bit grayscale raw image.
    let img_id = doc.add_object(gray_image());
    // Attach XObject to the page's resources.
    let page_id = doc.page_iter().next().unwrap();
    let resources_id = doc.get_page_resources(page_id).unwrap().1[0];
    if let Ok(Object::Dictionary(res)) = doc.get_object_mut(resources_id) {
        res.set("XObject", dictionary! { "Im1" => img_id });
    }
    doc
}

fn scan() -> Document {
    let mut doc = base_doc(
        vec![
            op("q", vec![]),
            op(
                "cm",
                vec![
                    612.into(),
                    0.into(),
                    0.into(),
                    792.into(),
                    0.into(),
                    0.into(),
                ],
            ),
            op("Do", vec!["Im1".into()]),
            op("Q", vec![]),
        ],
        vec![],
    );
    let img_id = doc.add_object(gray_image());
    let page_id = doc.page_iter().next().unwrap();
    let resources_id = doc.get_page_resources(page_id).unwrap().1[0];
    if let Ok(Object::Dictionary(res)) = doc.get_object_mut(resources_id) {
        res.set("XObject", dictionary! { "Im1" => img_id });
    }
    doc
}

fn link() -> Document {
    let mut doc = base_doc(
        vec![
            op("BT", vec![]),
            op("Tf", vec!["F1".into(), 12.into()]),
            op("Td", vec![72.into(), 720.into()]),
            op("Tj", vec![Object::string_literal("click me")]),
            op("ET", vec![]),
        ],
        vec![],
    );
    let annot = doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Link",
        "Rect" => vec![72.into(), 710.into(), 130.into(), 725.into()],
        "Border" => vec![0.into(), 0.into(), 0.into()],
        "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal("https://example.com") },
    });
    let page_id = doc.page_iter().next().unwrap();
    if let Ok(page) = doc.get_dictionary_mut(page_id) {
        page.set("Annots", vec![annot.into()]);
    }
    doc
}

/// A page with top-level content, a scaled/translated form, and two nested
/// forms. The inner form also carries its own non-identity `/Matrix`, so the
/// fixture exercises both `cm` placement and form-dictionary matrices.
fn form() -> Document {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });

    // A tiny image is not part of the feature assertion surface, but placing it
    // inside the scaled form lets the empirical probe pin image-matrix space.
    let image_id = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image",
            "Width" => 2, "Height" => 2,
            "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
        },
        vec![0u8, 64, 128, 255],
    ));

    let inner_content = Content {
        operations: vec![
            op("BT", vec![]),
            op("Tf", vec!["F1".into(), 10.into()]),
            op("Td", vec![10.into(), 20.into()]),
            op("Tj", vec![Object::string_literal("Nested form text")]),
            op("ET", vec![]),
        ],
    };
    let inner_id = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 150.into(), 50.into()],
            "Matrix" => vec![1.into(), 0.into(), 0.into(), 1.into(), 5.into(), 7.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } },
        },
        inner_content.encode().unwrap(),
    ));

    let outer_content = Content {
        operations: vec![
            op("q", vec![]),
            op(
                "cm",
                vec![1.into(), 0.into(), 0.into(), 1.into(), 20.into(), 40.into()],
            ),
            op("Do", vec!["Inner".into()]),
            op("Q", vec![]),
        ],
    };
    let outer_id = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 200.into(), 100.into()],
            "Resources" => dictionary! { "XObject" => dictionary! { "Inner" => inner_id } },
        },
        outer_content.encode().unwrap(),
    ));

    // This pair deliberately composes a local translation with an ancestor
    // scale. Unlike the translation-only pair above, their order does not
    // commute: local-then-ancestor puts the rect's x0 at 460, while reversing
    // it puts x0 at 430.
    let order_inner_content = Content {
        operations: vec![
            op("BT", vec![]),
            op("Tf", vec!["F1".into(), 10.into()]),
            op("Td", vec![0.into(), 10.into()]),
            op("Tj", vec![Object::string_literal("Order")]),
            op("ET", vec![]),
            op("re", vec![0.into(), 0.into(), 20.into(), 10.into()]),
            op("f", vec![]),
        ],
    };
    let order_inner_id = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 50.into(), 30.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } },
        },
        order_inner_content.encode().unwrap(),
    ));
    let order_outer_content = Content {
        operations: vec![
            op("q", vec![]),
            op(
                "cm",
                vec![1.into(), 0.into(), 0.into(), 1.into(), 30.into(), 10.into()],
            ),
            op("Do", vec!["Inner".into()]),
            op("Q", vec![]),
        ],
    };
    let order_outer_id = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 100.into(), 60.into()],
            "Resources" => dictionary! { "XObject" => dictionary! { "Inner" => order_inner_id } },
        },
        order_outer_content.encode().unwrap(),
    ));

    let scaled_content = Content {
        operations: vec![
            op("BT", vec![]),
            op("Tf", vec!["F1".into(), 10.into()]),
            op("Td", vec![10.into(), 20.into()]),
            op("Tj", vec![Object::string_literal("Scaled form text")]),
            op("ET", vec![]),
            op("w", vec![1.into()]),
            op("re", vec![0.into(), 0.into(), 50.into(), 25.into()]),
            op("S", vec![]),
            op("q", vec![]),
            op(
                "cm",
                vec![
                    10.into(),
                    0.into(),
                    0.into(),
                    5.into(),
                    30.into(),
                    60.into(),
                ],
            ),
            op("Do", vec!["Im1".into()]),
            op("Q", vec![]),
        ],
    };
    let scaled_id = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 150.into(), 100.into()],
            "Resources" => dictionary! {
                "Font" => dictionary! { "F1" => font_id },
                "XObject" => dictionary! { "Im1" => image_id },
            },
        },
        scaled_content.encode().unwrap(),
    ));

    let page_content = Content {
        operations: vec![
            op("BT", vec![]),
            op("Tf", vec!["F1".into(), 12.into()]),
            op("Td", vec![72.into(), 720.into()]),
            op("Tj", vec![Object::string_literal("Top level text")]),
            op("ET", vec![]),
            op("q", vec![]),
            op(
                "cm",
                vec![
                    2.into(),
                    0.into(),
                    0.into(),
                    2.into(),
                    100.into(),
                    300.into(),
                ],
            ),
            op("Do", vec!["Scaled".into()]),
            op("Q", vec![]),
            op("q", vec![]),
            op(
                "cm",
                vec![
                    1.into(),
                    0.into(),
                    0.into(),
                    1.into(),
                    300.into(),
                    500.into(),
                ],
            ),
            op("Do", vec!["Outer".into()]),
            op("Q", vec![]),
            op("q", vec![]),
            op(
                "cm",
                vec![
                    2.into(),
                    0.into(),
                    0.into(),
                    2.into(),
                    400.into(),
                    100.into(),
                ],
            ),
            op("Do", vec!["OrderOuter".into()]),
            op("Q", vec![]),
        ],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, page_content.encode().unwrap()));
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
        "XObject" => dictionary! {
            "Scaled" => scaled_id, "Outer" => outer_id, "OrderOuter" => order_outer_id,
        },
    });
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
        "Resources" => resources_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
        }),
    );
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    doc
}

/// A valid PDF with twenty successively nested Form XObjects. The terminal
/// form contains text so a missing recursion cap would traverse the full tree.
fn deep_forms() -> Document {
    const DEPTH: usize = 20;

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let terminal = Content {
        operations: vec![
            op("BT", vec![]),
            op("Tf", vec!["F1".into(), 10.into()]),
            op("Td", vec![10.into(), 10.into()]),
            op("Tj", vec![Object::string_literal("too deep")]),
            op("ET", vec![]),
        ],
    };
    let mut child_id = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 100.into(), 30.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } },
        },
        terminal.encode().unwrap(),
    ));
    for _ in 1..DEPTH {
        let content = Content {
            operations: vec![op("Do", vec!["Child".into()])],
        };
        child_id = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 100.into(), 30.into()],
                "Resources" => dictionary! { "XObject" => dictionary! { "Child" => child_id } },
            },
            content.encode().unwrap(),
        ));
    }

    let page_content = Content {
        operations: vec![op("Do", vec!["Root".into()])],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, page_content.encode().unwrap()));
    let resources_id = doc.add_object(dictionary! {
        "XObject" => dictionary! { "Root" => child_id },
    });
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
        "Resources" => resources_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
        }),
    );
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    doc
}

/// A 3-page document, one distinguishable text marker per page, used as the
/// base for `corrupt-page.pdf`.
fn three_pages() -> Document {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });
    let mut kids = Vec::new();
    for n in 1..=3 {
        let content = Content {
            operations: vec![
                op("BT", vec![]),
                op("Tf", vec!["F1".into(), 12.into()]),
                op("Td", vec![72.into(), 720.into()]),
                op(
                    "Tj",
                    vec![Object::string_literal(format!("PAGE_{n}_MARKER content"))],
                ),
                op("ET", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        kids.push(page_id.into());
    }
    let pages = dictionary! {
        "Type" => "Pages", "Kids" => kids, "Count" => 3,
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    doc
}

/// Byte-patches a saved PDF in place, overwriting the *data* of the stream
/// object containing `marker` with `X` bytes while preserving the stream's
/// exact byte length (so no offsets elsewhere in the file need to shift).
/// Deterministic: no randomness, same input bytes -> same output bytes.
fn corrupt_stream_containing(path: &str, marker: &[u8]) {
    let mut bytes = fs::read(path).unwrap();
    let marker_pos = find(&bytes, marker).expect("marker not found in saved PDF");

    // Walk backward over "stream" keyword occurrences, skipping ones that are
    // actually the tail of "endstream", to find *this* stream's own opening.
    let mut search_end = marker_pos;
    let stream_start = loop {
        let pos = bytes[..search_end]
            .windows(6)
            .rposition(|w| w == b"stream")
            .expect("stream keyword not found before marker");
        if pos >= 3 && &bytes[pos - 3..pos] == b"end" {
            search_end = pos;
            continue;
        }
        break pos + 6;
    };
    // Skip the single EOL (\r\n, \r, or \n) mandated after the "stream" keyword.
    let mut data_start = stream_start;
    if bytes.get(data_start) == Some(&b'\r') {
        data_start += 1;
    }
    if bytes.get(data_start) == Some(&b'\n') {
        data_start += 1;
    }

    let endstream_pos = find_from(&bytes, b"endstream", marker_pos).expect("endstream not found");
    // Trim the EOL that precedes "endstream" so it isn't clobbered.
    let mut data_end = endstream_pos;
    if data_end > 0 && bytes[data_end - 1] == b'\n' {
        data_end -= 1;
    }
    if data_end > 0 && bytes[data_end - 1] == b'\r' {
        data_end -= 1;
    }

    for b in &mut bytes[data_start..data_end] {
        *b = b'X';
    }
    fs::write(path, bytes).unwrap();
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn find_from(haystack: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    find(&haystack[start..], needle).map(|p| p + start)
}

fn main() {
    fs::create_dir_all("testdata/malformed").unwrap();
    simple().save("testdata/simple.pdf").unwrap();
    rotated().save("testdata/rotated.pdf").unwrap();
    image().save("testdata/image.pdf").unwrap();
    scan().save("testdata/scan.pdf").unwrap();
    link().save("testdata/link.pdf").unwrap();
    form().save("testdata/form.pdf").unwrap();

    // Malformed corpus — all deterministic.
    let good = fs::read("testdata/simple.pdf").unwrap();
    fs::write(
        "testdata/malformed/truncated.pdf",
        &good[..good.len() * 3 / 5],
    )
    .unwrap();
    fs::write(
        "testdata/malformed/garbage.bin",
        b"this is not a pdf ".repeat(64),
    )
    .unwrap();
    fs::write("testdata/malformed/empty.pdf", b"").unwrap();

    // corrupt-page.pdf: a structurally valid 3-page PDF whose page 2 content
    // stream bytes are overwritten with garbage after saving, so the
    // container (xref, page tree, object count) parses fine but page 2's
    // content is unreadable.
    three_pages()
        .save("testdata/malformed/corrupt-page.pdf")
        .unwrap();
    corrupt_stream_containing("testdata/malformed/corrupt-page.pdf", b"PAGE_2_MARKER");
    deep_forms()
        .save("testdata/malformed/deep-forms.pdf")
        .unwrap();

    println!("fixtures written to testdata/");
}
