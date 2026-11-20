//! Tree navigation, structural command checks and projection primitives. Detaching
//! a node preserves its source identity and XML in the document arena.

use std::collections::BTreeMap;

use crate::edit::{Point, TextRange};
use crate::error::EditError;
use crate::import::NO_EDIT;
use crate::ir::{DrawingInfo, Made, Node, NodeId, NodeKind, ParagraphInfo, RunInfo, Support};
use crate::xml::{escape_attr, qname, W};
use crate::Document;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Loc {
    /// Inside a text node, at a scalar offset.
    Text(NodeId, usize),
    /// Before the child at `index` of a container.
    Child(NodeId, usize),
}

pub(crate) enum Leaf {
    Text(String),
    Break,
    Picture(Box<DrawingInfo>),
}

/// One step of a paragraph's inline walk: a position between children, a
/// text node, or an item the shared state holds whole.
#[derive(Clone, Copy)]
pub(crate) enum Walk {
    At(NodeId, usize),
    Text(NodeId),
    Item(NodeId),
}

/// What a range covers in document order: the characters of each text node,
/// the inline items, and the blocks after its first paragraph up to its last,
/// whose paragraphs join the first.
#[derive(Default)]
pub(crate) struct Covered {
    pub(crate) texts: Vec<(NodeId, TextRange)>,
    pub(crate) items: Vec<NodeId>,
    pub(crate) blocks: Vec<NodeId>,
}

/// Where a location is in a paragraph's walk.
pub(crate) fn walk_index(walk: &[Walk], loc: Loc) -> usize {
    walk.iter()
        .position(|step| match (*step, loc) {
            (Walk::At(c, i), Loc::Child(lc, li)) => c == lc && i == li,
            (Walk::Text(t), Loc::Text(lt, _)) => t == lt,
            _ => false,
        })
        .expect("inside its paragraph")
}

fn local(element: &str) -> &str {
    element.rsplit(':').next().unwrap_or(element)
}

pub(crate) fn refuse<T>(node: NodeId, reason: &'static str) -> Result<T, EditError> {
    Err(EditError::Refused { node, reason })
}

/// Whether a node of kind `child` may sit directly inside `parent`.
pub(crate) fn fits(child: &NodeKind, parent: &NodeKind) -> bool {
    use NodeKind::*;
    match parent {
        // A body ends with its own section properties.
        Story { .. } if matches!(child, Note { .. } | Comment { .. } | Section(_)) => true,
        Story { .. } | Cell(_) | Note { .. } => matches!(
            child,
            Paragraph(_) | Table(_) | ContentControl { .. } | Opaque { .. } | Marker { .. }
        ),
        ContentControl { .. } => true,
        Paragraph(_) | Hyperlink(_) | Revision(_) | Field { .. } => matches!(
            child,
            Run(_)
                | Hyperlink(_)
                | Revision(_)
                | Field { .. }
                | ContentControl { .. }
                | Opaque { .. }
                | Marker { .. }
        ),
        Run(_) => matches!(
            child,
            Text { .. }
                | DeletedText { .. }
                | FieldCode { .. }
                | Tab
                | Break { .. }
                | Drawing(_)
                | NoteReference { .. }
                | NoteMark { .. }
                | Marker { .. }
                | Opaque { .. }
        ),
        Table(_) => matches!(child, Row(_) | Marker { .. } | Opaque { .. }),
        Row(_) => matches!(child, Cell(_) | Marker { .. } | Opaque { .. }),
        _ => false,
    }
}

fn holds_reason(what: &str) -> &'static str {
    match what {
        "a footnote or endnote reference" => {
            "it holds a footnote or endnote reference; removing it would orphan the note"
        }
        "a comment" => "it holds a comment; no command removes comments yet",
        "a bookmark" => "it holds a bookmark whose other end may be elsewhere",
        "part of a field" => "it holds part of a field whose other parts may be elsewhere",
        "a permission range" => "it holds a permission range whose other end may be elsewhere",
        _ => "it holds one end of a tracked move; accept or reject the move first",
    }
}

impl Document {
    pub(crate) fn n(&self, id: NodeId) -> &Node {
        &self.nodes[id.index()]
    }

    /// The node, if it is in the document.
    pub(crate) fn attached(&self, id: NodeId) -> Result<&Node, EditError> {
        let node = self
            .nodes
            .get(id.index())
            .ok_or(EditError::UnknownNode { node: id })?;
        let mut top = node;
        while let Some(parent) = top.parent {
            top = self.n(parent);
        }
        match top.kind {
            NodeKind::Story { .. } => Ok(node),
            _ => Err(EditError::Detached { node: id }),
        }
    }

    pub(crate) fn parent_of(&self, id: NodeId) -> Result<NodeId, EditError> {
        match self.attached(id)?.parent {
            Some(parent) => Ok(parent),
            None => refuse(id, "a story is not inside anything"),
        }
    }

    pub(crate) fn index_of(&self, id: NodeId) -> usize {
        let parent = self.n(id).parent.expect("only a story has no parent");
        self.n(parent)
            .children
            .iter()
            .position(|&c| c == id)
            .expect("a parent lists its children")
    }

    /// Refuses unless the node and every ancestor may be edited.
    pub(crate) fn check_editable(&self, id: NodeId) -> Result<(), EditError> {
        let mut next = Some(id);
        while let Some(id) = next {
            let node = self.n(id);
            match node.support {
                Support::Editable => {}
                Support::ReadOnly(reason) => return Err(EditError::ReadOnly { node: id, reason }),
                Support::Opaque => {
                    return Err(EditError::ReadOnly {
                        node: id,
                        reason: "inside content the engine does not model",
                    })
                }
            }
            next = node.parent;
        }
        Ok(())
    }

    fn check_container(&self, id: NodeId) -> Result<(), EditError> {
        match &self.n(id).kind {
            NodeKind::Paragraph(_) => Ok(()),
            _ if self.entered(id) => Ok(()),
            kind => Err(EditError::WrongKind {
                node: id,
                kind: kind.name(),
                expected: "a paragraph, run, hyperlink or tracked insertion",
            }),
        }
    }

    /// Whether a paragraph's walk enters a node: a run, a hyperlink, or a
    /// tracked insertion a command made, whose text can be typed in.
    pub(crate) fn entered(&self, id: NodeId) -> bool {
        matches!(self.n(id).kind, NodeKind::Run(_) | NodeKind::Hyperlink(_))
            || self.made_insertion(id)
    }

    pub(crate) fn text_len(&self, id: NodeId) -> Result<usize, EditError> {
        match &self.n(id).kind {
            NodeKind::Text { text } => Ok(text.chars().count()),
            kind => Err(EditError::NotText {
                node: id,
                kind: kind.name(),
            }),
        }
    }

    pub(crate) fn locate(&self, at: Point) -> Result<Loc, EditError> {
        match at {
            Point::Text { node, offset } => {
                self.attached(node)?;
                let len = self.text_len(node)?;
                if offset > len {
                    return Err(EditError::Range {
                        node,
                        start: offset,
                        end: offset,
                        len,
                    });
                }
                Ok(Loc::Text(node, offset))
            }
            Point::Before { before: node } | Point::After { after: node } => {
                let parent = self.parent_of(node)?;
                let after = matches!(at, Point::After { .. });
                Ok(Loc::Child(parent, self.index_of(node) + usize::from(after)))
            }
            Point::Start { start: node } | Point::End { end: node } => {
                let len = self.attached(node)?.children.len();
                self.check_container(node)?;
                let end = matches!(at, Point::End { .. });
                Ok(Loc::Child(node, if end { len } else { 0 }))
            }
        }
    }

    /// A point at either end of a text node becomes the position beside it.
    pub(crate) fn edge(&self, loc: Loc) -> Result<Loc, EditError> {
        let Loc::Text(text, offset) = loc else {
            return Ok(loc);
        };
        let len = self.text_len(text)?;
        if offset != 0 && offset != len {
            return Ok(loc);
        }
        let parent = self.n(text).parent.expect("located nodes are attached");
        let after = offset != 0;
        Ok(Loc::Child(parent, self.index_of(text) + usize::from(after)))
    }

    pub(crate) fn paragraph_of(&self, mut id: NodeId) -> Option<NodeId> {
        loop {
            if matches!(self.n(id).kind, NodeKind::Paragraph(_)) {
                return Some(id);
            }
            id = self.n(id).parent?;
        }
    }

    /// Markers, and runs, hyperlinks and insertions holding only markers,
    /// take no space.
    pub(crate) fn invisible(&self, id: NodeId) -> bool {
        match self.n(id).kind {
            NodeKind::Marker { .. } => true,
            _ if self.entered(id) => self.n(id).children.iter().all(|&c| self.invisible(c)),
            _ => false,
        }
    }

    /// Whether only invisible nodes lie between a location and the end
    /// (`forward`) or start of its paragraph.
    fn bare_to_edge(
        &self,
        paragraph: NodeId,
        container: NodeId,
        index: usize,
        forward: bool,
    ) -> bool {
        let (mut current, mut at) = (container, index);
        loop {
            let children = &self.n(current).children;
            let side = if forward {
                &children[at..]
            } else {
                &children[..at]
            };
            if !side.iter().all(|&c| self.invisible(c)) {
                return false;
            }
            if current == paragraph {
                return true;
            }
            at = self.index_of(current) + usize::from(forward);
            current = self.n(current).parent.expect("below the paragraph");
        }
    }

    pub(crate) fn split_edge(&self, paragraph: NodeId, loc: Loc) -> Loc {
        match loc {
            Loc::Child(c, i) if self.bare_to_edge(paragraph, c, i, true) => {
                Loc::Child(paragraph, self.n(paragraph).children.len())
            }
            Loc::Child(c, i) if self.bare_to_edge(paragraph, c, i, false) => {
                Loc::Child(paragraph, 0)
            }
            other => other,
        }
    }

    pub(crate) fn field_char(&self, id: NodeId) -> Option<&str> {
        match &self.n(id).kind {
            NodeKind::Marker { element, name, .. } if local(element) == "fldChar" => {
                name.as_deref()
            }
            _ => None,
        }
    }

    /// Complex fields begun minus fields ended within a subtree.
    fn field_balance(&self, id: NodeId) -> i64 {
        let own = match self.field_char(id) {
            Some("begin") => 1,
            Some("end") => -1,
            _ => 0,
        };
        own + self
            .n(id)
            .children
            .iter()
            .map(|&c| self.field_balance(c))
            .sum::<i64>()
    }

    /// How many complex fields are open at a location: everything before it
    /// in document order counts, from the start of its story, because a field
    /// can span paragraphs. A story without a field's characters has none
    /// open anywhere, so the walk is skipped.
    fn field_depth(&self, container: NodeId, index: usize) -> i64 {
        if !self.stories[self.n(container).story as usize].fields {
            return 0;
        }
        let (mut current, mut at) = (container, index);
        let mut depth = 0;
        loop {
            depth += self.n(current).children[..at]
                .iter()
                .map(|&c| self.field_balance(c))
                .sum::<i64>();
            let Some(parent) = self.n(current).parent else {
                return depth;
            };
            at = self.index_of(current);
            current = parent;
        }
    }

    pub(crate) fn check_outside_fields(
        &self,
        container: NodeId,
        index: usize,
    ) -> Result<(), EditError> {
        if self.field_depth(container, index) > 0 {
            return refuse(container, "inside a field result; the field regenerates it");
        }
        Ok(())
    }

    /// A point for text to go in or a range to end at, outside every field.
    /// A point inside a field with nothing shown between it and the field's
    /// end, or between it and the start of the field's result, reads as
    /// beside the field, so it becomes the point after the field or before
    /// it; `forward` says which to look for first. Any other point inside a
    /// field is refused: a range cannot cut into an instruction or a result.
    pub(crate) fn outside_fields(&self, point: Point, forward: bool) -> Result<Point, EditError> {
        let loc = self.locate(point)?;
        let (container, index) = self.end_position(loc, true);
        let depth = self.field_depth(container, index);
        if depth == 0 {
            return Ok(point);
        }
        let inside = || refuse(container, "inside a field result; the field regenerates it");
        let mut walk = Vec::new();
        self.inline_walk(self.paragraph_at(loc)?, &mut walk);
        let found = walk.iter().position(|step| match (*step, loc) {
            (Walk::At(c, i), Loc::Child(lc, li)) => c == lc && i == li,
            (Walk::Text(t), Loc::Text(lt, _)) => t == lt,
            _ => false,
        });
        let Some(at) = found else {
            return inside();
        };
        let (before, after) = match loc {
            Loc::Child(..) => (&walk[..at], &walk[at..]),
            Loc::Text(text, offset) => (
                if offset == 0 { &walk[..at] } else { &[][..] },
                if offset == self.text_len(text)? {
                    &walk[at + 1..]
                } else {
                    &[][..]
                },
            ),
        };
        let past_end = || {
            let mut open = depth;
            for step in after {
                match *step {
                    Walk::At(..) => {}
                    Walk::Item(id) if self.field_char(id) == Some("end") => {
                        open -= 1;
                        if open == 0 {
                            return Some(Point::After { after: id });
                        }
                    }
                    _ => return None,
                }
            }
            None
        };
        let before_begin = || {
            let (mut open, mut in_result) = (depth, false);
            for step in before.iter().rev() {
                match *step {
                    Walk::At(..) => {}
                    Walk::Item(id) => match (self.field_char(id), &self.n(id).kind) {
                        (Some("separate"), _) => in_result = true,
                        (None, NodeKind::FieldCode { .. }) if in_result => {}
                        (Some("begin"), _) if in_result => {
                            (open, in_result) = (open - 1, false);
                            if open == 0 {
                                return Some(Point::Before { before: id });
                            }
                        }
                        _ => return None,
                    },
                    Walk::Text(_) => return None,
                }
            }
            None
        };
        let found = if forward {
            past_end().or_else(before_begin)
        } else {
            before_begin().or_else(past_end)
        };
        found.map_or_else(inside, Ok)
    }

    /// Refuses a paragraph whose mark holds something a split or join would
    /// have to move or copy: a section break, or a tracked change.
    pub(crate) fn check_mark(&self, id: NodeId) -> Result<(), EditError> {
        self.check_source_mark(id)?;
        if self.made_mark(id) {
            return refuse(
                id,
                "its paragraph mark is a tracked change; accept or reject it first",
            );
        }
        Ok(())
    }

    /// Refuses what `check_mark` refuses, apart from a tracked change a
    /// command made, which a command moves to the mark that stays.
    pub(crate) fn check_source_mark(&self, id: NodeId) -> Result<(), EditError> {
        let NodeKind::Paragraph(info) = &self.n(id).kind else {
            return Ok(());
        };
        if info.section.is_some() {
            return refuse(
                id,
                "its paragraph mark ends a section; no command moves a section break yet",
            );
        }
        if info.mark_revision.is_some() && !self.made_mark(id) {
            return refuse(
                id,
                "its paragraph mark is a tracked change; accept or reject it first",
            );
        }
        if info.format_revision {
            return refuse(
                id,
                "it has a tracked property change; accept or reject it first",
            );
        }
        Ok(())
    }

    pub(crate) fn story_text(&self, node: &Node) -> &str {
        &self.stories[node.story as usize].xml.text
    }

    /// The node's qualified element name, as its markup spells it.
    pub(crate) fn element_name(&self, id: NodeId) -> String {
        let node = self.n(id);
        match &node.made {
            Some(made) => made.head[1..]
                .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
                .next()
                .unwrap_or_default()
                .to_string(),
            None => qname(self.story_text(node), &node.range).to_string(),
        }
    }

    /// A WordprocessingML element name with the prefix `near` uses.
    pub(crate) fn w_name(&self, near: NodeId, local: &str) -> String {
        match self.element_name(near).split_once(':') {
            Some((prefix, _)) => format!("{prefix}:{local}"),
            None => local.to_string(),
        }
    }

    /// A WordprocessingML attribute name with the prefix `near` uses.
    /// Attributes never take a default namespace, so a document that writes
    /// its elements unprefixed gets a local declaration.
    pub(crate) fn w_attr_name(&self, near: NodeId, local: &str) -> String {
        match self.element_name(near).split_once(':') {
            Some((prefix, _)) => format!("{prefix}:{local}"),
            None => format!("xmlns:w=\"{W}\" w:{local}"),
        }
    }

    /// A container's start tag and property elements.
    pub(crate) fn head(&self, id: NodeId) -> String {
        let node = self.n(id);
        self.tags(self.story_text(node), node).0.into_owned()
    }

    pub(crate) fn create(
        &mut self,
        kind: NodeKind,
        support: Support,
        made: Made,
        story: u32,
    ) -> NodeId {
        let id = self.nodes.next_id();
        self.nodes.push(Node {
            id,
            parent: None,
            children: Vec::new(),
            kind,
            support,
            preserved_cells: None,
            range: 0..0,
            lead: 0,
            inner: 0..0,
            made: Some(Box::new(made)),
            story,
            dirty: false,
            dirty_below: false,
            head: None,
        });
        if self.field_char(id).is_some() {
            self.stories[story as usize].fields = true;
        }
        if self.comment_mark(id).is_some() {
            self.stories[story as usize].comments = true;
        }
        id
    }

    /// The node's children changed, so it and its ancestors are re-walked.
    pub(crate) fn touch(&mut self, id: NodeId) {
        self.nodes[id.index()].dirty_below = true;
        self.mark_ancestors(id);
    }

    pub(crate) fn attach(&mut self, id: NodeId, parent: NodeId, index: usize) {
        self.nodes[parent.index()].children.insert(index, id);
        self.nodes[id.index()].parent = Some(parent);
        self.touch(parent);
    }

    pub(crate) fn detach(&mut self, id: NodeId) -> (NodeId, usize) {
        let parent = self.n(id).parent.expect("only attached nodes are detached");
        let index = self.index_of(id);
        self.nodes[parent.index()].children.remove(index);
        self.nodes[id.index()].parent = None;
        self.touch(parent);
        (parent, index)
    }

    /// A new, empty node like `id`, to take `id`'s children from `index` on.
    ///
    /// A paragraph copy has a bare start tag, so Word's paragraph IDs stay
    /// unique, and a copy of the paragraph properties. After an Enter at the
    /// end of a paragraph whose style names a different next style, it gets
    /// that style alone, as in Word: a heading is followed by body text. The
    /// default style is written as no style, as Word writes it. The copy is
    /// like `id`, as [`like`](Document::like) reports, unless it is bare.
    pub(crate) fn copy_of(&mut self, id: NodeId, index: usize) -> NodeId {
        let node = self.n(id);
        let story = node.story;
        let name = self.element_name(id);
        let tail = format!("</{name}>");
        let mut like = self.like(id);
        let (kind, head) = match &node.kind {
            NodeKind::Paragraph(info) => {
                let at_end = node.children[index..].iter().all(|&c| self.invisible(c));
                match self.next_style(info).filter(|_| at_end) {
                    Some(next) => {
                        let default = self.styles.default_paragraph_style();
                        let style = (default.is_none_or(|s| s.id != next)).then_some(next);
                        let head = match &style {
                            Some(next) => {
                                let ppr = self.w_name(id, "pPr");
                                let style = self.w_name(id, "pStyle");
                                let val = self.w_attr_name(id, "val");
                                format!(
                                    "<{name}><{ppr}><{style} {val}=\"{}\"/></{ppr}>",
                                    escape_attr(next)
                                )
                            }
                            None => format!("<{name}>"),
                        };
                        let info = ParagraphInfo {
                            style,
                            ..ParagraphInfo::default()
                        };
                        like = None;
                        (NodeKind::Paragraph(Box::new(info)), head)
                    }
                    None => {
                        let (info, props) = self.paragraph_like(id, false);
                        (
                            NodeKind::Paragraph(Box::new(info)),
                            format!("<{name}>{props}"),
                        )
                    }
                }
            }
            kind => (kind.clone(), self.head(id)),
        };
        self.create(kind, Support::Editable, Made { head, tail, like }, story)
    }

    pub(crate) fn next_style(&self, info: &ParagraphInfo) -> Option<String> {
        if info.props.numbering.is_some() {
            return None;
        }
        let current = info
            .style
            .as_deref()
            .or_else(|| self.styles.default_paragraph_style().map(|s| s.id.as_str()))?;
        let next = self.styles.styles.get(current)?.next.as_deref()?;
        (next != current).then(|| next.to_string())
    }

    /// A run's properties and start tag like `like`, for a run in `near`.
    pub(crate) fn run_from(
        &self,
        near: NodeId,
        like: Option<NodeId>,
    ) -> Result<(RunInfo, String, Option<NodeId>), EditError> {
        let name = self.w_name(near, "r");
        let Some(like) = like else {
            return Ok((RunInfo::default(), format!("<{name}>"), None));
        };
        let node = self.source_node(like)?;
        match &node.kind {
            NodeKind::Run(info) if !info.format_revision => {
                let head = self.source_tags(self.story_text(node), node).0.into_owned();
                let mut info = RunInfo::clone(info);
                let rpr = self.source_properties(like, "rPr");
                (info.style, info.props) = self.run_properties_from(like, &rpr)?;
                Ok((info, head, Some(like)))
            }
            NodeKind::Paragraph(info) => match info.mark_run.as_deref() {
                Some(mark) if info.mark_revision.is_none() && !mark.info.format_revision => {
                    let props = &self.story_text(node)[mark.range.clone()];
                    Ok((mark.info.clone(), format!("<{name}>{props}"), Some(like)))
                }
                _ => refuse(like, "its paragraph mark has no run properties to copy"),
            },
            _ => Err(EditError::WrongKind {
                node: like,
                kind: node.kind.name(),
                expected: "a run without a tracked formatting change, or a paragraph",
            }),
        }
    }

    /// The information and properties element of a paragraph made like
    /// `like`: its properties without tracked changes, conditional
    /// formatting or a section break, as it has them now, or as its source
    /// has them when `source`.
    pub(crate) fn paragraph_like(&self, like: NodeId, source: bool) -> (ParagraphInfo, String) {
        let NodeKind::Paragraph(info) = &self.n(like).kind else {
            unreachable!("a paragraph");
        };
        // A change a command recorded on the mark is not the source's.
        let tracked = info.mark_revision.is_some() && !(source && self.made_mark(like));
        let info = ParagraphInfo {
            word_para_id: None,
            mark_revision: None,
            mark_change: None,
            format_revision: false,
            section: None,
            mark_run: if tracked { None } else { info.mark_run.clone() },
            ..ParagraphInfo::clone(info)
        };
        let props = match source {
            true => self.source_properties(like, "pPr"),
            false => self.properties(like, "pPr"),
        };
        let mut stripped = crate::tables::REVISIONS.to_vec();
        stripped.extend(["cnfStyle", "sectPr"]);
        (
            info,
            crate::format::without(&props, &self.scope(like), &stripped),
        )
    }

    /// A node from the source, whose properties a made node can copy.
    fn source_node(&self, id: NodeId) -> Result<&Node, EditError> {
        let node = self
            .nodes
            .get(id.index())
            .ok_or(EditError::UnknownNode { node: id })?;
        if node.made.is_some() {
            return refuse(id, "it was made by a command, not opened from the file");
        }
        Ok(node)
    }

    /// A new, empty paragraph for the story of `near`, in no container yet,
    /// with the properties of `like` or with none.
    pub(crate) fn make_paragraph(
        &mut self,
        near: NodeId,
        like: Option<NodeId>,
    ) -> Result<NodeId, EditError> {
        let name = self.w_name(near, "p");
        let (info, head) = match like {
            None => (ParagraphInfo::default(), format!("<{name}>")),
            Some(like) => {
                let kind = &self.source_node(like)?.kind;
                if !matches!(kind, NodeKind::Paragraph(_)) {
                    return Err(EditError::WrongKind {
                        node: like,
                        kind: kind.name(),
                        expected: "a paragraph",
                    });
                }
                let (mut info, props) = self.paragraph_like(like, true);
                (info.style, info.props) = self.paragraph_properties_from(like, &props)?;
                (info, format!("<{name}>{props}"))
            }
        };
        let tail = format!("</{name}>");
        let story = self.n(near).story;
        let paragraph = self.create(
            NodeKind::Paragraph(Box::new(info)),
            Support::Editable,
            Made { head, tail, like },
            story,
        );
        // The copied properties can use a prefix `like` declares on itself.
        if let Some(like) = like {
            self.declare_namespaces(paragraph, like);
        }
        Ok(paragraph)
    }

    /// A new text node, line break or picture, in no container yet, named as
    /// `near` names its element.
    pub(crate) fn make_leaf(&mut self, near: NodeId, leaf: Leaf) -> NodeId {
        let story = self.n(near).story;
        let (kind, support, made) = match leaf {
            Leaf::Text(text) => {
                let name = self.w_name(near, "t");
                let made = Made {
                    head: format!("<{name} xml:space=\"preserve\">"),
                    tail: format!("</{name}>"),
                    like: None,
                };
                (NodeKind::Text { text }, Support::Editable, made)
            }
            Leaf::Break => {
                let made = Made {
                    head: format!("<{}/>", self.w_name(near, "br")),
                    tail: String::new(),
                    like: None,
                };
                (
                    NodeKind::Break { break_type: None },
                    Support::ReadOnly(NO_EDIT),
                    made,
                )
            }
            Leaf::Picture(info) => {
                let made = Made {
                    head: crate::pictures::picture_xml(&self.w_name(near, "drawing"), &info),
                    tail: String::new(),
                    like: None,
                };
                (NodeKind::Drawing(info), Support::ReadOnly(NO_EDIT), made)
            }
        };
        self.create(kind, support, made, story)
    }

    pub(crate) fn paragraph_at(&self, loc: Loc) -> Result<NodeId, EditError> {
        let (Loc::Text(node, _) | Loc::Child(node, _)) = loc;
        match self.paragraph_of(node) {
            Some(paragraph) => Ok(paragraph),
            None => refuse(node, "it is not inside a paragraph"),
        }
    }

    /// A location as child indices from its story down, for ordering.
    pub(crate) fn path(&self, loc: Loc) -> Vec<usize> {
        let (Loc::Text(mut node, last) | Loc::Child(mut node, last)) = loc;
        let mut path = vec![last];
        while let Some(parent) = self.n(node).parent {
            path.push(self.index_of(node));
            node = parent;
        }
        path.reverse();
        path
    }

    /// A position at either end of a run, hyperlink or insertion becomes the
    /// position beside it, up to the paragraph, so it outlives the run's
    /// removal.
    pub(crate) fn lift_boundary(&self, loc: Loc, paragraph: NodeId) -> Loc {
        let Loc::Child(mut container, mut index) = loc else {
            return loc;
        };
        while container != paragraph && (index == 0 || index == self.n(container).children.len()) {
            index = self.index_of(container) + usize::from(index != 0);
            container = self.n(container).parent.expect("below the paragraph");
        }
        Loc::Child(container, index)
    }

    /// A paragraph's inline content in document order, as the shared state
    /// holds it: runs, hyperlinks and insertions a command made are entered,
    /// and anything else without editable text is one item.
    pub(crate) fn inline_walk(&self, node: NodeId, out: &mut Vec<Walk>) {
        let children = &self.n(node).children;
        for (i, &child) in children.iter().enumerate() {
            out.push(Walk::At(node, i));
            if self.editable_text(child).is_ok() {
                out.push(Walk::Text(child));
            } else if self.entered(child) {
                self.inline_walk(child, out);
            } else {
                out.push(Walk::Item(child));
            }
        }
        out.push(Walk::At(node, children.len()));
    }

    /// Checks a range for `replace_range` without changing anything, and
    /// returns what it covers.
    pub(crate) fn check_range(
        &self,
        a: Loc,
        b: Loc,
        pa: NodeId,
        pb: NodeId,
    ) -> Result<Covered, EditError> {
        self.check_end(a, pa)?;
        self.check_end(b, pb)?;
        self.check_editable(pa)?;
        self.check_editable(pb)?;
        let (ca, ia) = self.end_position(a, true);
        self.check_outside_fields(ca, ia)?;
        let mut covered = Covered::default();
        let mut walk = Vec::new();
        self.inline_walk(pa, &mut walk);
        let from = walk_index(&walk, a);
        if pa == pb {
            let to = walk_index(&walk, b);
            self.cover(&walk[from..=to], a, b, &mut covered);
        } else {
            self.check_mark(pa)?;
            self.cover(&walk[from..], a, b, &mut covered);
            let flow = self.n(pa).parent.expect("attached");
            let blocks = &self.n(flow).children[self.index_of(pa) + 1..=self.index_of(pb)];
            for &block in blocks {
                covered.blocks.push(block);
                if matches!(self.n(block).kind, NodeKind::Paragraph(_)) {
                    self.check_mark(block)?;
                    walk.clear();
                    self.inline_walk(block, &mut walk);
                    let to = if block == pb {
                        walk_index(&walk, b)
                    } else {
                        walk.len() - 1
                    };
                    self.cover(&walk[..=to], a, b, &mut covered);
                }
            }
        }
        let mut roots = covered.items.clone();
        roots.extend(
            covered
                .blocks
                .iter()
                .copied()
                .filter(|&b| !matches!(self.n(b).kind, NodeKind::Paragraph(_))),
        );
        self.check_range_items(&roots)?;
        Ok(covered)
    }

    pub(crate) fn check_range_items(&self, roots: &[NodeId]) -> Result<(), EditError> {
        for &root in roots {
            self.removable_field(root)?;
        }
        if !roots.is_empty() {
            self.check_contained(roots)?;
        }
        Ok(())
    }

    /// Refuses a range end that is not in editable text or between runs and
    /// links, where a paragraph's walk cannot find it.
    pub(crate) fn check_end(&self, loc: Loc, paragraph: NodeId) -> Result<(), EditError> {
        let mut up = match loc {
            Loc::Text(text, _) => {
                self.editable_text(text)?;
                self.n(text).parent.expect("attached")
            }
            Loc::Child(container, _) => container,
        };
        while up != paragraph {
            if !self.entered(up) {
                return refuse(
                    up,
                    "the range starts or ends inside a tracked change, field or content control; select all of it or none of it",
                );
            }
            up = self.n(up).parent.expect("below the paragraph");
        }
        Ok(())
    }

    /// Adds what a stretch of a paragraph's walk holds to `covered`: the
    /// characters of each text node between `a` and `b`, and each item.
    pub(crate) fn cover(&self, walk: &[Walk], a: Loc, b: Loc, covered: &mut Covered) {
        for step in walk {
            match *step {
                Walk::At(..) => {}
                Walk::Item(item) => covered.items.push(item),
                Walk::Text(text) => {
                    let start = match a {
                        Loc::Text(t, i) if t == text => i,
                        _ => 0,
                    };
                    let end = match b {
                        Loc::Text(t, j) if t == text => j,
                        _ => self.text_len(text).expect("text"),
                    };
                    if start < end {
                        covered.texts.push((text, TextRange { start, end }));
                    }
                }
            }
        }
    }

    /// Where a range end sits once the text on the range's side of it is
    /// deleted: a point inside a text node becomes the position after it at
    /// the range's start, and before it at the range's end.
    pub(crate) fn end_position(&self, loc: Loc, start: bool) -> (NodeId, usize) {
        match loc {
            Loc::Text(text, _) => (
                self.n(text).parent.expect("attached"),
                self.index_of(text) + usize::from(start),
            ),
            Loc::Child(container, index) => (container, index),
        }
    }

    /// Refuses what `remove_node` refuses, apart from what the rest of a
    /// range settles: the other end of a bookmark, permission range or field.
    fn removable_field(&self, id: NodeId) -> Result<Option<Vec<NodeId>>, EditError> {
        let parent = self.parent_of(id)?;
        self.check_editable(parent)?;
        if self.field_part(id, parent) {
            return self.field_span(id).map(Some);
        }
        match self.removal_refusal(id) {
            Some(reason) => refuse(id, reason),
            None => Ok(None),
        }
    }

    fn field_part(&self, id: NodeId, parent: NodeId) -> bool {
        self.field_char(id).is_some()
            || matches!(self.n(id).kind, NodeKind::FieldCode { .. })
            || self.field_depth(parent, self.index_of(id)) > 0
    }

    fn removal_refusal(&self, id: NodeId) -> Option<&'static str> {
        match &self.n(id).kind {
            NodeKind::Tab
            | NodeKind::Break { .. }
            | NodeKind::Drawing(_)
            | NodeKind::Field { .. }
            | NodeKind::Opaque { .. } => None,
            NodeKind::Revision(r) if r.change == "ins" || r.change == "del" => None,
            NodeKind::Revision(_) => {
                Some("a tracked move has two halves; accept or reject the move first")
            }
            NodeKind::ContentControl {
                lock: Some(lock), ..
            } if lock == "sdtLocked" || lock == "sdtContentLocked" => {
                Some("the content control is locked against deletion")
            }
            NodeKind::ContentControl { .. } | NodeKind::Table(_) | NodeKind::Row(_) => None,
            NodeKind::Text { .. }
                if self.n(id).support == Support::ReadOnly(crate::import::MIXED_TEXT) =>
            {
                None
            }
            NodeKind::Text { .. } => Some("change its text with replace_text instead"),
            NodeKind::Paragraph(_) => Some("join it with merge_paragraphs instead"),
            NodeKind::Marker { element, .. } => match local(element) {
                "bookmarkStart"
                | "bookmarkEnd"
                | "permStart"
                | "permEnd"
                | "proofErr"
                | "lastRenderedPageBreak" => None,
                _ => Some("no command removes a comment range or a move yet"),
            },
            NodeKind::NoteReference { .. } => Some(holds_reason("a footnote or endnote reference")),
            _ => Some("no command removes it yet"),
        }
    }

    /// The other end of a bookmark or permission range, in the same story.
    fn partner(&self, id: NodeId) -> Option<NodeId> {
        let NodeKind::Marker {
            element,
            marker_id: Some(marker_id),
            ..
        } = &self.n(id).kind
        else {
            return None;
        };
        let other = match local(element) {
            "bookmarkStart" => "bookmarkEnd",
            "bookmarkEnd" => "bookmarkStart",
            "permStart" => "permEnd",
            "permEnd" => "permStart",
            _ => return None,
        };
        let mut root = id;
        while let Some(parent) = self.n(root).parent {
            root = parent;
        }
        self.descendants(root).into_iter().find(|&d| {
            matches!(&self.n(d).kind, NodeKind::Marker { element, marker_id: Some(m), .. }
                if local(element) == other && m == marker_id)
        })
    }

    pub(crate) fn removal_nodes(&self, id: NodeId) -> Result<Vec<NodeId>, EditError> {
        if let Some(field) = self.removable_field(id)? {
            return Ok(field);
        }
        if matches!(self.n(id).kind, NodeKind::Row(_)) {
            self.check_row_removal(id)?;
        } else if self.n(id).preserved_cells.is_some() {
            self.check_merge_start_removal(self.parent_of(id)?, Some(id))?;
        }
        let ends: Vec<NodeId> = std::iter::once(id).chain(self.partner(id)).collect();
        self.check_contained(&ends)?;
        self.check_blocks_left(id)?;
        Ok(ends)
    }

    pub(crate) fn remove_node(&mut self, id: NodeId) -> Result<(), EditError> {
        for end in self.removal_nodes(id)? {
            let (parent, index) = self.detach(end);
            if self.n(end).preserved_cells.is_some() {
                for (i, blank) in self.wrapper_vacancies(parent, end)?.into_iter().enumerate() {
                    self.attach(blank, parent, index + i);
                }
            }
            if matches!(self.n(parent).kind, NodeKind::Run(_)) && self.n(parent).children.is_empty()
            {
                self.detach(parent);
            }
        }
        Ok(())
    }

    /// The runs a complex field takes, from its begin character to its end
    /// character, if a command can remove them.
    fn field_span(&self, id: NodeId) -> Result<Vec<NodeId>, EditError> {
        const SPANS: &str = "the field spans paragraphs; no command removes it yet";
        let Some(paragraph) = self.paragraph_of(id) else {
            return refuse(id, SPANS);
        };
        self.check_editable(paragraph)?;
        let parent = self.n(paragraph).parent.expect("attached");
        if self.field_depth(parent, self.index_of(paragraph)) != 0 {
            return refuse(id, SPANS);
        }
        let order = self.descendants(paragraph);
        let target = order
            .iter()
            .position(|&d| d == id)
            .expect("the node is inside its paragraph");
        let (mut depth, mut begin, mut field) = (0, 0, None);
        for (i, &d) in order.iter().enumerate() {
            match self.field_char(d) {
                Some("begin") => {
                    if depth == 0 {
                        begin = i;
                    }
                    depth += 1;
                }
                Some("end") => {
                    depth -= 1;
                    if depth == 0 && begin <= target && target <= i {
                        field = Some((order[begin], d));
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some((begin, end)) = field else {
            return refuse(id, SPANS);
        };
        let (first, last) = (
            self.n(begin).parent.expect("attached"),
            self.n(end).parent.expect("attached"),
        );
        let container = self.n(first).parent.expect("attached");
        let whole_runs = matches!(self.n(first).kind, NodeKind::Run(_))
            && matches!(self.n(last).kind, NodeKind::Run(_))
            && self.n(last).parent == Some(container)
            && self.n(first).children.first() == Some(&begin)
            && self.n(last).children.last() == Some(&end);
        if !whole_runs {
            return refuse(
                id,
                "the field shares a run with other content; no command removes part of a run yet",
            );
        }
        self.check_editable(container)?;
        let (from, to) = (self.index_of(first), self.index_of(last));
        let span = self.n(container).children[from..=to].to_vec();
        self.check_contained(&span)?;
        Ok(span)
    }

    /// Refuses removing content that holds one end of something whose other
    /// end would stay (a bookmark, a permission range, a field), or that holds
    /// a comment, a note reference or a move, which need commands of their own.
    pub(crate) fn check_contained(&self, roots: &[NodeId]) -> Result<(), EditError> {
        let mut open: BTreeMap<(&str, &str), i64> = BTreeMap::new();
        let mut fields = 0;
        for &root in roots {
            for id in std::iter::once(root).chain(self.descendants(root)) {
                match &self.n(id).kind {
                    NodeKind::Revision(r) if r.change != "ins" && r.change != "del" => {
                        return refuse(
                            id,
                            "it holds one end of a tracked move; accept or reject the move first",
                        );
                    }
                    NodeKind::ContentControl {
                        lock: Some(lock), ..
                    } if lock == "sdtLocked" || lock == "sdtContentLocked" => {
                        return refuse(
                            id,
                            "it holds a content control that is locked against deletion",
                        );
                    }
                    NodeKind::Opaque {
                        holds: Some(what), ..
                    } => return refuse(id, holds_reason(what)),
                    NodeKind::Drawing(info) if info.holds.is_some() => {
                        return refuse(id, holds_reason(info.holds.unwrap_or_default()))
                    }
                    NodeKind::NoteReference { .. } => {
                        return refuse(id, holds_reason("a footnote or endnote reference"))
                    }
                    NodeKind::Marker {
                        element,
                        marker_id,
                        name,
                    } => match local(element) {
                        start @ ("bookmarkStart" | "bookmarkEnd" | "permStart" | "permEnd") => {
                            let family = if start.starts_with("bookmark") {
                                "bookmark"
                            } else {
                                "perm"
                            };
                            let delta = if start.ends_with("Start") { 1 } else { -1 };
                            *open
                                .entry((family, marker_id.as_deref().unwrap_or_default()))
                                .or_default() += delta;
                        }
                        "commentRangeStart" | "commentRangeEnd" | "commentReference" => {
                            return refuse(id, holds_reason("a comment"))
                        }
                        "fldChar" => match name.as_deref() {
                            Some("begin") => fields += 1,
                            Some("end") if fields == 0 => {
                                return refuse(id, holds_reason("part of a field"))
                            }
                            Some("end") => fields -= 1,
                            _ => {}
                        },
                        m if m.starts_with("move") => return refuse(id, holds_reason("a move")),
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
        if fields != 0 {
            return refuse(roots[0], holds_reason("part of a field"));
        }
        if let Some(((family, _), _)) = open.iter().find(|(_, &count)| count != 0) {
            let what = if *family == "bookmark" {
                "a bookmark"
            } else {
                "a permission range"
            };
            return refuse(roots[0], holds_reason(what));
        }
        Ok(())
    }

    /// A story keeps at least one paragraph, and a table cell ends with one.
    fn check_blocks_left(&self, id: NodeId) -> Result<(), EditError> {
        let parent = self.n(id).parent.expect("attached");
        let rest: Vec<NodeId> = self
            .n(parent)
            .children
            .iter()
            .copied()
            .filter(|&c| c != id)
            .collect();
        let paragraph = |c: NodeId| matches!(self.n(c).kind, NodeKind::Paragraph(_));
        match self.n(parent).kind {
            NodeKind::Story { .. } if !rest.iter().any(|&c| paragraph(c)) => {
                refuse(id, "a story keeps at least one paragraph")
            }
            NodeKind::Cell(_) => {
                let last = rest
                    .iter()
                    .rev()
                    .find(|&&c| !matches!(self.n(c).kind, NodeKind::Marker { .. }));
                match last {
                    Some(&c) if paragraph(c) => Ok(()),
                    _ => refuse(id, "a table cell must end with a paragraph"),
                }
            }
            _ => Ok(()),
        }
    }
}
