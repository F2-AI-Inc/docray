//! Generates the committed test corpus. Run once and commit the outputs:
//! cargo run -p docray-pdf --example gen_fixtures
use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Dictionary, Document, Object, Stream, StringFormat};
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

/// A deterministic Type 3 font fixture. Each byte has a visible box glyph;
/// `unicode` optionally supplies the font's ToUnicode mapping. This lets the
/// broken fixture exercise missing character mappings without depending on a
/// platform font, while the CJK control proves that mapped non-Latin text is
/// not treated as garble.
fn type3_text(codes: &[u8], unicode: Option<&[u16]>) -> Document {
    assert!(!codes.is_empty());
    assert!(codes.windows(2).all(|pair| pair[1] == pair[0] + 1));
    assert!(unicode.is_none_or(|mapping| mapping.len() == codes.len()));

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let mut char_procs = Dictionary::new();
    let mut differences = vec![(codes[0] as i64).into()];
    let mut widths = Vec::new();

    for index in 0..codes.len() {
        let name = format!("glyph_{:02}", index + 1);
        let glyph = Stream::new(
            dictionary! {},
            b"600 0 0 0 500 700 d1\n0 0 500 700 re f\n".to_vec(),
        );
        let glyph_id = doc.add_object(glyph);
        char_procs.set(name.as_bytes(), glyph_id);
        differences.push(Object::Name(name.into_bytes()));
        widths.push(600.into());
    }

    let mut font = dictionary! {
        "Type" => "Font",
        "Subtype" => "Type3",
        "FontBBox" => vec![0.into(), 0.into(), 500.into(), 700.into()],
        "FontMatrix" => vec![Object::Real(0.001), 0.into(), 0.into(), Object::Real(0.001), 0.into(), 0.into()],
        "CharProcs" => char_procs,
        "Encoding" => dictionary! {
            "Type" => "Encoding",
            "Differences" => differences,
        },
        "FirstChar" => codes[0] as i64,
        "LastChar" => *codes.last().unwrap() as i64,
        "Widths" => widths,
        "Resources" => dictionary! {},
    };

    if let Some(mapping) = unicode {
        let bfchar = codes
            .iter()
            .zip(mapping)
            .map(|(code, value)| format!("<{code:02X}> <{value:04X}>"))
            .collect::<Vec<_>>()
            .join("\n");
        let cmap = format!(
            "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (docray) /Ordering (fixture) /Supplement 0 >> def\n/CMapName /docray-fixture def\n/CMapType 2 def\n1 begincodespacerange\n<01> <FF>\nendcodespacerange\n{} beginbfchar\n{}\nendbfchar\nendcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n",
            mapping.len(), bfchar
        );
        let to_unicode_id = doc.add_object(Stream::new(dictionary! {}, cmap.into_bytes()));
        font.set("ToUnicode", to_unicode_id);
    }

    let font_id = doc.add_object(font);
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });
    let content = Content {
        operations: vec![
            op("BT", vec![]),
            op("Tf", vec!["F1".into(), 24.into()]),
            op("Td", vec![72.into(), 700.into()]),
            op(
                "Tj",
                vec![Object::String(codes.to_vec(), StringFormat::Literal)],
            ),
            op("ET", vec![]),
        ],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
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

fn broken_encoding() -> Document {
    // C0 byte values with custom glyph names have no standard Unicode fallback
    // and no ToUnicode map. PDFium exposes them as unmapped/control glyphs.
    type3_text(&(0x0e..=0x19).collect::<Vec<_>>(), None)
}

fn cjk() -> Document {
    // 你好世界漢字文本 — eight mapped glyphs, all legitimate non-Latin text.
    type3_text(
        &(1..=8).collect::<Vec<_>>(),
        Some(&[
            0x4f60, 0x597d, 0x4e16, 0x754c, 0x6f22, 0x5b57, 0x6587, 0x672c,
        ]),
    )
}

/// A 2x2 ruled table. In PDF coordinates the outer box is x=72..360,
/// y=570..650 on a 612x792 page. PDFium expands its 1pt stroke bounds by 1pt,
/// producing the observed top-left bbox x=71..361, y=141..223. The interior
/// rulings at x=216 and y=610 divide it into four nominal 144x40pt cells. Text
/// baselines at y=625/585 place each label's center inside the corresponding
/// row; x=84/228 places them in the expected columns. The literal pipe in the
/// lower-left cell pins GFM escaping.
fn ruled_table() -> Document {
    base_doc(
        vec![
            op("w", vec![1.into()]),
            op("RG", vec![0.into(), 0.into(), 0.into()]),
            op("re", vec![72.into(), 570.into(), 288.into(), 80.into()]),
            op("S", vec![]),
            op("m", vec![72.into(), 610.into()]),
            op("l", vec![360.into(), 610.into()]),
            op("S", vec![]),
            op("m", vec![216.into(), 570.into()]),
            op("l", vec![216.into(), 650.into()]),
            op("S", vec![]),
            op("BT", vec![]),
            op("Tf", vec!["F1".into(), 12.into()]),
            op("Td", vec![84.into(), 625.into()]),
            op("Tj", vec![Object::string_literal("Name")]),
            op("ET", vec![]),
            op("BT", vec![]),
            op("Tf", vec!["F1".into(), 12.into()]),
            op("Td", vec![228.into(), 625.into()]),
            op("Tj", vec![Object::string_literal("Amount")]),
            op("ET", vec![]),
            op("BT", vec![]),
            op("Tf", vec!["F1".into(), 12.into()]),
            op("Td", vec![84.into(), 585.into()]),
            op("Tj", vec![Object::string_literal("Alpha | Beta")]),
            op("ET", vec![]),
            op("BT", vec![]),
            op("Tf", vec!["F1".into(), 12.into()]),
            op("Td", vec![228.into(), 585.into()]),
            op("Tj", vec![Object::string_literal("$42")]),
            op("ET", vec![]),
        ],
        vec![],
    )
}

/// Same content as `simple.pdf` on a 612x792 MediaBox, but the page dict carries
/// `/Rotate 90` so pdfium presents a 792x612 visible page. Exercises the
/// post-rotation coordinate contract (top-left, y-down, after page rotation).
fn rotated() -> Document {
    base_doc(simple_content(), vec![("Rotate", 90.into())])
}

/// Emits one `BT … Tj … ET` text run at PDF-space `(x, y)` (baseline), F1 12pt.
fn text_run(ops: &mut Vec<Operation>, x: i64, y: i64, content: &str) {
    ops.push(op("BT", vec![]));
    ops.push(op("Tf", vec!["F1".into(), 12.into()]));
    ops.push(op("Td", vec![x.into(), y.into()]));
    ops.push(op("Tj", vec![Object::string_literal(content)]));
    ops.push(op("ET", vec![]));
}

/// A stroked horizontal segment at PDF-space `y` from `x0` to `x1`.
fn hline(ops: &mut Vec<Operation>, x0: i64, x1: i64, y: i64) {
    ops.push(op("m", vec![x0.into(), y.into()]));
    ops.push(op("l", vec![x1.into(), y.into()]));
    ops.push(op("S", vec![]));
}

/// A stroked vertical segment at PDF-space `x` from `y0` to `y1`.
fn vline(ops: &mut Vec<Operation>, x: i64, y0: i64, y1: i64) {
    ops.push(op("m", vec![x.into(), y0.into()]));
    ops.push(op("l", vec![x.into(), y1.into()]));
    ops.push(op("S", vec![]));
}

/// A borderless 3×3 alignment table: three left-aligned columns at x=72/222/372
/// over three rows 20pt apart, no rulings. The whitespace gutters between the
/// columns are wide and stable, so the alignment detector reconstructs it; with
/// every span == 1 it renders as a GFM pipe table.
fn borderless_table() -> Document {
    let mut ops = Vec::new();
    let rows = [
        ["Name", "Qty", "Price"],
        ["Alpha", "2", "$4"],
        ["Beta", "5", "$9"],
    ];
    for (r, values) in rows.iter().enumerate() {
        let y = 700 - (r as i64) * 20;
        for (c, value) in values.iter().enumerate() {
            let x = 72 + (c as i64) * 150;
            text_run(&mut ops, x, y, value);
        }
    }
    base_doc(ops, vec![])
}

/// A ruled 3×3 grid whose two interior verticals stop below the header band, so
/// row 0 is one cell spanning all three columns. The missing interior separator
/// segments become a colspan, forcing the HTML `<table>` renderer (GFM pipe
/// cannot carry colspan/rowspan). Outer box x=72..372, y=650..750; row rulings
/// at y=683/717; interior verticals x=172/272 only over y=650..717.
fn merged_ruled_table() -> Document {
    let mut ops = vec![
        op("w", vec![1.into()]),
        op("RG", vec![0.into(), 0.into(), 0.into()]),
    ];
    ops.push(op(
        "re",
        vec![72.into(), 650.into(), 300.into(), 100.into()],
    ));
    ops.push(op("S", vec![]));
    hline(&mut ops, 72, 372, 683);
    hline(&mut ops, 72, 372, 717);
    vline(&mut ops, 172, 650, 717);
    vline(&mut ops, 272, 650, 717);
    text_run(&mut ops, 180, 727, "Summary");
    text_run(&mut ops, 90, 695, "Alpha");
    text_run(&mut ops, 200, 695, "2");
    text_run(&mut ops, 300, 695, "$4");
    text_run(&mut ops, 90, 662, "Beta");
    text_run(&mut ops, 200, 662, "5");
    text_run(&mut ops, 300, 662, "$9");
    base_doc(ops, vec![])
}

/// A borderless block whose first row is a wide title straddling the gutter
/// between the first two columns; the five data rows below keep the gutters
/// stable. The title becomes a colspan cell, forcing the HTML `<table>`
/// renderer. Columns at x=72/172/272 (the title reaches into the second column
/// but not the third).
fn merged_borderless_table() -> Document {
    let mut ops = Vec::new();
    text_run(&mut ops, 72, 720, "Quarterly Financial Report");
    for r in 0..5 {
        let y = 700 - r * 16;
        for c in 0..3 {
            let x = 72 + c * 100;
            text_run(&mut ops, x, y, &format!("v{r}{c}"));
        }
    }
    base_doc(ops, vec![])
}

/// A merged (colspan) ruled table whose header cell is hostile PDF text
/// (`</td><script>…`). Because it merges, it routes through the HTML renderer;
/// the fixture pins that untrusted cell text is HTML-escaped and cannot break
/// out of the `<table>`. Same 3×3 geometry as `merged_ruled_table` so the grid
/// carries enough filled cells to pass the occupancy gate.
fn hostile_cell_table() -> Document {
    let mut ops = vec![
        op("w", vec![1.into()]),
        op("RG", vec![0.into(), 0.into(), 0.into()]),
    ];
    ops.push(op(
        "re",
        vec![72.into(), 650.into(), 300.into(), 100.into()],
    ));
    ops.push(op("S", vec![]));
    hline(&mut ops, 72, 372, 683);
    hline(&mut ops, 72, 372, 717);
    vline(&mut ops, 172, 650, 717);
    vline(&mut ops, 272, 650, 717);
    text_run(&mut ops, 90, 727, "</td><script>alert(1)</script>");
    text_run(&mut ops, 90, 695, "safe");
    text_run(&mut ops, 200, 695, "cell");
    text_run(&mut ops, 300, 695, "text");
    text_run(&mut ops, 90, 662, "a&b");
    text_run(&mut ops, 200, 662, "c<d");
    text_run(&mut ops, 300, 662, "e>f");
    base_doc(ops, vec![])
}

/// Negative corpus: full-width running prose. Every line spans the content
/// column, so no stable interior gutter exists and detection is declined — the
/// lines must render as ordinary reading-order text.
fn prose() -> Document {
    let mut ops = Vec::new();
    for r in 0..5 {
        let y = 700 - r * 14;
        text_run(
            &mut ops,
            72,
            y,
            "This is a full width running prose line that fills the content column",
        );
    }
    base_doc(ops, vec![])
}

/// Negative corpus: ragged code indentation. Three token groups per line, but
/// the left/right edges jitter far more than a bucket width, so the column-edge
/// tightness guard rejects it. Must render as reading-order text.
fn code_block() -> Document {
    let mut ops = Vec::new();
    let lines = [
        ("def", "main", "():"),
        ("return", "value", "plus"),
        ("xx", "eq", "one"),
        ("yy", "eq", "two"),
    ];
    let shifts = [0i64, 20, 6, 14];
    for (r, (a, b, c)) in lines.iter().enumerate() {
        let y = 700 - (r as i64) * 14;
        let shift = shifts[r];
        text_run(&mut ops, 72 + shift, y, a);
        text_run(&mut ops, 162 + shift, y, b);
        text_run(&mut ops, 252 + shift, y, c);
    }
    base_doc(ops, vec![])
}

/// Negative corpus: a genuine three-column running-prose page (a newspaper-style
/// layout). The columns have clean gutters and tight left edges, so geometry
/// alone would accept it — but every cell is a sentence fragment, so the content
/// discriminator rejects it and it must render as reading-order text.
fn three_column_prose() -> Document {
    let mut ops = Vec::new();
    let rows = [
        [
            "the quick brown fox jumps",
            "beside a calm winding river",
            "while morning light fills sky",
        ],
        [
            "over the lazy sleeping dog",
            "under the tall green oak",
            "as gentle breezes cross fields",
        ],
        [
            "near the old stone bridge",
            "past the quiet village square",
            "toward the distant blue hills",
        ],
        [
            "with baskets full of apples",
            "and songs about the harvest",
            "they wander the narrow lanes",
        ],
        [
            "before the evening bells ring",
            "many travelers pause to rest",
            "sharing bread and warm stories",
        ],
        [
            "until the stars appear above",
            "and lanterns glow along paths",
            "guiding everyone safely back home",
        ],
    ];
    for (r, columns) in rows.iter().enumerate() {
        let y = 700 - (r as i64) * 16;
        for (c, phrase) in columns.iter().enumerate() {
            let x = 72 + (c as i64) * 178;
            text_run(&mut ops, x, y, phrase);
        }
    }
    base_doc(ops, vec![])
}

/// Negative corpus: a two-column key/value form. The first column is
/// colon-terminated labels, so the label/value discriminator declines it and
/// the form stays reading-order text.
fn key_value_form() -> Document {
    let mut ops = Vec::new();
    let pairs = [
        ("Name:", "John Doe"),
        ("Email:", "jdoe@example.test"),
        ("Phone:", "555-0100"),
        ("City:", "Springfield"),
    ];
    for (r, (key, value)) in pairs.iter().enumerate() {
        let y = 700 - (r as i64) * 16;
        text_run(&mut ops, 72, y, key);
        text_run(&mut ops, 200, y, value);
    }
    base_doc(ops, vec![])
}

/// Emits ONE `BT / Tf / Tm / Tj / ET` sequence per glyph — the pathological
/// per-glyph-text-object construct this fixture exists to exercise (some PDF
/// producers, notably certain CAD/plotter exports and font-subsetted
/// re-renderers, place every glyph in its own text object instead of
/// batching a run into a single `Tj`). `words` are placed left-to-right on
/// one baseline at `y`, starting at `x`; each glyph advances the cursor by
/// `char_advance`, and each inter-word gap advances it by `word_gap`
/// (replacing, not adding to, a glyph advance) so the word-boundary jump is
/// distinguishably larger than intra-word glyph spacing — the signal the
/// regroup pass keys on to recover words and lines from glyph fragments.
/// Each glyph's origin is authored directly via `Tm` (not accumulated via
/// `Td`), so positions are exact regardless of real font metrics.
fn glyph_run(
    ops: &mut Vec<Operation>,
    words: &[&str],
    start_x: f64,
    y: f64,
    char_advance: f64,
    word_gap: f64,
) {
    let mut x = start_x;
    for (i, word) in words.iter().enumerate() {
        if i > 0 {
            x += word_gap;
        }
        for ch in word.chars() {
            ops.push(op("BT", vec![]));
            ops.push(op("Tf", vec!["F1".into(), 12.into()]));
            ops.push(op(
                "Tm",
                vec![
                    Object::Real(1.0),
                    Object::Real(0.0),
                    Object::Real(0.0),
                    Object::Real(1.0),
                    Object::Real(x as f32),
                    Object::Real(y as f32),
                ],
            ));
            ops.push(op("Tj", vec![Object::string_literal(ch.to_string())]));
            ops.push(op("ET", vec![]));
            x += char_advance;
        }
    }
}

/// The pathological glyph-fragmented construct this whole feature fixes: two
/// short lines, each glyph its own text object (`BT/Tf/Tm/Tj/ET`), no
/// images. Baselines at PDF-space y=700 ("hello world") and y=680 ("docray
/// glyphs"), 20pt apart (single-spaced for 12pt Helvetica).
///
/// The intra-word advance (5.0pt) and inter-word gap (12.0pt) are tuned
/// against the word-split threshold the regroup pass applies —
/// `0.25 * font_size` = 3pt at this fixture's 12pt font
/// (`crates/docray-model/src/grouping.rs`), measured between *ink* bboxes,
/// not `Tm` origins. A real glyph-fragmented producer places each glyph at
/// its true advance width, so intra-word ink gaps are near zero; 5.0pt
/// keeps every intra-word gap (advance minus the narrowest real Helvetica
/// glyph ink width in this text, `l` at ~2.7pt @ 12pt) comfortably under
/// the 3pt threshold, while wider glyphs (e.g. `w` at ~8.7pt) simply
/// overlap slightly — harmless, since only *positive* gaps can trigger a
/// split. 12.0pt at a word boundary clears the threshold even against the
/// widest adjacent glyph. Verified empirically via
/// `docray extract testdata/glyph_fragmented.pdf --granularity element`,
/// which must read exactly `"hello world"` / `"docray glyphs"` with no
/// intra-word breaks and no merged words.
///
/// Line 1 ("hello world", 10 glyphs, y=700): h=72.0 e=77.0 l=82.0 l=87.0
/// o=92.0 [+12.0 gap] w=109.0 o=114.0 r=119.0 l=124.0 d=129.0.
/// Line 2 ("docray glyphs", 12 glyphs, y=680): d=72.0 o=77.0 c=82.0 r=87.0
/// a=92.0 y=97.0 [+12.0 gap] g=114.0 l=119.0 y=124.0 p=129.0 h=134.0
/// s=139.0.
fn glyph_fragmented() -> Document {
    let mut ops = Vec::new();
    glyph_run(&mut ops, &["hello", "world"], 72.0, 700.0, 5.0, 12.0);
    glyph_run(&mut ops, &["docray", "glyphs"], 72.0, 680.0, 5.0, 12.0);
    base_doc(ops, vec![])
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

/// Native text plus a meaningful, non-dominating image. The image is exactly
/// 300x400pt on a 612x792pt page, so its hand-computed coverage is
/// 120_000 / 484_704 = 0.248. The mapped title glyphs exceed the 0.001 text
/// coverage floor, pinning this page as mixed rather than image or scanned.
fn mixed() -> Document {
    let mut doc = base_doc(
        vec![
            op("BT", vec![]),
            op("Tf", vec!["F1".into(), 18.into()]),
            op("Td", vec![72.into(), 720.into()]),
            op(
                "Tj",
                vec![Object::string_literal("Native text beside artwork")],
            ),
            op("ET", vec![]),
            op("q", vec![]),
            op(
                "cm",
                vec![
                    300.into(),
                    0.into(),
                    0.into(),
                    400.into(),
                    100.into(),
                    200.into(),
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
    broken_encoding()
        .save("testdata/broken-encoding.pdf")
        .unwrap();
    cjk().save("testdata/cjk.pdf").unwrap();
    rotated().save("testdata/rotated.pdf").unwrap();
    image().save("testdata/image.pdf").unwrap();
    scan().save("testdata/scan.pdf").unwrap();
    mixed().save("testdata/mixed.pdf").unwrap();
    link().save("testdata/link.pdf").unwrap();
    form().save("testdata/form.pdf").unwrap();
    ruled_table().save("testdata/ruled-table.pdf").unwrap();
    borderless_table()
        .save("testdata/borderless-table.pdf")
        .unwrap();
    merged_ruled_table()
        .save("testdata/merged-ruled-table.pdf")
        .unwrap();
    merged_borderless_table()
        .save("testdata/merged-borderless-table.pdf")
        .unwrap();
    hostile_cell_table()
        .save("testdata/hostile-cell-table.pdf")
        .unwrap();
    prose().save("testdata/prose.pdf").unwrap();
    code_block().save("testdata/code-block.pdf").unwrap();
    key_value_form()
        .save("testdata/key-value-form.pdf")
        .unwrap();
    three_column_prose()
        .save("testdata/three-column-prose.pdf")
        .unwrap();
    glyph_fragmented()
        .save("testdata/glyph_fragmented.pdf")
        .unwrap();

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
