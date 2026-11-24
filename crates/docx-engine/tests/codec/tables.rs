use super::common;

use common::*;
use docx_engine::{Command, Document, NodeId, NodeKind, Point, COMMANDS};

const DOCUMENT: &str = "word/document.xml";

fn parent(doc: &Document, id: NodeId) -> NodeId {
    doc.node(id).unwrap().parent.unwrap()
}

/// The cell, row and table holding the text node with `needle`.
fn cell_of(doc: &Document, needle: &str) -> (NodeId, NodeId, NodeId) {
    let mut id = text_node(doc, needle);
    while !matches!(doc.node(id).unwrap().kind, NodeKind::Cell(_)) {
        id = parent(doc, id);
    }
    let row = parent(doc, id);
    (id, row, parent(doc, row))
}

/// Each row of a table as the text of its cells.
fn grid_text(doc: &Document, table: NodeId) -> Vec<Vec<String>> {
    doc.node(table)
        .unwrap()
        .children
        .iter()
        .map(|&row| {
            doc.node(row)
                .unwrap()
                .children
                .iter()
                .map(|&cell| {
                    doc.node(cell)
                        .unwrap()
                        .children
                        .iter()
                        .filter_map(|&p| doc.text(p))
                        .collect::<Vec<_>>()
                        .join("|")
                })
                .collect()
        })
        .collect()
}

fn widths(doc: &Document, table: NodeId) -> Vec<u32> {
    match &doc.node(table).unwrap().kind {
        NodeKind::Table(info) => info.grid.clone(),
        _ => panic!("not a table"),
    }
}

/// Each cell of a row as its span, vertical merge and width.
fn cells(doc: &Document, row: NodeId) -> Vec<(u32, Option<String>, Option<u32>)> {
    doc.node(row)
        .unwrap()
        .children
        .iter()
        .map(|&c| match &doc.node(c).unwrap().kind {
            NodeKind::Cell(info) => (info.grid_span, info.vertical_merge.clone(), info.width),
            _ => panic!("not a cell"),
        })
        .collect()
}

fn refused(name: &str, doc: &mut EditingDocument, command: Command, reason: &str) {
    let before = stories(doc);
    let dirty = doc.is_dirty();
    let error = doc.run(command.clone()).expect_err("refused");
    assert!(
        error.to_string().contains(reason),
        "{name}: {command:?}: {error}"
    );
    assert_eq!(
        stories(doc),
        before,
        "{name}: {command:?} changed the document"
    );
    assert_eq!(
        doc.is_dirty(),
        dirty,
        "{name}: {command:?} dirtied the document"
    );
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_row_inserted_below_copies_its_row_and_continues_a_merge_across_it() {
    let bytes = generated("table");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let (_, ada, table) = cell_of(&doc, "Ada");
    edit(
        &mut doc,
        Command::InsertRow {
            row: ada,
            below: true,
            like: None,
        },
    );
    let rows = grid_text(&doc, table);
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[2], s(&["", ""]));
    let new = doc.node(table).unwrap().children[2];
    assert_eq!(
        cells(&doc, new),
        [
            (1, Some("continue".into()), Some(3000)),
            (2, None, Some(3000))
        ]
    );
    let xml = part(&doc.export().unwrap(), DOCUMENT);
    assert!(
        xml.contains(r#"<w:p><w:r><w:t>Spans two columns</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/><w:vMerge/></w:tcPr><w:p></w:p></w:tc><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/><w:gridSpan w:val="2"/></w:tcPr><w:p></w:p></w:tc></w:tr><w:tr>"#),
        "{xml}"
    );
    let reopened = reopen("table", &doc);
    let (_, _, table) = cell_of(&reopened, "Ada");
    assert_eq!(
        cells(&reopened, reopened.node(table).unwrap().children[2]),
        cells(&doc, new)
    );
    assert_eq!(grid_text(&reopened, table), rows);
    assert_editor_undo("table", &bytes, &mut doc);
}

#[test]
fn a_row_inserted_above_the_header_copies_it_without_the_header_flag() {
    let bytes = generated("table");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let (_, header, table) = cell_of(&doc, "Name");
    let (_, last, _) = cell_of(&doc, "Last cell");
    edit(
        &mut doc,
        Command::InsertRow {
            row: header,
            below: false,
            like: None,
        },
    );
    let new = doc.node(table).unwrap().children[0];
    match &doc.node(new).unwrap().kind {
        NodeKind::Row(info) => assert!(!info.header),
        _ => panic!("not a row"),
    }
    // A merge that ends in the row above stays ended.
    edit(
        &mut doc,
        Command::InsertRow {
            row: last,
            below: true,
            like: None,
        },
    );
    let rows = grid_text(&doc, table);
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0], s(&["", "", ""]));
    assert_eq!(rows[4], s(&["", "", ""]));
    let added = doc.node(table).unwrap().children[4];
    assert_eq!(
        cells(&doc, added),
        [
            (1, None, Some(3000)),
            (1, None, Some(3000)),
            (1, None, Some(3000))
        ]
    );
    let xml = part(&doc.export().unwrap(), DOCUMENT);
    assert!(
        xml.contains(r#"<w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/></w:tcPr><w:p></w:p></w:tc>"#),
        "{xml}"
    );
    let reopened = reopen("table", &doc);
    let (_, _, again) = cell_of(&reopened, "Name");
    assert_eq!(widths(&reopened, again), [3000; 3]);
    let top = reopened.node(again).unwrap().children[0];
    assert!(matches!(&reopened.node(top).unwrap().kind, NodeKind::Row(info) if !info.header));
    assert_eq!(cells(&reopened, top), cells(&doc, new));
    assert_eq!(grid_text(&reopened, again), rows);
    assert!(doc.0.undo());
    assert_editor_undo("table", &bytes, &mut doc);
}

#[test]
fn a_column_inserted_halves_the_column_it_copies() {
    let bytes = generated("table");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let (_, header, table) = cell_of(&doc, "Name");
    let (_, ada, _) = cell_of(&doc, "Ada");
    edit(
        &mut doc,
        Command::InsertColumn {
            table,
            index: 1,
            like: 0,
        },
    );
    assert_eq!(widths(&doc, table), [1500, 1500, 3000, 3000]);
    assert_eq!(
        grid_text(&doc, table),
        [
            s(&["Name", "", "Role", "Notes"]),
            s(&["Ada", "", "Spans two columns"]),
            s(&["", "", "Inner A\nInner B|", "Last cell"]),
        ]
    );
    assert_eq!(
        cells(&doc, header),
        [
            (1, None, Some(1500)),
            (1, None, Some(1500)),
            (1, None, Some(3000)),
            (1, None, Some(3000))
        ]
    );
    // The copy of a merged cell is not merged.
    assert_eq!(
        cells(&doc, ada),
        [
            (1, Some("restart".into()), Some(1500)),
            (1, None, Some(1500)),
            (2, None, Some(3000))
        ]
    );
    let xml = part(&doc.export().unwrap(), DOCUMENT);
    assert!(
        xml.contains(r#"<w:tblGrid><w:gridCol w:w="1500"/><w:gridCol w:w="1500"/><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid>"#),
        "{xml}"
    );
    assert!(
        xml.contains(r#"<w:tc><w:tcPr><w:tcW w:w="1500" w:type="dxa"/><w:vMerge w:val="restart"/></w:tcPr><w:p><w:r><w:t>Ada</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:tcW w:w="1500" w:type="dxa"/></w:tcPr><w:p></w:p></w:tc>"#),
        "{xml}"
    );
    let reopened = reopen("table", &doc);
    let (_, ada_again, again) = cell_of(&reopened, "Ada");
    assert_eq!(widths(&reopened, again), widths(&doc, table));
    assert_eq!(cells(&reopened, ada_again), cells(&doc, ada));
    assert_eq!(grid_text(&reopened, again), grid_text(&doc, table));
    assert_editor_undo("table", &bytes, &mut doc);
}

#[test]
fn a_column_inserted_inside_a_spanning_cell_widens_it() {
    let bytes = generated("table");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let (_, ada, table) = cell_of(&doc, "Ada");
    edit(
        &mut doc,
        Command::InsertColumn {
            table,
            index: 2,
            like: 1,
        },
    );
    assert_eq!(widths(&doc, table), [3000, 1500, 1500, 3000]);
    assert_eq!(
        cells(&doc, ada),
        [
            (1, Some("restart".into()), Some(3000)),
            (3, None, Some(6000))
        ]
    );
    assert_eq!(grid_text(&doc, table)[0], s(&["Name", "Role", "", "Notes"]));
    reopen("table", &doc);
    assert_editor_undo("table", &bytes, &mut doc);
}

#[test]
fn a_column_removed_gives_its_width_to_its_neighbour() {
    let bytes = generated("table");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let (_, ada, table) = cell_of(&doc, "Ada");
    edit(&mut doc, Command::RemoveColumn { table, index: 1 });
    assert_eq!(widths(&doc, table), [6000, 3000]);
    assert_eq!(
        grid_text(&doc, table),
        [
            s(&["Name", "Notes"]),
            s(&["Ada", "Spans two columns"]),
            s(&["", "Last cell"]),
        ]
    );
    assert_eq!(
        cells(&doc, ada),
        [
            (1, Some("restart".into()), Some(6000)),
            (1, None, Some(3000))
        ]
    );
    reopen("table", &doc);
    assert_editor_undo("table", &bytes, &mut doc);

    // The first column gives its width to the one after it.
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let (_, _, table) = cell_of(&doc, "Ada");
    edit(&mut doc, Command::RemoveColumn { table, index: 0 });
    assert_eq!(widths(&doc, table), [6000, 3000]);
    assert_eq!(
        grid_text(&doc, table),
        [
            s(&["Role", "Notes"]),
            s(&["Spans two columns"]),
            s(&["Inner A\nInner B|", "Last cell"]),
        ]
    );
    reopen("table", &doc);
    assert_editor_undo("table", &bytes, &mut doc);
}

#[test]
fn a_row_is_removed_with_remove_node() {
    let bytes = generated("table");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let (_, last, table) = cell_of(&doc, "Last cell");
    edit(&mut doc, Command::RemoveNode { node: last });
    assert_eq!(grid_text(&doc, table).len(), 2);
    reopen("table", &doc);
    assert_editor_undo("table", &bytes, &mut doc);
}

#[test]
fn table_commands_refuse_and_change_nothing() {
    let mut doc = EditingDocument::open(&generated("table")).unwrap();
    let (_, ada, table) = cell_of(&doc, "Ada");
    let (_, inner_row, inner) = cell_of(&doc, "Inner A");
    refused(
        "table",
        &mut doc,
        Command::RemoveNode { node: ada },
        "continues below it",
    );
    refused(
        "table",
        &mut doc,
        Command::RemoveNode { node: inner_row },
        "at least one row",
    );
    refused(
        "table",
        &mut doc,
        Command::InsertRow {
            row: ada,
            below: true,
            like: Some(inner_row),
        },
        "another table",
    );
    refused(
        "table",
        &mut doc,
        Command::InsertColumn {
            table,
            index: 4,
            like: 0,
        },
        "no such column",
    );
    edit(
        &mut doc,
        Command::RemoveColumn {
            table: inner,
            index: 0,
        },
    );
    refused(
        "table",
        &mut doc,
        Command::RemoveColumn {
            table: inner,
            index: 0,
        },
        "at least one column",
    );
    assert!(doc.0.undo());

    let mut doc = EditingDocument::open(&generated("table-layout")).unwrap();
    let (_, _, table) = cell_of(&doc, "Header A");
    refused(
        "table-layout",
        &mut doc,
        Command::InsertColumn {
            table,
            index: 1,
            like: 0,
        },
        "does not fill the table grid",
    );
    refused(
        "table-layout",
        &mut doc,
        Command::RemoveColumn { table, index: 1 },
        "does not fill the table grid",
    );
}

const TRACKED: &str = r#"w:author="A" w:date="2024-01-01T00:00:00Z""#;

fn cell(text: &str) -> String {
    format!(
        r#"<w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>"#
    )
}

fn table(grid: &str, rows: &str) -> String {
    format!(
        r#"<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/>{grid}</w:tblGrid>{rows}</w:tbl><w:p/>"#
    )
}

#[test]
fn tracked_changes_stay_behind_a_copy_and_block_column_commands() {
    let added = format!(
        r#"<w:tr><w:trPr><w:ins w:id="1" {TRACKED}/></w:trPr><w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/></w:tcPr><w:p><w:pPr><w:jc w:val="center"/><w:rPr><w:ins w:id="2" {TRACKED}/></w:rPr></w:pPr><w:ins w:id="3" {TRACKED}><w:r><w:t>Added</w:t></w:r></w:ins></w:p></w:tc>{}</w:tr>"#,
        cell("Plain")
    );
    let deleted = format!(
        r#"<w:tr><w:trPr><w:del w:id="4" {TRACKED}/></w:trPr>{}{}</w:tr>"#,
        cell("Gone"),
        cell("Also gone")
    );
    let changed = format!(
        r#"<w:tr><w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/><w:tcPrChange w:id="5" {TRACKED}><w:tcPr><w:tcW w:w="1000" w:type="dxa"/></w:tcPr></w:tcPrChange></w:tcPr><w:p><w:r><w:t>Changed</w:t></w:r></w:p></w:tc>{}</w:tr>"#,
        cell("Beside")
    );
    let regridded = r#"<w:tblGridChange w:id="6"><w:tblGrid><w:gridCol w:w="1000"/><w:gridCol w:w="3000"/></w:tblGrid></w:tblGridChange>"#;
    let bytes = with_body(&format!(
        "{}{}{}",
        table("", &format!("{added}{deleted}")),
        table("", &changed),
        table(
            regridded,
            &format!("<w:tr>{}{}</w:tr>", cell("Regridded"), cell("Too"))
        )
    ));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let (_, row, first) = cell_of(&doc, "Added");
    let (_, _, second) = cell_of(&doc, "Changed");
    let (_, _, third) = cell_of(&doc, "Regridded");
    let name = "tracked tables";
    refused(
        name,
        &mut doc,
        Command::InsertColumn {
            table: first,
            index: 1,
            like: 0,
        },
        "read-only",
    );
    refused(
        name,
        &mut doc,
        Command::RemoveColumn {
            table: second,
            index: 1,
        },
        "a cell has a tracked change",
    );
    refused(
        name,
        &mut doc,
        Command::InsertColumn {
            table: third,
            index: 0,
            like: 0,
        },
        "the table grid has a tracked change",
    );
    edit(
        &mut doc,
        Command::InsertRow {
            row,
            below: false,
            like: None,
        },
    );
    // Text typed into the copy does not take the tracked mark's insertion.
    let copied = doc
        .node(doc.node(first).unwrap().children[0])
        .unwrap()
        .children[0];
    let paragraph = doc.node(copied).unwrap().children[0];
    edit(
        &mut doc,
        Command::InsertText {
            at: Point::Start { start: paragraph },
            text: "Typed".into(),
        },
    );
    let (_, changed_row, _) = cell_of(&doc, "Changed");
    edit(
        &mut doc,
        Command::InsertRow {
            row: changed_row,
            below: true,
            like: None,
        },
    );
    let xml = part(&doc.export().unwrap(), DOCUMENT);
    assert_eq!(xml.matches("<w:tcPrChange").count(), 1, "{xml}");
    assert!(
        xml.contains(r#"<w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/></w:tcPr><w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:t xml:space="preserve">Typed</w:t></w:r></w:p></w:tc><w:tc>"#),
        "{xml}"
    );
    assert!(
        xml.contains(r#"</w:tr><w:tr><w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/></w:tcPr><w:p></w:p></w:tc><w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/></w:tcPr><w:p></w:p></w:tc></w:tr></w:tbl>"#),
        "{xml}"
    );
    let reopened = reopen(name, &doc);
    let (_, typed_row, typed_table) = cell_of(&reopened, "Typed");
    assert_eq!(
        reopened
            .effective_paragraph(paragraph_with(&reopened, "Typed"))
            .alignment,
        "center"
    );
    assert_eq!(widths(&reopened, typed_table), [2000; 2]);
    assert_eq!(cells(&reopened, typed_row), vec![(1, None, Some(2000)); 2]);
    let (_, _, table) = cell_of(&reopened, "Changed");
    let copy = *reopened.node(table).unwrap().children.last().unwrap();
    assert_eq!(cells(&reopened, copy), vec![(1, None, Some(2000)); 2]);
    assert_eq!(grid_text(&reopened, table).last().unwrap(), &s(&["", ""]));
    assert!(doc.0.undo());
    assert!(doc.0.undo());
    assert_editor_undo(name, &bytes, &mut doc);
}

#[test]
fn a_column_holding_one_end_of_a_bookmark_stays() {
    let marked = r#"<w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/></w:tcPr><w:p><w:bookmarkStart w:id="9" w:name="Mark"/><w:r><w:t>Marked</w:t></w:r></w:p></w:tc>"#;
    let bytes = with_body(&format!(
        r#"{}<w:p><w:r><w:t>After</w:t></w:r><w:bookmarkEnd w:id="9"/></w:p>"#,
        table("", &format!("<w:tr>{}{marked}</w:tr>", cell("Plain")))
    ));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let (_, _, table) = cell_of(&doc, "Marked");
    refused(
        "bookmark",
        &mut doc,
        Command::RemoveColumn { table, index: 1 },
        "bookmark",
    );
    edit(&mut doc, Command::RemoveColumn { table, index: 0 });
    reopen("bookmark", &doc);
    assert_editor_undo("bookmark", &bytes, &mut doc);
}

#[test]
fn every_fixture_table_takes_a_row_and_a_column_and_undoes_exactly() {
    let mut changed = 0;
    for (name, bytes) in all_fixtures() {
        let Ok(doc) = Document::open(&bytes) else {
            continue;
        };
        let tables: Vec<NodeId> = doc
            .stories()
            .flat_map(|s| doc.descendants(s.id))
            .filter(|&id| matches!(doc.node(id).unwrap().kind, NodeKind::Table(_)))
            .collect();
        for table in tables {
            let rows: Vec<NodeId> = doc.node(table).unwrap().children.clone();
            let Some(&row) = rows
                .iter()
                .find(|&&r| matches!(doc.node(r).unwrap().kind, NodeKind::Row(_)))
            else {
                continue;
            };
            for command in [
                Command::InsertRow {
                    row,
                    below: true,
                    like: None,
                },
                Command::InsertColumn {
                    table,
                    index: 1,
                    like: 0,
                },
                Command::RemoveColumn { table, index: 0 },
            ] {
                let mut doc = EditingDocument::open(&bytes).unwrap();
                match doc.0.apply(&command) {
                    Ok(()) => (),
                    Err(error) => {
                        assert!(
                            !doc.is_dirty(),
                            "{name}: {command:?}: {error} dirtied the document"
                        );
                        continue;
                    }
                };
                changed += 1;
                reopen(&name, &doc);
                assert_editor_undo(&name, &bytes, &mut doc);
            }
        }
    }
    assert!(changed >= 30, "only {changed} table changes applied");
}

#[test]
fn table_commands_round_trip_through_json_and_are_listed() {
    let n = |s: &str| s.parse::<NodeId>().unwrap();
    for (command, json) in [
        (
            Command::InsertRow {
                row: n("n4"),
                below: true,
                like: None,
            },
            serde_json::json!({"command": "insert_row", "row": "n4", "below": true}),
        ),
        (
            Command::InsertColumn {
                table: n("n3"),
                index: 1,
                like: 0,
            },
            serde_json::json!({"command": "insert_column", "table": "n3", "index": 1, "like": 0}),
        ),
        (
            Command::RemoveColumn {
                table: n("n3"),
                index: 2,
            },
            serde_json::json!({"command": "remove_column", "table": "n3", "index": 2}),
        ),
    ] {
        assert_eq!(serde_json::to_value(&command).unwrap(), json);
        assert_eq!(serde_json::from_value::<Command>(json).unwrap(), command);
    }
    for name in ["insert_row", "insert_column", "remove_column"] {
        assert!(COMMANDS.contains(&name), "{name}");
    }
}

#[test]
fn removing_the_first_row_keeps_table_properties_after_a_bookmark() {
    let properties = r#"<w:tblPr><w:tblW w:w="4500" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="4500"/></w:tblGrid>"#;
    let bytes = with_body(&format!(
        r#"<w:tbl><w:bookmarkStart w:id="1" w:name="Table"/>{properties}<w:tr>{}</w:tr><w:tr>{}</w:tr><w:bookmarkEnd w:id="1"/></w:tbl>"#,
        cell("First"),
        cell("Last")
    ));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let (_, first, _) = cell_of(&doc, "First");
    edit(&mut doc, Command::RemoveNode { node: first });
    let bytes = doc.export().unwrap();
    assert!(part(&bytes, DOCUMENT).contains(properties));
    let reopened = Document::open(&bytes).unwrap();
    let (_, _, table) = cell_of(&reopened, "Last");
    assert_eq!(widths(&reopened, table), [4500]);
}

#[test]
fn a_column_is_added_to_a_table_whose_properties_follow_a_bookmark() {
    let properties = r#"<w:tblPr><w:tblW w:w="4000" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>"#;
    let bytes = with_body(&format!(
        r#"<w:tbl><w:bookmarkStart w:id="1" w:name="Table"/>{properties}<w:tr>{}{}</w:tr><w:bookmarkEnd w:id="1"/></w:tbl><w:p/>"#,
        cell("First"),
        cell("Last")
    ));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let (_, _, table) = cell_of(&doc, "First");
    edit(
        &mut doc,
        Command::InsertColumn {
            table,
            index: 1,
            like: 0,
        },
    );
    let out = doc.export().unwrap();
    assert_valid("bookmarked table", &out);
    let xml = part(&out, DOCUMENT);
    assert_eq!(xml.matches("<w:tblGrid>").count(), 1, "{xml}");
    assert_eq!(xml.matches("<w:tblPr>").count(), 1, "{xml}");
    assert_eq!(xml.matches(r#"<w:bookmarkStart w:id="1""#).count(), 1);
    let reopened = Document::open(&out).unwrap();
    let (_, _, table) = cell_of(&reopened, "Last");
    assert_eq!(widths(&reopened, table).len(), 3);
    assert_editor_undo("bookmarked table", &bytes, &mut doc);
}

#[test]
fn formatting_and_linking_into_a_new_cell_follow_the_visible_rows() {
    let bytes = generated("table");
    for link in [false, true] {
        let mut doc = EditingDocument::open(&bytes).unwrap();
        let (_, header, table) = cell_of(&doc, "Name");
        edit(
            &mut doc,
            Command::InsertRow {
                row: header,
                below: true,
                like: None,
            },
        );
        let row = doc.node(table).unwrap().children[1];
        let cell = doc.node(row).unwrap().children[0];
        let p = doc.node(cell).unwrap().children[0];
        edit(
            &mut doc,
            Command::InsertText {
                at: Point::Start { start: p },
                text: "NEW".into(),
            },
        );
        let from = at(text_node(&doc, "Name"), 0);
        let to = at(text_node(&doc, "NEW"), 3);
        let command = if link {
            Command::SetLink {
                from,
                to,
                target: Some("https://example.org".into()),
            }
        } else {
            Command::FormatRange {
                from,
                to,
                format: docx_engine::Format {
                    italic: Some(true),
                    ..Default::default()
                },
            }
        };
        edit(&mut doc, command);
        let bytes = doc.export().unwrap();
        let reopened = Document::open(&bytes).unwrap();
        for (text, selected) in [
            ("Name", true),
            ("NEW", true),
            ("Ada", false),
            ("Inner A", false),
            ("After the table.", false),
        ] {
            let leaf = text_node(&reopened, text);
            let run = parent(&reopened, leaf);
            let marked = if link {
                matches!(
                    reopened.node(parent(&reopened, run)).unwrap().kind,
                    NodeKind::Hyperlink(_)
                )
            } else {
                reopened.effective_run_props(run).italic == Some(true)
            };
            assert_eq!(marked, selected, "{text}, link={link}");
        }
    }
}

#[test]
fn a_copied_row_or_cell_keeps_the_namespace_its_properties_were_written_in() {
    let base = generated("table");
    let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    let xml = part(&base, DOCUMENT)
        .replacen(
            r#"<w:tr><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/><w:vMerge w:val="restart"/>"#,
            &format!(
                r#"<w:tr xmlns:q="{w}"><w:trPr><q:cantSplit/></w:trPr><w:tc xmlns:c="{w}"><w:tcPr><c:tcW w:w="3000" w:type="dxa"/><w:vMerge w:val="restart"/>"#
            ),
            1,
        );
    assert!(xml.contains("<q:cantSplit/>"));
    let bytes = rewrite(&base, DOCUMENT, Some(xml.as_bytes()));
    for column in [false, true] {
        let mut doc = EditingDocument::open(&bytes).unwrap();
        let (_, ada, table) = cell_of(&doc, "Ada");
        let command = match column {
            false => Command::InsertRow {
                row: ada,
                below: true,
                like: None,
            },
            true => Command::InsertColumn {
                table,
                index: 0,
                like: 0,
            },
        };
        edit(&mut doc, command);
        let out = doc.export().unwrap();
        assert_valid("scoped table properties", &out);
        let xml = part(&out, DOCUMENT);
        let rows = if column { 1 } else { 2 };
        assert_eq!(xml.matches("<q:cantSplit/>").count(), rows, "{xml}");
        reopen("scoped table properties", &doc);
        assert_editor_undo("scoped table properties", &bytes, &mut doc);
    }
}

#[test]
fn table_copies_preserve_source_names_and_widths_under_rebound_prefixes() {
    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    for (row_name, cell_name, prop, default) in [
        ("w:tr", "w:tc", "w", ""),
        ("q:tr", "q:tc", "q", ""),
        ("q:tr", "tc", "q", W),
    ] {
        let word_binding = format!(r#"xmlns:q="{W}""#);
        let cell = |text: &str, bindings: &str| {
            let kept = if default.is_empty() {
                "<keep/>"
            } else {
                r#"<keep xmlns=""/>"#
            };
            format!(
                concat!(
                    r#"<{cell_name} xmlns="{default}" {bindings}><{prop}:tcPr>"#,
                    r#"<{prop}:tcW {prop}:type="dxa" {prop}:w="&#50;000"/>{kept}</{prop}:tcPr>"#,
                    r#"<w:p xmlns:para="{W}"><para:pPr><para:jc para:val="center"/></para:pPr>"#,
                    r#"<w:r><w:t>{text}</w:t></w:r></w:p></{cell_name}>"#
                ),
                cell_name = cell_name,
                default = default,
                bindings = bindings,
                prop = prop,
                kept = kept,
                W = W,
                text = text
            )
        };
        // The table shadows the story's q. The first cell inherits Word's q
        // from its row, and the second declares it for itself.
        let source = cell("Source", "");
        let other = cell("Other", &word_binding);
        let body = format!(
            r#"<w:tbl xmlns:q="urn:extension" xmlns="urn:extension"><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><{row_name} {word_binding}>{source}</{row_name}><w:tr>{other}</w:tr></w:tbl>"#
        );
        let base = with_body(&body);
        let xml = part(&base, DOCUMENT).replace("<w:body>", &format!(r#"<w:body xmlns:q="{W}">"#));
        let bytes = rewrite(&base, DOCUMENT, Some(xml.as_bytes()));
        for column in [false, true] {
            let mut doc = EditingDocument::open(&bytes).unwrap();
            let (_, source, table) = cell_of(&doc, "Source");
            let (_, other, _) = cell_of(&doc, "Other");
            let cmd = if column {
                Command::InsertColumn {
                    table,
                    index: 0,
                    like: 0,
                }
            } else {
                Command::InsertRow {
                    row: other,
                    below: true,
                    like: Some(source),
                }
            };
            edit(&mut doc, cmd);
            let out = doc.export().unwrap();
            let reopened = Document::open(&out).unwrap();
            for view in [&*doc, &reopened] {
                let (_, _, table) = cell_of(view, "Source");
                let rows = &view.node(table).unwrap().children;
                let expected = if column {
                    vec![(1, None, Some(1000)); 2]
                } else {
                    vec![(1, None, Some(2000))]
                };
                assert_eq!(rows.len(), if column { 2 } else { 3 });
                for row in rows {
                    assert_eq!(cells(view, *row), expected);
                    for cell in &view.node(*row).unwrap().children {
                        let paragraph = view.node(*cell).unwrap().children[0];
                        assert_eq!(view.effective_paragraph(paragraph).alignment, "center");
                    }
                }
            }
            let xml = part(&out, DOCUMENT);
            let parsed = roxmltree::Document::parse(&xml).unwrap();
            assert_eq!(
                parsed
                    .descendants()
                    .filter(|e| e.is_element()
                        && e.tag_name().name() == "keep"
                        && e.tag_name().namespace().unwrap_or("").is_empty())
                    .count(),
                if column { 4 } else { 3 }
            );
            assert_editor_undo("scoped table copy", &bytes, &mut doc);
        }
    }
}

#[test]
fn column_edits_recognize_revision_names_in_their_xml_namespace() {
    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    for (in_grid, local) in [(true, "tblGridChange"), (false, "tcPrChange")] {
        let change = format!(
            r#"<q:{local} w:id="5" {TRACKED}><w:{}/></q:{local}>"#,
            if in_grid { "tblGrid" } else { "tcPr" }
        );
        for (namespace, refused) in [(W, true), ("urn:extension", false)] {
            let td = if in_grid {
                cell("A")
            } else {
                cell("A").replace("</w:tcPr>", &format!("{change}</w:tcPr>"))
            };
            let rows = format!("<w:tr>{td}{}</w:tr>", cell("B"));
            let xml = table(if in_grid { &change } else { "" }, &rows)
                .replace("<w:tbl>", &format!(r#"<w:tbl xmlns:q="{namespace}">"#));
            let mut doc = EditingDocument::open(&with_body(&xml)).unwrap();
            let (_, _, table) = cell_of(&doc, "A");
            let result = doc.run(Command::InsertColumn {
                table,
                index: 1,
                like: 0,
            });
            if refused {
                assert!(result.unwrap_err().to_string().contains("tracked change"));
            } else {
                result.unwrap();
            }
        }
    }
}

#[test]
fn an_edit_keeps_the_markup_beside_table_properties_that_follow_a_range_marker() {
    let bytes = with_body(
        r#"<w:tbl><w:bookmarkStart w:id="3" w:name="mark"/><!--before--><w:tblPr/><?keep between?><w:tblGrid><w:gridCol w:w="1200"/></w:tblGrid><!--after--><w:tr><w:tc><w:p><w:r><w:t>ABCDE</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:bookmarkEnd w:id="3"/></w:p>"#,
    );
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let text = text_node(&doc, "ABCDE");
    edit(
        &mut doc,
        Command::InsertText {
            at: at(text, 2),
            text: "x".into(),
        },
    );
    let xml = part(&doc.export().unwrap(), DOCUMENT);
    // The properties move before the marker with the markup that led to each.
    let head = r#"<w:tbl><!--before--><w:tblPr/><?keep between?><w:tblGrid><w:gridCol w:w="1200"/></w:tblGrid><w:bookmarkStart w:id="3" w:name="mark"/><!--after--><w:tr>"#;
    assert!(xml.contains(head), "{xml}");
    reopen("late table properties", &doc);
    assert_editor_undo("late table properties", &bytes, &mut doc);
}

#[test]
fn cell_properties_are_found_as_the_xml_parser_reads_them() {
    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    // WordprocessingML's namespace, written with a character reference.
    let escaped = r#"<q:tcPr xmlns:q="http://schemas.openxmlformats.org/wordprocessingml/2006&#x2F;main"><q:tcW q:w="2000" q:type="dxa"/><e:kept xmlns:e="urn:extension"/></q:tcPr>"#;
    // An extension whose text holds what looks like the start of a tag.
    let cdata = r#"<w:tcPr><w:tcW w:w="2000" w:type="dxa"/><e:ext xmlns:e="urn:extension"><![CDATA[1 < 2]]></e:ext></w:tcPr>"#;
    for (name, properties) in [("CDATA", cdata), ("escaped", escaped)] {
        let extension =
            &properties[properties.find("<e:").unwrap()..properties.rfind("</").unwrap()];
        let first = cell("A").replace(
            r#"<w:tcPr><w:tcW w:w="2000" w:type="dxa"/></w:tcPr>"#,
            properties,
        );
        assert!(first.contains(extension));
        let rows = format!("<w:tr>{first}{}</w:tr>", cell("B"));
        let bytes = with_body(&table("", &rows));
        let mut doc = EditingDocument::open(&bytes).unwrap();
        let (_, row, table) = cell_of(&doc, "A");
        // The first cell takes the width of the column removed after it.
        edit(&mut doc, Command::RemoveColumn { table, index: 1 });
        let out = doc.export().unwrap();
        let reopened = Document::open(&out).unwrap();
        assert_eq!(cells(&doc, row), [(1, None, Some(4000))], "{name}: live");
        let (_, row, _) = cell_of(&reopened, "A");
        assert_eq!(
            cells(&reopened, row),
            [(1, None, Some(4000))],
            "{name}: reopened"
        );
        let xml = part(&out, DOCUMENT);
        let tree = roxmltree::Document::parse(&xml).unwrap();
        let blocks: Vec<usize> = tree
            .descendants()
            .filter(|n| n.has_tag_name((W, "tc")))
            .map(|n| n.children().filter(|c| c.has_tag_name((W, "tcPr"))).count())
            .collect();
        assert_eq!(blocks, [1], "{name}: {xml}");
        let widths = tree
            .descendants()
            .filter(|n| n.has_tag_name((W, "tcW")))
            .count();
        assert_eq!(widths, 1, "{name}: {xml}");
        assert_eq!(xml.matches(extension).count(), 1, "{name}: {xml}");
        assert_editor_undo(name, &bytes, &mut doc);
    }
}
