//! Materializes a paragraph's inline content from its final shared facts.
//! Sources may move between paragraphs. Reserve untouched runs first and
//! track each source object as it is placed in the projected document.
use super::*;
use std::collections::BTreeSet;

/// Where a paragraph is written.
pub(super) struct Target<'a> {
    pub document: &'a mut Document,
    pub places: &'a mut Places,
    pub comments: &'a annotations::Comments,
}

enum RunChange {
    Style(Option<String>),
    Format(Format),
}

pub(super) struct InlineWriter<'a> {
    base: &'a Document,
    seeded: &'a HashMap<NodeId, Vec<Run>>,
    // Where each strand starts, and the source paragraph it was seeded from.
    owners: Vec<(usize, Option<NodeId>)>,
    runs: HashMap<NodeId, &'a Run>,
    repeated: HashSet<NodeId>,
    used: HashSet<NodeId>,
    shown_sources: HashSet<NodeId>,
    written_leaves: HashSet<NodeId>,
    shown_objects: HashSet<String>,
    used_drawing_ids: BTreeSet<u32>,
    reused_pictures: HashMap<String, Box<crate::DrawingInfo>>,
    edges: Edges,
}

impl<'a> InlineWriter<'a> {
    pub(super) fn new(
        base: &'a Document,
        seeded: &'a HashMap<NodeId, Vec<Run>>,
        located: &[Located],
        owners: Vec<(usize, Option<NodeId>)>,
    ) -> Self {
        // Concurrent undo can restore two sequence entries for one imported
        // object. It keeps its identity: its first occurrence is displayed.
        // Both facts stay stored, including their raw positions.
        let mut seen = HashSet::new();
        let mut repeated = HashSet::new();
        for inline in located.iter().flat_map(|p| &p.paragraph.text) {
            if let Inline::Source { node, .. } = inline {
                if !seen.insert(*node) {
                    repeated.insert(*node);
                }
            }
        }

        let mut writer = Self {
            base,
            seeded,
            owners,
            repeated,
            runs: seeded.values().flatten().map(|run| (run.id, run)).collect(),
            used: HashSet::new(),
            shown_sources: HashSet::new(),
            written_leaves: HashSet::new(),
            shown_objects: HashSet::new(),
            used_drawing_ids: BTreeSet::new(),
            reused_pictures: HashMap::new(),
            edges: Edges::default(),
        };
        writer.used = located
            .iter()
            .map(|p| &p.paragraph)
            .filter(|p| writer.unchanged(p))
            .flat_map(|p| p.text.iter().filter_map(Inline::basis))
            .collect();
        writer
    }

    /// Text can move around a picture without changing its document-wide drawing ID.
    /// A changed picture sequence goes through the full writer's allocation.
    pub(super) fn reuse_pictures(
        &mut self,
        doc: &Document,
        old_nodes: &[NodeId],
        before: &[Located],
        after: &[Located],
    ) -> bool {
        fn pictures(blocks: &[Located]) -> impl Iterator<Item = &InlineObject> {
            blocks
                .iter()
                .flat_map(|p| &p.paragraph.text)
                .filter_map(|inline| match inline {
                    Inline::Object { object, .. }
                        if matches!(object.kind, NewInline::Picture { .. }) =>
                    {
                        Some(object)
                    }
                    _ => None,
                })
        }
        let mut old = pictures(before);
        let mut drawings = old_nodes
            .iter()
            .flat_map(|&p| doc.descendants(p))
            .filter_map(|id| {
                let node = doc.n(id);
                match &node.kind {
                    NodeKind::Drawing(info) if node.made.is_some() => Some(info),
                    _ => None,
                }
            });
        for picture in pictures(after) {
            if old.next() != Some(picture) {
                return false;
            }
            let Some(info) = drawings.next() else {
                return false;
            };
            self.reused_pictures
                .insert(picture.identity.clone(), info.clone());
        }
        old.next().is_none() && drawings.next().is_none()
    }

    fn has_repeat(&self, text: &[Inline]) -> bool {
        text.iter()
            .any(|item| matches!(item, Inline::Source { node, .. } if self.repeated.contains(node)))
    }

    fn unchanged(&self, p: &Paragraph) -> bool {
        matches!(p.mark, Mark::Base(id) if self.seeded.get(&id).is_some_and(|runs|
            runs.iter().flat_map(|r| &r.text).eq(&p.text)))
            && !self.has_repeat(&p.text)
    }

    /// Whether the text at `unit` is in the strand of the source paragraph
    /// that holds `run`.
    fn owns(&self, unit: usize, run: NodeId) -> bool {
        let strand = self.owners.partition_point(|(start, _)| *start <= unit);
        let owner = strand.checked_sub(1).and_then(|i| self.owners[i].1);
        owner.is_some() && owner == self.base.paragraph_of(run)
    }

    pub(super) fn write(
        &mut self,
        target: &mut Target,
        node: NodeId,
        located: &Located,
    ) -> Result<(), SessionError> {
        let (document, places, comments) =
            (&mut *target.document, &mut *target.places, target.comments);
        let carry = |doc: &mut Document, source: NodeId, placed: NodeId| {
            let inherited = if matches!(self.base.n(source).kind, NodeKind::Paragraph(_)) {
                source // A run generated from this paragraph's mark.
            } else {
                self.base.n(source).parent.expect("an inline has a parent")
            };
            if inherited != node {
                doc.declare_namespaces(placed, &self.base.scope(inherited), &doc.scope(node));
            }
        };
        let paragraph = &located.paragraph;
        let mut unit = located.first;
        let plain = Format::default();
        let unchanged = self.unchanged(paragraph)
            && comments
                .edges
                .range(unit..=unit + paragraph.text.len())
                .next()
                .is_none();
        // One run for each stretch of text that reads alike. A run of the
        // file is kept where it holds what it held, and copied otherwise.
        let mut children: Vec<BuiltRun> = Vec::new();
        let mut changes = Vec::new();
        let base = self.base;
        let link_of = |inline: &Inline| match stated_link(base, inline) {
            Some(UNLINKED) => None,
            Some(target) => Some(Link::Target(target.to_string())),
            None => inline
                .basis()
                .and_then(|id| self.base.node(id)?.parent)
                .filter(|id| {
                    matches!(
                        self.base.node(*id).map(|n| &n.kind),
                        Some(NodeKind::Hyperlink(_))
                    )
                })
                .map(Link::Source),
        };
        let alike = |a: &Inline, b: &Inline| {
            matches!(a, Inline::HiddenBoundary) == matches!(b, Inline::HiddenBoundary)
                && a.basis() == b.basis()
                && a.paragraph_child(base).is_none()
                && b.paragraph_child(base).is_none()
                && stated_format(base, a) == stated_format(base, b)
                && a.revision() == b.revision()
                && a.deletion() == b.deletion()
                && stated_link(base, a) == stated_link(base, b)
        };
        let first = unit;
        let indices: Vec<_> = (0..paragraph.text.len()).collect();
        for group in indices.chunk_by(|a, b| {
            alike(&paragraph.text[*a], &paragraph.text[*b])
                && !comments.edges.contains_key(&(first + *b))
        }) {
            let stretch = &paragraph.text[group[0]..=group[group.len() - 1]];
            if matches!(stretch[0], Inline::HiddenBoundary) {
                unit += stretch.len();
                continue;
            }
            if let Some(source) = stretch[0].paragraph_child(base) {
                if self.shown_sources.insert(source) {
                    let wrapper = self
                        .base
                        .n(source)
                        .parent
                        .filter(|p| matches!(self.base.n(*p).kind, NodeKind::Hyperlink(_)))
                        .map(Link::Source);
                    if wrapper.is_none() {
                        carry(document, source, source);
                    }
                    places.items.insert(source, (unit, 1));
                    let original = self.base.n(source);
                    if matches!(original.kind, NodeKind::Run(_))
                        && original.children.is_empty()
                        && original.support == Support::Editable
                    {
                        places.runs.insert(source, (unit, 0));
                    }
                    children.push(BuiltRun {
                        node: source,
                        insertion: None,
                        deletion: None,
                        link: wrapper,
                        first: unit,
                        len: 1,
                    });
                }
                unit += 1;
                continue;
            }
            let (basis, format) = (stretch[0].basis(), stated_format(base, &stretch[0]));
            let link = stated_link(base, &stretch[0]);
            let wrapper = link_of(&stretch[0]);
            let run = basis.and_then(|basis| self.runs.get(&basis));
            let own = run.is_some_and(|run| self.owns(unit, run.id));
            let kept = run.filter(|run| {
                own && format == plain && run.text == stretch && !self.has_repeat(stretch)
            });
            if let Some(run) = kept.filter(|run| unchanged || self.used.insert(run.id)) {
                let mut at = unit;
                for (node, len) in &run.nodes {
                    if matches!(
                        self.base.node(*node).map(|n| &n.kind),
                        Some(NodeKind::Text { .. })
                    ) {
                        let text_len = if matches!(&self.base.n(*node).kind, NodeKind::Text { text } if text.is_empty())
                        {
                            0
                        } else {
                            *len
                        };
                        places.texts.insert(*node, (at, text_len));
                    } else {
                        places.items.insert(*node, (at, *len));
                    }
                    at += len;
                }
                places.runs.insert(run.id, (unit, stretch.len()));
                self.edges
                    .placed(document, run.id, run.id)
                    .map_err(unbuilt)?;
                if !matches!(wrapper, Some(Link::Source(_))) {
                    carry(document, run.id, run.id);
                }
                children.push(BuiltRun {
                    node: run.id,
                    insertion: stretch[0].revision().cloned(),
                    deletion: None,
                    link: wrapper,
                    first: unit,
                    len: stretch.len(),
                });
            } else {
                let source = run.map(|run| run.id);
                let copy = match (run, basis) {
                    // The first stretch that reads as its source run stays in
                    // that run, so the markup around its children is written
                    // once. A later copy takes the run's properties, so a
                    // stretch that changes them is a copy too. An empty source
                    // run is already placed whole.
                    (Some(run), _)
                        if own
                            && format == plain
                            && link.is_none()
                            && run.nodes.iter().all(|(node, _)| *node != run.id)
                            && self.used.insert(run.id) =>
                    {
                        Ok(run.id)
                    }
                    (Some(run), _) => self.edges.copy(document, run.id),
                    // Text typed where there was none is like its paragraph's
                    // mark, or bare where the mark has no properties.
                    (None, Some(basis))
                        if matches!(
                            self.base.node(basis).map(|n| &n.kind),
                            Some(NodeKind::Paragraph(_))
                        ) =>
                    {
                        (document.empty_run(basis, Some(basis)))
                            .or_else(|_| document.empty_run(basis, None))
                    }
                    (None, None) => document.empty_run(node, None),
                    _ => return Err(unbuilt(format!("{basis:?} is not a run of the file"))),
                };
                let run = copy.map_err(unbuilt)?;
                if let Some(basis) = basis.filter(|_| !matches!(wrapper, Some(Link::Source(_)))) {
                    carry(document, basis, run);
                }
                let mut run_children = Vec::new();
                let mut at = unit;
                for pieces in
                    stretch.chunk_by(|a, b| matches!((a, b), (Inline::Char(x), Inline::Char(y)) if x.source_text == y.source_text))
                {
                    match &pieces[0] {
                        Inline::HiddenBoundary => {
                            unreachable!("hidden boundaries are skipped above")
                        }
                        Inline::Char(first_char) => {
                            let text: String = pieces
                                .iter()
                                .filter_map(|piece| match piece {
                                    Inline::Char(c) => Some(c.c),
                                    _ => None,
                                })
                                .collect();
                            let node = if let Some(source) = first_char.source_text {
                                if first_char.basis == Some(run) && self.written_leaves.insert(source) {
                                    if matches!(&document.n(source).kind, NodeKind::Text { text: old } if *old == text) {
                                        Ok(source)
                                    } else {
                                        document.compose_text(source, &text).map(|_| source)
                                    }
                                } else {
                                    Ok(copy_text_leaf(self.base, document, source, first_char.basis, &text, &mut self.written_leaves)?)
                                }
                            } else {
                                document.loose_text(run, &text)
                            }
                            .map_err(unbuilt)?;
                            if first_char.deletion.is_some() {
                                document.mark_text_deleted(node, text);
                            } else {
                                places.texts.insert(node, (at, pieces.len()));
                            }
                            run_children.push(node);
                        }
                        Inline::Object { object, .. } => {
                            if self.shown_objects.insert(object.identity.clone()) {
                                let node = match &object.kind {
                                    NewInline::Break => document.loose_break(run),
                                    NewInline::Picture { media, width_emu, height_emu, description, id } => {
                                        if let Some(info) = self.reused_pictures.remove(&object.identity) {
                                            Ok(document.make_leaf(run, crate::structure::Leaf::Picture(info)))
                                        } else {
                                            document.loose_picture(
                                                run,
                                                media,
                                                *width_emu,
                                                *height_emu,
                                                description.as_deref(),
                                                id.filter(|id| self.used_drawing_ids.insert(*id)),
                                            )
                                        }
                                    }
                                }
                                .map_err(unbuilt)?;
                                if let NodeKind::Drawing(info) = &document.n(node).kind {
                                    if let Some(id) = info.id {
                                        self.used_drawing_ids.insert(id);
                                    }
                                }
                                if stretch[0].deletion().is_none() {
                                    places.items.insert(node, (at, 1));
                                }
                                run_children.push(node);
                            }
                        }
                        Inline::Source { node, basis, .. } => {
                            let original = self.base
                                .node(*node)
                                .ok_or_else(|| unbuilt("inline source is absent"))?;
                            if original.parent != Some(*basis)
                                || !matches!(
                                    self.base.node(*basis).map(|n| &n.kind),
                                    Some(NodeKind::Run(_))
                                )
                            {
                                return Err(unbuilt("inline source is not in its stated run"));
                            }
                            if self.shown_sources.insert(*node) {
                                if stretch[0].deletion().is_none() {
                                    if matches!(&original.kind, NodeKind::Text { text } if text.is_empty()) {
                                        places.texts.insert(*node, (at, 0));
                                    } else {
                                        places.items.insert(*node, (at, 1));
                                    }
                                }
                                run_children.push(*node);
                            }
                        }
                    }
                    at += pieces.len();
                }
                if run_children.is_empty() {
                    unit += stretch.len();
                    continue;
                }
                document.arrange(run, &run_children).map_err(unbuilt)?;
                if let Some(source) = source {
                    self.edges.placed(document, source, run).map_err(unbuilt)?;
                }
                if let Some(target) = link {
                    let style = if target == UNLINKED {
                        let existing = document.existing_link_style();
                        let is_link_style = matches!(&document.node(run).unwrap().kind, NodeKind::Run(info) if info.style == existing);
                        if !is_link_style {
                            None
                        } else {
                            Some(None)
                        }
                    } else {
                        document.link_style(self.base).map(Some)
                    };
                    if let Some(style) = style {
                        changes.push((run, RunChange::Style(style)));
                    }
                }
                if stretch[0].deletion().is_none() {
                    places.runs.insert(run, (unit, stretch.len()));
                }
                if format != plain {
                    changes.push((run, RunChange::Format(format)));
                }
                children.push(BuiltRun {
                    node: run,
                    insertion: stretch[0].revision().cloned(),
                    deletion: stretch[0].deletion().cloned(),
                    link: wrapper,
                    first: unit,
                    len: stretch.len(),
                });
            }
            unit += stretch.len();
        }
        if !unchanged {
            let end = first + paragraph.text.len();
            attach_runs(target, node, &children, end, carry, &mut self.edges)?;
        }
        // Run styles inherit from their paragraph and any enclosing hyperlink.
        let document = &mut *target.document;
        for (run, change) in changes {
            match change {
                RunChange::Style(style) => {
                    let properties = document
                        .run_styled(run, style.as_deref())
                        .map_err(unbuilt)?;
                    document
                        .compose_run_properties(run, &properties)
                        .map_err(unbuilt)?;
                }
                RunChange::Format(format) => document.format_run(run, format).map_err(unbuilt)?,
            }
        }
        Ok(())
    }
}

/// The format an inline states beyond what its source run has. A command
/// states its whole selection, and a run that already reads so stays as the
/// file has it.
fn stated_format(base: &Document, inline: &Inline) -> Format {
    let mut format = inline.format();
    if format == Format::default() {
        return format;
    }
    let run = inline.basis().and_then(|id| base.node(id));
    let Some(NodeKind::Run(info)) = run.map(|n| &n.kind) else {
        return format;
    };
    if format.bold == info.props.bold {
        format.bold = None;
    }
    if format.italic == info.props.italic {
        format.italic = None;
    }
    let underlined = info.props.underline.as_deref().map(|u| u != "none");
    if format.underline == underlined {
        format.underline = None;
    }
    format
}

/// The link an inline states, unless its source run is in that link already,
/// or is stated out of a link it was never in.
fn stated_link<'i>(base: &Document, inline: &'i Inline) -> Option<&'i str> {
    let target = inline.link()?;
    let source = inline
        .basis()
        .and_then(|id| base.node(id)?.parent)
        .and_then(|id| match &base.node(id)?.kind {
            NodeKind::Hyperlink(info) => crate::links::href(info),
            _ => None,
        });
    let stays = match &source {
        Some(source) => source == target,
        None => target == UNLINKED,
    };
    (!stays).then_some(target)
}

/// A run or link of the file can hold markup at its edges, such as an XML
/// comment before its first child or after its last. Its content can end up
/// in several parts: the source node and copies of it. The first part keeps
/// the leading markup and the last part the trailing markup, so each is
/// written once, where it was. Each such source maps to the part that now
/// ends with its trailing markup, and to that markup.
#[derive(Default)]
struct Edges(HashMap<NodeId, (NodeId, String)>);

impl Edges {
    /// A copy of `source` for part of its content.
    fn copy(
        &self,
        document: &mut Document,
        source: NodeId,
    ) -> Result<NodeId, crate::error::EditError> {
        document.empty_like(source, !self.0.contains_key(&source))
    }

    /// `part`, the source or a copy of it, took its place after every
    /// earlier part.
    fn placed(
        &mut self,
        document: &mut Document,
        source: NodeId,
        part: NodeId,
    ) -> Result<(), crate::error::EditError> {
        let trail = match self.0.remove(&source) {
            Some((last, trail)) => {
                if part == source {
                    document.clear_lead(source)?;
                }
                document.clear_trail(last);
                trail
            }
            None if document.has_edge_markup(source) => document.trail(source),
            None => return Ok(()),
        };
        if part != source {
            document.set_trail(part, &trail);
        }
        self.0.insert(source, (part, trail));
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq)]
enum Link {
    Source(NodeId),
    Target(String),
}
struct BuiltRun {
    node: NodeId,
    insertion: Option<Track>,
    deletion: Option<Track>,
    link: Option<Link>,
    first: usize,
    len: usize,
}

fn attach_runs(
    target: &mut Target,
    node: NodeId,
    children: &[BuiltRun],
    end: usize,
    carry: impl Fn(&mut Document, NodeId, NodeId),
    edges: &mut Edges,
) -> Result<(), SessionError> {
    let (document, places, comments) =
        (&mut *target.document, &mut *target.places, target.comments);
    let mut wrappers = Vec::new();
    for group in children.chunk_by(|a, b| {
        a.insertion == b.insertion
            && a.deletion == b.deletion
            && a.link == b.link
            && !comments.edges.contains_key(&b.first)
    }) {
        wrappers.extend(comments.marks(document, places, node, group[0].first));
        let mut runs: Vec<_> = group.iter().map(|c| c.node).collect();
        if let Some(link) = &group[0].link {
            let link = match link {
                Link::Source(source) => {
                    let copy = edges.copy(document, *source).map_err(unbuilt)?;
                    edges.placed(document, *source, copy).map_err(unbuilt)?;
                    carry(document, *source, copy);
                    copy
                }
                Link::Target(target) => document.make_link(node, target).map_err(unbuilt)?,
            };
            document.arrange(link, &runs).map_err(unbuilt)?;
            places
                .runs
                .insert(link, (group[0].first, group.iter().map(|c| c.len).sum()));
            runs = vec![link];
        }
        if let Some(track) = &group[0].deletion {
            let wrapper = document
                .loose_revision(node, Change::Deletion, track)
                .map_err(unbuilt)?;
            document.arrange(wrapper, &runs).map_err(unbuilt)?;
            places
                .items
                .insert(wrapper, (group[0].first, group.iter().map(|c| c.len).sum()));
            runs = vec![wrapper];
        }
        if let Some(track) = &group[0].insertion {
            let wrapper = document
                .loose_revision(node, Change::Insertion, track)
                .map_err(unbuilt)?;
            document.arrange(wrapper, &runs).map_err(unbuilt)?;
            wrappers.push(wrapper);
        } else {
            wrappers.extend(runs);
        }
    }
    wrappers.extend(comments.marks(document, places, node, end));
    document.arrange(node, &wrappers).map_err(unbuilt)?;
    Ok(())
}

fn copy_text_leaf(
    base: &Document,
    document: &mut Document,
    source: NodeId,
    basis: Option<NodeId>,
    text: &str,
    written_leaves: &mut HashSet<NodeId>,
) -> Result<NodeId, SessionError> {
    let original = base
        .node(source)
        .filter(|n| n.parent == basis && matches!(n.kind, NodeKind::Text { .. }))
        .ok_or_else(|| unbuilt("text has an invalid source leaf"))?;
    Ok(document.copy_source_text(original, text, written_leaves.insert(source)))
}
