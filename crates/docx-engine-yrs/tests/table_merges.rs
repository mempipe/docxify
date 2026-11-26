//! Vertical merges in the exported file, checked without the table reader.
//!
//! The test knows what each continuation was made to continue: a source cell
//! from the fixture, a created cell from the merge crossing its insertion point in the
//! author's grid before the edit. It reads the exported XML itself and
//! applies the rule of spec R6: a continuation is
//! part of its merge only when the cell directly above is that merge's start,
//! or a continuation that is part of it, and both hold the same columns.

#[path = "../../docx-engine/tests/common/mod.rs"]
mod common;
use common::*;
use docx_engine::editing::Editor;
use docx_engine::editing::Resolution;
use docx_engine::{Command, Document, NodeId, NodeKind};
use docx_engine_yrs::Replica;
use std::collections::{BTreeMap, BTreeSet};

/// A source cell: its label, its source columns and its vertical merge.
type SourceCell = (&'static str, &'static [usize], Option<&'static str>);
/// Two merges of equal width stand side by side (A and P) and two are
/// stacked in one column (A and B), so a cell that joins the wrong merge has
/// a neighbour of the right width to join.
const ROWS: [&[SourceCell]; 6] = [
    &[
        ("A", &[0], Some("restart")),
        ("P", &[1], Some("restart")),
        ("x0", &[2], None),
    ],
    &[
        ("a1", &[0], Some("continue")),
        ("p1", &[1], Some("continue")),
        ("x1", &[2], None),
    ],
    &[
        ("a2", &[0], Some("continue")),
        ("t2", &[1], None),
        ("x2", &[2], None),
    ],
    &[
        ("B", &[0], Some("restart")),
        ("y3", &[1], None),
        ("z3", &[2], None),
    ],
    &[
        ("b4", &[0], Some("continue")),
        ("y4", &[1], None),
        ("z4", &[2], None),
    ],
    &[("W", &[0, 1], None), ("z5", &[2], None)],
];
const STARTS: [&str; 3] = ["A", "P", "B"];
/// The fixtures: no content control, one around the middle cell of merge A,
/// and one around a cell that no merge runs through.
const FIXTURES: [Option<&str>; 3] = [None, Some("a1"), Some("t2")];
fn intended(label: &str) -> Option<&'static str> {
    match label {
        "a1" | "a2" => Some("A"),
        "p1" => Some("P"),
        "b4" => Some("B"),
        _ => None,
    }
}
fn fixture(wrapped: Option<&str>) -> Vec<u8> {
    let mut rows = String::new();
    for row in ROWS {
        rows.push_str("<w:tr>");
        for (label, columns, merge) in row {
            let merge = match merge {
                Some("restart") => r#"<w:vMerge w:val="restart"/>"#,
                Some(_) => "<w:vMerge/>",
                None => "",
            };
            let span = columns.len();
            let cell = format!(
                r#"<w:tc><w:tcPr><w:tcW w:w="{}" w:type="dxa"/><w:gridSpan w:val="{span}"/>{merge}</w:tcPr><w:p><w:r><w:t>{label}</w:t></w:r></w:p></w:tc>"#,
                2000 * span
            );
            if wrapped == Some(*label) {
                rows.push_str(&format!(
                    r#"<w:sdt><w:sdtPr><w:tag w:val="held"/></w:sdtPr><w:sdtContent>{cell}</w:sdtContent></w:sdt>"#
                ));
            } else {
                rows.push_str(&cell);
            }
        }
        rows.push_str("</w:tr>");
    }
    with_body(&format!(
        "<w:tbl><w:tblGrid>{}</w:tblGrid>{rows}</w:tbl><w:p/>",
        r#"<w:gridCol w:w="2000"/>"#.repeat(3)
    ))
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Cell {
    label: String,
    start: usize,
    span: usize,
    merge: Option<String>,
    wrapped: bool,
}
/// The table as the file states it: the grid's column count and each row's
/// cells. Nothing here uses the engine's reading of a table.
fn read(bytes: &[u8]) -> (usize, Vec<Vec<Cell>>) {
    let xml = part(bytes, "word/document.xml");
    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    let doc = roxmltree::Document::parse(&xml).unwrap();
    let table = doc
        .descendants()
        .find(|n| n.has_tag_name((W, "tbl")))
        .unwrap();
    let grid = table
        .children()
        .find(|n| n.has_tag_name((W, "tblGrid")))
        .unwrap();
    let columns = grid
        .children()
        .filter(|n| n.has_tag_name((W, "gridCol")))
        .count();
    let mut rows = Vec::new();
    for row in table.children().filter(|n| n.has_tag_name((W, "tr"))) {
        let mut start = row
            .children()
            .find(|n| n.has_tag_name((W, "trPr")))
            .and_then(|p| p.children().find(|n| n.has_tag_name((W, "gridBefore"))))
            .and_then(|n| n.attribute((W, "val")))
            .map(|v| v.parse().unwrap())
            .unwrap_or(0);
        let mut cells = Vec::new();
        for cell in row.descendants().filter(|n| {
            n.has_tag_name((W, "tc"))
                && n.ancestors().find(|a| a.has_tag_name((W, "tr"))) == Some(row)
        }) {
            let props = cell.children().find(|n| n.has_tag_name((W, "tcPr")));
            let property =
                |name| props.and_then(|p| p.children().find(|n| n.has_tag_name((W, name))));
            let span = property("gridSpan")
                .and_then(|n| n.attribute((W, "val")))
                .map(|v| v.parse().unwrap())
                .unwrap_or(1);
            let merge = property("vMerge")
                .map(|n| n.attribute((W, "val")).unwrap_or("continue").to_owned());
            let wrapped = cell
                .ancestors()
                .take_while(|a| *a != row)
                .any(|a| a.has_tag_name((W, "sdtContent")));
            let label = cell
                .descendants()
                .filter(|n| n.has_tag_name((W, "t")))
                .filter_map(|n| n.text())
                .collect();
            cells.push(Cell {
                label,
                start,
                span,
                merge,
                wrapped,
            });
            start += span;
        }
        rows.push(cells);
    }
    (columns, rows)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Insert { at: usize, like: usize },
    Row(usize),
    Column(usize),
    Wrapper,
}
struct Nodes {
    table: NodeId,
    rows: Vec<NodeId>,
    wrapper: Option<NodeId>,
}
fn nodes(doc: &Document, wrapped: Option<&str>) -> Nodes {
    let table = doc
        .stories()
        .flat_map(|s| doc.descendants(s.id))
        .find(|id| matches!(doc.node(*id).unwrap().kind, NodeKind::Table(_)))
        .unwrap();
    let rows = doc.node(table).unwrap().children.clone();
    let wrapper = wrapped.map(|held| {
        let row = (ROWS.iter()).position(|cells| cells.iter().any(|c| c.0 == held));
        let row = row.unwrap();
        let cell = ROWS[row].iter().position(|c| c.0 == held).unwrap();
        doc.node(rows[row]).unwrap().children[cell]
    });
    Nodes {
        table,
        rows,
        wrapper,
    }
}
/// Every action of the enumeration. Rows are inserted at every place and
/// copied from a row of the first merge, a row of the second and the wide
/// row; `brief` leaves out the places and the copy that add no new case.
fn actions(wrapped: Option<&str>, brief: bool) -> Vec<Action> {
    let mut all = Vec::new();
    for at in 0..=ROWS.len() {
        for like in [0, 2, 3, 5] {
            // The plain continuation row matters where it holds the wrapper.
            let plain = like == 2 && wrapped != Some("t2");
            if brief && (at == 0 || at == 5 || plain) {
                continue;
            }
            all.push(Action::Insert { at, like });
        }
    }
    all.extend((0..ROWS.len()).map(Action::Row));
    all.extend((0..3).map(Action::Column));
    if wrapped.is_some() {
        all.push(Action::Wrapper);
    }
    all
}

/// One editing session, shared or not.
trait Session {
    fn run(&mut self, command: &Command) -> bool;
    fn back(&mut self) -> bool;
    fn file(&mut self) -> Vec<u8>;
    fn state(&mut self) -> Vec<u8>;
    fn columns(&mut self, nodes: &Nodes) -> usize;
}
impl Session for Replica {
    fn run(&mut self, command: &Command) -> bool {
        self.apply(command).is_ok()
    }
    fn back(&mut self) -> bool {
        self.undo().unwrap()
    }
    fn file(&mut self) -> Vec<u8> {
        self.export().unwrap()
    }
    fn state(&mut self) -> Vec<u8> {
        self.save()
    }

    fn columns(&mut self, nodes: &Nodes) -> usize {
        grid(self.view().unwrap(), nodes.table)
    }
}
impl Session for Editor {
    fn run(&mut self, command: &Command) -> bool {
        self.apply(command).is_ok()
    }
    fn back(&mut self) -> bool {
        self.undo()
    }
    fn file(&mut self) -> Vec<u8> {
        self.export().unwrap()
    }
    fn state(&mut self) -> Vec<u8> {
        self.export().unwrap()
    }

    fn columns(&mut self, nodes: &Nodes) -> usize {
        grid(self.view().unwrap(), nodes.table)
    }
}
fn grid(doc: &Document, table: NodeId) -> usize {
    match &doc.node(table).unwrap().kind {
        NodeKind::Table(info) => info.grid.len(),
        _ => unreachable!(),
    }
}

/// What the test knows of a history, kept apart from the engine.
#[derive(Clone, Default)]
struct Known {
    wrapped: Option<&'static str>,
    /// Each action that took effect, and whether it is still in effect.
    done: Vec<(Action, bool)>,
    /// The created row: its template, and for each template cell (named by
    /// its first source column) the start its author saw it continue.
    created: Option<(usize, BTreeMap<usize, String>)>,
}
impl Known {
    fn active(&self) -> impl Iterator<Item = Action> + '_ {
        self.done.iter().filter(|(_, on)| *on).map(|(a, _)| *a)
    }
    fn dropped(&self) -> BTreeSet<usize> {
        self.active()
            .filter_map(|a| match a {
                Action::Column(c) => Some(c),
                _ => None,
            })
            .collect()
    }
}
#[derive(Default)]
struct Seen {
    states: usize,
    refused: usize,
    held: usize,
    cut: usize,
    /// The same shown rows, columns and stated merges give the same file.
    same: BTreeMap<String, Vec<Vec<Cell>>>,
}

/// Applies one action. A refusal must leave the session as it was.
fn act(session: &mut impl Session, nodes: &Nodes, action: Action, known: &mut Known) -> bool {
    let command = match action {
        Action::Insert { at, like } => Command::InsertRow {
            row: nodes.rows[at.min(ROWS.len() - 1)],
            below: at == ROWS.len(),
            like: Some(nodes.rows[like]),
        },
        Action::Row(row) => Command::RemoveNode {
            node: nodes.rows[row],
        },
        Action::Column(column) => {
            // The command counts shown columns; the action names a source one.
            let before = known.dropped();
            if before.contains(&column) || session.columns(nodes) != 3 - before.len() {
                return false;
            }
            Command::RemoveColumn {
                table: nodes.table,
                index: column - before.iter().filter(|c| **c < column).count(),
            }
        }
        Action::Wrapper => Command::RemoveNode {
            node: nodes.wrapper.unwrap(),
        },
    };
    let input = matches!(action, Action::Insert { .. }).then(|| read(&session.file()));
    let before = session.state();
    if !session.run(&command) {
        assert!(session.state() == before, "a refused {action:?} wrote");
        return false;
    }
    known.done.push((action, true));
    if let (Action::Insert { like, at }, Some((count, rows))) = (action, input) {
        let columns = shown_columns(count, known);
        let mut stated = BTreeMap::new();
        if let Some(target) = ROWS.get(at) {
            let below = rows
                .iter()
                .position(|row| {
                    row.iter()
                        .any(|cell| target.iter().any(|(label, ..)| *label == cell.label))
                })
                .expect("the author inserted before a visible source row");
            for cell in rows[below]
                .iter()
                .filter(|c| c.merge.as_deref() == Some("continue"))
            {
                if let Some(start) = owner(&rows, below, cell) {
                    let own = &columns[cell.start..cell.start + cell.span];
                    stated.insert(template_cell(like, own), start);
                }
            }
        }
        known.created = Some((like, stated));
    }
    true
}
/// The source columns a grid of `count` columns shows.
fn shown_columns(count: usize, known: &Known) -> Vec<usize> {
    let dropped = known.dropped();
    let kept: Vec<usize> = (0..3).filter(|c| !dropped.contains(c)).collect();
    // A table held on its source grid shows every column.
    if count == 3 {
        (0..3).collect()
    } else {
        assert_eq!(count, kept.len(), "the grid matches no known column set");
        kept
    }
}
fn template_cell(like: usize, columns: &[usize]) -> usize {
    let cell = ROWS[like]
        .iter()
        .find(|(_, held, _)| columns.iter().all(|c| held.contains(c)));
    cell.expect("a copy of one template cell").1[0]
}
/// The start a continuation reaches through the merged cells right above it.
fn owner(rows: &[Vec<Cell>], at: usize, cell: &Cell) -> Option<String> {
    let above = rows[at.checked_sub(1)?]
        .iter()
        .find(|c| c.start == cell.start && c.span == cell.span)?;
    match above.merge.as_deref() {
        Some("restart") => Some(above.label.clone()),
        Some(_) => owner(rows, at - 1, above),
        None => None,
    }
}

fn check(
    file: &[u8],
    reports: Option<Vec<Resolution>>,
    known: &Known,
    seen: &mut Seen,
    case: &str,
) {
    let (count, rows) = read(file);
    let columns = shown_columns(count, known);
    if count == 3 && !known.dropped().is_empty() {
        seen.held += 1;
        if let Some(reports) = &reports {
            assert!(
                reports.contains(&Resolution::SourceGrid),
                "{case}: columns are back with no report"
            );
        }
    }
    let deleted: BTreeSet<usize> = known
        .active()
        .filter_map(|a| match a {
            Action::Row(r) => Some(r),
            _ => None,
        })
        .collect();
    let mut found = BTreeSet::new();
    let mut labels = BTreeSet::new();
    let mut key = format!("{:?} {columns:?}", known.wrapped);
    // The cells above that are part of a merge: their columns and its start.
    let mut above: Vec<(Vec<usize>, String)> = Vec::new();
    for (at, row) in rows.iter().enumerate() {
        let width: usize = row.iter().map(|c| c.span).sum();
        assert_eq!(width, count, "{case}: row {at} does not fill the grid");
        let source = ROWS.iter().position(|cells| {
            (row.iter()).any(|c| cells.iter().any(|(label, ..)| *label == c.label))
        });
        let mut here = Vec::new();
        match source {
            Some(index) => {
                assert!(found.insert(index), "{case}: row {index} shows twice");
                key.push_str(&format!(" r{index}"));
                // Every cell of the row with a shown column is there, once,
                // in its own columns. Only the wrapper's cell may be vacated.
                let mut cells = row.iter();
                for (label, held, _) in ROWS[index] {
                    let held: Vec<usize> = held
                        .iter()
                        .copied()
                        .filter(|c| columns.contains(c))
                        .collect();
                    if held.is_empty() {
                        continue;
                    }
                    let cell = cells.next().expect("a cell for every shown column");
                    let own = &columns[cell.start..cell.start + cell.span];
                    assert_eq!(own, held, "{case}: {label} is in the wrong columns");
                    let vacated = known.wrapped == Some(*label) && cell.label.is_empty();
                    if vacated {
                        key.push_str(" vacated");
                    }
                    assert!(
                        vacated || cell.label == *label,
                        "{case}: {label} shows {:?}",
                        cell.label
                    );
                    assert!(
                        vacated || labels.insert(*label),
                        "{case}: {label} shows twice"
                    );
                }
                assert!(
                    cells.next().is_none(),
                    "{case}: row {index} has a cell too many"
                );
            }
            None => {
                let (like, stated) = known
                    .created
                    .as_ref()
                    .expect("an unlabelled row was created");
                key.push_str(&format!(" c{like}{stated:?}"));
                assert!(
                    row.iter().all(|c| c.label.is_empty()),
                    "{case}: a mixed row"
                );
            }
        }
        for cell in row {
            let own = columns[cell.start..cell.start + cell.span].to_vec();
            let stated = match source {
                Some(_) if cell.label.is_empty() => None,
                Some(_) => intended(&cell.label).map(str::to_string),
                None => {
                    let (like, stated) = known.created.as_ref().unwrap();
                    stated.get(&template_cell(*like, &own)).cloned()
                }
            };
            if STARTS.contains(&cell.label.as_str()) {
                assert_eq!(
                    cell.merge.as_deref(),
                    Some("restart"),
                    "{case}: {}",
                    cell.label
                );
                here.push((own, cell.label.clone()));
                continue;
            }
            let Some(start) = stated else {
                assert_eq!(cell.merge, None, "{case}: a cell with no merge is merged");
                continue;
            };
            let joined = above.contains(&(own.clone(), start.clone()));
            if cell.wrapped && !joined {
                // A wrapper's bytes cannot change, so its cell still says
                // it continues.
                seen.cut += 1;
                continue;
            }
            assert_eq!(
                cell.merge.is_some(),
                joined,
                "{case}: {:?} in row {at} over {own:?}, made to continue {start}",
                cell.label
            );
            if joined {
                assert_eq!(
                    owner(&rows, at, cell).as_deref(),
                    Some(start.as_str()),
                    "{case}"
                );
                here.push((own, start));
            }
        }
        above = here;
    }
    for index in 0..ROWS.len() {
        if !deleted.contains(&index) {
            assert!(found.contains(&index), "{case}: row {index} is missing");
        } else if found.contains(&index) {
            if let Some(reports) = &reports {
                assert!(
                    !reports.is_empty(),
                    "{case}: a deleted row shows with no report"
                );
            }
        }
    }
    if let Some(earlier) = seen.same.get(&key) {
        assert!(
            *earlier == rows,
            "{case}: the same table and merges, another file\n{key}"
        );
    } else {
        seen.same.insert(key, rows);
    }
    seen.states += 1;
}
fn insert(action: &Action) -> bool {
    matches!(action, Action::Insert { .. })
}
/// Undoes the session's latest action that the test still counts as active.
fn undo(session: &mut impl Session, known: &mut Known, action: Action) {
    assert!(session.back());
    let entry = (known.done.iter_mut().rev()).find(|(a, on)| *a == action && *on);
    entry.expect("an active action").1 = false;
    if insert(&action) {
        known.created = None;
    }
}

#[test]
fn one_author_merges_follow_what_each_cell_was_made_to_continue() {
    let mut seen = Seen::default();
    for wrapped in FIXTURES {
        let bytes = fixture(wrapped);
        let all = actions(wrapped, false);
        let nodes = nodes(Editor::open(&bytes).unwrap().view().unwrap(), wrapped);
        for &first in &all {
            for &second in &all {
                if insert(&first) && insert(&second) {
                    continue;
                }
                let mut editor = Editor::open(&bytes).unwrap();
                let mut known = Known {
                    wrapped,
                    ..Known::default()
                };
                let case = format!("{wrapped:?} {first:?} {second:?}");
                let mut taken = Vec::new();
                for action in [first, second] {
                    if act(&mut editor, &nodes, action, &mut known) {
                        taken.push(action);
                        check(&editor.file(), None, &known, &mut seen, &case);
                    } else {
                        seen.refused += 1;
                    }
                }
                // A new session on the written file reads the same table.
                let file = editor.file();
                let reopened = Editor::open(&file).unwrap().file();
                assert!(
                    read(&file) == read(&reopened),
                    "{case}: reopened differently"
                );
                for action in taken.into_iter().rev() {
                    undo(&mut editor, &mut known, action);
                    let case = format!("{case} undone");
                    check(&editor.file(), None, &known, &mut seen, &case);
                }
                assert!(
                    read(&editor.file()) == read(&bytes),
                    "{case}: undo left a change"
                );
            }
        }
    }
    assert_eq!(
        (
            seen.states,
            seen.refused,
            seen.held,
            seen.cut,
            seen.same.len()
        ),
        (4916, 1352, 0, 0, 516)
    );
    assert_eq!(seen.cut, 0);
}

fn sync(a: &mut Replica, b: &mut Replica) {
    let to_b = a.update_since(&b.state_vector()).unwrap();
    let to_a = b.update_since(&a.state_vector()).unwrap();
    a.apply_update(&to_a).unwrap();
    b.apply_update(&to_b).unwrap();
}
fn agree(a: &mut Replica, b: &mut Replica, known: &Known, seen: &mut Seen, case: &str) {
    sync(a, b);
    let file = a.export().unwrap();
    assert!(
        file == b.export().unwrap(),
        "{case}: the peers export different files"
    );
    let reports = a.resolutions().unwrap().to_vec();
    check(&file, Some(reports), known, seen, case);
}

#[test]
fn two_authors_merges_follow_what_each_cell_was_made_to_continue() {
    two_authors(FIXTURES[0]);
}
#[test]
fn two_authors_merges_follow_what_each_cell_was_made_to_continue_past_a_wrapper() {
    two_authors(FIXTURES[1]);
}
#[test]
fn two_authors_merges_follow_what_each_cell_was_made_to_continue_beside_a_wrapper() {
    two_authors(FIXTURES[2]);
}
fn two_authors(wrapped: Option<&'static str>) {
    let mut seen = Seen::default();
    {
        let bytes = fixture(wrapped);
        let all = actions(wrapped, true);
        let mut seed = Replica::seed(7, &bytes).unwrap();
        let nodes = nodes(seed.view().unwrap(), wrapped);
        let saved = seed.save();
        for (one, two) in [(1, 2), (2, 1)] {
            for &first in &all {
                for &second in &all {
                    if insert(&first) && insert(&second) {
                        continue;
                    }
                    // The second author either saw the first action or not. Once
                    // told, the actions are ordered by cause, so which author
                    // has the higher identity decides nothing.
                    for told in [false, true] {
                        if told && one > two {
                            continue;
                        }
                        let mut a = Replica::join(one, &bytes, &saved).unwrap();
                        let mut b = Replica::join(two, &bytes, &saved).unwrap();
                        let mut known = Known {
                            wrapped,
                            ..Known::default()
                        };
                        let case = format!("{wrapped:?} {one} {first:?} {second:?} told {told}");
                        if !act(&mut a, &nodes, first, &mut known) {
                            seen.refused += 1;
                            continue;
                        }
                        if told {
                            sync(&mut a, &mut b);
                        }
                        // An untold author acts on the table as it began.
                        let mut theirs = if told {
                            known.clone()
                        } else {
                            Known {
                                wrapped,
                                ..Known::default()
                            }
                        };
                        if !act(&mut b, &nodes, second, &mut theirs) {
                            seen.refused += 1;
                            continue;
                        }
                        known.done.push((second, true));
                        known.created = theirs.created.or(known.created);
                        agree(&mut a, &mut b, &known, &mut seen, &case);
                        undo(&mut a, &mut known, first);
                        agree(
                            &mut a,
                            &mut b,
                            &known,
                            &mut seen,
                            &format!("{case}, first undone"),
                        );
                        // With the first action undone, another column goes.
                        if insert(&second) && matches!(first, Action::Column(_)) {
                            for column in 0..3 {
                                let mut after = known.clone();
                                if act(&mut b, &nodes, Action::Column(column), &mut after) {
                                    let case = format!("{case}, then column {column}");
                                    agree(&mut a, &mut b, &after, &mut seen, &case);
                                    undo(&mut b, &mut after, Action::Column(column));
                                }
                            }
                        }
                        undo(&mut b, &mut known, second);
                        agree(
                            &mut a,
                            &mut b,
                            &known,
                            &mut seen,
                            &format!("{case}, both undone"),
                        );
                        let mut fresh = Replica::join(9, &bytes, &a.save()).unwrap();
                        assert!(fresh.export().unwrap() == a.export().unwrap(), "{case}");
                    }
                }
            }
        }
    }
    let expected = match wrapped {
        None => (2268, 411, 0, 0, 183),
        Some("a1") => (981, 873, 3, 0, 77),
        Some("t2") => (1293, 1069, 6, 0, 104),
        _ => unreachable!(),
    };
    assert_eq!(
        (
            seen.states,
            seen.refused,
            seen.held,
            seen.cut,
            seen.same.len()
        ),
        expected
    );
}

fn merges(bytes: &[u8]) -> Vec<Vec<(String, Option<String>)>> {
    let (_, rows) = read(bytes);
    rows.iter()
        .map(|row| {
            row.iter()
                .map(|c| (c.label.clone(), c.merge.clone()))
                .collect()
        })
        .collect()
}

// A row that does not line up lands inside the merge A, a1, a2 while the
// merge is hidden. The cells below it are ordinary until it goes.
#[test]
fn a_row_that_cuts_a_merge_leaves_the_cells_below_ordinary_until_it_goes() {
    let bytes = fixture(None);
    let mut alice = Replica::seed(1, &bytes).unwrap();
    let mut bob = Replica::join(2, &bytes, &alice.save()).unwrap();
    let nodes = nodes(alice.view().unwrap(), None);
    let wide = Command::InsertRow {
        row: nodes.rows[0],
        below: true,
        like: Some(nodes.rows[5]),
    };
    // With the merge in view the copy has no cell of its width, so it is refused.
    let before = bob.save();
    let refusal = bob.apply(&wide).unwrap_err().to_string();
    assert!(refusal.contains("merged cell runs across"), "{refusal}");
    assert!(bob.save() == before);
    alice
        .apply(&Command::RemoveColumn {
            table: nodes.table,
            index: 0,
        })
        .unwrap();
    sync(&mut alice, &mut bob);
    bob.apply(&wide).unwrap();
    sync(&mut alice, &mut bob);
    assert!(alice.undo().unwrap());
    sync(&mut alice, &mut bob);
    let cut = merges(&alice.export().unwrap());
    assert_eq!(cut[0][0], ("A".into(), Some("restart".into())));
    assert_eq!(cut[1][0], (String::new(), None));
    assert_eq!(cut[2][0], ("a1".into(), None));
    assert_eq!(cut[3][0], ("a2".into(), None));
    // The second merge of that width, further down, is untouched.
    assert_eq!(cut[5][0], ("b4".into(), Some("continue".into())));
    assert!(alice.resolutions().unwrap().is_empty());
    // Nothing depends on the start now, so its row can go.
    alice
        .apply(&Command::RemoveNode {
            node: nodes.rows[0],
        })
        .unwrap();
    assert!(alice.undo().unwrap());
    assert!(bob.undo().unwrap());
    sync(&mut alice, &mut bob);
    assert!(merges(&alice.export().unwrap()) == merges(&bytes));
}

#[test]
fn a_wrapper_inside_a_merge_stays_while_a_cell_below_continues_through_it() {
    let bytes = fixture(FIXTURES[1]);
    let mut alice = Replica::seed(1, &bytes).unwrap();
    let nodes = nodes(alice.view().unwrap(), FIXTURES[1]);
    let remove = Command::RemoveNode {
        node: nodes.wrapper.unwrap(),
    };
    let before = alice.save();
    let refusal = alice.apply(&remove).unwrap_err().to_string();
    assert!(refusal.contains("runs through this element"), "{refusal}");
    assert!(alice.save() == before);
    // With the row below gone nothing continues through it, and it can go.
    alice
        .apply(&Command::RemoveNode {
            node: nodes.rows[2],
        })
        .unwrap();
    alice.apply(&remove).unwrap();
    let rows = merges(&alice.export().unwrap());
    assert_eq!(rows[1][0], (String::new(), None));
}

#[test]
fn a_copy_of_a_row_outside_the_declared_grid_continues_the_merge_it_crosses() {
    let cell = |merge: &str| format!("<w:tc><w:tcPr>{merge}</w:tcPr><w:p/></w:tc>");
    for (grid, lead) in [
        ("", String::new()),
        (r#"<w:gridCol w:w="3000"/>"#, cell("")),
    ] {
        let bytes = with_body(&format!(
            "<w:tbl><w:tblGrid>{grid}</w:tblGrid><w:tr>{lead}{}</w:tr><w:tr>{lead}{}</w:tr></w:tbl><w:p/>",
            cell(r#"<w:vMerge w:val="restart"/>"#),
            cell("<w:vMerge/>")
        ));
        let insert = |d: &Document| Command::InsertRow {
            row: nodes(d, None).rows[0],
            below: true,
            like: None,
        };
        let mut one = Editor::open(&bytes).unwrap();
        let command = insert(one.view().unwrap());
        one.apply(&command).unwrap();
        let mut replica = Replica::seed(1, &bytes).unwrap();
        let command = insert(replica.view().unwrap());
        replica.apply(&command).unwrap();
        let written = merges(&replica.export().unwrap());
        let last = |row: &Vec<(String, Option<String>)>| row.last().unwrap().1.clone();
        assert_eq!(
            written.iter().map(last).collect::<Vec<_>>(),
            [
                Some("restart".to_string()),
                Some("continue".into()),
                Some("continue".into())
            ],
            "grid {grid:?}"
        );
        assert!(merges(&one.export().unwrap()) == written, "grid {grid:?}");
    }
}

#[test]
fn a_row_copied_from_a_vacated_row_has_an_ordinary_cell_there() {
    let cell = |text: &str| format!("<w:tc><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>");
    let bytes = with_body(&format!(
        r#"<w:tbl><w:tblGrid>{}</w:tblGrid><w:tr><w:sdt><w:sdtPr><w:tag w:val="held"/></w:sdtPr><w:sdtContent>{}</w:sdtContent></w:sdt>{}</w:tr></w:tbl><w:p/>"#,
        r#"<w:gridCol w:w="2000"/>"#.repeat(2),
        cell("w0"),
        cell("w1")
    ));
    let mut alice = Replica::seed(1, &bytes).unwrap();
    let mut bob = Replica::join(2, &bytes, &alice.save()).unwrap();
    let nodes = nodes(alice.view().unwrap(), None);
    let wrapper = alice.view().unwrap().node(nodes.rows[0]).unwrap().children[0];
    alice.apply(&Command::RemoveNode { node: wrapper }).unwrap();
    alice
        .apply(&Command::InsertRow {
            row: nodes.rows[0],
            below: true,
            like: None,
        })
        .unwrap();
    let view = alice.view().unwrap();
    let row = view.node(nodes.table).unwrap().children[1];
    let first = view.node(row).unwrap().children[0];
    let paragraph = view.node(first).unwrap().children[0];
    alice
        .apply(&Command::InsertText {
            at: docx_engine::Point::Start { start: paragraph },
            text: "typed".into(),
        })
        .unwrap();
    sync(&mut alice, &mut bob);
    let file = alice.export().unwrap();
    assert!(file == bob.export().unwrap());
    let labels = |bytes: &[u8]| -> Vec<Vec<String>> {
        let (_, rows) = read(bytes);
        rows.iter()
            .map(|r| r.iter().map(|c| c.label.clone()).collect())
            .collect()
    };
    assert_eq!(labels(&file), [vec!["", "w1"], vec!["typed", ""]]);
    // The wrapper comes back; the copy keeps its own cell and text.
    assert!(alice.undo().unwrap());
    assert!(alice.undo().unwrap());
    assert!(alice.undo().unwrap());
    assert!(alice.redo().unwrap());
    assert!(alice.redo().unwrap());
    assert!(alice.redo().unwrap());
    assert!(labels(&alice.export().unwrap()) == labels(&file));
}

#[test]
fn a_copy_of_a_vacated_row_outside_the_declared_grid_keeps_its_cells() {
    let cell = |text: &str| format!("<w:tc><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>");
    let held = |text: &str| {
        format!(
            r#"<w:sdt><w:sdtPr><w:tag w:val="held"/></w:sdtPr><w:sdtContent>{}</w:sdtContent></w:sdt>"#,
            cell(text)
        )
    };
    for (grid, row, wrapper) in [
        ("", held("w0"), 0),
        (r#"<w:gridCol w:w="3000"/>"#, cell("lead") + &held("w1"), 1),
    ] {
        let bytes = with_body(&format!(
            "<w:tbl><w:tblGrid>{grid}</w:tblGrid><w:tr>{row}</w:tr></w:tbl><w:p/>"
        ));
        let counts = |bytes: &[u8]| -> Vec<usize> { read(bytes).1.iter().map(Vec::len).collect() };
        let commands = |d: &Document| {
            let row = nodes(d, None).rows[0];
            let remove = Command::RemoveNode {
                node: d.node(row).unwrap().children[wrapper],
            };
            let copy = Command::InsertRow {
                row,
                below: true,
                like: None,
            };
            [remove, copy]
        };
        let mut one = Editor::open(&bytes).unwrap();
        let mut replica = Replica::seed(1, &bytes).unwrap();
        for step in 0..2 {
            let command = commands(one.view().unwrap())[step].clone();
            one.apply(&command).unwrap();
            let command = commands(replica.view().unwrap())[step].clone();
            replica.apply(&command).unwrap();
        }
        let written = replica.export().unwrap();
        assert_eq!(
            counts(&written),
            counts(&one.export().unwrap()),
            "grid {grid:?}"
        );
        let mut fresh = Replica::join(2, &bytes, &replica.save()).unwrap();
        assert!(fresh.export().unwrap() == written);
    }
}

// R6 leaves this state as written: a wrapper's bytes cannot change, so its
// cell keeps its merge mark after the merge above it is cut. This test shows
// the state is reachable and what the file then says. It asserts no more.
#[test]
fn a_cut_off_continuation_inside_a_wrapper_is_left_as_written() {
    let cell = |text: &str, merge: &str| {
        format!("<w:tc><w:tcPr>{merge}</w:tcPr><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>")
    };
    let held = |tc: String| {
        format!(
            r#"<w:sdt><w:sdtPr><w:tag w:val="held"/></w:sdtPr><w:sdtContent>{tc}</w:sdtContent></w:sdt>"#
        )
    };
    let bytes = with_body(&format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="3000"/></w:tblGrid><w:tr>{}</w:tr><w:tr>{}</w:tr><w:tr>{}</w:tr></w:tbl><w:p/>"#,
        cell("S", r#"<w:vMerge w:val="restart"/>"#),
        held(cell("m", "<w:vMerge/>")),
        held(cell("l", "<w:vMerge/>")),
    ));
    for (one, two) in [(1, 2), (2, 1)] {
        let mut alice = Replica::seed(one, &bytes).unwrap();
        let mut bob = Replica::join(two, &bytes, &alice.save()).unwrap();
        let nodes = nodes(alice.view().unwrap(), None);
        let middle = alice.view().unwrap().node(nodes.rows[1]).unwrap().children[0];
        alice
            .apply(&Command::RemoveNode {
                node: nodes.rows[2],
            })
            .unwrap();
        sync(&mut alice, &mut bob);
        bob.apply(&Command::RemoveNode { node: middle }).unwrap();
        assert!(alice.undo().unwrap());
        sync(&mut alice, &mut bob);
        let file = alice.export().unwrap();
        assert!(file == bob.export().unwrap());
        let (_, rows) = read(&file);
        let shown: Vec<_> = (rows.iter())
            .map(|r| (r[0].label.as_str(), r[0].merge.as_deref(), r[0].wrapped))
            .collect();
        assert_eq!(
            shown,
            [
                ("S", Some("restart"), false),
                ("", None, false),
                ("l", Some("continue"), true)
            ]
        );
        assert!(alice.resolutions().unwrap().is_empty());
    }
}

// Marks that a wrapper keeps after its merge is cut are not a merge. Commands
// read them as the reader does: a continuation counts only when the merged
// cells above it reach a start.
#[test]
fn marks_left_inside_wrappers_do_not_act_as_a_merge() {
    let cell = |text: &str, span: usize, merge: &str| {
        format!(
            r#"<w:tc><w:tcPr><w:gridSpan w:val="{span}"/>{merge}</w:tcPr><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>"#
        )
    };
    let held = |text: &str| {
        format!(
            r#"<w:sdt><w:sdtPr><w:tag w:val="{text}"/></w:sdtPr><w:sdtContent>{}</w:sdtContent></w:sdt>"#,
            cell(text, 1, "<w:vMerge/>")
        )
    };
    let bytes = with_body(&format!(
        "<w:tbl><w:tblGrid>{}</w:tblGrid><w:tr>{}{}</w:tr><w:tr>{}{}</w:tr><w:tr>{}{}</w:tr><w:tr>{}{}</w:tr><w:tr>{}</w:tr></w:tbl><w:p/>",
        r#"<w:gridCol w:w="2000"/>"#.repeat(2),
        cell("S", 1, r#"<w:vMerge w:val="restart"/>"#),
        cell("x0", 1, ""),
        held("m"),
        cell("x1", 1, ""),
        held("l"),
        cell("x2", 1, ""),
        held("k"),
        cell("x3", 1, ""),
        cell("W", 2, ""),
    ));
    for (one, two) in [(1, 2), (2, 1)] {
        let mut alice = Replica::seed(one, &bytes).unwrap();
        let mut bob = Replica::join(two, &bytes, &alice.save()).unwrap();
        let nodes = nodes(alice.view().unwrap(), None);
        let wrapper = |replica: &mut Replica, row: usize| {
            replica
                .view()
                .unwrap()
                .node(nodes.rows[row])
                .unwrap()
                .children[0]
        };
        for row in [3, 2] {
            alice
                .apply(&Command::RemoveNode {
                    node: nodes.rows[row],
                })
                .unwrap();
        }
        sync(&mut alice, &mut bob);
        let middle = wrapper(&mut bob, 1);
        bob.apply(&Command::RemoveNode { node: middle }).unwrap();
        assert!(alice.undo().unwrap());
        assert!(alice.undo().unwrap());
        sync(&mut alice, &mut bob);
        // l and k keep their marks under the vacated place, cut off from S.
        let cut = merges(&bob.export().unwrap());
        assert_eq!(cut[1][0], (String::new(), None));
        assert_eq!(cut[2][0], ("l".into(), Some("continue".into())));
        assert_eq!(cut[3][0], ("k".into(), Some("continue".into())));
        // No merge runs between l and k, so a row of any shape may go there,
        // and the codec and the reader agree on the copy.
        for like in [4, 0] {
            let between = Command::InsertRow {
                row: nodes.rows[3],
                below: false,
                like: Some(nodes.rows[like]),
            };
            bob.apply(&between).unwrap();
            let written = bob.export().unwrap();
            assert!(merges(&written)[3].iter().all(|(_, merge)| merge.is_none()));
            assert!(bob.undo().unwrap());
        }
        // Nothing continues through l's wrapper, so it can be removed.
        let lower = wrapper(&mut bob, 2);
        bob.apply(&Command::RemoveNode { node: lower }).unwrap();
        sync(&mut alice, &mut bob);
        assert!(alice.export().unwrap() == bob.export().unwrap());
    }
}

#[test]
fn a_column_inside_a_vacated_place_widens_it_and_its_copy() {
    let cell = |text: &str, span: usize| {
        format!(
            r#"<w:tc><w:tcPr><w:tcW w:w="{}" w:type="dxa"/><w:gridSpan w:val="{span}"/></w:tcPr><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>"#,
            2000 * span
        )
    };
    let bytes = with_body(&format!(
        r#"<w:tbl><w:tblGrid>{}</w:tblGrid><w:tr><w:sdt><w:sdtPr><w:tag w:val="held"/></w:sdtPr><w:sdtContent>{}</w:sdtContent></w:sdt>{}</w:tr><w:tr>{}{}{}</w:tr></w:tbl><w:p/>"#,
        r#"<w:gridCol w:w="2000"/>"#.repeat(3),
        cell("w", 2),
        cell("z", 1),
        cell("a", 1),
        cell("b", 1),
        cell("c", 1),
    ));
    let commands = |d: &Document, step: usize| {
        let nodes = nodes(d, None);
        match step {
            0 => Command::RemoveNode {
                node: d.node(nodes.rows[0]).unwrap().children[0],
            },
            1 => Command::InsertColumn {
                table: nodes.table,
                index: 1,
                like: 0,
            },
            _ => Command::InsertRow {
                row: nodes.rows[1],
                below: true,
                like: Some(nodes.rows[0]),
            },
        }
    };
    let mut one = Editor::open(&bytes).unwrap();
    let mut replica = Replica::seed(1, &bytes).unwrap();
    for step in 0..3 {
        let command = commands(one.view().unwrap(), step);
        one.apply(&command).unwrap();
        let command = commands(replica.view().unwrap(), step);
        replica.apply(&command).unwrap();
    }
    let shape = |bytes: &[u8]| -> Vec<Vec<(String, usize)>> {
        let (_, rows) = read(bytes);
        rows.iter()
            .map(|r| r.iter().map(|c| (c.label.clone(), c.span)).collect())
            .collect()
    };
    let written = replica.export().unwrap();
    assert_eq!(shape(&written), shape(&one.export().unwrap()));
    assert_eq!(shape(&written)[2], [(String::new(), 3), (String::new(), 1)]);
    let view = replica.view().unwrap();
    let copy = view.node(nodes(view, None).table).unwrap().children[2];
    let first = view.node(copy).unwrap().children[0];
    let paragraph = view.node(first).unwrap().children[0];
    replica
        .apply(&Command::InsertText {
            at: docx_engine::Point::Start { start: paragraph },
            text: "typed".into(),
        })
        .unwrap();
    assert_eq!(
        shape(&replica.export().unwrap())[2][0],
        ("typed".to_string(), 3)
    );
}

// A wrapper's cell has no formatting template to copy, but it is a cell. A
// column inserted beside its place gets a cell in that row too.
#[test]
fn a_column_beside_a_vacated_place_gets_a_cell_in_that_row() {
    let cell = |text: &str| format!("<w:tc><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>");
    let bytes = with_body(&format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:sdt><w:sdtPr><w:tag w:val="held"/></w:sdtPr><w:sdtContent>{}</w:sdtContent></w:sdt></w:tr><w:tr>{}</w:tr></w:tbl><w:p/>"#,
        cell("w"),
        cell("A")
    ));
    let counts = |bytes: &[u8]| -> Vec<usize> { read(bytes).1.iter().map(Vec::len).collect() };
    for (index, copied) in [(0, false), (1, false), (0, true), (1, true)] {
        let commands = |d: &Document| {
            let nodes = nodes(d, None);
            let mut all = vec![Command::RemoveNode {
                node: d.node(nodes.rows[0]).unwrap().children[0],
            }];
            if copied {
                all.push(Command::InsertRow {
                    row: nodes.rows[0],
                    below: true,
                    like: None,
                });
            }
            all.push(Command::InsertColumn {
                table: nodes.table,
                index,
                like: 0,
            });
            all
        };
        let mut one = Editor::open(&bytes).unwrap();
        let mut replica = Replica::seed(1, &bytes).unwrap();
        for step in 0..commands(one.view().unwrap()).len() {
            let command = commands(one.view().unwrap())[step].clone();
            one.apply(&command).unwrap();
            let command = commands(replica.view().unwrap())[step].clone();
            replica.apply(&command).unwrap();
        }
        let case = format!("column at {index}, copied {copied}");
        let written = replica.export().unwrap();
        assert_eq!(counts(&written), counts(&one.export().unwrap()), "{case}");
        assert!(
            !part(&written, "word/document.xml").contains("gridBefore"),
            "{case}"
        );
        assert!(replica.resolutions().unwrap().is_empty(), "{case}");
        // The grid is whole, so a further column still goes in.
        let table = nodes(replica.view().unwrap(), None).table;
        replica
            .apply(&Command::InsertColumn {
                table,
                index: 0,
                like: 0,
            })
            .unwrap();
    }
}

#[test]
fn the_reader_reports_a_kept_row_and_a_kept_wrapper_apart() {
    let cell = |text: &str, merge: &str| {
        format!("<w:tc><w:tcPr>{merge}</w:tcPr><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>")
    };
    let table = |held: bool| {
        let start = cell("S", r#"<w:vMerge w:val="restart"/>"#);
        let start = if held {
            format!(
                r#"<w:sdt><w:sdtPr><w:tag w:val="held"/></w:sdtPr><w:sdtContent>{start}</w:sdtContent></w:sdt>"#
            )
        } else {
            start
        };
        with_body(&format!(
            "<w:tbl><w:tblGrid>{}</w:tblGrid><w:tr>{start}{}</w:tr><w:tr>{}{}</w:tr><w:tr>{}{}</w:tr></w:tbl><w:p/>",
            r#"<w:gridCol w:w="2000"/>"#.repeat(2),
            cell("x0", ""),
            cell("c", "<w:vMerge/>"),
            cell("x1", ""),
            cell("z0", ""),
            cell("z1", ""),
        ))
    };
    for held in [false, true] {
        let bytes = table(held);
        let mut alice = Replica::seed(1, &bytes).unwrap();
        let mut bob = Replica::join(2, &bytes, &alice.save()).unwrap();
        let nodes = nodes(alice.view().unwrap(), None);
        bob.apply(&Command::RemoveNode {
            node: nodes.rows[1],
        })
        .unwrap();
        sync(&mut alice, &mut bob);
        // With the continuation gone, the start can go: its row, or its wrapper.
        let start = if held {
            alice.view().unwrap().node(nodes.rows[0]).unwrap().children[0]
        } else {
            nodes.rows[0]
        };
        alice.apply(&Command::RemoveNode { node: start }).unwrap();
        assert!(bob.undo().unwrap());
        sync(&mut alice, &mut bob);
        assert_eq!(read(&alice.export().unwrap()).1.len(), 3);
        let expected = if held {
            Resolution::MergeStartWrapper
        } else {
            Resolution::MergeStartRow
        };
        assert_eq!(alice.resolutions().unwrap(), &[expected]);
    }
}
