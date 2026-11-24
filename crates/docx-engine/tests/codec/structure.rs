use super::common;

use common::*;
use docx_engine::{Command, Document, EditError, NodeId, NodeKind, Point};

fn find(doc: &Document, pred: impl Fn(&NodeKind) -> bool) -> NodeId {
    let roots: Vec<NodeId> = doc.stories().map(|s| s.id).collect();
    roots
        .into_iter()
        .flat_map(|r| doc.descendants(r))
        .find(|&id| pred(&doc.node(id).unwrap().kind))
        .expect("a matching node")
}

#[test]
fn split_paragraph_keeps_the_first_half_and_copies_its_properties() {
    let bytes = generated("headings-styles");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let heading = paragraph_with(&doc, "Chapter one");
    let text = text_node(&doc, "Chapter one");
    edit(
        &mut doc,
        Command::SplitParagraph {
            at: Point::Text {
                node: text,
                offset: 4,
            },
        },
    );
    assert_eq!(doc.text(heading).unwrap(), "Chap");
    let paragraphs = doc.paragraphs();
    let second = paragraphs[paragraphs.iter().position(|&p| p == heading).unwrap() + 1];
    assert_eq!(doc.text(second).unwrap(), "ter one");
    assert_eq!(doc.heading_level(second), Some(1));

    let reopened = reopen("headings-styles", &doc);
    let mut expected = texts(&Document::open(&bytes).unwrap());
    let at = expected.iter().position(|t| t == "Chapter one").unwrap();
    expected.splice(at..=at, ["Chap".to_string(), "ter one".to_string()]);
    assert_eq!(texts(&reopened), expected);
    let split = paragraph_with(&reopened, "ter one");
    assert_eq!(reopened.heading_level(split), Some(1));
    assert_editor_undo("headings-styles", &bytes, &mut doc);
}

#[test]
fn a_split_paragraph_does_not_copy_words_paragraph_id() {
    let bytes = generated("rich-text");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let bold = text_node(&doc, "Bold");
    let paragraph = paragraph_with(&doc, "Bold");
    edit(
        &mut doc,
        Command::SplitParagraph {
            at: Point::Text {
                node: bold,
                offset: 2,
            },
        },
    );
    let paragraphs = doc.paragraphs();
    let split = paragraphs.iter().position(|&p| p == paragraph).unwrap();
    let id = |p: NodeId| match &doc.node(p).unwrap().kind {
        NodeKind::Paragraph(info) => info.word_para_id.clone(),
        _ => unreachable!(),
    };
    assert_eq!(id(paragraphs[split]).as_deref(), Some("1A2B3C4D"));
    assert_eq!(id(paragraphs[split + 1]), None);
    let xml = part(&doc.export().unwrap(), "word/document.xml");
    assert_eq!(xml.matches("1A2B3C4D").count(), 1);
    assert!(
        xml.contains(r#"<w:p><w:r><w:rPr><w:b/></w:rPr><w:t xml:space="preserve">ld</w:t></w:r>"#)
    );
}

#[test]
fn enter_at_the_end_of_a_heading_starts_its_next_style() {
    let bytes = generated("headings-styles");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let text = text_node(&doc, "Chapter one");
    edit(
        &mut doc,
        Command::SplitParagraph {
            at: Point::Text {
                node: text,
                offset: 11,
            },
        },
    );
    let xml = part(&doc.export().unwrap(), "word/document.xml");
    assert!(xml.contains(r#"Chapter one</w:t></w:r></w:p><w:p></w:p>"#));
    let reopened = reopen("headings-styles", &doc);
    let empty = reopened.paragraphs()[2];
    assert_eq!(reopened.text(empty).unwrap(), "");
    assert_eq!(reopened.heading_level(empty), None);
    assert_editor_undo("headings-styles", &bytes, &mut doc);
}

#[test]
fn every_fixture_splits_undoes_and_redoes_exactly() {
    let mut splits = 0;
    for (name, bytes) in all_fixtures() {
        let doc = Document::open(&bytes).unwrap();
        let targets: Vec<NodeId> = doc
            .paragraphs()
            .into_iter()
            .flat_map(|p| doc.descendants(p))
            .filter(|&id| {
                let node = doc.node(id).unwrap();
                node.support == docx_engine::Support::Editable
                    && matches!(&node.kind, NodeKind::Text { text } if text.chars().count() >= 2)
            })
            .collect();
        for target in targets {
            let mut doc = EditingDocument::open(&bytes).unwrap();
            let before = texts(&doc);
            let Ok(()) = doc.0.apply(&Command::SplitParagraph {
                at: Point::Text {
                    node: target,
                    offset: 1,
                },
            }) else {
                assert_eq!(
                    stories(&doc),
                    stories(&Document::open(&bytes).unwrap()),
                    "{name}: refusal changed the document"
                );
                continue;
            };
            splits += 1;
            let reopened = reopen(&name, &doc);
            assert_eq!(texts(&reopened).len(), before.len() + 1, "{name}");
            assert_eq!(texts(&reopened).concat(), before.concat(), "{name}");
            assert_editor_undo(&name, &bytes, &mut doc);
        }
    }
    assert!(splits > 50, "only {splits} splits applied");
}

#[test]
fn merge_paragraphs_keeps_the_first_and_undo_restores_the_second() {
    let bytes = generated("simple-paragraphs");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let first = paragraph_with(&doc, "First");
    edit(&mut doc, Command::MergeParagraphs { paragraph: first });
    assert_eq!(
        doc.text(first).unwrap(),
        "First paragraph.Second paragraph."
    );
    let reopened = reopen("simple-paragraphs", &doc);
    assert_eq!(
        texts(&reopened),
        ["First paragraph.Second paragraph.", "Third paragraph."]
    );
    assert_editor_undo("simple-paragraphs", &bytes, &mut doc);
}

#[test]
fn merging_away_an_empty_paragraph_keeps_the_next_ones_style() {
    let bytes = generated("headings-styles");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let body = text_node(&doc, "Body text");
    let len = doc.text(body).unwrap().chars().count();
    edit(
        &mut doc,
        Command::SplitParagraph {
            at: Point::Text {
                node: body,
                offset: len,
            },
        },
    );
    let heading = paragraph_with(&doc, "Section 1.1");
    let paragraphs = doc.paragraphs();
    let empty = paragraphs[paragraphs.iter().position(|&p| p == heading).unwrap() - 1];
    assert_eq!(doc.text(empty).unwrap(), "");
    edit(&mut doc, Command::MergeParagraphs { paragraph: empty });
    assert!(doc.paragraphs().contains(&heading));
    assert!(!doc.paragraphs().contains(&empty));
    assert_eq!(doc.heading_level(heading), Some(2));
    assert_eq!(
        texts(&reopen("headings-styles", &doc)),
        texts(&Document::open(&bytes).unwrap())
    );
}

#[test]
fn insert_text_fills_an_empty_self_closing_paragraph() {
    let bytes = generated("table");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let empty = doc
        .paragraphs()
        .into_iter()
        .find(|&p| doc.node(p).unwrap().children.is_empty())
        .unwrap();
    edit(
        &mut doc,
        Command::InsertText {
            at: Point::End { end: empty },
            text: "Typed".into(),
        },
    );
    assert_eq!(doc.text(empty).unwrap(), "Typed");
    let xml = part(&doc.export().unwrap(), "word/document.xml");
    assert!(xml
        .contains(r#"</w:tbl><w:p><w:r><w:t xml:space="preserve">Typed</w:t></w:r></w:p></w:tc>"#));
    assert!(texts(&reopen("table", &doc)).contains(&"Typed".to_string()));
    assert_editor_undo("table", &bytes, &mut doc);
}

#[test]
fn insert_text_beside_a_drawing_goes_into_its_run_or_a_copy_of_it() {
    let bytes = generated("image");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let drawing = find(&doc, |k| matches!(k, NodeKind::Drawing(_)));
    let run = doc.node(drawing).unwrap().parent.unwrap();
    let paragraph = doc.node(run).unwrap().parent.unwrap();
    edit(
        &mut doc,
        Command::InsertText {
            at: Point::After { after: drawing },
            text: "caption".into(),
        },
    );
    assert_eq!(
        doc.node(doc.node(drawing).unwrap().parent.unwrap())
            .unwrap()
            .children
            .len(),
        2
    );
    edit(
        &mut doc,
        Command::InsertText {
            at: Point::End { end: paragraph },
            text: " more".into(),
        },
    );
    assert_eq!(doc.text(paragraph).unwrap(), "caption more");
    let xml = part(&doc.export().unwrap(), "word/document.xml");
    assert!(xml.find("</w:drawing>").unwrap() < xml.find("caption").unwrap());
    assert_eq!(
        reopen("image", &doc).text(paragraph).as_deref(),
        Some("caption more")
    );
    assert!(doc.0.undo());
    assert_editor_undo("image", &bytes, &mut doc);
}

#[test]
fn insert_break_splits_the_text_inside_its_run() {
    let bytes = generated("simple-paragraphs");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let text = text_node(&doc, "First");
    edit(
        &mut doc,
        Command::InsertBreak {
            to: None,
            at: Point::Text {
                node: text,
                offset: 5,
            },
        },
    );
    let xml = part(&doc.export().unwrap(), "word/document.xml");
    assert!(xml.contains(r#"<w:r><w:t xml:space="preserve">First</w:t><w:br/><w:t xml:space="preserve"> paragraph.</w:t></w:r>"#));
    assert_eq!(
        texts(&reopen("simple-paragraphs", &doc))[0],
        "First\n paragraph."
    );
    assert_editor_undo("simple-paragraphs", &bytes, &mut doc);

    let mut doc = EditingDocument::open(&bytes).unwrap();
    let paragraph = paragraph_with(&doc, "Second");
    edit(
        &mut doc,
        Command::InsertBreak {
            to: None,
            at: Point::End { end: paragraph },
        },
    );
    assert_eq!(
        texts(&reopen("paragraph-end break", &doc))[1],
        "Second paragraph.\n"
    );
    assert_editor_undo("simple-paragraphs", &bytes, &mut doc);
}

#[test]
fn remove_node_takes_out_tabs_drawings_and_preserved_elements() {
    let bytes = generated("rich-text");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let tab = find(&doc, |k| matches!(k, NodeKind::Tab));
    edit(&mut doc, Command::RemoveNode { node: tab });
    assert!(
        texts(&reopen("rich-text", &doc)).contains(&"Before tabafter tab\nnext line".to_string())
    );
    assert_editor_undo("rich-text", &bytes, &mut doc);

    // A run left empty goes with what it held.
    let bytes = generated("image");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let drawing = find(&doc, |k| matches!(k, NodeKind::Drawing(_)));
    let run = doc.node(drawing).unwrap().parent.unwrap();
    let paragraph = doc.node(run).unwrap().parent.unwrap();
    edit(&mut doc, Command::RemoveNode { node: drawing });
    assert!(doc.node(paragraph).unwrap().children.is_empty());
    let xml = part(&doc.export().unwrap(), "word/document.xml");
    assert!(!xml.contains("w:drawing") && xml.contains("<w:p></w:p>"));
    reopen("image", &doc);
    assert_editor_undo("image", &bytes, &mut doc);

    let bytes = generated("unknown-ooxml");
    for element in ["x:inlineThing", "m:oMath", "x:customBlock", "w:customXml"] {
        let mut doc = EditingDocument::open(&bytes).unwrap();
        let node = find(
            &doc,
            |k| matches!(k, NodeKind::Opaque { element: e, .. } if e == element),
        );
        edit(&mut doc, Command::RemoveNode { node });
        let xml = part(&doc.export().unwrap(), "word/document.xml");
        assert!(!xml.contains(&format!("<{element}")), "{element}");
        reopen("unknown-ooxml", &doc);
        assert_editor_undo(element, &bytes, &mut doc);
    }
}

#[test]
fn remove_node_takes_out_a_whole_field() {
    let bytes = generated("header-footer");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let field = find(&doc, |k| matches!(k, NodeKind::Field { .. }));
    edit(&mut doc, Command::RemoveNode { node: field });
    assert!(texts(&reopen("header-footer", &doc)).contains(&"Header text, page ".to_string()));
    assert_editor_undo("header-footer", &bytes, &mut doc);

    // A complex field has no node; its result text stands for it.
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let footer = doc
        .stories()
        .find(|s| matches!(&s.kind, NodeKind::Story { part, .. } if part.contains("footer")))
        .unwrap()
        .id;
    let result = doc
        .descendants(footer)
        .into_iter()
        .find(|&id| matches!(&doc.node(id).unwrap().kind, NodeKind::Text { text } if text == "1"))
        .unwrap();
    edit(&mut doc, Command::RemoveNode { node: result });
    let xml = part(&doc.export().unwrap(), "word/footer1.xml");
    assert!(!xml.contains("fldChar") && !xml.contains("instrText"));
    assert!(xml.contains("Footer page </w:t></w:r></w:p>"));
    reopen("header-footer", &doc);
    assert_editor_undo("header-footer", &bytes, &mut doc);
}

#[test]
fn removing_a_tracked_insertion_or_deletion_resolves_it() {
    let bytes = generated("tracked-changes");
    for (change, text) in [
        ("ins", "Kept text, newly bold"),
        ("del", "Kept text, inserted text, newly bold"),
    ] {
        let mut doc = EditingDocument::open(&bytes).unwrap();
        let revision = find(
            &doc,
            |k| matches!(k, NodeKind::Revision(r) if r.change == change),
        );
        edit(&mut doc, Command::RemoveNode { node: revision });
        assert_eq!(texts(&reopen(change, &doc))[0], text);
        assert_editor_undo(change, &bytes, &mut doc);
    }
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let moved = find(
        &doc,
        |k| matches!(k, NodeKind::Revision(r) if r.change == "moveFrom"),
    );
    assert!(matches!(
        doc.run(Command::RemoveNode { node: moved }),
        Err(SessionError::Edit(EditError::Refused { .. }))
    ));
}

#[test]
fn structural_commands_refuse_what_would_dangle_or_lose_structure() {
    let bytes = generated("complex-mixed");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let original = stories(&doc);
    let marker = |doc: &Document, name: &str| {
        find(
            doc,
            |k| matches!(k, NodeKind::Marker { element, .. } if element == name),
        )
    };
    let note = find(&doc, |k| matches!(k, NodeKind::NoteReference { .. }));
    let toc = text_node(&doc, "Chapter one ... 1");
    let toc_paragraph = paragraph_with(&doc, "Chapter one ... 1");
    let bookmark = marker(&doc, "w:bookmarkStart");
    let comment = marker(&doc, "w:commentReference");
    let landscape = text_node(&doc, "Last paragraph of the landscape");
    let before_mark = paragraph_with(&doc, "Kept text");
    let before_table = paragraph_with(&doc, "A bullet");
    let before_unknown = paragraph_with(&doc, "Appendix");
    let comment_run = doc.node(comment).unwrap().parent.unwrap();

    let mut refused = |command: Command| {
        let result = doc.run(command.clone());
        assert!(
            matches!(
                result,
                Err(SessionError::Unsupported { .. }
                    | SessionError::Edit(EditError::Refused { .. } | EditError::ReadOnly { .. }))
            ),
            "{command:?}: {result:?}"
        );
    };
    refused(Command::RemoveNode { node: note });
    refused(Command::RemoveNode { node: toc });
    refused(Command::SplitParagraph {
        at: Point::End { end: toc_paragraph },
    });
    refused(Command::InsertText {
        at: Point::End { end: toc_paragraph },
        text: "x".into(),
    });
    refused(Command::RemoveNode { node: comment_run });
    refused(Command::SplitParagraph {
        at: Point::Text {
            node: landscape,
            offset: 4,
        },
    });
    refused(Command::MergeParagraphs {
        paragraph: before_mark,
    });
    refused(Command::MergeParagraphs {
        paragraph: before_table,
    });
    refused(Command::MergeParagraphs {
        paragraph: before_unknown,
    });
    assert_eq!(stories(&doc), original);
    assert!(!doc.is_dirty());

    // Either end of a bookmark stands for the whole bookmark.
    let ends = |doc: &Document| {
        doc.descendants(doc.stories().next().unwrap().id)
            .into_iter()
            .filter(|&id| {
                matches!(&doc.node(id).unwrap().kind,
                    NodeKind::Marker { element, .. } if element.starts_with("w:bookmark"))
            })
            .count()
    };
    let all = ends(&doc);
    edit(&mut doc, Command::RemoveNode { node: bookmark });
    assert_eq!(ends(&doc), all - 2);
    reopen("complex-mixed", &doc);
    assert_editor_undo("complex-mixed", &bytes, &mut doc);
}

#[test]
fn removed_nodes_are_refused_until_restored_and_stay_in_their_story() {
    let bytes = generated("header-footer");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let first = paragraph_with(&doc, "Body with");
    let second = paragraph_with(&doc, "Second body");
    let header = paragraph_with(&doc, "Header text");
    edit(&mut doc, Command::MergeParagraphs { paragraph: first });
    assert!(doc
        .run(Command::MergeParagraphs { paragraph: second })
        .is_err());
    assert!(doc.node(header).is_some());
    assert_editor_undo("header-footer", &bytes, &mut doc);
}

#[test]
fn structural_commands_round_trip_through_json() {
    let n = |s: &str| s.parse::<NodeId>().unwrap();
    let cases = [
        (
            Command::InsertText {
                at: Point::Text {
                    node: n("n3"),
                    offset: 2,
                },
                text: "x".into(),
            },
            r#"{"command":"insert_text","at":{"node":"n3","offset":2},"text":"x"}"#,
        ),
        (
            Command::SplitParagraph {
                at: Point::After { after: n("n4") },
            },
            r#"{"command":"split_paragraph","at":{"after":"n4"}}"#,
        ),
        (
            Command::InsertBreak {
                to: None,
                at: Point::End { end: n("n2") },
            },
            r#"{"command":"insert_break","at":{"end":"n2"}}"#,
        ),
    ];
    for (command, json) in cases {
        assert_eq!(serde_json::to_string(&command).unwrap(), json);
        assert_eq!(serde_json::from_str::<Command>(json).unwrap(), command);
    }
}

#[test]
fn edited_self_closing_elements_are_written_out() {
    let bytes = with_body(r#"<w:p><w:r><w:t/></w:r></w:p><w:p/>"#);
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let text = find(&doc, |k| matches!(k, NodeKind::Text { .. }));
    let empty = doc.paragraphs()[1];
    edit(
        &mut doc,
        Command::ReplaceText {
            node: text,
            range: None,
            text: "filled".into(),
        },
    );
    edit(
        &mut doc,
        Command::InsertText {
            at: Point::Start { start: empty },
            text: "typed".into(),
        },
    );
    assert_eq!(texts(&reopen("self-closing", &doc)), ["filled", "typed"]);
    assert!(doc.0.undo());
    let xml = part(&doc.export().unwrap(), "word/document.xml");
    assert!(xml.contains(r#"<w:t xml:space="preserve">filled</w:t></w:r></w:p><w:p/>"#));
}

#[test]
fn removal_keeps_bookmarks_whole() {
    let bytes = with_body(concat!(
        r#"<w:p><w:r><w:t>a</w:t></w:r>"#,
        r#"<w:ins w:id="1" w:author="A"><w:bookmarkStart w:id="5" w:name="half"/><w:r><w:t>b</w:t></w:r></w:ins>"#,
        r#"<w:ins w:id="2" w:author="A"><w:bookmarkStart w:id="6" w:name="whole"/><w:r><w:t>c</w:t></w:r><w:bookmarkEnd w:id="6"/></w:ins>"#,
        r#"<w:bookmarkEnd w:id="5"/></w:p>"#
    ));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let insertions: Vec<NodeId> = doc
        .descendants(doc.paragraphs()[0])
        .into_iter()
        .filter(|&id| matches!(doc.node(id).unwrap().kind, NodeKind::Revision(_)))
        .collect();
    assert!(matches!(
        doc.run(Command::RemoveNode {
            node: insertions[0]
        }),
        Err(SessionError::Edit(EditError::Refused { .. }))
    ));
    edit(
        &mut doc,
        Command::RemoveNode {
            node: insertions[1],
        },
    );
    assert_eq!(texts(&reopen("bookmarks", &doc)), ["ab"]);
}

#[test]
fn replacing_controls_removes_a_bookmark_only_when_both_ends_are_selected() {
    let bytes = with_body(concat!(
        "<w:p><w:r><w:t>left</w:t></w:r>",
        r#"<w:sdt><w:sdtPr/><w:sdtContent><w:bookmarkStart w:id="5" w:name="across"/><w:r><w:t>a</w:t></w:r></w:sdtContent></w:sdt>"#,
        r#"<w:sdt><w:sdtPr/><w:sdtContent><w:r><w:t>b</w:t></w:r><w:bookmarkEnd w:id="5"/></w:sdtContent></w:sdt>"#,
        "<w:r><w:t>right</w:t></w:r></w:p>"
    ));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let controls: Vec<_> = doc
        .descendants(doc.paragraphs()[0])
        .into_iter()
        .filter(|&id| matches!(doc.node(id).unwrap().kind, NodeKind::ContentControl { .. }))
        .collect();
    let [first, second] = controls[..] else {
        panic!("expected two controls")
    };
    let replace = |before, after| Command::ReplaceRange {
        from: Point::Before { before },
        to: Point::After { after },
        text: "X".into(),
        track: None,
    };
    for half in [first, second] {
        assert!(doc
            .run(replace(half, half))
            .unwrap_err()
            .to_string()
            .contains("bookmark"));
        assert!(!doc.0.undo());
    }
    edit(&mut doc, replace(first, second));
    let out = doc.export().unwrap();
    assert_eq!(texts(&Document::open(&out).unwrap()), ["leftXright"]);
    assert!(!part(&out, "word/document.xml").contains("w:bookmark"));
    assert_editor_undo("bookmark across controls", &bytes, &mut doc);
}

#[test]
fn enter_keeps_a_heading_bookmark_with_the_heading_text() {
    let bytes = with_body(concat!(
        r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:bookmarkStart w:id="0" w:name="_Toc1"/>"#,
        r#"<w:r><w:t>Title</w:t></w:r><w:bookmarkEnd w:id="0"/></w:p>"#
    ));
    for (offset, expected) in [
        (
            5,
            r#"Title</w:t></w:r><w:bookmarkEnd w:id="0"/></w:p><w:p></w:p>"#,
        ),
        (
            0,
            r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr></w:p><w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:bookmarkStart"#,
        ),
    ] {
        let mut doc = EditingDocument::open(&bytes).unwrap();
        let node = text_node(&doc, "Title");
        edit(
            &mut doc,
            Command::SplitParagraph {
                at: Point::Text { node, offset },
            },
        );
        let xml = part(&doc.export().unwrap(), "word/document.xml");
        assert!(xml.contains(expected), "offset {offset}: {xml}");
        assert_editor_undo("heading-bookmark", &bytes, &mut doc);
    }
}

#[test]
fn typing_in_an_empty_paragraph_takes_its_mark_formatting() {
    let bytes = with_body(r#"<w:p><w:pPr><w:rPr><w:b/></w:rPr></w:pPr></w:p>"#);
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let paragraph = doc.paragraphs()[0];
    edit(
        &mut doc,
        Command::InsertText {
            at: Point::Start { start: paragraph },
            text: "x".into(),
        },
    );
    let xml = part(&doc.export().unwrap(), "word/document.xml");
    assert!(
        xml.contains(r#"</w:pPr><w:r><w:rPr><w:b/></w:rPr><w:t"#),
        "{xml}"
    );

    // A mark under revision keeps its formatting to itself.
    for revised in [
        r#"<w:ins w:id="1" w:author="A" w:date="2026-11-01T00:00:00Z"/>"#,
        r#"<w:rPrChange w:id="1" w:author="A" w:date="2026-11-01T00:00:00Z"><w:rPr/></w:rPrChange>"#,
    ] {
        let bytes = with_body(&format!(
            r#"<w:p><w:pPr><w:rPr>{revised}<w:b/></w:rPr></w:pPr></w:p>"#
        ));
        let mut doc = EditingDocument::open(&bytes).unwrap();
        let paragraph = doc.paragraphs()[0];
        edit(
            &mut doc,
            Command::InsertText {
                at: Point::Start { start: paragraph },
                text: "x".into(),
            },
        );
        let xml = part(&doc.export().unwrap(), "word/document.xml");
        assert!(xml.contains(r#"</w:pPr><w:r><w:t"#), "{xml}");
    }
}

#[test]
fn locked_content_controls_and_last_paragraphs_stay() {
    let bytes = with_body(concat!(
        r#"<w:p><w:sdt><w:sdtPr><w:lock w:val="sdtLocked"/></w:sdtPr><w:sdtContent><w:r><w:t>locked</w:t></w:r></w:sdtContent></w:sdt>"#,
        r#"<w:sdt><w:sdtContent><w:r><w:t>free</w:t></w:r></w:sdtContent></w:sdt><w:r><w:t>x</w:t></w:r></w:p>"#
    ));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let controls: Vec<NodeId> = doc
        .descendants(doc.paragraphs()[0])
        .into_iter()
        .filter(|&id| matches!(doc.node(id).unwrap().kind, NodeKind::ContentControl { .. }))
        .collect();
    assert!(matches!(
        doc.run(Command::RemoveNode { node: controls[0] }),
        Err(SessionError::Edit(EditError::Refused { .. }))
    ));
    edit(&mut doc, Command::RemoveNode { node: controls[1] });
    assert_eq!(texts(&reopen("locked", &doc)), ["lockedx"]);

    let bytes = with_body(concat!(
        r#"<w:sdt><w:sdtContent><w:p><w:r><w:t>only</w:t></w:r></w:p></w:sdtContent></w:sdt>"#,
        r#"<w:tbl><w:tr><w:tc><w:tbl><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>"#,
        r#"<w:sdt><w:sdtContent><w:p/></w:sdtContent></w:sdt></w:tc></w:tr></w:tbl>"#
    ));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let blocks: Vec<NodeId> = doc
        .stories()
        .flat_map(|s| doc.descendants(s.id))
        .filter(|&id| matches!(doc.node(id).unwrap().kind, NodeKind::ContentControl { .. }))
        .collect();
    assert_eq!(blocks.len(), 2);
    for node in blocks {
        assert!(matches!(
            doc.run(Command::RemoveNode { node }),
            Err(SessionError::Edit(EditError::Refused { .. }))
        ));
    }
    assert!(!doc.is_dirty());
}

#[test]
fn a_refused_split_inside_a_field_result_leaves_the_document_clean() {
    let bytes = with_body(concat!(
        r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> TOC </w:instrText></w:r>"#,
        r#"<w:r><w:fldChar w:fldCharType="separate"/></w:r><w:hyperlink w:anchor="_Toc1"><w:r><w:t>Entry</w:t></w:r>"#,
        r#"<w:r><w:tab/></w:r><w:r><w:t>1</w:t></w:r></w:hyperlink></w:p><w:p><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#
    ));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let tab = find(&doc, |k| matches!(k, NodeKind::Tab));
    let tab_run = doc.node(tab).unwrap().parent.unwrap();
    let result = doc.run(Command::SplitParagraph {
        at: Point::Before { before: tab_run },
    });
    assert!(
        matches!(
            result,
            Err(SessionError::Unsupported { .. }
                | SessionError::Edit(EditError::Refused { .. } | EditError::ReadOnly { .. }))
        ),
        "{result:?}"
    );
    assert!(!doc.is_dirty());
}

/// Each story's and table cell's children, by flow.
fn replace_range(from: Point, to: Point, text: &str) -> Command {
    Command::ReplaceRange {
        from,
        to,
        text: text.into(),
        track: None,
    }
}

#[test]
fn headless_undo_and_redo_restore_each_intermediate_package() {
    let mut doc = EditingDocument::open(&generated("simple-paragraphs")).unwrap();
    let mut history = vec![entries(&doc.export().unwrap())];
    for (needle, start, end, replacement) in [
        ("First", 0, 5, "One"),
        ("Second", 3, 3, "\n"),
        ("Third", 0, 5, ""),
    ] {
        let node = text_node(&doc, needle);
        edit(
            &mut doc,
            replace_range(at(node, start), at(node, end), replacement),
        );
        let parts = entries(&doc.export().unwrap());
        assert_ne!(history.last(), Some(&parts));
        history.push(parts);
    }
    for expected in history[..history.len() - 1].iter().rev() {
        assert!(doc.0.undo());
        assert_eq!(&entries(&doc.export().unwrap()), expected);
    }
    assert!(!doc.0.undo());
    for expected in &history[1..] {
        assert!(doc.0.redo());
        assert_eq!(&entries(&doc.export().unwrap()), expected);
    }
    assert!(!doc.0.redo());
}

#[test]
fn replace_range_deletes_across_paragraphs_and_undo_restores_them() {
    let bytes = generated("simple-paragraphs");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let before = texts(&doc);
    let first = paragraph_with(&doc, "First");
    let (from, to) = (text_node(&doc, "First"), text_node(&doc, "Third"));
    edit(&mut doc, replace_range(at(from, 2), at(to, 3), ""));
    assert_eq!(doc.text(first).unwrap(), "Fird paragraph.");
    let mut expected = vec!["Fird paragraph.".to_string()];
    expected.extend(before[3..].iter().cloned());
    assert_eq!(texts(&reopen("simple-paragraphs", &doc)), expected);
    assert_editor_undo("simple-paragraphs", &bytes, &mut doc);
}

#[test]
fn replace_range_types_over_a_selection_and_pastes_paragraphs() {
    let bytes = generated("simple-paragraphs");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let before = texts(&doc);
    let (from, to) = (text_node(&doc, "First"), text_node(&doc, "Second"));
    edit(&mut doc, replace_range(at(from, 2), at(to, 3), "XY\nZ\nW"));
    let mut expected: Vec<String> = ["FiXY", "Z", "Wond paragraph."].map(String::from).to_vec();
    expected.extend(before[2..].iter().cloned());
    assert_eq!(texts(&reopen("simple-paragraphs", &doc)), expected);
    assert_editor_undo("simple-paragraphs", &bytes, &mut doc);

    // An empty range inserts: a paste with no selection.
    let mut doc = EditingDocument::open(&bytes).unwrap();
    edit(&mut doc, replace_range(at(from, 5), at(from, 5), "\nnew\n"));
    let mut expected: Vec<String> = ["First", "new", " paragraph."].map(String::from).to_vec();
    expected.extend(before[1..].iter().cloned());
    assert_eq!(texts(&reopen("simple-paragraphs", &doc)), expected);
    assert_editor_undo("simple-paragraphs", &bytes, &mut doc);
}

#[test]
fn replace_range_preserves_the_surviving_run_fragments() {
    let bytes = with_body(concat!(
        r#"<w:p><w:r><w:t>one </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>bold</w:t></w:r>"#,
        r#"<w:r><w:t xml:space="preserve"> two</w:t><w:tab/><w:t>three</w:t></w:r></w:p>"#
    ));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let paragraph = doc.paragraphs()[0];
    let (from, to) = (text_node(&doc, "one"), text_node(&doc, "three"));
    edit(&mut doc, replace_range(at(from, 2), at(to, 2), ""));
    assert_eq!(doc.text(paragraph).unwrap(), "onree");
    // Only the two surviving character stretches have runs.
    let runs: Vec<String> = doc
        .node(paragraph)
        .unwrap()
        .children
        .iter()
        .map(|&c| {
            doc.descendants(c)
                .into_iter()
                .filter_map(|d| match &doc.node(d).unwrap().kind {
                    NodeKind::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .collect::<String>()
        })
        .collect();
    assert_eq!(runs, ["on", "ree"]);
    assert_eq!(texts(&reopen("runs", &doc)), ["onree"]);
    assert_editor_undo("runs", &bytes, &mut doc);

    // Within one text node it is a text change.
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let bold = text_node(&doc, "bold");
    edit(&mut doc, replace_range(at(bold, 1), at(bold, 3), "ar"));
    assert_eq!(texts(&reopen("runs", &doc)), ["one bard two\tthree"]);
}

#[test]
fn replace_range_removes_a_table_between_paragraphs() {
    let bytes = with_body(concat!(
        r#"<w:p><w:r><w:t>Before</w:t></w:r></w:p>"#,
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
        r#"<w:p><w:r><w:t>After</w:t></w:r></w:p>"#
    ));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let (from, to) = (text_node(&doc, "Before"), text_node(&doc, "After"));
    edit(&mut doc, replace_range(at(from, 3), at(to, 2), ""));
    assert_eq!(texts(&reopen("table", &doc)), ["Befter"]);
    assert_editor_undo("table", &bytes, &mut doc);

    // A selection from outside a table into it is refused; a whole table can be removed.
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let cell = text_node(&doc, "cell");
    assert!(matches!(
        doc.run(replace_range(at(from, 3), at(cell, 2), "")),
        Err(SessionError::Unsupported {
            reason: "cannot replace across stories or table cells",
            ..
        })
    ));
    let table = find(&doc, |k| matches!(k, NodeKind::Table(_)));
    edit(&mut doc, Command::RemoveNode { node: table });
    assert_eq!(texts(&reopen("table", &doc)), ["Before", "After"]);
    assert_editor_undo("table", &bytes, &mut doc);
}

#[test]
fn replace_range_refuses_what_it_cannot_carry_and_changes_nothing() {
    let bytes = generated("complex-mixed");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let original = stories(&doc);
    let blocks = &doc.stories().next().unwrap().children;
    let whole_story = replace_range(
        Point::Before { before: blocks[0] },
        Point::After {
            after: *blocks.last().unwrap(),
        },
        "Replacement",
    );
    assert!(doc
        .run(whole_story)
        .unwrap_err()
        .to_string()
        .contains("ends of the selection inside paragraphs"));
    let heading = find(
        &doc,
        |k| matches!(k, NodeKind::Text { text } if text == "Chapter one"),
    );
    let toc = text_node(&doc, "Chapter one ... 1");
    let kept = text_node(&doc, "Kept text");
    let untouched = text_node(&doc, "Untouched paragraph");
    let landscape = text_node(&doc, "Last paragraph of the landscape");
    let mut refused = |command: Command| {
        let result = doc.run(command.clone());
        assert!(
            matches!(
                result,
                Err(SessionError::Unsupported { .. }
                    | SessionError::Edit(EditError::Refused { .. } | EditError::ReadOnly { .. }))
            ),
            "{command:?}: {result:?}"
        );
    };
    // Backwards.
    refused(replace_range(at(kept, 3), at(heading, 2), ""));
    // Into a field result.
    refused(replace_range(at(toc, 2), at(heading, 2), ""));
    // Over a comment, a note reference and the bookmarks between.
    refused(replace_range(at(heading, 2), at(kept, 2), ""));
    // Across the paragraph mark that ends the first section.
    refused(replace_range(at(untouched, 2), at(landscape, 2), ""));
    assert_eq!(stories(&doc), original);
    assert!(!doc.is_dirty());
}

#[test]
fn every_fixture_replaces_ranges_undoes_and_redoes_exactly() {
    let mut replaced = 0;
    for (name, bytes) in all_fixtures() {
        let doc = Document::open(&bytes).unwrap();
        let editable = |doc: &Document, p: NodeId| {
            doc.descendants(p).into_iter().find(|&id| {
                let node = doc.node(id).unwrap();
                node.support == docx_engine::Support::Editable
                    && matches!(&node.kind, NodeKind::Text { text } if text.chars().count() >= 2)
            })
        };
        let paragraphs = doc.paragraphs();
        for pair in paragraphs.windows(3) {
            let (Some(from), Some(to)) = (editable(&doc, pair[0]), editable(&doc, pair[2])) else {
                continue;
            };
            let mut doc = EditingDocument::open(&bytes).unwrap();
            let Ok(()) = doc.0.apply(&replace_range(at(from, 1), at(to, 1), "x\ny")) else {
                assert_eq!(
                    stories(&doc),
                    stories(&Document::open(&bytes).unwrap()),
                    "{name}: refusal changed the document"
                );
                assert!(!doc.is_dirty(), "{name}: refusal dirtied the document");
                continue;
            };
            replaced += 1;
            reopen(&name, &doc);
            assert_editor_undo(&name, &bytes, &mut doc);
        }
    }
    assert!(replaced > 50, "only {replaced} ranges replaced");
}

/// A split's copy is like the paragraph it was split from, and a run made in
/// it for typed text is like that paragraph's mark.
#[test]
fn a_split_and_typing_keep_the_source_formatting() {
    let bytes = generated("rich-text");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let paragraph = paragraph_with(&doc, "Bold");
    let bold = text_node(&doc, "Bold");
    let run = doc.node(bold).unwrap().parent.unwrap();
    edit(&mut doc, Command::SplitParagraph { at: at(bold, 2) });
    let paragraphs = doc.paragraphs();
    let second = paragraphs[paragraphs.iter().position(|&p| p == paragraph).unwrap() + 1];
    let tail = doc.node(second).unwrap().children[0];
    assert_eq!(doc.effective_run_props(tail), doc.effective_run_props(run));

    // Typing into the copy of an empty paragraph makes a run like its mark,
    // which in paragraph-layout is 20-point in one of them.
    let bytes = generated("paragraph-layout");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let empties: Vec<_> = doc
        .paragraphs()
        .into_iter()
        .filter(|&p| doc.text(p).unwrap().is_empty())
        .collect();
    let mut sizes = Vec::new();
    for marked in empties {
        edit(
            &mut doc,
            Command::SplitParagraph {
                at: Point::End { end: marked },
            },
        );
        let paragraphs = doc.paragraphs();
        let empty = paragraphs[paragraphs.iter().position(|&p| p == marked).unwrap() + 1];
        edit(
            &mut doc,
            Command::InsertText {
                at: Point::Start { start: empty },
                text: "typed".into(),
            },
        );
        let typed = doc.node(empty).unwrap().children[0];
        sizes.push(doc.effective_run_props(typed).size_half_points);
    }
    assert!(sizes.contains(&Some(40)), "{sizes:?}");
}

fn body_of(bytes: &[u8]) -> String {
    let xml = part(bytes, "word/document.xml");
    let (start, end) = (
        xml.find("<w:body>").unwrap() + 8,
        xml.find("<w:sectPr").unwrap(),
    );
    xml[start..end].to_string()
}

#[test]
fn typing_keeps_the_markup_around_its_run_in_the_view_and_in_a_fresh_export() {
    let bytes = with_body(
        "<w:p><w:r><w:t>one</w:t></w:r><!--between--><w:r><w:t>two</w:t><!--after--></w:r></w:p>",
    );
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let two = text_node(&doc, "two");
    edit(
        &mut doc,
        Command::ReplaceText {
            node: two,
            range: Some(docx_engine::TextRange { start: 3, end: 3 }),
            text: "!".into(),
        },
    );
    let expected = r#"<w:p><w:r><w:t>one</w:t></w:r><!--between--><w:r><w:t xml:space="preserve">two!</w:t><!--after--></w:r></w:p>"#;
    assert_eq!(body_of(&Document::export(&doc).unwrap()), expected, "view");
    assert_eq!(body_of(&doc.export().unwrap()), expected, "fresh export");
}

#[test]
fn a_run_moved_by_a_split_or_a_join_keeps_the_namespace_its_paragraph_declared() {
    let run = r#"<w:r extra:mark="1"><w:t>two</w:t></w:r>"#;
    let split = with_body(&format!(
        r#"<w:p xmlns:extra="urn:extra"><w:pPr><w:rPr extra:mark="2"/></w:pPr><w:r><w:t>one</w:t></w:r>{run}</w:p>"#
    ));
    let mut doc = EditingDocument::open(&split).unwrap();
    let two = text_node(&doc, "two");
    edit(&mut doc, Command::SplitParagraph { at: at(two, 0) });
    let out = doc.export().unwrap();
    assert_valid("split", &out);
    assert_eq!(texts(&Document::open(&out).unwrap()), ["one", "two"]);

    let join = with_body(&format!(
        r#"<w:p><w:r><w:t>one</w:t></w:r></w:p><w:p xmlns:extra="urn:extra">{run}</w:p>"#
    ));
    let mut doc = EditingDocument::open(&join).unwrap();
    let first = paragraph_with(&doc, "one");
    edit(&mut doc, Command::MergeParagraphs { paragraph: first });
    let out = doc.export().unwrap();
    assert_valid("join", &out);
    assert_eq!(texts(&Document::open(&out).unwrap()), ["onetwo"]);
    assert!(body_of(&out).contains(r#"extra:mark="1""#));
}

#[test]
fn joined_runs_and_hyperlinks_keep_their_source_namespace_in_the_destination() {
    let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    let bytes = with_body(&format!(
        r#"<a:p xmlns:a="{w}" xmlns:w="urn:extension"><a:r><a:t>A</a:t></a:r></a:p><w:p xmlns:q="{w}"><w:r><w:t>B</w:t></w:r><q:hyperlink w:anchor="chapter"><w:r><w:t>C</w:t></w:r></q:hyperlink></w:p>"#
    ));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let first = paragraph_with(&doc, "A");
    edit(&mut doc, Command::MergeParagraphs { paragraph: first });
    let check = |reopened: &Document| {
        let link = find(
            reopened,
            |kind| matches!(kind, NodeKind::Hyperlink(info) if info.anchor.as_deref() == Some("chapter")),
        );
        assert!(reopened
            .descendants(link)
            .contains(&text_node(reopened, "C")));
    };
    let joined = reopen("join across conflicting namespaces", &doc);
    assert_eq!(texts(&joined), ["ABC"]);
    check(&joined);
    let split = at(text_node(&doc, "B"), 0);
    edit(&mut doc, Command::SplitParagraph { at: split });
    let reopened = reopen("split after joining", &doc);
    assert_eq!(texts(&reopened), ["A", "BC"]);
    check(&reopened);
}

#[test]
fn embedded_xml_in_text_keeps_all_characters_and_its_source_markup() {
    let bytes = generated("simple-paragraphs");
    let mixed = "<w:t>First<!--keep-->&amp;<?word keep?><![CDATA[ more]]></w:t>";
    let document = part(&bytes, "word/document.xml").replace(
        "<w:t>First paragraph.</w:t>",
        &format!("{mixed}<w:t> next</w:t>"),
    );
    let bytes = rewrite(&bytes, "word/document.xml", Some(document.as_bytes()));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let p = paragraph_with(&doc, "First& more next");
    let t = text_node(&doc, "First& more");
    let replace = |from, to| Command::ReplaceRange {
        from,
        to,
        text: "X".into(),
        track: None,
    };
    assert!(doc.run(replace(at(t, 1), at(t, 2))).is_err());
    assert!(!doc.0.undo());
    let end = Point::End { end: p };
    edit(&mut doc, replace(end, end));
    let out = doc.export().unwrap();
    assert!(part(&out, "word/document.xml").contains(mixed));
    assert_eq!(
        Document::open(&out).unwrap().text(p).unwrap(),
        "First& more nextX"
    );
    assert_editor_undo("mixed text", &bytes, &mut doc);

    edit(
        &mut doc,
        replace(Point::Before { before: t }, Point::After { after: t }),
    );
    assert_eq!(doc.text(p).unwrap(), "X nextX");
    assert!(!part(&doc.export().unwrap(), "word/document.xml").contains("<!--keep-->"));
}

#[test]
fn deleting_a_table_respects_control_locks_at_every_depth() {
    for (lock, may_remove) in [
        ("sdtLocked", false),
        ("sdtContentLocked", false),
        ("contentLocked", true),
        ("unlocked", true),
    ] {
        let wrap = |inner: &str| {
            format!(
                r#"<w:sdt><w:sdtPr><w:lock w:val="{lock}"/></w:sdtPr><w:sdtContent>{inner}</w:sdtContent></w:sdt>"#
            )
        };
        for row in [
            format!("<w:tr><w:tc>{}</w:tc></w:tr>", wrap("<w:p/>")),
            format!("<w:tr>{}</w:tr>", wrap("<w:tc><w:p/></w:tc>")),
            wrap("<w:tr><w:tc><w:p/></w:tc></w:tr>"),
        ] {
            let bytes = with_body(&format!("<w:tbl>{row}</w:tbl><w:p/>"));
            let mut doc = EditingDocument::open(&bytes).unwrap();
            let table = find(&doc, |k| matches!(k, NodeKind::Table(_)));
            let result = doc.run(Command::RemoveNode { node: table });
            if may_remove {
                result.unwrap();
                assert!(!part(&doc.export().unwrap(), "word/document.xml").contains("<w:tbl>"));
            } else {
                assert!(result
                    .unwrap_err()
                    .to_string()
                    .contains("locked against deletion"));
                assert!(!doc.0.undo());
                assert_eq!(doc.export().unwrap(), bytes);
            }
        }
    }
}
