use super::{
    AnnotationElement, BBox, Block, BreakKind, Element, Extraction, FlowExtraction, FlowTableCell,
    Font, HiddenItem, ListKind, Page, PathElement, TableElement, TextElement, TextRun,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fmt::Write as _;

#[derive(Clone)]
struct Segment {
    text: String,
    bold: bool,
    italic: bool,
    href: Option<String>,
}

#[derive(Clone)]
struct PageBlock {
    id: String,
    bbox: BBox,
    content: String,
    size: f64,
    bold: bool,
    segments: Vec<Segment>,
    column: usize,
    heading: Option<usize>,
    rendered_override: Option<String>,
}

struct PageMarkdown {
    blocks: Vec<PageBlock>,
    warnings: Vec<String>,
}

#[derive(Default)]
struct TableDetection {
    tables: Vec<DetectedTable>,
    warnings: Vec<String>,
}

struct DetectedTable {
    bbox: BBox,
    rows: usize,
    cols: usize,
    cells: Vec<DetectedCell>,
    consumed_text: Vec<String>,
}

struct DetectedCell {
    row: usize,
    col: usize,
    row_span: usize,
    col_span: usize,
    content: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RulingAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy)]
struct Ruling {
    axis: RulingAxis,
    position: f64,
    start: f64,
    end: f64,
}

const RULING_SNAP_TOLERANCE: f64 = 1.5;
const RULING_JOIN_TOLERANCE: f64 = 2.0;
const MIN_RULING_LENGTH: f64 = 8.0;
const MAX_RULINGS_PER_PAGE: usize = 512;
const MAX_GRID_SEPARATORS: usize = 64;
/// An interior ruling separator counts as *present* over a cell band when its
/// coverage of that band clears this fraction; below it the separator is
/// missing and the two adjacent cells are merged (a colspan/rowspan).
const INTERIOR_SEPARATOR_PRESENT: f64 = 0.5;

// --- Borderless / alignment-table detection (Phase 2, Markdown-only) ---------
//
// Alignment detection is strictly gated and composes as a fallback *after*
// ruled detection: it only ever runs on text no ruled table already claimed.
// The gate below is deliberately conservative (≥3 columns, ≥3 rows, stable
// gutters, tight column edges, dense fill) so prose, code, and key/value forms
// are left as ordinary reading-order text. Enabled by default so the TEDS
// harness can measure the lift; the gate plus the negative-fixture corpus are
// the safety net.
const ENABLE_ALIGNMENT_TABLES: bool = true;
/// Histogram/gutter bucket width in points. Quantized to integers so column
/// detection is deterministic across sub-point PDFium glyph-metric drift.
const X_BUCKET: f64 = 3.0;
/// A gutter must read as whitespace in at least this fraction of row bands.
const GUTTER_STABILITY: f64 = 0.8;
/// Minimum row bands for an alignment table (kills 2-row label/value forms).
const MIN_ALIGN_ROWS: usize = 3;
/// Minimum columns for an alignment table. Three (not two) is deliberate: it
/// rejects two-column key/value forms, the most common false positive.
const MIN_ALIGN_COLS: usize = 3;
/// A vertical gap between adjacent lines larger than this floor (or
/// `ROW_GAP_K × median line height`, whichever is greater) breaks one table
/// region from the surrounding prose.
const ROW_GAP_FLOOR: f64 = 3.0;
const ROW_GAP_K: f64 = 1.5;
/// Minimum occupied-cell fraction; sparse accidental alignment is rejected.
const FILL_RATIO: f64 = 0.5;
/// At least this fraction of bands must reach ≥2 columns (kills paragraphs with
/// one stray tab stop).
const ROW_REGULARITY: f64 = 0.6;
/// Cap on candidate lines fed to alignment detection, mirroring the ruling cap.
const MAX_ALIGN_LINES: usize = 512;

impl PageBlock {
    fn width(&self) -> f64 {
        (self.bbox.x1 - self.bbox.x0).max(0.0)
    }

    fn height(&self) -> f64 {
        (self.bbox.y1 - self.bbox.y0).max(0.0)
    }
}

impl Extraction {
    /// Renders deterministic GFM Markdown from the lossless paged model.
    pub fn to_markdown(&self) -> String {
        let mut output = String::new();
        self.write_markdown(&mut output)
            .expect("writing to a String cannot fail");
        output
    }

    /// Streams deterministic GFM Markdown into a bounded or ordinary writer.
    pub fn write_markdown<W: fmt::Write>(&self, output: &mut W) -> fmt::Result {
        let page_blocks = self
            .pages
            .iter()
            .map(|page| (page.width, blocks_for_page(page, &self.source.format)))
            .collect::<Vec<_>>();
        let body_size =
            weighted_median_font(page_blocks.iter().flat_map(|(_, page)| page.blocks.iter()));
        let mut markdown_warnings = self.warnings.clone();
        let mut pages = Vec::new();
        for (page_width, page) in page_blocks {
            markdown_warnings.extend(page.warnings);
            let rendered = render_page(page.blocks, page_width, body_size);
            if !rendered.is_empty() {
                pages.push(rendered);
            }
        }
        for (index, page) in pages.iter().enumerate() {
            if index > 0 {
                output.write_str("\n\n---\n\n")?;
            }
            output.write_str(page)?;
        }
        let hidden = self
            .pages
            .iter()
            .flat_map(|page| &page.hidden)
            .collect::<Vec<_>>();
        let has_context = !markdown_warnings.is_empty() || !hidden.is_empty();
        if !pages.is_empty() && has_context {
            output.write_str("\n\n")?;
        }
        write_trailing_context(output, &markdown_warnings, &hidden)?;
        if !pages.is_empty() || has_context {
            output.write_char('\n')?;
        }
        Ok(())
    }
}

impl FlowExtraction {
    /// Renders deterministic GFM Markdown from authored flow structure.
    pub fn to_markdown(&self) -> String {
        let mut output = String::new();
        self.write_markdown(&mut output)
            .expect("writing to a String cannot fail");
        output
    }

    /// Streams deterministic GFM Markdown into a bounded or ordinary writer.
    pub fn write_markdown<W: fmt::Write>(&self, output: &mut W) -> fmt::Result {
        let wrote_document = {
            let mut state = BlockWriter::new(output);
            for section in &self.sections {
                for story in &section.headers {
                    state.write_flow_blocks(story.blocks())?;
                }
                state.write_flow_blocks(&section.blocks)?;
                for story in &section.footers {
                    state.write_flow_blocks(story.blocks())?;
                }
            }
            state.wrote_block
        };
        let hidden = self
            .sections
            .iter()
            .flat_map(|section| &section.hidden)
            .collect::<Vec<_>>();
        let has_context = !self.warnings.is_empty() || !hidden.is_empty();
        if wrote_document && has_context {
            output.write_str("\n\n")?;
        }
        write_trailing_context(output, &self.warnings, &hidden)?;
        if wrote_document || has_context {
            output.write_char('\n')?;
        }
        Ok(())
    }
}

struct BlockWriter<'a, W> {
    output: &'a mut W,
    wrote_block: bool,
}

impl<'a, W: fmt::Write> BlockWriter<'a, W> {
    fn new(output: &'a mut W) -> Self {
        Self {
            output,
            wrote_block: false,
        }
    }

    fn block(&mut self, text: &str) -> fmt::Result {
        if text.is_empty() {
            return Ok(());
        }
        if self.wrote_block {
            self.output.write_str("\n\n")?;
        }
        self.output.write_str(text)?;
        self.wrote_block = true;
        Ok(())
    }

    fn write_flow_blocks(&mut self, blocks: &[Block]) -> fmt::Result {
        for block in blocks {
            match block {
                Block::Paragraph {
                    role,
                    runs,
                    content,
                    list,
                    breaks_before,
                    ..
                } => {
                    for kind in breaks_before {
                        self.write_break(*kind)?;
                    }
                    // Headings carry prominence structurally; suppress whole-run
                    // bold so a heading is never also wrapped in `**bold**`.
                    let heading_level = if list.is_some() {
                        None
                    } else if let Some(level) = heading_role(role) {
                        Some(level.min(6))
                    } else if role == "title" {
                        Some(1)
                    } else {
                        None
                    };
                    let rendered = render_runs(content, runs, None, heading_level.is_some());
                    let line = if let Some(list) = list {
                        let indent = "  ".repeat(list.level as usize);
                        let marker = match list.kind {
                            ListKind::Ordered => "1.",
                            ListKind::Bullet => "-",
                        };
                        format!("{indent}{marker} {rendered}")
                    } else if let Some(level) = heading_level {
                        format!("{} {rendered}", "#".repeat(level))
                    } else if role == "quote" {
                        rendered
                            .lines()
                            .map(|line| format!("> {line}"))
                            .collect::<Vec<_>>()
                            .join("\n")
                    } else {
                        rendered
                    };
                    self.block(&line)?;
                }
                Block::Table {
                    rows, cols, cells, ..
                } => self.block(&render_flow_table(*rows, *cols, cells))?,
                Block::Image { .. } => self.block("<!-- image -->")?,
                Block::Textbox { blocks, .. } => self.write_flow_blocks(blocks)?,
                Block::Break { kind } => self.write_break(*kind)?,
            }
        }
        Ok(())
    }

    fn write_break(&mut self, kind: BreakKind) -> fmt::Result {
        match kind {
            BreakKind::Page | BreakKind::Section => self.block("---"),
            BreakKind::Column => Ok(()),
        }
    }
}

fn render_page(mut blocks: Vec<PageBlock>, page_width: f64, body_size: f64) -> String {
    if blocks.is_empty() {
        return String::new();
    }
    assign_columns(&mut blocks, page_width);
    blocks = merge_inline_blocks(blocks);
    infer_headings(&mut blocks, body_size);
    blocks.sort_by(|a, b| {
        a.column
            .cmp(&b.column)
            .then_with(|| a.bbox.y0.total_cmp(&b.bbox.y0))
            .then_with(|| a.bbox.x0.total_cmp(&b.bbox.x0))
            .then_with(|| a.bbox.y1.total_cmp(&b.bbox.y1))
            .then_with(|| a.id.cmp(&b.id))
    });

    let mut output: Vec<String> = Vec::new();
    let mut previous: Option<&PageBlock> = None;
    for block in &blocks {
        let rendered = if let Some(table) = &block.rendered_override {
            table.clone()
        } else if let Some((kind, body_start)) = list_item(&block.content) {
            let body_segments = segments_after_byte(&block.segments, body_start);
            let marker = match kind {
                ListMarker::Ordered(number) => format!("{number}."),
                ListMarker::Bullet => "-".to_string(),
                ListMarker::Letter(marker) => format!("- {marker}"),
            };
            format!("{marker} {}", render_segments(&body_segments))
        } else if let Some(level) = block.heading {
            // A heading conveys prominence structurally, so it must not also be
            // wrapped in `**bold**` — suppress whole-run bold inside the heading.
            let content = render_segments_styled(&block.segments, true);
            format!("{} {content}", "#".repeat(level.min(6)))
        } else {
            render_segments(&block.segments)
        };
        if rendered.is_empty() {
            continue;
        }
        let break_before = previous.is_none()
            || previous.is_some_and(|prev| needs_paragraph_break(prev, block, body_size));
        if break_before {
            output.push(rendered);
        } else if let Some(last) = output.last_mut() {
            last.push(' ');
            last.push_str(&rendered);
        }
        previous = Some(block);
    }
    output.join("\n\n")
}

fn blocks_for_page(page: &Page, source_format: &str) -> PageMarkdown {
    let roles: BTreeMap<&str, usize> = page
        .hidden
        .iter()
        .filter(|item| item.kind == "role")
        .filter_map(|item| {
            let id = item.element.as_deref()?;
            let level = match item.content.as_str() {
                "title" | "ctrTitle" => 1,
                "subTitle" => 2,
                role => heading_role(role)?,
            };
            Some((id, level))
        })
        .collect();
    let detection = detect_tables(page, source_format);
    let consumed_text = detection
        .tables
        .iter()
        .flat_map(|table| table.consumed_text.iter().cloned())
        .collect::<BTreeSet<_>>();
    let mut blocks = Vec::new();
    let mut annotations = Vec::new();
    for element in &page.elements {
        match element {
            Element::Text(text) if consumed_text.contains(&text.id) => {}
            Element::Text(text) if !clean_text(&text.content).is_empty() => {
                blocks.push(PageBlock {
                    id: text.id.clone(),
                    bbox: text.bbox,
                    content: text.content.clone(),
                    size: finite_or(text.font.size, 0.0),
                    bold: whole_line_bold(&text.font, text.runs.as_deref()),
                    segments: segments_from_text(&text.content, &text.font, text.runs.as_deref()),
                    column: 0,
                    heading: roles.get(text.id.as_str()).copied(),
                    rendered_override: None,
                });
            }
            Element::Table(table) if source_format == "pptx" => blocks.push(PageBlock {
                id: table.id.clone(),
                bbox: table.bbox,
                content: table
                    .cells
                    .iter()
                    .map(|cell| cell.content.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
                size: 0.0,
                bold: false,
                segments: Vec::new(),
                column: 0,
                heading: None,
                rendered_override: Some(render_paged_table(table)),
            }),
            Element::Table(table) => {
                // Non-PPTX first-class tables retain geometric text flow. The
                // PDF ruled-table projection above is deliberately path-based.
                for (index, cell) in table.cells.iter().enumerate() {
                    if clean_text(&cell.content).is_empty() {
                        continue;
                    }
                    let font = cell
                        .runs
                        .as_deref()
                        .and_then(|runs| runs.first())
                        .map(|run| &run.font)
                        .cloned()
                        .unwrap_or_else(default_font);
                    blocks.push(PageBlock {
                        id: format!("{}-c{index}", table.id),
                        bbox: cell.bbox,
                        content: cell.content.clone(),
                        size: finite_or(font.size, 0.0),
                        bold: whole_line_bold(&font, cell.runs.as_deref()),
                        segments: segments_from_text(&cell.content, &font, cell.runs.as_deref()),
                        column: 0,
                        heading: None,
                        rendered_override: None,
                    });
                }
            }
            Element::Annotation(annotation) if annotation.uri.is_some() => {
                annotations.push(annotation)
            }
            _ => {}
        }
    }
    for (index, table) in detection.tables.iter().enumerate() {
        let cells = table
            .cells
            .iter()
            .map(|cell| FlowTableCell {
                row: cell.row,
                col: cell.col,
                row_span: cell.row_span,
                col_span: cell.col_span,
                content: cell.content.clone(),
                runs: Vec::new(),
                blocks: None,
            })
            .collect::<Vec<_>>();
        // Simple all-span-1 grids stay GFM pipe tables (human-readable and
        // byte-stable). Any merged cell routes to a raw HTML `<table>`, the
        // only Markdown-embeddable form that can carry colspan/rowspan.
        let has_span = cells
            .iter()
            .any(|cell| cell.row_span > 1 || cell.col_span > 1);
        let rendered = if has_span {
            render_html_table(table.rows, table.cols, &cells)
        } else {
            render_flow_table(table.rows, table.cols, &cells)
        };
        blocks.push(PageBlock {
            id: format!("p{}-markdown-table-{index}", page.page_number),
            bbox: table.bbox,
            content: table
                .cells
                .iter()
                .map(|cell| cell.content.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            size: 0.0,
            bold: false,
            segments: Vec::new(),
            column: 0,
            heading: None,
            rendered_override: Some(rendered),
        });
    }
    associate_annotations(&mut blocks, &annotations);
    PageMarkdown {
        blocks,
        warnings: detection.warnings,
    }
}

/// Orchestrates PDF table detection: ruled grids are authoritative and claim
/// their regions first; borderless/alignment detection then runs only on text
/// no ruled table already consumed. Non-PDF sources use neither path (their
/// tables are first-class schema elements handled elsewhere). Results merge and
/// sort by geometry so a page can carry both kinds without them fighting over
/// the same text.
fn detect_tables(page: &Page, source_format: &str) -> TableDetection {
    if source_format != "pdf" {
        return TableDetection::default();
    }
    let mut detection = detect_ruled_tables(page);
    if ENABLE_ALIGNMENT_TABLES {
        let alignment = detect_alignment_tables(page, &detection);
        detection.tables.extend(alignment.tables);
        detection.warnings.extend(alignment.warnings);
    }
    detection.tables.sort_by(|a, b| {
        a.bbox
            .y0
            .total_cmp(&b.bbox.y0)
            .then_with(|| a.bbox.x0.total_cmp(&b.bbox.x0))
            .then_with(|| a.bbox.y1.total_cmp(&b.bbox.y1))
            .then_with(|| a.bbox.x1.total_cmp(&b.bbox.x1))
    });
    detection
}

/// Detects only high-confidence ruled PDF tables for the Markdown projection.
/// Merged cells (missing interior separators) become colspan/rowspan and route
/// to an HTML `<table>`; simple grids stay GFM pipe tables.
fn detect_ruled_tables(page: &Page) -> TableDetection {
    let text = page
        .elements
        .iter()
        .filter_map(|element| match element {
            Element::Text(text) if !clean_text(&text.content).is_empty() => Some(text),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut rulings = page
        .elements
        .iter()
        .filter_map(|element| match element {
            Element::Path(path) => Some(path),
            _ => None,
        })
        .flat_map(rulings_for_path)
        .collect::<Vec<_>>();
    rulings.sort_by(|a, b| {
        ruling_axis_rank(a.axis)
            .cmp(&ruling_axis_rank(b.axis))
            .then_with(|| a.position.total_cmp(&b.position))
            .then_with(|| a.start.total_cmp(&b.start))
            .then_with(|| a.end.total_cmp(&b.end))
    });

    let horizontal_count = rulings
        .iter()
        .filter(|ruling| ruling.axis == RulingAxis::Horizontal)
        .count();
    let vertical_count = rulings.len() - horizontal_count;
    if rulings.len() > MAX_RULINGS_PER_PAGE {
        let warnings = if horizontal_count >= 3 && vertical_count >= 3 {
            vec![format!(
                "page {}: ruled-table candidate skipped because path density exceeded the Markdown detector limit",
                page.page_number
            )]
        } else {
            Vec::new()
        };
        return TableDetection {
            tables: Vec::new(),
            warnings,
        };
    }
    if horizontal_count < 3 || vertical_count < 3 {
        return TableDetection::default();
    }

    let mut components = DisjointSet::new(rulings.len());
    for horizontal in 0..horizontal_count {
        for vertical in horizontal_count..rulings.len() {
            if rulings_intersect(rulings[horizontal], rulings[vertical]) {
                components.union(horizontal, vertical);
            }
        }
    }
    let mut grouped = BTreeMap::<usize, Vec<Ruling>>::new();
    for (index, ruling) in rulings.iter().copied().enumerate() {
        grouped
            .entry(components.find(index))
            .or_default()
            .push(ruling);
    }

    let mut detection = TableDetection::default();
    for component in grouped.values() {
        match table_from_component(component, &text) {
            ComponentTable::NotGrid => {}
            ComponentTable::Rejected(reason) => detection.warnings.push(format!(
                "page {}: ruled-table candidate skipped: {reason}",
                page.page_number
            )),
            ComponentTable::Detected(table) => detection.tables.push(table),
        }
    }
    detection.tables.sort_by(|a, b| {
        a.bbox
            .y0
            .total_cmp(&b.bbox.y0)
            .then_with(|| a.bbox.x0.total_cmp(&b.bbox.x0))
            .then_with(|| a.bbox.y1.total_cmp(&b.bbox.y1))
            .then_with(|| a.bbox.x1.total_cmp(&b.bbox.x1))
    });
    detection
}

fn ruling_axis_rank(axis: RulingAxis) -> u8 {
    match axis {
        RulingAxis::Horizontal => 0,
        RulingAxis::Vertical => 1,
    }
}

/// A visual line of free text: one baseline's worth of elements, left-to-right.
struct AlignLine<'a> {
    y0: f64,
    y1: f64,
    x0: f64,
    x1: f64,
    items: Vec<&'a TextElement>,
}

/// Detects borderless/alignment tables in the text no ruled table already
/// claimed. Strictly gated (≥3 columns, ≥3 rows, stable gutters, tight column
/// edges, dense fill) so prose, code, and key/value forms stay reading-order
/// text. Column detection is a whitespace-gutter projection over integer
/// buckets; rows are visual lines grouped into whitespace-separated regions.
fn detect_alignment_tables(page: &Page, ruled: &TableDetection) -> TableDetection {
    let consumed: BTreeSet<&str> = ruled
        .tables
        .iter()
        .flat_map(|table| table.consumed_text.iter().map(String::as_str))
        .collect();
    let ruled_boxes: Vec<BBox> = ruled.tables.iter().map(|table| table.bbox).collect();
    let inside_ruled = |x: f64, y: f64| {
        ruled_boxes.iter().any(|b| {
            x >= b.x0 - RULING_SNAP_TOLERANCE
                && x <= b.x1 + RULING_SNAP_TOLERANCE
                && y >= b.y0 - RULING_SNAP_TOLERANCE
                && y <= b.y1 + RULING_SNAP_TOLERANCE
        })
    };
    let mut items: Vec<&TextElement> = page
        .elements
        .iter()
        .filter_map(|element| match element {
            Element::Text(text) if !clean_text(&text.content).is_empty() => Some(text),
            _ => None,
        })
        .filter(|text| {
            [text.bbox.x0, text.bbox.y0, text.bbox.x1, text.bbox.y1]
                .into_iter()
                .all(f64::is_finite)
                && text.bbox.x1 >= text.bbox.x0
                && text.bbox.y1 >= text.bbox.y0
        })
        .filter(|text| !consumed.contains(text.id.as_str()))
        .filter(|text| {
            let cx = (text.bbox.x0 + text.bbox.x1) / 2.0;
            let cy = (text.bbox.y0 + text.bbox.y1) / 2.0;
            !inside_ruled(cx, cy)
        })
        .collect();
    if items.len() < MIN_ALIGN_ROWS * MIN_ALIGN_COLS || items.len() > MAX_ALIGN_LINES {
        return TableDetection::default();
    }
    items.sort_by(|a, b| {
        a.bbox
            .y0
            .total_cmp(&b.bbox.y0)
            .then_with(|| a.bbox.x0.total_cmp(&b.bbox.x0))
            .then_with(|| a.id.cmp(&b.id))
    });

    // Group into visual lines by vertical overlap (items are pre-sorted by y0,
    // so only the current line can match).
    let mut lines: Vec<AlignLine> = Vec::new();
    for item in items {
        let matched = lines.last_mut().is_some_and(|line| {
            let overlap = (line.y1.min(item.bbox.y1) - line.y0.max(item.bbox.y0)).max(0.0);
            let min_height = (line.y1 - line.y0)
                .min(item.bbox.y1 - item.bbox.y0)
                .max(1.0);
            overlap > 0.35 * min_height
        });
        if matched {
            let line = lines.last_mut().unwrap();
            line.y0 = line.y0.min(item.bbox.y0);
            line.y1 = line.y1.max(item.bbox.y1);
            line.x0 = line.x0.min(item.bbox.x0);
            line.x1 = line.x1.max(item.bbox.x1);
            line.items.push(item);
        } else {
            lines.push(AlignLine {
                y0: item.bbox.y0,
                y1: item.bbox.y1,
                x0: item.bbox.x0,
                x1: item.bbox.x1,
                items: vec![item],
            });
        }
    }
    for line in &mut lines {
        line.items.sort_by(|a, b| {
            a.bbox
                .x0
                .total_cmp(&b.bbox.x0)
                .then_with(|| a.bbox.x1.total_cmp(&b.bbox.x1))
                .then_with(|| a.id.cmp(&b.id))
        });
    }
    if lines.len() < MIN_ALIGN_ROWS {
        return TableDetection::default();
    }

    // A large vertical gap between adjacent lines separates a table block from
    // surrounding prose, so we test each whitespace-bounded region on its own.
    let mut heights = lines
        .iter()
        .map(|line| (line.y1 - line.y0).max(1.0))
        .collect::<Vec<_>>();
    let median_height = median(&mut heights).unwrap_or(10.0);
    let region_gap = ROW_GAP_FLOOR.max(median_height * ROW_GAP_K);
    let mut detection = TableDetection::default();
    let mut region_start = 0;
    for index in 1..=lines.len() {
        let split = index == lines.len() || {
            let gap = lines[index].y0 - lines[index - 1].y1;
            gap > region_gap
        };
        if !split {
            continue;
        }
        let region = &lines[region_start..index];
        region_start = index;
        if region.len() >= MIN_ALIGN_ROWS {
            match alignment_region_table(region) {
                AlignmentRegion::None => {}
                AlignmentRegion::NearMiss(reason) => detection.warnings.push(format!(
                    "page {}: borderless-table candidate skipped: {reason}",
                    page.page_number
                )),
                AlignmentRegion::Detected(table) => {
                    let overlaps = ruled_boxes.iter().any(|b| {
                        table.bbox.x1 > b.x0
                            && table.bbox.x0 < b.x1
                            && table.bbox.y1 > b.y0
                            && table.bbox.y0 < b.y1
                    });
                    if !overlaps {
                        detection.tables.push(table);
                    }
                }
            }
        }
    }
    detection
}

enum AlignmentRegion {
    None,
    NearMiss(&'static str),
    Detected(DetectedTable),
}

/// Runs column detection + the confidence gate on one whitespace-bounded region
/// of lines. Each line is a table row; columns come from stable whitespace
/// gutters. Returns a detected table, a near-miss warning, or nothing.
fn alignment_region_table(lines: &[AlignLine]) -> AlignmentRegion {
    let region_x0 = lines
        .iter()
        .map(|line| line.x0)
        .fold(f64::INFINITY, f64::min);
    let region_x1 = lines
        .iter()
        .map(|line| line.x1)
        .fold(f64::NEG_INFINITY, f64::max);
    if !region_x0.is_finite() || region_x1 <= region_x0 {
        return AlignmentRegion::None;
    }
    let bucket_count = (((region_x1 - region_x0) / X_BUCKET).ceil() as usize).saturating_add(1);
    if bucket_count == 0 || bucket_count > 4096 {
        return AlignmentRegion::None;
    }
    // Per-bucket count of lines whose text covers that bucket. A bucket that is
    // whitespace in ≥ GUTTER_STABILITY of lines is a gutter.
    let mut coverage = vec![0usize; bucket_count];
    for line in lines {
        let mut covered = vec![false; bucket_count];
        for item in &line.items {
            let lo = (((item.bbox.x0 - region_x0) / X_BUCKET).floor() as isize).max(0) as usize;
            let hi =
                (((item.bbox.x1 - region_x0) / X_BUCKET).floor() as usize).min(bucket_count - 1);
            for bucket in covered.iter_mut().take(hi + 1).skip(lo) {
                *bucket = true;
            }
        }
        for (bucket, hit) in covered.iter().enumerate() {
            if *hit {
                coverage[bucket] += 1;
            }
        }
    }
    let line_count = lines.len();
    let gutter_ceiling = ((1.0 - GUTTER_STABILITY) * line_count as f64).floor() as usize;
    // Columns are maximal runs of non-gutter buckets; leading/trailing gutter
    // runs are page margins and drop out naturally.
    let mut columns: Vec<(f64, f64)> = Vec::new();
    let mut run_start: Option<usize> = None;
    for (bucket, count) in coverage.iter().enumerate() {
        let is_gutter = *count <= gutter_ceiling;
        match (is_gutter, run_start) {
            (false, None) => run_start = Some(bucket),
            (true, Some(start)) => {
                columns.push((
                    region_x0 + start as f64 * X_BUCKET,
                    region_x0 + bucket as f64 * X_BUCKET,
                ));
                run_start = None;
            }
            _ => {}
        }
    }
    if let Some(start) = run_start {
        columns.push((
            region_x0 + start as f64 * X_BUCKET,
            region_x0 + bucket_count as f64 * X_BUCKET,
        ));
    }
    if columns.len() < MIN_ALIGN_COLS {
        return AlignmentRegion::None;
    }
    if columns.len() > MAX_GRID_SEPARATORS {
        return AlignmentRegion::None;
    }
    let cols = columns.len();
    let rows = lines.len();

    // Assign every item to the column range its x-interval overlaps; a wide
    // item straddling a gutter becomes a colspan.
    let mut occupied = vec![false; rows * cols];
    let mut left_edges: Vec<Vec<f64>> = vec![Vec::new(); cols];
    let mut right_edges: Vec<Vec<f64>> = vec![Vec::new(); cols];
    for line in lines {
        for item in &line.items {
            let Some((start_col, end_col)) = column_span_of(item.bbox.x0, item.bbox.x1, &columns)
            else {
                continue;
            };
            if start_col == end_col {
                left_edges[start_col].push(item.bbox.x0);
                right_edges[start_col].push(item.bbox.x1);
            }
        }
    }
    // Column edge tightness: left-aligned columns have a stable left edge,
    // right-aligned (numeric) columns a stable right edge. A column with enough
    // single-column items must be tight on at least one side; ragged code fails.
    for col in 0..cols {
        let tight_left = stdev(&left_edges[col]).map(|s| s <= X_BUCKET);
        let tight_right = stdev(&right_edges[col]).map(|s| s <= X_BUCKET);
        match (tight_left, tight_right) {
            (Some(l), Some(r)) if !l && !r => return AlignmentRegion::None,
            _ => {}
        }
    }

    let mut cells: Vec<DetectedCell> = Vec::new();
    let mut consumed_text: Vec<String> = Vec::new();
    let mut rows_reaching_two = 0usize;
    for (row, line) in lines.iter().enumerate() {
        // Anchor col -> (col_span, item contents, item ids).
        let mut row_cells: BTreeMap<usize, (usize, Vec<&TextElement>)> = BTreeMap::new();
        let mut owner: Vec<Option<usize>> = vec![None; cols];
        for item in &line.items {
            let Some((start_col, end_col)) = column_span_of(item.bbox.x0, item.bbox.x1, &columns)
            else {
                continue;
            };
            let anchor = owner[start_col].unwrap_or(start_col);
            let entry = row_cells.entry(anchor).or_insert((1, Vec::new()));
            entry.0 = entry.0.max(end_col - anchor + 1);
            entry.1.push(item);
            for slot in owner
                .iter_mut()
                .take(end_col.min(cols - 1) + 1)
                .skip(anchor)
            {
                *slot = Some(anchor);
            }
        }
        let distinct = row_cells.len();
        if distinct >= 2 {
            rows_reaching_two += 1;
        }
        for (col, (col_span, mut row_items)) in row_cells {
            row_items.sort_by(|a, b| {
                a.bbox
                    .x0
                    .total_cmp(&b.bbox.x0)
                    .then_with(|| a.id.cmp(&b.id))
            });
            for span in 0..col_span {
                if col + span < cols {
                    occupied[row * cols + col + span] = true;
                }
            }
            consumed_text.extend(row_items.iter().map(|item| item.id.clone()));
            cells.push(DetectedCell {
                row,
                col,
                row_span: 1,
                col_span,
                content: row_items
                    .iter()
                    .map(|item| clean_text(&item.content))
                    .filter(|content| !content.is_empty())
                    .collect::<Vec<_>>()
                    .join(" "),
            });
        }
    }

    let occupied_count = occupied.iter().filter(|cell| **cell).count();
    let fill = occupied_count as f64 / (rows * cols) as f64;
    let regularity = rows_reaching_two as f64 / rows as f64;
    if regularity < ROW_REGULARITY {
        return AlignmentRegion::None;
    }
    if fill < FILL_RATIO {
        // Strong near-miss: real column/row structure but too sparse to trust.
        if fill >= FILL_RATIO - 0.15 {
            return AlignmentRegion::NearMiss(
                "borderless grid found but too few cells were filled to trust it",
            );
        }
        return AlignmentRegion::None;
    }

    let bbox = BBox {
        x0: region_x0,
        y0: lines
            .iter()
            .map(|line| line.y0)
            .fold(f64::INFINITY, f64::min),
        x1: region_x1,
        y1: lines
            .iter()
            .map(|line| line.y1)
            .fold(f64::NEG_INFINITY, f64::max),
    };
    consumed_text.sort();
    consumed_text.dedup();
    cells.sort_by(|a, b| a.row.cmp(&b.row).then_with(|| a.col.cmp(&b.col)));
    AlignmentRegion::Detected(DetectedTable {
        bbox,
        rows,
        cols,
        cells,
        consumed_text,
    })
}

/// The inclusive column index range whose x-extent overlaps `[x0, x1]`.
fn column_span_of(x0: f64, x1: f64, columns: &[(f64, f64)]) -> Option<(usize, usize)> {
    let mut start = None;
    let mut end = None;
    for (index, column) in columns.iter().enumerate() {
        let overlaps =
            x1 > column.0 - RULING_SNAP_TOLERANCE && x0 < column.1 + RULING_SNAP_TOLERANCE;
        if overlaps {
            start.get_or_insert(index);
            end = Some(index);
        }
    }
    Some((start?, end?))
}

fn stdev(values: &[f64]) -> Option<f64> {
    if values.len() < 2 {
        return None;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance =
        values.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / values.len() as f64;
    Some(variance.sqrt())
}

fn rulings_for_path(path: &PathElement) -> Vec<Ruling> {
    let bbox = path.bbox;
    if ![bbox.x0, bbox.y0, bbox.x1, bbox.y1]
        .into_iter()
        .all(f64::is_finite)
        || bbox.x1 < bbox.x0
        || bbox.y1 < bbox.y0
    {
        return Vec::new();
    }
    let width = bbox.x1 - bbox.x0;
    let height = bbox.y1 - bbox.y0;
    let stroke_width = path.stroke_width.unwrap_or(1.0).abs();
    let painted = path.stroke.is_some() || path.fill.is_some();
    if !painted || (path.stroke.is_some() && stroke_width > 3.0) {
        return Vec::new();
    }
    let thin_limit = if path.stroke.is_some() {
        (stroke_width * 2.0).clamp(1.5, 6.0)
    } else {
        3.0
    };
    if width >= MIN_RULING_LENGTH && height <= thin_limit {
        return vec![Ruling {
            axis: RulingAxis::Horizontal,
            position: (bbox.y0 + bbox.y1) / 2.0,
            start: bbox.x0,
            end: bbox.x1,
        }];
    }
    if height >= MIN_RULING_LENGTH && width <= thin_limit {
        return vec![Ruling {
            axis: RulingAxis::Vertical,
            position: (bbox.x0 + bbox.x1) / 2.0,
            start: bbox.y0,
            end: bbox.y1,
        }];
    }
    if path.stroke.is_none() || width < MIN_RULING_LENGTH || height < MIN_RULING_LENGTH {
        return Vec::new();
    }
    // PDFium path bounds include the stroke expansion. Move rectangle edges
    // back to their centerlines so adjacent stroked cells snap together.
    let inset = stroke_width.min(width / 2.0).min(height / 2.0);
    let rect_x0 = bbox.x0 + inset;
    let rect_x1 = bbox.x1 - inset;
    let rect_y0 = bbox.y0 + inset;
    let rect_y1 = bbox.y1 - inset;
    vec![
        Ruling {
            axis: RulingAxis::Horizontal,
            position: rect_y0,
            start: rect_x0,
            end: rect_x1,
        },
        Ruling {
            axis: RulingAxis::Horizontal,
            position: rect_y1,
            start: rect_x0,
            end: rect_x1,
        },
        Ruling {
            axis: RulingAxis::Vertical,
            position: rect_x0,
            start: rect_y0,
            end: rect_y1,
        },
        Ruling {
            axis: RulingAxis::Vertical,
            position: rect_x1,
            start: rect_y0,
            end: rect_y1,
        },
    ]
}

fn rulings_intersect(horizontal: Ruling, vertical: Ruling) -> bool {
    horizontal.axis == RulingAxis::Horizontal
        && vertical.axis == RulingAxis::Vertical
        && vertical.position >= horizontal.start - RULING_SNAP_TOLERANCE
        && vertical.position <= horizontal.end + RULING_SNAP_TOLERANCE
        && horizontal.position >= vertical.start - RULING_SNAP_TOLERANCE
        && horizontal.position <= vertical.end + RULING_SNAP_TOLERANCE
}

enum ComponentTable {
    NotGrid,
    Rejected(&'static str),
    Detected(DetectedTable),
}

fn table_from_component(component: &[Ruling], text: &[&TextElement]) -> ComponentTable {
    let xs = clustered_positions(
        component
            .iter()
            .filter(|ruling| ruling.axis == RulingAxis::Vertical)
            .map(|ruling| ruling.position),
    );
    let ys = clustered_positions(
        component
            .iter()
            .filter(|ruling| ruling.axis == RulingAxis::Horizontal)
            .map(|ruling| ruling.position),
    );
    if xs.len() < 3 || ys.len() < 3 {
        return ComponentTable::NotGrid;
    }
    if xs.len() > MAX_GRID_SEPARATORS || ys.len() > MAX_GRID_SEPARATORS {
        return ComponentTable::Rejected("too many row or column separators");
    }

    let x0 = xs[0];
    let x1 = *xs.last().unwrap();
    let y0 = ys[0];
    let y1 = *ys.last().unwrap();
    let horizontal_coverage = ys
        .iter()
        .map(|position| ruling_coverage(component, RulingAxis::Horizontal, *position, x0, x1))
        .collect::<Vec<_>>();
    let vertical_coverage = xs
        .iter()
        .map(|position| ruling_coverage(component, RulingAxis::Vertical, *position, y0, y1))
        .collect::<Vec<_>>();
    let outer_closed = horizontal_coverage[0] >= 0.9
        && *horizontal_coverage.last().unwrap() >= 0.9
        && vertical_coverage[0] >= 0.9
        && *vertical_coverage.last().unwrap() >= 0.9;
    let separators_present = horizontal_coverage.iter().all(|coverage| *coverage >= 0.45)
        && vertical_coverage.iter().all(|coverage| *coverage >= 0.45);
    if !outer_closed || !separators_present {
        return ComponentTable::Rejected("rulings do not form a closed grid");
    }

    let rows = ys.len() - 1;
    let cols = xs.len() - 1;
    let Some(cell_count) = rows.checked_mul(cols) else {
        return ComponentTable::Rejected("grid dimensions overflowed");
    };
    let mut assigned = vec![Vec::<&TextElement>::new(); cell_count];
    for item in text {
        let center_x = (item.bbox.x0 + item.bbox.x1) / 2.0;
        let center_y = (item.bbox.y0 + item.bbox.y1) / 2.0;
        if center_x < x0 - RULING_SNAP_TOLERANCE
            || center_x > x1 + RULING_SNAP_TOLERANCE
            || center_y < y0 - RULING_SNAP_TOLERANCE
            || center_y > y1 + RULING_SNAP_TOLERANCE
        {
            continue;
        }
        let Some(col) = separator_interval(&xs, center_x) else {
            continue;
        };
        let Some(row) = separator_interval(&ys, center_y) else {
            continue;
        };
        assigned[row * cols + col].push(item);
    }
    let occupied = assigned.iter().filter(|cell| !cell.is_empty()).count();
    let occupied_rows = (0..rows)
        .filter(|row| (0..cols).any(|col| !assigned[row * cols + col].is_empty()))
        .count();
    let occupied_cols = (0..cols)
        .filter(|col| (0..rows).any(|row| !assigned[row * cols + col].is_empty()))
        .count();
    let minimum_occupied = 4_usize.max(cell_count.div_ceil(3));
    if occupied < minimum_occupied || occupied_rows < 2 || occupied_cols < 2 {
        return ComponentTable::Rejected("grid has too little cell text");
    }

    // An interior vertical separator `xs[col]` is present over row band `row`
    // when its clipped coverage clears the threshold; a missing one means the
    // cell on its left merges rightward (a colspan). Horizontals are symmetric.
    let vertical_present = |col: usize, row: usize| -> bool {
        ruling_coverage(
            component,
            RulingAxis::Vertical,
            xs[col],
            ys[row],
            ys[row + 1],
        ) >= INTERIOR_SEPARATOR_PRESENT
    };
    let horizontal_present = |row: usize, col: usize| -> bool {
        ruling_coverage(
            component,
            RulingAxis::Horizontal,
            ys[row],
            xs[col],
            xs[col + 1],
        ) >= INTERIOR_SEPARATOR_PRESENT
    };

    let mut covered = vec![false; cell_count];
    let mut cells = Vec::new();
    let mut consumed_text = Vec::new();
    for row in 0..rows {
        for col in 0..cols {
            if covered[row * cols + col] {
                continue;
            }
            // Extend right while the vertical separator on the cell's right edge
            // is missing over this row band.
            let mut col_span = 1;
            while col + col_span < cols && !vertical_present(col + col_span, row) {
                col_span += 1;
            }
            // Extend down while the horizontal separator below is missing across
            // the whole column span (a clean rectangular merge).
            let mut row_span = 1;
            'rows: while row + row_span < rows {
                for span_col in col..col + col_span {
                    if horizontal_present(row + row_span, span_col) {
                        break 'rows;
                    }
                }
                row_span += 1;
            }
            // Assign every base cell in the merged rectangle to this anchor; the
            // covered non-anchor positions emit no cell of their own.
            let mut items: Vec<&TextElement> = Vec::new();
            for span_row in row..row + row_span {
                for span_col in col..col + col_span {
                    if span_row != row || span_col != col {
                        covered[span_row * cols + span_col] = true;
                    }
                    items.append(&mut assigned[span_row * cols + span_col]);
                }
            }
            items.sort_by(|a, b| {
                a.bbox
                    .y0
                    .total_cmp(&b.bbox.y0)
                    .then_with(|| a.bbox.x0.total_cmp(&b.bbox.x0))
                    .then_with(|| a.bbox.y1.total_cmp(&b.bbox.y1))
                    .then_with(|| a.id.cmp(&b.id))
            });
            consumed_text.extend(items.iter().map(|item| item.id.clone()));
            cells.push(DetectedCell {
                row,
                col,
                row_span,
                col_span,
                content: items
                    .iter()
                    .map(|item| clean_text(&item.content))
                    .filter(|content| !content.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n"),
            });
        }
    }
    consumed_text.sort();
    consumed_text.dedup();
    ComponentTable::Detected(DetectedTable {
        bbox: BBox { x0, y0, x1, y1 },
        rows,
        cols,
        cells,
        consumed_text,
    })
}

fn clustered_positions(values: impl Iterator<Item = f64>) -> Vec<f64> {
    let mut values = values.filter(|value| value.is_finite()).collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    let mut clusters = Vec::<Vec<f64>>::new();
    for value in values {
        if clusters
            .last()
            .is_some_and(|cluster| value - cluster[0] <= RULING_SNAP_TOLERANCE)
        {
            clusters.last_mut().unwrap().push(value);
        } else {
            clusters.push(vec![value]);
        }
    }
    clusters
        .into_iter()
        .filter_map(|mut cluster| median(&mut cluster))
        .collect()
}

fn ruling_coverage(
    component: &[Ruling],
    axis: RulingAxis,
    position: f64,
    start: f64,
    end: f64,
) -> f64 {
    let length = end - start;
    if length <= 0.0 {
        return 0.0;
    }
    let mut intervals = component
        .iter()
        .filter(|ruling| {
            ruling.axis == axis && (ruling.position - position).abs() <= RULING_SNAP_TOLERANCE
        })
        .filter_map(|ruling| {
            let clipped_start = ruling.start.max(start);
            let clipped_end = ruling.end.min(end);
            (clipped_end > clipped_start).then_some((clipped_start, clipped_end))
        })
        .collect::<Vec<_>>();
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));
    let mut covered = 0.0;
    let mut current: Option<(f64, f64)> = None;
    for interval in intervals {
        match current {
            Some((current_start, current_end))
                if interval.0 <= current_end + RULING_JOIN_TOLERANCE =>
            {
                current = Some((current_start, current_end.max(interval.1)));
            }
            Some((current_start, current_end)) => {
                covered += current_end - current_start;
                current = Some(interval);
            }
            None => current = Some(interval),
        }
    }
    if let Some((current_start, current_end)) = current {
        covered += current_end - current_start;
    }
    (covered / length).clamp(0.0, 1.0)
}

fn separator_interval(separators: &[f64], value: f64) -> Option<usize> {
    separators.windows(2).position(|pair| {
        value >= pair[0] - RULING_SNAP_TOLERANCE && value <= pair[1] + RULING_SNAP_TOLERANCE
    })
}

struct DisjointSet {
    parents: Vec<usize>,
}

impl DisjointSet {
    fn new(len: usize) -> Self {
        Self {
            parents: (0..len).collect(),
        }
    }

    fn find(&mut self, mut index: usize) -> usize {
        while self.parents[index] != index {
            let parent = self.parents[index];
            self.parents[index] = self.parents[parent];
            index = self.parents[index];
        }
        index
    }

    fn union(&mut self, left: usize, right: usize) {
        let left_root = self.find(left);
        let right_root = self.find(right);
        if left_root != right_root {
            let (root, child) = if left_root < right_root {
                (left_root, right_root)
            } else {
                (right_root, left_root)
            };
            self.parents[child] = root;
        }
    }
}

fn default_font() -> Font {
    Font {
        name: String::new(),
        size: 0.0,
        bold: false,
        italic: false,
    }
}

fn whole_line_bold(font: &Font, runs: Option<&[TextRun]>) -> bool {
    let Some(runs) = runs.filter(|runs| !runs.is_empty()) else {
        return font.bold;
    };
    let visible = runs
        .iter()
        .filter(|run| !clean_text(&run.content).is_empty())
        .collect::<Vec<_>>();
    if visible.is_empty() {
        font.bold
    } else {
        visible.iter().all(|run| run.font.bold)
    }
}

fn segments_from_text(content: &str, font: &Font, runs: Option<&[TextRun]>) -> Vec<Segment> {
    let Some(runs) = runs.filter(|runs| !runs.is_empty()) else {
        return vec![segment(content, font, None)];
    };
    let mut result = Vec::new();
    let mut cursor = 0;
    for run in runs {
        if let Some(relative) = content[cursor..].find(&run.content) {
            let start = cursor + relative;
            if start > cursor {
                result.push(segment(&content[cursor..start], font, None));
            }
            result.push(segment(&run.content, &run.font, run.href.clone()));
            cursor = start + run.content.len();
        } else {
            result.push(segment(&run.content, &run.font, run.href.clone()));
        }
    }
    if cursor < content.len() {
        result.push(segment(&content[cursor..], font, None));
    }
    result
}

fn segment(text: &str, font: &Font, href: Option<String>) -> Segment {
    Segment {
        text: text.to_string(),
        bold: font.bold,
        italic: font.italic,
        href,
    }
}

fn associate_annotations(blocks: &mut [PageBlock], annotations: &[&AnnotationElement]) {
    for annotation in annotations {
        let Some(uri) = &annotation.uri else { continue };
        let Some((index, _)) = blocks.iter().enumerate().min_by(|(_, a), (_, b)| {
            overlap_area(b, &annotation.bbox)
                .total_cmp(&overlap_area(a, &annotation.bbox))
                .then_with(|| {
                    distance_to_box(a, &annotation.bbox)
                        .total_cmp(&distance_to_box(b, &annotation.bbox))
                })
                .then_with(|| a.bbox.y0.total_cmp(&b.bbox.y0))
                .then_with(|| a.bbox.x0.total_cmp(&b.bbox.x0))
                .then_with(|| a.id.cmp(&b.id))
        }) else {
            continue;
        };
        for segment in &mut blocks[index].segments {
            if segment.href.is_none() && !clean_text(&segment.text).is_empty() {
                segment.href = Some(uri.clone());
            }
        }
    }
}

fn overlap_area(block: &PageBlock, bbox: &BBox) -> f64 {
    (block.bbox.x1.min(bbox.x1) - block.bbox.x0.max(bbox.x0)).max(0.0)
        * (block.bbox.y1.min(bbox.y1) - block.bbox.y0.max(bbox.y0)).max(0.0)
}

fn distance_to_box(block: &PageBlock, bbox: &BBox) -> f64 {
    let dx = (block.bbox.x0 + block.bbox.x1 - bbox.x0 - bbox.x1) / 2.0;
    let dy = (block.bbox.y0 + block.bbox.y1 - bbox.y0 - bbox.y1) / 2.0;
    dx.hypot(dy)
}

fn weighted_median_font<'a>(blocks: impl IntoIterator<Item = &'a PageBlock>) -> f64 {
    let mut weighted = Vec::new();
    let mut fallback = Vec::new();
    for block in blocks
        .into_iter()
        .filter(|block| block.rendered_override.is_none() && block.size > 0.0)
    {
        fallback.push(block.size);
        let weight = (clean_text(&block.content).chars().count() / 8).clamp(1, 12);
        weighted.extend(std::iter::repeat_n(block.size, weight));
    }
    median(if weighted.is_empty() {
        &mut fallback
    } else {
        &mut weighted
    })
    .unwrap_or(10.0)
}

fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    Some(if values.len().is_multiple_of(2) {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    })
}

fn infer_headings(blocks: &mut [PageBlock], body_size: f64) {
    let threshold = (body_size + 1.0).max(body_size * 1.12);
    let candidate_sizes = blocks
        .iter()
        .filter(|block| {
            block.heading.is_none() && is_heading_candidate(block, body_size, threshold)
        })
        .map(|block| block.size)
        .collect::<Vec<_>>();
    let ranks = cluster_sizes(candidate_sizes, body_size);
    for block in blocks.iter_mut().filter(|block| block.heading.is_none()) {
        if !is_heading_candidate(block, body_size, threshold) {
            continue;
        }
        let Some((rank, _)) = ranks
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| (*a - block.size).abs().total_cmp(&(*b - block.size).abs()))
        else {
            continue;
        };
        let mut level = (rank + 1).min(4);
        if block.bold && level > 1 {
            level -= 1;
        }
        block.heading = Some(level);
    }
}

/// A block is a heading candidate when it is either markedly larger than body
/// text, or a body-size bold run that also *reads* like a heading (a short
/// line). The short-line guard keeps bold inline emphasis inside a paragraph
/// from being promoted to a heading.
fn is_heading_candidate(block: &PageBlock, body_size: f64, threshold: f64) -> bool {
    if block.rendered_override.is_some() {
        return false;
    }
    if block.size >= threshold {
        return true;
    }
    block.bold && block.size >= body_size * 0.98 && heading_like_text(&block.content)
}

fn heading_like_text(content: &str) -> bool {
    let clean = clean_text(content);
    if clean.is_empty() {
        return false;
    }
    clean.split_whitespace().count() <= 12 && clean.chars().count() <= 100
}

fn cluster_sizes(mut sizes: Vec<f64>, body_size: f64) -> Vec<f64> {
    sizes.sort_by(|a, b| b.total_cmp(a));
    sizes.dedup_by(|a, b| a.total_cmp(b).is_eq());
    let tolerance = 0.4_f64.max(body_size * 0.04);
    let mut clusters: Vec<Vec<f64>> = Vec::new();
    for size in sizes {
        let joins = clusters.last_mut().is_some_and(|cluster| {
            let mut copy = cluster.clone();
            (size - median(&mut copy).unwrap_or(size)).abs() <= tolerance
        });
        if joins {
            clusters.last_mut().unwrap().push(size);
        } else {
            clusters.push(vec![size]);
        }
    }
    clusters
        .into_iter()
        .filter_map(|mut cluster| median(&mut cluster))
        .collect()
}

fn assign_columns(blocks: &mut [PageBlock], page_width: f64) {
    if blocks.is_empty() {
        return;
    }
    let columns = detect_columns(blocks, page_width);
    for block in blocks.iter_mut() {
        block.column = column_for(block, &columns);
    }
}

/// A detected text column: the horizontal extent its body text occupies,
/// ordered left→right in the returned vector.
#[derive(Clone, Copy)]
struct ColumnRange {
    x0: f64,
    x1: f64,
}

/// Detects true multi-column layout by finding vertical gutters — bands of the
/// page width that no body text ever crosses.
///
/// The naive approach of clustering left edges misfires on indentation, ragged
/// lists, and centered lines. Instead we take substantial ("anchor") blocks,
/// drop the full-width spanning runs (titles/rules that straddle columns), then
/// union the remaining x-intervals. A gap wider than `gutter_min` between two
/// unions is a gutter; the intervals on either side are columns. Each surviving
/// column must carry real support and span most of the content height, so a
/// lone indented block or a short centered subtitle cannot fabricate a column.
///
/// Returns an empty vector for single-column pages (every block → column 0).
fn detect_columns(blocks: &[PageBlock], page_width: f64) -> Vec<ColumnRange> {
    let anchors = blocks
        .iter()
        .filter(|block| block.rendered_override.is_none())
        .filter(|block| {
            clean_text(&block.content).chars().count() >= 20 || block.width() >= page_width * 0.16
        })
        .collect::<Vec<_>>();
    // Two columns need a few lines each before the geometry is trustworthy.
    if anchors.len() < 4 {
        return Vec::new();
    }
    let content_x0 = anchors
        .iter()
        .map(|b| b.bbox.x0)
        .fold(f64::INFINITY, f64::min);
    let content_x1 = anchors
        .iter()
        .map(|b| b.bbox.x1)
        .fold(f64::NEG_INFINITY, f64::max);
    let content_y0 = anchors
        .iter()
        .map(|b| b.bbox.y0)
        .fold(f64::INFINITY, f64::min);
    let content_y1 = anchors
        .iter()
        .map(|b| b.bbox.y1)
        .fold(f64::NEG_INFINITY, f64::max);
    let content_width = content_x1 - content_x0;
    let content_height = content_y1 - content_y0;
    if !content_width.is_finite() || content_width <= 0.0 || content_height <= 0.0 {
        return Vec::new();
    }

    // Full-width runs straddle columns; excluding them keeps a spanning title
    // from filling the gutter and masking the split.
    let span_limit = content_width * 0.6;
    let mut intervals = anchors
        .iter()
        .filter(|block| block.width() <= span_limit)
        .map(|block| (block.bbox.x0, block.bbox.x1, block.bbox.y0, block.bbox.y1))
        .collect::<Vec<_>>();
    if intervals.len() < 4 {
        return Vec::new();
    }
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));

    // A real gutter is a wide horizontal gap; hairline inter-word gaps must not
    // split a column.
    let gutter_min = (page_width * 0.02).max(6.0);
    let mut runs: Vec<(f64, f64, f64, f64, usize)> = Vec::new();
    for (x0, x1, y0, y1) in intervals.iter().copied() {
        match runs.last_mut() {
            Some(run) if x0 <= run.1 + gutter_min => {
                run.1 = run.1.max(x1);
                run.2 = run.2.min(y0);
                run.3 = run.3.max(y1);
                run.4 += 1;
            }
            _ => runs.push((x0, x1, y0, y1, 1)),
        }
    }

    let min_support = 3.max(intervals.len() / 8);
    let min_column_width = page_width * 0.1;
    let min_column_span = content_height * 0.5;
    let columns = runs
        .into_iter()
        .filter(|run| {
            run.4 >= min_support
                && (run.1 - run.0) >= min_column_width
                && (run.3 - run.2) >= min_column_span
        })
        .map(|run| ColumnRange {
            x0: run.0,
            x1: run.1,
        })
        .collect::<Vec<_>>();
    if columns.len() < 2 {
        return Vec::new();
    }
    columns
}

/// Assigns a block to the column whose horizontal extent contains its reading
/// start (left edge), falling back to the nearest column. Ties resolve to the
/// left-most column so spanning runs lead their band.
fn column_for(block: &PageBlock, columns: &[ColumnRange]) -> usize {
    if columns.is_empty() {
        return 0;
    }
    let start = block.bbox.x0;
    columns
        .iter()
        .enumerate()
        .min_by(|(ai, a), (bi, b)| {
            column_distance(start, a)
                .total_cmp(&column_distance(start, b))
                .then_with(|| ai.cmp(bi))
        })
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn column_distance(x: f64, column: &ColumnRange) -> f64 {
    if x < column.x0 {
        column.x0 - x
    } else if x > column.x1 {
        x - column.x1
    } else {
        0.0
    }
}

fn vertical_overlap(a: &PageBlock, b: &PageBlock) -> f64 {
    (a.bbox.y1.min(b.bbox.y1) - a.bbox.y0.max(b.bbox.y0)).max(0.0)
}

fn merge_inline_blocks(mut blocks: Vec<PageBlock>) -> Vec<PageBlock> {
    blocks.sort_by(|a, b| {
        a.column
            .cmp(&b.column)
            .then_with(|| a.bbox.y0.total_cmp(&b.bbox.y0))
            .then_with(|| a.bbox.x0.total_cmp(&b.bbox.x0))
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut rows: Vec<Vec<PageBlock>> = Vec::new();
    for block in blocks {
        if block.rendered_override.is_some() {
            rows.push(vec![block]);
            continue;
        }
        let candidate = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row[0].rendered_override.is_none() && row[0].column == block.column)
            .filter(|(_, row)| {
                row.iter()
                    .map(|item| vertical_overlap(item, &block))
                    .fold(0.0, f64::max)
                    > row
                        .iter()
                        .map(PageBlock::height)
                        .fold(0.0, f64::max)
                        .min(block.height())
                        * 0.35
            })
            .min_by(|(_, a), (_, b)| {
                let mut ay = a.iter().map(|item| item.bbox.y0).collect::<Vec<_>>();
                let mut by = b.iter().map(|item| item.bbox.y0).collect::<Vec<_>>();
                (median(&mut ay).unwrap() - block.bbox.y0)
                    .abs()
                    .total_cmp(&(median(&mut by).unwrap() - block.bbox.y0).abs())
            })
            .map(|(index, _)| index);
        if let Some(index) = candidate {
            rows[index].push(block);
        } else {
            rows.push(vec![block]);
        }
    }
    rows.into_iter().map(merge_row).collect()
}

fn merge_row(mut row: Vec<PageBlock>) -> PageBlock {
    if row.len() == 1 {
        return row.pop().unwrap();
    }
    row.sort_by(|a, b| {
        a.bbox
            .x0
            .total_cmp(&b.bbox.x0)
            .then_with(|| a.bbox.x1.total_cmp(&b.bbox.x1))
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut merged = row[0].clone();
    merged.id = row
        .iter()
        .map(|item| item.id.as_str())
        .min()
        .unwrap()
        .to_string();
    merged.bbox = BBox {
        x0: row
            .iter()
            .map(|item| item.bbox.x0)
            .fold(f64::INFINITY, f64::min),
        y0: row
            .iter()
            .map(|item| item.bbox.y0)
            .fold(f64::INFINITY, f64::min),
        x1: row
            .iter()
            .map(|item| item.bbox.x1)
            .fold(f64::NEG_INFINITY, f64::max),
        y1: row
            .iter()
            .map(|item| item.bbox.y1)
            .fold(f64::NEG_INFINITY, f64::max),
    };
    merged.content.clear();
    merged.segments.clear();
    merged.size = row.iter().map(|item| item.size).fold(0.0, f64::max);
    merged.bold = row.iter().all(|item| item.bold);
    merged.heading = row.iter().filter_map(|item| item.heading).min();
    for (index, item) in row.iter().enumerate() {
        if index > 0 {
            let previous = &row[index - 1];
            if item.bbox.x0 - previous.bbox.x1 > 1.0_f64.max(item.size * 0.12) {
                merged.content.push(' ');
                merged.segments.push(Segment {
                    text: " ".into(),
                    bold: false,
                    italic: false,
                    href: None,
                });
            }
        }
        merged.content.push_str(&item.content);
        merged.segments.extend(item.segments.clone());
    }
    merged
}

#[derive(Clone)]
enum ListMarker {
    Ordered(usize),
    Bullet,
    Letter(String),
}

fn list_item(text: &str) -> Option<(ListMarker, usize)> {
    let leading = text.len() - text.trim_start().len();
    let trimmed = &text[leading..];
    let marker_end = trimmed.find(char::is_whitespace)?;
    let marker = &trimmed[..marker_end];
    let rest = &trimmed[marker_end..];
    let whitespace = rest.len() - rest.trim_start().len();
    if whitespace == 0 || rest.trim().is_empty() {
        return None;
    }
    let kind = if matches!(
        marker,
        "•" | "‣" | "◦" | "⁃" | "∙" | "▪" | "●" | "·" | "*" | "+" | "-"
    ) {
        ListMarker::Bullet
    } else {
        let core = marker.strip_prefix('(').unwrap_or(marker);
        let core = core.strip_suffix(['.', ')'])?;
        if core.len() == 1 && core.as_bytes()[0].is_ascii_alphabetic() {
            ListMarker::Letter(marker.to_string())
        } else if (1..=3).contains(&core.len()) && core.bytes().all(|byte| byte.is_ascii_digit()) {
            ListMarker::Ordered(core.parse().ok()?)
        } else {
            return None;
        }
    };
    Some((kind, leading + marker_end + whitespace))
}

fn segments_after_byte(segments: &[Segment], mut skip: usize) -> Vec<Segment> {
    let mut result = Vec::new();
    for segment in segments {
        if skip >= segment.text.len() {
            skip -= segment.text.len();
            continue;
        }
        let mut kept = segment.clone();
        kept.text = kept.text[skip..].to_string();
        skip = 0;
        result.push(kept);
    }
    result
}

fn needs_paragraph_break(previous: &PageBlock, current: &PageBlock, body_size: f64) -> bool {
    if previous.rendered_override.is_some()
        || current.rendered_override.is_some()
        || previous.heading.is_some()
        || current.heading.is_some()
        || list_item(&previous.content).is_some()
        || list_item(&current.content).is_some()
    {
        return true;
    }
    let vertical_gap = current.bbox.y0 - previous.bbox.y1;
    vertical_gap > 4.0_f64.max(body_size * 0.65)
        || (current.bbox.x0 - previous.bbox.x0).abs() > 18.0_f64.max(body_size * 1.8)
}

fn render_runs(
    content: &str,
    runs: &[TextRun],
    fallback_href: Option<&str>,
    suppress_bold: bool,
) -> String {
    let font = runs
        .first()
        .map(|run| &run.font)
        .cloned()
        .unwrap_or_else(default_font);
    let mut segments = segments_from_text(content, &font, Some(runs));
    if let Some(href) = fallback_href {
        for segment in &mut segments {
            if segment.href.is_none() {
                segment.href = Some(href.to_string());
            }
        }
    }
    render_segments_styled(&segments, suppress_bold)
}

fn render_segments(segments: &[Segment]) -> String {
    render_segments_styled(segments, false)
}

fn render_segments_styled(segments: &[Segment], suppress_bold: bool) -> String {
    let mut output = String::new();
    let mut pending = Separator::None;
    for segment in segments {
        let (leading, body, trailing) = split_whitespace(&segment.text);
        pending = pending.max(separator(leading));
        if !body.is_empty() {
            push_separator(&mut output, pending);
            pending = Separator::None;
            let mut text = escape_markdown(&normalize_inner_whitespace(body));
            let bold = segment.bold && !suppress_bold;
            if bold && segment.italic {
                text = format!("***{text}***");
            } else if bold {
                text = format!("**{text}**");
            } else if segment.italic {
                text = format!("*{text}*");
            }
            if let Some(href) = &segment.href {
                text = format!("[{text}]({})", escape_link_target(href));
            }
            output.push_str(&text);
        }
        pending = pending.max(separator(trailing));
    }
    output
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Separator {
    None,
    Space,
    Line,
}

fn split_whitespace(text: &str) -> (&str, &str, &str) {
    let start = text
        .find(|ch: char| !ch.is_whitespace())
        .unwrap_or(text.len());
    let end = text
        .rfind(|ch: char| !ch.is_whitespace())
        .map_or(start, |index| {
            index + text[index..].chars().next().unwrap().len_utf8()
        });
    (&text[..start], &text[start..end], &text[end..])
}

fn separator(text: &str) -> Separator {
    if text
        .chars()
        .any(|ch| matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}'))
    {
        Separator::Line
    } else if text.chars().any(char::is_whitespace) {
        Separator::Space
    } else {
        Separator::None
    }
}

fn push_separator(output: &mut String, separator: Separator) {
    if output.is_empty() {
        return;
    }
    match separator {
        Separator::None => {}
        Separator::Space => output.push(' '),
        Separator::Line => output.push_str("<br>\n"),
    }
}

fn normalize_inner_whitespace(text: &str) -> String {
    let mut output = String::new();
    let mut pending = Separator::None;
    for ch in text.chars() {
        if ch.is_whitespace() {
            pending = pending.max(if matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}') {
                Separator::Line
            } else {
                Separator::Space
            });
        } else {
            push_separator(&mut output, pending);
            pending = Separator::None;
            output.push(ch);
        }
    }
    output
}

fn escape_markdown(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '\\' | '*' | '_' | '[' | ']' | '`' | '|' | '#' | '!' => {
                output.push('\\');
                output.push(ch);
            }
            _ if ch.is_control() => output.push(' '),
            _ => output.push(ch),
        }
    }
    output
}

fn escape_link_target(uri: &str) -> String {
    let mut output = String::new();
    for ch in uri.chars() {
        if ch.is_ascii_control() || matches!(ch, ' ' | '\\' | '(' | ')' | '<' | '>') {
            for byte in ch.to_string().bytes() {
                write!(output, "%{byte:02X}").expect("writing to a String cannot fail");
            }
        } else {
            output.push(ch);
        }
    }
    output
}

fn clean_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn finite_or(value: f64, fallback: f64) -> f64 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}

fn heading_role(role: &str) -> Option<usize> {
    let level = role.strip_prefix('h')?.parse::<usize>().ok()?;
    (1..=9).contains(&level).then_some(level)
}

fn render_paged_table(table: &TableElement) -> String {
    let cells = table
        .cells
        .iter()
        .map(|cell| FlowTableCell {
            row: cell.row,
            col: cell.col,
            row_span: cell.row_span,
            col_span: cell.col_span,
            content: cell.content.clone(),
            runs: cell.runs.clone().unwrap_or_default(),
            blocks: None,
        })
        .collect::<Vec<_>>();
    render_flow_table(table.rows, table.cols, &cells)
}

fn render_flow_table(rows: usize, cols: usize, cells: &[FlowTableCell]) -> String {
    if rows == 0 || cols == 0 {
        return String::new();
    }
    let mut grid = vec![vec![String::new(); cols]; rows];
    for cell in cells {
        if cell.row < rows && cell.col < cols {
            grid[cell.row][cell.col] =
                render_runs(&cell.content, &cell.runs, None, false).replace("<br>\n", "<br>");
        }
    }
    let row = |values: &[String]| format!("| {} |", values.join(" | "));
    let mut output = row(&grid[0]);
    output.push('\n');
    output.push_str(&row(&vec!["---".to_string(); cols]));
    for values in grid.iter().skip(1) {
        output.push('\n');
        output.push_str(&row(values));
    }
    output
}

/// Renders a table with merged cells as a raw HTML `<table>` — the only
/// Markdown-embeddable form that can express `colspan`/`rowspan` (GFM/CommonMark
/// pass block-level raw HTML through verbatim). Row 0 is the header (`<th>` in
/// `<thead>`); later rows are `<td>` in `<tbody>`. Every cell string is
/// untrusted PDF text, so all content is HTML-escaped and only integer span
/// attributes reach the tag — hostile cell text can never break out of a cell.
fn render_html_table(rows: usize, cols: usize, cells: &[FlowTableCell]) -> String {
    if rows == 0 || cols == 0 {
        return String::new();
    }
    let mut anchors: BTreeMap<(usize, usize), &FlowTableCell> = BTreeMap::new();
    let mut covered = vec![false; rows * cols];
    for cell in cells {
        if cell.row >= rows || cell.col >= cols {
            continue;
        }
        anchors.insert((cell.row, cell.col), cell);
        let row_end = (cell.row + cell.row_span.max(1)).min(rows);
        let col_end = (cell.col + cell.col_span.max(1)).min(cols);
        for row in cell.row..row_end {
            for col in cell.col..col_end {
                if row != cell.row || col != cell.col {
                    covered[row * cols + col] = true;
                }
            }
        }
    }
    let mut output = String::from("<table>\n<thead>\n");
    render_html_row(&mut output, 0, cols, &anchors, &covered, true);
    output.push_str("</thead>\n");
    if rows > 1 {
        output.push_str("<tbody>\n");
        for row in 1..rows {
            render_html_row(&mut output, row, cols, &anchors, &covered, false);
        }
        output.push_str("</tbody>\n");
    }
    output.push_str("</table>");
    output
}

fn render_html_row(
    output: &mut String,
    row: usize,
    cols: usize,
    anchors: &BTreeMap<(usize, usize), &FlowTableCell>,
    covered: &[bool],
    header: bool,
) {
    let tag = if header { "th" } else { "td" };
    output.push_str("<tr>\n");
    for col in 0..cols {
        if covered[row * cols + col] {
            continue;
        }
        let (attrs, content) = match anchors.get(&(row, col)) {
            Some(cell) => {
                let mut attrs = String::new();
                if cell.col_span > 1 {
                    write!(attrs, " colspan=\"{}\"", cell.col_span)
                        .expect("writing to a String cannot fail");
                }
                if cell.row_span > 1 {
                    write!(attrs, " rowspan=\"{}\"", cell.row_span)
                        .expect("writing to a String cannot fail");
                }
                (attrs, render_html_cell(&cell.content, &cell.runs))
            }
            None => (String::new(), String::new()),
        };
        writeln!(output, "<{tag}{attrs}>{content}</{tag}>")
            .expect("writing to a String cannot fail");
    }
    output.push_str("</tr>\n");
}

fn render_html_cell(content: &str, runs: &[TextRun]) -> String {
    let font = runs
        .first()
        .map(|run| &run.font)
        .cloned()
        .unwrap_or_else(default_font);
    let segments = segments_from_text(content, &font, (!runs.is_empty()).then_some(runs));
    render_html_segments(&segments)
}

/// Renders styled segments as inline HTML for a table cell: bold → `<strong>`,
/// italic → `<em>`, links → `<a href>`, newlines → `<br>`. Markdown syntax is
/// *not* re-emitted here (GFM does not reliably parse Markdown inside a raw-HTML
/// block); text is HTML-escaped instead.
fn render_html_segments(segments: &[Segment]) -> String {
    let mut output = String::new();
    let mut pending = Separator::None;
    for segment in segments {
        let (leading, body, trailing) = split_whitespace(&segment.text);
        pending = pending.max(separator(leading));
        if !body.is_empty() {
            push_html_separator(&mut output, pending);
            pending = Separator::None;
            let mut text = html_escape_collapsing(body);
            if segment.bold && segment.italic {
                text = format!("<strong><em>{text}</em></strong>");
            } else if segment.bold {
                text = format!("<strong>{text}</strong>");
            } else if segment.italic {
                text = format!("<em>{text}</em>");
            }
            if let Some(href) = &segment.href {
                text = format!("<a href=\"{}\">{text}</a>", escape_html_attr(href));
            }
            output.push_str(&text);
        }
        pending = pending.max(separator(trailing));
    }
    output
}

fn push_html_separator(output: &mut String, separator: Separator) {
    if output.is_empty() {
        return;
    }
    match separator {
        Separator::None => {}
        Separator::Space => output.push(' '),
        Separator::Line => output.push_str("<br>"),
    }
}

/// Collapses a segment body's internal whitespace (newlines → `<br>`, other
/// runs → a single space) and HTML-escapes the visible characters. Visible runs
/// pass through `escape_html_text`; the injected `<br>`/space separators are
/// emitted raw so they are never themselves escaped.
fn html_escape_collapsing(text: &str) -> String {
    let mut output = String::new();
    let mut pending = Separator::None;
    let mut run = String::new();
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !run.is_empty() {
                push_html_separator(&mut output, pending);
                pending = Separator::None;
                output.push_str(&escape_html_text(&run));
                run.clear();
            }
            pending = pending.max(if matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}') {
                Separator::Line
            } else {
                Separator::Space
            });
        } else {
            run.push(ch);
        }
    }
    if !run.is_empty() {
        push_html_separator(&mut output, pending);
        output.push_str(&escape_html_text(&run));
    }
    output
}

/// HTML-escapes untrusted text for element content. Distinct from
/// `escape_markdown`: that helper backslash-escapes Markdown metacharacters
/// (`*_[]#|`) which would appear literally inside a raw-HTML block. Here only
/// the HTML-significant characters and control characters are neutralized.
fn escape_html_text(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            _ if ch.is_control() => output.push(' '),
            _ => output.push(ch),
        }
    }
    output
}

/// HTML-escapes untrusted text for a double-quoted attribute value. Adds `"`
/// and `'` to the text set so a hostile value can never close the attribute or
/// inject a new one. Span attributes are integers, but the helper stays honest.
fn escape_html_attr(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            _ if ch.is_control() => output.push(' '),
            _ => output.push(ch),
        }
    }
    output
}

fn write_trailing_context<W: fmt::Write>(
    output: &mut W,
    warnings: &[String],
    hidden: &[&HiddenItem],
) -> fmt::Result {
    let mut wrote = false;
    for warning in warnings {
        if wrote {
            output.write_str("\n\n")?;
        }
        write!(
            output,
            "> [!WARNING]\n> {}",
            escape_markdown(&clean_text(warning))
        )?;
        wrote = true;
    }
    if wrote && !hidden.is_empty() {
        output.write_str("\n\n")?;
    }
    for (index, item) in hidden.iter().enumerate() {
        if index > 0 {
            output.write_char('\n')?;
        }
        let element = item
            .element
            .as_deref()
            .map(|id| format!(" [{id}]"))
            .unwrap_or_default();
        write!(
            output,
            "<!-- docray-hidden: {}{} {} -->",
            safe_comment(&item.kind),
            safe_comment(&element),
            safe_comment(&clean_text(&item.content))
        )?;
    }
    Ok(())
}

fn safe_comment(text: &str) -> String {
    text.replace("--", "- -").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DocMetadata, DocumentInfo, Page, Source, TextColor, TextElement};

    fn text(id: &str, x: f64, y: f64, size: f64, bold: bool, content: &str) -> Element {
        Element::Text(TextElement {
            id: id.into(),
            bbox: BBox {
                x0: x,
                y0: y,
                x1: x + 180.0,
                y1: y + 10.0,
            },
            content: content.into(),
            font: Font {
                name: "Test Sans".into(),
                size,
                bold,
                italic: false,
            },
            color: TextColor {
                fill: Some([0, 0, 0]),
                stroke: None,
            },
            lines: None,
            runs: None,
        })
    }

    fn cell_text(id: &str, x: f64, y: f64, content: &str) -> Element {
        let mut element = text(id, x, y, 12.0, false, content);
        let Element::Text(item) = &mut element else {
            unreachable!()
        };
        item.bbox.x1 = x + 40.0;
        item.bbox.y1 = y + 10.0;
        element
    }

    fn path(id: &str, bbox: BBox) -> Element {
        Element::Path(PathElement {
            id: id.into(),
            bbox,
            fill: None,
            stroke: Some([0, 0, 0]),
            stroke_width: Some(1.0),
        })
    }

    fn ruled_grid(mut text: Vec<Element>) -> Vec<Element> {
        let mut elements = vec![
            path(
                "outer",
                BBox {
                    x0: 10.0,
                    y0: 10.0,
                    x1: 210.0,
                    y1: 90.0,
                },
            ),
            path(
                "row",
                BBox {
                    x0: 10.0,
                    y0: 49.5,
                    x1: 210.0,
                    y1: 50.5,
                },
            ),
            path(
                "column",
                BBox {
                    x0: 109.5,
                    y0: 10.0,
                    x1: 110.5,
                    y1: 90.0,
                },
            ),
        ];
        elements.append(&mut text);
        elements
    }

    fn extraction(elements: Vec<Element>) -> Extraction {
        Extraction {
            schema_version: "1.1".into(),
            source: Source {
                format: "pdf".into(),
                sha256: "unused".into(),
                size_bytes: 0,
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

    #[test]
    fn page_columns_are_left_to_right_not_content_stream_order() {
        let mut elements = Vec::new();
        for row in 0..4 {
            elements.push(text(
                &format!("right-{row}"),
                350.0,
                20.0 + row as f64 * 30.0,
                12.0,
                false,
                &format!("Right column line {row}"),
            ));
            elements.push(text(
                &format!("left-{row}"),
                50.0,
                20.0 + row as f64 * 30.0,
                12.0,
                false,
                &format!("Left column line {row}"),
            ));
        }
        let markdown = extraction(elements).to_markdown();
        let positions = [
            "Left column line 0",
            "Left column line 3",
            "Right column line 0",
            "Right column line 3",
        ]
        .map(|needle| markdown.find(needle).unwrap());
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn ruled_pdf_table_renders_once_as_gfm() {
        let markdown = extraction(ruled_grid(vec![
            cell_text("h1", 20.0, 22.0, "H1"),
            cell_text("h2", 120.0, 22.0, "H2"),
            cell_text("a", 20.0, 62.0, "A | value"),
            cell_text("b", 120.0, 62.0, "B"),
        ]))
        .to_markdown();
        assert_eq!(
            markdown,
            "| H1 | H2 |\n| --- | --- |\n| A \\| value | B |\n"
        );
        assert_eq!(markdown.matches("A \\| value").count(), 1);
    }

    #[test]
    fn ruled_candidate_with_sparse_text_is_left_in_flow_and_warned() {
        let markdown =
            extraction(ruled_grid(vec![cell_text("only", 20.0, 22.0, "Only")])).to_markdown();
        assert!(markdown.starts_with("Only\n\n"));
        assert!(!markdown.contains("| --- |"));
        assert!(markdown.contains(
            "> [!WARNING]\n> page 1: ruled-table candidate skipped: grid has too little cell text"
        ));
    }

    #[test]
    fn heading_inference_lists_links_and_styles_are_exact() {
        let mut linked = match text("link", 50.0, 90.0, 12.0, false, "• linked item") {
            Element::Text(text) => text,
            _ => unreachable!(),
        };
        linked.runs = Some(vec![TextRun {
            content: "• linked item".into(),
            font: Font {
                name: "Test Sans".into(),
                size: 12.0,
                bold: false,
                italic: true,
            },
            color: TextColor {
                fill: Some([0, 0, 0]),
                stroke: None,
            },
            href: None,
        }]);
        let annotation = Element::Annotation(AnnotationElement {
            id: "annotation".into(),
            bbox: linked.bbox,
            subtype: "link".into(),
            uri: Some("https://example.test/a b)".into()),
        });
        let markdown = extraction(vec![
            text("title", 50.0, 10.0, 24.0, false, "Large title"),
            text("body-a", 50.0, 45.0, 12.0, false, "Body line one"),
            text("body-b", 50.0, 57.0, 12.0, false, "body line two"),
            Element::Text(linked),
            annotation,
        ])
        .to_markdown();
        assert_eq!(
            markdown,
            "# Large title\n\nBody line one body line two\n\n- [*linked item*](https://example.test/a%20b%29)\n"
        );
    }

    #[test]
    fn hostile_hidden_content_cannot_close_the_markdown_comment() {
        let mut document = extraction(vec![text("body", 50.0, 10.0, 12.0, false, "safe")]);
        document.pages[0].hidden.push(HiddenItem {
            kind: "comment".into(),
            element: Some("body".into()),
            content: "--><script>alert(1)</script>".into(),
        });
        let markdown = document.to_markdown();
        assert!(!markdown.contains("--><script>"));
        assert!(markdown.contains("- -&gt;<script&gt;alert(1)</script&gt;"));
    }

    #[test]
    fn heading_baseline_is_document_wide_and_letter_markers_are_preserved() {
        let mut document = extraction(vec![text(
            "title",
            50.0,
            10.0,
            15.0,
            false,
            "Document title",
        )]);
        document.document.page_count = 2;
        document.pages.push(Page {
            page_number: 2,
            width: 612.0,
            height: 792.0,
            rotation: 0,
            scanned: false,
            elements: (0..4)
                .map(|row| {
                    text(
                        &format!("body-{row}"),
                        50.0,
                        20.0 + row as f64 * 12.0,
                        12.0,
                        false,
                        "A sufficiently long body line for weighting",
                    )
                })
                .collect(),
            hidden: vec![],
        });
        let markdown = document.to_markdown();
        assert!(markdown.starts_with("# Document title\n\n---\n\n"));

        let (marker, body_start) = list_item("a) alphabetic item").unwrap();
        assert!(matches!(marker, ListMarker::Letter(value) if value == "a)"));
        assert_eq!(&"a) alphabetic item"[body_start..], "alphabetic item");
    }

    #[test]
    fn bold_heading_is_not_double_emphasized() {
        // A fully bold, larger-than-body title must render as `# Title`, never
        // as the redundant `# **Title**`.
        let markdown = extraction(vec![
            text("title", 50.0, 10.0, 24.0, true, "Bold Title"),
            text(
                "body",
                50.0,
                45.0,
                12.0,
                false,
                "Body text long enough to anchor the body-size estimate",
            ),
        ])
        .to_markdown();
        assert!(markdown.contains("# Bold Title"), "{markdown:?}");
        assert!(!markdown.contains("**"), "{markdown:?}");
    }

    #[test]
    fn bold_body_paragraph_is_not_promoted_to_heading() {
        // A body-size bold run that reads like running prose (a long line) is
        // inline emphasis, not a heading.
        let mut elements = (0..4)
            .map(|row| {
                text(
                    &format!("body-{row}"),
                    50.0,
                    20.0 + row as f64 * 14.0,
                    12.0,
                    false,
                    "Regular body sentence used for sizing",
                )
            })
            .collect::<Vec<_>>();
        elements.push(text(
            "bold",
            50.0,
            120.0,
            12.0,
            true,
            "This entire sentence is emphasized in bold but it is clearly ordinary running prose",
        ));
        let markdown = extraction(elements).to_markdown();
        assert!(!markdown.contains('#'), "{markdown:?}");
        assert!(markdown.contains("**This entire sentence"), "{markdown:?}");
    }

    #[test]
    fn indented_lines_are_not_split_into_a_false_column() {
        // Alternating left edges (ragged indentation) must not be read as two
        // columns; reading order stays strictly top-to-bottom.
        let elements = (0..8)
            .map(|row| {
                let indent = if row % 2 == 0 { 50.0 } else { 90.0 };
                text(
                    &format!("line-{row}"),
                    indent,
                    20.0 + row as f64 * 16.0,
                    12.0,
                    false,
                    &format!("Body paragraph line number {row} carried across the page"),
                )
            })
            .collect::<Vec<_>>();
        let markdown = extraction(elements).to_markdown();
        let order = (0..8)
            .map(|row| {
                markdown
                    .find(&format!("line number {row} carried"))
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert!(
            order.windows(2).all(|pair| pair[0] < pair[1]),
            "{markdown:?}"
        );
    }

    #[test]
    fn powerpoint_title_roles_override_font_inference() {
        let mut document = extraction(vec![text(
            "slide-title",
            50.0,
            10.0,
            12.0,
            false,
            "Centered title",
        )]);
        document.source.format = "pptx".into();
        document.pages[0].hidden.push(HiddenItem {
            kind: "role".into(),
            element: Some("slide-title".into()),
            content: "ctrTitle".into(),
        });
        assert!(document.to_markdown().starts_with("# Centered title\n"));
    }

    /// A text element with a fully specified bounding box, for hand-computed
    /// borderless and merged-cell geometry.
    fn sized_text(id: &str, x0: f64, y0: f64, x1: f64, y1: f64, content: &str) -> Element {
        Element::Text(TextElement {
            id: id.into(),
            bbox: BBox { x0, y0, x1, y1 },
            content: content.into(),
            font: Font {
                name: "Test Sans".into(),
                size: 10.0,
                bold: false,
                italic: false,
            },
            color: TextColor {
                fill: Some([0, 0, 0]),
                stroke: None,
            },
            lines: None,
            runs: None,
        })
    }

    fn thin_h(id: &str, x0: f64, x1: f64, y: f64) -> Element {
        path(
            id,
            BBox {
                x0,
                y0: y - 0.5,
                x1,
                y1: y + 0.5,
            },
        )
    }

    fn thin_v(id: &str, x: f64, y0: f64, y1: f64) -> Element {
        path(
            id,
            BBox {
                x0: x - 0.5,
                y0,
                x1: x + 0.5,
                y1,
            },
        )
    }

    #[test]
    fn borderless_alignment_table_renders_as_gfm() {
        // Three tight columns over three rows with clean whitespace gutters is a
        // borderless table; with every span == 1 it stays a GFM pipe table.
        let rows = [
            ["Name", "Qty", "Price"],
            ["Alpha", "2", "$4"],
            ["Beta", "5", "$9"],
        ];
        let mut elements = Vec::new();
        for (r, values) in rows.iter().enumerate() {
            let y = 100.0 + r as f64 * 20.0;
            for (c, value) in values.iter().enumerate() {
                let x = 50.0 + c as f64 * 100.0;
                elements.push(sized_text(
                    &format!("r{r}c{c}"),
                    x,
                    y,
                    x + 40.0,
                    y + 10.0,
                    value,
                ));
            }
        }
        let markdown = extraction(elements).to_markdown();
        assert_eq!(
            markdown,
            "| Name | Qty | Price |\n| --- | --- | --- |\n| Alpha | 2 | $4 |\n| Beta | 5 | $9 |\n"
        );
    }

    #[test]
    fn ruled_merged_header_becomes_html_table_with_colspan() {
        // A 3×3 ruled grid whose two interior verticals are absent over the
        // header band: row 0 is a single cell spanning all three columns, so the
        // table must emit HTML `<table>` (GFM pipe cannot carry colspan).
        let mut elements = vec![
            thin_h("h0", 10.0, 310.0, 10.0),
            thin_h("h1", 10.0, 310.0, 50.0),
            thin_h("h2", 10.0, 310.0, 90.0),
            thin_h("h3", 10.0, 310.0, 130.0),
            thin_v("vL", 10.0, 10.0, 130.0),
            thin_v("vR", 310.0, 10.0, 130.0),
            // interior verticals exist only below the header band (rows 1–2).
            thin_v("v1", 110.0, 50.0, 130.0),
            thin_v("v2", 210.0, 50.0, 130.0),
            sized_text("hdr", 140.0, 25.0, 180.0, 35.0, "Summary"),
        ];
        let data = [["Alpha", "2", "$4"], ["Beta", "5", "$9"]];
        for (r, values) in data.iter().enumerate() {
            let y = 60.0 + r as f64 * 40.0;
            for (c, value) in values.iter().enumerate() {
                let x = 40.0 + c as f64 * 100.0;
                elements.push(sized_text(
                    &format!("d{r}{c}"),
                    x,
                    y,
                    x + 40.0,
                    y + 10.0,
                    value,
                ));
            }
        }
        let markdown = extraction(elements).to_markdown();
        assert_eq!(
            markdown,
            "<table>\n<thead>\n<tr>\n<th colspan=\"3\">Summary</th>\n</tr>\n</thead>\n<tbody>\n\
             <tr>\n<td>Alpha</td>\n<td>2</td>\n<td>$4</td>\n</tr>\n\
             <tr>\n<td>Beta</td>\n<td>5</td>\n<td>$9</td>\n</tr>\n</tbody>\n</table>\n"
        );
    }

    #[test]
    fn borderless_merged_title_becomes_html_table_with_colspan() {
        // A borderless block with a title spanning all columns over enough data
        // rows keeps its gutters stable (the title covers them in only one row),
        // yielding a colspan and therefore an HTML `<table>`.
        let mut elements = vec![sized_text("title", 50.0, 100.0, 290.0, 110.0, "Report")];
        for r in 0..5 {
            let y = 115.0 + r as f64 * 15.0;
            for c in 0..3 {
                let x = 50.0 + c as f64 * 100.0;
                elements.push(sized_text(
                    &format!("r{r}c{c}"),
                    x,
                    y,
                    x + 40.0,
                    y + 10.0,
                    &format!("v{r}{c}"),
                ));
            }
        }
        let markdown = extraction(elements).to_markdown();
        assert!(
            markdown.starts_with("<table>\n<thead>\n<tr>\n<th colspan=\"3\">Report</th>\n"),
            "{markdown:?}"
        );
        assert!(
            markdown.contains("<tbody>\n<tr>\n<td>v00</td>"),
            "{markdown:?}"
        );
        assert!(markdown.trim_end().ends_with("</table>"), "{markdown:?}");
    }

    #[test]
    fn hostile_cell_text_cannot_break_out_of_html_table() {
        // Untrusted cell text must be HTML-escaped; a payload trying to close the
        // cell and inject a script is neutralized, and the span attribute stays
        // an integer.
        let cells = vec![
            FlowTableCell {
                row: 0,
                col: 0,
                row_span: 1,
                col_span: 2,
                content: "</td><script>alert(1)</script>".into(),
                runs: Vec::new(),
                blocks: None,
            },
            FlowTableCell {
                row: 1,
                col: 0,
                row_span: 1,
                col_span: 1,
                content: "\"onmouseover=alert(2)".into(),
                runs: Vec::new(),
                blocks: None,
            },
            FlowTableCell {
                row: 1,
                col: 1,
                row_span: 1,
                col_span: 1,
                content: "safe".into(),
                runs: Vec::new(),
                blocks: None,
            },
        ];
        let html = render_html_table(2, 2, &cells);
        assert!(!html.contains("<script>"), "{html:?}");
        assert!(!html.contains("</td><script>"), "{html:?}");
        assert!(
            html.contains("&lt;/td&gt;&lt;script&gt;alert(1)&lt;/script&gt;"),
            "{html:?}"
        );
        // The `"onmouseover=` payload lands in text position (never an
        // attribute), so it is inert literal text safely enclosed in its cell.
        assert!(html.contains("<td>\"onmouseover=alert(2)</td>"), "{html:?}");
        // Row 0 is one colspan-2 header cell; row 1 has two body cells. Only the
        // renderer's own structural tags appear — none injected from cell text.
        assert_eq!(html.matches("</th>").count(), 1);
        assert_eq!(html.matches("</td>").count(), 2);
    }

    #[test]
    fn html_escaping_helpers_neutralize_metacharacters() {
        assert_eq!(escape_html_text("a<b>&c"), "a&lt;b&gt;&amp;c");
        assert_eq!(escape_html_attr("x\"y'z<&>"), "x&quot;y&#39;z&lt;&amp;&gt;");
    }

    #[test]
    fn borderless_bold_cell_renders_strong_in_html() {
        let mut bold = Segment {
            text: "Bad<x>".into(),
            bold: true,
            italic: false,
            href: None,
        };
        bold.href = None;
        assert_eq!(
            render_html_segments(std::slice::from_ref(&bold)),
            "<strong>Bad&lt;x&gt;</strong>"
        );
    }

    #[test]
    fn prose_is_not_misclassified_as_borderless_table() {
        // Full-width running lines have a single column; nothing is emitted as a
        // table.
        let mut elements = Vec::new();
        for r in 0..5 {
            let y = 100.0 + r as f64 * 14.0;
            elements.push(sized_text(
                &format!("line{r}"),
                50.0,
                y,
                500.0,
                y + 10.0,
                "A full width running prose line that spans the whole content column",
            ));
        }
        let markdown = extraction(elements).to_markdown();
        assert!(!markdown.contains("<table"), "{markdown:?}");
        assert!(!markdown.contains("| --- |"), "{markdown:?}");
    }

    #[test]
    fn two_column_key_value_form_is_not_a_table() {
        // A key/value form has only two columns; the ≥3-column gate rejects it so
        // forms stay reading-order text.
        let pairs = [
            ("Name:", "John Doe"),
            ("Email:", "j@example.test"),
            ("Phone:", "555-0100"),
            ("City:", "Springfield"),
        ];
        let mut elements = Vec::new();
        for (r, (key, value)) in pairs.iter().enumerate() {
            let y = 100.0 + r as f64 * 16.0;
            elements.push(sized_text(&format!("k{r}"), 50.0, y, 90.0, y + 10.0, key));
            elements.push(sized_text(
                &format!("v{r}"),
                150.0,
                y,
                260.0,
                y + 10.0,
                value,
            ));
        }
        let markdown = extraction(elements).to_markdown();
        assert!(!markdown.contains("<table"), "{markdown:?}");
        assert!(!markdown.contains("| --- |"), "{markdown:?}");
    }

    #[test]
    fn ragged_code_columns_are_rejected_by_edge_tightness() {
        // Three token groups per line but with jittered left/right edges: the
        // column-edge stdev exceeds the bucket width, so code is left as text.
        let jitter = [0.0, 20.0, 6.0, 14.0];
        let mut elements = Vec::new();
        for (r, shift) in jitter.iter().enumerate() {
            let y = 100.0 + r as f64 * 14.0;
            for c in 0..3 {
                let x = 50.0 + c as f64 * 90.0 + shift;
                elements.push(sized_text(
                    &format!("t{r}{c}"),
                    x,
                    y,
                    x + 30.0,
                    y + 10.0,
                    "tok",
                ));
            }
        }
        let markdown = extraction(elements).to_markdown();
        assert!(!markdown.contains("<table"), "{markdown:?}");
        assert!(!markdown.contains("| --- |"), "{markdown:?}");
    }
}
