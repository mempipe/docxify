use super::common;

use common::*;
use docx_engine::{Command, Document, ListKind, NodeId, NodeKind, Point, Support};

const DOCUMENT: &str = "word/document.xml";
const RELS: &str = "word/_rels/document.xml.rels";
const STYLES: &str = "word/styles.xml";
const HYPERLINK: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";

fn set_link(from: Point, to: Point, target: Option<&str>) -> Command {
    Command::SetLink {
        from,
        to,
        target: target.map(str::to_string),
    }
}

/// The relationship ID the engine gives a link to `target`.
fn link_id(target: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in target.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("rIdL{hash:016x}")
}

/// Each link in the document as the inspection shows it: its text and where
/// it points, with `#` before an anchor.
fn links(doc: &Document) -> Vec<(String, String)> {
    fn text(node: &serde_json::Value) -> String {
        let own = node["text"].as_str().filter(|_| node["type"] == "text");
        let inner: String = node["children"]
            .as_array()
            .into_iter()
            .flatten()
            .map(text)
            .collect();
        format!("{}{inner}", own.unwrap_or(""))
    }
    fn walk(node: &serde_json::Value, out: &mut Vec<(String, String)>) {
        if node["type"] == "hyperlink" {
            let target = node["target"].as_str().unwrap_or("");
            let href = match node["anchor"].as_str() {
                Some(anchor) => format!("{target}#{anchor}"),
                None => target.to_string(),
            };
            out.push((text(node), href));
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

/// The text nodes of the paragraph holding `needle`.
fn texts_of(doc: &Document, needle: &str) -> Vec<String> {
    let paragraph = paragraph_with(doc, needle);
    doc.descendants(paragraph)
        .into_iter()
        .filter_map(|id| match &doc.node(id).unwrap().kind {
            NodeKind::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn pair(text: &str, href: &str) -> (String, String) {
    (text.to_string(), href.to_string())
}

/// The character style and resolved colour and underline of the run
/// holding the text node whose text is `exact`.
fn run_look(doc: &Document, exact: &str) -> (Option<String>, Option<String>, Option<String>) {
    let text = doc
        .paragraphs()
        .into_iter()
        .flat_map(|p| doc.descendants(p))
        .find(|&id| matches!(&doc.node(id).unwrap().kind, NodeKind::Text { text } if text == exact))
        .unwrap_or_else(|| panic!("no text node {exact:?}"));
    let run = doc.node(text).unwrap().parent.unwrap();
    let NodeKind::Run(info) = &doc.node(run).unwrap().kind else {
        panic!("{exact:?} is not in a run");
    };
    let props = doc.effective_run_props(run);
    (info.style.clone(), props.color, props.underline)
}

#[test]
fn a_link_wraps_part_of_a_paragraph_and_adds_its_relationship_and_style() {
    let bytes = generated("simple-paragraphs");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let first = text_node(&doc, "First");
    edit(
        &mut doc,
        set_link(
            at(first, 6),
            at(first, 15),
            Some("https://example.com/a?b=1&c=2"),
        ),
    );
    assert_eq!(
        links(&doc),
        [pair("paragraph", "https://example.com/a?b=1&c=2")]
    );
    assert_eq!(texts(&doc)[0], "First paragraph.");
    assert_eq!(
        run_look(&doc, "paragraph"),
        (
            Some("Hyperlink".into()),
            Some("0563C1".into()),
            Some("single".into())
        )
    );

    let out = doc.export().unwrap();
    let id = link_id("https://example.com/a?b=1&c=2");
    let xml = part(&out, DOCUMENT);
    assert!(
        xml.contains(&format!(
            r#"<w:r><w:t xml:space="preserve">First </w:t></w:r><w:hyperlink r:id="{id}"><w:r><w:rPr><w:rStyle w:val="Hyperlink"/></w:rPr><w:t xml:space="preserve">paragraph</w:t></w:r></w:hyperlink><w:r><w:t xml:space="preserve">.</w:t></w:r>"#
        )),
        "{xml}"
    );
    let rels = part(&out, RELS);
    assert!(
        rels.contains(&format!(
            r#"<Relationship Id="{id}" Type="{HYPERLINK}" Target="https://example.com/a?b=1&amp;c=2" TargetMode="External"/>"#
        )),
        "{rels}"
    );
    let styles = part(&out, STYLES);
    assert!(
        styles.contains(r#"<w:style w:type="character" w:styleId="Hyperlink"><w:name w:val="Hyperlink"/><w:uiPriority w:val="99"/><w:unhideWhenUsed/><w:rPr><w:color w:val="0563C1" w:themeColor="hyperlink"/><w:u w:val="single"/></w:rPr></w:style></w:styles>"#),
        "{styles}"
    );
    assert_eq!(doc.dirty_parts(), [DOCUMENT, RELS, STYLES]);

    let reopened = reopen("simple-paragraphs", &doc);
    assert_eq!(links(&reopened), links(&doc));
    assert_eq!(
        run_look(&reopened, "paragraph"),
        run_look(&doc, "paragraph")
    );
    assert_editor_undo("simple-paragraphs", &bytes, &mut doc);
}

#[test]
fn a_link_across_paragraphs_makes_one_link_in_each_and_leaves_markers_outside() {
    let bytes = with_body(concat!(
        r#"<w:p><w:r><w:t>One two</w:t></w:r><w:bookmarkStart w:id="0" w:name="mark"/></w:p>"#,
        r#"<w:p><w:bookmarkEnd w:id="0"/><w:r><w:t>Three</w:t></w:r><w:r><w:tab/><w:t>four</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:t>Five six</w:t></w:r></w:p>"#,
    ));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let (one, five) = (text_node(&doc, "One"), text_node(&doc, "Five"));
    edit(&mut doc, set_link(at(one, 4), at(five, 4), Some("#mark")));
    assert_eq!(
        links(&doc),
        [
            pair("two", "#mark"),
            pair("Threefour", "#mark"),
            pair("Five", "#mark")
        ]
    );
    let out = doc.export().unwrap();
    let xml = part(&out, DOCUMENT);
    assert!(
        xml.contains(r#"<w:hyperlink w:anchor="mark"><w:r><w:rPr><w:rStyle w:val="Hyperlink"/></w:rPr><w:t xml:space="preserve">two</w:t></w:r></w:hyperlink><w:bookmarkStart w:id="0" w:name="mark"/></w:p><w:p><w:bookmarkEnd w:id="0"/><w:hyperlink w:anchor="mark">"#),
        "{xml}"
    );
    // A link to a place in the document needs no relationship.
    assert_eq!(doc.dirty_parts(), [DOCUMENT, STYLES]);
    let reopened = reopen("bookmarks", &doc);
    assert_eq!(links(&reopened), links(&doc));
    assert_editor_undo("bookmarks", &bytes, &mut doc);
}

#[test]
fn linking_over_a_link_replaces_it_and_the_same_link_again_changes_nothing() {
    let bytes = generated("complex-mixed");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    assert_eq!(
        links(&doc),
        [pair("the example site", "https://example.com/")]
    );
    let site = text_node(&doc, "the example site");
    edit(
        &mut doc,
        set_link(at(site, 0), at(site, 16), Some("https://example.com/")),
    );
    assert!(!doc.is_dirty());
    // Linking part of it to where it already points splits nothing.
    edit(
        &mut doc,
        set_link(at(site, 4), at(site, 11), Some("https://example.com/")),
    );
    assert!(!doc.is_dirty());

    let see = text_node(&doc, "See ");
    let after = text_node(&doc, " for details.");
    edit(
        &mut doc,
        set_link(at(see, 0), at(after, 4), Some("https://example.org/")),
    );
    assert_eq!(
        links(&doc),
        [pair("See the example site for", "https://example.org/")]
    );
    // The old link's own underline stays; it was direct formatting.
    let reopened = reopen("complex-mixed", &doc);
    assert_eq!(links(&reopened), links(&doc));
    let rels = part(&doc.export().unwrap(), RELS);
    assert!(rels.contains(r#"Id="rIdLink""#), "{rels}");
    assert!(rels.contains(&link_id("https://example.org/")), "{rels}");
    assert_editor_undo("complex-mixed", &bytes, &mut doc);
}

#[test]
fn removing_a_link_takes_out_what_the_range_covers_and_its_style() {
    let bytes = generated("simple-paragraphs");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let second = text_node(&doc, "Second");
    edit(
        &mut doc,
        set_link(at(second, 0), at(second, 17), Some("https://example.com/")),
    );
    let linked = doc.export().unwrap();
    let mut doc = EditingDocument::open(&linked).unwrap();
    let second = text_node(&doc, "Second paragraph.");
    edit(&mut doc, set_link(at(second, 7), at(second, 9), None));
    assert_eq!(
        links(&doc),
        [
            pair("Second ", "https://example.com/"),
            pair("ragraph.", "https://example.com/")
        ]
    );
    assert_eq!(run_look(&doc, "pa").0, None);
    assert_eq!(run_look(&doc, "ragraph.").0.as_deref(), Some("Hyperlink"));
    let out = doc.export().unwrap();
    // Both halves name the one relationship, which stays.
    let id = link_id("https://example.com/");
    assert_eq!(part(&out, DOCUMENT).matches(&id).count(), 2);
    assert_eq!(doc.dirty_parts(), [DOCUMENT]);
    let reopened = reopen("simple-paragraphs", &doc);
    assert_eq!(links(&reopened), links(&doc));
    assert_editor_undo("simple-paragraphs", &linked, &mut doc);

    // Text outside links is left whole: a range from plain text into a
    // link cuts only the link.
    let mut doc = EditingDocument::open(&linked).unwrap();
    let first = text_node(&doc, "First paragraph.");
    edit(&mut doc, set_link(at(first, 2), at(first, 9), None));
    assert!(!doc.is_dirty());
    let bytes = generated("complex-mixed");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let see = text_node(&doc, "See ");
    let site = text_node(&doc, "the example site");
    edit(&mut doc, set_link(at(see, 2), at(site, 8), None));
    assert_eq!(
        texts_of(&doc, "See "),
        ["See ", "the exam", "ple site", " for details."]
    );
    assert_eq!(links(&doc), [pair("ple site", "https://example.com/")]);
    assert_editor_undo("complex-mixed", &bytes, &mut doc);

    let mut doc = EditingDocument::open(&linked).unwrap();
    let paragraph = paragraph_with(&doc, "Second");
    edit(
        &mut doc,
        set_link(
            Point::Start { start: paragraph },
            Point::End { end: paragraph },
            None,
        ),
    );
    assert!(links(&doc).is_empty());
    assert_eq!(run_look(&doc, "Second paragraph.").0, None);
    assert_editor_undo("simple-paragraphs", &linked, &mut doc);
}

#[test]
fn a_document_with_a_hyperlink_style_uses_it_whatever_its_id() {
    let bytes = generated("simple-paragraphs");
    let styles = part(&bytes, STYLES).replace(
        "</w:styles>",
        r#"<w:style w:type="character" w:styleId="Lienhypertexte"><w:name w:val="Hyperlink"/><w:rPr><w:color w:val="0000FF"/><w:u w:val="single"/></w:rPr></w:style></w:styles>"#,
    );
    let bytes = rewrite(&bytes, STYLES, Some(styles.as_bytes()));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let third = text_node(&doc, "Third");
    edit(
        &mut doc,
        set_link(at(third, 0), at(third, 5), Some("https://example.com/")),
    );
    assert_eq!(
        run_look(&doc, "Third"),
        (
            Some("Lienhypertexte".into()),
            Some("0000FF".into()),
            Some("single".into())
        )
    );
    assert_eq!(doc.dirty_parts(), [DOCUMENT, RELS]);
}

#[test]
fn the_added_hyperlink_style_takes_an_id_no_style_or_run_names() {
    let bytes = generated("simple-paragraphs");
    let styles = part(&bytes, STYLES).replace(
        "</w:styles>",
        r#"<w:style w:type="paragraph" w:styleId="Hyperlink"><w:name w:val="Link List"/></w:style></w:styles>"#,
    );
    let document = part(&bytes, DOCUMENT).replace(
        "<w:r><w:t>Third paragraph.</w:t></w:r>",
        r#"<w:r><w:rPr><w:rStyle w:val="Hyperlink1"/></w:rPr><w:t>Third paragraph.</w:t></w:r>"#,
    );
    let bytes = rewrite(&bytes, STYLES, Some(styles.as_bytes()));
    let bytes = rewrite(&bytes, DOCUMENT, Some(document.as_bytes()));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let second = paragraph_with(&doc, "Second");
    edit(&mut doc, Command::MergeParagraphs { paragraph: second });
    let first = text_node(&doc, "First");
    edit(
        &mut doc,
        set_link(at(first, 0), at(first, 5), Some("https://example.com/")),
    );
    let reopened = Document::open(&doc.export().unwrap()).unwrap();
    for view in [&*doc, &reopened] {
        assert_eq!(run_look(view, "First").0.as_deref(), Some("Hyperlink2"));
        assert_eq!(
            run_look(view, "Third paragraph."),
            (Some("Hyperlink1".into()), None, None)
        );
    }
}

#[test]
fn adding_a_link_preserves_inherited_and_paragraph_mark_styles() {
    let bytes = generated("simple-paragraphs");
    let styles = part(&bytes, STYLES).replace(
        r#"<w:name w:val="Emphasis"/>"#,
        r#"<w:name w:val="Emphasis"/><w:basedOn w:val="Hyperlink"/>"#,
    );
    let document = part(&bytes, DOCUMENT)
        .replace(
            "<w:r><w:t>Second paragraph.",
            r#"<w:r><w:rPr><w:rStyle w:val="Emphasis"/></w:rPr><w:t>Second paragraph."#,
        )
        .replace(
            "<w:r><w:t>Third paragraph.</w:t></w:r>",
            r#"<w:pPr><w:rPr><w:rStyle w:val="Hyperlink1"/></w:rPr></w:pPr>"#,
        );
    let bytes = rewrite(&bytes, STYLES, Some(styles.as_bytes()));
    let bytes = rewrite(&bytes, DOCUMENT, Some(document.as_bytes()));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let first = text_node(&doc, "First");
    edit(
        &mut doc,
        set_link(at(first, 0), at(first, 5), Some("#here")),
    );
    let end = doc.paragraphs()[2];
    edit(
        &mut doc,
        Command::InsertText {
            at: Point::End { end },
            text: "Other".into(),
        },
    );

    let reopened = Document::open(&doc.export().unwrap()).unwrap();
    for view in [&*doc, &reopened] {
        assert_eq!(
            run_look(view, "Second paragraph."),
            (Some("Emphasis".into()), None, None)
        );
        assert_eq!(
            run_look(view, "Other"),
            (Some("Hyperlink1".into()), None, None)
        );
        assert_eq!(run_look(view, "First").0.as_deref(), Some("Hyperlink2"));
    }
}

#[test]
fn a_generated_character_style_never_formats_an_unrelated_paragraph() {
    let xml = r#"<w:p><w:r><w:t>First</w:t></w:r></w:p><w:p><w:pPr><w:pStyle w:val="Hyperlink"/></w:pPr><w:r><w:t>Other</w:t></w:r></w:p>"#;
    let mut doc = EditingDocument::open(&with_body(xml)).unwrap();
    let before = run_look(&doc, "Other");
    let first = text_node(&doc, "First");
    edit(
        &mut doc,
        set_link(at(first, 0), at(first, 5), Some("#here")),
    );
    let bytes = doc.export().unwrap();
    assert!(part(&bytes, DOCUMENT).contains(r#"<w:pStyle w:val="Hyperlink"/>"#));
    for view in [&*doc, &Document::open(&bytes).unwrap()] {
        assert_eq!(run_look(view, "Other"), before);
        assert_eq!(run_look(view, "First").0.as_deref(), Some("Hyperlink"));
    }
}

#[test]
fn a_link_in_a_header_adds_the_headers_relationships_part() {
    let bytes = generated("header-footer");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let header = text_node(&doc, "Header text");
    edit(
        &mut doc,
        set_link(at(header, 0), at(header, 6), Some("https://example.com/")),
    );
    let out = doc.export().unwrap();
    let rels = part(&out, "word/_rels/header1.xml.rels");
    assert!(
        rels.contains(&format!(
            r#"<Relationship Id="{}" Type="{HYPERLINK}" Target="https://example.com/" TargetMode="External"/>"#,
            link_id("https://example.com/")
        )),
        "{rels}"
    );
    assert_eq!(
        doc.dirty_parts(),
        ["word/header1.xml", "word/_rels/header1.xml.rels", STYLES]
    );
    let reopened = reopen("header-footer", &doc);
    assert_eq!(links(&reopened), [pair("Header", "https://example.com/")]);
    assert_editor_undo("header-footer", &bytes, &mut doc);
}

#[test]
fn a_link_and_a_new_list_share_the_main_relationships_part() {
    let bytes = generated("simple-paragraphs");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let first = text_node(&doc, "First");
    edit(
        &mut doc,
        Command::SetList {
            from: at(first, 0),
            to: at(first, 0),
            kind: Some(ListKind::Bullet),
        },
    );
    edit(
        &mut doc,
        set_link(at(first, 0), at(first, 5), Some("https://example.com/")),
    );
    let out = doc.export().unwrap();
    let rels = part(&out, RELS);
    assert!(rels.contains("relationships/numbering"), "{rels}");
    assert!(rels.contains("relationships/hyperlink"), "{rels}");
    assert_eq!(
        doc.dirty_parts(),
        [
            DOCUMENT,
            "[Content_Types].xml",
            RELS,
            "word/numbering.xml",
            STYLES
        ]
    );
    let reopened = reopen("simple-paragraphs", &doc);
    assert_eq!(links(&reopened), [pair("First", "https://example.com/")]);
}

#[test]
fn link_commands_refuse_and_change_nothing() {
    let refused = |name: &str, doc: &mut EditingDocument, command: Command, reason: &str| {
        let before = stories(doc);
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
        assert!(!doc.is_dirty(), "{name}: {command:?} dirtied the document");
    };
    let url = Some("https://example.com/");
    let mut doc = EditingDocument::open(&generated("simple-paragraphs")).unwrap();
    let (first, second) = (text_node(&doc, "First"), text_node(&doc, "Second"));
    for (command, reason) in [
        (set_link(at(first, 3), at(first, 3), url), "is empty"),
        (
            set_link(at(second, 0), at(first, 3), url),
            "ends before it starts",
        ),
        (
            set_link(at(first, 0), at(first, 3), Some(" ")),
            "somewhere to point",
        ),
        (
            set_link(at(first, 0), at(first, 3), Some("#")),
            "somewhere to point",
        ),
        (
            set_link(at(first, 0), at(first, 3), Some("a\tb")),
            "tabs and line breaks",
        ),
    ] {
        refused("simple-paragraphs", &mut doc, command, reason);
    }

    let mut doc = EditingDocument::open(&generated("header-footer")).unwrap();
    let (header, body) = (text_node(&doc, "Header text"), text_node(&doc, "Body with"));
    refused(
        "header-footer",
        &mut doc,
        set_link(at(body, 0), at(header, 3), url),
        "another story",
    );

    let mut doc = EditingDocument::open(&generated("tracked-changes")).unwrap();
    let kept = text_node(&doc, "Kept text");
    let inserted = text_node(&doc, "inserted text");
    refused(
        "tracked-changes",
        &mut doc,
        set_link(at(kept, 0), at(inserted, 3), url),
        "inside a tracked change",
    );
    let newly = text_node(&doc, "newly bold");
    refused(
        "tracked-changes",
        &mut doc,
        set_link(at(newly, 0), at(newly, 3), url),
        "inside text with a tracked formatting change",
    );

    // The table of contents is one field across two paragraphs.
    let mut doc = EditingDocument::open(&generated("complex-mixed")).unwrap();
    let title = text_node(&doc, "Mixed document");
    let heading = doc.paragraphs()[3];
    assert_eq!(doc.text(heading).unwrap(), "Chapter one");
    refused(
        "complex-mixed",
        &mut doc,
        set_link(at(title, 0), Point::End { end: heading }, url),
        "part of a field",
    );

    // A relationship of another kind has the ID the link would take.
    let bytes = generated("simple-paragraphs");
    let rels = part(&bytes, RELS).replace(
        "</Relationships>",
        &format!(
            r#"<Relationship Id="{}" Type="{HYPERLINK}" Target="https://elsewhere.example/" TargetMode="External"/></Relationships>"#,
            link_id("https://example.com/")
        ),
    );
    let mut doc = EditingDocument::open(&rewrite(&bytes, RELS, Some(rels.as_bytes()))).unwrap();
    let first = text_node(&doc, "First");
    refused(
        "simple-paragraphs",
        &mut doc,
        set_link(at(first, 0), at(first, 3), url),
        "already has a relationship",
    );
}

#[test]
fn every_fixture_links_ranges_unlinks_them_and_undoes_and_redoes_exactly() {
    let mut linked = 0;
    for (name, bytes) in all_fixtures() {
        let doc = Document::open(&bytes).unwrap();
        let first_text = |doc: &Document, p: NodeId| {
            doc.descendants(p).into_iter().find(|&id| {
                let node = doc.node(id).unwrap();
                node.support == Support::Editable
                    && matches!(&node.kind, NodeKind::Text { text } if text.chars().count() > 1)
            })
        };
        let paragraphs = doc.paragraphs();
        for triple in paragraphs.windows(3) {
            let (Some(from), Some(to)) = (first_text(&doc, triple[0]), first_text(&doc, triple[2]))
            else {
                continue;
            };
            let mut doc = EditingDocument::open(&bytes).unwrap();
            let text = |d: &Document| -> Vec<String> {
                triple.iter().map(|&p| d.text(p).unwrap()).collect()
            };
            let original = text(&doc);
            let command = set_link(at(from, 1), at(to, 1), Some("https://example.com/x"));
            let Ok(()) = doc.0.apply(&command) else {
                assert_eq!(
                    stories(&doc),
                    stories(&Document::open(&bytes).unwrap()),
                    "{name}: refusal changed the document"
                );
                assert!(!doc.is_dirty(), "{name}: refusal dirtied the document");
                continue;
            };
            if links(&doc) == links(&Document::open(&bytes).unwrap()) {
                continue;
            }
            linked += 1;
            assert!(
                links(&doc)
                    .iter()
                    .any(|(_, href)| href == "https://example.com/x"),
                "{name}: no link"
            );
            let reopened = reopen(&name, &doc);
            assert_eq!(links(&reopened), links(&doc), "{name}");
            assert_eq!(text(&doc), original, "{name}: the text changed");

            let before = links(&doc).len();
            let removed = doc.0.apply(&set_link(
                Point::Start { start: triple[0] },
                Point::End { end: triple[2] },
                None,
            ));
            removed.unwrap_or_else(|e| panic!("{name}: unlink: {e}"));
            assert!(links(&doc).len() < before, "{name}: nothing unlinked");
            assert!(
                links(&doc)
                    .iter()
                    .all(|(_, href)| href != "https://example.com/x"),
                "{name}: a link stayed"
            );
            assert_eq!(text(&doc), original, "{name}: unlinking changed the text");
            reopen(&name, &doc);
            assert!(doc.0.undo());

            assert_editor_undo(&name, &bytes, &mut doc);
        }
    }
    assert!(linked > 50, "only {linked} ranges linked");
}

#[test]
fn link_commands_round_trip_through_json_and_are_listed() {
    let n = |s: &str| s.parse::<NodeId>().unwrap();
    let from = Point::Text {
        node: n("n3"),
        offset: 2,
    };
    let to = Point::End { end: n("n5") };
    for (command, json) in [
        (
            set_link(from, to, Some("https://example.com/")),
            r#"{"command":"set_link","from":{"node":"n3","offset":2},"to":{"end":"n5"},"target":"https://example.com/"}"#,
        ),
        (
            set_link(from, to, None),
            r#"{"command":"set_link","from":{"node":"n3","offset":2},"to":{"end":"n5"},"target":null}"#,
        ),
    ] {
        assert_eq!(serde_json::to_string(&command).unwrap(), json);
        assert_eq!(serde_json::from_str::<Command>(json).unwrap(), command);
        assert!(docx_engine::COMMANDS.contains(&command.name()));
    }
}

#[test]
fn the_added_hyperlink_style_is_listed_only_while_a_run_uses_it() {
    let bytes = generated("simple-paragraphs");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let fresh = doc.inspect_json();
    let styles = |doc: &Document| -> Vec<String> {
        doc.inspect_json()["styles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_str().unwrap().to_string())
            .collect()
    };
    let first = text_node(&doc, "First");
    edit(
        &mut doc,
        set_link(at(first, 0), at(first, 5), Some("https://example.com/")),
    );
    assert!(styles(&doc).contains(&"Hyperlink".to_string()));
    assert!(doc.0.undo());
    assert_eq!(doc.inspect_json(), fresh);
}

#[test]
fn linking_the_start_of_a_run_leaves_the_rest_of_it_unstyled() {
    let bytes = generated("simple-paragraphs");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let first = text_node(&doc, "First");
    edit(
        &mut doc,
        set_link(at(first, 0), at(first, 5), Some("https://example.com")),
    );
    let xml = part(&doc.export().unwrap(), DOCUMENT);
    assert_eq!(xml.matches(r#"<w:rStyle w:val="Hyperlink"/>"#).count(), 1);
    assert!(
        xml.contains(r#"</w:hyperlink><w:r><w:t xml:space="preserve"> paragraph.</w:t></w:r>"#),
        "{xml}"
    );
}

#[test]
fn a_new_link_uses_the_relationship_namespace_at_its_paragraph() {
    let base = generated("simple-paragraphs");
    for container in ["<w:body>", "<w:p>"] {
        let xml = part(&base, DOCUMENT).replacen(container, &format!(
            "{} xmlns:r=\"urn:extension\" xmlns:rel=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">",
            container.trim_end_matches('>')), 1);
        let bytes = rewrite(&base, DOCUMENT, Some(xml.as_bytes()));
        let mut doc = EditingDocument::open(&bytes).unwrap();
        let text = text_node(&doc, "First");
        edit(
            &mut doc,
            set_link(at(text, 0), at(text, 5), Some("https://example.com")),
        );
        let out = doc.export().unwrap();
        let reopened = Document::open(&out).unwrap();
        assert_eq!(links(&reopened), [pair("First", "https://example.com")]);
        assert!(part(&out, DOCUMENT).contains("<w:hyperlink rel:id="));
    }
}
