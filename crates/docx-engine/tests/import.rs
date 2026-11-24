mod common;

use common::*;
use docx_engine::{Document, NodeKind, StoryKind, Support};

fn texts(doc: &Document) -> Vec<String> {
    doc.paragraphs()
        .into_iter()
        .map(|p| doc.text(p).unwrap())
        .collect()
}

#[test]
fn simple_paragraphs_are_blocks_with_runs_and_text() {
    let doc = Document::open(&generated("simple-paragraphs")).unwrap();
    assert_eq!(
        texts(&doc),
        ["First paragraph.", "Second paragraph.", "Third paragraph."]
    );
    let kinds: Vec<&str> = doc
        .blocks()
        .iter()
        .map(|&b| doc.node(b).unwrap().kind.name())
        .collect();
    assert_eq!(kinds, ["paragraph", "paragraph", "paragraph", "section"]);
    let first = doc.node(doc.blocks()[0]).unwrap();
    let run = doc.node(first.children[0]).unwrap();
    assert_eq!(run.kind.name(), "run");
    assert!(
        matches!(&doc.node(run.children[0]).unwrap().kind, NodeKind::Text { text } if text == "First paragraph.")
    );
    assert!(doc.diagnostics().is_empty());
}

#[test]
fn inspecting_a_subtree_matches_its_view_in_the_full_document() {
    for (name, bytes) in all_fixtures() {
        let doc = Document::open(&bytes).unwrap();
        let inspection = doc.inspect();
        let mut pending: Vec<_> = inspection.stories.iter().collect();
        while let Some(expected) = pending.pop() {
            let id = expected.id;
            let actual = doc.inspect_node(id).unwrap();
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                serde_json::to_value(expected).unwrap(),
                "{name}: node {id}"
            );
            // Start partway through as well as at the first child, so an open
            // comment or an earlier list item must be accounted for.
            for start in 0..=expected.children.len().min(1) {
                let children = doc
                    .inspect_children(id, start..expected.children.len())
                    .unwrap();
                assert_eq!(
                    serde_json::to_value(children).unwrap(),
                    serde_json::to_value(&expected.children[start..]).unwrap(),
                    "{name}: children of {id} from {start}"
                );
            }
            pending.extend(&expected.children);
        }
    }
}

#[test]
fn ids_are_deterministic_for_the_same_bytes() {
    for (name, bytes) in all_fixtures() {
        let a = Document::open(&bytes).unwrap().inspect_json();
        let b = Document::open(&bytes).unwrap().inspect_json();
        assert_eq!(a, b, "{name}");
    }
}

#[test]
fn rich_text_keeps_run_formatting_entities_and_unicode() {
    let doc = Document::open(&generated("rich-text")).unwrap();
    let t = texts(&doc);
    assert_eq!(t[0], "Bold and italic and underlined.");
    assert_eq!(t[1], "Coloured text  leading and trailing spaces  ");
    assert_eq!(t[2], "Ampersand & angle <brackets> \"quotes\"");
    assert_eq!(t[3], "Emoji 😀, CJK 漢字, combining e\u{301}");
    assert_eq!(t[4], "Before tab\tafter tab\nnext line");

    let bold = doc.node(text_node(&doc, "Bold")).unwrap().parent.unwrap();
    let NodeKind::Run(info) = &doc.node(bold).unwrap().kind else {
        panic!()
    };
    assert_eq!(info.props.bold, Some(true));
    let coloured = doc
        .node(text_node(&doc, "Coloured"))
        .unwrap()
        .parent
        .unwrap();
    let effective = doc.effective_run_props(coloured);
    assert_eq!(effective.color.as_deref(), Some("C00000"));
    assert_eq!(effective.size_half_points, Some(28));
    assert_eq!(effective.font.as_deref(), Some("Georgia"));

    let NodeKind::Paragraph(p) = &doc.node(doc.blocks()[0]).unwrap().kind else {
        panic!()
    };
    assert_eq!(p.word_para_id.as_deref(), Some("1A2B3C4D"));
}

#[test]
fn styles_resolve_headings_and_inherited_formatting() {
    let doc = Document::open(&generated("headings-styles")).unwrap();
    let levels: Vec<Option<u8>> = doc
        .paragraphs()
        .into_iter()
        .map(|p| doc.heading_level(p))
        .collect();
    assert_eq!(levels, [Some(1), Some(1), None, Some(2), None, Some(3)]);

    let h2_run = doc
        .node(text_node(&doc, "Section 1.1"))
        .unwrap()
        .parent
        .unwrap();
    let effective = doc.effective_run_props(h2_run);
    // Heading2 is based on Heading1: bold and colour inherited, size overridden.
    assert_eq!(effective.bold, Some(true));
    assert_eq!(effective.color.as_deref(), Some("2F5496"));
    assert_eq!(effective.size_half_points, Some(26));
    assert_eq!(effective.font.as_deref(), Some("Calibri"));

    let emphasised = doc
        .node(text_node(&doc, "emphasised"))
        .unwrap()
        .parent
        .unwrap();
    assert_eq!(doc.effective_run_props(emphasised).italic, Some(true));
    let chain: Vec<&str> = doc
        .styles()
        .chain("Heading2")
        .iter()
        .map(|s| s.id.as_str())
        .collect();
    assert_eq!(chain, ["Normal", "Heading1", "Heading2"]);
}

#[test]
fn lists_resolve_numbering_from_paragraphs_and_styles() {
    let doc = Document::open(&generated("lists")).unwrap();
    let json = doc.inspect_json();
    let blocks = json["stories"][0]["children"].as_array().unwrap();
    let list = |i: usize| &blocks[i]["list"];
    assert_eq!(list(0)["definition"]["format"], "bullet");
    assert_eq!(list(1)["level"], 1);
    assert_eq!(list(3)["definition"]["format"], "decimal");
    assert_eq!(list(3)["definition"]["text"], "%1.");
    assert_eq!(list(4)["definition"]["format"], "lowerLetter");
    assert_eq!(
        list(5)["num_id"],
        "1",
        "numbering inherited from the ListBullet style"
    );
    assert!(list(6).is_null(), "numId 0 removes the style's numbering");
}

#[test]
fn tables_have_rows_cells_spans_merges_and_nesting() {
    let doc = Document::open(&generated("table")).unwrap();
    let table = doc.blocks()[1];
    let NodeKind::Table(info) = &doc.node(table).unwrap().kind else {
        panic!()
    };
    assert_eq!(info.grid, [3000, 3000, 3000]);
    assert_eq!(info.style.as_deref(), Some("TableGrid"));
    let rows = &doc.node(table).unwrap().children;
    assert_eq!(rows.len(), 3);
    let NodeKind::Row(header) = &doc.node(rows[0]).unwrap().kind else {
        panic!()
    };
    assert!(header.header);
    let row2 = &doc.node(rows[1]).unwrap().children;
    let NodeKind::Cell(span) = &doc.node(row2[1]).unwrap().kind else {
        panic!()
    };
    assert_eq!(span.grid_span, 2);
    let NodeKind::Cell(merged) = &doc.node(row2[0]).unwrap().kind else {
        panic!()
    };
    assert_eq!(merged.vertical_merge.as_deref(), Some("restart"));

    let inner = text_node(&doc, "Inner B");
    let ancestors: Vec<&str> = std::iter::successors(doc.node(inner).unwrap().parent, |&p| {
        doc.node(p).unwrap().parent
    })
    .map(|p| doc.node(p).unwrap().kind.name())
    .filter(|k| *k == "table")
    .collect();
    assert_eq!(ancestors.len(), 2, "nested table inside a cell");
    assert!(doc
        .diagnostics()
        .iter()
        .all(|d| d.code != "table_grid_mismatch"));
}

#[test]
fn images_resolve_their_media_relationship() {
    let doc = Document::open(&generated("image")).unwrap();
    let (node, drawing) = doc
        .paragraphs()
        .into_iter()
        .flat_map(|p| doc.descendants(p))
        .find_map(|id| match &doc.node(id).unwrap().kind {
            NodeKind::Drawing(info) => Some((id, info)),
            _ => None,
        })
        .unwrap();
    let view = serde_json::to_value(doc.inspect_node(node).unwrap()).unwrap();
    assert_eq!(view["id"], node.to_string());
    assert_eq!(view["drawing_id"], drawing.id.unwrap());
    assert_eq!(drawing.width_emu, Some(914400));
    assert_eq!(
        drawing.media[0].part.as_deref(),
        Some("word/media/image1.png")
    );
    assert_eq!(drawing.media[0].content_type.as_deref(), Some("image/png"));
    assert!(doc.diagnostics().is_empty());
}

#[test]
fn headers_and_footers_are_stories_with_field_results_read_only() {
    let doc = Document::open(&generated("header-footer")).unwrap();
    let kinds: Vec<(StoryKind, String)> = doc
        .stories()
        .map(|s| match &s.kind {
            NodeKind::Story { story, part } => (*story, part.clone()),
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(
        kinds,
        [
            (StoryKind::Body, "word/document.xml".to_string()),
            (StoryKind::Footer, "word/footer1.xml".to_string()),
            (StoryKind::Header, "word/header1.xml".to_string()),
        ]
    );
    let footer_page = doc.node(text_node(&doc, "1")).unwrap();
    assert!(
        matches!(footer_page.support, Support::ReadOnly(_)),
        "field result"
    );
    let header = doc.node(text_node(&doc, "Header text")).unwrap();
    assert_eq!(header.support, Support::Editable);
    let NodeKind::Section(section) = &doc.node(*doc.blocks().last().unwrap()).unwrap().kind else {
        panic!()
    };
    assert_eq!(section.headers[0].part.as_deref(), Some("word/header1.xml"));
}

#[test]
fn tracked_changes_are_read_only_revisions_with_attribution() {
    let doc = Document::open(&generated("tracked-changes")).unwrap();
    let t = texts(&doc);
    assert_eq!(
        t[0], "Kept text, inserted text, newly bold",
        "accepted view: insertions shown, deletions hidden"
    );
    assert_eq!(t[2], "", "the source of a move reads as deleted");
    assert_eq!(t[3], "Destination: Moved sentence.");

    let inserted = doc.node(text_node(&doc, "inserted text")).unwrap();
    assert!(matches!(inserted.support, Support::ReadOnly(_)));
    let revision = doc
        .node(doc.node(inserted.parent.unwrap()).unwrap().parent.unwrap())
        .unwrap();
    let NodeKind::Revision(info) = &revision.kind else {
        panic!()
    };
    assert_eq!(
        (info.change.as_str(), info.author.as_deref()),
        ("ins", Some("Alice"))
    );

    let bold_run = doc
        .node(text_node(&doc, "newly bold"))
        .unwrap()
        .parent
        .unwrap();
    let NodeKind::Run(run) = &doc.node(bold_run).unwrap().kind else {
        panic!()
    };
    assert!(run.format_revision);
    let NodeKind::Paragraph(p) = &doc.node(doc.blocks()[1]).unwrap().kind else {
        panic!()
    };
    assert_eq!(p.mark_revision.as_deref(), Some("ins"));
}

#[test]
fn unknown_ooxml_is_reported_as_opaque_and_read_only() {
    let doc = Document::open(&generated("unknown-ooxml")).unwrap();
    let json = doc.inspect_json();
    let summary: Vec<(String, String)> = json["unsupported"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| {
            (
                u["element"].as_str().unwrap().to_string(),
                u["support"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    for expected in [
        ("x:customBlock", "opaque"),
        ("x:inlineThing", "opaque"),
        ("x:runThing", "opaque"),
        ("w:customXml", "opaque"),
        ("m:oMath", "opaque"),
        ("mc:AlternateContent", "opaque"),
        ("w:smartTag", "opaque"),
        ("w:sdt", "read_only"),
        ("w:bookmarkStart", "read_only"),
        ("w:commentRangeStart", "read_only"),
    ] {
        assert!(
            summary.contains(&(expected.0.to_string(), expected.1.to_string())),
            "missing {expected:?} in {summary:?}"
        );
    }
    let opaque = doc
        .descendants(doc.stories().next().unwrap().id)
        .into_iter()
        .find_map(|id| match &doc.node(id).unwrap().kind {
            NodeKind::Opaque {
                element,
                text_preview,
                ..
            } if element == "w:customXml" => text_preview.clone(),
            _ => None,
        });
    assert_eq!(opaque.as_deref(), Some("Custom XML wrapped run"));
    let bound = doc.node(text_node(&doc, "Bound client name")).unwrap();
    assert!(matches!(bound.support, Support::ReadOnly(_)));
    let parts = &json["package"]["parts"];
    let comments = parts
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "word/comments.xml")
        .unwrap();
    // Comments are a read-only story, and untouched parts export as they came.
    assert_eq!(comments["handling"], "parsed");
}

#[test]
fn complex_fields_spanning_paragraphs_are_read_only() {
    let doc = Document::open(&generated("complex-mixed")).unwrap();
    for text in ["Chapter one ... 1", "Chapter two ... 2"] {
        let node = doc.node(text_node(&doc, text)).unwrap();
        assert!(
            matches!(node.support, Support::ReadOnly(_)),
            "{text} is a TOC field result"
        );
    }
    let after = doc.node(text_node(&doc, "for details.")).unwrap();
    assert_eq!(
        after.support,
        Support::Editable,
        "the field ended before this paragraph"
    );
    let link = doc
        .descendants(doc.stories().next().unwrap().id)
        .into_iter()
        .find_map(|id| match &doc.node(id).unwrap().kind {
            NodeKind::Hyperlink(info) => Some(info.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(link.target.as_deref(), Some("https://example.com/"));
    let sections = doc
        .paragraphs()
        .into_iter()
        .filter_map(|p| match &doc.node(p).unwrap().kind {
            NodeKind::Paragraph(info) => info.section.clone(),
            _ => None,
        })
        .count();
    assert_eq!(
        sections, 1,
        "the landscape section break inside a paragraph"
    );
}

#[test]
fn widow_control_defaults_on_and_honors_the_paragraph_setting() {
    let bytes = with_body(
        r#"<w:p/><w:p><w:pPr><w:widowControl/></w:pPr></w:p>
        <w:p><w:pPr><w:widowControl w:val="0"/></w:pPr></w:p>"#,
    );
    let doc = Document::open(&bytes).unwrap();
    let settings: Vec<_> = doc
        .paragraphs()
        .into_iter()
        .map(|p| doc.effective_paragraph(p).widow_control)
        .collect();
    assert_eq!(settings, [true, true, false]);
}

#[test]
fn nested_list_markers_use_the_format_of_each_referenced_level() {
    let bytes = generated("lists");
    let numbering = part(&bytes, "word/numbering.xml").replace("%2)", "%1.%2");
    let bytes = rewrite(&bytes, "word/numbering.xml", Some(numbering.as_bytes()));
    let doc = Document::open(&bytes).unwrap();
    let paragraph = doc.blocks()[4];
    assert_eq!(
        doc.inspect_json()["stories"][0]["children"][4]["list"]["marker"],
        "1.a"
    );
    assert_eq!(
        doc.inspect_node(paragraph).unwrap().list.unwrap().marker,
        "1.a"
    );
}

#[test]
fn table_placements_keep_an_orphan_and_leave_a_wrappers_columns_occupied() {
    let bytes = with_body(
        r#"<w:tbl><w:tblGrid><w:gridCol/><w:gridCol/></w:tblGrid>
        <w:tr><w:sdt><w:sdtContent><w:tc><w:p/></w:tc></w:sdtContent></w:sdt>
        <w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr><w:p/></w:tc></w:tr>
        <w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p><w:r><w:t>Orphan</w:t></w:r></w:p></w:tc>
        <w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc></w:tr></w:tbl>"#,
    );
    let doc = Document::open(&bytes).unwrap();
    let table = &doc.inspect_json()["stories"][0]["children"][0];
    let rows = &table["children"];
    for (row, cell, col, height) in [(0, 1, 1, 2), (1, 0, 0, 1), (1, 1, 1, 0)] {
        let id = rows[row]["children"][cell]["id"].as_str().unwrap();
        assert_eq!(
            table["cells"][id],
            serde_json::json!({"column": col, "rows": height})
        );
    }
}
