//! Comment payloads have logical identities; Word numbers are export choices.
//! The paired range activates a comment. Payloads are immutable, so a reply
//! retains its parent even if that parent's author undoes the original add.
use super::*;
use crate::flow::Slot;
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Value {
    text: String,
    by: Track,
    initials: Option<String>,
    wanted: Option<u32>,
    parent: Option<Parent>,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
enum Parent {
    Source(String),
    Created(String),
}
#[derive(Clone, Default)]
pub(super) struct Comments {
    pub edges: BTreeMap<usize, Vec<(u32, bool)>>,
    pub names: HashMap<String, String>,
    // A retained root takes the range of its first active reply.
    retained: HashMap<String, String>,
}
impl View {
    /// Maps the current Word numbers to stable draft targets.
    pub fn comment_identities(&self) -> HashMap<String, String> {
        self.document
            .comments()
            .map(|node| {
                let number = self.document.comment_id(node);
                let identity = self
                    .comments
                    .names
                    .get(number)
                    .cloned()
                    .unwrap_or_else(|| format!("source/{number}"));
                (number.to_owned(), identity)
            })
            .collect()
    }
}

impl View {
    /// The selected part of each paragraph between two points, in the order
    /// the paragraphs are shown. A selection that holds no text keeps its ends.
    fn comment_stretches(
        &self,
        from: &Point,
        to: &Point,
        paragraphs: &[NodeId],
        command: &'static str,
    ) -> Result<Vec<(usize, usize)>, SessionError> {
        let unsupported = |reason| SessionError::Unsupported { command, reason };
        let from = self.position(from).map_err(unsupported)?;
        let to = self.position(to).map_err(unsupported)?;
        let stretches = self
            .paragraph_stretches(from, to, paragraphs)
            .ok_or_else(|| unsupported("the selection is outside its paragraph"))?;
        if stretches.is_empty() {
            let mut ends = vec![(from, from)];
            if to != from {
                ends.push((to, to));
            }
            return Ok(ends);
        }
        Ok(stretches.into_iter().map(|r| (r.start, r.end)).collect())
    }
}

/// Paired anchors in storage order. Replies read them only when submitted.
fn comment_pairs<'a>(
    slots: impl Iterator<Item = &'a Slot>,
) -> Result<BTreeMap<String, Vec<(usize, usize)>>, SessionError> {
    let mut parts = BTreeMap::<String, Vec<(usize, usize)>>::new();
    let mut open = HashMap::new();
    let mut at = 0;
    for slot in slots {
        match slot {
            Slot::Unit(_) => at += 1,
            Slot::Open(pair, span) if span.key == COMMENT => {
                let id = span
                    .value
                    .as_ref()
                    .ok_or_else(|| corrupt("comment pair has no identity".into()))?;
                open.insert(pair, (id, at));
            }
            Slot::Close(pair) => {
                if let Some((id, start)) = open.remove(pair) {
                    parts.entry(id.clone()).or_default().push((start, at));
                }
            }
            _ => {}
        }
    }
    Ok(parts)
}

impl Comments {
    /// Move the projected edges with the stored pairs of an edited strand.
    /// Outside this contiguous group of paragraphs only absolute offsets change.
    /// Activation or an anchor snapped from a missing paragraph needs the global reader.
    pub(super) fn relocate(
        &mut self,
        start: usize,
        end: usize,
        old: &[Strand],
        new: &[Strand],
        before: &[Located],
        after: &[Located],
    ) -> Result<bool, SessionError> {
        let old_pairs = comment_pairs(old.iter().flat_map(|s| s.flow.slots()))?;
        let new_pairs = comment_pairs(new.iter().flat_map(|s| s.flow.slots()))?;
        let on_caret = |at: usize, paragraphs: &[Located]| {
            let next = paragraphs.partition_point(|p| p.first <= at);
            next.checked_sub(1)
                .is_some_and(|i| at <= paragraphs[i].first + paragraphs[i].paragraph.text.len())
        };
        if old_pairs.keys().ne(new_pairs.keys())
            || old_pairs.iter().any(|(id, parts)| {
                let next = &new_pairs[id];
                parts.len() != next.len()
                    || [(parts, before), (next, after)]
                        .into_iter()
                        .any(|(ranges, ps)| {
                            ranges
                                .iter()
                                .any(|&(a, b)| !on_caret(a, ps) || !on_caret(b, ps))
                        })
            })
        {
            return Ok(false);
        }
        let delta =
            new.iter().map(|s| s.flow.len() as isize).sum::<isize>() - (end - start) as isize;
        let mut edges = BTreeMap::<usize, Vec<_>>::new();
        for (&at, marks) in &self.edges {
            for &(number, last) in marks {
                let next = if at <= start {
                    at
                } else if at > end {
                    at.saturating_add_signed(delta)
                } else {
                    let identity = &self.names[&number.to_string()];
                    let source = self.retained.get(identity).unwrap_or(identity);
                    let Some(parts) = old_pairs.get(source) else {
                        return Ok(false);
                    };
                    let extreme = |parts: &[(usize, usize)]| {
                        if last {
                            parts.iter().map(|p| p.1).max()
                        } else {
                            parts.iter().map(|p| p.0).min()
                        }
                    };
                    if extreme(parts) != Some(at - start) {
                        return Ok(false);
                    }
                    start + extreme(&new_pairs[source]).expect("active pair")
                };
                edges.entry(next).or_default().push((number, last));
            }
        }
        // Edges meeting after deletion use the same identity order as a full read.
        for marks in edges.values_mut().filter(|marks| marks.len() > 1) {
            marks.sort_by(|(a, ae), (b, be)| {
                self.names[&a.to_string()]
                    .cmp(&self.names[&b.to_string()])
                    .then(ae.cmp(be))
            });
        }
        self.edges = edges;
        Ok(true)
    }

    /// `shown` ranks each located paragraph by its place in the document,
    /// or gives none for a paragraph that is not shown.
    pub fn build(
        document: &mut Document,
        store: &PositionState,
        located: &[Located],
        shown: &dyn Fn(&Document) -> Vec<Option<usize>>,
    ) -> Result<Self, SessionError> {
        let mut all = BTreeMap::<String, Value>::new();
        for (key, fact) in store.facts() {
            if let Some(id) = key.strip_prefix(COMMENT_PAYLOAD) {
                all.insert(
                    id.to_owned(),
                    serde_json::from_slice(&fact.bytes)
                        .map_err(|_| corrupt("invalid comment payload".into()))?,
                );
            }
        }
        let stretches = comment_pairs(store.slots())?;
        if stretches.keys().any(|id| !all.contains_key(id)) {
            return Err(corrupt("comment pair has no payload".into()));
        }
        if stretches.is_empty() {
            return Ok(Self::default());
        }
        let shown = shown(document);
        // An anchor at a deleted mark lands at the nearest caret. The stored
        // pair is unchanged; this only chooses a valid output place. The
        // caret is ordered by where its paragraph is shown.
        let caret = |at: usize| {
            let next = located.partition_point(|p| p.first <= at);
            let (near, index) = [next.checked_sub(1), Some(next)]
                .into_iter()
                .flatten()
                .filter_map(|index| {
                    let p = located.get(index)?;
                    let near = at.clamp(p.first, p.first + p.paragraph.text.len());
                    Some((at.abs_diff(near), near, index))
                })
                .min()
                .map(|(_, near, index)| (near, index))?;
            Some(((shown[index]?, near), near))
        };
        // A comment runs from its first shown anchor to its last. One with
        // no shown anchor is inactive, as one in a hidden strand is.
        let mut ranges = BTreeMap::new();
        for (id, parts) in &stretches {
            let from = parts.iter().filter_map(|(from, _)| caret(*from)).min();
            let to = parts.iter().filter_map(|(_, to)| caret(*to)).max();
            if let (Some(from), Some(to)) = (from, to) {
                ranges.insert(
                    id.clone(),
                    (from.1, if to.0 < from.0 { from.1 } else { to.1 }),
                );
            }
        }
        let mut retained = HashMap::new();
        // Retain a root while any reply is active. Its own range wins;
        // otherwise the first active reply supplies the retained anchor.
        for (id, range) in ranges.clone() {
            if let Some(Parent::Created(root)) = &all[&id].parent {
                if !all.contains_key(root) {
                    return Err(corrupt("reply has no parent payload".into()));
                }
                if !ranges.contains_key(root) {
                    ranges.insert(root.clone(), range);
                    retained.insert(root.clone(), id);
                }
            }
        }
        let mut order: Vec<_> = ranges.keys().cloned().collect();
        // Assignment turns preserve causal posting order. A reply's parent
        // was observed first, so it is allocated before its replies.
        order.sort_by_key(|id| {
            (
                store.facts()[&format!("{COMMENT_PAYLOAD}{id}")].turn,
                id.clone(),
            )
        });
        let mut numbers: HashMap<String, u32> = HashMap::new();
        let mut names = HashMap::new();
        for id in &order {
            let v = &all[id];
            let number = document.free_comment_id(v.wanted).map_err(unbuilt)?;
            let parent = match &v.parent {
                None => None,
                Some(Parent::Source(id)) => Some(id.clone()),
                Some(Parent::Created(id)) => Some(
                    numbers
                        .get(id)
                        .ok_or_else(|| corrupt("reply parent is not a root".into()))?
                        .to_string(),
                ),
            };
            document.compose_comment(
                number,
                &v.text,
                &v.by,
                v.initials.as_deref(),
                parent.as_deref(),
            );
            numbers.insert(id.clone(), number);
            names.insert(number.to_string(), id.clone());
        }
        let mut edges: BTreeMap<usize, Vec<(u32, bool)>> = BTreeMap::new();
        for (id, (from, to)) in &ranges {
            edges.entry(*from).or_default().push((numbers[id], false));
            edges.entry(*to).or_default().push((numbers[id], true));
        }
        Ok(Self {
            edges,
            names,
            retained,
        })
    }
    pub fn marks(
        &self,
        document: &mut Document,
        places: &mut Places,
        paragraph: NodeId,
        at: usize,
    ) -> Vec<NodeId> {
        let mut out = Vec::new();
        if let Some(edges) = self.edges.get(&at) {
            for &(id, end) in edges {
                let nodes = document.loose_comment_marks(paragraph, id, end);
                for &node in &nodes {
                    places.items.insert(node, (at, 0));
                    for child in document.descendants(node) {
                        places.items.insert(child, (at, 0));
                    }
                }
                out.extend(nodes);
            }
        }
        out
    }
}
impl Working {
    fn add_annotation(
        &mut self,
        stretches: &[(usize, usize)],
        value: Value,
    ) -> Result<(), SessionError> {
        crate::revisions::check_track(&value.by)?;
        crate::comments::check_comment_text(&value.text, value.initials.as_deref())?;
        let (client, clock) = self.positions.next_identity();
        let id = format!("{client}.{clock}");
        self.positions.state.set_fact(
            format!("{COMMENT_PAYLOAD}{id}"),
            serde_json::to_vec(&value).expect("comment serializes"),
            true,
        )?;
        for (from, to) in stretches {
            self.positions
                .range(*from, to - from, (COMMENT, Some(&id)))?;
        }
        Ok(())
    }
    pub(super) fn comment(&mut self, command: &Command) -> Result<(), SessionError> {
        match command {
            Command::AddComment {
                from,
                to,
                text,
                by,
                initials,
                id,
            } => {
                let view = self.built()?;
                let paragraphs = view.document.paragraph_range(*from, *to)?;
                view.document.mark_place(*from)?;
                view.document.mark_place(*to)?;
                let stretches = view.comment_stretches(from, to, &paragraphs, command.name())?;
                self.add_annotation(
                    &stretches,
                    Value {
                        text: text.clone(),
                        by: by.clone(),
                        initials: initials.clone(),
                        wanted: *id,
                        parent: None,
                    },
                )?;
            }
            Command::ReplyToComment {
                comment,
                text,
                by,
                initials,
                id,
            } => {
                let view = self.built()?;
                let doc = &view.document;
                let mut node = doc.find_comment(comment).ok_or(crate::EditError::Comment {
                    comment: comment.clone(),
                    reason: "is not a comment in the document",
                })?;
                let mut seen = HashSet::new();
                while let Some(parent) = doc.comment_parent(node).and_then(|p| doc.find_comment(p))
                {
                    if !seen.insert(node) {
                        return Err(crate::EditError::Comment {
                            comment: comment.clone(),
                            reason: "has a cyclic reply thread",
                        }
                        .into());
                    }
                    node = parent;
                }
                let root = doc.comment_id(node).to_owned();
                if doc.last_para_id(node).is_none() {
                    return Err(crate::EditError::Comment {
                        comment: root,
                        reason: "has no paragraph ID, so Word could not thread a reply to it",
                    }
                    .into());
                }
                let (parent, stretches) = if let Some(identity) = view.comments.names.get(&root) {
                    (
                        Parent::Created(identity.clone()),
                        comment_pairs(view.state.slots())?
                            .remove(view.comments.retained.get(identity).unwrap_or(identity))
                            .ok_or_else(|| corrupt("active comment has no range".into()))?,
                    )
                } else {
                    let range = doc.reply_places(&root)?;
                    let paragraphs = doc.paragraph_range(range.start, range.end)?;
                    let stretches = view.comment_stretches(
                        &range.start,
                        &range.end,
                        &paragraphs,
                        command.name(),
                    )?;
                    (Parent::Source(root), stretches)
                };
                self.add_annotation(
                    &stretches,
                    Value {
                        text: text.clone(),
                        by: by.clone(),
                        initials: initials.clone(),
                        wanted: *id,
                        parent: Some(parent),
                    },
                )?;
            }
            _ => unreachable!("comment command"),
        }
        Ok(())
    }
}
