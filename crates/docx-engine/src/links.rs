//! Links: `SetLink`, which puts a range's content in `w:hyperlink` elements
//! or takes it out, the `WrapInLink` and `UnwrapLink` steps it compiles
//! into, and what export adds for the links commands make.
//!
//! A link a command makes to a URL names a relationship whose ID comes from
//! the URL alone. Export adds the relationships that links in use name, so
//! the same links give the same bytes in every replica, and undoing them
//! gives back the original parts. The Hyperlink character style is added the
//! same way when the document has none, and written while a run uses it.

use std::collections::BTreeSet;

use crate::error::{EditError, ExportError};
use crate::export::NewRelationship;
use crate::import::{Styles, Theme};
use crate::ir::{HyperlinkInfo, Made, NodeId, NodeKind, Support};
use crate::package::REL_HYPERLINK;
use crate::structure::refuse;
use crate::xml::{escape_attr, R, W};
use crate::Document;

/// Where a link points: its URL, with `#` and its anchor when it has one,
/// or `#` and a bookmark's name for a place in the document.
pub fn href(info: &HyperlinkInfo) -> Option<String> {
    match (&info.target, &info.anchor) {
        (Some(target), Some(anchor)) => Some(format!("{target}#{anchor}")),
        (Some(target), None) => Some(target.clone()),
        (None, Some(anchor)) => Some(format!("#{anchor}")),
        (None, None) => None,
    }
}

/// The relationship ID of a link a command makes to `target`. It depends on
/// the target alone, so replicas that make the same link write the same
/// bytes, and export needs no numbering of its own.
fn link_id(target: &str) -> String {
    format!("rIdL{:016x}", crate::fnv1a(target.as_bytes()))
}

/// Word's built-in Hyperlink character style.
fn style_xml(p: &str, declaration: &str, id: &str, based_on: Option<&str>) -> String {
    let based_on = based_on.map_or(String::new(), |b| {
        format!("<{p}basedOn {p}val=\"{}\"/>", escape_attr(b))
    });
    format!(
        "<{p}style{declaration} {p}type=\"character\" {p}styleId=\"{}\"><{p}name {p}val=\"Hyperlink\"/>{based_on}<{p}uiPriority {p}val=\"99\"/><{p}unhideWhenUsed/><{p}rPr><{p}color {p}val=\"0563C1\" {p}themeColor=\"hyperlink\"/><{p}u {p}val=\"single\"/></{p}rPr></{p}style>",
        escape_attr(id)
    )
}

impl Document {
    /// Refuses a URL whose relationship ID a relationship of another kind
    /// or target already has in the part.
    pub(crate) fn check_link_id(&self, near: NodeId, target: &str) -> Result<(), EditError> {
        let id = link_id(target);
        let part = &self.stories[self.n(near).story as usize].part;
        match self.package.relationship(part, &id) {
            Some(taken)
                if !(taken.rel_type == REL_HYPERLINK
                    && taken.external
                    && taken.target == target) =>
            {
                refuse(
                    near,
                    "the part already has a relationship with the ID this link takes",
                )
            }
            _ => Ok(()),
        }
    }

    pub(crate) fn holds_field_char(&self, id: NodeId) -> bool {
        self.field_char(id).is_some()
            || self
                .n(id)
                .children
                .iter()
                .any(|&c| self.holds_field_char(c))
    }

    /// A new, empty link to `target`: a bookmark's name after `#`, or a URL
    /// through a relationship export adds.
    pub(crate) fn make_link(&mut self, near: NodeId, target: &str) -> Result<NodeId, EditError> {
        let name = self.w_name(near, "hyperlink");
        let (info, head) = match target.strip_prefix('#') {
            Some(anchor) => {
                let attr = self.w_attr_name(near, "anchor");
                let head = format!("<{name} {attr}=\"{}\">", escape_attr(anchor));
                let info = HyperlinkInfo {
                    anchor: Some(anchor.to_string()),
                    ..HyperlinkInfo::default()
                };
                (info, head)
            }
            None => {
                self.check_link_id(near, target)?;
                let id = link_id(target);
                let head = format!(
                    "<{name} {}=\"{id}\">",
                    self.scope(near).attribute(R, "r", "id")
                );
                let info = HyperlinkInfo {
                    relationship: Some(id),
                    target: Some(target.to_string()),
                    anchor: None,
                };
                (info, head)
            }
        };
        let story = self.n(near).story;
        let made = Made {
            head,
            tail: format!("</{name}>"),
            like: None,
        };
        Ok(self.create(NodeKind::Hyperlink(info), Support::Editable, made, story))
    }

    /// The character style Word names Hyperlink, if the document has one.
    pub(crate) fn existing_link_style(&self) -> Option<String> {
        self.styles
            .styles
            .values()
            .find(|s| {
                s.kind == "character"
                    && s.name
                        .as_deref()
                        .is_some_and(|n| n.eq_ignore_ascii_case("hyperlink"))
            })
            .map(|s| s.id.clone())
    }

    /// The character style links take. A document without one gets Word's,
    /// kept with the document and written while a run uses it. A document
    /// with no styles part gets none, and its links look like its text.
    pub(crate) fn link_style(&mut self) -> Option<String> {
        if let Some(id) = self.existing_link_style() {
            return Some(id);
        }
        self.styles.part.as_ref()?;
        let named: BTreeSet<&str> = self
            .stories
            .iter()
            .flat_map(|s| self.descendants(s.root))
            .filter_map(|id| match &self.n(id).kind {
                NodeKind::Run(info) => info.style.as_deref(),
                _ => None,
            })
            .collect();
        let id = (0..)
            .map(|n| match n {
                0 => "Hyperlink".to_string(),
                n => format!("Hyperlink{n}"),
            })
            .find(|id| !self.styles.styles.contains_key(id) && !named.contains(id.as_str()))
            .expect("a free ID");
        let based_on = self
            .styles
            .styles
            .values()
            .find(|s| s.kind == "character" && s.is_default)
            .map(|s| s.id.clone());
        let part = format!(
            "<w:styles xmlns:w=\"{W}\">{}</w:styles>",
            style_xml("w:", "", &id, based_on.as_deref())
        );
        let mut parsed = Styles::parse("added", &part, &Theme::default()).expect("a valid style");
        let style = parsed.styles.remove(&id).expect("the style");
        self.styles.styles.insert(id.clone(), style);
        self.added_link_style = Some(id.clone());
        Some(id)
    }

    /// The Hyperlink style the engine added, if a run in an edited story uses it.
    fn added_style_in_use(&self) -> Option<&str> {
        let id = self.added_link_style.as_deref()?;
        let used = self
            .stories
            .iter()
            .filter(|s| self.subtree_dirty(s.root))
            .flat_map(|s| self.descendants(s.root))
            .any(|n| matches!(&self.n(n).kind, NodeKind::Run(info) if info.style.as_deref() == Some(id)));
        used.then_some(id)
    }

    /// The Hyperlink style the engine added, while no run uses it. Like a
    /// list a command added, it is not part of the document then.
    pub(crate) fn unused_added_style(&self) -> Option<&str> {
        let id = self.added_link_style.as_deref()?;
        self.added_style_in_use().is_none().then_some(id)
    }

    /// The relationships that links commands made name and the package
    /// lacks, one per ID, in story order.
    pub(crate) fn link_relationships(&self) -> Vec<NewRelationship> {
        let mut out: Vec<NewRelationship> = Vec::new();
        for story in self.stories.iter().filter(|s| self.subtree_dirty(s.root)) {
            for id in self.descendants(story.root) {
                let node = self.n(id);
                let NodeKind::Hyperlink(info) = &node.kind else {
                    continue;
                };
                let (Some(rel), Some(target), Some(_)) =
                    (&info.relationship, &info.target, &node.made)
                else {
                    continue;
                };
                let known = self.package.relationship(&story.part, rel).is_some()
                    || out.iter().any(|r| r.part == story.part && &r.id == rel);
                if !known {
                    out.push(NewRelationship {
                        part: story.part.clone(),
                        id: rel.clone(),
                        kind: REL_HYPERLINK,
                        target: target.clone(),
                        external: true,
                    });
                }
            }
        }
        out
    }

    /// The styles part export rewrites to hold the added Hyperlink style, if
    /// a run uses it.
    pub(crate) fn link_style_part(&self) -> Option<&str> {
        self.added_style_in_use()?;
        self.styles.part.as_deref()
    }

    /// The styles part's text with the added Hyperlink style, if a run uses it.
    pub(crate) fn link_style_text(&self) -> Result<Option<(String, String)>, ExportError> {
        let Some(id) = self.added_style_in_use() else {
            return Ok(None);
        };
        let part = self.styles.part.as_deref().expect("a styles part");
        let based_on = self
            .styles
            .styles
            .values()
            .find(|s| s.kind == "character" && s.is_default)
            .map(|s| s.id.as_str());
        let text = self.edit_root(part, |root, _, close| {
            let (p, declaration) = match root.lookup_prefix(W) {
                Some(prefix) => (format!("{prefix}:"), String::new()),
                None => ("w:".to_string(), format!(" xmlns:w=\"{W}\"")),
            };
            vec![(close, style_xml(&p, &declaration, id, based_on))]
        })?;
        Ok(Some((part.to_string(), text)))
    }
}
