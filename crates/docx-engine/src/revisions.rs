//! DOCX revision markup, authorship and export-time revision IDs.
//! The editing session decides what an insertion or deletion covers.

use std::ops::Range;

use crate::edit::{check_text, Change, Track};
use crate::error::EditError;
use crate::format::{child, rewrite, tag_end, PPR_ORDER};
use crate::ir::{Made, NodeId, NodeKind, RevisionInfo, Support, Tracked};
use crate::tables::REVISIONS;
use crate::xml::{escape_attr, qname, start_tag_name, W};
use crate::Document;

/// Stands for a revision's `w:id` in the markup commands make, until export
/// numbers it. XML cannot hold it, so no source text has it.
pub(crate) const REVISION_ID: char = '\u{FFFE}';

/// The `w:rPr` children of a paragraph mark that record its change, which
/// the schema puts first.
const MARK: [&str; 2] = ["ins", "del"];

pub(crate) fn check_track(track: &Track) -> Result<(), EditError> {
    let invalid = |reason| {
        Err(EditError::Track {
            author: track.author.clone(),
            date: track.date.clone(),
            reason,
        })
    };
    check_text(&track.author)?;
    if track.author.trim().is_empty() {
        return invalid("a tracked change needs an author");
    }
    if !is_date(&track.date) {
        return invalid("the date must be written YYYY-MM-DDThh:mm:ssZ");
    }
    Ok(())
}

/// Whether a date is written as Word writes a change's time.
fn is_date(date: &str) -> bool {
    let b = date.as_bytes();
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    b.len() == 20
        && digits(0..4)
        && b[4] == b'-'
        && digits(5..7)
        && b[7] == b'-'
        && digits(8..10)
        && b[10] == b'T'
        && digits(11..13)
        && b[13] == b':'
        && digits(14..16)
        && b[16] == b':'
        && digits(17..19)
        && b[19] == b'Z'
}

impl Document {
    /// The tracked change a command made, if `id` is one.
    pub(crate) fn made_revision(&self, id: NodeId) -> Option<&RevisionInfo> {
        let node = self.n(id);
        match &node.kind {
            NodeKind::Revision(info) if node.made.is_some() => Some(info),
            _ => None,
        }
    }

    pub(crate) fn made_insertion(&self, id: NodeId) -> bool {
        self.made_revision(id).is_some_and(|r| r.change == "ins")
    }

    /// Whether a paragraph's mark records a change a command made.
    pub(crate) fn made_mark(&self, id: NodeId) -> bool {
        matches!(&self.n(id).kind, NodeKind::Paragraph(p) if p.mark_revision.is_some())
            && self.head(id).contains(REVISION_ID)
    }

    /// ` w:id="…" w:author="…" w:date="…"`, with the prefix `near` uses.
    fn revision_attributes(&self, near: NodeId, track: &Track) -> String {
        let id = self.w_attr_name(near, "id");
        // A document that writes its elements unprefixed declares the
        // prefix once, on the first attribute.
        let (declaration, id) = match id.rsplit_once(' ') {
            Some((declaration, id)) => (format!(" {declaration}"), id.to_string()),
            None => (String::new(), id),
        };
        let prefix = id.strip_suffix("id").unwrap_or_default();
        format!(
            "{declaration} {id}=\"{REVISION_ID}\" {prefix}author=\"{}\" {prefix}date=\"{}\"",
            escape_attr(&track.author),
            escape_attr(&track.date)
        )
    }

    pub(crate) fn make_revision(&mut self, near: NodeId, change: Change, track: &Track) -> NodeId {
        let name = self.w_name(near, change.local());
        let made = Made {
            head: format!("<{name}{}>", self.revision_attributes(near, track)),
            tail: format!("</{name}>"),
            like: None,
        };
        let info = RevisionInfo {
            change: change.local().to_string(),
            author: Some(track.author.clone()),
            date: Some(track.date.clone()),
            ..RevisionInfo::default()
        };
        let story = self.n(near).story;
        self.create(NodeKind::Revision(info), Support::Editable, made, story)
    }

    pub(crate) fn compose_mark_changes(
        &mut self,
        id: NodeId,
        insertion: Option<&Track>,
        deletion: Option<&Track>,
    ) -> Result<(), EditError> {
        let mut set = Vec::new();
        for (change, track) in [(Change::Insertion, insertion), (Change::Deletion, deletion)] {
            if let Some(track) = track {
                check_track(track)?;
                let name = self.w_name(id, change.local());
                set.push(format!("<{name}{}/>", self.revision_attributes(id, track)));
            }
        }
        let (ppr_name, rpr_name) = (self.w_name(id, "pPr"), self.w_name(id, "rPr"));
        let ppr = self.properties(id, "pPr");
        let scope = &self.scope(id);
        let rpr = child(&ppr, scope, "rPr").unwrap_or_default();
        let inside = &scope.within(&ppr[..tag_end(&ppr, 0).unwrap_or(ppr.len())]);
        let rpr = rewrite(rpr, inside, &rpr_name, &MARK, &MARK, set);
        let rpr = (!rpr.is_empty()).then_some(rpr).into_iter().collect();
        let ppr = rewrite(&ppr, scope, &ppr_name, &PPR_ORDER, &["rPr"], rpr);
        self.replace_properties(
            id,
            "pPr",
            &ppr,
            |scope, e| scope.is(e, "rPr"),
            "only the paragraph mark's tracked change can change this way",
        )?;
        if let NodeKind::Paragraph(info) = &mut self.nodes[id.index()].kind {
            let tracked = |t: &Track| Tracked {
                author: Some(t.author.clone()),
                date: Some(t.date.clone()),
            };
            info.mark_revision = deletion
                .map(|_| "del")
                .or_else(|| insertion.map(|_| "ins"))
                .map(str::to_string);
            info.mark_change = deletion.or(insertion).map(tracked);
            info.mark_insertion = deletion.and(insertion).map(tracked);
        }
        Ok(())
    }
}

impl Document {
    /// `head`, the start tag and properties of a node of story `story`, with
    /// the ID of each tracked change in it left for export to give, as
    /// export gives one to a change a command makes. A copy of markup that
    /// records a tracked change records a change of its own: its properties
    /// are the source's, and its ID is not.
    ///
    /// Only the value of a WordprocessingML `id` attribute on a
    /// WordprocessingML tracked-change element is replaced. Another
    /// namespace's attribute or element of the same name is someone else's,
    /// and every other byte stays as the source has it. `None` when `head`
    /// cannot be read with the namespaces the story's root declares.
    pub(crate) fn renumbered(&self, story: u32, head: &str) -> Option<String> {
        let local = |name: &str| REVISIONS.contains(&name.rsplit(':').next().unwrap_or(name));
        // Most heads record no change, and need no reading.
        let mut tags = head
            .match_indices('<')
            .map(|(at, _)| start_tag_name(&head[at..]));
        if !tags.any(local) {
            return Some(head.to_string());
        }
        let story = &self.stories[story as usize];
        let text = &story.xml.text;
        let range = self.n(story.root).range.clone();
        let open = crate::format::tag_end(text, range.start)?;
        let root = &text[range.start..open];
        let wrapped = format!(
            "{root}{head}</{}></{}>",
            start_tag_name(head),
            qname(text, &range)
        );
        let doc = crate::xml::parse(&story.part, &wrapped).ok()?;
        let mut ids: Vec<Range<usize>> = (doc.root_element().descendants())
            .filter(|n| {
                let tag = n.tag_name();
                n.is_element() && tag.namespace() == Some(W) && REVISIONS.contains(&tag.name())
            })
            .flat_map(|n| n.attributes())
            .filter(|a| a.namespace() == Some(W) && a.name() == "id")
            .map(|a| a.range_value())
            .collect();
        ids.sort_by_key(|r| std::cmp::Reverse(r.start));
        let mut out = head.to_string();
        for id in ids {
            out.replace_range(
                id.start - root.len()..id.end - root.len(),
                &REVISION_ID.to_string(),
            );
        }
        Some(out)
    }
}
