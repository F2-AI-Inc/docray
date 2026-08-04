use serde::{Deserialize, Serialize};
use std::fmt;
use std::fmt::Write as _;
use std::str::FromStr;

mod classification;
mod flow;
pub mod grouping;
mod markdown;
mod regroup;

pub use classification::*;
pub use flow::*;

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
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub hidden: Vec<HiddenItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HiddenItem {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub element: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Element {
    Text(TextElement),
    Table(TableElement),
    Chart(ChartElement),
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
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub lines: Option<Vec<Line>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub runs: Option<Vec<TextRun>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextRun {
    pub content: String,
    pub font: Font,
    pub color: TextColor,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub href: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableElement {
    pub id: String,
    pub bbox: BBox,
    pub rows: usize,
    pub cols: usize,
    pub cells: Vec<TableCell>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableCell {
    pub bbox: BBox,
    pub row: usize,
    pub col: usize,
    pub row_span: usize,
    pub col_span: usize,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub runs: Option<Vec<TextRun>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartElement {
    pub id: String,
    pub bbox: BBox,
    pub chart_type: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub title: Option<String>,
    pub series: Vec<ChartSeries>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartSeries {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name: Option<String>,
    pub points: Vec<ChartPoint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartPoint {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub category: Option<String>,
    pub value: String,
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

    /// Orders granularities from coarsest to finest.
    pub const fn rank(self) -> u8 {
        match self {
            Granularity::Element => 0,
            Granularity::Word => 1,
            Granularity::Char => 2,
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

/// Wire representation requested by CLI and HTTP consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Json,
    Lean,
    Markdown,
}

impl OutputFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            OutputFormat::Json => "json",
            OutputFormat::Lean => "lean",
            OutputFormat::Markdown => "md",
        }
    }
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for OutputFormat {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "json" => Ok(OutputFormat::Json),
            "lean" => Ok(OutputFormat::Lean),
            "md" => Ok(OutputFormat::Markdown),
            _ => Err(format!("expected json, lean, or md; got {value:?}")),
        }
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
    Flow(CompactFlowExtraction),
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hidden: Vec<HiddenItem>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompactElement {
    Text(CompactTextElement),
    Table(CompactTableElement),
    Chart(CompactChartElement),
    Image(CompactBoxElement),
    Path(CompactPathElement),
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runs: Option<Vec<CompactTextRun>>,
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
pub struct CompactTableElement {
    pub bbox: [f64; 4],
    pub rows: usize,
    pub cols: usize,
    pub cells: Vec<CompactTableCell>,
}

#[derive(Serialize)]
pub struct CompactTableCell {
    pub bbox: [f64; 4],
    pub row: usize,
    pub col: usize,
    pub row_span: usize,
    pub col_span: usize,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runs: Option<Vec<CompactTextRun>>,
}

#[derive(Serialize)]
pub struct CompactChartElement {
    pub bbox: [f64; 4],
    pub chart_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub series: Vec<CompactChartSeries>,
}

#[derive(Serialize)]
pub struct CompactChartSeries {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub points: Vec<CompactChartPoint>,
}

#[derive(Serialize)]
pub struct CompactChartPoint {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub value: String,
}

#[derive(Serialize)]
pub struct CompactTextRun {
    pub content: String,
    pub font: CompactFont,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<CompactTextColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
}

#[derive(Serialize)]
pub struct CompactBoxElement {
    pub bbox: [f64; 4],
}

#[derive(Serialize)]
pub struct CompactPathElement {
    pub bbox: [f64; 4],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<[u8; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke: Option<[u8; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_width: Option<f64>,
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

const ELEMENT_LEGEND: &str = "#legend T x0 y0 x1 y1 font size style text | r font size style [href#<uri>] text | TB x0 y0 x1 y1 rows cols | c row col rowspan colspan x0 y0 x1 y1 font size style text | CH x0 y0 x1 y1 type [title] | s series-name | p [category] value | I/P x0 y0 x1 y1 | A x0 y0 x1 y1 subtype uri | pt, top-left origin";
const WORD_LEGEND: &str = "#legend T x0 y0 x1 y1 font size style | w x0 y0 x1 y1 word | r font size style [href#<uri>] text | TB x0 y0 x1 y1 rows cols | c row col rowspan colspan x0 y0 x1 y1 font size style text | CH x0 y0 x1 y1 type [title] | s series-name | p [category] value | I/P x0 y0 x1 y1 | A x0 y0 x1 y1 subtype uri | pt, top-left origin";
const LEGACY_ELEMENT_LEGEND: &str = "#legend T x0 y0 x1 y1 font size style text | I/P x0 y0 x1 y1 | A x0 y0 x1 y1 subtype uri | pt, top-left origin";
const LEGACY_WORD_LEGEND: &str = "#legend T x0 y0 x1 y1 font size style | w x0 y0 x1 y1 word | I/P x0 y0 x1 y1 | A x0 y0 x1 y1 subtype uri | pt, top-left origin";
const HIDDEN_LEGEND: &str =
    "#legend <hidden> kind [element-id] content | non-visible document context";

impl CompactExtraction {
    /// Renders the deterministic, line-oriented reading format. Compact
    /// extractions can only have element or word granularity.
    pub fn to_lean(&self) -> String {
        let mut output = String::new();
        self.write_lean(&mut output)
            .expect("writing to a String cannot fail");
        output
    }

    /// Streams the lean rendering into any [`fmt::Write`], so callers can
    /// bound output growth DURING generation (e.g. a size-capped writer that
    /// errors once a byte budget is exceeded) instead of materializing the
    /// whole document first.
    pub fn write_lean<W: fmt::Write>(&self, output: &mut W) -> fmt::Result {
        debug_assert!(matches!(
            self.granularity,
            Granularity::Element | Granularity::Word
        ));
        write!(
            output,
            "#docray {} v{} pages={}",
            self.granularity,
            self.schema_version,
            self.pages.len()
        )?;
        if !self.warnings.is_empty() {
            write!(output, " warnings={}", self.warnings.len())?;
        }
        output.write_char('\n')?;
        let has_structured_detail = self.pages.iter().any(|page| {
            page.elements.iter().any(|element| match element {
                CompactElement::Text(text) => text.runs.is_some(),
                CompactElement::Table(_) | CompactElement::Chart(_) => true,
                CompactElement::Image(_)
                | CompactElement::Path(_)
                | CompactElement::Annotation(_) => false,
            })
        });
        output.write_str(match (self.granularity, has_structured_detail) {
            (Granularity::Element, true) => ELEMENT_LEGEND,
            (Granularity::Word, true) => WORD_LEGEND,
            (Granularity::Element, false) => LEGACY_ELEMENT_LEGEND,
            (Granularity::Word, false) => LEGACY_WORD_LEGEND,
            (Granularity::Char, _) => unreachable!("char does not use compact output"),
        })?;
        output.write_char('\n')?;
        if self.pages.iter().any(|page| !page.hidden.is_empty()) {
            output.write_str(HIDDEN_LEGEND)?;
            output.write_char('\n')?;
        }

        for warning in &self.warnings {
            writeln!(output, "#warning {}", collapse_warning(warning))?;
        }

        for page in &self.pages {
            write!(
                output,
                "#page {} {}x{}",
                page.page_number,
                lean_number(page.width),
                lean_number(page.height)
            )?;
            if page.rotation != 0 {
                write!(output, " rot={}", page.rotation)?;
            }
            if page.scanned {
                output.write_str(" scanned")?;
            }
            output.write_char('\n')?;

            for element in &page.elements {
                match element {
                    CompactElement::Text(text) => {
                        let bbox = lean_bbox(&text.bbox);
                        let font = lean_font_name(&text.font.name);
                        let size = lean_number(text.font.size);
                        let style = lean_style(text);
                        let runs = text.runs.as_deref();
                        match &text.content {
                            CompactTextContent::Element { text } => {
                                writeln!(
                                    output,
                                    "T {bbox} {font} {size} {style} {}",
                                    escape_text(text)
                                )?;
                                write_lean_runs(output, runs)?;
                            }
                            CompactTextContent::Word { words } => {
                                writeln!(output, "T {bbox} {font} {size} {style}")?;
                                write_lean_runs(output, runs)?;
                                for word in words {
                                    writeln!(
                                        output,
                                        "w {} {} {} {} {}",
                                        lean_number(word.1),
                                        lean_number(word.2),
                                        lean_number(word.3),
                                        lean_number(word.4),
                                        escape_text(&word.0)
                                    )?;
                                }
                            }
                        }
                    }
                    CompactElement::Table(table) => {
                        writeln!(
                            output,
                            "TB {} {} {}",
                            lean_bbox(&table.bbox),
                            table.rows,
                            table.cols
                        )?;
                        for cell in &table.cells {
                            let (font, size, style) = cell
                                .runs
                                .as_deref()
                                .and_then(|runs| runs.first())
                                .map(|run| {
                                    (
                                        lean_font_name(&run.font.name),
                                        lean_number(run.font.size),
                                        lean_run_style(run),
                                    )
                                })
                                .unwrap_or_else(|| ("-".into(), "-".into(), "-".into()));
                            writeln!(
                                output,
                                "c {} {} {} {} {} {} {} {} {}",
                                cell.row,
                                cell.col,
                                cell.row_span,
                                cell.col_span,
                                lean_bbox(&cell.bbox),
                                font,
                                size,
                                style,
                                escape_text(&cell.content)
                            )?;
                            write_lean_runs(output, cell.runs.as_deref())?;
                        }
                    }
                    CompactElement::Chart(chart) => {
                        write!(output, "CH {} {}", lean_bbox(&chart.bbox), chart.chart_type)?;
                        if let Some(title) = &chart.title {
                            write!(output, " {}", escape_text(title))?;
                        }
                        output.write_char('\n')?;
                        for series in &chart.series {
                            if let Some(name) = &series.name {
                                writeln!(output, "s {}", escape_text(name))?;
                            }
                            for point in &series.points {
                                output.write_str("p ")?;
                                if let Some(category) = &point.category {
                                    write!(output, "{} ", escape_text(category))?;
                                }
                                writeln!(output, "{}", escape_text(&point.value))?;
                            }
                        }
                    }
                    CompactElement::Image(image) => {
                        writeln!(output, "I {}", lean_bbox(&image.bbox))?;
                    }
                    CompactElement::Path(path) => {
                        writeln!(output, "P {}", lean_bbox(&path.bbox))?;
                    }
                    CompactElement::Annotation(annotation) => {
                        // URIs are document-controlled: escape every physical
                        // line boundary so crafted content cannot inject fake
                        // element lines into the output an LLM reads.
                        writeln!(
                            output,
                            "A {} {} {}",
                            lean_bbox(&annotation.bbox),
                            annotation.subtype,
                            annotation
                                .uri
                                .as_deref()
                                .map(escape_text)
                                .unwrap_or_else(|| "-".to_string())
                        )?;
                    }
                }
            }

            if !page.hidden.is_empty() {
                output.write_str("<hidden>\n")?;
                for item in &page.hidden {
                    write!(output, "{} ", item.kind)?;
                    if let Some(element) = &item.element {
                        write!(output, "{element} ")?;
                    }
                    // Hidden content is document-controlled. Escaping every
                    // physical line boundary keeps each item on one line, so
                    // content cannot forge `</hidden>` or element records.
                    writeln!(output, "{}", escape_text(&item.content))?;
                }
                output.write_str("</hidden>\n")?;
            }
        }

        Ok(())
    }
}

fn lean_number(value: f64) -> String {
    let value = round1(value);
    let rendered = format!("{value:.1}");
    rendered.strip_suffix(".0").unwrap_or(&rendered).to_string()
}

fn lean_bbox(bbox: &[f64; 4]) -> String {
    bbox.iter()
        .map(|value| lean_number(*value))
        .collect::<Vec<_>>()
        .join(" ")
}

fn lean_font_name(name: &str) -> String {
    if name.is_empty() {
        return "-".to_string();
    }
    name.chars()
        .map(|ch| if ch.is_whitespace() { '_' } else { ch })
        .collect()
}

fn lean_style(text: &CompactTextElement) -> String {
    lean_style_parts(
        text.font.bold,
        text.font.italic,
        text.color.as_ref().and_then(|color| color.fill),
    )
}

fn lean_run_style(run: &CompactTextRun) -> String {
    lean_style_parts(
        run.font.bold,
        run.font.italic,
        run.color.as_ref().and_then(|color| color.fill),
    )
}

fn lean_style_parts(bold: bool, italic: bool, fill: Option<[u8; 3]>) -> String {
    let mut style = String::new();
    if bold {
        style.push('b');
    }
    if italic {
        style.push('i');
    }
    if style.is_empty() {
        style.push('-');
    }
    if let Some(fill) = fill.filter(|value| *value != [0, 0, 0]) {
        write!(style, "#{:02x}{:02x}{:02x}", fill[0], fill[1], fill[2])
            .expect("writing to a String cannot fail");
    }
    style
}

fn write_lean_runs<W: fmt::Write>(output: &mut W, runs: Option<&[CompactTextRun]>) -> fmt::Result {
    let Some(runs) =
        runs.filter(|runs| runs.len() > 1 || runs.iter().any(|run| run.href.is_some()))
    else {
        return Ok(());
    };
    for run in runs {
        write!(
            output,
            "r {} {} {} ",
            lean_font_name(&run.font.name),
            lean_number(run.font.size),
            lean_run_style(run)
        )?;
        if let Some(href) = &run.href {
            write!(output, "href#<{}> ", escape_text(href))?;
        }
        writeln!(output, "{}", escape_text(&run.content))?;
    }
    Ok(())
}

fn escape_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            _ if ch.is_control() || matches!(ch, '\u{2028}' | '\u{2029}') => {
                write!(escaped, "\\u{{{:x}}}", ch as u32).expect("writing to a String cannot fail");
            }
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn collapse_warning(warning: &str) -> String {
    let mut collapsed = String::with_capacity(warning.len());
    let mut in_break = false;
    for ch in warning.chars() {
        if matches!(ch, '\r' | '\n' | '\t') {
            if !in_break {
                collapsed.push(' ');
            }
            in_break = true;
        } else {
            collapsed.push(ch);
            in_break = false;
        }
    }
    collapsed
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

// `pub(crate)`: also called from `regroup::compact_fragmented_elements` when
// projecting regrouped lines / passthrough non-text elements to the compact
// shape. Widening from module-private is a visibility-only change — no
// behavior differs on the existing call sites in this file.
pub(crate) fn compact_bbox(bbox: &BBox) -> [f64; 4] {
    [
        round1(bbox.x0),
        round1(bbox.y0),
        round1(bbox.x1),
        round1(bbox.y1),
    ]
}

pub(crate) fn compact_font(font: &Font) -> CompactFont {
    CompactFont {
        name: font.name.clone(),
        size: font.size,
        bold: font.bold,
        italic: font.italic,
    }
}

pub(crate) fn compact_color(color: &TextColor) -> Option<CompactTextColor> {
    let fill = color.fill.filter(|value| *value != [0, 0, 0]);
    let stroke = color.stroke.filter(|value| *value != [0, 0, 0]);
    if fill.is_none() && stroke.is_none() {
        None
    } else {
        Some(CompactTextColor { fill, stroke })
    }
}

pub(crate) fn compact_runs(runs: &Option<Vec<TextRun>>) -> Option<Vec<CompactTextRun>> {
    runs.as_ref().map(|runs| {
        runs.iter()
            .map(|run| CompactTextRun {
                content: run.content.clone(),
                font: compact_font(&run.font),
                color: compact_color(&run.color),
                href: run.href.clone(),
            })
            .collect()
    })
}

impl Extraction {
    /// Converts an explicit granularity request. Callers must serialize an
    /// `Extraction` directly when granularity is absent, preserving v1.1 bytes.
    pub fn with_granularity(&self, granularity: Granularity) -> GranularExtraction<'_> {
        match granularity {
            Granularity::Char => GranularExtraction::Char(ExplicitCharExtraction {
                granularity,
                schema_version: "1.6",
                source: &self.source,
                document: &self.document,
                warnings: &self.warnings,
                pages: &self.pages,
            }),
            Granularity::Element | Granularity::Word => {
                GranularExtraction::Compact(CompactExtraction {
                    granularity,
                    schema_version: "1.9",
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
    let elements = if regroup::is_glyph_fragmented(&page.elements) {
        regroup::compact_fragmented_elements(&page.elements, granularity)
    } else {
        page.elements
            .iter()
            .map(|element| compact_element(element, granularity))
            .collect()
    };
    CompactPage {
        page_number: page.page_number,
        width: page.width,
        height: page.height,
        rotation: page.rotation,
        scanned: page.scanned,
        elements,
        hidden: page.hidden.clone(),
    }
}

pub(crate) fn compact_element(element: &Element, granularity: Granularity) -> CompactElement {
    match element {
        Element::Text(text) => {
            let content = match granularity {
                Granularity::Element => CompactTextContent::Element {
                    text: text.content.clone(),
                },
                Granularity::Word => CompactTextContent::Word {
                    // Preserve the extractor's content-stream order; DPS does
                    // not perform semantic reordering. A missing hierarchy is
                    // unreachable through capability-aware entry points for a
                    // word request, but keep this projection defined defensively.
                    words: text
                        .lines
                        .iter()
                        .flatten()
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
                runs: compact_runs(&text.runs),
            })
        }
        Element::Table(table) => CompactElement::Table(CompactTableElement {
            bbox: compact_bbox(&table.bbox),
            rows: table.rows,
            cols: table.cols,
            cells: table
                .cells
                .iter()
                .map(|cell| CompactTableCell {
                    bbox: compact_bbox(&cell.bbox),
                    row: cell.row,
                    col: cell.col,
                    row_span: cell.row_span,
                    col_span: cell.col_span,
                    content: cell.content.clone(),
                    runs: compact_runs(&cell.runs),
                })
                .collect(),
        }),
        Element::Chart(chart) => CompactElement::Chart(CompactChartElement {
            bbox: compact_bbox(&chart.bbox),
            chart_type: chart.chart_type.clone(),
            title: chart.title.clone(),
            series: chart
                .series
                .iter()
                .map(|series| CompactChartSeries {
                    name: series.name.clone(),
                    points: series
                        .points
                        .iter()
                        .map(|point| CompactChartPoint {
                            category: point.category.clone(),
                            value: point.value.clone(),
                        })
                        .collect(),
                })
                .collect(),
        }),
        Element::Image(image) => CompactElement::Image(CompactBoxElement {
            bbox: compact_bbox(&image.bbox),
        }),
        Element::Path(path) => CompactElement::Path(CompactPathElement {
            bbox: compact_bbox(&path.bbox),
            fill: path.fill,
            stroke: path.stroke,
            stroke_width: path.stroke_width.map(round1),
        }),
        Element::Annotation(annotation) => CompactElement::Annotation(CompactAnnotationElement {
            bbox: compact_bbox(&annotation.bbox),
            subtype: annotation.subtype.clone(),
            uri: annotation.uri.clone(),
        }),
    }
}

#[cfg(test)]
mod compact_page_regroup_tests {
    use super::*;
    use crate::regroup::is_glyph_fragmented;

    fn bbox() -> BBox {
        BBox {
            x0: 0.0,
            y0: 0.0,
            x1: 10.0,
            y1: 10.0,
        }
    }

    fn font() -> Font {
        Font {
            name: "Test".to_string(),
            size: 12.0,
            bold: false,
            italic: false,
        }
    }

    fn color() -> TextColor {
        TextColor {
            fill: Some([0, 0, 0]),
            stroke: None,
        }
    }

    fn base_extraction(elements: Vec<Element>) -> Extraction {
        Extraction {
            schema_version: "1.1".into(),
            source: Source {
                format: "pdf".into(),
                sha256: "abc".into(),
                size_bytes: 10,
            },
            document: DocumentInfo {
                page_count: 1,
                metadata: DocMetadata {
                    title: None,
                    author: None,
                },
            },
            warnings: vec![],
            pages: vec![Page {
                page_number: 1,
                width: 612.0,
                height: 792.0,
                rotation: 0,
                scanned: false,
                elements,
                hidden: vec![],
            }],
        }
    }

    /// Builds one single-glyph `Element::Text` per `(char, x0)` entry, all on
    /// one baseline, spelling out "hello world" glyph-by-glyph — the same
    /// shape a glyph-fragmented PDF (missing ToUnicode word boundaries)
    /// produces: one `Element::Text` per glyph instead of per word.
    fn fragmented_hello_world_elements() -> Vec<Element> {
        "hello world"
            .chars()
            .filter(|c| *c != ' ')
            .zip(
                // x0 for each letter: 8pt glyph width, no gap within a word,
                // 12pt gap between "hello" and "world" (exceeds the
                // `0.25 * font_size` = 3pt word-split threshold at 12pt font).
                [0.0, 8.0, 16.0, 24.0, 32.0, 52.0, 60.0, 68.0, 76.0, 84.0],
            )
            .enumerate()
            .map(|(i, (ch, x0))| {
                let bbox = BBox {
                    x0,
                    y0: 88.0,
                    x1: x0 + 8.0,
                    y1: 100.0,
                };
                let char = Char {
                    content: ch.to_string(),
                    bbox,
                    unicode: ch as u32,
                };
                let word = Word {
                    content: ch.to_string(),
                    bbox,
                    chars: vec![char],
                };
                let line = Line {
                    bbox,
                    baseline_y: 100.0,
                    words: vec![word],
                };
                Element::Text(TextElement {
                    id: format!("g{i}"),
                    bbox,
                    content: ch.to_string(),
                    font: font(),
                    color: color(),
                    lines: Some(vec![line]),
                    runs: None,
                })
            })
            .collect()
    }

    /// Builds `count` ordinary multi-word `Element::Text` items ("hello
    /// world" repeated), each with a full line/word/char hierarchy — the
    /// shape of a normal (non-fragmented) page.
    fn normal_multi_word_elements(count: usize) -> Vec<Element> {
        (0..count)
            .map(|i| {
                let content = "hello world";
                let words: Vec<Word> = content
                    .split(' ')
                    .map(|word_str| {
                        let chars: Vec<Char> = word_str
                            .chars()
                            .map(|c| Char {
                                content: c.to_string(),
                                bbox: bbox(),
                                unicode: c as u32,
                            })
                            .collect();
                        Word {
                            content: word_str.to_string(),
                            bbox: bbox(),
                            chars,
                        }
                    })
                    .collect();
                let line = Line {
                    bbox: bbox(),
                    baseline_y: 5.0,
                    words,
                };
                Element::Text(TextElement {
                    id: format!("t{i}"),
                    bbox: bbox(),
                    content: content.to_string(),
                    font: font(),
                    color: color(),
                    lines: Some(vec![line]),
                    runs: None,
                })
            })
            .collect()
    }

    #[test]
    fn fragmented_page_gets_schema_1_9_and_is_regrouped_into_real_words() {
        let elements = fragmented_hello_world_elements();
        let glyph_count = elements.len();
        assert_eq!(glyph_count, 10, "sanity: 10 single-glyph elements");

        let extraction = base_extraction(elements);
        let compact = extraction.with_granularity(Granularity::Element);
        let value = serde_json::to_value(&compact).unwrap();

        assert_eq!(
            value["schema_version"], "1.9",
            "element granularity must report the bumped schema version"
        );

        let page_elements = value["pages"][0]["elements"].as_array().unwrap();
        assert!(
            page_elements.len() < glyph_count,
            "regrouping must collapse per-glyph elements into fewer, word-shaped elements; got {} elements from {} glyphs",
            page_elements.len(),
            glyph_count
        );
        assert_eq!(
            page_elements.len(),
            1,
            "one shared baseline collapses to a single regrouped line"
        );
        assert_eq!(
            page_elements[0]["text"], "hello world",
            "regrouped text must contain real word boundaries (a space), not raw concatenated glyphs"
        );
    }

    #[test]
    fn normal_page_keeps_prechange_projection_with_only_version_bumped() {
        let elements = normal_multi_word_elements(3);
        // Below FRAGMENT_MIN_ELEMENTS and not single-glyph either way: this
        // page must NOT be classified as fragmented.
        assert!(!is_glyph_fragmented(&elements));

        let expected_elements: Vec<serde_json::Value> = elements
            .iter()
            .map(|el| serde_json::to_value(compact_element(el, Granularity::Element)).unwrap())
            .collect();

        let extraction = base_extraction(elements);
        let compact = extraction.with_granularity(Granularity::Element);
        let value = serde_json::to_value(&compact).unwrap();

        assert_eq!(
            value["schema_version"], "1.9",
            "non-fragmented pages still report the bumped element/word schema version"
        );
        assert_eq!(
            value["pages"][0]["elements"],
            serde_json::Value::Array(expected_elements),
            "content for a non-fragmented page must be byte-identical to the pre-change \
             per-element projection — only schema_version differs"
        );
    }
}

#[cfg(test)]
mod lean_header_page_count_tests {
    use super::*;

    fn compact_page(page_number: u32) -> CompactPage {
        CompactPage {
            page_number,
            width: 612.0,
            height: 792.0,
            rotation: 0,
            scanned: false,
            elements: vec![],
            hidden: vec![],
        }
    }

    /// A `CompactExtraction` whose `pages` vec (3 entries) is shorter than
    /// `document.page_count` (10) — the shape a page selection produces:
    /// the document has 10 pages total, but only 3 `#page` blocks were
    /// emitted for the requested selection.
    fn selected_pages_extraction() -> CompactExtraction {
        CompactExtraction {
            granularity: Granularity::Element,
            schema_version: "1.9",
            source: Source {
                format: "pdf".into(),
                sha256: "abc".into(),
                size_bytes: 10,
            },
            document: CompactDocumentInfo {
                page_count: 10,
                metadata: CompactDocMetadata {
                    title: None,
                    author: None,
                },
            },
            warnings: vec![],
            pages: vec![compact_page(3), compact_page(5), compact_page(7)],
        }
    }

    #[test]
    fn lean_header_pages_reports_emitted_block_count_not_document_total() {
        let compact = selected_pages_extraction();
        assert_eq!(compact.pages.len(), 3, "sanity: 3 page blocks emitted");
        assert_eq!(
            compact.document.page_count, 10,
            "sanity: document has 10 pages total"
        );

        let lean = compact.to_lean();
        let header = lean.lines().next().expect("lean output has a header line");

        assert!(
            header.contains("pages=3"),
            "LEAN header must report the number of #page blocks actually emitted \
             (3), not the document's total page count (10); got: {header:?}"
        );
        assert!(
            !header.contains("pages=10"),
            "LEAN header must not report the document total page count under a \
             page selection; got: {header:?}"
        );
    }
}
