//! Tracked deletion preserves document authorship independently of CRDT undo.
use docx_engine::{Command, Document, NodeId, NodeKind, Point, Track};
use docx_engine_yrs::Replica;

fn base() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/generated/simple-paragraphs.docx"
    ))
    .unwrap()
}
fn track(author: &str) -> Track {
    Track {
        author: author.into(),
        date: "2026-11-18T00:00:00Z".into(),
    }
}
fn first(doc: &Document, needle: &str) -> NodeId {
    doc.descendants(doc.stories().next().unwrap().id).into_iter().find(|id| {
        matches!(&doc.node(*id).unwrap().kind, NodeKind::Text { text } if text.contains(needle))
    }).unwrap()
}
fn replace(node: NodeId, a: usize, b: usize, text: &str, by: Option<&str>) -> Command {
    Command::ReplaceRange {
        from: Point::Text { node, offset: a },
        to: Point::Text { node, offset: b },
        text: text.into(),
        track: by.map(track),
    }
}
fn sync(a: &mut Replica, b: &mut Replica) {
    let to_a = b.update_since(&a.state_vector()).unwrap();
    let to_b = a.update_since(&b.state_vector()).unwrap();
    a.apply_update(&to_a).unwrap();
    b.apply_update(&to_b).unwrap();
    assert_eq!(a.export().unwrap(), b.export().unwrap());
}
#[derive(Debug, PartialEq, Eq)]
struct Seen {
    c: char,
    authors: Vec<(String, String)>,
}
fn seen(doc: &Document) -> Vec<Seen> {
    let p = doc.paragraphs()[0];
    doc.descendants(p)
        .into_iter()
        .flat_map(|id| {
            let node = doc.node(id).unwrap();
            let text = match &node.kind {
                NodeKind::Text { text } | NodeKind::DeletedText { text } => text,
                _ => return Vec::new(),
            };
            let mut at = node.parent;
            let mut authors = Vec::new();
            while let Some(id) = at {
                let n = doc.node(id).unwrap();
                if let NodeKind::Revision(r) = &n.kind {
                    authors.push((r.change.clone(), r.author.clone().unwrap()));
                }
                at = n.parent;
            }
            authors.reverse();
            text.chars()
                .map(|c| Seen {
                    c,
                    authors: authors.clone(),
                })
                .collect()
        })
        .collect()
}
fn live(doc: &Document) -> String {
    seen(doc)
        .into_iter()
        .filter(|c| c.authors.iter().all(|(kind, _)| kind != "del"))
        .map(|c| c.c)
        .collect()
}

#[test]
fn own_addition_cancels_but_anothers_keeps_both_authors_and_undo() {
    let bytes = base();
    let mut a = Replica::seed(10, &bytes).unwrap();
    let node = first(a.view().unwrap(), "First");
    a.apply(&replace(node, 0, 0, "blue", Some("Alice")))
        .unwrap();
    let node = first(a.view().unwrap(), "blue");
    let inserted = a.export().unwrap();
    a.apply(&replace(node, 0, 4, "", Some("Alice"))).unwrap();
    assert_eq!(live(a.view().unwrap()), "First paragraph.");
    assert!(seen(a.view().unwrap()).iter().all(|s| s.authors.is_empty()));
    a.undo().unwrap();
    assert_eq!(a.export().unwrap(), inserted);
    let mut b = Replica::join(20, &bytes, &a.save()).unwrap();
    let node = first(b.view().unwrap(), "blue");
    b.apply(&replace(node, 0, 4, "", Some("Bob"))).unwrap();
    sync(&mut a, &mut b);
    let expected = vec![("ins".into(), "Alice".into()), ("del".into(), "Bob".into())];
    let content = seen(a.view().unwrap());
    assert_eq!(
        content.iter().take(4).map(|s| s.c).collect::<String>(),
        "blue"
    );
    assert!(content[..4].iter().all(|s| s.authors == expected));
    assert_eq!(live(a.view().unwrap()), "First paragraph.");
    let exported = a.export().unwrap();
    assert!(docx_engine::validate_package(&exported).valid);
    assert_eq!(seen(&Document::open(&exported).unwrap()), content);
    let mut reopened = Replica::join(30, &bytes, &a.save()).unwrap();
    assert_eq!(seen(reopened.view().unwrap()), content);
    b.undo().unwrap();
    sync(&mut a, &mut b);
    assert_eq!(a.export().unwrap(), inserted);
    b.redo().unwrap();
    sync(&mut a, &mut b);
    assert_eq!(seen(a.view().unwrap()), content);
}

#[test]
fn deletion_does_not_capture_concurrent_or_later_insertions() {
    for order in [false, true] {
        let bytes = base();
        let mut a = Replica::seed(10, &bytes).unwrap();
        let node = first(a.view().unwrap(), "First");
        a.apply(&replace(node, 0, 0, "ABCDE", None)).unwrap();
        let saved = a.save();
        let mut b = Replica::join(20, &bytes, &saved).unwrap();
        let node = first(a.view().unwrap(), "ABCDE");
        a.apply(&replace(node, 1, 4, "", Some("Alice"))).unwrap();
        let bob_node = first(b.view().unwrap(), "ABCDE");
        b.apply(&replace(bob_node, 2, 2, "🦀", Some("Bob")))
            .unwrap();
        let mut receiver = Replica::join(30, &bytes, &saved).unwrap();
        let mut updates = [
            a.update_since(&receiver.state_vector()).unwrap(),
            b.update_since(&receiver.state_vector()).unwrap(),
        ];
        if order {
            updates.reverse();
        }
        for update in &updates {
            receiver.apply_update(update).unwrap();
        }
        sync(&mut a, &mut b);
        assert_eq!(receiver.export().unwrap(), a.export().unwrap());
        assert_eq!(live(a.view().unwrap()), "A🦀EFirst paragraph.");
        let crab = seen(a.view().unwrap())
            .into_iter()
            .find(|s| s.c == '🦀')
            .unwrap();
        assert_eq!(crab.authors, [("ins".into(), "Bob".into())]);
        let node = first(b.view().unwrap(), "🦀");
        b.apply(&replace(node, 1, 1, "!", None)).unwrap();
        sync(&mut a, &mut b);
        assert_eq!(live(a.view().unwrap()), "A🦀!EFirst paragraph.");
        assert!(seen(a.view().unwrap())
            .into_iter()
            .find(|s| s.c == '!')
            .unwrap()
            .authors
            .is_empty());
    }
}

#[test]
fn undo_restores_the_same_generation_under_an_active_deletion() {
    let bytes = base();
    let mut a = Replica::seed(10, &bytes).unwrap();
    let node = first(a.view().unwrap(), "First");
    a.apply(&replace(node, 0, 0, "blue", Some("Alice")))
        .unwrap();
    let mut b = Replica::join(20, &bytes, &a.save()).unwrap();
    let node = first(a.view().unwrap(), "blue");
    a.apply(&replace(node, 0, 4, "", None)).unwrap();
    b.apply(&replace(node, 0, 4, "", Some("Bob"))).unwrap();
    sync(&mut a, &mut b);
    a.undo().unwrap();
    sync(&mut a, &mut b);
    let restored = seen(a.view().unwrap());
    assert_eq!(
        restored.iter().take(4).map(|s| s.c).collect::<String>(),
        "blue"
    );
    assert!(restored[..4]
        .iter()
        .all(|s| s.authors == [("ins".into(), "Alice".into()), ("del".into(), "Bob".into())]));
    b.undo().unwrap();
    sync(&mut a, &mut b);
    assert_eq!(live(a.view().unwrap()), "blueFirst paragraph.");
}

#[test]
fn cancelling_own_insert_leaves_anothers_concurrent_insert() {
    let bytes = base();
    let mut a = Replica::seed(10, &bytes).unwrap();
    let node = first(a.view().unwrap(), "First");
    a.apply(&replace(node, 0, 0, "blue", Some("Alice")))
        .unwrap();
    let mut b = Replica::join(20, &bytes, &a.save()).unwrap();
    let node = first(a.view().unwrap(), "blue");
    a.apply(&replace(node, 0, 4, "", Some("Alice"))).unwrap();
    b.apply(&replace(node, 2, 2, "Q", Some("Bob"))).unwrap();
    sync(&mut a, &mut b);
    assert_eq!(live(a.view().unwrap()), "QFirst paragraph.");
    assert_eq!(
        seen(a.view().unwrap())[0].authors,
        [("ins".into(), "Bob".into())]
    );
}

#[test]
fn overlapping_deletions_keep_both_actions_and_undo_independently() {
    let bytes = base();
    let mut a = Replica::seed(10, &bytes).unwrap();
    let mut b = Replica::join(20, &bytes, &a.save()).unwrap();
    let node = first(a.view().unwrap(), "First");
    a.apply(&replace(node, 0, 4, "", Some("Alice"))).unwrap();
    b.apply(&replace(node, 2, 6, "", Some("Bob"))).unwrap();
    sync(&mut a, &mut b);
    assert_eq!(live(a.view().unwrap()), "paragraph.");
    assert_eq!(
        seen(a.view().unwrap())
            .into_iter()
            .map(|s| s.c)
            .collect::<String>(),
        "First paragraph."
    );
    b.undo().unwrap();
    sync(&mut a, &mut b);
    assert_eq!(live(a.view().unwrap()), "t paragraph.");
    a.undo().unwrap();
    sync(&mut a, &mut b);
    assert_eq!(live(a.view().unwrap()), "First paragraph.");
}

#[test]
fn inserted_paragraph_boundary_keeps_both_authors_and_round_trips() {
    let bytes = base();
    let mut a = Replica::seed(10, &bytes).unwrap();
    let node = first(a.view().unwrap(), "First");
    a.apply(&replace(node, 0, 0, "one\ntwo", Some("Alice")))
        .unwrap();
    let saved = a.save();
    let before = a.export().unwrap();
    let mut b = Replica::join(20, &bytes, &saved).unwrap();
    let paragraphs = b.view().unwrap().paragraphs();
    b.apply(&Command::ReplaceRange {
        from: Point::End { end: paragraphs[0] },
        to: Point::Start {
            start: paragraphs[1],
        },
        text: "".into(),
        track: Some(track("Bob")),
    })
    .unwrap();
    sync(&mut a, &mut b);
    let bytes_out = a.export().unwrap();
    for doc in [a.view().unwrap(), &Document::open(&bytes_out).unwrap()] {
        let NodeKind::Paragraph(info) = &doc.node(doc.paragraphs()[0]).unwrap().kind else {
            unreachable!()
        };
        assert_eq!(info.mark_revision.as_deref(), Some("del"));
        assert_eq!(
            info.mark_change.as_ref().unwrap().author.as_deref(),
            Some("Bob")
        );
        assert_eq!(
            info.mark_insertion.as_ref().unwrap().author.as_deref(),
            Some("Alice")
        );
    }
    b.undo().unwrap();
    sync(&mut a, &mut b);
    assert_eq!(a.export().unwrap(), before);
    a.apply(&Command::ReplaceRange {
        from: Point::End { end: paragraphs[0] },
        to: Point::Start {
            start: paragraphs[1],
        },
        text: "".into(),
        track: Some(track("Alice")),
    })
    .unwrap();
    assert_eq!(a.view().unwrap().paragraphs().len(), paragraphs.len() - 1);
    a.undo().unwrap();
    assert_eq!(a.export().unwrap(), before);
}

#[test]
fn replacement_and_repeated_deletion_share_the_headless_rules() {
    let bytes = base();
    let mut a = Replica::seed(10, &bytes).unwrap();
    let mut headless = docx_engine::editing::Editor::open(&bytes).unwrap();
    let node = first(a.view().unwrap(), "First");
    let command = replace(node, 0, 5, "Next", Some("Alice"));
    a.apply(&command).unwrap();
    headless.apply(&command).unwrap();
    assert_eq!(a.export().unwrap(), headless.export().unwrap());
    assert_eq!(live(a.view().unwrap()), "Next paragraph.");
    let doc = a.view().unwrap();
    let deleted = doc
        .descendants(doc.paragraphs()[0])
        .into_iter()
        .find(
            |id| matches!(&doc.node(*id).unwrap().kind, NodeKind::Revision(r) if r.change == "del"),
        )
        .unwrap();
    let command = Command::ReplaceRange {
        from: Point::Before { before: deleted },
        to: Point::After { after: deleted },
        text: "".into(),
        track: Some(track("Bob")),
    };
    let before = a.export().unwrap();
    a.apply(&command).unwrap();
    assert_eq!(a.export().unwrap(), before);
    a.undo().unwrap(); // a no-op must not consume or overwrite the earlier undo
    headless.undo();
    assert_eq!(a.export().unwrap(), headless.export().unwrap());
}

#[test]
fn deleting_a_deletion_of_the_file_again_writes_nothing() {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/generated/tracked-changes.docx"
    ))
    .unwrap();
    let mut a = Replica::seed(10, &bytes).unwrap();
    let mut headless = docx_engine::editing::Editor::open(&bytes).unwrap();
    let original = a.export().unwrap();
    let typed = replace(first(a.view().unwrap(), "Kept"), 0, 0, "A", None);
    a.apply(&typed).unwrap();
    headless.apply(&typed).unwrap();
    a.take_update().unwrap();
    let doc = a.view().unwrap();
    let deleted = doc
        .descendants(doc.paragraphs()[0])
        .into_iter()
        .find(
            |id| matches!(&doc.node(*id).unwrap().kind, NodeKind::Revision(r) if r.change == "del"),
        )
        .unwrap();
    let bold = first(doc, "newly bold");
    let again = Command::ReplaceRange {
        from: Point::Before { before: deleted },
        to: Point::After { after: deleted },
        text: "".into(),
        track: Some(track("Carol")),
    };
    let before = a.export().unwrap();
    a.apply(&again).unwrap();
    headless.apply(&again).unwrap();
    assert_eq!(a.export().unwrap(), before);
    assert!(a.take_update().unwrap().is_none());

    // Live text selected with it is still marked.
    let mixed = Command::ReplaceRange {
        from: Point::Before { before: deleted },
        to: Point::Text {
            node: bold,
            offset: 5,
        },
        text: "".into(),
        track: Some(track("Carol")),
    };
    a.apply(&mixed).unwrap();
    headless.apply(&mixed).unwrap();
    assert_eq!(a.export().unwrap(), headless.export().unwrap());
    let by = |doc: &Document, author: &str| -> String {
        seen(doc)
            .into_iter()
            .filter(|c| c.authors.contains(&("del".into(), author.into())))
            .map(|c| c.c)
            .collect()
    };
    assert_eq!(by(a.view().unwrap(), "Carol"), "newly");
    assert_eq!(by(a.view().unwrap(), "Bob"), "deleted text, ");

    for _ in 0..2 {
        assert!(a.undo().unwrap());
        assert!(headless.undo());
    }
    assert_eq!(a.export().unwrap(), original);
    assert_eq!(headless.export().unwrap(), original);
}

#[test]
fn a_preserved_drawing_can_be_marked_deleted_without_losing_its_payload() {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/generated/image.docx"
    ))
    .unwrap();
    let mut a = Replica::seed(10, &bytes).unwrap();
    let doc = a.view().unwrap();
    let root = doc.stories().next().unwrap().id;
    let drawing = doc
        .descendants(root)
        .into_iter()
        .find(|id| matches!(doc.node(*id).unwrap().kind, NodeKind::Drawing(_)))
        .unwrap();
    let before = a.export().unwrap();
    a.apply(&Command::ReplaceRange {
        from: Point::Before { before: drawing },
        to: Point::After { after: drawing },
        text: "".into(),
        track: Some(track("Alice")),
    })
    .unwrap();
    let doc = a.view().unwrap();
    let mut parent = doc.node(drawing).unwrap().parent;
    let mut deletion = None;
    while let Some(id) = parent {
        let node = doc.node(id).unwrap();
        if let NodeKind::Revision(r) = &node.kind {
            deletion = Some((r.change.clone(), r.author.clone()));
        }
        parent = node.parent;
    }
    assert_eq!(deletion, Some(("del".into(), Some("Alice".into()))));
    assert!(docx_engine::validate_package(&a.export().unwrap()).valid);
    let mut b = Replica::join(20, &bytes, &a.save()).unwrap();
    assert_eq!(a.export().unwrap(), b.export().unwrap());
    a.undo().unwrap();
    assert_eq!(a.export().unwrap(), before);
}

#[test]
fn deleting_a_section_boundary_is_refused_before_any_write() {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/generated/sections.docx"
    ))
    .unwrap();
    for tracking in [None, Some(track("Alice"))] {
        let mut a = Replica::seed(10, &bytes).unwrap();
        let doc = a.view().unwrap();
        let paragraphs = doc.paragraphs();
        let index = paragraphs.iter().position(|id| matches!(&doc.node(*id).unwrap().kind, NodeKind::Paragraph(info) if info.section.is_some())).unwrap();
        let before = a.save();
        let result = a.apply(&Command::ReplaceRange {
            from: Point::End {
                end: paragraphs[index],
            },
            to: Point::Start {
                start: paragraphs[index + 1],
            },
            text: "".into(),
            track: tracking,
        });
        assert!(result.is_err());
        assert_eq!(a.save(), before);
        assert!(a.take_update().unwrap().is_none());
    }
}

#[test]
fn three_replicas_resolve_deletion_and_two_inserts_in_every_delivery_order() {
    let bytes = base();
    let mut seed = Replica::seed(10, &bytes).unwrap();
    let node = first(seed.view().unwrap(), "First");
    seed.apply(&replace(node, 0, 0, "ABCDE", None)).unwrap();
    let saved = seed.save();
    let orders = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    for (from, to) in [(0, 5), (1, 4), (2, 3)] {
        for p in 0..=5 {
            for q in 0..=5 {
                let mut a = Replica::join(20, &bytes, &saved).unwrap();
                let mut b = Replica::join(30, &bytes, &saved).unwrap();
                let mut c = Replica::join(40, &bytes, &saved).unwrap();
                let an = first(a.view().unwrap(), "ABCDE");
                let bn = first(b.view().unwrap(), "ABCDE");
                let cn = first(c.view().unwrap(), "ABCDE");
                a.apply(&replace(an, from, to, "", Some("Alice"))).unwrap();
                b.apply(&replace(bn, p, p, "β", None)).unwrap();
                c.apply(&replace(cn, q, q, "γ", Some("Carol"))).unwrap();
                let updates = [
                    a.update_since(&seed.state_vector()).unwrap(),
                    b.update_since(&seed.state_vector()).unwrap(),
                    c.update_since(&seed.state_vector()).unwrap(),
                ];
                let mut expected = None;
                for order in orders {
                    let mut r = Replica::join(50, &bytes, &saved).unwrap();
                    for i in order {
                        r.apply_update(&updates[i]).unwrap();
                        r.apply_update(&updates[i]).unwrap();
                    }
                    let content = seen(r.view().unwrap());
                    let visible = live(r.view().unwrap());
                    assert_eq!(visible.matches('β').count(), 1);
                    assert_eq!(visible.matches('γ').count(), 1);
                    assert_eq!(
                        visible.replace(['β', 'γ'], ""),
                        format!("{}{}First paragraph.", &"ABCDE"[..from], &"ABCDE"[to..])
                    );
                    assert!(content
                        .iter()
                        .find(|s| s.c == 'β')
                        .unwrap()
                        .authors
                        .is_empty());
                    assert_eq!(
                        content.iter().find(|s| s.c == 'γ').unwrap().authors,
                        [("ins".into(), "Carol".into())]
                    );
                    let exported = r.export().unwrap();
                    if let Some(expected) = &expected {
                        assert_eq!(&exported, expected);
                    } else {
                        expected = Some(exported);
                    }
                }
            }
        }
    }
}
