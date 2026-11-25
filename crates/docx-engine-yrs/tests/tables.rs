#[path = "../../docx-engine/tests/common/mod.rs"]
mod common;
use common::*;
use docx_engine::editing::Resolution;
use docx_engine::{Command, Document, NodeId, NodeKind, Point};
use docx_engine_yrs::Replica;
fn sync(a: &mut Replica, b: &mut Replica) {
    let ua = a.update_since(&b.state_vector()).unwrap();
    let ub = b.update_since(&a.state_vector()).unwrap();
    a.apply_update(&ub).unwrap();
    b.apply_update(&ua).unwrap();
    assert!(
        a.export().unwrap() == b.export().unwrap(),
        "replicas differ"
    );
    assert_valid("tables", &a.export().unwrap());
}
fn table(d: &Document) -> NodeId {
    d.stories()
        .flat_map(|s| d.descendants(s.id))
        .find(|id| matches!(d.node(*id).unwrap().kind, NodeKind::Table(_)))
        .unwrap()
}
fn rows(d: &Document) -> Vec<NodeId> {
    d.node(table(d)).unwrap().children.clone()
}
fn cell(d: &Document, r: usize, c: usize) -> NodeId {
    d.node(rows(d)[r]).unwrap().children[c]
}
fn para(d: &Document, r: usize, c: usize) -> NodeId {
    d.node(cell(d, r, c)).unwrap().children[0]
}
#[test]
fn concurrent_row_and_column_keep_original_cells_and_new_intersection_accepts_both_edits() {
    let bytes = fixture();
    let mut a = Replica::seed(1, &bytes).unwrap();
    let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
    let t = table(a.view().unwrap());
    let rs = rows(a.view().unwrap());
    a.apply(&Command::InsertRow {
        row: rs[0],
        below: true,
        like: None,
    })
    .unwrap();
    b.apply(&Command::InsertColumn {
        table: t,
        index: 1,
        like: 0,
    })
    .unwrap();
    sync(&mut a, &mut b);
    assert_eq!(rows(a.view().unwrap()).len(), 3);
    let p = para(a.view().unwrap(), 1, 1);
    a.apply(&Command::InsertText {
        at: Point::Start { start: p },
        text: "Alice".into(),
    })
    .unwrap();
    b.apply(&Command::InsertText {
        at: Point::Start { start: p },
        text: "Bob".into(),
    })
    .unwrap();
    sync(&mut a, &mut b);
    let d = a.view().unwrap();
    let text = d.text(para(d, 1, 1)).unwrap();
    assert!(text.contains("Alice") && text.contains("Bob"), "{text}");
    a.undo().unwrap();
    sync(&mut a, &mut b);
    let d = a.view().unwrap();
    assert_eq!(d.text(para(d, 1, 1)).unwrap(), "Bob");
    let saved = a.save();
    let mut c = Replica::join(3, &bytes, &saved).unwrap();
    assert!(c.export().unwrap() == a.export().unwrap());
    b.undo().unwrap();
    sync(&mut a, &mut b);
    let d = a.view().unwrap();
    assert_eq!(d.text(para(d, 1, 1)).unwrap(), "");
}
#[test]
fn concurrent_last_row_deletion_resolves_and_undo_restores_text() {
    let bytes = fixture();
    let mut a = Replica::seed(1, &bytes).unwrap();
    let original = a.export().unwrap();
    let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
    let rs = rows(a.view().unwrap());
    a.apply(&Command::RemoveNode { node: rs[0] }).unwrap();
    b.apply(&Command::RemoveNode { node: rs[1] }).unwrap();
    sync(&mut a, &mut b);
    assert_eq!(rows(a.view().unwrap()).len(), 1);
    assert_eq!(a.resolutions().unwrap().len(), 1);
    a.undo().unwrap();
    b.undo().unwrap();
    sync(&mut a, &mut b);
    assert!(a.export().unwrap() == original);
}

fn fixture() -> Vec<u8> {
    let cell = |text: &str| {
        format!(
            r#"<w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>"#
        )
    };
    with_body(&format!(
        r#"<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid><w:tr>{}{}</w:tr><w:tr>{}{}</w:tr></w:tbl><w:p/>"#,
        cell("A"),
        cell("B"),
        cell("C"),
        cell("D")
    ))
}

#[test]
fn copied_cells_keep_current_style_and_list_and_then_remain_editable() {
    let bytes = generated("table");
    let mut a = Replica::seed(1, &bytes).unwrap();
    let mut d = docx_engine::editing::Editor::open(&bytes).unwrap();
    for step in 0..4 {
        let commands = |d: &Document| {
            let p = para(d, 0, 0);
            match step {
                0 => Command::SetParagraphStyle {
                    from: Point::Start { start: p },
                    to: Point::End { end: p },
                    style: "Heading2".into(),
                },
                1 => Command::SetList {
                    from: Point::Start { start: p },
                    to: Point::End { end: p },
                    kind: Some(docx_engine::ListKind::Bullet),
                },
                2 => Command::InsertColumn {
                    table: table(d),
                    index: 1,
                    like: 0,
                },
                _ => Command::InsertText {
                    at: Point::Start {
                        start: para(d, 0, 1),
                    },
                    text: "New cell".into(),
                },
            }
        };
        let ca = commands(a.view().unwrap());
        let cd = commands(d.view().unwrap());
        a.apply(&ca).unwrap();
        d.apply(&cd).unwrap();
        assert!(
            a.export().unwrap() == d.export().unwrap(),
            "difference after step {step}"
        );
    }
}

#[test]
fn carets_in_created_cells_keep_their_strand_after_other_cells_are_edited() {
    let bytes = fixture();
    let mut a = Replica::seed(1, &bytes).unwrap();
    let t = table(a.view().unwrap());
    a.apply(&Command::InsertColumn {
        table: t,
        index: 1,
        like: 0,
    })
    .unwrap();
    let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
    let p = para(a.view().unwrap(), 1, 1);
    a.apply(&Command::InsertText {
        at: Point::Start { start: p },
        text: "tail".into(),
    })
    .unwrap();
    sync(&mut a, &mut b);
    let p = para(a.view().unwrap(), 1, 1);
    let (key, token) = a.sticky(&Point::End { end: p }).unwrap().unwrap();
    let first = para(b.view().unwrap(), 0, 1);
    b.apply(&Command::InsertText {
        at: Point::Start { start: first },
        text: "first".into(),
    })
    .unwrap();
    sync(&mut a, &mut b);
    let (p, _) = a.resolve(&key, &token).unwrap().unwrap();
    assert_eq!(a.view().unwrap().text(p).unwrap(), "tail");
}
#[test]
fn block_removal_keeps_other_regions_and_concurrent_paragraph_removal_leaves_a_usable_empty_region()
{
    let bytes = fixture();
    let mut a = Replica::seed(1, &bytes).unwrap();
    let t = table(a.view().unwrap());
    a.apply(&Command::RemoveNode { node: t }).unwrap();
    assert_valid("removed table", &a.export().unwrap());
    assert_eq!(a.view().unwrap().paragraphs().len(), 1);
    a.undo().unwrap();
    assert!(a.export().unwrap() == Document::open(&bytes).unwrap().export().unwrap());
    let bytes = with_body("<w:p/><w:p><w:r><w:t>B</w:t></w:r></w:p>");
    let mut a = Replica::seed(1, &bytes).unwrap();
    let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
    let ps = a.view().unwrap().paragraphs();
    a.apply(&Command::MergeParagraphs { paragraph: ps[0] })
        .unwrap();
    b.apply(&Command::ReplaceRange {
        from: Point::Start { start: ps[0] },
        to: Point::End { end: ps[1] },
        text: String::new(),
        track: None,
    })
    .unwrap();
    sync(&mut a, &mut b);
    let p = a.view().unwrap().paragraphs()[0];
    a.apply(&Command::InsertText {
        at: Point::Start { start: p },
        text: "Still editable".into(),
    })
    .unwrap();
    sync(&mut a, &mut b);
}

#[test]
fn removing_a_block_exposes_the_adjacent_paragraph_boundary_without_moving_text() {
    let xml = part(&fixture(), "word/document.xml");
    let begin = xml.find("<w:tbl>").unwrap();
    let end = xml.find("</w:tbl>").unwrap() + 8;
    let bytes = with_body(&format!(
        "<w:p><w:r><w:t>Before</w:t></w:r></w:p>{}<w:p><w:r><w:t>After</w:t></w:r></w:p>",
        &xml[begin..end]
    ));
    let mut a = Replica::seed(1, &bytes).unwrap();
    let t = table(a.view().unwrap());
    a.apply(&Command::RemoveNode { node: t }).unwrap();
    let ps = a.view().unwrap().paragraphs();
    a.apply(&Command::MergeParagraphs { paragraph: ps[0] })
        .unwrap();
    let p = a.view().unwrap().paragraphs()[0];
    assert_eq!(a.view().unwrap().text(p).unwrap(), "BeforeAfter");
    let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
    sync(&mut a, &mut b);
    a.undo().unwrap();
    a.undo().unwrap();
    sync(&mut a, &mut b);
    assert!(a.export().unwrap() == Document::open(&bytes).unwrap().export().unwrap());
}

#[test]
fn replacing_across_a_table_is_atomic_and_undo_reveals_concurrent_cell_edits() {
    let xml = part(&fixture(), "word/document.xml");
    let start = xml.find("<w:tbl>").unwrap();
    let end = xml.find("</w:tbl>").unwrap() + 8;
    let bytes = with_body(&format!(
        "<w:p><w:r><w:t>Before</w:t></w:r></w:p>{}<w:p><w:r><w:t>After</w:t></w:r></w:p>",
        &xml[start..end]
    ));
    let mut a = Replica::seed(1, &bytes).unwrap();
    let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
    let root = a.view().unwrap().stories().next().unwrap().id;
    let children = a.view().unwrap().node(root).unwrap().children.clone();
    let first = children[0];
    let last = children[2];
    let point = |d: &Document, p: NodeId, offset| {
        let node = d
            .descendants(p)
            .into_iter()
            .find(|id| matches!(d.node(*id).unwrap().kind, NodeKind::Text { .. }))
            .unwrap();
        Point::Text { node, offset }
    };
    let from = point(a.view().unwrap(), first, 3);
    let to = point(a.view().unwrap(), last, 1);
    a.apply(&Command::ReplaceRange {
        from,
        to,
        text: "X".into(),
        track: None,
    })
    .unwrap();
    let p = a.view().unwrap().paragraphs()[0];
    assert_eq!(a.view().unwrap().text(p).unwrap(), "BefXfter");
    let cell = para(b.view().unwrap(), 0, 0);
    b.apply(&Command::InsertText {
        at: Point::End { end: cell },
        text: " remote".into(),
    })
    .unwrap();
    sync(&mut a, &mut b);
    a.undo().unwrap();
    sync(&mut a, &mut b);
    let d = a.view().unwrap();
    assert_eq!(d.text(para(d, 0, 0)).unwrap(), "A remote");
}

#[test]
fn a_picture_replaces_a_selection_across_a_whole_table_in_one_undo_step() {
    let xml = part(&fixture(), "word/document.xml");
    let start = xml.find("<w:tbl>").unwrap();
    let end = xml.find("</w:tbl>").unwrap() + 8;
    let bytes = with_body(&format!(
        "<w:p><w:r><w:t>Before</w:t></w:r></w:p>{}<w:p><w:r><w:t>After</w:t></w:r></w:p>",
        &xml[start..end]
    ));
    let mut a = Replica::seed(1, &bytes).unwrap();
    let original = a.export().unwrap();
    let png = entries(&generated("image"))
        .into_iter()
        .find(|(n, _)| n == "word/media/image1.png")
        .unwrap()
        .1;
    let media = a.media_name(&png).unwrap();
    let ps = a.view().unwrap().paragraphs();
    a.apply_all_with_media(
        &[Command::InsertPicture {
            at: Point::Start { start: ps[0] },
            to: Some(Point::End {
                end: *ps.last().unwrap(),
            }),
            media,
            width_emu: 914400,
            height_emu: 457200,
            description: None,
            id: None,
        }],
        &[&png],
    )
    .unwrap();
    let doc = a.view().unwrap();
    assert_eq!(doc.paragraphs().len(), 1);
    let children = doc.descendants(doc.stories().next().unwrap().id);
    assert!(!children
        .iter()
        .any(|id| matches!(doc.node(*id).unwrap().kind, NodeKind::Table(_))));
    assert_eq!(
        children
            .iter()
            .filter(|id| matches!(doc.node(**id).unwrap().kind, NodeKind::Drawing(_)))
            .count(),
        1
    );
    assert_valid("picture selection", &a.export().unwrap());
    a.undo().unwrap();
    assert!(a.export().unwrap() == original);
}

#[test]
fn an_empty_created_cell_has_a_read_only_caret_before_typing() {
    let bytes = fixture();
    let mut a = Replica::seed(1, &bytes).unwrap();
    let row = rows(a.view().unwrap())[0];
    a.apply(&Command::InsertRow {
        row,
        below: true,
        like: None,
    })
    .unwrap();
    let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
    let p = para(a.view().unwrap(), 1, 0);
    let before = a.save();
    let (base, token) = a
        .sticky(&Point::Start { start: p })
        .unwrap()
        .expect("empty cell caret");
    assert_eq!(a.save(), before, "caret read wrote shared state");
    assert_eq!(
        b.resolve(&base, &token).unwrap().unwrap().0,
        para(b.view().unwrap(), 1, 0)
    );
    b.apply(&Command::InsertText {
        at: Point::Start { start: p },
        text: "😀X".into(),
    })
    .unwrap();
    sync(&mut a, &mut b);
    let (_, point) = a.resolve(&base, &token).unwrap().unwrap();
    a.apply(&Command::InsertText {
        at: point,
        text: "A".into(),
    })
    .unwrap();
    let d = a.view().unwrap();
    assert_eq!(d.text(para(d, 1, 0)).unwrap(), "A😀X");
    let row = rows(a.view().unwrap())[1];
    a.apply(&Command::RemoveNode { node: row }).unwrap();
    assert!(a.resolve(&base, &token).unwrap().is_none());
    a.undo().unwrap();
    assert!(a.resolve(&base, &token).unwrap().is_some());
}

#[test]
fn concurrent_last_column_deletion_uses_the_explicit_resolution() {
    let bytes = fixture();
    let mut a = Replica::seed(1, &bytes).unwrap();
    let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
    let table = table(a.view().unwrap());
    a.apply(&Command::RemoveColumn { table, index: 0 }).unwrap();
    b.apply(&Command::RemoveColumn { table, index: 1 }).unwrap();
    sync(&mut a, &mut b);
    assert_eq!(a.resolutions().unwrap(), [Resolution::LastColumn]);
    assert_eq!(a.resolutions().unwrap(), b.resolutions().unwrap());
    let d = a.view().unwrap();
    assert_eq!(d.node(rows(d)[0]).unwrap().children.len(), 1);
}

fn merged_rows() -> Vec<u8> {
    with_body(
        r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>Outside</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr><w:p><w:r><w:t>Merged content</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc></w:tr></w:tbl><w:p/>"#,
    )
}

#[test]
fn a_surviving_merge_dependency_retains_its_deleted_start_without_repair() {
    for (left, right) in [(1, 2), (2, 1)] {
        let bytes = merged_rows();
        let mut a = Replica::seed(left, &bytes).unwrap();
        let mut b = Replica::join(right, &bytes, &a.save()).unwrap();
        let rs = rows(a.view().unwrap());
        a.apply(&Command::RemoveNode { node: rs[2] }).unwrap();
        a.apply(&Command::RemoveNode { node: rs[1] }).unwrap();
        b.apply(&Command::InsertRow {
            row: rs[1],
            below: true,
            like: None,
        })
        .unwrap();
        sync(&mut a, &mut b);
        assert_eq!(rows(a.view().unwrap()).len(), 3);
        assert!(part(&a.export().unwrap(), "word/document.xml").contains("Merged content"));
        let saved = a.save();
        let mut loaded = Replica::join(3, &bytes, &saved).unwrap();
        assert_eq!(loaded.export().unwrap(), a.export().unwrap());
        a.view().unwrap();
        a.resolutions().unwrap();
        a.export().unwrap();
        assert_eq!(saved, a.save(), "reading wrote a repair");
        b.undo().unwrap();
        sync(&mut a, &mut b);
        assert_eq!(
            rows(a.view().unwrap()).len(),
            1,
            "dependency removal releases the pending deletion"
        );
        b.redo().unwrap();
        sync(&mut a, &mut b);
        assert_eq!(rows(a.view().unwrap()).len(), 3);
    }
}

#[test]
fn a_deleted_row_remains_the_template_of_a_concurrent_insert() {
    let bytes = with_body(
        r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:sdt><w:sdtPr><w:alias w:val="kept"/></w:sdtPr><w:sdtContent><w:tc><w:p><w:r><w:t>Wrapped</w:t></w:r></w:p></w:tc></w:sdtContent></w:sdt></w:tr><w:tr><w:trPr><w:cantSplit/></w:trPr><w:tc><w:tcPr><w:shd w:fill="FF0000"/></w:tcPr><w:p><w:r><w:t>Template</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/>"#,
    );
    for (left, right) in [(1, 2), (2, 1)] {
        let mut a = Replica::seed(left, &bytes).unwrap();
        let mut b = Replica::join(right, &bytes, &a.save()).unwrap();
        let r = rows(a.view().unwrap())[1];
        a.apply(&Command::RemoveNode { node: r }).unwrap();
        b.apply(&Command::InsertRow {
            row: r,
            below: true,
            like: Some(r),
        })
        .unwrap();
        sync(&mut a, &mut b);
        let xml = part(&a.export().unwrap(), "word/document.xml");
        assert!(xml.contains("Wrapped") && xml.contains("FF0000") && xml.contains("cantSplit"));
        assert!(!xml.contains(">Template<"));
        assert_eq!(rows(a.view().unwrap()).len(), 2);
        let mut loaded = Replica::join(3, &bytes, &a.save()).unwrap();
        assert_eq!(loaded.export().unwrap(), a.export().unwrap());
        b.undo().unwrap();
        sync(&mut a, &mut b);
        b.redo().unwrap();
        sync(&mut a, &mut b);
    }
}

#[test]
fn an_undone_created_template_still_supplies_a_surviving_row() {
    for (left, right) in [(1, 2), (2, 1)] {
        let xml = part(&fixture(), "word/document.xml");
        let body = xml
            .split("<w:body>")
            .nth(1)
            .unwrap()
            .split("</w:body>")
            .next()
            .unwrap();
        let bytes = with_body(&body.replacen("<w:tcPr>", "<w:tcPr><w:shd w:fill=\"FF0000\"/>", 1));
        let mut a = Replica::seed(left, &bytes).unwrap();
        let rs = rows(a.view().unwrap());
        let original = rs[1];
        a.apply(&Command::InsertRow {
            row: original,
            below: true,
            like: Some(rs[0]),
        })
        .unwrap();
        let mut b = Replica::join(right, &bytes, &a.save()).unwrap();
        let created = rows(b.view().unwrap())[2];
        b.apply(&Command::InsertRow {
            row: created,
            below: true,
            like: None,
        })
        .unwrap();
        a.undo().unwrap();
        sync(&mut a, &mut b);
        assert_eq!(rows(a.view().unwrap()).len(), 3);
        let d = a.view().unwrap();
        let NodeKind::Cell(info) = &d.node(cell(d, 2, 0)).unwrap().kind else {
            unreachable!()
        };
        assert_eq!(info.props.shading.as_deref(), Some("FF0000"));
        let p = para(d, 2, 0);
        a.apply(&Command::InsertText {
            at: Point::Start { start: p },
            text: "Survives".into(),
        })
        .unwrap();
        sync(&mut a, &mut b);
        let mut loaded = Replica::join(3, &bytes, &a.save()).unwrap();
        assert_eq!(loaded.export().unwrap(), a.export().unwrap());
    }
}

#[test]
fn a_hidden_template_tracks_columns_before_a_concurrent_copy() {
    // The row insertion is delayed by harmless edits, so both id orderings
    // interpret a grid change while its named template is detached.
    for (left, right) in [(1, 2), (2, 1)] {
        let bytes = fixture();
        let mut a = Replica::seed(left, &bytes).unwrap();
        let mut b = Replica::join(right, &bytes, &a.save()).unwrap();
        let t = table(a.view().unwrap());
        let r = rows(a.view().unwrap())[1];
        a.apply(&Command::RemoveNode { node: r }).unwrap();
        a.apply(&Command::InsertColumn {
            table: t,
            index: 1,
            like: 0,
        })
        .unwrap();
        a.apply(&Command::RemoveColumn { table: t, index: 2 })
            .unwrap();
        for _ in 0..4 {
            let p = para(b.view().unwrap(), 0, 0);
            b.apply(&Command::InsertText {
                at: Point::End { end: p },
                text: "x".into(),
            })
            .unwrap();
        }
        b.apply(&Command::InsertRow {
            row: r,
            below: true,
            like: None,
        })
        .unwrap();
        sync(&mut a, &mut b);
        assert_eq!(rows(a.view().unwrap()).len(), 2);
        let d = a.view().unwrap();
        assert_eq!(d.node(rows(d)[1]).unwrap().children.len(), 2);
        let widths: Vec<_> = d
            .node(rows(d)[1])
            .unwrap()
            .children
            .iter()
            .filter_map(|c| {
                if let NodeKind::Cell(info) = &d.node(*c).unwrap().kind {
                    info.width
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(widths, [1500, 4500]);
        let p = para(d, 1, 1);
        a.apply(&Command::InsertText {
            at: Point::Start { start: p },
            text: "Intersection".into(),
        })
        .unwrap();
        sync(&mut a, &mut b);
        let mut loaded = Replica::join(3, &bytes, &a.save()).unwrap();
        assert_eq!(loaded.export().unwrap(), a.export().unwrap());
    }
}

#[test]
fn retaining_a_merge_start_does_not_reapply_column_editing_policy() {
    let xml = part(&merged_rows(), "word/document.xml");
    let body = xml
        .split("<w:body>")
        .nth(1)
        .unwrap()
        .split("</w:body>")
        .next()
        .unwrap();
    let bytes = with_body(&body.replace("<w:vMerge w:val=\"restart\"/>", r#"<w:vMerge w:val="restart"/><w:tcPrChange w:id="9" w:author="Original" w:date="2026-11-18T00:00:00Z"><w:tcPr><w:tcW w:w="3000" w:type="dxa"/></w:tcPr></w:tcPrChange>"#));
    let mut a = Replica::seed(1, &bytes).unwrap();
    let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
    let rs = rows(a.view().unwrap());
    let t = table(a.view().unwrap());
    a.apply(&Command::RemoveNode { node: rs[2] }).unwrap();
    a.apply(&Command::RemoveNode { node: rs[1] }).unwrap();
    a.apply(&Command::InsertColumn {
        table: t,
        index: 1,
        like: 0,
    })
    .unwrap();
    b.apply(&Command::InsertRow {
        row: rs[1],
        below: true,
        like: None,
    })
    .unwrap();
    sync(&mut a, &mut b);
    assert_eq!(rows(a.view().unwrap()).len(), 3);
    let xml = part(&a.export().unwrap(), "word/document.xml");
    assert!(
        xml.contains("Merged content") && xml.contains("tcPrChange") && xml.contains("Original")
    );
    let NodeKind::Table(info) = &a.view().unwrap().node(t).unwrap().kind else {
        unreachable!()
    };
    assert_eq!(info.grid.len(), 2);
}

#[test]
fn a_copied_partial_row_survives_a_concurrent_column_insert() {
    let bytes = with_body(
        r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:trPr><w:gridBefore w:val="1"/></w:trPr><w:tc><w:p><w:r><w:t>Partial</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>A</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>B</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/>"#,
    );
    for (left, right) in [(1, 2), (2, 1)] {
        let mut a = Replica::seed(left, &bytes).unwrap();
        let mut b = Replica::join(right, &bytes, &a.save()).unwrap();
        let r = rows(a.view().unwrap())[0];
        let t = table(a.view().unwrap());
        a.apply(&Command::RemoveNode { node: r }).unwrap();
        a.apply(&Command::InsertColumn {
            table: t,
            index: 0,
            like: 0,
        })
        .unwrap();
        b.apply(&Command::InsertRow {
            row: r,
            below: true,
            like: None,
        })
        .unwrap();
        sync(&mut a, &mut b);
        let d = a.view().unwrap();
        let NodeKind::Row(info) = &d.node(rows(d)[0]).unwrap().kind else {
            unreachable!()
        };
        assert_eq!(info.grid_before, 2);
        assert_eq!(d.node(rows(d)[0]).unwrap().children.len(), 1);
        let mut loaded = Replica::join(3, &bytes, &a.save()).unwrap();
        assert_eq!(loaded.export().unwrap(), a.export().unwrap());
        b.undo().unwrap();
        sync(&mut a, &mut b);
        b.redo().unwrap();
        sync(&mut a, &mut b);
    }
}

fn colored_columns() -> Vec<u8> {
    with_body(
        r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/><w:shd w:fill="0000FF"/></w:tcPr><w:p><w:r><w:t>Blue</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/><w:shd w:fill="FF0000"/></w:tcPr><w:p><w:r><w:t>Red</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/>"#,
    )
}
fn cell_colors(d: &Document) -> Vec<Option<String>> {
    d.node(rows(d)[0])
        .unwrap()
        .children
        .iter()
        .filter_map(|c| {
            if let NodeKind::Cell(info) = &d.node(*c).unwrap().kind {
                Some(info.props.shading.clone())
            } else {
                None
            }
        })
        .collect()
}
#[test]
fn a_deleted_column_still_supplies_its_named_cell_properties() {
    for (left, right) in [(1, 2), (2, 1)] {
        let bytes = colored_columns();
        let mut a = Replica::seed(left, &bytes).unwrap();
        let mut b = Replica::join(right, &bytes, &a.save()).unwrap();
        let t = table(a.view().unwrap());
        a.apply(&Command::RemoveColumn { table: t, index: 1 })
            .unwrap();
        b.apply(&Command::InsertColumn {
            table: t,
            index: 2,
            like: 1,
        })
        .unwrap();
        sync(&mut a, &mut b);
        assert_eq!(
            cell_colors(a.view().unwrap()),
            [Some("0000FF".into()), Some("FF0000".into())]
        );
        let mut loaded = Replica::join(3, &bytes, &a.save()).unwrap();
        assert_eq!(loaded.export().unwrap(), a.export().unwrap());
        b.undo().unwrap();
        sync(&mut a, &mut b);
        assert_eq!(cell_colors(a.view().unwrap()), [Some("0000FF".into())]);
        b.redo().unwrap();
        sync(&mut a, &mut b);
        assert_eq!(
            cell_colors(a.view().unwrap()),
            [Some("0000FF".into()), Some("FF0000".into())]
        );
    }
}

#[test]
fn an_undone_created_column_remains_the_named_template() {
    for (left, right) in [(1, 2), (2, 1)] {
        let bytes = colored_columns();
        let mut a = Replica::seed(left, &bytes).unwrap();
        let mut b = Replica::join(right, &bytes, &a.save()).unwrap();
        let t = table(a.view().unwrap());
        a.apply(&Command::InsertColumn {
            table: t,
            index: 2,
            like: 1,
        })
        .unwrap();
        sync(&mut a, &mut b);
        b.apply(&Command::InsertColumn {
            table: t,
            index: 3,
            like: 2,
        })
        .unwrap();
        a.undo().unwrap();
        sync(&mut a, &mut b);
        assert_eq!(
            cell_colors(a.view().unwrap()),
            [
                Some("0000FF".into()),
                Some("FF0000".into()),
                Some("FF0000".into())
            ]
        );
        let NodeKind::Table(info) = &a.view().unwrap().node(t).unwrap().kind else {
            unreachable!()
        };
        assert_eq!(info.grid.iter().sum::<u32>(), 6000);
        let mut loaded = Replica::join(3, &bytes, &a.save()).unwrap();
        assert_eq!(loaded.export().unwrap(), a.export().unwrap());
        a.redo().unwrap();
        sync(&mut a, &mut b);
        assert_eq!(cell_colors(a.view().unwrap()).len(), 4);
        b.undo().unwrap();
        sync(&mut a, &mut b);
        assert_eq!(cell_colors(a.view().unwrap()).len(), 3);
    }
}

#[test]
fn partial_rows_keep_their_grid_geometry_across_column_edits() {
    // One cell in the middle, with omitted slots on either side. Another
    // replica can copy it while its source is hidden from column validation.
    let bytes = with_body(
        r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:trPr><w:gridBefore w:val="1"/><w:gridAfter w:val="1"/></w:trPr><w:tc><w:tcPr><w:shd w:fill="FF0000"/></w:tcPr><w:p><w:r><w:t>Partial</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:p/></w:tc><w:tc><w:p/></w:tc><w:tc><w:p/></w:tc></w:tr></w:tbl><w:p/>"#,
    );
    for (left, right) in [(1, 2), (2, 1)] {
        for insert in [true, false] {
            for index in 0..if insert { 4 } else { 3 } {
                for like in 0..if insert { 3 } else { 1 } {
                    let mut a = Replica::seed(left, &bytes).unwrap();
                    let mut b = Replica::join(right, &bytes, &a.save()).unwrap();
                    let t = table(a.view().unwrap());
                    let r = rows(a.view().unwrap())[0];
                    a.apply(&Command::RemoveNode { node: r }).unwrap();
                    let command = if insert {
                        Command::InsertColumn {
                            table: t,
                            index,
                            like,
                        }
                    } else {
                        Command::RemoveColumn { table: t, index }
                    };
                    a.apply(&command).unwrap();
                    b.apply(&Command::InsertRow {
                        row: r,
                        below: true,
                        like: None,
                    })
                    .unwrap();
                    sync(&mut a, &mut b);
                    let d = a.view().unwrap();
                    let NodeKind::Table(grid) = &d.node(t).unwrap().kind else {
                        unreachable!()
                    };
                    assert_eq!(grid.grid.iter().sum::<u32>(), 6000);
                    for r in rows(d) {
                        let NodeKind::Row(info) = &d.node(r).unwrap().kind else {
                            unreachable!()
                        };
                        let spans: u32 = d
                            .node(r)
                            .unwrap()
                            .children
                            .iter()
                            .filter_map(|c| match &d.node(*c).unwrap().kind {
                                NodeKind::Cell(c) => Some(c.grid_span),
                                _ => None,
                            })
                            .sum();
                        assert!(spans > 0);
                        assert_eq!(
                            info.grid_before + spans + info.grid_after,
                            grid.grid.len() as u32,
                            "{insert} {index} {like}"
                        );
                    }
                    // Removing the copied row's sole column hides that row;
                    // undo restores its geometry and the independent row copy.
                    assert_eq!(rows(d).len(), if !insert && index == 1 { 1 } else { 2 });
                    let mut loaded = Replica::join(3, &bytes, &a.save()).unwrap();
                    assert_eq!(loaded.export().unwrap(), a.export().unwrap());
                    a.undo().unwrap();
                    sync(&mut a, &mut b);
                    assert_eq!(rows(a.view().unwrap()).len(), 2);
                    a.redo().unwrap();
                    sync(&mut a, &mut b);
                }
            }
        }
    }
}

fn wrapped_row() -> Vec<u8> {
    with_body(
        r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:sdt><w:sdtPr><w:tag w:val="keep"/></w:sdtPr><w:sdtContent><w:tc><w:p><w:r><w:t>Wrapped</w:t></w:r></w:p></w:tc></w:sdtContent></w:sdt><w:tc><w:p/></w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>Left</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Right</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/>"#,
    )
}
fn grid_len(d: &Document) -> usize {
    let NodeKind::Table(info) = &d.node(table(d)).unwrap().kind else {
        unreachable!()
    };
    info.grid.len()
}

#[test]
fn restored_wrapped_cells_suspend_columns_without_losing_their_text() {
    let bytes = wrapped_row();
    for (left, right) in [(1, 2), (2, 1)] {
        for remove_again in [false, true] {
            let mut a = Replica::seed(left, &bytes).unwrap();
            let mut b = Replica::join(right, &bytes, &a.save()).unwrap();
            let t = table(a.view().unwrap());
            let r = rows(a.view().unwrap())[0];
            a.apply(&Command::RemoveNode { node: r }).unwrap();
            sync(&mut a, &mut b);
            b.apply(&Command::InsertColumn {
                table: t,
                index: 0,
                like: 0,
            })
            .unwrap();
            let p = para(b.view().unwrap(), 0, 0);
            b.apply(&Command::InsertText {
                at: Point::Start { start: p },
                text: "Kept typing".into(),
            })
            .unwrap();
            // A later column may itself name the suppressed one.
            b.apply(&Command::InsertColumn {
                table: t,
                index: 1,
                like: 0,
            })
            .unwrap();
            a.undo().unwrap();
            sync(&mut a, &mut b);
            let before = a.state_vector();
            assert_eq!(grid_len(a.view().unwrap()), 2);
            assert_eq!(rows(a.view().unwrap()).len(), 2);
            let xml = part(&a.export().unwrap(), "word/document.xml");
            assert!(
                xml.contains("<w:tag w:val=\"keep\"/>")
                    && xml.contains("Wrapped")
                    && xml.contains("Left")
                    && xml.contains("Right")
            );
            assert!(!xml.contains("Kept typing"));
            assert!(a.resolutions().unwrap().contains(&Resolution::SourceGrid));
            assert_eq!(before, a.state_vector(), "reading wrote a repair");
            let mut loaded = Replica::join(3, &bytes, &a.save()).unwrap();
            assert_eq!(loaded.export().unwrap(), a.export().unwrap());
            if remove_again {
                a.apply(&Command::RemoveNode { node: r }).unwrap();
            } else {
                a.redo().unwrap();
            }
            sync(&mut a, &mut b);
            assert_eq!(grid_len(a.view().unwrap()), 4);
            assert_eq!(rows(a.view().unwrap()).len(), 1);
            assert_eq!(
                {
                    let d = a.view().unwrap();
                    d.text(para(d, 0, 0)).unwrap()
                },
                "Kept typing"
            );
            assert!(!a.resolutions().unwrap().contains(&Resolution::SourceGrid));
            b.undo().unwrap();
            sync(&mut a, &mut b);
            assert_eq!(grid_len(a.view().unwrap()), 3);
            b.undo().unwrap();
            sync(&mut a, &mut b);
            assert_eq!(
                {
                    let d = a.view().unwrap();
                    d.text(para(d, 0, 0)).unwrap()
                },
                ""
            );
            b.redo().unwrap();
            sync(&mut a, &mut b);
            assert_eq!(
                {
                    let d = a.view().unwrap();
                    d.text(para(d, 0, 0)).unwrap()
                },
                "Kept typing"
            );
        }
    }
}

#[test]
fn restored_wrapped_cells_suspend_column_removal_until_hidden_again() {
    let bytes = wrapped_row();
    for (left, right) in [(1, 2), (2, 1)] {
        for index in 0..2 {
            let mut a = Replica::seed(left, &bytes).unwrap();
            let mut b = Replica::join(right, &bytes, &a.save()).unwrap();
            let t = table(a.view().unwrap());
            let r = rows(a.view().unwrap())[0];
            a.apply(&Command::RemoveNode { node: r }).unwrap();
            sync(&mut a, &mut b);
            b.apply(&Command::RemoveColumn { table: t, index }).unwrap();
            a.undo().unwrap();
            sync(&mut a, &mut b);
            assert_eq!(grid_len(a.view().unwrap()), 2);
            let xml = part(&a.export().unwrap(), "word/document.xml");
            assert!(xml.contains("Wrapped") && xml.contains("Left") && xml.contains("Right"));
            a.redo().unwrap();
            sync(&mut a, &mut b);
            assert_eq!(grid_len(a.view().unwrap()), 1);
            assert_eq!(
                {
                    let d = a.view().unwrap();
                    d.text(para(d, 0, 0)).unwrap()
                },
                if index == 0 { "Right" } else { "Left" }
            );
            b.undo().unwrap();
            sync(&mut a, &mut b);
            assert_eq!(grid_len(a.view().unwrap()), 2);
            b.redo().unwrap();
            sync(&mut a, &mut b);
            assert_eq!(grid_len(a.view().unwrap()), 1);
        }
    }
}

#[test]
fn wrapped_row_precedence_is_local_to_its_table() {
    let one = part(&wrapped_row(), "word/document.xml");
    let first =
        &one[one.find("<w:tbl>").unwrap()..one.find("</w:tbl>").unwrap() + "</w:tbl>".len()];
    let bytes = with_body(&format!("{first}<w:tbl><w:tblGrid><w:gridCol w:w=\"3000\"/><w:gridCol w:w=\"3000\"/></w:tblGrid><w:tr><w:tc><w:p/></w:tc><w:tc><w:p/></w:tc></w:tr></w:tbl><w:p/>"));
    let mut a = Replica::seed(1, &bytes).unwrap();
    let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
    let d = a.view().unwrap();
    let tables: Vec<_> = d
        .stories()
        .flat_map(|s| d.descendants(s.id))
        .filter(|n| matches!(d.node(*n).unwrap().kind, NodeKind::Table(_)))
        .collect();
    let r = rows(d)[0];
    a.apply(&Command::RemoveNode { node: r }).unwrap();
    sync(&mut a, &mut b);
    for table in &tables {
        b.apply(&Command::InsertColumn {
            table: *table,
            index: 1,
            like: 0,
        })
        .unwrap();
    }
    a.undo().unwrap();
    sync(&mut a, &mut b);
    let d = a.view().unwrap();
    for (i, table) in tables.iter().enumerate() {
        let NodeKind::Table(info) = &d.node(*table).unwrap().kind else {
            unreachable!()
        };
        assert_eq!(info.grid.len(), if i == 0 { 2 } else { 3 });
    }
}

#[test]
fn cell_lifetime_narrowing_a_created_span_preserves_its_text() {
    let bytes = with_body(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr><w:p><w:r><w:t>Source</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/>"#,
    );
    let mut a = Replica::seed(1, &bytes).unwrap();
    let t = table(a.view().unwrap());
    let r = rows(a.view().unwrap())[0];
    a.apply(&Command::InsertRow {
        row: r,
        below: true,
        like: None,
    })
    .unwrap();
    let p = para(a.view().unwrap(), 1, 0);
    a.apply(&Command::InsertText {
        at: Point::Start { start: p },
        text: "Keep this".into(),
    })
    .unwrap();
    a.apply(&Command::RemoveColumn { table: t, index: 0 })
        .unwrap();
    let d = a.view().unwrap();
    assert_eq!(d.text(para(d, 1, 0)).unwrap(), "Keep this");
    let export = a.export().unwrap();
    assert_valid("narrowed span", &export);
    let reopened = Document::open(&export).unwrap();
    assert_eq!(reopened.text(para(&reopened, 1, 0)).unwrap(), "Keep this");
    let mut loaded = Replica::join(2, &bytes, &a.save()).unwrap();
    assert_eq!(loaded.export().unwrap(), export);
    a.undo().unwrap();
    a.redo().unwrap();
    let d = a.view().unwrap();
    assert_eq!(d.text(para(d, 1, 0)).unwrap(), "Keep this");
}

#[test]
fn cell_lifetime_new_coverage_keeps_a_continuations_merge_start() {
    let bytes = with_body(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc><w:p/></w:tc><w:tc><w:p/></w:tc></w:tr><w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/><w:vMerge w:val="restart"/></w:tcPr><w:p><w:r><w:t>Merged content</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/><w:vMerge/></w:tcPr><w:p/></w:tc></w:tr></w:tbl><w:p/>"#,
    );
    for (left, right) in [(1, 2), (2, 1)] {
        let mut a = Replica::seed(left, &bytes).unwrap();
        let mut b = Replica::join(right, &bytes, &a.save()).unwrap();
        let t = table(a.view().unwrap());
        let rs = rows(a.view().unwrap());
        a.apply(&Command::RemoveNode { node: rs[2] }).unwrap();
        a.apply(&Command::RemoveNode { node: rs[1] }).unwrap();
        b.apply(&Command::InsertRow {
            row: rs[1],
            below: true,
            like: None,
        })
        .unwrap();
        b.apply(&Command::InsertColumn {
            table: t,
            index: 1,
            like: 0,
        })
        .unwrap();
        b.apply(&Command::RemoveColumn { table: t, index: 0 })
            .unwrap();
        b.apply(&Command::RemoveColumn { table: t, index: 1 })
            .unwrap();
        sync(&mut a, &mut b);
        let xml = part(&a.export().unwrap(), "word/document.xml");
        assert!(
            xml.contains("Merged content") && xml.contains("w:vMerge w:val=\"restart\""),
            "{xml}"
        );
        assert_eq!(rows(a.view().unwrap()).len(), 3);
        let mut loaded = Replica::join(3, &bytes, &a.save()).unwrap();
        assert_eq!(loaded.export().unwrap(), a.export().unwrap());
        b.undo().unwrap();
        sync(&mut a, &mut b);
        b.redo().unwrap();
        sync(&mut a, &mut b);
        assert!(part(&a.export().unwrap(), "word/document.xml").contains("Merged content"));
        let last = *rows(b.view().unwrap()).last().unwrap();
        b.apply(&Command::RemoveNode { node: last }).unwrap();
        sync(&mut a, &mut b);
        assert_eq!(rows(a.view().unwrap()).len(), 1);
        assert!(!part(&a.export().unwrap(), "word/document.xml").contains("Merged content"));
        b.undo().unwrap();
        sync(&mut a, &mut b);
        assert_eq!(rows(a.view().unwrap()).len(), 3);
        assert!(part(&a.export().unwrap(), "word/document.xml").contains("Merged content"));
    }
}

#[test]
fn cell_origins_survive_copying_a_narrowed_cell_and_concurrent_typing() {
    let bytes = with_body(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr><w:p/></w:tc></w:tr></w:tbl><w:p/>"#,
    );
    for (left, right) in [(1, 2), (2, 1)] {
        let mut a = Replica::seed(left, &bytes).unwrap();
        let mut b = Replica::join(right, &bytes, &a.save()).unwrap();
        let t = table(a.view().unwrap());
        let r = rows(a.view().unwrap())[0];
        a.apply(&Command::InsertRow {
            row: r,
            below: true,
            like: None,
        })
        .unwrap();
        sync(&mut a, &mut b);
        let p = para(a.view().unwrap(), 1, 0);
        a.apply(&Command::InsertText {
            at: Point::Start { start: p },
            text: "Original typing".into(),
        })
        .unwrap();
        b.apply(&Command::RemoveColumn { table: t, index: 0 })
            .unwrap();
        sync(&mut a, &mut b);
        let r = rows(b.view().unwrap())[1];
        b.apply(&Command::InsertRow {
            row: r,
            below: true,
            like: None,
        })
        .unwrap();
        let p = para(b.view().unwrap(), 2, 0);
        b.apply(&Command::InsertText {
            at: Point::Start { start: p },
            text: "Copy typing".into(),
        })
        .unwrap();
        a.apply(&Command::InsertColumn {
            table: t,
            index: 0,
            like: 0,
        })
        .unwrap();
        sync(&mut a, &mut b);
        let d = a.view().unwrap();
        assert_eq!(d.text(para(d, 1, 1)).unwrap(), "Original typing");
        assert_eq!(d.text(para(d, 2, 1)).unwrap(), "Copy typing");
        let mut loaded = Replica::join(3, &bytes, &a.save()).unwrap();
        assert_eq!(loaded.export().unwrap(), a.export().unwrap());
        b.undo().unwrap();
        sync(&mut a, &mut b);
        let d = a.view().unwrap();
        assert_eq!(d.text(para(d, 1, 1)).unwrap(), "Original typing");
        assert_eq!(d.text(para(d, 2, 1)).unwrap(), "");
        b.redo().unwrap();
        sync(&mut a, &mut b);
        let d = a.view().unwrap();
        assert_eq!(d.text(para(d, 2, 1)).unwrap(), "Copy typing");
    }
}

fn wrapped_merge_rows() -> Vec<u8> {
    let cell = |merge: &str, text: &str| {
        format!("<w:tc><w:tcPr>{merge}</w:tcPr><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>")
    };
    let restart = r#"<w:vMerge w:val="restart"/>"#;
    let continued = "<w:vMerge/>";
    with_body(&format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid><w:tr>{}{}</w:tr><w:tr><w:sdt><w:sdtPr><w:tag w:val="keep-wrapper"/></w:sdtPr><w:sdtContent>{}</w:sdtContent></w:sdt>{}</w:tr><w:tr>{}{}</w:tr><w:tr>{}{}</w:tr></w:tbl><w:p/>"#,
        cell(restart, "S0"),
        cell("", "S1"),
        cell(continued, ""),
        cell(restart, "W1"),
        cell(continued, ""),
        cell(continued, ""),
        cell("", "Z0"),
        cell("", "Z1"),
    ))
}

fn merge_kinds(d: &Document, row: usize) -> Vec<Option<String>> {
    d.node(rows(d)[row])
        .unwrap()
        .children
        .iter()
        .filter_map(|c| {
            if let NodeKind::Cell(info) = &d.node(*c).unwrap().kind {
                Some(info.vertical_merge.clone())
            } else {
                None
            }
        })
        .collect()
}

#[test]
fn merge_discovery_counts_cells_inside_wrappers() {
    let original = wrapped_merge_rows();
    let xml = part(&original, "word/document.xml");
    let renamed = xml.replace("xmlns:w=", "xmlns:q=").replace("w:", "q:");
    let nested = xml.replace("</w:tc></w:sdtContent>", r#"<w:tbl><w:tblGrid><w:gridCol w:w="1000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr><w:p><w:r><w:t>Nested</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/></w:tc></w:sdtContent>"#);
    for bytes in [
        original.clone(),
        rewrite(&original, "word/document.xml", Some(renamed.as_bytes())),
        rewrite(&original, "word/document.xml", Some(nested.as_bytes())),
    ] {
        let plain = Document::open(&bytes).unwrap();
        let command = Command::InsertRow {
            row: rows(&plain)[1],
            below: true,
            like: Some(rows(&plain)[2]),
        };
        let expected = vec![Some("continue".into()), Some("continue".into())];
        let mut editor = docx_engine::editing::Editor::open(&bytes).unwrap();
        editor.apply(&command).unwrap();
        assert_eq!(
            merge_kinds(editor.view().unwrap(), 2),
            expected,
            "headless merge"
        );
        let mut replica = Replica::seed(1, &bytes).unwrap();
        replica.apply(&command).unwrap();
        assert_eq!(
            merge_kinds(replica.view().unwrap(), 2),
            expected,
            "replica merge"
        );
        let exported = replica.export().unwrap();
        assert_eq!(
            merge_kinds(&Document::open(&exported).unwrap(), 2),
            expected
        );
    }
}

#[test]
fn merge_discovery_through_wrappers_retains_the_actual_start_rows() {
    for (left, right) in [(1, 2), (2, 1)] {
        let bytes = wrapped_merge_rows();
        let mut a = Replica::seed(left, &bytes).unwrap();
        let mut b = Replica::join(right, &bytes, &a.save()).unwrap();
        let rs = rows(a.view().unwrap());
        for node in [rs[2], rs[1], rs[0]] {
            a.apply(&Command::RemoveNode { node }).unwrap();
        }
        b.apply(&Command::InsertRow {
            row: rs[1],
            below: true,
            like: Some(rs[2]),
        })
        .unwrap();
        sync(&mut a, &mut b);
        let saved = a.save();
        let xml = part(&a.export().unwrap(), "word/document.xml");
        assert!(
            xml.contains("S0") && xml.contains("W1"),
            "actual starts must survive: {xml}"
        );
        assert!(xml.contains("keep-wrapper"));
        assert_eq!(rows(a.view().unwrap()).len(), 4);
        assert_eq!(
            merge_kinds(a.view().unwrap(), 2),
            vec![Some("continue".into()), Some("continue".into())]
        );
        a.resolutions().unwrap();
        assert_eq!(saved, a.save(), "reading wrote a repair");
        let mut loaded = Replica::join(3, &bytes, &saved).unwrap();
        assert_eq!(loaded.export().unwrap(), a.export().unwrap());
        b.undo().unwrap();
        sync(&mut a, &mut b);
        assert_eq!(rows(a.view().unwrap()).len(), 1);
        b.redo().unwrap();
        sync(&mut a, &mut b);
        let xml = part(&a.export().unwrap(), "word/document.xml");
        assert!(xml.contains("S0") && xml.contains("W1"));
        assert_eq!(rows(a.view().unwrap()).len(), 4);
    }
}

#[test]
fn removing_a_preserved_merge_cell_releases_its_start_dependency() {
    for (left, right) in [(1, 2), (2, 1)] {
        let bytes = wrapped_merge_rows();
        let mut a = Replica::seed(left, &bytes).unwrap();
        let mut b = Replica::join(right, &bytes, &a.save()).unwrap();
        let rs = rows(a.view().unwrap());
        a.apply(&Command::RemoveNode { node: rs[2] }).unwrap();
        a.apply(&Command::RemoveNode { node: rs[1] }).unwrap();
        sync(&mut a, &mut b);
        b.apply(&Command::RemoveNode { node: rs[0] }).unwrap();
        sync(&mut a, &mut b);
        // The start deletion was accepted while no continuation existed.
        // Independently restoring the wrapper makes it a live dependency.
        a.undo().unwrap();
        sync(&mut a, &mut b);
        assert!(part(&a.export().unwrap(), "word/document.xml").contains("S0"));
        let wrapper = a.view().unwrap().node(rs[1]).unwrap().children[0];
        a.apply(&Command::RemoveNode { node: wrapper }).unwrap();
        sync(&mut a, &mut b);
        assert!(!part(&a.export().unwrap(), "word/document.xml").contains("S0"));
        a.undo().unwrap();
        sync(&mut a, &mut b);
        assert!(part(&a.export().unwrap(), "word/document.xml").contains("S0"));
        a.redo().unwrap();
        sync(&mut a, &mut b);
        assert!(!part(&a.export().unwrap(), "word/document.xml").contains("S0"));
        let mut loaded = Replica::join(3, &bytes, &a.save()).unwrap();
        assert_eq!(loaded.export().unwrap(), a.export().unwrap());
    }
}

#[test]
fn insertion_before_a_wrapped_row_continues_the_correct_column() {
    let bytes = with_body(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>S0</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr><w:p><w:r><w:t>S1</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:sdt><w:sdtContent><w:tc><w:p/></w:tc></w:sdtContent></w:sdt><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc></w:tr></w:tbl><w:p/>"#,
    );
    let plain = Document::open(&bytes).unwrap();
    let command = Command::InsertRow {
        row: rows(&plain)[0],
        below: true,
        like: None,
    };
    let expected = vec![None, Some("continue".into())];
    let mut headless = docx_engine::editing::Editor::open(&bytes).unwrap();
    headless.apply(&command).unwrap();
    assert_eq!(
        merge_kinds(headless.view().unwrap(), 1),
        expected,
        "headless"
    );
    let mut replica = Replica::seed(1, &bytes).unwrap();
    replica.apply(&command).unwrap();
    assert_eq!(merge_kinds(replica.view().unwrap(), 1), expected, "replica");
    let mut joined = Replica::join(2, &bytes, &replica.save()).unwrap();
    assert_eq!(joined.export().unwrap(), replica.export().unwrap());
}

#[test]
fn deleting_a_start_with_a_wrapped_continuation_is_refused_atomically() {
    let bytes = with_body(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr><w:p><w:r><w:t>S0</w:t></w:r></w:p></w:tc><w:tc><w:p/></w:tc></w:tr><w:tr><w:sdt><w:sdtContent><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc></w:sdtContent></w:sdt><w:tc><w:p/></w:tc></w:tr></w:tbl><w:p/>"#,
    );
    let plain = Document::open(&bytes).unwrap();
    let command = Command::RemoveNode {
        node: rows(&plain)[0],
    };
    let mut headless = docx_engine::editing::Editor::open(&bytes).unwrap();
    let before = headless.export().unwrap();
    assert!(headless.apply(&command).is_err());
    assert_eq!(headless.export().unwrap(), before);
    assert!(!headless.undo());
    let mut replica = Replica::seed(1, &bytes).unwrap();
    let state = replica.save();
    assert!(replica.apply(&command).is_err());
    assert_eq!(replica.save(), state);
    assert!(!replica.undo().unwrap());
}

#[test]
fn last_row_fallback_checks_merge_dependencies_before_choosing_the_grid() {
    let cell = |name: &str, props: &str| {
        format!("<w:tc>{props}<w:p><w:r><w:t>{name}</w:t></w:r></w:p></w:tc>")
    };
    let plain = cell("other", "");
    let wrapped = format!(
        "<w:sdt><w:sdtPr/><w:sdtContent>{}</w:sdtContent></w:sdt>",
        cell("Wrapped", "<w:tcPr><w:vMerge w:val=\"restart\"/></w:tcPr>")
    );
    let cont = cell("Continuation", "<w:tcPr><w:vMerge/></w:tcPr>");
    let restart = cell(
        "Second start",
        "<w:tcPr><w:vMerge w:val=\"restart\"/></w:tcPr>",
    );
    for (undo_full_row_creation, chained) in
        [(false, false), (true, false), (false, true), (true, true)]
    {
        // The middle row depends on the wrapped start in column 0 and starts
        // its own merge in column 1. The final continuation needs both starts.
        let chain = if chained {
            format!("<w:tr>{cont}{restart}{plain}</w:tr>")
        } else {
            String::new()
        };
        let continuation = if chained {
            format!("{plain}{cont}{plain}")
        } else {
            format!("{cont}{plain}{plain}")
        };
        let kept_columns = if chained { 2 } else { 1 };
        let partial = plain.repeat(3 - kept_columns);
        let arow = if undo_full_row_creation {
            String::new()
        } else {
            format!("<w:tr>{plain}{plain}{plain}</w:tr>")
        };
        let bytes = with_body(&format!("<w:tbl><w:tblPr/><w:tblGrid>{}</w:tblGrid>{arow}<w:tr>{wrapped}{plain}{plain}</w:tr>{chain}<w:tr>{continuation}</w:tr><w:tr><w:trPr><w:gridBefore w:val=\"{kept_columns}\"/></w:trPr>{partial}</w:tr></w:tbl><w:p/>","<w:gridCol w:w=\"2000\"/>".repeat(3)));
        for (left, right) in [(1, 2), (2, 1)] {
            let mut a = Replica::seed(left, &bytes).unwrap();
            let mut b = Replica::join(right, &bytes, &a.save()).unwrap();
            let d = b.view().unwrap();
            let t = table(d);
            let rs = rows(d);
            let ix = usize::from(!undo_full_row_creation);
            let f = rs[rs.len() - 2];
            let p = rs[rs.len() - 1];
            if undo_full_row_creation {
                b.apply(&Command::InsertRow {
                    row: p,
                    below: true,
                    like: Some(f),
                })
                .unwrap();
                sync(&mut a, &mut b);
            }
            for row in rs[ix..rs.len() - 1].iter().rev().copied().chain([p]) {
                a.apply(&Command::RemoveNode { node: row }).unwrap();
            }
            for index in (kept_columns..3).rev() {
                a.apply(&Command::RemoveColumn { table: t, index }).unwrap();
            }
            if undo_full_row_creation {
                assert!(b.undo().unwrap());
            } else {
                b.apply(&Command::RemoveNode { node: rs[0] }).unwrap();
            }
            sync(&mut a, &mut b);
            let d = a.view().unwrap();
            assert_eq!(
                grid_len(d),
                if undo_full_row_creation {
                    3
                } else {
                    kept_columns
                }
            );
            assert_eq!(
                rows(d),
                if undo_full_row_creation {
                    vec![p]
                } else {
                    vec![rs[0]]
                }
            );
            assert_eq!(
                a.resolutions().unwrap().contains(&Resolution::SourceGrid),
                undo_full_row_creation
            );
            let reopened = Document::open(&a.export().unwrap()).unwrap();
            assert!(reopened
                .diagnostics()
                .iter()
                .all(|d| d.code != "table_grid_mismatch"));
            let state = a.save();
            let mut fresh = Replica::join(3, &bytes, &state).unwrap();
            assert_eq!(fresh.export().unwrap(), a.export().unwrap());
            assert_eq!(state, a.save());
            if undo_full_row_creation {
                assert!(b.redo().unwrap());
                sync(&mut a, &mut b);
                assert_eq!(grid_len(a.view().unwrap()), kept_columns);
                assert!(b.undo().unwrap());
                sync(&mut a, &mut b);
                assert_eq!(grid_len(a.view().unwrap()), 3);
            }
        }
    }
}

#[test]
fn removing_a_row_obstruction_activates_the_final_column_grid() {
    let cell = "<w:tc><w:p/></w:tc>";
    let row = format!("<w:tr>{cell}{cell}</w:tr>");
    let bytes = with_body(&format!("<w:tbl><w:tblGrid><w:gridCol w:w=\"2000\"/><w:gridCol w:w=\"2000\"/></w:tblGrid><w:tr><x:odd xmlns:x=\"urn:test\"/>{cell}{cell}</w:tr>{row}{row}</w:tbl><w:p/>"));
    for column_first in [false, true] {
        let mut a = Replica::seed(1, &bytes).unwrap();
        let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
        let d = a.view().unwrap();
        let t = table(d);
        let rs = rows(d);
        let atom = d.node(rs[0]).unwrap().children[0];
        let add = Command::InsertColumn {
            table: t,
            index: 1,
            like: 0,
        };
        if column_first {
            a.apply(&Command::RemoveNode { node: rs[0] }).unwrap();
            sync(&mut a, &mut b);
            b.apply(&add).unwrap();
            a.undo().unwrap();
            sync(&mut a, &mut b);
            assert_eq!(grid_len(a.view().unwrap()), 2);
            a.apply(&Command::RemoveNode { node: atom }).unwrap();
        } else {
            a.apply(&Command::RemoveNode { node: rs[2] }).unwrap();
            a.apply(&Command::RemoveNode { node: atom }).unwrap();
            a.apply(&add).unwrap();
        }
        sync(&mut a, &mut b);
        let d = a.view().unwrap();
        assert_eq!(grid_len(d), 3, "column first: {column_first}");
        assert_eq!(
            d.node(rs[0]).unwrap().children.len(),
            3,
            "column first: {column_first}"
        );
        let xml = part(&a.export().unwrap(), "word/document.xml");
        assert!(!xml.contains("x:odd"));
        let fresh = Document::open(&a.export().unwrap()).unwrap();
        assert!(fresh
            .diagnostics()
            .iter()
            .all(|d| d.code != "table_grid_mismatch"));
        if column_first {
            assert!(a.undo().unwrap());
            sync(&mut a, &mut b);
            assert_eq!(grid_len(a.view().unwrap()), 2);
            assert!(a.redo().unwrap());
            sync(&mut a, &mut b);
            assert_eq!(grid_len(a.view().unwrap()), 3);
        }
    }
}

#[test]
fn choosing_the_outer_grid_preserves_edits_inside_its_rows() {
    let c = "<w:tc><w:p/></w:tc>";
    let z = format!("<w:tr>{c}{c}</w:tr>");
    let inner = format!("<w:tbl><w:tblGrid><w:gridCol w:w=\"2000\"/></w:tblGrid><w:tr>{c}</w:tr><w:tr>{c}</w:tr></w:tbl>");
    let bytes=with_body(&format!("<w:tbl><w:tblGrid><w:gridCol w:w=\"2000\"/><w:gridCol w:w=\"2000\"/></w:tblGrid><w:tr>{c}<w:tc>{inner}<w:p/></w:tc></w:tr><w:tr><w:sdt><w:sdtContent>{c}</w:sdtContent></w:sdt>{c}</w:tr>{z}</w:tbl><w:p/>"));
    for remove_table in [true, false] {
        let mut a = Replica::seed(1, &bytes).unwrap();
        let d = a.view().unwrap();
        let outer = table(d);
        let nested = d
            .descendants(outer)
            .into_iter()
            .find(|id| *id != outer && matches!(d.node(*id).unwrap().kind, NodeKind::Table(_)))
            .unwrap();
        let rs = rows(d);
        let removed = if remove_table {
            nested
        } else {
            d.node(nested).unwrap().children[1]
        };
        for cmd in [
            Command::InsertRow {
                row: rs[2],
                below: true,
                like: None,
            },
            Command::RemoveNode { node: removed },
            Command::RemoveNode { node: rs[1] },
            Command::RemoveColumn {
                table: outer,
                index: 0,
            },
        ] {
            a.apply(&cmd).unwrap();
        }
        let mut fresh = Replica::join(2, &bytes, &a.save()).unwrap();
        assert_eq!(a.export().unwrap(), fresh.export().unwrap());
        let d = Document::open(&a.export().unwrap()).unwrap();
        let tables: Vec<_> = d
            .descendants(table(&d))
            .into_iter()
            .filter(|id| matches!(d.node(*id).unwrap().kind, NodeKind::Table(_)))
            .collect();
        assert_eq!(tables.len(), usize::from(!remove_table));
        if !remove_table {
            assert_eq!(d.node(tables[0]).unwrap().children.len(), 1);
        }
    }
}

#[test]
fn a_nested_table_keeps_its_own_grid_decision_when_the_outer_fallback_reveals_it() {
    let make_table = |inside: &str| {
        let c = "<w:tc><w:p/></w:tc>";
        let g = "<w:gridCol w:w=\"2000\"/>".repeat(3);
        format!("<w:tbl><w:tblGrid>{g}</w:tblGrid><w:tr><w:sdt><w:sdtContent><w:tc><w:tcPr><w:vMerge w:val=\"restart\"/></w:tcPr><w:p/></w:tc></w:sdtContent></w:sdt>{c}{c}</w:tr><w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc>{c}{c}</w:tr><w:tr><w:trPr><w:gridBefore w:val=\"1\"/></w:trPr><w:tc>{inside}<w:p/></w:tc>{c}</w:tr></w:tbl>")
    };
    let bytes = with_body(&format!("{}<w:p/>", make_table(&make_table(""))));
    for (left, right) in [(1, 2), (2, 1)] {
        let mut a = Replica::seed(left, &bytes).unwrap();
        let mut b = Replica::join(right, &bytes, &a.save()).unwrap();
        let d = b.view().unwrap();
        let outer = table(d);
        let inner = d
            .descendants(outer)
            .into_iter()
            .find(|id| matches!(d.node(*id).unwrap().kind, NodeKind::Table(_)))
            .unwrap();
        let targets = [inner, outer].map(|id| (id, d.node(id).unwrap().children.clone()));
        for (_, rs) in &targets {
            b.apply(&Command::InsertRow {
                row: rs[2],
                below: true,
                like: Some(rs[1]),
            })
            .unwrap();
        }
        sync(&mut a, &mut b);
        for (id, rs) in &targets {
            for row in [rs[1], rs[0], rs[2]] {
                a.apply(&Command::RemoveNode { node: row }).unwrap();
            }
            for index in [2, 1] {
                a.apply(&Command::RemoveColumn { table: *id, index })
                    .unwrap();
            }
        }
        assert!(b.undo().unwrap());
        assert!(b.undo().unwrap());
        sync(&mut a, &mut b);
        let d = a.view().unwrap();
        assert!(d.descendants(outer).contains(&inner));
        for (id, rs) in &targets {
            let n = d.node(*id).unwrap();
            assert_eq!(n.children, vec![rs[2]]);
            assert!(matches!(&n.kind,NodeKind::Table(t) if t.grid.len()==3));
        }
        let mut fresh = Replica::join(3, &bytes, &a.save()).unwrap();
        assert_eq!(a.export().unwrap(), fresh.export().unwrap());
    }
}

#[test]
fn a_row_copied_outside_the_declared_grid_keeps_its_template_cells() {
    let cell = |t: &str| format!("<w:tc><w:p><w:r><w:t>{t}</w:t></w:r></w:p></w:tc>");
    for (grid, cells) in [
        ("", cell("A")),
        (r#"<w:gridCol w:w="3000"/>"#, cell("A") + &cell("B")),
    ] {
        let bytes = with_body(&format!(
            "<w:tbl><w:tblGrid>{grid}</w:tblGrid><w:tr>{cells}</w:tr></w:tbl><w:p/>"
        ));
        let insert = |d: &Document| Command::InsertRow {
            row: rows(d)[0],
            below: true,
            like: None,
        };
        let counts = |d: &Document| -> Vec<usize> {
            rows(d)
                .iter()
                .map(|r| d.node(*r).unwrap().children.len())
                .collect()
        };
        let mut one = docx_engine::editing::Editor::open(&bytes).unwrap();
        let command = insert(one.view().unwrap());
        one.apply(&command).unwrap();
        let mut a = Replica::seed(1, &bytes).unwrap();
        let command = insert(a.view().unwrap());
        a.apply(&command).unwrap();
        assert_eq!(
            counts(a.view().unwrap()),
            counts(one.view().unwrap()),
            "grid {grid:?}"
        );
        let d = a.view().unwrap();
        let last = *d.node(rows(d)[1]).unwrap().children.last().unwrap();
        let p = d.node(last).unwrap().children[0];
        a.apply(&Command::InsertText {
            at: Point::Start { start: p },
            text: "typed".into(),
        })
        .unwrap();
        let exported = a.export().unwrap();
        assert!(part(&exported, "word/document.xml").contains("typed"));
        let mut fresh = Replica::join(2, &bytes, &a.save()).unwrap();
        assert!(fresh.export().unwrap() == exported);
        assert!(a.undo().unwrap());
        assert!(a.undo().unwrap());
        assert_eq!(counts(a.view().unwrap()).len(), 1);
    }
}

#[test]
fn copying_a_created_column_uses_its_covering_cells_properties() {
    let bytes = with_body(
        r#"<w:tbl><w:tblGrid><w:gridCol/><w:gridCol/><w:gridCol/></w:tblGrid><w:tr>
        <w:tc><w:tcPr><w:gridSpan w:val="2"/><w:shd w:fill="112233"/></w:tcPr><w:p><w:pPr><w:jc w:val="right"/></w:pPr><w:r><w:t>G</w:t></w:r></w:p></w:tc>
        <w:tc><w:tcPr><w:shd w:fill="AABBCC"/></w:tcPr><w:p><w:r><w:t>H</w:t></w:r></w:p></w:tc>
        </w:tr></w:tbl>"#,
    );
    let mut replica = Replica::seed(1, &bytes).unwrap();
    let t = table(replica.view().unwrap());
    for (index, like) in [(1, 2), (4, 1)] {
        replica
            .apply(&Command::InsertColumn {
                table: t,
                index,
                like,
            })
            .unwrap();
    }
    let doc = Document::open(&replica.export().unwrap()).unwrap();
    assert_eq!(cell_colors(&doc).last().unwrap().as_deref(), Some("112233"));
    let last = *doc.node(rows(&doc)[0]).unwrap().children.last().unwrap();
    let p = doc.node(last).unwrap().children[0];
    let shown = serde_json::to_value(doc.inspect_node(p).unwrap()).unwrap();
    assert_eq!(shown["effective"]["alignment"], "right");
}
