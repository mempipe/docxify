//! A JSON-compatible diagnostic view of the IR.
//!
//! This is for looking at a document, not for storing one: it omits the raw
//! OOXML the engine preserves, so it cannot rebuild the DOCX.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::comments::Mark;
use crate::diagnostics::Diagnostic;
use crate::import::{Level, Settings, Style};
use crate::ir::{
    Border, Edges, NodeId, NodeKind, ParaProps, RunInfo, RunProps, StoryKind, TableProps,
};
use crate::Document;

pub const INSPECT_FORMAT: &str = "docx-engine.inspect/2";

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Serialize)]
pub struct Inspection<'a> {
    pub format: &'static str,
    pub package: PackageView<'a>,
    pub commands: &'static [&'static str],
    pub stories: Vec<NodeView<'a>>,
    /// The comments story, which [`Document::inspect_node`] reads. A
    /// document without comments has one the engine made, which is among
    /// `stories` only while it holds a comment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comments_story: Option<NodeId>,
    pub settings: &'a Settings,
    pub styles: Vec<&'a Style>,
    pub unsupported: Vec<UnsupportedSummary>,
    pub diagnostics: &'a [Diagnostic],
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Serialize)]
pub struct PackageView<'a> {
    pub main_part: &'a str,
    pub parts: Vec<PartView<'a>>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Serialize)]
pub struct PartView<'a> {
    pub name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<&'a str>,
    /// `parsed` parts are modelled as stories or tables; `preserved` parts are
    /// carried byte for byte.
    pub handling: &'static str,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Serialize)]
pub struct NodeView<'a> {
    pub id: NodeId,
    #[serde(flatten)]
    pub kind: &'a NodeKind,
    pub support: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    /// A paragraph's text with tracked insertions shown and deletions hidden.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paragraph_text: Option<String>,
    /// A text node's offset, in Unicode scalar values, within its paragraph's text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading_level: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list: Option<ListView<'a>>,
    /// The instruction of the innermost field whose result a text node is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<&'a str>,
    /// The page number a layout draws in place of a text node, when it is
    /// the result of a page field the file does not lock.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_field: Option<crate::fields::PageField>,
    /// The comments whose ranges hold a run, by ID, in the order they open.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<&'a str>,
    #[serde(flatten)]
    pub resolved: Option<Resolved>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<NodeView<'a>>,
}

/// A node's formatting with its styles, its context and Word's defaults
/// applied: what a layout needs and the source leaves implicit.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Resolved {
    Paragraph {
        effective: EffectiveParagraph,
        /// The paragraph mark's run properties.
        mark: RunProps,
    },
    Run {
        effective: RunProps,
    },
    Table {
        cells: BTreeMap<NodeId, CellPlacement>,
        indent_twips: i32,
        alignment: &'static str,
    },
    Cell {
        borders: Edges<Border>,
        #[serde(skip_serializing_if = "Option::is_none")]
        shading: Option<String>,
        /// All four sides.
        margins_twips: Edges<i32>,
        vertical_align: &'static str,
    },
}

/// Position on the displayed grid. Zero rows means a continuation covered by a visible start.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Serialize)]
pub struct CellPlacement {
    pub column: usize,
    pub rows: usize,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Serialize)]
pub struct EffectiveParagraph {
    pub alignment: &'static str,
    pub indent: Indent,
    pub spacing: Spacing,
    pub contextual_spacing: bool,
    pub keep_next: bool,
    pub keep_lines: bool,
    pub widow_control: bool,
    pub page_break_before: bool,
    /// Custom stops in position order.
    pub tabs: Vec<TabStop>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Serialize)]
pub struct Indent {
    pub left_twips: i32,
    pub right_twips: i32,
    /// Negative for a hanging indent.
    pub first_line_twips: i32,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Serialize)]
pub struct Spacing {
    pub before_twips: i32,
    pub after_twips: i32,
    /// With `auto`, 240ths of a line; otherwise twips.
    pub line: i32,
    pub line_rule: &'static str,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Serialize)]
pub struct TabStop {
    pub pos_twips: i32,
    pub align: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leader: Option<&'static str>,
}

/// Word's spacing for `beforeAutospacing` and `afterAutospacing`: 14 points.
const AUTOSPACING: i32 = 280;

impl EffectiveParagraph {
    fn of(props: ParaProps) -> Self {
        let autospaced = |auto: Option<bool>, twips: Option<i32>| match auto {
            Some(true) => AUTOSPACING,
            _ => twips.unwrap_or(0),
        };
        let mut tabs: Vec<TabStop> = props
            .tabs
            .iter()
            .map(|t| TabStop {
                pos_twips: t.pos,
                align: match t.align.as_str() {
                    "center" => "center",
                    "right" | "end" => "right",
                    "decimal" => "decimal",
                    "bar" => "bar",
                    _ => "left",
                },
                leader: match t.leader.as_deref() {
                    Some("dot") => Some("dot"),
                    Some("hyphen") => Some("hyphen"),
                    Some("underscore") => Some("underscore"),
                    Some("heavy") => Some("heavy"),
                    Some("middleDot") => Some("middle_dot"),
                    _ => None,
                },
            })
            .collect();
        tabs.sort_by_key(|t| t.pos_twips);
        EffectiveParagraph {
            alignment: match props.alignment.as_deref() {
                None | Some("left" | "start") => "left",
                Some("right" | "end") => "right",
                Some("center") => "center",
                Some(_) => "justify",
            },
            indent: Indent {
                left_twips: props.indent_left.unwrap_or(0),
                right_twips: props.indent_right.unwrap_or(0),
                first_line_twips: props.indent_first_line.unwrap_or(0),
            },
            spacing: Spacing {
                before_twips: autospaced(props.before_autospacing, props.spacing_before),
                after_twips: autospaced(props.after_autospacing, props.spacing_after),
                line: props.line.unwrap_or(240),
                line_rule: match props.line_rule.as_deref() {
                    Some("exact") => "exact",
                    Some("atLeast") => "at_least",
                    _ => "auto",
                },
            },
            contextual_spacing: props.contextual_spacing.unwrap_or(false),
            keep_next: props.keep_next.unwrap_or(false),
            keep_lines: props.keep_lines.unwrap_or(false),
            widow_control: props.widow_control.unwrap_or(false),
            page_break_before: props.page_break_before.unwrap_or(false),
            tabs,
        }
    }
}

/// The styles a paragraph and its runs take formatting from: the paragraph
/// style's chain, with the chain of the table style it sits in before or
/// after it.
struct Layers<'a> {
    before: Vec<&'a Style>,
    chain: Vec<&'a Style>,
    after: Vec<&'a Style>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Serialize)]
pub struct ListView<'a> {
    pub marker: String,
    pub num_id: &'a str,
    pub level: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<&'a Level>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Serialize)]
pub struct UnsupportedSummary {
    pub element: String,
    pub support: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    pub count: usize,
}

impl Document {
    pub fn inspect(&self) -> Inspection<'_> {
        let parts = self
            .package
            .entries()
            .iter()
            .filter(|name| !name.ends_with('/'))
            .map(|name| PartView {
                name,
                content_type: self.package.content_type(name),
                handling: if self.parsed_parts.iter().any(|p| p == name) {
                    "parsed"
                } else {
                    "preserved"
                },
            })
            .collect();
        Inspection {
            format: INSPECT_FORMAT,
            package: PackageView {
                main_part: self.package.main_part(),
                parts,
            },
            commands: &crate::COMMANDS,
            stories: self
                .present_stories()
                .map(|s| self.view(s.root, None, &mut Vec::new(), &mut BTreeMap::new()))
                .collect(),
            comments_story: self.comments_story().map(|s| self.stories[s].root),
            settings: &self.settings,
            styles: {
                let unused = self.unused_added_style();
                self.styles
                    .styles
                    .values()
                    .filter(|s| Some(s.id.as_str()) != unused)
                    .collect()
            },
            unsupported: self.unsupported_summary(),
            diagnostics: &self.diagnostics,
        }
    }

    /// One node's view, such as a story's after an edit, if it is in the document.
    pub fn inspect_node(&self, id: NodeId) -> Option<NodeView<'_>> {
        self.attached(id).ok()?;
        let mut counts = BTreeMap::new();
        let story = self.stories[self.n(id).story as usize].root;
        if id != story
            && std::iter::once(id)
                .chain(self.descendants(id))
                .any(|n| self.list_ref(n).is_some())
        {
            for prior in self.descendants(story).into_iter().take_while(|&n| n != id) {
                if let Some(num) = self.list_ref(prior) {
                    self.list_marker(num, &mut counts);
                }
            }
        }
        Some(self.view(id, None, &mut self.comments_open(id), &mut counts))
    }

    pub fn inspect_json(&self) -> serde_json::Value {
        serde_json::to_value(self.inspect()).expect("inspection serializes")
    }

    /// The comments whose ranges are open where a node starts. A story
    /// without comment markers has none open anywhere, so the walk is
    /// skipped.
    fn comments_open(&self, id: NodeId) -> Vec<&str> {
        let mut open = Vec::new();
        if !self.stories[self.nodes[id.index()].story as usize].comments {
            return open;
        }
        let mut chain = vec![id];
        while let Some(parent) = self.nodes[chain[chain.len() - 1].index()].parent {
            chain.push(parent);
        }
        for pair in chain.windows(2).rev() {
            let (child, parent) = (pair[0], pair[1]);
            for &before in self.nodes[parent.index()]
                .children
                .iter()
                .take_while(|&&c| c != child)
            {
                self.pass_comment_mark(before, &mut open);
                for inside in self.descendants(before) {
                    self.pass_comment_mark(inside, &mut open);
                }
            }
        }
        open
    }

    /// Opens or closes a comment's range at its marker.
    fn pass_comment_mark<'d>(&'d self, id: NodeId, open: &mut Vec<&'d str>) {
        match self.comment_mark(id) {
            Some((Mark::Start, comment)) => open.push(comment),
            Some((Mark::End, comment)) => open.retain(|c| *c != comment),
            _ => {}
        }
    }

    fn cell_placements(&self, table: NodeId) -> BTreeMap<NodeId, CellPlacement> {
        let mut cells = BTreeMap::<NodeId, CellPlacement>::new();
        let mut above = BTreeMap::new();
        for row in self.rows(table) {
            let mut ongoing = BTreeMap::new();
            for cell in self.row_geometry(row) {
                let coverage = (cell.start, cell.span);
                let visible = matches!(self.n(cell.owner).kind, NodeKind::Cell(_));
                let mut rows = 1;
                match cell.merge {
                    Some("restart") if visible => {
                        ongoing.insert(coverage, cell.owner);
                    }
                    Some("continue") => {
                        if let Some(&head) = above.get(&coverage) {
                            cells.get_mut(&head).expect("visible merge start").rows += 1;
                            ongoing.insert(coverage, head);
                            rows = 0;
                        }
                    }
                    _ => {}
                }
                if visible {
                    cells.insert(
                        cell.owner,
                        CellPlacement {
                            column: cell.start,
                            rows,
                        },
                    );
                }
            }
            above = ongoing;
        }
        cells
    }

    fn list_marker(&self, num: &crate::NumRef, counts: &mut BTreeMap<String, Vec<u32>>) -> String {
        let Some(level) = self.numbering.level(num) else {
            return "•".into();
        };
        let counters = counts.entry(num.num_id.clone()).or_default();
        let i = usize::from(num.level);
        if let Some(n) = counters.get_mut(i) {
            *n = n.saturating_add(1);
            counters.truncate(i + 1);
        } else {
            for j in counters.len()..=i {
                let start = self
                    .numbering
                    .level(&crate::NumRef {
                        num_id: num.num_id.clone(),
                        level: j as u8,
                    })
                    .map_or(1, |d| d.start);
                counters.push(start);
            }
        }
        if level.format == "bullet" {
            return if level.text.is_empty() {
                "•".into()
            } else {
                level.text.clone()
            };
        }
        let mut marker = String::new();
        let mut chars = level.text.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '%' {
                if let Some(d @ '1'..='9') = chars.peek().copied() {
                    chars.next();
                    let j = (d as u8 - b'1') as usize;
                    if let Some(&value) = counters.get(j) {
                        let format = self
                            .numbering
                            .level(&crate::NumRef {
                                num_id: num.num_id.clone(),
                                level: j as u8,
                            })
                            .map_or("decimal", |d| d.format.as_str());
                        marker.push_str(&crate::format_number(value, format));
                    }
                    continue;
                }
            }
            marker.push(c);
        }
        marker
    }

    fn view<'d>(
        &'d self,
        id: NodeId,
        offsets: Option<&BTreeMap<NodeId, usize>>,
        open: &mut Vec<&'d str>,
        counts: &mut BTreeMap<String, Vec<u32>>,
    ) -> NodeView<'d> {
        let node = &self.nodes[id.index()];
        self.pass_comment_mark(id, open);
        let mut view = NodeView {
            id,
            kind: &node.kind,
            support: node.support.label(),
            reason: node.support.reason(),
            paragraph_text: None,
            offset: offsets.and_then(|o| o.get(&id).copied()),
            heading_level: None,
            list: None,
            field: None,
            page_field: None,
            comments: Vec::new(),
            resolved: None,
            children: Vec::new(),
        };
        let mut own_offsets = None;
        match &node.kind {
            NodeKind::Paragraph(info) => {
                let (text, map) = self.paragraph_text_and_offsets(id);
                view.paragraph_text = Some(text);
                own_offsets = Some(map);
                view.heading_level = self.heading_level(id);
                view.list = self.list_ref(id).map(|num| ListView {
                    marker: self.list_marker(num, counts),
                    num_id: &num.num_id,
                    level: num.level,
                    definition: self.numbering.level(num),
                });
                view.resolved = Some(Resolved::Paragraph {
                    effective: self.effective_paragraph(id),
                    mark: self.resolve_run(Some(id), info.mark_run.as_ref().map(|m| &m.info)),
                });
            }
            NodeKind::Run(_) => {
                view.comments = open.clone();
                view.resolved = Some(Resolved::Run {
                    effective: self.effective_run_props(id),
                })
            }
            NodeKind::Text { .. } => {
                let field = self.fields.results.get(&id);
                let info = field.and_then(|field| Some((*field, self.fields.fields.get(field)?)));
                if let Some((field, info)) = info {
                    view.field = Some(&info.instruction);
                    view.page_field = info.page_field(field);
                }
            }
            NodeKind::Table(_) => {
                let props = self.table_props(id);
                view.resolved = Some(Resolved::Table {
                    cells: self.cell_placements(id),
                    indent_twips: props.indent.unwrap_or(0),
                    alignment: match props.alignment.as_deref() {
                        Some("center") => "center",
                        Some("right" | "end") => "right",
                        _ => "left",
                    },
                });
            }
            NodeKind::Cell(_) => view.resolved = Some(self.resolve_cell(id)),
            _ => {}
        }
        let offsets = own_offsets.as_ref().or(offsets);
        view.children = node
            .children
            .iter()
            .map(|&c| self.view(c, offsets, open, counts))
            .collect();
        view
    }

    /// The items in the document that commands edit only partly or not at
    /// all; removed ones, which undo can bring back, are not counted.
    fn unsupported_summary(&self) -> Vec<UnsupportedSummary> {
        let mut counts: BTreeMap<(String, &'static str, Option<&'static str>), usize> =
            BTreeMap::new();
        let nodes = self
            .present_stories()
            .flat_map(|s| self.descendants(s.root));
        for node in nodes.map(|id| &self.nodes[id.index()]) {
            let element = match &node.kind {
                NodeKind::Opaque { element, .. } => element.clone(),
                NodeKind::Marker { element, .. } => element.clone(),
                NodeKind::Revision(info) => format!("w:{}", info.change),
                NodeKind::ContentControl { .. } => "w:sdt".to_string(),
                NodeKind::Field { .. } => "w:fldSimple".to_string(),
                NodeKind::Drawing(_) => "w:drawing".to_string(),
                _ => continue,
            };
            *counts
                .entry((element, node.support.label(), node.support.reason()))
                .or_default() += 1;
        }
        counts
            .into_iter()
            .map(|((element, support, reason), count)| UnsupportedSummary {
                element,
                support,
                reason,
                count,
            })
            .collect()
    }

    /// The body story's top-level blocks.
    pub fn blocks(&self) -> &[NodeId] {
        self.stories
            .iter()
            .find(|s| {
                matches!(
                    self.nodes[s.root.index()].kind,
                    NodeKind::Story {
                        story: StoryKind::Body,
                        ..
                    }
                )
            })
            .map(|s| self.nodes[s.root.index()].children.as_slice())
            .unwrap_or(&[])
    }

    /// Every node below `id`, in document order, excluding `id`.
    pub fn descendants(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack: Vec<NodeId> = self.nodes[id.index()]
            .children
            .iter()
            .rev()
            .copied()
            .collect();
        while let Some(next) = stack.pop() {
            out.push(next);
            stack.extend(self.nodes[next.index()].children.iter().rev());
        }
        out
    }

    /// Every paragraph in every story, tables included, in document order.
    pub fn paragraphs(&self) -> Vec<NodeId> {
        self.stories
            .iter()
            .flat_map(|s| self.descendants(s.root))
            .filter(|id| matches!(self.nodes[id.index()].kind, NodeKind::Paragraph(_)))
            .collect()
    }

    /// A paragraph's text, a text-like node's text, or the paragraph texts of
    /// any other node joined by newlines.
    pub fn text(&self, id: NodeId) -> Option<String> {
        let node = self.nodes.get(id.index())?;
        Some(match &node.kind {
            NodeKind::Paragraph(_) => self.paragraph_text_and_offsets(id).0,
            NodeKind::Text { text }
            | NodeKind::DeletedText { text }
            | NodeKind::FieldCode { text } => text.clone(),
            _ => {
                let paragraphs: Vec<String> = self
                    .descendants(id)
                    .into_iter()
                    .filter(|d| matches!(self.nodes[d.index()].kind, NodeKind::Paragraph(_)))
                    .map(|p| self.paragraph_text_and_offsets(p).0)
                    .collect();
                paragraphs.join("\n")
            }
        })
    }

    fn paragraph_text_and_offsets(&self, id: NodeId) -> (String, BTreeMap<NodeId, usize>) {
        let mut text = String::new();
        let mut offsets = BTreeMap::new();
        self.collect_text(id, &mut text, &mut 0, &mut offsets);
        (text, offsets)
    }

    fn collect_text(
        &self,
        id: NodeId,
        text: &mut String,
        chars: &mut usize,
        offsets: &mut BTreeMap<NodeId, usize>,
    ) {
        for &c in &self.nodes[id.index()].children {
            match &self.nodes[c.index()].kind {
                NodeKind::Text { text: t } => {
                    offsets.insert(c, *chars);
                    *chars += t.chars().count();
                    text.push_str(t);
                }
                NodeKind::Tab => {
                    *chars += 1;
                    text.push('\t');
                }
                NodeKind::Break { .. } => {
                    *chars += 1;
                    text.push('\n');
                }
                // The source of a move reads as deleted once changes are accepted.
                NodeKind::Revision(r) if r.change == "moveFrom" || r.change == "del" => {}
                _ => self.collect_text(c, text, chars, offsets),
            }
        }
    }

    pub(crate) fn paragraph_style(&self, id: NodeId) -> Option<&str> {
        match &self.nodes[id.index()].kind {
            NodeKind::Paragraph(info) => info
                .style
                .as_deref()
                .or_else(|| self.styles.default_paragraph_style().map(|s| s.id.as_str())),
            _ => None,
        }
    }

    pub fn heading_level(&self, id: NodeId) -> Option<u8> {
        let NodeKind::Paragraph(info) = &self.nodes[id.index()].kind else {
            return None;
        };
        let chain = self
            .paragraph_style(id)
            .map(|s| self.styles.chain(s))
            .unwrap_or_default();
        let outline = info
            .props
            .outline_level
            .or_else(|| chain.iter().rev().find_map(|s| s.paragraph.outline_level));
        if let Some(level) = outline {
            return (level < 9).then_some(level + 1);
        }
        let name = chain.last()?.name.as_deref()?.to_ascii_lowercase();
        if name == "title" {
            return Some(1);
        }
        name.strip_prefix("heading ")?
            .parse()
            .ok()
            .filter(|l| (1..=9).contains(l))
    }

    pub(crate) fn list_ref(&self, id: NodeId) -> Option<&crate::ir::NumRef> {
        let NodeKind::Paragraph(info) = &self.nodes[id.index()].kind else {
            return None;
        };
        let chain = self
            .paragraph_style(id)
            .map(|s| self.styles.chain(s))
            .unwrap_or_default();
        let num = info.props.numbering.as_ref().or_else(|| {
            chain
                .iter()
                .rev()
                .find_map(|s| s.paragraph.numbering.as_ref())
        })?;
        // numId 0 removes numbering a style would otherwise apply.
        (num.num_id != "0").then_some(num)
    }

    /// The nearest ancestor of a kind, `id` included.
    fn ancestor(&self, id: NodeId, kind: fn(&NodeKind) -> bool) -> Option<NodeId> {
        std::iter::successors(Some(id), |id| self.nodes[id.index()].parent)
            .find(|id| kind(&self.nodes[id.index()].kind))
    }

    fn layers(&self, paragraph: NodeId) -> Layers<'_> {
        let style = self.paragraph_style(paragraph);
        let chain = style.map(|s| self.styles.chain(s)).unwrap_or_default();
        let table = self
            .ancestor(paragraph, |k| matches!(k, NodeKind::Table(_)))
            .and_then(|t| match &self.nodes[t.index()].kind {
                NodeKind::Table(info) => info.style.as_deref(),
                _ => None,
            })
            .map(|s| self.styles.chain(s))
            .unwrap_or_default();
        // A table style applies before the paragraph style, except that Word
        // lets it override the default paragraph style.
        let default = self.styles.default_paragraph_style().map(|s| s.id.as_str());
        if style.is_some() && style == default {
            Layers {
                before: Vec::new(),
                chain,
                after: table,
            }
        } else {
            Layers {
                before: table,
                chain,
                after: Vec::new(),
            }
        }
    }

    /// Paragraph formatting after document defaults, the table and paragraph
    /// styles, the numbering level and direct formatting, with Word's
    /// defaults where none of them states a property.
    pub fn effective_paragraph(&self, paragraph: NodeId) -> EffectiveParagraph {
        let mut props = self.styles.default_paragraph.clone();
        if let NodeKind::Paragraph(info) = &self.nodes[paragraph.index()].kind {
            let layers = self.layers(paragraph);
            let level = self
                .list_ref(paragraph)
                .and_then(|num| self.numbering.level(num))
                .map(|l| &l.paragraph);
            // A list's level sits just below whatever applied the list: the
            // paragraph style, or the paragraph itself.
            let direct_list = info.props.numbering.is_some();
            for s in &layers.before {
                props.overlay(&s.paragraph);
            }
            if let Some(level) = level.filter(|_| !direct_list) {
                props.overlay(level);
            }
            for s in layers.chain.iter().chain(&layers.after) {
                props.overlay(&s.paragraph);
            }
            if let Some(level) = level.filter(|_| direct_list) {
                props.overlay(level);
            }
            props.overlay(&info.props);
        }
        EffectiveParagraph::of(props)
    }

    /// Run formatting after document defaults, the table and paragraph
    /// styles, the character style chain and direct formatting.
    pub fn effective_run_props(&self, run: NodeId) -> RunProps {
        let NodeKind::Run(info) = &self.nodes[run.index()].kind else {
            return self.resolve_run(None, None);
        };
        let paragraph = self.ancestor(run, |k| matches!(k, NodeKind::Paragraph(_)));
        self.resolve_run(paragraph, Some(info))
    }

    /// A run's formatting in a paragraph, with Word's font and size where
    /// nothing states one. The paragraph mark resolves as a run does.
    pub(crate) fn resolve_run(&self, paragraph: Option<NodeId>, run: Option<&RunInfo>) -> RunProps {
        let mut props = self.styles.default_run.clone();
        if let Some(layers) = paragraph.map(|p| self.layers(p)) {
            for s in layers
                .before
                .iter()
                .chain(&layers.chain)
                .chain(&layers.after)
            {
                props.overlay(&s.run);
            }
        }
        if let Some(run) = run {
            if let Some(style) = &run.style {
                for s in self.styles.chain(style) {
                    props.overlay(&s.run);
                }
            }
            props.overlay(&run.props);
        }
        props
            .font
            .get_or_insert_with(|| "Times New Roman".to_string());
        props.size_half_points.get_or_insert(20);
        props
    }

    /// A table's properties over its style chain's.
    fn table_props(&self, table: NodeId) -> TableProps {
        let mut props = TableProps::default();
        if let NodeKind::Table(info) = &self.nodes[table.index()].kind {
            if let Some(style) = &info.style {
                for s in self.styles.chain(style) {
                    props.overlay(&s.table);
                }
            }
            props.overlay(&info.props);
        }
        props
    }

    /// A cell's borders, fill, margins and alignment from its table's style,
    /// its table and itself. The table's inside borders apply to the edges
    /// it shares with other cells, and its outside borders to the rest.
    fn resolve_cell(&self, cell: NodeId) -> Resolved {
        let NodeKind::Cell(info) = &self.nodes[cell.index()].kind else {
            unreachable!("a cell");
        };
        let row = self.nodes[cell.index()].parent.expect("a cell is in a row");
        let table = self.nodes[row.index()].parent.expect("a row is in a table");
        let table_props = self.table_props(table);
        let rows: Vec<NodeId> = self.nodes[table.index()]
            .children
            .iter()
            .copied()
            .filter(|&r| matches!(self.nodes[r.index()].kind, NodeKind::Row(_)))
            .collect();
        let at = rows.iter().position(|&r| r == row).unwrap_or(0);
        // The row's columns as every table reader counts them, so a cell
        // inside a content control takes its columns here too.
        let slots = self.row_geometry(row);
        let (start, end) = (slots.iter().find(|slot| slot.owner == cell))
            .map_or((0, 0), |slot| (slot.start, slot.start + slot.span));
        let mut columns = slots.last().map_or(0, |slot| slot.start + slot.span);
        if let NodeKind::Row(row_info) = &self.nodes[row.index()].kind {
            columns = columns.max(row_info.grid_before as usize) + row_info.grid_after as usize;
        }
        let table_borders = &table_props.borders;
        let mut borders = Edges {
            top: if at == 0 {
                table_borders.outer.top.clone()
            } else {
                table_borders.inside_h.clone()
            },
            left: if start == 0 {
                table_borders.outer.left.clone()
            } else {
                table_borders.inside_v.clone()
            },
            bottom: if at + 1 == rows.len() {
                table_borders.outer.bottom.clone()
            } else {
                table_borders.inside_h.clone()
            },
            right: if end >= columns {
                table_borders.outer.right.clone()
            } else {
                table_borders.inside_v.clone()
            },
        };
        borders.overlay(&info.props.borders);
        for edge in [
            &mut borders.top,
            &mut borders.left,
            &mut borders.bottom,
            &mut borders.right,
        ] {
            if edge.as_ref().is_some_and(|b| !b.drawn()) {
                *edge = None;
            }
        }
        // Word's default cell margins: 0.075 inch on each side, none above or below.
        let mut margins = Edges {
            top: Some(0),
            left: Some(108),
            bottom: Some(0),
            right: Some(108),
        };
        margins.overlay(&table_props.cell_margins);
        margins.overlay(&info.props.margins);
        Resolved::Cell {
            borders,
            shading: info
                .props
                .shading
                .clone()
                .or(table_props.shading)
                .filter(|fill| fill != "auto"),
            margins_twips: margins,
            vertical_align: match info.props.vertical_align.as_deref() {
                Some("center") => "center",
                Some("bottom") => "bottom",
                _ => "top",
            },
        }
    }
}
