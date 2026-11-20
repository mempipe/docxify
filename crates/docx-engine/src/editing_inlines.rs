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
    pub(super) formats: Vec<(NodeId, Format)>,
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
            used_drawing_ids: base.drawing_ids.clone(),
            formats: Vec::new(),
        };
        writer.used = located
            .iter()
            .map(|p| &p.paragraph)
            .filter(|p| writer.unchanged(p))
            .flat_map(|p| p.text.iter().filter_map(Inline::basis))
            .collect();
        writer
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

    /// Source content can use a prefix that a source element around it,
    /// up to its paragraph, declares. Placed in another paragraph, or out of
    /// a wrapper that is built anew, it declares the prefix itself.
    fn carry_namespaces(
        &self,
        document: &mut Document,
        origin: NodeId,
        placed: NodeId,
        paragraph: NodeId,
    ) {
        let is_paragraph = |id: NodeId| matches!(self.base.n(id).kind, NodeKind::Paragraph(_));
        let mut next = match is_paragraph(origin) {
            true => Some(origin),
            false => self.base.n(origin).parent,
        };
        while let Some(id) = next.filter(|id| *id != paragraph) {
            document.declare_namespaces(placed, id);
            next = self.base.n(id).parent.filter(|_| !is_paragraph(id));
        }
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
        parent: NodeId,
        node: NodeId,
        located: &Located,
    ) -> Result<(), SessionError> {
        let (document, places, comments) =
            (&mut *target.document, &mut *target.places, target.comments);
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
            if let Inline::Source { basis, .. } = &stretch[0] {
                if self.seeded.contains_key(basis) {
                    for item in stretch {
                        let Inline::Source { node: source, .. } = item else {
                            return Err(unbuilt("mixed atomic container"));
                        };
                        if self.shown_sources.insert(*source) {
                            self.carry_namespaces(document, *source, *source, node);
                            places.items.insert(*source, (unit, 1));
                            let original = self.base.n(*source);
                            if matches!(original.kind, NodeKind::Run(_))
                                && original.children.is_empty()
                                && original.support == Support::Editable
                            {
                                places.runs.insert(*source, (unit, 0));
                            }
                            children.push(BuiltRun {
                                node: *source,
                                insertion: None,
                                deletion: None,
                                link: self
                                    .base
                                    .n(*source)
                                    .parent
                                    .filter(|p| {
                                        matches!(self.base.n(*p).kind, NodeKind::Hyperlink(_))
                                    })
                                    .map(Link::Source),
                                first: unit,
                                len: 1,
                            });
                        }
                        unit += 1;
                    }
                    continue;
                }
            }
            let (basis, format) = (stretch[0].basis(), stated_format(base, &stretch[0]));
            let link = stated_link(base, &stretch[0]);
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
                self.carry_namespaces(document, run.id, run.id, node);
                children.push(BuiltRun {
                    node: run.id,
                    insertion: stretch[0].revision().cloned(),
                    deletion: None,
                    link: link_of(&stretch[0]),
                    first: unit,
                    len: stretch.len(),
                });
            } else {
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
                    (Some(run), _) => document.empty_like(run.id),
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
                    (None, None) => document.empty_run(parent, None),
                    _ => return Err(unbuilt(format!("{basis:?} is not a run of the file"))),
                };
                let run = copy.map_err(unbuilt)?;
                if let Some(basis) = basis {
                    self.carry_namespaces(document, basis, run, node);
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
                            let node = if stretch[0].deletion().is_some() {
                                document.loose_deleted_text(run, &text)
                            } else if let Some(source) = first_char.source_text {
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
                            if stretch[0].deletion().is_none() {
                                places.texts.insert(node, (at, pieces.len()));
                            }
                            run_children.push(node);
                        }
                        Inline::Object { object, .. } => {
                            if self.shown_objects.insert(object.identity.clone()) {
                                let node = match &object.kind {
                                    NewInline::Break => document.loose_break(run),
                                    NewInline::Picture {
                                        media,
                                        width_emu,
                                        height_emu,
                                        description,
                                        id,
                                    } => document.loose_picture(
                                        run,
                                        media,
                                        *width_emu,
                                        *height_emu,
                                        description.as_deref(),
                                        id.filter(|id| self.used_drawing_ids.insert(*id)),
                                    ),
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
                        document.link_style().map(Some)
                    };
                    if let Some(style) = style {
                        let properties = document.run_styled(run, style.as_deref());
                        document
                            .compose_run_properties(run, &properties)
                            .map_err(unbuilt)?;
                    }
                }
                if stretch[0].deletion().is_none() {
                    places.runs.insert(run, (unit, stretch.len()));
                }
                if format != plain {
                    self.formats.push((run, format));
                }
                children.push(BuiltRun {
                    node: run,
                    insertion: stretch[0].revision().cloned(),
                    deletion: stretch[0].deletion().cloned(),
                    link: link_of(&stretch[0]),
                    first: unit,
                    len: stretch.len(),
                });
            }
            unit += stretch.len();
        }
        if !unchanged {
            attach_runs(
                document,
                places,
                node,
                &children,
                comments,
                first + paragraph.text.len(),
            )?;
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
    document: &mut Document,
    places: &mut Places,
    node: NodeId,
    children: &[BuiltRun],
    comments: &annotations::Comments,
    end: usize,
) -> Result<(), SessionError> {
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
                Link::Source(source) => document.empty_like(*source),
                Link::Target(target) => document.make_link(node, target),
            }
            .map_err(unbuilt)?;
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
