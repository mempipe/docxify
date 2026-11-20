//! Building part of the tree outright.
//!
//! A document derived from a flat editing state is not edited into shape.
//! Whoever derives it knows what each paragraph holds in the end, makes the
//! nodes that are new, and says what each container holds. Nothing here is
//! an edit: none of it has an inverse, records a change, or asks what a
//! user may do. It checks only that the tree stays well formed.

use crate::edit::{Change, Format, Track};
use crate::error::EditError;
use crate::format::{declarations, tag_end};
use crate::ir::{Made, NodeId, NodeKind, Support};
use crate::structure::{fits, refuse, Leaf};
use crate::Document;

impl Document {
    /// Makes `parent` hold exactly `children`, in this order. A child comes
    /// from wherever it is, in the document or out of it, and what `parent`
    /// held and `children` leaves out is left detached. A list equal to the
    /// one `parent` holds changes nothing, so the paragraph keeps its source
    /// bytes.
    pub(crate) fn arrange(&mut self, parent: NodeId, children: &[NodeId]) -> Result<(), EditError> {
        let unknown = |node| EditError::UnknownNode { node };
        let node = self.nodes.get(parent.index()).ok_or(unknown(parent))?;
        if node.children == children {
            return Ok(());
        }
        self.check_editable(parent)?;
        let mut above = Some(parent);
        while let Some(id) = above {
            if children.contains(&id) {
                return refuse(id, "it cannot hold itself");
            }
            above = self.n(id).parent;
        }
        let node = self.n(parent);
        let mut seen = std::collections::HashSet::with_capacity(children.len());
        for &id in children {
            let child = self.nodes.get(id.index()).ok_or(unknown(id))?;
            if child.story != node.story {
                return refuse(id, "it belongs to another story");
            }
            if !fits(&child.kind, &node.kind) {
                return Err(EditError::WrongKind {
                    node: id,
                    kind: child.kind.name(),
                    expected: "content its container can hold",
                });
            }
            if !seen.insert(id) {
                return refuse(id, "it is listed twice");
            }
        }
        for id in std::mem::take(&mut self.nodes[parent.index()].children) {
            self.nodes[id.index()].parent = None;
        }
        for &id in children {
            if self.n(id).parent.is_some() {
                self.detach(id);
            }
            self.nodes[id.index()].parent = Some(parent);
        }
        self.nodes[parent.index()].children = children.to_vec();
        self.touch(parent);
        Ok(())
    }

    /// Copy a fragment of a source text leaf. Only the first fragment carries
    /// the XML between that leaf and the preceding child.
    pub(crate) fn copy_source_text(
        &mut self,
        original: &crate::ir::Node,
        text: &str,
        include_lead: bool,
    ) -> NodeId {
        let mut leaf = original.clone();
        leaf.id = self.nodes.next_id();
        leaf.parent = None;
        if !include_lead {
            leaf.lead = leaf.range.start;
        }
        let id = leaf.id;
        if let NodeKind::Text { text: old } = &mut leaf.kind {
            if old != text {
                text.clone_into(old);
                leaf.dirty = true;
            }
        }
        self.nodes.push(leaf);
        id
    }

    /// Restore the immutable subtrees before applying the new paragraph state.
    /// A source paragraph that disappeared in a join starts detached.
    pub(crate) fn restore_paragraph_sources(
        &mut self,
        base: &Document,
        owners: impl Iterator<Item = NodeId>,
        siblings: &[NodeId],
    ) -> std::collections::HashSet<NodeId> {
        let mut restored = std::collections::HashSet::new();
        for owner in owners {
            for id in std::iter::once(owner).chain(base.descendants(owner)) {
                self.nodes[id.index()] = base.nodes[id.index()].clone();
                restored.insert(id);
            }
            if !siblings.contains(&owner) {
                self.nodes[owner.index()].parent = None;
            }
        }
        restored
    }

    /// Puts back a node a builder made, as it was when it was made.
    pub(crate) fn restore_made(&mut self, node: &crate::ir::Node) {
        self.nodes[node.id.index()] = node.clone();
    }

    /// Reuse the projected paragraph's identity if its mark survived.
    pub(crate) fn rebuild_paragraph(
        &mut self,
        parent: NodeId,
        like: Option<NodeId>,
        previous: Option<NodeId>,
    ) -> Result<NodeId, EditError> {
        if let Some(id) = previous {
            self.nodes.release(id);
        }
        let id = self.loose_paragraph(parent, like)?;
        if previous.is_some() {
            debug_assert_eq!(previous, Some(id));
            self.nodes[id.index()].parent = Some(parent);
        }
        Ok(id)
    }

    /// A new, empty run or hyperlink with the start tag and properties of
    /// `node`, in no container yet. A tracked change `node`'s properties
    /// record is a change of the copy's own, which export gives its own ID.
    pub(crate) fn empty_like(&mut self, node: NodeId) -> Result<NodeId, EditError> {
        let kind = &self
            .nodes
            .get(node.index())
            .ok_or(EditError::UnknownNode { node })?
            .kind;
        if !matches!(kind, NodeKind::Run(_) | NodeKind::Hyperlink(_)) {
            return Err(EditError::WrongKind {
                node,
                kind: kind.name(),
                expected: "a run or hyperlink",
            });
        }
        let (story, head) = (self.n(node).story, self.head(node));
        let Some(head) = self.renumbered(story, &head) else {
            return refuse(node, "its properties cannot be read to copy");
        };
        let copy = self.copy_of(node, 0);
        if let Some(made) = &mut self.nodes[copy.index()].made {
            made.head = head;
        }
        Ok(copy)
    }

    /// Declares on `node` each namespace the start tag of the source element
    /// `from` declares and `node` does not. Content that moves out of `from`
    /// can use a prefix only `from` declared.
    pub(crate) fn declare_namespaces(&mut self, node: NodeId, from: NodeId) {
        let source = self.n(from);
        if source.made.is_some() {
            return;
        }
        let text = self.story_text(source);
        let Some(end) = tag_end(text, source.range.start) else {
            return;
        };
        let tag = &text[source.range.start..end];
        if !tag.contains("xmlns") {
            return;
        }
        let head = self.head(node);
        let open = tag_end(&head, 0).unwrap_or(head.len());
        let own = declarations(&head[..open]);
        let missing: String = declarations(tag)
            .into_iter()
            .filter(|(name, _)| own.iter().all(|(declared, _)| declared != name))
            .map(|(name, value)| format!(" {name}={value}"))
            .collect();
        if missing.is_empty() {
            return;
        }
        let at = match head[..open].ends_with("/>") {
            true => open - 2,
            false => open - 1,
        };
        let head = format!("{}{missing}{}", &head[..at], &head[at..]);
        let placed = &mut self.nodes[node.index()];
        match &mut placed.made {
            Some(made) => made.head = head,
            None => {
                placed.head = Some(head.into_boxed_str());
                placed.dirty = true;
            }
        }
        self.mark_ancestors(node);
    }

    /// A new, empty run for a paragraph of `near`'s story, in no container
    /// yet, formatted like `like`: a source run, the mark of a source
    /// paragraph, or nothing for a bare run.
    pub(crate) fn empty_run(
        &mut self,
        near: NodeId,
        like: Option<NodeId>,
    ) -> Result<NodeId, EditError> {
        let story = self
            .nodes
            .get(near.index())
            .ok_or(EditError::UnknownNode { node: near })?
            .story;
        let (info, head, like) = self.run_from(near, like)?;
        let tail = format!("</{}>", crate::xml::start_tag_name(&head));
        Ok(self.create(
            NodeKind::Run(Box::new(info)),
            Support::Editable,
            Made { head, tail, like },
            story,
        ))
    }

    /// A new, empty paragraph for the story of `near`, in no container yet,
    /// with the properties of the source paragraph `like` or with none.
    pub(crate) fn loose_paragraph(
        &mut self,
        near: NodeId,
        like: Option<NodeId>,
    ) -> Result<NodeId, EditError> {
        self.nodes
            .get(near.index())
            .ok_or(EditError::UnknownNode { node: near })?;
        self.make_paragraph(near, like)
    }

    /// Sets bold, italic and underline on a run that is in a paragraph of
    /// the document, each on, off, or left as the run has it. A property the run's style
    /// already gives is not repeated.
    pub(crate) fn format_run(&mut self, run: NodeId, format: Format) -> Result<(), EditError> {
        let node = self.nodes.get(run.index());
        let node = node.ok_or(EditError::UnknownNode { node: run })?;
        let placed = self.paragraph_of(run).is_some();
        if !matches!(node.kind, NodeKind::Run(_)) || !placed {
            return Err(EditError::WrongKind {
                node: run,
                kind: node.kind.name(),
                expected: "a run within a paragraph",
            });
        }
        let properties = self.formatted(run, format);
        self.compose_run_properties(run, &properties).map(drop)
    }

    /// Builds a tracked-change wrapper without applying an editing command.
    pub(crate) fn loose_revision(
        &mut self,
        near: NodeId,
        change: Change,
        track: &Track,
    ) -> Result<NodeId, EditError> {
        self.nodes
            .get(near.index())
            .ok_or(EditError::UnknownNode { node: near })?;
        crate::revisions::check_track(track)?;
        Ok(self.make_revision(near, change, track))
    }

    /// Materializes both authors when an inserted boundary is deleted.
    pub(crate) fn revision_mark(
        &mut self,
        paragraph: NodeId,
        insertion: Option<&Track>,
        deletion: Option<&Track>,
    ) -> Result<(), EditError> {
        let node = self
            .nodes
            .get(paragraph.index())
            .ok_or(EditError::UnknownNode { node: paragraph })?;
        if !matches!(node.kind, NodeKind::Paragraph(_)) {
            return Err(EditError::WrongKind {
                node: paragraph,
                kind: node.kind.name(),
                expected: "a paragraph",
            });
        }
        self.compose_mark_changes(paragraph, insertion, deletion)
    }

    /// A new text node holding `text`, in no container yet.
    pub(crate) fn loose_text(&mut self, near: NodeId, text: &str) -> Result<NodeId, EditError> {
        crate::edit::check_xml_text(text)?;
        self.nodes
            .get(near.index())
            .ok_or(EditError::UnknownNode { node: near })?;
        Ok(self.make_leaf(near, Leaf::Text(text.to_string())))
    }

    /// Deleted text has its own OOXML element; it stays available to review.
    pub(crate) fn loose_deleted_text(
        &mut self,
        near: NodeId,
        text: &str,
    ) -> Result<NodeId, EditError> {
        let node = self.loose_text(near, text)?;
        self.nodes[node.index()].kind = NodeKind::DeletedText { text: text.into() };
        Ok(node)
    }

    /// A new line break, in no container yet.
    pub(crate) fn loose_break(&mut self, near: NodeId) -> Result<NodeId, EditError> {
        self.nodes
            .get(near.index())
            .ok_or(EditError::UnknownNode { node: near })?;
        Ok(self.make_leaf(near, Leaf::Break))
    }

    /// A new inline picture of `media`, as
    /// [`InsertPicture`](crate::Command::InsertPicture) makes one, in no
    /// container yet.
    pub(crate) fn loose_picture(
        &mut self,
        near: NodeId,
        media: &str,
        width_emu: u64,
        height_emu: u64,
        description: Option<&str>,
        id: Option<u32>,
    ) -> Result<NodeId, EditError> {
        self.nodes
            .get(near.index())
            .ok_or(EditError::UnknownNode { node: near })?;
        let info = self.picture_info(media, width_emu, height_emu, description, id)?;
        Ok(self.make_leaf(near, Leaf::Picture(Box::new(info))))
    }
}

#[cfg(test)]
mod tests;
