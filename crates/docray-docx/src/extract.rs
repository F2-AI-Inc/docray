use crate::numbering::{Counters, Numbering, ResolvedList};
use crate::styles::{ParagraphStyle, Styles};
use docray_core::{Capabilities, ExtractError, Extractor, GeometryKind};
use docray_model::{
    round3, Block, BreakKind, DocMetadata, FlowDocumentInfo, FlowExtraction, FlowLayout,
    FlowTableCell, Granularity, HiddenItem, Margins, Placement, PlacementFrame, Section, Source,
    Story, TextRun,
};
use docray_ooxml::{
    parse, preprocess_alternate_content, relationships, resolve_target, Node, Package,
    Relationships, Theme, EMU_PER_POINT, TWIPS_PER_POINT,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const CFB_MAGIC: &[u8; 8] = b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1";
const MAX_MC_DEPTH: usize = 32;
const MAX_FIELD_DEPTH: usize = 16;
const MAX_TEXTBOX_DEPTH: usize = 8;
const MAX_BLOCKS_PER_STORY: usize = 250_000;
const MAX_RUNS_PER_PARAGRAPH: usize = 100;

const SUPPORTED_MC_NAMESPACES: &[&str] = &[
    "http://schemas.microsoft.com/office/word/2010/wordprocessingShape",
    "http://schemas.microsoft.com/office/word/2010/wordprocessingGroup",
    "http://schemas.microsoft.com/office/word/2010/wordml",
    "http://schemas.microsoft.com/office/word/2012/wordml",
    "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
    "urn:schemas-microsoft-com:vml",
];

pub struct DocxExtractor;

impl Extractor for DocxExtractor {
    type Output = FlowExtraction;

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            finest_granularity: Granularity::Element,
            geometry: GeometryKind::Flow,
        }
    }

    fn extract(
        &self,
        bytes: &[u8],
        max_pages: Option<u32>,
    ) -> Result<FlowExtraction, ExtractError> {
        if bytes.starts_with(CFB_MAGIC) {
            return Err(ExtractError::UnsupportedFormatMessage(
                "legacy or encrypted Office documents are not supported".into(),
            ));
        }
        let mut package = Package::open(bytes)?;
        if !package.contains("word/document.xml") {
            return Err(ExtractError::UnsupportedFormatMessage(
                "zip archive is not a Word document".into(),
            ));
        }

        let mut warnings = Vec::new();
        let format = source_format(&mut package)?;
        if format == "docm" {
            warnings.push("macro project ignored".into());
        }
        let metadata = metadata(&mut package)?;
        let document = xml_required_mc(&mut package, "word/document.xml", &mut warnings)?;
        let styles_root = xml_optional_mc(&mut package, "word/styles.xml", &mut warnings)?;
        let numbering_root = xml_optional_mc(&mut package, "word/numbering.xml", &mut warnings)?;
        let theme_root = xml_optional_mc(&mut package, "word/theme/theme1.xml", &mut warnings)?;
        let comments = load_comments(&mut package, &mut warnings)?;
        let notes = load_notes(&mut package, &mut warnings)?;
        let document_rels = relationships(&mut package, "word/document.xml")?;

        let styles = Styles::from_xml(styles_root.as_ref());
        let numbering = Numbering::from_xml(numbering_root.as_ref());
        let theme = theme_root.as_ref().map(Theme::from_xml).unwrap_or_default();
        let mut context = Context {
            package: &mut package,
            styles,
            numbering,
            theme,
            comments,
            notes,
            warnings,
        };
        let body = document
            .first_descendant("body")
            .ok_or_else(|| parse_failure("word/document.xml has no w:body"))?;
        let mut sections = extract_body(body, &document_rels, &mut context)?;
        attach_section_stories(&mut sections, &document_rels, &mut context)?;
        attach_notes(&mut sections, &document_rels, &mut context)?;

        let mut page_hints = 0_u32;
        let mut max_approx_page = 1_u32;
        for section in &sections {
            visit_blocks(&section.blocks, &mut |block| {
                if let Block::Paragraph { approx_page, .. } = block {
                    if let Some(page) = approx_page {
                        page_hints = page_hints.max(page.saturating_sub(1));
                        max_approx_page = max_approx_page.max(*page);
                    }
                }
            });
        }
        let approx_pages = if page_hints == 0 {
            for section in &mut sections {
                clear_approx_pages(&mut section.blocks);
            }
            context
                .warnings
                .push("no pagination hints; approx_page omitted".into());
            None
        } else {
            Some(max_approx_page)
        };

        enforce_max_pages(max_pages, approx_pages, &sections, &mut context.warnings)?;

        Ok(FlowExtraction {
            schema_version: "1.7".into(),
            layout: FlowLayout::Flow,
            source: Source {
                format: format.into(),
                sha256: hex(&Sha256::digest(bytes)),
                size_bytes: bytes.len() as u64,
            },
            document: FlowDocumentInfo { metadata },
            warnings: context.warnings,
            approx_pages,
            sections,
        })
    }
}

struct Context<'a, 'bytes> {
    package: &'a mut Package<'bytes>,
    styles: Styles,
    numbering: Numbering,
    theme: Theme,
    comments: BTreeMap<String, String>,
    notes: BTreeMap<(NoteKind, String), Node>,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum NoteKind {
    Footnote,
    Endnote,
}

#[derive(Clone)]
struct NoteReference {
    kind: NoteKind,
    id: String,
    block_id: String,
}

#[derive(Default)]
struct StoryState {
    fields: FieldMachine,
    counters: Counters,
    pending_breaks: Vec<BreakKind>,
    current_page: u32,
    block_count: usize,
}

impl StoryState {
    fn body() -> Self {
        Self {
            current_page: 1,
            ..Self::default()
        }
    }
}

struct IdAllocator {
    prefix: String,
    next: usize,
}

impl IdAllocator {
    fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            next: 0,
        }
    }

    fn peek(&self) -> String {
        format!("{}{}", self.prefix, self.next)
    }

    fn next(&mut self) -> String {
        let id = self.peek();
        self.next += 1;
        id
    }
}

fn extract_body(
    body: &Node,
    rels: &Relationships,
    context: &mut Context<'_, '_>,
) -> Result<Vec<Section>, ExtractError> {
    let mut drafts = Vec::new();
    let mut section_index = 0_usize;
    let mut blocks = Vec::new();
    let mut hidden = Vec::new();
    let mut refs = Vec::new();
    let mut state = StoryState::body();
    let mut ids = IdAllocator::new("s0-b");
    let mut final_sect_pr = None;

    for child in &body.children {
        match child.local_name() {
            "p" => {
                let sect_pr = child.child("pPr").and_then(|node| node.child("sectPr"));
                let mut produced = extract_paragraph(
                    child,
                    rels,
                    &mut ids,
                    &mut state,
                    &mut hidden,
                    &mut refs,
                    context,
                    0,
                    true,
                )?;
                push_blocks(&mut blocks, &mut produced, &mut state, context)?;
                if let Some(sect_pr) = sect_pr {
                    blocks.push(Block::Break {
                        kind: BreakKind::Section,
                    });
                    drafts.push(make_section(blocks, hidden, refs, Some(sect_pr.clone())));
                    section_index += 1;
                    blocks = Vec::new();
                    hidden = Vec::new();
                    refs = Vec::new();
                    ids = IdAllocator::new(format!("s{section_index}-b"));
                }
            }
            "tbl" => {
                let block =
                    extract_table(child, rels, &mut ids, &mut hidden, &mut refs, context, 0)?;
                let mut produced = vec![block];
                push_blocks(&mut blocks, &mut produced, &mut state, context)?;
            }
            "sectPr" => final_sect_pr = Some(child.clone()),
            "altChunk" | "object" | "oleObject" => context.warnings.push(format!(
                "{} content is not extracted; fallback text unavailable",
                child.local_name()
            )),
            _ => {}
        }
    }
    finish_story(&mut blocks, &mut state, context);
    drafts.push(make_section(blocks, hidden, refs, final_sect_pr));
    Ok(drafts)
}

fn make_section(
    blocks: Vec<Block>,
    hidden: Vec<HiddenItem>,
    note_refs: Vec<NoteReference>,
    sect_pr: Option<Node>,
) -> Section {
    let (page_width, page_height, margins, columns) = section_geometry(sect_pr.as_ref());
    let mut section = Section {
        page_width,
        page_height,
        margins,
        columns,
        headers: Vec::new(),
        footers: Vec::new(),
        blocks,
        hidden,
    };
    // Keep extraction-only data in deterministic hidden sentinels until the
    // post-pass attaches stories and notes; these are removed before output.
    if let Some(sect_pr) = &sect_pr {
        section.hidden.push(HiddenItem {
            kind: "__sect_pr".into(),
            element: None,
            content: serialize_refs(sect_pr),
        });
    }
    for reference in &note_refs {
        section.hidden.push(HiddenItem {
            kind: "__note_ref".into(),
            element: Some(reference.block_id.clone()),
            content: format!(
                "{}:{}",
                match reference.kind {
                    NoteKind::Footnote => "footnote",
                    NoteKind::Endnote => "endnote",
                },
                reference.id
            ),
        });
    }
    section
}

// SectionDraft is intentionally collapsed above to keep the public model free
// of extractor state. The two tiny sentinel forms are decoded and removed by
// the post-passes below.
fn serialize_refs(sect_pr: &Node) -> String {
    sect_pr
        .children
        .iter()
        .filter(|node| matches!(node.local_name(), "headerReference" | "footerReference"))
        .filter_map(|node| {
            Some(format!(
                "{}|{}|{}",
                node.local_name(),
                node.attr("type").unwrap_or("default"),
                node.attr("id")?
            ))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn section_geometry(sect_pr: Option<&Node>) -> (f64, f64, Margins, Option<u32>) {
    let size = sect_pr.and_then(|node| node.child("pgSz"));
    let margin = sect_pr.and_then(|node| node.child("pgMar"));
    let width = twips(size.and_then(|node| node.attr("w"))).unwrap_or(612.0);
    let height = twips(size.and_then(|node| node.attr("h"))).unwrap_or(792.0);
    let margins = Margins {
        top: twips(margin.and_then(|node| node.attr("top"))).unwrap_or(72.0),
        right: twips(margin.and_then(|node| node.attr("right"))).unwrap_or(72.0),
        bottom: twips(margin.and_then(|node| node.attr("bottom"))).unwrap_or(72.0),
        left: twips(margin.and_then(|node| node.attr("left"))).unwrap_or(72.0),
    };
    let columns = sect_pr
        .and_then(|node| node.child("cols"))
        .and_then(|node| node.attr("num"))
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 1);
    (width, height, margins, columns)
}

fn push_blocks(
    destination: &mut Vec<Block>,
    source: &mut Vec<Block>,
    state: &mut StoryState,
    context: &mut Context<'_, '_>,
) -> Result<(), ExtractError> {
    for mut block in source.drain(..) {
        if !state.pending_breaks.is_empty() {
            if let Block::Paragraph { breaks_before, .. } = &mut block {
                breaks_before.append(&mut state.pending_breaks);
            } else {
                destination.extend(
                    state
                        .pending_breaks
                        .drain(..)
                        .map(|kind| Block::Break { kind }),
                );
            }
        }
        state.block_count += count_blocks(&block);
        if state.block_count > MAX_BLOCKS_PER_STORY {
            context.warnings.push(format!(
                "story block limit {MAX_BLOCKS_PER_STORY} exceeded; remaining content skipped"
            ));
            return Ok(());
        }
        destination.push(block);
    }
    Ok(())
}

fn finish_story(blocks: &mut Vec<Block>, state: &mut StoryState, context: &mut Context<'_, '_>) {
    blocks.extend(
        state
            .pending_breaks
            .drain(..)
            .map(|kind| Block::Break { kind }),
    );
    state.fields.finish(&mut context.warnings);
}

#[derive(Clone)]
enum InlineEvent {
    Text {
        content: String,
        properties: Option<Node>,
        href: Option<String>,
    },
    FieldBegin,
    FieldSeparate,
    FieldEnd,
    Instruction(String),
    Hidden {
        kind: &'static str,
        content: String,
    },
    Comment(String),
    Note(NoteKind, String),
    Break(BreakKind),
    PageHint,
    Drawing(Node),
}

#[derive(Default)]
struct FieldMachine {
    stack: Vec<FieldFrame>,
    overflow: usize,
}

struct FieldFrame {
    instruction: String,
    result: bool,
}

impl FieldMachine {
    fn begin(&mut self, warnings: &mut Vec<String>) {
        if self.stack.len() >= MAX_FIELD_DEPTH {
            self.overflow += 1;
            warnings.push(format!(
                "field nesting depth limit {MAX_FIELD_DEPTH} exceeded; nested field suppressed"
            ));
            return;
        }
        self.stack.push(FieldFrame {
            instruction: String::new(),
            result: false,
        });
    }

    fn instruction(
        &mut self,
        value: &str,
        target: &str,
        hidden: &mut Vec<HiddenItem>,
        warnings: &mut Vec<String>,
    ) {
        if self.overflow > 0 {
            return;
        }
        if let Some(field) = self.stack.last_mut() {
            field.instruction.push_str(value);
            upsert_field_hidden(target, &field.instruction, hidden);
        } else {
            warnings.push("field instruction appeared outside a field; instruction hidden".into());
            push_field_hidden(target, value, hidden);
        }
    }

    fn separate(&mut self, target: &str, hidden: &mut Vec<HiddenItem>, warnings: &mut Vec<String>) {
        if self.overflow > 0 {
            return;
        }
        let Some(field) = self.stack.last_mut() else {
            warnings.push("field separator appeared without a matching begin".into());
            return;
        };
        field.result = true;
        push_field_hidden(target, &field.instruction, hidden);
    }

    fn end(&mut self, target: &str, hidden: &mut Vec<HiddenItem>, warnings: &mut Vec<String>) {
        if self.overflow > 0 {
            self.overflow -= 1;
            return;
        }
        let Some(field) = self.stack.pop() else {
            warnings.push("field end appeared without a matching begin".into());
            return;
        };
        push_field_hidden(target, &field.instruction, hidden);
        if !field.result {
            warnings.push("field ended without a result separator; instruction hidden".into());
        }
    }

    fn visible(&self) -> bool {
        self.overflow == 0 && self.stack.iter().all(|field| field.result)
    }

    fn finish(&mut self, warnings: &mut Vec<String>) {
        if !self.stack.is_empty() || self.overflow != 0 {
            warnings.push(format!(
                "story ended with {} unclosed field level(s); instructions hidden",
                self.stack.len() + self.overflow
            ));
        }
        self.stack.clear();
        self.overflow = 0;
    }
}

fn push_field_hidden(target: &str, instruction: &str, hidden: &mut Vec<HiddenItem>) {
    let instruction = instruction.trim();
    if instruction.is_empty()
        || hidden.iter().any(|item| {
            item.kind == "field"
                && item.element.as_deref() == Some(target)
                && item.content == instruction
        })
    {
        return;
    }
    hidden.push(HiddenItem {
        kind: "field".into(),
        element: Some(target.to_string()),
        content: instruction.to_string(),
    });
}

fn upsert_field_hidden(target: &str, instruction: &str, hidden: &mut Vec<HiddenItem>) {
    let instruction = instruction.trim();
    if instruction.is_empty() {
        return;
    }
    if let Some(item) = hidden
        .iter_mut()
        .rev()
        .find(|item| item.kind == "field" && item.element.as_deref() == Some(target))
    {
        item.content = instruction.to_string();
    } else {
        hidden.push(HiddenItem {
            kind: "field".into(),
            element: Some(target.to_string()),
            content: instruction.to_string(),
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn extract_paragraph(
    paragraph: &Node,
    rels: &Relationships,
    ids: &mut IdAllocator,
    state: &mut StoryState,
    hidden: &mut Vec<HiddenItem>,
    note_refs: &mut Vec<NoteReference>,
    context: &mut Context<'_, '_>,
    textbox_depth: usize,
    track_pages: bool,
) -> Result<Vec<Block>, ExtractError> {
    let p_pr = paragraph.child("pPr");
    let style = context.styles.paragraph(p_pr, &mut context.warnings);
    let list = style.num_id.as_deref().and_then(|num_id| {
        context.numbering.next(
            &mut state.counters,
            num_id,
            style.num_level,
            &mut context.warnings,
        )
    });
    let mut events = Vec::new();
    for child in &paragraph.children {
        if child.local_name() != "pPr" {
            collect_inline_events(child, rels, None, &mut events, &mut context.warnings);
        }
    }

    let mut blocks = Vec::new();
    let mut runs = Vec::new();
    let mut first_segment = true;
    let mut emitted_any = false;
    if style.page_break_before {
        state.pending_breaks.push(BreakKind::Page);
    }

    for event in events {
        let target = ids.peek();
        match event {
            InlineEvent::Text {
                content,
                properties,
                href,
            } => {
                if !state.fields.visible() || content.is_empty() {
                    continue;
                }
                let (font, color) = context.styles.resolve_run(
                    &style,
                    list.as_ref().and_then(|list| list.run.as_ref()),
                    properties.as_ref(),
                    &context.theme,
                    &mut context.warnings,
                );
                merge_run(
                    &mut runs,
                    TextRun {
                        content,
                        font,
                        color,
                        href,
                    },
                );
            }
            InlineEvent::FieldBegin => state.fields.begin(&mut context.warnings),
            InlineEvent::FieldSeparate => {
                state
                    .fields
                    .separate(&target, hidden, &mut context.warnings)
            }
            InlineEvent::FieldEnd => state.fields.end(&target, hidden, &mut context.warnings),
            InlineEvent::Instruction(content) => {
                state
                    .fields
                    .instruction(&content, &target, hidden, &mut context.warnings)
            }
            InlineEvent::Hidden { kind, content } => hidden.push(HiddenItem {
                kind: kind.into(),
                element: Some(target),
                content,
            }),
            InlineEvent::Comment(id) => match context.comments.get(&id) {
                Some(content) => {
                    if !hidden.iter().any(|item| {
                        item.kind == "comment"
                            && item.element.as_deref() == Some(target.as_str())
                            && item.content == *content
                    }) {
                        hidden.push(HiddenItem {
                            kind: "comment".into(),
                            element: Some(target),
                            content: content.clone(),
                        });
                    }
                }
                None => context
                    .warnings
                    .push(format!("comment {id:?} is referenced but missing")),
            },
            InlineEvent::Note(kind, id) => note_refs.push(NoteReference {
                kind,
                id,
                block_id: target,
            }),
            InlineEvent::Break(kind) => {
                flush_paragraph_segment(
                    &mut blocks,
                    &mut runs,
                    &style,
                    &list,
                    &mut first_segment,
                    ids,
                    state,
                    track_pages,
                    &mut emitted_any,
                    context,
                );
                state.pending_breaks.push(kind);
            }
            InlineEvent::PageHint => {
                flush_paragraph_segment(
                    &mut blocks,
                    &mut runs,
                    &style,
                    &list,
                    &mut first_segment,
                    ids,
                    state,
                    track_pages,
                    &mut emitted_any,
                    context,
                );
                if track_pages {
                    state.current_page = state.current_page.saturating_add(1);
                }
            }
            InlineEvent::Drawing(drawing) => {
                if !runs.is_empty() {
                    flush_paragraph_segment(
                        &mut blocks,
                        &mut runs,
                        &style,
                        &list,
                        &mut first_segment,
                        ids,
                        state,
                        track_pages,
                        &mut emitted_any,
                        context,
                    );
                }
                if let Some(block) = extract_drawing(
                    &drawing,
                    rels,
                    ids,
                    hidden,
                    note_refs,
                    context,
                    textbox_depth,
                )? {
                    blocks.push(block);
                    emitted_any = true;
                }
            }
        }
    }

    if !runs.is_empty() || !emitted_any {
        flush_paragraph_segment(
            &mut blocks,
            &mut runs,
            &style,
            &list,
            &mut first_segment,
            ids,
            state,
            track_pages,
            &mut emitted_any,
            context,
        );
    }
    Ok(blocks)
}

#[allow(clippy::too_many_arguments)]
fn flush_paragraph_segment(
    blocks: &mut Vec<Block>,
    runs: &mut Vec<TextRun>,
    style: &ParagraphStyle,
    list: &Option<ResolvedList>,
    first_segment: &mut bool,
    ids: &mut IdAllocator,
    state: &mut StoryState,
    track_pages: bool,
    emitted_any: &mut bool,
    context: &mut Context<'_, '_>,
) {
    if runs.is_empty() && *emitted_any {
        return;
    }
    enforce_run_soft_cap(runs, &mut context.warnings);
    let content = runs.iter().map(|run| run.content.as_str()).collect();
    let mut breaks_before = Vec::new();
    breaks_before.append(&mut state.pending_breaks);
    blocks.push(Block::Paragraph {
        id: ids.next(),
        role: style.role.clone(),
        runs: std::mem::take(runs),
        content,
        list: (*first_segment)
            .then(|| list.as_ref().map(|list| list.info.clone()))
            .flatten(),
        placement: None,
        approx_page: track_pages.then_some(state.current_page.max(1)),
        breaks_before,
    });
    *first_segment = false;
    *emitted_any = true;
}

fn collect_inline_events(
    node: &Node,
    rels: &Relationships,
    href: Option<String>,
    events: &mut Vec<InlineEvent>,
    warnings: &mut Vec<String>,
) {
    match node.local_name() {
        "ins" | "moveTo" => {
            let content = visible_descendant_text(node);
            if !content.is_empty() {
                events.push(InlineEvent::Hidden {
                    kind: "tracked-insert",
                    content,
                });
            }
            for child in &node.children {
                collect_inline_events(child, rels, href.clone(), events, warnings);
            }
        }
        "del" | "moveFrom" => {
            let content = visible_descendant_text(node);
            if !content.is_empty() {
                events.push(InlineEvent::Hidden {
                    kind: "tracked-delete",
                    content,
                });
            }
        }
        "fldSimple" => {
            events.push(InlineEvent::FieldBegin);
            if let Some(instruction) = node.attr("instr") {
                events.push(InlineEvent::Instruction(instruction.to_string()));
            }
            events.push(InlineEvent::FieldSeparate);
            for child in &node.children {
                collect_inline_events(child, rels, href.clone(), events, warnings);
            }
            events.push(InlineEvent::FieldEnd);
        }
        "hyperlink" => {
            let next_href = node
                .attr("id")
                .and_then(|id| rels.get(id))
                .map(|relation| relation.target.clone())
                .or_else(|| node.attr("anchor").map(|anchor| format!("#{anchor}")))
                .or(href);
            for child in &node.children {
                collect_inline_events(child, rels, next_href.clone(), events, warnings);
            }
        }
        "r" => {
            let properties = node.child("rPr").cloned();
            for child in &node.children {
                match child.local_name() {
                    "rPr" => {}
                    "t" => events.push(InlineEvent::Text {
                        content: child.text.clone(),
                        properties: properties.clone(),
                        href: href.clone(),
                    }),
                    "tab" => events.push(InlineEvent::Text {
                        content: "\t".into(),
                        properties: properties.clone(),
                        href: href.clone(),
                    }),
                    "noBreakHyphen" => events.push(InlineEvent::Text {
                        content: "‑".into(),
                        properties: properties.clone(),
                        href: href.clone(),
                    }),
                    "instrText" => events.push(InlineEvent::Instruction(child.text.clone())),
                    "fldChar" => match child.attr("fldCharType") {
                        Some("begin") => events.push(InlineEvent::FieldBegin),
                        Some("separate") => events.push(InlineEvent::FieldSeparate),
                        Some("end") => events.push(InlineEvent::FieldEnd),
                        _ => warnings.push("field character has an unknown type".into()),
                    },
                    "br" => match child.attr("type") {
                        Some("page") => events.push(InlineEvent::Break(BreakKind::Page)),
                        Some("column") => events.push(InlineEvent::Break(BreakKind::Column)),
                        _ => events.push(InlineEvent::Text {
                            content: "\n".into(),
                            properties: properties.clone(),
                            href: href.clone(),
                        }),
                    },
                    "lastRenderedPageBreak" => events.push(InlineEvent::PageHint),
                    "drawing" | "pict" | "object" => {
                        events.push(InlineEvent::Drawing(child.clone()))
                    }
                    "footnoteReference" => {
                        if let Some(id) = child.attr("id") {
                            events.push(InlineEvent::Note(NoteKind::Footnote, id.to_string()));
                        }
                    }
                    "endnoteReference" => {
                        if let Some(id) = child.attr("id") {
                            events.push(InlineEvent::Note(NoteKind::Endnote, id.to_string()));
                        }
                    }
                    "commentReference" => {
                        if let Some(id) = child.attr("id") {
                            events.push(InlineEvent::Comment(id.to_string()));
                        }
                    }
                    _ => collect_inline_events(child, rels, href.clone(), events, warnings),
                }
            }
        }
        "commentRangeStart" => {
            if let Some(id) = node.attr("id") {
                events.push(InlineEvent::Comment(id.to_string()));
            }
        }
        "proofErr" | "bookmarkStart" | "bookmarkEnd" | "commentRangeEnd" => {}
        _ => {
            for child in &node.children {
                collect_inline_events(child, rels, href.clone(), events, warnings);
            }
        }
    }
}

fn merge_run(runs: &mut Vec<TextRun>, run: TextRun) {
    if let Some(previous) = runs.last_mut() {
        if previous.font == run.font && previous.color == run.color && previous.href == run.href {
            previous.content.push_str(&run.content);
            return;
        }
    }
    runs.push(run);
}

fn enforce_run_soft_cap(runs: &mut Vec<TextRun>, warnings: &mut Vec<String>) {
    if runs.len() <= MAX_RUNS_PER_PARAGRAPH {
        return;
    }
    warnings.push(format!(
        "paragraph run soft cap {MAX_RUNS_PER_PARAGRAPH} exceeded after adjacent-run merging; excess text coalesced"
    ));
    let excess = runs
        .drain(MAX_RUNS_PER_PARAGRAPH..)
        .map(|run| run.content)
        .collect::<String>();
    if let Some(last) = runs.last_mut() {
        last.content.push_str(&excess);
    }
}

fn visible_descendant_text(node: &Node) -> String {
    let mut output = String::new();
    collect_visible_text(node, &mut output);
    output
}

fn collect_visible_text(node: &Node, output: &mut String) {
    if matches!(node.local_name(), "instrText") {
        return;
    }
    if matches!(node.local_name(), "t" | "delText") {
        output.push_str(&node.text);
    }
    for child in &node.children {
        collect_visible_text(child, output);
    }
}

#[allow(clippy::too_many_arguments)]
fn extract_table(
    table: &Node,
    rels: &Relationships,
    ids: &mut IdAllocator,
    hidden: &mut Vec<HiddenItem>,
    note_refs: &mut Vec<NoteReference>,
    context: &mut Context<'_, '_>,
    depth: usize,
) -> Result<Block, ExtractError> {
    if depth > MAX_TEXTBOX_DEPTH {
        context.warnings.push(format!(
            "nested table depth limit {MAX_TEXTBOX_DEPTH} exceeded; table content skipped"
        ));
    }
    let id = ids.next();
    let col_widths: Vec<f64> = table
        .child("tblGrid")
        .into_iter()
        .flat_map(|grid| grid.children_named("gridCol"))
        .filter_map(|column| twips(column.attr("w")))
        .collect();
    if table.child("tblGrid").is_some() && col_widths.is_empty() {
        context.warnings.push(format!(
            "{id}: table grid has no valid authored column widths"
        ));
    }
    let rows_nodes: Vec<_> = table.children_named("tr").collect();
    let rows = rows_nodes.len();
    let mut cols = col_widths.len();
    let mut cells: Vec<FlowTableCell> = Vec::new();
    for (row_index, row) in rows_nodes.into_iter().enumerate() {
        let mut column = 0_usize;
        for cell in row.children_named("tc") {
            let properties = cell.child("tcPr");
            let span = properties
                .and_then(|node| node.child("gridSpan"))
                .and_then(|node| node.attr("val"))
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(1)
                .max(1)
                .min(10_000);
            cols = cols.max(column.saturating_add(span));
            let vertical_merge = properties
                .and_then(|node| node.child("vMerge"))
                .map(|node| node.attr("val").unwrap_or("continue"));
            if vertical_merge == Some("continue") {
                if let Some(anchor) = cells
                    .iter_mut()
                    .rev()
                    .find(|candidate| candidate.col == column && candidate.col_span == span)
                {
                    anchor.row_span = anchor.row_span.saturating_add(1);
                } else {
                    context.warnings.push(format!(
                        "{id}: vertical-merge continuation at row {row_index}, column {column} has no anchor"
                    ));
                }
                column = column.saturating_add(span);
                continue;
            }

            let mut cell_state = StoryState::default();
            let mut cell_ids = IdAllocator::new(format!("{id}-c{row_index}-{column}-b"));
            let mut blocks = Vec::new();
            let mut has_nested = false;
            for child in &cell.children {
                match child.local_name() {
                    "p" => {
                        let mut parsed = extract_paragraph(
                            child,
                            rels,
                            &mut cell_ids,
                            &mut cell_state,
                            hidden,
                            note_refs,
                            context,
                            depth,
                            false,
                        )?;
                        push_blocks(&mut blocks, &mut parsed, &mut cell_state, context)?;
                    }
                    "tbl" if depth < MAX_TEXTBOX_DEPTH => {
                        let nested = extract_table(
                            child,
                            rels,
                            &mut cell_ids,
                            hidden,
                            note_refs,
                            context,
                            depth + 1,
                        )?;
                        let mut parsed = vec![nested];
                        push_blocks(&mut blocks, &mut parsed, &mut cell_state, context)?;
                        has_nested = true;
                    }
                    "tbl" => context.warnings.push(format!(
                        "{id}: nested table depth limit {MAX_TEXTBOX_DEPTH} exceeded; subtree skipped"
                    )),
                    _ => {}
                }
            }
            finish_story(&mut blocks, &mut cell_state, context);
            let content = block_text(&blocks);
            let runs = blocks
                .iter()
                .filter_map(|block| match block {
                    Block::Paragraph { runs, .. } => Some(runs.as_slice()),
                    _ => None,
                })
                .flatten()
                .cloned()
                .collect();
            if blocks
                .iter()
                .any(|block| !matches!(block, Block::Paragraph { .. } | Block::Break { .. }))
            {
                has_nested = true;
            }
            cells.push(FlowTableCell {
                row: row_index,
                col: column,
                row_span: 1,
                col_span: span,
                content,
                runs,
                blocks: has_nested.then_some(blocks),
            });
            column = column.saturating_add(span);
        }
    }
    let placement = table
        .child("tblPr")
        .and_then(|node| node.child("tblpPr"))
        .map(table_placement);
    Ok(Block::Table {
        id,
        col_widths,
        rows,
        cols,
        cells,
        placement,
    })
}

fn table_placement(node: &Node) -> Placement {
    Placement {
        frame: placement_frame(node.attr("horzAnchor").unwrap_or("page")),
        x: twips(node.attr("tblpX")),
        y: twips(node.attr("tblpY")),
        width: None,
        height: None,
        align_h: node.attr("tblpXSpec").map(str::to_owned),
        align_v: node.attr("tblpYSpec").map(str::to_owned),
    }
}

#[allow(clippy::too_many_arguments)]
fn extract_drawing(
    drawing: &Node,
    rels: &Relationships,
    ids: &mut IdAllocator,
    hidden: &mut Vec<HiddenItem>,
    note_refs: &mut Vec<NoteReference>,
    context: &mut Context<'_, '_>,
    textbox_depth: usize,
) -> Result<Option<Block>, ExtractError> {
    let container = drawing
        .first_descendant("anchor")
        .or_else(|| drawing.first_descendant("inline"));
    let placement = container
        .filter(|node| node.local_name() == "anchor")
        .map(anchor_placement);
    let extent = container
        .and_then(|node| node.child("extent"))
        .or_else(|| drawing.first_descendant("extent"));
    let width = emu(extent.and_then(|node| node.attr("cx")));
    let height = emu(extent.and_then(|node| node.attr("cy")));

    if let Some(textbox) = drawing.first_descendant("txbxContent") {
        let id = ids.next();
        if textbox_depth >= MAX_TEXTBOX_DEPTH {
            context.warnings.push(format!(
                "{id}: textbox nesting depth limit {MAX_TEXTBOX_DEPTH} exceeded; content skipped"
            ));
            return Ok(Some(Block::Textbox {
                id,
                placement,
                blocks: Vec::new(),
            }));
        }
        let mut nested_ids = IdAllocator::new(format!("{id}-b"));
        let mut state = StoryState::default();
        let mut blocks = Vec::new();
        for child in &textbox.children {
            match child.local_name() {
                "p" => {
                    let mut parsed = extract_paragraph(
                        child,
                        rels,
                        &mut nested_ids,
                        &mut state,
                        hidden,
                        note_refs,
                        context,
                        textbox_depth + 1,
                        false,
                    )?;
                    push_blocks(&mut blocks, &mut parsed, &mut state, context)?;
                }
                "tbl" => {
                    let nested = extract_table(
                        child,
                        rels,
                        &mut nested_ids,
                        hidden,
                        note_refs,
                        context,
                        textbox_depth + 1,
                    )?;
                    let mut parsed = vec![nested];
                    push_blocks(&mut blocks, &mut parsed, &mut state, context)?;
                }
                _ => {}
            }
        }
        finish_story(&mut blocks, &mut state, context);
        return Ok(Some(Block::Textbox {
            id,
            placement: placement.map(|mut value| {
                value.width = width;
                value.height = height;
                value
            }),
            blocks,
        }));
    }

    let relation_id = drawing
        .first_descendant("blip")
        .and_then(|node| node.attr("embed").or_else(|| node.attr("link")))
        .or_else(|| {
            drawing
                .first_descendant("imagedata")
                .and_then(|node| node.attr("id"))
        });
    if relation_id.is_none() && drawing.first_descendant("imagedata").is_none() {
        context
            .warnings
            .push("drawing has no image or textbox content; subtree skipped".into());
        return Ok(None);
    }
    let id = ids.next();
    let mut content_hash = None;
    match relation_id.and_then(|relation_id| rels.get(relation_id)) {
        Some(relation) if relation.external => context
            .warnings
            .push(format!("{id}: external image target was not fetched")),
        Some(relation) => {
            let source_part = drawing_part_for_rels(rels);
            let media_path = resolve_target(source_part, &relation.target)?;
            match context.package.read(&media_path)? {
                Some(bytes) => content_hash = Some(hex(&Sha256::digest(bytes))),
                None => context.warnings.push(format!(
                    "{id}: referenced image media part is missing: {media_path}"
                )),
            }
        }
        None => context.warnings.push(format!(
            "{id}: image media relationship is missing or broken"
        )),
    }
    if let Some(alt) = alternative_text(drawing) {
        hidden.push(HiddenItem {
            kind: "alt".into(),
            element: Some(id.clone()),
            content: alt,
        });
    }
    Ok(Some(Block::Image {
        id,
        width,
        height,
        content_hash,
        placement: placement.map(|mut value| {
            value.width = width;
            value.height = height;
            value
        }),
    }))
}

// Relationship targets are resolved relative to the current story part. The
// caller passes that part explicitly through story-specific relationship sets;
// document.xml is the common path and header/footer media use ../media, which
// normalizes identically from word/*.xml.
fn drawing_part_for_rels(_rels: &Relationships) -> &'static str {
    "word/document.xml"
}

fn anchor_placement(anchor: &Node) -> Placement {
    let horizontal = anchor.child("positionH");
    let vertical = anchor.child("positionV");
    let frame = horizontal
        .and_then(|node| node.attr("relativeFrom"))
        .or_else(|| vertical.and_then(|node| node.attr("relativeFrom")))
        .map(placement_frame)
        .unwrap_or(PlacementFrame::Paragraph);
    Placement {
        frame,
        x: horizontal
            .and_then(|node| node.child("posOffset"))
            .and_then(|node| emu(Some(node.text.as_str()))),
        y: vertical
            .and_then(|node| node.child("posOffset"))
            .and_then(|node| emu(Some(node.text.as_str()))),
        width: None,
        height: None,
        align_h: horizontal
            .and_then(|node| node.child("align"))
            .map(|node| node.text.clone()),
        align_v: vertical
            .and_then(|node| node.child("align"))
            .map(|node| node.text.clone()),
    }
}

fn placement_frame(value: &str) -> PlacementFrame {
    match value {
        "page" => PlacementFrame::Page,
        "margin" | "leftMargin" | "rightMargin" | "insideMargin" | "outsideMargin" => {
            PlacementFrame::Margin
        }
        "column" => PlacementFrame::Column,
        "line" => PlacementFrame::Line,
        _ => PlacementFrame::Paragraph,
    }
}

fn alternative_text(node: &Node) -> Option<String> {
    node.first_descendant("docPr")
        .and_then(|properties| {
            properties
                .attr("descr")
                .filter(|value| !value.is_empty())
                .or_else(|| properties.attr("title").filter(|value| !value.is_empty()))
        })
        .or_else(|| {
            node.first_descendant("shape").and_then(|shape| {
                shape
                    .attr("alt")
                    .filter(|value| !value.is_empty())
                    .or_else(|| shape.attr("title").filter(|value| !value.is_empty()))
            })
        })
        .map(str::to_owned)
}

fn block_text(blocks: &[Block]) -> String {
    let mut parts = Vec::new();
    collect_block_text(blocks, &mut parts);
    parts.join("\n")
}

fn collect_block_text(blocks: &[Block], parts: &mut Vec<String>) {
    for block in blocks {
        match block {
            Block::Paragraph { content, .. } if !content.is_empty() => parts.push(content.clone()),
            Block::Table { cells, .. } => parts.extend(
                cells
                    .iter()
                    .filter(|cell| !cell.content.is_empty())
                    .map(|cell| cell.content.clone()),
            ),
            Block::Textbox { blocks, .. } => collect_block_text(blocks, parts),
            _ => {}
        }
    }
}

fn attach_section_stories(
    sections: &mut [Section],
    document_rels: &Relationships,
    context: &mut Context<'_, '_>,
) -> Result<(), ExtractError> {
    for (section_index, section) in sections.iter_mut().enumerate() {
        let encoded = section
            .hidden
            .iter()
            .find(|item| item.kind == "__sect_pr")
            .map(|item| item.content.clone())
            .unwrap_or_default();
        section.hidden.retain(|item| item.kind != "__sect_pr");
        for record in encoded.lines() {
            let mut fields = record.split('|');
            let (Some(kind), Some(variant), Some(rel_id)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            let Some(relation) = document_rels.get(rel_id) else {
                context.warnings.push(format!(
                    "section {section_index}: {kind} relationship {rel_id:?} is missing"
                ));
                continue;
            };
            if relation.external {
                context.warnings.push(format!(
                    "section {section_index}: external {kind} relationship ignored"
                ));
                continue;
            }
            let path = resolve_target("word/document.xml", &relation.target)?;
            match extract_story_part(
                &path,
                format!(
                    "s{section_index}-{}{}-b",
                    if kind == "headerReference" { "h" } else { "f" },
                    match variant {
                        "first" => 1,
                        "even" => 2,
                        _ => 0,
                    }
                ),
                section,
                context,
            ) {
                Ok(blocks) => {
                    let story = match variant {
                        "first" => Story::First { blocks },
                        "even" => Story::Even { blocks },
                        _ => Story::Default { blocks },
                    };
                    if kind == "headerReference" {
                        section.headers.push(story);
                    } else {
                        section.footers.push(story);
                    }
                }
                Err(error) => context.warnings.push(format!(
                    "section {section_index}: {kind} part {path:?} failed to parse: {error}"
                )),
            }
        }
    }
    Ok(())
}

fn extract_story_part(
    path: &str,
    id_prefix: String,
    section: &mut Section,
    context: &mut Context<'_, '_>,
) -> Result<Vec<Block>, ExtractError> {
    let root = xml_required_mc(context.package, path, &mut context.warnings)?;
    let rels = relationships(context.package, path)?;
    let mut ids = IdAllocator::new(id_prefix);
    let mut state = StoryState::default();
    let mut refs = Vec::new();
    let mut blocks = Vec::new();
    for child in &root.children {
        match child.local_name() {
            "p" => {
                let mut parsed = extract_paragraph(
                    child,
                    &rels,
                    &mut ids,
                    &mut state,
                    &mut section.hidden,
                    &mut refs,
                    context,
                    0,
                    false,
                )?;
                push_blocks(&mut blocks, &mut parsed, &mut state, context)?;
            }
            "tbl" => {
                let block = extract_table(
                    child,
                    &rels,
                    &mut ids,
                    &mut section.hidden,
                    &mut refs,
                    context,
                    0,
                )?;
                let mut parsed = vec![block];
                push_blocks(&mut blocks, &mut parsed, &mut state, context)?;
            }
            _ => {}
        }
    }
    if !refs.is_empty() {
        context.warnings.push(format!(
            "{path}: note references in a header/footer are not section body notes"
        ));
    }
    finish_story(&mut blocks, &mut state, context);
    Ok(blocks)
}

fn attach_notes(
    sections: &mut [Section],
    _document_rels: &Relationships,
    context: &mut Context<'_, '_>,
) -> Result<(), ExtractError> {
    for (section_index, section) in sections.iter_mut().enumerate() {
        let encoded: Vec<_> = section
            .hidden
            .iter()
            .filter(|item| item.kind == "__note_ref")
            .filter_map(|item| {
                let (kind, id) = item.content.split_once(':')?;
                Some((
                    if kind == "endnote" {
                        NoteKind::Endnote
                    } else {
                        NoteKind::Footnote
                    },
                    id.to_string(),
                    item.element.clone().unwrap_or_default(),
                ))
            })
            .collect();
        section.hidden.retain(|item| item.kind != "__note_ref");
        let mut seen = BTreeSet::new();
        for (kind, id, reference_block) in encoded {
            if !seen.insert((kind, id.clone())) {
                continue;
            }
            let Some(note) = context.notes.get(&(kind, id.clone())).cloned() else {
                context.warnings.push(format!(
                    "section {section_index}: {:?} {id:?} is referenced but missing",
                    kind
                ));
                continue;
            };
            let path = match kind {
                NoteKind::Footnote => "word/footnotes.xml",
                NoteKind::Endnote => "word/endnotes.xml",
            };
            let rels = relationships(context.package, path)?;
            let mut ids = IdAllocator::new(format!("s{section_index}-b"));
            ids.next = section.blocks.len();
            let mut state = StoryState::default();
            let mut nested_refs = Vec::new();
            let mut note_blocks = Vec::new();
            for child in &note.children {
                match child.local_name() {
                    "p" => {
                        let mut parsed = extract_paragraph(
                            child,
                            &rels,
                            &mut ids,
                            &mut state,
                            &mut section.hidden,
                            &mut nested_refs,
                            context,
                            0,
                            false,
                        )?;
                        push_blocks(&mut note_blocks, &mut parsed, &mut state, context)?;
                    }
                    "tbl" => {
                        let block = extract_table(
                            child,
                            &rels,
                            &mut ids,
                            &mut section.hidden,
                            &mut nested_refs,
                            context,
                            0,
                        )?;
                        let mut parsed = vec![block];
                        push_blocks(&mut note_blocks, &mut parsed, &mut state, context)?;
                    }
                    _ => {}
                }
            }
            finish_story(&mut note_blocks, &mut state, context);
            let content = block_text(&note_blocks);
            section.hidden.push(HiddenItem {
                kind: "footnote".into(),
                element: Some(reference_block),
                content,
            });
            section.blocks.extend(note_blocks);
        }
    }
    Ok(())
}

fn load_comments(
    package: &mut Package<'_>,
    warnings: &mut Vec<String>,
) -> Result<BTreeMap<String, String>, ExtractError> {
    let Some(root) = xml_optional_mc(package, "word/comments.xml", warnings)? else {
        return Ok(BTreeMap::new());
    };
    let mut comments = BTreeMap::new();
    for comment in root.children_named("comment") {
        if let Some(id) = comment.attr("id") {
            let content = comment
                .descendants("p")
                .map(visible_descendant_text)
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            comments.insert(id.to_string(), content);
        }
    }
    Ok(comments)
}

fn load_notes(
    package: &mut Package<'_>,
    warnings: &mut Vec<String>,
) -> Result<BTreeMap<(NoteKind, String), Node>, ExtractError> {
    let mut notes = BTreeMap::new();
    for (kind, path, element_name) in [
        (NoteKind::Footnote, "word/footnotes.xml", "footnote"),
        (NoteKind::Endnote, "word/endnotes.xml", "endnote"),
    ] {
        let Some(root) = xml_optional_mc(package, path, warnings)? else {
            continue;
        };
        for note in root.children_named(element_name) {
            let Some(id) = note.attr("id") else {
                continue;
            };
            if id.starts_with('-') {
                continue;
            }
            notes.insert((kind, id.to_string()), note.clone());
        }
    }
    Ok(notes)
}

fn metadata(package: &mut Package<'_>) -> Result<DocMetadata, ExtractError> {
    let Some(bytes) = package.read("docProps/core.xml")? else {
        return Ok(DocMetadata {
            title: None,
            author: None,
        });
    };
    let root = parse(&bytes, "docProps/core.xml")?;
    Ok(DocMetadata {
        title: root
            .first_descendant("title")
            .map(|node| node.text.clone())
            .filter(|value| !value.is_empty()),
        author: root
            .first_descendant("creator")
            .map(|node| node.text.clone())
            .filter(|value| !value.is_empty()),
    })
}

fn source_format(package: &mut Package<'_>) -> Result<&'static str, ExtractError> {
    let Some(bytes) = package.read("[Content_Types].xml")? else {
        return Ok("docx");
    };
    let content = std::str::from_utf8(&bytes)
        .map_err(|error| parse_failure(format!("[Content_Types].xml is not UTF-8: {error}")))?;
    Ok(if content.contains("macroEnabled.main+xml") {
        "docm"
    } else {
        "docx"
    })
}

fn xml_required_mc(
    package: &mut Package<'_>,
    path: &str,
    warnings: &mut Vec<String>,
) -> Result<Node, ExtractError> {
    let bytes = package.read_required(path)?;
    let root = parse(&bytes, path)?;
    Ok(preprocess_alternate_content(
        &root,
        SUPPORTED_MC_NAMESPACES,
        MAX_MC_DEPTH,
        warnings,
    ))
}

fn xml_optional_mc(
    package: &mut Package<'_>,
    path: &str,
    warnings: &mut Vec<String>,
) -> Result<Option<Node>, ExtractError> {
    let Some(bytes) = package.read(path)? else {
        return Ok(None);
    };
    let root = parse(&bytes, path)?;
    Ok(Some(preprocess_alternate_content(
        &root,
        SUPPORTED_MC_NAMESPACES,
        MAX_MC_DEPTH,
        warnings,
    )))
}

fn enforce_max_pages(
    max_pages: Option<u32>,
    approx_pages: Option<u32>,
    sections: &[Section],
    warnings: &mut Vec<String>,
) -> Result<(), ExtractError> {
    let Some(limit) = max_pages else {
        return Ok(());
    };
    if let Some(actual) = approx_pages {
        if actual > limit {
            return Err(ExtractError::TooManyPages { limit, actual });
        }
        return Ok(());
    }
    warnings.push("max_pages approximated as block cap for flow documents".into());
    let blocks: usize = sections
        .iter()
        .map(|section| section.blocks.iter().map(count_blocks).sum::<usize>())
        .sum();
    let cap = (limit as usize).saturating_mul(200);
    if blocks > cap {
        let actual = u32::try_from(blocks.div_ceil(200)).unwrap_or(u32::MAX);
        return Err(ExtractError::TooManyPages { limit, actual });
    }
    Ok(())
}

fn count_blocks(block: &Block) -> usize {
    1 + match block {
        Block::Textbox { blocks, .. } => blocks.iter().map(count_blocks).sum(),
        Block::Table { cells, .. } => cells
            .iter()
            .flat_map(|cell| cell.blocks.iter().flatten())
            .map(count_blocks)
            .sum(),
        _ => 0,
    }
}

fn visit_blocks(blocks: &[Block], visitor: &mut impl FnMut(&Block)) {
    for block in blocks {
        visitor(block);
        match block {
            Block::Textbox { blocks, .. } => visit_blocks(blocks, visitor),
            Block::Table { cells, .. } => {
                for nested in cells.iter().flat_map(|cell| cell.blocks.iter().flatten()) {
                    visit_blocks(std::slice::from_ref(nested), visitor);
                }
            }
            _ => {}
        }
    }
}

fn clear_approx_pages(blocks: &mut [Block]) {
    for block in blocks {
        match block {
            Block::Paragraph { approx_page, .. } => *approx_page = None,
            Block::Textbox { blocks, .. } => clear_approx_pages(blocks),
            Block::Table { cells, .. } => {
                for blocks in cells.iter_mut().filter_map(|cell| cell.blocks.as_mut()) {
                    clear_approx_pages(blocks);
                }
            }
            _ => {}
        }
    }
}

fn twips(value: Option<&str>) -> Option<f64> {
    value
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite())
        .map(|value| round3(value / TWIPS_PER_POINT))
}

fn emu(value: Option<&str>) -> Option<f64> {
    value
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite())
        .map(|value| round3(value / EMU_PER_POINT))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn parse_failure(message: impl Into<String>) -> ExtractError {
    ExtractError::ParseFailure(message.into())
}
