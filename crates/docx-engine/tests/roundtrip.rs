mod common;
use docx_engine::editing::Editor;

use common::*;
use docx_engine::{Command, Document, NodeKind, TextRange};

#[test]
fn no_op_export_copies_every_entry_raw() {
    for (name, bytes) in all_fixtures() {
        let doc = Document::open(&bytes).unwrap();
        assert!(!doc.is_dirty());
        let out = doc.export().unwrap();
        assert_valid(&name, &out);
        assert_eq!(
            raw_entries(&out),
            raw_entries(&bytes),
            "{name}: compressed entries differ"
        );
    }
}

#[test]
fn edit_rewrites_only_the_edited_text_element() {
    for (name, bytes) in all_fixtures() {
        let mut doc = Editor::open(&bytes).unwrap();
        let Some(target) = first_editable_text(doc.view().unwrap()) else {
            continue;
        };
        doc.apply(&Command::ReplaceText {
            node: target,
            range: None,
            text: "Edited & <checked>".into(),
        })
        .unwrap();
        let dirty: Vec<String> = doc.view().unwrap().dirty_parts();
        assert_eq!(dirty.len(), 1, "{name}");
        let out = doc.export().unwrap();
        assert_valid(&name, &out);

        // Every other entry is byte-identical, compressed bytes included.
        let (old_raw, new_raw) = (raw_entries(&bytes), raw_entries(&out));
        assert_eq!(old_raw.len(), new_raw.len());
        for ((on, od), (nn, nd)) in old_raw.iter().zip(&new_raw) {
            assert_eq!(on, nn, "{name}: entry order");
            if *on != dirty[0] {
                assert_eq!(od, nd, "{name}: {on} changed");
            }
        }

        // The edited part differs from the original only inside the edited element.
        let (old, new) = (part(&bytes, &dirty[0]), part(&out, &dirty[0]));
        let prefix = old
            .bytes()
            .zip(new.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        let suffix = old
            .bytes()
            .rev()
            .zip(new.bytes().rev())
            .take_while(|(a, b)| a == b)
            .count();
        let changed_old = &old[prefix..old.len() - suffix.min(old.len() - prefix)];
        let changed_new = &new[prefix..new.len() - suffix.min(new.len() - prefix)];
        assert!(
            !changed_old.contains("<w:p") && !changed_old.contains("</w:r>"),
            "{name}: {changed_old}"
        );
        assert!(
            changed_new.contains("Edited &amp; &lt;checked&gt;"),
            "{name}: {changed_new}"
        );

        let reopened = Document::open(&out).unwrap();
        let edited = reopened.node(target).unwrap();
        assert!(
            matches!(&edited.kind, NodeKind::Text { text } if text == "Edited & <checked>"),
            "{name}: same id after re-import, new text"
        );
        let original_others: Vec<_> = Document::open(&bytes)
            .unwrap()
            .paragraphs()
            .into_iter()
            .filter(|p| !doc.view().unwrap().descendants(*p).contains(&target))
            .collect();
        let original = Document::open(&bytes).unwrap();
        for p in original_others {
            assert_eq!(
                original.text(p),
                reopened.text(p),
                "{name}: unrelated paragraph {p} changed"
            );
        }
    }
}

#[test]
fn partial_replace_counts_unicode_scalars_and_inverse_restores() {
    let bytes = generated("rich-text");
    let mut doc = Editor::open(&bytes).unwrap();
    let target = text_node(doc.view().unwrap(), "Emoji");
    // "Emoji 😀, CJK 漢字, ..." — "漢字" is scalars 13..15; the emoji counts as one.
    doc.apply(&Command::ReplaceText {
        node: target,
        range: Some(TextRange { start: 13, end: 15 }),
        text: "characters".into(),
    })
    .unwrap();
    assert_eq!(
        doc.view().unwrap().text(target).unwrap(),
        "Emoji 😀, CJK characters, combining e\u{301}"
    );
    assert!(doc.undo());
    assert_eq!(
        doc.view().unwrap().text(target).unwrap(),
        "Emoji 😀, CJK 漢字, combining e\u{301}"
    );

    // Reopening the undone edit recovers the original story.
    let out = doc.export().unwrap();
    assert_valid("rich-text", &out);
    let reopened = Document::open(&out).unwrap();
    assert_eq!(
        reopened.inspect_json()["stories"],
        Document::open(&bytes).unwrap().inspect_json()["stories"]
    );
}

#[test]
fn whitespace_survives_regeneration() {
    let mut doc = Editor::open(&generated("simple-paragraphs")).unwrap();
    let target = text_node(doc.view().unwrap(), "Second");
    doc.apply(&Command::ReplaceText {
        node: target,
        range: None,
        text: "  padded  ".into(),
    })
    .unwrap();
    let out = doc.export().unwrap();
    assert!(
        part(&out, "word/document.xml").contains(r#"<w:t xml:space="preserve">  padded  </w:t>"#)
    );
    assert_eq!(
        Document::open(&out).unwrap().text(target).unwrap(),
        "  padded  "
    );
}

#[test]
fn header_edit_touches_only_the_header_part() {
    let bytes = generated("header-footer");
    let mut doc = Editor::open(&bytes).unwrap();
    let target = text_node(doc.view().unwrap(), "Header text");
    doc.apply(&Command::ReplaceText {
        node: target,
        range: Some(TextRange { start: 0, end: 6 }),
        text: "Running".into(),
    })
    .unwrap();
    assert_eq!(doc.view().unwrap().dirty_parts(), ["word/header1.xml"]);
    let out = doc.export().unwrap();
    assert_valid("header-footer", &out);
    assert_eq!(
        part(&out, "word/document.xml"),
        part(&bytes, "word/document.xml")
    );
    assert!(part(&out, "word/header1.xml").contains("Running text, page "));
}

#[test]
fn opaque_neighbours_survive_an_edit_in_the_same_paragraph() {
    let bytes = generated("unknown-ooxml");
    let mut doc = Editor::open(&bytes).unwrap();
    let target = text_node(doc.view().unwrap(), "Editable run");
    doc.apply(&Command::ReplaceText {
        node: target,
        range: None,
        text: "Changed run, ".into(),
    })
    .unwrap();
    let out = doc.export().unwrap();
    assert_valid("unknown-ooxml", &out);
    let xml = part(&out, "word/document.xml");
    for kept in [
        r#"<w:r x:runFlag="keep"><w:t xml:space="preserve">Changed run, </w:t></w:r><x:inlineThing x:value="42"/>"#,
        r#"<w:r><x:runThing/><w:t>after inline unknowns.</w:t></w:r>"#,
        r#"<x:customBlock x:attr="1"><x:inner>keep me</x:inner></x:customBlock>"#,
        "<!-- a comment the engine must keep -->",
        "<?x-processing keep?>",
        r#"<w:p x:paraFlag="keep">"#,
    ] {
        assert!(xml.contains(kept), "lost {kept}");
    }
}

#[test]
fn tracked_changes_survive_an_edit_elsewhere() {
    let bytes = generated("tracked-changes");
    let mut doc = Editor::open(&bytes).unwrap();
    let target = text_node(doc.view().unwrap(), "Untouched paragraph");
    doc.apply(&Command::ReplaceText {
        node: target,
        range: None,
        text: "Edited paragraph after the revisions.".into(),
    })
    .unwrap();
    let out = doc.export().unwrap();
    let (old, new) = (
        part(&bytes, "word/document.xml"),
        part(&out, "word/document.xml"),
    );
    let cut = |s: &str| {
        let text = s
            .find("Untouched")
            .or_else(|| s.find("Edited paragraph"))
            .unwrap();
        s[..s[..text].rfind("<w:p>").unwrap()].to_string()
    };
    assert_eq!(
        cut(&old),
        cut(&new),
        "everything before the edit, revisions included, is byte-identical"
    );
    let reopened = Document::open(&out).unwrap();
    assert_eq!(
        reopened.inspect_json()["unsupported"],
        doc.view().unwrap().inspect_json()["unsupported"]
    );
}
