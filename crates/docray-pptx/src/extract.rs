use crate::package::Package;
use crate::xml::{parse, Node};
use docray_core::{Capabilities, ExtractError, Extractor};
use docray_model::{
    round3, AnnotationElement, BBox, DocMetadata, DocumentInfo, Element, Extraction, Font,
    Granularity, HiddenItem, ImageElement, Page, PathElement, Source, TableCell, TableElement,
    TextColor, TextElement, TextRun,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const EMU_PER_POINT: f64 = 12_700.0;
const CFB_MAGIC: &[u8; 8] = b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1";
const MAX_GROUP_DEPTH: usize = 64;

pub struct PptxExtractor;

impl Extractor for PptxExtractor {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            finest_granularity: Granularity::Element,
        }
    }

    fn extract(&self, bytes: &[u8], max_pages: Option<u32>) -> Result<Extraction, ExtractError> {
        if bytes.starts_with(CFB_MAGIC) {
            return Err(ExtractError::UnsupportedFormatMessage(
                "legacy or encrypted Office documents are not supported".into(),
            ));
        }

        let mut package = Package::open(bytes)?;
        if !package.contains("ppt/presentation.xml") {
            return Err(ExtractError::UnsupportedFormatMessage(
                "zip archive is not a PowerPoint file".into(),
            ));
        }

        let presentation = xml_required(&mut package, "ppt/presentation.xml")?;
        let presentation_rels = relationships(&mut package, "ppt/presentation.xml")?;
        let (width, height) = slide_size(&presentation)?;
        let slide_paths = slide_paths(&presentation, &presentation_rels)?;
        let page_count = slide_paths.len() as u32;
        if let Some(limit) = max_pages {
            if page_count > limit {
                return Err(ExtractError::TooManyPages {
                    limit,
                    actual: page_count,
                });
            }
        }

        let metadata = metadata(&mut package)?;
        let mut warnings = Vec::new();
        let mut pages = Vec::with_capacity(slide_paths.len());
        for (index, slide_path) in slide_paths.iter().enumerate() {
            let page_number = index as u32 + 1;
            match extract_slide(
                &mut package,
                slide_path,
                page_number,
                width,
                height,
                &mut warnings,
            ) {
                Ok(page) => pages.push(page),
                Err(error) => {
                    warnings.push(format!("page {page_number} failed to parse: {error}"));
                    pages.push(Page {
                        page_number,
                        width,
                        height,
                        rotation: 0,
                        scanned: false,
                        elements: Vec::new(),
                        hidden: Vec::new(),
                    });
                }
            }
        }

        Ok(Extraction {
            // The base model remains schema 1.1 internally; dispatch requires
            // explicit element granularity, whose wrapper serializes as 1.4.
            schema_version: "1.1".into(),
            source: Source {
                format: "pptx".into(),
                sha256: hex(&Sha256::digest(bytes)),
                size_bytes: bytes.len() as u64,
            },
            document: DocumentInfo {
                page_count,
                metadata,
            },
            warnings,
            pages,
        })
    }
}

fn metadata(package: &mut Package<'_>) -> Result<DocMetadata, ExtractError> {
    let Some(bytes) = package.read("docProps/core.xml")? else {
        return Ok(DocMetadata {
            title: None,
            author: None,
        });
    };
    let core = parse(&bytes, "docProps/core.xml")?;
    Ok(DocMetadata {
        title: core
            .first_descendant("title")
            .map(|node| node.text.clone())
            .filter(|value| !value.is_empty()),
        author: core
            .first_descendant("creator")
            .map(|node| node.text.clone())
            .filter(|value| !value.is_empty()),
    })
}

fn slide_size(presentation: &Node) -> Result<(f64, f64), ExtractError> {
    let size = presentation
        .first_descendant("sldSz")
        .ok_or_else(|| parse_failure("ppt/presentation.xml has no p:sldSz"))?;
    let width = size.attr("cx").and_then(parse_finite_number);
    let height = size.attr("cy").and_then(parse_finite_number);
    let (Some(width), Some(height)) = (width, height) else {
        return Err(parse_failure("ppt/presentation.xml has invalid slide size"));
    };
    if width <= 0.0 || height <= 0.0 {
        return Err(parse_failure("ppt/presentation.xml has invalid slide size"));
    }
    Ok((
        round3(width / EMU_PER_POINT),
        round3(height / EMU_PER_POINT),
    ))
}

fn slide_paths(presentation: &Node, rels: &Relationships) -> Result<Vec<String>, ExtractError> {
    let list = presentation
        .first_descendant("sldIdLst")
        .ok_or_else(|| parse_failure("ppt/presentation.xml has no p:sldIdLst"))?;
    let mut paths = Vec::new();
    for slide_id in list.children_named("sldId") {
        let rel_id = slide_id
            .attr("r:id")
            .ok_or_else(|| parse_failure("p:sldId has no relationship id"))?;
        let relation = rels
            .get(rel_id)
            .ok_or_else(|| parse_failure(format!("slide relationship {rel_id:?} is missing")))?;
        if relation.external {
            return Err(parse_failure(format!(
                "slide relationship {rel_id:?} cannot be external"
            )));
        }
        paths.push(resolve_target("ppt/presentation.xml", &relation.target)?);
    }
    Ok(paths)
}

fn extract_slide(
    package: &mut Package<'_>,
    slide_path: &str,
    page_number: u32,
    width: f64,
    height: f64,
    warnings: &mut Vec<String>,
) -> Result<Page, ExtractError> {
    let slide = xml_required(package, slide_path)?;
    let slide_rels = relationships(package, slide_path)?;

    let layout_path = slide_rels
        .first_internal_type("slideLayout")
        .map(|relation| resolve_target(slide_path, &relation.target))
        .transpose()?;
    let layout = layout_path
        .as_deref()
        .map(|path| xml_required(package, path))
        .transpose()?;
    let layout_rels = layout_path
        .as_deref()
        .map(|path| relationships(package, path))
        .transpose()?
        .unwrap_or_default();

    let master_path = layout_path
        .as_deref()
        .and_then(|path| {
            layout_rels
                .first_internal_type("slideMaster")
                .map(|relation| resolve_target(path, &relation.target))
        })
        .transpose()?;
    let master = master_path
        .as_deref()
        .map(|path| xml_required(package, path))
        .transpose()?;
    let master_rels = master_path
        .as_deref()
        .map(|path| relationships(package, path))
        .transpose()?
        .unwrap_or_default();

    let theme_path = master_path
        .as_deref()
        .and_then(|path| {
            master_rels
                .first_internal_type("theme")
                .map(|relation| resolve_target(path, &relation.target))
        })
        .transpose()?;
    let theme = theme_path
        .as_deref()
        .map(|path| xml_required(package, path).map(|root| Theme::from_xml(&root)))
        .transpose()?
        .unwrap_or_default();
    let color_map = master
        .as_ref()
        .and_then(|root| root.first_descendant("clrMap"))
        .map(color_map)
        .unwrap_or_default();

    let context = SlideContext {
        slide_path,
        slide_rels: &slide_rels,
        layout: layout.as_ref(),
        master: master.as_ref(),
        theme: &theme,
        color_map: &color_map,
    };
    let tree = slide
        .first_descendant("spTree")
        .ok_or_else(|| parse_failure(format!("{slide_path} has no p:spTree")))?;
    let mut elements = Vec::new();
    let mut hidden = Vec::new();
    let mut groups = Vec::new();
    extract_children(
        package,
        tree,
        &context,
        page_number,
        &mut groups,
        &mut elements,
        &mut hidden,
        warnings,
    )?;

    match notes_text(package, slide_path, &slide_rels) {
        Ok(Some(content)) => hidden.push(HiddenItem {
            kind: "notes".into(),
            element: None,
            content,
        }),
        Ok(None) => {}
        Err(error) => warnings.push(format!(
            "page {page_number}: speaker notes failed to parse: {error}"
        )),
    }
    if slide.attr("show") == Some("0") {
        hidden.push(HiddenItem {
            kind: "hidden-slide".into(),
            element: None,
            content: "true".into(),
        });
    }

    Ok(Page {
        page_number,
        width,
        height,
        rotation: 0,
        scanned: false,
        elements,
        hidden,
    })
}

struct SlideContext<'a> {
    slide_path: &'a str,
    slide_rels: &'a Relationships,
    layout: Option<&'a Node>,
    master: Option<&'a Node>,
    theme: &'a Theme,
    color_map: &'a BTreeMap<String, String>,
}

#[allow(clippy::too_many_arguments)]
fn extract_children(
    package: &mut Package<'_>,
    parent: &Node,
    context: &SlideContext<'_>,
    page_number: u32,
    groups: &mut Vec<GroupTransform>,
    elements: &mut Vec<Element>,
    hidden: &mut Vec<HiddenItem>,
    warnings: &mut Vec<String>,
) -> Result<(), ExtractError> {
    for child in &parent.children {
        match child.local_name() {
            "sp" => extract_shape(
                child,
                context,
                page_number,
                groups,
                elements,
                hidden,
                warnings,
            ),
            "pic" => extract_picture(
                package,
                child,
                context,
                page_number,
                groups,
                elements,
                hidden,
                warnings,
            )?,
            "cxnSp" => extract_path(
                child,
                context,
                page_number,
                groups,
                elements,
                hidden,
                warnings,
            ),
            "graphicFrame" => {
                extract_graphic_frame(child, context, page_number, groups, elements, warnings)
            }
            "grpSp" => {
                if groups.len() >= MAX_GROUP_DEPTH {
                    warnings.push(format!(
                        "page {page_number}: group nesting depth limit exceeded, subtree skipped"
                    ));
                    continue;
                }
                let Some(group) = parse_group_transform(child) else {
                    let name = object_name(child, "unnamed group");
                    warnings.push(format!(
                        "page {page_number}: group {name:?} has invalid or missing transform, subtree skipped"
                    ));
                    continue;
                };
                if group.ch_ext.x == 0.0 || group.ch_ext.y == 0.0 {
                    warnings.push(format!(
                        "page {page_number}: group has zero chExt, subtree skipped"
                    ));
                    continue;
                }
                groups.push(group);
                extract_children(
                    package,
                    child,
                    context,
                    page_number,
                    groups,
                    elements,
                    hidden,
                    warnings,
                )?;
                groups.pop();
            }
            // Non-visual and transform property records are not page elements.
            "nvGrpSpPr" | "grpSpPr" => {}
            _ => {}
        }
    }
    Ok(())
}

fn extract_shape(
    shape: &Node,
    context: &SlideContext<'_>,
    page_number: u32,
    groups: &[GroupTransform],
    elements: &mut Vec<Element>,
    hidden: &mut Vec<HiddenItem>,
    warnings: &mut Vec<String>,
) {
    let placeholder = placeholder(shape);
    let layout_shape = placeholder
        .as_ref()
        .and_then(|key| context.layout.and_then(|root| find_placeholder(root, key)));
    let master_shape = placeholder
        .as_ref()
        .and_then(|key| context.master.and_then(|root| find_placeholder(root, key)));
    let own_xfrm = shape.child("spPr").and_then(|node| node.child("xfrm"));
    let xfrm = match own_xfrm {
        Some(node) => parse_xfrm(node),
        None => layout_shape
            .and_then(shape_transform)
            .or_else(|| master_shape.and_then(shape_transform)),
    };
    let Some(xfrm) = xfrm else {
        let name = object_name(shape, "unnamed shape");
        warnings.push(format!(
            "page {page_number}: shape {name:?} geometry could not be resolved, shape skipped"
        ));
        return;
    };
    let bbox = bbox_for_transform(xfrm, groups);
    let tx_body = shape.child("txBody");
    let content = tx_body.map(text_content).unwrap_or_default();

    if !content.is_empty() {
        let id = next_id(page_number, elements);
        push_shape_hidden(shape, placeholder.as_ref(), &id, hidden);
        let text_style = resolve_text_style(
            tx_body.expect("non-empty text requires txBody"),
            layout_shape,
            placeholder.as_ref(),
            context,
        );
        let runs = resolve_shape_runs(
            tx_body.expect("non-empty text requires txBody"),
            layout_shape,
            placeholder.as_ref(),
            context,
        );
        elements.push(Element::Text(TextElement {
            id,
            bbox,
            content,
            font: Font {
                name: text_style.font_name,
                size: round3(text_style.size * autofit_scale(tx_body.unwrap())),
                bold: text_style.bold,
                italic: text_style.italic,
            },
            color: TextColor {
                fill: text_style.color,
                stroke: None,
            },
            lines: None,
            runs: Some(runs),
        }));

        for hyperlink in tx_body
            .unwrap()
            .descendants("hlinkClick")
            .filter_map(|node| node.attr("id"))
        {
            if let Some(relation) = context.slide_rels.get(hyperlink) {
                if relation.external {
                    let id = next_id(page_number, elements);
                    elements.push(Element::Annotation(AnnotationElement {
                        id,
                        bbox,
                        subtype: "link".into(),
                        uri: Some(relation.target.clone()),
                    }));
                }
            }
        }
    } else if has_geometry(shape) {
        let id = next_id(page_number, elements);
        push_shape_hidden(shape, placeholder.as_ref(), &id, hidden);
        elements.push(Element::Path(path_from_shape(id, bbox, shape, context)));
    }
}

fn extract_path(
    shape: &Node,
    context: &SlideContext<'_>,
    page_number: u32,
    groups: &[GroupTransform],
    elements: &mut Vec<Element>,
    hidden: &mut Vec<HiddenItem>,
    warnings: &mut Vec<String>,
) {
    let Some(xfrm) = shape_transform(shape) else {
        let name = object_name(shape, "unnamed connector");
        warnings.push(format!(
            "page {page_number}: connector {name:?} geometry could not be resolved, connector skipped"
        ));
        return;
    };
    let bbox = bbox_for_transform(xfrm, groups);
    let id = next_id(page_number, elements);
    push_alt(shape, &id, hidden);
    elements.push(Element::Path(path_from_shape(id, bbox, shape, context)));
}

fn path_from_shape(
    id: String,
    bbox: BBox,
    shape: &Node,
    context: &SlideContext<'_>,
) -> PathElement {
    let properties = shape.child("spPr");
    PathElement {
        id,
        bbox,
        fill: properties
            .and_then(|node| node.child("solidFill"))
            .and_then(|fill| resolve_fill(fill, context.theme, context.color_map)),
        stroke: properties
            .and_then(|node| node.child("ln"))
            .and_then(|line| line.child("solidFill"))
            .and_then(|fill| resolve_fill(fill, context.theme, context.color_map)),
        stroke_width: properties
            .and_then(|node| node.child("ln"))
            .and_then(|line| line.attr("w"))
            .and_then(parse_finite_number)
            .map(|width| round3(width / EMU_PER_POINT)),
    }
}

#[allow(clippy::too_many_arguments)]
fn extract_picture(
    package: &mut Package<'_>,
    picture: &Node,
    context: &SlideContext<'_>,
    page_number: u32,
    groups: &[GroupTransform],
    elements: &mut Vec<Element>,
    hidden: &mut Vec<HiddenItem>,
    warnings: &mut Vec<String>,
) -> Result<(), ExtractError> {
    let Some(xfrm) = shape_transform(picture) else {
        let name = object_name(picture, "unnamed picture");
        warnings.push(format!(
            "page {page_number}: picture {name:?} geometry could not be resolved, picture skipped"
        ));
        return Ok(());
    };
    let corners = transformed_corners(xfrm, groups);
    let bbox = bbox_from_points(&corners);
    let id = next_id(page_number, elements);
    push_alt(picture, &id, hidden);
    let embed = picture
        .first_descendant("blip")
        .and_then(|node| node.attr("embed"));
    let mut content_hash = None;
    match embed.and_then(|rel_id| context.slide_rels.get(rel_id)) {
        Some(relation) if !relation.external => {
            let media_path = resolve_target(context.slide_path, &relation.target)?;
            match package.read(&media_path)? {
                Some(bytes) => content_hash = Some(hex(&Sha256::digest(bytes))),
                None => warnings.push(format!(
                    "{id}: referenced picture media part is missing: {media_path}"
                )),
            }
        }
        _ => warnings.push(format!(
            "{id}: picture media relationship is missing or broken"
        )),
    }
    elements.push(Element::Image(ImageElement {
        id,
        bbox,
        quad: corners.map(point_to_points),
        pixel_width: None,
        pixel_height: None,
        colorspace: None,
        content_hash,
    }));
    Ok(())
}

fn push_shape_hidden(
    shape: &Node,
    placeholder: Option<&Placeholder>,
    element_id: &str,
    hidden: &mut Vec<HiddenItem>,
) {
    if let Some(placeholder) = placeholder {
        hidden.push(HiddenItem {
            kind: "role".into(),
            element: Some(element_id.to_owned()),
            content: placeholder.kind.clone().unwrap_or_else(|| "body".into()),
        });
    }
    push_alt(shape, element_id, hidden);
}

fn push_alt(node: &Node, element_id: &str, hidden: &mut Vec<HiddenItem>) {
    let Some(content) = alternative_text(node) else {
        return;
    };
    hidden.push(HiddenItem {
        kind: "alt".into(),
        element: Some(element_id.to_owned()),
        content,
    });
}

fn alternative_text(node: &Node) -> Option<String> {
    let properties = node.first_descendant("cNvPr")?;
    properties
        .attr("descr")
        .filter(|value| !value.is_empty())
        .or_else(|| properties.attr("title").filter(|value| !value.is_empty()))
        .map(str::to_owned)
}

fn notes_text(
    package: &mut Package<'_>,
    slide_path: &str,
    slide_rels: &Relationships,
) -> Result<Option<String>, ExtractError> {
    let Some(relation) = slide_rels.first_internal_type("notesSlide") else {
        return Ok(None);
    };
    let notes_path = resolve_target(slide_path, &relation.target)?;
    let notes = xml_required(package, &notes_path)?;
    let content = notes
        .descendants("sp")
        .filter(|shape| {
            placeholder(shape)
                .is_some_and(|placeholder| placeholder.kind.as_deref().unwrap_or("body") == "body")
        })
        .filter_map(|shape| shape.child("txBody"))
        .map(text_content)
        .filter(|content| !content.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    Ok((!content.is_empty()).then_some(content))
}

fn extract_graphic_frame(
    frame: &Node,
    context: &SlideContext<'_>,
    page_number: u32,
    groups: &[GroupTransform],
    elements: &mut Vec<Element>,
    warnings: &mut Vec<String>,
) {
    let Some(table) = frame.first_descendant("tbl") else {
        let kind = if frame.first_descendant("chart").is_some() {
            "chart"
        } else if frame.first_descendant("relIds").is_some() {
            "SmartArt"
        } else {
            "graphic"
        };
        warnings.push(format!(
            "page {page_number}: unsupported {kind} graphicFrame skipped"
        ));
        return;
    };
    let table_name = object_name(frame, "unnamed table");
    let Some(frame_xfrm) = frame_transform(frame) else {
        warnings.push(format!(
            "page {page_number}: table {table_name:?} frame geometry could not be resolved, table skipped"
        ));
        return;
    };
    let columns: Option<Vec<f64>> = table.first_descendant("tblGrid").and_then(|grid| {
        grid.children_named("gridCol")
            .map(|column| {
                column
                    .attr("w")
                    .and_then(parse_finite_number)
                    .filter(|width| *width >= 0.0)
            })
            .collect::<Option<Vec<_>>>()
    });
    let rows: Vec<&Node> = table.children_named("tr").collect();
    let heights: Option<Vec<f64>> = rows
        .iter()
        .map(|row| {
            row.attr("h")
                .and_then(parse_finite_number)
                .filter(|height| *height >= 0.0)
        })
        .collect();
    let (Some(columns), Some(heights)) = (columns, heights) else {
        warnings.push(format!(
            "page {page_number}: table {table_name:?} grid geometry is invalid, table skipped"
        ));
        return;
    };
    if columns.is_empty() || rows.is_empty() || heights.contains(&0.0) {
        warnings.push(format!(
            "page {page_number}: table {table_name:?} grid geometry is incomplete, table skipped"
        ));
        return;
    }

    let (Some(col_prefix), Some(row_prefix)) = (prefix_sums(&columns), prefix_sums(&heights))
    else {
        warnings.push(format!(
            "page {page_number}: table {table_name:?} grid geometry overflowed, table skipped"
        ));
        return;
    };
    let mut cells = Vec::new();
    for (row_index, row) in rows.iter().enumerate() {
        for (col_index, cell) in row.children_named("tc").enumerate() {
            if col_index >= columns.len() || bool_attr(cell, "hMerge") || bool_attr(cell, "vMerge")
            {
                continue;
            }
            let col_span = clamped_span(cell, "gridSpan", columns.len() - col_index);
            let row_span = clamped_span(cell, "rowSpan", rows.len() - row_index);
            let col_end = col_index.saturating_add(col_span).min(columns.len());
            let row_end = row_index.saturating_add(row_span).min(rows.len());
            let local = Xfrm {
                off: Point {
                    x: frame_xfrm.off.x + col_prefix[col_index],
                    y: frame_xfrm.off.y + row_prefix[row_index],
                },
                ext: Point {
                    x: col_prefix[col_end] - col_prefix[col_index],
                    y: row_prefix[row_end] - row_prefix[row_index],
                },
                rot: frame_xfrm.rot,
                flip_h: frame_xfrm.flip_h,
                flip_v: frame_xfrm.flip_v,
                rotation_center: Some(Point {
                    x: frame_xfrm.off.x + frame_xfrm.ext.x / 2.0,
                    y: frame_xfrm.off.y + frame_xfrm.ext.y / 2.0,
                }),
            };
            let bbox = bbox_for_transform(local, groups);
            let tx_body = cell.child("txBody");
            cells.push(TableCell {
                bbox,
                row: row_index,
                col: col_index,
                row_span,
                col_span,
                content: tx_body.map(text_content).unwrap_or_default(),
                runs: tx_body.map(|body| resolve_cell_runs(body, context)),
            });
        }
    }
    elements.push(Element::Table(TableElement {
        id: next_id(page_number, elements),
        bbox: bbox_for_transform(frame_xfrm, groups),
        rows: rows.len(),
        cols: columns.len(),
        cells,
    }));
}

fn prefix_sums(values: &[f64]) -> Option<Vec<f64>> {
    let mut out = Vec::with_capacity(values.len() + 1);
    out.push(0.0);
    for value in values {
        let sum = out.last().copied().unwrap_or(0.0) + value;
        if !sum.is_finite() {
            return None;
        }
        out.push(sum);
    }
    Some(out)
}

#[derive(Debug, Clone, Copy)]
struct Point {
    x: f64,
    y: f64,
}

#[derive(Debug, Clone, Copy)]
struct Xfrm {
    off: Point,
    ext: Point,
    rot: f64,
    flip_h: bool,
    flip_v: bool,
    /// Table cells rotate about the graphic frame, not each cell.
    rotation_center: Option<Point>,
}

#[derive(Debug, Clone, Copy)]
struct GroupTransform {
    off: Point,
    ext: Point,
    ch_off: Point,
    ch_ext: Point,
    rot: f64,
    flip_h: bool,
    flip_v: bool,
}

fn shape_transform(node: &Node) -> Option<Xfrm> {
    let properties = node.child("spPr")?;
    parse_xfrm(properties.child("xfrm")?)
}

fn frame_transform(node: &Node) -> Option<Xfrm> {
    parse_xfrm(node.child("xfrm")?)
}

fn parse_xfrm(node: &Node) -> Option<Xfrm> {
    let rot = match node.attr("rot") {
        Some(value) => parse_finite_number(value)? / 60_000.0,
        None => 0.0,
    };
    Some(Xfrm {
        off: parse_point(node.child("off")?)?,
        ext: parse_extent(node.child("ext")?)?,
        rot,
        flip_h: bool_attr(node, "flipH"),
        flip_v: bool_attr(node, "flipV"),
        rotation_center: None,
    })
}

fn parse_group_transform(group: &Node) -> Option<GroupTransform> {
    let xfrm = group.child("grpSpPr")?.child("xfrm")?;
    let rot = match xfrm.attr("rot") {
        Some(value) => parse_finite_number(value)? / 60_000.0,
        None => 0.0,
    };
    Some(GroupTransform {
        off: parse_point(xfrm.child("off")?)?,
        ext: parse_extent(xfrm.child("ext")?)?,
        ch_off: parse_point(xfrm.child("chOff")?)?,
        ch_ext: parse_extent(xfrm.child("chExt")?)?,
        rot,
        flip_h: bool_attr(xfrm, "flipH"),
        flip_v: bool_attr(xfrm, "flipV"),
    })
}

fn parse_point(node: &Node) -> Option<Point> {
    Some(Point {
        x: parse_finite_number(node.attr("x")?)?,
        y: parse_finite_number(node.attr("y")?)?,
    })
}

fn parse_extent(node: &Node) -> Option<Point> {
    Some(Point {
        x: parse_finite_number(node.attr("cx")?)?,
        y: parse_finite_number(node.attr("cy")?)?,
    })
}

fn transformed_corners(xfrm: Xfrm, groups: &[GroupTransform]) -> [Point; 4] {
    let mut corners = [
        xfrm.off,
        Point {
            x: xfrm.off.x + xfrm.ext.x,
            y: xfrm.off.y,
        },
        Point {
            x: xfrm.off.x + xfrm.ext.x,
            y: xfrm.off.y + xfrm.ext.y,
        },
        Point {
            x: xfrm.off.x,
            y: xfrm.off.y + xfrm.ext.y,
        },
    ];
    let center = xfrm.rotation_center.unwrap_or(Point {
        x: xfrm.off.x + xfrm.ext.x / 2.0,
        y: xfrm.off.y + xfrm.ext.y / 2.0,
    });
    for point in &mut corners {
        *point = flip_rotate(*point, center, xfrm.flip_h, xfrm.flip_v, xfrm.rot);
        for group in groups.iter().rev() {
            *point = transform_group(*point, *group);
        }
    }
    corners
}

fn transform_group(point: Point, group: GroupTransform) -> Point {
    let mapped = Point {
        x: group.off.x + (point.x - group.ch_off.x) * group.ext.x / group.ch_ext.x,
        y: group.off.y + (point.y - group.ch_off.y) * group.ext.y / group.ch_ext.y,
    };
    flip_rotate(
        mapped,
        Point {
            x: group.off.x + group.ext.x / 2.0,
            y: group.off.y + group.ext.y / 2.0,
        },
        group.flip_h,
        group.flip_v,
        group.rot,
    )
}

fn flip_rotate(mut point: Point, center: Point, flip_h: bool, flip_v: bool, degrees: f64) -> Point {
    if flip_h {
        point.x = 2.0 * center.x - point.x;
    }
    if flip_v {
        point.y = 2.0 * center.y - point.y;
    }
    if degrees != 0.0 {
        let radians = degrees.to_radians();
        let dx = point.x - center.x;
        let dy = point.y - center.y;
        point = Point {
            // y-down coordinate space: this matrix is visually clockwise.
            x: center.x + radians.cos() * dx - radians.sin() * dy,
            y: center.y + radians.sin() * dx + radians.cos() * dy,
        };
    }
    point
}

fn bbox_for_transform(xfrm: Xfrm, groups: &[GroupTransform]) -> BBox {
    bbox_from_points(&transformed_corners(xfrm, groups))
}

fn bbox_from_points(points: &[Point; 4]) -> BBox {
    let min_x = points
        .iter()
        .map(|point| point.x)
        .fold(f64::INFINITY, f64::min);
    let min_y = points
        .iter()
        .map(|point| point.y)
        .fold(f64::INFINITY, f64::min);
    let max_x = points
        .iter()
        .map(|point| point.x)
        .fold(f64::NEG_INFINITY, f64::max);
    let max_y = points
        .iter()
        .map(|point| point.y)
        .fold(f64::NEG_INFINITY, f64::max);
    BBox {
        x0: round3(min_x / EMU_PER_POINT),
        y0: round3(min_y / EMU_PER_POINT),
        x1: round3(max_x / EMU_PER_POINT),
        y1: round3(max_y / EMU_PER_POINT),
    }
}

fn point_to_points(point: Point) -> [f64; 2] {
    [
        round3(point.x / EMU_PER_POINT),
        round3(point.y / EMU_PER_POINT),
    ]
}

#[derive(Default)]
struct Theme {
    colors: BTreeMap<String, [u8; 3]>,
    major_latin: Option<String>,
    minor_latin: Option<String>,
}

impl Theme {
    fn from_xml(root: &Node) -> Self {
        let mut theme = Self::default();
        if let Some(scheme) = root.first_descendant("clrScheme") {
            for slot in &scheme.children {
                if let Some(color) = direct_color(slot) {
                    theme.colors.insert(slot.local_name().to_string(), color);
                }
            }
        }
        if let Some(font_scheme) = root.first_descendant("fontScheme") {
            theme.major_latin = font_scheme
                .child("majorFont")
                .and_then(|node| node.child("latin"))
                .and_then(|node| node.attr("typeface"))
                .map(str::to_owned);
            theme.minor_latin = font_scheme
                .child("minorFont")
                .and_then(|node| node.child("latin"))
                .and_then(|node| node.attr("typeface"))
                .map(str::to_owned);
        }
        theme
    }

    fn font(&self, value: &str) -> String {
        match value {
            "+mj-lt" => self.major_latin.clone().unwrap_or_else(|| "Arial".into()),
            "+mn-lt" => self.minor_latin.clone().unwrap_or_else(|| "Arial".into()),
            other => other.to_string(),
        }
    }
}

fn color_map(node: &Node) -> BTreeMap<String, String> {
    node.attrs
        .iter()
        .filter(|(key, _)| !key.starts_with("xmlns"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn resolve_fill(
    fill: &Node,
    theme: &Theme,
    color_map: &BTreeMap<String, String>,
) -> Option<[u8; 3]> {
    let color_node = fill.children.first()?;
    let mut color = match color_node.local_name() {
        "srgbClr" => parse_hex_color(color_node.attr("val")?)?,
        "schemeClr" => {
            let value = color_node.attr("val")?;
            let mapped = color_map.get(value).map(String::as_str).unwrap_or(value);
            *theme.colors.get(mapped)?
        }
        "sysClr" => parse_hex_color(color_node.attr("lastClr")?)?,
        _ => return None,
    };
    for modifier in &color_node.children {
        let Some(value) = modifier
            .attr("val")
            .and_then(|value| value.parse::<f64>().ok())
        else {
            continue;
        };
        let factor = value / 100_000.0;
        for channel in &mut color {
            let current = f64::from(*channel);
            let adjusted = match modifier.local_name() {
                "tint" => current + (255.0 - current) * factor,
                "shade" | "lumMod" => current * factor,
                "lumOff" => current + 255.0 * factor,
                _ => current,
            };
            *channel = adjusted.round().clamp(0.0, 255.0) as u8;
        }
    }
    Some(color)
}

fn direct_color(slot: &Node) -> Option<[u8; 3]> {
    let node = slot.children.first()?;
    match node.local_name() {
        "srgbClr" => parse_hex_color(node.attr("val")?),
        "sysClr" => parse_hex_color(node.attr("lastClr")?),
        _ => None,
    }
}

fn parse_hex_color(value: &str) -> Option<[u8; 3]> {
    if value.len() != 6 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some([
        u8::from_str_radix(&value[0..2], 16).ok()?,
        u8::from_str_radix(&value[2..4], 16).ok()?,
        u8::from_str_radix(&value[4..6], 16).ok()?,
    ])
}

#[derive(Default, Clone)]
struct PartialStyle {
    font_name: Option<String>,
    size: Option<f64>,
    bold: Option<bool>,
    italic: Option<bool>,
    color: Option<[u8; 3]>,
}

struct ResolvedStyle {
    font_name: String,
    size: f64,
    bold: bool,
    italic: bool,
    color: Option<[u8; 3]>,
}

fn resolve_text_style(
    tx_body: &Node,
    layout_shape: Option<&Node>,
    placeholder: Option<&Placeholder>,
    context: &SlideContext<'_>,
) -> ResolvedStyle {
    let paragraph = tx_body
        .children_named("p")
        .find(|paragraph| paragraph.descendants("t").any(|text| !text.text.is_empty()));
    let run = paragraph.and_then(first_text_run);
    let run_style = run
        .and_then(|node| node.child("rPr"))
        .map(|node| partial_style(node, context));
    let paragraph_style =
        paragraph.and_then(|node| paragraph_default_style(tx_body, node, context));
    let layout_style = layout_shape
        .and_then(|shape| shape.child("txBody"))
        .and_then(|body| body.first_descendant("defRPr"))
        .map(|node| partial_style(node, context));
    let master_style = context
        .master
        .and_then(|master| master_text_style(master, placeholder))
        .map(|node| partial_style(node, context));
    combine_styles(
        &[run_style, paragraph_style, layout_style, master_style],
        context.theme,
    )
}

fn resolve_shape_runs(
    tx_body: &Node,
    layout_shape: Option<&Node>,
    placeholder: Option<&Placeholder>,
    context: &SlideContext<'_>,
) -> Vec<TextRun> {
    let inherited = [
        layout_shape
            .and_then(|shape| shape.child("txBody"))
            .and_then(|body| body.first_descendant("defRPr"))
            .map(|node| partial_style(node, context)),
        context
            .master
            .and_then(|master| master_text_style(master, placeholder))
            .map(|node| partial_style(node, context)),
    ];
    resolve_runs(tx_body, &inherited, context, autofit_scale(tx_body))
}

fn resolve_cell_runs(tx_body: &Node, context: &SlideContext<'_>) -> Vec<TextRun> {
    resolve_runs(tx_body, &[], context, 1.0)
}

fn resolve_runs(
    tx_body: &Node,
    inherited: &[Option<PartialStyle>],
    context: &SlideContext<'_>,
    size_scale: f64,
) -> Vec<TextRun> {
    let mut runs = Vec::new();
    for paragraph in tx_body.children_named("p") {
        let paragraph_style = paragraph_default_style(tx_body, paragraph, context);
        for run in &paragraph.children {
            if !matches!(run.local_name(), "r" | "fld") {
                continue;
            }
            let run_style = run.child("rPr").map(|node| partial_style(node, context));
            let mut styles = Vec::with_capacity(2 + inherited.len());
            styles.push(run_style);
            styles.push(paragraph_style.clone());
            styles.extend(inherited.iter().cloned());
            let style = combine_styles(&styles, context.theme);
            let content = run
                .descendants("t")
                .map(|text| text.text.as_str())
                .collect();
            let href = run
                .first_descendant("hlinkClick")
                .and_then(|node| node.attr("id"))
                .and_then(|id| context.slide_rels.get(id))
                .filter(|relation| relation.external)
                .map(|relation| relation.target.clone());
            runs.push(TextRun {
                content,
                font: Font {
                    name: style.font_name,
                    size: scaled_font_size(style.size, size_scale),
                    bold: style.bold,
                    italic: style.italic,
                },
                color: TextColor {
                    fill: style.color,
                    stroke: None,
                },
                href,
            });
        }
    }
    runs
}

fn combine_styles(styles: &[Option<PartialStyle>], theme: &Theme) -> ResolvedStyle {
    let first = |select: fn(&PartialStyle) -> Option<&String>| {
        styles.iter().flatten().find_map(select).map(String::as_str)
    };
    ResolvedStyle {
        font_name: theme.font(first(|style| style.font_name.as_ref()).unwrap_or("+mn-lt")),
        size: styles
            .iter()
            .flatten()
            .find_map(|style| style.size)
            .unwrap_or(18.0),
        bold: styles
            .iter()
            .flatten()
            .find_map(|style| style.bold)
            .unwrap_or(false),
        italic: styles
            .iter()
            .flatten()
            .find_map(|style| style.italic)
            .unwrap_or(false),
        color: styles.iter().flatten().find_map(|style| style.color),
    }
}

fn partial_style(node: &Node, context: &SlideContext<'_>) -> PartialStyle {
    PartialStyle {
        font_name: node
            .child("latin")
            .and_then(|latin| latin.attr("typeface"))
            .map(str::to_owned),
        size: node.attr("sz").and_then(parse_font_size),
        bold: node.attr("b").map(parse_bool),
        italic: node.attr("i").map(parse_bool),
        color: node
            .child("solidFill")
            .and_then(|fill| resolve_fill(fill, context.theme, context.color_map)),
    }
}

fn paragraph_default_style(
    tx_body: &Node,
    paragraph: &Node,
    context: &SlideContext<'_>,
) -> Option<PartialStyle> {
    if let Some(style) = paragraph.child("pPr").and_then(|node| node.child("defRPr")) {
        return Some(partial_style(style, context));
    }
    let level = paragraph
        .child("pPr")
        .and_then(|node| node.attr("lvl"))
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
        .min(8) as usize;
    let level = level.saturating_add(1);
    let level_name = format!("lvl{level}pPr");
    tx_body
        .child("lstStyle")
        .and_then(|list| list.child(&level_name))
        .and_then(|node| node.child("defRPr"))
        .map(|node| partial_style(node, context))
}

fn master_text_style<'a>(master: &'a Node, placeholder: Option<&Placeholder>) -> Option<&'a Node> {
    let tx_styles = master.first_descendant("txStyles")?;
    let kind = placeholder
        .map(|key| key.kind.as_deref().unwrap_or("body"))
        .unwrap_or("other");
    let style_name = if title_kind(kind) {
        "titleStyle"
    } else if matches!(kind, "body" | "subTitle" | "obj") {
        "bodyStyle"
    } else {
        "otherStyle"
    };
    tx_styles
        .child(style_name)
        .and_then(|style| style.first_descendant("defRPr"))
}

fn autofit_scale(tx_body: &Node) -> f64 {
    tx_body
        .first_descendant("normAutofit")
        .and_then(|node| node.attr("fontScale"))
        .and_then(parse_finite_number)
        .map(|value| value / 100_000.0)
        .unwrap_or(1.0)
}

fn first_text_run(paragraph: &Node) -> Option<&Node> {
    paragraph.children.iter().find(|node| {
        matches!(node.local_name(), "r" | "fld")
            && node.descendants("t").any(|text| !text.text.is_empty())
    })
}

fn text_content(tx_body: &Node) -> String {
    tx_body
        .children_named("p")
        .map(|paragraph| {
            paragraph
                .children
                .iter()
                .filter_map(|child| match child.local_name() {
                    "r" | "fld" => Some(
                        child
                            .descendants("t")
                            .map(|text| text.text.as_str())
                            .collect::<String>(),
                    ),
                    "br" => Some("\n".to_owned()),
                    _ => None,
                })
                .collect::<String>()
        })
        .filter(|paragraph| !paragraph.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug)]
struct Placeholder {
    index: Option<String>,
    kind: Option<String>,
}

fn placeholder(shape: &Node) -> Option<Placeholder> {
    let ph = shape
        .child("nvSpPr")
        .and_then(|node| node.first_descendant("ph"))?;
    Some(Placeholder {
        index: ph.attr("idx").map(str::to_owned),
        kind: ph.attr("type").map(str::to_owned),
    })
}

fn find_placeholder<'a>(root: &'a Node, wanted: &Placeholder) -> Option<&'a Node> {
    root.descendants("sp").find(|shape| {
        let Some(candidate) = placeholder(shape) else {
            return false;
        };
        match &wanted.index {
            Some(index) => candidate.index.as_ref() == Some(index),
            None => placeholder_kinds_match(wanted.kind.as_deref(), candidate.kind.as_deref()),
        }
    })
}

fn placeholder_kinds_match(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) if title_kind(left) && title_kind(right) => true,
        (left, right) => left == right,
    }
}

fn title_kind(kind: &str) -> bool {
    matches!(kind, "title" | "ctrTitle")
}

fn has_geometry(shape: &Node) -> bool {
    shape.child("spPr").is_some_and(|properties| {
        properties.child("prstGeom").is_some() || properties.child("custGeom").is_some()
    })
}

#[derive(Default)]
struct Relationships {
    by_id: BTreeMap<String, Relationship>,
}

struct Relationship {
    target: String,
    kind: String,
    external: bool,
}

impl Relationships {
    fn get(&self, id: &str) -> Option<&Relationship> {
        self.by_id.get(id)
    }

    fn first_internal_type(&self, suffix: &str) -> Option<&Relationship> {
        self.by_id
            .values()
            .find(|relation| !relation.external && relation.kind.rsplit('/').next() == Some(suffix))
    }
}

fn relationships(
    package: &mut Package<'_>,
    source_part: &str,
) -> Result<Relationships, ExtractError> {
    let path = rels_path(source_part);
    let Some(bytes) = package.read(&path)? else {
        return Ok(Relationships::default());
    };
    let root = parse(&bytes, &path)?;
    let mut relationships = Relationships::default();
    for relation in root.descendants("Relationship") {
        let Some(id) = relation.attr("Id") else {
            continue;
        };
        let Some(target) = relation.attr("Target") else {
            continue;
        };
        relationships.by_id.insert(
            id.to_string(),
            Relationship {
                target: target.to_string(),
                kind: relation.attr("Type").unwrap_or_default().to_string(),
                external: relation.attr("TargetMode") == Some("External"),
            },
        );
    }
    Ok(relationships)
}

fn rels_path(source_part: &str) -> String {
    match source_part.rsplit_once('/') {
        Some((directory, file)) => format!("{directory}/_rels/{file}.rels"),
        None => format!("_rels/{source_part}.rels"),
    }
}

fn resolve_target(source_part: &str, target: &str) -> Result<String, ExtractError> {
    let target = target
        .split('#')
        .next()
        .unwrap_or(target)
        .replace('\\', "/");
    let mut parts: Vec<&str> = if target.starts_with('/') {
        Vec::new()
    } else {
        source_part
            .rsplit_once('/')
            .map_or_else(Vec::new, |(dir, _)| dir.split('/').collect())
    };
    for part in target.trim_start_matches('/').split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(parse_failure(format!(
                        "relationship target escapes the package root: {target:?}"
                    )));
                }
            }
            value => parts.push(value),
        }
    }
    Ok(parts.join("/"))
}

fn xml_required(package: &mut Package<'_>, path: &str) -> Result<Node, ExtractError> {
    let bytes = package.read_required(path)?;
    parse(&bytes, path)
}

fn parse_finite_number(value: &str) -> Option<f64> {
    value.parse::<f64>().ok().filter(|value| value.is_finite())
}

fn parse_font_size(value: &str) -> Option<f64> {
    let points = parse_finite_number(value)? / 100.0;
    round3(points).is_finite().then_some(points)
}

fn scaled_font_size(size: f64, scale: f64) -> f64 {
    let scaled = round3(size * scale);
    if scaled.is_finite() {
        scaled
    } else {
        round3(size)
    }
}

fn bool_attr(node: &Node, name: &str) -> bool {
    node.attr(name).is_some_and(parse_bool)
}

fn parse_bool(value: &str) -> bool {
    matches!(value, "1" | "true" | "on")
}

fn clamped_span(node: &Node, name: &str, remaining: usize) -> usize {
    let remaining_u64 = u64::try_from(remaining).unwrap_or(u64::MAX);
    let span = node
        .attr(name)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1)
        .max(1)
        .min(remaining_u64);
    usize::try_from(span).unwrap_or(remaining)
}

fn object_name(node: &Node, fallback: &str) -> String {
    node.first_descendant("cNvPr")
        .and_then(|properties| properties.attr("name"))
        .filter(|name| !name.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

fn next_id(page_number: u32, elements: &[Element]) -> String {
    format!("p{page_number}-e{}", elements.len())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn parse_failure(message: impl Into<String>) -> ExtractError {
    ExtractError::ParseFailure(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_test_xml(xml: &str) -> Node {
        parse(xml.as_bytes(), "test.xml").unwrap()
    }

    fn test_context<'a>(
        slide_rels: &'a Relationships,
        theme: &'a Theme,
        color_map: &'a BTreeMap<String, String>,
    ) -> SlideContext<'a> {
        SlideContext {
            slide_path: "ppt/slides/slide1.xml",
            slide_rels,
            layout: None,
            master: None,
            theme,
            color_map,
        }
    }

    #[test]
    fn group_transform_scales_then_rotates_about_group_center() {
        let group = GroupTransform {
            off: Point { x: 100.0, y: 200.0 },
            ext: Point { x: 400.0, y: 200.0 },
            ch_off: Point { x: 10.0, y: 20.0 },
            ch_ext: Point { x: 200.0, y: 100.0 },
            rot: 90.0,
            flip_h: false,
            flip_v: false,
        };
        // (10,20) maps to group off (100,200), then clockwise 90 degrees
        // around (300,300), yielding (400,100) in y-down coordinates.
        let point = transform_group(Point { x: 10.0, y: 20.0 }, group);
        assert!((point.x - 400.0).abs() < 1e-9);
        assert!((point.y - 100.0).abs() < 1e-9);
    }

    #[test]
    fn non_ascii_six_byte_hex_color_is_unresolvable_without_panicking() {
        assert_eq!("aéaaa".len(), 6);
        assert_eq!(parse_hex_color("aéaaa"), None);
    }

    #[test]
    fn non_finite_slide_size_is_a_parse_failure() {
        let presentation =
            parse_test_xml(r#"<presentation><sldSz cx="NaN" cy="6858000"/></presentation>"#);
        let error = slide_size(&presentation).unwrap_err();
        assert_eq!(error.code(), "parse_failure");
        assert!(error.to_string().contains("invalid slide size"));
    }

    #[test]
    fn non_finite_shape_geometry_is_skipped_with_a_warning() {
        let tree = parse_test_xml(
            r#"<spTree>
                <sp><nvSpPr><cNvPr name="Hostile Shape"/></nvSpPr><spPr><xfrm><off x="NaN" y="0"/><ext cx="12700" cy="12700"/></xfrm></spPr><txBody><p><r><t>bad</t></r></p></txBody></sp>
                <sp><nvSpPr><cNvPr name="Good Shape"/></nvSpPr><spPr><xfrm><off x="12700" y="12700"/><ext cx="12700" cy="12700"/></xfrm></spPr><txBody><p><r><t>good</t></r></p></txBody></sp>
            </spTree>"#,
        );
        let slide_rels = Relationships::default();
        let theme = Theme::default();
        let color_map = BTreeMap::new();
        let context = test_context(&slide_rels, &theme, &color_map);
        let mut elements = Vec::new();
        let mut hidden = Vec::new();
        let mut warnings = Vec::new();
        for shape in tree.children_named("sp") {
            extract_shape(
                shape,
                &context,
                1,
                &[],
                &mut elements,
                &mut hidden,
                &mut warnings,
            );
        }

        assert_eq!(elements.len(), 1);
        let Element::Text(text) = &elements[0] else {
            panic!("valid sibling shape must remain extracted");
        };
        assert_eq!(text.content, "good");
        assert!(warnings
            .iter()
            .any(|warning| warning.contains("Hostile Shape")));
        let json = serde_json::to_string(&elements).unwrap();
        for coordinate in ["x0", "y0", "x1", "y1"] {
            assert!(!json.contains(&format!(r#""{coordinate}":null"#)));
        }
    }

    #[test]
    fn invalid_run_sizes_fall_back_to_finite_inherited_size() {
        let tx_body = parse_test_xml(
            r#"<txBody>
                <lstStyle/>
                <p>
                    <pPr><defRPr sz="2400"/></pPr>
                    <r><rPr sz="NaN"/><t>nan</t></r>
                    <r><rPr sz="1e308"/><t>huge</t></r>
                </p>
            </txBody>"#,
        );
        let slide_rels = Relationships::default();
        let theme = Theme::default();
        let color_map = BTreeMap::new();
        let context = test_context(&slide_rels, &theme, &color_map);
        let runs = resolve_cell_runs(&tx_body, &context);

        assert_eq!(runs.len(), 2);
        assert!(runs.iter().all(|run| run.font.size == 24.0));
        assert!(runs.iter().all(|run| run.font.size.is_finite()));

        let extraction = Extraction {
            schema_version: "1.1".into(),
            source: Source {
                format: "pptx".into(),
                sha256: "test".into(),
                size_bytes: 0,
            },
            document: DocumentInfo {
                page_count: 1,
                metadata: DocMetadata {
                    title: None,
                    author: None,
                },
            },
            warnings: Vec::new(),
            pages: vec![Page {
                page_number: 1,
                width: 100.0,
                height: 100.0,
                rotation: 0,
                scanned: false,
                elements: vec![Element::Text(TextElement {
                    id: "p1-e0".into(),
                    bbox: BBox {
                        x0: 0.0,
                        y0: 0.0,
                        x1: 10.0,
                        y1: 10.0,
                    },
                    content: "nanhuge".into(),
                    font: runs[0].font.clone(),
                    color: runs[0].color.clone(),
                    lines: None,
                    runs: Some(runs),
                })],
                hidden: Vec::new(),
            }],
        };
        let docray_model::GranularExtraction::Compact(compact) =
            extraction.with_granularity(Granularity::Element)
        else {
            unreachable!();
        };
        let json = serde_json::to_string(&compact).unwrap();
        assert!(!json.contains(r#""size":null"#));
        let lean = compact.to_lean();
        assert!(!lean.contains("NaN"));
        assert!(!lean.to_ascii_lowercase().contains("inf"));
    }

    #[test]
    fn hard_breaks_and_fields_preserve_paragraph_document_order() {
        let tx_body = parse_test_xml(
            r#"<txBody><lstStyle/><p>
                <r><rPr/><t>A</t></r>
                <br/>
                <fld id="{field}"><rPr b="1"/><t>7</t></fld>
            </p></txBody>"#,
        );
        let slide_rels = Relationships::default();
        let theme = Theme::default();
        let color_map = BTreeMap::new();
        let context = test_context(&slide_rels, &theme, &color_map);

        assert_eq!(text_content(&tx_body), "A\n7");
        let runs = resolve_cell_runs(&tx_body, &context);
        assert_eq!(runs.len(), 2, "a:br is content, not a styled TextRun");
        assert_eq!(runs[0].content, "A");
        assert!(!runs[0].font.bold);
        assert_eq!(runs[1].content, "7");
        assert!(runs[1].font.bold);
    }

    #[test]
    fn malformed_table_grid_skips_named_table_and_preserves_other_elements() {
        let frame = parse_test_xml(
            r#"<graphicFrame>
                <nvGraphicFramePr><cNvPr name="Hostile Table"/></nvGraphicFramePr>
                <xfrm><off x="0" y="0"/><ext cx="254000" cy="127000"/></xfrm>
                <graphic><graphicData><tbl>
                    <tblGrid><gridCol w="bogus"/><gridCol w="127000"/></tblGrid>
                    <tr h="127000"><tc><txBody><p><r><t>shifted</t></r></p></txBody></tc><tc><txBody><p><r><t>later</t></r></p></txBody></tc></tr>
                </tbl></graphicData></graphic>
            </graphicFrame>"#,
        );
        let slide_rels = Relationships::default();
        let theme = Theme::default();
        let color_map = BTreeMap::new();
        let context = test_context(&slide_rels, &theme, &color_map);
        let mut elements = vec![Element::Path(PathElement {
            id: "outside-table".into(),
            bbox: BBox {
                x0: 1.0,
                y0: 1.0,
                x1: 2.0,
                y1: 2.0,
            },
            fill: None,
            stroke: None,
            stroke_width: None,
        })];
        let mut warnings = Vec::new();

        extract_graphic_frame(&frame, &context, 1, &[], &mut elements, &mut warnings);

        assert_eq!(
            elements.len(),
            1,
            "unreliable table cells must not be emitted"
        );
        assert!(warnings
            .iter()
            .any(|warning| warning.contains("Hostile Table")));
    }

    #[test]
    fn huge_grid_spans_and_paragraph_levels_are_clamped() {
        let frame = parse_test_xml(
            r#"<graphicFrame>
                <nvGraphicFramePr><cNvPr name="Huge Values"/></nvGraphicFramePr>
                <xfrm><off x="0" y="0"/><ext cx="254000" cy="254000"/></xfrm>
                <graphic><graphicData><tbl>
                    <tblGrid><gridCol w="127000"/><gridCol w="127000"/></tblGrid>
                    <tr h="127000"><tc gridSpan="18446744073709551615" rowSpan="18446744073709551615"><txBody>
                        <lstStyle><lvl9pPr><defRPr sz="900"/></lvl9pPr></lstStyle>
                        <p><pPr lvl="18446744073709551615"/><r><t>clamped</t></r></p>
                    </txBody></tc></tr>
                    <tr h="127000"/>
                </tbl></graphicData></graphic>
            </graphicFrame>"#,
        );
        let slide_rels = Relationships::default();
        let theme = Theme::default();
        let color_map = BTreeMap::new();
        let context = test_context(&slide_rels, &theme, &color_map);
        let mut elements = Vec::new();
        let mut warnings = Vec::new();

        extract_graphic_frame(&frame, &context, 1, &[], &mut elements, &mut warnings);

        assert!(
            warnings.is_empty(),
            "clamping is valid recovery: {warnings:?}"
        );
        assert_eq!(elements.len(), 1);
        let Element::Table(table) = &elements[0] else {
            panic!("table must be first-class");
        };
        assert_eq!(table.cells.len(), 1);
        let cell = &table.cells[0];
        assert_eq!(
            (cell.bbox.x0, cell.bbox.y0, cell.bbox.x1, cell.bbox.y1),
            (0.0, 0.0, 20.0, 20.0)
        );
        assert_eq!((cell.row_span, cell.col_span), (2, 2));
        assert_eq!(
            cell.runs.as_ref().unwrap()[0].font.size,
            9.0,
            "lvl must clamp to OOXML level 8"
        );
    }
}
