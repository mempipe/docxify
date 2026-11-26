#[path = "../../docx-engine/tests/common/mod.rs"]
mod common;
use common::*;
use docx_engine::{Command, Document, NodeKind, Point, Track};
use docx_engine_yrs::Replica;
fn by(name: &str) -> Track {
    Track {
        author: name.into(),
        date: "2026-11-18T00:00:00Z".into(),
    }
}
fn comments(doc: &Document) -> Vec<(String, Option<String>, String)> {
    doc.stories()
        .flat_map(|s| doc.descendants(s.id))
        .filter_map(|id| {
            if let NodeKind::Comment {
                comment_id, parent, ..
            } = &doc.node(id)?.kind
            {
                let text = doc
                    .descendants(id)
                    .into_iter()
                    .filter_map(|id| match &doc.node(id)?.kind {
                        NodeKind::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                Some((comment_id.clone(), parent.clone(), text))
            } else {
                None
            }
        })
        .collect()
}
fn sync(a: &mut Replica, b: &mut Replica) {
    let ua = a.update_since(&b.state_vector()).unwrap();
    let ub = b.update_since(&a.state_vector()).unwrap();
    a.apply_update(&ub).unwrap();
    b.apply_update(&ua).unwrap();
    assert!(
        a.export().unwrap() == b.export().unwrap(),
        "replicas differ"
    );
    assert_valid("comments", &a.export().unwrap());
}
#[test]
fn overlapping_comments_keep_both_ranges_through_deletion_and_undo() {
    let bytes = generated("simple-paragraphs");
    let mut a = Replica::seed(1, &bytes).unwrap();
    let original = a.export().unwrap();
    let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
    let t = text_node(a.view().unwrap(), "First");
    for (replica, name, start, end) in [(&mut a, "Alice", 1, 5), (&mut b, "Bob", 3, 9)] {
        replica
            .apply(&Command::AddComment {
                from: at(t, start),
                to: at(t, end),
                text: name.into(),
                by: by(name),
                initials: None,
                id: Some(0),
            })
            .unwrap();
    }
    sync(&mut a, &mut b);
    assert_eq!(comments(a.view().unwrap()).len(), 2);
    let p = a.view().unwrap().paragraphs()[0];
    a.apply(&Command::ReplaceRange {
        from: Point::Start { start: p },
        to: Point::End { end: p },
        text: "".into(),
        track: None,
    })
    .unwrap();
    sync(&mut a, &mut b);
    assert_eq!(comments(a.view().unwrap()).len(), 2);
    a.undo().unwrap();
    sync(&mut a, &mut b);
    a.undo().unwrap();
    b.undo().unwrap();
    sync(&mut a, &mut b);
    assert!(a.export().unwrap() == original);
}
#[test]
fn concurrent_comments_wanting_the_highest_id_both_stay_readable() {
    let bytes = generated("simple-paragraphs");
    let mut a = Replica::seed(1, &bytes).unwrap();
    let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
    let t = text_node(a.view().unwrap(), "First");
    for (replica, name) in [(&mut a, "Alice"), (&mut b, "Bob")] {
        replica
            .apply(&Command::AddComment {
                from: at(t, 1),
                to: at(t, 5),
                text: name.into(),
                by: by(name),
                initials: None,
                id: Some(i32::MAX as u32),
            })
            .unwrap();
    }
    sync(&mut a, &mut b);
    let mut ids: Vec<String> = comments(a.view().unwrap())
        .into_iter()
        .map(|c| c.0)
        .collect();
    ids.sort();
    assert_eq!(ids, ["0", "2147483647"]);
}
#[test]
fn a_reply_keeps_its_root_when_the_root_author_undoes() {
    let bytes = generated("simple-paragraphs");
    let mut a = Replica::seed(1, &bytes).unwrap();
    let original = a.export().unwrap();
    let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
    let p = a.view().unwrap().paragraphs()[0];
    a.apply(&Command::AddComment {
        from: Point::Start { start: p },
        to: Point::End { end: p },
        text: "Root".into(),
        by: by("Alice"),
        initials: None,
        id: None,
    })
    .unwrap();
    sync(&mut a, &mut b);
    let root = comments(b.view().unwrap())[0].0.clone();
    b.apply(&Command::ReplyToComment {
        comment: root,
        text: "Reply".into(),
        by: by("Bob"),
        initials: None,
        id: None,
    })
    .unwrap();
    a.undo().unwrap();
    sync(&mut a, &mut b);
    let cs = comments(a.view().unwrap());
    assert_eq!(cs.len(), 2);
    assert_eq!(cs[0].2, "Root");
    assert_eq!(cs[1].2, "Reply");
    assert!(cs
        .iter()
        .any(|(_, parent, text)| parent.is_some() && text == "Reply"));
    b.undo().unwrap();
    sync(&mut a, &mut b);
    assert!(a.export().unwrap() == original);
    a.redo().unwrap();
    b.redo().unwrap();
    sync(&mut a, &mut b);
    assert_eq!(comments(a.view().unwrap()).len(), 2);
}
#[test]
fn replies_to_original_comments_and_collapsed_comments_are_exportable() {
    let bytes = generated("comments");
    let mut a = Replica::seed(1, &bytes).unwrap();
    let original = a.export().unwrap();
    let root = comments(a.view().unwrap())
        .into_iter()
        .find(|(_, p, _)| p.is_none())
        .unwrap()
        .0;
    a.apply(&Command::ReplyToComment {
        comment: root,
        text: "New reply".into(),
        by: by("Alice"),
        initials: None,
        id: None,
    })
    .unwrap();
    assert_valid("reply", &a.export().unwrap());
    a.undo().unwrap();
    assert!(a.export().unwrap() == original);
    let p = a.view().unwrap().paragraphs()[0];
    let point = Point::Start { start: p };
    a.apply(&Command::AddComment {
        from: point,
        to: point,
        text: "Caret".into(),
        by: by("Alice"),
        initials: None,
        id: None,
    })
    .unwrap();
    assert_valid("collapsed", &a.export().unwrap());
    let out = a.export().unwrap();
    let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
    assert!(b.export().unwrap() == out);
}

/// The text between a comment's range marks in the exported main part.
fn covered(bytes: &[u8]) -> Vec<String> {
    let xml = part(bytes, "word/document.xml").replace(r#"<w:t xml:space="preserve">"#, "<w:t>");
    let start = xml.find("<w:commentRangeStart").expect("range start");
    let end = xml.find("<w:commentRangeEnd").expect("range end");
    assert!(start < end, "the range ends before it starts");
    xml[start..end]
        .split("<w:t>")
        .skip(1)
        .map(|rest| rest.split("</w:t>").next().unwrap().to_string())
        .filter(|text| !text.is_empty())
        .collect()
}

fn with_typed_row() -> (Replica, Replica) {
    let bytes = generated("table");
    let mut alice = Replica::seed(1, &bytes).unwrap();
    let mut bob = Replica::join(2, &bytes, &alice.save()).unwrap();
    let doc = alice.view().unwrap();
    let header = doc
        .node(paragraph_with(doc, "Name"))
        .unwrap()
        .parent
        .unwrap();
    let header = doc.node(header).unwrap().parent.unwrap();
    alice
        .apply(&Command::InsertRow {
            row: header,
            below: true,
            like: None,
        })
        .unwrap();
    let doc = alice.view().unwrap();
    let table = doc.node(header).unwrap().parent.unwrap();
    let row = doc.node(table).unwrap().children[1];
    let cell = doc.node(row).unwrap().children[0];
    let paragraph = doc.node(cell).unwrap().children[0];
    alice
        .apply(&Command::InsertText {
            at: Point::End { end: paragraph },
            text: "NEW".into(),
        })
        .unwrap();
    sync(&mut alice, &mut bob);
    (alice, bob)
}

#[test]
fn a_comment_into_a_created_row_covers_what_was_selected_after_the_row_is_undone() {
    let (mut alice, mut bob) = with_typed_row();
    let doc = bob.view().unwrap();
    let (from, to) = (text_node(doc, "Name"), text_node(doc, "NEW"));
    bob.apply(&Command::AddComment {
        from: at(from, 0),
        to: at(to, 3),
        text: "Check".into(),
        by: by("Bob"),
        initials: None,
        id: None,
    })
    .unwrap();
    sync(&mut alice, &mut bob);
    assert_eq!(
        covered(&alice.export().unwrap()),
        ["Name", "Role", "Notes", "NEW"]
    );
    assert!(alice.undo().unwrap());
    assert!(alice.undo().unwrap());
    sync(&mut alice, &mut bob);
    assert_eq!(covered(&bob.export().unwrap()), ["Name", "Role", "Notes"]);
}

#[test]
fn a_comment_from_a_created_row_into_the_next_source_row_follows_the_visible_order() {
    let (mut alice, mut bob) = with_typed_row();
    let doc = bob.view().unwrap();
    let (from, to) = (text_node(doc, "NEW"), text_node(doc, "Ada"));
    bob.apply(&Command::AddComment {
        from: at(from, 0),
        to: at(to, 3),
        text: "Check".into(),
        by: by("Bob"),
        initials: None,
        id: None,
    })
    .unwrap();
    sync(&mut alice, &mut bob);
    assert_eq!(covered(&alice.export().unwrap()), ["NEW", "Ada"]);
    let root = comments(bob.view().unwrap())[0].0.clone();
    alice
        .apply(&Command::ReplyToComment {
            comment: root,
            text: "Done".into(),
            by: by("Alice"),
            initials: None,
            id: None,
        })
        .unwrap();
    sync(&mut alice, &mut bob);
    assert_eq!(comments(bob.view().unwrap()).len(), 2);
}
