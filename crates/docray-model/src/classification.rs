use crate::{round3, CompactElement, Element, Extraction, Granularity, HiddenItem};
use serde::{Deserialize, Serialize};

const TEXT_COVERAGE_FLOOR: f64 = 0.001;
const MEANINGFUL_TEXT_COVERAGE: f64 = 0.02;
const TEXT_COVERAGE_OPERATING_POINT: f64 = 0.15;
// The confidence ramp for a correct Text/Mixed page starts well below the
// classification threshold: genuinely dense-but-low-coverage pages (small
// fonts, tight leading) sit at coverage 0.005-0.02 yet are unambiguously
// text, and must not be pinned to the 0.5 floor. This only moves the
// confidence ramp's lower bound; the 0.02 classification threshold is
// unchanged.
const TEXT_CONFIDENCE_RAMP_FLOOR: f64 = 0.005;
// Significant image area carrying almost no mapped text is an under-read scan
// that OCR should recover. The low text gate protects figure pages that DO
// carry real text (they keep needs_ocr=false).
const OCR_SPARSE_TEXT_COVERAGE: f64 = 0.05;
const MEANINGFUL_TEXT_GLYPHS: usize = 8;
const MEANINGFUL_IMAGE_COVERAGE: f64 = 0.01;
const MIXED_IMAGE_COVERAGE: f64 = 0.20;
const DOMINATING_IMAGE_COVERAGE: f64 = 0.60;
const FULL_PAGE_IMAGE_COVERAGE: f64 = 0.85;
const MEANINGFUL_PATH_DENSITY: f64 = 0.10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageKind {
    Text,
    Scanned,
    Image,
    Mixed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageClassification {
    pub kind: PageKind,
    pub confidence: f64,
    pub needs_ocr: bool,
    pub reasons: Vec<String>,
}

/// Schema-1.8 paged response selected only by an explicit classify request.
///
/// The full and compact variants deliberately leave the frozen `Extraction`
/// and schema-1.6 projections unchanged. An optional granularity discriminator
/// preserves the existing explicit-char envelope while classify-only output
/// retains the full lossless hierarchy without inventing a granularity field.
#[derive(Serialize)]
#[serde(untagged)]
pub enum ClassifiedExtraction<'a> {
    Full(ClassifiedFullExtraction<'a>),
    Compact(ClassifiedCompactExtraction),
}

#[derive(Serialize)]
pub struct ClassifiedFullExtraction<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub granularity: Option<Granularity>,
    pub schema_version: &'static str,
    pub source: &'a crate::Source,
    pub document: &'a crate::DocumentInfo,
    pub warnings: &'a Vec<String>,
    pub pages: Vec<ClassifiedFullPage<'a>>,
}

#[derive(Serialize)]
pub struct ClassifiedFullPage<'a> {
    pub page_number: u32,
    pub width: f64,
    pub height: f64,
    pub rotation: i32,
    pub scanned: bool,
    pub classification: PageClassification,
    pub elements: &'a Vec<Element>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hidden: &'a Vec<HiddenItem>,
}

#[derive(Serialize)]
pub struct ClassifiedCompactExtraction {
    pub granularity: Granularity,
    pub schema_version: &'static str,
    pub source: crate::Source,
    pub document: crate::CompactDocumentInfo,
    pub warnings: Vec<String>,
    pub pages: Vec<ClassifiedCompactPage>,
}

#[derive(Serialize)]
pub struct ClassifiedCompactPage {
    pub page_number: u32,
    pub width: f64,
    pub height: f64,
    pub rotation: i32,
    pub scanned: bool,
    pub classification: PageClassification,
    pub elements: Vec<CompactElement>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hidden: Vec<HiddenItem>,
}

#[derive(Default)]
struct PageSignals {
    text_coverage: f64,
    text_elements: usize,
    glyphs: usize,
    image_coverage: f64,
    largest_image_ratio: f64,
    path_density: f64,
    image_count: usize,
    path_count: usize,
}

impl Extraction {
    /// Produces schema 1.8 without changing any default or schema-1.6 bytes.
    pub fn with_classification(
        &self,
        granularity: Option<Granularity>,
    ) -> ClassifiedExtraction<'_> {
        match granularity {
            None | Some(Granularity::Char) => {
                ClassifiedExtraction::Full(ClassifiedFullExtraction {
                    granularity,
                    schema_version: "1.8",
                    source: &self.source,
                    document: &self.document,
                    warnings: &self.warnings,
                    pages: self
                        .pages
                        .iter()
                        .map(|page| ClassifiedFullPage {
                            page_number: page.page_number,
                            width: page.width,
                            height: page.height,
                            rotation: page.rotation,
                            scanned: page.scanned,
                            classification: classify_page(page, &self.warnings),
                            elements: &page.elements,
                            hidden: &page.hidden,
                        })
                        .collect(),
                })
            }
            Some(granularity @ (Granularity::Element | Granularity::Word)) => {
                ClassifiedExtraction::Compact(ClassifiedCompactExtraction {
                    granularity,
                    schema_version: "1.8",
                    source: self.source.clone(),
                    document: crate::CompactDocumentInfo {
                        page_count: self.document.page_count,
                        metadata: crate::CompactDocMetadata {
                            title: self.document.metadata.title.clone(),
                            author: self.document.metadata.author.clone(),
                        },
                    },
                    warnings: self.warnings.clone(),
                    pages: self
                        .pages
                        .iter()
                        .map(|page| {
                            let compact = super::compact_page(page, granularity);
                            ClassifiedCompactPage {
                                page_number: compact.page_number,
                                width: compact.width,
                                height: compact.height,
                                rotation: compact.rotation,
                                scanned: compact.scanned,
                                classification: classify_page(page, &self.warnings),
                                elements: compact.elements,
                                hidden: compact.hidden,
                            }
                        })
                        .collect(),
                })
            }
        }
    }
}

pub fn classify_page(page: &crate::Page, warnings: &[String]) -> PageClassification {
    let signals = page_signals(page);
    let full_page_raster = signals.largest_image_ratio >= DOMINATING_IMAGE_COVERAGE;
    let sparse_text = signals.text_coverage < MEANINGFUL_TEXT_COVERAGE;
    // A dominant raster carrying only a stray glyph layer is a scan, not a text
    // page: glyph count must not vouch for "meaningful text" in that case.
    let meaningful_text = signals.text_coverage >= MEANINGFUL_TEXT_COVERAGE
        || (signals.glyphs >= MEANINGFUL_TEXT_GLYPHS && !(full_page_raster && sparse_text));
    let full_page_image = signals.largest_image_ratio >= FULL_PAGE_IMAGE_COVERAGE;
    let mixed = meaningful_text && signals.image_coverage >= MIXED_IMAGE_COVERAGE;
    let garbled = warnings.iter().any(|warning| {
        warning.starts_with(&format!(
            "page {}: suspected_garbled_text:",
            page.page_number
        ))
    });

    let kind = if !meaningful_text && full_page_image {
        PageKind::Scanned
    } else if mixed {
        PageKind::Mixed
    } else if meaningful_text {
        PageKind::Text
    } else {
        // With no meaningful mapped text, a significant raster, vector-only
        // artwork, and a genuinely blank page all need visual recovery. The
        // four-value public contract groups those cases under `image`.
        PageKind::Image
    };

    // Significant image + almost no text = an under-read scan. The low text
    // gate keeps figure pages that carry real text at needs_ocr=false.
    let image_present_sparse_text = signals.image_coverage >= MIXED_IMAGE_COVERAGE
        && signals.text_coverage < OCR_SPARSE_TEXT_COVERAGE;
    let needs_ocr = garbled
        || matches!(kind, PageKind::Scanned | PageKind::Image)
        || signals.largest_image_ratio >= DOMINATING_IMAGE_COVERAGE
        || signals.text_coverage < TEXT_COVERAGE_FLOOR
        || image_present_sparse_text;

    let mut reasons = vec![
        ratio_reason("text_coverage", signals.text_coverage),
        format!("text_elements={}", signals.text_elements),
    ];
    if signals.image_count > 0 {
        reasons.push(ratio_reason("image_coverage", signals.image_coverage));
        reasons.push(ratio_reason(
            "largest_image_ratio",
            signals.largest_image_ratio,
        ));
    }
    if full_page_image {
        reasons.push("image_covers_page".into());
    }
    if signals.path_count > 0 {
        reasons.push(ratio_reason("vector_path_density", signals.path_density));
    }
    if garbled {
        reasons.push("suspected_garbled_text".into());
    }
    if image_present_sparse_text {
        reasons.push("image_present_sparse_text".into());
    }

    PageClassification {
        kind,
        // This is a deterministic decision-margin score, not a statistically
        // calibrated probability. Corpus calibration is intentionally separate.
        confidence: classification_confidence(kind, &signals),
        needs_ocr,
        reasons,
    }
}

fn page_signals(page: &crate::Page) -> PageSignals {
    let page_area = positive_area(0.0, 0.0, page.width, page.height).max(1.0);
    let mut signals = PageSignals::default();
    let mut text_area = 0.0;
    let mut image_area = 0.0;

    for element in &page.elements {
        match element {
            Element::Text(text) => {
                signals.text_elements += 1;
                let mut hierarchy_area = 0.0;
                let mut hierarchy_glyphs = 0;
                for ch in text
                    .lines
                    .iter()
                    .flatten()
                    .flat_map(|line| &line.words)
                    .flat_map(|word| &word.chars)
                {
                    hierarchy_area += clipped_bbox_area(&ch.bbox, page.width, page.height);
                    hierarchy_glyphs += 1;
                }
                if hierarchy_glyphs > 0 {
                    text_area += hierarchy_area;
                    signals.glyphs += hierarchy_glyphs;
                } else {
                    // PPTX has element boxes but intentionally no glyph hierarchy.
                    text_area += clipped_bbox_area(&text.bbox, page.width, page.height);
                    signals.glyphs += text.content.chars().count();
                }
            }
            Element::Table(table) => {
                let content_chars: usize = table
                    .cells
                    .iter()
                    .map(|cell| cell.content.chars().count())
                    .sum();
                if content_chars > 0 {
                    signals.text_elements += 1;
                    signals.glyphs += content_chars;
                    // Table cells retain text but not glyph boxes. Bound the fallback
                    // estimate so a large sparse grid cannot claim full text coverage.
                    text_area += (content_chars as f64 * 60.0).min(clipped_bbox_area(
                        &table.bbox,
                        page.width,
                        page.height,
                    ));
                }
            }
            Element::Image(image) => {
                signals.image_count += 1;
                let area = clipped_bbox_area(&image.bbox, page.width, page.height);
                image_area += area;
                signals.largest_image_ratio = signals.largest_image_ratio.max(area / page_area);
            }
            Element::Path(_) => signals.path_count += 1,
            Element::Chart(_) | Element::Annotation(_) => {}
        }
    }

    signals.text_coverage = (text_area / page_area).clamp(0.0, 1.0);
    signals.image_coverage = (image_area / page_area).clamp(0.0, 1.0);
    signals.path_density = signals.path_count as f64 / (page_area / 10_000.0);
    signals
}

fn classification_confidence(kind: PageKind, signals: &PageSignals) -> f64 {
    let text_above = ((signals.text_coverage - TEXT_CONFIDENCE_RAMP_FLOOR)
        / (TEXT_COVERAGE_OPERATING_POINT - TEXT_CONFIDENCE_RAMP_FLOOR))
        .clamp(0.0, 1.0);
    let text_below = normalized_below(signals.text_coverage, TEXT_COVERAGE_FLOOR);
    let image_above = normalized_above(signals.image_coverage, MEANINGFUL_IMAGE_COVERAGE);
    let image_below_mixed = normalized_below(signals.image_coverage, MIXED_IMAGE_COVERAGE);
    let mixed_image_above = normalized_above(signals.image_coverage, MIXED_IMAGE_COVERAGE);
    let scan_image_above =
        normalized_to_ceiling(signals.largest_image_ratio, FULL_PAGE_IMAGE_COVERAGE);
    let path_above = normalized_above(signals.path_density, MEANINGFUL_PATH_DENSITY);

    let margin = match kind {
        PageKind::Text => text_above.min(image_below_mixed),
        PageKind::Scanned => text_below.min(scan_image_above),
        PageKind::Image => text_below.min(image_above.max(path_above)),
        PageKind::Mixed => text_above.min(mixed_image_above),
    };
    round3((0.5 + 0.5 * margin).clamp(0.0, 1.0))
}

fn normalized_above(value: f64, threshold: f64) -> f64 {
    ((value - threshold) / threshold.max(f64::EPSILON)).clamp(0.0, 1.0)
}

fn normalized_below(value: f64, threshold: f64) -> f64 {
    ((threshold - value) / threshold.max(f64::EPSILON)).clamp(0.0, 1.0)
}

fn normalized_to_ceiling(value: f64, threshold: f64) -> f64 {
    ((value - threshold) / (1.0 - threshold).max(f64::EPSILON)).clamp(0.0, 1.0)
}

fn ratio_reason(name: &str, value: f64) -> String {
    format!("{name}={:.3}", round3(value))
}

fn clipped_bbox_area(bbox: &crate::BBox, width: f64, height: f64) -> f64 {
    positive_area(
        bbox.x0.clamp(0.0, width),
        bbox.y0.clamp(0.0, height),
        bbox.x1.clamp(0.0, width),
        bbox.y1.clamp(0.0, height),
    )
}

fn positive_area(x0: f64, y0: f64, x1: f64, y1: f64) -> f64 {
    (x1 - x0).max(0.0) * (y1 - y0).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BBox, Char, Element, Font, ImageElement, Line, Page, TextColor, TextElement, Word,
    };

    const PAGE_W: f64 = 600.0;
    const PAGE_H: f64 = 800.0;

    /// Builds a single text element carrying `glyphs` characters, each with a
    /// `size x size` bbox, so total text coverage is controllable.
    fn text_element(glyphs: usize, glyph_size: f64) -> Element {
        let chars: Vec<Char> = (0..glyphs)
            .map(|i| {
                let x = i as f64 * glyph_size;
                Char {
                    content: "a".into(),
                    bbox: BBox {
                        x0: x,
                        y0: 0.0,
                        x1: x + glyph_size,
                        y1: glyph_size,
                    },
                    unicode: u32::from(b'a'),
                }
            })
            .collect();
        let word_bbox = BBox {
            x0: 0.0,
            y0: 0.0,
            x1: glyphs as f64 * glyph_size,
            y1: glyph_size,
        };
        Element::Text(TextElement {
            id: "t0".into(),
            bbox: word_bbox,
            content: "a".repeat(glyphs),
            font: Font {
                name: "Test".into(),
                size: 10.0,
                bold: false,
                italic: false,
            },
            color: TextColor {
                fill: Some([0, 0, 0]),
                stroke: None,
            },
            lines: Some(vec![Line {
                bbox: word_bbox,
                baseline_y: glyph_size,
                words: vec![Word {
                    content: "a".repeat(glyphs),
                    bbox: word_bbox,
                    chars,
                }],
            }]),
            runs: None,
        })
    }

    fn full_page_image() -> Element {
        Element::Image(ImageElement {
            id: "i0".into(),
            bbox: BBox {
                x0: 0.0,
                y0: 0.0,
                x1: PAGE_W,
                y1: PAGE_H,
            },
            quad: [[0.0, 0.0], [PAGE_W, 0.0], [PAGE_W, PAGE_H], [0.0, PAGE_H]],
            pixel_width: Some(1200),
            pixel_height: Some(1600),
            colorspace: Some("DeviceRGB".into()),
            content_hash: Some("deadbeef".into()),
        })
    }

    /// An image element occupying a `w x h` box anchored at the origin, so
    /// `image_coverage` and `largest_image_ratio` are directly controllable
    /// and stay below the full-page/dominating thresholds.
    fn partial_image(w: f64, h: f64) -> Element {
        Element::Image(ImageElement {
            id: "i1".into(),
            bbox: BBox {
                x0: 0.0,
                y0: 0.0,
                x1: w,
                y1: h,
            },
            quad: [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]],
            pixel_width: Some(600),
            pixel_height: Some(800),
            colorspace: Some("DeviceRGB".into()),
            content_hash: Some("cafef00d".into()),
        })
    }

    fn page(elements: Vec<Element>) -> Page {
        Page {
            page_number: 1,
            width: PAGE_W,
            height: PAGE_H,
            rotation: 0,
            scanned: false,
            elements,
            hidden: vec![],
        }
    }

    #[test]
    fn scanned_page_with_stray_glyph_layer_is_scanned_and_needs_ocr() {
        // Full-page raster covering the page (ratio 1.0) plus a stray 10-glyph
        // text layer with 1pt boxes → text_coverage ≈ 10/480000 ≈ 2e-5, well
        // under MEANINGFUL_TEXT_COVERAGE. Glyph count must not vouch for text.
        let page = page(vec![full_page_image(), text_element(10, 1.0)]);
        let signals = page_signals(&page);
        assert!(signals.largest_image_ratio >= FULL_PAGE_IMAGE_COVERAGE);
        assert!(signals.text_coverage < MEANINGFUL_TEXT_COVERAGE);
        assert_eq!(signals.glyphs, 10);

        let c = classify_page(&page, &[]);
        assert_eq!(c.kind, PageKind::Scanned);
        assert!(c.needs_ocr);
    }

    #[test]
    fn sparse_text_page_without_image_stays_text_and_no_ocr() {
        // 10 glyphs, no image. glyph_size chosen so coverage ≈ 0.005: between
        // TEXT_COVERAGE_FLOOR and MEANINGFUL_TEXT_COVERAGE. A genuinely sparse
        // but complete text page must not be dragged into needs_ocr.
        // area per glyph = 240 (√240 ≈ 15.49) → 10 glyphs → 2400 / 480000 = 0.005.
        let page = page(vec![text_element(10, 240.0_f64.sqrt())]);
        let signals = page_signals(&page);
        assert!(signals.largest_image_ratio < DOMINATING_IMAGE_COVERAGE);
        assert!(signals.text_coverage >= TEXT_COVERAGE_FLOOR);
        assert!(signals.text_coverage < MEANINGFUL_TEXT_COVERAGE);
        assert_eq!(signals.glyphs, 10);

        let c = classify_page(&page, &[]);
        assert_eq!(c.kind, PageKind::Text);
        assert!(!c.needs_ocr);
    }

    #[test]
    fn image_with_sparse_text_layer_needs_ocr() {
        // Significant image (~30% of page, ratio 0.30 < DOMINATING 0.60) plus a
        // stray text layer at coverage ≈ 0.01: 10 glyphs of area 480 each →
        // 4800 / 480000 = 0.01, under OCR_SPARSE_TEXT_COVERAGE (0.05). This is
        // the under-read-scan pattern; the gated trigger must force needs_ocr.
        let page = page(vec![
            partial_image(360.0, 400.0),
            text_element(10, 480.0_f64.sqrt()),
        ]);
        let signals = page_signals(&page);
        assert!((signals.image_coverage - 0.30).abs() < 1e-6);
        assert!(signals.largest_image_ratio < DOMINATING_IMAGE_COVERAGE);
        assert!(signals.text_coverage < OCR_SPARSE_TEXT_COVERAGE);
        assert!((signals.text_coverage - 0.01).abs() < 1e-6);

        let c = classify_page(&page, &[]);
        assert!(c.needs_ocr);
        assert!(c.reasons.iter().any(|r| r == "image_present_sparse_text"));
    }

    #[test]
    fn figure_page_with_good_text_no_forced_ocr() {
        // Same ~30% image, but the text layer carries real content: 6 glyphs of
        // area 9216 each → 55296 / 480000 ≈ 0.115, above OCR_SPARSE_TEXT_COVERAGE
        // (0.05). The sparse-text gate must stay inert so figure-with-caption
        // pages are not force-flagged for OCR.
        let page = page(vec![partial_image(360.0, 400.0), text_element(6, 96.0)]);
        let signals = page_signals(&page);
        assert!((signals.image_coverage - 0.30).abs() < 1e-6);
        assert!(signals.largest_image_ratio < DOMINATING_IMAGE_COVERAGE);
        assert!(signals.text_coverage >= OCR_SPARSE_TEXT_COVERAGE);

        let c = classify_page(&page, &[]);
        assert!(!c.reasons.iter().any(|r| r == "image_present_sparse_text"));
        assert!(!c.needs_ocr);
    }
}
