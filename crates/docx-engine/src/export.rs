//! IR → DOCX by splicing.
//!
//! A part with no dirty node is copied as its raw ZIP entry. In a dirty part,
//! every clean subtree is copied from the source text. A re-walked container
//! writes its own start tag and property elements and its end tag from the
//! source, and each child brings the whitespace that preceded it, so children
//! may be added, removed or moved between containers. Only dirty text and
//! nodes a command created are generated.

use std::borrow::Cow;
use std::collections::HashMap;
use std::io::{Cursor, Write};
use std::sync::Arc;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::error::ExportError;
use crate::format::tag_end;
use crate::ir::{Node, NodeId, NodeKind};
use crate::pictures::AddedMedia;
use crate::revisions::REVISION_ID;
use crate::xml::{self, escape_attr, escape_text, qname};
use crate::{Document, Story};

pub(crate) const DECLARATION: &str =
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n";
const PACKAGE_RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

const CONTENT_TYPES: &str = "[Content_Types].xml";

/// The parts export rewrites besides the stories, keyed by their names in
/// lower case, and the parts it adds: XML, then media.
#[derive(Default)]
pub(crate) struct Additions {
    pub replaced: HashMap<String, String>,
    pub added: Vec<(String, String)>,
    pub media: Vec<(String, Arc<[u8]>)>,
}

/// A relationship export adds to `part`'s relationships.
pub(crate) struct NewRelationship {
    pub part: String,
    pub id: String,
    pub kind: &'static str,
    pub target: String,
    pub external: bool,
}

/// A package change has a name before its XML is rendered. Inspection and
/// export enumerate the same parts, including generated metadata.
enum PartChange<'a> {
    Story(&'a Story),
    Types(Vec<(String, &'static str)>),
    Media(&'a AddedMedia),
    Relationships(Vec<NewRelationship>),
    Numbering,
    Styles,
    Threads(bool),
    Settings,
}

impl Document {
    pub fn is_dirty(&self) -> bool {
        !self.dirty_parts().is_empty()
    }

    pub(crate) fn subtree_dirty(&self, id: NodeId) -> bool {
        let node = &self.nodes[id.index()];
        node.dirty || node.dirty_below
    }

    /// The modified parts' names: the stories in package order, then the
    /// parts that hold what commands added: the content types, media,
    /// relationships, lists, the Hyperlink style, comment threads and
    /// settings.
    pub fn dirty_parts(&self) -> Vec<String> {
        self.changed_parts()
            .into_iter()
            .map(|(name, _)| name)
            .collect()
    }

    fn changed_parts(&self) -> Vec<(String, PartChange<'_>)> {
        use PartChange::*;
        let mut parts = Vec::new();
        let mut types = Vec::new();
        let mut relationships = Vec::new();
        let mut auxiliary = Vec::new();
        let mut related = |part: &str, content_type, kind, id_prefix: &str, first: usize| {
            types.push((part.to_owned(), content_type));
            let main = self.package.main_part();
            let id = (first..)
                .map(|i| match (id_prefix, i) {
                    ("rId", _) => format!("rId{i}"),
                    (_, 0) => id_prefix.to_owned(),
                    ("rIdSettings", _) => format!("{id_prefix}{}", i + 1),
                    _ => format!("{id_prefix}{i}"),
                })
                .find(|id| self.package.relationship(main, id).is_none())
                .expect("a free relationship id");
            let dir = main.rfind('/').map_or("", |i| &main[..=i]);
            relationships.push(NewRelationship {
                part: main.to_owned(),
                id,
                kind,
                target: part.strip_prefix(dir).unwrap_or(part).to_owned(),
                external: false,
            });
        };
        for story in self
            .stories
            .iter()
            .filter(|s| !s.added && self.subtree_dirty(s.root))
        {
            parts.push((story.part.clone(), Story(story)));
        }
        if let Some(story) = self.added_comments() {
            related(
                &story.part,
                crate::comments::CT_COMMENTS,
                crate::package::REL_COMMENTS,
                "rIdComments",
                0,
            );
            parts.push((story.part.clone(), Story(story)));
        }
        if let Some(part) = self.list_part() {
            if self.numbering.part.is_none() {
                related(
                    &part,
                    crate::lists::CT_NUMBERING,
                    crate::package::REL_NUMBERING,
                    "rId",
                    1,
                );
            }
            auxiliary.push((part, Numbering));
        }
        if let Some(part) = self.link_style_part() {
            auxiliary.push((part.to_owned(), Styles));
        }
        if let Some((part, added)) = self.threads_part() {
            if added {
                related(
                    &part,
                    crate::comments::CT_COMMENTS_EXTENDED,
                    crate::comments::REL_COMMENTS_EXTENDED,
                    "rIdCommentsExtended",
                    0,
                );
            }
            auxiliary.push((part, Threads(added)));
        }
        if let Some(part) = self.settings_part() {
            if self.settings_part.is_none() {
                related(
                    &part,
                    crate::settings::CT_SETTINGS,
                    crate::package::REL_SETTINGS,
                    "rIdSettings",
                    0,
                );
            }
            auxiliary.push((part, Settings));
        }
        let shown = self.shown();
        let media = self.media_in_use(&shown);
        types.extend(
            media
                .iter()
                .filter(|m| self.package.content_type(&m.name) != Some(m.content_type))
                .map(|m| (m.name.clone(), m.content_type)),
        );
        if !types.is_empty() {
            parts.push((
                self.package
                    .actual_name(CONTENT_TYPES)
                    .unwrap_or(CONTENT_TYPES)
                    .to_owned(),
                Types(types),
            ));
        }
        parts.extend(media.into_iter().map(|m| (m.name.clone(), Media(m))));
        relationships.extend(self.link_relationships());
        relationships.extend(self.picture_relationships(&shown));
        let mut by_part: Vec<(String, Vec<NewRelationship>)> = Vec::new();
        for relationship in relationships {
            let name = rels_part(&relationship.part);
            if let Some((_, group)) = by_part.iter_mut().find(|(part, _)| part == &name) {
                group.push(relationship);
            } else {
                by_part.push((name, vec![relationship]));
            }
        }
        parts.extend(by_part.into_iter().map(|(name, group)| {
            (
                self.package
                    .actual_name(&name)
                    .map(str::to_owned)
                    .unwrap_or(name),
                Relationships(group),
            )
        }));
        parts.extend(auxiliary);
        parts
    }

    fn write_types(
        &self,
        part: &str,
        types: &[(String, &'static str)],
    ) -> Result<String, ExportError> {
        let text = self.edit_root(part, |root, _, close| {
            let p = prefix(root, xml::CONTENT_TYPES);
            let overrides = types
                .iter()
                .map(|(name, ct)| {
                    format!(
                        "<{p}Override PartName=\"/{}\" ContentType=\"{ct}\"/>",
                        escape_attr(name)
                    )
                })
                .collect();
            vec![(close, overrides)]
        })?;
        Ok(text)
    }

    fn write_relationships(
        &self,
        part: &str,
        relationships: &[NewRelationship],
        out: &mut Additions,
    ) -> Result<(), ExportError> {
        let markup = |p: &str| -> String {
            relationships
                .iter()
                .map(|r| {
                    let mode = if r.external {
                        " TargetMode=\"External\""
                    } else {
                        ""
                    };
                    format!(
                        "<{p}Relationship Id=\"{}\" Type=\"{}\" Target=\"{}\"{mode}/>",
                        r.id,
                        r.kind,
                        escape_attr(&r.target)
                    )
                })
                .collect()
        };
        if let Some(actual) = self.package.actual_name(part) {
            let text = self.edit_root(actual, |root, _, close| {
                vec![(close, markup(&prefix(root, PACKAGE_RELS)))]
            })?;
            out.replaced.insert(actual.to_ascii_lowercase(), text);
        } else {
            out.added.push((
                part.to_owned(),
                format!(
                    "{DECLARATION}<Relationships xmlns=\"{PACKAGE_RELS}\">{}</Relationships>",
                    markup("")
                ),
            ));
        }
        Ok(())
    }
    pub fn export(&self) -> Result<Vec<u8>, ExportError> {
        let mut dirty = HashMap::new();
        let mut additions = Additions::default();
        for (part, change) in self.changed_parts() {
            match change {
                PartChange::Story(story) if story.added => {
                    additions.added.push((part, self.serialize_story(story)))
                }
                PartChange::Story(story) => {
                    dirty.insert(story.part.as_str(), story);
                }
                PartChange::Types(types) => {
                    additions
                        .replaced
                        .insert(part.to_ascii_lowercase(), self.write_types(&part, &types)?);
                }
                PartChange::Relationships(group) => {
                    self.write_relationships(&part, &group, &mut additions)?
                }
                PartChange::Media(media) => additions.media.push((part, media.bytes.clone())),
                PartChange::Numbering => self.list_parts(&mut additions)?,
                PartChange::Styles => {
                    let (_, text) = self.link_style_text()?.expect("planned hyperlink style");
                    additions.replaced.insert(part.to_ascii_lowercase(), text);
                }
                PartChange::Threads(added) => self.write_threads(part, added, &mut additions)?,
                PartChange::Settings => self.settings_parts(part, &mut additions)?,
            }
        }
        // Word gives comments and revisions IDs from one sequence.
        let mut revision = self.revision_ids.max(self.made_comment_ids());

        let mut archive = self
            .package
            .archive()
            .map_err(|e| ExportError::Zip(e.to_string()))?;
        let mut writer = ZipWriter::new(Cursor::new(Vec::with_capacity(self.package.len_bytes())));
        for i in 0..archive.len() {
            let entry = archive.by_index_raw(i)?;
            let name = entry.name()?.to_string();
            match dirty.get(name.as_str()) {
                Some(story) => {
                    let mut options = SimpleFileOptions::default()
                        .compression_method(CompressionMethod::Deflated);
                    if let Some(modified) = entry.last_modified() {
                        options = options.last_modified_time(modified);
                    }
                    drop(entry);
                    writer.start_file(name, options)?;
                    let text = number_revisions(self.serialize_story(story), &mut revision);
                    writer.write_all(text.as_bytes())?;
                }
                None => match additions.replaced.remove(&name.to_ascii_lowercase()) {
                    Some(text) => {
                        let options = SimpleFileOptions::default()
                            .compression_method(CompressionMethod::Deflated);
                        drop(entry);
                        writer.start_file(name, options)?;
                        writer.write_all(text.as_bytes())?;
                    }
                    None => writer.raw_copy_file(entry)?,
                },
            }
        }
        for (name, text) in additions.added {
            let options =
                SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
            writer.start_file(name, options)?;
            writer.write_all(text.as_bytes())?;
        }
        // Images are compressed already, so deflating them again costs time
        // and saves nothing.
        for (name, bytes) in additions.media {
            let options =
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            writer.start_file(name, options)?;
            writer.write_all(&bytes)?;
        }
        if !archive.comment().is_empty() {
            writer.set_raw_comment(archive.comment().into())?;
        }
        Ok(writer.finish()?.into_inner())
    }

    pub(crate) fn serialize_story(&self, story: &Story) -> String {
        let text = &story.xml.text;
        let root = &self.nodes[story.root.index()];
        let mut out = String::with_capacity(text.len() + 64);
        if story.xml.bom {
            out.push('\u{FEFF}');
        }
        out.push_str(&text[..root.range.start]);
        self.write_node(text, story.root, &mut out);
        out.push_str(&text[root.range.end..]);
        out
    }

    fn write_node(&self, text: &str, id: NodeId, out: &mut String) {
        let node = &self.nodes[id.index()];
        // A self-closing element whose children were added and removed again.
        let empty_again = !node.dirty && node.children.is_empty() && self.self_closing(text, node);
        if node.made.is_none() && (!node.dirty && !node.dirty_below || empty_again) {
            out.push_str(&text[node.range.clone()]);
            return;
        }
        let text_like = match &node.kind {
            NodeKind::Text { text } => Some((text, "t")),
            NodeKind::DeletedText { text } => Some((text, "delText")),
            _ => None,
        };
        if let Some((value, local)) = text_like {
            // Text edited back to what the source holds writes the source bytes.
            if node.made.is_none() {
                let source = &text[node.range.clone()];
                let content = match (source.find('>'), source.rfind("</")) {
                    (Some(open), Some(close)) if open < close => &source[open + 1..close],
                    _ => "",
                };
                let same = qname(text, &node.range).rsplit(':').next() == Some(local);
                if same && content == escape_text(value) {
                    out.push_str(source);
                    return;
                }
            }
            // A text node a command marked deleted, or back, keeps its prefix
            // and takes the element its kind is written with.
            let name = match &node.made {
                Some(made) => made.head[1..]
                    .split(|c: char| c.is_whitespace() || c == '>')
                    .next()
                    .unwrap_or_default(),
                None => qname(text, &node.range),
            };
            let name = match name.split_once(':') {
                Some((prefix, _)) => format!("{prefix}:{local}"),
                None => local.to_string(),
            };
            if node.made.is_none() {
                let original = &text[node.range.clone()];
                let end = tag_end(original, 0).expect("parsed XML text element");
                let old_name = crate::format::qname(original);
                let attributes = original[1 + old_name.len()..end - 1].trim_end_matches('/');
                let opening = format!("<{name}{attributes}>");
                out.push_str(&crate::format::preserve_space(&opening));
            } else {
                out.push_str(&format!("<{name} xml:space=\"preserve\">"));
            }
            out.push_str(&escape_text(value));
            out.push_str(&format!("</{name}>"));
            return;
        }
        let (head, tail) = self.tags(text, node);
        out.push_str(&head);
        for &child in &node.children {
            let c = &self.nodes[child.index()];
            if c.made.is_none() {
                out.push_str(&text[c.lead..c.range.start]);
            }
            self.write_node(text, child, out);
        }
        out.push_str(&tail);
    }

    /// The markup before and after a container's children.
    pub(crate) fn tags<'t>(
        &'t self,
        text: &'t str,
        node: &'t Node,
    ) -> (Cow<'t, str>, Cow<'t, str>) {
        if let Some(made) = &node.made {
            return (Cow::Borrowed(&made.head), Cow::Borrowed(&made.tail));
        }
        let (head, tail) = self.source_tags(text, node);
        match &node.head {
            Some(changed) => (Cow::Borrowed(&**changed), tail),
            None => (head, tail),
        }
    }

    /// The markup before and after a source node's children as its source
    /// has it, a self-closing element written open.
    pub(crate) fn source_tags<'t>(
        &'t self,
        text: &'t str,
        node: &'t Node,
    ) -> (Cow<'t, str>, Cow<'t, str>) {
        let range = &node.range;
        if self.self_closing(text, node) {
            let open = text[range.start..range.end - 2].trim_end();
            let name = qname(text, range);
            (
                Cow::Owned(format!("{open}>")),
                Cow::Owned(format!("</{name}>")),
            )
        } else {
            (
                Cow::Borrowed(&text[range.start..node.inner.start]),
                Cow::Borrowed(&text[node.inner.end..range.end]),
            )
        }
    }

    fn self_closing(&self, text: &str, node: &Node) -> bool {
        node.made.is_none()
            && node.inner.start == node.range.end
            && text[node.range.clone()].ends_with("/>")
    }
}

impl Document {
    /// An XML part with markup inserted in its root element. `insert` gets
    /// the root, where its start tag ends and where its end tag starts, and
    /// returns what to insert where.
    pub(crate) fn edit_root(
        &self,
        part: &str,
        insert: impl FnOnce(roxmltree::Node, usize, usize) -> Vec<(usize, String)>,
    ) -> Result<String, ExportError> {
        self.edit_root_ranges(part, |root, open, close| {
            insert(root, open, close)
                .into_iter()
                .map(|(at, markup)| (at..at, markup))
                .collect()
        })
    }

    /// Like [`edit_root`](Document::edit_root), with each edit replacing a
    /// byte range of the part's text.
    pub(crate) fn edit_root_ranges(
        &self,
        part: &str,
        edit: impl FnOnce(roxmltree::Node, usize, usize) -> Vec<(std::ops::Range<usize>, String)>,
    ) -> Result<String, ExportError> {
        let source = self.package.read_text(part)?;
        let text = opened(part, &source.text)?;
        let doc = xml::parse(part, &text)?;
        let root = doc.root_element();
        let range = root.range();
        let open = range.start + tag_end(&text[range.clone()], 0).expect("a start tag");
        let close = range.start + text[range.clone()].rfind("</").expect("an end tag");
        let mut edits = edit(root, open, close);
        edits.sort_by_key(|e| e.0.start);
        let mut out =
            String::with_capacity(text.len() + edits.iter().map(|e| e.1.len()).sum::<usize>() + 3);
        if source.bom {
            out.push('\u{FEFF}');
        }
        let mut last = 0;
        for (range, markup) in edits {
            out.push_str(&text[last..range.start]);
            out.push_str(&markup);
            last = range.end;
        }
        out.push_str(&text[last..]);
        Ok(out)
    }
}

/// A story's markup with each revision ID a command left for export
/// numbered, from one above `last`, which ends as the last one given.
fn number_revisions(text: String, last: &mut u32) -> String {
    if !text.contains(REVISION_ID) {
        return text;
    }
    let mut out = String::with_capacity(text.len() + 64);
    for (i, piece) in text.split(REVISION_ID).enumerate() {
        if i > 0 {
            *last += 1;
            out.push_str(&last.to_string());
        }
        out.push_str(piece);
    }
    out
}

/// The relationships part of `part`.
pub(crate) fn rels_part(part: &str) -> String {
    match part.rfind('/') {
        Some(i) => format!("{}_rels/{}.rels", &part[..=i], &part[i + 1..]),
        None => format!("_rels/{part}.rels"),
    }
}

/// The prefix, with its colon, that `root` binds to `namespace`, or none
/// when the namespace is the default one.
pub(crate) fn prefix(root: roxmltree::Node, namespace: &str) -> String {
    match root.lookup_prefix(namespace) {
        Some(prefix) => format!("{prefix}:"),
        None => String::new(),
    }
}

/// The part's text with a self-closing root element written open, so
/// content can go inside it.
fn opened(part: &str, text: &str) -> Result<String, ExportError> {
    let doc = xml::parse(part, text)?;
    let range = doc.root_element().range();
    let open = range.start + tag_end(&text[range.clone()], 0).expect("a start tag");
    if open == range.end && text[..range.end].ends_with("/>") {
        let name = xml::qname(text, &range);
        return Ok(format!(
            "{}></{name}>{}",
            &text[..range.end - 2],
            &text[range.end..]
        ));
    }
    Ok(text.to_string())
}
