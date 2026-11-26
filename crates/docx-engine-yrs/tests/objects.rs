#[path = "../../docx-engine/tests/common/mod.rs"]
mod common;
use common::*;
use docx_engine::{Command, Document, Fill, NodeId, NodeKind, Point};
use docx_engine_yrs::Replica;
fn sync(a: &mut Replica, b: &mut Replica) {
    let ua = a.update_since(&b.state_vector()).unwrap();
    let ub = b.update_since(&a.state_vector()).unwrap();
    a.apply_update(&ub).unwrap();
    b.apply_update(&ua).unwrap();
    assert_eq!(a.export().unwrap(), b.export().unwrap());
    assert_valid("merged", &a.export().unwrap());
}
fn drawings(doc: &Document) -> Vec<(NodeId, u32)> {
    doc.stories()
        .flat_map(|s| doc.descendants(s.id))
        .filter_map(|id| {
            if let NodeKind::Drawing(info) = &doc.node(id)?.kind {
                Some((id, info.id.unwrap()))
            } else {
                None
            }
        })
        .collect()
}
#[test]
fn concurrent_pictures_have_unique_drawing_ids_and_media_survives_undo() {
    let bytes = generated("simple-paragraphs");
    let mut a = Replica::seed(1, &bytes).unwrap();
    let original = a.export().unwrap();
    let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
    let png = entries(&generated("image"))
        .into_iter()
        .find(|(n, _)| n == "word/media/image1.png")
        .unwrap()
        .1;
    let media = a.media_name(&png).unwrap();
    assert_eq!(b.media_name(&png).unwrap(), media);
    let p = a.view().unwrap().paragraphs()[0];
    let command = Command::InsertPicture {
        to: None,
        at: Point::Start { start: p },
        media: media.clone(),
        width_emu: 914400,
        height_emu: 457200,
        description: Some("A picture".into()),
        // The highest drawing ID Word loads.
        id: Some(2_147_483_646),
    };
    a.apply_all_with_media(std::slice::from_ref(&command), &[&png])
        .unwrap();
    b.apply_all_with_media(std::slice::from_ref(&command), &[&png])
        .unwrap();
    sync(&mut a, &mut b);
    let ds = drawings(a.view().unwrap());
    assert_eq!(ds.len(), 2);
    assert_eq!(
        ds.iter()
            .map(|d| d.1)
            .collect::<std::collections::BTreeSet<_>>(),
        [1, 2_147_483_646].into()
    );
    for replica in [&mut a, &mut b] {
        replica.take_redraw().unwrap();
        replica
            .apply(&Command::InsertText {
                at: Point::End { end: p },
                text: "caption".into(),
            })
            .unwrap();
        let redraw = replica.take_redraw().unwrap();
        assert!(
            matches!(&redraw, docx_engine_yrs::Redraw::Splices(s)
            if s.len() == 1 && s[0].blocks == [p]),
            "{redraw:?}"
        );
        assert_eq!(
            replica.view().unwrap().export().unwrap(),
            replica.export().unwrap()
        );
        assert!(replica.undo().unwrap());
        assert_eq!(
            replica.view().unwrap().export().unwrap(),
            replica.export().unwrap()
        );
    }
    a.undo().unwrap();
    sync(&mut a, &mut b);
    for replica in [&mut a, &mut b] {
        assert_eq!(drawings(replica.view().unwrap()).len(), 1);
        assert_eq!(
            replica.view().unwrap().export().unwrap(),
            replica.export().unwrap()
        );
    }
    b.undo().unwrap();
    sync(&mut a, &mut b);
    assert!(
        a.export().unwrap() == original,
        "undo did not restore original"
    );
    a.apply(&command).unwrap();
    sync(&mut a, &mut b);
    let drawing = drawings(a.view().unwrap())[0].0;
    a.apply(&Command::RemoveNode { node: drawing }).unwrap();
    sync(&mut a, &mut b);
    assert!(drawings(a.view().unwrap()).is_empty());
    a.undo().unwrap();
    assert_eq!(drawings(a.view().unwrap()).len(), 1);
    let mut headless = docx_engine::editing::Editor::open(&bytes).unwrap();
    assert_eq!(headless.media_name(&png).unwrap(), media);
    headless
        .apply_all_with_media(std::slice::from_ref(&command), &[&png])
        .unwrap();
    headless.undo();
    headless.apply(&command).unwrap();
    assert_valid("headless", &headless.export().unwrap());
}
#[test]
fn typing_beside_a_tracked_deleted_picture_redraws_only_its_paragraph() {
    let bytes = generated("image");
    let mut replica = Replica::seed(1, &bytes).unwrap();
    let p = replica.view().unwrap().paragraphs()[0];
    replica
        .apply(&Command::InsertPicture {
            at: Point::Start { start: p },
            to: None,
            media: "word/media/image1.png".into(),
            width_emu: 914400,
            height_emu: 457200,
            description: None,
            id: None,
        })
        .unwrap();
    let drawing = drawings(replica.view().unwrap())[0].0;
    replica
        .apply(&Command::ReplaceRange {
            from: Point::Before { before: drawing },
            to: Point::After { after: drawing },
            text: String::new(),
            track: Some(docx_engine::Track {
                author: "Alice".into(),
                date: "2026-11-23T00:00:00Z".into(),
            }),
        })
        .unwrap();
    replica.take_redraw().unwrap();
    replica
        .apply(&Command::InsertText {
            at: Point::End { end: p },
            text: "caption".into(),
        })
        .unwrap();
    let redraw = replica.take_redraw().unwrap();
    assert!(
        matches!(&redraw, docx_engine_yrs::Redraw::Splices(s) if s.len() == 1 && s[0].blocks == [p]),
        "{redraw:?}"
    );
    assert_eq!(
        replica.view().unwrap().export().unwrap(),
        replica.export().unwrap()
    );
}

fn control(doc: &Document, alias: &str) -> NodeId {
    doc.stories().flat_map(|s| doc.descendants(s.id)).find(|id| matches!(&doc.node(*id).unwrap().kind, NodeKind::ContentControl{alias:Some(a),..} if a==alias)).unwrap()
}
#[test]
fn control_values_match_commands_and_concurrent_fills_remain_complete() {
    let bytes = generated("content-controls");
    let mut a = Replica::seed(1, &bytes).unwrap();
    let original = a.export().unwrap();
    let mut plain = docx_engine::editing::Editor::open(&bytes).unwrap();
    let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
    let fills = [
        ("Name", Fill::Text("Ada".into())),
        ("Name", Fill::Text("Grace".into())),
        ("Agree", Fill::Checked(true)),
        ("Size", Fill::Item("M".into())),
        ("Colour", Fill::Item("Green".into())),
        ("Start date", Fill::Date("2026-11-18".into())),
        ("Address", Fill::Text("One\nTwo".into())),
    ];
    for (alias, fill) in fills {
        let command = Command::FillContentControl {
            control: control(plain.view().unwrap(), alias),
            fill,
        };
        plain.apply(&command).unwrap();
        a.apply(&command).unwrap();
        assert_eq!(
            part(&a.export().unwrap(), "word/document.xml"),
            part(&plain.export().unwrap(), "word/document.xml")
        );
    }
    sync(&mut a, &mut b);
    let id = control(a.view().unwrap(), "Agree");
    a.apply(&Command::FillContentControl {
        control: id,
        fill: Fill::Checked(false),
    })
    .unwrap();
    let name = control(b.view().unwrap(), "Name");
    b.apply(&Command::FillContentControl {
        control: name,
        fill: Fill::Text("Concurrent".into()),
    })
    .unwrap();
    sync(&mut a, &mut b);
    b.undo().unwrap();
    a.undo().unwrap();
    sync(&mut a, &mut b);
    for _ in 0..7 {
        a.undo().unwrap();
    }
    sync(&mut a, &mut b);
    assert!(
        a.export().unwrap() == original,
        "undo did not restore original"
    );
}
#[test]
fn a_joined_paragraph_keeps_its_filled_control_after_typing() {
    let bytes = with_body(
        r#"<w:p><w:r><w:t>First</w:t></w:r></w:p><w:p><w:sdt><w:sdtPr><w:alias w:val="Address"/><w:text/></w:sdtPr><w:sdtContent><w:r><w:t>Old value</w:t></w:r></w:sdtContent></w:sdt></w:p>"#,
    );
    let mut a = Replica::seed(1, &bytes).unwrap();
    let first = a.view().unwrap().paragraphs()[0];
    let address = control(a.view().unwrap(), "Address");
    for command in [
        Command::FillContentControl {
            control: address,
            fill: Fill::Text("1 New Street".into()),
        },
        Command::MergeParagraphs { paragraph: first },
        Command::InsertText {
            at: Point::Start { start: first },
            text: "X".into(),
        },
    ] {
        a.apply(&command).unwrap();
        let live = part(&a.view().unwrap().export().unwrap(), "word/document.xml");
        assert!(live.contains("1 New Street"));
        assert_eq!(live, part(&a.export().unwrap(), "word/document.xml"));
    }
}
#[test]
fn a_fill_of_a_removed_control_waits_for_the_control_to_return() {
    let bytes = generated("content-controls");
    let mut a = Replica::seed(1, &bytes).unwrap();
    let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
    let address = control(a.view().unwrap(), "Address");
    let fill = Command::FillContentControl {
        control: address,
        fill: Fill::Text("1 New Street".into()),
    };
    a.apply(&fill).unwrap();
    b.apply(&Command::RemoveNode { node: address }).unwrap();
    sync(&mut a, &mut b);
    assert!(!part(&a.export().unwrap(), "word/document.xml").contains("1 New Street"));
    b.undo().unwrap();
    sync(&mut a, &mut b);
    assert!(part(&a.export().unwrap(), "word/document.xml").contains("1 New Street"));

    let mut alone = docx_engine::editing::Editor::open(&bytes).unwrap();
    alone.apply(&fill).unwrap();
    alone.apply(&Command::RemoveNode { node: address }).unwrap();
    assert!(!part(&alone.export().unwrap(), "word/document.xml").contains("1 New Street"));
    alone.undo();
    assert!(part(&alone.export().unwrap(), "word/document.xml").contains("1 New Street"));
}
#[test]
fn fields_and_settings_share_undo_and_a_refused_batch_writes_nothing() {
    let bytes = with_body(
        r#"<w:p><w:r><w:t>Before </w:t></w:r><w:fldSimple w:instr="DATE"><w:r><w:t>old</w:t></w:r></w:fldSimple></w:p>"#,
    );
    let mut a = Replica::seed(1, &bytes).unwrap();
    let original = a.export().unwrap();
    let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
    let doc = a.view().unwrap();
    let field = doc
        .paragraphs()
        .into_iter()
        .flat_map(|p| doc.descendants(p))
        .find(|id| matches!(doc.node(*id).unwrap().kind, NodeKind::Field { .. }))
        .unwrap();
    a.apply(&Command::SetFieldResult {
        field,
        text: "new".into(),
    })
    .unwrap();
    b.apply(&Command::SetUpdateFields { on: true }).unwrap();
    sync(&mut a, &mut b);
    assert_eq!(
        a.view().unwrap().field_result(field).as_deref(),
        Some("new")
    );
    assert!(a.view().unwrap().settings().update_fields);
    let before = a.save();
    let text = text_node(a.view().unwrap(), "Before");
    assert!(a
        .apply_all(&[
            Command::SetFieldResult {
                field,
                text: "bad".into()
            },
            Command::RemoveNode { node: text }
        ])
        .is_err());
    assert_eq!(before, a.save());
    a.undo().unwrap();
    b.undo().unwrap();
    sync(&mut a, &mut b);
    assert!(
        a.export().unwrap() == original,
        "undo did not restore original"
    );
    a.apply(&Command::UpdateFields {
        now: "2026-11-18T09:00:00".into(),
        pages: None,
    })
    .unwrap();
    sync(&mut a, &mut b);
    assert_ne!(
        a.view().unwrap().field_result(field).as_deref(),
        Some("old")
    );
}
#[test]
fn breaks_are_single_items_and_remove_undo_restores_them() {
    let bytes = generated("simple-paragraphs");
    let mut a = Replica::seed(1, &bytes).unwrap();
    let original = a.export().unwrap();
    let p = a.view().unwrap().paragraphs()[0];
    a.apply(&Command::InsertBreak {
        to: None,
        at: Point::Start { start: p },
    })
    .unwrap();
    let br = a
        .view()
        .unwrap()
        .descendants(p)
        .into_iter()
        .find(|id| {
            matches!(
                a.view().unwrap().node(*id).unwrap().kind,
                NodeKind::Break { .. }
            )
        })
        .unwrap();
    a.apply(&Command::RemoveNode { node: br }).unwrap();
    assert_eq!(a.view().unwrap().text(p).unwrap(), "First paragraph.");
    a.undo().unwrap();
    a.undo().unwrap();
    assert_eq!(a.export().unwrap(), original);
}

#[test]
fn a_picture_replaces_a_selection_as_one_intent_and_one_undo() {
    let bytes = generated("simple-paragraphs");
    let mut a = Replica::seed(1, &bytes).unwrap();
    let original = a.export().unwrap();
    let png = entries(&generated("image"))
        .into_iter()
        .find(|(n, _)| n == "word/media/image1.png")
        .unwrap()
        .1;
    let media = a.media_name(&png).unwrap();
    let node = text_node(a.view().unwrap(), "Second");
    let command = Command::InsertPicture {
        at: at(node, 2),
        to: Some(at(node, 6)),
        media,
        width_emu: 914400,
        height_emu: 457200,
        description: None,
        id: None,
    };
    a.apply_all_with_media(&[command], &[&png]).unwrap();
    let doc = a.view().unwrap();
    assert_eq!(doc.text(doc.paragraphs()[1]).unwrap(), "Se paragraph.");
    assert_eq!(drawings(a.view().unwrap()).len(), 1);
    let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
    sync(&mut a, &mut b);
    a.undo().unwrap();
    sync(&mut a, &mut b);
    assert!(a.export().unwrap() == original);
}

#[test]
fn break_replaces_a_multi_paragraph_selection_atomically_and_undo_restores_it() {
    let bytes = generated("simple-paragraphs");
    let mut a = Replica::seed(1, &bytes).unwrap();
    let before = a.export().unwrap();
    let codec = Document::open(&bytes).unwrap();
    let ps = codec.paragraphs();
    let text = |p| {
        codec
            .descendants(p)
            .into_iter()
            .find(|id| matches!(codec.node(*id).unwrap().kind, NodeKind::Text { .. }))
            .unwrap()
    };
    let command = Command::InsertBreak {
        at: Point::Text {
            node: text(ps[0]),
            offset: 3,
        },
        to: Some(Point::Text {
            node: text(ps[1]),
            offset: 4,
        }),
    };
    a.apply(&command).unwrap();

    let p = a.view().unwrap().paragraphs()[0];
    assert_eq!(
        a.view().unwrap().text(p),
        Some("Fir\nnd paragraph.".to_string())
    );
    assert!(a
        .view()
        .unwrap()
        .descendants(p)
        .into_iter()
        .any(|id| matches!(
            a.view().unwrap().node(id).unwrap().kind,
            NodeKind::Break { .. }
        )));
    assert_valid("break selection", &a.export().unwrap());
    a.undo().unwrap();
    assert!(a.export().unwrap() == before);
}

#[test]
fn refused_picture_batch_retains_no_bytes_or_history_and_sends_nothing() {
    let bytes = generated("simple-paragraphs");
    let mut a = Replica::seed(1, &bytes).unwrap();
    let png = entries(&generated("image"))
        .into_iter()
        .find(|(n, _)| n == "word/media/image1.png")
        .unwrap()
        .1;
    let p = a.view().unwrap().paragraphs()[0];
    a.apply(&Command::InsertText {
        at: Point::Start { start: p },
        text: "X".into(),
    })
    .unwrap();
    a.undo().unwrap();
    a.take_update().unwrap();
    let before = a.save();
    let original = a.export().unwrap();
    let media = a.media_name(&png).unwrap();
    assert_eq!(a.save(), before, "preparation wrote shared state");
    let command = Command::InsertPicture {
        at: Point::Start { start: p },
        to: None,
        media,
        width_emu: 0,
        height_emu: 457200,
        description: None,
        id: None,
    };
    assert!(a
        .apply_all_with_media(std::slice::from_ref(&command), &[&png])
        .is_err());
    assert_eq!(a.save(), before);
    // Also refuse after a valid picture has already been prepared privately.
    let mut valid = command.clone();
    if let Command::InsertPicture { width_emu, .. } = &mut valid {
        *width_emu = 914400;
    }
    assert!(a
        .apply_all_with_media(&[valid, command.clone()], &[&png])
        .is_err());
    assert_eq!(a.save(), before);
    assert_eq!(a.export().unwrap(), original);
    assert!(a.take_update().unwrap().is_none());
    assert!(!a.undo().unwrap());
    assert!(a.take_update().unwrap().is_none());
    assert!(a.redo().unwrap(), "refusal erased redo");
    a.take_update().unwrap();
    assert!(!a.redo().unwrap());
    assert!(a.take_update().unwrap().is_none());
    let mut headless = docx_engine::editing::Editor::open(&bytes).unwrap();
    let original = headless.export().unwrap();
    assert!(headless.apply_all_with_media(&[command], &[&png]).is_err());
    assert_eq!(headless.export().unwrap(), original);
    assert!(!headless.undo());
}

#[test]
fn a_body_containing_only_a_control_is_preserved_and_can_be_filled() {
    let bytes = with_body(
        r#"<w:sdt><w:sdtPr><w:alias w:val="Only"/><w:text/></w:sdtPr><w:sdtContent><w:p><w:r><w:t>Kept content</w:t></w:r></w:p></w:sdtContent></w:sdt>"#,
    );
    assert_valid("source", &bytes);
    let mut a = Replica::seed(1, &bytes).unwrap();
    let id = control(a.view().unwrap(), "Only");
    assert_eq!(
        part(&a.export().unwrap(), "word/document.xml"),
        part(&bytes, "word/document.xml")
    );
    let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
    a.apply(&Command::FillContentControl {
        control: id,
        fill: Fill::Text("Still kept".into()),
    })
    .unwrap();
    sync(&mut a, &mut b);
    assert!(part(&a.export().unwrap(), "word/document.xml").contains("Still kept"));
    a.undo().unwrap();
    sync(&mut a, &mut b);
    assert_eq!(
        part(&a.export().unwrap(), "word/document.xml"),
        part(&bytes, "word/document.xml")
    );
}

#[test]
fn prepared_picture_preserves_the_current_text_address() {
    let bytes = generated("simple-paragraphs");
    let png = entries(&generated("image"))
        .into_iter()
        .find(|(name, _)| name == "word/media/image1.png")
        .unwrap()
        .1;
    let mut a = Replica::seed(1, &bytes).unwrap();
    let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
    let p = a.view().unwrap().paragraphs()[0];
    a.apply(&Command::InsertText {
        at: Point::Start { start: p },
        text: "Typed ".into(),
    })
    .unwrap();
    let before = a.export().unwrap();
    let node = text_node(a.view().unwrap(), "Typed ");
    let command = Command::InsertPicture {
        at: at(node, 3),
        to: None,
        media: a.media_name(&png).unwrap(),
        width_emu: 914400,
        height_emu: 457200,
        description: None,
        id: None,
    };
    a.apply_all_with_media(&[command], &[&png]).unwrap();
    sync(&mut a, &mut b);
    assert_eq!(drawings(a.view().unwrap()).len(), 1);
    let xml = part(&a.export().unwrap(), "word/document.xml");
    assert!(xml.find("Typ</").unwrap() < xml.find(":drawing>").unwrap());
    assert!(xml.find(":drawing>").unwrap() < xml.find("ed ").unwrap());
    a.undo().unwrap();
    sync(&mut a, &mut b);
    assert_eq!(a.export().unwrap(), before);
    a.redo().unwrap();
    sync(&mut a, &mut b);
    assert_eq!(drawings(a.view().unwrap()).len(), 1);
    let mut headless = docx_engine::editing::Editor::open(&bytes).unwrap();
    let p = headless.view().unwrap().paragraphs()[0];
    headless
        .apply(&Command::InsertText {
            at: Point::Start { start: p },
            text: "Typed ".into(),
        })
        .unwrap();
    let node = text_node(headless.view().unwrap(), "Typed ");
    let media = headless.media_name(&png).unwrap();
    headless
        .apply_all_with_media(
            &[Command::InsertPicture {
                at: at(node, 3),
                to: None,
                media,
                width_emu: 914400,
                height_emu: 457200,
                description: None,
                id: None,
            }],
            &[&png],
        )
        .unwrap();
    assert_eq!(
        part(&headless.export().unwrap(), "word/document.xml"),
        part(&a.export().unwrap(), "word/document.xml")
    );
}

#[test]
fn unused_supplied_media_is_refused_without_a_write() {
    let bytes = generated("simple-paragraphs");
    let png = entries(&generated("image"))
        .into_iter()
        .find(|(name, _)| name == "word/media/image1.png")
        .unwrap()
        .1;
    let mut a = Replica::seed(1, &bytes).unwrap();
    let p = a.view().unwrap().paragraphs()[0];
    let command = Command::InsertText {
        at: Point::Start { start: p },
        text: "x".into(),
    };
    a.apply(&command).unwrap();
    a.undo().unwrap();
    let saved = a.save();
    for commands in [vec![], vec![command]] {
        assert!(a.apply_all_with_media(&commands, &[&png]).is_err());
        assert_eq!(a.save(), saved);
    }
    assert!(a.redo().unwrap());
}

#[test]
fn states_without_cell_lifetime_identity_require_their_original_editor() {
    use yrs::updates::decoder::Decode;
    use yrs::{Doc, Map, ReadTxn, StateVector, Transact};
    let bytes = generated("simple-paragraphs");
    let a = Replica::seed(1, &bytes).unwrap();
    let saved = a.save();
    for format in [
        "docx-engine.yrs/3",
        "docx-engine.yrs/4",
        "docx-engine.yrs/5",
        "docx-engine.yrs/6",
    ] {
        let old = Doc::with_client_id(9);
        old.transact_mut()
            .apply_update(yrs::Update::decode_v1(&saved).unwrap())
            .unwrap();
        old.get_or_insert_map("meta")
            .insert(&mut old.transact_mut(), "format", format);
        let saved = old
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        assert!(matches!(
            Replica::join(2, &bytes, &saved),
            Err(docx_engine_yrs::CollabError::Unsupported {
                command: "join",
                ..
            })
        ));
    }
}
