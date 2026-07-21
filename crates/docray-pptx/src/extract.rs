use crate::package::Package;
use crate::xml::{parse, Node};
use docray_core::{Capabilities, ExtractError, Extractor};
use docray_model::{
    round3, AnnotationElement, BBox, DocMetadata, DocumentInfo, Element, Extraction, Font,
    Granularity, ImageElement, Page, PathElement, Source, TextColor, TextElement,
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
                    });
                }
            }
        }

        Ok(Extraction {
            // The base model remains schema 1.1 internally; dispatch requires
            // explicit element granularity, whose wrapper serializes as 1.2.
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
    let width = attr_f64(size, "cx")?;
    let height = attr_f64(size, "cy")?;
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
    let mut groups = Vec::new();
    extract_children(
        package,
        tree,
        &context,
        page_number,
        &mut groups,
        &mut elements,
        warnings,
    )?;

    Ok(Page {
        page_number,
        width,
        height,
        rotation: 0,
        scanned: false,
        elements,
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
    warnings: &mut Vec<String>,
) -> Result<(), ExtractError> {
    for child in &parent.children {
        match child.local_name() {
            "sp" => extract_shape(child, context, page_number, groups, elements, warnings),
            "pic" => extract_picture(
                package,
                child,
                context,
                page_number,
                groups,
                elements,
                warnings,
            )?,
            "cxnSp" => extract_path(child, context, page_number, groups, elements, warnings),
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
                    warnings.push(format!(
                        "page {page_number}: group has missing transform, subtree skipped"
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
    warnings: &mut Vec<String>,
) {
    let placeholder = placeholder(shape);
    let layout_shape = placeholder
        .as_ref()
        .and_then(|key| context.layout.and_then(|root| find_placeholder(root, key)));
    let master_shape = placeholder
        .as_ref()
        .and_then(|key| context.master.and_then(|root| find_placeholder(root, key)));
    let xfrm = shape_transform(shape)
        .or_else(|| layout_shape.and_then(shape_transform))
        .or_else(|| master_shape.and_then(shape_transform));
    let Some(xfrm) = xfrm else {
        warnings.push(format!(
            "page {page_number}: shape geometry could not be resolved, shape skipped"
        ));
        return;
    };
    let bbox = bbox_for_transform(xfrm, groups);
    let tx_body = shape.child("txBody");
    let content = tx_body.map(text_content).unwrap_or_default();

    if !content.is_empty() {
        let id = next_id(page_number, elements);
        let text_style = resolve_text_style(
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
        elements.push(Element::Path(path_from_shape(id, bbox, shape, context)));
    }
}

fn extract_path(
    shape: &Node,
    context: &SlideContext<'_>,
    page_number: u32,
    groups: &[GroupTransform],
    elements: &mut Vec<Element>,
    warnings: &mut Vec<String>,
) {
    let Some(xfrm) = shape_transform(shape) else {
        warnings.push(format!(
            "page {page_number}: connector geometry could not be resolved, connector skipped"
        ));
        return;
    };
    let bbox = bbox_for_transform(xfrm, groups);
    let id = next_id(page_number, elements);
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
            .and_then(|value| value.parse::<f64>().ok())
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
    warnings: &mut Vec<String>,
) -> Result<(), ExtractError> {
    let Some(xfrm) = shape_transform(picture) else {
        warnings.push(format!(
            "page {page_number}: picture geometry could not be resolved, picture skipped"
        ));
        return Ok(());
    };
    let corners = transformed_corners(xfrm, groups);
    let bbox = bbox_from_points(&corners);
    let id = next_id(page_number, elements);
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
    let Some(frame_xfrm) = frame_transform(frame) else {
        warnings.push(format!(
            "page {page_number}: table frame geometry could not be resolved, table skipped"
        ));
        return;
    };
    let columns: Vec<f64> = table
        .first_descendant("tblGrid")
        .map(|grid| {
            grid.children_named("gridCol")
                .filter_map(|column| column.attr("w")?.parse::<f64>().ok())
                .collect()
        })
        .unwrap_or_default();
    let rows: Vec<&Node> = table.children_named("tr").collect();
    let heights: Vec<f64> = rows
        .iter()
        .map(|row| {
            row.attr("h")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0.0)
        })
        .collect();
    if columns.is_empty() || rows.is_empty() || heights.contains(&0.0) {
        warnings.push(format!(
            "page {page_number}: table grid geometry is incomplete, table skipped"
        ));
        return;
    }

    let col_prefix = prefix_sums(&columns);
    let row_prefix = prefix_sums(&heights);
    for (row_index, row) in rows.iter().enumerate() {
        for (col_index, cell) in row.children_named("tc").enumerate() {
            if col_index >= columns.len() || bool_attr(cell, "hMerge") || bool_attr(cell, "vMerge")
            {
                continue;
            }
            let content = cell.child("txBody").map(text_content).unwrap_or_default();
            if content.is_empty() {
                continue;
            }
            let col_span = usize_attr(cell, "gridSpan").unwrap_or(1).max(1);
            let row_span = usize_attr(cell, "rowSpan").unwrap_or(1).max(1);
            let col_end = (col_index + col_span).min(columns.len());
            let row_end = (row_index + row_span).min(rows.len());
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
            let style = resolve_cell_style(cell.child("txBody").unwrap(), context);
            let id = next_id(page_number, elements);
            elements.push(Element::Text(TextElement {
                id,
                bbox,
                content,
                font: Font {
                    name: style.font_name,
                    size: round3(style.size),
                    bold: style.bold,
                    italic: style.italic,
                },
                color: TextColor {
                    fill: style.color,
                    stroke: None,
                },
                lines: None,
            }));
        }
    }
}

fn prefix_sums(values: &[f64]) -> Vec<f64> {
    let mut out = Vec::with_capacity(values.len() + 1);
    out.push(0.0);
    for value in values {
        out.push(out.last().copied().unwrap_or(0.0) + value);
    }
    out
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
    Some(Xfrm {
        off: parse_point(node.child("off")?)?,
        ext: parse_extent(node.child("ext")?)?,
        rot: node
            .attr("rot")
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(0.0)
            / 60_000.0,
        flip_h: bool_attr(node, "flipH"),
        flip_v: bool_attr(node, "flipV"),
        rotation_center: None,
    })
}

fn parse_group_transform(group: &Node) -> Option<GroupTransform> {
    let xfrm = group.child("grpSpPr")?.child("xfrm")?;
    Some(GroupTransform {
        off: parse_point(xfrm.child("off")?)?,
        ext: parse_extent(xfrm.child("ext")?)?,
        ch_off: parse_point(xfrm.child("chOff")?)?,
        ch_ext: parse_extent(xfrm.child("chExt")?)?,
        rot: xfrm
            .attr("rot")
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(0.0)
            / 60_000.0,
        flip_h: bool_attr(xfrm, "flipH"),
        flip_v: bool_attr(xfrm, "flipV"),
    })
}

fn parse_point(node: &Node) -> Option<Point> {
    Some(Point {
        x: node.attr("x")?.parse().ok()?,
        y: node.attr("y")?.parse().ok()?,
    })
}

fn parse_extent(node: &Node) -> Option<Point> {
    Some(Point {
        x: node.attr("cx")?.parse().ok()?,
        y: node.attr("cy")?.parse().ok()?,
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
    if value.len() != 6 {
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

fn resolve_cell_style(tx_body: &Node, context: &SlideContext<'_>) -> ResolvedStyle {
    let paragraph = tx_body.children_named("p").next();
    let run_style = paragraph
        .and_then(first_text_run)
        .and_then(|node| node.child("rPr"))
        .map(|node| partial_style(node, context));
    let paragraph_style =
        paragraph.and_then(|node| paragraph_default_style(tx_body, node, context));
    combine_styles(&[run_style, paragraph_style], context.theme)
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
        size: node
            .attr("sz")
            .and_then(|value| value.parse::<f64>().ok())
            .map(|value| value / 100.0),
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
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0)
        + 1;
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
        .and_then(|key| key.kind.as_deref())
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
        .and_then(|value| value.parse::<f64>().ok())
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
                .descendants("t")
                .map(|text| text.text.as_str())
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

fn attr_f64(node: &Node, name: &str) -> Result<f64, ExtractError> {
    node.attr(name)
        .ok_or_else(|| parse_failure(format!("{} is missing {name}", node.name)))?
        .parse()
        .map_err(|_| parse_failure(format!("{} has invalid {name}", node.name)))
}

fn bool_attr(node: &Node, name: &str) -> bool {
    node.attr(name).is_some_and(parse_bool)
}

fn parse_bool(value: &str) -> bool {
    matches!(value, "1" | "true" | "on")
}

fn usize_attr(node: &Node, name: &str) -> Option<usize> {
    node.attr(name)?.parse().ok()
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
}
