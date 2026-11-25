#[path = "../../docx-engine/tests/common/mod.rs"]
mod common;
use common::*;
use docx_engine::{Command, Format, Point, TextRange};
use docx_engine_yrs::{Redraw, Replica};
fn fresh(d: &mut Replica) {
    let shown = d.view().unwrap().export().unwrap();
    let rebuilt = d.export().unwrap();
    assert!(shown == rebuilt, "cached and fresh exports differ");
}
#[test]
fn typing_rebuilds_one_paragraph_and_matches_fresh_after_undo_remote_edits_and_formatting() {
    let bytes = generated("simple-paragraphs");
    let mut a = Replica::seed(1, &bytes).unwrap();
    let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
    let ps = a.view().unwrap().paragraphs();
    a.take_redraw().unwrap();
    for text in ["x", "🦀", "more"] {
        a.apply(&Command::InsertText {
            at: Point::End { end: ps[0] },
            text: text.into(),
        })
        .unwrap();
        fresh(&mut a);
        let redraw = a.take_redraw().unwrap();
        assert!(
            matches!(redraw,Redraw::Splices(ref v) if v.len()==1&&v[0].blocks==[ps[0]]),
            "{redraw:?}"
        );
    }
    b.view().unwrap();
    b.take_redraw().unwrap();
    b.apply_update(&a.update_since(&b.state_vector()).unwrap())
        .unwrap();
    fresh(&mut b);
    assert!(matches!(b.take_redraw().unwrap(), Redraw::Splices(_)));
    for _ in 0..3 {
        a.undo().unwrap();
        fresh(&mut a);
    }
    for _ in 0..3 {
        a.redo().unwrap();
        fresh(&mut a);
    }
    a.apply(&Command::FormatRange {
        from: Point::Start { start: ps[0] },
        to: Point::End { end: ps[0] },
        format: Format {
            bold: Some(true),
            ..Default::default()
        },
    })
    .unwrap();
    fresh(&mut a);
    a.apply(&Command::InsertText {
        at: Point::End { end: ps[0] },
        text: "Bold".into(),
    })
    .unwrap();
    fresh(&mut a);
}
#[test]
fn typing_after_a_large_paste_keeps_unchanged_paragraph_nodes_and_matches_fresh() {
    let bytes = generated("simple-paragraphs");
    let mut a = Replica::seed(1, &bytes).unwrap();
    let p = a.view().unwrap().paragraphs()[0];
    a.apply(&Command::InsertText {
        at: Point::Start { start: p },
        text: "pasted\n".repeat(1000),
    })
    .unwrap();
    let ps = a.view().unwrap().paragraphs();
    a.take_redraw().unwrap();
    a.apply(&Command::InsertText {
        at: Point::Start { start: ps[500] },
        text: "middle".into(),
    })
    .unwrap();
    assert_eq!(a.view().unwrap().paragraphs(), ps);
    fresh(&mut a);
    assert!(matches!(a.take_redraw().unwrap(),Redraw::Splices(ref v) if v.len()==1));
}
#[test]
fn several_text_edits_in_one_batch_keep_global_positions_correct() {
    let bytes = generated("simple-paragraphs");
    let mut a = Replica::seed(1, &bytes).unwrap();
    let ps = a.view().unwrap().paragraphs();
    let node = a
        .view()
        .unwrap()
        .descendants(ps[0])
        .into_iter()
        .find(|id| {
            matches!(
                a.view().unwrap().node(*id).unwrap().kind,
                docx_engine::NodeKind::Text { .. }
            )
        })
        .unwrap();
    a.take_redraw().unwrap();
    a.apply_all(&[
        Command::ReplaceText {
            node,
            range: Some(TextRange { start: 0, end: 3 }),
            text: "Longer".into(),
        },
        Command::InsertText {
            at: Point::End { end: ps[1] },
            text: "Next".into(),
        },
    ])
    .unwrap();
    assert!(matches!(a.take_redraw().unwrap(), Redraw::All));
    fresh(&mut a);
}

#[test]
fn a_split_redraws_the_two_resulting_blocks_in_a_plain_story() {
    let bytes = generated("simple-paragraphs");
    let mut engine = Replica::seed(23, &bytes).unwrap();
    let node = text_node(engine.view().unwrap(), "Second");
    engine.take_redraw().unwrap();
    engine
        .apply(&Command::SplitParagraph { at: at(node, 2) })
        .unwrap();
    let redrawn = engine.take_redraw().unwrap();
    assert!(
        matches!(&redrawn, Redraw::Splices(v) if v.len() == 1 && v[0].at == 1 && v[0].removed == 1 && v[0].blocks.len() == 2),
        "{redrawn:?}"
    );
    fresh(&mut engine);
}

#[test]
fn redundant_list_commands_leave_only_the_pending_split_to_redraw() {
    let mut replica = Replica::seed(23, &generated("simple-paragraphs")).unwrap();
    let paragraph = replica.view().unwrap().paragraphs()[0];
    let at = Point::End { end: paragraph };
    replica.apply(&Command::SplitParagraph { at }).unwrap();
    let inserted = replica.view().unwrap().paragraphs()[1];
    let commands = [
        Command::SetList {
            from: at,
            to: at,
            kind: None,
        },
        Command::SetNumbering {
            from: at,
            to: at,
            numbering: None,
        },
    ];

    replica.apply(&commands[0]).unwrap();
    let pending = replica.take_redraw().unwrap();
    assert!(
        matches!(&pending, Redraw::Splices(s) if s.len() == 1
        && s[0].at == 1 && s[0].removed == 0 && s[0].blocks == [inserted]),
        "{pending:?}"
    );
    for command in &commands {
        replica.apply(command).unwrap();
        assert_eq!(replica.take_redraw().unwrap(), Redraw::Splices(vec![]));
    }
}

#[test]
fn an_undone_split_does_not_emit_redraws_for_removed_blocks() {
    let mut engine = Replica::seed(23, &generated("simple-paragraphs")).unwrap();
    let node = text_node(engine.view().unwrap(), "Second");
    engine.take_redraw().unwrap();
    engine
        .apply(&Command::SplitParagraph { at: at(node, 2) })
        .unwrap();
    engine.undo().unwrap();
    assert!(matches!(engine.take_redraw().unwrap(), Redraw::All));
    fresh(&mut engine);
}

/// Splits at `needle`, types and breaks a line in the new paragraph, and
/// takes all three back. No step may fall back to a full redraw, and each
/// must read as a fresh build.
fn split_type_undo(fixture: &str, needle: &str, offset: usize) {
    let bytes = generated(fixture);
    let mut author = Replica::seed(31, &bytes).unwrap();
    let mut peer = Replica::join(32, &bytes, &author.save()).unwrap();
    let original = author.export().unwrap();
    let node = text_node(author.view().unwrap(), needle);
    author.take_redraw().unwrap();
    peer.take_redraw().unwrap();
    author
        .apply(&Command::SplitParagraph {
            at: at(node, offset),
        })
        .unwrap();
    let split = author.take_redraw().unwrap();
    assert!(
        matches!(&split, Redraw::Splices(v) if v.len() == 1 && v[0].removed == 1 && v[0].blocks.len() == 2),
        "{fixture}: split {split:?}"
    );
    fresh(&mut author);
    let second = match &split {
        Redraw::Splices(v) => v[0].blocks[1],
        Redraw::All => unreachable!(),
    };
    author
        .apply(&Command::InsertText {
            at: Point::Start { start: second },
            text: "typed".into(),
        })
        .unwrap();
    assert!(
        matches!(author.take_redraw().unwrap(), Redraw::Splices(v) if v.len() == 1),
        "{fixture}: typing"
    );
    fresh(&mut author);
    author
        .apply(&Command::InsertBreak {
            at: Point::End { end: second },
            to: None,
        })
        .unwrap();
    let broken = author.take_redraw().unwrap();
    assert!(
        matches!(&broken, Redraw::Splices(v) if v.len() == 1),
        "{fixture}: line break {broken:?}"
    );
    fresh(&mut author);
    peer.apply_update(&author.take_update().unwrap().unwrap())
        .unwrap();
    let remote = peer.take_redraw().unwrap();
    assert!(
        matches!(&remote, Redraw::Splices(v) if v.len() == 1 && v[0].blocks.len() == 2),
        "{fixture}: remote {remote:?}"
    );
    fresh(&mut peer);
    assert!(author.export().unwrap() == peer.export().unwrap());
    for step in ["line break", "typing", "split"] {
        assert!(author.undo().unwrap());
        let undone = author.take_redraw().unwrap();
        assert!(
            matches!(&undone, Redraw::Splices(v) if v.len() == 1),
            "{fixture}: undo of {step} {undone:?}"
        );
        fresh(&mut author);
    }
    assert!(author.export().unwrap() == original, "{fixture}: undone");
}

#[test]
fn a_split_rebuilds_one_strand_beside_other_stories_lists_tables_and_formatting() {
    split_type_undo("header-footer", "Body with", 4);
    split_type_undo("rich-text", "Before tab", 7);
    split_type_undo("table", "Ada", 1);
    split_type_undo("complex-mixed", "the example site", 4);
}

#[test]
fn a_split_in_a_list_draws_the_story_again_and_reads_as_a_fresh_build() {
    let bytes = generated("lists");
    let mut engine = Replica::seed(33, &bytes).unwrap();
    let node = text_node(engine.view().unwrap(), "Bullet one");
    engine.take_redraw().unwrap();
    engine
        .apply(&Command::SplitParagraph { at: at(node, 2) })
        .unwrap();
    assert!(matches!(engine.take_redraw().unwrap(), Redraw::All));
    fresh(&mut engine);
    engine.undo().unwrap();
    assert!(matches!(engine.take_redraw().unwrap(), Redraw::All));
    fresh(&mut engine);
}

/// The view and a fresh build hold the same parts. They are compared as
/// text: a view that was edited and put back writes a part again that a
/// fresh build copies as the file has it.
fn same(engine: &mut Replica, step: &str) {
    let shown = entries(&engine.view().unwrap().export().unwrap());
    assert!(shown == entries(&engine.export().unwrap()), "{step}");
}

#[test]
fn a_split_typing_and_their_undo_read_as_a_fresh_build_in_every_fixture() {
    let mut splits = 0;
    for (name, bytes) in all_fixtures() {
        let texts: Vec<_> = {
            let doc = docx_engine::Document::open(&bytes).unwrap();
            let roots: Vec<_> = doc.stories().map(|s| s.id).collect();
            roots
                .into_iter()
                .flat_map(|r| doc.descendants(r))
                .filter(|id| doc.editable_text(*id).is_ok_and(|t| t.chars().count() > 1))
                .take(12)
                .collect()
        };
        for node in texts {
            let mut engine = Replica::seed(34, &bytes).unwrap();
            engine.view().unwrap();
            if engine
                .apply(&Command::SplitParagraph { at: at(node, 1) })
                .is_err()
            {
                continue;
            }
            splits += 1;
            same(&mut engine, &format!("{name} {node}: split"));
            let typed = engine
                .view()
                .unwrap()
                .paragraphs()
                .into_iter()
                .find_map(|p| {
                    let command = Command::InsertText {
                        at: Point::End { end: p },
                        text: "x".into(),
                    };
                    engine.apply(&command).ok()
                });
            for step in ["typed", "split"]
                .into_iter()
                .skip(usize::from(typed.is_none()))
            {
                same(&mut engine, &format!("{name} {node}: {step}"));
                assert!(engine.undo().unwrap());
            }
            same(&mut engine, &format!("{name} {node}: undone"));
        }
    }
    assert!(splits > 100, "only {splits} splits");
}

#[test]
fn typing_in_a_comment_redraws_only_its_paragraph_and_keeps_the_anchor_on_undo() {
    let mut replica = Replica::seed(30, &generated("simple-paragraphs")).unwrap();
    let text = text_node(replica.view().unwrap(), "First");
    let paragraph = replica.view().unwrap().paragraphs()[0];
    replica
        .apply(&Command::AddComment {
            from: at(text, 1),
            to: at(text, 4),
            text: "Check".into(),
            by: docx_engine::Track {
                author: "Ada".into(),
                date: "2026-11-20T00:00:00Z".into(),
            },
            initials: None,
            id: None,
        })
        .unwrap();
    replica.take_redraw().unwrap();
    let before = replica.export().unwrap();
    let selected = text_node(replica.view().unwrap(), "irs");
    replica
        .apply(&Command::InsertText {
            at: at(selected, 1),
            text: "x".into(),
        })
        .unwrap();
    for undo in [false, true] {
        if undo {
            assert!(replica.undo().unwrap());
        }
        assert!(
            matches!(replica.take_redraw().unwrap(), Redraw::Splices(s) if s.len()==1 && s[0].blocks==[paragraph])
        );
        same(&mut replica, "comment anchor");
    }
    assert_eq!(before, replica.export().unwrap());
}

#[test]
fn joining_source_paragraphs_and_undo_redraw_only_the_joined_blocks() {
    let mut r = Replica::seed(17, &generated("simple-paragraphs")).unwrap();
    let ps = r.view().unwrap().paragraphs();
    r.take_redraw().unwrap();
    r.apply(&Command::MergeParagraphs { paragraph: ps[0] })
        .unwrap();
    assert!(
        matches!(r.take_redraw().unwrap(),Redraw::Splices(s) if s.len()==1 && s[0].removed==2 && s[0].blocks==ps[..1])
    );
    same(&mut r, "merge");
    r.undo().unwrap();
    assert!(
        matches!(r.take_redraw().unwrap(),Redraw::Splices(s) if s.len()==1 && s[0].removed==1 && s[0].blocks==ps[..2])
    );
    same(&mut r, "undo merge");
}

#[test]
fn authoring_paragraphs_keeps_the_derived_tree_proportional_to_the_document() {
    let mut r = Replica::seed(19, &generated("simple-paragraphs")).unwrap();
    let mut p = r.view().unwrap().paragraphs()[0];
    for _ in 0..100 {
        r.apply(&Command::InsertText {
            at: Point::End { end: p },
            text: "One line.".into(),
        })
        .unwrap();
        r.take_redraw().unwrap();
        r.apply(&Command::SplitParagraph {
            at: Point::End { end: p },
        })
        .unwrap();
        let Redraw::Splices(s) = r.take_redraw().unwrap() else {
            panic!("full redraw for Enter");
        };
        assert!(s[0].removed <= 1 && s[0].blocks.len() <= 2);
        p = *s[0].blocks.last().unwrap();
    }
    same(&mut r, "authored");
    let doc = r.view().unwrap();
    let visible = doc
        .stories()
        .map(|s| 1 + doc.descendants(s.id).len())
        .sum::<usize>();
    assert!(
        doc.node_count() <= 2 * visible,
        "{} arena slots for {visible} visible nodes",
        doc.node_count()
    );
}

#[test]
fn a_tracked_split_rewrites_the_preceding_word_paragraph_mark() {
    let mut replica = Replica::seed(47, &generated("simple-paragraphs")).unwrap();
    let text = text_node(replica.view().unwrap(), "Second");
    replica.take_redraw().unwrap();
    replica
        .apply(&Command::ReplaceRange {
            from: at(text, 2),
            to: at(text, 2),
            text: "\n".into(),
            track: Some(docx_engine::Track {
                author: "Ada".into(),
                date: "2026-11-20T00:00:00Z".into(),
            }),
        })
        .unwrap();
    for undone in [false, true] {
        if undone {
            assert!(replica.undo().unwrap());
        }
        let redraw = replica.take_redraw().unwrap();
        assert!(
            matches!(redraw, Redraw::Splices(ref changes) if changes.len() == 1),
            "{redraw:?}"
        );
        same(&mut replica, "tracked paragraph mark");
    }
}

#[test]
fn a_redraw_describes_the_view_the_caller_saw_when_a_range_removes_a_block() {
    let bytes = with_body(
        r#"<w:p><w:r><w:t>First paragraph.</w:t></w:r></w:p><w:customXml w:element="kept"><w:p><w:r><w:t>Preserved block</w:t></w:r></w:p></w:customXml><w:p><w:r><w:t>Second paragraph.</w:t></w:r></w:p>"#,
    );
    let mut replica = Replica::seed(48, &bytes).unwrap();
    let blocks = |replica: &mut Replica| {
        let view = replica.view().unwrap();
        let body = view.stories().next().unwrap();
        (body.id, body.children.clone())
    };
    let (body, mut drawn) = blocks(&mut replica);
    let view = replica.view().unwrap();
    let (first, second) = (
        text_node(view, "First paragraph."),
        text_node(view, "Second paragraph."),
    );
    replica.take_redraw().unwrap();
    replica
        .apply(&Command::ReplaceRange {
            from: at(first, 16),
            to: at(second, 0),
            text: String::new(),
            track: None,
        })
        .unwrap();
    let redraw = replica.take_redraw().unwrap();
    let (_, shown) = blocks(&mut replica);
    match redraw {
        Redraw::All => drawn = shown.clone(),
        Redraw::Splices(splices) => {
            for splice in splices {
                assert_eq!(splice.flow, body);
                drawn.splice(splice.at..splice.at + splice.removed, splice.blocks);
            }
        }
    }
    assert_eq!(drawn, shown, "the redraw applied to what was drawn");
    assert_eq!(
        texts(replica.view().unwrap()),
        ["First paragraph.Second paragraph."]
    );
}

/// The first paragraph of each cell of a table's row, in document order.
fn cell_paragraphs(doc: &docx_engine::Document, row: usize) -> Vec<docx_engine::NodeId> {
    use docx_engine::NodeKind;
    let kind = |id| &doc.node(id).unwrap().kind;
    let all: Vec<_> = (doc.stories().flat_map(|s| doc.descendants(s.id))).collect();
    let rows: Vec<_> = (all.iter().copied())
        .filter(|id| matches!(kind(*id), NodeKind::Row(_)))
        .collect();
    (doc.node(rows[row]).unwrap().children.iter())
        .filter(|cell| matches!(kind(**cell), NodeKind::Cell(_)))
        .filter_map(|cell| {
            let children = &doc.node(*cell).unwrap().children;
            (children.iter().copied()).find(|p| matches!(kind(*p), NodeKind::Paragraph(_)))
        })
        .collect()
}

#[test]
fn typing_in_a_created_cell_redraws_its_paragraphs_and_reads_as_a_fresh_build() {
    for fixture in ["table", "table-layout", "complex-mixed"] {
        let bytes = generated(fixture);
        let mut author = Replica::seed(51, &bytes).unwrap();
        let mut peer = Replica::join(52, &bytes, &author.save()).unwrap();
        let row = {
            let doc = author.view().unwrap();
            let first = cell_paragraphs(doc, 0)[0];
            let cell = doc.node(first).unwrap().parent.unwrap();
            doc.node(cell).unwrap().parent.unwrap()
        };
        author
            .apply(&Command::InsertRow {
                row,
                below: true,
                like: None,
            })
            .unwrap();
        author.take_redraw().unwrap();
        peer.apply_update(&author.update_since(&peer.state_vector()).unwrap())
            .unwrap();
        peer.take_redraw().unwrap();
        let narrow = |replica: &mut Replica, step: &str| {
            let redraw = replica.take_redraw().unwrap();
            assert!(
                matches!(&redraw, Redraw::Splices(v) if v.len() == 1),
                "{fixture} {step}: {redraw:?}"
            );
            same(replica, &format!("{fixture} {step}"));
        };
        // The first key declares the cell's strand; later ones change it.
        let created = cell_paragraphs(author.view().unwrap(), 1)[0];
        author
            .apply(&Command::InsertText {
                at: Point::Start { start: created },
                text: "a".into(),
            })
            .unwrap();
        narrow(&mut author, "first key");
        for (step, text) in [("second key", "b"), ("third key", "🦀")] {
            let created = cell_paragraphs(author.view().unwrap(), 1)[0];
            author
                .apply(&Command::InsertText {
                    at: Point::End { end: created },
                    text: text.into(),
                })
                .unwrap();
            narrow(&mut author, step);
        }
        let typed = text_node(author.view().unwrap(), "ab🦀");
        author
            .apply(&Command::ReplaceRange {
                from: at(typed, 1),
                to: at(typed, 1),
                text: "\n".into(),
                track: None,
            })
            .unwrap();
        narrow(&mut author, "Enter");
        let after = text_node(author.view().unwrap(), "b🦀");
        author
            .apply(&Command::ReplaceText {
                node: after,
                range: Some(TextRange { start: 0, end: 0 }),
                text: "c".into(),
            })
            .unwrap();
        narrow(&mut author, "a key after Enter");
        assert!(author.undo().unwrap());
        narrow(&mut author, "undo of that key");

        peer.apply_update(&author.update_since(&peer.state_vector()).unwrap())
            .unwrap();
        same(&mut peer, &format!("{fixture} the peer"));
        assert!(
            peer.export().unwrap() == author.export().unwrap(),
            "{fixture}: the replicas differ"
        );
        assert_valid(fixture, &author.export().unwrap());
    }
}
