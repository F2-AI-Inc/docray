use super::{
    AnnotationElement, BBox, Block, BreakKind, Element, Extraction, FlowExtraction, FlowTableCell,
    Font, HiddenItem, ListKind, Page, TableElement, TextRun,
};
use std::collections::BTreeMap;
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
            weighted_median_font(page_blocks.iter().flat_map(|(_, blocks)| blocks.iter()));
        let pages = page_blocks
            .into_iter()
            .map(|(page_width, blocks)| render_page(blocks, page_width, body_size))
            .filter(|page| !page.is_empty())
            .collect::<Vec<_>>();
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
        let has_context = !self.warnings.is_empty() || !hidden.is_empty();
        if !pages.is_empty() && has_context {
            output.write_str("\n\n")?;
        }
        write_trailing_context(output, &self.warnings, &hidden)?;
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

fn blocks_for_page(page: &Page, source_format: &str) -> Vec<PageBlock> {
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
    let mut blocks = Vec::new();
    let mut annotations = Vec::new();
    for element in &page.elements {
        match element {
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
                // PDF table structure is intentionally deferred to #63. Until then,
                // emit only cell text in geometric reading order.
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
    associate_annotations(&mut blocks, &annotations);
    blocks
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
