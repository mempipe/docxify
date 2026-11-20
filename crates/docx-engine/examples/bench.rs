//! Times the engine on one large generated document.
//!
//!     cargo run --release -p docx-engine --example bench [paragraphs]
//!
//! The document has `paragraphs` paragraphs of three runs each (default
//! 10,000) and a 3×3 table after every hundredth. It is also written to
//! `target/bench/large-<paragraphs>.docx` so other tools can reuse it. Peak
//! memory comes from the OS, e.g. `/usr/bin/time -l` on macOS.

use std::io::{Cursor, Write};
use std::path::Path;
use std::time::Instant;

use docx_engine::{editing::Editor, validate_package, Command, Document, Node, NodeKind, Support};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn main() {
    let paragraphs: usize = std::env::args()
        .nth(1)
        .map_or(10_000, |n| n.parse().expect("a paragraph count"));
    let bytes = generate(paragraphs);
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/bench");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("large-{paragraphs}.docx")), &bytes).unwrap();

    let t = Instant::now();
    let parsed = Document::open(&bytes).unwrap();
    let parse = t.elapsed();
    drop(parsed);
    let t = Instant::now();
    let mut editor = Editor::open(&bytes).unwrap();
    let doc = editor.view().unwrap();
    let nodes = doc.node_count();
    let open = t.elapsed();
    let t = Instant::now();
    let json = serde_json::to_vec(&doc.inspect()).unwrap();
    let inspect = t.elapsed();

    let roots: Vec<_> = doc.stories().map(|s| s.id).collect();
    let texts: Vec<_> = roots
        .into_iter()
        .flat_map(|r| doc.descendants(r))
        .filter(|&id| {
            let node = doc.node(id).unwrap();
            matches!(node.kind, NodeKind::Text { .. }) && node.support == Support::Editable
        })
        .collect();
    let target = texts[texts.len() / 2];
    let t = Instant::now();
    editor
        .apply(&Command::ReplaceText {
            node: target,
            range: None,
            text: "Edited in the middle".into(),
        })
        .unwrap();
    let edit = t.elapsed();
    let t = Instant::now();
    let out = editor.export().unwrap();
    let export = t.elapsed();
    let t = Instant::now();
    let report = validate_package(&out);
    let validate = t.elapsed();
    assert!(report.valid, "{:?}", report.issues);

    println!(
        "{paragraphs} paragraphs: {} KiB DOCX, {} nodes of {} bytes each, inspection JSON {} KiB",
        bytes.len() / 1024,
        nodes,
        std::mem::size_of::<Node>(),
        json.len() / 1024
    );
    println!(
        "parse {parse:.1?}, open editor {open:.1?}, inspect {inspect:.1?}, edit {edit:.1?}, export {export:.1?}, validate {validate:.1?}"
    );
}

fn generate(paragraphs: usize) -> Vec<u8> {
    let mut body = String::new();
    for i in 0..paragraphs {
        body.push_str(&format!(
            r#"<w:p><w:r><w:t xml:space="preserve">Paragraph {i} begins with plain text, </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t xml:space="preserve">continues in bold, </w:t></w:r><w:r><w:t>and ends plainly.</w:t></w:r></w:p>"#
        ));
        if i % 100 == 99 {
            body.push_str(r#"<w:tbl><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid>"#);
            for r in 0..3 {
                body.push_str("<w:tr>");
                for c in 0..3 {
                    body.push_str(&format!(
                        r#"<w:tc><w:p><w:r><w:t>Cell {r},{c}</w:t></w:r></w:p></w:tc>"#
                    ));
                }
                body.push_str("</w:tr>");
            }
            body.push_str("</w:tbl>");
        }
    }
    let decl = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;
    let entries = [
        (
            "[Content_Types].xml",
            format!(
                r#"{decl}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#
            ),
        ),
        (
            "_rels/.rels",
            format!(
                r#"{decl}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}/officeDocument" Target="word/document.xml"/></Relationships>"#
            ),
        ),
        (
            "word/document.xml",
            format!(
                r#"{decl}<w:document xmlns:w="{W}" xmlns:r="{REL}"><w:body>{body}<w:sectPr/></w:body></w:document>"#
            ),
        ),
    ];
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(DateTime::default());
    for (name, data) in entries {
        writer.start_file(name, options).unwrap();
        writer.write_all(data.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}
