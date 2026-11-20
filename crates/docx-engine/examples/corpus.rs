//! Run every `.docx` under a directory through the engine and report.
//!
//!     cargo run --release -p docx-engine --example corpus -- <dir>
//!
//! For each file: open, inspect, export unchanged, edit the first editable
//! text node, export, validate both exports, and re-open the edited one.
//! Then, from a fresh open, split the first paragraph that can be split in the
//! middle of its text, and separately merge the first pair that can be merged.
//! Separately again, replace the first range that can be replaced, from the
//! middle of one paragraph's text to the middle of the text two paragraphs on,
//! with two lines, and the same as a change tracked by "Corpus run", which
//! must re-open with its insertions and deletions, each with an ID no other
//! revision has. Comment on the first such range that can take a comment,
//! and separately reply to the first comment the file holds that can take a
//! reply; each must re-open with the same comments, threads and markers, its
//! comment with an ID no other comment has. Bold the first such range that is not all bold, give the
//! first such range a paragraph style that some of it lacks, bullet the first
//! such range not all bulleted, move the first such range whose list can
//! go deeper one level down, and link the first such range that can be
//! linked. In the first table that allows each, add a row below the first
//! row, add a column after the first column, remove the first column and
//! remove the last row. Put a picture in the middle of the first paragraph
//! that takes one. Each is validated, re-opened to count paragraphs and
//! compare all text, run formatting, paragraph styles, lists, links, table
//! shapes and pictures, and undone, which must give back the freshly opened
//! inspection and the package's parts.
//! Fill every content control a form fill takes: text, the other state of a
//! check box, a list item other than the last chosen, or a date. The export
//! must validate and re-open with what each control showed and held, and
//! undoing every fill must give back the inspection and every part's
//! uncompressed bytes. Refusals are counted by control type and reason.
//! Update the body's fields as of a fixed time, with each body paragraph on
//! a made-up page. The export must validate and re-open with no field left
//! to update, and undoing must give back the inspection and every part's
//! uncompressed bytes.
//! Validation of the input is reported separately, so problems the input
//! already had are not blamed on the engine.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::time::Instant;

use docx_engine::{
    editing::{Editor, SessionError},
    validate_package, Command, Control, Document, EditError, Fill, Format, ListKind, NodeId,
    NodeKind, Pages, Point, Support, Track,
};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("docx"))
        {
            out.push(path);
        }
    }
}

fn raw_entries(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..archive.len())
        .map(|i| {
            let mut f = archive.by_index_raw(i).unwrap();
            let mut data = Vec::new();
            f.read_to_end(&mut data).unwrap();
            (f.name().unwrap().to_string(), data)
        })
        .collect()
}

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).expect("usage: corpus <dir>"));
    let mut files = Vec::new();
    walk(&dir, &mut files);
    files.sort();

    let mut tally: BTreeMap<&str, usize> = BTreeMap::new();
    let mut problems = Vec::new();
    let mut opaque: BTreeMap<String, usize> = BTreeMap::new();
    let mut refusals: BTreeMap<String, usize> = BTreeMap::new();
    let mut fill_refusals: BTreeMap<String, usize> = BTreeMap::new();
    let (mut open_ms, mut export_ms, mut bytes_total) = (0.0, 0.0, 0usize);

    for path in &files {
        let name = path.strip_prefix(&dir).unwrap().display().to_string();
        let bytes = std::fs::read(path).unwrap();
        bytes_total += bytes.len();
        let input_valid = validate_package(&bytes).valid;
        if !input_valid {
            *tally.entry("input_invalid").or_default() += 1;
        }

        let t = Instant::now();
        let doc = match Document::open(&bytes) {
            Ok(doc) => doc,
            Err(e) => {
                *tally.entry("open_failed").or_default() += 1;
                problems.push(format!("{name}: open: {e}"));
                continue;
            }
        };
        open_ms += t.elapsed().as_secs_f64() * 1000.0;
        *tally.entry("opened").or_default() += 1;
        let _ = doc.inspect_json();
        for id in (0..doc.node_count()).map(|i| format!("n{i}").parse().unwrap()) {
            if let NodeKind::Opaque { element, .. } = &doc.node(id).unwrap().kind {
                *opaque.entry(element.clone()).or_default() += 1;
            }
        }

        let unchanged = doc.export().unwrap();
        if raw_entries(&unchanged) != raw_entries(&bytes) {
            problems.push(format!("{name}: unchanged export differs"));
        }
        if input_valid && !validate_package(&unchanged).valid {
            problems.push(format!("{name}: unchanged export invalid"));
        }
        let mut structural = Structural {
            name: &name,
            bytes: &bytes,
            input_valid,
            tally: &mut tally,
            problems: &mut problems,
            refusals: &mut refusals,
            fill_refusals: &mut fill_refusals,
        };
        structural.split();
        structural.merge();
        structural.range();
        structural.tracked();
        structural.comment();
        structural.reply();
        structural.format();
        structural.style();
        structural.bullets();
        structural.level();
        structural.link();
        structural.tables();
        structural.picture();
        structural.update_fields();
        structural.fill_controls();
        structural.field_results();

        let mut editor = Editor::open(&bytes).unwrap();
        let doc = editor.view().unwrap();
        let target = doc
            .stories()
            .map(|s| s.id)
            .collect::<Vec<_>>()
            .into_iter()
            .flat_map(|s| doc.descendants(s))
            .find(|&id| {
                let node = doc.node(id).unwrap();
                node.support == Support::Editable
                    && matches!(&node.kind, NodeKind::Text { text } if !text.is_empty())
            });
        let Some(target) = target else {
            *tally.entry("no_editable_text").or_default() += 1;
            continue;
        };
        editor
            .apply(&Command::ReplaceText {
                node: target,
                range: None,
                text: "Edited by the corpus run".into(),
            })
            .unwrap();
        let t = Instant::now();
        let edited = editor.export().unwrap();
        export_ms += t.elapsed().as_secs_f64() * 1000.0;
        *tally.entry("edited").or_default() += 1;

        if input_valid {
            let report = validate_package(&edited);
            if !report.valid {
                problems.push(format!(
                    "{name}: edited export invalid: {:?}",
                    report.issues.first()
                ));
                continue;
            }
        }
        match Document::open(&edited) {
            Ok(re) if all_text(&re).contains("Edited by the corpus run") => {
                *tally.entry("edit_verified").or_default() += 1;
            }
            Ok(_) => problems.push(format!(
                "{name}: edited text not found at {target} after re-open"
            )),
            Err(e) => problems.push(format!("{name}: re-open failed: {e}")),
        }
    }

    println!(
        "files: {} ({:.1} MB)",
        files.len(),
        bytes_total as f64 / 1e6
    );
    for (k, v) in &tally {
        println!("{k}: {v}");
    }
    println!("open total: {open_ms:.1} ms, edited-export total: {export_ms:.1} ms");
    let mut top: Vec<_> = opaque.into_iter().collect();
    top.sort_by_key(|t| std::cmp::Reverse(t.1));
    println!(
        "most common opaque elements: {:?}",
        &top[..top.len().min(15)]
    );
    let mut reasons: Vec<_> = refusals.into_iter().collect();
    reasons.sort_by_key(|r| std::cmp::Reverse(r.1));
    println!(
        "why a file had no split, merge, range, tracked range, comment, reply, bold, style, bullets, level, link, table change or picture:"
    );
    for (reason, count) in &reasons {
        println!("  {count}: {reason}");
    }
    let mut reasons: Vec<_> = fill_refusals.into_iter().collect();
    reasons.sort_by_key(|r| std::cmp::Reverse(r.1));
    println!("why a content control took no fill:");
    for (reason, count) in &reasons {
        println!("  {count}: {reason}");
    }
    println!("problems: {}", problems.len());
    for p in &problems {
        println!("  {p}");
    }
    if !problems.is_empty() {
        std::process::exit(1);
    }
}

struct Structural<'a> {
    name: &'a str,
    bytes: &'a [u8],
    input_valid: bool,
    tally: &'a mut BTreeMap<&'static str, usize>,
    problems: &'a mut Vec<String>,
    refusals: &'a mut BTreeMap<String, usize>,
    fill_refusals: &'a mut BTreeMap<String, usize>,
}

/// The content controls of every story, in order.
fn content_controls(doc: &Document) -> Vec<NodeId> {
    let stories: Vec<NodeId> = doc.stories().map(|s| s.id).collect();
    stories
        .into_iter()
        .flat_map(|s| doc.descendants(s))
        .filter(|&id| matches!(doc.node(id).unwrap().kind, NodeKind::ContentControl { .. }))
        .collect()
}

/// What a content control shows, a line break as a line feed, and its
/// type and placeholder flag as inspection reports them.
fn control_state(doc: &Document, id: NodeId) -> (String, String) {
    let shown = doc
        .descendants(id)
        .into_iter()
        .filter_map(|d| match &doc.node(d).unwrap().kind {
            NodeKind::Text { text } => Some(text.clone()),
            NodeKind::Break { .. } => Some("\n".to_string()),
            _ => None,
        })
        .collect();
    let NodeKind::ContentControl {
        control,
        placeholder,
        ..
    } = &doc.node(id).unwrap().kind
    else {
        unreachable!("a content control");
    };
    (shown, format!("{control:?} {placeholder}"))
}

fn all_text(doc: &Document) -> String {
    doc.paragraphs()
        .iter()
        .map(|&p| doc.text(p).unwrap())
        .collect()
}

/// Every character, and every other item in a run as U+FFFC, with the bold,
/// italic and underline its run shows.
fn formatting(doc: &Document) -> Vec<(char, bool, bool, bool)> {
    let mut out = Vec::new();
    for p in doc.paragraphs() {
        for id in doc.descendants(p) {
            let node = doc.node(id).unwrap();
            let Some(run) = node
                .parent
                .filter(|&r| matches!(doc.node(r).unwrap().kind, NodeKind::Run(_)))
            else {
                continue;
            };
            let props = doc.effective_run_props(run);
            let shown = (
                props.bold.unwrap_or(false),
                props.italic.unwrap_or(false),
                props.underline.as_deref().is_some_and(|u| u != "none"),
            );
            let item = |c| (c, shown.0, shown.1, shown.2);
            match &node.kind {
                NodeKind::Text { text } => out.extend(text.chars().map(item)),
                _ => out.push(item('\u{FFFC}')),
            }
        }
    }
    out
}

/// Each paragraph's style ID, the default style's when it names none.
fn paragraph_styles(doc: &Document) -> Vec<Option<String>> {
    let default = doc.styles().default_paragraph_style().map(|s| &s.id);
    doc.paragraphs()
        .into_iter()
        .map(|p| match &doc.node(p).unwrap().kind {
            NodeKind::Paragraph(info) => info.style.as_ref().or(default).cloned(),
            _ => None,
        })
        .collect()
}

/// Each paragraph's list as the inspection shows it: number, level and
/// marker format.
fn lists(doc: &Document) -> Vec<Option<(String, u64, String)>> {
    fn walk(node: &serde_json::Value, out: &mut Vec<Option<(String, u64, String)>>) {
        if node["type"] == "paragraph" {
            out.push(node.get("list").filter(|l| !l.is_null()).map(|l| {
                (
                    l["num_id"].as_str().unwrap_or("").to_string(),
                    l["level"].as_u64().unwrap_or(0),
                    l["definition"]["format"].as_str().unwrap_or("").to_string(),
                )
            }));
        }
        for child in node["children"].as_array().into_iter().flatten() {
            walk(child, out);
        }
    }
    let mut out = Vec::new();
    for story in doc.inspect_json()["stories"].as_array().unwrap() {
        walk(story, &mut out);
    }
    out
}

/// Each link as the inspection shows it: its text and where it points.
fn links(doc: &Document) -> Vec<(String, String)> {
    fn text(node: &serde_json::Value) -> String {
        let own = node["text"].as_str().filter(|_| node["type"] == "text");
        let inner: String = node["children"]
            .as_array()
            .into_iter()
            .flatten()
            .map(text)
            .collect();
        format!("{}{inner}", own.unwrap_or(""))
    }
    fn walk(node: &serde_json::Value, out: &mut Vec<(String, String)>) {
        if node["type"] == "hyperlink" {
            let target = node["target"].as_str().unwrap_or("");
            let anchor = node["anchor"].as_str().map(|a| format!("#{a}"));
            out.push((
                text(node),
                format!("{target}{}", anchor.unwrap_or_default()),
            ));
        }
        for child in node["children"].as_array().into_iter().flatten() {
            walk(child, out);
        }
    }
    let mut out = Vec::new();
    for story in doc.inspect_json()["stories"].as_array().unwrap() {
        walk(story, &mut out);
    }
    out
}

/// Each table's grid widths and each of its rows' cell count.
fn table_shapes(doc: &Document) -> Vec<(Vec<u32>, Vec<usize>)> {
    let roots: Vec<NodeId> = doc.stories().map(|s| s.id).collect();
    let children = |id: NodeId, kind: fn(&NodeKind) -> bool| -> Vec<NodeId> {
        doc.node(id)
            .unwrap()
            .children
            .iter()
            .copied()
            .filter(|&c| kind(&doc.node(c).unwrap().kind))
            .collect()
    };
    roots
        .into_iter()
        .flat_map(|r| doc.descendants(r))
        .filter_map(|id| match &doc.node(id).unwrap().kind {
            NodeKind::Table(info) => Some((
                info.grid.clone(),
                children(id, |k| matches!(k, NodeKind::Row(_)))
                    .into_iter()
                    .map(|r| children(r, |k| matches!(k, NodeKind::Cell(_))).len())
                    .collect(),
            )),
            _ => None,
        })
        .collect()
}

/// A drawing's ID, media and size.
type Picture = (Option<u32>, Vec<Option<String>>, Option<u64>, Option<u64>);

/// Each drawing, in document order.
fn pictures(doc: &Document) -> Vec<Picture> {
    let roots: Vec<NodeId> = doc.stories().map(|s| s.id).collect();
    roots
        .into_iter()
        .flat_map(|r| doc.descendants(r))
        .filter_map(|id| match &doc.node(id).unwrap().kind {
            NodeKind::Drawing(info) => Some((
                info.id,
                info.media.iter().map(|m| m.part.clone()).collect(),
                info.width_emu,
                info.height_emu,
            )),
            _ => None,
        })
        .collect()
}

/// A tracked change: what it records, who made it, and when. A paragraph
/// mark's change is named `mark ins` or `mark del`.
type Revision = (String, Option<String>, Option<String>);
/// A change wrapper's revision ID and author.
type Wrapper = (Option<String>, Option<String>);

const CORPUS: &str = "Corpus run";

fn corpus_track() -> Track {
    Track {
        author: CORPUS.into(),
        date: "2026-11-16T00:00:00Z".into(),
    }
}

/// Each tracked change, in document order, with the IDs of the wrappers.
fn revisions(doc: &Document) -> (Vec<Revision>, Vec<Wrapper>) {
    let roots: Vec<NodeId> = doc.stories().map(|s| s.id).collect();
    let (mut changes, mut ids) = (Vec::new(), Vec::new());
    for id in roots.into_iter().flat_map(|r| doc.descendants(r)) {
        match &doc.node(id).unwrap().kind {
            NodeKind::Revision(info) => {
                changes.push((info.change.clone(), info.author.clone(), info.date.clone()));
                ids.push((info.revision_id.clone(), info.author.clone()));
            }
            NodeKind::Paragraph(info) => {
                if let Some(change) = &info.mark_revision {
                    let by = info.mark_change.as_ref();
                    let (author, date) = (
                        by.and_then(|t| t.author.clone()),
                        by.and_then(|t| t.date.clone()),
                    );
                    changes.push((format!("mark {change}"), author, date));
                }
            }
            _ => {}
        }
    }
    (changes, ids)
}

/// The one-pixel PNG of the `image` fixture.
fn png() -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/generated/image.docx");
    let doc = Document::open(&std::fs::read(path).unwrap()).unwrap();
    doc.part_bytes("word/media/image1.png").unwrap()
}

/// A comment: its ID, author, the comment it replies to, and its text.
type Remark = (String, Option<String>, Option<String>, String);

/// Each comment, in the order its part holds them.
fn comments(doc: &Document) -> Vec<Remark> {
    let roots: Vec<NodeId> = doc.stories().map(|s| s.id).collect();
    roots
        .into_iter()
        .flat_map(|r| doc.descendants(r))
        .filter_map(|id| match &doc.node(id).unwrap().kind {
            NodeKind::Comment {
                comment_id,
                author,
                parent,
                ..
            } => {
                let text: Vec<String> = doc
                    .descendants(id)
                    .into_iter()
                    .filter_map(|n| match &doc.node(n).unwrap().kind {
                        NodeKind::Text { text } => Some(text.clone()),
                        _ => None,
                    })
                    .collect();
                Some((
                    comment_id.clone(),
                    author.clone(),
                    parent.clone(),
                    text.concat(),
                ))
            }
            _ => None,
        })
        .collect()
}

/// The comment markers in document order: their element and comment ID.
fn comment_markers(doc: &Document) -> Vec<(String, Option<String>)> {
    let roots: Vec<NodeId> = doc.stories().map(|s| s.id).collect();
    roots
        .into_iter()
        .flat_map(|r| doc.descendants(r))
        .filter_map(|id| match &doc.node(id).unwrap().kind {
            NodeKind::Marker {
                element, marker_id, ..
            } if element.contains(":comment") => Some((element.clone(), marker_id.clone())),
            _ => None,
        })
        .collect()
}

/// Whether each change and comment this run made has an ID no other change,
/// or no other comment, has.
fn distinct_ids(doc: &Document) -> bool {
    let ids = revisions(doc).1;
    let comments = comments(doc);
    ids.iter()
        .filter(|(_, author)| author.as_deref() == Some(CORPUS))
        .all(|(id, _)| id.is_some() && ids.iter().filter(|(other, _)| other == id).count() == 1)
        && comments
            .iter()
            .filter(|c| c.1.as_deref() == Some(CORPUS))
            .all(|c| comments.iter().filter(|other| other.0 == c.0).count() == 1)
}

/// Each part's name and its bytes, uncompressed.
fn entries(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..archive.len())
        .map(|i| {
            let mut f = archive.by_index(i).unwrap();
            let mut data = Vec::new();
            f.read_to_end(&mut data).unwrap();
            (f.name().unwrap().to_string(), data)
        })
        .collect()
}

fn entry_names(bytes: &[u8]) -> Vec<String> {
    raw_entries(bytes).into_iter().map(|e| e.0).collect()
}

/// The middle of the first editable text in `p` with two characters or more.
fn middle(doc: &Document, p: NodeId) -> Option<Point> {
    doc.descendants(p).into_iter().find_map(|id| {
        let node = doc.node(id).unwrap();
        match &node.kind {
            NodeKind::Text { text }
                if node.support == Support::Editable && text.chars().count() >= 2 =>
            {
                Some(Point::Text {
                    node: id,
                    offset: text.chars().count() / 2,
                })
            }
            _ => None,
        }
    })
}

impl Structural<'_> {
    fn split(&mut self) {
        let mut doc = Document::open(self.bytes).unwrap();
        let candidates: Vec<(NodeId, usize)> = doc
            .paragraphs()
            .into_iter()
            .flat_map(|p| doc.descendants(p))
            .filter_map(|id| {
                let node = doc.node(id).unwrap();
                match &node.kind {
                    NodeKind::Text { text }
                        if node.support == Support::Editable && text.chars().count() >= 2 =>
                    {
                        Some((id, text.chars().count() / 2))
                    }
                    _ => None,
                }
            })
            .collect();
        let commands = candidates
            .into_iter()
            .map(|(node, offset)| Command::SplitParagraph {
                at: Point::Text { node, offset },
            });
        self.check("split", &mut doc, commands, Some(1));
    }

    fn merge(&mut self) {
        let mut doc = Document::open(self.bytes).unwrap();
        let commands: Vec<Command> = doc
            .paragraphs()
            .into_iter()
            .map(|paragraph| Command::MergeParagraphs { paragraph })
            .collect();
        self.check("merge", &mut doc, commands, Some(-1));
    }

    fn range(&mut self) {
        let mut doc = Document::open(self.bytes).unwrap();
        let paragraphs = doc.paragraphs();
        let commands: Vec<Command> = paragraphs
            .windows(3)
            .filter_map(|w| {
                Some(Command::ReplaceRange {
                    from: middle(&doc, w[0])?,
                    to: middle(&doc, w[2])?,
                    text: "corpus\nrun".into(),
                    track: None,
                })
            })
            .collect();
        self.check("range", &mut doc, commands, None);
    }

    fn tracked(&mut self) {
        let mut doc = Document::open(self.bytes).unwrap();
        let track = corpus_track();
        let paragraphs = doc.paragraphs();
        let commands: Vec<Command> = paragraphs
            .windows(3)
            .filter_map(|w| {
                Some(Command::ReplaceRange {
                    from: middle(&doc, w[0])?,
                    to: middle(&doc, w[2])?,
                    text: "corpus\nrun".into(),
                    track: Some(track.clone()),
                })
            })
            .collect();
        self.check("tracked", &mut doc, commands, None);
    }

    fn comment(&mut self) {
        let mut doc = Document::open(self.bytes).unwrap();
        let paragraphs = doc.paragraphs();
        let commands: Vec<Command> = paragraphs
            .windows(3)
            .filter_map(|w| {
                Some(Command::AddComment {
                    from: middle(&doc, w[0])?,
                    to: middle(&doc, w[2])?,
                    text: "corpus\nrun".into(),
                    by: corpus_track(),
                    initials: Some("CR".into()),
                    id: None,
                })
            })
            .collect();
        self.check("comment", &mut doc, commands, None);
    }

    fn reply(&mut self) {
        let mut doc = Document::open(self.bytes).unwrap();
        let commands: Vec<Command> = comments(&doc)
            .into_iter()
            .map(|c| Command::ReplyToComment {
                comment: c.0,
                text: "corpus run".into(),
                by: corpus_track(),
                initials: None,
                id: None,
            })
            .collect();
        self.check("reply", &mut doc, commands, None);
    }

    fn format(&mut self) {
        let mut doc = Document::open(self.bytes).unwrap();
        let bold = Format {
            bold: Some(true),
            ..Format::default()
        };
        let paragraphs = doc.paragraphs();
        let commands: Vec<Command> = paragraphs
            .windows(3)
            .filter_map(|w| {
                Some(Command::FormatRange {
                    from: middle(&doc, w[0])?,
                    to: middle(&doc, w[2])?,
                    format: bold,
                })
            })
            .collect();
        self.check("bold", &mut doc, commands, Some(0));
    }

    /// Heading 1, or else the first paragraph style by ID that is not the
    /// default.
    fn style(&mut self) {
        let mut doc = Document::open(self.bytes).unwrap();
        let style = doc
            .styles()
            .styles
            .values()
            .filter(|s| s.kind == "paragraph" && !s.is_default)
            .map(|s| s.id.clone())
            .min_by_key(|id| (id != "Heading1", id.clone()));
        let Some(style) = style else {
            *self.tally.entry("no_style").or_default() += 1;
            *self
                .refusals
                .entry("style: no paragraph style besides the default".into())
                .or_default() += 1;
            return;
        };
        let paragraphs = doc.paragraphs();
        let commands: Vec<Command> = paragraphs
            .windows(3)
            .filter_map(|w| {
                Some(Command::SetParagraphStyle {
                    from: middle(&doc, w[0])?,
                    to: middle(&doc, w[2])?,
                    style: style.clone(),
                })
            })
            .collect();
        self.check("style", &mut doc, commands, Some(0));
    }

    fn bullets(&mut self) {
        let mut doc = Document::open(self.bytes).unwrap();
        let paragraphs = doc.paragraphs();
        let commands: Vec<Command> = paragraphs
            .windows(3)
            .filter_map(|w| {
                Some(Command::SetList {
                    from: middle(&doc, w[0])?,
                    to: middle(&doc, w[2])?,
                    kind: Some(ListKind::Bullet),
                })
            })
            .collect();
        self.check("bullets", &mut doc, commands, Some(0));
    }

    fn level(&mut self) {
        let mut doc = Document::open(self.bytes).unwrap();
        let paragraphs = doc.paragraphs();
        let commands: Vec<Command> = paragraphs
            .windows(3)
            .filter_map(|w| {
                Some(Command::ChangeListLevel {
                    from: middle(&doc, w[0])?,
                    to: middle(&doc, w[2])?,
                    by: 1,
                })
            })
            .collect();
        self.check("level", &mut doc, commands, Some(0));
    }

    fn link(&mut self) {
        let mut doc = Document::open(self.bytes).unwrap();
        let paragraphs = doc.paragraphs();
        let commands: Vec<Command> = paragraphs
            .windows(3)
            .filter_map(|w| {
                Some(Command::SetLink {
                    from: middle(&doc, w[0])?,
                    to: middle(&doc, w[2])?,
                    target: Some("https://example.com/corpus".into()),
                })
            })
            .collect();
        self.check("link", &mut doc, commands, Some(0));
    }

    fn tables(&mut self) {
        let doc = Document::open(self.bytes).unwrap();
        let roots: Vec<NodeId> = doc.stories().map(|s| s.id).collect();
        let tables: Vec<(NodeId, Vec<NodeId>)> = roots
            .into_iter()
            .flat_map(|r| doc.descendants(r))
            .filter(|&id| matches!(doc.node(id).unwrap().kind, NodeKind::Table(_)))
            .map(|t| {
                let rows = doc.node(t).unwrap().children.iter().copied();
                let rows = rows.filter(|&r| matches!(doc.node(r).unwrap().kind, NodeKind::Row(_)));
                (t, rows.collect())
            })
            .filter(|(_, rows): &(NodeId, Vec<NodeId>)| !rows.is_empty())
            .collect();
        type Make = fn(NodeId, &[NodeId]) -> Command;
        let edits: [(&'static str, Make); 4] = [
            ("row", |_, rows| Command::InsertRow {
                row: rows[0],
                below: true,
                like: None,
            }),
            ("column", |table, _| Command::InsertColumn {
                table,
                index: 1,
                like: 0,
            }),
            ("column removed", |table, _| Command::RemoveColumn {
                table,
                index: 0,
            }),
            ("row removed", |_, rows| Command::RemoveNode {
                node: rows[rows.len() - 1],
            }),
        ];
        for (what, make) in edits {
            let mut doc = Document::open(self.bytes).unwrap();
            let commands: Vec<Command> = tables.iter().map(|(t, rows)| make(*t, rows)).collect();
            self.check(what, &mut doc, commands, None);
        }
    }

    fn picture(&mut self) {
        let mut doc = Document::open(self.bytes).unwrap();
        let media = Editor::open(self.bytes)
            .unwrap()
            .media_name(&png())
            .unwrap();
        let commands: Vec<Command> = doc
            .paragraphs()
            .into_iter()
            .filter_map(|p| {
                Some(Command::InsertPicture {
                    to: None,
                    at: middle(&doc, p)?,
                    media: media.clone(),
                    width_emu: 914400,
                    height_emu: 914400,
                    description: Some("Added by the corpus run".into()),
                    id: None,
                })
            })
            .collect();
        self.check("picture", &mut doc, commands, Some(0));
    }

    /// Applies the first command that is not refused, then checks it.
    /// Asks Word to update fields on open. Only the settings part may
    /// change, or for a file without one, the new part, the content types
    /// and the main part's relationships. The export must re-open with the
    /// flag and otherwise the same inspection, and undo must give back every
    /// part's bytes.
    fn update_fields(&mut self) {
        let name = self.name;
        let mut editor = Editor::open(self.bytes).unwrap();
        let doc = editor.view().unwrap();
        let on = !doc.settings().update_fields;
        match editor.apply(&Command::SetUpdateFields { on }) {
            Ok(()) => (),
            Err(e) => {
                self.problems.push(format!("{name}: update fields: {e}"));
                return;
            }
        };
        let doc = editor.view().unwrap();
        let out = editor.export().unwrap();
        if self.input_valid && !validate_package(&out).valid {
            self.problems
                .push(format!("{name}: update fields: export invalid"));
            return;
        }
        let before: BTreeMap<String, Vec<u8>> = raw_entries(self.bytes).into_iter().collect();
        let after: BTreeMap<String, Vec<u8>> = raw_entries(&out).into_iter().collect();
        let changed: Vec<&String> = after
            .iter()
            .filter(|(name, data)| before.get(*name) != Some(*data))
            .map(|(name, _)| name)
            .collect();
        let added = changed.iter().filter(|n| !before.contains_key(**n)).count();
        let expected = match added {
            0 => changed.len() == 1,
            _ => {
                added == 1
                    && changed.len() == 3
                    && changed.iter().any(|n| *n == "[Content_Types].xml")
            }
        };
        if !expected || after.len() != before.len() + added {
            self.problems.push(format!(
                "{name}: update fields: changed parts {changed:?}, {added} added"
            ));
            return;
        }
        let without_package = |mut inspection: serde_json::Value| {
            inspection.as_object_mut().unwrap().remove("package");
            inspection
        };
        match Document::open(&out) {
            Ok(re)
                if re.settings().update_fields == on
                    && without_package(re.inspect_json())
                        == without_package(doc.inspect_json()) =>
            {
                *self.tally.entry("update_fields_verified").or_default() += 1;
                if added == 1 {
                    *self.tally.entry("update_fields_part_added").or_default() += 1;
                }
            }
            Ok(re) => {
                self.problems.push(format!(
                    "{name}: update fields: re-opened with the flag {}, inspection equal: {}",
                    re.settings().update_fields,
                    without_package(re.inspect_json()) == without_package(doc.inspect_json())
                ));
                return;
            }
            Err(e) => {
                self.problems
                    .push(format!("{name}: update fields: re-open failed: {e}"));
                return;
            }
        }
        assert!(editor.undo());
        let undone: BTreeMap<String, Vec<u8>> =
            raw_entries(&editor.export().unwrap()).into_iter().collect();
        if undone == before {
            *self.tally.entry("update_fields_undone").or_default() += 1;
        } else {
            self.problems.push(format!(
                "{name}: update fields: undone export has other bytes"
            ));
        }
    }

    fn fill_controls(&mut self) {
        let name = self.name;
        let mut editor = Editor::open(self.bytes).unwrap();
        let doc = editor.view().unwrap();
        let controls = content_controls(doc);
        if controls.is_empty() {
            return;
        }
        let mut fills = 0;
        for &id in &controls {
            *self.tally.entry("controls_seen").or_default() += 1;
            let NodeKind::ContentControl { control, .. } =
                &editor.view().unwrap().node(id).unwrap().kind
            else {
                unreachable!("a content control");
            };
            let (kind, fill) = match control {
                Control::RichText => ("rich text", Fill::Text("Filled by the corpus run".into())),
                Control::Text { multi_line } => {
                    let text = match multi_line {
                        true => "Filled by\nthe corpus run",
                        false => "Filled by the corpus run",
                    };
                    ("text", Fill::Text(text.into()))
                }
                Control::Checkbox { checked, .. } => ("check box", Fill::Checked(!checked)),
                Control::DropDown { items, last_value }
                | Control::ComboBox { items, last_value } => {
                    let other = items.iter().find(|i| Some(&i.value) != last_value.as_ref());
                    match other {
                        Some(item) => ("list", Fill::Item(item.value.clone())),
                        None => {
                            *self
                                .fill_refusals
                                .entry("list: no other item".into())
                                .or_default() += 1;
                            continue;
                        }
                    }
                }
                Control::Date { .. } => ("date", Fill::Date("2026-11-16".into())),
                Control::Other { element } => {
                    *self
                        .fill_refusals
                        .entry(format!("other ({element})"))
                        .or_default() += 1;
                    continue;
                }
            };
            match editor.apply(&Command::FillContentControl { control: id, fill }) {
                Ok(()) => {
                    *self.tally.entry("controls_filled").or_default() += 1;
                    fills += 1;
                }
                Err(e) => {
                    let reason = match e {
                        SessionError::Edit(
                            EditError::Refused { reason, .. } | EditError::ReadOnly { reason, .. },
                        ) => reason.to_string(),
                        e => e.to_string(),
                    };
                    *self
                        .fill_refusals
                        .entry(format!("{kind}: {reason}"))
                        .or_default() += 1;
                }
            }
        }
        if fills == 0 {
            return;
        }
        let doc = editor.view().unwrap();
        let out = editor.export().unwrap();
        if self.input_valid && !validate_package(&out).valid {
            self.problems.push(format!(
                "{name}: fill: export invalid: {:?}",
                validate_package(&out).issues.first()
            ));
            return;
        }
        let states = |doc: &Document| -> Vec<(String, String)> {
            content_controls(doc)
                .into_iter()
                .map(|id| control_state(doc, id))
                .collect()
        };
        match Document::open(&out) {
            Ok(re) if states(&re) == states(doc) => {
                *self.tally.entry("fill_files_verified").or_default() += 1;
            }
            Ok(re) => {
                let (a, b) = (states(doc), states(&re));
                let first = a.iter().zip(&b).find(|(x, y)| x != y);
                self.problems.push(format!(
                    "{name}: fill: re-opened controls differ ({} vs {}): {first:?}",
                    a.len(),
                    b.len()
                ));
                return;
            }
            Err(e) => {
                self.problems
                    .push(format!("{name}: fill: re-open failed: {e}"));
                return;
            }
        }
        for _ in 0..fills {
            assert!(editor.undo());
        }
        let doc = editor.view().unwrap();
        let original = Document::open(self.bytes).unwrap();
        let undone = editor.export().unwrap();
        if doc.inspect_json()["stories"] != original.inspect_json()["stories"] {
            self.problems
                .push(format!("{name}: fill: undo left another inspection"));
        } else if entries(&undone) != entries(self.bytes) {
            self.problems
                .push(format!("{name}: fill: undone export has other parts"));
        } else {
            *self.tally.entry("fill_files_undone").or_default() += 1;
        }
    }

    fn field_results(&mut self) {
        const NOW: &str = "2026-11-16T14:05:09";
        let name = self.name;
        let mut editor = Editor::open(self.bytes).unwrap();
        let doc = editor.view().unwrap();
        let body = doc.stories().next().unwrap().id;
        let paragraphs: Vec<NodeId> = doc
            .descendants(body)
            .into_iter()
            .filter(|&d| matches!(doc.node(d).unwrap().kind, NodeKind::Paragraph(_)))
            .collect();
        let pages = Pages {
            total: paragraphs.len() as u32 / 10 + 3,
            numbers: paragraphs
                .iter()
                .enumerate()
                .map(|(i, &p)| (p, (i / 10 + 2).to_string()))
                .collect(),
        };
        let updates = doc.field_updates(NOW, Some(&pages)).unwrap();
        if updates.is_empty() {
            return;
        }
        *self.tally.entry("field_files").or_default() += 1;
        *self.tally.entry("fields_updated").or_default() += updates.len();
        match editor.apply(&Command::UpdateFields {
            now: NOW.into(),
            pages: Some(pages.clone()),
        }) {
            Ok(()) => (),
            Err(e) => {
                self.problems.push(format!("{name}: fields: {e}"));
                return;
            }
        };
        let doc = editor.view().unwrap();
        let out = editor.export().unwrap();
        if self.input_valid && !validate_package(&out).valid {
            self.problems.push(format!(
                "{name}: fields: export invalid: {:?}",
                validate_package(&out).issues.first()
            ));
            return;
        }
        match Document::open(&out) {
            Ok(re)
                if all_text(&re) == all_text(doc)
                    && re.field_updates(NOW, Some(&pages)).unwrap().is_empty() =>
            {
                *self.tally.entry("field_files_verified").or_default() += 1;
            }
            Ok(re) => {
                self.problems.push(format!(
                    "{name}: fields: re-opened with {} left to update, text equal: {}",
                    re.field_updates(NOW, Some(&pages)).unwrap().len(),
                    all_text(&re) == all_text(doc)
                ));
                return;
            }
            Err(e) => {
                self.problems
                    .push(format!("{name}: fields: re-open failed: {e}"));
                return;
            }
        }
        assert!(editor.undo());
        let doc = editor.view().unwrap();
        let original = Document::open(self.bytes).unwrap();
        if doc.inspect_json()["stories"] != original.inspect_json()["stories"] {
            self.problems
                .push(format!("{name}: fields: undo left another inspection"));
        } else if entries(&editor.export().unwrap()) != entries(self.bytes) {
            self.problems
                .push(format!("{name}: fields: undone export has other parts"));
        } else {
            *self.tally.entry("field_files_undone").or_default() += 1;
        }
    }

    fn check(
        &mut self,
        what: &'static str,
        doc: &mut Document,
        commands: impl IntoIterator<Item = Command>,
        delta: Option<isize>,
    ) {
        let name = self.name;
        let fresh = doc.inspect_json();
        let fresh_formatting = formatting(doc);
        let fresh_styles = paragraph_styles(doc);
        let fresh_lists = lists(doc);
        let fresh_links = links(doc);
        let fresh_tables = table_shapes(doc);
        let fresh_pictures = pictures(doc);
        let (count, text) = (doc.paragraphs().len() as isize, all_text(doc));
        let mut first_refusal = None;
        let mut editor = Editor::open(self.bytes).unwrap();
        let image = png();
        let mut applied = false;
        for command in commands {
            let media: &[&[u8]] = if matches!(command, Command::InsertPicture { .. }) {
                &[&image]
            } else {
                &[]
            };
            match editor.apply_all_with_media(&[command], media) {
                Ok(())
                    if match what {
                        "bold" => formatting(editor.view().unwrap()) == fresh_formatting,
                        "bullets" | "level" => lists(editor.view().unwrap()) == fresh_lists,
                        _ => editor.view().unwrap().inspect_json() == fresh,
                    } =>
                {
                    // A command may set an explicit value equal to the inherited one.
                    // Try the next candidate against the original file.
                    editor = Editor::open(self.bytes).unwrap();
                    first_refusal.get_or_insert(match what {
                        "bold" => "already bold".into(),
                        "bullets" => "already bulleted".into(),
                        "level" => "no list that can go deeper".into(),
                        "link" => "already that link".into(),
                        "row" | "column" | "column removed" | "row removed" | "picture" => {
                            "changed nothing".into()
                        }
                        _ => "already in that style".into(),
                    });
                }
                Ok(()) => {
                    applied = true;
                    break;
                }
                Err(
                    SessionError::Edit(EditError::Refused { reason, .. })
                    | SessionError::Unsupported { reason, .. },
                ) => {
                    first_refusal.get_or_insert(reason.to_string());
                }
                Err(SessionError::Edit(EditError::ReadOnly { .. })) => {
                    first_refusal.get_or_insert("read-only".to_string());
                }
                Err(SessionError::Edit(EditError::Comment { reason, .. })) => {
                    first_refusal.get_or_insert(format!("the comment {reason}"));
                }
                Err(e) => {
                    self.problems.push(format!("{name}: {what}: {e}"));
                    return;
                }
            }
        }
        if !applied {
            *self
                .tally
                .entry(match what {
                    "split" => "no_split",
                    "merge" => "no_merge",
                    "range" => "no_range",
                    "tracked" => "no_tracked",
                    "comment" => "no_comment",
                    "reply" => "no_reply",
                    "bold" => "no_bold",
                    "bullets" => "no_bullets",
                    "level" => "no_level",
                    "link" => "no_link",
                    "row" => "no_row",
                    "column" => "no_column",
                    "column removed" => "no_column_removed",
                    "row removed" => "no_row_removed",
                    "picture" => "no_picture",
                    _ => "no_style",
                })
                .or_default() += 1;
            let reason = first_refusal.unwrap_or_else(|| "nothing to try".into());
            *self
                .refusals
                .entry(format!("{what}: {reason}"))
                .or_default() += 1;
            return;
        };
        let doc = editor.view().unwrap();
        let out = editor.export().unwrap();
        if self.input_valid && !validate_package(&out).valid {
            self.problems.push(format!("{name}: {what} export invalid"));
            return;
        }
        // A range changes the text, so the re-opened file must read as the
        // edited document does.
        let (count, text) = match delta {
            Some(delta) => (count + delta, text),
            None => (doc.paragraphs().len() as isize, all_text(doc)),
        };
        if what == "bold" && formatting(doc) == fresh_formatting {
            self.problems
                .push(format!("{name}: {what}: no item changed"));
            return;
        }
        if what == "style" && paragraph_styles(doc) == fresh_styles {
            self.problems
                .push(format!("{name}: {what}: no paragraph changed"));
            return;
        }
        if matches!(what, "bullets" | "level") && lists(doc) == fresh_lists {
            self.problems
                .push(format!("{name}: {what}: no list changed"));
            return;
        }
        if what == "link" && links(doc) == fresh_links {
            self.problems
                .push(format!("{name}: {what}: no link changed"));
            return;
        }
        if (what.starts_with("row") || what.starts_with("column"))
            && table_shapes(doc) == fresh_tables
        {
            self.problems
                .push(format!("{name}: {what}: no table changed"));
            return;
        }
        if what == "tracked" {
            let made: BTreeSet<String> = revisions(doc)
                .0
                .into_iter()
                .filter(|r| r.1.as_deref() == Some(CORPUS))
                .map(|r| r.0)
                .collect();
            if !["ins", "del", "mark ins", "mark del"]
                .iter()
                .all(|c| made.contains(*c))
            {
                self.problems
                    .push(format!("{name}: {what}: the changes made: {made:?}"));
                return;
            }
        }
        if matches!(what, "comment" | "reply") {
            let made: Vec<Remark> = comments(doc)
                .into_iter()
                .filter(|c| c.1.as_deref() == Some(CORPUS))
                .collect();
            let markers = comment_markers(doc);
            let marked = |id: &str| {
                ["commentRangeStart", "commentRangeEnd", "commentReference"]
                    .iter()
                    .all(|local| {
                        markers
                            .iter()
                            .filter(|(e, m)| e.ends_with(local) && m.as_deref() == Some(id))
                            .count()
                            == 1
                    })
            };
            let threaded = made.iter().all(|c| c.2.is_some() == (what == "reply"));
            if made.len() != 1 || !marked(&made[0].0) || !threaded {
                self.problems.push(format!(
                    "{name}: {what}: the comments made: {made:?}, each with its markers: {}",
                    made.iter().all(|c| marked(&c.0))
                ));
                return;
            }
        }
        if what == "picture" {
            let now = pictures(doc);
            let ids = |pictures: &[Picture]| -> BTreeSet<Option<u32>> {
                pictures.iter().map(|p| p.0).collect()
            };
            let added = now.len() == fresh_pictures.len() + 1;
            // A new ID adds one more distinct ID, unless the file repeated some before.
            let unique = ids(&now).len() == ids(&fresh_pictures).len() + 1;
            if !added || !unique {
                self.problems.push(format!(
                    "{name}: {what}: one more picture: {added}; with an ID no drawing had: {unique}"
                ));
                return;
            }
        }
        match Document::open(&out) {
            Ok(re)
                if re.paragraphs().len() as isize == count
                    && all_text(&re) == text
                    && formatting(&re) == formatting(doc)
                    && paragraph_styles(&re) == paragraph_styles(doc)
                    && lists(&re) == lists(doc)
                    && links(&re) == links(doc)
                    && table_shapes(&re) == table_shapes(doc)
                    && pictures(&re) == pictures(doc)
                    && revisions(&re).0 == revisions(doc).0
                    && comments(&re) == comments(doc)
                    && comment_markers(&re) == comment_markers(doc)
                    && distinct_ids(&re) =>
            {
                *self
                    .tally
                    .entry(match what {
                        "split" => "split_verified",
                        "merge" => "merge_verified",
                        "range" => "range_verified",
                        "tracked" => "tracked_verified",
                        "comment" => "comment_verified",
                        "reply" => "reply_verified",
                        "bold" => "bold_verified",
                        "bullets" => "bullets_verified",
                        "level" => "level_verified",
                        "link" => "link_verified",
                        "row" => "row_verified",
                        "column" => "column_verified",
                        "column removed" => "column_removed_verified",
                        "row removed" => "row_removed_verified",
                        "picture" => "picture_verified",
                        _ => "style_verified",
                    })
                    .or_default() += 1;
            }
            Ok(re) => self.problems.push(format!(
                "{name}: {what}: {} paragraphs after re-open, expected {}; text equal: {}; formatting equal: {}; styles equal: {}; lists equal: {}; links equal: {}; tables equal: {}; pictures equal: {}; revisions equal: {}; comments equal: {}; comment markers equal: {}; made IDs distinct: {}",
                re.paragraphs().len(),
                count,
                all_text(&re) == text,
                formatting(&re) == formatting(doc),
                paragraph_styles(&re) == paragraph_styles(doc),
                lists(&re) == lists(doc),
                links(&re) == links(doc),
                table_shapes(&re) == table_shapes(doc),
                pictures(&re) == pictures(doc),
                revisions(&re).0 == revisions(doc).0,
                comments(&re) == comments(doc),
                comment_markers(&re) == comment_markers(doc),
                distinct_ids(&re)
            )),
            Err(e) => self
                .problems
                .push(format!("{name}: {what}: re-open failed: {e}")),
        }
        if !editor.undo() {
            self.problems.push(format!("{name}: {what}: missing undo"));
            return;
        }
        let doc = editor.view().unwrap();
        let undone = editor.export().unwrap();
        if doc.inspect_json() != fresh {
            self.problems
                .push(format!("{name}: {what}: undo differs from the original"));
        } else if self.input_valid && !validate_package(&undone).valid {
            self.problems
                .push(format!("{name}: {what}: undone export invalid"));
        } else if entry_names(&undone) != entry_names(self.bytes) {
            self.problems
                .push(format!("{name}: {what}: undone export has other parts"));
        } else {
            *self
                .tally
                .entry(match what {
                    "split" => "split_undone",
                    "merge" => "merge_undone",
                    "range" => "range_undone",
                    "tracked" => "tracked_undone",
                    "comment" => "comment_undone",
                    "reply" => "reply_undone",
                    "bold" => "bold_undone",
                    "bullets" => "bullets_undone",
                    "level" => "level_undone",
                    "link" => "link_undone",
                    "row" => "row_undone",
                    "column" => "column_undone",
                    "column removed" => "column_removed_undone",
                    "row removed" => "row_removed_undone",
                    "picture" => "picture_undone",
                    _ => "style_undone",
                })
                .or_default() += 1;
        }
    }
}
