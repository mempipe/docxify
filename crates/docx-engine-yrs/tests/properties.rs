use docx_engine::{Command, Document, ListKind, NodeId, Point};
use docx_engine_yrs::Replica;
fn bytes() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/generated/headings-styles.docx"
    ))
    .unwrap()
}
fn range(p: NodeId) -> (Point, Point) {
    (Point::Start { start: p }, Point::End { end: p })
}
fn properties(doc: &Document) -> Vec<serde_json::Value> {
    // Export list numbers are allocation details; preserve which paragraphs
    // share a list while comparing its level, definition and layout.
    let mut identities = std::collections::HashMap::new();
    doc.paragraphs().into_iter().map(|p| {
        let mut n=serde_json::to_value(doc.inspect_node(p).unwrap()).unwrap();
        if let Some(number) = n["list"]["num_id"].as_str().map(str::to_string) {
            let next = identities.len();
            n["list"]["num_id"] = (*identities.entry(number).or_insert(next)).into();
        }
        serde_json::json!({"text":n["paragraph_text"],"style":n["style"],"list":n["list"],"effective":n["effective"]})
    }).collect()
}
#[test]
fn styles_and_lists_match_the_single_document_commands_and_undo() {
    let bytes = bytes();
    let mut a = Replica::seed(1, &bytes).unwrap();
    let original = a.export().unwrap();
    let mut reference = docx_engine::editing::Editor::open(&bytes).unwrap();
    let ps = reference.view().unwrap().paragraphs();
    let (from, to) = range(ps[1]);
    let commands = [
        Command::SetParagraphStyle {
            from,
            to,
            style: "Heading1".into(),
        },
        Command::SetList {
            from,
            to,
            kind: Some(ListKind::Bullet),
        },
        Command::ChangeListLevel { from, to, by: 1 },
        Command::SetList {
            from,
            to,
            kind: Some(ListKind::Number),
        },
        Command::SetList {
            from,
            to,
            kind: None,
        },
    ];
    for command in commands {
        reference.apply(&command).unwrap();
        a.apply(&command).unwrap();
        assert_eq!(
            properties(a.view().unwrap()),
            properties(reference.view().unwrap()),
            "{}",
            command.name()
        );
    }
    for _ in 0..5 {
        a.undo().unwrap();
    }
    assert_eq!(a.export().unwrap(), original);
}
#[test]
fn concurrent_style_and_numbering_keep_both_and_new_lists_have_distinct_identity() {
    let bytes = bytes();
    let mut a = Replica::seed(1, &bytes).unwrap();
    let saved = a.save();
    let mut b = Replica::join(2, &bytes, &saved).unwrap();
    let ps = a.view().unwrap().paragraphs();
    let (from, to) = range(ps[1]);
    a.apply(&Command::SetParagraphStyle {
        from,
        to,
        style: "Heading1".into(),
    })
    .unwrap();
    b.apply(&Command::SetList {
        from,
        to,
        kind: Some(ListKind::Bullet),
    })
    .unwrap();
    let ua = a.update_since(&b.state_vector()).unwrap();
    let ub = b.update_since(&a.state_vector()).unwrap();
    a.apply_update(&ub).unwrap();
    b.apply_update(&ua).unwrap();
    assert_eq!(a.export().unwrap(), b.export().unwrap());
    let n = serde_json::to_value(a.view().unwrap().inspect_node(ps[1]).unwrap()).unwrap();
    assert_eq!(n["style"], "Heading1");
    assert!(!n["list"].is_null());
    let (from, to) = range(ps[0]);
    a.apply(&Command::SetList {
        from,
        to,
        kind: Some(ListKind::Number),
    })
    .unwrap();
    let (from, to) = range(ps[2]);
    b.apply(&Command::SetList {
        from,
        to,
        kind: Some(ListKind::Number),
    })
    .unwrap();
    let ua = a.update_since(&b.state_vector()).unwrap();
    let ub = b.update_since(&a.state_vector()).unwrap();
    a.apply_update(&ub).unwrap();
    b.apply_update(&ua).unwrap();
    assert_eq!(a.export().unwrap(), b.export().unwrap());
    let doc = a.view().unwrap();
    let x = serde_json::to_value(doc.inspect_node(ps[0]).unwrap()).unwrap();
    let y = serde_json::to_value(doc.inspect_node(ps[2]).unwrap()).unwrap();
    assert_ne!(x["list"]["num_id"], y["list"]["num_id"]);
}

fn links(doc: &Document) -> Vec<(String, String)> {
    doc.stories()
        .flat_map(|s| doc.descendants(s.id))
        .filter_map(|id| {
            if let docx_engine::NodeKind::Hyperlink(info) = &doc.node(id)?.kind {
                Some((
                    doc.descendants(id)
                        .into_iter()
                        .filter_map(|n| match &doc.node(n).unwrap().kind {
                            docx_engine::NodeKind::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<String>(),
                    docx_engine::href(info).unwrap(),
                ))
            } else {
                None
            }
        })
        .collect()
}
#[test]
fn links_survive_concurrent_deletions_and_retarget_without_nesting() {
    let bytes = bytes();
    let mut a = Replica::seed(1, &bytes).unwrap();
    let p = a.view().unwrap().paragraphs()[1];
    let (from, to) = range(p);
    a.apply(&Command::SetLink {
        from,
        to,
        target: Some("https://example.com/a".into()),
    })
    .unwrap();
    let saved = a.save();
    let mut b = Replica::join(2, &bytes, &saved).unwrap();
    let doc = a.view().unwrap();
    let node = doc
        .descendants(p)
        .into_iter()
        .find(|id| {
            matches!(
                doc.node(*id).unwrap().kind,
                docx_engine::NodeKind::Text { .. }
            )
        })
        .unwrap();
    a.apply(&Command::ReplaceText {
        node,
        range: Some(docx_engine::TextRange { start: 0, end: 2 }),
        text: "".into(),
    })
    .unwrap();
    b.apply(&Command::ReplaceText {
        node,
        range: Some(docx_engine::TextRange { start: 2, end: 4 }),
        text: "".into(),
    })
    .unwrap();
    let ua = a.update_since(&b.state_vector()).unwrap();
    let ub = b.update_since(&a.state_vector()).unwrap();
    a.apply_update(&ub).unwrap();
    b.apply_update(&ua).unwrap();
    assert_eq!(a.export().unwrap(), b.export().unwrap());
    assert_eq!(
        links(a.view().unwrap()),
        vec![("ter one".into(), "https://example.com/a".into())]
    );
    a.apply(&Command::SetLink {
        from,
        to,
        target: Some("https://example.com/b".into()),
    })
    .unwrap();
    assert_eq!(
        links(a.view().unwrap()),
        vec![("ter one".into(), "https://example.com/b".into())]
    );
    a.apply(&Command::SetLink {
        from,
        to,
        target: None,
    })
    .unwrap();
    assert!(links(a.view().unwrap()).is_empty());
    a.undo().unwrap();
    assert_eq!(links(a.view().unwrap())[0].1, "https://example.com/b");
    assert!(docx_engine::validate_package(&a.export().unwrap()).valid);
}
#[test]
fn split_retains_the_current_list_and_follows_a_headings_next_style() {
    let bytes = bytes();
    let mut a = Replica::seed(1, &bytes).unwrap();
    let mut reference = docx_engine::editing::Editor::open(&bytes).unwrap();
    let p = reference.view().unwrap().paragraphs()[2];
    let (from, to) = range(p);
    for command in [
        Command::SetParagraphStyle {
            from,
            to,
            style: "Heading1".into(),
        },
        Command::SplitParagraph { at: to },
    ] {
        reference.apply(&command).unwrap();
        a.apply(&command).unwrap();
    }
    assert_eq!(
        properties(a.view().unwrap()),
        properties(reference.view().unwrap())
    );
}
