use super::common;

use common::*;
use docx_engine::{Command, Control, Document, EditError, Fill, NodeId, NodeKind};

const DOCUMENT: &str = "word/document.xml";

/// The content control with this alias.
fn control(doc: &Document, alias: &str) -> NodeId {
    let body = doc.stories().next().unwrap().id;
    doc.descendants(body)
        .into_iter()
        .find(|&id| {
            matches!(&doc.node(id).unwrap().kind,
                NodeKind::ContentControl { alias: Some(a), .. } if a == alias)
        })
        .unwrap_or_else(|| panic!("no control {alias:?}"))
}

/// What a control shows, a line break as a line feed.
fn shown(doc: &Document, id: NodeId) -> String {
    doc.descendants(id)
        .into_iter()
        .filter_map(|d| match &doc.node(d).unwrap().kind {
            NodeKind::Text { text } => Some(text.clone()),
            NodeKind::Break { .. } => Some("\n".to_string()),
            _ => None,
        })
        .collect()
}

/// A control's type and whether its placeholder shows.
fn state(doc: &Document, id: NodeId) -> (Control, bool) {
    match &doc.node(id).unwrap().kind {
        NodeKind::ContentControl {
            control,
            placeholder,
            ..
        } => (control.clone(), *placeholder),
        kind => panic!("{kind:?}"),
    }
}

fn fill(doc: &Document, alias: &str, fill: Fill) -> Command {
    Command::FillContentControl {
        control: control(doc, alias),
        fill,
    }
}

fn fills() -> Vec<(&'static str, Fill)> {
    vec![
        ("Name", Fill::Text("Ada Lovelace".into())),
        ("Agree", Fill::Checked(true)),
        ("Size", Fill::Item("M".into())),
        ("Colour", Fill::Item("Green".into())),
        ("Start date", Fill::Date("2026-11-16".into())),
        ("Address", Fill::Text("1 New Street\nLondon".into())),
        ("Notes", Fill::Text("Two lines\nhere".into())),
    ]
}

#[test]
fn each_type_fills_as_word_writes_it_and_reopens_with_its_value() {
    let base = generated("content-controls");
    let mut doc = EditingDocument::open(&base).unwrap();
    for (alias, value) in fills() {
        let command = fill(&doc, alias, value);
        edit(&mut doc, command);
    }
    assert_eq!(doc.dirty_parts(), [DOCUMENT]);
    let xml = part(&doc.export().unwrap(), DOCUMENT);
    for expected in [
        // The placeholder flag goes, and the run takes the control's run
        // properties instead of the placeholder style.
        concat!(
            r#"<w:sdtPr><w:rPr><w:b/></w:rPr><w:alias w:val="Name"/><w:tag w:val="name"/><w:id w:val="101"/><w:text/></w:sdtPr>"#,
            r#"<w:sdtContent><w:r><w:rPr><w:b/></w:rPr><w:t xml:space="preserve">Ada Lovelace</w:t></w:r></w:sdtContent>"#,
        ),
        // A check box keeps its symbol's font.
        r#"<w14:checkbox><w14:checked w14:val="1"/><w14:checkedState"#,
        r#"<w:sdtContent><w:r><w:rPr><w:rFonts w:ascii="MS Gothic" w:eastAsia="MS Gothic" w:hAnsi="MS Gothic" w:hint="eastAsia"/></w:rPr><w:t xml:space="preserve">☒</w:t></w:r></w:sdtContent>"#,
        // A list shows the item's display text and records its value.
        r#"<w:id w:val="103"/><w:dropDownList w:lastValue="M"><w:listItem"#,
        r#"<w:t xml:space="preserve">Medium</w:t>"#,
        r#"<w:comboBox w:lastValue="Green">"#,
        r#"<w:t xml:space="preserve">Sea green</w:t>"#,
        // A date shows through its picture, in a paragraph like the one it replaces.
        r#"<w:date w:fullDate="2026-11-16T00:00:00Z"><w:dateFormat w:val="dddd, MMMM d, yyyy"/>"#,
        r#"<w:sdtContent><w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:t xml:space="preserve">Monday, November 16, 2026</w:t></w:r></w:p></w:sdtContent>"#,
        // A line feed is a line break, formatted like the text it replaces.
        r#"<w:p><w:r><w:rPr><w:i/></w:rPr><w:t xml:space="preserve">1 New Street</w:t><w:br/><w:t xml:space="preserve">London</w:t></w:r></w:p>"#,
        r#"<w:t xml:space="preserve">Two lines</w:t><w:br/><w:t xml:space="preserve">here</w:t>"#,
    ] {
        assert!(xml.contains(expected), "{expected}\n{xml}");
    }
    assert!(!xml.contains("showingPlcHdr") && !xml.contains("PlaceholderText"));

    let doc = reopen("content-controls", &doc);
    let id = |alias| control(&doc, alias);
    assert_eq!(shown(&doc, id("Name")), "Ada Lovelace");
    assert_eq!(shown(&doc, id("Address")), "1 New Street\nLondon");
    assert_eq!(
        doc.text(id("Start date")).unwrap(),
        "Monday, November 16, 2026"
    );
    assert!(matches!(
        state(&doc, id("Agree")),
        (Control::Checkbox { checked: true, .. }, false)
    ));
    assert!(matches!(
        state(&doc, id("Size")),
        (Control::DropDown { last_value: Some(v), .. }, false) if v == "M"
    ));
    assert!(matches!(
        state(&doc, id("Start date")),
        (Control::Date { full_date: Some(d), .. }, false) if d == "2026-11-16T00:00:00Z"
    ));
    assert!(!state(&doc, id("Name")).1);
}

#[test]
fn undo_gives_back_the_file_and_redo_the_fill() {
    let base = generated("content-controls");
    for (alias, value) in fills() {
        let mut doc = EditingDocument::open(&base).unwrap();
        let command = fill(&doc, alias, value);
        edit(&mut doc, command);
        assert_editor_undo(alias, &base, &mut doc);
    }
}

#[test]
fn the_engine_reports_the_fill_as_it_does_an_edit() {
    let mut doc = EditingDocument::open(&generated("content-controls")).unwrap();
    let command = fill(&doc, "Name", Fill::Text("Ada".into()));
    edit(&mut doc, command);

    let command = fill(&doc, "Start date", Fill::Date("2026-01-02".into()));
    edit(&mut doc, command);

    // What a fill makes stays out of other commands' reach, as the
    // control's content did.
    let text = text_node(&doc, "Ada");
    let refused = doc.run(Command::ReplaceText {
        node: text,
        range: None,
        text: "Bob".into(),
    });
    assert!(
        matches!(refused, Err(SessionError::Edit(EditError::ReadOnly { .. }))),
        "{refused:?}"
    );

    // A filled control fills again, its run formatted like the last fill.
    let command = fill(&doc, "Name", Fill::Text("Grace Hopper".into()));
    edit(&mut doc, command);
    let xml = part(&doc.export().unwrap(), DOCUMENT);
    assert!(
        xml.contains(
            r#"<w:r><w:rPr><w:b/></w:rPr><w:t xml:space="preserve">Grace Hopper</w:t></w:r>"#
        ),
        "{xml}"
    );
}

#[test]
fn a_fill_refuses_what_a_form_would_not_allow() {
    let base = generated("content-controls");
    let mut doc = EditingDocument::open(&base).unwrap();
    let text = |s: &str| Fill::Text(s.into());
    let cases: Vec<(&str, Fill, &str)> = vec![
        ("Reference", text("REF-2"), "it is locked against editing"),
        (
            "Company",
            text("Acme"),
            "it is bound to custom XML, which a fill would leave stale",
        ),
        (
            "Signature block",
            text("x"),
            "it holds more than text, which a fill would lose",
        ),
        (
            "Source",
            text("x"),
            "the control takes another kind of value",
        ),
        (
            "Name",
            Fill::Checked(true),
            "the control takes another kind of value",
        ),
        (
            "Agree",
            text("yes"),
            "the control takes another kind of value",
        ),
        (
            "Size",
            Fill::Date("2026-01-01".into()),
            "the control takes another kind of value",
        ),
        ("Name", text(""), "a text fill needs text"),
        ("Name", text("two\nlines"), "the control holds one line"),
        ("Agree", Fill::Checked(false), "it is unchecked already"),
        (
            "Size",
            Fill::Item("XL".into()),
            "no list item has that value",
        ),
        (
            "Colour",
            Fill::Item("Sea green".into()),
            "no list item has that value",
        ),
        (
            "Start date",
            Fill::Date("2026-02-30".into()),
            "the date is not a real date written YYYY-MM-DD",
        ),
        (
            "Start date",
            Fill::Date("16/11/2026".into()),
            "the date is not a real date written YYYY-MM-DD",
        ),
    ];
    for (alias, value, reason) in cases {
        let command = fill(&doc, alias, value);
        match doc.run(command.clone()) {
            Err(SessionError::Edit(EditError::Refused { reason: r, .. })) => {
                assert_eq!(r, reason, "{command:?}")
            }
            other => panic!("{command:?}: {other:?}"),
        }
    }
    let command = fill(&doc, "Name", text("tab\there"));
    assert!(matches!(
        doc.run(command),
        Err(SessionError::Edit(EditError::InvalidText {
            found: '\t',
            ..
        }))
    ));
    // A control inside another is as read-only as the rest of its content.
    let command = fill(&doc, "Signature", text("Ada"));
    assert!(matches!(
        doc.run(command),
        Err(SessionError::Edit(EditError::ReadOnly { .. }))
    ));
    let paragraph = paragraph_with(&doc, "A form");
    let refused = doc.run(Command::FillContentControl {
        control: paragraph,
        fill: text("x"),
    });
    assert!(
        matches!(
            refused,
            Err(SessionError::Edit(EditError::WrongKind { .. }))
        ),
        "{refused:?}"
    );
    assert!(doc.dirty_parts().is_empty());
    assert_eq!(stories(&doc), stories(&Document::open(&base).unwrap()));
}

#[test]
fn a_check_box_fills_whatever_prefix_the_document_gives_word_2010_markup() {
    let base = generated("content-controls");
    let xml = part(&base, DOCUMENT)
        .replace("xmlns:w14=", "xmlns:v14=")
        .replace("mc:Ignorable=\"w14 x\"", "mc:Ignorable=\"v14 x\"")
        .replace("w14:", "v14:");
    let bytes = rewrite(&base, DOCUMENT, Some(xml.as_bytes()));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let command = fill(&doc, "Agree", Fill::Checked(true));
    edit(&mut doc, command);
    let out = part(&doc.export().unwrap(), DOCUMENT);
    assert!(
        out.contains(r#"<v14:checkbox><v14:checked v14:val="1"/>"#),
        "{out}"
    );
    assert!(matches!(
        state(&doc, control(&doc, "Agree")),
        (Control::Checkbox { checked: true, .. }, false)
    ));
    assert_editor_undo("v14", &bytes, &mut doc);

    // A check box without a checked state gets one.
    let xml = part(&base, DOCUMENT).replace(r#"<w14:checked w14:val="0"/>"#, "");
    let bytes = rewrite(&base, DOCUMENT, Some(xml.as_bytes()));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let command = fill(&doc, "Agree", Fill::Checked(true));
    edit(&mut doc, command);
    let out = part(&doc.export().unwrap(), DOCUMENT);
    assert!(
        out.contains(r#"<w14:checkbox><w14:checked w14:val="1"/><w14:checkedState"#),
        "{out}"
    );
}

#[test]
fn checking_a_box_only_changes_the_word_checkbox_state() {
    let base = generated("content-controls");
    let original = part(&base, DOCUMENT);
    let w14 = "http://schemas.microsoft.com/office/word/2010/wordml";
    for (source, expected) in [
        (
            r#"<e:checked e:val="foreign"/><c:checked e:val="keep" c:val="0"/>"#,
            r#"<e:checked e:val="foreign"/><c:checked e:val="keep" c:val="1"/>"#,
        ),
        (r#"<e:checked/>"#, r#"<c:checked c:val="1"/><e:checked/>"#),
    ] {
        let xml = original
            .replace(
                "<w:body>",
                &format!(r#"<w:body xmlns:c="{w14}" xmlns:e="urn:ext">"#),
            )
            .replace(
                "<w14:checkbox>",
                r#"<e:checkbox e:val="keep"/><c:checkbox>"#,
            )
            .replace("</w14:checkbox>", "</c:checkbox>")
            .replace(r#"<w14:checked w14:val="0"/>"#, source);
        let bytes = rewrite(&base, DOCUMENT, Some(xml.as_bytes()));
        let mut doc = EditingDocument::open(&bytes).unwrap();
        let command = fill(&doc, "Agree", Fill::Checked(true));
        edit(&mut doc, command);
        let out = doc.export().unwrap();
        let xml = part(&out, DOCUMENT);
        assert!(xml.contains(expected), "{xml}");
        assert!(xml.contains(r#"<e:checkbox e:val="keep"/>"#));
        let doc = Document::open(&out).unwrap();
        assert!(matches!(
            state(&doc, control(&doc, "Agree")).0,
            Control::Checkbox { checked: true, .. }
        ));
    }
}

#[test]
fn filling_a_list_preserves_foreign_and_single_quoted_attributes() {
    let base = generated("content-controls");
    let xml = part(&base, DOCUMENT)
        .replace(
            r#"w:lastValue="Red""#,
            "x:lastValue='extension' w:lastValue='Red'",
        )
        .replace(r#"w:value="Green""#, r#"w:value="Robin's green""#);
    let bytes = rewrite(&base, DOCUMENT, Some(xml.as_bytes()));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let command = fill(&doc, "Colour", Fill::Item("Robin's green".into()));
    edit(&mut doc, command);
    let out = doc.export().unwrap();
    assert!(
        part(&out, DOCUMENT).contains("x:lastValue='extension' w:lastValue='Robin&apos;s green'")
    );
    let reopened = Document::open(&out).unwrap();
    assert_eq!(shown(&reopened, control(&reopened, "Colour")), "Sea green");
}

/// The properties of the first run a control holds.
fn run_props(doc: &Document, id: NodeId) -> docx_engine::ir::RunProps {
    doc.descendants(id)
        .into_iter()
        .find_map(|d| match &doc.node(d).unwrap().kind {
            NodeKind::Run(info) => Some(info.props.clone()),
            _ => None,
        })
        .expect("a run")
}

#[test]
fn a_fill_keeps_the_namespace_its_copied_run_properties_were_written_in() {
    let base = generated("content-controls");
    let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    let xml = part(&base, DOCUMENT)
        // What the control shows is formatted by its first run.
        .replace(
            "<w:r><w:rPr><w:i/></w:rPr><w:t>1 Old Street</w:t></w:r>",
            &format!(r#"<w:r xmlns:q="{w}"><w:rPr><q:i/></w:rPr><w:t>1 Old Street</w:t></w:r>"#),
        )
        // A placeholder is replaced in the format the control's properties give.
        .replace(
            "<w:sdtPr><w:rPr><w:b/></w:rPr>",
            &format!(r#"<w:sdtPr xmlns:q="{w}"><w:rPr><q:b/></w:rPr>"#),
        );
    let bytes = rewrite(&base, DOCUMENT, Some(xml.as_bytes()));
    let mut doc = EditingDocument::open(&bytes).unwrap();
    for (alias, text) in [("Address", "2 New Street"), ("Name", "Ada")] {
        let command = fill(&doc, alias, Fill::Text(text.into()));
        edit(&mut doc, command);
    }
    assert_eq!(run_props(&doc, control(&doc, "Address")).italic, Some(true));
    assert_eq!(run_props(&doc, control(&doc, "Name")).bold, Some(true));
    let out = doc.export().unwrap();
    assert_valid("scoped run properties", &out);
    let reopened = Document::open(&out).unwrap();
    assert_eq!(
        shown(&reopened, control(&reopened, "Address")),
        "2 New Street"
    );
    assert_eq!(
        run_props(&reopened, control(&reopened, "Address")).italic,
        Some(true)
    );
    assert_eq!(
        run_props(&reopened, control(&reopened, "Name")).bold,
        Some(true)
    );
    assert!(doc.0.undo() && doc.0.undo());
    assert_eq!(entries(&doc.export().unwrap()), entries(&bytes));
}
