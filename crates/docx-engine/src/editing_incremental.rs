//! Rebuild the changed strand with the canonical paragraph writer.
use super::*;
use crate::Splice;

pub(super) fn update(
    base: &Document,
    store: &State,
    previous: &Arc<View>,
) -> Result<Option<Arc<View>>, SessionError> {
    if store.facts() != previous.state.facts() {
        return Ok(None);
    }
    let store = PositionState::new(store.clone(), previous.shape().clone());
    if store.strands().len() != previous.state.strands().len() {
        return Ok(None);
    }
    let mut changed = None;
    let mut at = 0;
    for (index, (old, new)) in previous
        .state
        .strands()
        .iter()
        .zip(store.strands())
        .enumerate()
    {
        if old.key != new.key || old.order != new.order || old.prefix != new.prefix {
            return Ok(None);
        }
        if store.visible(new)
            && !Arc::ptr_eq(&old.flow, &new.flow)
            && old.flow.slots() != new.flow.slots()
            && changed.replace((index, at)).is_some()
        {
            return Ok(None);
        }
        at += previous.state.strand_len(old);
    }
    let mut view = previous.as_ref().clone();
    view.state = store;
    view.changes = Some(Vec::new());
    if let Some((index, start)) = changed {
        if !restructure(base, &mut view, previous, index, start)? {
            return Ok(None);
        }
    }
    Ok(Some(Arc::new(view)))
}

/// A missing paragraph mark joins two strands. Read both sides of that
/// boundary together, including either side needed by undo.
fn starts_paragraph(strand: &Strand) -> Result<bool, SessionError> {
    let units = strand.flow.read_shared()?;
    let Some(unit) = units.first() else {
        return Ok(false);
    };
    Ok(mark_of(unit)?.is_some() && insertion_of(unit)?.is_none() && deletion_of(unit)?.is_none())
}

fn restructure(
    base: &Document,
    view: &mut View,
    previous: &View,
    index: usize,
    old_first: usize,
) -> Result<bool, SessionError> {
    let left = previous.state.strands();
    let right = view.state.strands();
    let boundary = |i: usize| -> Result<bool, SessionError> {
        Ok(view.state.prefix(&right[i]).is_some()
            || (starts_paragraph(&left[i])? && starts_paragraph(&right[i])?))
    };
    let (mut first, mut last, mut start) = (index, index + 1, old_first);
    while first > 0 && !boundary(first)? {
        first -= 1;
        start -= previous.state.strand_len(&left[first]);
    }
    while last < right.len() && !boundary(last)? {
        last += 1;
    }
    let old = &left[first..last];
    let new = &right[first..last];
    let is_paragraph = |id| matches!(base.node(id).map(|n| &n.kind), Some(NodeKind::Paragraph(_)));
    let mut owners = Vec::new();
    start += usize::from(view.state.prefix(&new[0]).is_some());
    let mut at = start;
    let mut seeded = HashMap::new();
    // A strand is a source paragraph's, or a created cell's, which has no
    // source runs of its own.
    let shape = view.state.shape.clone();
    let mut cells = HashSet::new();
    for strand in new {
        if !view.state.visible(strand) {
            return Ok(false);
        }
        let owner = strand.key.parse().ok().filter(|id| is_paragraph(*id));
        if let Some(owner) = owner {
            let mut runs = Vec::new();
            collect(base, owner, owner, &mut runs);
            seeded.insert(owner, runs);
        } else if shape.cells.contains_key(&strand.key) {
            cells.insert(strand.key.as_str());
        } else {
            return Ok(false);
        }
        owners.push((at, owner));
        at += strand.flow.len();
    }
    let reading = |strands: &[Strand]| Reading::from_strands(strands.iter().map(|s| (s, None)));
    let (old_units, new_units) = (reading(old)?, reading(new)?);
    let end = start + old_units.len();
    let (before, after) = (group(&old_units)?, group(&new_units)?);
    let within = |id| {
        seeded.contains_key(&id)
            || base
                .paragraph_of(id)
                .is_some_and(|p| seeded.contains_key(&p))
    };
    // Text typed in a created cell is like a paragraph of the file, which a
    // run is made from without reading any other strand.
    let based = |id| within(id) || (!cells.is_empty() && is_paragraph(id));
    let own = |p: &Located| {
        let marked = match &p.paragraph.mark {
            Mark::Base(id) => seeded.contains_key(id),
            Mark::Made { like, .. } => like.is_none_or(is_paragraph),
            Mark::Cell { key, .. } => cells.contains(key.as_str()),
            Mark::Lost => false,
        };
        marked
            && p.paragraph.text.iter().all(|inline| match inline {
                Inline::Char(c) => c.basis.is_none_or(based) && c.source_text.is_none_or(within),
                Inline::Source { node, basis, .. } => within(*node) && within(*basis),
                // A picture's drawing ID is chosen against the whole document.
                Inline::Object { object, basis, .. } => {
                    matches!(object.kind, NewInline::Break) && basis.is_none_or(based)
                }
                Inline::HiddenBoundary => false,
            })
    };
    if before.is_empty() || !before.iter().chain(&after).all(own) {
        return Ok(false);
    }
    // A list a mark defines is made, and dropped, for the whole document.
    let list_numbers: HashMap<String, String> = (view.lists.iter())
        .filter_map(|(number, basis)| match basis {
            ListBasis::Created { identity, .. } => Some((identity.clone(), number.clone())),
            ListBasis::Source(_) => None,
        })
        .collect();
    if defined_lists(&new_units, &after)? != defined_lists(&old_units, &before)? {
        return Ok(false);
    }
    if !view
        .comments
        .relocate(start, end, old, new, &before, &after)?
    {
        return Ok(false);
    }
    let spans: HashMap<usize, usize> = (before.iter())
        .map(|p| (start + p.first, p.paragraph.text.len()))
        .collect();
    let shown: HashMap<usize, NodeId> = (previous.places.paragraphs.iter())
        .filter(|(_, (first, len))| spans.get(first) == Some(len))
        .map(|(node, (first, _))| (*first, *node))
        .collect();
    if shown.len() != before.len() {
        return Ok(false);
    }
    let old_nodes: Vec<NodeId> = before.iter().map(|p| shown[&(start + p.first)]).collect();
    let Some(parent) = view.document.n(old_nodes[0]).parent else {
        return Ok(false);
    };
    let mut children = view.document.n(parent).children.clone();
    let Some(at) = children.iter().position(|c| *c == old_nodes[0]) else {
        return Ok(false);
    };
    if children.get(at..at + old_nodes.len()) != Some(&old_nodes[..]) {
        return Ok(false);
    }
    // New paragraphs whose text owns no source nodes can be written independently.
    // A shared source run can split across paragraphs; those paragraphs still
    // go through one writer so its source markup is placed exactly once.
    let same = |(a, b): (&Located, &Located)| {
        let old_at = start + a.first;
        let new_at = start + b.first;
        a.paragraph == b.paragraph
            && old_units[a.first - 1].over == new_units[b.first - 1].over
            && previous
                .comments
                .edges
                .range(old_at..=old_at + a.paragraph.text.len())
                .map(|(i, m)| (i - old_at, m))
                .eq(view
                    .comments
                    .edges
                    .range(new_at..=new_at + b.paragraph.text.len())
                    .map(|(i, m)| (i - new_at, m)))
    };
    let prefix = before
        .iter()
        .zip(&after)
        .take_while(|pair| same(*pair))
        .count();
    let suffix = before[prefix..]
        .iter()
        .rev()
        .zip(after[prefix..].iter().rev())
        .take_while(|pair| same(*pair))
        .count();
    let uses_only_authored_text = |p: &Located| {
        matches!(p.paragraph.mark, Mark::Made { .. }) && p.paragraph.text.iter().all(|inline| {
            matches!(inline, Inline::Char(c) if c.source_text.is_none() && c.basis.is_none_or(is_paragraph))
        })
    };
    let independent = before[prefix..before.len() - suffix]
        .iter()
        .chain(&after[prefix..after.len() - suffix])
        .all(uses_only_authored_text);
    // Rewriting the next mark can change the preceding paragraph’s revision.
    let independent = independent
        && !before
            .iter()
            .chain(&after)
            .any(|p| p.paragraph.revision.is_some() || p.paragraph.deletion.is_some());
    let (prefix, suffix) = if independent {
        (prefix, suffix)
    } else {
        (0, 0)
    };
    let at = at + prefix;
    let boundary = |i: usize| before.get(i).map_or(end, |p| start + p.first - 1);
    let (from, to) = (boundary(prefix), boundary(before.len() - suffix));
    let old_nodes = &old_nodes[prefix..old_nodes.len() - suffix];
    let before = &before[prefix..before.len() - suffix];
    let take = after.len() - prefix - suffix;
    // A list numbers the items after these paragraphs by them.
    let mut listed = old_nodes
        .iter()
        .any(|p| view.document.list_ref(*p).is_some());

    let delta = new_units.len() as isize - old_units.len() as isize;
    for map in [
        &mut view.places.texts,
        &mut view.places.runs,
        &mut view.places.items,
        &mut view.places.paragraphs,
    ] {
        map.retain(|_, span| span.0 <= from || span.0 > to);
        for span in map.values_mut().filter(|span| span.0 > to) {
            span.0 = span.0.saturating_add_signed(delta);
        }
    }

    // Only nodes owned by the replaced paragraphs can be recycled. Their
    // paragraph identities stay reserved until the new block order is known.
    for &p in old_nodes {
        for id in previous.document.descendants(p).into_iter().rev() {
            if base.node(id).is_none() {
                view.document.nodes.release(id);
            }
        }
    }
    // The strand's source nodes read as the file has them before they are
    // written, as in a full build. Facts about them are stated again.
    if !independent {
        let source =
            view.document
                .restore_paragraph_sources(base, seeded.keys().copied(), &children);
        apply_node_facts(&mut view.document, &view.state, Some(&source))?;
    }
    let located: Vec<Located> = (after.into_iter().skip(prefix).take(take))
        .map(|p| Located {
            first: start + p.first,
            ..p
        })
        .collect();
    let mut inlines = inlines::InlineWriter::new(base, &seeded, &located, owners);
    let mut kept: HashMap<_, _> = before
        .iter()
        .zip(old_nodes)
        .filter_map(|(p, id)| {
            if let Mark::Made { identity, .. } = &p.paragraph.mark {
                Some((identity, *id))
            } else {
                None
            }
        })
        .collect();
    let mut blocks = Vec::new();
    for (i, p) in located.iter().enumerate() {
        let node = match &p.paragraph.mark {
            Mark::Base(id) => *id,
            Mark::Made { identity, like } => view
                .document
                .rebuild_paragraph(parent, *like, kept.remove(identity))
                .map_err(unbuilt)?,
            // The cell's own paragraph starts as the table writer left it,
            // as it does in a full build.
            Mark::Cell { key, .. } => {
                let node = shape.cells[key].0;
                view.document.restore_made(&shape.cell_paragraphs[&node]);
                if let Some(basis) = stated_cell_basis(&view.state, key)? {
                    write_cell_basis(&mut view.document, node, &basis, &list_numbers)?;
                }
                node
            }
            Mark::Lost => unreachable!("only owned paragraph marks were admitted"),
        };
        write_paragraph(
            &mut inlines,
            &mut inlines::Target {
                document: &mut view.document,
                places: &mut view.places,
                comments: &view.comments,
            },
            &list_numbers,
            (parent, node),
            p,
            Some(&new_units[p.first - start - 1]),
            located.get(i + 1).map(|p| &p.paragraph),
        )?;
        blocks.push(node);
    }
    children.splice(at..at + old_nodes.len(), blocks.iter().copied());
    view.document.arrange(parent, &children).map_err(unbuilt)?;
    for id in kept.into_values() {
        view.document.nodes.release(id);
    }
    for (run, format) in inlines.formats {
        view.document.format_run(run, format).map_err(unbuilt)?;
    }
    listed = listed || blocks.iter().any(|p| view.document.list_ref(*p).is_some());
    let list_order_changed = listed
        && (old_nodes != blocks
            || blocks.iter().any(|p| {
                previous
                    .document
                    .node(*p)
                    .and_then(|_| previous.document.list_ref(*p))
                    != view.document.list_ref(*p)
            }));

    let story = view.document.stories[view.document.n(parent).story as usize].root;
    view.changes = (!list_order_changed).then(|| {
        vec![Splice {
            story,
            flow: parent,
            at,
            removed: old_nodes.len(),
            blocks,
        }]
    });
    Ok(true)
}
