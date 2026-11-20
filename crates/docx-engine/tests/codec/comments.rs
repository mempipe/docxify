use super::common;

use common::*;
use docx_engine::{Command, Document, EditError, NodeKind, Point, Track};

const DOCUMENT: &str = "word/document.xml";
const COMMENTS: &str = "word/comments.xml";
const THREADS: &str = "word/commentsExtended.xml";

fn alice() -> Track {
    Track {
        author: "Alice".into(),
        date: "2026-11-16T10:00:00Z".into(),
    }
}

fn comment(from: Point, to: Point, text: &str) -> Command {
    Command::AddComment {
        from,
        to,
        text: text.into(),
        by: alice(),
        initials: Some("A".into()),
        id: None,
    }
}

fn reply(to: &str, text: &str) -> Command {
    Command::ReplyToComment {
        comment: to.into(),
        text: text.into(),
        by: alice(),
        initials: None,
        id: None,
    }
}

/// The body's paragraphs as `document.xml` writes them.
fn body(out: &[u8]) -> String {
    let xml = part(out, DOCUMENT);
    let (start, end) = (
        xml.find("<w:body>").unwrap() + 8,
        xml.find("<w:sectPr").unwrap(),
    );
    xml[start..end].to_string()
}

fn refused(doc: &Document, command: Command) -> SessionError {
    let mut copy = EditingDocument::open(&doc.export().unwrap()).unwrap();
    let err = copy.run(command).expect_err("refused");
    assert!(copy.dirty_parts().is_empty(), "a refusal changes nothing");
    err
}

/// Each comment in the comments story: its ID, author, the comment it
/// replies to, and its text.
fn comments(doc: &Document) -> Vec<(String, String, Option<String>, String)> {
    let story = doc
        .stories()
        .find(|s| {
            matches!(
                s.kind,
                NodeKind::Story {
                    story: docx_engine::StoryKind::Comments,
                    ..
                }
            )
        })
        .expect("a comments story");
    story
        .children
        .iter()
        .filter_map(|&c| match &doc.node(c).unwrap().kind {
            NodeKind::Comment {
                comment_id,
                author,
                parent,
                ..
            } => {
                let text: Vec<String> = doc
                    .descendants(c)
                    .into_iter()
                    .filter_map(|n| match &doc.node(n).unwrap().kind {
                        NodeKind::Paragraph(_) => Some(doc.text(n).unwrap()),
                        _ => None,
                    })
                    .collect();
                Some((
                    comment_id.clone(),
                    author.clone().unwrap_or_default(),
                    parent.clone(),
                    text.join("\n"),
                ))
            }
            _ => None,
        })
        .collect()
}

/// The comments covering each run holding `needle`, as the inspection says.
fn covering(doc: &Document, needle: &str) -> Vec<String> {
    fn walk(v: &serde_json::Value, needle: &str, out: &mut Vec<String>) {
        if v["type"] == "run" {
            let text = v["children"].as_array().into_iter().flatten();
            if text
                .clone()
                .any(|t| t["text"].as_str().is_some_and(|t| t.contains(needle)))
            {
                let ids = v["comments"].as_array().into_iter().flatten();
                out.push(
                    ids.map(|i| i.as_str().unwrap())
                        .collect::<Vec<_>>()
                        .join(","),
                );
            }
        }
        for child in v["children"].as_array().into_iter().flatten() {
            walk(child, needle, out);
        }
    }
    let mut out = Vec::new();
    for story in stories(doc).as_array().unwrap() {
        walk(story, needle, &mut out);
    }
    out
}

#[test]
fn a_comment_on_a_document_without_comments_adds_its_parts() {
    let original = generated("simple-paragraphs");
    let mut doc = EditingDocument::open(&original).unwrap();
    assert_eq!(
        doc.stories().count(),
        1,
        "the made comments part is hidden while empty"
    );
    // The inspection still names it, for an editor to read when it changes.
    let story = doc.inspect().comments_story.expect("a comments story");
    assert_eq!(doc.inspect().stories.len(), 1);
    let first = text_node(&doc, "First");
    edit(
        &mut doc,
        comment(at(first, 0), at(first, 5), "Why first?\nSay more."),
    );
    let inspection = doc.inspect();
    assert_eq!(inspection.comments_story, Some(story));
    assert_eq!(inspection.stories[1].id, story);
    let out = doc.export().unwrap();
    assert_valid("comment", &out);
    assert_eq!(
        body(&out),
        concat!(
            r#"<w:p><w:commentRangeStart w:id="0"/><w:r><w:t xml:space="preserve">First</w:t></w:r>"#,
            r#"<w:commentRangeEnd w:id="0"/><w:r><w:commentReference w:id="0"/></w:r>"#,
            r#"<w:r><w:t xml:space="preserve"> paragraph.</w:t></w:r></w:p>"#,
            r#"<w:p><w:r><w:t>Second paragraph.</w:t></w:r></w:p><w:p><w:r><w:t>Third paragraph.</w:t></w:r></w:p>"#,
        )
    );
    let xml = part(&out, COMMENTS);
    assert!(
        xml.contains(r#"<w:comment w:id="0" w:author="Alice" w:date="2026-11-16T10:00:00Z" w:initials="A"><w:p xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" w14:paraId="#),
        "{xml}"
    );
    assert!(
        xml.contains(r#"<w:r><w:t xml:space="preserve">Why first?</w:t></w:r></w:p>"#),
        "{xml}"
    );
    assert!(
        xml.contains(
            r#"<w:r><w:t xml:space="preserve">Say more.</w:t></w:r></w:p></w:comment></w:comments>"#
        ),
        "{xml}"
    );
    let rels = part(&out, "word/_rels/document.xml.rels");
    assert!(rels.contains(r#"<Relationship Id="rIdComments" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/>"#), "{rels}");
    assert!(rels.contains(r#"<Relationship Id="rIdCommentsExtended" Type="http://schemas.microsoft.com/office/2011/relationships/commentsExtended" Target="commentsExtended.xml"/>"#), "{rels}");
    let types = part(&out, "[Content_Types].xml");
    assert!(types.contains(r#"<Override PartName="/word/comments.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"/>"#), "{types}");
    assert!(types.contains(r#"<Override PartName="/word/commentsExtended.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.commentsExtended+xml"/>"#), "{types}");
    let threads = part(&out, THREADS);
    assert!(
        threads.contains(r#"<w15:commentEx w15:paraId=""#)
            && threads.contains(r#" w15:done="0"/></w15:commentsEx>"#),
        "{threads}"
    );

    let reopened = Document::open(&out).unwrap();
    assert_eq!(
        comments(&reopened),
        [(
            "0".into(),
            "Alice".into(),
            None,
            "Why first?\nSay more.".into()
        )]
    );
    assert_eq!(covering(&reopened, "First"), ["0"]);
    assert_eq!(covering(&reopened, "paragraph."), ["", "", ""]);
    assert_eq!(
        covering(&doc, "First"),
        ["0"],
        "the open document says so too"
    );
}

#[test]
fn a_comment_inside_a_run_splits_it_and_spans_paragraphs() {
    let mut doc = EditingDocument::open(&generated("simple-paragraphs")).unwrap();
    let (first, third) = (text_node(&doc, "First"), text_node(&doc, "Third"));
    edit(&mut doc, comment(at(first, 6), at(third, 5), "Long range"));
    let out = doc.export().unwrap();
    assert_valid("spanning", &out);
    assert_eq!(
        body(&out),
        concat!(
            r#"<w:p><w:r><w:t xml:space="preserve">First </w:t></w:r><w:commentRangeStart w:id="0"/>"#,
            r#"<w:r><w:t xml:space="preserve">paragraph.</w:t></w:r></w:p>"#,
            r#"<w:p><w:r><w:t>Second paragraph.</w:t></w:r></w:p>"#,
            r#"<w:p><w:r><w:t xml:space="preserve">Third</w:t></w:r><w:commentRangeEnd w:id="0"/>"#,
            r#"<w:r><w:commentReference w:id="0"/></w:r><w:r><w:t xml:space="preserve"> paragraph.</w:t></w:r></w:p>"#,
        )
    );
    assert_eq!(covering(&doc, "Second"), ["0"]);
    assert_eq!(covering(&doc, "First"), [""]);
    // A paragraph inspected alone, as an editor redraws it, knows the
    // comments open before it.
    let second = paragraph_with(&doc, "Second");
    let view = serde_json::to_value(doc.inspect_node(second).unwrap()).unwrap();
    assert_eq!(view["children"][0]["comments"], serde_json::json!(["0"]));
}

#[test]
fn a_comment_at_one_place_starts_before_it_ends() {
    let mut doc = EditingDocument::open(&generated("simple-paragraphs")).unwrap();
    let paragraph = paragraph_with(&doc, "Second");
    let end = Point::End { end: paragraph };
    edit(&mut doc, comment(end, end, "Here"));
    let first = text_node(&doc, "First");
    edit(&mut doc, comment(at(first, 3), at(first, 3), "Inside"));
    let out = doc.export().unwrap();
    assert!(
        normalized(&body(&out)).contains(concat!(
            r#"<w:p><w:r><w:t>Second paragraph.</w:t></w:r><w:commentRangeStart w:id="0"/>"#,
            r#"<w:commentRangeEnd w:id="0"/><w:r><w:commentReference w:id="0"/></w:r></w:p>"#
        )),
        "{}",
        body(&out)
    );
    assert!(
        normalized(&body(&out)).starts_with(concat!(
            r#"<w:p><w:r><w:t>Fir</w:t></w:r><w:commentRangeStart w:id="1"/>"#,
            r#"<w:commentRangeEnd w:id="1"/><w:r><w:commentReference w:id="1"/></w:r>"#
        )),
        "{}",
        body(&out)
    );
}

#[test]
fn a_link_over_a_comment_leaves_its_reference_mark_unstyled() {
    let mut doc = EditingDocument::open(&generated("simple-paragraphs")).unwrap();
    let first = text_node(&doc, "First");
    edit(&mut doc, comment(at(first, 0), at(first, 5), "Why?"));
    let first = text_node(&doc, "First");
    let rest = text_node(&doc, " paragraph.");
    edit(
        &mut doc,
        Command::SetLink {
            from: at(first, 0),
            to: at(rest, 10),
            target: Some("https://example.com/".into()),
        },
    );
    let out = doc.export().unwrap();
    assert_valid("linked", &out);
    let body = body(&out);
    assert!(
        body.contains(concat!(
            r#"</w:hyperlink><w:commentRangeEnd w:id="0"/>"#,
            r#"<w:r><w:commentReference w:id="0"/></w:r><w:hyperlink "#,
        )),
        "{body}"
    );
}

#[test]
fn a_reply_joins_the_thread_of_the_first_comment() {
    let original = generated("comments");
    let mut doc = EditingDocument::open(&original).unwrap();
    assert_eq!(
        comments(&doc)
            .iter()
            .map(|c| c.2.clone())
            .collect::<Vec<_>>(),
        [None, Some("0".into()), None],
        "comment 1 replies to comment 0, as commentsExtended says"
    );
    // A reply to the reply answers the first comment, as Word threads it.
    edit(&mut doc, reply("1", "Agreed."));
    let out = doc.export().unwrap();
    assert_valid("reply", &out);

    let threads = part(&out, THREADS);
    assert!(
        threads.contains(r#"w15:paraId="10000002" w15:paraIdParent="10000001" w15:done="0"/><w15:commentEx w15:paraId=""#)
            && threads.contains(r#" w15:paraIdParent="10000001" w15:done="0"/></w15:commentsEx>"#),
        "{threads}"
    );
    let reopened = Document::open(&out).unwrap();
    let last = comments(&reopened).pop().unwrap();
    assert_eq!(
        last,
        (
            "3".into(),
            "Alice".into(),
            Some("0".into()),
            "Agreed.".into()
        )
    );
    assert_eq!(covering(&reopened, "discussed"), ["0,1,3"]);

    // A reply to a comment a command made threads the same way.
    let mut doc = EditingDocument::open(&generated("simple-paragraphs")).unwrap();
    let first = text_node(&doc, "First");
    edit(&mut doc, comment(at(first, 0), at(first, 5), "Why?"));
    edit(&mut doc, reply("0", "Because."));
    edit(&mut doc, reply("1", "Fine."));
    let out = doc.export().unwrap();
    assert_valid("replies", &out);

    let reopened = Document::open(&out).unwrap();
    let parents: Vec<Option<String>> = comments(&reopened).into_iter().map(|c| c.2).collect();
    assert_eq!(parents, [None, Some("0".into()), Some("0".into())]);
    assert_eq!(covering(&reopened, "First"), ["0,1,2"]);
}

#[test]
fn comment_ids_go_above_every_comment_and_revisions_above_them() {
    let mut doc = EditingDocument::open(&generated("comments")).unwrap();
    let wanted = |plain, id| Command::AddComment {
        from: at(plain, 0),
        to: at(plain, 1),
        text: "x".into(),
        by: alice(),
        initials: None,
        id: Some(id),
    };
    let para = paragraph_with(&doc, "A plain");
    let first = |doc: &Document| {
        doc.descendants(para)
            .into_iter()
            .find(|&n| matches!(&doc.node(n).unwrap().kind, NodeKind::Text { .. }))
            .unwrap()
    };
    for id in [7, 1, 5] {
        let point = first(&doc);
        edit(&mut doc, wanted(point, id));
    }
    let plain = first(&doc);
    edit(&mut doc, comment(at(plain, 0), at(plain, 1), "y"));
    let ids: Vec<String> = comments(&doc).into_iter().map(|c| c.0).collect();
    assert_eq!(ids, ["0", "1", "2", "5", "7", "8", "9"], "in ID order");
    let plain = first(&doc);
    edit(
        &mut doc,
        Command::ReplaceRange {
            from: at(plain, 0),
            to: at(plain, 0),
            text: "Z".into(),
            track: Some(alice()),
        },
    );
    let out = doc.export().unwrap();
    assert_valid("ids", &out);
    assert!(
        body(&out).contains(r#"<w:ins w:id="10" w:author="Alice""#),
        "{}",
        body(&out)
    );
}

#[test]
fn undo_takes_back_a_comment_and_its_parts() {
    for (name, command) in [
        ("simple-paragraphs", None),
        ("comments", Some(reply("0", "Undo me."))),
        ("comments", None),
    ] {
        let original = generated(name);
        let mut doc = EditingDocument::open(&original).unwrap();
        let command = command.unwrap_or_else(|| {
            let first = first_editable_text(&doc).unwrap();
            comment(at(first, 1), at(first, 3), "Undo me.")
        });
        edit(&mut doc, command);
        assert_editor_undo(name, &original, &mut doc);
    }
}

#[test]
fn comments_that_cannot_be_written_are_refused() {
    let doc = Document::open(&generated("comments")).unwrap();
    let plain = text_node(&doc, "A plain");
    let err = refused(&doc, reply("2", "Hm."));
    assert!(err.to_string().contains("no paragraph ID"), "{err}");
    let err = refused(&doc, reply("9", "Hm."));
    assert!(err.to_string().contains("is not a comment"), "{err}");
    let err = refused(&doc, comment(at(plain, 3), at(plain, 1), "Backwards"));
    assert!(err.to_string().contains("ends before it starts"), "{err}");
    let err = refused(&doc, comment(at(plain, 0), at(plain, 1), "A\ttab"));
    assert!(
        matches!(err, SessionError::Edit(EditError::InvalidText { .. })),
        "{err}"
    );
    let blank = Command::AddComment {
        from: at(plain, 0),
        to: at(plain, 1),
        text: "x".into(),
        by: Track {
            author: " ".into(),
            date: "2026-11-16T10:00:00Z".into(),
        },
        initials: None,
        id: None,
    };
    assert!(matches!(
        refused(&doc, blank),
        SessionError::Edit(EditError::Track { .. })
    ));

    let stories = Document::open(&generated("header-footer")).unwrap();
    let (header, body) = (
        text_node(&stories, "Header text"),
        text_node(&stories, "Body with"),
    );
    let err = refused(&stories, comment(at(header, 0), at(header, 1), "Hm."));
    assert!(err.to_string().contains("headers and footers"), "{err}");
    let err = refused(&stories, comment(at(body, 0), at(header, 1), "Hm."));
    assert!(err.to_string().contains("another story"), "{err}");

    let fields = Document::open(&generated("complex-mixed")).unwrap();
    let result = fields
        .paragraphs()
        .into_iter()
        .flat_map(|p| fields.descendants(p))
        .find(|&n| fields.inspect_node(n).is_some_and(|v| v.field.is_some()))
        .expect("a field result");
    refused(&fields, comment(at(result, 0), at(result, 0), "In a field"));
}
