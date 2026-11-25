//! Bounded command/encoding/materialization check: 240 seeded histories,
//! three authors, up to six attempted edits each, all six delivery orders, duplicate
//! updates. The plain source makes formatting explicit. This is not an
//! exhaustive oracle for arbitrary editing histories or DOCX structures.
#[path = "../../docx-engine/tests/common/mod.rs"]
mod common;
use docx_engine::{Command, Document, Format, NodeKind, Point, Track};
use docx_engine_yrs::{paragraphs, store::Store, Inline, Replica};

#[derive(Debug, PartialEq, Eq)]
struct Seen {
    c: char,
    bold: bool,
    italic: bool,
    underline: bool,
    insertion: Option<String>,
    deletion: Option<String>,
}
fn shown(doc: &Document) -> Vec<Vec<Seen>> {
    doc.paragraphs()
        .into_iter()
        .map(|p| {
            doc.descendants(p)
                .into_iter()
                .flat_map(|id| {
                    let n = doc.node(id).unwrap();
                    let text = match &n.kind {
                        NodeKind::Text { text } | NodeKind::DeletedText { text } => text,
                        _ => return vec![],
                    };
                    let (mut bold, mut italic, mut underline) = (false, false, false);
                    let (mut insertion, mut deletion) = (None, None);
                    let mut parent = n.parent;
                    while let Some(id) = parent {
                        let n = doc.node(id).unwrap();
                        match &n.kind {
                            NodeKind::Run(r) => {
                                bold = r.props.bold.unwrap_or(false);
                                italic = r.props.italic.unwrap_or(false);
                                underline = r.props.underline.as_ref().is_some_and(|s| s != "none");
                            }
                            NodeKind::Revision(r) if r.change == "ins" => {
                                insertion = r.author.clone()
                            }
                            NodeKind::Revision(r) if r.change == "del" => {
                                deletion = r.author.clone()
                            }
                            _ => {}
                        }
                        parent = n.parent;
                    }
                    text.chars()
                        .map(|c| Seen {
                            c,
                            bold,
                            italic,
                            underline,
                            insertion: insertion.clone(),
                            deletion: deletion.clone(),
                        })
                        .collect::<Vec<_>>()
                })
                .collect()
        })
        .collect()
}
fn check(d: &mut Replica, seed: u64, base: &Document) {
    let store = Store::load(999, &d.save()).unwrap();
    let wanted: Vec<Vec<Seen>> = paragraphs(base, &store)
        .unwrap()
        .into_iter()
        .map(|p| {
            p.text
                .into_iter()
                .filter_map(|i| match i {
                    Inline::Char(c) => Some(Seen {
                        c: c.c,
                        bold: c.format.bold.unwrap_or(false),
                        italic: c.format.italic.unwrap_or(false),
                        underline: c.format.underline.unwrap_or(false),
                        insertion: c.revision.map(|r| r.author),
                        deletion: c.deletion.map(|r| r.author),
                    }),
                    Inline::HiddenBoundary => None,
                    _ => panic!("unexpected non-text item"),
                })
                .collect()
        })
        .collect();
    assert_eq!(
        shown(d.view().unwrap()),
        wanted,
        "seed {seed}: materialized meaning"
    );
    let fresh = d.export().unwrap();
    let live = d.view().unwrap().export().unwrap();
    assert!(
        live == fresh,
        "seed {seed}: live and fresh DOCX exports differ"
    );
    assert_eq!(
        shown(&Document::open(&fresh).unwrap()),
        wanted,
        "seed {seed}: fresh meaning"
    );
    assert!(
        docx_engine::validate_package(&fresh).valid,
        "seed {seed}: invalid export"
    );
}
fn next(r: &mut u64, limit: usize) -> usize {
    *r ^= *r << 13;
    *r ^= *r >> 7;
    *r ^= *r << 17;
    (*r as usize) % limit
}
fn edit(d: &mut Replica, r: &mut u64, author: &str, seed: u64) {
    let action = next(r, 10);
    if action == 8 {
        d.undo().unwrap();
        return;
    }
    if action == 9 {
        d.redo().unwrap();
        return;
    }
    let doc = d.view().unwrap();
    let ps = doc.paragraphs();
    let p = ps[next(r, ps.len())];
    let texts: Vec<_> = doc
        .descendants(p)
        .into_iter()
        .filter_map(|id| match &doc.node(id).unwrap().kind {
            NodeKind::Text { text } => Some((id, text.chars().count())),
            _ => None,
        })
        .collect();
    let (from, to) = if texts.is_empty() {
        (Point::Start { start: p }, Point::Start { start: p })
    } else {
        let (node, len) = texts[next(r, texts.len())];
        let start = next(r, len + 1);
        let end = start + next(r, len - start + 1);
        (
            Point::Text {
                node,
                offset: start,
            },
            Point::Text { node, offset: end },
        )
    };
    let track = Some(Track {
        author: author.into(),
        date: "2026-11-18T00:00:00Z".into(),
    });
    let command = match action {
        0 => Command::InsertText {
            at: from,
            text: "🦀".into(),
        },
        1 => Command::ReplaceRange {
            from,
            to,
            text: "x".into(),
            track: None,
        },
        2 => Command::SplitParagraph { at: from },
        3 if ps.last() != Some(&p) => Command::MergeParagraphs { paragraph: p },
        3 => Command::InsertText {
            at: from,
            text: "z".into(),
        },
        4..=6 => Command::FormatRange {
            from,
            to,
            format: Format {
                bold: (action == 4).then(|| next(r, 2) == 0),
                italic: (action == 5).then(|| next(r, 2) == 0),
                underline: (action == 6).then(|| next(r, 2) == 0),
            },
        },
        7 => Command::ReplaceRange {
            from,
            to,
            text: "T".into(),
            track,
        },
        _ => unreachable!(),
    };
    let before = d.save();
    if let Err(e) = d.apply(&command) {
        // Editing inside a tracked wrapper or across a tracked boundary
        // may be refused by policy. A refusal must leave no shared writes.
        assert!(before == d.save(), "seed {seed}: refusal wrote state: {e}");
        assert!(
            matches!(
                e,
                docx_engine_yrs::CollabError::Edit(_)
                    | docx_engine_yrs::CollabError::Unsupported { .. }
            ),
            "seed {seed}: unexpected refusal: {e}"
        );
    }
}
#[test]
fn three_authors_preserve_the_shared_meaning_in_every_delivery_order() {
    let bytes = common::with_body(
        "<w:p><w:r><w:t>abcdef</w:t></w:r></w:p><w:p><w:r><w:t>ghijkl</w:t></w:r></w:p>",
    );
    let initial = Replica::seed(100, &bytes).unwrap().save();
    let orders = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    let base = Document::open(&bytes).unwrap();
    for seed in 1..=240 {
        let mut peers: Vec<_> = (1..=3)
            .map(|id| Replica::join(id, &bytes, &initial).unwrap())
            .collect();
        let mut r = seed;
        for (i, peer) in peers.iter_mut().enumerate() {
            for _ in 0..6 {
                edit(peer, &mut r, &format!("Author{i}"), seed);
                check(peer, seed, &base);
            }
        }
        let updates: Vec<_> = peers.iter().map(Replica::save).collect();
        let mut expected = None;
        for order in orders {
            let mut receiver = Replica::join(90, &bytes, &initial).unwrap();
            for i in order {
                receiver.apply_update(&updates[i]).unwrap();
                receiver.apply_update(&updates[i]).unwrap();
                check(&mut receiver, seed, &base);
            }
            let export = receiver.export().unwrap();
            if let Some(ref expected) = expected {
                assert!(
                    expected == &export,
                    "seed {seed}: delivery order changed export"
                );
            } else {
                expected = Some(export);
            }
        }
    }
}
