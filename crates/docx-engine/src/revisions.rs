//! DOCX revision markup, authorship and export-time revision IDs.
//! The editing session decides what an insertion or deletion covers.

use std::ops::Range;

use crate::edit::{check_text, Change, Track};
use crate::error::EditError;
use crate::format::{child, rewrite, tag_end, Unreadable, PPR_ORDER};
use crate::ir::{Made, NodeId, NodeKind, RevisionInfo, Support, Tracked};
use crate::tables::REVISIONS;
use crate::xml::{escape_attr, is, start_tag_name, w_local, W};
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
    if track
        .date
        .strip_suffix('Z')
        .and_then(crate::dates::Date::parse_time)
        .is_none()
    {
        return invalid("the date must be written YYYY-MM-DDThh:mm:ssZ");
    }
    Ok(())
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
        let (declaration, prefix) = self.w_attr_prefix(near);
        format!(
            " {declaration}{prefix}:id=\"{REVISION_ID}\" {prefix}:author=\"{}\" {prefix}:date=\"{}\"",
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
        let (ppr_name, rpr_name) = (self.w_name(id, "pPr"), self.w_name(id, "rPr"));
        let ppr = self.properties(id, "pPr")?;
        let scope = &self.scope(id);
        let unreadable = |e: Unreadable| e.on(id);
        let rpr = child(&ppr, scope, "rPr").map_err(unreadable)?;
        // An existing rPr stays in the scope of pPr. A new rPr and its marks
        // are made in the paragraph's scope and carried into pPr together.
        let owner = match rpr {
            Some(_) => scope.within(&ppr[..tag_end(&ppr, 0).expect("parsed pPr")]),
            None => scope.clone(),
        };
        let mut set = Vec::new();
        for (change, track) in [(Change::Insertion, insertion), (Change::Deletion, deletion)] {
            if let Some(track) = track {
                check_track(track)?;
                let name = self.w_name(id, change.local());
                let mark = format!("<{name}{}/>", self.revision_attributes(id, track));
                set.push(scope.carried_to(&owner, &mark));
            }
        }
        let rpr = rewrite(
            rpr.unwrap_or_default(),
            &owner,
            &rpr_name,
            &MARK,
            &MARK,
            set,
        )
        .map_err(unreadable)?;
        let rpr = (!rpr.is_empty()).then_some(rpr).into_iter().collect();
        let ppr =
            rewrite(&ppr, &owner, &ppr_name, &PPR_ORDER, &["rPr"], rpr).map_err(unreadable)?;
        self.replace_properties(
            id,
            "pPr",
            &ppr,
            |e| is(e, W, "rPr"),
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
    /// `head`, the start tag and properties of `node`, with
    /// the ID of each tracked change in it left for export to give, as
    /// export gives one to a change a command makes. A copy of markup that
    /// records a tracked change records a change of its own: its properties
    /// are the source's, and its ID is not.
    ///
    /// Only the value of a WordprocessingML `id` attribute on a
    /// WordprocessingML tracked-change element is replaced. Another
    /// namespace's attribute or element of the same name is someone else's,
    /// and every other byte stays as the source has it. `None` when `head`
    /// cannot be read in its source namespace context.
    pub(crate) fn renumbered(&self, node: NodeId, head: &str) -> Option<String> {
        let local = |name: &str| REVISIONS.contains(&name.rsplit(':').next().unwrap_or(name));
        // Most heads record no change, and need no reading.
        let mut tags = head
            .match_indices('<')
            .map(|(at, _)| start_tag_name(&head[at..]));
        if !tags.any(local) {
            return Some(head.to_string());
        }
        let end = format!("</{}>", start_tag_name(head));
        let read = self.scope(node).read(head, &end, |root, offset| {
            (root.descendants())
                .filter(|n| w_local(*n).is_some_and(|local| REVISIONS.contains(&local)))
                .flat_map(|n| n.attributes())
                .filter(|a| a.namespace() == Some(W) && a.name() == "id")
                .map(|a| a.range_value().start - offset..a.range_value().end - offset)
                .collect()
        });
        let mut ids: Vec<Range<usize>> = read.ok()?;
        ids.sort_by_key(|r| std::cmp::Reverse(r.start));
        let mut out = head.to_string();
        for id in ids {
            out.replace_range(id, &REVISION_ID.to_string());
        }
        Some(out)
    }
}
