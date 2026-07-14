// Some pdfium-render internal accessors (e.g. `get_raw_image_data`, the URI
// action chain) assert on malformed structures and can panic on hostile input.
// Those panics are library-internal, not ours to catch here: they are contained
// by the CLI-subprocess isolation layer — the server spawns a fresh `docray-cli`
// per document, so a worker abort surfaces as a 'crash' error by architecture
// rather than taking down the service.
use crate::{bind::pdfium, coords::PageSpace};
use docray_core::grouping::{group_into_lines, RawChar};
use docray_core::{sniff_format, ExtractError, Extractor, Format};
use docray_model::*;
use pdfium_render::prelude::*;
use sha2::{Digest, Sha256};

const MAX_FORM_DEPTH: usize = 16;
// A raster must cover at least 85% of the page to classify a zero-text page as scanned.
const SCANNED_IMAGE_COVERAGE_THRESHOLD: f64 = 0.85;

pub struct PdfExtractor;

impl Extractor for PdfExtractor {
    fn extract(&self, bytes: &[u8], max_pages: Option<u32>) -> Result<Extraction, ExtractError> {
        if sniff_format(bytes) != Some(Format::Pdf) {
            return Err(ExtractError::UnsupportedFormat);
        }
        let pdfium = pdfium()?;
        let doc = pdfium
            .load_pdf_from_byte_slice(bytes, None)
            .map_err(map_load_err)?;

        let page_count = doc.pages().len() as u32;
        if let Some(limit) = max_pages {
            if page_count > limit {
                return Err(ExtractError::TooManyPages {
                    limit,
                    actual: page_count,
                });
            }
        }

        let mut warnings = Vec::new();
        let mut pages = Vec::with_capacity(page_count as usize);
        // Index-based access rather than `pages().iter()`: a fused iterator would
        // silently terminate on the first page that fails to load, truncating the
        // document. Every page 1..=page_count must appear in the output.
        for idx in 0..page_count {
            let page_number = idx + 1;
            match doc.pages().get(idx as u16) {
                Ok(page) => match extract_page(&page, page_number, &mut warnings) {
                    Ok(p) => pages.push(p),
                    Err(e) => {
                        warnings.push(format!("page {page_number} failed to parse: {e}"));
                        pages.push(empty_page(&page, page_number));
                    }
                },
                Err(e) => {
                    // The page object itself could not be loaded; dimensions are
                    // unreadable, so emit a placeholder page with 0.0 geometry.
                    warnings.push(format!("page {page_number} failed to parse: {e:?}"));
                    pages.push(Page {
                        page_number,
                        width: 0.0,
                        height: 0.0,
                        rotation: 0,
                        scanned: false,
                        elements: vec![],
                    });
                }
            }
        }

        Ok(Extraction {
            schema_version: "1.1".into(),
            source: Source {
                format: "pdf".into(),
                sha256: hex(&Sha256::digest(bytes)),
                size_bytes: bytes.len() as u64,
            },
            document: DocumentInfo {
                page_count,
                metadata: DocMetadata {
                    title: meta(&doc, PdfDocumentMetadataTagType::Title),
                    author: meta(&doc, PdfDocumentMetadataTagType::Author),
                },
            },
            warnings,
            pages,
        })
    }
}

fn map_load_err(e: PdfiumError) -> ExtractError {
    // pdfium reports password-protected files with a dedicated internal error.
    if format!("{e:?}").contains("Password") {
        ExtractError::EncryptedPdf
    } else {
        ExtractError::ParseFailure(format!("{e:?}"))
    }
}

fn meta(doc: &PdfDocument, tag: PdfDocumentMetadataTagType) -> Option<String> {
    doc.metadata()
        .get(tag)
        .map(|t| t.value().to_string())
        .filter(|s| !s.is_empty())
}

fn rotation_degrees(page: &PdfPage) -> i32 {
    match page.rotation() {
        Ok(PdfPageRenderRotation::None) | Err(_) => 0,
        Ok(PdfPageRenderRotation::Degrees90) => 90,
        Ok(PdfPageRenderRotation::Degrees180) => 180,
        Ok(PdfPageRenderRotation::Degrees270) => 270,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Placeholder page emitted when a page's content fails to parse but the page
/// object itself loaded, so dimensions are still readable.
fn empty_page(page: &PdfPage, page_number: u32) -> Page {
    Page {
        page_number,
        width: round3(page.width().value as f64),
        height: round3(page.height().value as f64),
        rotation: rotation_degrees(page),
        scanned: false,
        elements: vec![],
    }
}

fn page_is_scanned(width: f64, height: f64, elements: &[Element]) -> bool {
    let page_area = width * height;
    if page_area <= 0.0 || !page_area.is_finite() {
        return false;
    }
    if elements
        .iter()
        .any(|element| matches!(element, Element::Text(_)))
    {
        return false;
    }

    let max_image_coverage = elements
        .iter()
        .filter_map(|element| match element {
            Element::Image(image) => {
                let width = (image.bbox.x1 - image.bbox.x0).max(0.0);
                let height = (image.bbox.y1 - image.bbox.y0).max(0.0);
                Some(width * height / page_area)
            }
            _ => None,
        })
        .fold(0.0_f64, f64::max);

    max_image_coverage >= SCANNED_IMAGE_COVERAGE_THRESHOLD
}

fn extract_page(
    page: &PdfPage,
    page_number: u32,
    warnings: &mut Vec<String>,
) -> Result<Page, String> {
    // pdfium reports width/height as the rotated (visible) dims but object/char
    // geometry in the unrotated media-box space; `space` bridges the two.
    let rotation = rotation_degrees(page);
    let space = PageSpace::new(
        rotation,
        page.width().value as f64,
        page.height().value as f64,
    );
    let page_text = page.text().map_err(|e| format!("{e:?}"))?;
    let mut elements = Vec::new();

    // Index-based object access rather than `objects().iter()`: a fused iterator
    // would silently drop the remainder of a page's objects if one failed to
    // load. `get`/`len` come from `PdfPageObjectsCommon` in pdfium-render 0.8.37.
    let objects = page.objects();
    let object_count = objects.len();
    for i in 0..object_count {
        match objects.get(i) {
            Ok(object) => extract_object(
                &object,
                &page_text,
                &space,
                page_number,
                None,
                0,
                &mut elements,
                warnings,
            ),
            Err(e) => {
                warnings.push(format!(
                    "page {page_number}: object {i} failed to load: {e:?}"
                ));
            }
        }
    }

    // Annotations are appended after all page objects, so their element IDs
    // continue the per-page index sequence. Index-based access (as with objects)
    // avoids a fused iterator silently truncating the annotation list mid-page.
    let annotations = page.annotations();
    let annotation_count = annotations.len();
    for i in 0..annotation_count {
        match annotations.get(i) {
            Ok(annotation) => {
                let id = format!("p{page_number}-e{}", elements.len());
                match annotation_element(id, &annotation, &space) {
                    Ok(el) => elements.push(Element::Annotation(el)),
                    Err(w) => warnings.push(w),
                }
            }
            Err(e) => {
                warnings.push(format!(
                    "page {page_number}: annotation {i} failed to load: {e:?}"
                ));
            }
        }
    }

    let width = round3(page.width().value as f64);
    let height = round3(page.height().value as f64);
    let scanned = page_is_scanned(width, height, &elements);

    Ok(Page {
        page_number,
        width,
        height,
        rotation,
        scanned,
        elements,
    })
}

/// Extracts one page object, flattening form children depth-first at the form's
/// position in the page object stream. `ancestor` maps coordinates in the
/// object's containing form to unrotated page space; top-level objects use
/// `None` so their established geometry path remains byte-identical.
#[allow(clippy::too_many_arguments)]
fn extract_object(
    object: &PdfPageObject,
    page_text: &PdfPageText,
    space: &PageSpace,
    page_number: u32,
    ancestor: Option<PdfMatrix>,
    form_depth: usize,
    elements: &mut Vec<Element>,
    warnings: &mut Vec<String>,
) {
    if let Some(text_obj) = object.as_text_object() {
        let id = format!("p{page_number}-e{}", elements.len());
        match text_element(id, text_obj, page_text, space, page_number, ancestor) {
            Ok(Some(el)) => elements.push(Element::Text(el)),
            // Whitespace-only run: no element by design (plan-mandated).
            Ok(None) => {}
            Err(w) => warnings.push(w),
        }
    } else if let Some(img_obj) = object.as_image_object() {
        let id = format!("p{page_number}-e{}", elements.len());
        elements.push(Element::Image(image_element(
            id, object, img_obj, ancestor, space, warnings,
        )));
    } else if let Some(path_obj) = object.as_path_object() {
        let id = format!("p{page_number}-e{}", elements.len());
        elements.push(Element::Path(path_element(
            id, object, path_obj, ancestor, space, warnings,
        )));
    } else if let Some(form) = object.as_x_object_form_object() {
        if form_depth >= MAX_FORM_DEPTH {
            warnings.push(format!(
                "p{page_number}: form nesting depth exceeded, subtree skipped"
            ));
            return;
        }

        // The probe established that a form object's matrix maps from its own
        // child coordinate space into its containing form (or page), while its
        // children's bounds and matrices are local. With row-vector PdfMatrix
        // semantics, local-to-parent must therefore precede parent-to-page.
        let local_to_parent = match object.matrix() {
            Ok(matrix) => matrix,
            Err(e) => {
                warnings.push(format!(
                    "page {page_number}: form matrix unavailable, using identity: {e:?}"
                ));
                PdfMatrix::IDENTITY
            }
        };
        let child_to_page = match ancestor {
            Some(parent_to_page) => local_to_parent.multiply(parent_to_page),
            None => local_to_parent,
        };

        // `len()` is backed by FPDFFormObj_CountObjects(), whose error sentinel
        // -1 is cast to usize by pdfium-render 0.8.37. Detect it explicitly so
        // malformed forms cannot turn an error into an effectively infinite loop.
        let child_count = form.len();
        if child_count == usize::MAX {
            warnings.push(format!(
                "page {page_number}: form children failed to load: invalid object count"
            ));
            return;
        }
        for index in 0..child_count {
            match form.get(index) {
                Ok(child) => extract_object(
                    &child,
                    page_text,
                    space,
                    page_number,
                    Some(child_to_page),
                    form_depth + 1,
                    elements,
                    warnings,
                ),
                Err(e) => warnings.push(format!(
                    "page {page_number}: form object {index} failed to load: {e:?}"
                )),
            }
        }
    } else {
        // Shadings and unknown object kinds remain explicitly unsupported,
        // whether they occur at page level or inside a form.
        let kind = object.object_type();
        warnings.push(format!(
            "p{page_number}-e-skipped: unsupported object type {kind:?}"
        ));
    }
}

/// Builds a text element for one text object.
///
/// Returns `Ok(None)` for the deliberate case of a run whose characters are all
/// whitespace (no element by design). Any real failure to read the object's
/// chars or glyph bounds surfaces as `Err(warning)` so the data loss is recorded
/// on the document rather than silently discarded.
fn text_element(
    id: String,
    obj: &PdfPageTextObject,
    page_text: &PdfPageText,
    space: &PageSpace,
    page_number: u32,
    ancestor: Option<PdfMatrix>,
) -> Result<Option<TextElement>, String> {
    // Pdfium's `scaled_font_size()` includes the text object's own matrix but,
    // for nested objects, not any containing form matrices. Character geometry
    // from FPDFText is already page-space; only the reported font size needs the
    // accumulated form scale applied here.
    let ancestor_scale = ancestor.map(matrix_vertical_scale).unwrap_or(1.0);
    let font_size = obj.scaled_font_size().value as f64 * ancestor_scale;
    let mut raw = Vec::new();
    let mut content = String::new();

    let chars = page_text
        .chars_for_object(obj)
        .map_err(|e| format!("page {page_number}: text object skipped: {e:?}"))?;
    for ch in chars.iter() {
        let c = ch.unicode_char().map(String::from).unwrap_or_default();
        content.push_str(&c);
        let b = ch
            .loose_bounds()
            .map_err(|e| format!("page {page_number}: text object skipped: {e:?}"))?;
        let bbox = space.bbox(
            b.left().value as f64,
            b.bottom().value as f64,
            b.right().value as f64,
            b.top().value as f64,
        );
        // Baseline: transform the char's origin into rotated top-left space and
        // take its y, falling back to the glyph box bottom-left corner only when
        // the origin is unavailable. (On unrotated pages this reduces to
        // `page_height - origin_y`, matching the pre-rotation behaviour.)
        let baseline = match (ch.origin_x(), ch.origin_y()) {
            (Ok(ox), Ok(oy)) => round3(space.point(ox.value as f64, oy.value as f64).1),
            _ => round3(
                space
                    .point(b.left().value as f64, b.bottom().value as f64)
                    .1,
            ),
        };
        raw.push(RawChar {
            unicode: ch.unicode_char().map(|c| c as u32).unwrap_or(0),
            content: c,
            bbox,
            font_size,
            baseline_y: baseline,
        });
    }
    if raw.is_empty() {
        return Ok(None);
    }

    let lines = group_into_lines(&raw);
    if lines.is_empty() {
        return Ok(None); // whitespace-only run
    }
    let bbox = lines
        .iter()
        .skip(1)
        .fold(lines[0].bbox, |acc, l| acc.union(&l.bbox));

    // Fill / stroke colour come from the text object itself: `PdfPageTextObject`
    // implements `PdfPageObjectCommon` (blanket impl over `PdfPageObjectPrivate`
    // in pdfium-render 0.8.37), which exposes object-level `fill_color()` /
    // `stroke_color()` backed by `FPDFPageObj_GetFillColor`.
    let fill = obj
        .fill_color()
        .ok()
        .map(|c| [c.red(), c.green(), c.blue()]);
    let stroke = obj
        .stroke_color()
        .ok()
        .map(|c| [c.red(), c.green(), c.blue()]);

    let font = obj.font();
    // Use the base font name (e.g. "Helvetica-Bold"), not family(): pdfium
    // substitutes the family to a system font ("Arial"), which loses the
    // weight/style hints the fixture encodes in the PostScript name.
    let font_name = font.name();
    let name_lower = font_name.to_lowercase();
    let weight_bold = matches!(
        font.weight(),
        Ok(w) if font_weight_value(&w) >= 600
    );
    Ok(Some(TextElement {
        id,
        bbox,
        content: content.trim_end().to_string(),
        font: Font {
            name: font_name,
            size: round3(font_size),
            bold: weight_bold || name_lower.contains("bold"),
            italic: font.is_italic() || name_lower.contains("italic"),
        },
        color: TextColor { fill, stroke },
        lines,
    }))
}

/// Object bounding box in top-left space.
///
/// `bounds()` returns a `PdfQuadPoints` in pdfium-render 0.8.37 (not a rect); its
/// `left()/bottom()/right()/top()` accessors give the axis-aligned extremes we
/// need. Failure yields `None` so callers can fall back to a zero box rather than
/// panic.
fn object_bbox(
    object: &PdfPageObject,
    ancestor: Option<PdfMatrix>,
    space: &PageSpace,
) -> Option<BBox> {
    let b = object.bounds().ok()?;
    let left = b.left().value as f64;
    let bottom = b.bottom().value as f64;
    let right = b.right().value as f64;
    let top = b.top().value as f64;
    match ancestor {
        None => Some(space.bbox(left, bottom, right, top)),
        Some(matrix) => {
            let corners = [(left, bottom), (right, bottom), (right, top), (left, top)];
            let transformed = corners.map(|(x, y)| transform_point(matrix, x, y));
            let min_x = transformed
                .iter()
                .map(|p| p.0)
                .fold(f64::INFINITY, f64::min);
            let min_y = transformed
                .iter()
                .map(|p| p.1)
                .fold(f64::INFINITY, f64::min);
            let max_x = transformed
                .iter()
                .map(|p| p.0)
                .fold(f64::NEG_INFINITY, f64::max);
            let max_y = transformed
                .iter()
                .map(|p| p.1)
                .fold(f64::NEG_INFINITY, f64::max);
            Some(space.bbox(min_x, min_y, max_x, max_y))
        }
    }
}

fn transform_point(matrix: PdfMatrix, x: f64, y: f64) -> (f64, f64) {
    (
        matrix.a() as f64 * x + matrix.c() as f64 * y + matrix.e() as f64,
        matrix.b() as f64 * x + matrix.d() as f64 * y + matrix.f() as f64,
    )
}

fn matrix_vertical_scale(matrix: PdfMatrix) -> f64 {
    ((matrix.c() as f64).powi(2) + (matrix.d() as f64).powi(2)).sqrt()
}

fn unit_square_bbox(matrix: PdfMatrix, space: &PageSpace) -> BBox {
    let transformed = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
        .map(|(x, y)| transform_point(matrix, x, y));
    let min_x = transformed
        .iter()
        .map(|p| p.0)
        .fold(f64::INFINITY, f64::min);
    let min_y = transformed
        .iter()
        .map(|p| p.1)
        .fold(f64::INFINITY, f64::min);
    let max_x = transformed
        .iter()
        .map(|p| p.0)
        .fold(f64::NEG_INFINITY, f64::max);
    let max_y = transformed
        .iter()
        .map(|p| p.1)
        .fold(f64::NEG_INFINITY, f64::max);
    space.bbox(min_x, min_y, max_x, max_y)
}

/// Object bbox, or a zero box plus a page warning when bounds are unreadable.
/// GEOMETRY is lossless-but-labeled: the element is still emitted (with the zero
/// box) so nothing is silently dropped, but the failure is recorded.
fn object_bbox_or_warn(
    object: &PdfPageObject,
    ancestor: Option<PdfMatrix>,
    space: &PageSpace,
    id: &str,
    warnings: &mut Vec<String>,
) -> BBox {
    match object_bbox(object, ancestor, space) {
        Some(b) => b,
        None => {
            warnings.push(format!("{id}: bounds unavailable"));
            BBox {
                x0: 0.0,
                y0: 0.0,
                x1: 0.0,
                y1: 0.0,
            }
        }
    }
}

fn image_element(
    id: String,
    object: &PdfPageObject,
    img: &PdfPageImageObject,
    ancestor: Option<PdfMatrix>,
    space: &PageSpace,
    warnings: &mut Vec<String>,
) -> ImageElement {
    // GEOMETRY (bbox, quad) is lossless-but-labeled: a failure emits a page
    // warning and the element is still emitted with a best-effort value.
    //
    // Option-typed METADATA (pixel_width/height, colorspace, content_hash) is
    // NOT warned on: a `None` here is the schema's declared way of saying "this
    // attribute was unavailable", so failing accessors stay silently `None`.
    let matrix = object.matrix().ok().map(|matrix| match ancestor {
        Some(parent_to_page) => matrix.multiply(parent_to_page),
        None => matrix,
    });
    // For nested images, transforming the local axis-aligned object bbox would
    // become too large when an ancestor rotates or skews the form. The composed
    // image matrix gives the exact page-space unit-square bounds instead.
    let bbox = match (ancestor, matrix) {
        (Some(_), Some(matrix)) => unit_square_bbox(matrix, space),
        _ => object_bbox_or_warn(object, ancestor, space, &id, warnings),
    };
    let quad = image_quad(matrix, &bbox, space, &id, warnings);

    ImageElement {
        id,
        bbox,
        quad,
        // `width()`/`height()` return the pixel dimensions from pdfium's image
        // metadata (the raw `FPDF_IMAGEOBJ_METADATA` accessor is `pub(crate)`).
        pixel_width: img.width().ok().map(|w| w as u32),
        pixel_height: img.height().ok().map(|h| h as u32),
        colorspace: img
            .color_space()
            .ok()
            .map(|cs| colorspace_name(cs as i32).to_string()),
        content_hash: img
            .get_raw_image_data()
            .ok()
            .filter(|d| !d.is_empty())
            .map(|d| hex(&Sha256::digest(&d))),
    }
}

/// Image placement quad in top-left space. The entire quad policy lives here so
/// there is exactly one place that decides the corner ordering:
/// - matrix unavailable -> warn and approximate the quad from the bbox corners;
/// - axis-aligned matrix (after round3: `b == 0 && c == 0 && a > 0 && d > 0`) ->
///   emit the canonical [tl, tr, br, bl] quad straight from the bbox, avoiding
///   the float-equality pitfalls of comparing transformed corners to the bbox;
/// - otherwise -> the transformed unit-square corners, y-flipped.
fn image_quad(
    matrix: Option<PdfMatrix>,
    bbox: &BBox,
    space: &PageSpace,
    id: &str,
    warnings: &mut Vec<String>,
) -> [[f64; 2]; 4] {
    let canonical = [
        [bbox.x0, bbox.y0],
        [bbox.x1, bbox.y0],
        [bbox.x1, bbox.y1],
        [bbox.x0, bbox.y1],
    ];
    let m = match matrix {
        Some(m) => m,
        None => {
            warnings.push(format!(
                "{id}: matrix unavailable, quad approximated from bbox"
            ));
            return canonical;
        }
    };
    // Decide axis-alignment from the matrix itself rather than by comparing
    // rounded transformed corners against f32-derived bbox corners.
    if round3(m.b() as f64) == 0.0
        && round3(m.c() as f64) == 0.0
        && round3(m.a() as f64) > 0.0
        && round3(m.d() as f64) > 0.0
    {
        return canonical;
    }
    // Image object space is the unit square transformed by the object matrix;
    // the quad is that square's corners, y-flipped into top-left space.
    let corners = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
    corners.map(|(x, y)| {
        let px = m.a() as f64 * x + m.c() as f64 * y + m.e() as f64;
        let py = m.b() as f64 * x + m.d() as f64 * y + m.f() as f64;
        let (qx, qy) = space.point(px, py);
        [round3(qx), round3(qy)]
    })
}

/// Maps pdfium's `FPDF_COLORSPACE_*` integer to a human-readable name. The
/// discriminants of `PdfColorSpace` equal these constants, so callers cast the
/// enum to `i32` before passing it in.
fn colorspace_name(cs: i32) -> &'static str {
    match cs {
        1 => "DeviceGray",
        2 => "DeviceRGB",
        3 => "DeviceCMYK",
        4 => "CalGray",
        5 => "CalRGB",
        6 => "Lab",
        7 => "ICCBased",
        8 => "Separation",
        9 => "DeviceN",
        10 => "Indexed",
        11 => "Pattern",
        _ => "Unknown",
    }
}

fn path_element(
    id: String,
    object: &PdfPageObject,
    _path: &PdfPagePathObject,
    ancestor: Option<PdfMatrix>,
    space: &PageSpace,
    warnings: &mut Vec<String>,
) -> PathElement {
    let bbox = object_bbox_or_warn(object, ancestor, space, &id, warnings);
    // Fill/stroke colours and stroke width come from the `PdfPageObjectCommon`
    // blanket impl over the generic object wrapper.
    PathElement {
        id,
        bbox,
        fill: object
            .fill_color()
            .ok()
            .map(|c| [c.red(), c.green(), c.blue()]),
        stroke: object
            .stroke_color()
            .ok()
            .map(|c| [c.red(), c.green(), c.blue()]),
        stroke_width: object.stroke_width().ok().map(|w| {
            let ancestor_scale = ancestor.map(matrix_vertical_scale).unwrap_or(1.0);
            round3(w.value as f64 * ancestor_scale)
        }),
    }
}

/// Builds an annotation element. Returns `Err(warning)` when the annotation has
/// no readable bounds, so the data loss is recorded on the document rather than
/// silently dropped (consistent with the hardened per-page error policy).
fn annotation_element(
    id: String,
    annotation: &PdfPageAnnotation,
    space: &PageSpace,
) -> Result<AnnotationElement, String> {
    let b = annotation
        .bounds()
        .map_err(|_| format!("{id}: annotation skipped: no bounds"))?;
    let bbox = space.bbox(
        b.left().value as f64,
        b.bottom().value as f64,
        b.right().value as f64,
        b.top().value as f64,
    );
    let subtype = format!("{:?}", annotation.annotation_type()).to_lowercase();
    // Link URI chain: annotation -> link -> action -> URI action -> uri string.
    // `PdfLink::action()` returns an `Option` (not a `Result`) in 0.8.37.
    let uri = annotation.as_link_annotation().and_then(|link| {
        link.link()
            .ok()
            .and_then(|l| l.action())
            .and_then(|a| a.as_uri_action().and_then(|u| u.uri().ok()))
    });
    Ok(AnnotationElement {
        id,
        bbox,
        subtype,
        uri,
    })
}

fn font_weight_value(w: &PdfFontWeight) -> u32 {
    match w {
        PdfFontWeight::Weight100 => 100,
        PdfFontWeight::Weight200 => 200,
        PdfFontWeight::Weight300 => 300,
        PdfFontWeight::Weight400Normal => 400,
        PdfFontWeight::Weight500 => 500,
        PdfFontWeight::Weight600 => 600,
        PdfFontWeight::Weight700Bold => 700,
        PdfFontWeight::Weight800 => 800,
        PdfFontWeight::Weight900 => 900,
        PdfFontWeight::Custom(v) => *v,
    }
}
