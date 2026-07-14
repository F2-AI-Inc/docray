use docray_core::Extractor;
use docray_model::Granularity;
use docray_pdf::PdfExtractor;
use std::fs;
use std::path::PathBuf;

/// cargo runs test binaries with CWD = crate dir, so bind()'s relative
/// `./.pdfium/lib` candidate never resolves against the workspace-root lib.
/// Point the documented env var at the workspace-root .pdfium/lib.
fn ensure_pdfium_dir() {
    if std::env::var_os("DOCRAY_PDFIUM_DIR").is_none() {
        std::env::set_var(
            "DOCRAY_PDFIUM_DIR",
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../.pdfium/lib"),
        );
    }
}

fn testdata() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata")
}

#[test]
fn goldens_match() {
    ensure_pdfium_dir();
    let update = std::env::var("UPDATE_GOLDEN").is_ok();
    let mut checked = 0;
    for entry in fs::read_dir(testdata()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("pdf") {
            continue;
        }
        let name = path.file_stem().unwrap().to_str().unwrap().to_string();
        let bytes = fs::read(&path).unwrap();
        let out = PdfExtractor.extract(&bytes, None).unwrap();
        let json = serde_json::to_string_pretty(&out).unwrap();
        let golden_path = testdata().join("golden").join(format!("{name}.json"));
        if update {
            fs::create_dir_all(golden_path.parent().unwrap()).unwrap();
            fs::write(&golden_path, &json).unwrap();
        } else {
            let expected = fs::read_to_string(&golden_path).unwrap_or_else(|_| {
                panic!("missing golden {golden_path:?} - run with UPDATE_GOLDEN=1")
            });
            assert_eq!(json, expected, "golden mismatch for {name}. Diff the files; if the change is intended, rerun with UPDATE_GOLDEN=1 and review the diff in git.");
        }
        checked += 1;
    }
    assert!(
        checked >= 3,
        "expected at least 3 fixture PDFs, found {checked}"
    );
}

#[test]
fn compact_simple_goldens_match() {
    ensure_pdfium_dir();
    let update = std::env::var("UPDATE_GOLDEN").is_ok();
    let bytes = fs::read(testdata().join("simple.pdf")).unwrap();
    let out = PdfExtractor.extract(&bytes, None).unwrap();
    for (level, suffix) in [
        (Granularity::Word, "word"),
        (Granularity::Element, "element"),
    ] {
        let json = serde_json::to_string_pretty(&out.with_granularity(level)).unwrap();
        let golden_path = testdata()
            .join("golden")
            .join(format!("simple.{suffix}.json"));
        if update {
            fs::write(&golden_path, json).unwrap();
        } else {
            let expected = fs::read_to_string(&golden_path).unwrap_or_else(|_| {
                panic!("missing golden {golden_path:?} - run with UPDATE_GOLDEN=1")
            });
            assert_eq!(
                json, expected,
                "compact golden mismatch for {suffix}; run UPDATE_GOLDEN=1 and review the diff"
            );
        }
    }
}

#[test]
fn compact_styles_omit_defaults_but_keep_bold() {
    ensure_pdfium_dir();
    let bytes = fs::read(testdata().join("simple.pdf")).unwrap();
    let out = PdfExtractor.extract(&bytes, None).unwrap();
    let value = serde_json::to_value(out.with_granularity(Granularity::Element)).unwrap();
    let elements = value["pages"][0]["elements"].as_array().unwrap();
    let hello = elements
        .iter()
        .find(|element| element["text"] == "Hello World")
        .unwrap();
    assert!(hello["font"].get("bold").is_none());
    assert!(hello["font"].get("italic").is_none());
    assert!(hello.get("color").is_none());

    let title = elements
        .iter()
        .find(|element| element["text"] == "Bold Title")
        .unwrap();
    assert_eq!(title["font"]["bold"], true);
    assert!(title["font"].get("italic").is_none());
}

/// FIX-1 regression: a `/Rotate 90` page must report the rotated (visible)
/// dimensions and place every coordinate inside that visible box — the pre-fix
/// transform y-flipped raw (unrotated-space) coordinates against the rotated
/// height and produced out-of-range / negative values.
#[test]
fn rotated_page_dims_and_coords_are_post_rotation() {
    ensure_pdfium_dir();
    let bytes = fs::read(testdata().join("rotated.pdf")).unwrap();
    let out = PdfExtractor.extract(&bytes, None).unwrap();
    let page = &out.pages[0];
    // Rotated visible dims: a 612x792 media box with /Rotate 90 is 792x612.
    assert_eq!(
        page.width, 792.0,
        "rotated page width must be visible width"
    );
    assert_eq!(
        page.height, 612.0,
        "rotated page height must be visible height"
    );
    assert_eq!(page.rotation, 90);

    // Every bbox must lie inside the visible page (no negatives, no overflow) —
    // this is exactly what the buggy transform violated.
    let check = |b: &docray_model::BBox| {
        for v in [b.x0, b.y0, b.x1, b.y1] {
            assert!(v >= 0.0, "coordinate {v} is negative: {b:?}");
        }
        assert!(b.x1 <= page.width + 0.001, "x1 {} exceeds page width", b.x1);
        assert!(
            b.y1 <= page.height + 0.001,
            "y1 {} exceeds page height",
            b.y1
        );
    };
    for el in &page.elements {
        match el {
            docray_model::Element::Text(t) => {
                check(&t.bbox);
                for l in &t.lines {
                    check(&l.bbox);
                }
            }
            docray_model::Element::Path(p) => check(&p.bbox),
            docray_model::Element::Image(i) => check(&i.bbox),
            docray_model::Element::Annotation(a) => check(&a.bbox),
        }
    }
}

#[test]
fn extraction_is_deterministic() {
    ensure_pdfium_dir();
    let bytes = fs::read(testdata().join("simple.pdf")).unwrap();
    let a = serde_json::to_string(&PdfExtractor.extract(&bytes, None).unwrap()).unwrap();
    let b = serde_json::to_string(&PdfExtractor.extract(&bytes, None).unwrap()).unwrap();
    assert_eq!(a, b, "same input must produce byte-identical output");
}
