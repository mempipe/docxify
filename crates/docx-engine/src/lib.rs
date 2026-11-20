//! A preservation-first DOCX engine.
//!
//! ```text
//! DOCX bytes → Document → inspection and selective export
//! Editor → semantic state → projected Document
//! ```
//!
//! The engine models what it understands and carries everything else byte for
//! byte. Export copies untouched ZIP entries raw, and within an edited part
//! regenerates only the edited nodes. See the crate README for the support
//! matrix and the design record.

mod arena;
mod comments;
mod compose;
mod controls;
mod dates;
mod diagnostics;
mod edit;
pub mod editing;
mod error;
mod export;
mod fields;
pub mod flow;
mod format;
mod import;
mod inspect;
pub mod ir;
mod links;
mod lists;
mod package;
mod pictures;
mod revisions;
mod settings;
pub mod state;
mod structure;
mod tables;
mod validate;
mod xml;

pub use diagnostics::{Diagnostic, Severity};
pub use edit::COMMANDS;
pub use edit::{check_text, Change, Command, Fill, Format, Point, Splice, TextRange, Track};
pub use error::{EditError, ExportError, OpenError};
pub use fields::{FieldUpdate, PageField, PageNumber, Pages};
pub use import::{Level, NoteNumbering, Numbering, Settings, Style, Styles, Theme};
pub use inspect::{EffectiveParagraph, Inspection, NodeView, Resolved, INSPECT_FORMAT};
use ir::NoteKind;
pub use ir::{Control, DrawingInfo, Node, NodeId, NodeKind, NumRef, StoryKind, Support};
pub use links::href;
pub use lists::{ListKind, MAX_LEVEL};
use package::Package;
pub use validate::{validate_package, Issue, ValidationReport};

use std::collections::{BTreeMap, BTreeSet};

use package::{
    REL_COMMENTS, REL_ENDNOTES, REL_FOOTER, REL_FOOTNOTES, REL_HEADER, REL_NUMBERING, REL_SETTINGS,
    REL_STYLES, REL_THEME,
};

/// A part the engine models as a tree of nodes.
#[derive(Clone)]
pub(crate) struct Story {
    pub part: String,
    pub xml: xml::XmlText,
    pub root: NodeId,
    pub scope: format::Scope,
    /// Some of its nodes, attached or not, are a complex field's characters.
    /// Nodes are never dropped, so once set this stays set.
    pub fields: bool,
    /// Some of its nodes, attached or not, are comment markers. This stays
    /// set too.
    pub comments: bool,
    /// The engine made the part, for comments added to a document without
    /// any. Export writes it while it holds a comment.
    pub added: bool,
}

#[derive(Clone)]
pub struct Document {
    package: Package,
    nodes: arena::Nodes,
    stories: Vec<Story>,
    styles: Styles,
    numbering: Numbering,
    /// Lists commands added, in the order they were added.
    added_lists: Vec<lists::AddedList>,
    /// The Hyperlink character style a link added, when the document had none.
    added_link_style: Option<String>,
    /// Media `add_media` added.
    added_media: Vec<pictures::AddedMedia>,
    /// The `wp:docPr` IDs of the drawings the document opened with.
    drawing_ids: BTreeSet<u32>,
    /// The highest `w:id` in any story the document opened with. Export
    /// numbers the revisions commands made above it.
    revision_ids: u32,
    /// The part that records comment threads, if the document has one.
    comments_extended: Option<String>,
    settings: Settings,
    /// The theme's fonts, for run properties commands read from markup.
    theme: Theme,
    /// The settings part, if the document has one.
    settings_part: Option<String>,
    /// Whether the file asked Word to update its fields on open.
    opened_update_fields: bool,
    fields: fields::FieldIndex,
    parsed_parts: Vec<String>,
    diagnostics: Vec<Diagnostic>,
}

impl Document {
    pub fn open(bytes: &[u8]) -> Result<Document, OpenError> {
        let package = Package::open(bytes)?;
        let main = package.main_part().to_string();
        let mut doc = Document {
            nodes: arena::Nodes::default(),
            stories: Vec::new(),
            styles: Styles::default(),
            numbering: Numbering::default(),
            added_lists: Vec::new(),
            added_link_style: None,
            added_media: Vec::new(),
            drawing_ids: BTreeSet::new(),
            revision_ids: 0,
            comments_extended: None,
            settings: Settings::default(),
            theme: Theme::default(),
            settings_part: None,
            opened_update_fields: false,
            fields: fields::FieldIndex::default(),
            parsed_parts: Vec::new(),
            diagnostics: Vec::new(),
            package,
        };

        let mut theme = Theme::default();
        if let Some(part) = doc.package.related_part(&main, REL_THEME) {
            let text = doc.package.read_text(&part)?;
            theme = Theme::parse(&part, &text.text)?;
            doc.parsed_parts.push(part);
        }
        if let Some(part) = doc.package.related_part(&main, REL_SETTINGS) {
            let text = doc.package.read_text(&part)?;
            doc.settings = Settings::parse(&part, &text.text)?;
            doc.opened_update_fields = doc.settings.update_fields;
            doc.parsed_parts.push(part.clone());
            doc.settings_part = Some(part);
        }
        if let Some(part) = doc.package.related_part(&main, REL_STYLES) {
            let text = doc.package.read_text(&part)?;
            doc.styles = Styles::parse(&part, &text.text, &theme)?;
            doc.parsed_parts.push(part);
        }
        if let Some(part) = doc.package.related_part(&main, REL_NUMBERING) {
            let text = doc.package.read_text(&part)?;
            doc.numbering = Numbering::parse(&part, &text.text, &theme)?;
            doc.parsed_parts.push(part);
        }

        doc.import_story(&main, StoryKind::Body, &theme)?;
        // Header and footer parts in name order, so IDs are the same on every
        // replica that opens the same bytes.
        let mut others: Vec<(String, StoryKind)> = doc
            .package
            .relationships(&main)
            .iter()
            .filter_map(|r| match r.rel_type.as_str() {
                REL_HEADER => Some((r.part.clone()?, StoryKind::Header)),
                REL_FOOTER => Some((r.part.clone()?, StoryKind::Footer)),
                _ => None,
            })
            .filter_map(|(p, k)| doc.package.actual_name(&p).map(|p| (p.to_string(), k)))
            .collect();
        others.sort();
        others.dedup();
        // Notes, then comments, come last, so adding them left the other
        // stories' IDs alone.
        for (rel_type, kind) in [
            (REL_FOOTNOTES, StoryKind::Footnotes),
            (REL_ENDNOTES, StoryKind::Endnotes),
            (REL_COMMENTS, StoryKind::Comments),
        ] {
            if let Some(part) = doc.package.related_part(&main, rel_type) {
                others.push((part, kind));
            }
        }
        for (part, kind) in others {
            doc.import_story(&part, kind, &theme)?;
        }
        doc.open_comments(&main, &theme)?;
        doc.theme = theme;
        doc.number_notes();
        doc.check_relationships();
        Ok(doc)
    }

    fn import_story(
        &mut self,
        part: &str,
        kind: StoryKind,
        theme: &Theme,
    ) -> Result<(), OpenError> {
        let text = self.package.read_text(part)?;
        self.import_story_text(part, text, kind, theme, false)?;
        self.parsed_parts.push(part.to_string());
        Ok(())
    }

    pub(crate) fn import_story_text(
        &mut self,
        part: &str,
        text: xml::XmlText,
        kind: StoryKind,
        theme: &Theme,
        added: bool,
    ) -> Result<(), OpenError> {
        let first;
        let (root, scope) = {
            let parsed = xml::parse(part, &text.text)?;
            let root_el = parsed.root_element();
            if root_el.tag_name().namespace() == Some(xml::W_STRICT) {
                return Err(OpenError::Unsupported {
                    part: part.to_string(),
                    reason: "Strict Open XML conformance".to_string(),
                });
            }
            if root_el.tag_name().namespace() != Some(xml::W) {
                return Err(OpenError::Unsupported {
                    part: part.to_string(),
                    reason: format!(
                        "root element {} is not WordprocessingML",
                        xml::display_name(root_el)
                    ),
                });
            }
            let story = self.stories.len() as u32;
            first = self.nodes.len();
            let mut importer = import::Importer::new(
                &mut self.nodes,
                &mut self.diagnostics,
                &mut self.fields,
                &self.package,
                theme,
                part,
                story,
            );
            let root = importer.story(root_el, kind);
            self.drawing_ids.extend(importer.drawing_ids);
            let ids = parsed
                .descendants()
                .filter_map(|n| n.attribute((xml::W, "id"))?.trim().parse::<u32>().ok());
            self.revision_ids = ids.fold(self.revision_ids, u32::max);
            root
        };
        let made = first..self.nodes.len();
        let fields = made
            .clone()
            .any(|i| self.field_char(NodeId(i as u32)).is_some());
        let comments = made
            .into_iter()
            .any(|i| self.comment_mark(NodeId(i as u32)).is_some());
        self.stories.push(Story {
            part: part.to_string(),
            xml: text,
            root,
            scope,
            fields,
            comments,
            added,
        });
        Ok(())
    }

    /// Numbers note references in document order, in the format the settings
    /// give each kind, and gives each note's own mark its reference's number.
    /// No command adds, removes or reorders references, so this holds while
    /// the document is open.
    fn number_notes(&mut self) {
        let order: Vec<NodeId> = self
            .stories
            .iter()
            .flat_map(|s| self.descendants(s.root))
            .collect();
        let mut numbers: BTreeMap<(NoteKind, String), String> = BTreeMap::new();
        let (mut footnotes, mut endnotes) =
            (self.settings.footnotes.start, self.settings.endnotes.start);
        for &id in &order {
            if let NodeKind::NoteReference {
                kind,
                note_id,
                number,
                custom_mark: false,
            } = &mut self.nodes[id.index()].kind
            {
                let (next, format) = match kind {
                    NoteKind::Footnote => (&mut footnotes, &self.settings.footnotes.format),
                    NoteKind::Endnote => (&mut endnotes, &self.settings.endnotes.format),
                };
                let formatted = numbers.entry((*kind, note_id.clone())).or_insert_with(|| {
                    *next += 1;
                    format_number(*next - 1, format)
                });
                *number = Some(formatted.clone());
            }
        }
        for id in order {
            let note =
                std::iter::successors(Some(id), |n| self.nodes[n.index()].parent).find_map(|n| {
                    match &self.nodes[n.index()].kind {
                        NodeKind::Note { kind, note_id, .. } => Some((*kind, note_id.clone())),
                        _ => None,
                    }
                });
            if let NodeKind::NoteMark { number, .. } = &mut self.nodes[id.index()].kind {
                *number = note.and_then(|n| numbers.get(&n).cloned());
            }
        }
    }

    /// Report relationship IDs that stories use but cannot resolve, and
    /// internal relationship targets missing from the package. The document
    /// still opens: preserving the problem is better than refusing to show it.
    fn check_relationships(&mut self) {
        for story in &self.stories {
            let Ok(doc) = xml::parse(&story.part, &story.xml.text) else {
                continue;
            };
            xml::check_relationship_ids(
                &doc,
                |id| self.package.relationship(&story.part, id).is_some(),
                |message| {
                    self.diagnostics.push(Diagnostic {
                        severity: Severity::Error,
                        code: "unresolved_relationship_id",
                        message,
                        part: Some(story.part.clone()),
                        node: None,
                    })
                },
            );
        }
        for (source, rel) in self.package.all_relationships() {
            if let Some(target) = rel.part.as_deref() {
                if self.package.actual_name(target).is_none() {
                    self.diagnostics.push(Diagnostic {
                        severity: Severity::Error,
                        code: "broken_relationship",
                        message: format!("{} targets {target}, which is missing", rel.id),
                        part: Some(if source.is_empty() {
                            "_rels/.rels".to_string()
                        } else {
                            source.to_string()
                        }),
                        node: None,
                    });
                }
            }
        }
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.index())
    }

    /// Every story root: the body first, then headers and footers, then
    /// notes, then comments.
    pub fn stories(&self) -> impl Iterator<Item = &Node> {
        self.present_stories().map(|s| &self.nodes[s.root.index()])
    }

    /// The stories, without a comments part the engine made that holds no
    /// comment.
    pub(crate) fn present_stories(&self) -> impl Iterator<Item = &Story> {
        self.stories
            .iter()
            .filter(|s| !s.added || !self.nodes[s.root.index()].children.is_empty())
    }

    pub fn styles(&self) -> &Styles {
        &self.styles
    }

    pub fn numbering(&self) -> &Numbering {
        &self.numbering
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// A part's bytes as opened, such as an image's, or as
    /// an edit supplied them. A story part an edit
    /// changed still reads as it was; `export` writes the change.
    pub fn part_bytes(&self, part: &str) -> Option<Vec<u8>> {
        match self.added_media_bytes(part) {
            Some(bytes) => Some(bytes.to_vec()),
            None => self.package.read_bytes(part).ok(),
        }
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// The node from the file whose properties a paragraph or run has: a
    /// node from the file is like itself; one a command made is like the
    /// node it copied them from. A run made in an empty paragraph is like
    /// that paragraph's mark. `None` for a bare node, or one with nothing to
    /// copy.
    pub(crate) fn like(&self, id: NodeId) -> Option<NodeId> {
        let node = self.nodes.get(id.index())?;
        match &node.made {
            None => Some(id),
            Some(made) => made.like,
        }
    }
}

/// 64-bit FNV-1a, which names what commands add from its content alone,
/// so replicas that add the same content write the same bytes.
pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// A note number in one of Word's formats: `decimal`, `lowerRoman`,
/// `upperRoman`, `lowerLetter`, `upperLetter` or `chicago`. Other formats
/// read as decimal.
pub fn format_number(n: u32, format: &str) -> String {
    match format {
        "lowerRoman" => roman(n).to_lowercase(),
        "upperRoman" => roman(n),
        // Word's letters repeat rather than carry: 27 is "aa", 28 "bb".
        "lowerLetter" | "upperLetter" if n > 0 => {
            let letter = (b'a' + ((n - 1) % 26) as u8) as char;
            let text = letter.to_string().repeat((n as usize - 1) / 26 + 1);
            if format == "upperLetter" {
                text.to_uppercase()
            } else {
                text
            }
        }
        "chicago" if n > 0 => {
            ["*", "†", "‡", "§"][(n as usize - 1) % 4].repeat((n as usize - 1) / 4 + 1)
        }
        _ => n.to_string(),
    }
}

fn roman(mut n: u32) -> String {
    const NUMERALS: [(u32, &str); 13] = [
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
    ];
    let mut out = String::new();
    for (value, numeral) in NUMERALS {
        while n >= value {
            out.push_str(numeral);
            n -= value;
        }
    }
    out
}

#[cfg(test)]
extern crate self as docx_engine;
