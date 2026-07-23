use crate::Node;
use std::collections::BTreeMap;

/// Shared DrawingML theme data used by both presentation and word-processing
/// extractors. Keeping theme interpretation here prevents the two OOXML
/// backends from drifting on the same font and color scheme records.
#[derive(Default)]
pub struct Theme {
    colors: BTreeMap<String, [u8; 3]>,
    major_latin: Option<String>,
    major_east_asia: Option<String>,
    major_complex_script: Option<String>,
    minor_latin: Option<String>,
    minor_east_asia: Option<String>,
    minor_complex_script: Option<String>,
}

impl Theme {
    pub fn from_xml(root: &Node) -> Self {
        let mut theme = Self::default();
        if let Some(scheme) = root.first_descendant("clrScheme") {
            for slot in &scheme.children {
                if let Some(color) = direct_color(slot) {
                    theme.colors.insert(slot.local_name().to_string(), color);
                }
            }
        }
        if let Some(font_scheme) = root.first_descendant("fontScheme") {
            let major = font_scheme.child("majorFont");
            let minor = font_scheme.child("minorFont");
            theme.major_latin = typeface(major, "latin");
            theme.major_east_asia = typeface(major, "ea");
            theme.major_complex_script = typeface(major, "cs");
            theme.minor_latin = typeface(minor, "latin");
            theme.minor_east_asia = typeface(minor, "ea");
            theme.minor_complex_script = typeface(minor, "cs");
        }
        theme
    }

    /// Resolves DrawingML's `+mj-lt` / `+mn-lt` syntax. The Arial fallback is
    /// retained for byte-identical PPTX behavior when a theme omits a face.
    pub fn font(&self, value: &str) -> String {
        match value {
            "+mj-lt" => self.major_latin.clone().unwrap_or_else(|| "Arial".into()),
            "+mj-ea" => self
                .major_east_asia
                .clone()
                .or_else(|| self.major_latin.clone())
                .unwrap_or_else(|| "Arial".into()),
            "+mj-cs" => self
                .major_complex_script
                .clone()
                .or_else(|| self.major_latin.clone())
                .unwrap_or_else(|| "Arial".into()),
            "+mn-lt" => self.minor_latin.clone().unwrap_or_else(|| "Arial".into()),
            "+mn-ea" => self
                .minor_east_asia
                .clone()
                .or_else(|| self.minor_latin.clone())
                .unwrap_or_else(|| "Arial".into()),
            "+mn-cs" => self
                .minor_complex_script
                .clone()
                .or_else(|| self.minor_latin.clone())
                .unwrap_or_else(|| "Arial".into()),
            other => other.to_string(),
        }
    }

    /// Resolves WordprocessingML theme font tokens such as `majorHAnsi` and
    /// `minorBidi`. Empty theme faces are treated as unresolved.
    pub fn word_font(&self, value: &str) -> Option<String> {
        let font = match value {
            "majorAscii" | "majorHAnsi" => self.major_latin.as_ref(),
            "majorEastAsia" => self.major_east_asia.as_ref().or(self.major_latin.as_ref()),
            "majorBidi" => self
                .major_complex_script
                .as_ref()
                .or(self.major_latin.as_ref()),
            "minorAscii" | "minorHAnsi" => self.minor_latin.as_ref(),
            "minorEastAsia" => self.minor_east_asia.as_ref().or(self.minor_latin.as_ref()),
            "minorBidi" => self
                .minor_complex_script
                .as_ref()
                .or(self.minor_latin.as_ref()),
            "+mj-lt" | "+mj-ea" | "+mj-cs" | "+mn-lt" | "+mn-ea" | "+mn-cs" => {
                return Some(self.font(value));
            }
            _ => return None,
        }?;
        (!font.is_empty()).then(|| font.clone())
    }

    pub fn color(&self, slot: &str) -> Option<[u8; 3]> {
        self.colors.get(slot).copied()
    }

    pub fn word_color(
        &self,
        value: Option<&str>,
        theme_color: Option<&str>,
        tint: Option<&str>,
        shade: Option<&str>,
    ) -> Option<[u8; 3]> {
        let color = theme_color.and_then(|slot| self.color(slot)).or_else(|| {
            value
                .filter(|value| *value != "auto")
                .and_then(parse_hex_color)
        })?;
        Some(apply_word_tint_shade(color, tint, shade))
    }
}

fn typeface(parent: Option<&Node>, child: &str) -> Option<String> {
    parent
        .and_then(|node| node.child(child))
        .and_then(|node| node.attr("typeface"))
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

pub fn resolve_drawing_fill(
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
            theme.color(mapped)?
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

pub fn parse_hex_color(value: &str) -> Option<[u8; 3]> {
    if value.len() != 6 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some([
        u8::from_str_radix(&value[0..2], 16).ok()?,
        u8::from_str_radix(&value[2..4], 16).ok()?,
        u8::from_str_radix(&value[4..6], 16).ok()?,
    ])
}

fn direct_color(slot: &Node) -> Option<[u8; 3]> {
    let node = slot.children.first()?;
    match node.local_name() {
        "srgbClr" => parse_hex_color(node.attr("val")?),
        "sysClr" => parse_hex_color(node.attr("lastClr")?),
        _ => None,
    }
}

fn apply_word_tint_shade(mut color: [u8; 3], tint: Option<&str>, shade: Option<&str>) -> [u8; 3] {
    if let Some(value) = shade.and_then(parse_hex_byte) {
        for channel in &mut color {
            *channel = ((u16::from(*channel) * u16::from(value)) / 255) as u8;
        }
    }
    if let Some(value) = tint.and_then(parse_hex_byte) {
        for channel in &mut color {
            let current = u16::from(*channel);
            *channel = (current + ((255 - current) * u16::from(value)) / 255) as u8;
        }
    }
    color
}

fn parse_hex_byte(value: &str) -> Option<u8> {
    (value.len() == 2)
        .then(|| u8::from_str_radix(value, 16).ok())
        .flatten()
}
