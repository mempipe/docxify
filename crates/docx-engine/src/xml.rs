use std::ops::Range;

use roxmltree::{Node, ParsingOptions};

use crate::error::OpenError;

pub const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
pub const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
pub const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
pub const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
pub const W15: &str = "http://schemas.microsoft.com/office/word/2012/wordml";
pub const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
pub const WP: &str = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing";
pub const PKG_RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
pub const CONTENT_TYPES: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
/// ISO 29500 Strict uses its own namespaces; the engine reads Transitional only.
pub const W_STRICT: &str = "http://purl.oclc.org/ooxml/wordprocessingml/main";

/// A decoded XML part. Byte offsets in the IR index into `text`; the BOM is
/// kept out of `text` and written back on export.
#[derive(Debug, Clone)]
pub struct XmlText {
    pub text: std::sync::Arc<str>,
    pub bom: bool,
}

pub fn decode(part: &str, bytes: Vec<u8>) -> Result<XmlText, OpenError> {
    if bytes.starts_with(&[0xFE, 0xFF]) || bytes.starts_with(&[0xFF, 0xFE]) {
        return Err(OpenError::Unsupported {
            part: part.to_string(),
            reason: "UTF-16 encoded XML".to_string(),
        });
    }
    let (bom, body) = match bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        Some(rest) => (true, rest.to_vec()),
        None => (false, bytes),
    };
    let text = String::from_utf8(body).map_err(|e| OpenError::Xml {
        part: part.to_string(),
        message: format!("invalid UTF-8 at byte {}", e.utf8_error().valid_up_to()),
    })?;
    Ok(XmlText {
        text: text.into(),
        bom,
    })
}

pub fn parse<'a>(part: &str, text: &'a str) -> Result<roxmltree::Document<'a>, OpenError> {
    // DTDs stay disallowed (the default): OOXML never needs one, and refusing
    // them rules out entity-expansion attacks.
    let options = ParsingOptions {
        allow_dtd: false,
        ..ParsingOptions::default()
    };
    roxmltree::Document::parse_with_options(text, options).map_err(|e| OpenError::Xml {
        part: part.to_string(),
        message: e.to_string(),
    })
}

pub fn is(node: Node, ns: &str, local: &str) -> bool {
    node.is_element() && node.tag_name().namespace() == Some(ns) && node.tag_name().name() == local
}

/// The local name of a WordprocessingML element.
pub fn w_local<'i>(node: Node<'_, 'i>) -> Option<&'i str> {
    let tag = node.tag_name();
    (node.is_element() && tag.namespace() == Some(W)).then(|| tag.name())
}

pub fn child<'a, 'i>(node: Node<'a, 'i>, ns: &str, local: &str) -> Option<Node<'a, 'i>> {
    node.children().find(|c| is(*c, ns, local))
}

pub fn elements<'a, 'i>(node: Node<'a, 'i>) -> impl Iterator<Item = Node<'a, 'i>> {
    node.children().filter(|c| c.is_element())
}

pub fn w_attr<'a>(node: Node<'a, '_>, local: &str) -> Option<&'a str> {
    node.attribute((W, local))
}

pub fn w_val<'a>(node: Node<'a, '_>, child_local: &str) -> Option<&'a str> {
    child(node, W, child_local).and_then(|c| w_attr(c, "val"))
}

/// A WordprocessingML on/off property: present with no `w:val`, or with a
/// true value, means on.
pub fn on_off(node: Node, child_local: &str) -> Option<bool> {
    child(node, W, child_local)
        .map(|c| !matches!(w_attr(c, "val"), Some("0" | "false" | "off" | "none")))
}

/// The element's qualified name exactly as the source spells it, so
/// regenerated markup keeps the document's own namespace prefixes.
pub fn qname<'t>(text: &'t str, range: &Range<usize>) -> &'t str {
    let tag = &text[range.start + 1..range.end];
    let end = tag
        .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
        .unwrap_or(tag.len());
    &tag[..end]
}

pub fn display_name(node: Node) -> String {
    let tag = node.tag_name();
    match tag.namespace().and_then(|ns| node.lookup_prefix(ns)) {
        Some(prefix) if !prefix.is_empty() => format!("{prefix}:{}", tag.name()),
        _ => tag.name().to_string(),
    }
}

pub fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            // A literal CR would be normalized to LF by the next reader.
            '\r' => out.push_str("&#13;"),
            _ => out.push(c),
        }
    }
    out
}

pub fn escape_attr(s: &str) -> String {
    escape_text(s).replace('"', "&quot;")
}

/// Characters XML 1.0 cannot carry at all.
pub fn is_xml_char(c: char) -> bool {
    matches!(c, '\u{9}' | '\u{A}' | '\u{D}' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..)
}

/// XML comments and processing instructions can separate character data in one leaf.
pub fn character_data(node: Node) -> String {
    node.children()
        .filter(Node::is_text)
        .filter_map(|n| n.text())
        .collect()
}

/// Concatenated `w:t` text below an element, for reporting what an opaque
/// subtree contains without modelling it.
pub fn text_preview(node: Node) -> Option<String> {
    let text: String = node
        .descendants()
        .filter(|d| is(*d, W, "t"))
        .map(character_data)
        .collect();
    if text.is_empty() {
        None
    } else {
        Some(text.chars().take(200).collect())
    }
}

/// The qualified name of the element whose start tag `markup` begins with.
pub fn start_tag_name(markup: &str) -> &str {
    let name = markup.strip_prefix('<').unwrap_or(markup);
    let end = name.find(|c: char| c.is_whitespace() || c == '>' || c == '/');
    &name[..end.unwrap_or(name.len())]
}

/// Report each relationship reference in this XML part that has no target ID.
/// Some producers emit empty optional references. They name no relationship.
pub(crate) fn check_relationship_ids(
    document: &roxmltree::Document<'_>,
    exists: impl Fn(&str) -> bool,
    mut missing: impl FnMut(String),
) {
    for element in document.descendants().filter(|node| node.is_element()) {
        for attr in element
            .attributes()
            .filter(|attr| attr.namespace() == Some(R))
        {
            if !attr.value().is_empty() && !exists(attr.value()) {
                missing(format!(
                    "r:{}=\"{}\" on {} has no relationship",
                    attr.name(),
                    attr.value(),
                    element.tag_name().name()
                ));
            }
        }
    }
}
