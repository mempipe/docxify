use super::common;

use common::*;
use docx_engine::{Command, Document, EditError, NodeId, NodeKind, StoryKind};
use serde_json::{json, Value};

fn view(doc: &Document, id: NodeId) -> Value {
    serde_json::to_value(doc.inspect_node(id).unwrap()).unwrap()
}

/// The nearest node holding `needle`'s text node that `kind` names.
fn around(doc: &Document, needle: &str, kind: &str) -> NodeId {
    let mut id = text_node(doc, needle);
    while doc.node(id).unwrap().kind.name() != kind {
        id = doc.node(id).unwrap().parent.unwrap();
    }
    id
}

fn effective(doc: &Document, needle: &str) -> Value {
    view(doc, around(doc, needle, "paragraph"))["effective"].clone()
}

fn every(doc: &Document, kind: &str) -> Vec<Value> {
    let roots: Vec<NodeId> = doc.stories().map(|s| s.id).collect();
    roots
        .into_iter()
        .flat_map(|r| doc.descendants(r))
        .filter(|&id| doc.node(id).unwrap().kind.name() == kind)
        .map(|id| view(doc, id))
        .collect()
}

#[test]
fn notes_are_stories_and_references_are_numbered_in_order() {
    let mut doc = EditingDocument::open(&generated("notes")).unwrap();
    let kinds: Vec<StoryKind> = doc
        .stories()
        .map(|s| match &s.kind {
            NodeKind::Story { story, .. } => *story,
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(
        kinds,
        [StoryKind::Body, StoryKind::Footnotes, StoryKind::Endnotes]
    );

    let references: Vec<(Value, Value, Value)> = every(&doc, "note_reference")
        .into_iter()
        .map(|r| (r["kind"].clone(), r["note_id"].clone(), r["number"].clone()))
        .collect();
    assert_eq!(
        references,
        [
            (json!("footnote"), json!("1"), json!("1")),
            (json!("footnote"), json!("2"), json!("2")),
            // A custom mark takes no number, so the next reference is 3.
            (json!("footnote"), json!("3"), Value::Null),
            (json!("footnote"), json!("4"), json!("3")),
            // The settings start endnotes at 4, in upper-case Roman numerals.
            (json!("endnote"), json!("1"), json!("IV")),
        ]
    );
    let marks: Vec<Value> = every(&doc, "note_mark")
        .into_iter()
        .map(|m| m["number"].clone())
        .collect();
    assert_eq!(marks, [json!("1"), json!("2"), json!("3"), json!("IV")]);

    let notes = every(&doc, "note");
    let separators: Vec<bool> = notes.iter().map(|n| n["separator"] == true).collect();
    assert_eq!(
        separators,
        [true, true, false, false, false, false, true, true, false]
    );
    assert_eq!(notes[2]["support"], "read_only");
    let note_text = text_node(&doc, "The first footnote.");
    assert!(matches!(
        doc.run(Command::ReplaceText {
            node: note_text,
            range: None,
            text: "x".into()
        }),
        Err(SessionError::Edit(EditError::ReadOnly { .. }))
    ));

    let reference = doc
        .descendants(doc.blocks()[0])
        .into_iter()
        .find(|&id| matches!(doc.node(id).unwrap().kind, NodeKind::NoteReference { .. }))
        .unwrap();
    let refused = doc.run(Command::RemoveNode { node: reference });
    assert!(
        matches!(
            refused,
            Err(SessionError::Edit(EditError::Refused { reason, .. })) if reason.contains("would orphan the note")
        ),
        "{refused:?}"
    );

    let bytes = generated("notes");
    let first = r#"<w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><w:footnoteReference w:id="1"/></w:r>"#;
    let inserted = part(&bytes, "word/document.xml").replace(
        first,
        &format!(r#"<w:ins w:id="90" w:author="A" w:date="2026-11-01T00:00:00Z">{first}</w:ins>"#),
    );
    let mut doc = EditingDocument::open(&rewrite(
        &bytes,
        "word/document.xml",
        Some(inserted.as_bytes()),
    ))
    .unwrap();
    let insertion = doc
        .descendants(doc.blocks()[0])
        .into_iter()
        .find(|&id| matches!(doc.node(id).unwrap().kind, NodeKind::Revision(_)))
        .unwrap();
    let refused = doc.run(Command::RemoveNode { node: insertion });
    assert!(
        matches!(
            refused,
            Err(SessionError::Edit(EditError::Refused { reason, .. })) if reason.contains("would orphan the note")
        ),
        "removing what holds a reference: {refused:?}"
    );
}

#[test]
fn note_numbers_follow_the_settings_format() {
    let bytes = generated("notes");
    let settings = part(&bytes, "word/settings.xml")
        .replace(
            "<w:footnotePr>",
            r#"<w:footnotePr><w:numFmt w:val="lowerLetter"/><w:numStart w:val="26"/>"#,
        )
        .replace(
            r#"<w:numFmt w:val="upperRoman"/><w:numStart w:val="4"/>"#,
            r#"<w:numFmt w:val="chicago"/><w:numStart w:val="5"/>"#,
        );
    let doc = Document::open(&rewrite(
        &bytes,
        "word/settings.xml",
        Some(settings.as_bytes()),
    ))
    .unwrap();
    let numbers: Vec<Value> = every(&doc, "note_reference")
        .into_iter()
        .map(|r| r["number"].clone())
        .collect();
    assert_eq!(
        numbers,
        [
            json!("z"),
            json!("aa"),
            Value::Null,
            json!("bb"),
            json!("**")
        ]
    );
}

#[test]
fn notes_then_comments_import_after_the_other_stories_so_their_ids_stay() {
    let doc = Document::open(&generated("complex-mixed")).unwrap();
    let roots: Vec<&docx_engine::Node> = doc.stories().collect();
    let kinds: Vec<StoryKind> = roots
        .iter()
        .map(|r| match &r.kind {
            NodeKind::Story { story, .. } => *story,
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(
        kinds[kinds.len() - 2..],
        [StoryKind::Footnotes, StoryKind::Comments]
    );
    for pair in roots.windows(2) {
        let before = doc.descendants(pair[0].id).into_iter().max().unwrap();
        assert!(pair[1].id > before);
    }
}

#[test]
fn paragraphs_resolve_defaults_styles_numbering_and_direct_formatting() {
    let doc = Document::open(&generated("paragraph-layout")).unwrap();
    assert_eq!(
        doc.inspect_json()["settings"],
        json!({ "default_tab_stop_twips": 709, "even_and_odd_headers": false, "update_fields": false })
    );

    let heading = effective(&doc, "Paragraph layout");
    assert_eq!(heading["keep_next"], true);
    assert_eq!(heading["keep_lines"], true);
    assert_eq!(
        heading["spacing"],
        json!({ "before_twips": 480, "after_twips": 120, "line": 259, "line_rule": "auto" }),
        "the style's spacing over the document defaults' line"
    );

    let justified = effective(&doc, "runs long enough");
    assert_eq!(justified["alignment"], "justify");
    assert_eq!(
        justified["indent"],
        json!({ "left_twips": 0, "right_twips": 0, "first_line_twips": 720 })
    );
    assert_eq!(
        justified["spacing"],
        json!({ "before_twips": 120, "after_twips": 240, "line": 360, "line_rule": "auto" })
    );
    let exact = effective(&doc, "Exactly fifteen");
    assert_eq!(exact["alignment"], "center");
    assert_eq!(exact["spacing"]["line_rule"], "exact");
    assert_eq!(exact["spacing"]["line"], 300);

    let hanging = view(&doc, doc.paragraphs()[3])["effective"].clone();
    assert_eq!(hanging["alignment"], "right");
    assert_eq!(hanging["spacing"]["line_rule"], "at_least");
    assert_eq!(
        hanging["indent"],
        json!({ "left_twips": 1440, "right_twips": 720, "first_line_twips": -360 }),
        "a hanging indent wins over a first-line one in the same layer"
    );

    assert_eq!(
        effective(&doc, "Contextual spacing")["contextual_spacing"],
        true
    );
    assert_eq!(
        effective(&doc, "Centre")["tabs"],
        json!([
            { "pos_twips": 1440, "align": "left" },
            { "pos_twips": 4320, "align": "center" },
            { "pos_twips": 7200, "align": "decimal" },
            { "pos_twips": 9360, "align": "right", "leader": "dot" },
        ]),
        "the paragraph clears the style's stop at 4680 and adds its own"
    );
    assert_eq!(effective(&doc, "Kept with")["keep_next"], true);
    assert_eq!(
        effective(&doc, "Starts on a new page")["page_break_before"],
        true
    );
    assert_eq!(
        effective(&doc, "Automatic spacing")["spacing"]["before_twips"],
        280
    );

    let indent = |needle: &str| effective(&doc, needle)["indent"].clone();
    assert_eq!(
        indent("A numbered item"),
        json!({ "left_twips": 720, "right_twips": 0, "first_line_twips": -360 })
    );
    assert_eq!(indent("A lettered sub-item")["left_twips"], 1440);
    assert_eq!(
        indent("Direct numbering")["left_twips"],
        720,
        "a level applied directly overrides the paragraph style's indent"
    );
    assert_eq!(
        indent("The style's indent"),
        json!({ "left_twips": 2160, "right_twips": 0, "first_line_twips": -360 }),
        "a level a style applies sits below that style"
    );

    let list =
        |needle: &str| view(&doc, around(&doc, needle, "paragraph"))["list"]["definition"].clone();
    assert_eq!(
        list("A numbered item"),
        json!({ "format": "decimal", "text": "%1.", "start": 1, "suffix": "tab", "justification": "left", "run": { "bold": true } })
    );
    assert_eq!(list("A lettered")["suffix"], "space");
    assert_eq!(list("A lettered")["justification"], "right");
    assert!(list("A lettered").get("run").is_none());
}

#[test]
fn runs_and_marks_resolve_theme_fonts_and_word_defaults() {
    let doc = Document::open(&generated("paragraph-layout")).unwrap();
    let run = |needle: &str| view(&doc, around(&doc, needle, "run"))["effective"].clone();
    assert_eq!(
        run("Paragraph layout"),
        json!({ "size_half_points": 32, "font": "Georgia" }),
        "the heading style names the theme's major font"
    );
    assert_eq!(
        run("Exactly fifteen")["font"],
        "Verdana",
        "the defaults name the minor font"
    );
    assert_eq!(run("small capitals")["small_caps"], true);

    let empty = *doc.paragraphs().last().unwrap();
    assert_eq!(
        view(&doc, empty)["mark"],
        json!({ "size_half_points": 40, "font": "Verdana" })
    );

    let bare = Document::open(&rewrite(
        &generated("simple-paragraphs"),
        "word/_rels/document.xml.rels",
        Some(br#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#),
    ))
    .unwrap();
    let first = bare.node(bare.blocks()[0]).unwrap().children[0];
    assert_eq!(
        view(&bare, first)["effective"],
        json!({ "size_half_points": 20, "font": "Times New Roman" }),
        "Word's own defaults when nothing states a font or size"
    );
}

#[test]
fn tables_resolve_borders_by_position_with_fills_margins_and_alignment() {
    let doc = Document::open(&generated("table-layout")).unwrap();
    let tables = every(&doc, "table");
    assert_eq!(
        (&tables[0]["indent_twips"], &tables[0]["alignment"]),
        (&json!(144), &json!("left"))
    );
    assert_eq!(
        (&tables[1]["indent_twips"], &tables[1]["alignment"]),
        (&json!(0), &json!("center"))
    );

    let rows = every(&doc, "row");
    assert_eq!(rows[0]["header"], true);
    assert_eq!(rows[0]["cant_split"], true);
    assert_eq!(rows[1]["height"], json!({ "twips": 720, "rule": "exact" }));
    assert_eq!(
        rows[2]["height"],
        json!({ "twips": 1080, "rule": "at_least" })
    );
    assert_eq!(rows[3]["grid_before"], 1);

    let cell = |needle: &str| view(&doc, around(&doc, needle, "cell"));
    let heavy = json!({ "style": "single", "size_eighths": 12 });
    let thin = json!({ "style": "single", "size_eighths": 4 });
    let dotted = json!({ "style": "dotted", "size_eighths": 4, "color": "808080" });
    assert_eq!(
        cell("Header A")["borders"],
        json!({ "top": heavy, "left": thin, "bottom": thin, "right": dotted }),
        "the table's top and left, and inside borders where it meets other cells"
    );
    assert_eq!(cell("Header C")["borders"]["right"], thin);
    assert_eq!(cell("Padded")["borders"]["top"], thin);
    assert_eq!(
        cell("Own borders")["borders"],
        json!({ "top": { "style": "double", "size_eighths": 12, "color": "FF0000" }, "left": dotted, "right": dotted }),
        "the cell's own top replaces the inside border, and nil removes its bottom"
    );
    assert_eq!(
        cell("After a skipped column")["borders"],
        json!({ "top": thin, "left": dotted, "bottom": heavy, "right": thin }),
        "a skipped grid column puts an inside border on the cell's left"
    );

    assert_eq!(cell("Header A")["shading"], "D9E2F3");
    assert!(cell("Merged down").get("shading").is_none());
    assert_eq!(cell("Style fill")["shading"], "E2EFDA");
    assert_eq!(cell("Own fill")["shading"], "FFF2CC");

    assert_eq!(
        cell("Header A")["margins_twips"],
        json!({ "top": 0, "left": 108, "bottom": 0, "right": 108 })
    );
    assert_eq!(
        cell("Padded")["margins_twips"],
        json!({ "top": 120, "left": 360, "bottom": 0, "right": 360 })
    );
    assert_eq!(
        cell("Style fill")["margins_twips"],
        json!({ "top": 0, "left": 144, "bottom": 0, "right": 144 })
    );
    assert_eq!(cell("Header A")["vertical_align"], "top");
    assert_eq!(cell("Bottom aligned")["vertical_align"], "bottom");
    assert_eq!(cell("Centred")["vertical_align"], "center");

    assert_eq!(
        effective(&doc, "Header A")["spacing"],
        json!({ "before_twips": 0, "after_twips": 0, "line": 240, "line_rule": "auto" }),
        "the table style's paragraph properties"
    );
    let colour =
        |needle: &str| view(&doc, around(&doc, needle, "run"))["effective"]["color"].clone();
    assert_eq!(colour("Before the tables"), "333333");
    assert_eq!(
        colour("Style fill"),
        "C00000",
        "a table style overrides the default paragraph style"
    );
    assert_eq!(
        colour("Own fill"),
        "333333",
        "but not a paragraph style the author chose"
    );
}

#[test]
fn sections_report_how_they_start_and_lay_out_pages() {
    let doc = Document::open(&generated("sections")).unwrap();
    let json = doc.inspect_json();
    assert_eq!(json["settings"]["even_and_odd_headers"], true);
    let sections: Vec<Value> = json["stories"][0]["children"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|b| match b["type"].as_str() {
            Some("paragraph") => b.get("section").cloned(),
            Some("section") => Some(b.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(sections.len(), 3);
    assert_eq!(sections[0]["start"], "nextPage");
    assert_eq!(sections[0]["title_page"], true);
    assert_eq!(
        sections[0]["page_numbers"],
        json!({ "start": 1, "format": "lowerRoman" })
    );
    let kinds: Vec<&Value> = sections[0]["headers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| &h["kind"])
        .collect();
    assert_eq!(kinds, ["default", "first", "even"]);
    assert_eq!(sections[1]["orientation"], "landscape");
    assert_eq!(sections[1]["start"], "nextPage");
    assert!(sections[1].get("title_page").is_none());
    assert!(sections[1].get("columns").is_none());
    assert_eq!(sections[2]["start"], "continuous");
    assert_eq!(
        sections[2]["columns"],
        json!({ "count": 2, "space_twips": 708 })
    );
}

#[test]
fn field_results_name_the_innermost_field_holding_them() {
    let doc = Document::open(&generated("sections")).unwrap();
    let field = |doc: &Document, needle: &str| view(doc, text_node(doc, needle))["field"].clone();
    assert_eq!(field(&doc, "1"), " PAGE ");
    assert_eq!(field(&doc, "4"), " NUMPAGES ");
    assert!(field(&doc, "Page ").is_null());

    let mixed = Document::open(&generated("complex-mixed")).unwrap();
    assert_eq!(
        field(&mixed, "Chapter two ... 2"),
        r#" TOC \o "1-3" "#,
        "a field's result can span paragraphs"
    );

    let bytes = generated("header-footer");
    let nested = part(&bytes, "word/footer1.xml").replace(
        r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> PAGE </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>1</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#,
        concat!(
            r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> IF </w:instrText></w:r>"#,
            r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> PAGE </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>7</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#,
            r#"<w:r><w:instrText xml:space="preserve"> = 1 "one" "many" </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>many</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#,
            r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> TOC </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>Intro</w:t></w:r>"#,
            r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> PAGEREF _Toc1 </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>3</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#,
            r#"<w:r><w:fldChar w:fldCharType="end"/></w:r>"#
        ),
    );
    let doc = Document::open(&rewrite(
        &bytes,
        "word/footer1.xml",
        Some(nested.as_bytes()),
    ))
    .unwrap();
    assert_eq!(field(&doc, "7"), " PAGE ");
    assert_eq!(field(&doc, "many"), r#" IF  = 1 "one" "many" "#);
    assert_eq!(field(&doc, "Intro"), " TOC ");
    assert_eq!(
        field(&doc, "3"),
        " PAGEREF _Toc1 ",
        "a field inside another's result names its own instruction"
    );
}

#[test]
fn drawings_report_whether_they_sit_inline_or_anchored() {
    let doc = Document::open(&generated("anchored-image")).unwrap();
    let placements: Vec<Value> = every(&doc, "drawing")
        .into_iter()
        .map(|d| d["placement"].clone())
        .collect();
    assert_eq!(
        placements,
        [
            json!({
                "kind": "anchor",
                "horizontal": { "relative_to": "column", "offset_emu": 3657600 },
                "vertical": { "relative_to": "paragraph", "align": "top" },
                "wrap": "square",
                "behind": false,
                "distance_emu": { "top": 0, "left": 114300, "bottom": 0, "right": 114300 },
            }),
            json!({
                "kind": "anchor",
                "horizontal": { "relative_to": "page", "align": "center" },
                "vertical": { "relative_to": "page", "offset_emu": 914400 },
                "wrap": "none",
                "behind": true,
                "distance_emu": { "top": 0, "left": 0, "bottom": 0, "right": 0 },
            }),
            json!({ "kind": "inline" }),
        ]
    );
}

#[test]
fn runs_keep_their_formatting_before_a_tracked_change() {
    let doc = Document::open(&generated("tracked-changes")).unwrap();
    let run = view(&doc, around(&doc, "newly bold", "run"));
    assert_eq!(run["format_revision"], true);
    assert_eq!(run["props"], json!({ "bold": true }));
    assert_eq!(run["previous"], json!({}));
    assert_eq!(
        run["format_change"],
        json!({ "author": "Alice", "date": "2026-11-01T10:05:00Z" })
    );
    assert!(view(&doc, around(&doc, "Kept text", "run"))
        .get("previous")
        .is_none());
}

#[test]
fn tracked_paragraph_marks_name_who_changed_them() {
    let doc = Document::open(&generated("tracked-changes")).unwrap();
    let paragraph = view(
        &doc,
        around(&doc, "A whole inserted paragraph.", "paragraph"),
    );
    assert_eq!(paragraph["mark_revision"], "ins");
    assert_eq!(
        paragraph["mark_change"],
        json!({ "author": "Alice", "date": "2026-11-01T10:06:00Z" })
    );
    assert!(view(&doc, around(&doc, "Kept text", "paragraph"))
        .get("mark_change")
        .is_none());
}

#[test]
fn a_move_names_its_range_so_its_source_and_destination_pair() {
    let doc = Document::open(&generated("tracked-changes")).unwrap();
    let revisions: Vec<(Value, Value)> = every(&doc, "revision")
        .into_iter()
        .map(|r| (r["change"].clone(), r["move_name"].clone()))
        .collect();
    assert_eq!(
        revisions,
        [
            (json!("ins"), Value::Null),
            (json!("del"), Value::Null),
            (json!("ins"), Value::Null),
            (json!("moveFrom"), json!("move1")),
            (json!("moveTo"), json!("move1")),
        ]
    );
}

#[test]
fn comments_are_a_story_and_runs_name_the_comments_holding_them() {
    let doc = Document::open(&generated("complex-mixed")).unwrap();
    let comments = every(&doc, "comment");
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0]["comment_id"], "0");
    assert_eq!(comments[0]["author"], "Reviewer");
    assert_eq!(comments[0]["initials"], "R");
    assert_eq!(comments[0]["date"], "2026-11-14T00:00:00Z");
    assert_eq!(
        around(&doc, "Check this wording.", "comment").to_string(),
        comments[0]["id"]
    );
    assert!(matches!(
        doc.node(text_node(&doc, "Check this wording.")).unwrap().support,
        docx_engine::Support::ReadOnly(reason) if reason.contains("comment")
    ));

    assert_eq!(
        view(
            &doc,
            around(&doc, "Closing paragraph with a comment.", "run")
        )["comments"],
        json!(["0"])
    );
    // The range ends before the reference's own run.
    let reference = every(&doc, "marker")
        .into_iter()
        .find(|m| m["element"].as_str().unwrap().ends_with("commentReference"))
        .unwrap();
    assert_eq!(reference["marker_id"], "0");
    let marker: NodeId = reference["id"].as_str().unwrap().parse().unwrap();
    let run = doc.node(marker).unwrap().parent.unwrap();
    assert!(view(&doc, run).get("comments").is_none());
}

#[test]
fn part_bytes_read_any_part_as_opened() {
    let doc = Document::open(&generated("image")).unwrap();
    let png = doc.part_bytes("word/media/image1.png").unwrap();
    assert!(png.starts_with(b"\x89PNG"));
    assert_eq!(
        png,
        part_bytes_of(&generated("image"), "word/media/image1.png")
    );
    assert!(doc.part_bytes("word/media/missing.png").is_none());
}

fn part_bytes_of(bytes: &[u8], name: &str) -> Vec<u8> {
    entries(bytes)
        .into_iter()
        .find(|(n, _)| n == name)
        .unwrap()
        .1
}

#[test]
fn a_cell_beside_a_wrapped_cell_shares_an_inside_border_with_it() {
    let cell = |text: &str| {
        format!(
            r#"<w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>"#
        )
    };
    let wrapped = |text: &str| {
        format!(
            "<w:sdt><w:sdtPr/><w:sdtContent>{}</w:sdtContent></w:sdt>",
            cell(text)
        )
    };
    let borders = r#"<w:tblBorders><w:top w:val="single" w:sz="24"/><w:left w:val="single" w:sz="24"/><w:bottom w:val="single" w:sz="24"/><w:right w:val="single" w:sz="24"/><w:insideH w:val="single" w:sz="4"/><w:insideV w:val="single" w:sz="4"/></w:tblBorders>"#;
    let table = |row: String| {
        with_body(&format!(
            r#"<w:tbl><w:tblPr><w:tblW w:w="4000" w:type="dxa"/>{borders}</w:tblPr><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid><w:tr>{row}</w:tr></w:tbl><w:p/>"#
        ))
    };
    let (outer, inside) = (
        json!({ "style": "single", "size_eighths": 24 }),
        json!({ "style": "single", "size_eighths": 4 }),
    );

    let doc = Document::open(&table(wrapped("Wrapped") + &cell("Plain"))).unwrap();
    let plain = view(&doc, around(&doc, "Plain", "cell"));
    assert_eq!(plain["borders"]["left"], inside, "after a wrapped cell");
    assert_eq!(plain["borders"]["right"], outer);

    let doc = Document::open(&table(cell("Plain") + &wrapped("Wrapped"))).unwrap();
    let plain = view(&doc, around(&doc, "Plain", "cell"));
    assert_eq!(plain["borders"]["left"], outer);
    assert_eq!(plain["borders"]["right"], inside, "before a wrapped cell");
}

#[test]
fn page_fields_tell_a_layout_which_field_and_what_number_to_draw() {
    let field = |begin: &str, instruction: &str, result: &str| {
        format!(
            r#"<w:r><w:fldChar w:fldCharType="begin"{begin}/></w:r><w:r><w:instrText xml:space="preserve"> {instruction} </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>{result}</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#
        )
    };
    let bytes = with_body(&[
        format!(
            "<w:p>{}<w:r><w:tab/></w:r>{}</w:p>",
            field("", "PAGE", "first"),
            field("", "PAGE", "second")
        ),
        format!("<w:p>{}</w:p>", field("", r"PAGE \* roman \* MERGEFORMAT", "rom")),
        format!("<w:p>{}</w:p>", field("", r"NUMPAGES \* Arabic", "total")),
        format!("<w:p>{}</w:p>", field("", "SECTIONPAGES", "section")),
        format!("<w:p>{}</w:p>", field(r#" w:fldLock="1""#, "PAGE", "42")),
        format!("<w:p>{}</w:p>", field("", r"PAGE \* Ordinal", "1st")),
        format!("<w:p>{}</w:p>", field("", "DATE", "today")),
        r#"<w:p><w:fldSimple w:instr=" PAGE "><w:r><w:t>simple</w:t></w:r></w:fldSimple></w:p>"#.to_string(),
    ]
    .concat());
    let doc = Document::open(&bytes).unwrap();
    let page = |needle: &str| view(&doc, text_node(&doc, needle))["page_field"].clone();
    // Two fields with one instruction are two fields.
    assert_eq!(page("first")["number"], "page");
    assert_ne!(page("first")["field"], page("second")["field"]);
    assert!(page("first").get("format").is_none());
    assert_eq!(
        page("rom"),
        json!({ "field": page("rom")["field"], "number": "page", "format": "lowerRoman" })
    );
    assert_eq!(page("total")["number"], "num_pages");
    assert_eq!(page("total")["format"], "decimal");
    assert_eq!(page("section")["number"], "section_pages");
    // A locked field, a format the engine does not know and another kind of
    // field keep the result the file holds.
    for kept in ["42", "1st", "today"] {
        assert!(page(kept).is_null(), "{kept}");
    }
    assert_eq!(page("simple")["number"], "page");
    assert_eq!(view(&doc, text_node(&doc, "simple"))["field"], "PAGE");
}
