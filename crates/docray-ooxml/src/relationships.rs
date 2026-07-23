use crate::{parse, Package};
use docray_core::ExtractError;
use std::collections::BTreeMap;

#[derive(Default, Clone)]
pub struct Relationships {
    by_id: BTreeMap<String, Relationship>,
}

#[derive(Clone)]
pub struct Relationship {
    pub target: String,
    pub kind: String,
    pub external: bool,
}

impl Relationships {
    pub fn get(&self, id: &str) -> Option<&Relationship> {
        self.by_id.get(id)
    }

    pub fn first_internal_type(&self, suffix: &str) -> Option<&Relationship> {
        self.by_id
            .values()
            .find(|relation| !relation.external && relation.kind.rsplit('/').next() == Some(suffix))
    }
}

pub fn relationships(
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

pub fn resolve_target(source_part: &str, target: &str) -> Result<String, ExtractError> {
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

fn parse_failure(message: impl Into<String>) -> ExtractError {
    ExtractError::ParseFailure(message.into())
}
