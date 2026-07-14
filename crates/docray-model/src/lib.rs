use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

pub fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BBox {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl BBox {
    pub fn union(&self, other: &BBox) -> BBox {
        BBox {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Extraction {
    pub schema_version: String,
    pub source: Source,
    pub document: DocumentInfo,
    pub warnings: Vec<String>,
    pub pages: Vec<Page>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub format: String,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentInfo {
    pub page_count: u32,
    pub metadata: DocMetadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocMetadata {
    pub title: Option<String>,
    pub author: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page {
    pub page_number: u32,
    pub width: f64,
    pub height: f64,
    pub rotation: i32,
    pub scanned: bool,
    pub elements: Vec<Element>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Element {
    Text(TextElement),
    Image(ImageElement),
    Path(PathElement),
    Annotation(AnnotationElement),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextElement {
    pub id: String,
    pub bbox: BBox,
    pub content: String,
    pub font: Font,
    pub color: TextColor,
    pub lines: Vec<Line>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Font {
    pub name: String,
    pub size: f64,
    pub bold: bool,
    pub italic: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextColor {
    pub fill: Option<[u8; 3]>,
    pub stroke: Option<[u8; 3]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub bbox: BBox,
    pub baseline_y: f64,
    pub words: Vec<Word>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Word {
    pub content: String,
    pub bbox: BBox,
    pub chars: Vec<Char>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Char {
    pub content: String,
    pub bbox: BBox,
    pub unicode: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageElement {
    pub id: String,
    pub bbox: BBox,
    pub quad: [[f64; 2]; 4],
    pub pixel_width: Option<u32>,
    pub pixel_height: Option<u32>,
    pub colorspace: Option<String>,
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PathElement {
    pub id: String,
    pub bbox: BBox,
    pub fill: Option<[u8; 3]>,
    pub stroke: Option<[u8; 3]>,
    pub stroke_width: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnnotationElement {
    pub id: String,
    pub bbox: BBox,
    pub subtype: String,
    pub uri: Option<String>,
}

/// Requested representation of an extraction response.
///
/// `char` is the full lossless model. The CLI/server only constructs this
/// output wrapper for an *explicit* request; an absent parameter serializes
/// `Extraction` directly to preserve the v1.1 bytes exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Granularity {
    Element,
    Word,
    Char,
}

impl Granularity {
    pub fn as_str(self) -> &'static str {
        match self {
            Granularity::Element => "element",
            Granularity::Word => "word",
            Granularity::Char => "char",
        }
    }
}

impl fmt::Display for Granularity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Granularity {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "element" => Ok(Granularity::Element),
            "word" => Ok(Granularity::Word),
            "char" => Ok(Granularity::Char),
            _ => Err(format!("expected element, word, or char; got {value:?}")),
        }
    }
}

impl Serialize for Granularity {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

/// The explicit-granularity response. It deliberately borrows char data so
/// that char mode can retain every lossless nested field while only changing
/// the envelope version/discriminator.
#[derive(Serialize)]
#[serde(untagged)]
pub enum GranularExtraction<'a> {
    Char(ExplicitCharExtraction<'a>),
    Compact(CompactExtraction),
}

#[derive(Serialize)]
pub struct ExplicitCharExtraction<'a> {
    pub granularity: Granularity,
    pub schema_version: &'static str,
    pub source: &'a Source,
    pub document: &'a DocumentInfo,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: &'a Vec<String>,
    pub pages: &'a Vec<Page>,
}

#[derive(Serialize)]
pub struct CompactExtraction {
    pub granularity: Granularity,
    pub schema_version: &'static str,
    pub source: Source,
    pub document: CompactDocumentInfo,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub pages: Vec<CompactPage>,
}

#[derive(Serialize)]
pub struct CompactDocumentInfo {
    pub page_count: u32,
    pub metadata: CompactDocMetadata,
}

#[derive(Serialize)]
pub struct CompactDocMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
}

#[derive(Serialize)]
pub struct CompactPage {
    pub page_number: u32,
    pub width: f64,
    pub height: f64,
    pub rotation: i32,
    pub scanned: bool,
    pub elements: Vec<CompactElement>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompactElement {
    Text(CompactTextElement),
    Image(CompactBoxElement),
    Path(CompactBoxElement),
    Annotation(CompactAnnotationElement),
}

#[derive(Serialize)]
pub struct CompactTextElement {
    pub bbox: [f64; 4],
    #[serde(flatten)]
    pub content: CompactTextContent,
    pub font: CompactFont,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<CompactTextColor>,
}

#[derive(Serialize)]
#[serde(untagged)]
pub enum CompactTextContent {
    Element { text: String },
    Word { words: Vec<CompactWord> },
}

/// Positional word record: `[text, x0, y0, x1, y1]` in extraction/content-stream order.
#[derive(Serialize)]
pub struct CompactWord(pub String, pub f64, pub f64, pub f64, pub f64);

#[derive(Serialize)]
pub struct CompactBoxElement {
    pub bbox: [f64; 4],
}

#[derive(Serialize)]
pub struct CompactAnnotationElement {
    pub bbox: [f64; 4],
    pub subtype: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

#[derive(Serialize)]
pub struct CompactFont {
    pub name: String,
    pub size: f64,
    #[serde(skip_serializing_if = "is_false")]
    pub bold: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub italic: bool,
}

#[derive(Serialize)]
pub struct CompactTextColor {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<[u8; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke: Option<[u8; 3]>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn round1(value: f64) -> f64 {
    let rounded = (value * 10.0).round() / 10.0;
    if rounded == 0.0 {
        0.0
    } else {
        rounded
    }
}

fn compact_bbox(bbox: &BBox) -> [f64; 4] {
    [
        round1(bbox.x0),
        round1(bbox.y0),
        round1(bbox.x1),
        round1(bbox.y1),
    ]
}

fn compact_font(font: &Font) -> CompactFont {
    CompactFont {
        name: font.name.clone(),
        size: font.size,
        bold: font.bold,
        italic: font.italic,
    }
}

fn compact_color(color: &TextColor) -> Option<CompactTextColor> {
    let fill = color.fill.filter(|value| *value != [0, 0, 0]);
    let stroke = color.stroke.filter(|value| *value != [0, 0, 0]);
    if fill.is_none() && stroke.is_none() {
        None
    } else {
        Some(CompactTextColor { fill, stroke })
    }
}

impl Extraction {
    /// Converts an explicit granularity request. Callers must serialize an
    /// `Extraction` directly when granularity is absent, preserving v1.1 bytes.
    pub fn with_granularity(&self, granularity: Granularity) -> GranularExtraction<'_> {
        match granularity {
            Granularity::Char => GranularExtraction::Char(ExplicitCharExtraction {
                granularity,
                schema_version: "1.2",
                source: &self.source,
                document: &self.document,
                warnings: &self.warnings,
                pages: &self.pages,
            }),
            Granularity::Element | Granularity::Word => {
                GranularExtraction::Compact(CompactExtraction {
                    granularity,
                    schema_version: "1.2",
                    source: self.source.clone(),
                    document: CompactDocumentInfo {
                        page_count: self.document.page_count,
                        metadata: CompactDocMetadata {
                            title: self.document.metadata.title.clone(),
                            author: self.document.metadata.author.clone(),
                        },
                    },
                    warnings: self.warnings.clone(),
                    pages: self
                        .pages
                        .iter()
                        .map(|page| compact_page(page, granularity))
                        .collect(),
                })
            }
        }
    }
}

fn compact_page(page: &Page, granularity: Granularity) -> CompactPage {
    CompactPage {
        page_number: page.page_number,
        width: page.width,
        height: page.height,
        rotation: page.rotation,
        scanned: page.scanned,
        elements: page
            .elements
            .iter()
            .map(|element| compact_element(element, granularity))
            .collect(),
    }
}

fn compact_element(element: &Element, granularity: Granularity) -> CompactElement {
    match element {
        Element::Text(text) => {
            let content = match granularity {
                Granularity::Element => CompactTextContent::Element {
                    text: text.content.clone(),
                },
                Granularity::Word => CompactTextContent::Word {
                    // Preserve the extractor's content-stream order; DPS does
                    // not perform semantic reordering.
                    words: text
                        .lines
                        .iter()
                        .flat_map(|line| line.words.iter())
                        .map(|word| {
                            let [x0, y0, x1, y1] = compact_bbox(&word.bbox);
                            CompactWord(word.content.clone(), x0, y0, x1, y1)
                        })
                        .collect(),
                },
                Granularity::Char => unreachable!("char does not use compact elements"),
            };
            CompactElement::Text(CompactTextElement {
                bbox: compact_bbox(&text.bbox),
                content,
                font: compact_font(&text.font),
                color: compact_color(&text.color),
            })
        }
        Element::Image(image) => CompactElement::Image(CompactBoxElement {
            bbox: compact_bbox(&image.bbox),
        }),
        Element::Path(path) => CompactElement::Path(CompactBoxElement {
            bbox: compact_bbox(&path.bbox),
        }),
        Element::Annotation(annotation) => CompactElement::Annotation(CompactAnnotationElement {
            bbox: compact_bbox(&annotation.bbox),
            subtype: annotation.subtype.clone(),
            uri: annotation.uri.clone(),
        }),
    }
}
