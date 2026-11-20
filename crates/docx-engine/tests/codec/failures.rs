use super::common;

use common::*;
use docx_engine::{validate_package, Command, Document, EditError, NodeId, OpenError, TextRange};

#[test]
fn corrupted_zip_fails_to_open() {
    let mut bytes = generated("simple-paragraphs");
    let len = bytes.len();
    bytes.truncate(len / 2);
    assert!(matches!(Document::open(&bytes), Err(OpenError::Zip(_))));
    assert!(matches!(
        Document::open(b"not a zip"),
        Err(OpenError::Zip(_))
    ));
    assert!(!validate_package(&bytes).valid);
}

#[test]
fn missing_main_document_part_fails_to_open() {
    let bytes = rewrite(&generated("simple-paragraphs"), "word/document.xml", None);
    assert!(
        matches!(Document::open(&bytes), Err(OpenError::MissingMainPart(p)) if p == "word/document.xml")
    );
    let report = validate_package(&bytes);
    assert!(report
        .issues
        .iter()
        .any(|i| i.code == "broken_relationship"));
}

#[test]
fn missing_package_relationships_fail_to_open() {
    let bytes = rewrite(&generated("simple-paragraphs"), "_rels/.rels", None);
    assert!(matches!(Document::open(&bytes), Err(OpenError::MissingPart(p)) if p == "_rels/.rels"));
}

#[test]
fn invalid_xml_fails_to_open_with_the_part_named() {
    let bytes = generated("simple-paragraphs");
    let broken = part(&bytes, "word/document.xml").replace("</w:body>", "");
    let bytes = rewrite(&bytes, "word/document.xml", Some(broken.as_bytes()));
    assert!(
        matches!(Document::open(&bytes), Err(OpenError::Xml { part, .. }) if part == "word/document.xml")
    );
    assert!(validate_package(&bytes)
        .issues
        .iter()
        .any(|i| i.code == "malformed_xml"));
}

#[test]
fn dtd_is_refused() {
    let bytes = generated("simple-paragraphs");
    let xml = part(&bytes, "word/document.xml").replacen(
        "<w:document",
        "<!DOCTYPE w:document [<!ENTITY a \"aaaaaaaaaa\">]><w:document",
        1,
    );
    let bytes = rewrite(&bytes, "word/document.xml", Some(xml.as_bytes()));
    assert!(matches!(Document::open(&bytes), Err(OpenError::Xml { .. })));
}

#[test]
fn broken_relationships_open_with_diagnostics_and_fail_validation() {
    let bytes = rewrite(&generated("image"), "word/media/image1.png", None);
    let doc = Document::open(&bytes).unwrap();
    assert!(doc
        .diagnostics()
        .iter()
        .any(|d| d.code == "broken_relationship"));
    assert!(!validate_package(&bytes).valid);
    // Exporting does not hide the problem, and does not make it worse.
    let report = validate_package(&doc.export().unwrap());
    assert_eq!(report.issues.len(), 1, "{:?}", report.issues);

    let bytes = generated("image");
    let rels = part(&bytes, "word/_rels/document.xml.rels").replace("rIdImage1", "rIdOther");
    let bytes = rewrite(
        &bytes,
        "word/_rels/document.xml.rels",
        Some(rels.as_bytes()),
    );
    let doc = Document::open(&bytes).unwrap();
    assert!(doc
        .diagnostics()
        .iter()
        .any(|d| d.code == "unresolved_relationship_id"));
    assert!(validate_package(&bytes)
        .issues
        .iter()
        .any(|i| i.code == "unresolved_relationship_id"));
}

#[test]
fn strict_conformance_and_non_word_packages_are_refused() {
    let bytes = generated("simple-paragraphs");
    let rels = part(&bytes, "_rels/.rels").replace(
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument",
        "http://purl.oclc.org/ooxml/officeDocument/relationships/officeDocument",
    );
    let strict = rewrite(&bytes, "_rels/.rels", Some(rels.as_bytes()));
    assert!(matches!(
        Document::open(&strict),
        Err(OpenError::Unsupported { .. })
    ));

    let ct = part(&bytes, "[Content_Types].xml").replace(
        "wordprocessingml.document.main+xml",
        "spreadsheetml.sheet.main+xml",
    );
    let sheet = rewrite(&bytes, "[Content_Types].xml", Some(ct.as_bytes()));
    assert!(matches!(
        Document::open(&sheet),
        Err(OpenError::NotWordprocessing(_))
    ));
}

#[test]
fn utf16_parts_are_refused_and_bom_is_preserved() {
    let bytes = generated("simple-paragraphs");
    let mut utf16 = vec![0xFF, 0xFE];
    utf16.extend(
        part(&bytes, "word/document.xml")
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes()),
    );
    let doc16 = rewrite(&bytes, "word/document.xml", Some(&utf16));
    assert!(matches!(
        Document::open(&doc16),
        Err(OpenError::Unsupported { .. })
    ));

    let mut bom = vec![0xEF, 0xBB, 0xBF];
    bom.extend(part(&bytes, "word/document.xml").into_bytes());
    let with_bom = rewrite(&bytes, "word/document.xml", Some(&bom));
    let mut doc = EditingDocument::open(&with_bom).unwrap();
    let target = text_node(&doc, "First");
    doc.run(Command::ReplaceText {
        node: target,
        range: None,
        text: "Changed".into(),
    })
    .unwrap();
    let out = doc.export().unwrap();
    let (_, data) = entries(&out)
        .into_iter()
        .find(|(n, _)| n == "word/document.xml")
        .unwrap();
    assert!(data.starts_with(&[0xEF, 0xBB, 0xBF]));
    assert_valid("bom", &out);
}

#[test]
fn invalid_edit_targets_are_refused_without_changing_the_document() {
    let mut doc = EditingDocument::open(&generated("complex-mixed")).unwrap();
    let paragraph = doc.paragraphs()[0];
    let inserted = text_node(&doc, "inserted text");
    let toc = text_node(&doc, "Chapter one ... 1");
    let field_code = doc
        .descendants(doc.stories().next().unwrap().id)
        .into_iter()
        .find(|&id| doc.node(id).unwrap().kind.name() == "field_code")
        .unwrap();
    let plain = text_node(&doc, "First point");
    let replace = |node: NodeId, range: Option<TextRange>, text: &str| Command::ReplaceText {
        node,
        range,
        text: text.into(),
    };

    type RefusalCase = (Command, fn(&EditError) -> bool);
    let cases: Vec<RefusalCase> = vec![
        (replace("n999999".parse().unwrap(), None, "x"), |e| {
            matches!(e, EditError::UnknownNode { .. })
        }),
        (replace(paragraph, None, "x"), |e| {
            matches!(
                e,
                EditError::NotText {
                    kind: "paragraph",
                    ..
                }
            )
        }),
        (replace(inserted, None, "x"), |e| {
            matches!(e, EditError::ReadOnly { .. })
        }),
        (replace(toc, None, "x"), |e| {
            matches!(e, EditError::ReadOnly { .. })
        }),
        (replace(field_code, None, "x"), |e| {
            matches!(
                e,
                EditError::NotText {
                    kind: "field_code",
                    ..
                }
            )
        }),
        (
            replace(plain, Some(TextRange { start: 3, end: 99 }), "x"),
            |e| matches!(e, EditError::Range { len: 11, .. }),
        ),
        (
            replace(plain, Some(TextRange { start: 5, end: 2 }), "x"),
            |e| matches!(e, EditError::Range { .. }),
        ),
        (replace(plain, None, "tab\there"), |e| {
            matches!(e, EditError::InvalidText { found: '\t', .. })
        }),
        (replace(plain, None, "bell\u{7}"), |e| {
            matches!(e, EditError::InvalidText { .. })
        }),
    ];
    for (command, expected) in cases {
        let err = doc.run(command.clone()).unwrap_err();
        assert!(
            matches!(&err, SessionError::Edit(e) if expected(e)),
            "{command:?} gave {err:?}"
        );
    }
    assert!(!doc.is_dirty());
}

#[test]
fn commands_round_trip_through_json() {
    assert!(serde_json::from_str::<Command>(
        r#"{"command":"restore_node","node":"n7","parent":"n2","index":0}"#,
    )
    .is_err());
    let json = r#"{"command":"replace_text","node":"n7","range":{"start":1,"end":3},"text":"hi"}"#;
    let command: Command = serde_json::from_str(json).unwrap();
    assert_eq!(serde_json::to_string(&command).unwrap(), json);
    assert!(
        serde_json::from_str::<Command>(r#"{"command":"replace_text","node":"7","text":""}"#)
            .is_err()
    );
}
