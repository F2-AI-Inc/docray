use docray_core::Extractor;
use docray_model::{BBox, Element};
use docray_pdf::PdfExtractor;

fn ensure_pdfium_dir() {
    if std::env::var_os("DOCRAY_PDFIUM_DIR").is_none() {
        std::env::set_var(
            "DOCRAY_PDFIUM_DIR",
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../.pdfium/lib"),
        );
    }
}

fn extract(fixture: &str) -> docray_model::Extraction {
    ensure_pdfium_dir();
    let path = format!("{}/../../testdata/{fixture}", env!("CARGO_MANIFEST_DIR"));
    PdfExtractor
        .extract(&std::fs::read(path).unwrap(), None)
        .unwrap()
}

fn near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 2.0,
        "expected {expected} +/- 2, got {actual}"
    );
}

fn element_bbox(element: &Element) -> BBox {
    match element {
        Element::Text(element) => element.bbox,
        Element::Table(element) => element.bbox,
        Element::Image(element) => element.bbox,
        Element::Path(element) => element.bbox,
        Element::Annotation(element) => element.bbox,
    }
}

#[test]
fn recursively_flattens_form_objects_in_page_space() {
    let out = extract("form.pdf");
    let page = &out.pages[0];

    let texts = page
        .elements
        .iter()
        .filter_map(|element| match element {
            Element::Text(text) => Some(text),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(texts.iter().any(|text| text.content == "Top level text"));

    let scaled = texts
        .iter()
        .find(|text| text.content == "Scaled form text")
        .expect("scaled form text missing");
    // Local baseline (10,20) under [2 0 0 2 100 300] becomes PDF-space
    // (120,340), or top-left baseline y=792-340=452.
    near(scaled.bbox.x0, 120.0);
    near(
        scaled.lines.as_ref().expect("PDF hierarchy missing")[0].baseline_y,
        452.0,
    );
    assert_eq!(scaled.font.size, 20.0, "10pt text under 2x form scale");

    let path = page
        .elements
        .iter()
        .find_map(|element| match element {
            Element::Path(path) => Some(path),
            _ => None,
        })
        .expect("form rect missing");
    // The local rect [0,0]-[50,25] becomes PDF [100,300]-[200,350],
    // hence top-left [100,442]-[200,492]. Pdfium expands its object bounds
    // by the full 1pt local stroke width, which becomes 2pt at this scale.
    near(path.bbox.x0, 98.0);
    near(path.bbox.y0, 440.0);
    near(path.bbox.x1, 202.0);
    near(path.bbox.y1, 494.0);
    assert_eq!(path.stroke_width, Some(2.0));

    let image = page
        .elements
        .iter()
        .find_map(|element| match element {
            Element::Image(image) => Some(image),
            _ => None,
        })
        .expect("form image missing");
    assert_eq!(
        image.bbox,
        BBox {
            x0: 160.0,
            y0: 362.0,
            x1: 180.0,
            y1: 372.0
        }
    );
    assert_eq!(
        image.quad,
        [
            [160.0, 362.0],
            [180.0, 362.0],
            [180.0, 372.0],
            [160.0, 372.0]
        ]
    );

    let nested = texts
        .iter()
        .find(|text| text.content == "Nested form text")
        .expect("nested form text missing");
    // (10,20) + inner /Matrix (5,7) + inner cm (20,40) + outer cm
    // (300,500) = PDF-space baseline (335,567), top-left y=225.
    near(nested.bbox.x0, 335.0);
    near(
        nested.lines.as_ref().expect("PDF hierarchy missing")[0].baseline_y,
        225.0,
    );

    let order_text = texts
        .iter()
        .find(|text| text.content == "Order")
        .expect("non-commutative form text missing");
    // Text local (0,10), then inner cm (30,10), then outer [2 0 0 2 400 100]
    // gives PDF baseline (460,140), or top-left y=652.
    near(order_text.bbox.x0, 460.0);
    near(
        order_text.lines.as_ref().expect("PDF hierarchy missing")[0].baseline_y,
        652.0,
    );

    let order_path = page
        .elements
        .iter()
        .find_map(|element| match element {
            Element::Path(path) if path.bbox.x0 > 400.0 => Some(path),
            _ => None,
        })
        .expect("non-commutative form rect missing");
    // multiply order: local-then-ancestor. The local rect x0=0 first receives
    // the inner +30 translation, then the outer 2x scale/+400 translation:
    // (0 + 30) * 2 + 400 = 460. Reversing the multiply order yields 430.
    assert_eq!(
        order_path.bbox,
        BBox {
            x0: 460.0,
            y0: 652.0,
            x1: 500.0,
            y1: 672.0,
        }
    );

    assert!(
        out.warnings
            .iter()
            .all(|warning| !warning.contains("XObjectForm")),
        "unexpected form skip warning: {:?}",
        out.warnings
    );
    assert!(
        out.warnings.is_empty(),
        "unexpected warnings: {:?}",
        out.warnings
    );

    for (index, element) in page.elements.iter().enumerate() {
        let id = match element {
            Element::Text(element) => &element.id,
            Element::Table(element) => &element.id,
            Element::Image(element) => &element.id,
            Element::Path(element) => &element.id,
            Element::Annotation(element) => &element.id,
        };
        assert_eq!(id, &format!("p1-e{index}"));
        let bbox = element_bbox(element);
        assert!(bbox.x0 <= bbox.x1 && bbox.y0 <= bbox.y1);
    }
}

#[test]
fn form_recursion_stops_at_depth_cap() {
    let out = extract("malformed/deep-forms.pdf");
    assert_eq!(out.pages.len(), 1);
    assert!(out.pages[0].elements.is_empty());
    assert_eq!(
        out.warnings,
        vec!["p1: form nesting depth exceeded, subtree skipped"]
    );
}
