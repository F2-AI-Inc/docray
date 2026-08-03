use crate::{round3, CompactElement, Element, Extraction, Granularity, HiddenItem};
use serde::{Deserialize, Serialize};

const TEXT_COVERAGE_FLOOR: f64 = 0.001;
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
    let meaningful_text =
        signals.text_coverage >= TEXT_COVERAGE_FLOOR || signals.glyphs >= MEANINGFUL_TEXT_GLYPHS;
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

    let needs_ocr = garbled
        || matches!(kind, PageKind::Scanned | PageKind::Image)
        || signals.largest_image_ratio >= DOMINATING_IMAGE_COVERAGE
        || signals.text_coverage < TEXT_COVERAGE_FLOOR;

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
    let text_above = normalized_above(signals.text_coverage, TEXT_COVERAGE_FLOOR);
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
