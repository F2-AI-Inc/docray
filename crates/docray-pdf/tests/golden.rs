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

/// Goldens are canonical on Linux (the CI and production platform) and
/// compared byte-exactly there. Fixtures use non-embedded base-14 fonts, and
/// pdfium substitutes platform fonts for those, so glyph metrics drift by
/// fractions of a point across OSes. On non-Linux dev machines we therefore
/// fall back to a structural comparison with a small numeric tolerance —
/// (metric drift scales with font size: ~1pt at 18pt). Still catches real regressions (missing elements, wrong fields, moved
/// boxes) without failing on font-substitution noise.
const NUMERIC_TOLERANCE: f64 = 1.5;

fn assert_matches_golden(actual: &str, expected: &str, what: &str) {
    if actual == expected {
        return;
    }
    if cfg!(target_os = "linux") {
        panic!(
            "golden mismatch for {what} (byte-exact on Linux). \
             If the change is intended, rerun with UPDATE_GOLDEN=1 on Linux \
             (e.g. inside the Docker builder) and review the diff."
        );
    }
    let a: serde_json::Value = serde_json::from_str(actual).unwrap();
    let e: serde_json::Value = serde_json::from_str(expected).unwrap();
    if let Err(path) = json_close(&a, &e) {
        panic!("golden mismatch for {what} beyond numeric tolerance at {path}");
    }
    eprintln!(
        "note: {what} matched goldens structurally (non-Linux font metrics differ; \
         byte-exact comparison happens in CI)"
    );
}

/// Recursive equality with |a-b| <= NUMERIC_TOLERANCE for numbers.
/// Returns the JSON path of the first mismatch.
fn json_close(a: &serde_json::Value, b: &serde_json::Value) -> Result<(), String> {
    use serde_json::Value::*;
    match (a, b) {
        (Number(x), Number(y)) => {
            let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
            if (x - y).abs() <= NUMERIC_TOLERANCE {
                Ok(())
            } else {
                Err(format!("number {x} vs {y}"))
            }
        }
        (Array(xs), Array(ys)) => {
            if xs.len() != ys.len() {
                return Err(format!("array length {} vs {}", xs.len(), ys.len()));
            }
            for (i, (x, y)) in xs.iter().zip(ys).enumerate() {
                json_close(x, y).map_err(|p| format!("[{i}].{p}"))?;
            }
            Ok(())
        }
        (Object(xm), Object(ym)) => {
            if xm.len() != ym.len() {
                return Err(format!("object size {} vs {}", xm.len(), ym.len()));
            }
            for (k, x) in xm {
                let y = ym.get(k).ok_or_else(|| format!("missing key {k}"))?;
                json_close(x, y).map_err(|p| format!("{k}.{p}"))?;
            }
            Ok(())
        }
        _ => {
            if a == b {
                Ok(())
            } else {
                Err(format!("{a} vs {b}"))
            }
        }
    }
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
            assert_matches_golden(&json, &expected, &name);
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
            assert_matches_golden(&json, &expected, &format!("simple.{suffix}"));
        }
    }
}

#[test]
fn lean_simple_goldens_match_on_linux() {
    if !cfg!(target_os = "linux") {
        eprintln!(
            "note: lean goldens are skipped off Linux; JSON goldens already guard geometry with tolerance"
        );
        return;
    }

    ensure_pdfium_dir();
    let update = std::env::var("UPDATE_GOLDEN").is_ok();
    let bytes = fs::read(testdata().join("simple.pdf")).unwrap();
    let out = PdfExtractor.extract(&bytes, None).unwrap();
    for (level, suffix) in [
        (Granularity::Word, "word"),
        (Granularity::Element, "element"),
    ] {
        let lean = match out.with_granularity(level) {
            docray_model::GranularExtraction::Compact(compact) => compact.to_lean(),
            docray_model::GranularExtraction::Char(_) => unreachable!(),
        };
        let golden_path = testdata()
            .join("golden")
            .join(format!("simple.{suffix}.lean.txt"));
        if update {
            fs::write(&golden_path, lean).unwrap();
        } else {
            let expected = fs::read_to_string(&golden_path).unwrap_or_else(|_| {
                panic!("missing golden {golden_path:?} - run with UPDATE_GOLDEN=1 on Linux")
            });
            assert_eq!(
                lean, expected,
                "lean golden mismatch for simple.{suffix} (byte-exact on Linux)"
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
