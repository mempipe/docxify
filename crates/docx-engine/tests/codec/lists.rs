use super::common;

use common::*;
use docx_engine::{Command, Document, ListKind, NodeId, NodeKind, NumRef, Point, Support};

const DOCUMENT: &str = "word/document.xml";
const NUMBERING: &str = "word/numbering.xml";

fn set_list(from: Point, to: Point, kind: Option<ListKind>) -> Command {
    Command::SetList { from, to, kind }
}

fn change_level(from: Point, to: Point, by: i8) -> Command {
    Command::ChangeListLevel { from, to, by }
}

/// A list as the inspection shows it: number, level and marker format.
type Shown = Option<(String, u64, String)>;

/// Each paragraph's text with the list it shows.
fn lists(doc: &Document) -> Vec<(String, Shown)> {
    fn walk(node: &serde_json::Value, out: &mut Vec<(String, Shown)>) {
        if node["type"] == "paragraph" {
            let list = node.get("list").filter(|l| !l.is_null()).map(|l| {
                (
                    l["num_id"].as_str().unwrap().to_string(),
                    l["level"].as_u64().unwrap(),
                    l["definition"]["format"].as_str().unwrap_or("").to_string(),
                )
            });
            out.push((node["paragraph_text"].as_str().unwrap().to_string(), list));
        }
        for child in node["children"].as_array().into_iter().flatten() {
            walk(child, out);
        }
    }
    let mut out = Vec::new();
    for story in stories(doc).as_array().unwrap() {
        walk(story, &mut out);
    }
    out
}

fn list_of(doc: &Document, needle: &str) -> Shown {
    lists(doc)
        .into_iter()
        .find(|(text, _)| text.contains(needle))
        .unwrap_or_else(|| panic!("no paragraph containing {needle:?}"))
        .1
}

fn shown(num_id: &str, level: u64, format: &str) -> Shown {
    Some((num_id.to_string(), level, format.to_string()))
}

/// The `lists` fixture, whose numbering defines a bullet list (1) and a
/// numbered one (2), each with levels 0 and 1, with its body replaced.
fn lists_with_body(body: &str) -> Vec<u8> {
    let bytes = generated("lists");
    let xml = part(&bytes, DOCUMENT);
    let (start, end) = (
        xml.find("<w:body>").unwrap() + 8,
        xml.find("<w:sectPr").unwrap(),
    );
    let xml = format!("{}{body}{}", &xml[..start], &xml[end..]);
    rewrite(&bytes, DOCUMENT, Some(xml.as_bytes()))
}

fn item(text: &str, num_id: u32, level: u8) -> String {
    format!(
        r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="{level}"/><w:numId w:val="{num_id}"/></w:numPr></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#
    )
}

fn plain(text: &str) -> String {
    format!(r#"<w:p><w:r><w:t>{text}</w:t></w:r></w:p>"#)
}

#[test]
fn bullets_in_a_document_without_lists_add_a_list_and_its_numbering_part() {
    let bytes = generated("simple-paragraphs");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let (first, second) = (text_node(&doc, "First"), text_node(&doc, "Second"));
    edit(
        &mut doc,
        set_list(at(first, 3), at(second, 3), Some(ListKind::Bullet)),
    );
    // Its List Bullet style names list 1, which the document never defines.
    assert_eq!(list_of(&doc, "First"), shown("2", 0, "bullet"));
    assert_eq!(list_of(&doc, "Second"), shown("2", 0, "bullet"));
    assert_eq!(list_of(&doc, "Third"), None);
    assert_eq!(
        doc.added_lists().collect::<Vec<_>>(),
        [("2", ListKind::Bullet)]
    );

    let out = doc.export().unwrap();
    let xml = part(&out, DOCUMENT);
    assert!(
        xml.contains(r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="2"/></w:numPr></w:pPr><w:r><w:t>First"#),
        "{xml}"
    );
    let numbering = part(&out, NUMBERING);
    assert!(
        numbering.contains(r#"<w:abstractNum w:abstractNumId="0">"#),
        "{numbering}"
    );
    assert!(
        numbering.contains(r#"<w:num w:numId="2"><w:abstractNumId w:val="0"/></w:num>"#),
        "{numbering}"
    );
    assert!(part(&out, "[Content_Types].xml").contains(r#"<Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/>"#));
    let rels = part(&out, "word/_rels/document.xml.rels");
    assert!(
        rels.contains(r#"Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/>"#),
        "{rels}"
    );
    assert_eq!(
        doc.dirty_parts(),
        [
            DOCUMENT,
            "[Content_Types].xml",
            "word/_rels/document.xml.rels",
            NUMBERING
        ]
    );

    let reopened = reopen("simple-paragraphs", &doc);
    assert_eq!(lists(&reopened), lists(&doc));
    let level = reopened
        .numbering()
        .level(&NumRef {
            num_id: "2".into(),
            level: 1,
        })
        .unwrap();
    assert_eq!(
        (level.format.as_str(), level.text.as_str()),
        ("bullet", "\u{25E6}")
    );
    assert_editor_undo("simple-paragraphs", &bytes, &mut doc);
}

#[test]
fn a_list_next_to_the_range_is_continued_and_the_numbering_part_is_left_alone() {
    let bytes = lists_with_body(&format!(
        "{}{}{}",
        item("One", 2, 0),
        plain("Two"),
        plain("Three")
    ));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let (two, three) = (text_node(&doc, "Two"), text_node(&doc, "Three"));
    edit(
        &mut doc,
        set_list(at(two, 0), at(three, 0), Some(ListKind::Number)),
    );
    for needle in ["One", "Two", "Three"] {
        assert_eq!(list_of(&doc, needle), shown("2", 0, "decimal"), "{needle}");
    }
    assert_eq!(doc.added_lists().count(), 0);
    assert_eq!(doc.dirty_parts(), [DOCUMENT]);
    assert_editor_undo("lists", &bytes, &mut doc);

    // The paragraph after the range counts too.
    let bytes = lists_with_body(&format!("{}{}", plain("Zero"), item("One", 1, 0)));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let zero = text_node(&doc, "Zero");
    edit(
        &mut doc,
        set_list(at(zero, 0), at(zero, 0), Some(ListKind::Bullet)),
    );
    assert_eq!(list_of(&doc, "Zero"), shown("1", 0, "bullet"));
}

#[test]
fn a_list_in_the_range_is_continued_and_levels_are_kept() {
    let bytes = lists_with_body(&format!(
        "{}{}{}{}",
        plain("Before"),
        item("Nested", 1, 1),
        item("Numbered", 2, 1),
        plain("After")
    ));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let (before, after) = (text_node(&doc, "Before"), text_node(&doc, "After"));
    edit(
        &mut doc,
        set_list(at(before, 0), at(after, 0), Some(ListKind::Bullet)),
    );
    assert_eq!(list_of(&doc, "Before"), shown("1", 0, "bullet"));
    assert_eq!(list_of(&doc, "Nested"), shown("1", 1, "bullet"));
    assert_eq!(list_of(&doc, "Numbered"), shown("1", 1, "bullet"));
    assert_eq!(list_of(&doc, "After"), shown("1", 0, "bullet"));
    // "Nested" was in that list at that level already.
    assert_editor_undo("lists", &bytes, &mut doc);
}

#[test]
fn a_new_list_follows_the_existing_definitions_and_keeps_levels() {
    let bytes = lists_with_body(&format!("{}{}", item("One", 1, 0), item("Two", 1, 1)));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let (one, two) = (text_node(&doc, "One"), text_node(&doc, "Two"));
    edit(
        &mut doc,
        set_list(at(one, 0), at(two, 0), Some(ListKind::Number)),
    );
    assert_eq!(list_of(&doc, "One"), shown("3", 0, "decimal"));
    assert_eq!(list_of(&doc, "Two"), shown("3", 1, "lowerLetter"));
    let numbering = part(&doc.export().unwrap(), NUMBERING);
    // After the last definition and the last instance, as the schema orders them.
    assert!(
        numbering
            .contains(r#"</w:abstractNum><w:abstractNum w:abstractNumId="21"><w:multiLevelType"#),
        "{numbering}"
    );
    assert!(
        numbering.ends_with(r#"<w:num w:numId="2"><w:abstractNumId w:val="20"/></w:num><w:num w:numId="3"><w:abstractNumId w:val="21"/></w:num></w:numbering>"#),
        "{numbering}"
    );
    assert_eq!(doc.dirty_parts(), [DOCUMENT, NUMBERING]);
    let reopened = reopen("lists", &doc);
    assert_eq!(lists(&reopened), lists(&doc));
    assert_editor_undo("lists", &bytes, &mut doc);
}

#[test]
fn taking_paragraphs_out_of_lists_removes_their_numbering_or_turns_off_a_styles() {
    let bytes = generated("lists");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let (bullet, removed) = (
        text_node(&doc, "Bullet two"),
        text_node(&doc, "Style bullet removed"),
    );
    edit(&mut doc, set_list(at(bullet, 0), at(removed, 0), None));
    for needle in [
        "Bullet two",
        "Step one",
        "Sub-step a",
        "Bullet from the style",
        "Style bullet removed",
    ] {
        assert_eq!(list_of(&doc, needle), None, "{needle}");
    }
    // The last one had no list to take away.
    let xml = part(&doc.export().unwrap(), DOCUMENT);
    for expected in [
        "<w:p><w:r><w:t>Bullet two</w:t>",
        r#"<w:p><w:pPr><w:pStyle w:val="ListBullet"/><w:numPr><w:ilvl w:val="0"/><w:numId w:val="0"/></w:numPr></w:pPr><w:r><w:t>Bullet from the style"#,
    ] {
        assert!(xml.contains(expected), "{expected} in {xml}");
    }
    let reopened = reopen("lists", &doc);
    assert_eq!(lists(&reopened), lists(&doc));
    assert_editor_undo("lists", &bytes, &mut doc);
}

#[test]
fn a_style_bullet_turned_off_and_on_again_continues_the_list_next_to_it() {
    let bytes = generated("lists");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let removed = text_node(&doc, "Style bullet removed");
    edit(
        &mut doc,
        set_list(at(removed, 0), at(removed, 0), Some(ListKind::Bullet)),
    );
    let style_list = list_of(&doc, "Bullet from the style").unwrap();
    assert_eq!(list_of(&doc, "Style bullet removed"), Some(style_list));
}

#[test]
fn list_levels_move_within_the_levels_the_list_defines() {
    let bytes = lists_with_body(&format!(
        "{}{}{}",
        item("One", 1, 0),
        plain("Plain"),
        item("Two", 1, 1)
    ));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let (one, two) = (text_node(&doc, "One"), text_node(&doc, "Two"));
    edit(&mut doc, change_level(at(one, 0), at(two, 0), 1));
    assert_eq!(list_of(&doc, "One"), shown("1", 1, "bullet"));
    assert_eq!(list_of(&doc, "Plain"), None);
    // The list defines levels 0 and 1 only.
    assert_eq!(list_of(&doc, "Two"), shown("1", 1, "bullet"));
    assert_editor_undo("lists", &bytes, &mut doc);

    let mut doc = EditingDocument::open(&bytes).unwrap();
    for (needle, by) in [("One", -1), ("Two", 1), ("Plain", 1)] {
        let text = text_node(&doc, needle);
        edit(&mut doc, change_level(at(text, 0), at(text, 0), by));
    }
    assert!(!doc.is_dirty());

    // A list the engine adds has all nine.
    let mut doc = EditingDocument::open(&generated("simple-paragraphs")).unwrap();
    let first = text_node(&doc, "First");
    edit(
        &mut doc,
        set_list(at(first, 0), at(first, 0), Some(ListKind::Number)),
    );
    edit(&mut doc, change_level(at(first, 0), at(first, 0), 20));
    assert_eq!(list_of(&doc, "First"), shown("2", 8, "lowerRoman"));
    edit(&mut doc, change_level(at(first, 0), at(first, 0), -3));
    assert_eq!(list_of(&doc, "First"), shown("2", 5, "lowerRoman"));
}

#[test]
fn a_paragraph_split_in_a_list_continues_it() {
    let bytes = generated("simple-paragraphs");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let first = text_node(&doc, "First");
    edit(
        &mut doc,
        set_list(at(first, 0), at(first, 0), Some(ListKind::Number)),
    );
    edit(&mut doc, change_level(at(first, 0), at(first, 0), 1));
    edit(&mut doc, Command::SplitParagraph { at: at(first, 5) });
    let reopened = reopen("simple-paragraphs", &doc);
    assert_eq!(list_of(&reopened, "First"), shown("2", 1, "lowerLetter"));
    assert_eq!(
        list_of(&reopened, " paragraph."),
        shown("2", 1, "lowerLetter")
    );
}

#[test]
fn set_numbering_gives_exactly_what_it_names() {
    let bytes = lists_with_body(&format!("{}{}", plain("Plain"), item("One", 1, 0)));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let (plain, one) = (text_node(&doc, "Plain"), text_node(&doc, "One"));
    let numbering = Some(NumRef {
        num_id: "2".into(),
        level: 1,
    });
    edit(
        &mut doc,
        Command::SetNumbering {
            from: at(plain, 0),
            to: at(one, 0),
            numbering,
        },
    );
    assert_eq!(list_of(&doc, "Plain"), shown("2", 1, "lowerLetter"));
    assert_eq!(list_of(&doc, "One"), shown("2", 1, "lowerLetter"));
    assert_editor_undo("lists", &bytes, &mut doc);

    // Undone and redone, so both are in list 2.
    let none = Command::SetNumbering {
        from: at(plain, 0),
        to: at(one, 0),
        numbering: None,
    };
    edit(&mut doc, none.clone());
    assert_eq!(list_of(&doc, "Plain"), None);
    assert_eq!(list_of(&doc, "One"), None);
    edit(&mut doc, none);
    assert_eq!(list_of(&doc, "One"), None);
}

/// A list number the document names but does not define, as a style or a
/// paragraph can, is one `SetNumbering` writes: a level change or a split
/// can leave it on a paragraph, and the shared replica's replay writes the
/// same with `SetNumbering`.
#[test]
fn set_numbering_takes_a_number_the_document_names_but_does_not_define() {
    // The List Bullet style names list 1, which the document does not define.
    let bytes = generated("simple-paragraphs");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let first = text_node(&doc, "First paragraph.");
    let one = |level| Command::SetNumbering {
        from: at(first, 0),
        to: at(first, 0),
        numbering: Some(NumRef {
            num_id: "1".into(),
            level,
        }),
    };
    edit(&mut doc, one(1));
    assert_editor_undo("simple-paragraphs", &bytes, &mut doc);

    // A paragraph names list 7, which nothing defines.
    let bytes = with_body(&format!("{}{}", item("Seven", 7, 0), plain("Plain")));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let plain = text_node(&doc, "Plain");
    edit(
        &mut doc,
        Command::SetNumbering {
            from: at(plain, 0),
            to: at(plain, 0),
            numbering: Some(NumRef {
                num_id: "7".into(),
                level: 2,
            }),
        },
    );
    let out = doc.export().unwrap();
    assert_valid("seven", &out);
    assert!(part(&out, DOCUMENT).contains(r#"<w:ilvl w:val="2"/><w:numId w:val="7"/>"#));
}

#[test]
fn list_commands_refuse_and_change_nothing() {
    fn refused(name: &str, doc: &mut EditingDocument, command: Command, reason: &str) {
        let before = stories(doc);
        let added = doc.added_lists().count();
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
            doc.added_lists().count(),
            added,
            "{name}: {command:?} added a list"
        );
        assert!(!doc.is_dirty(), "{name}: {command:?} dirtied the document");
    }
    let mut doc = EditingDocument::open(&generated("lists")).unwrap();
    let (one, two) = (text_node(&doc, "Bullet one"), text_node(&doc, "Bullet two"));
    refused(
        "lists",
        &mut doc,
        set_list(at(two, 0), at(one, 0), Some(ListKind::Bullet)),
        "ends before it starts",
    );
    let numbering = |num_id: &str, level| {
        Some(NumRef {
            num_id: num_id.into(),
            level,
        })
    };
    for (numbering, reason) in [
        (numbering("9", 0), "no list with that number"),
        (numbering("1", 9), "from 0 to 8"),
    ] {
        refused(
            "lists",
            &mut doc,
            Command::SetNumbering {
                from: at(one, 0),
                to: at(two, 0),
                numbering,
            },
            reason,
        );
    }

    let mut doc = EditingDocument::open(&generated("header-footer")).unwrap();
    let roots: Vec<NodeId> = doc.stories().map(|s| s.id).collect();
    let first_in = |doc: &Document, root: NodeId| {
        doc.descendants(root).into_iter().find(|&id| {
            matches!(&doc.node(id).unwrap().kind, NodeKind::Text { text } if !text.is_empty())
        })
    };
    let body = first_in(&doc, roots[0]).unwrap();
    let header = roots[1..].iter().find_map(|&r| first_in(&doc, r)).unwrap();
    refused(
        "header-footer",
        &mut doc,
        set_list(at(body, 0), at(header, 0), Some(ListKind::Number)),
        "another story",
    );
}

#[test]
fn an_added_list_is_added_once_and_never_over_another() {
    let mut doc = Document::open(&generated("lists")).unwrap();
    doc.add_list("7", ListKind::Bullet).unwrap();
    doc.add_list("7", ListKind::Bullet).unwrap();
    assert_eq!(
        doc.added_lists().collect::<Vec<_>>(),
        [("7", ListKind::Bullet)]
    );
    for (num_id, reason) in [
        ("7", "another kind"),
        ("1", "already a list in the document"),
        ("0", "not a positive number"),
        ("x", "not a positive number"),
    ] {
        let error = doc.add_list(num_id, ListKind::Number).expect_err(num_id);
        assert!(error.to_string().contains(reason), "{num_id}: {error}");
    }
}

#[test]
fn definitions_go_inside_a_numbering_part_whatever_its_shape() {
    for (numbering, label) in [
        (
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:numbering xmlns:w="{W}"/>"#
            ),
            "self-closing",
        ),
        (
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><numbering xmlns="{W}"></numbering>"#
            ),
            "unprefixed",
        ),
        (
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:numbering xmlns:w="{W}"><w:numIdMacAtCleanup w:val="1"/></w:numbering>"#
            ),
            "with a cleanup mark",
        ),
    ] {
        let bytes = rewrite(&generated("lists"), NUMBERING, Some(numbering.as_bytes()));
        let mut doc = EditingDocument::open(&bytes).unwrap();
        let one = text_node(&doc, "Bullet one");
        edit(
            &mut doc,
            set_list(at(one, 0), at(one, 0), Some(ListKind::Number)),
        );
        let reopened = reopen(label, &doc);
        // Its paragraphs name lists 1 and 2, which this part leaves undefined.
        assert_eq!(
            list_of(&reopened, "Bullet one"),
            shown("3", 0, "decimal"),
            "{label}"
        );
        assert_editor_undo(label, &bytes, &mut doc);
    }
}

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

#[test]
fn every_fixture_bullets_ranges_undoes_and_redoes_exactly() {
    let mut listed = 0;
    for (name, bytes) in all_fixtures() {
        let doc = Document::open(&bytes).unwrap();
        let first_text = |doc: &Document, p: NodeId| {
            doc.descendants(p).into_iter().find(|&id| {
                let node = doc.node(id).unwrap();
                node.support == Support::Editable
                    && matches!(&node.kind, NodeKind::Text { text } if !text.is_empty())
            })
        };
        let paragraphs = doc.paragraphs();
        for triple in paragraphs.windows(3) {
            let (Some(from), Some(to)) = (first_text(&doc, triple[0]), first_text(&doc, triple[2]))
            else {
                continue;
            };
            let mut doc = EditingDocument::open(&bytes).unwrap();
            let Ok(()) = doc
                .0
                .apply(&set_list(at(from, 0), at(to, 0), Some(ListKind::Bullet)))
            else {
                assert_eq!(
                    stories(&doc),
                    stories(&Document::open(&bytes).unwrap()),
                    "{name}: refusal changed the document"
                );
                assert!(!doc.is_dirty(), "{name}: refusal dirtied the document");
                continue;
            };
            if lists(&doc) == lists(&Document::open(&bytes).unwrap()) {
                continue;
            }
            listed += 1;
            let reopened = reopen(&name, &doc);
            assert_eq!(lists(&reopened), lists(&doc), "{name}");
            let text = |d: &Document| -> Vec<String> {
                triple.iter().map(|&p| d.text(p).unwrap()).collect()
            };
            assert_eq!(
                text(&doc),
                text(&Document::open(&bytes).unwrap()),
                "{name}: the text changed"
            );
            assert_editor_undo(&name, &bytes, &mut doc);
        }
    }
    assert!(listed > 50, "only {listed} ranges listed");
}

#[test]
fn list_commands_round_trip_through_json_and_are_listed() {
    let n = |s: &str| s.parse::<NodeId>().unwrap();
    let from = Point::Text {
        node: n("n3"),
        offset: 2,
    };
    let to = Point::End { end: n("n5") };
    for (command, json) in [
        (
            set_list(from, to, Some(ListKind::Bullet)),
            r#"{"command":"set_list","from":{"node":"n3","offset":2},"to":{"end":"n5"},"kind":"bullet"}"#,
        ),
        (
            set_list(from, to, None),
            r#"{"command":"set_list","from":{"node":"n3","offset":2},"to":{"end":"n5"},"kind":null}"#,
        ),
        (
            change_level(from, to, -1),
            r#"{"command":"change_list_level","from":{"node":"n3","offset":2},"to":{"end":"n5"},"by":-1}"#,
        ),
        (
            Command::SetNumbering {
                from,
                to,
                numbering: Some(NumRef {
                    num_id: "4".into(),
                    level: 2,
                }),
            },
            r#"{"command":"set_numbering","from":{"node":"n3","offset":2},"to":{"end":"n5"},"numbering":{"num_id":"4","level":2}}"#,
        ),
    ] {
        assert_eq!(serde_json::to_string(&command).unwrap(), json);
        assert_eq!(serde_json::from_str::<Command>(json).unwrap(), command);
    }
    let doc = Document::open(&generated("simple-paragraphs")).unwrap();
    let listed = doc.inspect_json()["commands"].clone();
    for name in ["set_list", "change_list_level", "set_numbering"] {
        assert!(
            listed.as_array().unwrap().iter().any(|c| c == name),
            "{name} is not in {listed}"
        );
    }
}
