#![allow(dead_code)]

use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use docx_engine::{validate_package, Document, NodeId, NodeKind, Point, Support};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub const GENERATED: [&str; 17] = [
    "simple-paragraphs",
    "rich-text",
    "headings-styles",
    "lists",
    "table",
    "image",
    "header-footer",
    "tracked-changes",
    "unknown-ooxml",
    "complex-mixed",
    "notes",
    "sections",
    "paragraph-layout",
    "table-layout",
    "anchored-image",
    "comments",
    "content-controls",
];

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

pub fn generated(name: &str) -> Vec<u8> {
    std::fs::read(
        fixtures_dir()
            .join("generated")
            .join(format!("{name}.docx")),
    )
    .unwrap()
}

/// Every `.docx` under `fixtures/`, generated and real-world.
pub fn all_fixtures() -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for dir in ["generated", "real-world"] {
        let Ok(entries) = std::fs::read_dir(fixtures_dir().join(dir)) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries.map(|e| e.unwrap().path()).collect();
        paths.sort();
        for path in paths
            .into_iter()
            .filter(|p| p.extension().is_some_and(|e| e == "docx"))
        {
            let name = format!("{dir}/{}", path.file_name().unwrap().to_string_lossy());
            out.push((name, std::fs::read(&path).unwrap()));
        }
    }
    out
}

/// Entry names and decompressed contents, in archive order.
pub fn entries(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..archive.len())
        .map(|i| {
            let mut f = archive.by_index(i).unwrap();
            let mut data = Vec::new();
            f.read_to_end(&mut data).unwrap();
            (f.name().unwrap().to_string(), data)
        })
        .collect()
}

/// Raw (compressed) entry data, to check entries were copied without recompression.
pub fn raw_entries(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..archive.len())
        .map(|i| {
            let mut f = archive.by_index_raw(i).unwrap();
            let mut data = Vec::new();
            f.read_to_end(&mut data).unwrap();
            (f.name().unwrap().to_string(), data)
        })
        .collect()
}

pub fn part(bytes: &[u8], name: &str) -> String {
    let (_, data) = entries(bytes).into_iter().find(|(n, _)| n == name).unwrap();
    String::from_utf8(data).unwrap()
}

/// Rebuild a package with one entry replaced (`Some`) or removed (`None`).
pub fn rewrite(bytes: &[u8], name: &str, replacement: Option<&[u8]>) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (n, data) in entries(bytes) {
        let data = if n == name {
            match replacement {
                Some(r) => r.to_vec(),
                None => continue,
            }
        } else {
            data
        };
        writer.start_file(n, options).unwrap();
        writer.write_all(&data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

/// Rebuild a package with an entry added at the end.
pub fn with_entry(bytes: &[u8], name: &str, data: &[u8]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (n, d) in entries(bytes)
        .into_iter()
        .chain([(name.to_string(), data.to_vec())])
    {
        writer.start_file(n, options).unwrap();
        writer.write_all(&d).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

pub fn assert_valid(name: &str, bytes: &[u8]) {
    let report = validate_package(bytes);
    assert!(
        report.valid,
        "{name} failed validation: {:#?}",
        report.issues
    );
}

/// The first text node whose text contains `needle`.
pub fn text_node(doc: &Document, needle: &str) -> NodeId {
    find_text(doc, needle).unwrap_or_else(|| panic!("no text node containing {needle:?}"))
}

pub fn find_text(doc: &Document, needle: &str) -> Option<NodeId> {
    let roots: Vec<NodeId> = doc.stories().map(|s| s.id).collect();
    roots.into_iter().flat_map(|r| doc.descendants(r)).find(|&id| {
        matches!(&doc.node(id).unwrap().kind, NodeKind::Text { text } if text.contains(needle))
    })
}

pub fn first_editable_text(doc: &Document) -> Option<NodeId> {
    let roots: Vec<NodeId> = doc.stories().map(|s| s.id).collect();
    roots
        .into_iter()
        .flat_map(|r| doc.descendants(r))
        .find(|&id| {
            let node = doc.node(id).unwrap();
            matches!(&node.kind, NodeKind::Text { text } if !text.is_empty())
                && node.support == Support::Editable
        })
}

pub fn paragraph_with(doc: &Document, needle: &str) -> NodeId {
    doc.paragraphs()
        .into_iter()
        .find(|&p| doc.text(p).unwrap().contains(needle))
        .unwrap_or_else(|| panic!("no paragraph containing {needle:?}"))
}

pub fn texts(doc: &Document) -> Vec<String> {
    doc.paragraphs()
        .into_iter()
        .map(|p| doc.text(p).unwrap())
        .collect()
}

pub fn stories(doc: &Document) -> serde_json::Value {
    doc.inspect_json()["stories"].clone()
}

/// `<w:t ...>` and `<w:t/>` as `<w:t>` and `<w:t></w:t>`: what regenerating
/// an unchanged text element may alter.
pub fn normalized(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(i) = rest.find("<w:t") {
        let after = &rest[i + 4..];
        if !after.starts_with([' ', '>', '/']) {
            out.push_str(&rest[..i + 4]);
            rest = after;
            continue;
        }
        let end = after.find('>').unwrap();
        out.push_str(&rest[..i]);
        out.push_str(if after[..end].ends_with('/') {
            "<w:t></w:t>"
        } else {
            "<w:t>"
        });
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

pub fn reopen(name: &str, doc: &Document) -> Document {
    let out = doc.export().unwrap();
    assert_valid(name, &out);
    Document::open(&out).unwrap()
}

pub fn at(node: NodeId, offset: usize) -> Point {
    Point::Text { node, offset }
}

/// `simple-paragraphs` with its body replaced.
pub fn with_body(body: &str) -> Vec<u8> {
    let bytes = generated("simple-paragraphs");
    let xml = part(&bytes, "word/document.xml");
    let (start, end) = (
        xml.find("<w:body>").unwrap() + 8,
        xml.find("<w:sectPr").unwrap(),
    );
    let xml = format!("{}{body}{}", &xml[..start], &xml[end..]);
    rewrite(&bytes, "word/document.xml", Some(xml.as_bytes()))
}
