use super::{
    compact_color, compact_font, escape_text, lean_number, write_lean_runs, CompactDocMetadata,
    CompactTextRun, DocMetadata, GranularExtraction, Granularity, HiddenItem, Source, TextRun,
};
use serde::{Deserialize, Serialize};
use std::fmt;

/// A document whose authored structure is flow-based rather than paged.
///
/// Block IDs use the stable `s{section}-b{index}` convention. Hidden items
/// may target those IDs through [`HiddenItem::element`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlowExtraction {
    pub schema_version: String,
    pub layout: FlowLayout,
    pub source: Source,
    pub document: FlowDocumentInfo,
    pub warnings: Vec<String>,
    pub approx_pages: Option<u32>,
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowLayout {
    Flow,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlowDocumentInfo {
    pub metadata: DocMetadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub page_width: f64,
    pub page_height: f64,
    pub margins: Margins,
    pub columns: Option<u32>,
    pub headers: Vec<Story>,
    pub footers: Vec<Story>,
    pub blocks: Vec<Block>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub hidden: Vec<HiddenItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Margins {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "variant", rename_all = "snake_case")]
pub enum Story {
    Default { blocks: Vec<Block> },
    First { blocks: Vec<Block> },
    Even { blocks: Vec<Block> },
}

impl Story {
    fn blocks(&self) -> &[Block] {
        match self {
            Story::Default { blocks } | Story::First { blocks } | Story::Even { blocks } => blocks,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Paragraph {
        id: String,
        role: String,
        runs: Vec<TextRun>,
        content: String,
        list: Option<ListInfo>,
        placement: Option<Placement>,
        approx_page: Option<u32>,
        breaks_before: Vec<BreakKind>,
    },
    Table {
        id: String,
        col_widths: Vec<f64>,
        rows: usize,
        cols: usize,
        cells: Vec<FlowTableCell>,
        placement: Option<Placement>,
    },
    Image {
        id: String,
        width: Option<f64>,
        height: Option<f64>,
        content_hash: Option<String>,
        placement: Option<Placement>,
    },
    Textbox {
        id: String,
        placement: Option<Placement>,
        blocks: Vec<Block>,
    },
    Break {
        kind: BreakKind,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListInfo {
    pub list_id: String,
    pub level: u32,
    pub kind: ListKind,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListKind {
    Ordered,
    Bullet,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlowTableCell {
    pub row: usize,
    pub col: usize,
    pub row_span: usize,
    pub col_span: usize,
    pub content: String,
    pub runs: Vec<TextRun>,
    pub blocks: Option<Vec<Block>>,
}

/// Authored placement constraints. These values are never resolved into a
/// bounding box because flow layout requires an external layout engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub frame: PlacementFrame,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub align_h: Option<String>,
    pub align_v: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlacementFrame {
    Page,
    Margin,
    Column,
    Paragraph,
    Line,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BreakKind {
    Page,
    Column,
    Section,
}

#[derive(Serialize)]
pub struct CompactFlowExtraction {
    pub granularity: Granularity,
    pub schema_version: &'static str,
    pub layout: FlowLayout,
    pub source: Source,
    pub document: CompactFlowDocumentInfo,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approx_pages: Option<u32>,
    pub sections: Vec<CompactSection>,
}

#[derive(Serialize)]
pub struct CompactFlowDocumentInfo {
    pub metadata: CompactDocMetadata,
}

#[derive(Serialize)]
pub struct CompactSection {
    pub page_width: f64,
    pub page_height: f64,
    pub margins: Margins,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<u32>,
    pub headers: Vec<CompactStory>,
    pub footers: Vec<CompactStory>,
    pub blocks: Vec<CompactBlock>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hidden: Vec<HiddenItem>,
}

#[derive(Serialize)]
#[serde(tag = "variant", rename_all = "snake_case")]
pub enum CompactStory {
    Default { blocks: Vec<CompactBlock> },
    First { blocks: Vec<CompactBlock> },
    Even { blocks: Vec<CompactBlock> },
}

impl CompactStory {
    fn blocks(&self) -> &[CompactBlock] {
        match self {
            CompactStory::Default { blocks }
            | CompactStory::First { blocks }
            | CompactStory::Even { blocks } => blocks,
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompactBlock {
    Paragraph {
        id: String,
        role: String,
        runs: Vec<CompactTextRun>,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        list: Option<ListInfo>,
        #[serde(skip_serializing_if = "Option::is_none")]
        placement: Option<Placement>,
        #[serde(skip_serializing_if = "Option::is_none")]
        approx_page: Option<u32>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        breaks_before: Vec<BreakKind>,
    },
    Table {
        id: String,
        col_widths: Vec<f64>,
        rows: usize,
        cols: usize,
        cells: Vec<CompactFlowTableCell>,
        #[serde(skip_serializing_if = "Option::is_none")]
        placement: Option<Placement>,
    },
    Image {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        width: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        height: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        content_hash: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        placement: Option<Placement>,
    },
    Textbox {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        placement: Option<Placement>,
        blocks: Vec<CompactBlock>,
    },
    Break {
        kind: BreakKind,
    },
}

#[derive(Serialize)]
pub struct CompactFlowTableCell {
    pub row: usize,
    pub col: usize,
    pub row_span: usize,
    pub col_span: usize,
    pub content: String,
    pub runs: Vec<CompactTextRun>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocks: Option<Vec<CompactBlock>>,
}

impl FlowExtraction {
    /// Produces the only supported compact projection for a flow document.
    /// Finer requests return `None`; callers normally reject them through the
    /// extractor capability gate before reaching this method.
    pub fn with_granularity(&self, granularity: Granularity) -> Option<GranularExtraction<'_>> {
        if granularity != Granularity::Element {
            return None;
        }

        Some(GranularExtraction::Flow(CompactFlowExtraction {
            granularity,
            schema_version: "1.7",
            layout: FlowLayout::Flow,
            source: self.source.clone(),
            document: CompactFlowDocumentInfo {
                metadata: CompactDocMetadata {
                    title: self.document.metadata.title.clone(),
                    author: self.document.metadata.author.clone(),
                },
            },
            warnings: self.warnings.clone(),
            approx_pages: self.approx_pages,
            sections: self.sections.iter().map(compact_section).collect(),
        }))
    }
}

fn compact_section(section: &Section) -> CompactSection {
    CompactSection {
        page_width: super::round1(section.page_width),
        page_height: super::round1(section.page_height),
        margins: compact_margins(section.margins),
        columns: section.columns,
        headers: section.headers.iter().map(compact_story).collect(),
        footers: section.footers.iter().map(compact_story).collect(),
        blocks: section.blocks.iter().map(compact_block).collect(),
        hidden: section.hidden.clone(),
    }
}

fn compact_margins(margins: Margins) -> Margins {
    Margins {
        top: super::round1(margins.top),
        right: super::round1(margins.right),
        bottom: super::round1(margins.bottom),
        left: super::round1(margins.left),
    }
}

fn compact_story(story: &Story) -> CompactStory {
    let blocks = || story.blocks().iter().map(compact_block).collect();
    match story {
        Story::Default { .. } => CompactStory::Default { blocks: blocks() },
        Story::First { .. } => CompactStory::First { blocks: blocks() },
        Story::Even { .. } => CompactStory::Even { blocks: blocks() },
    }
}

fn compact_block(block: &Block) -> CompactBlock {
    match block {
        Block::Paragraph {
            id,
            role,
            runs,
            content,
            list,
            placement,
            approx_page,
            breaks_before,
        } => CompactBlock::Paragraph {
            id: id.clone(),
            role: role.clone(),
            runs: runs.iter().map(compact_run).collect(),
            content: content.clone(),
            list: list.clone(),
            placement: placement.as_ref().map(compact_placement),
            approx_page: *approx_page,
            breaks_before: breaks_before.clone(),
        },
        Block::Table {
            id,
            col_widths,
            rows,
            cols,
            cells,
            placement,
        } => CompactBlock::Table {
            id: id.clone(),
            col_widths: col_widths
                .iter()
                .map(|width| super::round1(*width))
                .collect(),
            rows: *rows,
            cols: *cols,
            cells: cells.iter().map(compact_cell).collect(),
            placement: placement.as_ref().map(compact_placement),
        },
        Block::Image {
            id,
            width,
            height,
            content_hash,
            placement,
        } => CompactBlock::Image {
            id: id.clone(),
            width: width.map(super::round1),
            height: height.map(super::round1),
            content_hash: content_hash.clone(),
            placement: placement.as_ref().map(compact_placement),
        },
        Block::Textbox {
            id,
            placement,
            blocks,
        } => CompactBlock::Textbox {
            id: id.clone(),
            placement: placement.as_ref().map(compact_placement),
            blocks: blocks.iter().map(compact_block).collect(),
        },
        Block::Break { kind } => CompactBlock::Break { kind: *kind },
    }
}

fn compact_cell(cell: &FlowTableCell) -> CompactFlowTableCell {
    CompactFlowTableCell {
        row: cell.row,
        col: cell.col,
        row_span: cell.row_span,
        col_span: cell.col_span,
        content: cell.content.clone(),
        runs: cell.runs.iter().map(compact_run).collect(),
        blocks: cell
            .blocks
            .as_ref()
            .map(|blocks| blocks.iter().map(compact_block).collect()),
    }
}

fn compact_run(run: &TextRun) -> CompactTextRun {
    CompactTextRun {
        content: run.content.clone(),
        font: compact_font(&run.font),
        color: compact_color(&run.color),
        href: run.href.clone(),
    }
}

fn compact_placement(placement: &Placement) -> Placement {
    Placement {
        frame: placement.frame,
        x: placement.x.map(super::round1),
        y: placement.y.map(super::round1),
        width: placement.width.map(super::round1),
        height: placement.height.map(super::round1),
        align_h: placement.align_h.clone(),
        align_v: placement.align_v.clone(),
    }
}

const FLOW_LEGEND: &str = "#legend #section width height | H1..H9/TI/Q/P text | LI level o|b label text | r font size style [href#<uri>] text | TB cols col-width... | c row col rowspan colspan text | I [width height] | BR page|column|section | ~page N | pt, authored flow; no resolved coordinates";
const HIDDEN_LEGEND: &str =
    "#legend <hidden> kind [element-id] content | non-visible document context";

impl CompactFlowExtraction {
    pub fn to_lean(&self) -> String {
        let mut output = String::new();
        self.write_lean(&mut output)
            .expect("writing to a String cannot fail");
        output
    }

    pub fn write_lean<W: fmt::Write>(&self, output: &mut W) -> fmt::Result {
        write!(
            output,
            "#docray element v{} sections={}",
            self.schema_version,
            self.sections.len()
        )?;
        if !self.warnings.is_empty() {
            write!(output, " warnings={}", self.warnings.len())?;
        }
        output.write_char('\n')?;
        output.write_str(FLOW_LEGEND)?;
        output.write_char('\n')?;
        if self
            .sections
            .iter()
            .any(|section| !section.hidden.is_empty())
        {
            output.write_str(HIDDEN_LEGEND)?;
            output.write_char('\n')?;
        }

        for warning in &self.warnings {
            writeln!(output, "#warning {}", super::collapse_warning(warning))?;
        }

        for section in &self.sections {
            writeln!(
                output,
                "#section {} {}",
                lean_number(section.page_width),
                lean_number(section.page_height)
            )?;
            for story in &section.headers {
                write_lean_blocks(output, story.blocks())?;
            }
            write_lean_blocks(output, &section.blocks)?;
            for story in &section.footers {
                write_lean_blocks(output, story.blocks())?;
            }
            write_hidden(output, &section.hidden)?;
        }
        Ok(())
    }
}

fn write_lean_blocks<W: fmt::Write>(output: &mut W, blocks: &[CompactBlock]) -> fmt::Result {
    for block in blocks {
        match block {
            CompactBlock::Paragraph {
                role,
                runs,
                content,
                list,
                approx_page,
                breaks_before,
                ..
            } => {
                for kind in breaks_before {
                    writeln!(output, "BR {}", break_name(*kind))?;
                }
                if let Some(page) = approx_page {
                    writeln!(output, "~page {page}")?;
                }
                if let Some(list) = list {
                    writeln!(
                        output,
                        "LI {} {} {} {}",
                        list.level,
                        list_kind_code(list.kind),
                        escape_text(&list.label),
                        escape_text(content)
                    )?;
                } else {
                    writeln!(
                        output,
                        "{} {}",
                        paragraph_record(role),
                        escape_text(content)
                    )?;
                }
                write_lean_runs(output, Some(runs))?;
            }
            CompactBlock::Table {
                col_widths,
                cols,
                cells,
                ..
            } => {
                write!(output, "TB {cols}")?;
                for width in col_widths {
                    write!(output, " {}", lean_number(*width))?;
                }
                output.write_char('\n')?;
                for cell in cells {
                    writeln!(
                        output,
                        "c {} {} {} {} {}",
                        cell.row,
                        cell.col,
                        cell.row_span,
                        cell.col_span,
                        escape_text(&cell.content)
                    )?;
                    write_lean_runs(output, Some(&cell.runs))?;
                    if let Some(blocks) = &cell.blocks {
                        write_lean_blocks(output, blocks)?;
                    }
                }
            }
            CompactBlock::Image { width, height, .. } => match (width, height) {
                (Some(width), Some(height)) => {
                    writeln!(output, "I {} {}", lean_number(*width), lean_number(*height))?;
                }
                _ => output.write_str("I\n")?,
            },
            CompactBlock::Textbox { blocks, .. } => write_lean_blocks(output, blocks)?,
            CompactBlock::Break { kind } => writeln!(output, "BR {}", break_name(*kind))?,
        }
    }
    Ok(())
}

fn write_hidden<W: fmt::Write>(output: &mut W, hidden: &[HiddenItem]) -> fmt::Result {
    if hidden.is_empty() {
        return Ok(());
    }
    output.write_str("<hidden>\n")?;
    for item in hidden {
        write!(output, "{} ", item.kind)?;
        if let Some(element) = &item.element {
            write!(output, "{element} ")?;
        }
        writeln!(output, "{}", escape_text(&item.content))?;
    }
    output.write_str("</hidden>\n")
}

fn paragraph_record(role: &str) -> &str {
    match role {
        "h1" => "H1",
        "h2" => "H2",
        "h3" => "H3",
        "h4" => "H4",
        "h5" => "H5",
        "h6" => "H6",
        "h7" => "H7",
        "h8" => "H8",
        "h9" => "H9",
        "title" => "TI",
        "quote" => "Q",
        _ => "P",
    }
}

fn list_kind_code(kind: ListKind) -> &'static str {
    match kind {
        ListKind::Ordered => "o",
        ListKind::Bullet => "b",
    }
}

fn break_name(kind: BreakKind) -> &'static str {
    match kind {
        BreakKind::Page => "page",
        BreakKind::Column => "column",
        BreakKind::Section => "section",
    }
}
