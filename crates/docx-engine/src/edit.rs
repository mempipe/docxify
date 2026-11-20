//! Semantic edit commands.

use serde::{Deserialize, Serialize};

use crate::error::EditError;
use crate::fields::Pages;
use crate::ir::{NodeId, NodeKind, NumRef, Support};
use crate::lists::ListKind;
use crate::xml::is_xml_char;
use crate::Document;

/// A range of Unicode scalar values within one text node.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextRange {
    pub start: usize,
    pub end: usize,
}

/// A position for an insertion or a split.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Point {
    /// Inside a text node, `offset` Unicode scalar values from its start.
    Text { node: NodeId, offset: usize },
    /// Just before a node, in its parent.
    Before { before: NodeId },
    /// Just after a node, in its parent.
    After { after: NodeId },
    /// At the start of a paragraph, run or hyperlink.
    Start { start: NodeId },
    /// At the end of a paragraph, run or hyperlink.
    End { end: NodeId },
}

/// Character formatting a range can be given. `None` leaves a property as
/// it is.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Format {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    /// A single underline, or none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underline: Option<bool>,
}

/// Who makes a tracked change, and when: Word's author name, and a time as
/// `YYYY-MM-DDThh:mm:ssZ`.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub author: String,
    pub date: String,
}

/// What a tracked change records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    Insertion,
    Deletion,
}

impl Change {
    /// The element a change is written with.
    pub(crate) fn local(self) -> &'static str {
        match self {
            Change::Insertion => "ins",
            Change::Deletion => "del",
        }
    }
}

/// What a form fill puts in a content control.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fill {
    /// Text, for a plain or rich text control. A line feed is a line break,
    /// which only a multi-line plain text control and a rich text control take.
    Text(String),
    /// A check box's state.
    Checked(bool),
    /// The list item of a drop-down list or combo box with this value.
    Item(String),
    /// A date picker's date, as `YYYY-MM-DD`.
    Date(String),
}

/// An editing intent accepted by the headless editor and the replica.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    /// Replace `range` of a text node's text, or all of it when `range` is `None`.
    ReplaceText {
        node: NodeId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        range: Option<TextRange>,
        text: String,
    },
    /// Insert text at a point. Inside a text node this is a text change;
    /// elsewhere it adds a text node, in a run of its own between runs.
    InsertText { at: Point, text: String },
    /// Insert a line break (`w:br`) at a point.
    InsertBreak {
        at: Point,
        /// Replace the selection from `at` to this point as one edit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<Point>,
    },
    /// Insert a picture in the line at a point: `media`, a part name
    /// [`Document::media_name`] returned or an image the package holds, at
    /// `width_emu` by `height_emu` EMUs, with `description` as its
    /// alternative text. It takes drawing ID `id` when no other drawing has
    /// it, else the next free one.
    InsertPicture {
        at: Point,
        /// When supplied, replace the range from `at` to this point as one edit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<Point>,
        media: String,
        width_emu: u64,
        height_emu: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<u32>,
    },
    /// Split the paragraph holding a point. The paragraph keeps its ID and
    /// what comes before the point; a new paragraph after it takes the rest.
    SplitParagraph { at: Point },
    /// Join a paragraph with the paragraph after it. The first keeps its ID
    /// and properties. An empty first paragraph is removed instead, so the
    /// second keeps its own.
    MergeParagraphs { paragraph: NodeId },
    /// Remove a tab, break, drawing, simple field, tracked insertion or
    /// deletion, content control, table, table row, bookmark, permission
    /// range, proofing mark or preserved element. Any part of a complex field removes the
    /// whole field, and either end of a bookmark or permission range removes
    /// both.
    RemoveNode { node: NodeId },
    /// Replace everything from `from` to `to`, in one story or table cell,
    /// with `text`, in which a line feed starts a new paragraph. Empty text
    /// deletes the range. The paragraph at `from` keeps its ID and properties
    /// and takes what follows `to`. Text nodes in the range keep their nodes,
    /// emptied, and the items in it are removed; the text at `from` takes the
    /// first line.
    ///
    /// With `track`, the change is recorded as Word records typing with
    /// Track Changes on: the text, and each new paragraph mark, is a tracked
    /// insertion at `to`, and what the range held is marked deleted rather
    /// than removed. What the same author inserted is removed outright, and
    /// what is deleted already stays as it is.
    ReplaceRange {
        from: Point,
        to: Point,
        #[serde(default)]
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        track: Option<Track>,
    },
    /// Give the editable text from `from` to `to`, in one story, the
    /// formatting `format` names. Runs are split where the range ends inside
    /// them, and a property that matches the run's style is removed rather
    /// than repeated.
    FormatRange {
        from: Point,
        to: Point,
        format: Format,
    },
    /// Give every paragraph from the one holding `from` to the one holding
    /// `to`, in one story, the paragraph style `style`, a style ID. The
    /// default paragraph style is given by removing `w:pStyle`.
    SetParagraphStyle {
        from: Point,
        to: Point,
        style: String,
    },
    /// Put every paragraph from the one holding `from` to the one holding
    /// `to`, in one story, in a list of `kind`, or take them out of any list
    /// when `kind` is `None`. A list of that kind in the range, or in the
    /// paragraph just before or after it, is continued; otherwise the
    /// command adds one. Paragraphs already in a list keep their level.
    SetList {
        from: Point,
        to: Point,
        kind: Option<ListKind>,
    },
    /// Move every list paragraph from the one holding `from` to the one
    /// holding `to`, in one story, `by` levels, within the levels its list
    /// defines.
    ChangeListLevel { from: Point, to: Point, by: i8 },
    /// Give every paragraph from the one holding `from` to the one holding
    /// `to`, in one story, the `w:numPr` that `numbering` names, or remove
    /// it. A `num_id` of `"0"` takes away a list the style gives.
    SetNumbering {
        from: Point,
        to: Point,
        numbering: Option<NumRef>,
    },
    /// Make the content from `from` to `to`, in one story, a link to
    /// `target` in each paragraph it covers, or take it out of any link
    /// when `target` is `None`. A target starting with `#` names a bookmark
    /// in the document; anything else is a URL.
    SetLink {
        from: Point,
        to: Point,
        target: Option<String>,
    },
    /// Add a row just above `row`, or below it, with the cells and
    /// properties of `like`, a row of the same table, or of `row`. Each new
    /// cell holds an empty paragraph like the first one of its model.
    InsertRow {
        row: NodeId,
        below: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        like: Option<NodeId>,
    },
    /// Add a grid column to a table at `index`, with a cell in each row like
    /// the row's cell in column `like`. The new column takes half of
    /// `like`'s width, so the table keeps its width.
    InsertColumn {
        table: NodeId,
        index: usize,
        like: usize,
    },
    /// Remove grid column `index` of a table. The column before it, or the
    /// one after a first column, takes its width.
    RemoveColumn { table: NodeId, index: usize },
    /// Comment on the content from `from` to `to`, in one story, with
    /// `text`, in which a line feed starts a new paragraph, by `by`. The
    /// comment takes ID `id` when no comment has it, else one above every
    /// comment's.
    AddComment {
        from: Point,
        to: Point,
        text: String,
        by: Track,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        initials: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<u32>,
    },
    /// Reply to comment `comment`, by its ID, as Word does: the reply joins
    /// the thread of the comment that began it, and comments on its range.
    ReplyToComment {
        comment: String,
        text: String,
        by: Track,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        initials: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<u32>,
    },
    /// Ask Word to update the document's fields when it opens the file,
    /// or stop asking.
    SetUpdateFields { on: bool },
    /// Fill content control `control` as a form does. One that is data-bound,
    /// locked against editing, or inside other content the engine keeps
    /// read-only is refused, as is a fill of the wrong type.
    FillContentControl { control: NodeId, fill: Fill },
    /// Update the body's fields as Word does when every field is selected
    /// and updated: `PAGE`, `NUMPAGES` and `PAGEREF` from `pages`, and
    /// `DATE` and `TIME` from `now`, the local date and time written
    /// `YYYY-MM-DDThh:mm:ss`. Other fields keep their results. Refused
    /// when no result would change.
    UpdateFields {
        now: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pages: Option<Pages>,
    },
    /// Make `text` the result of field `field`, by its begin character or
    /// simple field: the result's first text node takes it and the others
    /// are emptied. A result that is more than text in the field's
    /// paragraph is refused.
    SetFieldResult { field: NodeId, text: String },
}

macro_rules! command_names {
    ($($variant:ident => $name:literal),+ $(,)?) => {
        pub const COMMANDS: [&str; [$($name),+].len()] = [$($name),+];
        impl Command {
            pub fn name(&self) -> &'static str {
                match self { $(Self::$variant { .. } => $name),+ }
            }
        }
    };
}

command_names! {
    ReplaceText => "replace_text",
    InsertText => "insert_text",
    InsertBreak => "insert_break",
    InsertPicture => "insert_picture",
    SplitParagraph => "split_paragraph",
    MergeParagraphs => "merge_paragraphs",
    RemoveNode => "remove_node",
    ReplaceRange => "replace_range",
    FormatRange => "format_range",
    SetParagraphStyle => "set_paragraph_style",
    SetList => "set_list",
    ChangeListLevel => "change_list_level",
    SetNumbering => "set_numbering",
    SetLink => "set_link",
    InsertRow => "insert_row",
    InsertColumn => "insert_column",
    RemoveColumn => "remove_column",
    AddComment => "add_comment",
    ReplyToComment => "reply_to_comment",
    SetUpdateFields => "set_update_fields",
    FillContentControl => "fill_content_control",
    UpdateFields => "update_fields",
    SetFieldResult => "set_field_result",
}

/// Children `at..at + removed` of `flow`, a story or table cell of `story`,
/// are now `blocks`.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Splice {
    pub story: NodeId,
    pub flow: NodeId,
    pub at: usize,
    pub removed: usize,
    pub blocks: Vec<NodeId>,
}

impl Document {
    /// The text of `id` if a command may edit it.
    pub fn editable_text(&self, id: NodeId) -> Result<&str, EditError> {
        let node = self
            .nodes
            .get(id.index())
            .ok_or(EditError::UnknownNode { node: id })?;
        let NodeKind::Text { text } = &node.kind else {
            return Err(EditError::NotText {
                node: id,
                kind: node.kind.name(),
            });
        };
        match node.support {
            Support::Editable => Ok(text),
            Support::ReadOnly(reason) => Err(EditError::ReadOnly { node: id, reason }),
            Support::Opaque => Err(EditError::ReadOnly {
                node: id,
                reason: "inside content the engine does not model",
            }),
        }
    }

    pub(crate) fn mark_ancestors(&mut self, id: NodeId) {
        let mut next = self.nodes[id.index()].parent;
        while let Some(parent) = next {
            let node = &mut self.nodes[parent.index()];
            node.dirty_below = true;
            next = node.parent;
        }
    }
}

pub(crate) fn check_xml_text(text: &str) -> Result<(), EditError> {
    if let Some(found) = text.chars().find(|c| !is_xml_char(*c)) {
        return Err(EditError::InvalidText {
            found,
            reason: "XML cannot represent it",
        });
    }
    Ok(())
}

/// Whether `text` can be the content of a text node.
pub fn check_text(text: &str) -> Result<(), EditError> {
    check_xml_text(text)?;
    // Tabs and breaks are their own OOXML elements; inserting them needs
    // a structural command, not a text change.
    if let Some(found) = text.chars().find(|c| matches!(c, '\t' | '\n' | '\r')) {
        return Err(EditError::InvalidText {
            found,
            reason: "tabs and line breaks are elements, not text",
        });
    }
    Ok(())
}
