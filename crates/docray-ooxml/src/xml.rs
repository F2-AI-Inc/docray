use docray_core::ExtractError;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::BTreeMap;

pub const MAX_XML_DEPTH: usize = 256;

/// Bounds the parsed DOM independently of inflated byte size. A part's inflated
/// bytes are already capped, but the `Node` tree is ~10-25x larger than its
/// source, and the OPC compression-ratio guard is computed from spoofable
/// central-directory sizes. This cap turns a DOM-amplification attempt into a
/// clean `parse_failure` instead of relying on the worker memory rlimit.
pub const MAX_XML_NODES: usize = 5_000_000;

type Namespaces = BTreeMap<String, String>;

#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    pub attrs: BTreeMap<String, String>,
    pub children: Vec<Node>,
    pub text: String,
    namespace_uri: Option<String>,
    attr_namespace_uris: BTreeMap<String, String>,
    in_scope_namespaces: Namespaces,
}

impl Node {
    pub fn local_name(&self) -> &str {
        local_name(&self.name)
    }

    pub fn namespace_uri(&self) -> Option<&str> {
        self.namespace_uri.as_deref()
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .get(name)
            .or_else(|| {
                self.attrs
                    .iter()
                    .find(|(key, _)| local_name(key) == name)
                    .map(|(_, value)| value)
            })
            .map(String::as_str)
    }

    pub fn attr_ns(&self, namespace_uri: &str, local: &str) -> Option<&str> {
        self.attrs.iter().find_map(|(key, value)| {
            (local_name(key) == local
                && self.attr_namespace_uris.get(key).map(String::as_str) == Some(namespace_uri))
            .then_some(value.as_str())
        })
    }

    pub fn namespace_uri_for_prefix(&self, prefix: &str) -> Option<&str> {
        self.in_scope_namespaces.get(prefix).map(String::as_str)
    }

    pub fn child(&self, local: &str) -> Option<&Node> {
        self.children.iter().find(|node| node.local_name() == local)
    }

    pub fn child_ns(&self, namespace_uri: &str, local: &str) -> Option<&Node> {
        self.children
            .iter()
            .find(|node| node.local_name() == local && node.namespace_uri() == Some(namespace_uri))
    }

    pub fn children_named<'a>(&'a self, local: &'a str) -> impl Iterator<Item = &'a Node> {
        self.children
            .iter()
            .filter(move |node| node.local_name() == local)
    }

    pub fn descendants<'a, 'b>(&'a self, local: &'b str) -> Descendants<'a, 'b> {
        Descendants {
            stack: self.children.iter().rev().collect(),
            local,
        }
    }

    pub fn first_descendant(&self, local: &str) -> Option<&Node> {
        self.descendants(local).next()
    }
}

pub struct Descendants<'a, 'b> {
    stack: Vec<&'a Node>,
    local: &'b str,
}

impl<'a, 'b> Iterator for Descendants<'a, 'b> {
    type Item = &'a Node;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(node) = self.stack.pop() {
            self.stack.extend(node.children.iter().rev());
            if node.local_name() == self.local {
                return Some(node);
            }
        }
        None
    }
}

pub fn parse(bytes: &[u8], part: &str) -> Result<Node, ExtractError> {
    parse_with_limits(bytes, part, MAX_XML_NODES)
}

fn parse_with_limits(bytes: &[u8], part: &str, max_nodes: usize) -> Result<Node, ExtractError> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().check_end_names = true;
    reader.config_mut().trim_text(false);
    let mut stack: Vec<Node> = Vec::new();
    let mut namespace_stack: Vec<Namespaces> = Vec::new();
    let mut root = None;
    let mut node_count = 0_usize;

    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) => {
                if stack.len() >= MAX_XML_DEPTH {
                    return Err(parse_error(part, "XML nesting depth limit exceeded"));
                }
                node_count += 1;
                if node_count > max_nodes {
                    return Err(parse_error(part, "XML element count limit exceeded"));
                }
                let attrs = attributes(&start, reader.decoder(), part)?;
                let namespaces = namespaces_for_element(namespace_stack.last(), &attrs);
                stack.push(make_node(
                    decode(start.name().as_ref(), part)?,
                    attrs,
                    &namespaces,
                ));
                namespace_stack.push(namespaces);
            }
            Ok(Event::Empty(empty)) => {
                node_count += 1;
                if node_count > max_nodes {
                    return Err(parse_error(part, "XML element count limit exceeded"));
                }
                let attrs = attributes(&empty, reader.decoder(), part)?;
                let namespaces = namespaces_for_element(namespace_stack.last(), &attrs);
                let node = make_node(decode(empty.name().as_ref(), part)?, attrs, &namespaces);
                attach(node, &mut stack, &mut root, part)?;
            }
            Ok(Event::End(_)) => {
                let node = stack
                    .pop()
                    .ok_or_else(|| parse_error(part, "unexpected XML end tag"))?;
                namespace_stack.pop();
                attach(node, &mut stack, &mut root, part)?;
            }
            Ok(Event::Text(text)) => {
                if let Some(node) = stack.last_mut() {
                    let decoded = decode(text.as_ref(), part)?;
                    node.text.push_str(&decode_predefined_entities(&decoded));
                }
            }
            Ok(Event::CData(text)) => {
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(&decode(text.as_ref(), part)?);
                }
            }
            Ok(Event::GeneralRef(reference)) => {
                if let Some(node) = stack.last_mut() {
                    let name = decode(reference.as_ref(), part)?;
                    node.text.push_str(&decode_reference(&name));
                }
            }
            // DTD declarations are deliberately ignored. Unknown entity
            // references remain literal text; no external resolver exists.
            Ok(Event::DocType(_) | Event::Decl(_) | Event::PI(_) | Event::Comment(_)) => {}
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(parse_error(part, &format!("malformed XML: {error}")));
            }
        }
    }
    if !stack.is_empty() {
        return Err(parse_error(part, "unclosed XML element"));
    }
    root.ok_or_else(|| parse_error(part, "XML document has no root element"))
}

fn make_node(name: String, attrs: BTreeMap<String, String>, namespaces: &Namespaces) -> Node {
    let namespace_uri = namespace_for_name(&name, namespaces, false).map(str::to_owned);
    let attr_namespace_uris = attrs
        .keys()
        .filter_map(|key| {
            namespace_for_name(key, namespaces, true).map(|uri| (key.clone(), uri.to_owned()))
        })
        .collect();
    Node {
        name,
        attrs,
        children: Vec::new(),
        text: String::new(),
        namespace_uri,
        attr_namespace_uris,
        in_scope_namespaces: namespaces.clone(),
    }
}

/// Resolves `mc:AlternateContent` before format-specific extraction. A chosen
/// branch is flattened into the parent exactly once; unsupported constructs
/// therefore cannot leak both Choice and Fallback text into visible output.
pub fn preprocess_alternate_content(
    root: &Node,
    supported_namespace_uris: &[&str],
    max_depth: usize,
    warnings: &mut Vec<String>,
) -> Node {
    let supported: std::collections::BTreeSet<&str> =
        supported_namespace_uris.iter().copied().collect();
    let mut nodes = preprocess_node(root, &supported, 0, max_depth, warnings);
    nodes.pop().unwrap_or_else(|| {
        let mut empty = root.clone();
        empty.children.clear();
        empty
    })
}

fn preprocess_node(
    node: &Node,
    supported: &std::collections::BTreeSet<&str>,
    mc_depth: usize,
    max_depth: usize,
    warnings: &mut Vec<String>,
) -> Vec<Node> {
    const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
    if node.local_name() == "AlternateContent" && node.namespace_uri() == Some(MC) {
        if mc_depth >= max_depth {
            warnings.push(format!(
                "markup-compatibility nesting depth limit {max_depth} exceeded; subtree skipped"
            ));
            return Vec::new();
        }
        let choice = node.children.iter().find(|candidate| {
            if candidate.local_name() != "Choice" || candidate.namespace_uri() != Some(MC) {
                return false;
            }
            let Some(requires) = candidate.attr("Requires") else {
                return false;
            };
            let mut any = false;
            for prefix in requires.split_whitespace() {
                any = true;
                let Some(uri) = candidate.namespace_uri_for_prefix(prefix) else {
                    return false;
                };
                if !supported.contains(uri) {
                    return false;
                }
            }
            any
        });
        let selected = choice.or_else(|| {
            node.children.iter().find(|candidate| {
                candidate.local_name() == "Fallback" && candidate.namespace_uri() == Some(MC)
            })
        });
        let Some(selected) = selected else {
            warnings.push(
                "markup-compatibility AlternateContent has no supported Choice or Fallback; subtree skipped"
                    .into(),
            );
            return Vec::new();
        };
        return selected
            .children
            .iter()
            .flat_map(|child| preprocess_node(child, supported, mc_depth + 1, max_depth, warnings))
            .collect();
    }

    let mut clone = node.clone();
    clone.children = node
        .children
        .iter()
        .flat_map(|child| preprocess_node(child, supported, mc_depth, max_depth, warnings))
        .collect();
    vec![clone]
}

fn namespaces_for_element(
    parent: Option<&Namespaces>,
    attrs: &BTreeMap<String, String>,
) -> Namespaces {
    let mut namespaces = parent.cloned().unwrap_or_else(|| {
        BTreeMap::from([(
            "xml".to_string(),
            "http://www.w3.org/XML/1998/namespace".to_string(),
        )])
    });
    for (key, value) in attrs {
        if key == "xmlns" {
            if value.is_empty() {
                namespaces.remove("");
            } else {
                namespaces.insert(String::new(), value.clone());
            }
        } else if let Some(prefix) = key.strip_prefix("xmlns:") {
            if value.is_empty() {
                namespaces.remove(prefix);
            } else {
                namespaces.insert(prefix.to_string(), value.clone());
            }
        }
    }
    namespaces
}

fn namespace_for_name<'a>(
    name: &str,
    namespaces: &'a Namespaces,
    attribute: bool,
) -> Option<&'a str> {
    if name == "xmlns" || name.starts_with("xmlns:") {
        return Some("http://www.w3.org/2000/xmlns/");
    }
    match name.rsplit_once(':') {
        Some((prefix, _)) => namespaces.get(prefix).map(String::as_str),
        None if !attribute => namespaces.get("").map(String::as_str),
        None => None,
    }
}

fn attributes(
    element: &quick_xml::events::BytesStart<'_>,
    decoder: quick_xml::encoding::Decoder,
    part: &str,
) -> Result<BTreeMap<String, String>, ExtractError> {
    let mut attrs = BTreeMap::new();
    for attr in element.attributes().with_checks(true) {
        let attr =
            attr.map_err(|error| parse_error(part, &format!("invalid XML attribute: {error}")))?;
        let key = decode(attr.key.as_ref(), part)?;
        let value = attr
            .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, decoder)
            .map_err(|error| parse_error(part, &format!("invalid XML attribute value: {error}")))?
            .into_owned();
        attrs.insert(key, value);
    }
    Ok(attrs)
}

fn attach(
    node: Node,
    stack: &mut [Node],
    root: &mut Option<Node>,
    part: &str,
) -> Result<(), ExtractError> {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(node);
    } else if root.replace(node).is_some() {
        return Err(parse_error(part, "XML document has multiple root elements"));
    }
    Ok(())
}

fn decode(bytes: &[u8], part: &str) -> Result<String, ExtractError> {
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|error| parse_error(part, &format!("XML is not UTF-8: {error}")))
}

fn decode_predefined_entities(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn decode_reference(name: &str) -> String {
    match name {
        "lt" => "<".into(),
        "gt" => ">".into(),
        "quot" => "\"".into(),
        "apos" => "'".into(),
        "amp" => "&".into(),
        value if value.starts_with("#x") => u32::from_str_radix(&value[2..], 16)
            .ok()
            .and_then(char::from_u32)
            .map(String::from)
            .unwrap_or_else(|| format!("&{name};")),
        value if value.starts_with('#') => value[1..]
            .parse::<u32>()
            .ok()
            .and_then(char::from_u32)
            .map(String::from)
            .unwrap_or_else(|| format!("&{name};")),
        _ => format!("&{name};"),
    }
}

fn parse_error(part: &str, message: &str) -> ExtractError {
    ExtractError::ParseFailure(format!("{part}: {message}"))
}

pub fn local_name(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, local)| local)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_count_over_limit_is_a_parse_failure_not_a_panic() {
        // 50 sibling elements against a limit of 8 must be rejected cleanly.
        let mut xml = String::from("<root>");
        for _ in 0..50 {
            xml.push_str("<a/>");
        }
        xml.push_str("</root>");

        let err = parse_with_limits(xml.as_bytes(), "test.xml", 8).unwrap_err();
        let ExtractError::ParseFailure(message) = err else {
            panic!("expected parse_failure");
        };
        assert!(
            message.contains("element count limit exceeded"),
            "{message}"
        );
    }

    #[test]
    fn node_count_at_limit_parses() {
        // root + 4 children = 5 nodes, exactly at the limit.
        let node = parse_with_limits(b"<root><a/><a/><a/><a/></root>", "test.xml", 5).unwrap();
        assert_eq!(node.local_name(), "root");
        assert_eq!(node.children.len(), 4);
    }

    #[test]
    fn namespace_lookups_resolve_in_scope_prefixes() {
        let root = parse(
            br#"<w:document xmlns:w="urn:word" xmlns:r="urn:rels"><w:body><w:p r:id="rId1"/><other:p r:id="wrong" xmlns:other="urn:other"/></w:body></w:document>"#,
            "test.xml",
        )
        .unwrap();
        let body = root.child_ns("urn:word", "body").unwrap();
        let paragraph = body.child_ns("urn:word", "p").unwrap();
        assert_eq!(paragraph.namespace_uri(), Some("urn:word"));
        assert_eq!(paragraph.attr_ns("urn:rels", "id"), Some("rId1"));
        assert!(body.child_ns("urn:missing", "p").is_none());
    }
}
