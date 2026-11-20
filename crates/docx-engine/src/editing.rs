//! Semantic command preparation and projection, independent of Yrs.
//! Text edits write directly to strands. Table commands record structural
//! facts, and the table reader computes the visible grid from those facts.
//!
//! The flow holds, for each paragraph, a mark and then its text. A mark
//! names the paragraph of the file it is, or the one a paragraph a replica
//! added is like. Text states its *basis*: the run of the file it came
//! with, whose properties are the file's own bytes, known to the engine or
//! not. Bold, italic and underline are ranges over text.
//!
//! A document is built from a reading with the engine's building calls
//! (`Document::arrange` and the makers of loose nodes). A paragraph that
//! reads as the file has it is not touched, so export copies its bytes.
//! Runs are an output: a changed paragraph gets a run for each stretch of
//! text that reads alike.
//!
//! In `docx-engine.yrs/9`, a basis names the source run or paragraph.
//! Source text uses `run/leaf` to carry the original XML leaf as well.
//! `BARE` (the empty string) requests bare properties. A missing basis is damage. A new paragraph mark is `+` followed by its
//! source node ID (empty for bare properties), then `@client.clock`, a
//! logical identity carried through undo and distinct for independent Enters.
//!
//! A command is written to the flow as what it means there. It names nodes
//! of the view, and the view says where each node's content is in the flow.
//!
//! Editable regions cover every story and table cell. Rich inline objects,
//! paragraph properties and annotations are shared facts; immutable source
//! containers preserve the content this editing model does not enter.

#[path = "editing_annotations.rs"]
mod annotations;
#[path = "editing_incremental.rs"]
mod incremental;
#[path = "editing_inlines.rs"]
mod inlines;
#[path = "editing_tables.rs"]
mod tables;
use crate::{Change, ListKind, NumRef, Track};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use crate::{check_text, Command, Document, Format, NodeId, NodeKind, Point, Support, TextRange};

use crate::flow::{Content, Flow, Insert, Seeded, Slot, Stated, Unit};
use crate::state::{Reading, State, StateUndo, Strand};

/// A visible table constraint that holds a stored edit inactive.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    LastRow,
    LastColumn,
    MergeStartRow,
    MergeStartWrapper,
    SourceGrid,
}

#[path = "editing_keys.rs"]
mod keys;
use keys::*;
pub use keys::{BASIS, BIRTH, CELL, FORMAT, MARK};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum ListBasis {
    Source(String),
    Created { identity: String, kind: ListKind },
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct ListState {
    basis: ListBasis,
    level: u8,
}
fn list_state(value: &str) -> Result<Option<ListState>, SessionError> {
    serde_json::from_str(value).map_err(|_| corrupt("invalid paragraph numbering".into()))
}
/// A deletion is a range over generations already known to its author.
/// Later/concurrent insertions in that range remain independent content.
/// Birth pairs are immutable and outside undo, so restored content keeps
/// its generation even though Yrs gives the restored text new item IDs.
#[derive(serde::Serialize, serde::Deserialize)]
struct Deletion {
    by: Track,
    before: BTreeMap<u64, u32>,
}

fn birth(value: &str) -> Result<(u64, u32), SessionError> {
    value
        .split_once('.')
        .and_then(|(client, clock)| Some((client.parse().ok()?, clock.parse().ok()?)))
        .ok_or_else(|| corrupt("invalid insertion generation".into()))
}

fn deletion_of(unit: &Unit) -> Result<Option<Track>, SessionError> {
    let born = unit.one(BIRTH).map(birth).transpose()?;
    for (_, value) in unit.over.iter().rev().filter(|(key, _)| key == DELETION) {
        let Some(value) = value else { continue };
        let deletion: Deletion =
            serde_json::from_str(value).map_err(|_| corrupt("invalid tracked deletion".into()))?;
        if born.is_none_or(|(client, clock)| {
            deletion
                .before
                .get(&client)
                .is_some_and(|seen| clock <= *seen)
        }) {
            return Ok(Some(deletion.by));
        }
    }
    Ok(None)
}
/// What starts a paragraph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mark {
    /// The mark of a paragraph of the file.
    Base(NodeId),
    /// A mark a replica added, with a source template or bare properties.
    Made {
        like: Option<NodeId>,
        identity: (u64, u32),
    },
    /// No mark: text before the first one. One person joined the first
    /// paragraph away while another typed in it.
    Lost,
    Cell {
        key: String,
        like: Option<NodeId>,
    },
}

/// One character as the flow gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Char {
    pub c: char,
    pub source_text: Option<NodeId>,
    /// None explicitly means a bare run, as in an empty derived paragraph.
    pub basis: Option<NodeId>,
    /// Bold, italic and underline as replicas set them, over the basis.
    pub format: Format,
    pub revision: Option<Track>,
    pub deletion: Option<Track>,
    pub link: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NewInline {
    Break,
    Picture {
        media: String,
        width_emu: u64,
        height_emu: u64,
        description: Option<String>,
        id: Option<u32>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InlineObject {
    pub identity: String,
    pub kind: NewInline,
}

/// Text or one preserved inline node, with explicit source properties.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    /// An ignored restoration of an already present boundary. It keeps
    /// a raw position, but contributes no visible content.
    HiddenBoundary,
    Char(Char),
    Object {
        object: InlineObject,
        basis: Option<NodeId>,
        format: Format,
        revision: Option<Track>,
        deletion: Option<Track>,
        link: Option<String>,
    },
    Source {
        node: NodeId,
        basis: NodeId,
        format: Format,
        revision: Option<Track>,
        deletion: Option<Track>,
        link: Option<String>,
    },
}
impl Inline {
    fn basis(&self) -> Option<NodeId> {
        match self {
            Self::HiddenBoundary => None,
            Self::Char(c) => c.basis,
            Self::Source { basis, .. } => Some(*basis),
            Self::Object { basis, .. } => *basis,
        }
    }
    fn format(&self) -> Format {
        match self {
            Self::HiddenBoundary => Format::default(),
            Self::Char(c) => c.format,
            Self::Source { format, .. } | Self::Object { format, .. } => *format,
        }
    }
    fn revision(&self) -> Option<&Track> {
        match self {
            Self::HiddenBoundary => None,
            Self::Char(c) => c.revision.as_ref(),
            Self::Source { revision, .. } | Self::Object { revision, .. } => revision.as_ref(),
        }
    }
    fn link(&self) -> Option<&str> {
        match self {
            Self::HiddenBoundary => None,
            Self::Char(c) => c.link.as_deref(),
            Self::Source { link, .. } | Self::Object { link, .. } => link.as_deref(),
        }
    }
    fn deletion(&self) -> Option<&Track> {
        match self {
            Self::HiddenBoundary => None,
            Self::Char(c) => c.deletion.as_ref(),
            Self::Source { deletion, .. } | Self::Object { deletion, .. } => deletion.as_ref(),
        }
    }
}

/// One paragraph as the flow gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paragraph {
    pub mark: Mark,
    pub revision: Option<Track>,
    pub deletion: Option<Track>,
    pub text: Vec<Inline>,
}

fn corrupt(reason: String) -> SessionError {
    let flow = "body".to_string();
    SessionError::Corrupt { flow, reason }
}

fn unbuilt(reason: impl ToString) -> SessionError {
    SessionError::Unbuilt {
        flow: "body".to_string(),
        reason: reason.to_string(),
    }
}

/// A run of the file: its text, and its text nodes with how many
/// characters each holds.
struct Run {
    id: NodeId,
    text: Vec<Inline>,
    nodes: Vec<(NodeId, usize)>,
}

/// A maximal consecutive set of editable paragraphs under one container.
/// Tables and other blocks remain in place between regions; no text edit
/// can cross their boundary. A cell and a note are containers of their own.
struct Region {
    parent: NodeId,
    paragraphs: Vec<NodeId>,
}
fn regions(document: &Document) -> Vec<Region> {
    fn visit(doc: &Document, parent: NodeId, out: &mut Vec<Region>) {
        let mut paragraphs = Vec::new();
        for &id in &doc.node(parent).expect("source container").children {
            let node = doc.node(id).expect("source child");
            if matches!(node.kind, NodeKind::Paragraph(_)) && node.support == Support::Editable {
                paragraphs.push(id);
                continue;
            }
            if !paragraphs.is_empty() {
                out.push(Region {
                    parent,
                    paragraphs: std::mem::take(&mut paragraphs),
                });
            }
            if matches!(
                node.kind,
                NodeKind::Table(_) | NodeKind::Row(_) | NodeKind::Cell(_) | NodeKind::Note { .. }
            ) {
                visit(doc, id, out);
            }
        }
        if !paragraphs.is_empty() {
            out.push(Region { parent, paragraphs });
        }
    }
    let mut out = Vec::new();
    for story in document.stories() {
        visit(document, story.id, &mut out);
    }
    out
}

/// The source runs and atomic containers of one paragraph.
fn collect(doc: &Document, parent: NodeId, paragraph: NodeId, runs: &mut Vec<Run>) {
    for &id in &doc.node(parent).expect("source parent").children {
        let node = doc.node(id).expect("source child");
        if matches!(node.kind, NodeKind::Hyperlink(_))
            && node.support == Support::Editable
            && !node.children.is_empty()
        {
            collect(doc, id, paragraph, runs);
        } else if matches!(node.kind, NodeKind::Run(_))
            && node.support == Support::Editable
            && !node.children.is_empty()
            && node.children.iter().all(|c| {
                !matches!(
                    doc.node(*c).unwrap().kind,
                    NodeKind::FieldCode { .. } | NodeKind::DeletedText { .. }
                )
            })
        {
            let (mut text, mut nodes) = (Vec::new(), Vec::new());
            for &child in &node.children {
                match &doc.node(child).expect("source inline").kind {
                    NodeKind::Text { text: more }
                        if doc.node(child).unwrap().support == Support::Editable
                            && !more.is_empty() =>
                    {
                        text.extend(more.chars().map(|c| {
                            Inline::Char(Char {
                                c,
                                source_text: Some(child),
                                basis: Some(id),
                                format: Format::default(),
                                revision: None,
                                deletion: None,
                                link: None,
                            })
                        }));
                        nodes.push((child, more.chars().count()));
                    }
                    _ => {
                        text.push(Inline::Source {
                            node: child,
                            basis: id,
                            format: Format::default(),
                            revision: None,
                            deletion: None,
                            link: None,
                        });
                        nodes.push((child, 1));
                    }
                }
            }
            runs.push(Run { id, text, nodes });
        } else {
            runs.push(Run {
                id,
                text: vec![Inline::Source {
                    node: id,
                    basis: paragraph,
                    format: Format::default(),
                    revision: None,
                    deletion: None,
                    link: None,
                }],
                nodes: vec![(id, 1)],
            });
        }
    }
}

/// Source runs and atomic containers. A read-only container is carried as
/// one source item, never flattened into editable characters.
fn body(document: &Document) -> Result<Vec<(NodeId, Vec<Run>)>, SessionError> {
    let mut result = Vec::new();
    for region in regions(document) {
        for paragraph in region.paragraphs {
            let mut runs = Vec::new();
            collect(document, paragraph, paragraph, &mut runs);
            result.push((paragraph, runs));
        }
    }
    Ok(result)
}

/// Seeds a flow from the body of `document`. Source inline entries name
/// their node and basis as `node/run`; both are immutable references.
pub fn seed(client: u64, document: &Document) -> Result<State, SessionError> {
    let body = body(document)?;
    // Keep owned seed strings alive while the store borrows them.
    let mut strands = Vec::new();
    let mut clock = 0;
    let boundaries: HashSet<_> = regions(document)
        .into_iter()
        .skip(1)
        .map(|r| r.paragraphs[0])
        .collect();
    for (index, (paragraph, runs)) in body.iter().enumerate() {
        let mut parts: Vec<(String, String, Option<String>)> = Vec::new();
        let prefix = boundaries.contains(paragraph).then(|| {
            Slot::Unit(Content::Item {
                kind: SCOPE.into(),
                name: paragraph.to_string(),
            })
        });
        parts.push((MARK.into(), paragraph.to_string(), None));
        for run in runs {
            for (node, _) in &run.nodes {
                if let NodeKind::Text { text } = &document.n(*node).kind {
                    if document.n(*node).support == Support::Editable && !text.is_empty() {
                        parts.push((
                            BASIS.into(),
                            text.clone(),
                            Some(format!("{}/{node}", run.id)),
                        ));
                        continue;
                    }
                }
                let basis = if run.id == *node { *paragraph } else { run.id };
                parts.push((INLINE.into(), format!("{node}/{basis}"), None));
            }
        }
        let content: Vec<_> = parts
            .iter()
            .map(|(kind, value, basis)| match basis {
                Some(basis) => Seeded::Text {
                    text: value,
                    stated: vec![(BASIS, Some(basis))],
                },
                None => Seeded::Item { kind, name: value },
            })
            .collect();
        let mut flow = Flow::seeded(client, &content);
        clock += flow.next_identity().1;
        flow.set_clock(clock);
        strands.push(Strand {
            key: paragraph.to_string(),
            order: index as u64,
            prefix,
            flow: Arc::new(flow),
            declared: true,
        });
    }
    Ok(State::new(client, clock, strands, BTreeMap::new()))
}

/// Register immutable image data outside undo. Content names are checked by
/// the same package codec headless and collaborative callers use.
fn add_media(base: &Document, flow: &mut State, bytes: &[u8]) -> Result<String, SessionError> {
    let name = base.media_name(bytes)?;
    flow.set_fact(format!("{MEDIA}{name}"), bytes.to_vec(), true)?;
    Ok(name)
}

fn node(name: &str) -> Result<NodeId, SessionError> {
    let parsed = name.parse::<NodeId>();
    parsed.map_err(|_| corrupt(format!("{name} is not a node")))
}

/// The mark an item is. A mark a replica added names what it is like after
/// a `+`.
fn mark_of(unit: &Unit) -> Result<Option<Mark>, SessionError> {
    match &unit.content {
        Content::Char(_) => Ok(None),
        Content::Item { kind, .. } if kind == INLINE || kind == SCOPE || kind == NEW_INLINE => {
            Ok(None)
        }
        Content::Item { kind, name } if kind == MARK && name.starts_with('=') => {
            let (key, like) = serde_json::from_str(&name[1..])
                .map_err(|_| corrupt("invalid cell boundary".into()))?;
            Ok(Some(Mark::Cell { key, like }))
        }
        Content::Item { kind, name } if kind == MARK => Ok(Some(match name.strip_prefix('+') {
            Some(made) => {
                let (like, id) = made
                    .split_once('@')
                    .ok_or_else(|| corrupt("boundary has no identity".into()))?;
                let (client, clock) = id
                    .split_once('.')
                    .ok_or_else(|| corrupt("invalid boundary identity".into()))?;
                let identity = (
                    client
                        .parse()
                        .map_err(|_| corrupt("invalid boundary client".into()))?,
                    clock
                        .parse()
                        .map_err(|_| corrupt("invalid boundary clock".into()))?,
                );
                Mark::Made {
                    like: optional_node(like)?,
                    identity,
                }
            }
            None => Mark::Base(node(name)?),
        })),
        Content::Item { kind, .. } => Err(corrupt(format!("unknown item {kind}"))),
    }
}

fn optional_node(name: &str) -> Result<Option<NodeId>, SessionError> {
    if name == BARE {
        Ok(None)
    } else {
        node(name).map(Some)
    }
}

fn made(like: Option<NodeId>, identity: (u64, u32)) -> String {
    format!(
        "+{}@{}.{}",
        like.map(|id| id.to_string())
            .unwrap_or_else(|| BARE.to_owned()),
        identity.0,
        identity.1
    )
}

impl Mark {
    /// The paragraph of the file whose properties the paragraph has.
    fn like(&self) -> Option<NodeId> {
        match self {
            Mark::Base(id) => Some(*id),
            Mark::Made { like, .. } | Mark::Cell { like, .. } => *like,
            Mark::Lost => None,
        }
    }
}

/// What a character states and what is over it, or nothing for an item.
/// A missing basis is damage. An explicitly empty basis is a bare run.
fn insertion_of(unit: &Unit) -> Result<Option<Track>, SessionError> {
    unit.one(INSERTION)
        .map(|value| {
            serde_json::from_str(value).map_err(|_| corrupt("invalid tracked insertion".into()))
        })
        .transpose()
}

fn inline_of(unit: &Unit) -> Result<Option<Inline>, SessionError> {
    let set = |key| unit.one(key).map(|value| value == "1");
    let format = Format {
        bold: set(BOLD),
        italic: set(ITALIC),
        underline: set(UNDERLINE),
    };
    match &unit.content {
        Content::Char(c) => {
            let basis = unit
                .one(BASIS)
                .ok_or_else(|| corrupt(format!("{c} has no basis")))?;
            let (basis, leaf) = basis
                .split_once('/')
                .map_or((basis, None), |(run, leaf)| (run, Some(leaf)));
            Ok(Some(Inline::Char(Char {
                source_text: leaf.map(node).transpose()?,
                c: *c,
                basis: optional_node(basis)?,
                format,
                revision: insertion_of(unit)?,
                deletion: deletion_of(unit)?,
                link: unit.one(LINK).map(str::to_string),
            })))
        }
        Content::Item { kind, name } if kind == NEW_INLINE => {
            let basis = unit
                .one(BASIS)
                .ok_or_else(|| corrupt("new inline has no basis".into()))?;
            let object =
                serde_json::from_str(name).map_err(|_| corrupt("invalid new inline".into()))?;
            Ok(Some(Inline::Object {
                object,
                basis: optional_node(basis)?,
                format,
                revision: insertion_of(unit)?,
                deletion: deletion_of(unit)?,
                link: unit.one(LINK).map(str::to_string),
            }))
        }
        Content::Item { kind, name } if kind == INLINE => {
            let (id, basis) = name
                .split_once('/')
                .ok_or_else(|| corrupt("inline has no source and basis".into()))?;
            Ok(Some(Inline::Source {
                node: node(id)?,
                basis: node(basis)?,
                format,
                revision: insertion_of(unit)?,
                deletion: deletion_of(unit)?,
                link: unit.one(LINK).map(str::to_string),
            }))
        }
        _ => Ok(None),
    }
}

struct Located {
    first: usize,
    scope: Option<String>,
    paragraph: Paragraph,
}
fn group(units: &Reading) -> Result<Vec<Located>, SessionError> {
    let lost = || Paragraph {
        mark: Mark::Lost,
        revision: None,
        deletion: None,
        text: Vec::new(),
    };
    let mut current = None;
    let mut located = Vec::new();
    let mut scope = None;
    let mut first = 0;
    let mut seen = HashSet::new();
    for (at, unit) in units.range(..).enumerate() {
        if let Content::Item { kind, name } = &unit.content {
            if kind == SCOPE {
                located.push(Located {
                    first,
                    scope: scope.take(),
                    paragraph: current.take().unwrap_or_else(lost),
                });
                scope = Some(name.clone());
                first = at + 1;
                continue;
            }
        }
        if let Some(mark) = mark_of(unit)? {
            let identity = match &mark {
                Mark::Base(id) => Some(format!("base:{id}")),
                Mark::Made {
                    identity: (client, clock),
                    ..
                } => Some(format!("made:{client}.{clock}")),
                Mark::Cell { key, .. } => Some(key.clone()),
                _ => None,
            };
            if identity.is_some_and(|id| !seen.insert(id)) {
                current
                    .get_or_insert_with(lost)
                    .text
                    .push(Inline::HiddenBoundary);
                continue;
            }
            if let Some(paragraph) = current.take() {
                located.push(Located {
                    first,
                    scope: scope.clone(),
                    paragraph,
                });
            }
            first = at + 1;
            current = Some(Paragraph {
                mark,
                text: Vec::new(),
                revision: insertion_of(unit)?,
                deletion: deletion_of(unit)?,
            });
        } else if let Some(char) = inline_of(unit)? {
            current.get_or_insert_with(lost).text.push(char);
        }
    }
    located.push(Located {
        first,
        scope,
        paragraph: current.unwrap_or_else(lost),
    });
    Ok(located)
}

/// Reads a flow as paragraphs. Every flow that edits here can leave has a
/// reading.
pub fn paragraphs(base: &Document, raw: &State) -> Result<Vec<Paragraph>, SessionError> {
    let shape = Arc::new(tables::Shape::build(base, raw)?);
    let store = PositionState::new(raw.clone(), shape);
    Ok(group(&store.reading()?)?
        .into_iter()
        .map(|p| p.paragraph)
        .collect())
}

/// Where the content of a view's nodes is in the flow. A range is the unit
/// its first character is, and how many characters it holds.
#[derive(Clone, Default)]
struct Places {
    texts: HashMap<NodeId, (usize, usize)>,
    runs: HashMap<NodeId, (usize, usize)>,
    items: HashMap<NodeId, (usize, usize)>,
    /// A paragraph's text, which follows its mark.
    paragraphs: HashMap<NodeId, (usize, usize)>,
}

/// The document a flow reads as, and where its content is in the flow.
#[derive(Clone)]
pub struct View {
    pub document: Document,
    places: Places,
    lists: HashMap<String, ListBasis>,
    comments: annotations::Comments,
    state: PositionState,
    pub changes: Option<Vec<crate::Splice>>,
    pub resolutions: Vec<Resolution>,
}

/// A state read on a table shape, which says which strands are shown. A
/// position is a count of shown units, so only this type answers for one.
#[derive(Clone)]
pub struct PositionState {
    state: State,
    shape: Arc<tables::Shape>,
}

impl std::ops::Deref for PositionState {
    type Target = State;
    fn deref(&self) -> &State {
        &self.state
    }
}

type Flowed<T> = Result<T, crate::flow::FlowError>;

impl PositionState {
    fn new(state: State, shape: Arc<tables::Shape>) -> Self {
        let mut positions = PositionState { state, shape };
        positions.shape.place_cells(&mut positions.state);
        positions
    }
    /// Reads the same state on the shape of a view built from it.
    fn reshape(&mut self, shape: &Arc<tables::Shape>) {
        self.shape = shape.clone();
        self.shape.place_cells(&mut self.state);
    }
    pub fn raw_at(&self, at: usize) -> Flowed<(String, u32)> {
        self.state.raw_at(&*self.shape, at)
    }
    pub fn unit_at(&self, key: &str, raw: u32) -> Option<usize> {
        self.state.unit_at(&*self.shape, key, raw)
    }
    fn visible(&self, strand: &Strand) -> bool {
        self.state.visible(&*self.shape, strand)
    }
    fn prefix<'a>(&self, strand: &'a Strand) -> Option<&'a Slot> {
        self.state.prefix(&*self.shape, strand)
    }
    fn strand_len(&self, strand: &Strand) -> usize {
        self.state.strand_len(&*self.shape, strand)
    }
    fn slots(&self) -> impl Iterator<Item = &Slot> {
        self.state.slots(&*self.shape)
    }
    fn reading(&self) -> Flowed<Reading> {
        self.state.reading(&*self.shape)
    }
    fn anchor(&self, at: usize) -> Flowed<(String, usize)> {
        self.state.anchor(&*self.shape, at)
    }
    fn position(&self, key: &str, local: usize) -> Flowed<usize> {
        self.state.position(&*self.shape, key, local)
    }
    fn insert(&mut self, at: usize, text: &str, stated: &[Stated]) -> Flowed<()> {
        self.state.insert(&*self.shape, at, text, stated, BIRTH)
    }
    fn item_stated(&mut self, at: usize, kind: &str, name: &str, stated: &[Stated]) -> Flowed<()> {
        self.state
            .insert_at(&*self.shape, at, Insert::Item { kind, name }, stated, BIRTH)
    }
    fn delete(&mut self, at: usize, len: usize) -> Flowed<()> {
        self.state.delete(&*self.shape, at, len)
    }
    fn range(&mut self, at: usize, len: usize, stated: Stated) -> Flowed<()> {
        self.state.range(&*self.shape, at, len, stated)
    }
}

impl View {
    pub fn position_state(&self) -> &PositionState {
        &self.state
    }
    fn shape(&self) -> &Arc<tables::Shape> {
        &self.state.shape
    }
    /// Where a point of the view is in the flow: the unit content put
    /// there would come before.
    pub fn position(&self, point: &Point) -> Result<usize, &'static str> {
        let places = &self.places;
        let content = |id: &NodeId| {
            places
                .texts
                .get(id)
                .or(places.items.get(id))
                .or(places.runs.get(id))
                .copied()
        };
        let found = match point {
            Point::Text { node, offset } => (places.texts.get(node).copied())
                .filter(|(_, len)| offset <= len)
                .map(|(first, _)| first + offset),
            Point::Before { before } => (content(before).map(|(first, _)| first))
                // Before a paragraph is before its mark.
                .or_else(|| {
                    places
                        .paragraphs
                        .get(before)
                        .map(|(first, _)| first.saturating_sub(1))
                }),
            Point::After { after } => (content(after).or(places.paragraphs.get(after).copied()))
                .map(|(first, len)| first + len),
            Point::Start { start } => {
                (places.runs.get(start).or(places.paragraphs.get(start))).map(|(first, _)| *first)
            }
            Point::End { end } => (places.runs.get(end).or(places.paragraphs.get(end)))
                .map(|(first, len)| first + len),
        };
        found.ok_or("names a place this path does not hold")
    }

    fn selected_ranges(
        &self,
        from: &Point,
        to: &Point,
        paragraphs: &[NodeId],
        command: &'static str,
    ) -> Result<Vec<std::ops::Range<usize>>, SessionError> {
        let unsupported = |reason| SessionError::Unsupported { command, reason };
        for point in [from, to] {
            let loc = self.document.locate(*point)?;
            self.document
                .check_end(loc, self.document.paragraph_at(loc)?)?;
        }
        let from = self.position(from).map_err(unsupported)?;
        let to = self.position(to).map_err(unsupported)?;
        self.paragraph_stretches(from, to, paragraphs)
            .ok_or_else(|| unsupported("the selection is outside its paragraph"))
    }

    /// The part of each listed paragraph that lies between two positions.
    /// The paragraphs are in the order they are shown, which storage order
    /// does not always follow. None if a position is outside its paragraph.
    fn paragraph_stretches(
        &self,
        from: usize,
        to: usize,
        paragraphs: &[NodeId],
    ) -> Option<Vec<std::ops::Range<usize>>> {
        let mut ranges = Vec::new();
        for (i, paragraph) in paragraphs.iter().enumerate() {
            let Some(&(at, len)) = self.places.paragraphs.get(paragraph) else {
                continue;
            };
            let start = if i == 0 { from } else { at };
            let end = if i + 1 == paragraphs.len() {
                to
            } else {
                at + len
            };
            if start > end || start < at || end > at + len {
                return None;
            }
            if start < end {
                ranges.push(start..end);
            }
        }
        Some(ranges)
    }

    /// Whether a command gives any selected run another value than it has;
    /// `changes` says so for one run. A run with a tracked formatting change
    /// changes whole or not at all: part of it cannot carry the change.
    fn changes_runs(
        &self,
        paragraphs: &[NodeId],
        selection: &[std::ops::Range<usize>],
        changes: impl Fn(NodeId) -> bool,
    ) -> Result<bool, SessionError> {
        let mut any = false;
        let mut selection = selection.iter().peekable();
        for &p in paragraphs {
            let Some(&(first, len)) = self.places.paragraphs.get(&p) else {
                continue;
            };
            let Some(range) = selection.next_if(|r| r.start >= first && r.end <= first + len)
            else {
                continue;
            };
            for run in self.document.descendants(p) {
                let NodeKind::Run(info) = &self.document.n(run).kind else {
                    continue;
                };
                let Some(&(at, len)) = self.places.runs.get(&run) else {
                    continue;
                };
                let part = at.max(range.start)..(at + len).min(range.end);
                if part.is_empty() || !changes(run) {
                    continue;
                }
                if info.format_revision && (part.start != at || part.len() != len) {
                    return Err(crate::EditError::Refused { node: run, reason: "the range starts or ends inside text with a tracked formatting change; select all of it or none of it" }.into());
                }
                any = true;
            }
        }
        Ok(any)
    }

    /// A position in the flow as a visible caret. Boundary facts that the
    /// resolver ignores still occupy raw positions; fall back to the
    /// containing paragraph when there is no displayed leaf at that unit.
    pub fn point_at(&self, at: usize) -> Option<(NodeId, Point)> {
        let (&paragraph, &(first, len)) = self
            .places
            .paragraphs
            .iter()
            .filter(|(_, (first, len))| *first <= at && at <= *first + *len)
            .max_by_key(|(id, (first, _))| (*first, **id))
            .or_else(|| {
                self.places
                    .paragraphs
                    .iter()
                    .filter(|(_, (first, _))| at + 1 == *first)
                    .min_by_key(|(id, _)| **id)
            })?;
        if let Some((&node, &(start, _))) = self
            .places
            .texts
            .iter()
            .filter(|(_, (start, count))| {
                *start >= first && *start <= first + len && *start <= at && at <= *start + *count
            })
            .max_by_key(|(id, (start, _))| (*start, **id))
        {
            return Some((
                paragraph,
                Point::Text {
                    node,
                    offset: at - start,
                },
            ));
        }
        if let Some((&node, &(start, _))) = self
            .places
            .items
            .iter()
            .filter(|(_, (start, count))| {
                *start >= first && *start < first + len && *start <= at && at <= *start + *count
            })
            .max_by_key(|(id, (start, _))| (*start, **id))
        {
            return Some((
                paragraph,
                if at < start + self.places.items[&node].1 {
                    Point::Before { before: node }
                } else {
                    Point::After { after: node }
                },
            ));
        }
        Some((
            paragraph,
            if at <= first {
                Point::Start { start: paragraph }
            } else {
                Point::End { end: paragraph }
            },
        ))
    }
}

/// The document a flow reads as: a copy of `base`, with each paragraph that
/// reads otherwise than the file has it built anew.
pub fn rebuild(
    base: &Document,
    store: &State,
    previous: Option<&Arc<View>>,
) -> Result<Arc<View>, SessionError> {
    if let Some(previous) = previous {
        if let Some(view) = incremental::update(base, store, previous)? {
            return Ok(view);
        }
    }
    Ok(Arc::new(build(base, store)?))
}

fn apply_document_facts(document: &mut Document, store: &State) -> Result<(), SessionError> {
    for (key, fact) in store.facts() {
        if let Some(media) = key.strip_prefix(MEDIA) {
            let name = document.add_media(&fact.bytes).map_err(unbuilt)?;
            if name != media {
                return Err(corrupt("media content does not match its name".into()));
            }
        } else if key == UPDATE_FIELDS {
            document.settings.update_fields = serde_json::from_slice(&fact.bytes)
                .map_err(|_| corrupt("invalid field setting".into()))?;
        }
    }
    apply_node_facts(document, store, None)
}

/// States what facts say of source nodes: of all of them, or of `only`.
fn apply_node_facts(
    document: &mut Document,
    store: &State,
    only: Option<&HashSet<NodeId>>,
) -> Result<(), SessionError> {
    let wanted = |id: NodeId| only.is_none_or(|only| only.contains(&id));
    for (key, fact) in store.facts() {
        if let Some(source) = key.strip_prefix(CONTROL) {
            let value = serde_json::from_slice(&fact.bytes)
                .map_err(|_| corrupt("invalid control value".into()))?;
            let control = node(source)?;
            // A fill of a removed control stays stored and shows again when
            // the removal is undone.
            if wanted(control) {
                document.compose_control(control, &value).map_err(unbuilt)?;
            }
        } else if let Some(source) = key.strip_prefix(FIELD) {
            let values: Vec<(NodeId, String)> = serde_json::from_slice(&fact.bytes)
                .map_err(|_| corrupt("invalid field result".into()))?;
            let _ = node(source)?;
            for (id, text) in values.into_iter().filter(|(id, _)| wanted(*id)) {
                document.compose_text(id, &text).map_err(unbuilt)?;
            }
        }
    }
    Ok(())
}

/// Applies what a paragraph's mark states about it, then writes its inline
/// content. The full build and the rebuild of one strand share this.
fn write_paragraph(
    inlines: &mut inlines::InlineWriter,
    target: &mut inlines::Target,
    list_numbers: &HashMap<String, String>,
    (parent, node): (NodeId, NodeId),
    located: &Located,
    mark: Option<&Unit>,
    next: Option<&Paragraph>,
) -> Result<(), SessionError> {
    let document = &mut *target.document;
    if let Some(mark) = mark {
        if let Some(style) = mark.one(STYLE) {
            let properties = document.styled(node, style);
            document
                .compose_paragraph_properties(node, &properties)
                .map_err(unbuilt)?;
        }
        if let Some(numbering) = mark.one(NUMBERING) {
            let state = list_state(numbering)?;
            let number = match state {
                None => None,
                Some(state) => Some(NumRef {
                    num_id: match state.basis {
                        ListBasis::Source(n) => n,
                        ListBasis::Created { identity, .. } => list_numbers
                            .get(&identity)
                            .ok_or_else(|| unbuilt("a list was not defined"))?
                            .clone(),
                    },
                    level: state.level,
                }),
            };
            let properties = document.numbered(node, number.as_ref());
            document
                .compose_paragraph_properties(node, &properties)
                .map_err(unbuilt)?;
        }
    }
    target
        .places
        .paragraphs
        .insert(node, (located.first, located.paragraph.text.len()));
    inlines.write(target, parent, node, located)?;
    // A stored mark starts the next paragraph. Word writes that revision
    // on the end mark of the preceding paragraph in the same region.
    if let Some(next) = next.filter(|p| p.revision.is_some() || p.deletion.is_some()) {
        target
            .document
            .revision_mark(node, next.revision.as_ref(), next.deletion.as_ref())
            .map_err(unbuilt)?;
    }
    Ok(())
}

/// What a created cell's first paragraph takes from the cell it copies,
/// if a fact states it.
fn stated_cell_basis(store: &State, key: &str) -> Result<Option<tables::CellBasis>, SessionError> {
    (store.facts().get(&format!("{CELL_BASIS}{key}")))
        .map(|fact| serde_json::from_slice(&fact.bytes))
        .transpose()
        .map_err(|_| corrupt("invalid cell basis".into()))
}

/// Gives a created cell's first paragraph the properties and the list its
/// basis states. The full build and the rebuild of one strand share this.
fn write_cell_basis(
    document: &mut Document,
    node: NodeId,
    basis: &tables::CellBasis,
    list_numbers: &HashMap<String, String>,
) -> Result<(), SessionError> {
    document
        .compose_paragraph_properties(node, &basis.properties)
        .map_err(unbuilt)?;
    if let Some(state) = &basis.numbering {
        let number = NumRef {
            num_id: match &state.basis {
                ListBasis::Source(n) => n.clone(),
                ListBasis::Created { identity, .. } => list_numbers[identity].clone(),
            },
            level: state.level,
        };
        let props = document.numbered(node, Some(&number));
        document
            .compose_paragraph_properties(node, &props)
            .map_err(unbuilt)?;
    }
    Ok(())
}

fn defined_lists(
    units: &Reading,
    located: &[Located],
) -> Result<BTreeMap<String, ListKind>, SessionError> {
    let mut definitions = BTreeMap::new();
    for p in located {
        if let Some(value) = p
            .first
            .checked_sub(1)
            .and_then(|i| units.get(i))
            .and_then(|u| u.one(NUMBERING))
        {
            if let Some(ListState {
                basis: ListBasis::Created { identity, kind },
                ..
            }) = list_state(value)?
            {
                definitions.insert(identity, kind);
            }
        }
    }
    Ok(definitions)
}

pub fn build(base: &Document, store: &State) -> Result<View, SessionError> {
    let mut shape = tables::Shape::build(base, store)?;
    let mut document = shape.document.take().unwrap_or_else(|| base.clone());
    let shape = Arc::new(shape);
    let hydrated = PositionState::new(store.clone(), shape.clone());
    let store = &hydrated;
    let mut regions = regions(base);
    let first_scope = regions.first().map(|r| r.paragraphs[0].to_string());
    regions.retain(|r| tables::attached(&document, r.parent));
    for (p, _) in shape.cells.values() {
        regions.push(Region {
            parent: document.node(*p).unwrap().parent.unwrap(),
            paragraphs: vec![*p],
        });
    }
    let cell_scopes: HashMap<_, _> = shape
        .cells
        .iter()
        .map(|(key, (p, _))| (*p, key.clone()))
        .collect();
    let scopes: HashMap<_, _> = regions
        .iter()
        .map(|r| {
            (
                cell_scopes
                    .get(&r.paragraphs[0])
                    .cloned()
                    .unwrap_or_else(|| r.paragraphs[0].to_string()),
                r,
            )
        })
        .collect();
    let parent_children: HashMap<_, _> = regions
        .iter()
        .map(|r| (r.parent, document.node(r.parent).unwrap().children.clone()))
        .collect();
    let seeded: HashMap<_, _> = body(base)?.into_iter().collect();
    let units = store.reading()?;
    // A preserved-only document has no editable region. Do not invent a
    // paragraph for it; facts (such as control fills) still apply below.
    let located = if units.is_empty() && regions.is_empty() {
        Vec::new()
    } else {
        group(&units)?
    };
    let read: Vec<_> = located.iter().map(|p| &p.paragraph).collect();
    // A source run stays a node only where its own strand holds the text, so
    // that one strand can be rebuilt without reading the others.
    let mut owners = Vec::new();
    let mut at = 0;
    for strand in store.strands().iter().filter(|s| store.visible(s)) {
        let paragraph = strand.key.parse::<NodeId>().ok();
        owners.push((at, paragraph.filter(|id| seeded.contains_key(id))));
        at += store.strand_len(strand);
    }
    let mut inlines = inlines::InlineWriter::new(base, &seeded, &located, owners);
    apply_document_facts(&mut document, store)?;
    let mut definitions = defined_lists(&units, &located)?;
    let mut cell_basis = HashMap::new();
    for key in shape.cells.keys() {
        if let Some(basis) = stated_cell_basis(store, key)? {
            if let Some(ListState {
                basis: ListBasis::Created { identity, kind },
                ..
            }) = &basis.numbering
            {
                definitions.insert(identity.clone(), *kind);
            }
            cell_basis.insert(key.clone(), basis);
        }
    }
    let mut lists = HashMap::new();
    let mut list_numbers = HashMap::new();
    for (identity, kind) in definitions {
        let number = document.new_list(kind).map_err(unbuilt)?;
        lists.insert(
            number.clone(),
            ListBasis::Created {
                identity: identity.clone(),
                kind,
            },
        );
        list_numbers.insert(identity, number);
    }
    // The regions are already where they will be shown, so the order of
    // their first paragraphs is the order of everything placed in them.
    let shown = |document: &Document| -> Vec<Option<usize>> {
        let rank: HashMap<NodeId, usize> = document
            .stories
            .iter()
            .flat_map(|story| document.descendants(story.root))
            .enumerate()
            .map(|(rank, id)| (id, rank))
            .collect();
        located
            .iter()
            .map(|p| {
                let scope = p.scope.as_ref().or(first_scope.as_ref())?;
                rank.get(&scopes.get(scope)?.paragraphs[0]).copied()
            })
            .collect()
    };
    let comments = annotations::Comments::build(&mut document, store, &located, &shown)?;
    let mut places = Places::default();
    let mut orders: BTreeMap<String, Vec<NodeId>> = BTreeMap::new();
    for (index, paragraph) in read.iter().enumerate() {
        let scope = located[index]
            .scope
            .clone()
            .or_else(|| first_scope.clone())
            .ok_or_else(|| corrupt("content has no source region".into()))?;
        let Some(region) = scopes.get(&scope) else {
            continue;
        }; // A deliberately removed container retains its facts.
        let unit = located[index].first;
        let like = paragraph.mark.like();
        // A copied cell can inherit a paragraph that was read-only in the file.
        if like.is_some_and(|id| {
            !matches!(base.node(id).map(|n| &n.kind), Some(NodeKind::Paragraph(_)))
        }) {
            return Err(unbuilt(format!("{like:?} is not a paragraph of the file")));
        }
        let node = match &paragraph.mark {
            Mark::Base(id) => *id,
            Mark::Cell { key, .. } => shape.cells[key].0,
            Mark::Made { .. } | Mark::Lost => {
                (document.loose_paragraph(region.parent, like)).map_err(unbuilt)?
            }
        };
        if let Mark::Cell { key, .. } = &paragraph.mark {
            if let Some(basis) = cell_basis.get(key) {
                write_cell_basis(&mut document, node, basis, &list_numbers)?;
            }
        }
        orders.entry(scope).or_default().push(node);
        let mark = (paragraph.mark != Mark::Lost).then(|| &units[unit - 1]);
        write_paragraph(
            &mut inlines,
            &mut inlines::Target {
                document: &mut document,
                places: &mut places,
                comments: &comments,
            },
            &list_numbers,
            (region.parent, node),
            &located[index],
            mark,
            located
                .get(index + 1)
                .filter(|p| p.scope == located[index].scope)
                .map(|p| &p.paragraph),
        )?;
    }
    // Substitute each region in its source container, keeping intervening
    // tables and unknown blocks exactly where they were.
    let parents: BTreeMap<_, Vec<_>> = regions.iter().fold(BTreeMap::new(), |mut all, r| {
        all.entry(r.parent).or_default().push(r);
        all
    });
    for (parent, regions) in parents {
        let starts: HashSet<_> = regions.iter().map(|r| r.paragraphs[0]).collect();
        let members: HashSet<_> = regions
            .iter()
            .flat_map(|r| r.paragraphs.iter().copied())
            .collect();
        let mut children = Vec::new();
        for &id in &parent_children[&parent] {
            if starts.contains(&id) {
                let key = cell_scopes
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| id.to_string());
                children.extend(orders.remove(&key).unwrap_or_default());
            } else if !members.contains(&id) {
                children.push(id);
            }
        }
        document.arrange(parent, &children).map_err(unbuilt)?;
    }
    // A run is formatted once it is in the document, where its paragraph's
    // style counts.
    for (run, format) in inlines.formats {
        document.format_run(run, format).map_err(unbuilt)?;
    }
    Ok(View {
        document,
        places,
        lists,
        comments,
        resolutions: shape.ignored.clone(),
        state: hydrated,
        changes: None,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    Open(#[from] crate::OpenError),
    #[error(transparent)]
    Edit(#[from] crate::EditError),
    #[error(transparent)]
    Export(#[from] crate::ExportError),
    #[error(transparent)]
    Flow(#[from] crate::flow::FlowError),
    #[error("{command} {reason}")]
    Unsupported {
        command: &'static str,
        reason: &'static str,
    },
    #[error("flow {flow}: {reason}")]
    Corrupt { flow: String, reason: String },
    #[error("flow {flow}: its paragraphs could not be built: {reason}")]
    Unbuilt { flow: String, reason: String },
}

enum InsertionBasis {
    Unit(usize),
    EmptyRun { run: NodeId, leaf: Option<NodeId> },
}

struct Working {
    previous: Option<Arc<View>>,
    base: Arc<Document>,
    positions: PositionState,
    view: Option<Arc<View>>,
    /// How many views this batch has built after the caller's.
    rebuilds: usize,
}

/// A validated batch. The store adapter commits these exact positioned
/// writes; it does not run the commands or infer changes from the tree.
pub struct Prepared {
    pub state: State,
    pub view: Arc<View>,
}

pub fn prepare(
    base: Arc<Document>,
    state: State,
    view: Option<Arc<View>>,
    commands: &[Command],
    media: &[&[u8]],
) -> Result<Prepared, SessionError> {
    let mut view = match view {
        Some(view) => view,
        None => Arc::new(build(&base, &state)?),
    };
    let mut positions = PositionState::new(state, view.shape().clone());
    for bytes in media {
        let name = add_media(&base, &mut positions.state, bytes)?;
        if !commands.iter().any(
            |command| matches!(command, Command::InsertPicture { media, .. } if media == &name),
        ) {
            return Err(SessionError::Unsupported {
                command: "supply media",
                reason: "the batch has no picture insertion naming this media",
            });
        }
        // Commands name nodes in the caller's current view. Add media to
        // that address space privately; rebuilding here would renumber it.
        Arc::make_mut(&mut view).document.add_media(bytes)?;
    }
    let mut work = Working {
        previous: None,
        base,
        positions,
        view: Some(view),
        rebuilds: 0,
    };
    for command in commands {
        work.write(command)?;
    }
    work.built()?;
    let mut view = work.view.expect("built");
    // A view's changes are relative to the view it was built from. After a
    // second build that is no longer the view the caller drew.
    if work.rebuilds > 1 && view.changes.is_some() {
        Arc::make_mut(&mut view).changes = None;
    }
    Ok(Prepared {
        state: work.positions.state.into_stored(),
        view,
    })
}

/// Serialize from the base and final facts so all sessions use the same
/// materialization regardless of which paragraphs their views have cached.
pub fn export(base: &Document, state: &State) -> Result<Vec<u8>, SessionError> {
    Ok(build(base, state)?.document.export()?)
}

/// Headless editing through the same command preparation and builder as
/// collaboration. Its state and undo history have no Yrs dependency.
pub struct Editor {
    base: Arc<Document>,
    state: State,
    view: std::sync::OnceLock<Arc<View>>,
    past: Vec<StateUndo>,
    future: Vec<StateUndo>,
}

impl Editor {
    pub fn open(bytes: &[u8]) -> Result<Self, SessionError> {
        let base = Arc::new(Document::open(bytes)?);
        let state = seed(0, &base)?;
        Ok(Self {
            base,
            state,
            view: Default::default(),
            past: Vec::new(),
            future: Vec::new(),
        })
    }
    /// Validates image bytes and names them without changing editing state.
    pub fn media_name(&self, bytes: &[u8]) -> Result<String, SessionError> {
        Ok(self.base.media_name(bytes)?)
    }
    pub fn view(&self) -> Result<&Document, SessionError> {
        let view = match self.view.get() {
            Some(view) => view,
            None => {
                let view = Arc::new(build(&self.base, &self.state)?);
                self.view.get_or_init(|| view)
            }
        };
        Ok(&view.document)
    }
    pub fn export(&self) -> Result<Vec<u8>, SessionError> {
        export(&self.base, &self.state)
    }
    pub fn apply(&mut self, command: &Command) -> Result<(), SessionError> {
        self.apply_all(std::slice::from_ref(command))
    }
    pub fn apply_all(&mut self, commands: &[Command]) -> Result<(), SessionError> {
        self.apply_all_with_media(commands, &[])
    }
    /// Commits image bytes only if the entire command batch succeeds.
    pub fn apply_all_with_media(
        &mut self,
        commands: &[Command],
        media: &[&[u8]],
    ) -> Result<(), SessionError> {
        let mut prepared = prepare(
            self.base.clone(),
            self.state.clone(),
            self.view.get().cloned(),
            commands,
            media,
        )?;
        let writes = prepared.state.take_writes();
        if !writes.is_empty() {
            self.past
                .push(StateUndo::between(&self.state, &prepared.state, &writes));
            self.state = prepared.state;
            self.future.clear();
        }
        self.view = prepared.view.into();
        Ok(())
    }
    pub fn undo(&mut self) -> bool {
        let Some(mut previous) = self.past.pop() else {
            return false;
        };
        self.view.take();
        previous.swap(&mut self.state);
        self.future.push(previous);
        true
    }
    pub fn redo(&mut self) -> bool {
        let Some(mut next) = self.future.pop() else {
            return false;
        };
        self.view.take();
        next.swap(&mut self.state);
        self.past.push(next);
        true
    }
}
fn value(set: Option<bool>) -> Option<&'static str> {
    set.map(|on| if on { "1" } else { "0" })
}

impl Working {
    fn built(&mut self) -> Result<&View, SessionError> {
        if self.view.is_none() {
            let view = rebuild(&self.base, &self.positions, self.previous.as_ref())?;
            self.positions.reshape(view.shape());
            self.view = Some(view);
            self.previous = None;
            self.rebuilds += 1;
        }
        Ok(self.view.as_ref().expect("just built"))
    }

    fn position(&mut self, point: &Point) -> Result<usize, &'static str> {
        self.built()
            .map_err(|_| "names a view that cannot be built")?
            .position(point)
    }

    fn split_position(&mut self, at: &Point) -> Result<usize, SessionError> {
        let at = &self.outside_fields(at, true)?;
        let view = self.built()?;
        let document = &view.document;
        let loc = document.edge(document.locate(*at)?)?;
        let paragraph = document.paragraph_at(loc)?;
        document.check_source_mark(paragraph)?;
        let point = match document.split_edge(paragraph, loc) {
            crate::structure::Loc::Child(id, index) if id == paragraph && index == 0 => {
                Point::Start { start: id }
            }
            crate::structure::Loc::Child(id, index)
                if id == paragraph && index == document.n(id).children.len() =>
            {
                Point::End { end: id }
            }
            _ => *at,
        };
        view.position(&point)
            .map_err(|reason| SessionError::Unsupported {
                command: "split_paragraph",
                reason,
            })
    }

    /// The point outside every field, named as this view holds it. A field
    /// character in a run that holds more than field characters is part of
    /// an item that is its run.
    fn outside_fields(&mut self, point: &Point, forward: bool) -> Result<Point, SessionError> {
        let view = self.built()?;
        let document = &view.document;
        let outside = document.outside_fields(*point, forward)?;
        let (marker, first) = match outside {
            _ if outside == *point => return Ok(outside),
            Point::Before { before } => (before, true),
            Point::After { after } => (after, false),
            _ => return Ok(outside),
        };
        if view.places.items.contains_key(&marker) {
            return Ok(outside);
        }
        let run = document
            .n(marker)
            .parent
            .expect("a field character is in a run");
        let children = &document.n(run).children;
        let edge = if first {
            children.first()
        } else {
            children.last()
        };
        if edge != Some(&marker) {
            return Err(crate::EditError::Refused {
                node: run,
                reason: "inside a field result; the field regenerates it",
            }
            .into());
        }
        Ok(if first {
            Point::Before { before: run }
        } else {
            Point::After { after: run }
        })
    }

    /// The point's position and, when it names text or a run, the character
    /// supplying its insertion properties. At a run boundary the target
    /// still determines formatting; the preceding run does not take over.
    fn insertion(
        &mut self,
        point: &Point,
    ) -> Result<(usize, Option<InsertionBasis>), &'static str> {
        let at = self.position(point)?;
        let places = &self
            .built()
            .map_err(|_| "names a view that cannot be built")?
            .places;
        let id = match point {
            Point::Text { node, .. } => node,
            Point::Before { before } => before,
            Point::After { after } => after,
            Point::Start { start } => start,
            Point::End { end } => end,
        };
        let basis = places
            .texts
            .get(id)
            .or(places.runs.get(id))
            .or(places.items.get(id))
            .copied();
        let basis = match basis {
            Some((first, len)) if len > 0 => {
                Some(InsertionBasis::Unit(at.clamp(first, first + len - 1)))
            }
            Some(_) => {
                let leaf = matches!(point, Point::Text { .. }).then_some(*id);
                let node = self.base.node(*id);
                let run = node.and_then(|n| {
                    if matches!(n.kind, NodeKind::Run(_)) {
                        Some(n.id)
                    } else {
                        n.parent
                    }
                });
                run.map(|run| InsertionBasis::EmptyRun { run, leaf })
            }
            None => None,
        };
        Ok((at, basis))
    }

    fn check_replacement(
        &mut self,
        command: &'static str,
        units: &Reading,
        at: usize,
        len: usize,
        tracked: bool,
    ) -> Result<(), SessionError> {
        if at == 0 && matches!(units.first().map(mark_of), Some(Ok(Some(_)))) {
            let reason = "names a place before the first paragraph";
            return Err(SessionError::Unsupported { command, reason });
        }
        if units
            .range(at..at + len)
            .any(|u| matches!(&u.content, Content::Item { kind, .. } if kind == SCOPE))
        {
            return Err(SessionError::Unsupported {
                command,
                reason: "cannot replace across a table, block or story boundary",
            });
        }
        let mut items = Vec::new();
        for (offset, unit) in units.range(at..at + len).enumerate() {
            if let Some(mark) = mark_of(unit)? {
                if let Some(like) = mark.like() {
                    self.base.check_source_mark(like)?;
                }
                // This sequence entry begins the next paragraph, but in
                // DOCX the boundary belongs to the paragraph before it.
                let view = self.built()?;
                if let Some((&previous, _)) = view
                    .places
                    .paragraphs
                    .iter()
                    .find(|(_, (first, len))| *first + *len == at + offset)
                {
                    view.document.check_source_mark(previous)?;
                }
            }
            if let Some(Inline::Source { node, .. }) = inline_of(unit)? {
                let source = self.base.n(node);
                // Empty editable XML is carried whole for round-trip fidelity.
                // It contains no protected text to validate when replacing a range.
                if source.support == Support::Editable
                    && (matches!(&source.kind, NodeKind::Text { text } if text.is_empty())
                        || (matches!(source.kind, NodeKind::Run(_) | NodeKind::Hyperlink(_))
                            && source.children.is_empty()))
                {
                    continue;
                }
                // A preserved container is projected whole. Accepting a tracked
                // deletion over it would record a mark the writer cannot show.
                if tracked {
                    match &self.base.n(node).kind {
                        NodeKind::Tab
                        | NodeKind::Break { .. }
                        | NodeKind::Drawing(_)
                        | NodeKind::Marker { .. } => {}
                        NodeKind::Revision(revision) if revision.change == "del" => {}
                        _ => {
                            return Err(SessionError::Unsupported {
                                command,
                                reason: "cannot track deletion of a preserved source container",
                            })
                        }
                    }
                }
                items.push(node);
            }
        }
        if !items.is_empty() {
            let document = &self.built()?.document;
            // Removing an intervening block also removes its paired bookmark
            // marker. The source entry can still occur in the selected flow.
            items.retain(|&id| {
                !matches!(
                    document.attached(id),
                    Err(crate::EditError::Detached { .. })
                )
            });
            document.check_range_items(&items)?;
        }
        Ok(())
    }

    /// Puts `text` before unit `at` and deletes `len` units after it, as
    /// one change. A line feed in `text` starts a paragraph.
    fn replace(
        &mut self,
        command: &'static str,
        at: usize,
        len: usize,
        text: &str,
        basis_at: Option<InsertionBasis>,
        track: Option<&Track>,
    ) -> Result<(), SessionError> {
        // Refused before anything is written, by the engine's own rule, so
        // no replica is left with text a view cannot hold.
        if let Some(track) = track {
            crate::revisions::check_track(track)?;
        }
        let revision =
            track.map(|track| serde_json::to_string(track).expect("Track contains only strings"));
        let lines: Vec<&str> = text.split('\n').collect();
        lines.iter().try_for_each(|line| check_text(line))?;
        let units = self.positions.reading()?;
        let empty_leaf = match basis_at {
            Some(InsertionBasis::EmptyRun { leaf, .. }) => leaf,
            _ => None,
        };
        // Filling an empty source text replaces its placeholder with characters
        // carrying that leaf's XML attributes. Undo restores the original leaf.
        let len = if len == 0 && empty_leaf.is_some() && !text.is_empty() && track.is_none() {
            1
        } else {
            len
        };
        self.check_replacement(command, &units, at, len, track.is_some())?;
        // Use the named text/run's properties when there is a target.
        // Otherwise use the adjoining text, or the paragraph in an empty
        // paragraph. A derived paragraph with no template is explicitly bare.
        let named_run = match basis_at {
            Some(InsertionBasis::EmptyRun { run, .. }) => Some(run),
            _ => None,
        };
        let named_unit = match basis_at {
            Some(InsertionBasis::Unit(at)) => Some(at),
            _ => None,
        };
        let beside = [named_unit, at.checked_sub(1), Some(at)]
            .into_iter()
            .flatten();
        let mut beside = beside.filter_map(|unit| units.get(unit));
        let char = beside
            .find_map(|unit| inline_of(unit).transpose())
            .transpose()?;
        let insert_at = if track.is_some() { at + len } else { at };
        let mark = units
            .range(..insert_at)
            .rev()
            .find_map(|unit| mark_of(unit).transpose());
        let like = mark.transpose()?.and_then(|m| m.like());
        let mark_unit = units
            .range(..insert_at)
            .rev()
            .find(|u| matches!(mark_of(u), Ok(Some(_))));
        let mut inherited_style = mark_unit.and_then(|u| u.one(STYLE)).map(str::to_string);
        let inherited_numbering = mark_unit.and_then(|u| u.one(NUMBERING)).map(str::to_string);
        if lines.len() > 1 {
            let view = self.built()?;
            if let Some((paragraph, _)) = view.point_at(insert_at) {
                let &(first, len) = &view.places.paragraphs[&paragraph];
                if insert_at == first + len {
                    if let NodeKind::Paragraph(info) = &view.document.node(paragraph).unwrap().kind
                    {
                        if let Some(next) = view.document.next_style(info) {
                            inherited_style = Some(next);
                        }
                    }
                }
            }
            if let Some(like) = like {
                self.base.check_source_mark(like)?;
            }
        }
        let link = if named_run.is_some() {
            None
        } else {
            char.as_ref().and_then(Inline::link).map(str::to_string)
        };
        let source_text = match &char {
            Some(Inline::Char(c)) if named_run.is_none() => c.source_text,
            _ => empty_leaf,
        };
        let (basis, format) = if let Some(run) = named_run {
            (Some(run), Format::default())
        } else {
            match (char, like) {
                (Some(inline), _) => (inline.basis(), inline.format()),
                (None, like) => (like, Format::default()),
            }
        };
        let basis = basis
            .map(|id| source_text.map_or_else(|| id.to_string(), |leaf| format!("{id}/{leaf}")))
            .unwrap_or_else(|| BARE.to_owned());
        let mut next = insert_at;
        if let Some(track) = track.filter(|_| len > 0) {
            let mut before = BTreeMap::new();
            for slot in self.positions.slots() {
                if let crate::flow::Slot::Open(_, span) = slot {
                    if span.key == BIRTH {
                        let (client, clock) = birth(
                            span.value
                                .as_deref()
                                .ok_or_else(|| corrupt("missing insertion generation".into()))?,
                        )?;
                        before
                            .entry(client)
                            .and_modify(|seen: &mut u32| *seen = (*seen).max(clock))
                            .or_insert(clock);
                    }
                }
            }
            let mut cancel = Vec::new();
            let mut mark: Vec<(usize, usize)> = Vec::new();
            for (offset, unit) in units.range(at..at + len).enumerate() {
                if deletion_of(unit)?.is_some() {
                    continue;
                }
                if insertion_of(unit)?.is_some_and(|by| by.author == track.author) {
                    cancel.push(at + offset);
                } else {
                    let at = at + offset;
                    match mark.last_mut() {
                        Some((first, len)) if *first + *len == at => *len += 1,
                        _ => mark.push((at, 1)),
                    }
                }
            }
            if !mark.is_empty() {
                let value = serde_json::to_string(&Deletion {
                    by: track.clone(),
                    before,
                })
                .expect("deletion serializes");
                for (at, len) in mark {
                    self.positions.range(at, len, (DELETION, Some(&value)))?;
                }
            }
            next -= cancel.len();
            for at in cancel.into_iter().rev() {
                self.positions.delete(at, 1)?;
            }
        }
        let (client, clock) = self.positions.next_identity();
        let born = format!("{client}.{clock}");
        let stated = [
            (BASIS, Some(basis.as_str())),
            (LINK, link.as_deref()),
            (BIRTH, Some(born.as_str())),
            (BOLD, value(format.bold)),
            (ITALIC, value(format.italic)),
            (UNDERLINE, value(format.underline)),
            (INSERTION, revision.as_deref()),
        ];
        // The new content goes in before the old comes out, so it lands
        // where a text would hold it had each change been made in turn.
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                // A paragraph that Enter makes is like the one it was in.
                let name = made(like, self.positions.next_identity());
                self.positions.item_stated(
                    next,
                    MARK,
                    &name,
                    &[
                        (INSERTION, revision.as_deref()),
                        (BIRTH, Some(&born)),
                        (STYLE, inherited_style.as_deref()),
                        (NUMBERING, inherited_numbering.as_deref()),
                    ],
                )?;
                next += 1;
            }
            if !line.is_empty() {
                self.positions.insert(next, line, &stated)?;
                next += line.chars().count();
            }
        }
        if track.is_none() {
            self.positions.delete(next, len)?;
        }
        Ok(())
    }

    fn replace_object(
        &mut self,
        at: &Point,
        to: Option<&Point>,
        kind: NewInline,
    ) -> Result<(), SessionError> {
        let (start, end, named) = self.replacement_span(at, to.unwrap_or(at), false)?;
        self.check_replacement(
            "insert inline",
            &self.positions.reading()?,
            start,
            end - start,
            false,
        )?;
        self.insert_object_at(start, named, kind)?;
        self.positions.delete(start + 1, end - start)?;
        Ok(())
    }

    fn insert_object_at(
        &mut self,
        at: usize,
        named: Option<InsertionBasis>,
        kind: NewInline,
    ) -> Result<(), SessionError> {
        let units = self.positions.reading()?;
        let named_run = match named {
            Some(InsertionBasis::EmptyRun { run, .. }) => Some(run),
            _ => None,
        };
        let named_unit = match named {
            Some(InsertionBasis::Unit(i)) => Some(i),
            _ => None,
        };
        let inline = [named_unit, at.checked_sub(1), Some(at)]
            .into_iter()
            .flatten()
            .filter_map(|i| units.get(i))
            .find_map(|u| inline_of(u).transpose())
            .transpose()?;
        let like = units
            .range(..at)
            .rev()
            .find_map(|u| mark_of(u).transpose())
            .transpose()?
            .and_then(|m| m.like());
        let basis = named_run
            .or_else(|| inline.as_ref().and_then(Inline::basis))
            .or(like)
            .map(|id| id.to_string())
            .unwrap_or_default();
        let format = if named_run.is_some() {
            Format::default()
        } else {
            inline.as_ref().map_or(Format::default(), Inline::format)
        };
        let link = if named_run.is_some() {
            None
        } else {
            inline.as_ref().and_then(Inline::link)
        };
        let (client, clock) = self.positions.next_identity();
        let identity = format!("{client}.{clock}");
        let object = serde_json::to_string(&InlineObject {
            identity: identity.clone(),
            kind,
        })
        .expect("inline serializes");
        self.positions.item_stated(
            at,
            NEW_INLINE,
            &object,
            &[
                (BASIS, Some(&basis)),
                (BIRTH, Some(&identity)),
                (INSERTION, None),
                (LINK, link),
                (BOLD, value(format.bold)),
                (ITALIC, value(format.italic)),
                (UNDERLINE, value(format.underline)),
            ],
        )?;
        Ok(())
    }

    fn paragraph_values(
        &mut self,
        values: Vec<(NodeId, &'static str, String)>,
    ) -> Result<(), SessionError> {
        let units = self.positions.reading()?;
        let view = self.built()?;
        let mut writes = Vec::new();
        for (paragraph, key, value) in values {
            let (first, _) = view.places.paragraphs.get(&paragraph).copied().ok_or(
                SessionError::Unsupported {
                    command: "paragraph properties",
                    reason: "names a paragraph outside an editable region",
                },
            )?;
            let mark = first
                .checked_sub(1)
                .filter(|i| matches!(mark_of(&units[*i]), Ok(Some(_))));
            writes.push((first, mark, view.document.like(paragraph), key, value));
        }
        writes.sort_by_key(|(first, ..)| std::cmp::Reverse(*first));
        for (first, mark, like, key, value) in writes {
            let at = match mark {
                Some(at) => at,
                None => {
                    let name = made(like, self.positions.next_identity());
                    let (client, clock) = self.positions.next_identity();
                    let born = format!("{client}.{clock}");
                    self.positions
                        .item_stated(first, MARK, &name, &[(BIRTH, Some(&born))])?;
                    first
                }
            };
            self.positions.range(at, 1, (key, Some(&value)))?;
        }
        Ok(())
    }

    /// Removing complete intervening blocks is part of a range replacement.
    /// Keep the endpoint anchors while structural facts hide those blocks;
    /// then prepare the text edit in the resulting visible sequence.
    fn replacement_span(
        &mut self,
        from: &Point,
        to: &Point,
        tracked: bool,
    ) -> Result<(usize, usize, Option<InsertionBasis>), SessionError> {
        let unsupported = |reason| SessionError::Unsupported {
            command: "replace_range",
            reason,
        };
        // A caret beside a field looks past the field's end first. A range
        // that starts at a result's first character takes the field's start.
        let (from, to) = (
            &self.outside_fields(from, from == to)?,
            &self.outside_fields(to, true)?,
        );
        let (a, basis) = self.insertion(from).map_err(unsupported)?;
        let b = self.position(to).map_err(unsupported)?;
        if a > b {
            return Err(unsupported("names a range that ends before it starts"));
        }
        if !self
            .positions
            .reading()?
            .range(a..b)
            .any(|u| matches!(&u.content,Content::Item{kind,..} if kind==SCOPE))
        {
            return Ok((a, b, basis));
        }
        if tracked {
            return Err(unsupported(
                "cannot track a replacement across a table or block",
            ));
        }
        let view = self.built()?;
        let da = view.document.edge(view.document.locate(*from)?)?;
        let db = view.document.edge(view.document.locate(*to)?)?;
        let pa = view.document.paragraph_at(da)?;
        let pb = view.document.paragraph_at(db)?;
        if view.document.node(pa).unwrap().parent != view.document.node(pb).unwrap().parent {
            return Err(unsupported("cannot replace across stories or table cells"));
        }
        let covered = view.document.check_range(da, db, pa, pb)?;
        let blocks: Vec<_> = covered
            .blocks
            .into_iter()
            .filter(|id| {
                !matches!(
                    view.document.node(*id).unwrap().kind,
                    NodeKind::Paragraph(_)
                )
            })
            .collect();
        if blocks.is_empty() {
            return Err(unsupported("cannot replace across this boundary"));
        }
        let from = self.positions.anchor(a)?;
        let to = self.positions.anchor(b)?;
        let basis_anchor = if let Some(InsertionBasis::Unit(at)) = basis {
            Some(self.positions.anchor(at)?)
        } else {
            None
        };
        for node in blocks {
            let view = self.view.as_ref().unwrap();
            view.shape().write(
                &self.base,
                &view.document,
                &mut self.positions.state,
                &Command::RemoveNode { node },
                &view.lists,
            )?;
        }
        self.previous = self.view.take();
        self.built()?;
        let basis = match basis_anchor {
            Some((key, at)) => Some(InsertionBasis::Unit(self.positions.position(&key, at)?)),
            None => basis,
        };
        Ok((
            self.positions.position(&from.0, from.1)?,
            self.positions.position(&to.0, to.1)?,
            basis,
        ))
    }

    fn field_value(&mut self, field: NodeId, text: &str) -> Result<(), SessionError> {
        if self.base.node(field).is_none() {
            return Err(unbuilt("the field has no source identity"));
        }
        let nodes = self.built()?.document.field_result_nodes(field, text)?;
        let value: Vec<_> = nodes
            .into_iter()
            .enumerate()
            .map(|(i, node)| (node, if i == 0 { text } else { "" }))
            .collect();
        self.positions.state.set_fact(
            format!("{FIELD}{field}"),
            serde_json::to_vec(&value).expect("field serializes"),
            false,
        )?;
        Ok(())
    }

    fn numbering_value(&mut self, number: &NumRef) -> Result<ListState, SessionError> {
        let basis = self
            .built()?
            .lists
            .get(&number.num_id)
            .cloned()
            .unwrap_or_else(|| ListBasis::Source(number.num_id.clone()));
        Ok(ListState {
            basis,
            level: number.level,
        })
    }

    /// Writes one command to the flow, without ending the change.
    fn write(&mut self, command: &Command) -> Result<(), SessionError> {
        let unsupported = |reason: &'static str| SessionError::Unsupported {
            command: command.name(),
            reason,
        };
        match command {
            Command::ReplaceText { node, range, text } => {
                let whole = Point::Text {
                    node: *node,
                    offset: 0,
                };
                let doc = &self.built()?.document;
                doc.attached(*node)?;
                doc.editable_text(*node)?;
                let first = self.position(&whole).map_err(unsupported)?;
                let len = self.built()?.places.texts[node].1;
                let range = range.unwrap_or(TextRange { start: 0, end: len });
                if range.start > range.end || range.end > len {
                    return Err(crate::EditError::Range {
                        node: *node,
                        start: range.start,
                        end: range.end,
                        len,
                    }
                    .into());
                }
                self.replace(
                    command.name(),
                    first + range.start,
                    range.end - range.start,
                    text,
                    if len > 0 {
                        Some(InsertionBasis::Unit(first + range.start.min(len - 1)))
                    } else {
                        self.base.node(*node).and_then(|n| n.parent).map(|run| {
                            InsertionBasis::EmptyRun {
                                run,
                                leaf: Some(*node),
                            }
                        })
                    },
                    None,
                )?;
            }
            Command::InsertText { at, text } => {
                let at = &self.outside_fields(at, true)?;
                let (at, basis) = self.insertion(at).map_err(unsupported)?;
                self.replace(command.name(), at, 0, text, basis, None)?;
            }
            Command::ReplaceRange {
                from,
                to,
                text,
                track,
            } => {
                let (from, to, basis) = if from == to && text == "\n" {
                    let pos = self.split_position(from)?;
                    (pos, pos, None)
                } else {
                    self.replacement_span(from, to, track.is_some())?
                };
                self.replace(command.name(), from, to - from, text, basis, track.as_ref())?;
            }
            Command::AddComment { .. } | Command::ReplyToComment { .. } => self.comment(command)?,
            Command::InsertRow { .. }
            | Command::InsertColumn { .. }
            | Command::RemoveColumn { .. } => {
                self.built()?;
                let view = self.view.as_ref().unwrap();
                view.shape().write(
                    &self.base,
                    &view.document,
                    &mut self.positions.state,
                    command,
                    &view.lists,
                )?;
            }
            Command::RemoveNode { node } => {
                self.built()?;
                if !self.view.as_ref().unwrap().places.items.contains_key(node)
                    && !self.view.as_ref().unwrap().places.runs.contains_key(node)
                {
                    let view = self.view.as_ref().unwrap();
                    view.shape().write(
                        &self.base,
                        &view.document,
                        &mut self.positions.state,
                        command,
                        &view.lists,
                    )?;
                    self.previous = self.view.take();
                    return Ok(());
                }
                let view = self.built()?;
                let nodes = view.document.removal_nodes(*node)?;
                let mut spans = Vec::new();
                for id in nodes {
                    if let Some(span) = view.places.items.get(&id).or(view.places.runs.get(&id)) {
                        spans.push(*span);
                    } else {
                        return Err(unsupported(
                            "names a block whose removal is not on this path yet",
                        ));
                    }
                }
                spans.sort_unstable();
                spans.dedup();
                for (at, len) in spans.into_iter().rev() {
                    self.positions.delete(at, len)?;
                }
            }
            Command::FillContentControl { control, fill } => {
                if self.base.node(*control).is_none() {
                    return Err(unsupported("the control has no source identity"));
                }
                let value = self
                    .built()?
                    .document
                    .prepare_control_fill(*control, fill.clone())?;
                self.positions.state.set_fact(
                    format!("{CONTROL}{control}"),
                    serde_json::to_vec(&value).expect("control serializes"),
                    false,
                )?;
            }
            Command::SetFieldResult { field, text } => self.field_value(*field, text)?,
            Command::UpdateFields { now, pages } => {
                let updates = self.built()?.document.field_updates(now, pages.as_ref())?;
                if updates.is_empty() {
                    return Err(crate::EditError::Fields {
                        reason: "have no newer results",
                    }
                    .into());
                }
                for update in updates {
                    self.field_value(update.field, &update.text)?;
                }
            }
            Command::SetUpdateFields { on } => {
                if self.built()?.document.settings.update_fields == *on {
                    return Err(crate::EditError::Setting {
                        setting: "updateFields",
                        reason: if *on {
                            "is on already"
                        } else {
                            "is off already"
                        },
                    }
                    .into());
                }
                self.positions.state.set_fact(
                    UPDATE_FIELDS.into(),
                    serde_json::to_vec(on).expect("bool serializes"),
                    false,
                )?;
            }
            Command::InsertBreak { at, to } => {
                self.replace_object(at, to.as_ref(), NewInline::Break)?
            }
            Command::InsertPicture {
                at,
                to,
                media,
                width_emu,
                height_emu,
                description,
                id,
            } => {
                // Validate the requested image before any shared write.
                let document = &self.built()?.document;
                document.picture_info(
                    media,
                    *width_emu,
                    *height_emu,
                    description.as_deref(),
                    *id,
                )?;
                for paragraph in document.paragraph_range(*at, *at)? {
                    document.check_picture_id(paragraph, media)?;
                }
                self.replace_object(
                    at,
                    to.as_ref(),
                    NewInline::Picture {
                        media: media.clone(),
                        width_emu: *width_emu,
                        height_emu: *height_emu,
                        description: description.clone(),
                        id: *id,
                    },
                )?;
            }
            Command::SplitParagraph { at } => {
                let at = self.split_position(at)?;
                self.replace(command.name(), at, 0, "\n", None, None)?;
            }
            Command::MergeParagraphs { paragraph } => {
                let view = self.built()?;
                let places = &view.places;
                let Some(&(first, len)) = places.paragraphs.get(paragraph) else {
                    return Err(unsupported("names a paragraph this path does not hold"));
                };
                view.document.check_mark(*paragraph)?;
                if len != 0 {
                    if let Some((&next, _)) = places
                        .paragraphs
                        .iter()
                        .find(|(_, (at, _))| *at == first + len + 1)
                    {
                        view.document.check_mark(next)?;
                    }
                }
                let units = self.positions.reading()?;
                let next = first + len;
                if !matches!(units.get(next).map(mark_of), Some(Ok(Some(_)))) {
                    return Err(unsupported("names the last paragraph"));
                }
                // An empty paragraph is removed, so the next keeps its own
                // properties. Otherwise the next one's mark goes.
                let mark = if len == 0 && first > 0 {
                    first - 1
                } else {
                    next
                };
                if let Some(like) = mark_of(&units[mark])?.and_then(|m| m.like()) {
                    self.base.check_source_mark(like)?;
                }
                self.positions.delete(mark, 1)?;
            }
            Command::SetLink { from, to, target } => {
                let view = self.built()?;
                let paragraphs = view.document.paragraph_range(*from, *to)?;
                if let Some(target) = target {
                    check_text(target)?;
                    if target.trim().is_empty() || target == "#" {
                        return Err(unsupported("a link needs somewhere to point"));
                    }
                    for p in &paragraphs {
                        if !target.starts_with('#') {
                            view.document.check_link_id(*p, target)?;
                        }
                    }
                }
                let ranges = view.selected_ranges(from, to, &paragraphs, command.name())?;
                if ranges.is_empty() {
                    return Err(unsupported("the range is empty or ends before it starts"));
                }
                let changes = view.changes_runs(&paragraphs, &ranges, |run| {
                    let parent = view
                        .document
                        .n(run)
                        .parent
                        .map(|id| &view.document.n(id).kind);
                    let here = match parent {
                        Some(NodeKind::Hyperlink(link)) => crate::links::href(link),
                        _ => None,
                    };
                    here.as_deref() != target.as_deref()
                })?;
                let units = self.positions.reading()?;
                for range in &ranges {
                    for unit in units.range(range.clone()) {
                        if let Some(Inline::Source { node, .. }) = inline_of(unit)? {
                            if self.base.holds_field_char(node) {
                                return Err(unsupported(
                                    "the range holds part of a field; select text outside fields",
                                ));
                            }
                        }
                    }
                }
                // A command that changes something states its whole selection,
                // so it holds where another replica's link is later taken back.
                for range in ranges.into_iter().filter(|_| changes) {
                    self.positions.range(
                        range.start,
                        range.len(),
                        (LINK, Some(target.as_deref().unwrap_or(UNLINKED))),
                    )?;
                }
            }
            Command::SetParagraphStyle { from, to, style } => {
                let view = self.built()?;
                let paragraphs = view.document.paragraph_range(*from, *to)?;
                if view
                    .document
                    .styles
                    .styles
                    .get(style)
                    .is_none_or(|s| s.kind != "paragraph")
                {
                    return Err(unsupported(
                        "the document has no paragraph style with that ID",
                    ));
                }
                let default = view
                    .document
                    .styles
                    .default_paragraph_style()
                    .map(|s| s.id.as_str());
                let has = |p: &NodeId| matches!(&view.document.n(*p).kind, NodeKind::Paragraph(info) if info.style.as_deref().or(default) == Some(style.as_str()));
                // As for a format: all of the selection, or nothing.
                let values = match paragraphs.iter().all(has) {
                    true => Vec::new(),
                    false => paragraphs
                        .into_iter()
                        .map(|p| (p, STYLE, style.clone()))
                        .collect(),
                };
                self.paragraph_values(values)?;
            }
            Command::SetNumbering {
                from,
                to,
                numbering,
            } => {
                let view = self.built()?;
                let paragraphs = view.document.paragraph_range(*from, *to)?;
                if let Some(n) = numbering {
                    if n.level > crate::MAX_LEVEL {
                        return Err(unsupported("a list level is from 0 to 8"));
                    }
                    if n.num_id != "0"
                        && !view.document.numbering.instances.contains_key(&n.num_id)
                        && !view.document.numbering.num_ids.contains(&n.num_id)
                        && !view.document.named_lists().any(|id| id == n.num_id)
                    {
                        return Err(unsupported("the document has no list with that number"));
                    }
                }
                let state = numbering
                    .as_ref()
                    .map(|n| self.numbering_value(n))
                    .transpose()?;
                let value = serde_json::to_string(&state).expect("numbering serializes");
                self.paragraph_values(
                    paragraphs
                        .into_iter()
                        .map(|p| (p, NUMBERING, value.clone()))
                        .collect(),
                )?;
            }
            Command::ChangeListLevel { from, to, by } => {
                let view = self.built()?;
                let paragraphs = view.document.paragraph_range(*from, *to)?;
                let numbers: Vec<_> = paragraphs
                    .into_iter()
                    .filter_map(|p| {
                        let n = view.document.list_ref(p)?;
                        let level = (n.level as i16 + *by as i16)
                            .clamp(0, view.document.deepest(&n.num_id) as i16)
                            as u8;
                        (level != n.level).then(|| {
                            (
                                p,
                                NumRef {
                                    num_id: n.num_id.clone(),
                                    level,
                                },
                            )
                        })
                    })
                    .collect();
                let mut values = Vec::new();
                for (p, number) in numbers {
                    let state = self.numbering_value(&number)?;
                    values.push((
                        p,
                        NUMBERING,
                        serde_json::to_string(&Some(state)).expect("numbering serializes"),
                    ));
                }
                self.paragraph_values(values)?;
            }
            Command::SetList { from, to, kind } => {
                let identity = self.positions.next_identity();
                let view = self.built()?;
                let paragraphs = view.document.paragraph_range(*from, *to)?;
                let near = kind.and_then(|kind| {
                    paragraphs
                        .iter()
                        .copied()
                        .chain(view.document.sibling_paragraph(paragraphs[0], -1))
                        .chain(
                            view.document
                                .sibling_paragraph(*paragraphs.last().unwrap(), 1),
                        )
                        .find(|p| view.document.list_kind(*p) == Some(kind))
                        .and_then(|p| view.document.list_ref(p))
                        .cloned()
                });
                let rows: Vec<_> = paragraphs
                    .into_iter()
                    .map(|p| {
                        let level = view.document.list_ref(p).map_or(0, |n| n.level);
                        let styled = view
                            .document
                            .style_numbering(p)
                            .is_some_and(|n| n.num_id != "0");
                        (p, level, styled)
                    })
                    .collect();
                let deepest = near
                    .as_ref()
                    .map_or(8, |n| view.document.deepest(&n.num_id));
                let basis = if let Some(number) = near {
                    Some(self.numbering_value(&number)?.basis)
                } else {
                    kind.map(|kind| ListBasis::Created {
                        identity: format!("{}.{}", identity.0, identity.1),
                        kind,
                    })
                };
                let mut values = Vec::new();
                for (p, level, styled) in rows {
                    let state = basis
                        .clone()
                        .map(|basis| ListState {
                            basis,
                            level: level.min(deepest),
                        })
                        .or_else(|| {
                            styled.then(|| ListState {
                                basis: ListBasis::Source("0".into()),
                                level: 0,
                            })
                        });
                    values.push((
                        p,
                        NUMBERING,
                        serde_json::to_string(&state).expect("numbering serializes"),
                    ));
                }
                self.paragraph_values(values)?;
            }
            Command::FormatRange { from, to, format } => {
                let view = self.built()?;
                let paragraphs = view.document.paragraph_range(*from, *to)?;
                let ranges = view.selected_ranges(from, to, &paragraphs, command.name())?;
                let mut writes = Vec::new();
                for (key, wanted) in [
                    (BOLD, format.bold),
                    (ITALIC, format.italic),
                    (UNDERLINE, format.underline),
                ] {
                    let Some(wanted) = wanted else { continue };
                    let changes = view.changes_runs(&paragraphs, &ranges, |run| {
                        let props = view.document.effective_run_props(run);
                        let current = match key {
                            BOLD => props.bold.unwrap_or(false),
                            ITALIC => props.italic.unwrap_or(false),
                            _ => props.underline.as_deref().is_some_and(|u| u != "none"),
                        };
                        current != wanted
                    })?;
                    // A command that changes something states its whole
                    // selection, so it holds where another replica's format
                    // is later taken back.
                    if changes {
                        writes.extend(ranges.iter().map(|r| (r.clone(), key, wanted)));
                    }
                }
                for (range, key, wanted) in writes {
                    self.positions
                        .range(range.start, range.len(), (key, value(Some(wanted))))?;
                }
            }
        }
        self.previous = self.view.take();
        Ok(())
    }
}
