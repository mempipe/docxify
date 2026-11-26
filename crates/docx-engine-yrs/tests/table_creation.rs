#[path = "../../docx-engine/tests/common/mod.rs"]
mod common;
use docx_engine::editing::Resolution;
use docx_engine::{Command, Document, NodeId, NodeKind};
use docx_engine_yrs::{store::Store, Replica};

fn example(wrapped_start: bool) -> Vec<u8> {
    let row = |cells: String| format!("<w:tr>{cells}</w:tr>");
    let cell = |name: char, merge: &str| {
        format!(
            r#"<w:tc><w:tcPr><w:shd w:fill="00FF00"/>{merge}</w:tcPr><w:p><w:r><w:t>{name}</w:t></w:r></w:p></w:tc>"#
        )
    };
    let wrap = |tc: String| {
        format!(
            r#"<w:sdt><w:sdtPr><w:tag w:val="P"/></w:sdtPr><w:sdtContent>{tc}</w:sdtContent></w:sdt>"#
        )
    };
    let start = if wrapped_start {
        r#"<w:vMerge w:val="restart"/>"#
    } else {
        ""
    };
    let continuation = if wrapped_start { "<w:vMerge/>" } else { "" };
    let rows = [
        row(cell('A', "") + &cell('B', "")),
        row(wrap(cell('W', start)) + &cell('F', "")),
        row(cell('Y', continuation) + &cell('Z', "")),
    ]
    .join("");
    common::with_body(&format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>{rows}</w:tbl><w:p/>"#
    ))
}
fn nodes(doc: &Document) -> (NodeId, Vec<NodeId>, NodeId) {
    let t = doc
        .stories()
        .flat_map(|s| doc.descendants(s.id))
        .find(|id| matches!(doc.node(*id).unwrap().kind, NodeKind::Table(_)))
        .unwrap();
    let rows = doc.node(t).unwrap().children.clone();
    let wrapper = doc.node(rows[1]).unwrap().children[0];
    (t, rows, wrapper)
}
#[test]
fn replica_commands_supply_the_vacancy_facts_in_either_client_order() {
    let bytes = example(false);

    for col in 0..2 {
        for (left, right) in [(1, 2), (2, 1)] {
            let mut a = Replica::seed(left, &bytes).unwrap();
            let base = a.save();
            let mut b = Replica::join(right, &bytes, &base).unwrap();
            let (table, rows, wrapper) = nodes(a.view().unwrap());
            b.apply(&Command::RemoveNode { node: rows[1] }).unwrap();
            a.apply_update(&b.update_since(&a.state_vector()).unwrap())
                .unwrap();
            a.apply(&Command::RemoveColumn { table, index: col })
                .unwrap();
            assert!(b.undo().unwrap());
            b.apply_update(&a.update_since(&b.state_vector()).unwrap())
                .unwrap();
            assert!(b.resolutions().unwrap().contains(&Resolution::SourceGrid));
            b.apply(&Command::RemoveNode { node: wrapper }).unwrap();
            let d = b.view().unwrap();
            let w = d.node(rows[1]).unwrap();
            assert_eq!(w.children.len(), 1);
            let p = d.node(w.children[0]).unwrap().children[0];
            assert_eq!(d.text(p).unwrap(), if col == 0 { "F" } else { "" });
            if col == 1 {
                let prior = b.save();
                assert!(b
                    .apply(&Command::InsertText {
                        at: docx_engine::Point::Start { start: p },
                        text: "unexpected".into()
                    })
                    .is_err());
                assert_eq!(b.save(), prior);
            }
            let fresh = Document::open(&b.export().unwrap()).unwrap();
            assert!(fresh
                .diagnostics()
                .iter()
                .all(|d| d.code != "table_grid_mismatch"));
            // The observers receive the same causally dependent deltas in
            // opposite orders. Each packet also arrives a second time.
            let empty = Store::load(9, &base).unwrap();
            let ua = a.update_since(&empty.state_vector()).unwrap();
            let ub = b.update_since(&a.state_vector()).unwrap();
            for packets in [[&ua, &ub], [&ub, &ua]] {
                let mut reader = Replica::join(9, &bytes, &base).unwrap();
                for u in packets {
                    reader.apply_update(u).unwrap();
                    reader.apply_update(u).unwrap();
                }
                let vector = reader.state_vector();
                assert!(reader.resolutions().unwrap().is_empty());
                assert_eq!(reader.export().unwrap(), b.export().unwrap());
                assert_eq!(reader.state_vector(), vector);
            }
        }
    }
}

#[test]
fn deletion_of_a_wrapped_merge_start_checks_the_visible_dependency() {
    let bytes = example(true);
    let doc = Document::open(&bytes).unwrap();
    let (_, _, wrapper) = nodes(&doc);
    let remove = Command::RemoveNode { node: wrapper };
    let mut headless = docx_engine::editing::Editor::open(&bytes).unwrap();
    let before = headless.export().unwrap();
    assert!(headless
        .apply(&remove)
        .unwrap_err()
        .to_string()
        .contains("continues below"));
    assert_eq!(headless.export().unwrap(), before);
    let mut a = Replica::seed(1, &bytes).unwrap();
    let saved = a.save();
    assert!(a
        .apply(&remove)
        .unwrap_err()
        .to_string()
        .contains("continues below"));
    assert_eq!(a.save(), saved);
    assert!(!a.undo().unwrap());
}

#[test]
fn undo_revealing_a_continuation_retains_the_entire_start_wrapper() {
    for (first, second) in [(1, 2), (2, 1)] {
        let bytes = example(true);
        let xml = common::part(&bytes, "word/document.xml");
        let raw =
            &xml[xml.find("<w:sdt>").unwrap()..xml.find("</w:sdt>").unwrap() + "</w:sdt>".len()];
        let mut a = Replica::seed(first, &bytes).unwrap();
        let (_, rows, wrapper) = nodes(a.view().unwrap());
        let mut b = Replica::join(second, &bytes, &a.save()).unwrap();
        a.apply(&Command::RemoveNode { node: rows[2] }).unwrap();
        b.apply_update(&a.update_since(&b.state_vector()).unwrap())
            .unwrap();
        b.apply(&Command::RemoveNode { node: wrapper }).unwrap();
        a.undo().unwrap();
        a.apply_update(&b.update_since(&a.state_vector()).unwrap())
            .unwrap();
        b.apply_update(&a.update_since(&b.state_vector()).unwrap())
            .unwrap();
        for replica in [&mut a, &mut b] {
            let saved = replica.save();
            assert!(replica
                .resolutions()
                .unwrap()
                .contains(&Resolution::MergeStartWrapper));
            assert!(common::part(&replica.export().unwrap(), "word/document.xml").contains(raw));
            let mut fresh = Replica::join(9, &bytes, &saved).unwrap();
            assert_eq!(fresh.export().unwrap(), replica.export().unwrap());
            assert_eq!(replica.save(), saved);
        }
        assert!(a.redo().unwrap());
        b.apply_update(&a.update_since(&b.state_vector()).unwrap())
            .unwrap();
        assert!(!common::part(&b.export().unwrap(), "word/document.xml").contains(raw));
        assert!(b.resolutions().unwrap().is_empty());
    }
}

// A literal grid names the independently expected cell at every column.
// Distinct donor shades make a wrong template visible even for empty copies.
#[test]
fn default_reader_matches_concurrent_row_and_column_creation_grid() {
    let cell = |label: &str, span: usize, shade: &str| {
        format!(
            r#"<w:tc><w:tcPr><w:gridSpan w:val="{span}"/><w:shd w:fill="{shade}"/></w:tcPr><w:p><w:r><w:t>{label}</w:t></w:r></w:p></w:tc>"#
        )
    };
    let rows = [
        cell("A", 1, "000001") + &cell("B", 1, "000002") + &cell("C", 1, "000003"),
        cell("G", 2, "000004") + &cell("H", 1, "000005"),
    ];
    let bytes = common::with_body(&format!(
        "<w:tbl><w:tblGrid>{}</w:tblGrid><w:tr>{}</w:tr><w:tr>{}</w:tr></w:tbl><w:p/>",
        r#"<w:gridCol w:w="2000"/>"#.repeat(3),
        rows[0],
        rows[1]
    ));
    let mut seed = Replica::seed(90, &bytes).unwrap();
    let base = seed.save();
    let vector = seed.state_vector();
    let doc = Document::open(&bytes).unwrap();
    let (t, rs, _) = nodes(seed.view().unwrap());
    let update = |author, cmd: Command| {
        let mut replica = Replica::join(author, &bytes, &base).unwrap();
        replica.apply(&cmd).unwrap();
        replica.update_since(&vector).unwrap()
    };
    let mut histories = 0;
    for [row_author, col_author, drop_author] in [
        [1, 2, 3],
        [1, 3, 2],
        [2, 1, 3],
        [2, 3, 1],
        [3, 1, 2],
        [3, 2, 1],
    ] {
        let drops: Vec<_> = (0..3)
            .map(|index| update(drop_author, Command::RemoveColumn { table: t, index }))
            .collect();
        for at in 0usize..=2 {
            for template in 0..2 {
                let row_update = update(
                    row_author,
                    Command::InsertRow {
                        row: rs[at.min(1)],
                        below: at == 2,
                        like: Some(rs[template]),
                    },
                );
                for index in 0..=3 {
                    for donor in 0..3 {
                        let col_update = update(
                            col_author,
                            Command::InsertColumn {
                                table: t,
                                index,
                                like: donor,
                            },
                        );
                        for drop in [None, Some(0usize), Some(1), Some(2)] {
                            let mut receiver = Store::load(91, &base).unwrap();
                            if let Some(i) = drop {
                                receiver.apply_update(&drops[i][..]).unwrap();
                            }
                            receiver.apply_update(&col_update).unwrap();
                            receiver.apply_update(&row_update).unwrap();
                            let view = docx_engine_yrs::build(&doc, &receiver).unwrap();
                            assert!(view.resolutions.is_empty());
                            let rendered = &view.document;
                            let mut source_rows = vec![(0, false), (1, false)];
                            source_rows.insert(at, (template, true));
                            let actual = &rendered.node(t).unwrap().children;
                            assert_eq!(actual.len(), 3);
                            for (&r, (source, copy)) in actual.iter().zip(source_rows) {
                                let labels = if source == 0 {
                                    ["A", "B", "C"]
                                } else {
                                    ["G", "G", "H"]
                                };
                                let shades = if source == 0 {
                                    ["000001", "000002", "000003"]
                                } else {
                                    ["000004", "000004", "000005"]
                                };
                                let mut expected = Vec::new();
                                for c in 0..=3 {
                                    if c == index {
                                        let extended = source == 1 && index == 1;
                                        let basis = if extended { 0 } else { donor };
                                        expected.push((
                                            if extended { labels[basis] } else { "new" },
                                            if extended && !copy { labels[basis] } else { "" },
                                            shades[basis],
                                        ));
                                    }
                                    if c < 3 && Some(c) != drop {
                                        expected.push((
                                            labels[c],
                                            if copy { "" } else { labels[c] },
                                            shades[c],
                                        ));
                                    }
                                }
                                let cells = &rendered.node(r).unwrap().children;
                                let groups: Vec<_> = expected.chunk_by(|a, b| a.0 == b.0).collect();
                                assert_eq!(cells.len(), groups.len());
                                for (&id, group) in cells.iter().zip(groups) {
                                    let n = rendered.node(id).unwrap();
                                    let NodeKind::Cell(info) = &n.kind else {
                                        panic!("expected cell")
                                    };
                                    assert_eq!(info.grid_span as usize, group.len());
                                    assert_eq!(info.props.shading.as_deref(), Some(group[0].2));
                                    assert_eq!(rendered.text(n.children[0]).unwrap(), group[0].1);
                                }
                            }
                            histories += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(histories, 1728);
}

#[test]
fn retained_atom_and_opaque_continuation_keep_all_their_source_bytes() {
    let xml = common::part(&example(true), "word/document.xml");
    let table = xml
        .split("<w:body>")
        .nth(1)
        .unwrap()
        .split("</w:body>")
        .next()
        .unwrap();
    let wrapped = table
        .split("<w:sdt>")
        .nth(1)
        .unwrap()
        .split("</w:sdt>")
        .next()
        .unwrap();
    let start = format!(
        "<w:sdt>{}</w:sdt>",
        wrapped.replace(
            "</w:sdtContent>",
            "<w:tc><w:p><w:r><w:t>sibling</w:t></w:r></w:p></w:tc></w:sdtContent>"
        )
    );
    let continuation = r#"<w:sdt><w:sdtPr><w:tag w:val="continuation"/></w:sdtPr><w:sdtContent><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc></w:sdtContent></w:sdt>"#;
    let bytes = common::with_body(&format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid><w:tr>{start}</w:tr><w:tr>{continuation}<w:tc><w:p/></w:tc></w:tr></w:tbl><w:p/>"#
    ));
    let mut a = Replica::seed(1, &bytes).unwrap();
    let doc = a.view().unwrap();
    let (_, rows, _) = nodes(doc);
    let remove = Command::RemoveNode {
        node: doc.node(rows[0]).unwrap().children[0],
    };
    assert!(a.apply(&remove).is_err());
    let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
    a.apply(&Command::RemoveNode { node: rows[1] }).unwrap();
    b.apply_update(&a.update_since(&b.state_vector()).unwrap())
        .unwrap();
    b.apply(&remove).unwrap();
    assert!(a.undo().unwrap());
    b.apply_update(&a.update_since(&b.state_vector()).unwrap())
        .unwrap();
    let output = common::part(&b.export().unwrap(), "word/document.xml");
    assert!(output.contains(&start) && output.contains(continuation));
    assert!(a.redo().unwrap());
    b.apply_update(&a.update_since(&b.state_vector()).unwrap())
        .unwrap();
    let output = common::part(&b.export().unwrap(), "word/document.xml");
    assert!(!output.contains("sibling"));
}

#[test]
fn restoring_a_column_holds_an_insertion_that_would_split_an_owner() {
    let bytes = common::with_body(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr><w:p><w:r><w:t>G</w:t></w:r></w:p></w:tc><w:tc><w:p/></w:tc></w:tr><w:tr><w:tc><w:p/></w:tc><w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr><w:p/></w:tc></w:tr></w:tbl><w:p/>"#,
    );
    let mut a = Replica::seed(1, &bytes).unwrap();
    let (t, _, _) = nodes(a.view().unwrap());
    let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
    a.apply(&Command::RemoveColumn { table: t, index: 1 })
        .unwrap();
    b.apply_update(&a.update_since(&b.state_vector()).unwrap())
        .unwrap();
    b.apply(&Command::InsertColumn {
        table: t,
        index: 1,
        like: 0,
    })
    .unwrap();
    a.undo().unwrap();
    b.apply_update(&a.update_since(&b.state_vector()).unwrap())
        .unwrap();
    assert_eq!(
        common::part(&b.export().unwrap(), "word/document.xml"),
        common::part(&bytes, "word/document.xml")
    );
    assert!(b.resolutions().unwrap().contains(&Resolution::SourceGrid));
    a.redo().unwrap();
    b.apply_update(&a.update_since(&b.state_vector()).unwrap())
        .unwrap();
    assert!(b.resolutions().unwrap().is_empty());
}

#[test]
fn removing_a_table_after_removing_its_bookmark_uses_the_current_document() {
    let bytes = common::with_body(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:p><w:bookmarkStart w:id="3" w:name="inside"/><w:r><w:t>inside</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:bookmarkEnd w:id="3"/></w:p>"#,
    );
    let mut a = Replica::seed(1, &bytes).unwrap();
    let d = a.view().unwrap();
    let ids = d
        .stories()
        .flat_map(|s| d.descendants(s.id))
        .collect::<Vec<_>>();
    let t = *ids
        .iter()
        .find(|id| matches!(d.node(**id).unwrap().kind, NodeKind::Table(_)))
        .unwrap();
    let start=*ids.iter().find(|id|matches!(&d.node(**id).unwrap().kind,NodeKind::Marker{element,..} if element.ends_with("bookmarkStart"))).unwrap();
    assert!(a.apply(&Command::RemoveNode { node: t }).is_err());
    a.apply(&Command::RemoveNode { node: start }).unwrap();
    a.apply(&Command::RemoveNode { node: t }).unwrap();
    let mut fresh = Replica::join(2, &bytes, &a.save()).unwrap();
    assert_eq!(fresh.export().unwrap(), a.export().unwrap());
    assert!(!common::part(&a.export().unwrap(), "word/document.xml").contains("<w:tbl>"));
    assert!(a.undo().unwrap());
    let xml = common::part(&a.export().unwrap(), "word/document.xml");
    assert!(xml.contains("<w:tbl>") && !xml.contains("bookmarkStart"));
}

#[test]
fn a_last_column_is_released_when_another_author_supplies_a_column() {
    let bytes = common::with_body(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:p/></w:tc><w:tc><w:p/></w:tc></w:tr><w:tr><w:tc><w:p/></w:tc><w:tc><w:p/></w:tc></w:tr></w:tbl><w:p/>"#,
    );
    let mut a = Replica::seed(1, &bytes).unwrap();
    let (t, _, _) = nodes(a.view().unwrap());
    let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
    let mut c = Replica::join(3, &bytes, &a.save()).unwrap();
    let base = a.state_vector();
    a.apply(&Command::RemoveColumn { table: t, index: 0 })
        .unwrap();
    b.apply(&Command::RemoveColumn { table: t, index: 1 })
        .unwrap();
    c.apply(&Command::InsertColumn {
        table: t,
        index: 1,
        like: 0,
    })
    .unwrap();
    a.apply_update(&b.update_since(&base).unwrap()).unwrap();
    assert!(a.resolutions().unwrap().contains(&Resolution::LastColumn));
    a.apply_update(&c.update_since(&base).unwrap()).unwrap();
    let NodeKind::Table(info) = &a.view().unwrap().node(t).unwrap().kind else {
        unreachable!()
    };
    assert_eq!(info.grid, [4000]);
    assert!(a.resolutions().unwrap().is_empty());
}
