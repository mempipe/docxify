//! Named, ordered text strands and document-wide assignments. Splitting a
//! paragraph changes a boundary in its strand; it never moves characters.
//! A prepared edit copies only the strands it writes. The Yrs adapter uses
//! the same strand names and commits already-positioned leaf writes.
use crate::flow::{Action, Fact, Flow, FlowError, FlowUndo, Insert, Slot, Stated, Unit, Write};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone)]
pub struct Strand {
    pub key: String,
    pub order: u64,
    pub prefix: Option<Slot>,
    pub flow: Arc<Flow>,
    pub declared: bool,
}
#[derive(Clone)]
pub struct State {
    strands: Vec<Strand>,
    facts: BTreeMap<String, Fact>,
    pending: PendingWrites,
}

/// Headless history retains changed slots and mutable facts. Permanent facts
/// and the identity clock belong to the current state across undo and redo.
pub(crate) struct StateUndo {
    strands: BTreeMap<String, StrandUndo>,
    facts: BTreeMap<String, Option<Fact>>,
}

enum StrandUndo {
    Edit(FlowUndo),
    Created(Option<Strand>),
}

impl StateUndo {
    pub(crate) fn between(before: &State, after: &State, writes: &[Write]) -> Self {
        let mut strands = BTreeMap::new();
        let mut facts = BTreeMap::new();
        for write in writes {
            if let Action::Fact { key, value } = &write.action {
                if !value.permanent {
                    facts
                        .entry(key.clone())
                        .or_insert_with(|| before.facts.get(key).cloned());
                }
            } else {
                strands.entry(write.strand.clone()).or_insert_with(|| {
                    match before.strands.iter().find(|s| s.key == write.strand) {
                        Some(old) => {
                            let new = after
                                .strands
                                .iter()
                                .find(|s| s.key == write.strand)
                                .expect("a written strand is stored");
                            StrandUndo::Edit(FlowUndo::between(&old.flow, &new.flow))
                        }
                        None => StrandUndo::Created(None),
                    }
                });
            }
        }
        Self { strands, facts }
    }

    pub(crate) fn swap(&mut self, state: &mut State) {
        for (key, change) in &mut self.strands {
            if let StrandUndo::Created(saved) = change {
                if let Some(strand) = saved.take() {
                    state.ensure(strand);
                    continue;
                }
            }
            let i = state
                .strands
                .iter()
                .position(|s| &s.key == key)
                .expect("undo and redo follow the command order");
            match change {
                StrandUndo::Edit(edit) => edit.swap(Arc::make_mut(&mut state.strands[i].flow)),
                StrandUndo::Created(saved) => *saved = Some(state.strands.remove(i)),
            }
        }
        for (key, other) in &mut self.facts {
            *other = match other.take() {
                Some(fact) => state.facts.insert(key.clone(), fact),
                None => state.facts.remove(key),
            };
        }
    }
}

/// Which strands a reading shows. A table's shape decides that from the
/// structural facts, so a state does not hold it: every method that counts
/// positions is told the layout to count them in.
pub trait Layout {
    /// Whether a strand is left out whole. Structural interpretation hides
    /// the strands of content a removal took out; the stored text stays, and
    /// undo shows it again without moving anything.
    fn hides(&self, key: &str) -> bool;
    /// Whether the boundary that opens a strand is left out, which joins the
    /// strand to the one before it after the block between them is removed.
    fn joins(&self, key: &str) -> bool;
}

/// Every strand shown with its boundary, as where no table or block changed.
pub struct Plain;
impl Layout for Plain {
    fn hides(&self, _: &str) -> bool {
        false
    }
    fn joins(&self, _: &str) -> bool {
        false
    }
}

/// Only preparation advances this clock. The adapter commits the positioned
/// writes and checks the resulting clock before publishing the change.
#[derive(Clone)]
struct PendingWrites {
    client: u64,
    clock: u32,
    writes: Vec<Write>,
}

fn corrupt(reason: &str) -> FlowError {
    FlowError::Corrupt {
        flow: "state".into(),
        reason: reason.into(),
    }
}
impl State {
    pub fn new(
        client: u64,
        clock: u32,
        mut strands: Vec<Strand>,
        facts: BTreeMap<String, Fact>,
    ) -> Self {
        strands.sort_by(|a, b| (a.order, &a.key).cmp(&(b.order, &b.key)));
        Self {
            strands,
            facts,
            pending: PendingWrites {
                client,
                clock,
                writes: Vec::new(),
            },
        }
    }
    pub fn ensure(&mut self, strand: Strand) {
        if !self.strands.iter().any(|s| s.key == strand.key) {
            self.strands.push(strand);
            self.strands
                .sort_by(|a, b| (a.order, &a.key).cmp(&(b.order, &b.key)));
        }
    }
    pub fn visible(&self, layout: &dyn Layout, s: &Strand) -> bool {
        !layout.hides(&s.key)
    }
    pub fn prefix<'a>(&self, layout: &dyn Layout, s: &'a Strand) -> Option<&'a Slot> {
        if layout.joins(&s.key) || layout.hides(&s.key) {
            None
        } else {
            s.prefix.as_ref()
        }
    }
    pub fn strand_len(&self, layout: &dyn Layout, s: &Strand) -> usize {
        if self.visible(layout, s) {
            usize::from(self.prefix(layout, s).is_some()) + s.flow.len()
        } else {
            0
        }
    }
    pub fn strands(&self) -> &[Strand] {
        &self.strands
    }
    /// Empty created cells belong to the current table shape until their first write.
    pub(crate) fn into_stored(mut self) -> Self {
        self.strands.retain(|s| s.declared);
        self
    }
    pub fn facts(&self) -> &BTreeMap<String, Fact> {
        &self.facts
    }
    pub fn slots<'a>(&'a self, layout: &'a dyn Layout) -> impl Iterator<Item = &'a Slot> {
        self.strands
            .iter()
            .filter(|s| self.visible(layout, s))
            .flat_map(|s| self.prefix(layout, s).into_iter().chain(s.flow.slots()))
    }
    pub fn read(&self, layout: &dyn Layout) -> Result<Vec<Unit>, FlowError> {
        let mut out = Vec::new();
        for s in &self.strands {
            if !self.visible(layout, s) {
                continue;
            }
            if let Some(Slot::Unit(content)) = self.prefix(layout, s) {
                out.push(Unit {
                    content: content.clone(),
                    over: Default::default(),
                });
            }
            out.extend(s.flow.read()?);
        }
        Ok(out)
    }
    pub fn len(&self, layout: &dyn Layout) -> usize {
        (self.strands.iter())
            .map(|s| self.strand_len(layout, s))
            .sum()
    }
    pub fn next_identity(&self) -> (u64, u32) {
        (self.pending.client, self.pending.clock)
    }
    pub fn take_writes(&mut self) -> Vec<Write> {
        std::mem::take(&mut self.pending.writes)
    }
    pub fn set_fact(
        &mut self,
        key: String,
        bytes: Vec<u8>,
        permanent: bool,
    ) -> Result<(), FlowError> {
        if self
            .facts
            .get(&key)
            .is_some_and(|v| v.bytes.as_ref() == bytes && v.permanent == permanent)
        {
            return Ok(());
        }
        if self.facts.get(&key).is_some_and(|v| v.permanent) {
            return Err(corrupt("an immutable fact changed"));
        }
        let value = Fact {
            identity: format!("{}.{}", self.pending.client, self.pending.clock),
            turn: self.facts.values().map(|v| v.turn).max().unwrap_or(0) + 1,
            bytes: bytes.into(),
            permanent,
        };
        self.facts.insert(key.clone(), value.clone());
        self.pending.clock += 1;
        self.pending.writes.push(Write {
            strand: String::new(),
            tracked: !permanent,
            action: Action::Fact { key, value },
        });
        Ok(())
    }
    /// Index of the strand and its local content offset. At a strand gap
    /// stay at the preceding strand's end. Both positions precede the next
    /// mark, but this keeps typing at a paragraph end in its own strand.
    fn locate(&self, layout: &dyn Layout, at: usize) -> Result<(usize, usize), FlowError> {
        let mut first = 0;
        for (i, s) in self.strands.iter().enumerate() {
            if !self.visible(layout, s) {
                continue;
            }
            let end = first + self.strand_len(layout, s);
            if at <= end {
                if at < first + usize::from(self.prefix(layout, s).is_some()) {
                    return if let Some((j, previous)) = self.strands[..i]
                        .iter()
                        .enumerate()
                        .rev()
                        .find(|(_, s)| self.visible(layout, s))
                    {
                        Ok((j, previous.flow.len()))
                    } else {
                        Err(corrupt("position is before a text region"))
                    };
                }
                return Ok((
                    i,
                    at - first - usize::from(self.prefix(layout, s).is_some()),
                ));
            }
            first = end;
        }
        Err(corrupt("position is outside the strands"))
    }
    pub(crate) fn anchor(
        &self,
        layout: &dyn Layout,
        at: usize,
    ) -> Result<(String, usize), FlowError> {
        let (i, local) = self.locate(layout, at)?;
        Ok((self.strands[i].key.clone(), local))
    }
    pub(crate) fn position(
        &self,
        layout: &dyn Layout,
        key: &str,
        local: usize,
    ) -> Result<usize, FlowError> {
        let mut first = 0;
        for s in &self.strands {
            if !self.visible(layout, s) {
                continue;
            }
            if s.key == key && local <= s.flow.len() {
                return Ok(first + usize::from(self.prefix(layout, s).is_some()) + local);
            }
            first += self.strand_len(layout, s);
        }
        Err(corrupt("anchor is outside visible strands"))
    }
    pub(crate) fn raw_at(
        &self,
        layout: &dyn Layout,
        at: usize,
    ) -> Result<(String, u32), FlowError> {
        let (i, local) = self.locate(layout, at)?;
        let s = &self.strands[i];
        Ok((s.key.clone(), s.flow.raw_at(local)?))
    }
    pub(crate) fn unit_at(&self, layout: &dyn Layout, key: &str, raw: u32) -> Option<usize> {
        let mut first = 0;
        for s in &self.strands {
            if !self.visible(layout, s) {
                continue;
            }
            if s.key == key {
                return s
                    .flow
                    .unit_at(raw)
                    .map(|at| first + usize::from(self.prefix(layout, s).is_some()) + at);
            }
            first += self.strand_len(layout, s);
        }
        None
    }
    fn mutate(
        &mut self,
        i: usize,
        f: impl FnOnce(&mut Flow) -> Result<(), FlowError>,
    ) -> Result<(), FlowError> {
        let s = &mut self.strands[i];
        if !s.declared {
            let prefix = s.prefix.as_ref().and_then(|s| {
                if let Slot::Unit(c) = s {
                    Some(c.clone())
                } else {
                    None
                }
            });
            self.pending.writes.push(Write {
                strand: s.key.clone(),
                tracked: false,
                action: Action::Declare {
                    key: s.key.clone(),
                    order: s.order,
                    prefix,
                },
            });
            self.pending.clock += 1;
            s.declared = true;
            let mut at = 0;
            for slot in s.flow.slots() {
                self.pending.writes.push(Write {
                    strand: s.key.clone(),
                    tracked: false,
                    action: Action::Item {
                        at,
                        slot: slot.clone(),
                    },
                });
                at += slot.width();
                self.pending.clock += slot.width();
            }
        }
        let flow = Arc::make_mut(&mut s.flow);
        flow.set_clock(self.pending.clock);
        f(flow)?;
        self.pending.clock = flow.next_identity().1;
        for mut write in flow.take_writes() {
            write.strand = s.key.clone();
            self.pending.writes.push(write);
        }
        Ok(())
    }
    pub fn insert(
        &mut self,
        layout: &dyn Layout,
        at: usize,
        text: &str,
        stated: &[Stated],
        fresh_key: &str,
    ) -> Result<(), FlowError> {
        self.insert_at(layout, at, Insert::Text(text), stated, fresh_key)
    }
    pub fn item(
        &mut self,
        layout: &dyn Layout,
        at: usize,
        kind: &str,
        name: &str,
    ) -> Result<(), FlowError> {
        let (i, local) = self.locate(layout, at)?;
        self.mutate(i, |flow| flow.item(local, kind, name))
    }
    pub(crate) fn insert_at(
        &mut self,
        layout: &dyn Layout,
        at: usize,
        content: Insert,
        stated: &[Stated],
        fresh_key: &str,
    ) -> Result<(), FlowError> {
        let (i, local) = self.locate(layout, at)?;
        self.mutate(i, |flow| flow.insert_at(local, content, stated, fresh_key))
    }
    fn covered(
        &self,
        layout: &dyn Layout,
        at: usize,
        len: usize,
    ) -> Result<Vec<(usize, usize, usize)>, FlowError> {
        let end = at
            .checked_add(len)
            .ok_or_else(|| corrupt("range overflows"))?;
        if end > self.len(layout) {
            return Err(corrupt("range is outside the strands"));
        }
        if len == 0 {
            let (i, at) = self.locate(layout, at)?;
            return Ok(vec![(i, at, 0)]);
        }
        let mut parts = Vec::new();
        let mut first = 0;
        for (i, s) in self.strands.iter().enumerate() {
            if !self.visible(layout, s) {
                continue;
            }
            let start = first + usize::from(self.prefix(layout, s).is_some());
            let stop = first + self.strand_len(layout, s);
            let a = at.max(start);
            let b = end.min(stop);
            if a < b {
                parts.push((i, a - start, b - a));
            }
            first = stop;
        }
        Ok(parts)
    }
    pub fn delete(&mut self, layout: &dyn Layout, at: usize, len: usize) -> Result<(), FlowError> {
        for (i, at, len) in self.covered(layout, at, len)?.into_iter().rev() {
            self.mutate(i, |flow| flow.delete(at, len))?;
        }
        Ok(())
    }
    pub fn range(
        &mut self,
        layout: &dyn Layout,
        at: usize,
        len: usize,
        stated: Stated,
    ) -> Result<(), FlowError> {
        let turn = self
            .strands
            .iter()
            .map(|s| s.flow.turn())
            .max()
            .unwrap_or(0)
            + 1;
        for (i, at, len) in self.covered(layout, at, len)? {
            self.mutate(i, |flow| flow.range_at_turn(at, len, stated, turn))?;
        }
        Ok(())
    }
}

/// An immutable indexed reading shares each strand's decoded units. Reading
/// a short range does not copy or decode the rest of the document.
pub struct Reading {
    parts: Vec<(usize, Arc<Vec<Unit>>)>,
    len: usize,
}
impl Reading {
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn get(&self, at: usize) -> Option<&Unit> {
        let index = self
            .parts
            .partition_point(|(start, _)| *start <= at)
            .checked_sub(1)?;
        let (start, units) = &self.parts[index];
        units.get(at - start)
    }
    pub fn first(&self) -> Option<&Unit> {
        self.get(0)
    }
    pub fn range(
        &self,
        range: impl std::ops::RangeBounds<usize>,
    ) -> impl DoubleEndedIterator<Item = &Unit> {
        use std::ops::Bound;
        let start = match range.start_bound() {
            Bound::Included(i) => *i,
            Bound::Excluded(i) => i + 1,
            Bound::Unbounded => 0,
        };
        let end = match range.end_bound() {
            Bound::Excluded(i) => *i,
            Bound::Included(i) => i + 1,
            Bound::Unbounded => self.len,
        };
        let a = self
            .parts
            .partition_point(|(at, _)| *at <= start)
            .saturating_sub(1);
        let b = self.parts.partition_point(|(at, _)| *at < end).max(a);
        self.parts[a..b].iter().flat_map(move |(at, units)| {
            let a = start.saturating_sub(*at).min(units.len());
            let b = end.saturating_sub(*at).min(units.len());
            units[a..b].iter()
        })
    }
}
impl std::ops::Index<usize> for Reading {
    type Output = Unit;
    fn index(&self, at: usize) -> &Unit {
        self.get(at).expect("unit in reading")
    }
}
impl State {
    pub fn reading(&self, layout: &dyn Layout) -> Result<Reading, FlowError> {
        Reading::from_strands(
            self.strands
                .iter()
                .filter(|s| self.visible(layout, s))
                .map(|s| (s, self.prefix(layout, s))),
        )
    }
}

impl Reading {
    pub(super) fn from_strands<'a>(
        strands: impl Iterator<Item = (&'a Strand, Option<&'a Slot>)>,
    ) -> Result<Self, FlowError> {
        let mut parts = Vec::new();
        let mut len = 0;
        for (strand, prefix) in strands {
            if let Some(Slot::Unit(content)) = prefix {
                parts.push((
                    len,
                    Arc::new(vec![Unit {
                        content: content.clone(),
                        over: Default::default(),
                    }]),
                ));
                len += 1;
            }
            let units = strand.flow.read_shared()?;
            if !units.is_empty() {
                let n = units.len();
                parts.push((len, units));
                len += n;
            }
        }
        Ok(Reading { parts, len })
    }
}
