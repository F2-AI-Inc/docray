use docray_core::ExtractError;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::BTreeMap;

const MAX_XML_DEPTH: usize = 256;

#[derive(Debug, Clone)]
pub(crate) struct Node {
    pub(crate) name: String,
    pub(crate) attrs: BTreeMap<String, String>,
    pub(crate) children: Vec<Node>,
    pub(crate) text: String,
}

impl Node {
    pub(crate) fn local_name(&self) -> &str {
        local_name(&self.name)
    }

    pub(crate) fn attr(&self, name: &str) -> Option<&str> {
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

    pub(crate) fn child(&self, local: &str) -> Option<&Node> {
        self.children.iter().find(|node| node.local_name() == local)
    }

    pub(crate) fn children_named<'a>(&'a self, local: &'a str) -> impl Iterator<Item = &'a Node> {
        self.children
            .iter()
            .filter(move |node| node.local_name() == local)
    }

    pub(crate) fn descendants<'a, 'b>(&'a self, local: &'b str) -> Descendants<'a, 'b> {
        Descendants {
            stack: self.children.iter().rev().collect(),
            local,
        }
    }

    pub(crate) fn first_descendant(&self, local: &str) -> Option<&Node> {
        self.descendants(local).next()
    }
}

pub(crate) struct Descendants<'a, 'b> {
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

pub(crate) fn parse(bytes: &[u8], part: &str) -> Result<Node, ExtractError> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().check_end_names = true;
    reader.config_mut().trim_text(false);
    let mut stack: Vec<Node> = Vec::new();
    let mut root = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) => {
                if stack.len() >= MAX_XML_DEPTH {
                    return Err(parse_error(part, "XML nesting depth limit exceeded"));
                }
                stack.push(Node {
                    name: decode(start.name().as_ref(), part)?,
                    attrs: attributes(&start, reader.decoder(), part)?,
                    children: Vec::new(),
                    text: String::new(),
                });
            }
            Ok(Event::Empty(empty)) => {
                let node = Node {
                    name: decode(empty.name().as_ref(), part)?,
                    attrs: attributes(&empty, reader.decoder(), part)?,
                    children: Vec::new(),
                    text: String::new(),
                };
                attach(node, &mut stack, &mut root, part)?;
            }
            Ok(Event::End(_)) => {
                let node = stack
                    .pop()
                    .ok_or_else(|| parse_error(part, "unexpected XML end tag"))?;
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

pub(crate) fn local_name(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, local)| local)
}
