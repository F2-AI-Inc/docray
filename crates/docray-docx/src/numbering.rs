use crate::styles::{run_props, RunProps};
use docray_model::{ListInfo, ListKind};
use docray_ooxml::Node;
use std::collections::BTreeMap;

#[derive(Clone)]
struct Level {
    start: i64,
    format: String,
    text: String,
    restart: Option<u32>,
    picture: bool,
    run: Option<RunProps>,
}

#[derive(Clone)]
struct Override {
    start: Option<i64>,
    level: Option<Level>,
}

#[derive(Clone)]
struct Num {
    abstract_id: String,
    overrides: BTreeMap<u32, Override>,
}

#[derive(Default)]
pub(crate) struct Numbering {
    abstracts: BTreeMap<String, BTreeMap<u32, Level>>,
    nums: BTreeMap<String, Num>,
}

#[derive(Default)]
pub(crate) struct Counters {
    values: BTreeMap<String, [Option<i64>; 9]>,
}

pub(crate) struct ResolvedList {
    pub(crate) info: ListInfo,
    pub(crate) run: Option<RunProps>,
}

impl Numbering {
    pub(crate) fn from_xml(root: Option<&Node>) -> Self {
        let mut numbering = Self::default();
        let Some(root) = root else {
            return numbering;
        };
        for abstract_num in root.children_named("abstractNum") {
            let Some(id) = abstract_num.attr("abstractNumId") else {
                continue;
            };
            let mut levels = BTreeMap::new();
            for level in abstract_num.children_named("lvl") {
                if let Some(index) = level.attr("ilvl").and_then(|value| value.parse().ok()) {
                    levels.insert(index, parse_level(level));
                }
            }
            numbering.abstracts.insert(id.to_string(), levels);
        }
        for num in root.children_named("num") {
            let (Some(id), Some(abstract_id)) = (
                num.attr("numId"),
                child_val(num, "abstractNumId").map(str::to_owned),
            ) else {
                continue;
            };
            let mut overrides = BTreeMap::new();
            for node in num.children_named("lvlOverride") {
                let Some(level) = node.attr("ilvl").and_then(|value| value.parse().ok()) else {
                    continue;
                };
                overrides.insert(
                    level,
                    Override {
                        start: child_val(node, "startOverride")
                            .and_then(|value| value.parse().ok()),
                        level: node.child("lvl").map(parse_level),
                    },
                );
            }
            numbering.nums.insert(
                id.to_string(),
                Num {
                    abstract_id,
                    overrides,
                },
            );
        }
        numbering
    }

    pub(crate) fn next(
        &self,
        counters: &mut Counters,
        num_id: &str,
        level_index: u32,
        warnings: &mut Vec<String>,
    ) -> Option<ResolvedList> {
        let level_index = level_index.min(8);
        let num = match self.nums.get(num_id) {
            Some(num) => num,
            None => {
                warnings.push(format!("numbering instance {num_id:?} is missing"));
                return None;
            }
        };
        let levels = match self.abstracts.get(&num.abstract_id) {
            Some(levels) => levels,
            None => {
                warnings.push(format!(
                    "abstract numbering definition {:?} for numId {num_id:?} is missing",
                    num.abstract_id
                ));
                return None;
            }
        };
        let level = resolved_level(levels, num, level_index, warnings)?;
        let values = counters.values.entry(num_id.to_string()).or_default();
        let index = level_index as usize;
        values[index] = Some(match values[index] {
            Some(value) => value.saturating_add(1),
            None => level.start,
        });

        // A level normally restarts when its immediately preceding level is
        // advanced. lvlRestart names a different trigger; zero means never.
        let deepest = levels
            .keys()
            .chain(num.overrides.keys())
            .copied()
            .max()
            .unwrap_or(0)
            .min(8) as usize;
        for (deeper, value) in values
            .iter_mut()
            .enumerate()
            .take(deepest + 1)
            .skip(index + 1)
        {
            let Some(deeper_level) = resolved_level(levels, num, deeper as u32, warnings) else {
                continue;
            };
            let trigger = deeper_level.restart.unwrap_or(deeper as u32);
            if trigger != 0 && index <= trigger.saturating_sub(1) as usize {
                *value = None;
            }
        }

        let mut picture = level.picture;
        let label = substitute_label(&level.text, values, levels, num, warnings, &mut picture);
        let kind = if picture || level.format == "bullet" {
            ListKind::Bullet
        } else {
            ListKind::Ordered
        };
        let label = if picture {
            warnings.push(format!(
                "picture bullet for numId {num_id:?} level {level_index} rendered as a bullet"
            ));
            "•".into()
        } else {
            label
        };
        Some(ResolvedList {
            info: ListInfo {
                list_id: num_id.to_string(),
                level: level_index,
                kind,
                label,
            },
            run: level.run,
        })
    }
}

fn resolved_level(
    levels: &BTreeMap<u32, Level>,
    num: &Num,
    index: u32,
    warnings: &mut Vec<String>,
) -> Option<Level> {
    let base = levels.get(&index).cloned();
    let override_value = num.overrides.get(&index);
    let mut level = override_value
        .and_then(|value| value.level.clone())
        .or(base);
    if let (Some(level), Some(start)) =
        (level.as_mut(), override_value.and_then(|value| value.start))
    {
        level.start = start;
    }
    if level.is_none() {
        warnings.push(format!(
            "numbering level {index} is missing from abstract definition {:?}",
            num.abstract_id
        ));
    }
    level
}

fn parse_level(node: &Node) -> Level {
    Level {
        start: child_val(node, "start")
            .and_then(|value| value.parse().ok())
            .unwrap_or(1),
        format: child_val(node, "numFmt").unwrap_or("decimal").to_string(),
        text: child_val(node, "lvlText").unwrap_or("%1.").to_string(),
        restart: child_val(node, "lvlRestart").and_then(|value| value.parse().ok()),
        picture: node.child("lvlPicBulletId").is_some(),
        run: node.child("rPr").map(run_props),
    }
}

fn substitute_label(
    template: &str,
    values: &[Option<i64>; 9],
    levels: &BTreeMap<u32, Level>,
    num: &Num,
    warnings: &mut Vec<String>,
    picture: &mut bool,
) -> String {
    let mut output = String::new();
    let mut chars = template.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '%' {
            if let Some(digit @ '1'..='9') = chars.peek().copied() {
                chars.next();
                let index = digit as usize - '1' as usize;
                let level = resolved_level(levels, num, index as u32, warnings);
                if level.as_ref().is_some_and(|level| level.picture) {
                    *picture = true;
                }
                let value = values[index]
                    .or_else(|| level.as_ref().map(|level| level.start))
                    .unwrap_or(1);
                output.push_str(&format_number(
                    value,
                    level
                        .as_ref()
                        .map(|level| level.format.as_str())
                        .unwrap_or("decimal"),
                    warnings,
                ));
                continue;
            }
        }
        output.push(character);
    }
    output
}

fn format_number(value: i64, format: &str, warnings: &mut Vec<String>) -> String {
    match format {
        "decimal" => value.to_string(),
        "lowerLetter" => letters(value, false),
        "upperLetter" => letters(value, true),
        "lowerRoman" => roman(value).to_ascii_lowercase(),
        "upperRoman" => roman(value),
        "bullet" => "•".into(),
        unsupported => {
            warnings.push(format!(
                "unsupported numbering format {unsupported:?}; decimal fallback used"
            ));
            value.to_string()
        }
    }
}

fn letters(value: i64, uppercase: bool) -> String {
    if value <= 0 {
        return value.to_string();
    }
    let mut value = value as u64;
    let mut output = Vec::new();
    while value > 0 {
        value -= 1;
        output.push((b'a' + (value % 26) as u8) as char);
        value /= 26;
    }
    output.reverse();
    let output: String = output.into_iter().collect();
    if uppercase {
        output.to_ascii_uppercase()
    } else {
        output
    }
}

fn roman(value: i64) -> String {
    if !(1..=3999).contains(&value) {
        return value.to_string();
    }
    let mut value = value;
    let mut output = String::new();
    for (amount, digits) in [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ] {
        while value >= amount {
            output.push_str(digits);
            value -= amount;
        }
    }
    output
}

fn child_val<'a>(node: &'a Node, child: &str) -> Option<&'a str> {
    node.child(child).and_then(|node| node.attr("val"))
}
