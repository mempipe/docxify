//! Semantic text strands in Yrs, with no Yrs range formats (spec R4–R6).
//!
//! Each strand is one `Y.Text`. Text is plain characters. An item, such as a
//! paragraph mark, is an embed. What is over a range is a *pair* of embeds
//! around it: one where it starts, with a key, a value and a count, and one
//! where it ends. Deleting content never removes half of a pair, so content
//! that is left and content another replica is putting keep what the pair
//! gives them.
//!
//! The store knows keys and values and not what they mean. A reader gets
//! every pair over a unit, earliest first, and decides by the kind of key:
//! the latest for a key with one value at a time, all of them for a key
//! that holds several. `tests/contract.rs` runs the collaboration contract
//! against it.
//!
//! Positions count units: a character or an item. Yrs counts UTF-16 units,
//! and the halves of pairs take one each. `place` is where one becomes the
//! other.
//!
//! Changed strands invalidate their cached native readings. Untouched
//! strands share their readings with command preparation and the view.

// Ephemeral awareness tokens: tag 0 + Yrs sticky index, or tag 1 + UTF-8
// stable cell key. A new cell has a paragraph before its first Yrs item.
const ITEM_CURSOR: u8 = 0;
const EMPTY_CELL_CURSOR: u8 = 1;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::Mutex;
use yrs::Observable;

use yrs::types::text::YChange;
use yrs::undo::Options as UndoOptions;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{
    Any, Assoc, ClientID, Doc, IndexedSequence, Map, MapRef, OffsetKind, Options, Origin, Out,
    ReadTxn, StateVector, StickyIndex, Text, TextRef, Transact, UndoManager, Update,
};

use crate::CollabError;

const LOCAL: &str = "local";
const FLOW_ROOT: &str = "flow";
const TEXT_ROOT: &str = "text/";
/// A pair's opening half: its name, and with `KEY`, `VALUE` and `TURN` what
/// it gives.
const OPEN: &str = "s";
/// A pair's closing half: the name of the pair it closes.
const CLOSE: &str = "e";
const KEY: &str = "k";
const VALUE: &str = "v";
const TURN: &str = "t";
/// An item: its kind, and with `NAME` what it names.
const ITEM: &str = "item";
const NAME: &str = "name";

use docx_engine::flow::{Action, Fact, Flow, Slot, Span, Write};
pub use docx_engine::flow::{Content, Seeded, Stated, Unit};
use docx_engine::state::{Plain, State, Strand};

fn corrupt(reason: impl Into<String>) -> CollabError {
    CollabError::Corrupt {
        flow: "store".into(),
        reason: reason.into(),
    }
}

fn map(entries: &[(&str, Any)]) -> Any {
    let entries = entries
        .iter()
        .map(|(key, value)| (key.to_string(), value.clone()));
    Any::from(HashMap::from_iter(entries))
}

fn text(value: Option<&Any>) -> Option<String> {
    match value {
        Some(Any::String(text)) => Some(text.to_string()),
        _ => None,
    }
}

fn slot_of(embed: &HashMap<String, Any>) -> Result<Slot, CollabError> {
    let named = |key: &str| text(embed.get(key));
    if let Some(name) = named(OPEN) {
        let turn = named(TURN).and_then(|turn| turn.parse().ok());
        let (Some(key), Some(turn)) = (named(KEY), turn) else {
            return Err(corrupt("a pair's start has no key or no count"));
        };
        let value = named(VALUE);
        return Ok(Slot::Open(name, Span { key, value, turn }));
    }
    if let Some(name) = named(CLOSE) {
        return Ok(Slot::Close(name));
    }
    match (named(ITEM), named(NAME)) {
        (Some(kind), Some(name)) => Ok(Slot::Unit(Content::Item { kind, name })),
        _ => Err(corrupt(format!("unknown embed {embed:?}"))),
    }
}

type StrandDefinitions = Arc<[(String, u64, Option<Slot>)]>;

/// One replica's copy of one flow.
pub struct Store {
    doc: Doc,
    strands: MapRef,
    definitions: Arc<Mutex<Option<StrandDefinitions>>>,
    meta: MapRef,
    facts: MapRef,
    undo: UndoManager,
    roots: RefCell<HashMap<String, TextRef>>,
    cached: RefCell<HashMap<String, Arc<Flow>>>,
    changed: Arc<Mutex<HashSet<String>>>,
}

impl Store {
    pub(crate) fn observe_local_updates(&self, updates: Arc<Mutex<Vec<Vec<u8>>>>) {
        self.doc
            .observe_update_v1("outgoing", move |txn, event| {
                if txn.origin() != Some(&Origin::from("remote")) {
                    updates.lock().unwrap().push(event.update.clone());
                }
            })
            .expect("no transaction is open when a replica starts");
    }

    pub(crate) fn stop_local_updates(&self) {
        self.doc
            .unobserve_update_v1("outgoing")
            .expect("no transaction is open");
    }

    pub(crate) fn local_clock(&self) -> u32 {
        self.doc
            .transact()
            .state_vector()
            .get(&self.doc.client_id())
    }

    /// A private, CRDT-free preparation state. Its clock is the next ID
    /// this replica will write; the caller keeps exclusive access through
    /// preparation and commit.
    pub fn snapshot(&self) -> Result<State, CollabError> {
        {
            let mut cache = self.cached.borrow_mut();
            for key in self.changed.lock().unwrap().drain() {
                cache.remove(&key);
            }
        }
        let definitions = self.strand_defs()?;
        let roots: Vec<_> = definitions
            .iter()
            .map(|(key, _, _)| self.text(key))
            .collect();
        let client = self.doc.client_id();
        let clock = self.doc.transact().state_vector().get(&client);
        let txn = self.doc.transact();
        let mut facts = std::collections::BTreeMap::<String, Fact>::new();
        for (identity, value) in self.facts.iter(&txn) {
            let Out::Any(Any::Map(fields)) = value else {
                return Err(corrupt("invalid fact"));
            };
            let (
                Some(Any::Buffer(bytes)),
                Some(Any::Bool(permanent)),
                Some(Any::String(key)),
                Some(Any::String(turn)),
            ) = (
                fields.get("bytes"),
                fields.get("permanent"),
                fields.get("key"),
                fields.get("turn"),
            )
            else {
                return Err(corrupt("invalid fact payload"));
            };
            let turn = turn
                .parse::<u64>()
                .map_err(|_| corrupt("invalid fact turn"))?;
            let value = Fact {
                identity: identity.to_owned(),
                turn,
                bytes: bytes.clone(),
                permanent: *permanent,
            };
            if facts
                .get(key.as_ref())
                .is_none_or(|old| (old.turn, old.identity.as_str()) < (turn, identity))
            {
                facts.insert(key.to_string(), value);
            }
        }
        let mut strands = Vec::new();
        for ((key, order, prefix), root) in definitions.iter().cloned().zip(roots) {
            let mut cache = self.cached.borrow_mut();
            let flow = if let Some(flow) = cache.get(&key) {
                flow.clone()
            } else {
                let flow = Arc::new(Flow::from_slots(
                    client.get(),
                    clock,
                    Self::slots(&root, &txn)?,
                )?);
                cache.insert(key.clone(), flow.clone());
                flow
            };
            strands.push(Strand {
                key,
                order,
                prefix,
                flow,
                declared: true,
            });
        }
        Ok(State::new(client.get(), clock, strands, facts))
    }

    fn text(&self, key: &str) -> TextRef {
        let mut roots = self.roots.borrow_mut();
        if let Some(root) = roots.get(key) {
            return root.clone();
        }
        let root = self.doc.get_or_insert_text(if key.is_empty() {
            FLOW_ROOT.to_owned()
        } else {
            format!("{TEXT_ROOT}{key}")
        });
        let changed = self.changed.clone();
        let name = key.to_owned();
        root.observe("projection-cache", move |_, _| {
            changed.lock().unwrap().insert(name.clone());
        });
        roots.insert(key.to_owned(), root.clone());
        root
    }
    fn strand_defs(&self) -> Result<StrandDefinitions, CollabError> {
        if let Some(definitions) = self.definitions.lock().unwrap().as_ref() {
            return Ok(definitions.clone());
        }
        let txn = self.doc.transact();
        let mut out = Vec::new();
        for (key, value) in self.strands.iter(&txn) {
            let Out::Any(Any::Map(fields)) = value else {
                return Err(corrupt("invalid strand"));
            };
            let Some(order) = text(fields.get("order")).and_then(|s| s.parse().ok()) else {
                return Err(corrupt("invalid strand order"));
            };
            let prefix = match fields.get("prefix") {
                None | Some(Any::Null) => None,
                Some(Any::Map(embed)) => Some(slot_of(embed)?),
                _ => return Err(corrupt("invalid strand boundary")),
            };
            out.push((key.to_owned(), order, prefix));
        }
        if out.is_empty() {
            out.push((String::new(), 0, None));
        }
        out.sort_by(|a, b| (a.1, &a.0).cmp(&(b.1, &b.0)));
        let definitions = StrandDefinitions::from(out);
        *self.definitions.lock().unwrap() = Some(definitions.clone());
        Ok(definitions)
    }
    pub(crate) fn scope_strands(&mut self) -> Result<(), CollabError> {
        for (key, _, _) in self.strand_defs()?.iter() {
            let flow = self.text(key);
            self.undo.expand_scope(&self.doc, &flow);
        }
        Ok(())
    }
    pub(crate) fn sticky(
        &self,
        state: &docx_engine::editing::PositionState,
        at: usize,
    ) -> Result<Option<Vec<u8>>, CollabError> {
        let (key, raw) = state.raw_at(at)?;
        if state
            .strands()
            .iter()
            .any(|s| s.key == key && !s.declared && key.starts_with(docx_engine::editing::CELL))
        {
            return Ok(Some(
                std::iter::once(EMPTY_CELL_CURSOR)
                    .chain(key.bytes())
                    .collect(),
            ));
        }
        let flow = self.text(&key);
        let txn = self.doc.transact();
        let assoc = if raw == flow.len(&txn) {
            Assoc::Before
        } else {
            Assoc::After
        };
        Ok(flow.sticky_index(&txn, raw, assoc).map(|index| {
            std::iter::once(ITEM_CURSOR)
                .chain(index.encode_v1())
                .collect()
        }))
    }
    pub(crate) fn resolve(
        &self,
        state: &docx_engine::editing::PositionState,
        token: &[u8],
    ) -> Result<Option<usize>, CollabError> {
        let token = match token.split_first() {
            Some((&EMPTY_CELL_CURSOR, key)) => {
                // Stay at the initial paragraph's start when typing begins.
                // Hidden or removed cells do not resolve.
                return Ok(std::str::from_utf8(key)
                    .ok()
                    .and_then(|key| state.unit_at(key, 1)));
            }
            Some((&ITEM_CURSOR, bytes)) => bytes,
            _ => return Ok(None),
        };
        let Ok(index) = StickyIndex::decode_v1(token) else {
            return Ok(None);
        };
        let txn = self.doc.transact();
        let Some(offset) = index.get_offset(&txn) else {
            return Ok(None);
        };
        // A strand's text is a root that `text` names after the strand.
        let yrs::branch::BranchID::Root(name) = offset.branch.id() else {
            return Ok(None);
        };
        let key = match &*name {
            FLOW_ROOT => "",
            name => match name.strip_prefix(TEXT_ROOT) {
                Some(key) => key,
                None => return Ok(None),
            },
        };
        Ok(state.unit_at(key, offset.index))
    }

    fn embed(slot: &Slot) -> Any {
        match slot {
            Slot::Open(name, span) => {
                let mut entries = vec![
                    (OPEN, Any::from(name.as_str())),
                    (KEY, Any::from(span.key.as_str())),
                    (TURN, Any::from(span.turn.to_string())),
                ];
                entries.extend(span.value.as_deref().map(|value| (VALUE, Any::from(value))));
                map(&entries)
            }
            Slot::Close(name) => map(&[(CLOSE, Any::from(name.as_str()))]),
            Slot::Unit(Content::Item { kind, name }) => map(&[
                (ITEM, Any::from(kind.as_str())),
                (NAME, Any::from(name.as_str())),
            ]),
            Slot::Unit(Content::Char(_)) => unreachable!("characters use text writes"),
        }
    }

    /// Commit already-validated positions without rerunning the commands.
    /// Adjacent writes with one origin share a transaction, so both halves
    /// of a range are atomic. Insertion bases remain outside local undo.
    pub(crate) fn commit_prepared(&mut self, writes: Vec<Write>) {
        for group in writes.chunk_by(|a, b| a.tracked == b.tracked) {
            let roots: Vec<_> = group.iter().map(|w| self.text(&w.strand)).collect();
            for root in &roots {
                self.undo.expand_scope(&self.doc, root);
            }
            let mut txn = if group[0].tracked {
                self.doc.transact_mut_with(LOCAL)
            } else {
                self.doc.transact_mut()
            };
            for (write, flow) in group.iter().zip(roots) {
                match &write.action {
                    Action::Text { at, text } => flow.insert(&mut txn, *at, text),
                    Action::Item { at, slot } => {
                        flow.insert_embed(&mut txn, *at, Self::embed(slot));
                    }
                    Action::Delete { at, len } => flow.remove_range(&mut txn, *at, *len),
                    Action::Declare { key, order, prefix } => {
                        self.strands.insert(
                            &mut txn,
                            key.as_str(),
                            map(&[
                                ("order", Any::String(order.to_string().into())),
                                (
                                    "prefix",
                                    prefix
                                        .as_ref()
                                        .map(|c| Self::embed(&Slot::Unit(c.clone())))
                                        .unwrap_or(Any::Null),
                                ),
                            ]),
                        );
                    }
                    Action::Fact { key, value } => {
                        self.facts.insert(
                            &mut txn,
                            value.identity.as_str(),
                            map(&[
                                ("key", Any::String(key.as_str().into())),
                                ("turn", Any::String(value.turn.to_string().into())),
                                ("bytes", Any::Buffer(value.bytes.clone())),
                                ("permanent", Any::Bool(value.permanent)),
                            ]),
                        );
                    }
                }
            }
        }
    }

    pub(crate) fn from_seed(client: u64, meta: &[(&str, &str)], state: &State) -> Store {
        let mut store = Store::new(client);
        let roots: Vec<_> = state.strands().iter().map(|s| store.text(&s.key)).collect();
        let mut txn = store.doc.transact_mut();
        for (strand, root) in state.strands().iter().zip(roots) {
            let mut raw = 0;
            let mut text = String::new();
            for slot in strand.flow.slots() {
                if let Slot::Unit(Content::Char(c)) = slot {
                    text.push(*c);
                    continue;
                }
                if !text.is_empty() {
                    root.insert(&mut txn, raw, &text);
                    raw += text.encode_utf16().count() as u32;
                    text.clear();
                }
                root.insert_embed(&mut txn, raw, Self::embed(slot));
                raw += 1;
            }
            if !text.is_empty() {
                root.insert(&mut txn, raw, &text);
            }
            store.strands.insert(
                &mut txn,
                strand.key.as_str(),
                map(&[
                    ("order", Any::String(strand.order.to_string().into())),
                    (
                        "prefix",
                        strand.prefix.as_ref().map(Self::embed).unwrap_or(Any::Null),
                    ),
                ]),
            );
        }
        for (key, value) in state.facts() {
            store.facts.insert(
                &mut txn,
                value.identity.as_str(),
                map(&[
                    ("key", Any::String(key.as_str().into())),
                    ("turn", Any::String(value.turn.to_string().into())),
                    ("bytes", Any::Buffer(value.bytes.clone())),
                    ("permanent", Any::Bool(value.permanent)),
                ]),
            );
        }
        for (key, value) in meta {
            store.meta.insert(&mut txn, *key, *value);
        }
        drop(txn);
        store.scope_strands().expect("seed has valid strands");
        store
    }

    /// An empty flow, written to as Yrs client `client`.
    pub fn new(client: u64) -> Store {
        let doc = Doc::with_options(Options {
            client_id: ClientID::new(client),
            offset_kind: OffsetKind::Utf16,
            ..Options::default()
        });
        let flow = doc.get_or_insert_text(FLOW_ROOT);
        let strands = doc.get_or_insert_map("strands");
        let meta = doc.get_or_insert_map("meta");
        let facts = doc.get_or_insert_map("facts");
        // A step ends where `step` says, never because time passed, so no
        // clock is read. Yrs takes a time of zero to mean a step has ended,
        // so the time it is given is always one.
        let mut undo = UndoManager::with_options(UndoOptions {
            capture_timeout_millis: u64::MAX,
            tracked_origins: Default::default(),
            capture_transaction: None,
            timestamp: Arc::new(|| 1),
            init_undo_stack: Vec::new(),
            init_redo_stack: Vec::new(),
        });
        undo.expand_scope(&doc, &flow);
        undo.expand_scope(&doc, &facts);
        undo.include_origin(Origin::from(LOCAL));
        let definitions = Arc::new(Mutex::new(None));
        let cache = definitions.clone();
        strands.observe("strand-definitions", move |_, _| {
            cache.lock().unwrap().take();
        });
        Store {
            doc,
            strands,
            definitions,
            meta,
            facts,
            undo,
            roots: Default::default(),
            cached: Default::default(),
            changed: Default::default(),
        }
    }

    /// A flow opened from what `save` gave.
    pub fn load(client: u64, saved: &[u8]) -> Result<Store, CollabError> {
        let mut store = Self::from_update(client, saved)?;
        store.scope_strands()?;
        Ok(store)
    }

    pub(crate) fn from_update(client: u64, saved: &[u8]) -> Result<Self, CollabError> {
        let mut store = Store::new(client);
        store.receive(saved)?;
        Ok(store)
    }

    /// What the flow says of itself under `key`.
    pub fn meta(&self, key: &str) -> Option<String> {
        match self.meta.get(&self.doc.transact(), key) {
            Some(Out::Any(Any::String(value))) => Some(value.to_string()),
            _ => None,
        }
    }

    /// The whole state, which is also an update for a replica with nothing.
    pub fn save(&self) -> Vec<u8> {
        (self.doc.transact()).encode_state_as_update_v1(&StateVector::default())
    }

    pub fn state_vector(&self) -> Vec<u8> {
        self.doc.transact().state_vector().encode_v1()
    }

    /// What this replica has that a replica with `state_vector` has not.
    pub fn update_since(&self, state_vector: &[u8]) -> Result<Vec<u8>, CollabError> {
        let known = StateVector::decode_v1(state_vector);
        let known = known.map_err(|e| CollabError::Update(e.to_string()))?;
        Ok(self.doc.transact().encode_state_as_update_v1(&known))
    }

    pub fn apply_update(&mut self, update: &[u8]) -> Result<(), CollabError> {
        self.receive(update)?;
        self.scope_strands()
    }

    fn receive(&mut self, update: &[u8]) -> Result<(), CollabError> {
        let update = Update::decode_v1(update).map_err(|e| CollabError::Update(e.to_string()))?;
        let applied = self.doc.transact_mut_with("remote").apply_update(update);
        applied.map_err(|e| CollabError::Update(e.to_string()))
    }

    fn slots(flow: &TextRef, txn: &impl ReadTxn) -> Result<Vec<Slot>, CollabError> {
        let mut slots = Vec::new();
        for chunk in flow.diff(txn, YChange::identity) {
            match &chunk.insert {
                Out::Any(Any::String(text)) => {
                    slots.extend(text.chars().map(|c| Slot::Unit(Content::Char(c))))
                }
                Out::Any(Any::Map(embed)) => slots.push(slot_of(embed)?),
                other => return Err(corrupt(format!("the flow holds {other:?}"))),
            }
        }
        Ok(slots)
    }

    /// Every unit, with the pairs over it.
    pub fn read(&self) -> Result<Vec<Unit>, CollabError> {
        Ok(self.snapshot()?.read(&Plain)?)
    }

    pub fn insert(&mut self, at: usize, text: &str, stated: &[Stated]) -> Result<(), CollabError> {
        let mut flow = self.snapshot()?;
        let (client, clock) = flow.next_identity();
        let generation = format!("{client}.{clock}");
        let mut stated = stated.to_vec();
        stated.push((docx_engine::editing::BIRTH, Some(&generation)));
        flow.insert(&Plain, at, text, &stated, docx_engine::editing::BIRTH)?;
        self.commit_prepared(flow.take_writes());
        Ok(())
    }
    pub fn item(&mut self, at: usize, kind: &str, name: &str) -> Result<(), CollabError> {
        let mut flow = self.snapshot()?;
        flow.item(&Plain, at, kind, name)?;
        self.commit_prepared(flow.take_writes());
        Ok(())
    }
    pub fn delete(&mut self, at: usize, len: usize) -> Result<(), CollabError> {
        let mut flow = self.snapshot()?;
        flow.delete(&Plain, at, len)?;
        self.commit_prepared(flow.take_writes());
        Ok(())
    }
    pub fn range(&mut self, at: usize, len: usize, stated: Stated) -> Result<(), CollabError> {
        let mut flow = self.snapshot()?;
        flow.range(&Plain, at, len, stated)?;
        self.commit_prepared(flow.take_writes());
        Ok(())
    }

    /// Ends a step: the edits since the last one are one change, which one
    /// undo takes back.
    pub fn step(&mut self) {
        self.undo.reset();
    }

    /// Takes back this replica's last step. Returns whether there was one.
    pub fn undo(&mut self) -> bool {
        self.undo.undo_blocking()
    }

    /// Puts back the step this replica last took back.
    pub fn redo(&mut self) -> bool {
        self.undo.redo_blocking()
    }
}
