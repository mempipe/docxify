//! A flow and preparation of its changes, without a CRDT dependency.
//! Boundaries and their interpretation are shared by the headless editor
//! and the Yrs adapter. Prepared writes retain their undo ownership.
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum FlowError {
    #[error("flow {flow}: {reason}")]
    Corrupt { flow: String, reason: String },
}

/// A character, or an item such as a paragraph mark.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Content {
    Char(char),
    Item { kind: String, name: String },
}

/// One unit of the flow, and every pair over it, earliest first. A value of
/// `None` says the key is not set, over what an earlier pair gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    pub content: Content,
    pub over: Arc<[(String, Option<String>)]>,
}

impl Unit {
    /// What a key with one value at a time has: what the latest pair gives.
    pub fn one(&self, key: &str) -> Option<&str> {
        let latest = self.over.iter().rev().find(|(k, _)| k == key)?;
        latest.1.as_deref()
    }

    /// What a key that holds several has: every value, in order.
    pub fn all(&self, key: &str) -> Vec<&str> {
        let over = self.over.iter().filter(|(k, _)| k == key);
        let mut values: Vec<&str> = over.filter_map(|(_, v)| v.as_deref()).collect();
        values.sort_unstable();
        values
    }
}

/// Content a flow starts with (`Store::seeded`).
pub enum Seeded<'a> {
    Item {
        kind: &'a str,
        name: &'a str,
    },
    /// Text, with each key it has. A key with no value is left out.
    Text {
        text: &'a str,
        stated: Vec<Stated<'a>>,
    },
}

/// What one index of the Yrs text holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slot {
    Unit(Content),
    Open(String, Span),
    Close(String),
}

impl Slot {
    /// How many UTF-16 units of the Yrs text it takes.
    pub fn width(&self) -> u32 {
        match self {
            Slot::Unit(Content::Char(c)) => c.len_utf16() as u32,
            _ => 1,
        }
    }
}

/// A pair's key and value, and a count that is higher than that of every
/// pair its writer had seen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub key: String,
    pub value: Option<String>,
    pub turn: u64,
}

/// The pairs open at a place, by name.
type Open = BTreeMap<String, Span>;

/// The open pairs, earliest first: by count, then by name.
fn ordered(open: &Open) -> Vec<(String, Option<String>)> {
    let mut pairs: Vec<(&String, &Span)> = open.iter().collect();
    pairs.sort_unstable_by(|(a, x), (b, y)| x.turn.cmp(&y.turn).then_with(|| a.cmp(b)));
    let given = |(_, span): (&String, &Span)| (span.key.clone(), span.value.clone());
    pairs.into_iter().map(given).collect()
}

/// What the open pairs give a key with one value at a time.
fn held<'a>(open: &'a Open, key: &str) -> Option<&'a str> {
    let over = open.iter().filter(|(_, span)| span.key == key);
    let latest = over.max_by(|(a, x), (b, y)| x.turn.cmp(&y.turn).then_with(|| a.cmp(b)))?;
    latest.1.value.as_deref()
}

fn corrupt(reason: impl Into<String>) -> FlowError {
    FlowError::Corrupt {
        flow: "store".to_string(),
        reason: reason.into(),
    }
}

/// Passes one slot. A half with no other half would change what all the
/// content after it has. No edit here leaves one: both halves are written,
/// undone and redone in one transaction, and a deletion takes neither.
fn pass(open: &mut Open, slot: &Slot) -> Result<(), FlowError> {
    let known = match slot {
        Slot::Open(name, span) => open.insert(name.clone(), span.clone()).is_none(),
        Slot::Close(name) => open.remove(name).is_some(),
        Slot::Unit(_) => true,
    };
    match known {
        true => Ok(()),
        false => Err(corrupt("a pair has one half, or its end before its start")),
    }
}

/// The Yrs index unit `at` starts at, and its place among the slots. One
/// past the last unit is the end of the text. Every index into the text
/// comes from here.
fn place(slots: &[Slot], at: usize) -> Result<(u32, usize), FlowError> {
    let (mut raw, mut units) = (0, 0);
    for (i, slot) in slots.iter().enumerate() {
        if matches!(slot, Slot::Unit(_)) {
            if units == at {
                return Ok((raw, i));
            }
            units += 1;
        }
        raw += slot.width();
    }
    match units == at {
        true => Ok((raw, slots.len())),
        false => Err(corrupt(format!("unit {at} of {units}"))),
    }
}

/// The place just after the unit before `at`, which is inside every pair
/// that unit is in.
fn after(slots: &[Slot], at: usize) -> Result<(u32, usize), FlowError> {
    let Some(before) = at.checked_sub(1) else {
        return Ok((0, 0));
    };
    let (raw, slot) = place(slots, before)?;
    match slots.get(slot) {
        Some(unit) => Ok((raw + unit.width(), slot + 1)),
        None => Err(corrupt(format!("unit {at} is past the end"))),
    }
}

/// A key and the value content states for it.
pub type Stated<'a> = (&'a str, Option<&'a str>);

pub(crate) enum Insert<'a> {
    Text(&'a str),
    Item { kind: &'a str, name: &'a str },
}

/// Where an insert before unit `at` goes, and the pairs it has to bring.
/// The places among the halves between two units are one place in the
/// document, so the insert takes the first of them that leaves it the
/// fewest pairs to add. On a tie it may pass the end of an older pair for
/// the key the command minted, if it writes its own pair in either place.
/// Crossing an inherited range could change what survives another author's undo.
fn fit<'a>(
    slots: &[Slot],
    at: usize,
    stated: &[Stated<'a>],
    fresh_key: &str,
) -> Result<(u32, Vec<Stated<'a>>), FlowError> {
    let (mut raw, mut slot) = after(slots, at)?;
    let mut open = Open::new();
    for passed in &slots[..slot] {
        pass(&mut open, passed)?;
    }
    let missing = |open: &Open| -> Vec<Stated<'a>> {
        let differs = stated
            .iter()
            .filter(|(key, value)| held(open, key) != *value);
        differs.copied().collect()
    };
    let mut best = (raw, missing(&open));
    let mut only_fresh_closes = true;
    while let Some(half) = slots.get(slot).filter(|s| !matches!(s, Slot::Unit(_))) {
        only_fresh_closes &= matches!(half, Slot::Close(name)
            if open.get(name).is_some_and(|span| span.key == fresh_key));
        pass(&mut open, half)?;
        (raw, slot) = (raw + 1, slot + 1);
        let here = missing(&open);
        if here.len() < best.1.len()
            || (only_fresh_closes
                && here == best.1
                && here.iter().any(|(key, _)| *key == fresh_key))
        {
            best = (raw, here);
            only_fresh_closes = true;
        }
    }
    Ok(best)
}

/// One already-positioned mutation. Indices are UTF-16 units, including
/// boundaries. The Yrs adapter commits these without repeating placement.
#[derive(Clone, Debug)]
pub enum Action {
    Text {
        at: u32,
        text: String,
    },
    Item {
        at: u32,
        slot: Slot,
    },
    Delete {
        at: u32,
        len: u32,
    },
    Fact {
        key: String,
        value: Fact,
    },
    /// Declare a named strand before its first local mutation.
    Declare {
        key: String,
        order: u64,
        prefix: Option<Content>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fact {
    pub identity: String,
    pub turn: u64,
    pub bytes: Arc<[u8]>,
    pub permanent: bool,
}

#[derive(Clone, Debug)]
pub struct Write {
    pub strand: String,
    pub tracked: bool,
    pub action: Action,
}

#[derive(Clone)]
pub struct Flow {
    slots: Vec<Slot>,
    client: u64,
    clock: u32,
    writes: Vec<Write>,
    reading: std::sync::OnceLock<Arc<Vec<Unit>>>,
    length: usize,
    turn: u64,
}

/// The changed portion of one strand. Swapping it again redoes the edit.
pub(crate) struct FlowUndo {
    start: usize,
    replaced: usize,
    slots: Vec<Slot>,
    length: usize,
    turn: u64,
}

impl FlowUndo {
    pub(crate) fn between(before: &Flow, after: &Flow) -> Self {
        let a = before.slots();
        let b = after.slots();
        let start = a.iter().zip(b).take_while(|(a, b)| a == b).count();
        let suffix = a[start..]
            .iter()
            .rev()
            .zip(b[start..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        Self {
            start,
            replaced: b.len() - suffix - start,
            slots: a[start..a.len() - suffix].to_vec(),
            length: before.length,
            turn: before.turn,
        }
    }

    pub(crate) fn swap(&mut self, flow: &mut Flow) {
        let end = self.start + self.replaced;
        self.replaced = self.slots.len();
        self.slots = flow
            .slots
            .splice(self.start..end, std::mem::take(&mut self.slots))
            .collect();
        std::mem::swap(&mut self.length, &mut flow.length);
        std::mem::swap(&mut self.turn, &mut flow.turn);
        flow.reading.take();
    }
}

impl Flow {
    pub fn from_slots(client: u64, clock: u32, slots: Vec<Slot>) -> Result<Self, FlowError> {
        let length = slots.iter().filter(|s| matches!(s, Slot::Unit(_))).count();
        let turn = slots
            .iter()
            .filter_map(|s| {
                if let Slot::Open(_, span) = s {
                    Some(span.turn)
                } else {
                    None
                }
            })
            .max()
            .unwrap_or(0);
        let flow = Self {
            length,
            turn,
            reading: Default::default(),
            slots,
            client,
            clock,
            writes: Vec::new(),
        };
        let mut open = Open::new();
        for slot in &flow.slots {
            pass(&mut open, slot)?;
        }
        if !open.is_empty() {
            return Err(corrupt("a pair has a start and no end"));
        }
        Ok(flow)
    }

    /// Mapping between semantic units and encoded positions, also used
    /// by relative cursors. A surrogate pair is one semantic character.
    pub(crate) fn set_clock(&mut self, clock: u32) {
        self.clock = clock;
    }
    pub fn len(&self) -> usize {
        self.length
    }
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }
    pub fn turn(&self) -> u64 {
        self.turn
    }
    pub fn raw_at(&self, at: usize) -> Result<u32, FlowError> {
        Ok(place(&self.slots, at)?.0)
    }
    pub fn unit_at(&self, raw: u32) -> Option<usize> {
        let (mut offset, mut units) = (0, 0);
        for slot in &self.slots {
            if offset >= raw {
                return Some(units);
            }
            offset += slot.width();
            if matches!(slot, Slot::Unit(_)) {
                if raw < offset {
                    return Some(units);
                }
                units += 1;
            }
        }
        (offset == raw).then_some(units)
    }

    /// Identity available for an inserted semantic object. Unlike the Yrs
    /// item's eventual ID, the payload keeps this identity through undo.
    pub fn next_identity(&self) -> (u64, u32) {
        (self.client, self.clock)
    }

    pub fn slots(&self) -> &[Slot] {
        &self.slots
    }
    pub fn take_writes(&mut self) -> Vec<Write> {
        std::mem::take(&mut self.writes)
    }

    pub fn seeded(client: u64, content: &[Seeded]) -> Self {
        let mut flow = Self {
            length: 0,
            turn: 0,
            reading: Default::default(),
            slots: Vec::new(),
            client,
            clock: 0,
            writes: Vec::new(),
        };
        let mut raw = 0;
        for seeded in content {
            match seeded {
                Seeded::Item { kind, name } => {
                    flow.put_item(
                        raw,
                        flow.slots.len(),
                        Slot::Unit(Content::Item {
                            kind: (*kind).into(),
                            name: (*name).into(),
                        }),
                        false,
                    );
                    raw += 1;
                }
                Seeded::Text { text, stated } => {
                    let mut closes = Vec::new();
                    for stated in stated.iter().filter(|(_, v)| v.is_some()) {
                        let (open, close) = flow.pair_at(*stated, 1);
                        flow.put_item(raw, flow.slots.len(), open, false);
                        closes.push(close);
                        raw += 1;
                    }
                    flow.put_text(raw, flow.slots.len(), text, false);
                    raw += text.encode_utf16().count() as u32;
                    for close in closes.into_iter().rev() {
                        flow.put_item(raw, flow.slots.len(), close, false);
                        raw += 1;
                    }
                }
            }
        }
        flow.writes.clear();
        flow
    }

    pub fn read(&self) -> Result<Vec<Unit>, FlowError> {
        Ok(self.read_shared()?.as_ref().clone())
    }
    pub fn read_shared(&self) -> Result<Arc<Vec<Unit>>, FlowError> {
        if let Some(read) = self.reading.get() {
            return Ok(read.clone());
        }
        let read = Arc::new(self.decode()?);
        let _ = self.reading.set(read.clone());
        Ok(read)
    }
    fn decode(&self) -> Result<Vec<Unit>, FlowError> {
        let (mut open, mut units) = (Open::new(), Vec::new());
        // Adjacent units between the same pair boundaries share one annotation reading.
        let mut annotations = None;
        for slot in &self.slots {
            match slot {
                Slot::Unit(content) => {
                    let over = annotations.get_or_insert_with(|| Arc::from(ordered(&open)));
                    units.push(Unit {
                        content: content.clone(),
                        over: Arc::clone(over),
                    });
                }
                _ => {
                    pass(&mut open, slot)?;
                    annotations = None;
                }
            }
        }
        if !open.is_empty() {
            return Err(corrupt("a pair has a start and no end"));
        }
        Ok(units)
    }

    fn pair(&self, stated: Stated) -> (Slot, Slot) {
        let turn = self.turn + 1;
        self.pair_at(stated, turn)
    }

    fn pair_at(&self, stated: Stated, turn: u64) -> (Slot, Slot) {
        let name = format!("{}.{}", self.client, self.clock);
        (
            Slot::Open(
                name.clone(),
                Span {
                    key: stated.0.into(),
                    value: stated.1.map(str::to_owned),
                    turn,
                },
            ),
            Slot::Close(name),
        )
    }

    fn put_item(&mut self, raw: u32, index: usize, slot: Slot, tracked: bool) {
        self.reading.take();
        if matches!(slot, Slot::Unit(_)) {
            self.length += 1;
        }
        if let Slot::Open(_, span) = &slot {
            self.turn = self.turn.max(span.turn);
        }
        self.slots.insert(index, slot.clone());
        self.clock += 1;
        self.writes.push(Write {
            strand: String::new(),
            tracked,
            action: Action::Item { at: raw, slot },
        });
    }

    fn put_text(&mut self, raw: u32, index: usize, text: &str, tracked: bool) {
        if text.is_empty() {
            return;
        }
        self.reading.take();
        self.length += text.chars().count();
        self.slots.splice(
            index..index,
            text.chars().map(|c| Slot::Unit(Content::Char(c))),
        );
        self.clock += text.encode_utf16().count() as u32;
        self.writes.push(Write {
            strand: String::new(),
            tracked,
            action: Action::Text {
                at: raw,
                text: text.into(),
            },
        });
    }

    fn index_at(&self, raw: u32) -> usize {
        let mut at = 0;
        for (i, slot) in self.slots.iter().enumerate() {
            if at == raw {
                return i;
            }
            at += slot.width();
        }
        assert_eq!(at, raw, "prepared position is a slot boundary");
        self.slots.len()
    }

    pub fn item(&mut self, at: usize, kind: &str, name: &str) -> Result<(), FlowError> {
        let (raw, index) = after(&self.slots, at)?;
        self.put_item(
            raw,
            index,
            Slot::Unit(Content::Item {
                kind: kind.into(),
                name: name.into(),
            }),
            true,
        );
        Ok(())
    }

    pub(crate) fn insert_at(
        &mut self,
        at: usize,
        content: Insert,
        stated: &[Stated],
        fresh_key: &str,
    ) -> Result<(), FlowError> {
        let (raw, pairs) = fit(&self.slots, at, stated, fresh_key)?;
        let index = self.index_at(raw);
        for (i, stated) in pairs.iter().enumerate() {
            let (open, close) = self.pair(*stated);
            self.put_item(raw + i as u32, index + i, open, false);
            self.put_item(raw + i as u32 + 1, index + i + 1, close, false);
        }
        let (raw, index) = (raw + pairs.len() as u32, index + pairs.len());
        match content {
            Insert::Text(text) => self.put_text(raw, index, text, true),
            Insert::Item { kind, name } => self.put_item(
                raw,
                index,
                Slot::Unit(Content::Item {
                    kind: kind.into(),
                    name: name.into(),
                }),
                true,
            ),
        }
        Ok(())
    }

    pub fn delete(&mut self, at: usize, len: usize) -> Result<(), FlowError> {
        if len == 0 {
            return Ok(());
        }
        let (mut raw, first) = place(&self.slots, at)?;
        // The boundaries between the units stay, so each stretch of units is one removal.
        let mut removals: Vec<(u32, u32)> = Vec::new();
        let (mut end, mut left, mut adjoins) = (first, len, false);
        while left > 0 {
            let slot = (self.slots.get(end)).ok_or_else(|| corrupt("deletion passes the end"))?;
            let width = slot.width();
            let unit = matches!(slot, Slot::Unit(_));
            if unit {
                match removals.last_mut() {
                    Some((_, len)) if adjoins => *len += width,
                    _ => removals.push((raw, width)),
                }
                left -= 1;
            }
            adjoins = unit;
            raw += width;
            end += 1;
        }
        self.reading.take();
        self.length -= len;
        let kept: Vec<Slot> = (self.slots[first..end].iter())
            .filter(|slot| !matches!(slot, Slot::Unit(_)))
            .cloned()
            .collect();
        self.slots.splice(first..end, kept);
        // Each position was read before any removal, so the later stretch goes first.
        for (at, len) in removals.into_iter().rev() {
            self.writes.push(Write {
                strand: String::new(),
                tracked: true,
                action: Action::Delete { at, len },
            });
        }
        Ok(())
    }

    pub fn range(&mut self, at: usize, len: usize, stated: Stated) -> Result<(), FlowError> {
        self.range_at_turn(at, len, stated, self.turn() + 1)
    }
    pub(crate) fn range_at_turn(
        &mut self,
        at: usize,
        len: usize,
        stated: Stated,
        turn: u64,
    ) -> Result<(), FlowError> {
        let (from, first) = place(&self.slots, at)?;
        // A collapsed range has both boundaries at the same encoded gap.
        // `after(at)` can precede `place(at)` across existing boundaries.
        let (to, last) = if len == 0 {
            (from, first)
        } else {
            after(&self.slots, at + len)?
        };
        let (open, close) = self.pair_at(stated, turn);
        self.put_item(to, last, close, true);
        self.put_item(from, first, open, true);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deleted_range_is_one_removal_for_each_stretch_between_boundaries() {
        let seeded = |text| Seeded::Text {
            text,
            stated: vec![("basis", Some("r"))],
        };
        let mut flow = Flow::seeded(0, &[seeded("a🙂b"), seeded("cd")]);
        // Open a🙂b Close Open cd Close: the two boundaries between b and c stay.
        let before = flow.slots().to_vec();
        flow.delete(1, 3).unwrap();
        let removed: Vec<_> = (flow.take_writes().into_iter())
            .map(|write| match write.action {
                Action::Delete { at, len } => (at, len),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(removed, [(7, 1), (2, 3)]);
        let kept: Vec<_> = [0, 1, 4, 5, 7, 8].map(|i| before[i].clone()).into();
        assert_eq!(flow.slots(), kept);
        assert_eq!(flow.len(), 2);
        assert!(flow.delete(1, 2).is_err());
    }

    #[test]
    fn a_headless_typing_history_keeps_only_the_inserted_slots_on_redo() {
        let mut flow = Flow::seeded(0, &[]);
        let mut history = Vec::new();
        for i in 0..256 {
            let before = flow.clone();
            let birth = i.to_string();
            flow.insert_at(i, Insert::Text("🙂"), &[("birth", Some(&birth))], "birth")
                .unwrap();
            flow.take_writes();
            history.push(FlowUndo::between(&before, &flow));
        }
        let typed = flow.slots().to_vec();
        assert!(history
            .iter()
            .all(|edit| edit.slots.is_empty() && edit.replaced == 3));
        for edit in history.iter_mut().rev() {
            edit.swap(&mut flow);
        }
        assert!(flow.slots().is_empty());
        assert_eq!(
            history.iter().map(|edit| edit.slots.len()).sum::<usize>(),
            typed.len()
        );
        for edit in &mut history {
            edit.swap(&mut flow);
        }
        assert_eq!(flow.slots(), typed);
    }
}
