//! Updating the body's fields as Word does when every field is selected and
//! updated: `UpdateFields` works out each result it can, and
//! `SetFieldResult` sets one.
//!
//! The engine works out `PAGE`, `NUMPAGES` and `PAGEREF` from the pages a
//! layout gives it, and `DATE` and `TIME` from the time an editor gives
//! it. It leaves other fields, and fields Word keeps locked, as they are.
//! A table of contents keeps its entries; the `PAGEREF` fields in them
//! update, as Word's "Update page numbers only" does.
//!
//! A result is set only when it is text in the field's own paragraph: the
//! first text node takes the new result and the others are emptied, so the
//! runs keep their formatting.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::dates::{Date, DEFAULT_PICTURE, DEFAULT_TIME_PICTURE};
use crate::edit::check_text;
use crate::error::EditError;
use crate::import::{IN_CONTENT_CONTROL, IN_FIELD};
use crate::ir::{NodeId, NodeKind, StoryKind, Support};
use crate::structure::refuse;
use crate::Document;

/// Where a layout put the body, for the page fields.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pages {
    /// How many pages the document has.
    pub total: u32,
    /// The page each body paragraph starts on, by the paragraph's ID, as
    /// its section numbers pages: `iii` on a page numbered in roman.
    pub numbers: BTreeMap<NodeId, String>,
}

/// A field's new result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldUpdate {
    /// The field's begin character, or the simple field.
    pub field: NodeId,
    pub text: String,
}

/// A page number a layout draws in place of the result a field holds.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PageField {
    /// The field's begin character or simple field. Every text node of one
    /// result names the same field; the number is drawn once for them.
    pub field: NodeId,
    pub number: PageNumber,
    /// The number format the instruction asks for, as `w:numFmt` names it.
    /// Without one, a page number takes its section's format.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<&'static str>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PageNumber {
    /// The page the field is drawn on.
    Page,
    /// How many pages the document has.
    NumPages,
    /// How many pages the field's section has.
    SectionPages,
}

/// What the file says of a field, read once when it opens.
#[derive(Debug, Clone)]
pub(crate) struct FieldInfo {
    pub instruction: Arc<str>,
    /// `w:fldLock`: Word keeps the result as it is.
    pub locked: bool,
}

impl FieldInfo {
    /// What a layout draws for the field `field`, unless the file keeps its
    /// result or the engine does not work the instruction out.
    pub(crate) fn page_field(&self, field: NodeId) -> Option<PageField> {
        let (number, format) = match kind(&self.instruction)? {
            Kind::Page(format) => (PageNumber::Page, format),
            Kind::NumPages(format) => (PageNumber::NumPages, format),
            Kind::SectionPages(format) => (PageNumber::SectionPages, format),
            Kind::PageRef(_) | Kind::Date(_) => return None,
        };
        (!self.locked).then_some(PageField {
            field,
            number,
            format,
        })
    }
}

/// Every field by its begin character or simple field, and for each text
/// node in a result, the innermost field it is the result of.
#[derive(Debug, Clone, Default)]
pub(crate) struct FieldIndex {
    pub fields: BTreeMap<NodeId, FieldInfo>,
    pub results: BTreeMap<NodeId, NodeId>,
}

const LOCKED: &str = "the field is locked; Word keeps its result as it is";
const NOT_PLAIN: &str = "its result is more than text in its paragraph; no command sets it yet";

/// A field and the text nodes of its result.
struct Found {
    field: NodeId,
    instruction: String,
    results: Vec<NodeId>,
    /// Why neither `UpdateFields` nor `SetFieldResult` sets the result. The
    /// engine sets one that is text in the field's paragraph, where nothing
    /// around it keeps it as it is.
    refusal: Option<&'static str>,
}

/// A field the engine works out. A page number carries the number format a
/// `\*` switch gives it, as `w:numFmt` names it.
enum Kind {
    Page(Option<&'static str>),
    NumPages(Option<&'static str>),
    SectionPages(Option<&'static str>),
    PageRef(String),
    Date(String),
}

impl Document {
    /// The results `UpdateFields` would give the body's fields, for those
    /// it would change. `now` is the local date and time, written
    /// `YYYY-MM-DDThh:mm:ss`. Without `pages`, the page fields keep their
    /// results.
    pub fn field_updates(
        &self,
        now: &str,
        pages: Option<&Pages>,
    ) -> Result<Vec<FieldUpdate>, EditError> {
        let now = Date::parse_time(now).ok_or(EditError::Fields {
            reason: "need the time written YYYY-MM-DDThh:mm:ss",
        })?;
        let Some(body) = self.body_root() else {
            return Ok(Vec::new());
        };
        let order = self.descendants(body);
        let bookmarks: BTreeMap<&str, NodeId> = order
            .iter()
            .filter_map(|&id| match &self.n(id).kind {
                NodeKind::Marker {
                    element,
                    name: Some(name),
                    ..
                } if element.rsplit(':').next() == Some("bookmarkStart") => {
                    Some((name.as_str(), id))
                }
                _ => None,
            })
            .collect();
        let page_of = |id: NodeId| {
            let paragraph = self.paragraph_of(id)?;
            pages?.numbers.get(&paragraph).cloned()
        };
        let mut updates = Vec::new();
        for paragraph in order
            .iter()
            .filter(|&&id| matches!(self.n(id).kind, NodeKind::Paragraph(_)))
        {
            for found in self.fields_in(*paragraph) {
                if found.refusal.is_some() {
                    continue;
                }
                // The layout gives each page as its section numbers it, and
                // no section totals, so a field with a format of its own or a
                // section's page count keeps its result.
                let text = match kind(&found.instruction) {
                    Some(Kind::Page(None)) => page_of(found.field),
                    Some(Kind::NumPages(None)) => pages.map(|p| p.total.to_string()),
                    Some(Kind::Page(_) | Kind::NumPages(_) | Kind::SectionPages(_)) => None,
                    Some(Kind::PageRef(name)) => {
                        bookmarks.get(name.as_str()).and_then(|&b| page_of(b))
                    }
                    Some(Kind::Date(picture)) => Some(now.format(&picture)),
                    None => None,
                };
                let Some(text) = text.filter(|t| check_text(t).is_ok()) else {
                    continue;
                };
                if self.result_text(&found) != text {
                    updates.push(FieldUpdate {
                        field: found.field,
                        text,
                    });
                }
            }
        }
        Ok(updates)
    }

    /// The text a body field's result shows, by its begin character or
    /// simple field, when `SetFieldResult` can set it.
    pub fn field_result(&self, field: NodeId) -> Option<String> {
        self.attached(field).ok()?;
        let paragraph = self.paragraph_of(field).filter(|_| self.in_body(field))?;
        let found = self
            .fields_in(paragraph)
            .into_iter()
            .find(|f| f.field == field && f.refusal.is_none())?;
        Some(self.result_text(&found))
    }

    pub(crate) fn field_result_nodes(
        &self,
        field: NodeId,
        text: &str,
    ) -> Result<Vec<NodeId>, EditError> {
        self.attached(field)?;
        check_text(text)?;
        let found = self
            .paragraph_of(field)
            .filter(|_| self.in_body(field))
            .and_then(|p| self.fields_in(p).into_iter().find(|f| f.field == field));
        let Some(found) = found else {
            return Err(EditError::WrongKind {
                node: field,
                kind: self.n(field).kind.name(),
                expected: "a field's begin character, or a simple field, in the body",
            });
        };
        match found.refusal {
            Some(reason) => refuse(field, reason),
            None => Ok(found.results),
        }
    }

    pub(crate) fn compose_text(&mut self, id: NodeId, text: &str) -> Result<String, EditError> {
        crate::edit::check_xml_text(text)?;
        let node = &mut self.nodes[id.index()];
        let NodeKind::Text { text: current } = &mut node.kind else {
            return Err(EditError::NotText {
                node: id,
                kind: node.kind.name(),
            });
        };
        let old = std::mem::replace(current, text.to_string());
        node.dirty = true;
        self.mark_ancestors(id);
        Ok(old)
    }

    fn body_root(&self) -> Option<NodeId> {
        self.stories.iter().map(|s| s.root).find(|&root| {
            matches!(
                self.n(root).kind,
                NodeKind::Story {
                    story: StoryKind::Body,
                    ..
                }
            )
        })
    }

    fn in_body(&self, id: NodeId) -> bool {
        self.body_root()
            .is_some_and(|root| self.stories[self.n(id).story as usize].root == root)
    }

    fn text_of(&self, id: NodeId) -> &str {
        match &self.n(id).kind {
            NodeKind::Text { text } => text,
            _ => "",
        }
    }

    fn result_text(&self, found: &Found) -> String {
        found.results.iter().map(|&t| self.text_of(t)).collect()
    }

    /// The fields that begin and end in `paragraph`, in order. A field that
    /// holds another field is not plain, though the field inside may be.
    fn fields_in(&self, paragraph: NodeId) -> Vec<Found> {
        let mut open: Vec<(Found, bool)> = Vec::new();
        let mut out = Vec::new();
        for id in self.descendants(paragraph) {
            let node = self.n(id);
            match (self.field_char(id), &node.kind) {
                (Some("begin"), _) => {
                    for (outer, _) in &mut open {
                        outer.refusal.get_or_insert(NOT_PLAIN);
                    }
                    let found = Found {
                        field: id,
                        instruction: String::new(),
                        results: Vec::new(),
                        refusal: self.locked(id).then_some(LOCKED),
                    };
                    open.push((found, false));
                }
                (Some("separate"), _) => {
                    if let Some((_, separated)) = open.last_mut() {
                        *separated = true;
                    }
                }
                (Some("end"), _) => {
                    if let Some((mut found, _)) = open.pop() {
                        if found.results.is_empty() {
                            found.refusal.get_or_insert(NOT_PLAIN);
                        }
                        out.push(found);
                    }
                }
                (_, NodeKind::FieldCode { text }) => {
                    if let Some((found, false)) = open.last_mut() {
                        found.instruction.push_str(text);
                    }
                }
                (_, NodeKind::Text { .. }) => {
                    if let Some((found, true)) = open.last_mut() {
                        found.results.push(id);
                        if let Some(reason) = self.unsettable(id) {
                            found.refusal.get_or_insert(reason);
                        }
                    }
                }
                (_, NodeKind::Field { instruction }) => {
                    for (outer, _) in &mut open {
                        outer.refusal.get_or_insert(NOT_PLAIN);
                    }
                    let inside = self.descendants(id);
                    let results: Vec<NodeId> = inside
                        .iter()
                        .copied()
                        .filter(|&d| matches!(self.n(d).kind, NodeKind::Text { .. }))
                        .collect();
                    let mut refusal = self.locked(id).then_some(LOCKED);
                    if results.is_empty() {
                        refusal.get_or_insert(NOT_PLAIN);
                    }
                    for &d in &inside {
                        let reason = match &self.n(d).kind {
                            NodeKind::Run(_) => None,
                            NodeKind::Text { .. } => self.unsettable(d),
                            NodeKind::Marker { .. } if self.field_char(d).is_none() => None,
                            _ => Some(NOT_PLAIN),
                        };
                        if let Some(reason) = reason {
                            refusal.get_or_insert(reason);
                        }
                    }
                    out.push(Found {
                        field: id,
                        instruction: instruction.clone(),
                        results,
                        refusal,
                    });
                }
                (_, NodeKind::Run(_) | NodeKind::Marker { .. }) => {}
                _ => {
                    if let Some((found, true)) = open.last_mut() {
                        found.refusal.get_or_insert(NOT_PLAIN);
                    }
                }
            }
        }
        out
    }

    /// Why a result's text stays as it is: something other than its field
    /// or a content control makes it read-only, or a control around it is
    /// locked against editing or bound to data.
    fn unsettable(&self, text: NodeId) -> Option<&'static str> {
        match self.n(text).support {
            Support::ReadOnly(r) if r == IN_FIELD || r == IN_CONTENT_CONTROL => {}
            Support::ReadOnly(reason) => return Some(reason),
            _ => return Some(NOT_PLAIN),
        }
        let mut next = self.n(text).parent;
        while let Some(id) = next {
            if let NodeKind::ContentControl {
                data_bound, lock, ..
            } = &self.n(id).kind
            {
                if *data_bound {
                    return Some("a content control around it is bound to custom XML");
                }
                if matches!(lock.as_deref(), Some("contentLocked" | "sdtContentLocked")) {
                    return Some("a content control around it is locked against editing");
                }
            }
            next = self.n(id).parent;
        }
        None
    }

    fn locked(&self, field: NodeId) -> bool {
        (self.fields.fields.get(&field)).is_some_and(|info| info.locked)
    }
}

/// The field an instruction names, if the engine works it out: its name,
/// its arguments, and switches it knows. `\* MERGEFORMAT` and
/// `\* CHARFORMAT`, which keep the result's formatting, change nothing.
fn kind(instruction: &str) -> Option<Kind> {
    let tokens = tokens(instruction);
    let (name, rest) = tokens.split_first()?;
    let mut arguments = Vec::new();
    let mut picture = None;
    let mut format = None;
    let mut rest = rest.iter();
    while let Some(token) = rest.next() {
        match token.as_str() {
            "\\*" => match rest.next()?.as_str() {
                keep if ["MERGEFORMAT", "CHARFORMAT"].contains(&&*keep.to_ascii_uppercase()) => {}
                "Arabic" | "arabic" | "ARABIC" => format = Some("decimal"),
                "roman" => format = Some("lowerRoman"),
                "Roman" | "ROMAN" => format = Some("upperRoman"),
                "alphabetic" => format = Some("lowerLetter"),
                "Alphabetic" | "ALPHABETIC" => format = Some("upperLetter"),
                _ => return None,
            },
            "\\@" => picture = Some(rest.next()?.clone()),
            "\\h" => arguments.push(token.clone()),
            switch if switch.starts_with('\\') => return None,
            argument => arguments.push(argument.to_string()),
        }
    }
    let field = match (name.to_ascii_uppercase().as_str(), arguments.as_slice()) {
        ("PAGE", []) => Kind::Page(format.take()),
        ("NUMPAGES", []) => Kind::NumPages(format.take()),
        ("SECTIONPAGES", []) => Kind::SectionPages(format.take()),
        ("PAGEREF", [bookmark, switches @ ..])
            if !bookmark.starts_with('\\') && switches.iter().all(|s| s == "\\h") =>
        {
            Kind::PageRef(bookmark.clone())
        }
        ("DATE", []) => Kind::Date(picture.take().unwrap_or(DEFAULT_PICTURE.into())),
        ("TIME", []) => Kind::Date(picture.take().unwrap_or(DEFAULT_TIME_PICTURE.into())),
        _ => return None,
    };
    // A picture only dates and times take, and a number format only pages.
    (picture.is_none() && format.is_none()).then_some(field)
}

/// A field instruction's words, with a quoted one taken whole.
fn tokens(instruction: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = instruction.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '"' {
            chars.next();
            out.push(chars.by_ref().take_while(|&q| q != '"').collect());
        } else {
            let mut word = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() || c == '"' {
                    break;
                }
                word.push(c);
                chars.next();
            }
            out.push(word);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_fields_the_engine_works_out() {
        let date = |i: &str| match kind(i) {
            Some(Kind::Date(picture)) => Some(picture),
            _ => None,
        };
        assert!(matches!(kind(" PAGE "), Some(Kind::Page(None))));
        assert!(matches!(
            kind("page \\* MERGEFORMAT"),
            Some(Kind::Page(None))
        ));
        assert!(matches!(
            kind("NUMPAGES \\* charformat"),
            Some(Kind::NumPages(None))
        ));
        assert!(matches!(
            kind("PAGE \\* roman \\* MERGEFORMAT"),
            Some(Kind::Page(Some("lowerRoman")))
        ));
        assert!(matches!(
            kind("SECTIONPAGES \\* Arabic"),
            Some(Kind::SectionPages(Some("decimal")))
        ));
        assert!(matches!(kind(" PAGEREF _Toc1 \\h "), Some(Kind::PageRef(b)) if b == "_Toc1"));
        assert_eq!(
            date(r#" DATE \@ "dddd, MMMM d, yyyy" \* MERGEFORMAT "#).as_deref(),
            Some("dddd, MMMM d, yyyy")
        );
        assert_eq!(date(" DATE ").as_deref(), Some(DEFAULT_PICTURE));
        assert_eq!(date("DATE \\@ yyyy-MM-dd").as_deref(), Some("yyyy-MM-dd"));
        assert_eq!(date(" TIME ").as_deref(), Some(DEFAULT_TIME_PICTURE));
        for unknown in [
            "",
            "PAGE \\* Ordinal",
            "DATE \\* Arabic",
            "PAGE \\# 00",
            "PAGE \\@ yyyy",
            "PAGE extra",
            "PAGEREF",
            "PAGEREF _Toc1 extra",
            "PAGEREF _Toc1 \\p",
            "PAGEREF _Toc1 \\@ d",
            "PAGEREF \\h",
            "DATE \\l",
            "DATE \\@",
            "SEQ Figure \\* ARABIC",
            "REF _Ref1 \\h",
            "TOC \\o \"1-3\" \\h \\z \\u",
            "SAVEDATE",
        ] {
            assert!(kind(unknown).is_none(), "{unknown:?}");
        }
    }
}
