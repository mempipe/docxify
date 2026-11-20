use super::common;

use common::*;
use docx_engine::{Command, Document, EditError, Format, NodeId, NodeKind, Point, Track};

const DOCUMENT: &str = "word/document.xml";
const DATE: &str = "2026-11-16T10:00:00Z";

fn alice() -> Track {
    Track {
        author: "Alice".into(),
        date: DATE.into(),
    }
}

fn bob() -> Track {
    Track {
        author: "Bob".into(),
        date: "2026-11-16T11:00:00Z".into(),
    }
}

fn tracked(from: Point, to: Point, text: &str, track: &Track) -> Command {
    Command::ReplaceRange {
        from,
        to,
        text: text.into(),
        track: Some(track.clone()),
    }
}

/// The body's paragraphs as `document.xml` writes them.
fn body(doc: &Document) -> String {
    let xml = part(&doc.export().unwrap(), DOCUMENT);
    let (start, end) = (
        xml.find("<w:body>").unwrap() + 8,
        xml.find("<w:sectPr").unwrap(),
    );
    xml[start..end].to_string()
}

fn ins(id: u32, track: &Track, inner: &str) -> String {
    format!(
        r#"<w:ins w:id="{id}" w:author="{}" w:date="{}">{inner}</w:ins>"#,
        track.author, track.date
    )
}

fn del(id: u32, track: &Track, inner: &str) -> String {
    format!(
        r#"<w:del w:id="{id}" w:author="{}" w:date="{}">{inner}</w:del>"#,
        track.author, track.date
    )
}

fn run(text: &str) -> String {
    format!(r#"<w:r><w:t xml:space="preserve">{text}</w:t></w:r>"#)
}

fn deleted(text: &str) -> String {
    format!(r#"<w:r><w:delText xml:space="preserve">{text}</w:delText></w:r>"#)
}

/// The text node holding exactly `exact`.
fn exact(doc: &Document, exact: &str) -> NodeId {
    doc.paragraphs()
        .into_iter()
        .flat_map(|p| doc.descendants(p))
        .find(|&id| matches!(&doc.node(id).unwrap().kind, NodeKind::Text { text } if text == exact))
        .unwrap_or_else(|| panic!("no text node {exact:?}"))
}

#[test]
fn typing_with_track_changes_inserts_a_revision() {
    let mut doc = EditingDocument::open(&generated("simple-paragraphs")).unwrap();
    let first = text_node(&doc, "First");
    edit(
        &mut doc,
        tracked(at(first, 5), at(first, 5), " big", &alice()),
    );
    assert_eq!(texts(&doc)[0], "First big paragraph.");
    let expected = format!(
        "<w:p>{}{}{}</w:p>",
        run("First"),
        ins(1, &alice(), &run(" big")),
        run(" paragraph.")
    );
    assert!(body(&doc).starts_with(&expected), "{}", body(&doc));
    let reopened = reopen("typed", &doc);
    let revision = reopened
        .descendants(reopened.paragraphs()[0])
        .into_iter()
        .find(|&id| matches!(reopened.node(id).unwrap().kind, NodeKind::Revision(_)))
        .unwrap();
    let NodeKind::Revision(info) = &reopened.node(revision).unwrap().kind else {
        unreachable!();
    };
    assert_eq!(
        (
            info.change.as_str(),
            info.author.as_deref(),
            info.date.as_deref()
        ),
        ("ins", Some("Alice"), Some(DATE))
    );
}

#[test]
fn typing_letter_by_letter_gives_what_typing_the_word_gives() {
    let bytes = generated("simple-paragraphs");
    let mut whole = EditingDocument::open(&bytes).unwrap();
    let first = text_node(&whole, "First");
    edit(
        &mut whole,
        tracked(at(first, 5), at(first, 5), " big", &alice()),
    );

    // The caret after each letter is at the end of the insertion, or at the
    // start of the text after it.
    for after in [true, false] {
        let mut doc = EditingDocument::open(&bytes).unwrap();
        let first = text_node(&doc, "First");
        edit(&mut doc, tracked(at(first, 5), at(first, 5), " ", &alice()));
        for (typed, letter) in [(" ", "b"), (" b", "i"), (" bi", "g")] {
            let caret = match after {
                true => at(exact(&doc, typed), typed.len()),
                false => at(exact(&doc, " paragraph."), 0),
            };
            edit(&mut doc, tracked(caret, caret, letter, &alice()));
        }
        assert_eq!(body(&doc), body(&whole), "caret after: {after}");
    }
}

#[test]
fn backspacing_letter_by_letter_gives_what_deleting_the_word_gives() {
    let bytes = generated("simple-paragraphs");
    let mut whole = EditingDocument::open(&bytes).unwrap();
    let first = text_node(&whole, "First");
    edit(
        &mut whole,
        tracked(at(first, 6), at(first, 15), "", &alice()),
    );
    let expected = format!(
        "<w:p>{}{}{}</w:p>",
        run("First "),
        del(1, &alice(), &deleted("paragraph")),
        run(".")
    );
    assert!(body(&whole).starts_with(&expected), "{}", body(&whole));

    let mut doc = EditingDocument::open(&bytes).unwrap();
    for end in (7..=15).rev() {
        let first = text_node(&doc, "First");
        edit(
            &mut doc,
            tracked(at(first, end - 1), at(first, end), "", &alice()),
        );
    }
    assert_eq!(body(&doc), body(&whole));

    // Forward, with Delete: the caret moves past each deleted letter.
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let first = text_node(&doc, "First");
    edit(&mut doc, tracked(at(first, 6), at(first, 7), "", &alice()));
    for _ in 7..15 {
        let rest = text_node(&doc, ".");
        edit(&mut doc, tracked(at(rest, 0), at(rest, 1), "", &alice()));
    }
    assert_eq!(body(&doc), body(&whole));
}

#[test]
fn typing_over_a_selection_marks_it_deleted_before_the_insertion() {
    let mut doc = EditingDocument::open(&generated("simple-paragraphs")).unwrap();
    let second = text_node(&doc, "Second");
    edit(
        &mut doc,
        tracked(at(second, 7), at(second, 16), "line", &alice()),
    );
    assert_eq!(texts(&doc)[1], "Second line.");
    let expected = format!(
        "<w:p>{}{}{}{}</w:p>",
        run("Second "),
        del(1, &alice(), &deleted("paragraph")),
        ins(2, &alice(), &run("line")),
        run(".")
    );
    assert!(body(&doc).contains(&expected), "{}", body(&doc));
    assert_valid("typed over", &doc.export().unwrap());
}

#[test]
fn deleting_what_the_same_author_inserted_removes_it() {
    let bytes = generated("simple-paragraphs");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let first = text_node(&doc, "First");
    edit(
        &mut doc,
        tracked(at(first, 5), at(first, 5), " big", &alice()),
    );
    let big = exact(&doc, " big");
    edit(&mut doc, tracked(at(big, 1), at(big, 4), "", &alice()));
    let expected = format!(
        "<w:p>{}{}{}</w:p>",
        run("First"),
        ins(1, &alice(), &run(" ")),
        run(" paragraph.")
    );
    assert!(body(&doc).starts_with(&expected), "{}", body(&doc));

    // Backspacing the rest leaves no empty revision, nor any other trace.
    let space = exact(&doc, " ");
    edit(&mut doc, tracked(at(space, 0), at(space, 1), "", &alice()));
    let untouched = Document::open(&bytes).unwrap().export().unwrap();
    assert!(doc.export().unwrap() == untouched, "{}", body(&doc));
}

#[test]
fn another_author_deleting_an_insertion_nests_the_deletion() {
    let mut doc = EditingDocument::open(&generated("simple-paragraphs")).unwrap();
    let first = text_node(&doc, "First");
    edit(
        &mut doc,
        tracked(at(first, 5), at(first, 5), " big", &alice()),
    );
    let big = exact(&doc, " big");
    edit(&mut doc, tracked(at(big, 1), at(big, 4), "", &bob()));
    let nested = ins(2, &alice(), &del(3, &bob(), &deleted("big")));
    let expected = format!(
        "<w:p>{}{}{nested}{}</w:p>",
        run("First"),
        ins(1, &alice(), &run(" ")),
        run(" paragraph.")
    );
    assert!(body(&doc).starts_with(&expected), "{}", body(&doc));
    assert_valid("nested", &doc.export().unwrap());
}

#[test]
fn another_author_typing_inside_an_insertion_splits_it() {
    let mut doc = EditingDocument::open(&generated("simple-paragraphs")).unwrap();
    let first = text_node(&doc, "First");
    edit(
        &mut doc,
        tracked(at(first, 5), at(first, 5), " big", &alice()),
    );
    let big = exact(&doc, " big");
    edit(&mut doc, tracked(at(big, 2), at(big, 2), "X", &bob()));
    let expected = format!(
        "<w:p>{}{}{}{}{}</w:p>",
        run("First"),
        ins(1, &alice(), &run(" b")),
        ins(2, &bob(), &run("X")),
        ins(3, &alice(), &run("ig")),
        run(" paragraph.")
    );
    assert!(body(&doc).starts_with(&expected), "{}", body(&doc));
}

#[test]
fn enter_with_track_changes_marks_the_paragraph_mark_inserted() {
    let bytes = generated("simple-paragraphs");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let first = text_node(&doc, "First");
    edit(
        &mut doc,
        tracked(at(first, 6), at(first, 6), "\n", &alice()),
    );
    assert_eq!(&texts(&doc)[..2], ["First ", "paragraph."]);
    let mark = format!(
        r#"<w:pPr><w:rPr><w:ins w:id="1" w:author="Alice" w:date="{DATE}"/></w:rPr></w:pPr>"#
    );
    let expected = format!(
        "<w:p>{mark}{}</w:p><w:p>{}</w:p>",
        run("First "),
        run("paragraph.")
    );
    assert!(body(&doc).starts_with(&expected), "{}", body(&doc));

    // Backspace at the start of the new paragraph takes the mark back.
    let second = paragraph_with(&doc, "paragraph.");
    let first = paragraph_with(&doc, "First");
    edit(
        &mut doc,
        tracked(
            Point::End { end: first },
            Point::Start { start: second },
            "",
            &alice(),
        ),
    );
    assert_eq!(texts(&doc)[0], "First paragraph.");
    assert!(!body(&doc).contains("w:ins"), "{}", body(&doc));
}

#[test]
fn enter_inside_a_paragraph_whose_mark_was_inserted_keeps_the_change_on_that_mark() {
    let mut doc = EditingDocument::open(&generated("simple-paragraphs")).unwrap();
    let first = text_node(&doc, "First");
    edit(
        &mut doc,
        tracked(at(first, 6), at(first, 6), "\n", &alice()),
    );
    // Without tracking, Enter in "First " splits it; the inserted mark ends
    // the second half.
    let first = text_node(&doc, "First");
    edit(
        &mut doc,
        Command::ReplaceRange {
            from: at(first, 2),
            to: at(first, 2),
            text: "\n".into(),
            track: None,
        },
    );
    assert_eq!(&texts(&doc)[..3], ["Fi", "rst ", "paragraph."]);
    let xml = body(&doc);
    let second = xml.split("</w:p>").nth(1).unwrap();
    assert!(second.contains("<w:ins w:id=\"1\""), "{xml}");
    assert_eq!(xml.matches("<w:ins ").count(), 1, "{xml}");
}

#[test]
fn deleting_a_paragraph_mark_marks_it_deleted() {
    let mut doc = EditingDocument::open(&generated("simple-paragraphs")).unwrap();
    let (a, b) = (text_node(&doc, "First"), text_node(&doc, "Second"));
    edit(&mut doc, tracked(at(a, 6), at(b, 6), "", &alice()));
    assert_eq!(texts(&doc).len(), 3);
    let mark = format!(
        r#"<w:pPr><w:rPr><w:del w:id="1" w:author="Alice" w:date="{DATE}"/></w:rPr></w:pPr>"#
    );
    // The mark's change is in the paragraph's properties, so it comes first.
    let expected = format!(
        "<w:p>{mark}{}{}</w:p><w:p>{}{}</w:p>",
        run("First "),
        del(2, &alice(), &deleted("paragraph.")),
        del(3, &alice(), &deleted("Second")),
        run(" paragraph.")
    );
    assert!(body(&doc).starts_with(&expected), "{}", body(&doc));
    assert_valid("mark deleted", &doc.export().unwrap());
}

#[test]
fn undo_restores_the_document_and_redo_the_revisions() {
    let bytes = generated("simple-paragraphs");
    type Case = (&'static str, fn(&Document) -> Command);
    let cases: [Case; 3] = [
        ("type over a word", |d| {
            let t = text_node(d, "Second");
            tracked(at(t, 7), at(t, 16), "line", &alice())
        }),
        ("enter", |d| {
            let t = text_node(d, "First");
            tracked(at(t, 6), at(t, 6), "\n", &alice())
        }),
        ("delete across paragraphs", |d| {
            let (a, b) = (text_node(d, "First"), text_node(d, "Third"));
            tracked(at(a, 2), at(b, 3), "", &alice())
        }),
    ];
    for (name, command) in cases {
        let mut doc = EditingDocument::open(&bytes).unwrap();
        let command = command(&doc);
        edit(&mut doc, command);
        assert_editor_undo(name, &bytes, &mut doc);
    }
}

#[test]
fn revision_ids_start_above_the_documents_own() {
    let mut doc = EditingDocument::open(&generated("tracked-changes")).unwrap();
    let t = text_node(&doc, "Untouched");
    edit(&mut doc, tracked(at(t, 0), at(t, 0), "Now ", &alice()));
    assert!(
        body(&doc).contains(&ins(10, &alice(), &run("Now "))),
        "{}",
        body(&doc)
    );
    assert_valid("tracked-changes", &doc.export().unwrap());
}

#[test]
fn formatting_and_links_reach_text_inside_an_insertion() {
    let mut doc = EditingDocument::open(&generated("simple-paragraphs")).unwrap();
    let first = text_node(&doc, "First");
    edit(
        &mut doc,
        tracked(at(first, 5), at(first, 5), " big", &alice()),
    );
    let big = exact(&doc, " big");
    let bold = Format {
        bold: Some(true),
        ..Format::default()
    };
    edit(
        &mut doc,
        Command::FormatRange {
            from: at(big, 1),
            to: at(big, 4),
            format: bold,
        },
    );
    let inner = format!(
        "{}{}",
        run(" "),
        r#"<w:r><w:rPr><w:b/><w:bCs/></w:rPr><w:t xml:space="preserve">big</w:t></w:r>"#
    );
    assert!(
        body(&doc).contains(&ins(1, &alice(), &inner)),
        "{}",
        body(&doc)
    );

    let space = exact(&doc, " ");
    let big = exact(&doc, "big");
    edit(
        &mut doc,
        Command::SetLink {
            from: at(space, 0),
            to: at(big, 3),
            target: Some("#top".into()),
        },
    );
    let xml = body(&doc);
    assert!(
        xml.contains(&format!(
            "{}<w:hyperlink w:anchor=\"top\">",
            ins(1, &alice(), "").trim_end_matches("</w:ins>")
        )),
        "{xml}"
    );
    assert_valid("linked insertion", &doc.export().unwrap());
}

#[test]
fn tracked_changes_that_cannot_be_recorded_are_refused() {
    let tracked_doc = Document::open(&generated("tracked-changes")).unwrap();
    let kept = text_node(&tracked_doc, "Kept text");
    let after = text_node(&tracked_doc, "newly bold");
    let refused = |doc: &Document, command: Command| {
        let mut copy = EditingDocument::open(&doc.export().unwrap()).unwrap();
        let err = copy.0.apply(&command).unwrap_err();
        assert!(!copy.is_dirty(), "{err}");
        err
    };
    // Across a change the document opened with.
    let err = refused(
        &tracked_doc,
        tracked(at(kept, 2), at(after, 3), "", &alice()),
    );
    assert!(
        matches!(
            err,
            SessionError::Unsupported { .. }
                | SessionError::Edit(EditError::Refused { .. } | EditError::ReadOnly { .. })
        ),
        "{err}"
    );

    // Across a table.
    let table = Document::open(&generated("table")).unwrap();
    let (a, b) = (
        text_node(&table, "Before the table."),
        text_node(&table, "After the table."),
    );
    let err = refused(&table, tracked(at(a, 2), at(b, 3), "", &alice()));
    assert!(err.to_string().contains("table"), "{err}");

    // From a header into the body, at the same place in each story.
    let stories = Document::open(&generated("header-footer")).unwrap();
    let (header, body) = (
        text_node(&stories, "Header text"),
        text_node(&stories, "Body with"),
    );
    let err = refused(&stories, tracked(at(header, 1), at(body, 1), "x", &alice()));
    assert!(matches!(err, SessionError::Unsupported { .. }), "{err}");

    // A missing author or a date Word cannot read.
    let simple = Document::open(&generated("simple-paragraphs")).unwrap();
    let first = text_node(&simple, "First");
    for track in [
        Track {
            author: " ".into(),
            date: DATE.into(),
        },
        Track {
            author: "Alice".into(),
            date: "16 November 2026".into(),
        },
    ] {
        let err = refused(&simple, tracked(at(first, 0), at(first, 1), "x", &track));
        assert!(
            matches!(err, SessionError::Edit(EditError::Track { .. })),
            "{err}"
        );
    }
}

#[test]
fn removing_a_session_insertion_is_refused_without_consuming_undo() {
    let bytes = generated("simple-paragraphs");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let at = at(text_node(&doc, "First"), 2);
    edit(&mut doc, tracked(at, at, "new", &alice()));
    let rev = doc
        .descendants(doc.paragraphs()[0])
        .into_iter()
        .find(|&id| matches!(doc.node(id).unwrap().kind, NodeKind::Revision(_)))
        .unwrap();
    let before = doc.export().unwrap();
    assert!(doc.run(Command::RemoveNode { node: rev }).is_err());
    assert_eq!(doc.export().unwrap(), before);
    assert_editor_undo("session insertion", &bytes, &mut doc);
}
