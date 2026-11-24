//! List properties and numbering definitions used by editing and export.
//!
//! A list the engine adds is a numbering instance (`w:num`) of an abstract
//! definition of its own, kept with the document rather than written into
//! `numbering.xml` straight away. Export writes the added lists some
//! paragraph uses, creating the numbering part when the document has none,
//! so undoing the command that added one gives back the original bytes, and
//! a list another replica still uses is never left without its definition.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::error::{EditError, ExportError};
use crate::export::{Additions, DECLARATION};
use crate::format::{rewrite, PPR_ORDER};
use crate::ids;
use crate::import::{Level, Numbering, Theme};
use crate::ir::{NodeId, NodeKind, NumRef};
use crate::xml::{self, W};
use crate::Document;

/// What a list's markers are.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListKind {
    Bullet,
    Number,
}

/// The highest level `w:ilvl` can name.
pub const MAX_LEVEL: u8 = 8;

const NUMPR: [&str; 1] = ["numPr"];
pub(crate) const CT_NUMBERING: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml";

/// A list a command added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AddedList {
    pub num_id: String,
    pub kind: ListKind,
}

/// The key an added list's levels are kept under in the numbering table. A
/// real `w:abstractNumId` is a number, so the two never meet.
fn abstract_key(num_id: &str) -> String {
    format!("added {num_id}")
}

/// An added list's abstract definition, as Word's ribbon makes one: nine
/// levels, each indented a further half inch. Bullets are characters every
/// core font has, so no symbol font is needed.
fn abstract_xml(p: &str, declaration: &str, id: u32, kind: ListKind) -> String {
    const BULLETS: [&str; 3] = ["\u{2022}", "\u{25E6}", "\u{25AA}"];
    const NUMBERS: [&str; 3] = ["decimal", "lowerLetter", "lowerRoman"];
    let mut out = format!(
        "<{p}abstractNum{declaration} {p}abstractNumId=\"{id}\"><{p}multiLevelType {p}val=\"hybridMultilevel\"/>"
    );
    for level in 0..=MAX_LEVEL as usize {
        let (format, text) = match kind {
            ListKind::Bullet => ("bullet", BULLETS[level % 3].to_string()),
            ListKind::Number => (NUMBERS[level % 3], format!("%{}.", level + 1)),
        };
        let left = 720 * (level + 1);
        out.push_str(&format!(
            "<{p}lvl {p}ilvl=\"{level}\"><{p}start {p}val=\"1\"/><{p}numFmt {p}val=\"{format}\"/><{p}lvlText {p}val=\"{text}\"/><{p}lvlJc {p}val=\"left\"/><{p}pPr><{p}ind {p}left=\"{left}\" {p}hanging=\"360\"/></{p}pPr></{p}lvl>"
        ));
    }
    out.push_str(&format!("</{p}abstractNum>"));
    out
}

fn num_xml(p: &str, declaration: &str, num_id: &str, abstract_id: u32) -> String {
    format!("<{p}num{declaration} {p}numId=\"{num_id}\"><{p}abstractNumId {p}val=\"{abstract_id}\"/></{p}num>")
}

/// The levels an added list's definition has, read back the way opening the
/// exported part reads them.
fn levels(kind: ListKind) -> BTreeMap<u8, Level> {
    let part = format!(
        "<w:numbering xmlns:w=\"{W}\">{}</w:numbering>",
        abstract_xml("w:", "", 0, kind)
    );
    let mut parsed =
        Numbering::parse("added", &part, &Theme::default()).expect("a valid definition");
    parsed.abstracts.remove("0").expect("the definition")
}

impl Document {
    /// The lists commands have added, with what their markers are.
    pub fn added_lists(&self) -> impl Iterator<Item = (&str, ListKind)> {
        self.added_lists.iter().map(|l| (l.num_id.as_str(), l.kind))
    }

    /// A new list of `kind`, with a number that no list the document has
    /// takes and no paragraph or style names, so it gives no stray reference
    /// a definition.
    pub(crate) fn new_list(&mut self, kind: ListKind) -> Result<String, EditError> {
        let mut used = self.source_ids.lists.clone();
        used.extend(
            self.added_lists()
                .filter_map(|(id, _)| id.parse::<u32>().ok()),
        );
        let num_id = ids::next_free(&used, 1..=ids::HIGHEST_NUMBER)
            .ok_or_else(|| EditError::List {
                num_id: ids::HIGHEST_NUMBER.to_string(),
                reason: "no unused list number remains",
            })?
            .to_string();
        let key = abstract_key(&num_id);
        self.numbering.abstracts.insert(key.clone(), levels(kind));
        self.numbering.instances.insert(num_id.clone(), key);
        self.added_lists.push(AddedList {
            num_id: num_id.clone(),
            kind,
        });
        Ok(num_id)
    }

    /// The list numbers paragraphs and styles name, defined or not.
    pub(crate) fn named_lists(&self) -> impl Iterator<Item = &str> {
        self.stories
            .iter()
            .flat_map(|s| self.descendants(s.root))
            .filter_map(|id| self.direct_numbering(id))
            .chain(
                self.styles
                    .styles
                    .values()
                    .filter_map(|s| s.paragraph.numbering.as_ref()),
            )
            .map(|n| n.num_id.as_str())
    }

    /// Continue a list of this kind in or beside the selection if it also
    /// supplies that kind of marker at every level the selection will take.
    pub(crate) fn continuing_list(&self, paragraphs: &[NodeId], kind: ListKind) -> Option<&NumRef> {
        let levels: BTreeSet<_> = paragraphs
            .iter()
            .map(|&p| self.list_ref(p).map_or(0, |n| n.level))
            .collect();
        paragraphs
            .iter()
            .copied()
            .chain(self.sibling_paragraph(*paragraphs.first()?, -1))
            .chain(self.sibling_paragraph(*paragraphs.last()?, 1))
            .filter_map(|p| self.list_ref(p))
            .find(|num| {
                let Some(definition) = self.numbering.levels(&num.num_id) else {
                    return false;
                };
                let matches = |level| {
                    definition.get(&level).is_some_and(|l| match kind {
                        ListKind::Bullet => l.format == "bullet",
                        ListKind::Number => l.format != "bullet",
                    })
                };
                let deepest = self.deepest(&num.num_id);
                matches(num.level) && levels.iter().all(|&level| matches(level.min(deepest)))
            })
    }

    /// The highest level a list defines, or the highest `w:ilvl` can name
    /// when its definition is missing.
    pub(crate) fn deepest(&self, num_id: &str) -> u8 {
        self.numbering
            .levels(num_id)
            .and_then(|levels| levels.keys().next_back().copied())
            .unwrap_or(MAX_LEVEL)
            .min(MAX_LEVEL)
    }

    /// The paragraph's `w:pPr` with its `w:numPr` set to `numbering`, or
    /// removed.
    pub(crate) fn numbered(
        &self,
        paragraph: NodeId,
        numbering: Option<&NumRef>,
    ) -> Result<String, EditError> {
        let name = self.w_name(paragraph, "pPr");
        let set = numbering.map(|n| {
            let numpr = self.w_name(paragraph, "numPr");
            let ilvl = self.w_name(paragraph, "ilvl");
            let num_id = self.w_name(paragraph, "numId");
            let val = self.w_attr_name(paragraph, "val");
            vec![format!(
                "<{numpr}><{ilvl} {val}=\"{}\"/><{num_id} {val}=\"{}\"/></{numpr}>",
                n.level, n.num_id
            )]
        });
        let current = self.properties(paragraph, "pPr")?;
        rewrite(
            &current,
            &self.scope(paragraph),
            &name,
            &PPR_ORDER,
            &NUMPR,
            set.unwrap_or_default(),
        )
        .map_err(|e| e.on(paragraph))
    }

    pub(crate) fn direct_numbering(&self, paragraph: NodeId) -> Option<&NumRef> {
        match &self.n(paragraph).kind {
            NodeKind::Paragraph(info) => info.props.numbering.as_ref(),
            _ => None,
        }
    }

    /// The numbering the paragraph's style chain gives.
    pub(crate) fn style_numbering(&self, paragraph: NodeId) -> Option<&NumRef> {
        let style = self.paragraph_style(paragraph)?;
        self.styles
            .chain(style)
            .into_iter()
            .rev()
            .find_map(|s| s.paragraph.numbering.as_ref())
    }

    /// The paragraph just before (`-1`) or after (`1`) this one in its
    /// story or cell, if the node there is a paragraph.
    pub(crate) fn sibling_paragraph(&self, paragraph: NodeId, step: isize) -> Option<NodeId> {
        let parent = self.n(paragraph).parent?;
        let i = self.index_of(paragraph).checked_add_signed(step)?;
        let sibling = *self.n(parent).children.get(i)?;
        matches!(self.n(sibling).kind, NodeKind::Paragraph(_)).then_some(sibling)
    }

    /// The added lists some paragraph uses, by number.
    fn lists_in_use(&self) -> Vec<&AddedList> {
        if self.added_lists.is_empty() {
            return Vec::new();
        }
        let mut numbers = BTreeSet::new();
        for story in &self.stories {
            for id in self.descendants(story.root) {
                if let Some(n) = self.direct_numbering(id) {
                    numbers.insert(n.num_id.as_str());
                }
            }
        }
        let mut used: Vec<&AddedList> = self
            .added_lists
            .iter()
            .filter(|l| numbers.contains(l.num_id.as_str()))
            .collect();
        used.sort_by_key(|l| l.num_id.parse::<u32>().unwrap_or(u32::MAX));
        used
    }

    /// The numbering part export writes: the document's, or a new one beside
    /// the main part.
    fn numbering_part(&self) -> String {
        if let Some(part) = &self.numbering.part {
            return part.clone();
        }
        let main = self.package.main_part();
        let dir = main.rfind('/').map_or("", |i| &main[..=i]);
        (0..)
            .map(|n| match n {
                0 => format!("{dir}numbering.xml"),
                n => format!("{dir}numbering{n}.xml"),
            })
            .find(|p| self.package.actual_name(p).is_none())
            .expect("a free name")
    }

    /// The numbering part export writes, if a paragraph uses an added list.
    pub(crate) fn list_part(&self) -> Option<String> {
        (!self.lists_in_use().is_empty()).then(|| self.numbering_part())
    }

    /// Writes the added lists in use into `out`: into the numbering part,
    /// or into a new one.
    pub(crate) fn list_parts(&self, out: &mut Additions) -> Result<(), ExportError> {
        let used = self.lists_in_use();
        if used.is_empty() {
            return Ok(());
        }
        let mut taken = self.source_ids.abstracts.clone();
        let mut abstract_ids = Vec::new();
        for _ in &used {
            let id = ids::next_free(&taken, 0..=ids::HIGHEST_NUMBER)
                .ok_or_else(|| ExportError::Zip("no list definition number remains".into()))?;
            taken.insert(id);
            abstract_ids.push(id);
        }
        let definitions = |p: &str, declaration: &str| {
            let mut abstracts = String::new();
            let mut nums = String::new();
            for (list, &id) in used.iter().zip(&abstract_ids) {
                abstracts.push_str(&abstract_xml(p, declaration, id, list.kind));
                nums.push_str(&num_xml(p, declaration, &list.num_id, id));
            }
            (abstracts, nums)
        };
        if let Some(part) = &self.numbering.part {
            let text = self.edit_root(part, |root, open, close| {
                let (p, declaration) = match root.lookup_prefix(W) {
                    Some(prefix) => (format!("{prefix}:"), String::new()),
                    None => ("w:".to_string(), format!(" xmlns:w=\"{W}\"")),
                };
                let (abstracts, nums) = definitions(&p, &declaration);
                let last = |local: &str| {
                    xml::elements(root)
                        .filter(|c| xml::is(*c, W, local))
                        .map(|c| c.range().end)
                        .last()
                };
                // The schema puts abstract definitions after picture bullets,
                // and instances after them, before numIdMacAtCleanup.
                let at_abstracts = last("abstractNum").or(last("numPicBullet")).unwrap_or(open);
                let cleanup = xml::elements(root)
                    .find(|c| xml::is(*c, W, "numIdMacAtCleanup"))
                    .map(|c| c.range().start);
                let at_nums = last("num").or(cleanup).unwrap_or(close).max(at_abstracts);
                vec![(at_abstracts, abstracts), (at_nums, nums)]
            })?;
            out.replaced.insert(part.to_ascii_lowercase(), text);
            return Ok(());
        }
        let part = self.numbering_part();
        let (abstracts, nums) = definitions("w:", "");
        out.added.push((
            part,
            format!("{DECLARATION}<w:numbering xmlns:w=\"{W}\">{abstracts}{nums}</w:numbering>"),
        ));
        Ok(())
    }
}
