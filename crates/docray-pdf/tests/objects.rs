use docray_core::Extractor;
use docray_model::Element;
use docray_pdf::PdfExtractor;
use sha2::{Digest, Sha256};

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

fn extract(fixture: &str) -> docray_model::Extraction {
    ensure_pdfium_dir();
    let path = format!("{}/../../testdata/{fixture}", env!("CARGO_MANIFEST_DIR"));
    PdfExtractor
        .extract(&std::fs::read(path).unwrap(), None, None)
        .unwrap()
}

#[test]
fn extracts_image_with_quad_and_metadata() {
    let out = extract("image.pdf");
    let img = out.pages[0]
        .elements
        .iter()
        .find_map(|e| match e {
            Element::Image(i) => Some(i),
            _ => None,
        })
        .expect("no image element found");
    // Placed at (100,500) size 100x100 in PDF space; page height 792.
    // Top-left space: x 100..200, y 192..292.
    assert!((img.bbox.x0 - 100.0).abs() < 1.0, "{:?}", img.bbox);
    assert!((img.bbox.y0 - 192.0).abs() < 1.0, "{:?}", img.bbox);
    assert_eq!(img.pixel_width, Some(2));
    assert_eq!(img.pixel_height, Some(2));
    // The 2x2 image HAS raw data: content_hash is a real 64-char lowercase-hex
    // SHA-256, not None.
    let hash = img
        .content_hash
        .as_deref()
        .expect("content_hash should be Some");
    assert_eq!(hash.len(), 64, "{hash}");
    assert!(
        hash.chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
        "{hash}"
    );
    assert_eq!(img.colorspace.as_deref(), Some("DeviceGray"));
    // Axis-aligned placement: all four quad corners match bbox corners in
    // canonical [tl, tr, br, bl] order.
    assert_eq!(img.quad[0], [img.bbox.x0, img.bbox.y0]);
    assert_eq!(img.quad[1], [img.bbox.x1, img.bbox.y0]);
    assert_eq!(img.quad[2], [img.bbox.x1, img.bbox.y1]);
    assert_eq!(img.quad[3], [img.bbox.x0, img.bbox.y1]);
}

/// `content_hash` must hash the image stream bytes stored in the file. In an
/// encrypted document PDFium decodes the JPEG from the same in-memory
/// decrypted buffer the raw bytes are later read from, so a decoder that
/// patches its input in place leaks into the hash.
#[test]
fn encrypted_jpeg_content_hash_is_stored_stream_bytes() {
    let path = format!(
        "{}/../../testdata/owner-password-jpeg.pdf",
        env!("CARGO_MANIFEST_DIR")
    );
    let source = lopdf::Document::load_mem(&std::fs::read(path).unwrap()).unwrap();
    assert!(source.was_encrypted());
    let stream = source
        .objects
        .values()
        .find_map(|object| match object {
            lopdf::Object::Stream(s) if s.dict.has_type(b"XObject") => Some(s),
            _ => None,
        })
        .expect("fixture has an image stream");
    assert!(stream.content.starts_with(&[0xFF, 0xD8]));
    assert!(!stream.content.ends_with(&[0xFF, 0xD9]));
    let expected: String = Sha256::digest(&stream.content)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    let out = extract("owner-password-jpeg.pdf");
    let img = out.pages[0]
        .elements
        .iter()
        .find_map(|e| match e {
            Element::Image(i) => Some(i),
            _ => None,
        })
        .expect("no image element found");
    assert_eq!(img.colorspace.as_deref(), Some("DeviceGray"));
    assert_eq!(img.content_hash.as_deref(), Some(expected.as_str()));
}

#[test]
fn extracts_stroked_rect_as_path() {
    let out = extract("simple.pdf");
    let path = out.pages[0]
        .elements
        .iter()
        .find_map(|e| match e {
            Element::Path(p) => Some(p),
            _ => None,
        })
        .expect("no path element found");
    // re 100 100 200 50 -> PDF space rect; top-left space y = 792-150..792-100.
    assert!((path.bbox.x0 - 100.0).abs() < 2.0, "{:?}", path.bbox);
    assert!((path.bbox.y0 - 642.0).abs() < 2.0, "{:?}", path.bbox);
    assert_eq!(path.stroke, Some([255, 0, 0]));
    assert_eq!(path.stroke_width, Some(1.5));
}

#[test]
fn extracts_link_annotation_with_uri() {
    let out = extract("link.pdf");
    let annot = out.pages[0]
        .elements
        .iter()
        .find_map(|e| match e {
            Element::Annotation(a) => Some(a),
            _ => None,
        })
        .expect("no annotation element found");
    assert_eq!(annot.subtype, "link");
    assert_eq!(annot.uri.as_deref(), Some("https://example.com"));
}
