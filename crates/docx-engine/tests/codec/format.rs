use super::common;

use common::*;
use docx_engine::{Command, Document, Format, NodeId, NodeKind, Point, Support};

const DOCUMENT: &str = "word/document.xml";

fn format_range(from: Point, to: Point, format: Format) -> Command {
    Command::FormatRange { from, to, format }
}

const BOLD: Format = Format {
    bold: Some(true),
    italic: None,
    underline: None,
};

/// The run that holds `text`, and its effective bold, italic and underline.
fn shown(doc: &Document, text: NodeId) -> (bool, bool, bool) {
    let run = doc.node(text).unwrap().parent.unwrap();
    let props = doc.effective_run_props(run);
    (
        props.bold.unwrap_or(false),
        props.italic.unwrap_or(false),
        props.underline.as_deref().is_some_and(|u| u != "none"),
    )
}

#[test]
fn bold_in_the_middle_of_a_run_splits_it_in_three() {
    let bytes = generated("simple-paragraphs");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let first = text_node(&doc, "First");
    let paragraph = paragraph_with(&doc, "First");
    let text = doc.text(paragraph).unwrap();
    edit(&mut doc, format_range(at(first, 6), at(first, 15), BOLD));
    assert_eq!(doc.text(paragraph).unwrap(), text);
    let out = doc.export().unwrap();
    let xml = part(&out, DOCUMENT);
    assert!(
        xml.contains(
            r#"<w:r><w:rPr><w:b/><w:bCs/></w:rPr><w:t xml:space="preserve">paragraph</w:t></w:r>"#
        ),
        "{xml}"
    );
    let reopened = reopen("simple-paragraphs", &doc);
    let runs: Vec<(String, bool)> = reopened
        .descendants(paragraph_with(&reopened, "First"))
        .into_iter()
        .filter_map(|id| match &reopened.node(id).unwrap().kind {
            NodeKind::Text { text } => Some((text.clone(), shown(&reopened, id).0)),
            _ => None,
        })
        .collect();
    assert_eq!(
        runs,
        [
            ("First ".to_string(), false),
            ("paragraph".to_string(), true),
            (".".to_string(), false),
        ]
    );
    assert_editor_undo("simple-paragraphs", &bytes, &mut doc);
}

#[test]
fn unbolding_a_heading_writes_bold_off_and_bolding_it_again_removes_that() {
    let bytes = generated("headings-styles");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let chapter = text_node(&doc, "Chapter one");
    assert_eq!(shown(&doc, chapter), (true, false, false));
    let off = Format {
        bold: Some(false),
        ..Format::default()
    };
    edit(&mut doc, format_range(at(chapter, 0), at(chapter, 11), off));
    let chapter = text_node(&doc, "Chapter one");
    assert_eq!(shown(&doc, chapter), (false, false, false));
    let xml = part(&doc.export().unwrap(), DOCUMENT);
    assert!(
        normalized(&xml).contains(
            r#"<w:r><w:rPr><w:b w:val="0"/><w:bCs w:val="0"/></w:rPr><w:t>Chapter one</w:t></w:r>"#
        ),
        "{xml}"
    );
    let reopened = reopen("headings-styles", &doc);
    assert_eq!(
        shown(&reopened, text_node(&reopened, "Chapter one")),
        (false, false, false)
    );

    // The style gives bold, so bold again needs no property of its own.
    edit(
        &mut doc,
        format_range(at(chapter, 0), at(chapter, 11), BOLD),
    );
    assert_eq!(
        normalized(&part(&doc.export().unwrap(), DOCUMENT)),
        normalized(&part(&bytes, DOCUMENT))
    );
    let chapter = text_node(&doc, "Chapter one");
    edit(&mut doc, format_range(at(chapter, 0), at(chapter, 11), off));
    assert!(doc.0.undo());
    assert_eq!(
        normalized(&part(&doc.export().unwrap(), DOCUMENT)),
        normalized(&part(&bytes, DOCUMENT))
    );
    assert!(doc.0.redo());
    assert!(!shown(&doc, text_node(&doc, "Chapter one")).0);
}

#[test]
fn underline_is_removed_and_added() {
    let bytes = generated("rich-text");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let underlined = text_node(&doc, "underlined");
    let off = Format {
        underline: Some(false),
        ..Format::default()
    };
    edit(
        &mut doc,
        format_range(at(underlined, 0), at(underlined, 10), off),
    );
    let xml = part(&doc.export().unwrap(), DOCUMENT);
    assert!(
        normalized(&xml).contains("<w:r><w:t>underlined</w:t></w:r>"),
        "{xml}"
    );

    let on = Format {
        underline: Some(true),
        ..Format::default()
    };
    let and = text_node(&doc, " and ");
    edit(&mut doc, format_range(at(and, 1), at(and, 4), on));
    let xml = part(&doc.export().unwrap(), DOCUMENT);
    assert!(
        xml.contains(
            r#"<w:r><w:rPr><w:u w:val="single"/></w:rPr><w:t xml:space="preserve">and</w:t></w:r>"#
        ),
        "{xml}"
    );
    reopen("rich-text", &doc);
}

#[test]
fn properties_go_where_the_schema_orders_them_and_the_rest_keep_their_bytes() {
    let bytes = generated("rich-text");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let coloured = text_node(&doc, "Coloured text");
    let all = Format {
        bold: Some(true),
        italic: Some(true),
        underline: Some(true),
    };
    edit(
        &mut doc,
        format_range(at(coloured, 0), at(coloured, 13), all),
    );
    let coloured = text_node(&doc, "Coloured text");
    assert_eq!(shown(&doc, coloured), (true, true, true));
    let xml = part(&doc.export().unwrap(), DOCUMENT);
    assert!(
        xml.contains(concat!(
            r#"<w:rPr><w:rFonts w:ascii="Georgia" w:hAnsi="Georgia"/><w:b/><w:bCs/><w:i/><w:iCs/>"#,
            r#"<w:color w:val="C00000"/><w:sz w:val="28"/><w:u w:val="single"/></w:rPr>"#
        )),
        "{xml}"
    );
    let props = doc.effective_run_props(doc.node(coloured).unwrap().parent.unwrap());
    assert_eq!(props.color.as_deref(), Some("C00000"));
    assert_eq!(props.size_half_points, Some(28));
    reopen("rich-text", &doc);
    assert_editor_undo("rich-text", &bytes, &mut doc);
}

#[test]
fn bold_across_runs_changes_only_those_without_it() {
    let bytes = generated("rich-text");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let (bold, underlined) = (text_node(&doc, "Bold"), text_node(&doc, "underlined"));
    edit(&mut doc, format_range(at(bold, 0), at(underlined, 5), BOLD));
    // The bold run is left alone; the others in the range become bold.
    let paragraph = paragraph_with(&doc, "Bold");
    let runs: Vec<(String, (bool, bool, bool))> = doc
        .descendants(paragraph)
        .into_iter()
        .filter_map(|id| match &doc.node(id).unwrap().kind {
            NodeKind::Text { text } => Some((text.clone(), shown(&doc, id))),
            _ => None,
        })
        .collect();
    let want = [
        ("Bold", (true, false, false)),
        (" and ", (true, false, false)),
        ("italic", (true, true, false)),
        (" and ", (true, false, false)),
        ("under", (true, false, true)),
        ("lined", (false, false, true)),
        (".", (false, false, false)),
    ];
    let want: Vec<(String, (bool, bool, bool))> =
        want.iter().map(|(t, s)| (t.to_string(), *s)).collect();
    assert_eq!(runs, want);
    reopen("rich-text", &doc);
    assert_editor_undo("rich-text", &bytes, &mut doc);
}

#[test]
fn formatting_tabs_and_breaks_carries_the_run_across_them() {
    let bytes = generated("rich-text");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let (before, next) = (text_node(&doc, "Before tab"), text_node(&doc, "next line"));
    edit(&mut doc, format_range(at(before, 7), at(next, 4), BOLD));
    let xml = part(&doc.export().unwrap(), DOCUMENT);
    assert!(
        xml.contains(concat!(
            r#"<w:r><w:t xml:space="preserve">Before </w:t></w:r>"#,
            r#"<w:r><w:rPr><w:b/><w:bCs/></w:rPr><w:t xml:space="preserve">tab</w:t><w:tab/><w:t>after tab</w:t><w:br/><w:t xml:space="preserve">next</w:t></w:r>"#,
            r#"<w:r><w:t xml:space="preserve"> line</w:t></w:r>"#
        )),
        "{xml}"
    );
    reopen("rich-text", &doc);
    assert_editor_undo("rich-text", &bytes, &mut doc);
}

#[test]
fn formatting_what_already_has_it_changes_nothing() {
    let bytes = generated("rich-text");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let bold = text_node(&doc, "Bold");
    edit(&mut doc, format_range(at(bold, 1), at(bold, 3), BOLD));
    assert!(!doc.is_dirty());
    assert!(!doc.0.undo());
}

#[test]
fn format_range_refuses_and_changes_nothing() {
    fn refused(name: &str, doc: &mut EditingDocument, command: Command, reason: &str) {
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
    }
    let italic = Format {
        italic: Some(true),
        ..Format::default()
    };

    let mut doc = EditingDocument::open(&generated("tracked-changes")).unwrap();
    let newly = text_node(&doc, "newly bold");
    let plain = first_editable_text(&doc).unwrap();
    refused(
        "tracked-changes",
        &mut doc,
        format_range(at(newly, 0), at(newly, 3), italic),
        "inside text with a tracked formatting change",
    );
    // Bold is what it shows already, so there is nothing to refuse.
    edit(&mut doc, format_range(at(newly, 0), at(newly, 5), BOLD));
    assert!(!doc.is_dirty());
    refused(
        "tracked-changes",
        &mut doc,
        format_range(at(newly, 3), at(plain, 0), italic),
        "ends before it starts",
    );

    let mut doc = EditingDocument::open(&generated("header-footer")).unwrap();
    let editable_in = |doc: &Document, root: NodeId| {
        doc.descendants(root).into_iter().find(|&id| {
            let node = doc.node(id).unwrap();
            node.support == Support::Editable
                && matches!(&node.kind, NodeKind::Text { text } if !text.is_empty())
        })
    };
    let roots: Vec<NodeId> = doc.stories().map(|s| s.id).collect();
    let body = editable_in(&doc, roots[0]).unwrap();
    let header = roots[1..]
        .iter()
        .find_map(|&r| editable_in(&doc, r))
        .unwrap();
    refused(
        "header-footer",
        &mut doc,
        format_range(at(body, 0), at(header, 1), BOLD),
        "another story",
    );
}

#[test]
fn every_fixture_formats_ranges_undoes_and_redoes_exactly() {
    let all = Format {
        bold: Some(true),
        italic: Some(true),
        underline: Some(true),
    };
    let mut formatted = 0;
    for (name, bytes) in all_fixtures() {
        let doc = Document::open(&bytes).unwrap();
        let editable = |doc: &Document, p: NodeId| {
            doc.descendants(p).into_iter().find(|&id| {
                let node = doc.node(id).unwrap();
                node.support == Support::Editable
                    && matches!(&node.kind, NodeKind::Text { text } if text.chars().count() >= 2)
            })
        };
        let paragraphs = doc.paragraphs();
        for pair in paragraphs.windows(3) {
            let (Some(from), Some(to)) = (editable(&doc, pair[0]), editable(&doc, pair[2])) else {
                continue;
            };
            let mut doc = EditingDocument::open(&bytes).unwrap();
            let Ok(()) = doc.0.apply(&format_range(at(from, 1), at(to, 1), all)) else {
                assert_eq!(
                    stories(&doc),
                    stories(&Document::open(&bytes).unwrap()),
                    "{name}: refusal changed the document"
                );
                assert!(!doc.is_dirty(), "{name}: refusal dirtied the document");
                continue;
            };
            formatted += 1;
            // Every editable character the range covers now shows all three.
            for p in &pair[1..2] {
                for id in doc.descendants(*p) {
                    let node = doc.node(id).unwrap();
                    let in_run = node
                        .parent
                        .is_some_and(|r| matches!(doc.node(r).unwrap().kind, NodeKind::Run(_)));
                    if node.support == Support::Editable
                        && in_run
                        && matches!(&node.kind, NodeKind::Text { text } if !text.is_empty())
                    {
                        assert_eq!(shown(&doc, id), (true, true, true), "{name}: {id}");
                    }
                }
            }
            let text = |d: &Document| -> Vec<String> {
                pair.iter().map(|&p| d.text(p).unwrap()).collect()
            };
            let before = text(&Document::open(&bytes).unwrap());
            assert_eq!(text(&doc), before, "{name}: formatting changed the text");
            reopen(&name, &doc);
            assert_editor_undo(&name, &bytes, &mut doc);
        }
    }
    assert!(formatted > 50, "only {formatted} ranges formatted");
}

#[test]
fn format_range_round_trips_through_json_and_is_listed() {
    let n = |s: &str| s.parse::<NodeId>().unwrap();
    let command = format_range(
        Point::Text {
            node: n("n3"),
            offset: 2,
        },
        Point::End { end: n("n5") },
        Format {
            bold: Some(true),
            underline: Some(false),
            ..Format::default()
        },
    );
    let json = r#"{"command":"format_range","from":{"node":"n3","offset":2},"to":{"end":"n5"},"format":{"bold":true,"underline":false}}"#;
    assert_eq!(serde_json::to_string(&command).unwrap(), json);
    assert_eq!(serde_json::from_str::<Command>(json).unwrap(), command);

    let doc = Document::open(&generated("simple-paragraphs")).unwrap();
    let listed = doc.inspect_json()["commands"].clone();
    for name in ["replace_range", "format_range"] {
        assert!(
            listed.as_array().unwrap().iter().any(|c| c == name),
            "{name} is not in {listed}"
        );
    }
}

#[test]
fn a_run_with_a_tracked_formatting_change_is_formatted_whole_and_keeps_it() {
    let change = r#"<w:rPrChange w:id="7" w:author="Alice" w:date="2026-11-01T10:05:00Z"><w:rPr><w:i/><w:u w:val="double"/></w:rPr></w:rPrChange>"#;
    let bytes = with_body(&format!(
        r#"<w:p><w:r><w:t xml:space="preserve">Plain </w:t></w:r><w:r><w:rPr><w:b/>{change}</w:rPr><w:t>changed</w:t></w:r></w:p>"#
    ));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let (plain, changed) = (text_node(&doc, "Plain"), text_node(&doc, "changed"));
    let all = Format {
        bold: Some(true),
        italic: Some(true),
        underline: Some(true),
    };
    edit(&mut doc, format_range(at(plain, 0), at(changed, 7), all));
    let changed = text_node(&doc, "changed");
    assert_eq!(shown(&doc, changed), (true, true, true));
    let xml = part(&doc.export().unwrap(), DOCUMENT);
    assert!(
        xml.contains(&change.replace(r#"w:id="7""#, r#"w:id="8""#)),
        "{xml}"
    );
    let reopened = reopen("tracked format", &doc);
    let run = reopened
        .node(text_node(&reopened, "changed"))
        .unwrap()
        .parent
        .unwrap();
    assert!(
        matches!(&reopened.node(run).unwrap().kind, NodeKind::Run(info) if info.format_revision)
    );
    assert_editor_undo("tracked format", &bytes, &mut doc);
}

fn set_style(from: Point, to: Point, style: &str) -> Command {
    Command::SetParagraphStyle {
        from,
        to,
        style: style.into(),
    }
}

/// The paragraph's style ID, the default style's when it names none.
fn style_of(doc: &Document, paragraph: NodeId) -> Option<String> {
    let NodeKind::Paragraph(info) = &doc.node(paragraph).unwrap().kind else {
        panic!("{paragraph} is not a paragraph");
    };
    let default = doc.styles().default_paragraph_style().map(|s| s.id.clone());
    info.style.clone().or(default)
}

#[test]
fn a_paragraph_style_goes_first_in_its_properties_and_the_rest_keep_their_bytes() {
    let bytes = generated("headings-styles");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let (chapter, direct) = (
        text_node(&doc, "Chapter one"),
        text_node(&doc, "Direct outline"),
    );
    edit(
        &mut doc,
        set_style(at(chapter, 3), at(direct, 3), "Heading1"),
    );
    // "Chapter one" is a Heading 1 already.
    let xml = part(&doc.export().unwrap(), DOCUMENT);
    for expected in [
        r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Body text"#,
        r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Section 1.1"#,
        r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/><w:outlineLvl w:val="2"/></w:pPr><w:r><w:t>Direct"#,
    ] {
        assert!(xml.contains(expected), "{expected} in {xml}");
    }
    // The style's bold now shows in the text it holds.
    assert_eq!(
        shown(&doc, text_node(&doc, "Body text")),
        (true, false, false)
    );
    let reopened = reopen("headings-styles", &doc);
    for needle in [
        "Chapter one",
        "Body text",
        "Section 1.1",
        "Plain then",
        "Direct",
    ] {
        let p = paragraph_with(&reopened, needle);
        assert_eq!(
            style_of(&reopened, p).as_deref(),
            Some("Heading1"),
            "{needle}"
        );
    }
    assert_eq!(
        style_of(&reopened, paragraph_with(&reopened, "Document title")).as_deref(),
        Some("Title")
    );
    assert_editor_undo("headings-styles", &bytes, &mut doc);
}

#[test]
fn the_default_style_removes_the_style_and_an_emptied_ppr() {
    let bytes = generated("headings-styles");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let title = text_node(&doc, "Document title");
    edit(&mut doc, set_style(at(title, 0), at(title, 0), "Normal"));
    let paragraph = paragraph_with(&doc, "Document title");
    assert!(
        matches!(&doc.node(paragraph).unwrap().kind, NodeKind::Paragraph(info) if info.style.is_none())
    );
    let xml = part(&doc.export().unwrap(), DOCUMENT);
    assert!(
        xml.contains("<w:body><w:p><w:r><w:t>Document title</w:t></w:r></w:p>"),
        "{xml}"
    );
    reopen("headings-styles", &doc);
    assert_editor_undo("headings-styles", &bytes, &mut doc);
}

#[test]
fn a_range_across_a_table_styles_the_paragraphs_in_its_cells() {
    let bytes = generated("table");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let (before, after) = (text_node(&doc, "Before"), text_node(&doc, "After"));
    edit(&mut doc, set_style(at(before, 0), at(after, 0), "Heading2"));
    let reopened = reopen("table", &doc);
    let styled: Vec<Option<String>> = reopened
        .paragraphs()
        .into_iter()
        .map(|p| style_of(&reopened, p))
        .collect();
    assert_eq!(styled.len(), 12);
    assert!(
        styled.iter().all(|s| s.as_deref() == Some("Heading2")),
        "{styled:?}"
    );
    assert_editor_undo("table", &bytes, &mut doc);
}

#[test]
fn a_paragraph_split_after_a_style_change_keeps_the_new_style() {
    let bytes = generated("headings-styles");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let body = text_node(&doc, "Body text");
    edit(&mut doc, set_style(at(body, 0), at(body, 0), "Heading2"));
    edit(&mut doc, Command::SplitParagraph { at: at(body, 4) });
    let reopened = reopen("headings-styles", &doc);
    for needle in ["Body", " text under"] {
        let p = paragraph_with(&reopened, needle);
        assert_eq!(
            style_of(&reopened, p).as_deref(),
            Some("Heading2"),
            "{needle}"
        );
    }
}

#[test]
fn styling_what_has_the_style_changes_nothing() {
    let bytes = generated("headings-styles");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let (chapter, body) = (text_node(&doc, "Chapter one"), text_node(&doc, "Body text"));
    edit(
        &mut doc,
        set_style(at(chapter, 0), at(chapter, 5), "Heading1"),
    );
    edit(&mut doc, set_style(at(body, 0), at(body, 5), "Normal"));
    assert!(!doc.is_dirty());
}

#[test]
fn set_paragraph_style_refuses_and_changes_nothing() {
    fn refused(name: &str, doc: &mut EditingDocument, command: Command, reason: &str) {
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
    }
    let mut doc = EditingDocument::open(&generated("headings-styles")).unwrap();
    let (title, body) = (
        text_node(&doc, "Document title"),
        text_node(&doc, "Body text"),
    );
    let no_style = "no paragraph style with that ID";
    refused(
        "headings-styles",
        &mut doc,
        set_style(at(title, 0), at(body, 0), "Nope"),
        no_style,
    );
    // A character style is not a paragraph style.
    refused(
        "headings-styles",
        &mut doc,
        set_style(at(title, 0), at(body, 0), "Emphasis"),
        no_style,
    );
    refused(
        "headings-styles",
        &mut doc,
        set_style(at(body, 0), at(title, 0), "Heading1"),
        "ends before it starts",
    );

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
        set_style(at(body, 0), at(header, 0), "Heading1"),
        "another story",
    );
}

#[test]
fn every_fixture_styles_paragraphs_undoes_and_redoes_exactly() {
    let mut styled = 0;
    for (name, bytes) in all_fixtures() {
        let doc = Document::open(&bytes).unwrap();
        let Some(style) = doc
            .styles()
            .styles
            .values()
            .filter(|s| s.kind == "paragraph" && !s.is_default)
            .map(|s| s.id.clone())
            .min_by_key(|id| (id != "Heading1", id.clone()))
        else {
            continue;
        };
        let first_text = |doc: &Document, p: NodeId| {
            doc.descendants(p).into_iter().find(|&id| {
                let node = doc.node(id).unwrap();
                node.support == Support::Editable
                    && matches!(&node.kind, NodeKind::Text { text } if !text.is_empty())
            })
        };
        let paragraphs = doc.paragraphs();
        for pair in paragraphs.windows(3) {
            let (Some(from), Some(to)) = (first_text(&doc, pair[0]), first_text(&doc, pair[2]))
            else {
                continue;
            };
            let mut doc = EditingDocument::open(&bytes).unwrap();
            let Ok(()) = doc.0.apply(&set_style(at(from, 0), at(to, 0), &style)) else {
                assert_eq!(
                    stories(&doc),
                    stories(&Document::open(&bytes).unwrap()),
                    "{name}: refusal changed the document"
                );
                assert!(!doc.is_dirty(), "{name}: refusal dirtied the document");
                continue;
            };
            styled += 1;
            let text = |d: &Document| -> Vec<String> {
                pair.iter().map(|&p| d.text(p).unwrap()).collect()
            };
            let before = text(&Document::open(&bytes).unwrap());
            assert_eq!(text(&doc), before, "{name}: styling changed the text");
            let reopened = reopen(&name, &doc);
            for (&p, &q) in doc.paragraphs().iter().zip(&reopened.paragraphs()) {
                assert_eq!(style_of(&doc, p), style_of(&reopened, q), "{name}: {p}");
            }
            for &p in pair {
                assert_eq!(
                    style_of(&doc, p).as_deref(),
                    Some(style.as_str()),
                    "{name}: {p}"
                );
            }
            assert_editor_undo(&name, &bytes, &mut doc);
        }
    }
    assert!(styled > 50, "only {styled} ranges styled");
}

#[test]
fn set_paragraph_style_round_trips_through_json_and_is_listed() {
    let n = |s: &str| s.parse::<NodeId>().unwrap();
    let command = set_style(
        Point::Text {
            node: n("n3"),
            offset: 2,
        },
        Point::End { end: n("n5") },
        "Heading1",
    );
    let json = r#"{"command":"set_paragraph_style","from":{"node":"n3","offset":2},"to":{"end":"n5"},"style":"Heading1"}"#;
    assert_eq!(serde_json::to_string(&command).unwrap(), json);
    assert_eq!(serde_json::from_str::<Command>(json).unwrap(), command);

    let doc = Document::open(&generated("simple-paragraphs")).unwrap();
    let listed = doc.inspect_json()["commands"].clone();
    assert!(listed
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c == "set_paragraph_style"));
}

#[test]
fn source_tabs_and_untouched_text_leaves_survive_an_edit() {
    let bytes = with_body("<w:p><w:r><w:t>A\tB</w:t><!--gap--><w:t xmlns:x=\"urn:test\" x:keep=\"yes\">Next</w:t><w:t xml:space=\"preserve\"/></w:r><w:r><w:rPr><w:i/></w:rPr></w:r></w:p>");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let leaf = text_node(&doc, "A\tB");
    edit(
        &mut doc,
        Command::InsertText {
            at: at(leaf, 1),
            text: "X".into(),
        },
    );
    let leaf = text_node(&doc, "AX\tB");
    edit(&mut doc, format_range(at(leaf, 0), at(leaf, 1), BOLD));
    let bytes = doc.export().unwrap();
    assert_valid("source tab", &bytes);
    let xml = part(&bytes, DOCUMENT);
    assert!(
        xml.contains("<!--gap--><w:t xmlns:x=\"urn:test\" x:keep=\"yes\">Next</w:t>"),
        "{xml}"
    );
    assert!(xml.contains(r#"<w:t xml:space="preserve"/>"#), "{xml}");
    assert!(xml.contains("<w:r><w:rPr><w:i/></w:rPr></w:r>"), "{xml}");
    assert_eq!(texts(&Document::open(&bytes).unwrap()), ["AX\tBNext"]);
}

#[test]
fn typing_beside_a_hyperlink_keeps_its_field_in_its_namespace_scope() {
    let bytes = with_body(
        r#"<w:p><w:r><w:t>Before</w:t></w:r><w:hyperlink w:anchor="Here" xmlns:ext="urn:field"><w:fldSimple w:instr="DATE" ext:keep="yes"><w:r><w:t>today</w:t></w:r></w:fldSimple></w:hyperlink></w:p>"#,
    );
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let first = text_node(&doc, "Before");
    edit(
        &mut doc,
        Command::InsertText {
            at: at(first, 6),
            text: "!".into(),
        },
    );
    let bytes = doc.export().unwrap();
    assert_valid("field namespace", &bytes);
    let xml = part(&bytes, DOCUMENT);
    assert!(
        xml.contains(r#"<w:hyperlink w:anchor="Here" xmlns:ext="urn:field"><w:fldSimple"#),
        "{xml}"
    );
    Document::open(&bytes).unwrap();
}

#[test]
fn typing_at_an_empty_source_run_uses_its_format() {
    let bytes = with_body("<w:p><w:r><w:t>A</w:t></w:r><w:r><w:rPr><w:i/></w:rPr></w:r></w:p>");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let run = *doc
        .node(doc.paragraphs()[0])
        .unwrap()
        .children
        .last()
        .unwrap();
    edit(
        &mut doc,
        Command::InsertText {
            at: Point::Start { start: run },
            text: "X".into(),
        },
    );
    let reopened = Document::open(&doc.export().unwrap()).unwrap();
    let leaf = text_node(&reopened, "X");
    assert!(shown(&reopened, leaf).1);
    assert_eq!(texts(&reopened), ["AX"]);
}

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

#[test]
fn unprefixed_properties_keep_the_word_namespace_of_their_attributes() {
    let bytes = with_body(&format!(
        r#"<p xmlns="{W}"><pPr xmlns:w="urn:extension"><keepNext/></pPr><r><rPr xmlns:w="urn:extension"><i/></rPr><t>Target</t></r></p>"#
    ));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let text = text_node(&doc, "Target");
    edit(&mut doc, set_style(at(text, 0), at(text, 0), "Heading1"));
    let text = text_node(&doc, "Target");
    let underline = Format {
        underline: Some(true),
        ..Format::default()
    };
    edit(&mut doc, format_range(at(text, 0), at(text, 6), underline));

    let reopened = reopen("rebound attribute prefix", &doc);
    for view in [&*doc, &reopened] {
        let paragraph = paragraph_with(view, "Target");
        assert_eq!(style_of(view, paragraph).as_deref(), Some("Heading1"));
        assert!(shown(view, text_node(view, "Target")).2);
    }
}

#[test]
fn properties_are_found_by_namespace_and_keep_their_own_markup() {
    // Another prefix for WordprocessingML, declared on the paragraph.
    let base = with_body(&format!(
        r#"<w:p xmlns:x="{W}"><w:r><x:rPr><x:i/></x:rPr><w:t>Slanted words</w:t></w:r></w:p>"#
    ));
    let mut doc = EditingDocument::open(&base).unwrap();
    let text = text_node(&doc, "Slanted");
    assert_eq!(shown(&doc, text), (false, true, false));
    edit(&mut doc, format_range(at(text, 0), at(text, 13), BOLD));
    let text = text_node(&doc, "Slanted");
    assert_eq!(shown(&doc, text), (true, true, false), "live");
    let xml = part(&doc.export().unwrap(), DOCUMENT);
    assert!(
        xml.contains("<w:r><x:rPr><w:b/><w:bCs/><x:i/></x:rPr><w:t>Slanted words</w:t></w:r>"),
        "{xml}"
    );
    let reopened = reopen("another prefix", &doc);
    let text = text_node(&reopened, "Slanted");
    assert_eq!(shown(&reopened, text), (true, true, false), "reopened");

    // A self-closing properties element keeps its attributes.
    let base = with_body(
        r#"<w:p><w:r><w:rPr xmlns:e="urn:example" e:kept="yes"/><w:t>Plain words</w:t></w:r></w:p>"#,
    );
    let mut doc = EditingDocument::open(&base).unwrap();
    let text = text_node(&doc, "Plain");
    edit(&mut doc, format_range(at(text, 0), at(text, 11), BOLD));
    let xml = part(&doc.export().unwrap(), DOCUMENT);
    assert!(
        xml.contains(
            r#"<w:rPr xmlns:e="urn:example" e:kept="yes"><w:b/><w:bCs/></w:rPr><w:t>Plain words</w:t>"#
        ),
        "{xml}"
    );
    let reopened = reopen("self-closing properties", &doc);
    let text = text_node(&reopened, "Plain");
    assert_eq!(shown(&reopened, text), (true, false, false));
}

#[test]
fn a_split_copies_a_property_written_with_an_end_tag() {
    let base = with_body(
        r#"<w:p><w:pPr><w:keepNext></w:keepNext><w:jc w:val="center"/></w:pPr><w:r><w:t>Kept together</w:t></w:r></w:p>"#,
    );
    let mut doc = EditingDocument::open(&base).unwrap();
    let text = text_node(&doc, "Kept");
    edit(
        &mut doc,
        Command::ReplaceRange {
            from: at(text, 4),
            to: at(text, 4),
            text: "\n".into(),
            track: None,
        },
    );
    let kept = |doc: &Document| -> Vec<bool> {
        doc.paragraphs()
            .into_iter()
            .map(|p| doc.effective_paragraph(p).keep_next)
            .collect()
    };
    assert_eq!(kept(&doc), [true, true], "live");
    assert_eq!(kept(&reopen("split", &doc)), [true, true], "reopened");
}

#[test]
fn formatting_reads_the_word_attribute_and_keeps_a_comment_beside_a_property() {
    let bytes = with_body(concat!(
        r#"<w:p><w:r><w:rPr><w:b xmlns:x="urn:extension" x:val="off" w:val="1"/></w:rPr><w:t>bold</w:t></w:r>"#,
        r#"<w:r><w:rPr><!--keep--><w:b/></w:rPr><w:t>plain</w:t></w:r></w:p>"#,
    ));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let bold = text_node(&doc, "bold");
    assert_eq!(shown(&doc, bold), (true, false, false));
    let italic = Format {
        bold: None,
        italic: Some(true),
        underline: None,
    };
    edit(&mut doc, format_range(at(bold, 0), at(bold, 4), italic));
    // The extension's attribute of the same local name decides nothing.
    assert_eq!(shown(&doc, text_node(&doc, "bold")), (true, true, false));

    let plain = text_node(&doc, "plain");
    let unbold = Format {
        bold: Some(false),
        italic: None,
        underline: None,
    };
    edit(&mut doc, format_range(at(plain, 0), at(plain, 5), unbold));
    assert_eq!(shown(&doc, text_node(&doc, "plain")), (false, false, false));
    let out = doc.export().unwrap();
    assert!(part(&out, DOCUMENT).contains("<w:rPr><!--keep--></w:rPr>"));
    let reopened = Document::open(&out).unwrap();
    assert_eq!(
        shown(&reopened, text_node(&reopened, "bold")),
        (true, true, false)
    );
}

#[test]
fn formatting_uses_decoded_property_values_before_and_after_save() {
    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    for off in ["0", "&#48;", "&#x30;"] {
        let bytes = with_body(&format!(
            r#"<w:p><w:r><q:rPr xmlns:q="{W}" xmlns:w="urn:extension"><q:rStyle q:val="A&amp;B"/><q:b q:val="{off}"/></q:rPr><w:t>text</w:t></w:r></w:p>"#
        ));
        let mut doc = EditingDocument::open(&bytes).unwrap();
        let text = text_node(&doc, "text");
        edit(
            &mut doc,
            format_range(
                at(text, 0),
                at(text, 4),
                Format {
                    italic: Some(true),
                    ..Default::default()
                },
            ),
        );
        let reopened = Document::open(&doc.export().unwrap()).unwrap();
        for view in [&*doc, &reopened] {
            let text = text_node(view, "text");
            assert_eq!(shown(view, text), (false, true, false), "{off}");
            let run = view.node(text).unwrap().parent.unwrap();
            let NodeKind::Run(info) = &view.node(run).unwrap().kind else {
                panic!("expected run")
            };
            assert_eq!(info.style.as_deref(), Some("A&B"));
        }
    }
}

#[test]
fn typing_from_an_empty_paragraph_mark_keeps_its_property_scope() {
    let bytes = with_body(
        r#"<w:p><w:pPr xmlns:q="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:rPr><q:b/></w:rPr></w:pPr></w:p>"#,
    );
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let start = doc.paragraphs()[0];
    edit(
        &mut doc,
        Command::InsertText {
            at: Point::Start { start },
            text: "X".into(),
        },
    );
    let reopened = reopen("paragraph mark namespace", &doc);
    let run = reopened
        .descendants(reopened.paragraphs()[0])
        .into_iter()
        .find(|&id| matches!(reopened.node(id).unwrap().kind, NodeKind::Run(_)))
        .unwrap();
    assert_eq!(reopened.effective_run_props(run).bold, Some(true));
    assert_editor_undo("paragraph mark namespace", &bytes, &mut doc);
}

/// The body's text and the XML comments around it, in document order.
fn text_and_comments(bytes: &[u8]) -> String {
    let xml = part(bytes, DOCUMENT);
    let tree = roxmltree::Document::parse(&xml).unwrap();
    tree.descendants()
        .filter_map(|n| match n.node_type() {
            roxmltree::NodeType::Comment => Some(format!("<!--{}-->", n.text()?)),
            roxmltree::NodeType::Text if n.parent()?.has_tag_name((W, "t")) => {
                n.text().map(str::to_string)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn markup_at_the_edges_of_a_run_or_link_is_written_once_where_it_was() {
    let run = "<w:p><w:r><!--before--><w:t>ABCDE</w:t><!--after--></w:r></w:p>";
    let link = r#"<w:p><w:hyperlink w:anchor="mark"><!--before--><w:r><w:t>ABCDE</w:t></w:r><!--after--></w:hyperlink></w:p>"#;
    for body in [run, link] {
        let bytes = with_body(body);
        let check = |name: &str, command: &dyn Fn(NodeId) -> Command, want: &str| {
            let mut doc = EditingDocument::open(&bytes).unwrap();
            let text = text_node(&doc, "ABCDE");
            edit(&mut doc, command(text));
            let out = doc.export().unwrap();
            assert_eq!(text_and_comments(&out), want, "{name} in {body}");
            assert_editor_undo(name, &bytes, &mut doc);
        };
        for (from, to) in [(0, 5), (1, 3), (0, 2), (3, 5)] {
            check(
                &format!("bold {from}..{to}"),
                &|text| format_range(at(text, from), at(text, to), BOLD),
                "<!--before-->ABCDE<!--after-->",
            );
        }
        check(
            "typing",
            &|text| Command::InsertText {
                at: at(text, 2),
                text: "x".into(),
            },
            "<!--before-->ABxCDE<!--after-->",
        );
        check(
            "split",
            &|text| Command::SplitParagraph { at: at(text, 2) },
            "<!--before-->ABCDE<!--after-->",
        );
        // The untouched text stays in its source run, beside a copy for the insertion.
        for (offset, want) in [
            (0, "<!--before-->xABCDE<!--after-->"),
            (2, "<!--before-->ABxCDE<!--after-->"),
            (5, "<!--before-->ABCDEx<!--after-->"),
        ] {
            check(
                &format!("tracked insertion at {offset}"),
                &|text| Command::ReplaceRange {
                    from: at(text, offset),
                    to: at(text, offset),
                    text: "x".into(),
                    track: Some(docx_engine::Track {
                        author: "Alice".into(),
                        date: "2026-11-16T10:00:00Z".into(),
                    }),
                },
                want,
            );
        }
    }
}

/// WordprocessingML's namespace, written with a character reference.
const ESCAPED: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006&#x2F;main";
/// An extension whose text holds what looks like the start of a tag.
const CDATA: &str = r#"<e:ext xmlns:e="urn:extension"><![CDATA[1 < 2]]></e:ext>"#;
const KEPT: &str = r#"<e:kept xmlns:e="urn:extension"/>"#;

/// How many WordprocessingML `local` children each `parent` element of the
/// body holds, as an XML parser reads the exported part.
fn children_named(bytes: &[u8], parent: &str, local: &str) -> Vec<usize> {
    let xml = part(bytes, DOCUMENT);
    let tree = roxmltree::Document::parse(&xml).unwrap();
    tree.descendants()
        .filter(|n| n.has_tag_name((W, parent)))
        .map(|n| n.children().filter(|c| c.has_tag_name((W, local))).count())
        .collect()
}

#[test]
fn run_properties_are_found_as_the_xml_parser_reads_them() {
    let escaped = format!(
        r#"<w:p><w:r><q:rPr xmlns:q="{ESCAPED}"><q:b/>{KEPT}</q:rPr><w:t>ABCDE</w:t></w:r></w:p>"#
    );
    let cdata = format!("<w:p><w:r><w:rPr><w:b/>{CDATA}</w:rPr><w:t>ABCDE</w:t></w:r></w:p>");
    let italic = Format {
        italic: Some(true),
        ..Format::default()
    };
    type Shown = Vec<(String, bool, bool)>;
    let whole: Shown = vec![("ABCDE".into(), true, true)];
    let middle: Shown = vec![
        ("A".into(), true, false),
        ("BC".into(), true, true),
        ("DE".into(), true, false),
    ];
    for (name, body, extension) in [("CDATA", &cdata, CDATA), ("escaped", &escaped, KEPT)] {
        let bytes = with_body(body);
        for (from, to, want) in [(0, 5, &whole), (1, 3, &middle)] {
            let name = format!("{name} {from}..{to}");
            let mut doc = EditingDocument::open(&bytes).unwrap();
            let text = text_node(&doc, "ABCDE");
            assert_eq!(shown(&doc, text), (true, false, false), "{name}: opened");
            edit(&mut doc, format_range(at(text, from), at(text, to), italic));
            let out = doc.export().unwrap();
            let reopened = Document::open(&out).unwrap();
            for (which, view) in [("live", &*doc), ("reopened", &reopened)] {
                let runs: Shown = view
                    .descendants(view.paragraphs()[0])
                    .into_iter()
                    .filter_map(|id| match &view.node(id).unwrap().kind {
                        NodeKind::Text { text } => {
                            let (bold, italic, _) = shown(view, id);
                            Some((text.clone(), bold, italic))
                        }
                        _ => None,
                    })
                    .collect();
                assert_eq!(&runs, want, "{name}: {which}");
            }
            // Each run keeps one properties element, with the extension in it.
            let blocks = children_named(&out, "r", "rPr");
            assert_eq!(blocks, vec![1; want.len()], "{name}");
            let xml = part(&out, DOCUMENT);
            assert_eq!(xml.matches(extension).count(), want.len(), "{name}: {xml}");
            assert_editor_undo(&name, &bytes, &mut doc);
        }
    }
}

#[test]
fn paragraph_properties_are_found_as_the_xml_parser_reads_them() {
    let escaped = format!(
        r#"<w:p><q:pPr xmlns:q="{ESCAPED}"><q:pStyle q:val="Heading1"/><q:keepNext/>{KEPT}</q:pPr><w:r><w:t>Styled</w:t></w:r></w:p>"#
    );
    let cdata = format!(
        r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/><w:keepNext/>{CDATA}</w:pPr><w:r><w:t>Styled</w:t></w:r></w:p>"#
    );
    let base = generated("headings-styles");
    let source = part(&base, DOCUMENT);
    let (start, end) = (
        source.find("<w:body>").unwrap() + 8,
        source.find("<w:sectPr").unwrap(),
    );
    for (name, body, extension) in [("CDATA", &cdata, CDATA), ("escaped", &escaped, KEPT)] {
        let xml = format!("{}{body}{}", &source[..start], &source[end..]);
        let bytes = rewrite(&base, DOCUMENT, Some(xml.as_bytes()));
        let mut doc = EditingDocument::open(&bytes).unwrap();
        let text = text_node(&doc, "Styled");
        edit(&mut doc, set_style(at(text, 0), at(text, 0), "Title"));
        let out = doc.export().unwrap();
        let reopened = Document::open(&out).unwrap();
        for (which, view) in [("live", &*doc), ("reopened", &reopened)] {
            let paragraph = paragraph_with(view, "Styled");
            assert_eq!(
                style_of(view, paragraph).as_deref(),
                Some("Title"),
                "{name}: {which}"
            );
            assert!(
                view.effective_paragraph(paragraph).keep_next,
                "{name}: {which}"
            );
        }
        assert_eq!(children_named(&out, "p", "pPr"), [1], "{name}");
        assert_eq!(children_named(&out, "pPr", "pStyle"), [1], "{name}");
        let xml = part(&out, DOCUMENT);
        assert_eq!(xml.matches(extension).count(), 1, "{name}: {xml}");
        assert_editor_undo(name, &bytes, &mut doc);
    }
}

#[test]
fn style_commands_use_the_same_default_as_the_picker() {
    let bytes = generated("undefined-paragraph-style");
    let styles = part(&bytes, "word/styles.xml")
        .replace(r#"<w:name w:val="Normal"/>"#, r#"<w:name w:val="Normal"/><w:next w:val="Body"/>"#)
        .replace("</w:styles>", r#"<w:style w:type="paragraph" w:styleId="Body"><w:name w:val="Body"/></w:style></w:styles>"#);
    let bytes = rewrite(&bytes, "word/styles.xml", Some(styles.as_bytes()));
    for index in [0, 1] {
        let mut doc = EditingDocument::open(&bytes).unwrap();
        let at = Point::End {
            end: doc.paragraphs()[index],
        };
        edit(
            &mut doc,
            Command::SetParagraphStyle {
                from: at,
                to: at,
                style: "Normal".into(),
            },
        );
        assert!(!doc.0.undo(), "the effective style already matched");
        assert!(doc.export().unwrap() == bytes);
        edit(&mut doc, Command::SplitParagraph { at });
        let saved = Document::open(&doc.export().unwrap()).unwrap();
        assert_eq!(
            saved
                .inspect_node(saved.paragraphs()[index + 1])
                .unwrap()
                .effective_style,
            Some("Body")
        );
    }
}
