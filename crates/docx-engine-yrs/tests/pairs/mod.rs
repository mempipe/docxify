//! The crate's store (`docx_engine_yrs::store`), which holds ranges as pairs
//! of items in a Yrs text, for the contract tests. This file
//! says what the keys mean: a basis, a tracked insertion and bold have one
//! value at a time, and comments hold several.

use docx_engine_yrs::store::{Content, Store};

use super::Unit;

const BASIS: &str = "basis";
const REVISION: &str = "rev";
const BOLD: &str = "bold";
const COMMENT: &str = "comment";

pub struct Peer {
    client: u64,
    store: Store,
}

impl Peer {
    pub(super) fn new(peer: u8) -> Peer {
        let client = u64::from(peer);
        let store = Store::new(client);
        Peer { client, store }
    }

    pub(super) fn reopen(self) -> Peer {
        let store = Store::load(self.client, &self.store.save()).unwrap();
        Peer { store, ..self }
    }

    pub(super) fn insert(
        &mut self,
        at: usize,
        text: &str,
        basis: &str,
        revision: Option<&str>,
        bold: Option<&str>,
    ) {
        let stated = [(BASIS, Some(basis)), (BOLD, bold), (REVISION, revision)];
        self.store.insert(at, text, &stated).unwrap();
        self.store.step();
    }

    pub(super) fn enter(&mut self, at: usize) {
        self.store.item(at, "p", "").unwrap();
        self.store.step();
    }

    pub(super) fn delete(&mut self, at: usize, len: usize) {
        self.store.delete(at, len).unwrap();
        self.store.step();
    }

    pub(super) fn bold(&mut self, at: usize, len: usize, value: Option<&str>) {
        self.store.range(at, len, (BOLD, value)).unwrap();
        self.store.step();
    }

    pub(super) fn comment(&mut self, at: usize, len: usize, name: &str) {
        self.store.range(at, len, (COMMENT, Some(name))).unwrap();
        self.store.step();
    }

    pub(super) fn undo(&mut self) {
        self.store.undo();
    }

    pub(super) fn read(&self) -> Vec<Unit> {
        let units = self.store.read().unwrap().into_iter();
        let unit = |unit: docx_engine_yrs::store::Unit| {
            let c = match unit.content {
                Content::Char(c) => Some(c),
                Content::Item { .. } => None,
            };
            let one = |key| unit.one(key).filter(|_| c.is_some()).map(str::to_string);
            Unit {
                c,
                basis: one(BASIS),
                revision: one(REVISION),
                bold: unit.one(BOLD).map(str::to_string),
                comments: unit.all(COMMENT).into_iter().map(str::to_string).collect(),
            }
        };
        units.map(unit).collect()
    }

    pub(super) fn sync(a: &mut Peer, b: &mut Peer, a_first: bool) {
        let to_a = b.store.update_since(&a.store.state_vector()).unwrap();
        let to_b = a.store.update_since(&b.store.state_vector()).unwrap();
        let (first, then) = match a_first {
            true => ((a, to_a), (b, to_b)),
            false => ((b, to_b), (a, to_a)),
        };
        first.0.store.apply_update(&first.1).unwrap();
        then.0.store.apply_update(&then.1).unwrap();
    }
}

#[test]
fn competing_updates_arrive_in_every_order_at_one_receiver() {
    use super::{check, seeded, Edit, Part, PARTS};
    let edits = [
        Edit::Insert(0),
        Edit::Insert(2),
        Edit::Delete(0, 2),
        Edit::Delete(2, 2),
        Edit::Bold(1, 3, "on"),
        Edit::Bold(2, 2, "off"),
        Edit::Enter(2),
    ];
    let orders = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    for a in edits {
        for b in edits {
            for c in edits {
                let mut peers = seeded(3);
                let base = peers[0].store.save();
                let vector = peers[0].store.state_vector();
                let parts: Vec<_> = [a, b, c]
                    .into_iter()
                    .zip(PARTS)
                    .map(|(edit, part)| Part { edit, ..part })
                    .collect();
                for (part, peer) in parts.iter().zip(&mut peers) {
                    part.apply(peer);
                }
                let updates: Vec<_> = peers
                    .iter()
                    .map(|peer| peer.store.update_since(&vector).unwrap())
                    .collect();
                let mut agreed = None;
                for order in orders {
                    let mut receiver = Peer {
                        client: 99,
                        store: Store::load(99, &base).unwrap(),
                    };
                    for index in order {
                        receiver.store.apply_update(&updates[index]).unwrap();
                        receiver.store.apply_update(&updates[index]).unwrap();
                        receiver.read(); // Every prefix is usable, including duplicates.
                    }
                    let shown = receiver.read();
                    check(&shown, &parts, None).unwrap();
                    if let Some(expected) = &agreed {
                        assert_eq!(expected, &shown, "{a:?} {b:?} {c:?}: {order:?}");
                    }
                    agreed = Some(shown);
                }
            }
        }
    }
}

#[test]
fn a_range_deleted_across_boundaries_and_emoji_converges_and_undoes() {
    let mut writer = Peer::new(1);
    writer.insert(0, "a🦀bc🙂d", "R1", None, None);
    writer.bold(2, 2, Some("on"));
    let mut reader = Peer {
        client: 2,
        store: Store::load(2, &writer.store.save()).unwrap(),
    };
    let before = writer.read();
    writer.delete(1, 4);
    Peer::sync(&mut writer, &mut reader, true);
    let left: String = reader.read().iter().filter_map(|unit| unit.c).collect();
    assert_eq!(left, "ad");
    assert_eq!(reader.read(), writer.read());
    writer.undo();
    Peer::sync(&mut writer, &mut reader, true);
    assert_eq!(writer.read(), before);
    assert_eq!(reader.read(), before);
}

#[test]
fn dependent_updates_can_arrive_before_their_prerequisites() {
    let mut writer = Peer::new(1);
    writer.insert(0, "a🦀b", "R1", None, None);
    let initial = writer.store.save();
    let before = writer.store.state_vector();
    writer.insert(2, "🙂", "R2", Some("author"), None);
    let first = writer.store.update_since(&before).unwrap();
    let before = writer.store.state_vector();
    writer.bold(1, 2, Some("on"));
    writer.delete(0, 1);
    let second = writer.store.update_since(&before).unwrap();
    let mut receiver = Peer {
        client: 2,
        store: Store::load(2, &initial).unwrap(),
    };
    receiver.store.apply_update(&second).unwrap();
    receiver.read();
    receiver.store.apply_update(&first).unwrap();
    receiver.store.apply_update(&second).unwrap();
    assert_eq!(receiver.read(), writer.read());
    writer.undo();
    Peer::sync(&mut writer, &mut receiver, true);
    assert_eq!(receiver.read(), writer.read());
}
