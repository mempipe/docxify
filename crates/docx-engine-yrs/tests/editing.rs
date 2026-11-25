//! Semantic editing, projection and collaboration regressions.

#[path = "../../docx-engine/tests/common/mod.rs"]
mod common;
use std::path::PathBuf;

use docx_engine::{Command, Document, Format, NodeId, NodeKind, Point, TextRange};
use docx_engine_yrs::store::Store;
use docx_engine_yrs::{build, paragraphs, seed, Char, Inline, Mark, Paragraph, Replica, BASIS};

fn fixtures() -> Vec<(String, Vec<u8>)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/generated");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "docx") {
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            found.push((name, std::fs::read(&path).unwrap()));
        }
    }
    found.sort();
    found
}

/// The body's paragraphs as the engine's own tree gives them: each
/// character with the run it is in.
fn inspected(document: &Document) -> Vec<Paragraph> {
    let body = document.stories().next().unwrap();
    let paragraph = |&id: &NodeId| {
        let mut text = Vec::new();
        for &basis in &document.node(id).unwrap().children {
            for &child in &document.node(basis).unwrap().children {
                if let NodeKind::Text { text: more } = &document.node(child).unwrap().kind {
                    let format = Format::default();
                    text.extend(more.chars().map(|c| {
                        Inline::Char(Char {
                            c,
                            source_text: Some(child),
                            basis: Some(basis),
                            format,
                            revision: None,
                            deletion: None,
                            link: None,
                        })
                    }));
                } else {
                    text.push(Inline::Source {
                        node: child,
                        basis,
                        format: Format::default(),
                        revision: None,
                        deletion: None,
                        link: None,
                    });
                }
            }
        }
        let mark = Mark::Base(id);
        Paragraph {
            mark,
            text,
            revision: None,
            deletion: None,
        }
    };
    let paragraphs = body
        .children
        .iter()
        .filter(|id| matches!(document.node(**id).unwrap().kind, NodeKind::Paragraph(_)));
    paragraphs.map(paragraph).collect()
}

/// Every fixture this path takes reads back as the engine inspects it:
/// the same paragraphs, and each character with the run it came with. A
/// replica that opens what the first saved reads the same.
#[test]
fn a_seeded_flow_reads_as_the_engine_inspects_the_file() {
    let all = [
        "anchored-image",
        "header-footer",
        "headings-styles",
        "image",
        "lists",
        "paragraph-layout",
        "rich-text",
        "sections",
        "simple-paragraphs",
    ];
    let mut taken = Vec::new();
    for (name, bytes) in fixtures()
        .into_iter()
        .filter(|(name, _)| all.contains(&name.as_str()))
    {
        let document = Document::open(&bytes).unwrap();
        let store = seed(1, &document, &[]).unwrap();
        let read = paragraphs(&document, &store).unwrap();
        let body = document.stories().next().unwrap();
        let body_read: Vec<_> = read.iter().filter(|p| matches!(p.mark, Mark::Base(id) if document.node(id).unwrap().parent == Some(body.id))).cloned().collect();
        assert_eq!(body_read, inspected(&document), "{name}");
        let other = Store::load(2, &store.save()).unwrap();
        assert_eq!(paragraphs(&document, &other).unwrap(), read, "{name}");
        taken.push(name);
    }
    assert_eq!(taken, all);
}

/// Tables stay between the body paragraphs and their cells have separate
/// text regions, with their own positions and properties.
#[test]
fn a_table_is_preserved_and_its_cells_are_editable() {
    let bytes = bytes("table");
    let mut replica = Replica::seed(1, &bytes).unwrap();
    let baseline = replica.export().unwrap();
    let doc = replica.view().unwrap();
    let cell = doc
        .descendants(doc.stories().next().unwrap().id)
        .into_iter()
        .find(|id| matches!(doc.node(*id).unwrap().kind, NodeKind::Cell(_)))
        .unwrap();
    let node = doc
        .descendants(cell)
        .into_iter()
        .find(|id| matches!(doc.node(*id).unwrap().kind, NodeKind::Text { .. }))
        .unwrap();
    replica
        .apply(&Command::ReplaceText {
            node,
            range: Some(TextRange { start: 0, end: 0 }),
            text: "cell edit".into(),
        })
        .unwrap();
    assert!(docx_engine::validate_package(&replica.export().unwrap()).valid);
    replica.undo().unwrap();
    assert_eq!(replica.export().unwrap(), baseline);
}

fn fixture(name: &str) -> Document {
    let found = fixtures().into_iter().find(|(n, _)| n == name);
    Document::open(&found.unwrap().1).unwrap()
}

/// What a document shows, read back from its exported bytes: each
/// paragraph's characters, each with the properties of its run.
fn shown(document: &Document) -> Vec<Vec<(char, String)>> {
    let document = Document::open(&document.export().unwrap()).unwrap();
    let body = document.stories().next().unwrap();
    let mut paragraphs = Vec::new();
    for &id in &body.children {
        if !matches!(document.node(id).unwrap().kind, NodeKind::Paragraph(_)) {
            continue;
        }
        let mut text = Vec::new();
        for &run in &document.node(id).unwrap().children {
            let NodeKind::Run(info) = &document.node(run).unwrap().kind else {
                panic!("a paragraph holds only runs");
            };
            for &child in &document.node(run).unwrap().children {
                if let NodeKind::Text { text: more } = &document.node(child).unwrap().kind {
                    text.extend(more.chars().map(|c| (c, format!("{info:?}"))));
                }
            }
        }
        paragraphs.push(text);
    }
    paragraphs
}

/// The unit in the flow where `needle` starts, and the run it is in.
fn find(store: &Store, needle: &str) -> (usize, String) {
    let units = store.read().unwrap();
    let text: String = (units.iter())
        .map(|unit| match unit.content {
            docx_engine_yrs::store::Content::Char(c) => c,
            _ => '\n',
        })
        .collect();
    let at = text[..text.find(needle).unwrap()].chars().count();
    (at, units[at].one(BASIS).unwrap().to_string())
}

/// A flow nobody edited builds the file's own tree, and exports its bytes.
#[test]
fn an_unedited_flow_exports_the_files_bytes() {
    for (name, bytes) in fixtures() {
        let base = Document::open(&bytes).unwrap();
        let Ok(store) = seed(1, &base, &[]) else {
            continue;
        };
        let built = build(&base, &store).unwrap().document;
        assert!(built.export().unwrap() == base.export().unwrap(), "{name}");
    }
}

/// Text typed and deleted in the flow shows what the same edits show as
/// commands on the document. The runs may be cut at other places.
#[test]
fn text_typed_and_deleted_shows_what_the_commands_show() {
    let base = fixture("simple-paragraphs");
    let mut store = seed(1, &base, &[]).unwrap();
    let (at, basis) = find(&store, "paragraph.");
    store
        .insert(at, "typed ", &[(BASIS, Some(&basis))])
        .unwrap();
    let (at, _) = find(&store, "Second");
    store.delete(at, 7).unwrap();

    let mut direct = docx_engine::editing::Editor::open(&base.export().unwrap()).unwrap();
    let node = |document: &Document, needle: &str| {
        let texts = document
            .paragraphs()
            .into_iter()
            .flat_map(|p| document.node(p).unwrap().children.clone());
        let mut texts = texts.flat_map(|run| document.node(run).unwrap().children.clone());
        texts
            .find(|id| document.text(*id).is_some_and(|text| text.contains(needle)))
            .unwrap()
    };
    let typed = TextRange { start: 6, end: 6 };
    let first = node(direct.view().unwrap(), "First");
    direct
        .apply(&Command::ReplaceText {
            node: first,
            range: Some(typed),
            text: "typed ".into(),
        })
        .unwrap();
    let erased = TextRange { start: 0, end: 7 };
    let second = node(direct.view().unwrap(), "Second");
    direct
        .apply(&Command::ReplaceText {
            node: second,
            range: Some(erased),
            text: String::new(),
        })
        .unwrap();

    let built = build(&base, &store).unwrap().document;
    assert_eq!(shown(&built), shown(direct.view().unwrap()));
    let text =
        |paragraph: &Vec<(char, String)>| paragraph.iter().map(|(c, _)| *c).collect::<String>();
    assert_eq!(text(&shown(&built)[0]), "First typed paragraph.");
    assert_eq!(text(&shown(&built)[1]), "paragraph.");
}

/// Text that states another run's basis is shown with that run's
/// properties, between the two halves of the text it was typed into.
#[test]
fn text_with_another_basis_gets_a_run_with_that_runs_properties() {
    let base = fixture("headings-styles");
    let mut store = seed(1, &base, &[]).unwrap();
    let before = shown(&base);
    // The basis of the first character with other properties than the first.
    let units = store.read().unwrap();
    let other = (before
        .iter()
        .flatten()
        .zip(units.iter().filter(|u| u.one(BASIS).is_some())))
    .find(|((_, properties), _)| *properties != before[0][0].1);
    let ((_, properties), unit) = other.expect("the fixture has two kinds of run");
    let basis = unit.one(BASIS).unwrap().to_string();
    store.insert(3, "Q", &[(BASIS, Some(&basis))]).unwrap();

    let built = shown(&build(&base, &store).unwrap().document);
    let mut wanted = before.clone();
    wanted[0].insert(2, ('Q', properties.clone()));
    assert_eq!(built, wanted);
}

/// The text node of `document` that holds `needle`.
fn node(document: &Document, needle: &str) -> NodeId {
    let runs = (document.paragraphs().into_iter())
        .flat_map(|paragraph| document.node(paragraph).unwrap().children.clone());
    let mut texts = runs.flat_map(|run| document.node(run).unwrap().children.clone());
    let holds = |id: &NodeId| document.text(*id).is_some_and(|text| text.contains(needle));
    texts.find(holds).unwrap()
}

fn bytes(name: &str) -> Vec<u8> {
    let found = fixtures().into_iter().find(|(n, _)| n == name);
    found.unwrap().1
}

fn sync(a: &mut Replica, b: &mut Replica) {
    let to_b = a.update_since(&b.state_vector()).unwrap();
    let to_a = b.update_since(&a.state_vector()).unwrap();
    b.apply_update(&to_b).unwrap();
    a.apply_update(&to_a).unwrap();
}

fn type_at(replica: &mut Replica, needle: &str, offset: usize, text: &str) {
    let node = node(replica.view().unwrap(), needle);
    let range = Some(TextRange {
        start: offset,
        end: offset,
    });
    let text = text.to_string();
    replica
        .apply(&Command::ReplaceText { node, range, text })
        .unwrap();
}

/// Row 2 of the contract on a real file, through commands: one person
/// types into text while another deletes all of it. The typed text is
/// left, with the properties of the run it was typed into. On the first
/// path it loses them (spec R6).
#[test]
fn text_typed_into_text_another_deletes_keeps_its_runs_properties() {
    let file = bytes("headings-styles");
    let before = shown(&Document::open(&file).unwrap());
    let heading: String = before[0].iter().map(|(c, _)| *c).collect();
    let mut alice = Replica::seed(1, &file).unwrap();
    let mut bob = Replica::join(2, &file, &alice.save()).unwrap();
    type_at(&mut alice, &heading, 3, "typed");
    let whole = node(bob.view().unwrap(), &heading);
    bob.apply(&Command::ReplaceText {
        node: whole,
        range: None,
        text: String::new(),
    })
    .unwrap();
    sync(&mut alice, &mut bob);
    let shown_by = |replica: &mut Replica| shown(replica.view().unwrap());
    let both = shown_by(&mut alice);
    assert_eq!(both, shown_by(&mut bob));
    let wanted: Vec<(char, String)> = "typed"
        .chars()
        .map(|c| (c, before[0][0].1.clone()))
        .collect();
    assert_eq!(both[0], wanted);
    assert_eq!(both[1..], before[1..]);
}

/// Undo takes back this replica's change and leaves the other's.
#[test]
fn undo_takes_back_ones_own_typing_only() {
    let file = bytes("simple-paragraphs");
    let mut alice = Replica::seed(1, &file).unwrap();
    let mut bob = Replica::join(2, &file, &alice.save()).unwrap();
    type_at(&mut alice, "First", 0, "A");
    type_at(&mut bob, "Second", 0, "B");
    sync(&mut alice, &mut bob);
    assert!(alice.undo().unwrap());
    sync(&mut alice, &mut bob);
    let text = |replica: &mut Replica| {
        let shown = shown(replica.view().unwrap());
        let line =
            |paragraph: &Vec<(char, String)>| paragraph.iter().map(|(c, _)| *c).collect::<String>();
        shown.iter().map(line).collect::<Vec<_>>()
    };
    assert_eq!(
        text(&mut alice)[..2],
        ["First paragraph.", "BSecond paragraph."]
    );
    assert_eq!(text(&mut alice), text(&mut bob));
}

/// Text is removed by a text command, not by deleting its projected node.
#[test]
fn removing_text_as_an_object_is_refused() {
    let file = bytes("simple-paragraphs");
    let mut alice = Replica::seed(1, &file).unwrap();
    let before = alice.save();
    let node = node(alice.view().unwrap(), "First");
    let refused = alice.apply(&Command::RemoveNode { node }).err().unwrap();
    assert_eq!(
        refused.to_string(),
        format!("{node}: change its text with replace_text instead")
    );
    assert!(alice.save() == before);
}

fn lines(replica: &mut Replica) -> Vec<String> {
    let shown = shown(replica.view().unwrap());
    let line = |paragraph: &Vec<(char, String)>| paragraph.iter().map(|(c, _)| *c).collect();
    shown.iter().map(line).collect()
}

/// Typing over text is one change: one undo takes back the new text and
/// brings back the old.
#[test]
fn text_typed_over_text_is_undone_in_one_step() {
    let file = bytes("simple-paragraphs");
    let mut alice = Replica::seed(1, &file).unwrap();
    let before = lines(&mut alice);
    let node = node(alice.view().unwrap(), "First");
    let range = Some(TextRange { start: 0, end: 5 });
    let text = "New".to_string();
    alice
        .apply(&Command::ReplaceText { node, range, text })
        .unwrap();
    assert_eq!(lines(&mut alice)[0], "New paragraph.");
    assert!(alice.undo().unwrap());
    assert_eq!(lines(&mut alice), before);
}

#[test]
fn an_older_session_is_identified_before_its_strands_are_decoded() {
    use yrs::{Doc, Map, ReadTxn, StateVector, Transact};

    let doc = Doc::new();
    let meta = doc.get_or_insert_map("meta");
    let strands = doc.get_or_insert_map("strands");
    let mut txn = doc.transact_mut();
    meta.insert(&mut txn, "format", "docx-engine.yrs/6");
    strands.insert(&mut txn, "old-key", "old representation");
    let saved = txn.encode_state_as_update_v1(&StateVector::default());
    assert!(matches!(
        Replica::join(2, &bytes("simple-paragraphs"), &saved),
        Err(docx_engine_yrs::CollabError::Unsupported {
            command: "join",
            ..
        })
    ));
}

/// A shared state is joined only with the file it was started from.
#[test]
fn a_shared_state_is_not_joined_with_another_file() {
    let alice = Replica::seed(1, &bytes("simple-paragraphs")).unwrap();
    let refused = Replica::join(2, &bytes("headings-styles"), &alice.save());
    let refused = refused.err().expect("another file is refused").to_string();
    assert!(
        refused.starts_with("the shared state was seeded from base "),
        "{refused}"
    );
}

/// Text the engine cannot hold is refused before anything is written, so
/// no replica is left with a state it cannot build.
#[test]
fn text_xml_cannot_hold_is_refused_before_it_is_written() {
    let file = bytes("simple-paragraphs");
    let mut alice = Replica::seed(1, &file).unwrap();
    let before = alice.save();
    let node = node(alice.view().unwrap(), "First");
    for text in ["\u{FFFF}", "a\tb"] {
        let range = Some(TextRange { start: 0, end: 0 });
        let text = text.to_string();
        let refused = alice.apply(&Command::ReplaceText { node, range, text });
        assert!(refused.is_err());
        assert!(alice.save() == before);
        alice.view().unwrap();
    }
}

/// What a document shows with each paragraph's own properties too.
fn laid_out(document: &Document) -> Vec<(String, Vec<(char, String)>)> {
    let reopened = Document::open(&document.export().unwrap()).unwrap();
    let body = reopened.stories().next().unwrap();
    let info = |id: &NodeId| match &reopened.node(*id).unwrap().kind {
        NodeKind::Paragraph(info) => Some(format!("{info:?}")),
        _ => None,
    };
    let infos = body.children.iter().filter_map(info);
    infos.zip(shown(document)).collect()
}

/// A command resolved against the document it is applied to, so a replica's
/// view and a plain document each name their own nodes.
type Step = Box<dyn Fn(&Document) -> Command>;

fn at(needle: &'static str, offset: usize) -> impl Fn(&Document) -> Point {
    move |document| {
        let node = node(document, needle);
        let start = document.text(node).unwrap().find(needle).unwrap();
        let offset = document.text(node).unwrap()[..start].chars().count() + offset;
        Point::Text { node, offset }
    }
}

/// The place `offset` characters into the body's paragraph `index`.
fn spot(index: usize, offset: usize) -> impl Fn(&Document) -> Point {
    move |document| {
        let mut left = offset;
        for &run in &document
            .node(document.paragraphs()[index])
            .unwrap()
            .children
        {
            for &node in &document.node(run).unwrap().children {
                let len = document.text(node).unwrap().chars().count();
                if left <= len {
                    return Point::Text { node, offset: left };
                }
                left -= len;
            }
        }
        panic!("paragraph {index} holds {offset} characters");
    }
}

/// Runs the steps as commands on a replica and on a plain document, and
/// requires the replica to show what the document shows.
fn matches_commands(fixture: &str, steps: Vec<Step>) -> Replica {
    let file = bytes(fixture);
    let mut replica = Replica::seed(1, &file).unwrap();
    let mut direct = docx_engine::editing::Editor::open(&file).unwrap();
    for step in &steps {
        let command = step(replica.view().unwrap());
        replica
            .apply(&command)
            .unwrap_or_else(|e| panic!("{command:?}: {e}"));
        let command = step(direct.view().unwrap());
        direct.apply(&command).unwrap();
        assert_eq!(
            laid_out(replica.view().unwrap()),
            laid_out(direct.view().unwrap()),
            "{command:?}"
        );
    }
    replica
}

#[test]
fn enter_and_joining_show_what_the_commands_show() {
    let split: Step = Box::new(|d| Command::SplitParagraph {
        at: at("First", 5)(d),
    });
    let typed: Step = Box::new(|d| Command::InsertText {
        at: at(" paragraph.", 0)(d),
        text: "New".into(),
    });
    let join: Step = Box::new(|d| {
        let first = d.paragraphs()[0];
        Command::MergeParagraphs { paragraph: first }
    });
    let mut replica = matches_commands("simple-paragraphs", vec![split, typed, join]);
    assert_eq!(lines(&mut replica)[0], "FirstNew paragraph.");
    // Each command is one change to take back.
    for wanted in ["New paragraph.", " paragraph."] {
        assert!(replica.undo().unwrap());
        assert_eq!(lines(&mut replica)[1], wanted);
    }
    assert!(replica.undo().unwrap());
    assert_eq!(lines(&mut replica)[0], "First paragraph.");
}

#[test]
fn a_range_over_paragraphs_is_replaced_with_lines() {
    let replace: Step = Box::new(|d| Command::ReplaceRange {
        from: at("First", 2)(d),
        to: at("Second", 3)(d),
        text: "X\nY".into(),
        track: None,
    });
    let mut replica = matches_commands("simple-paragraphs", vec![replace]);
    assert_eq!(lines(&mut replica)[..2], ["FiX", "Yond paragraph."]);
}

#[test]
fn text_typed_in_an_empty_paragraph_is_like_its_paragraph() {
    let empty: Step = Box::new(|d| Command::ReplaceText {
        node: node(d, "Second"),
        range: None,
        text: String::new(),
    });
    let mut replica = matches_commands("simple-paragraphs", vec![empty]);
    let second = replica.view().unwrap().paragraphs()[1];
    let typed = Command::InsertText {
        at: Point::End { end: second },
        text: "typed".into(),
    };
    replica.apply(&typed).unwrap();
    assert_eq!(lines(&mut replica)[1], "typed");
}

#[test]
fn bold_italic_and_underline_show_what_the_commands_show() {
    let format = |bold, italic, underline| Format {
        bold,
        italic,
        underline,
    };
    let range = |from: usize, to: usize, format: Format| -> Step {
        Box::new(move |d| Command::FormatRange {
            from: spot(1, from)(d),
            to: spot(1, to)(d),
            format,
        })
    };
    let on = Some(true);
    let steps = vec![
        range(7, 16, format(on, None, None)),
        range(9, 10, format(Some(false), on, None)),
        range(0, 9, format(None, None, on)),
    ];
    matches_commands("simple-paragraphs", steps);
}

/// Row 1 of the contract on a real file, through commands: `pa` and
/// `agraph` are bold and the `r` between is not. One person deletes `pa`
/// and another deletes `r`. `agraph`, which neither touched, stays bold. On
/// the first path it does not (spec R6).
#[test]
fn text_keeps_its_bold_when_text_on_each_side_of_plain_text_is_deleted() {
    let file = bytes("simple-paragraphs");
    let mut alice = Replica::seed(1, &file).unwrap();
    let format = |bold: bool| Format {
        bold: Some(bold),
        ..Format::default()
    };
    let range = |replica: &mut Replica, a: usize, b: usize, text: Option<Format>| {
        let view = replica.view().unwrap();
        let (from, to) = (spot(1, 7 + a)(view), spot(1, 7 + b)(view));
        let command = match text {
            Some(format) => Command::FormatRange { from, to, format },
            None => Command::ReplaceRange {
                from,
                to,
                text: String::new(),
                track: None,
            },
        };
        replica.apply(&command).unwrap();
    };
    range(&mut alice, 0, 9, Some(format(true)));
    range(&mut alice, 2, 3, Some(format(false)));
    let mut bob = Replica::join(2, &file, &alice.save()).unwrap();
    range(&mut alice, 0, 2, None);
    range(&mut bob, 2, 3, None);
    sync(&mut alice, &mut bob);
    let bold = |replica: &mut Replica| -> String {
        let second = shown(replica.view().unwrap()).swap_remove(1).into_iter();
        (second.filter(|(_, run)| run.contains("bold: Some(true)")))
            .map(|(c, _)| c)
            .collect()
    };
    assert_eq!(lines(&mut alice)[1], "Second agraph.");
    assert_eq!(bold(&mut alice), "agraph");
    assert_eq!(bold(&mut bob), "agraph");
}

/// Row 10 of the contract through commands: two people press Enter at one
/// place, and there is an empty paragraph between.
#[test]
fn two_enters_at_one_place_give_an_empty_paragraph() {
    let file = bytes("simple-paragraphs");
    let mut alice = Replica::seed(1, &file).unwrap();
    let mut bob = Replica::join(2, &file, &alice.save()).unwrap();
    for replica in [&mut alice, &mut bob] {
        let at = at("First", 5)(replica.view().unwrap());
        replica.apply(&Command::SplitParagraph { at }).unwrap();
    }
    sync(&mut alice, &mut bob);
    assert_eq!(lines(&mut alice)[..3], ["First", "", " paragraph."]);
    assert_eq!(lines(&mut alice), lines(&mut bob));
}

/// Commands sent together are one change, and none of it stays if one of
/// them is refused. Another replica that gets what was sent still agrees.
#[test]
fn commands_sent_together_are_all_written_or_none() {
    let file = bytes("simple-paragraphs");
    let mut alice = Replica::seed(1, &file).unwrap();
    let mut bob = Replica::join(2, &file, &alice.save()).unwrap();
    let before = lines(&mut alice);
    let typed = Command::InsertText {
        at: at("First", 0)(alice.view().unwrap()),
        text: "A".into(),
    };
    let node = node(alice.view().unwrap(), "Second");
    let refused = alice.apply_all(&[typed.clone(), Command::RemoveNode { node }]);
    assert!(refused.is_err());
    assert_eq!(lines(&mut alice), before);
    assert!(!alice.undo().unwrap());

    let again = Command::InsertText {
        at: at("First", 0)(alice.view().unwrap()),
        text: "B".into(),
    };
    let enter = Command::SplitParagraph {
        at: at("Second", 3)(alice.view().unwrap()),
    };
    alice.apply_all(&[again, enter]).unwrap();
    while let Some(update) = alice.take_update().unwrap() {
        bob.apply_update(&update).unwrap();
    }
    assert_eq!(
        lines(&mut bob)[..3],
        ["BFirst paragraph.", "Sec", "ond paragraph."]
    );
    assert_eq!(lines(&mut alice), lines(&mut bob));
    assert!(alice.undo().unwrap());
    assert_eq!(lines(&mut alice), before);
}

#[test]
fn a_refused_batch_after_a_noop_keeps_earlier_edits_and_history() {
    let file = bytes("simple-paragraphs");
    let mut alice = Replica::seed(1, &file).unwrap();
    type_at(&mut alice, "First", 0, "A");
    let before = alice.save();
    let update = alice.take_update().unwrap();
    let id = node(alice.view().unwrap(), "First");
    let noop = Command::ReplaceText {
        node: id,
        range: Some(TextRange { start: 0, end: 0 }),
        text: String::new(),
    };
    assert!(alice
        .apply_all(&[noop, Command::RemoveNode { node: id }])
        .is_err());
    assert_eq!(alice.save(), before);
    assert!(alice.take_update().unwrap().is_none());
    assert_eq!(lines(&mut alice)[0], "AFirst paragraph.");
    assert!(alice.undo().unwrap());
    assert_eq!(lines(&mut alice)[0], "First paragraph.");
    assert!(alice.redo().unwrap());
    assert_eq!(lines(&mut alice)[0], "AFirst paragraph.");
    assert!(update.is_some());
}

#[test]
fn a_refused_batch_that_wrote_text_keeps_redo_and_sends_nothing() {
    let file = bytes("simple-paragraphs");
    let mut alice = Replica::seed(1, &file).unwrap();
    type_at(&mut alice, "First", 0, "A");
    alice.undo().unwrap();
    alice.take_update().unwrap();
    let before = alice.save();
    let id = node(alice.view().unwrap(), "First");
    let typed = Command::InsertText {
        at: Point::Text {
            node: id,
            offset: 0,
        },
        text: "discarded".into(),
    };
    assert!(alice
        .apply_all(&[typed, Command::RemoveNode { node: id }])
        .is_err());
    assert_eq!(alice.save(), before);
    assert!(alice.take_update().unwrap().is_none());
    assert!(alice.redo().unwrap());
    assert_eq!(lines(&mut alice)[0], "AFirst paragraph.");
}

#[test]
fn a_paragraph_kept_after_a_concurrent_join_can_be_split_and_edited() {
    let file = bytes("simple-paragraphs");
    let mut alice = Replica::seed(1, &file).unwrap();
    let first = node(alice.view().unwrap(), "First");
    alice
        .apply(&Command::ReplaceText {
            node: first,
            range: None,
            text: String::new(),
        })
        .unwrap();
    let mut bob = Replica::join(2, &file, &alice.save()).unwrap();
    let first = alice.view().unwrap().paragraphs()[0];
    alice
        .apply(&Command::MergeParagraphs { paragraph: first })
        .unwrap();
    bob.apply(&Command::InsertText {
        at: Point::End { end: first },
        text: "Q".into(),
    })
    .unwrap();
    sync(&mut alice, &mut bob);
    assert_eq!(lines(&mut alice)[0], "Q");
    let first = alice.view().unwrap().paragraphs()[0];
    alice
        .apply(&Command::InsertText {
            at: Point::End { end: first },
            text: "Z\n".into(),
        })
        .unwrap();
    assert_eq!(lines(&mut alice)[..2], ["QZ", ""]);
    let empty = alice.view().unwrap().paragraphs()[1];
    alice
        .apply(&Command::InsertText {
            at: Point::End { end: empty },
            text: "W".into(),
        })
        .unwrap();
    sync(&mut alice, &mut bob);
    assert_eq!(lines(&mut alice)[..2], ["QZ", "W"]);
    assert_eq!(
        laid_out(alice.view().unwrap()),
        laid_out(bob.view().unwrap())
    );
    assert!(alice.undo().unwrap());
    assert!(alice.undo().unwrap());
    assert_eq!(lines(&mut alice)[..2], ["Q", "Second paragraph."]);
    assert!(alice.redo().unwrap());
    assert!(alice.redo().unwrap());
    sync(&mut alice, &mut bob);
    assert_eq!(lines(&mut alice), lines(&mut bob));
}

#[test]
fn replacing_the_start_of_a_bold_run_keeps_the_targets_properties() {
    let mut base = docx_engine::editing::Editor::open(&bytes("simple-paragraphs")).unwrap();
    let id = node(base.view().unwrap(), "First");
    base.apply(&Command::FormatRange {
        from: Point::Text {
            node: id,
            offset: 1,
        },
        to: Point::Text {
            node: id,
            offset: 5,
        },
        format: Format {
            bold: Some(true),
            ..Format::default()
        },
    })
    .unwrap();
    let file = base.export().unwrap();
    for insert in [false, true] {
        let mut alice = Replica::seed(1, &file).unwrap();
        let mut plain = docx_engine::editing::Editor::open(&file).unwrap();
        let id = node(alice.view().unwrap(), "irst");
        let command = if insert {
            Command::InsertText {
                at: Point::Text {
                    node: id,
                    offset: 0,
                },
                text: "X".into(),
            }
        } else {
            Command::ReplaceText {
                node: id,
                range: Some(TextRange { start: 0, end: 1 }),
                text: "X".into(),
            }
        };
        alice.apply(&command).unwrap();
        plain.apply(&command).unwrap();
        assert_eq!(shown(alice.view().unwrap()), shown(plain.view().unwrap()));
    }
}

#[test]
fn headless_and_collaborative_editing_share_commands_building_and_refusal() {
    use docx_engine::editing::Editor;
    let (_, bytes) = fixtures()
        .into_iter()
        .find(|(name, _)| name == "simple-paragraphs")
        .unwrap();
    let mut plain = Editor::open(&bytes).unwrap();
    let mut shared = Replica::seed(101, &bytes).unwrap();
    for text in ["X", "🦀", "first\nsecond", ""] {
        let doc = plain.view().unwrap();
        let story = doc.stories().next().unwrap().id;
        let node = doc.descendants(story).into_iter().find(|id| matches!(&doc.node(*id).unwrap().kind, NodeKind::Text { text } if !text.is_empty())).unwrap();
        let command = Command::ReplaceText {
            node,
            range: Some(TextRange { start: 0, end: 1 }),
            text: text.into(),
        };
        plain.apply(&command).unwrap();
        shared.apply(&command).unwrap();
        assert_eq!(plain.export().unwrap(), shared.export().unwrap());
    }
    let current = plain.export().unwrap();
    assert!(plain.undo());
    assert!(shared.undo().unwrap());
    assert_eq!(plain.export().unwrap(), shared.export().unwrap());
    let before = plain.export().unwrap();
    let doc = plain.view().unwrap();
    let story = doc.stories().next().unwrap().id;
    let node = doc.descendants(story).into_iter().find(|id| matches!(&doc.node(*id).unwrap().kind, NodeKind::Text { text } if !text.is_empty())).unwrap();
    let batch = [
        Command::InsertText {
            at: Point::Text { node, offset: 0 },
            text: "private".into(),
        },
        Command::InsertText {
            at: Point::Text {
                node,
                offset: usize::MAX,
            },
            text: "refused".into(),
        },
    ];
    assert!(plain.apply_all(&batch).is_err());
    assert!(shared.apply_all(&batch).is_err());
    assert_eq!(plain.export().unwrap(), before);
    assert_eq!(shared.export().unwrap(), before);
    assert!(plain.redo());
    assert!(shared.redo().unwrap());
    assert_eq!(plain.export().unwrap(), current);
    assert_eq!(shared.export().unwrap(), current);
}

fn inline_fixture() -> Vec<u8> {
    use std::io::{Cursor, Read, Write};
    let (_, bytes) = fixtures()
        .into_iter()
        .find(|(name, _)| name == "simple-paragraphs")
        .unwrap();
    let mut input = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..input.len() {
        let mut entry = input.by_index(i).unwrap();
        let mut content = Vec::new();
        entry.read_to_end(&mut content).unwrap();
        if entry.name().unwrap() == "word/document.xml" {
            content = String::from_utf8(content).unwrap().replacen("</w:t>", "</w:t><w:tab/><w:br/><x:keep xmlns:x=\"urn:test:preserve\" x:value=\"unchanged\"/>", 1).into_bytes();
        }
        output
            .start_file(
                entry.name().unwrap(),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        output.write_all(&content).unwrap();
    }
    output.finish().unwrap().into_inner()
}

#[test]
fn undoing_a_split_after_a_control_keeps_the_other_authors_text() {
    let bytes = common::generated("content-controls");
    let mut alice = Replica::seed(1, &bytes).unwrap();
    let mut bob = Replica::join(2, &bytes, &alice.save()).unwrap();
    let view = alice.view().unwrap();
    let paragraphs = view.paragraphs();
    let index = paragraphs
        .iter()
        .position(|p| view.text(*p).unwrap().starts_with("Name:"))
        .unwrap();
    let name = paragraphs[index];
    let expected = view.text(name).unwrap() + "X";
    alice
        .apply(&Command::SplitParagraph {
            at: Point::End { end: name },
        })
        .unwrap();
    sync(&mut alice, &mut bob);
    let start = bob.view().unwrap().paragraphs()[index + 1];
    bob.apply(&Command::InsertText {
        at: Point::Start { start },
        text: "X".into(),
    })
    .unwrap();
    sync(&mut alice, &mut bob);
    assert!(alice.undo().unwrap());
    sync(&mut alice, &mut bob);

    let mut joined = Replica::join(3, &bytes, &alice.save()).unwrap();
    for replica in [&mut alice, &mut bob, &mut joined] {
        let check =
            |doc: &Document| {
                let p = doc.paragraphs()[index];
                assert_eq!(doc.text(p).unwrap(), expected);
                assert!(doc.node(p).unwrap().children.iter().any(|id| matches!(
                    doc.node(*id).unwrap().kind,
                    NodeKind::ContentControl { .. }
                )));
            };
        check(replica.view().unwrap());
        check(&Document::open(&replica.export().unwrap()).unwrap());
    }
}

#[test]
fn inline_source_content_survives_edits_splits_sync_and_undo() {
    let bytes = inline_fixture();
    let base = Document::open(&bytes).unwrap();
    let root = base.stories().next().unwrap().id;
    let held: Vec<_> = base
        .descendants(root)
        .into_iter()
        .filter(|id| {
            matches!(
                base.node(*id).unwrap().kind,
                NodeKind::Tab | NodeKind::Break { .. } | NodeKind::Opaque { .. }
            )
        })
        .collect();
    assert_eq!(held.len(), 3);
    let mut alice = Replica::seed(1, &bytes).unwrap();
    let mut bob = Replica::join(2, &bytes, &alice.save()).unwrap();
    assert_eq!(alice.export().unwrap(), base.export().unwrap());
    let text = base
        .descendants(root)
        .into_iter()
        .find(|id| matches!(base.node(*id).unwrap().kind, NodeKind::Text { .. }))
        .unwrap();
    alice
        .apply(&Command::InsertText {
            at: Point::Text {
                node: text,
                offset: 1,
            },
            text: "A\nB".into(),
        })
        .unwrap();
    bob.apply(&Command::InsertText {
        at: Point::Text {
            node: text,
            offset: 2,
        },
        text: "🦀".into(),
    })
    .unwrap();
    let a = alice.update_since(&bob.state_vector()).unwrap();
    let b = bob.update_since(&alice.state_vector()).unwrap();
    alice.apply_update(&b).unwrap();
    bob.apply_update(&a).unwrap();
    assert_eq!(alice.export().unwrap(), bob.export().unwrap());
    let doc = alice.view().unwrap();
    let shown = doc.descendants(root);
    for id in &held {
        assert_eq!(shown.iter().filter(|found| *found == id).count(), 1);
    }
    let before = alice.export().unwrap();
    let unknown = held[2];
    let remove = Command::ReplaceRange {
        from: Point::Before { before: unknown },
        to: Point::After { after: unknown },
        text: String::new(),
        track: None,
    };
    alice.apply(&remove).unwrap();
    assert!(!alice.view().unwrap().descendants(root).contains(&unknown));
    bob.apply_update(&alice.update_since(&bob.state_vector()).unwrap())
        .unwrap();
    assert_eq!(alice.export().unwrap(), bob.export().unwrap());
    alice.undo().unwrap();
    assert_eq!(alice.export().unwrap(), before);
    alice.undo().unwrap();
    bob.apply_update(&alice.update_since(&bob.state_vector()).unwrap())
        .unwrap();
    assert_eq!(alice.export().unwrap(), bob.export().unwrap());
    let exported = alice.export().unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(exported)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").unwrap(), &mut xml)
        .unwrap();
    assert!(xml.contains("<x:keep xmlns:x=\"urn:test:preserve\" x:value=\"unchanged\"/>"));
}

#[test]
fn source_drawings_are_kept_when_their_paragraph_is_edited() {
    for name in ["image", "anchored-image", "paragraph-layout"] {
        let (_, bytes) = fixtures().into_iter().find(|(n, _)| n == name).unwrap();
        let mut replica = Replica::seed(1, &bytes).unwrap();
        let doc = replica.view().unwrap();
        let story = doc.stories().next().unwrap().id;
        let nodes = doc.descendants(story);
        let held: Vec<_> = nodes
            .iter()
            .copied()
            .filter(|id| {
                matches!(
                    doc.node(*id).unwrap().kind,
                    NodeKind::Drawing(_) | NodeKind::Tab | NodeKind::Break { .. }
                )
            })
            .collect();
        assert!(!held.is_empty(), "{name}");
        let parent = doc.node(held[0]).unwrap().parent.unwrap();
        replica
            .apply(&Command::InsertText {
                at: Point::Start { start: parent },
                text: "added".into(),
            })
            .unwrap();
        let doc = replica.view().unwrap();
        for id in held {
            assert!(doc.descendants(story).contains(&id), "{name}: {id}");
        }
        assert!(docx_engine::validate_package(&replica.export().unwrap()).valid);
    }
}

#[test]
fn concurrent_deletion_and_undo_restore_an_inline_object_once() {
    let bytes = inline_fixture();
    let mut alice = Replica::seed(1, &bytes).unwrap();
    let mut bob = Replica::join(2, &bytes, &alice.save()).unwrap();
    let doc = alice.view().unwrap();
    let story = doc.stories().next().unwrap().id;
    let tab = doc
        .descendants(story)
        .into_iter()
        .find(|id| matches!(doc.node(*id).unwrap().kind, NodeKind::Tab))
        .unwrap();
    let command = Command::ReplaceRange {
        from: Point::Before { before: tab },
        to: Point::After { after: tab },
        text: String::new(),
        track: None,
    };
    alice.apply(&command).unwrap();
    bob.apply(&command).unwrap();
    let sync = |alice: &mut Replica, bob: &mut Replica| {
        let a = alice.update_since(&bob.state_vector()).unwrap();
        let b = bob.update_since(&alice.state_vector()).unwrap();
        alice.apply_update(&b).unwrap();
        bob.apply_update(&a).unwrap();
    };
    sync(&mut alice, &mut bob);
    assert!(alice.undo().unwrap());
    assert!(bob.undo().unwrap());
    sync(&mut alice, &mut bob);
    assert_eq!(alice.export().unwrap(), bob.export().unwrap());
    let doc = alice.view().unwrap();
    assert_eq!(
        doc.descendants(story)
            .into_iter()
            .filter(|id| matches!(doc.node(*id).unwrap().kind, NodeKind::Tab))
            .count(),
        1
    );
}

#[test]
fn concurrent_undo_of_a_join_restores_one_logical_boundary() {
    let (_, bytes) = fixtures()
        .into_iter()
        .find(|(name, _)| name == "simple-paragraphs")
        .unwrap();
    let mut alice = Replica::seed(1, &bytes).unwrap();
    let mut bob = Replica::join(2, &bytes, &alice.save()).unwrap();
    let original = alice.view().unwrap().paragraphs().len();
    let original_text = shown(alice.view().unwrap());
    let paragraph = alice.view().unwrap().paragraphs()[0];
    let command = Command::MergeParagraphs { paragraph };
    alice.apply(&command).unwrap();
    bob.apply(&command).unwrap();
    let sync = |a: &mut Replica, b: &mut Replica| {
        let to_a = b.update_since(&a.state_vector()).unwrap();
        let to_b = a.update_since(&b.state_vector()).unwrap();
        a.apply_update(&to_a).unwrap();
        b.apply_update(&to_b).unwrap();
    };
    sync(&mut alice, &mut bob);
    alice.undo().unwrap();
    bob.undo().unwrap();
    sync(&mut alice, &mut bob);
    assert_eq!(alice.export().unwrap(), bob.export().unwrap());
    assert_eq!(alice.view().unwrap().paragraphs().len(), original);
    assert_eq!(shown(alice.view().unwrap()), original_text);
}

#[test]
fn tracked_typing_states_each_authors_insertion_and_plain_typing_clears_it() {
    use docx_engine::Track;
    let (_, bytes) = fixtures()
        .into_iter()
        .find(|(name, _)| name == "simple-paragraphs")
        .unwrap();
    let mut alice = Replica::seed(1, &bytes).unwrap();
    let first_text = |doc: &Document, needle: &str| {
        let story = doc.stories().next().unwrap().id;
        doc.descendants(story)
            .into_iter()
            .find(|id| matches!(&doc.node(*id).unwrap().kind, NodeKind::Text { text } if text.contains(needle)))
            .unwrap()
    };
    let node = first_text(alice.view().unwrap(), "First");
    let at = Point::Text { node, offset: 0 };
    let track = |author: &str| Track {
        author: author.into(),
        date: "2026-11-18T00:00:00Z".into(),
    };
    alice
        .apply(&Command::ReplaceRange {
            from: at,
            to: at,
            text: "αβ".into(),
            track: Some(track("Alice")),
        })
        .unwrap();
    let mut bob = Replica::join(2, &bytes, &alice.save()).unwrap();
    let a_node = first_text(alice.view().unwrap(), "αβ");
    let b_node = first_text(bob.view().unwrap(), "αβ");
    alice
        .apply(&Command::InsertText {
            at: Point::Text {
                node: a_node,
                offset: 1,
            },
            text: "δ".into(),
        })
        .unwrap();
    let at = Point::Text {
        node: b_node,
        offset: 1,
    };
    bob.apply(&Command::ReplaceRange {
        from: at,
        to: at,
        text: "γ".into(),
        track: Some(track("Bob")),
    })
    .unwrap();
    let a = alice.update_since(&bob.state_vector()).unwrap();
    let b = bob.update_since(&alice.state_vector()).unwrap();
    alice.apply_update(&b).unwrap();
    bob.apply_update(&a).unwrap();
    assert_eq!(alice.export().unwrap(), bob.export().unwrap());
    for (text, author) in [
        ("α", Some("Alice")),
        ("β", Some("Alice")),
        ("γ", Some("Bob")),
        ("δ", None),
    ] {
        let doc = alice.view().unwrap();
        let mut node = doc.node(first_text(doc, text)).unwrap();
        let mut found = None;
        while let Some(parent) = node.parent {
            node = doc.node(parent).unwrap();
            if let NodeKind::Revision(info) = &node.kind {
                found = info.author.as_deref();
                break;
            }
        }
        assert_eq!(found, author, "{text}");
    }
    bob.undo().unwrap();
    alice
        .apply_update(&bob.update_since(&alice.state_vector()).unwrap())
        .unwrap();
    assert_eq!(alice.export().unwrap(), bob.export().unwrap());
    let doc = alice.view().unwrap();
    assert!(!doc
        .descendants(doc.stories().next().unwrap().id)
        .into_iter()
        .any(|id| doc.text(id).is_some_and(|text| text.contains('γ'))));
}

#[test]
fn tracked_multiline_typing_marks_the_inserted_paragraph_and_undoes_as_one() {
    use docx_engine::Track;
    let (_, bytes) = fixtures()
        .into_iter()
        .find(|(name, _)| name == "simple-paragraphs")
        .unwrap();
    let mut replica = Replica::seed(1, &bytes).unwrap();
    let before = replica.export().unwrap();
    let doc = replica.view().unwrap();
    let first = doc.paragraphs()[0];
    let at = Point::Start { start: first };
    replica
        .apply(&Command::ReplaceRange {
            from: at,
            to: at,
            text: "one\ntwo".into(),
            track: Some(Track {
                author: "Alice".into(),
                date: "2026-11-18T00:00:00Z".into(),
            }),
        })
        .unwrap();
    let doc = replica.view().unwrap();
    let marked: Vec<_> = doc
        .paragraphs()
        .into_iter()
        .filter_map(|id| match &doc.node(id).unwrap().kind {
            NodeKind::Paragraph(info) if info.mark_revision.as_deref() == Some("ins") => Some(info),
            _ => None,
        })
        .collect();
    assert_eq!(marked.len(), 1);
    assert_eq!(
        marked[0].mark_change.as_ref().unwrap().author.as_deref(),
        Some("Alice")
    );
    // DOCX stores a paragraph boundary on the paragraph it ends; the
    // sequence stores the boundary before the paragraph it starts.
    let marked_texts: Vec<_> = doc.paragraphs().into_iter().filter(|id| {
        matches!(&doc.node(*id).unwrap().kind, NodeKind::Paragraph(info) if info.mark_revision.as_deref() == Some("ins"))
    }).map(|id| doc.text(id).unwrap()).collect();
    assert_eq!(marked_texts, ["one"]);
    let before_refusal = replica.save();
    let paragraph = replica.view().unwrap().paragraphs()[0];
    assert!(replica
        .apply(&Command::MergeParagraphs { paragraph })
        .is_err());
    assert_eq!(replica.save(), before_refusal);
    // Splitting before an inserted boundary moves Word's revision to the
    // new tail without changing its shared identity or author.
    replica
        .apply(&Command::SplitParagraph {
            at: Point::Start { start: paragraph },
        })
        .unwrap();
    let doc = replica.view().unwrap();
    let marked_texts: Vec<_> = doc.paragraphs().into_iter().filter(|id| {
        matches!(&doc.node(*id).unwrap().kind, NodeKind::Paragraph(info) if info.mark_revision.as_deref() == Some("ins"))
    }).map(|id| doc.text(id).unwrap()).collect();
    assert_eq!(marked_texts, ["one"]);
    replica.undo().unwrap();
    assert!(docx_engine::validate_package(&replica.export().unwrap()).valid);
    replica.undo().unwrap();
    assert_eq!(replica.export().unwrap(), before);
}

#[test]
fn concurrent_undo_restores_a_created_boundary_once() {
    let (_, bytes) = fixtures()
        .into_iter()
        .find(|(name, _)| name == "simple-paragraphs")
        .unwrap();
    let mut alice = Replica::seed(1, &bytes).unwrap();
    let first = alice.view().unwrap().paragraphs()[0];
    alice
        .apply(&Command::SplitParagraph {
            at: Point::End { end: first },
        })
        .unwrap();
    let mut bob = Replica::join(2, &bytes, &alice.save()).unwrap();
    let original = alice.view().unwrap().paragraphs().len();
    let original_text = shown(alice.view().unwrap());
    let paragraph = alice.view().unwrap().paragraphs()[0];
    let command = Command::MergeParagraphs { paragraph };
    alice.apply(&command).unwrap();
    bob.apply(&command).unwrap();
    let sync = |a: &mut Replica, b: &mut Replica| {
        let to_a = b.update_since(&a.state_vector()).unwrap();
        let to_b = a.update_since(&b.state_vector()).unwrap();
        a.apply_update(&to_a).unwrap();
        b.apply_update(&to_b).unwrap();
    };
    sync(&mut alice, &mut bob);
    alice.undo().unwrap();
    bob.undo().unwrap();
    sync(&mut alice, &mut bob);
    assert_eq!(alice.export().unwrap(), bob.export().unwrap());
    assert_eq!(alice.view().unwrap().paragraphs().len(), original);
    assert_eq!(shown(alice.view().unwrap()), original_text);
}

#[test]
fn relative_carets_follow_unicode_content_and_reject_other_documents() {
    let (_, bytes) = fixtures()
        .into_iter()
        .find(|(name, _)| name == "simple-paragraphs")
        .unwrap();
    let base = Document::open(&bytes).unwrap();
    let mut alice = Replica::seed(1, &bytes).unwrap();
    let first = alice.view().unwrap().paragraphs()[0];
    alice
        .apply(&Command::InsertText {
            at: Point::Start { start: first },
            text: "🦀\n🙂".into(),
        })
        .unwrap();
    let doc = alice.view().unwrap();
    let story = doc.stories().next().unwrap().id;
    let mut points = Vec::new();
    for paragraph in doc.paragraphs() {
        points.extend([
            Point::Start { start: paragraph },
            Point::End { end: paragraph },
        ]);
    }
    for node in doc.descendants(story) {
        if let NodeKind::Text { text } = &doc.node(node).unwrap().kind {
            points.extend((0..=text.chars().count()).map(|offset| Point::Text { node, offset }));
        }
    }
    let store = Store::load(99, &alice.save()).unwrap();
    let view = build(&base, &store).unwrap();
    for point in points {
        let (key, token) = alice
            .sticky(&point)
            .unwrap()
            .unwrap_or_else(|| panic!("no caret for {point:?}"));
        let (_, resolved) = alice.resolve(&key, &token).unwrap().unwrap();
        assert_eq!(
            view.position(&point),
            view.position(&resolved),
            "{point:?} => {resolved:?}"
        );
    }
    let doc = alice.view().unwrap();
    let node = doc.descendants(story).into_iter().find(|id| matches!(&doc.node(*id).unwrap().kind, NodeKind::Text { text } if text.contains("First"))).unwrap();
    let at = Point::Text { node, offset: 2 };
    let before = view.position(&at).unwrap();
    let (key, token) = alice.sticky(&at).unwrap().unwrap();
    let mut bob = Replica::join(2, &bytes, &alice.save()).unwrap();
    bob.apply(&Command::InsertText {
        at: Point::Text { node, offset: 0 },
        text: "remote".into(),
    })
    .unwrap();
    alice
        .apply_update(&bob.update_since(&alice.state_vector()).unwrap())
        .unwrap();
    let (_, moved) = alice.resolve(&key, &token).unwrap().unwrap();
    let view = build(&base, &Store::load(99, &alice.save()).unwrap()).unwrap();
    assert_eq!(view.position(&moved).unwrap(), before + 6);
    let mut reopened = Replica::join(3, &bytes, &alice.save()).unwrap();
    assert_eq!(
        reopened.resolve(&key, &token).unwrap(),
        alice.resolve(&key, &token).unwrap()
    );
    let (_, other) = fixtures()
        .into_iter()
        .find(|(name, _)| name == "rich-text")
        .unwrap();
    assert!(Replica::seed(1, &other)
        .unwrap()
        .resolve(&key, &token)
        .unwrap()
        .is_none());
    assert!(alice.resolve(&key, &[]).unwrap().is_none());
}

#[test]
fn replacing_a_range_checks_note_and_bookmark_ownership_on_both_stores() {
    for (body, accepted) in [
        (r#"<w:r><w:footnoteReference w:id="1"/></w:r>"#, false),
        (r#"<w:bookmarkStart w:id="1" w:name="ref"/>"#, false),
        (
            r#"<w:bookmarkStart w:id="1" w:name="ref"/><w:bookmarkEnd w:id="1"/>"#,
            true,
        ),
        (
            r#"<w:proofErr w:type="spellStart"/><w:proofErr w:type="spellEnd"/>"#,
            true,
        ),
    ] {
        let extra = if body.contains("bookmarkStart") && !accepted {
            r#"<w:p><w:bookmarkEnd w:id="1"/></w:p>"#
        } else {
            ""
        };
        let bytes = common::with_body(&format!(
            "<w:p><w:r><w:t>Left</w:t></w:r>{body}<w:r><w:t>Right</w:t></w:r></w:p>{extra}"
        ));
        let mut editor = docx_engine::editing::Editor::open(&bytes).unwrap();
        let mut replica = Replica::seed(1, &bytes).unwrap();
        let doc = editor.view().unwrap();
        let replace = Command::ReplaceRange {
            from: Point::Text {
                node: common::text_node(doc, "Left"),
                offset: 1,
            },
            to: Point::Text {
                node: common::text_node(doc, "Right"),
                offset: 4,
            },
            text: "X".into(),
            track: None,
        };
        let before = replica.save();
        assert_eq!(editor.apply(&replace).is_ok(), accepted, "{body}");
        assert_eq!(replica.apply(&replace).is_ok(), accepted, "{body}");
        assert_eq!(replica.export().unwrap(), editor.export().unwrap());
        if accepted {
            let doc = editor.view().unwrap();
            assert_eq!(doc.text(doc.paragraphs()[0]).as_deref(), Some("LXt"));
        } else {
            assert_eq!(replica.save(), before);
            assert!(!editor.undo());
            assert!(!replica.undo().unwrap());
        }
    }
}

#[test]
fn live_updates_send_only_the_current_transaction_after_many_deletions() {
    let base = common::generated("simple-paragraphs");
    let mut author = Replica::seed(100, &base).unwrap();
    let mut peer = Replica::join(200, &base, &author.save()).unwrap();
    let mut lengths = Vec::new();
    for i in 0..305 {
        let leaf = common::text_node(author.view().unwrap(), "paragraph.");
        author
            .apply(&Command::ReplaceText {
                node: leaf,
                range: Some(TextRange { start: 0, end: 1 }),
                text: if i % 2 == 0 { "x" } else { "y" }.into(),
            })
            .unwrap();
        let packet = author.take_update().unwrap().unwrap();
        lengths.push(packet.len());
        peer.apply_update(&packet).unwrap();
        assert!(peer.take_update().unwrap().is_none());
    }
    assert!(
        lengths[304] <= lengths[0] + 32,
        "{:?}",
        [lengths[0], lengths[304]]
    );
    assert_eq!(author.export().unwrap(), peer.export().unwrap());
}

#[test]
fn tracking_a_range_keeps_its_bookmark_without_an_invisible_undo_step() {
    for inside in ["", "<w:r><w:t>word</w:t></w:r>"] {
        let bytes = common::with_body(&format!(
            r#"<w:p><w:bookmarkStart w:id="0" w:name="_GoBack"/>{inside}<w:bookmarkEnd w:id="0"/></w:p>"#
        ));
        let mut replica = Replica::seed(10, &bytes).unwrap();
        let doc = replica.view().unwrap();
        let children = &doc.node(doc.paragraphs()[0]).unwrap().children;
        let cmd = Command::ReplaceRange {
            from: Point::Before {
                before: children[0],
            },
            to: Point::After {
                after: *children.last().unwrap(),
            },
            text: String::new(),
            track: Some(docx_engine::Track {
                author: "Ada".into(),
                date: "2026-11-20T10:00:00Z".into(),
            }),
        };
        let before = replica.export().unwrap();
        replica.apply(&cmd).unwrap();
        let out = replica.export().unwrap();
        let xml = common::part(&out, "word/document.xml");
        assert!(xml.contains("w:bookmarkStart") && xml.contains("w:bookmarkEnd"));
        assert_eq!(xml.contains("<w:delText"), !inside.is_empty());
        assert_eq!(replica.take_update().unwrap().is_some(), !inside.is_empty());
        assert_eq!(replica.undo().unwrap(), !inside.is_empty());
        assert_eq!(replica.export().unwrap(), before);
    }
}
