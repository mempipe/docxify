#[path = "../../tests/common/mod.rs"]
mod common;

use crate::{Document, EditError, NodeId, NodeKind};
use common::*;

const BODY: &str = concat!(
    r#"<w:p><w:r><w:rPr><w:b/></w:rPr><w:t>Bold</w:t></w:r><w:r><w:t> plain</w:t></w:r></w:p>"#,
    r#"<w:p><w:r><w:rPr><w:i/></w:rPr><w:t>Italic</w:t></w:r></w:p>"#,
);

fn children(doc: &Document, id: NodeId) -> Vec<NodeId> {
    doc.node(id).unwrap().children.clone()
}

fn document_xml(doc: &Document) -> String {
    part(&doc.export().unwrap(), "word/document.xml")
}

#[test]
fn arranging_what_a_container_already_holds_changes_nothing() {
    for (name, bytes) in all_fixtures() {
        let mut doc = Document::open(&bytes).unwrap();
        let roots: Vec<NodeId> = doc.stories().map(|s| s.id).collect();
        let nodes: Vec<NodeId> = (roots.into_iter())
            .flat_map(|r| doc.descendants(r))
            .collect();
        for id in nodes {
            let node = doc.node(id).unwrap();
            if matches!(node.kind, NodeKind::Paragraph(_) | NodeKind::Run(_)) {
                let held = node.children.clone();
                doc.arrange(id, &held).unwrap();
            }
        }
        assert!(!doc.is_dirty(), "{name}");
    }
}

#[test]
fn a_container_holds_what_it_is_arranged_to_hold() {
    let bytes = with_body(BODY);
    let mut doc = Document::open(&bytes).unwrap();
    let [first, second] = doc.paragraphs()[..] else {
        panic!("two paragraphs");
    };
    let [bold, plain] = children(&doc, first)[..] else {
        panic!("two runs");
    };
    let italic = children(&doc, second)[0];
    // A run from the other paragraph comes between this one's, and one is
    // left out.
    doc.arrange(first, &[plain, italic]).unwrap();
    assert_eq!(doc.text(first).unwrap(), " plainItalic");
    assert_eq!(doc.text(second).unwrap(), "");
    assert_eq!(doc.node(italic).unwrap().parent, Some(first));
    assert_eq!(doc.node(bold).unwrap().parent, None);
    let xml = document_xml(&doc);
    assert!(
        xml.contains(concat!(
            r#"<w:p><w:r><w:t> plain</w:t></w:r>"#,
            r#"<w:r><w:rPr><w:i/></w:rPr><w:t>Italic</w:t></w:r></w:p><w:p></w:p>"#
        )),
        "{xml}"
    );
    assert_valid("arranged", &doc.export().unwrap());
    // What was left out can be put back.
    doc.arrange(second, &[bold]).unwrap();
    assert_eq!(texts(&reopen("arranged", &doc)), [" plainItalic", "Bold"]);
}

#[test]
fn new_content_is_made_loose_and_arranged_in() {
    let bytes = with_body(BODY);
    let mut doc = Document::open(&bytes).unwrap();
    let [first, second] = doc.paragraphs()[..] else {
        panic!("two paragraphs");
    };
    let [bold, plain] = children(&doc, first)[..] else {
        panic!("two runs");
    };
    let word = children(&doc, bold)[0];
    // A copy of the bold run takes new text and a line break, and a run
    // like the italic one takes text in the first paragraph.
    let copy = doc.empty_like(bold, true).unwrap();
    let more = doc.loose_text(first, "er").unwrap();
    let line = doc.loose_break(first).unwrap();
    doc.arrange(copy, &[line, more]).unwrap();
    let italic = children(&doc, second)[0];
    let run = doc.empty_run(first, Some(italic)).unwrap();
    let typed = doc.loose_text(first, "typed").unwrap();
    doc.arrange(run, &[typed]).unwrap();
    let bare = doc.empty_run(first, None).unwrap();
    let end = doc.loose_text(first, ".").unwrap();
    doc.arrange(bare, &[end]).unwrap();
    doc.arrange(first, &[bold, run, copy, plain, bare]).unwrap();
    assert_eq!(doc.node(word).unwrap().parent, Some(bold));
    assert_eq!(doc.like(copy), Some(bold));
    assert_eq!(doc.like(run), Some(italic));
    assert_eq!(doc.like(bare), None);
    let xml = document_xml(&doc);
    assert!(
        xml.contains(concat!(
            r#"<w:p><w:r><w:rPr><w:b/></w:rPr><w:t>Bold</w:t></w:r>"#,
            r#"<w:r><w:rPr><w:i/></w:rPr><w:t xml:space="preserve">typed</w:t></w:r>"#,
            r#"<w:r><w:rPr><w:b/></w:rPr><w:br/><w:t xml:space="preserve">er</w:t></w:r>"#,
            r#"<w:r><w:t> plain</w:t></w:r>"#,
            r#"<w:r><w:t xml:space="preserve">.</w:t></w:r></w:p>"#
        )),
        "{xml}"
    );
    assert_valid("composed", &doc.export().unwrap());
    assert_eq!(
        texts(&reopen("composed", &doc)),
        ["Boldtyped\ner plain.", "Italic"]
    );
}

#[test]
fn arranging_refuses_what_would_not_be_a_tree() {
    let bytes = with_body(BODY);
    let mut doc = Document::open(&bytes).unwrap();
    let first = doc.paragraphs()[0];
    let [bold, plain] = children(&doc, first)[..] else {
        panic!("two runs");
    };
    let word = children(&doc, bold)[0];
    let before = document_xml(&doc);
    let refused = [
        doc.arrange(first, &[bold, plain, bold]),
        doc.arrange(bold, &[first]),
        doc.arrange(bold, &[bold]),
        doc.arrange(first, &[word, plain]),
        doc.arrange(bold, &[plain]),
    ];
    for result in refused {
        assert!(
            matches!(
                result,
                Err(EditError::Refused { .. } | EditError::WrongKind { .. })
            ),
            "{result:?}"
        );
    }
    assert!(!doc.is_dirty());
    assert_eq!(document_xml(&doc), before);
    assert!(matches!(
        doc.empty_like(first, true),
        Err(EditError::WrongKind { .. })
    ));
    assert!(matches!(
        doc.empty_like(word, true),
        Err(EditError::WrongKind { .. })
    ));
}

#[test]
fn a_copy_of_a_run_with_a_tracked_formatting_change_gets_its_own_revision_id() {
    let bytes = with_body(concat!(
        r#"<w:p><w:r><w:rPr><w:b/><w:rPrChange w:author="Ada" w:id="7" w:date="2026-01-01T00:00:00Z">"#,
        r#"<w:rPr/></w:rPrChange></w:rPr><w:t>newly bold</w:t></w:r></w:p>"#,
    ));
    let mut doc = Document::open(&bytes).unwrap();
    let paragraph = doc.paragraphs()[0];
    let run = children(&doc, paragraph)[0];
    let copy = doc.empty_like(run, true).unwrap();
    let more = doc.loose_text(paragraph, " still").unwrap();
    doc.arrange(copy, &[more]).unwrap();
    doc.arrange(paragraph, &[run, copy]).unwrap();
    let xml = document_xml(&doc);
    // The source's change keeps its ID, and the copy's is the next one free.
    assert!(
        xml.contains(concat!(
            r#"<w:r><w:rPr><w:b/><w:rPrChange w:author="Ada" w:id="7" w:date="2026-01-01T00:00:00Z">"#,
            r#"<w:rPr/></w:rPrChange></w:rPr><w:t>newly bold</w:t></w:r>"#,
            r#"<w:r><w:rPr><w:b/><w:rPrChange w:author="Ada" w:id="8" w:date="2026-01-01T00:00:00Z">"#,
            r#"<w:rPr/></w:rPrChange></w:rPr><w:t xml:space="preserve"> still</w:t></w:r></w:p>"#,
        )),
        "{xml}"
    );
    let reopened = reopen("copied-revision", &doc);
    assert_eq!(texts(&reopened), ["newly bold still"]);
}

#[test]
fn only_a_word_revision_id_on_a_word_tracked_change_is_renumbered_in_a_copy() {
    // An extension's `id` on the tracked change, and an extension's element
    // named as a tracked change is, are not the engine's to renumber.
    let bytes = with_body(concat!(
        r#"<w:p><w:r><w:rPr><w:b/>"#,
        r#"<w:rPrChange xmlns:x="urn:example:x" x:id="keep-me" w:id="7" w:author="Ada" w:date="2026-01-01T00:00:00Z">"#,
        r#"<w:rPr><x:rPrChange w:id="9" x:id="also"/></w:rPr></w:rPrChange></w:rPr><w:t>newly bold</w:t></w:r></w:p>"#,
    ));
    let mut doc = Document::open(&bytes).unwrap();
    let paragraph = doc.paragraphs()[0];
    let run = children(&doc, paragraph)[0];
    let copy = doc.empty_like(run, true).unwrap();
    let more = doc.loose_text(paragraph, " still").unwrap();
    doc.arrange(copy, &[more]).unwrap();
    doc.arrange(paragraph, &[run, copy]).unwrap();
    let xml = document_xml(&doc);
    assert!(
        xml.contains(concat!(
            r#"<w:r><w:rPr><w:b/>"#,
            r#"<w:rPrChange xmlns:x="urn:example:x" x:id="keep-me" w:id="10" w:author="Ada" w:date="2026-01-01T00:00:00Z">"#,
            r#"<w:rPr><x:rPrChange w:id="9" x:id="also"/></w:rPr></w:rPrChange></w:rPr>"#,
            r#"<w:t xml:space="preserve"> still</w:t></w:r></w:p>"#,
        )),
        "{xml}"
    );
    assert_valid("copied-extension", &doc.export().unwrap());
}

#[test]
fn a_loose_paragraph_and_a_formatted_run_are_built_without_a_command() {
    let body = concat!(
        r#"<w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:t>Centred</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:rPr><w:b/></w:rPr><w:t>Bold</w:t></w:r></w:p>"#,
    );
    let mut doc = Document::open(&with_body(body)).unwrap();
    let story = doc.stories().next().unwrap().id;
    let held = children(&doc, story);
    let (centred, bold) = (held[0], children(&doc, held[1])[0]);

    let paragraph = doc.loose_paragraph(story, Some(centred)).unwrap();
    let run = doc.empty_like(bold, true).unwrap();
    let text = doc.loose_text(run, "new").unwrap();
    doc.arrange(run, &[text]).unwrap();
    // A run is formatted where it is in the document, since its
    // paragraph's style counts.
    let unplaced = doc.format_run(run, Default::default()).unwrap_err();
    assert!(
        matches!(unplaced, EditError::WrongKind { .. }),
        "{unplaced}"
    );
    doc.arrange(paragraph, &[run]).unwrap();
    let mut all = held.clone();
    all.insert(1, paragraph);
    doc.arrange(story, &all).unwrap();
    let off = crate::Format {
        bold: Some(false),
        italic: Some(true),
        underline: None,
    };
    doc.format_run(run, off).unwrap();

    let xml = document_xml(&doc);
    let made = concat!(
        r#"<w:p><w:pPr><w:jc w:val="center"/></w:pPr>"#,
        r#"<w:r><w:rPr><w:i/><w:iCs/></w:rPr><w:t xml:space="preserve">new</w:t></w:r></w:p>"#,
    );
    assert!(xml.contains(made), "{xml}");
    assert!(
        xml.contains(r#"<w:r><w:rPr><w:b/></w:rPr><w:t>Bold</w:t></w:r>"#),
        "{xml}"
    );
}

#[test]
fn revision_builders_assign_unique_ids_and_format_runs_inside_wrappers() {
    use crate::{Change, Format, Track};
    let bytes = with_body(BODY);
    let mut doc = Document::open(&bytes).unwrap();
    let first = doc.paragraphs()[0];
    let run = doc.node(first).unwrap().children[0];
    let track = Track {
        author: "Alice".into(),
        date: "2026-11-18T00:00:00Z".into(),
    };
    let wrapper = doc
        .loose_revision(first, Change::Insertion, &track)
        .unwrap();
    doc.arrange(wrapper, &[run]).unwrap();
    let mut children = doc.node(first).unwrap().children.clone();
    children.insert(0, wrapper);
    doc.arrange(first, &children).unwrap();
    doc.format_run(
        run,
        Format {
            italic: Some(true),
            ..Format::default()
        },
    )
    .unwrap();
    doc.revision_mark(
        first,
        Some(&track),
        Some(&Track {
            author: "Bob".into(),
            date: track.date.clone(),
        }),
    )
    .unwrap();
    let xml = document_xml(&doc);
    let parsed = roxmltree::Document::parse(&xml).unwrap();
    let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    let ids: Vec<_> = parsed
        .descendants()
        .filter(|n| n.has_tag_name((w, "ins")) || n.has_tag_name((w, "del")))
        .map(|n| n.attribute((w, "id")).unwrap())
        .collect();
    assert_eq!(ids.len(), 3);
    assert_eq!(
        ids.iter().collect::<std::collections::HashSet<_>>().len(),
        3
    );
    assert_valid("built revisions", &doc.export().unwrap());
}
