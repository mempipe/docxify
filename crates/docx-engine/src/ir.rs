//! The semantic document IR.
//!
//! Nodes live in one arena and are addressed by [`NodeId`]. Every imported
//! node also records the byte range of the XML element it came from, so the
//! exporter can copy unchanged subtrees verbatim and regenerate only what an
//! edit touched. That source mapping is internal: the public IR exposes
//! semantics, and the XML stays the engine's business.

use std::collections::BTreeMap;
use std::fmt;
use std::ops::Range;
use std::str::FromStr;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A node's identity within one opened document.
///
/// IDs are assigned in document order at import, so the same bytes always
/// produce the same source IDs. An editing view may recycle IDs of derived
/// nodes on rebuild. Resolve a fresh point before issuing a later command.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(with = "String", extend("tsType" = "`n${number}`")))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub(crate) u32);

impl NodeId {
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "n{}", self.0)
    }
}

impl FromStr for NodeId {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.strip_prefix('n')
            .and_then(|n| n.parse().ok())
            .map(NodeId)
            .ok_or_else(|| format!("not a node id: {s:?}"))
    }
}

impl Serialize for NodeId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for NodeId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// How the engine treats a node and everything inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    /// Modelled, and engine commands may change it or its descendants.
    Editable,
    /// Modelled and preserved; commands refuse to change it, for the reason given.
    ReadOnly(&'static str),
    /// Not modelled; preserved byte for byte.
    Opaque,
}

impl Support {
    pub fn label(self) -> &'static str {
        match self {
            Support::Editable => "editable",
            Support::ReadOnly(_) => "read_only",
            Support::Opaque => "opaque",
        }
    }

    pub fn reason(self) -> Option<&'static str> {
        match self {
            Support::ReadOnly(reason) => Some(reason),
            _ => None,
        }
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StoryKind {
    Body,
    Header,
    Footer,
    Footnotes,
    Endnotes,
    Comments,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteKind {
    Footnote,
    Endnote,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RunProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strike: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caps: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub small_caps: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_half_points: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight: Option<String>,
    /// The family Word asks for, with theme fonts resolved at import.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vert_align: Option<String>,
}

impl RunProps {
    /// Later layers override earlier ones property by property. Word's toggle
    /// properties (bold, italic, caps, strike) XOR across style levels instead;
    /// that subtlety is not modelled yet.
    pub fn overlay(&mut self, top: &RunProps) {
        macro_rules! take {
            ($($f:ident),*) => { $( if top.$f.is_some() { self.$f = top.$f.clone(); } )* };
        }
        take!(
            bold,
            italic,
            underline,
            strike,
            caps,
            small_caps,
            hidden,
            size_half_points,
            color,
            highlight,
            font,
            vert_align
        );
    }

    pub fn is_empty(&self) -> bool {
        *self == RunProps::default()
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NumRef {
    pub num_id: String,
    pub level: u8,
}

/// A tab stop as one layer of formatting states it. `clear` removes a stop
/// an earlier layer set at the same position.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TabDef {
    pub pos: i32,
    pub align: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leader: Option<String>,
}

/// Paragraph properties as one layer states them: the defaults, a style, a
/// numbering level or the paragraph itself. Absent means inherited.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ParaProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alignment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline_level: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub numbering: Option<NumRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indent_left: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indent_right: Option<i32>,
    /// Positive for a first-line indent, negative for a hanging one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indent_first_line: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spacing_before: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spacing_after: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_autospacing: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_autospacing: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_rule: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contextual_spacing: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_next: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_lines: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub widow_control: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_break_before: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tabs: Vec<TabDef>,
}

impl ParaProps {
    /// Later layers override earlier ones property by property, and their tab
    /// stops replace or clear earlier stops at the same position.
    pub fn overlay(&mut self, top: &ParaProps) {
        macro_rules! take {
            ($($f:ident),*) => { $( if top.$f.is_some() { self.$f = top.$f.clone(); } )* };
        }
        take!(
            alignment,
            outline_level,
            numbering,
            indent_left,
            indent_right,
            indent_first_line,
            spacing_before,
            spacing_after,
            before_autospacing,
            after_autospacing,
            line,
            line_rule,
            contextual_spacing,
            keep_next,
            keep_lines,
            widow_control,
            page_break_before
        );
        for tab in &top.tabs {
            self.tabs.retain(|t| t.pos != tab.pos);
            if tab.align != "clear" {
                self.tabs.push(tab.clone());
            }
        }
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, Serialize)]
pub struct ParagraphInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(skip_serializing_if = "is_default")]
    pub props: ParaProps,
    /// Word's own paragraph identifier (`w14:paraId`), when the producer wrote one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub word_para_id: Option<String>,
    /// A tracked insertion or deletion of the paragraph mark itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mark_revision: Option<String>,
    /// Who made that change to the paragraph mark, and when.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mark_change: Option<Tracked>,
    /// The insertion retained beneath a tracked deletion of the same mark.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mark_insertion: Option<Tracked>,
    /// The paragraph carries a tracked property change (`w:pPrChange`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub format_revision: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<SectionInfo>,
    /// The paragraph mark's run properties. Text typed into an empty
    /// paragraph takes them, as Word does, unless they carry a revision.
    #[serde(skip)]
    pub(crate) mark_run: Option<Box<MarkRun>>,
}

#[derive(Debug, Clone)]
pub(crate) struct MarkRun {
    pub range: Range<usize>,
    pub info: RunInfo,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, Serialize)]
pub struct RunInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(skip_serializing_if = "RunProps::is_empty")]
    pub props: RunProps,
    /// The run carries a tracked formatting change (`w:rPrChange`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub format_revision: bool,
    /// The direct formatting before that change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<RunProps>,
    /// Who made that change, and when.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_change: Option<Tracked>,
}

/// The author and time of a tracked change.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Tracked {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, Serialize)]
pub struct HyperlinkInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, Serialize)]
pub struct RevisionInfo {
    /// `ins`, `del`, `moveFrom` or `moveTo`.
    pub change: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// For `moveFrom` and `moveTo`, the name of the move range it is in,
    /// which a move's source and destination share.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub move_name: Option<String>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, Serialize)]
pub struct DrawingInfo {
    /// `wp:docPr`'s `id`, which no other drawing in the document shares.
    #[serde(rename = "drawing_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Size in EMUs (`wp:extent`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width_emu: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height_emu: Option<u64>,
    /// `r:embed` / `r:link` relationship IDs and the parts or URLs they resolve to.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub media: Vec<MediaRef>,
    /// An enclosed reference or control lock that prevents removing the drawing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub holds: Option<&'static str>,
    pub placement: Placement,
}

/// Where a drawing sits: in the line, or anchored and positioned on the page.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Placement {
    #[default]
    Inline,
    Anchor(Box<Anchor>),
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize)]
pub struct Anchor {
    pub horizontal: Position,
    pub vertical: Position,
    /// `none`, `square`, `tight`, `through` or `top_and_bottom`.
    pub wrap: &'static str,
    pub behind: bool,
    pub distance_emu: Edges<i64>,
}

/// An anchored drawing's position on one axis: an offset from, or an
/// alignment within, what `relative_to` names.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, Serialize)]
pub struct Position {
    pub relative_to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset_emu: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<String>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Edges<T> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bottom: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<T>,
}

impl<T> Default for Edges<T> {
    fn default() -> Self {
        Edges {
            top: None,
            left: None,
            bottom: None,
            right: None,
        }
    }
}

impl<T: Clone> Edges<T> {
    pub fn overlay(&mut self, top: &Edges<T>) {
        for (mine, theirs) in [
            (&mut self.top, &top.top),
            (&mut self.left, &top.left),
            (&mut self.bottom, &top.bottom),
            (&mut self.right, &top.right),
        ] {
            if theirs.is_some() {
                mine.clone_from(theirs);
            }
        }
    }
}

/// One border as OOXML states it. A `nil` or `none` style removes a border
/// an earlier layer drew.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Border {
    pub style: String,
    pub size_eighths: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

impl Border {
    pub fn drawn(&self) -> bool {
        !matches!(self.style.as_str(), "nil" | "none")
    }
}

/// A table's borders as one layer states them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableBorders {
    pub outer: Edges<Border>,
    pub inside_h: Option<Border>,
    pub inside_v: Option<Border>,
}

/// Table properties as the table style or the table itself states them.
/// `shading` comes from a table style's whole-table cell properties.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableProps {
    pub borders: TableBorders,
    pub cell_margins: Edges<i32>,
    pub indent: Option<i32>,
    pub alignment: Option<String>,
    pub shading: Option<String>,
}

impl TableProps {
    pub fn overlay(&mut self, top: &TableProps) {
        self.borders.outer.overlay(&top.borders.outer);
        if top.borders.inside_h.is_some() {
            self.borders.inside_h.clone_from(&top.borders.inside_h);
        }
        if top.borders.inside_v.is_some() {
            self.borders.inside_v.clone_from(&top.borders.inside_v);
        }
        self.cell_margins.overlay(&top.cell_margins);
        if top.indent.is_some() {
            self.indent = top.indent;
        }
        if top.alignment.is_some() {
            self.alignment.clone_from(&top.alignment);
        }
        if top.shading.is_some() {
            self.shading.clone_from(&top.shading);
        }
    }
}

/// A cell's own properties, before the table and its style are applied.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CellProps {
    pub borders: Edges<Border>,
    pub shading: Option<String>,
    pub margins: Edges<i32>,
    pub vertical_align: Option<String>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, Serialize)]
pub struct MediaRef {
    pub relationship: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    pub external: bool,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, Serialize)]
pub struct TableInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// Column widths in twentieths of a point (`w:tblGrid`).
    pub grid: Vec<u32>,
    /// The table's own properties, which inspection lays over its style's.
    #[serde(skip)]
    pub props: Box<TableProps>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, Serialize)]
pub struct RowInfo {
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub header: bool,
    #[serde(skip_serializing_if = "is_zero")]
    pub grid_before: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub grid_after: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub cant_split: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<RowHeight>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize)]
pub struct RowHeight {
    pub twips: u32,
    /// `at_least` or `exact`.
    pub rule: &'static str,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize)]
pub struct CellInfo {
    pub grid_span: u32,
    /// `restart` or `continue` for a vertically merged cell.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertical_merge: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip)]
    pub props: Box<CellProps>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, Serialize)]
pub struct SectionInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orientation: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub margins: BTreeMap<String, i32>,
    /// `nextPage`, `continuous`, `evenPage`, `oddPage` or `nextColumn`.
    pub start: String,
    /// The first page has its own header and footer.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub title_page: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_numbers: Option<PageNumbers>,
    /// Present only for more than one column.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<Columns>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<StoryRef>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub footers: Vec<StoryRef>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize)]
pub struct PageNumbers {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize)]
pub struct Columns {
    pub count: u32,
    pub space_twips: u32,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize)]
pub struct StoryRef {
    /// `default`, `first` or `even`.
    pub kind: String,
    pub relationship: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeKind {
    Story {
        story: StoryKind,
        part: String,
    },
    Paragraph(Box<ParagraphInfo>),
    Run(Box<RunInfo>),
    Text {
        text: String,
    },
    DeletedText {
        text: String,
    },
    FieldCode {
        text: String,
    },
    Tab,
    Break {
        #[serde(skip_serializing_if = "Option::is_none")]
        break_type: Option<String>,
    },
    Drawing(Box<DrawingInfo>),
    Hyperlink(HyperlinkInfo),
    Revision(RevisionInfo),
    Field {
        instruction: String,
    },
    ContentControl {
        #[serde(skip_serializing_if = "Option::is_none")]
        tag: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        alias: Option<String>,
        data_bound: bool,
        /// The combined locks of its possible markup branches.
        /// `sdtLocked` or `sdtContentLocked` forbid deleting it,
        /// and `contentLocked` or `sdtContentLocked` editing its content.
        #[serde(skip_serializing_if = "Option::is_none")]
        lock: Option<String>,
        control: Control,
        /// Source offset of the unmodeled sdtContent wrapper.
        #[serde(skip)]
        content_start: Option<usize>,
        /// `w:showingPlcHdr`: the content is the placeholder, not a value.
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        placeholder: bool,
    },
    Table(TableInfo),
    Row(RowInfo),
    Cell(CellInfo),
    Section(Box<SectionInfo>),
    /// A point or range marker: bookmark, comment range, proofing mark,
    /// complex-field character and the like.
    Marker {
        element: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        marker_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Opaque {
        element: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        namespace: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        text_preview: Option<String>,
        /// An enclosed reference or control lock that prevents removing the element.
        #[serde(skip_serializing_if = "Option::is_none")]
        holds: Option<&'static str>,
    },
    /// One footnote or endnote, in its notes story.
    Note {
        kind: NoteKind,
        note_id: String,
        /// A separator or continuation note, which Word draws itself.
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        separator: bool,
    },
    NoteReference {
        kind: NoteKind,
        note_id: String,
        /// The formatted number, in reference order. A reference the author
        /// follows with their own mark has none and takes none.
        #[serde(skip_serializing_if = "Option::is_none")]
        number: Option<String>,
        #[serde(skip)]
        custom_mark: bool,
    },
    /// The note's own number at its start, as its reference shows it.
    NoteMark {
        kind: NoteKind,
        #[serde(skip_serializing_if = "Option::is_none")]
        number: Option<String>,
    },
    /// One comment, in the comments story. Its range and reference are
    /// markers in the story it comments on, with the same ID.
    Comment {
        comment_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        author: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        date: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        initials: Option<String>,
        /// The comment it replies to, by ID: the first of its thread.
        #[serde(skip_serializing_if = "Option::is_none")]
        parent: Option<String>,
    },
}

impl NodeKind {
    pub fn name(&self) -> &'static str {
        match self {
            NodeKind::Story { .. } => "story",
            NodeKind::Paragraph(_) => "paragraph",
            NodeKind::Run(_) => "run",
            NodeKind::Text { .. } => "text",
            NodeKind::DeletedText { .. } => "deleted_text",
            NodeKind::FieldCode { .. } => "field_code",
            NodeKind::Tab => "tab",
            NodeKind::Break { .. } => "break",
            NodeKind::Drawing(_) => "drawing",
            NodeKind::Hyperlink(_) => "hyperlink",
            NodeKind::Revision(_) => "revision",
            NodeKind::Field { .. } => "field",
            NodeKind::ContentControl { .. } => "content_control",
            NodeKind::Table(_) => "table",
            NodeKind::Row(_) => "row",
            NodeKind::Cell(_) => "cell",
            NodeKind::Section(_) => "section",
            NodeKind::Marker { .. } => "marker",
            NodeKind::Opaque { .. } => "opaque",
            NodeKind::Note { .. } => "note",
            NodeKind::NoteReference { .. } => "note_reference",
            NodeKind::NoteMark { .. } => "note_mark",
            NodeKind::Comment { .. } => "comment",
        }
    }
}

/// Read-only geometry inside a preserved row atom, parsed alongside its XML.
/// The cells remain opaque and share the atom's lifetime.
#[derive(Debug, Clone)]
pub(crate) struct PreservedCell {
    pub grid_span: u32,
    pub vertical_merge: Option<String>,
}

/// A node is attached while its parent chain reaches a story root. A source node
/// keeps its ID and source bytes when detached. The projection may reuse
/// a discarded derived node’s slot.
#[derive(Debug, Clone)]
pub struct Node {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub kind: NodeKind,
    pub support: Support,
    pub(crate) preserved_cells: Option<Arc<Vec<PreservedCell>>>,
    /// Byte range of the source element in its story's text. Empty for a
    /// node a command created.
    pub(crate) range: Range<usize>,
    /// Where the whitespace before the element starts. It travels with the
    /// node, so moving children keeps the source's formatting.
    pub(crate) lead: usize,
    /// The span from the first child's start to the last child's end, as
    /// imported. A split source run can extend the span to its end tag when
    /// a later fragment takes its trailing markup. A childless container has
    /// an empty span at its end tag or self-closing end.
    pub(crate) inner: Range<usize>,
    /// Markup for a node a command created, which has no source bytes.
    pub(crate) made: Option<Box<Made>>,
    /// The start tag and property elements a command gave a node from the
    /// source, in place of the source's.
    pub(crate) head: Option<Box<str>>,
    /// Index of the story whose text `range` points into.
    pub(crate) story: u32,
    /// This node's own content changed and must be regenerated.
    pub(crate) dirty: bool,
    /// Some descendant changed, or the children changed, so this node is
    /// re-walked rather than copied.
    pub(crate) dirty_below: bool,
}

/// What a content control holds, from the type element in its `w:sdtPr`.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Control {
    /// No type element, or `w:richText`.
    RichText,
    /// `w:text`; `multi_line` lets its text hold line breaks.
    Text {
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        multi_line: bool,
    },
    /// `w14:checkbox`, with the symbols it shows checked and unchecked.
    Checkbox {
        checked: bool,
        #[serde(skip)]
        checked_symbol: char,
        #[serde(skip)]
        unchecked_symbol: char,
    },
    /// `w:dropDownList`: one of its items.
    DropDown {
        items: Vec<ListItem>,
        #[serde(skip_serializing_if = "Option::is_none")]
        last_value: Option<String>,
    },
    /// `w:comboBox`: one of its items, or any text.
    ComboBox {
        items: Vec<ListItem>,
        #[serde(skip_serializing_if = "Option::is_none")]
        last_value: Option<String>,
    },
    /// `w:date`: `full_date` is `w:fullDate`, and `format` its date picture.
    Date {
        #[serde(skip_serializing_if = "Option::is_none")]
        full_date: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        format: Option<String>,
    },
    /// Any other type, such as a picture, a building block, a group or a
    /// repeating section, by its element's local name.
    Other { element: String },
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ListItem {
    pub display_text: String,
    pub value: String,
}

/// The tags around a created node's children. A created leaf has its whole
/// element in `head`.
#[derive(Debug, Clone)]
pub(crate) struct Made {
    pub head: String,
    pub tail: String,
    /// The source node whose properties a made paragraph or run took, as
    /// [`Document::like`](crate::Document::like) reports it.
    pub like: Option<NodeId>,
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}
