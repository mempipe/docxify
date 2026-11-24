use super::common;

use common::*;
use std::collections::BTreeMap;

use docx_engine::{Command, Document, EditError, NodeId, NodeKind, Pages, Point};

const SETTINGS: &str = "word/settings.xml";
const RELS: &str = "word/_rels/document.xml.rels";
const TYPES: &str = "[Content_Types].xml";

fn flag(on: bool) -> Command {
    Command::SetUpdateFields { on }
}

/// The names of the parts that differ between two packages, or that only
/// one has.
fn changed(before: &[u8], after: &[u8]) -> Vec<String> {
    let (a, b) = (entries(before), entries(after));
    let mut names: Vec<String> = a
        .iter()
        .chain(&b)
        .map(|(name, _)| name.clone())
        .filter(|name| {
            a.iter().find(|(n, _)| n == name).map(|e| &e.1)
                != b.iter().find(|(n, _)| n == name).map(|e| &e.1)
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

fn with_settings(base: &[u8], settings: &str) -> Vec<u8> {
    rewrite(base, SETTINGS, Some(settings.as_bytes()))
}

#[test]
fn the_flag_goes_into_the_settings_part_where_the_schema_puts_it() {
    let base = generated("notes");
    let mut doc = EditingDocument::open(&base).unwrap();
    assert!(!doc.settings().update_fields);
    doc.0.apply(&flag(true)).unwrap();
    assert!(doc.is_dirty());
    assert_eq!(doc.dirty_parts(), [SETTINGS]);
    let out = doc.export().unwrap();
    assert_valid("notes", &out);
    assert_eq!(changed(&base, &out), [SETTINGS]);
    let settings = part(&out, SETTINGS);
    assert!(
        settings.contains(r#"<w:updateFields w:val="true"/><w:footnotePr>"#),
        "{settings}"
    );
    assert!(Document::open(&out).unwrap().settings().update_fields);

    // With nothing the schema puts after it, the flag ends the settings.
    let mut doc = EditingDocument::open(&generated("paragraph-layout")).unwrap();
    doc.0.apply(&flag(true)).unwrap();
    let settings = part(&doc.export().unwrap(), SETTINGS);
    assert!(
        settings.ends_with(
            r#"<w:defaultTabStop w:val="709"/><w:updateFields w:val="true"/></w:settings>"#
        ),
        "{settings}"
    );

    // Nor anything from another namespace, such as Word's own settings.
    let settings = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:zoom w:percent="100"/><w14:docId w14:val="1A2B3C4D"/></w:settings>"#;
    let mut doc = EditingDocument::open(&with_settings(&base, settings)).unwrap();
    doc.0.apply(&flag(true)).unwrap();
    let settings = part(&doc.export().unwrap(), SETTINGS);
    assert!(
        settings.contains(r#"<w:zoom w:percent="100"/><w:updateFields w:val="true"/><w14:docId"#),
        "{settings}"
    );

    // Undo leaves the file as it was.
    let mut doc = EditingDocument::open(&base).unwrap();
    doc.0.apply(&flag(true)).unwrap();
    assert!(doc.0.undo());
    assert!(doc.dirty_parts().is_empty());
    assert!(changed(&base, &doc.export().unwrap()).is_empty());
}

#[test]
fn turning_the_flag_off_takes_it_out_and_a_false_one_is_replaced() {
    let base = generated("notes");
    let mut doc = EditingDocument::open(&base).unwrap();
    doc.0.apply(&flag(true)).unwrap();
    let on = doc.export().unwrap();
    let mut doc = EditingDocument::open(&on).unwrap();
    assert!(doc.settings().update_fields);
    doc.0.apply(&flag(false)).unwrap();
    let off = doc.export().unwrap();
    assert_eq!(part(&off, SETTINGS), part(&base, SETTINGS));
    assert!(!Document::open(&off).unwrap().settings().update_fields);

    let settings = part(&base, SETTINGS).replace(
        "<w:footnotePr>",
        r#"<w:updateFields w:val="false"/><w:footnotePr>"#,
    );
    let mut doc = EditingDocument::open(&with_settings(&base, &settings)).unwrap();
    assert!(!doc.settings().update_fields);
    doc.0.apply(&flag(true)).unwrap();
    let written = part(&doc.export().unwrap(), SETTINGS);
    assert_eq!(written.matches("updateFields").count(), 1, "{written}");
    assert!(written.contains(r#"<w:updateFields w:val="true"/><w:footnotePr>"#));
}

#[test]
fn settings_in_the_default_namespace_get_a_prefixed_flag() {
    let base = generated("notes");
    let settings = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><settings xmlns="{}"><defaultTabStop val="720"/></settings>"#,
        "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
    );
    let mut doc = EditingDocument::open(&with_settings(&base, &settings)).unwrap();
    doc.0.apply(&flag(true)).unwrap();
    let out = doc.export().unwrap();
    assert!(part(&out, SETTINGS).contains(
        r#"<defaultTabStop val="720"/><w:updateFields xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" w:val="true"/></settings>"#
    ));
    assert!(Document::open(&out).unwrap().settings().update_fields);
}

#[test]
fn a_document_without_settings_gets_a_part_that_holds_the_flag() {
    let base = generated("simple-paragraphs");
    assert!(!entries(&base).iter().any(|(n, _)| n == SETTINGS));
    let mut doc = EditingDocument::open(&base).unwrap();
    doc.0.apply(&flag(true)).unwrap();
    let out = doc.export().unwrap();
    assert_valid("simple-paragraphs", &out);
    assert_eq!(changed(&base, &out), [TYPES, RELS, SETTINGS]);
    assert!(part(&out, SETTINGS).contains(r#"<w:updateFields w:val="true"/>"#));
    assert!(part(&out, RELS).contains(
        r#"Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml""#
    ));
    assert!(part(&out, TYPES).contains(
        r#"<Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/>"#
    ));
    assert!(Document::open(&out).unwrap().settings().update_fields);

    assert!(doc.0.undo());
    assert!(changed(&base, &doc.export().unwrap()).is_empty());
}

#[test]
fn setting_the_flag_it_already_has_is_refused() {
    let mut doc = EditingDocument::open(&generated("notes")).unwrap();
    let err = doc.0.apply(&flag(false)).unwrap_err();
    assert!(
        matches!(
            err,
            SessionError::Edit(EditError::Setting {
                setting: "updateFields",
                ..
            })
        ),
        "{err}"
    );
    assert!(doc.dirty_parts().is_empty());
    doc.0.apply(&flag(true)).unwrap();
    assert!(matches!(
        doc.0.apply(&flag(true)),
        Err(SessionError::Edit(EditError::Setting { .. }))
    ));
}

const NOW: &str = "2026-11-16T14:05:09";

/// A complex field whose result is a bold run for each of `results`.
fn complex(instruction: &str, results: &[&str]) -> String {
    complex_with("", instruction, results)
}

fn complex_with(begin: &str, instruction: &str, results: &[&str]) -> String {
    let runs: String = results
        .iter()
        .map(|r| format!(r#"<w:r><w:rPr><w:b/></w:rPr><w:t xml:space="preserve">{r}</w:t></w:r>"#))
        .collect();
    format!(
        r#"<w:r><w:fldChar w:fldCharType="begin"{begin}/></w:r><w:r><w:instrText xml:space="preserve"> {instruction} </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>{runs}<w:r><w:fldChar w:fldCharType="end"/></w:r>"#
    )
}

fn plain(text: &str) -> String {
    format!(r#"<w:r><w:t xml:space="preserve">{text}</w:t></w:r>"#)
}

fn update(pages: Option<Pages>) -> Command {
    Command::UpdateFields {
        now: NOW.into(),
        pages,
    }
}

/// The text of each body paragraph, field codes left out.
fn shown(doc: &Document) -> Vec<String> {
    doc.paragraphs()
        .into_iter()
        .map(|p| {
            doc.descendants(p)
                .into_iter()
                .filter_map(|d| match &doc.node(d).unwrap().kind {
                    NodeKind::Text { text } => Some(text.as_str()),
                    NodeKind::Tab => Some("\t"),
                    _ => None,
                })
                .collect()
        })
        .collect()
}

fn pages(total: u32, numbers: &[(NodeId, &str)]) -> Pages {
    Pages {
        total,
        numbers: numbers
            .iter()
            .map(|(p, n)| (*p, n.to_string()))
            .collect::<BTreeMap<_, _>>(),
    }
}

#[test]
fn dates_and_times_take_the_time_the_editor_gives() {
    let base = with_body(&format!(
        r#"<w:p>{}{}</w:p><w:p><w:fldSimple w:instr=" TIME "><w:r><w:t>9:00 AM</w:t></w:r></w:fldSimple></w:p>"#,
        plain("Dated "),
        complex(
            r#"DATE \@ "dddd, MMMM d, yyyy" \* MERGEFORMAT"#,
            &["Monday, ", "January 1, 2024"]
        ),
    ));
    let mut doc = EditingDocument::open(&base).unwrap();
    let updates = doc.field_updates(NOW, None).unwrap();
    assert_eq!(
        updates.iter().map(|u| u.text.as_str()).collect::<Vec<_>>(),
        ["Monday, November 16, 2026", "2:05 PM"]
    );
    edit(&mut doc, update(None));
    assert_eq!(shown(&doc), ["Dated Monday, November 16, 2026", "2:05 PM"]);
    let out = doc.export().unwrap();
    assert_valid("dates", &out);
    let xml = part(&out, "word/document.xml");
    // The first result run keeps its formatting and takes the result.
    assert!(
        xml.contains(r#"<w:rPr><w:b/></w:rPr><w:t xml:space="preserve">Monday, November 16, 2026</w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t xml:space="preserve"></w:t></w:r>"#),
        "{xml}"
    );
    assert_eq!(
        shown(&Document::open(&out).unwrap()),
        ["Dated Monday, November 16, 2026", "2:05 PM"]
    );
    assert!(doc.field_updates(NOW, None).unwrap().is_empty());
    assert_editor_undo("dates", &base, &mut doc);
}

#[test]
fn page_fields_take_the_pages_the_layout_gives() {
    let base = with_body(&format!(
        r#"<w:p><w:bookmarkStart w:id="0" w:name="_Toc1"/>{}<w:bookmarkEnd w:id="0"/></w:p><w:p>{}{}{}{}{}{}</w:p>"#,
        plain("Intro"),
        plain("See page "),
        complex("PAGEREF _Toc1 \\h", &["1"]),
        plain("; this is page "),
        complex("PAGE \\* MERGEFORMAT", &["1"]),
        plain(" of "),
        complex("NUMPAGES", &["1"]),
    ));
    let mut doc = EditingDocument::open(&base).unwrap();
    let [heading, second] = doc.paragraphs()[..2] else {
        panic!("two paragraphs");
    };

    // Without a layout, there is nothing to update.
    let err = doc.run(update(None)).unwrap_err();
    assert!(
        matches!(err, SessionError::Edit(EditError::Fields { .. })),
        "{err}"
    );
    assert!(doc.dirty_parts().is_empty());

    edit(
        &mut doc,
        update(Some(pages(7, &[(heading, "iii"), (second, "4")]))),
    );
    assert_eq!(shown(&doc)[1], "See page iii; this is page 4 of 7");
    assert_valid("pages", &doc.export().unwrap());
    assert_editor_undo("pages", &base, &mut doc);

    // A page the layout does not give keeps its result.
    let mut doc = EditingDocument::open(&base).unwrap();
    edit(&mut doc, update(Some(pages(7, &[(second, "4")]))));
    assert_eq!(shown(&doc)[1], "See page 1; this is page 4 of 7");
}

#[test]
fn fields_the_engine_cannot_work_out_keep_their_results() {
    let body = [
        format!("<w:p>{}</w:p>", complex("REF _Ref1 \\h", &["Ref"])),
        format!("<w:p>{}</w:p>", complex("SEQ Figure \\* ARABIC", &["Seq"])),
        format!("<w:p>{}</w:p>", complex("PAGE \\* roman", &["Roman"])),
        format!(
            "<w:p>{}</w:p>",
            complex_with(r#" w:fldLock="1""#, "DATE", &["Locked"])
        ),
        r#"<w:p><w:fldSimple w:instr=" DATE " w:fldLock="true"><w:r><w:t>Simple locked</w:t></w:r></w:fldSimple></w:p>"#.to_string(),
        r#"<w:p><w:fldSimple w:instr=" DATE " w:fldLock='on'><w:r><w:t>Quoted locked</w:t></w:r></w:fldSimple></w:p>"#.to_string(),
        format!(
            "<w:p>{}</w:p>",
            complex_with(r#" w:fldLock = "1""#, "DATE", &["Spaced lock"])
        ),
        r#"<w:p><w:ins w:id="8" w:author="A" w:date="2024-01-01T00:00:00Z"><w:fldSimple w:instr=" DATE "><w:r><w:t>Tracked simple</w:t></w:r></w:fldSimple></w:ins></w:p>"#.to_string(),
        // A date whose result holds another field.
        format!(
            r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> DATE </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>{}{}{}<w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
            plain("Out"),
            complex("SEQ Inner", &["in"]),
            plain("er"),
        ),
        // A picture that would put a tab in the result.
        format!("<w:p>{}</w:p>", complex("DATE \\@ \"d\tM\"", &["Tabbed"])),
        r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> DATE </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>Tab</w:t><w:tab/><w:t>bed</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#.to_string(),
        r#"<w:p><w:fldSimple w:instr=" DATE "><w:r><w:t>Simple</w:t><w:tab/><w:t>tab</w:t></w:r></w:fldSimple></w:p>"#.to_string(),
        r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> DATE </w:instrText></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r><w:r><w:t>No result</w:t></w:r></w:p>"#.to_string(),
        r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> DATE </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>Two</w:t></w:r></w:p><w:p><w:r><w:t>paragraphs</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#.to_string(),
        format!(
            r#"<w:p><w:ins w:id="9" w:author="A" w:date="2024-01-01T00:00:00Z">{}</w:ins></w:p>"#,
            complex("DATE", &["Tracked"])
        ),
        format!(
            "<w:p>{}</w:p>",
            complex("IF 1 = 1 \"yes\" \"no\"", &["Unknown"])
        ),
        format!(
            r#"<w:sdt><w:sdtPr><w:lock w:val="sdtContentLocked"/></w:sdtPr><w:sdtContent><w:p>{}</w:p></w:sdtContent></w:sdt>"#,
            complex("DATE", &["Locked control"])
        ),
        format!(
            r#"<w:sdt><w:sdtPr><w:dataBinding w:xpath="/a[1]" w:storeItemID="{{00000000-0000-0000-0000-000000000001}}"/></w:sdtPr><w:sdtContent><w:p>{}</w:p></w:sdtContent></w:sdt>"#,
            complex("DATE", &["Bound control"])
        ),
        format!(
            r#"<w:sdt><w:sdtPr><w15:dataBinding xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml" w15:xpath="/a[1]" w15:storeItemID="{{00000000-0000-0000-0000-000000000001}}"/></w:sdtPr><w:sdtContent><w:p>{}</w:p></w:sdtContent></w:sdt>"#,
            complex("DATE", &["Bound by Word 2013"])
        ),
        format!(
            r#"<w:sdt><w:sdtPr><mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice Requires="w"><w:lock w:val="contentLocked"/></mc:Choice><mc:Fallback/></mc:AlternateContent></w:sdtPr><w:sdtContent><w:p>{}</w:p></w:sdtContent></w:sdt>"#,
            complex("DATE", &["Conditional lock"])
        ),
        format!("<w:p>{}</w:p>", complex("DATE", &["Updated"])),
    ]
    .concat();
    let base = with_body(&body);
    let mut doc = EditingDocument::open(&base).unwrap();
    let before = shown(&doc);
    let updates = doc.field_updates(NOW, Some(&pages(3, &[]))).unwrap();
    assert_eq!(updates.len(), 1, "{updates:?}");
    edit(&mut doc, update(Some(pages(3, &[]))));
    let after = shown(&doc);
    let last = after.len() - 1;
    assert_eq!(after[last], "11/16/2026");
    assert_eq!(after[..last], before[..last]);
}

#[test]
fn a_table_of_contents_keeps_its_entries_and_updates_its_page_numbers() {
    let entry = |anchor: &str, text: &str, page: &str| {
        format!(
            r#"<w:hyperlink w:anchor="{anchor}">{}<w:r><w:tab/></w:r>{}</w:hyperlink>"#,
            plain(text),
            complex(&format!("PAGEREF {anchor} \\h"), &[page])
        )
    };
    let heading = |id: u32, anchor: &str, text: &str| {
        format!(
            r#"<w:p><w:bookmarkStart w:id="{id}" w:name="{anchor}"/>{}<w:bookmarkEnd w:id="{id}"/></w:p>"#,
            plain(text)
        )
    };
    let toc = format!(
        r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> TOC \o "1-3" \h \z \u </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>{}</w:p><w:p>{}</w:p><w:p><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
        entry("_Toc1", "Intro", "1"),
        entry("_Toc2", "Method", "2"),
    );
    // Word puts the table of contents it makes in a content control.
    let control = format!(
        r#"<w:sdt><w:sdtPr><w:docPartObj><w:docPartGallery w:val="Table of Contents"/><w:docPartUnique/></w:docPartObj></w:sdtPr><w:sdtContent>{toc}</w:sdtContent></w:sdt>"#
    );
    for (name, toc) in [("toc", toc.clone()), ("toc in a control", control)] {
        let base = with_body(&format!(
            "{toc}{}{}",
            heading(1, "_Toc1", "Intro"),
            heading(2, "_Toc2", "Method"),
        ));
        let mut doc = EditingDocument::open(&base).unwrap();
        let paragraphs = doc.paragraphs();
        let layout = pages(9, &[(paragraphs[3], "3"), (paragraphs[4], "5")]);
        edit(&mut doc, update(Some(layout)));
        assert_eq!(shown(&doc)[..2], ["Intro\t3", "Method\t5"], "{name}");
        assert_valid(name, &doc.export().unwrap());
        assert_editor_undo(name, &base, &mut doc);
    }

    // The fixture's table of contents is text in two paragraphs, which
    // stays as it is.
    let mut doc = EditingDocument::open(&generated("complex-mixed")).unwrap();
    let all: Vec<(NodeId, &str)> = doc.paragraphs().into_iter().map(|p| (p, "8")).collect();
    let layout = pages(8, &all);
    let updates = doc.field_updates(NOW, Some(&layout)).unwrap();
    assert!(updates.is_empty(), "{updates:?}");
    assert!(matches!(
        doc.run(update(Some(layout))),
        Err(SessionError::Edit(EditError::Fields { .. }))
    ));
}

#[test]
fn setting_a_field_result_checks_the_field() {
    let base = with_body(&format!(
        r#"<w:p>{}{}</w:p><w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> DATE </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>A</w:t><w:tab/><w:t>B</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
        plain("Text "),
        complex("DATE", &["one", "two"]),
    ));
    let mut doc = EditingDocument::open(&base).unwrap();
    let begins: Vec<NodeId> = doc
        .paragraphs()
        .into_iter()
        .flat_map(|p| doc.descendants(p))
        .filter(|&d| {
            matches!(&doc.node(d).unwrap().kind, NodeKind::Marker { name: Some(n), .. } if n == "begin")
        })
        .collect();
    let text = text_node(&doc, "Text");
    let set = |field, text: &str| Command::SetFieldResult {
        field,
        text: text.into(),
    };

    let err = doc.run(set(text, "x")).unwrap_err();
    assert!(
        matches!(err, SessionError::Edit(EditError::WrongKind { .. })),
        "{err}"
    );
    let err = doc.run(set(begins[1], "x")).unwrap_err();
    assert!(
        matches!(err, SessionError::Edit(EditError::Refused { .. })),
        "{err}"
    );
    let err = doc.run(set(begins[0], "a\tb")).unwrap_err();
    assert!(
        matches!(err, SessionError::Edit(EditError::InvalidText { .. })),
        "{err}"
    );
    assert!(doc.dirty_parts().is_empty());

    edit(&mut doc, set(begins[0], "joined"));
    assert_eq!(shown(&doc)[0], "Text joined");
    assert_editor_undo("set", &base, &mut doc);

    // A header's page field is the layout's to show, not a body field.
    let mut doc = EditingDocument::open(&generated("header-footer")).unwrap();
    let header_field = doc
        .stories()
        .flat_map(|s| doc.descendants(s.id))
        .find(|&d| matches!(doc.node(d).unwrap().kind, NodeKind::Field { .. }))
        .unwrap();
    assert_eq!(doc.field_result(header_field), None);
    let err = doc.run(set(header_field, "9")).unwrap_err();
    assert!(
        matches!(err, SessionError::Edit(EditError::WrongKind { .. })),
        "{err}"
    );
}

#[test]
fn a_locked_or_tracked_field_refuses_a_result_and_an_extension_attribute_locks_nothing() {
    let base = with_body(&[
        format!(
            "<w:p>{}</w:p>",
            complex_with(r#" w:fldLock="1""#, "DATE", &["Locked"])
        ),
        r#"<w:p><w:fldSimple w:fldLock = 'true' w:instr=" DATE "><w:r><w:t>Simple locked</w:t></w:r></w:fldSimple></w:p>"#.to_string(),
        r#"<w:p><w:ins w:id="8" w:author="A" w:date="2024-01-01T00:00:00Z"><w:fldSimple w:instr=" DATE "><w:r><w:t>Tracked</w:t></w:r></w:fldSimple></w:ins></w:p>"#.to_string(),
        r#"<w:p><w:ins w:id="9" w:author="A"><w:sdt><w:sdtContent><w:fldSimple w:instr="DATE"><w:r><w:t>Nested</w:t></w:r></w:fldSimple></w:sdtContent></w:sdt></w:ins></w:p>"#.to_string(),
        format!(
            "<w:p>{}</w:p>",
            complex_with(r#" xmlns:x="urn:extension" x:fldLock="1""#, "DATE", &["Open"])
        ),
    ]
    .concat());
    let mut doc = EditingDocument::open(&base).unwrap();
    let fields: Vec<NodeId> = doc
        .paragraphs()
        .into_iter()
        .flat_map(|p| doc.descendants(p))
        .filter(|&d| match &doc.node(d).unwrap().kind {
            NodeKind::Marker { name: Some(n), .. } => n == "begin",
            NodeKind::Field { .. } => true,
            _ => false,
        })
        .collect();
    let set = |field| Command::SetFieldResult {
        field,
        text: "x".into(),
    };
    for (field, reason) in [
        (fields[0], "locked"),
        (fields[1], "locked"),
        (fields[2], "tracked change"),
        (fields[3], "tracked change"),
    ] {
        let err = doc.run(set(field)).unwrap_err();
        assert!(
            matches!(err, SessionError::Edit(EditError::Refused { .. }))
                && err.to_string().contains(reason),
            "{err}"
        );
        assert_eq!(doc.field_result(field), None);
    }
    assert!(doc.dirty_parts().is_empty());
    let updates = doc.field_updates(NOW, None).unwrap();
    assert_eq!(updates.len(), 1, "{updates:?}");
    assert_eq!(updates[0].field, fields[4]);
    edit(&mut doc, set(fields[4]));
    assert_eq!(
        shown(&doc),
        ["Locked", "Simple locked", "Tracked", "Nested", "x"]
    );
}

#[test]
fn the_update_reads_as_an_editor_sends_it() {
    let base = with_body(&format!("<w:p>{}</w:p>", complex("PAGE", &["1"])));
    let mut doc = EditingDocument::open(&base).unwrap();
    let paragraph = doc.paragraphs()[0];
    let command: Command = serde_json::from_value(serde_json::json!({
        "command": "update_fields",
        "now": NOW,
        "pages": { "total": 2, "numbers": { paragraph.to_string(): "2" } },
    }))
    .unwrap();
    assert_eq!(command.name(), "update_fields");
    assert!(docx_engine::COMMANDS.contains(&"update_fields"));
    edit(&mut doc, command);
    assert_eq!(shown(&doc), ["2"]);

    for bad in ["2026-11-16", "2026-11-16 14:05:09", "now"] {
        let err = doc
            .run(Command::UpdateFields {
                now: bad.into(),
                pages: None,
            })
            .unwrap_err();
        assert!(
            matches!(err, SessionError::Edit(EditError::Fields { .. })),
            "{bad}: {err}"
        );
    }
}

fn replace(from: Point, to: Point, text: &str) -> Command {
    Command::ReplaceRange {
        from,
        to,
        text: text.into(),
        track: None,
    }
}

#[test]
fn typing_beside_a_field_stays_outside_it_and_a_range_cannot_cut_into_one() {
    let base = with_body(&format!(
        "<w:p>{}{}{}{}</w:p>",
        plain("Page "),
        complex("PAGE", &["1", "2"]),
        complex("NUMPAGES", &["34"]),
        plain(" end")
    ));
    let mut doc = EditingDocument::open(&base).unwrap();
    let (one, two, total) = (
        text_node(&doc, "1"),
        text_node(&doc, "2"),
        text_node(&doc, "34"),
    );
    let (before, after) = (
        |before| Point::Before { before },
        |after| Point::After { after },
    );

    // A caret between the two results is between the two fields.
    edit(&mut doc, replace(after(two), after(two), "Z"));
    assert_eq!(shown(&doc), ["Page 12Z34 end"]);
    let paragraph = doc.paragraphs()[0];
    edit(&mut doc, update(Some(pages(4, &[(paragraph, "9")]))));
    assert_eq!(shown(&doc), ["Page 9Z4 end"], "each field has its result");
    assert_valid("typed between fields", &doc.export().unwrap());
    assert_eq!(
        shown(&reopen("typed between fields", &doc)),
        ["Page 9Z4 end"]
    );

    for split in [false, true] {
        let mut doc = EditingDocument::open(&base).unwrap();
        let at = after(two);
        let command = if split {
            Command::SplitParagraph { at }
        } else {
            Command::InsertBreak { at, to: None }
        };
        edit(&mut doc, command);
        let mut doc = EditingDocument::open(&doc.export().unwrap()).unwrap();
        let first = doc.paragraphs()[0];
        edit(&mut doc, update(Some(pages(4, &[(first, "9")]))));
        if split {
            assert_eq!(shown(&doc), ["Page 9", "4 end"]);
        } else {
            assert_eq!(shown(&doc), ["Page 94 end"]);
            let xml = part(&doc.export().unwrap(), "word/document.xml");
            let br = xml.find("<w:br").unwrap();
            assert!(xml[..br].contains(r#"fldCharType="end""#));
            assert!(xml[br..].contains(r#"fldCharType="begin""#));
        }
    }

    let mut doc = EditingDocument::open(&base).unwrap();
    edit(
        &mut doc,
        Command::InsertText {
            at: before(total),
            text: "Y".into(),
        },
    );
    assert_eq!(shown(&doc), ["Page 12Y34 end"]);

    // Inside a result, and across one end of a field, nothing is written.
    let mut doc = EditingDocument::open(&base).unwrap();
    let unchanged = doc.export().unwrap();
    let code = doc
        .descendants(doc.paragraphs()[0])
        .into_iter()
        .find(|id| matches!(doc.node(*id).unwrap().kind, NodeKind::FieldCode { .. }))
        .unwrap();
    let lead = text_node(&doc, "Page ");
    for (label, command) in [
        ("typing in a result", replace(after(one), after(one), "Z")),
        ("part of a result", replace(before(one), after(one), "")),
        ("split a result", Command::SplitParagraph { at: after(one) }),
        (
            "break a result",
            Command::InsertBreak {
                at: after(one),
                to: None,
            },
        ),
        (
            "from text into a result",
            replace(at(lead, 2), after(one), ""),
        ),
        (
            "the instruction alone",
            replace(before(code), after(code), ""),
        ),
        (
            "inserting in a result",
            Command::InsertText {
                at: before(two),
                text: "Z".into(),
            },
        ),
    ] {
        let err = doc.run(command).unwrap_err();
        assert!(err.to_string().contains("inside a field"), "{label}: {err}");
        assert!(
            doc.export().unwrap() == unchanged,
            "{label}: wrote something"
        );
    }

    // A range over a whole result takes the whole field.
    edit(&mut doc, replace(at(lead, 5), after(two), ""));
    assert_eq!(shown(&doc), ["Page 34 end"]);
    let xml = part(&doc.export().unwrap(), "word/document.xml");
    assert!(!xml.contains(" PAGE "), "{xml}");
    assert!(xml.contains(" NUMPAGES "), "{xml}");
    assert_valid("a field removed whole", &doc.export().unwrap());
}
