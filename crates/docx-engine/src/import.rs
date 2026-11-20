//! OOXML → IR. Reads the story parts (body, headers, footers, notes) into
//! nodes, and styles, numbering, settings and the theme into read-only tables.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use roxmltree::Node as X;
use serde::Serialize;

use crate::diagnostics::{Diagnostic, Severity};
use crate::fields::{FieldIndex, FieldInfo};
use crate::ir::*;
use crate::package::{Package, Relationship};
use crate::xml::{self, child, elements, is, on_off, w_attr, w_val, A, R, W, W14, WP};

pub(crate) struct Importer<'a> {
    pub nodes: &'a mut crate::arena::Nodes,
    pub diagnostics: &'a mut Vec<Diagnostic>,
    pub field_index: &'a mut FieldIndex,
    pub package: &'a Package,
    pub theme: &'a Theme,
    pub part: &'a str,
    story: u32,
    /// Open complex fields (`w:fldChar` begin without end), innermost last.
    /// Fields can span paragraphs, so they stay open across the whole story.
    fields: Vec<OpenField>,
    /// Open move ranges as (ID, name), innermost last; they can span paragraphs too.
    moves: Vec<(Option<String>, Option<String>)>,
    /// Every `wp:docPr` ID in drawings and in preserved content.
    pub drawing_ids: Vec<u32>,
}

/// A field being read: its begin character or simple field, its instruction
/// so far, and whether its separator is past, so that text is its result.
struct OpenField {
    field: NodeId,
    locked: bool,
    instruction: String,
    separated: bool,
}

const IN_REVISION: &str = "inside a tracked change; accept or reject it first";
pub(crate) const IN_FIELD: &str = "field result; the field regenerates it";
pub(crate) const IN_CONTENT_CONTROL: &str = "inside a content control, which may be data-bound";
const IN_NOTE: &str = "inside a footnote or endnote; no command edits notes yet";
pub(crate) const IN_COMMENT: &str = "inside a comment; no command edits comments yet";
const FIELD_CODE: &str = "field instruction";
const DELETED_TEXT: &str = "deleted text in a tracked change";
pub(crate) const MIXED_TEXT: &str =
    "the text contains embedded XML markup; select the whole item to replace it";
pub(crate) const NO_COMMAND: &str = "preserved verbatim; no command edits it yet";
pub(crate) const NO_EDIT: &str = "no command edits it; remove_node removes it";

impl<'a> Importer<'a> {
    pub fn new(
        nodes: &'a mut crate::arena::Nodes,
        diagnostics: &'a mut Vec<Diagnostic>,
        field_index: &'a mut FieldIndex,
        package: &'a Package,
        theme: &'a Theme,
        part: &'a str,
        story: u32,
    ) -> Self {
        Importer {
            nodes,
            diagnostics,
            field_index,
            package,
            theme,
            part,
            story,
            fields: Vec::new(),
            moves: Vec::new(),
            drawing_ids: Vec::new(),
        }
    }

    fn push(&mut self, parent: Option<NodeId>, kind: NodeKind, support: Support, el: X) -> NodeId {
        let id = NodeId(self.nodes.len() as u32);
        let range = el.range();
        let lead = parent
            .and_then(|p| self.nodes[p.index()].children.last())
            .map_or(range.start, |s| self.nodes[s.index()].range.end);
        self.nodes.push(Node {
            id,
            parent,
            children: Vec::new(),
            kind,
            support,
            preserved_cells: None,
            inner: range.end..range.end,
            range,
            lead,
            made: None,
            story: self.story,
            dirty: false,
            dirty_below: false,
            head: None,
        });
        if let Some(parent) = parent {
            self.nodes[parent.index()].children.push(id);
        }
        id
    }

    /// Record where a container's children sit, once they are all pushed.
    /// `content` is the element that holds them.
    fn close(&mut self, id: NodeId, content: X) {
        let children = &self.nodes[id.index()].children;
        let inner = match (children.first(), children.last()) {
            (Some(first), Some(last)) => {
                self.nodes[first.index()].range.start..self.nodes[last.index()].range.end
            }
            _ => {
                let end = end_tag_start(content);
                end..end
            }
        };
        self.nodes[id.index()].inner = inner;
    }

    fn rel(&self, id: &str) -> Option<&'a Relationship> {
        self.package.relationship(self.part, id)
    }

    fn content(ro: Option<&'static str>) -> Support {
        ro.map_or(Support::Editable, Support::ReadOnly)
    }

    pub fn story(&mut self, root: X, kind: StoryKind) -> (NodeId, crate::format::Scope) {
        let id = self.push(
            None,
            NodeKind::Story {
                story: kind,
                part: self.part.to_string(),
            },
            Support::Editable,
            root,
        );
        let container = if kind == StoryKind::Body {
            child(root, W, "body").unwrap_or(root)
        } else {
            root
        };
        // The body element itself is not a node; its children belong to the
        // story, and the story's range is the whole root element.
        for el in elements(container) {
            self.block(id, el, None);
        }
        self.close(id, container);
        (id, crate::format::Scope::from_element(container))
    }

    fn block(&mut self, parent: NodeId, el: X, ro: Option<&'static str>) {
        if el.tag_name().namespace() == Some(W) {
            match el.tag_name().name() {
                "p" => return self.paragraph(parent, el, ro),
                "tbl" => return self.table(parent, el, ro),
                "sdt" => return self.content_control(parent, el, true),
                "sectPr" => {
                    let info = self.section(el);
                    self.push(
                        Some(parent),
                        NodeKind::Section(Box::new(info)),
                        Support::ReadOnly(NO_COMMAND),
                        el,
                    );
                    return;
                }
                "footnote" | "endnote" => return self.note(parent, el),
                "comment" => return self.comment(parent, el),
                "tcPr" => return,
                _ if is_marker(el) => {
                    self.marker(parent, el);
                    return;
                }
                _ => {}
            }
        }
        self.opaque(parent, el);
    }

    fn paragraph(&mut self, parent: NodeId, el: X, ro: Option<&'static str>) {
        let mut info = ParagraphInfo {
            word_para_id: el.attribute((W14, "paraId")).map(str::to_string),
            ..ParagraphInfo::default()
        };
        if let Some(ppr) = child(el, W, "pPr") {
            info.style = w_val(ppr, "pStyle").map(str::to_string);
            info.props = para_props(ppr);
            info.format_revision = child(ppr, W, "pPrChange").is_some();
            if let Some(rpr) = child(ppr, W, "rPr") {
                let mark = ["del", "ins", "moveFrom", "moveTo"]
                    .into_iter()
                    .find_map(|k| child(rpr, W, k).map(|c| (k, c)));
                info.mark_revision = mark.map(|(k, _)| k.to_string());
                info.mark_change = mark.map(|(_, c)| tracked(c));
                if info.mark_revision.as_deref() == Some("del") {
                    info.mark_insertion = child(rpr, W, "ins").map(tracked);
                }
                info.mark_run = Some(Box::new(MarkRun {
                    range: rpr.range(),
                    info: RunInfo {
                        style: w_val(rpr, "rStyle").map(str::to_string),
                        props: run_props(rpr, self.theme),
                        format_revision: child(rpr, W, "rPrChange").is_some(),
                        ..RunInfo::default()
                    },
                }));
            }
            info.section = child(ppr, W, "sectPr").map(|s| self.section(s));
        }
        let id = self.push(
            Some(parent),
            NodeKind::Paragraph(Box::new(info)),
            Self::content(ro),
            el,
        );
        self.inlines(id, el, ro);
        self.close(id, el);
    }

    fn inlines(&mut self, parent: NodeId, el: X, ro: Option<&'static str>) {
        for c in elements(el) {
            self.inline(parent, c, ro);
        }
    }

    fn inline(&mut self, parent: NodeId, el: X, ro: Option<&'static str>) {
        if el.tag_name().namespace() != Some(W) {
            return self.opaque(parent, el);
        }
        match el.tag_name().name() {
            "pPr" => {}
            "r" => self.run(parent, el, ro),
            "hyperlink" => {
                let relationship = el.attribute((R, "id")).map(str::to_string);
                let target = relationship
                    .as_deref()
                    .and_then(|r| self.rel(r))
                    .map(|r| r.target.clone());
                let info = HyperlinkInfo {
                    relationship,
                    target,
                    anchor: w_attr(el, "anchor").map(str::to_string),
                };
                let id = self.push(
                    Some(parent),
                    NodeKind::Hyperlink(info),
                    Self::content(ro),
                    el,
                );
                self.inlines(id, el, ro);
                self.close(id, el);
            }
            change @ ("ins" | "del" | "moveFrom" | "moveTo") => {
                let move_name = change
                    .starts_with("move")
                    .then(|| self.moves.last().and_then(|(_, name)| name.clone()))
                    .flatten();
                let info = RevisionInfo {
                    change: change.to_string(),
                    revision_id: w_attr(el, "id").map(str::to_string),
                    author: w_attr(el, "author").map(str::to_string),
                    date: w_attr(el, "date").map(str::to_string),
                    move_name,
                };
                let id = self.push(
                    Some(parent),
                    NodeKind::Revision(info),
                    Support::ReadOnly(IN_REVISION),
                    el,
                );
                self.inlines(id, el, Some(IN_REVISION));
                self.close(id, el);
            }
            "fldSimple" => {
                let instruction = w_attr(el, "instr").unwrap_or_default().trim().to_string();
                // A tracked change or control around the field still decides
                // whether its result can change.
                let ro = ro.or(Some(IN_FIELD));
                let kind = NodeKind::Field {
                    instruction: instruction.clone(),
                };
                let id = self.push(Some(parent), kind, Self::content(ro), el);
                let open = OpenField {
                    field: id,
                    locked: field_locked(el),
                    instruction,
                    separated: true,
                };
                self.record_field(&open);
                let depth = self.fields.len();
                self.fields.push(open);
                self.inlines(id, el, ro);
                self.fields.truncate(depth);
                self.close(id, el);
            }
            "sdt" => self.content_control(parent, el, false),
            _ if is_marker(el) => {
                self.marker(parent, el);
            }
            _ => self.opaque(parent, el),
        }
    }

    fn run(&mut self, parent: NodeId, el: X, ro: Option<&'static str>) {
        let mut info = RunInfo::default();
        if let Some(rpr) = child(el, W, "rPr") {
            info.style = w_val(rpr, "rStyle").map(str::to_string);
            info.props = run_props(rpr, self.theme);
            let change = child(rpr, W, "rPrChange");
            info.format_revision = change.is_some();
            info.previous = change.map(|c| {
                child(c, W, "rPr")
                    .map(|r| run_props(r, self.theme))
                    .unwrap_or_default()
            });
            info.format_change = change.map(tracked);
        }
        let id = self.push(
            Some(parent),
            NodeKind::Run(Box::new(info)),
            Self::content(ro),
            el,
        );
        for c in elements(el) {
            if c.tag_name().namespace() != Some(W) {
                self.opaque(id, c);
                continue;
            }
            let leaf_ro = ro.or((!self.fields.is_empty()).then_some(IN_FIELD));
            match c.tag_name().name() {
                "rPr" => {}
                "t" => {
                    let text = xml::character_data(c);
                    let support = if c.children().any(|child| !child.is_text()) {
                        Support::ReadOnly(MIXED_TEXT)
                    } else {
                        Self::content(leaf_ro)
                    };
                    let t = self.push(Some(id), NodeKind::Text { text }, support, c);
                    if let Some(open) = self.fields.iter().rev().find(|f| f.separated) {
                        self.field_index.results.insert(t, open.field);
                    }
                }
                "delText" => {
                    let text = xml::character_data(c);
                    self.push(
                        Some(id),
                        NodeKind::DeletedText { text },
                        Support::ReadOnly(DELETED_TEXT),
                        c,
                    );
                }
                "instrText" | "delInstrText" => {
                    let text = xml::character_data(c);
                    if let Some(field) = self.fields.last_mut().filter(|f| !f.separated) {
                        field.instruction.push_str(&text);
                    }
                    self.push(
                        Some(id),
                        NodeKind::FieldCode { text },
                        Support::ReadOnly(FIELD_CODE),
                        c,
                    );
                }
                "tab" => {
                    self.push(Some(id), NodeKind::Tab, Support::ReadOnly(NO_EDIT), c);
                }
                "br" | "cr" => {
                    let break_type = w_attr(c, "type").map(str::to_string);
                    self.push(
                        Some(id),
                        NodeKind::Break { break_type },
                        Support::ReadOnly(NO_EDIT),
                        c,
                    );
                }
                "drawing" => {
                    let info = self.drawing(c);
                    self.push(
                        Some(id),
                        NodeKind::Drawing(Box::new(info)),
                        Support::ReadOnly(NO_EDIT),
                        c,
                    );
                }
                "fldChar" => {
                    let marker = self.marker(id, c);
                    match w_attr(c, "fldCharType") {
                        Some("begin") => self.fields.push(OpenField {
                            field: marker,
                            locked: field_locked(c),
                            instruction: String::new(),
                            separated: false,
                        }),
                        Some("separate") => {
                            if let Some(mut open) = self.fields.pop() {
                                open.separated = true;
                                self.record_field(&open);
                                self.fields.push(open);
                            }
                        }
                        Some("end") => {
                            if let Some(open) = self.fields.pop().filter(|f| !f.separated) {
                                self.record_field(&open);
                            }
                        }
                        _ => {}
                    }
                }
                "footnoteReference" | "endnoteReference" => {
                    let kind = NodeKind::NoteReference {
                        kind: note_kind(c),
                        note_id: w_attr(c, "id").unwrap_or_default().to_string(),
                        number: None,
                        custom_mark: matches!(
                            w_attr(c, "customMarkFollows"),
                            Some("1" | "true" | "on")
                        ),
                    };
                    self.push(Some(id), kind, Support::ReadOnly(NO_COMMAND), c);
                }
                "footnoteRef" | "endnoteRef" => {
                    let kind = NodeKind::NoteMark {
                        kind: note_kind(c),
                        number: None,
                    };
                    self.push(Some(id), kind, Support::ReadOnly(NO_COMMAND), c);
                }
                _ if is_marker(c) => {
                    self.marker(id, c);
                }
                _ => self.opaque(id, c),
            }
        }
        self.close(id, el);
    }

    fn table(&mut self, parent: NodeId, el: X, ro: Option<&'static str>) {
        let mut info = TableInfo::default();
        if let Some(tbl_pr) = child(el, W, "tblPr") {
            info.style = w_val(tbl_pr, "tblStyle").map(str::to_string);
            info.props = Box::new(table_props(tbl_pr));
        }
        if let Some(grid) = child(el, W, "tblGrid") {
            info.grid = grid_columns(grid);
        }
        let columns = info.grid.len() as u32;
        let id = self.push(Some(parent), NodeKind::Table(info), Self::content(ro), el);
        // The schema puts range markers before the properties. Properties
        // that follow a marker still belong to the table's head, so that they
        // are written with the table and not with the child after them.
        let mut late = Vec::new();
        for c in elements(el) {
            let before = self.nodes[id.index()].children.len();
            match (c.tag_name().namespace() == Some(W), c.tag_name().name()) {
                (true, "tblPr" | "tblGrid") => {
                    if before > 0 {
                        late.push(c.range());
                    }
                }
                (true, "tr") => self.row(id, c, ro, columns),
                (true, _) if is_marker(c) => {
                    self.marker(id, c);
                }
                _ => self.opaque(id, c),
            }
            if let (Some(property), Some(&child)) =
                (late.last(), self.nodes[id.index()].children.get(before))
            {
                let child = &mut self.nodes[child.index()];
                child.lead = child.lead.max(property.end);
            }
        }
        if let Some(first) = self.nodes[id.index()].children.first() {
            if !late.is_empty() {
                let text = el.document().input_text();
                let first = self.nodes[first.index()].range.start;
                let mut head = text[el.range().start..first].to_string();
                late.iter()
                    .for_each(|property| head.push_str(&text[property.clone()]));
                self.nodes[id.index()].head = Some(head.into_boxed_str());
            }
        }
        self.close(id, el);
    }

    fn row(&mut self, parent: NodeId, el: X, ro: Option<&'static str>, columns: u32) {
        let mut info = RowInfo::default();
        if let Some(tr_pr) = child(el, W, "trPr") {
            info.header = on_off(tr_pr, "tblHeader").unwrap_or(false);
            info.grid_before = w_val(tr_pr, "gridBefore")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            info.grid_after = w_val(tr_pr, "gridAfter")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            info.revision = ["ins", "del"]
                .into_iter()
                .find(|k| child(tr_pr, W, k).is_some())
                .map(str::to_string);
            info.cant_split = on_off(tr_pr, "cantSplit").unwrap_or(false);
            info.height = child(tr_pr, W, "trHeight").and_then(|h| {
                let rule = match w_attr(h, "hRule") {
                    Some("exact") => "exact",
                    Some("auto") => return None,
                    _ => "at_least",
                };
                Some(RowHeight {
                    twips: w_attr(h, "val")?.parse().ok()?,
                    rule,
                })
            });
        }
        let row_ro = ro.or(info.revision.is_some().then_some(IN_REVISION));
        let mut spans = info.grid_before + info.grid_after;
        let id = self.push(Some(parent), NodeKind::Row(info), Self::content(row_ro), el);
        for c in elements(el) {
            match (c.tag_name().namespace() == Some(W), c.tag_name().name()) {
                (true, "trPr" | "tblPrEx") => {}
                (true, "tc") => spans += self.cell(id, c, row_ro),
                (true, _) if is_marker(c) => {
                    self.marker(id, c);
                }
                _ => self.opaque(id, c),
            }
        }
        self.close(id, el);
        spans += self.nodes[id.index()]
            .children
            .iter()
            .filter_map(|child| self.nodes[child.index()].preserved_cells.as_ref())
            .flat_map(|cells| cells.iter())
            .map(|cell| cell.grid_span)
            .sum::<u32>();
        if columns > 0 && spans != columns {
            self.diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                code: "table_grid_mismatch",
                message: format!("row spans {spans} grid columns; the table grid has {columns}"),
                part: Some(self.part.to_string()),
                node: Some(id),
            });
        }
    }

    fn cell(&mut self, parent: NodeId, el: X, ro: Option<&'static str>) -> u32 {
        let geometry = cell_geometry(child(el, W, "tcPr"));
        let mut info = CellInfo {
            grid_span: geometry.grid_span,
            vertical_merge: geometry.vertical_merge,
            width: None,
            props: Box::default(),
        };
        if let Some(tc_pr) = child(el, W, "tcPr") {
            info.width = cell_width(tc_pr);
            info.props = Box::new(cell_props(tc_pr));
        }
        let span = info.grid_span;
        let id = self.push(Some(parent), NodeKind::Cell(info), Self::content(ro), el);
        for c in elements(el) {
            self.block(id, c, ro);
        }
        self.close(id, el);
        span
    }

    fn content_control(&mut self, parent: NodeId, el: X, block: bool) {
        let props = child(el, W, "sdtPr");
        let (control, placeholder) = crate::controls::control_of(props);
        let kind = NodeKind::ContentControl {
            tag: props.and_then(|p| w_val(p, "tag")).map(str::to_string),
            alias: props.and_then(|p| w_val(p, "alias")).map(str::to_string),
            data_bound: props.is_some_and(|p| child(p, W, "dataBinding").is_some()),
            lock: props.and_then(|p| w_val(p, "lock")).map(str::to_string),
            control,
            placeholder,
        };
        let id = self.push(
            Some(parent),
            kind,
            Support::ReadOnly(IN_CONTENT_CONTROL),
            el,
        );
        let content = child(el, W, "sdtContent");
        if let Some(content) = content {
            for c in elements(content) {
                if block {
                    self.block(id, c, Some(IN_CONTENT_CONTROL));
                } else {
                    self.inline(id, c, Some(IN_CONTENT_CONTROL));
                }
            }
        }
        self.close(id, content.unwrap_or(el));
    }

    fn drawing(&mut self, el: X) -> DrawingInfo {
        let mut info = DrawingInfo {
            holds: holds(el),
            placement: child(el, WP, "anchor").map_or(Placement::Inline, placement),
            ..DrawingInfo::default()
        };
        for d in el.descendants().filter(|d| d.is_element()) {
            if is(d, WP, "extent") {
                info.width_emu = d.attribute("cx").and_then(|v| v.parse().ok());
                info.height_emu = d.attribute("cy").and_then(|v| v.parse().ok());
            } else if is(d, WP, "docPr") {
                let id = d.attribute("id").and_then(|v| v.parse().ok());
                self.drawing_ids.extend(id);
                // A text box's content can hold drawings of its own; the
                // drawing's own `docPr` comes first.
                if info.id.is_none() {
                    info.id = id;
                    info.name = d.attribute("name").map(str::to_string);
                    info.description = d.attribute("descr").map(str::to_string);
                }
            } else if is(d, A, "blip") {
                for attr in ["embed", "link"] {
                    if let Some(rel_id) = d.attribute((R, attr)) {
                        let rel = self.rel(rel_id);
                        let part = rel.and_then(|r| r.part.clone());
                        info.media.push(MediaRef {
                            relationship: rel_id.to_string(),
                            content_type: part
                                .as_deref()
                                .and_then(|p| self.package.content_type(p))
                                .map(str::to_string),
                            part,
                            external: rel.is_some_and(|r| r.external),
                        });
                    }
                }
            }
        }
        info
    }

    fn section(&mut self, el: X) -> SectionInfo {
        let mut info = SectionInfo {
            start: w_val(el, "type").unwrap_or("nextPage").to_string(),
            title_page: on_off(el, "titlePg").unwrap_or(false),
            ..SectionInfo::default()
        };
        if let Some(numbers) = child(el, W, "pgNumType") {
            let start = w_attr(numbers, "start").and_then(|v| v.parse().ok());
            let format = w_attr(numbers, "fmt").map(str::to_string);
            if start.is_some() || format.is_some() {
                info.page_numbers = Some(PageNumbers { start, format });
            }
        }
        if let Some(cols) = child(el, W, "cols") {
            let count = w_attr(cols, "num")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            if count > 1 {
                info.columns = Some(Columns {
                    count,
                    space_twips: w_attr(cols, "space")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(720),
                });
            }
        }
        if let Some(sz) = child(el, W, "pgSz") {
            info.page_width = w_attr(sz, "w").and_then(|v| v.parse().ok());
            info.page_height = w_attr(sz, "h").and_then(|v| v.parse().ok());
            info.orientation = w_attr(sz, "orient").map(str::to_string);
        }
        if let Some(mar) = child(el, W, "pgMar") {
            for side in [
                "top", "right", "bottom", "left", "header", "footer", "gutter",
            ] {
                if let Some(v) = w_attr(mar, side).and_then(|v| v.parse().ok()) {
                    info.margins.insert(side.to_string(), v);
                }
            }
        }
        for reference in elements(el) {
            let list = if is(reference, W, "headerReference") {
                &mut info.headers
            } else if is(reference, W, "footerReference") {
                &mut info.footers
            } else {
                continue;
            };
            let relationship = reference
                .attribute((R, "id"))
                .unwrap_or_default()
                .to_string();
            let part = self.rel(&relationship).and_then(|r| r.part.clone());
            list.push(StoryRef {
                kind: w_attr(reference, "type").unwrap_or("default").to_string(),
                relationship,
                part,
            });
        }
        info
    }

    fn note(&mut self, parent: NodeId, el: X) {
        let kind = NodeKind::Note {
            kind: note_kind(el),
            note_id: w_attr(el, "id").unwrap_or_default().to_string(),
            separator: matches!(
                w_attr(el, "type"),
                Some("separator" | "continuationSeparator" | "continuationNotice")
            ),
        };
        let id = self.push(Some(parent), kind, Support::ReadOnly(IN_NOTE), el);
        for c in elements(el) {
            self.block(id, c, Some(IN_NOTE));
        }
        self.close(id, el);
    }

    fn record_field(&mut self, open: &OpenField) {
        let info = FieldInfo {
            instruction: Arc::from(open.instruction.as_str()),
            locked: open.locked,
        };
        self.field_index.fields.insert(open.field, info);
    }

    fn marker(&mut self, parent: NodeId, el: X) -> NodeId {
        let id = w_attr(el, "id").map(str::to_string);
        match el.tag_name().name() {
            "moveFromRangeStart" | "moveToRangeStart" => self
                .moves
                .push((id.clone(), w_attr(el, "name").map(str::to_string))),
            "moveFromRangeEnd" | "moveToRangeEnd" => {
                if let Some(i) = self.moves.iter().rposition(|(open, _)| *open == id) {
                    self.moves.remove(i);
                }
            }
            _ => {}
        }
        let kind = NodeKind::Marker {
            element: xml::display_name(el),
            marker_id: w_attr(el, "id").map(str::to_string),
            name: w_attr(el, "name")
                .or_else(|| w_attr(el, "fldCharType"))
                .map(str::to_string),
        };
        self.push(Some(parent), kind, Support::ReadOnly(NO_COMMAND), el)
    }

    fn comment(&mut self, parent: NodeId, el: X) {
        let kind = NodeKind::Comment {
            comment_id: w_attr(el, "id").unwrap_or_default().to_string(),
            author: w_attr(el, "author").map(str::to_string),
            date: w_attr(el, "date").map(str::to_string),
            initials: w_attr(el, "initials").map(str::to_string),
            parent: None,
        };
        let id = self.push(Some(parent), kind, Support::ReadOnly(IN_COMMENT), el);
        for c in elements(el) {
            self.block(id, c, Some(IN_COMMENT));
        }
        self.close(id, el);
    }

    fn opaque(&mut self, parent: NodeId, el: X) {
        let ids = el
            .descendants()
            .filter(|d| is(*d, WP, "docPr"))
            .filter_map(|d| d.attribute("id")?.parse::<u32>().ok());
        self.drawing_ids.extend(ids);
        let kind = NodeKind::Opaque {
            element: xml::display_name(el),
            namespace: el.tag_name().namespace().map(str::to_string),
            text_preview: xml::text_preview(el),
            holds: holds(el),
        };
        let id = self.push(Some(parent), kind, Support::Opaque, el);
        if matches!(self.nodes[parent.index()].kind, NodeKind::Row(_)) {
            let mut cells = Vec::new();
            let mut pending = vec![el];
            while let Some(el) = pending.pop() {
                if is(el, W, "tc") {
                    cells.push(cell_geometry(child(el, W, "tcPr")));
                    // A nested table inside this cell belongs to another grid.
                } else {
                    let children: Vec<_> = elements(el).collect();
                    pending.extend(children.into_iter().rev());
                }
            }
            if !cells.is_empty() {
                self.nodes[id.index()].preserved_cells = Some(Arc::new(cells));
            }
        }
    }
}

pub(crate) fn grid_columns(grid: X) -> Vec<u32> {
    elements(grid)
        .filter(|g| is(*g, W, "gridCol"))
        .map(|g| w_attr(g, "w").and_then(|w| w.parse().ok()).unwrap_or(0))
        .collect()
}

pub(crate) fn cell_width(tc_pr: X) -> Option<u32> {
    child(tc_pr, W, "tcW")
        .filter(|w| w_attr(*w, "type") == Some("dxa"))
        .and_then(|w| w_attr(w, "w"))
        .and_then(|w| w.parse().ok())
}

/// The same source geometry for editable cells and cells kept inside atoms.
pub(crate) fn cell_geometry(props: Option<X>) -> PreservedCell {
    PreservedCell {
        grid_span: props
            .and_then(|p| w_val(p, "gridSpan"))
            .and_then(|v| v.parse().ok())
            .unwrap_or(1),
        vertical_merge: props
            .and_then(|p| child(p, W, "vMerge"))
            .map(|v| w_attr(v, "val").unwrap_or("continue").to_owned()),
    }
}

/// Where an element's end tag starts, or its end when it is self-closing.
fn end_tag_start(el: X) -> usize {
    let range = el.range();
    let source = &el.document().input_text()[range.clone()];
    if source.ends_with("/>") {
        range.end
    } else {
        range.start
            + source
                .rfind("</")
                .expect("an element with content has an end tag")
    }
}

/// What, inside content the engine does not model, points at something
/// elsewhere, so that removing the content would leave that dangling.
fn holds(el: X) -> Option<&'static str> {
    el.descendants()
        .filter(|d| d.is_element() && d.tag_name().namespace() == Some(W))
        .find_map(|d| match d.tag_name().name() {
            "footnoteReference" | "endnoteReference" => Some("a footnote or endnote reference"),
            "commentRangeStart" | "commentRangeEnd" | "commentReference" => Some("a comment"),
            "bookmarkStart" | "bookmarkEnd" => Some("a bookmark"),
            "fldChar" => Some("part of a field"),
            "permStart" | "permEnd" => Some("a permission range"),
            "moveFromRangeStart" | "moveFromRangeEnd" | "moveToRangeStart" | "moveToRangeEnd" => {
                Some("one end of a tracked move")
            }
            _ => None,
        })
}

/// The author and time a change element records.
fn tracked(el: X) -> Tracked {
    Tracked {
        author: w_attr(el, "author").map(str::to_string),
        date: w_attr(el, "date").map(str::to_string),
    }
}

fn note_kind(el: X) -> NoteKind {
    if el.tag_name().name().starts_with("foot") {
        NoteKind::Footnote
    } else {
        NoteKind::Endnote
    }
}

fn placement(anchor: X) -> Placement {
    let position = |name, default| {
        let pos = child(anchor, WP, name);
        let value = |name| pos.and_then(|p| child(p, WP, name)).and_then(|v| v.text());
        Position {
            relative_to: pos
                .and_then(|p| p.attribute("relativeFrom"))
                .unwrap_or(default)
                .to_string(),
            offset_emu: value("posOffset").and_then(|v| v.trim().parse().ok()),
            align: value("align").map(|v| v.trim().to_string()),
        }
    };
    let distance = |name| anchor.attribute(name).and_then(|v| v.parse().ok());
    let wrap = elements(anchor)
        .filter(|w| w.tag_name().namespace() == Some(WP))
        .find_map(|w| match w.tag_name().name() {
            "wrapSquare" => Some("square"),
            "wrapTight" => Some("tight"),
            "wrapThrough" => Some("through"),
            "wrapTopAndBottom" => Some("top_and_bottom"),
            "wrapNone" => Some("none"),
            _ => None,
        });
    Placement::Anchor(Box::new(Anchor {
        horizontal: position("positionH", "column"),
        vertical: position("positionV", "paragraph"),
        wrap: wrap.unwrap_or("none"),
        behind: matches!(anchor.attribute("behindDoc"), Some("1" | "true" | "on")),
        distance_emu: Edges {
            top: distance("distT"),
            left: distance("distL"),
            bottom: distance("distB"),
            right: distance("distR"),
        },
    }))
}

/// `w:fldLock` on a begin character or simple field: Word keeps the result.
fn field_locked(el: X) -> bool {
    matches!(w_attr(el, "fldLock"), Some("1" | "true" | "on"))
}

fn is_marker(el: X) -> bool {
    el.tag_name().namespace() == Some(W)
        && matches!(
            el.tag_name().name(),
            "bookmarkStart"
                | "bookmarkEnd"
                | "commentRangeStart"
                | "commentRangeEnd"
                | "commentReference"
                | "proofErr"
                | "permStart"
                | "permEnd"
                | "moveFromRangeStart"
                | "moveFromRangeEnd"
                | "moveToRangeStart"
                | "moveToRangeEnd"
                | "lastRenderedPageBreak"
                | "fldChar"
        )
}

pub(crate) fn run_props(rpr: X, theme: &Theme) -> RunProps {
    RunProps {
        bold: on_off(rpr, "b"),
        italic: on_off(rpr, "i"),
        underline: w_val(rpr, "u").map(str::to_string),
        strike: on_off(rpr, "strike").or(on_off(rpr, "dstrike")),
        caps: on_off(rpr, "caps"),
        small_caps: on_off(rpr, "smallCaps"),
        hidden: on_off(rpr, "vanish"),
        size_half_points: w_val(rpr, "sz").and_then(|v| v.parse().ok()),
        color: w_val(rpr, "color").map(str::to_string),
        highlight: w_val(rpr, "highlight").map(str::to_string),
        font: child(rpr, W, "rFonts").and_then(|f| font(f, theme)),
        vert_align: w_val(rpr, "vertAlign").map(str::to_string),
    }
}

/// The font Latin text asks for. A theme font wins over a named one in the
/// same slot, and the ASCII slot over the high-ANSI one.
fn font(fonts: X, theme: &Theme) -> Option<String> {
    let themed = |attr| w_attr(fonts, attr).and_then(|role| theme.font(role));
    let named = |attr| w_attr(fonts, attr).map(str::to_string);
    themed("asciiTheme")
        .or_else(|| named("ascii"))
        .or_else(|| themed("hAnsiTheme"))
        .or_else(|| named("hAnsi"))
}

pub(crate) fn para_props(ppr: X) -> ParaProps {
    let ind = child(ppr, W, "ind");
    let ind_attr = |names: &[&str]| {
        ind.and_then(|i| names.iter().find_map(|n| w_attr(i, n)))
            .and_then(|v| v.parse::<i32>().ok())
    };
    let spacing = child(ppr, W, "spacing");
    let spacing_attr = |name| spacing.and_then(|s| w_attr(s, name));
    let twips = |name| spacing_attr(name).and_then(|v| v.parse().ok());
    let auto = |name| spacing_attr(name).map(|v| !matches!(v, "0" | "false" | "off"));
    let line = twips("line");
    ParaProps {
        alignment: w_val(ppr, "jc").map(str::to_string),
        outline_level: w_val(ppr, "outlineLvl").and_then(|v| v.parse().ok()),
        numbering: child(ppr, W, "numPr").map(|num| NumRef {
            num_id: w_val(num, "numId").unwrap_or("0").to_string(),
            level: w_val(num, "ilvl").and_then(|v| v.parse().ok()).unwrap_or(0),
        }),
        indent_left: ind_attr(&["left", "start"]),
        indent_right: ind_attr(&["right", "end"]),
        indent_first_line: ind_attr(&["hanging"])
            .map(|h| -h)
            .or_else(|| ind_attr(&["firstLine"])),
        spacing_before: twips("before"),
        spacing_after: twips("after"),
        before_autospacing: auto("beforeAutospacing"),
        after_autospacing: auto("afterAutospacing"),
        line,
        // A line height stated without a rule is proportional.
        line_rule: spacing_attr("lineRule")
            .or(line.and(Some("auto")))
            .map(str::to_string),
        contextual_spacing: on_off(ppr, "contextualSpacing"),
        keep_next: on_off(ppr, "keepNext"),
        keep_lines: on_off(ppr, "keepLines"),
        widow_control: on_off(ppr, "widowControl"),
        page_break_before: on_off(ppr, "pageBreakBefore"),
        tabs: child(ppr, W, "tabs")
            .map(|tabs| {
                elements(tabs)
                    .filter(|t| is(*t, W, "tab"))
                    .filter_map(|t| {
                        Some(TabDef {
                            pos: w_attr(t, "pos")?.parse().ok()?,
                            align: w_attr(t, "val")?.to_string(),
                            leader: w_attr(t, "leader").map(str::to_string),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

pub(crate) fn table_props(tbl_pr: X) -> TableProps {
    let borders = child(tbl_pr, W, "tblBorders");
    let inside = |name| borders.and_then(|b| child(b, W, name)).and_then(border);
    TableProps {
        borders: TableBorders {
            outer: borders.map(|b| edges(b, border)).unwrap_or_default(),
            inside_h: inside("insideH"),
            inside_v: inside("insideV"),
        },
        cell_margins: child(tbl_pr, W, "tblCellMar")
            .map(|m| edges(m, twips))
            .unwrap_or_default(),
        indent: child(tbl_pr, W, "tblInd").and_then(twips),
        alignment: w_val(tbl_pr, "jc").map(str::to_string),
        shading: None,
    }
}

fn cell_props(tc_pr: X) -> CellProps {
    CellProps {
        borders: child(tc_pr, W, "tcBorders")
            .map(|b| edges(b, border))
            .unwrap_or_default(),
        shading: shading(tc_pr),
        margins: child(tc_pr, W, "tcMar")
            .map(|m| edges(m, twips))
            .unwrap_or_default(),
        vertical_align: w_val(tc_pr, "vAlign").map(str::to_string),
    }
}

/// The four sides an element lists, under either name for left and right.
fn edges<T>(el: X, read: impl Fn(X) -> Option<T>) -> Edges<T> {
    let side = |names: &[&str]| names.iter().find_map(|n| child(el, W, n)).and_then(&read);
    Edges {
        top: side(&["top"]),
        left: side(&["left", "start"]),
        bottom: side(&["bottom"]),
        right: side(&["right", "end"]),
    }
}

fn border(el: X) -> Option<Border> {
    Some(Border {
        style: w_attr(el, "val")?.to_string(),
        size_eighths: w_attr(el, "sz").and_then(|v| v.parse().ok()).unwrap_or(0),
        color: w_attr(el, "color")
            .filter(|c| *c != "auto")
            .map(str::to_string),
    })
}

/// A width in twips. Percentages and `auto` widths are not modelled.
fn twips(el: X) -> Option<i32> {
    match w_attr(el, "type") {
        None | Some("dxa") => w_attr(el, "w")?.parse().ok(),
        _ => None,
    }
}

/// A fill colour as stated, `auto` included, so a layer can clear one.
fn shading(pr: X) -> Option<String> {
    child(pr, W, "shd")
        .and_then(|s| w_attr(s, "fill"))
        .map(str::to_string)
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize)]
pub struct Style {
    pub id: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub based_on: Option<String>,
    /// The style Word gives a new paragraph after one in this style (`w:next`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub is_default: bool,
    #[serde(skip_serializing_if = "RunProps::is_empty")]
    pub run: RunProps,
    pub paragraph: ParaProps,
    /// A table style's own table properties, before conditional formatting.
    #[serde(skip)]
    pub table: TableProps,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Styles {
    pub part: Option<String>,
    pub default_run: RunProps,
    pub default_paragraph: ParaProps,
    pub styles: BTreeMap<String, Style>,
}

impl Styles {
    pub fn parse(part: &str, text: &str, theme: &Theme) -> Result<Self, crate::OpenError> {
        let doc = xml::parse(part, text)?;
        let root = doc.root_element();
        let mut styles = Styles {
            part: Some(part.to_string()),
            ..Styles::default()
        };
        let defaults = child(root, W, "docDefaults");
        let default = |name, props| {
            defaults
                .and_then(|d| child(d, W, name))
                .and_then(|d| child(d, W, props))
        };
        if let Some(rpr) = default("rPrDefault", "rPr") {
            styles.default_run = run_props(rpr, theme);
        }
        if let Some(ppr) = default("pPrDefault", "pPr") {
            styles.default_paragraph = para_props(ppr);
        }
        for el in elements(root).filter(|e| is(*e, W, "style")) {
            let Some(id) = w_attr(el, "styleId") else {
                continue;
            };
            styles.styles.insert(
                id.to_string(),
                Style {
                    id: id.to_string(),
                    kind: w_attr(el, "type").unwrap_or("paragraph").to_string(),
                    name: w_val(el, "name").map(str::to_string),
                    based_on: w_val(el, "basedOn").map(str::to_string),
                    next: w_val(el, "next").map(str::to_string),
                    is_default: matches!(w_attr(el, "default"), Some("1" | "true" | "on")),
                    run: child(el, W, "rPr")
                        .map(|r| run_props(r, theme))
                        .unwrap_or_default(),
                    paragraph: child(el, W, "pPr").map(para_props).unwrap_or_default(),
                    table: TableProps {
                        shading: child(el, W, "tcPr").and_then(shading),
                        ..child(el, W, "tblPr").map(table_props).unwrap_or_default()
                    },
                },
            );
        }
        Ok(styles)
    }

    /// The style and its `basedOn` ancestors, root first. Cycles stop at the
    /// first repeat.
    pub fn chain(&self, id: &str) -> Vec<&Style> {
        let mut chain = Vec::new();
        let mut next = Some(id);
        while let Some(id) = next {
            let Some(style) = self.styles.get(id) else {
                break;
            };
            if chain.iter().any(|s: &&Style| s.id == style.id) {
                break;
            }
            chain.push(style);
            next = style.based_on.as_deref();
        }
        chain.reverse();
        chain
    }

    pub fn default_paragraph_style(&self) -> Option<&Style> {
        self.styles
            .values()
            .find(|s| s.kind == "paragraph" && s.is_default)
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize)]
pub struct Level {
    pub format: String,
    pub text: String,
    pub start: u32,
    /// What follows the marker: `tab`, `space` or `nothing`.
    pub suffix: &'static str,
    /// `left`, `center` or `right`.
    pub justification: &'static str,
    /// The marker's own run properties.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<RunProps>,
    /// The indents and tabs a paragraph in the list takes.
    #[serde(skip)]
    pub paragraph: ParaProps,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Numbering {
    pub part: Option<String>,
    /// `w:num` instances: numId → abstractNumId.
    pub instances: BTreeMap<String, String>,
    /// abstractNumId → level index → level definition.
    pub abstracts: BTreeMap<String, BTreeMap<u8, Level>>,
    /// Every `w:num`'s numId, those naming no definition included, so a
    /// list a command adds takes a number no instance has.
    #[serde(skip)]
    pub num_ids: BTreeSet<String>,
    /// The highest numeric `w:abstractNumId`.
    #[serde(skip)]
    pub highest_abstract_id: Option<u32>,
}

impl Numbering {
    pub fn parse(part: &str, text: &str, theme: &Theme) -> Result<Self, crate::OpenError> {
        let doc = xml::parse(part, text)?;
        let mut numbering = Numbering {
            part: Some(part.to_string()),
            ..Numbering::default()
        };
        for el in elements(doc.root_element()) {
            if is(el, W, "abstractNum") {
                let Some(id) = w_attr(el, "abstractNumId") else {
                    continue;
                };
                if let Ok(n) = id.parse::<u32>() {
                    numbering.highest_abstract_id = numbering.highest_abstract_id.max(Some(n));
                }
                let levels = elements(el)
                    .filter(|l| is(*l, W, "lvl"))
                    .filter_map(|l| {
                        let ilvl = w_attr(l, "ilvl")?.parse().ok()?;
                        Some((
                            ilvl,
                            Level {
                                format: w_val(l, "numFmt").unwrap_or("decimal").to_string(),
                                text: w_val(l, "lvlText").unwrap_or_default().to_string(),
                                start: w_val(l, "start").and_then(|v| v.parse().ok()).unwrap_or(1),
                                suffix: match w_val(l, "suff") {
                                    Some("space") => "space",
                                    Some("nothing") => "nothing",
                                    _ => "tab",
                                },
                                justification: match w_val(l, "lvlJc") {
                                    Some("center") => "center",
                                    Some("right" | "end") => "right",
                                    _ => "left",
                                },
                                run: child(l, W, "rPr")
                                    .map(|r| run_props(r, theme))
                                    .filter(|r| !r.is_empty()),
                                paragraph: child(l, W, "pPr").map(para_props).unwrap_or_default(),
                            },
                        ))
                    })
                    .collect();
                numbering.abstracts.insert(id.to_string(), levels);
            } else if is(el, W, "num") {
                if let Some(num_id) = w_attr(el, "numId") {
                    numbering.num_ids.insert(num_id.to_string());
                }
                if let (Some(num_id), Some(abs)) = (w_attr(el, "numId"), w_val(el, "abstractNumId"))
                {
                    numbering
                        .instances
                        .insert(num_id.to_string(), abs.to_string());
                }
            }
        }
        Ok(numbering)
    }

    pub fn level(&self, num: &NumRef) -> Option<&Level> {
        let abs = self.instances.get(&num.num_id)?;
        self.abstracts.get(abs)?.get(&num.level)
    }
}

/// The theme's Latin fonts, which `w:rFonts` can name by role.
#[derive(Debug, Clone, Default)]
pub struct Theme {
    major: Option<String>,
    minor: Option<String>,
}

impl Theme {
    pub fn parse(part: &str, text: &str) -> Result<Self, crate::OpenError> {
        let doc = xml::parse(part, text)?;
        let scheme = doc.descendants().find(|d| is(*d, A, "fontScheme"));
        let latin = |name| {
            scheme
                .and_then(|s| child(s, A, name))
                .and_then(|f| child(f, A, "latin"))
                .and_then(|l| l.attribute("typeface"))
                .filter(|t| !t.is_empty())
                .map(str::to_string)
        };
        Ok(Theme {
            major: latin("majorFont"),
            minor: latin("minorFont"),
        })
    }

    /// The font for a role such as `minorHAnsi`.
    fn font(&self, role: &str) -> Option<String> {
        if role.starts_with("major") {
            self.major.clone()
        } else if role.starts_with("minor") {
            self.minor.clone()
        } else {
            None
        }
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize)]
pub struct Settings {
    pub default_tab_stop_twips: u32,
    pub even_and_odd_headers: bool,
    /// `w:updateFields`: Word updates the fields when it opens the file.
    pub update_fields: bool,
    #[serde(skip)]
    pub footnotes: NoteNumbering,
    #[serde(skip)]
    pub endnotes: NoteNumbering,
}

/// How a document numbers its footnotes or endnotes.
#[derive(Debug, Clone)]
pub struct NoteNumbering {
    pub format: String,
    pub start: u32,
}

impl Default for Settings {
    fn default() -> Self {
        let numbering = |format: &str| NoteNumbering {
            format: format.to_string(),
            start: 1,
        };
        Settings {
            default_tab_stop_twips: 720,
            even_and_odd_headers: false,
            update_fields: false,
            footnotes: numbering("decimal"),
            endnotes: numbering("lowerRoman"),
        }
    }
}

impl Settings {
    pub fn parse(part: &str, text: &str) -> Result<Self, crate::OpenError> {
        let doc = xml::parse(part, text)?;
        let root = doc.root_element();
        let mut settings = Settings::default();
        if let Some(stop) = w_val(root, "defaultTabStop").and_then(|v| v.parse().ok()) {
            settings.default_tab_stop_twips = stop;
        }
        settings.even_and_odd_headers = on_off(root, "evenAndOddHeaders").unwrap_or(false);
        settings.update_fields = on_off(root, "updateFields").unwrap_or(false);
        for (name, numbering) in [
            ("footnotePr", &mut settings.footnotes),
            ("endnotePr", &mut settings.endnotes),
        ] {
            let Some(pr) = child(root, W, name) else {
                continue;
            };
            if let Some(format) = w_val(pr, "numFmt") {
                numbering.format = format.to_string();
            }
            if let Some(start) = w_val(pr, "numStart").and_then(|v| v.parse().ok()) {
                numbering.start = start;
            }
        }
        Ok(settings)
    }
}
