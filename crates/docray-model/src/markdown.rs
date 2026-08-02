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
                    let rendered = render_runs(content, runs, None);
                    let line = if let Some(list) = list {
                        let indent = "  ".repeat(list.level as usize);
                        let marker = match list.kind {
                            ListKind::Ordered => "1.",
                            ListKind::Bullet => "-",
                        };
                        format!("{indent}{marker} {rendered}")
                    } else if let Some(level) = heading_role(role) {
                        format!("{} {rendered}", "#".repeat(level.min(6)))
                    } else if role == "title" {
                        format!("# {rendered}")
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
        } else {
            let content = render_segments(&block.segments);
            if let Some(level) = block.heading {
                format!("{} {content}", "#".repeat(level.min(6)))
            } else {
                content
            }
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
    let detection = if source_format == "pdf" {
        detect_ruled_tables(page)
    } else {
        TableDetection::default()
    };
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
                row_span: 1,
                col_span: 1,
                content: cell.content.clone(),
                runs: Vec::new(),
                blocks: None,
            })
            .collect::<Vec<_>>();
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
            rendered_override: Some(render_flow_table(table.rows, table.cols, &cells)),
        });
    }
    associate_annotations(&mut blocks, &annotations);
    PageMarkdown {
        blocks,
        warnings: detection.warnings,
    }
}

/// Detects only high-confidence ruled PDF tables for the Markdown projection.
/// Phase 2 = borderless/alignment detection; structured TableElement-in-JSON
/// behind a granularity parameter is a follow-up.
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

    let mut cells = Vec::with_capacity(cell_count);
    let mut consumed_text = Vec::new();
    for row in 0..rows {
        for col in 0..cols {
            let items = &mut assigned[row * cols + col];
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
            block.rendered_override.is_none()
                && block.heading.is_none()
                && (block.size >= threshold || (block.bold && block.size >= body_size * 0.98))
        })
        .map(|block| block.size)
        .collect::<Vec<_>>();
    let ranks = cluster_sizes(candidate_sizes, body_size);
    for block in blocks.iter_mut().filter(|block| block.heading.is_none()) {
        if block.rendered_override.is_some()
            || !(block.size >= threshold || (block.bold && block.size >= body_size * 0.98))
        {
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
    let threshold = (page_width * 0.055).clamp(18.0, 42.0);
    let mut anchors = blocks
        .iter()
        .enumerate()
        .filter(|(_, block)| {
            clean_text(&block.content).chars().count() >= 20 || block.width() >= page_width * 0.16
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if anchors.is_empty() {
        anchors.extend(0..blocks.len());
    }
    anchors.sort_by(|a, b| {
        blocks[*a]
            .bbox
            .x0
            .total_cmp(&blocks[*b].bbox.x0)
            .then_with(|| blocks[*a].bbox.y0.total_cmp(&blocks[*b].bbox.y0))
            .then_with(|| blocks[*a].id.cmp(&blocks[*b].id))
    });
    let mut raw: Vec<Vec<usize>> = Vec::new();
    for index in anchors.iter().copied() {
        let new_cluster = raw.last().is_none_or(|cluster| {
            blocks[index].bbox.x0 - blocks[*cluster.last().unwrap()].bbox.x0 > threshold
        });
        if new_cluster {
            raw.push(vec![index]);
        } else {
            raw.last_mut().unwrap().push(index);
        }
    }
    let minimum_support = 4_usize.max((anchors.len() * 8).div_ceil(100));
    let mut major = raw
        .iter()
        .filter(|cluster| cluster.len() >= minimum_support)
        .cloned()
        .collect::<Vec<_>>();
    if major.is_empty() {
        major.push(
            raw.into_iter()
                .max_by(|a, b| {
                    a.len()
                        .cmp(&b.len())
                        .then_with(|| blocks[b[0]].bbox.x0.total_cmp(&blocks[a[0]].bbox.x0))
                })
                .unwrap(),
        );
    }
    let centers = major
        .iter()
        .map(|cluster| {
            let mut starts = cluster
                .iter()
                .map(|index| blocks[*index].bbox.x0)
                .collect::<Vec<_>>();
            median(&mut starts).unwrap()
        })
        .collect::<Vec<_>>();
    let anchor_columns = major
        .iter()
        .enumerate()
        .flat_map(|(column, cluster)| cluster.iter().map(move |index| (*index, column)))
        .collect::<BTreeMap<_, _>>();
    for index in 0..blocks.len() {
        if let Some(column) = anchor_columns.get(&index) {
            blocks[index].column = *column;
            continue;
        }
        let same_line = anchors.iter().copied().filter(|anchor| {
            vertical_overlap(&blocks[index], &blocks[*anchor])
                > blocks[index].height().min(blocks[*anchor].height()) * 0.35
        });
        let nearest_anchor = same_line.min_by(|a, b| {
            horizontal_distance(&blocks[index], &blocks[*a])
                .total_cmp(&horizontal_distance(&blocks[index], &blocks[*b]))
                .then_with(|| {
                    (blocks[index].bbox.x0 - blocks[*a].bbox.x0)
                        .abs()
                        .total_cmp(&(blocks[index].bbox.x0 - blocks[*b].bbox.x0).abs())
                })
                .then_with(|| blocks[*a].id.cmp(&blocks[*b].id))
        });
        let x = nearest_anchor.map_or(blocks[index].bbox.x0, |anchor| blocks[anchor].bbox.x0);
        blocks[index].column = centers
            .iter()
            .enumerate()
            .min_by(|(ai, a), (bi, b)| (x - **a).abs().total_cmp(&(x - **b).abs()).then(ai.cmp(bi)))
            .map(|(column, _)| column)
            .unwrap_or(0);
    }
    let mut order = (0..centers.len()).collect::<Vec<_>>();
    order.sort_by(|a, b| centers[*a].total_cmp(&centers[*b]).then(a.cmp(b)));
    let remap = order
        .iter()
        .enumerate()
        .map(|(new, old)| (*old, new))
        .collect::<BTreeMap<_, _>>();
    for block in blocks {
        block.column = remap[&block.column];
    }
}

fn vertical_overlap(a: &PageBlock, b: &PageBlock) -> f64 {
    (a.bbox.y1.min(b.bbox.y1) - a.bbox.y0.max(b.bbox.y0)).max(0.0)
}

fn horizontal_distance(a: &PageBlock, b: &PageBlock) -> f64 {
    let center = (a.bbox.x0 + a.bbox.x1) / 2.0;
    if b.bbox.x0 <= center && center <= b.bbox.x1 {
        0.0
    } else {
        (a.bbox.x0 - b.bbox.x1)
            .abs()
            .min((a.bbox.x1 - b.bbox.x0).abs())
    }
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

fn render_runs(content: &str, runs: &[TextRun], fallback_href: Option<&str>) -> String {
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
    render_segments(&segments)
}

fn render_segments(segments: &[Segment]) -> String {
    let mut output = String::new();
    let mut pending = Separator::None;
    for segment in segments {
        let (leading, body, trailing) = split_whitespace(&segment.text);
        pending = pending.max(separator(leading));
        if !body.is_empty() {
            push_separator(&mut output, pending);
            pending = Separator::None;
            let mut text = escape_markdown(&normalize_inner_whitespace(body));
            if segment.bold && segment.italic {
                text = format!("***{text}***");
            } else if segment.bold {
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
                render_runs(&cell.content, &cell.runs, None).replace("<br>\n", "<br>");
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
}
