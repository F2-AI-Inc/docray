use docray_model::{round3, Font, TextColor};
use docray_ooxml::{parse_hex_color, Node, Theme};
use std::collections::{BTreeMap, BTreeSet};

const STYLE_DEPTH_LIMIT: usize = 16;

#[derive(Default, Clone)]
pub(crate) struct RunProps {
    font: Option<String>,
    font_theme: Option<String>,
    font_cs: Option<String>,
    font_cs_theme: Option<String>,
    size: Option<f64>,
    size_cs: Option<f64>,
    bold: Option<bool>,
    italic: Option<bool>,
    bold_cs: Option<bool>,
    italic_cs: Option<bool>,
    color: Option<String>,
    theme_color: Option<String>,
    theme_tint: Option<String>,
    theme_shade: Option<String>,
    pub(crate) complex_script: bool,
    pub(crate) run_style: Option<String>,
}

#[derive(Default, Clone)]
struct ParagraphProps {
    outline_level: Option<u32>,
    page_break_before: Option<bool>,
    num_id: Option<String>,
    num_level: Option<u32>,
}

#[derive(Clone)]
struct Style {
    style_type: String,
    name: String,
    based_on: Option<String>,
    linked: Option<String>,
    run: RunProps,
    paragraph: ParagraphProps,
}

pub(crate) struct Styles {
    styles: BTreeMap<String, Style>,
    defaults: RunProps,
}

#[derive(Clone)]
pub(crate) struct ParagraphStyle {
    pub(crate) role: String,
    pub(crate) page_break_before: bool,
    pub(crate) num_id: Option<String>,
    pub(crate) num_level: u32,
    layers: Vec<RunProps>,
}

impl Styles {
    pub(crate) fn from_xml(root: Option<&Node>) -> Self {
        let mut result = Self {
            styles: BTreeMap::new(),
            defaults: RunProps::default(),
        };
        let Some(root) = root else {
            return result;
        };
        if let Some(r_pr) = root
            .first_descendant("docDefaults")
            .and_then(|node| node.first_descendant("rPrDefault"))
            .and_then(|node| node.child("rPr"))
        {
            result.defaults = run_props(r_pr);
        }
        for node in root.descendants("style") {
            let Some(id) = node.attr("styleId") else {
                continue;
            };
            result.styles.insert(
                id.to_string(),
                Style {
                    style_type: node.attr("type").unwrap_or_default().to_string(),
                    name: node
                        .child("name")
                        .and_then(|value| value.attr("val"))
                        .unwrap_or(id)
                        .to_string(),
                    based_on: child_val(node, "basedOn").map(str::to_owned),
                    linked: child_val(node, "link").map(str::to_owned),
                    run: node.child("rPr").map(run_props).unwrap_or_default(),
                    paragraph: node.child("pPr").map(paragraph_props).unwrap_or_default(),
                },
            );
        }
        result
    }

    pub(crate) fn paragraph(
        &self,
        p_pr: Option<&Node>,
        warnings: &mut Vec<String>,
    ) -> ParagraphStyle {
        let style_id = p_pr.and_then(|node| child_val(node, "pStyle"));
        let chain = style_id
            .map(|id| self.chain(id, Some("paragraph"), warnings))
            .unwrap_or_default();
        let direct = p_pr.map(paragraph_props).unwrap_or_default();
        let outline_level = direct.outline_level.or_else(|| {
            chain
                .iter()
                .rev()
                .find_map(|style| style.paragraph.outline_level)
        });
        let page_break_before = direct.page_break_before.or_else(|| {
            chain
                .iter()
                .rev()
                .find_map(|style| style.paragraph.page_break_before)
        });
        let num_id = direct.num_id.clone().or_else(|| {
            chain
                .iter()
                .rev()
                .find_map(|style| style.paragraph.num_id.clone())
        });
        let num_level = direct.num_level.or_else(|| {
            chain
                .iter()
                .rev()
                .find_map(|style| style.paragraph.num_level)
        });

        let mut layers = vec![self.defaults.clone()];
        layers.extend(chain.iter().map(|style| style.run.clone()));
        if let Some(linked) = chain.last().and_then(|style| style.linked.as_deref()) {
            layers.extend(
                self.chain(linked, Some("character"), warnings)
                    .into_iter()
                    .map(|style| style.run.clone()),
            );
        }
        if let Some(mark_run) = p_pr.and_then(|node| node.child("rPr")) {
            layers.push(run_props(mark_run));
        }

        let role = if let Some(level) = outline_level.filter(|level| *level <= 8) {
            format!("h{}", level + 1)
        } else {
            let style_name = chain
                .last()
                .map(|style| style.name.as_str())
                .or(style_id)
                .unwrap_or_default();
            match normalize_name(style_name).as_str() {
                "title" => "title".into(),
                "quote" | "intensequote" => "quote".into(),
                _ => "body".into(),
            }
        };

        ParagraphStyle {
            role,
            page_break_before: page_break_before.unwrap_or(false),
            num_id,
            num_level: num_level.unwrap_or(0).min(8),
            layers,
        }
    }

    pub(crate) fn resolve_run(
        &self,
        paragraph: &ParagraphStyle,
        numbering: Option<&RunProps>,
        direct_node: Option<&Node>,
        theme: &Theme,
        warnings: &mut Vec<String>,
    ) -> (Font, TextColor) {
        let direct = direct_node.map(run_props).unwrap_or_default();
        let mut layers = paragraph.layers.clone();
        if let Some(style_id) = direct.run_style.as_deref() {
            layers.extend(
                self.chain(style_id, Some("character"), warnings)
                    .into_iter()
                    .map(|style| style.run.clone()),
            );
        }
        if let Some(numbering) = numbering {
            layers.push(numbering.clone());
        }
        layers.push(direct.clone());
        resolve_layers(&layers, direct.complex_script, theme)
    }

    fn chain<'a>(
        &'a self,
        id: &str,
        expected_type: Option<&str>,
        warnings: &mut Vec<String>,
    ) -> Vec<&'a Style> {
        let mut reverse = Vec::new();
        let mut visited = BTreeSet::new();
        let mut current = Some(id);
        while let Some(style_id) = current {
            if reverse.len() >= STYLE_DEPTH_LIMIT {
                warnings.push(format!(
                    "style chain depth limit {STYLE_DEPTH_LIMIT} exceeded at {style_id:?}"
                ));
                break;
            }
            if !visited.insert(style_id.to_string()) {
                warnings.push(format!("style basedOn cycle detected at {style_id:?}"));
                break;
            }
            let Some(style) = self.styles.get(style_id) else {
                warnings.push(format!("referenced style {style_id:?} is missing"));
                break;
            };
            if reverse.is_empty()
                && expected_type.is_some_and(|expected| style.style_type != expected)
            {
                warnings.push(format!(
                    "style {style_id:?} has type {:?}, expected {}",
                    style.style_type,
                    expected_type.unwrap_or_default()
                ));
            }
            reverse.push(style);
            current = style.based_on.as_deref();
        }
        reverse.reverse();
        reverse
    }
}

pub(crate) fn run_props(node: &Node) -> RunProps {
    let fonts = node.child("rFonts");
    let color = node.child("color");
    RunProps {
        font: fonts
            .and_then(|node| node.attr("ascii").or_else(|| node.attr("hAnsi")))
            .map(str::to_owned),
        font_theme: fonts
            .and_then(|node| node.attr("asciiTheme").or_else(|| node.attr("hAnsiTheme")))
            .map(str::to_owned),
        font_cs: fonts
            .and_then(|node| node.attr("cs").or_else(|| node.attr("eastAsia")))
            .map(str::to_owned),
        font_cs_theme: fonts
            .and_then(|node| node.attr("cstheme").or_else(|| node.attr("eastAsiaTheme")))
            .map(str::to_owned),
        size: half_points(node.child("sz")),
        size_cs: half_points(node.child("szCs")),
        bold: toggle(node.child("b")),
        italic: toggle(node.child("i")),
        bold_cs: toggle(node.child("bCs")),
        italic_cs: toggle(node.child("iCs")),
        color: color.and_then(|node| node.attr("val")).map(str::to_owned),
        theme_color: color
            .and_then(|node| node.attr("themeColor"))
            .map(str::to_owned),
        theme_tint: color
            .and_then(|node| node.attr("themeTint"))
            .map(str::to_owned),
        theme_shade: color
            .and_then(|node| node.attr("themeShade"))
            .map(str::to_owned),
        complex_script: node.child("rtl").is_some() || node.child("cs").is_some(),
        run_style: child_val(node, "rStyle").map(str::to_owned),
    }
}

fn resolve_layers(layers: &[RunProps], complex: bool, theme: &Theme) -> (Font, TextColor) {
    let mut name = "Calibri".to_string();
    let mut size = 11.0;
    let mut bold = false;
    let mut italic = false;
    let mut color = Some([0, 0, 0]);
    for layer in layers {
        let font = if complex {
            layer.font_cs.as_ref().or(layer.font.as_ref())
        } else {
            layer.font.as_ref()
        };
        let themed = if complex {
            layer.font_cs_theme.as_ref().or(layer.font_theme.as_ref())
        } else {
            layer.font_theme.as_ref()
        };
        if let Some(value) = font {
            name = value.clone();
        } else if let Some(value) = themed.and_then(|value| theme.word_font(value)) {
            name = value;
        }
        if let Some(value) = if complex {
            layer.size_cs.or(layer.size)
        } else {
            layer.size
        } {
            size = value;
        }
        if if complex {
            layer.bold_cs.or(layer.bold)
        } else {
            layer.bold
        }
        .unwrap_or(false)
        {
            bold = !bold;
        }
        if if complex {
            layer.italic_cs.or(layer.italic)
        } else {
            layer.italic
        }
        .unwrap_or(false)
        {
            italic = !italic;
        }
        if layer.color.is_some() || layer.theme_color.is_some() {
            color = theme
                .word_color(
                    layer.color.as_deref(),
                    layer.theme_color.as_deref(),
                    layer.theme_tint.as_deref(),
                    layer.theme_shade.as_deref(),
                )
                .or_else(|| layer.color.as_deref().and_then(parse_hex_color));
        }
    }
    (
        Font {
            name,
            size: round3(size),
            bold,
            italic,
        },
        TextColor {
            fill: color,
            stroke: None,
        },
    )
}

fn paragraph_props(node: &Node) -> ParagraphProps {
    let num_pr = node.child("numPr");
    ParagraphProps {
        outline_level: child_val(node, "outlineLvl").and_then(parse_u32),
        page_break_before: node.child("pageBreakBefore").map(on_off),
        num_id: num_pr
            .and_then(|node| child_val(node, "numId"))
            .map(str::to_owned),
        num_level: num_pr
            .and_then(|node| child_val(node, "ilvl"))
            .and_then(parse_u32),
    }
}

fn half_points(node: Option<&Node>) -> Option<f64> {
    node.and_then(|node| node.attr("val"))
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value / 2.0)
}

fn toggle(node: Option<&Node>) -> Option<bool> {
    node.map(on_off)
}

fn on_off(node: &Node) -> bool {
    !matches!(node.attr("val"), Some("0" | "false" | "off"))
}

fn child_val<'a>(node: &'a Node, child: &str) -> Option<&'a str> {
    node.child(child).and_then(|node| node.attr("val"))
}

fn parse_u32(value: &str) -> Option<u32> {
    value.parse().ok()
}

fn normalize_name(value: &str) -> String {
    value
        .chars()
        .filter(|value| value.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
