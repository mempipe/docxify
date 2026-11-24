//! Structural payloads outlive their activation. The table grid resolves
//! named columns and vertical-merge dependencies on data, then writes the
//! selected table once. Other source blocks use the block interpreter.
use super::*;
#[path = "editing_table_grid.rs"]
mod grid;
#[derive(Clone, serde::Serialize, serde::Deserialize)]
enum Op {
    AddRow {
        table: NodeId,
        key: String,
        before: Option<String>,
        like: String,
        merges: Vec<MergeBasis>,
    },
    AddColumn {
        table: NodeId,
        key: String,
        before: Option<String>,
        left: Option<String>,
        like: String,
    },
    DropRow {
        table: NodeId,
        key: String,
    },
    DropColumn {
        table: NodeId,
        key: String,
    },
    Remove {
        node: NodeId,
    },
}
impl Op {
    fn table(&self) -> Option<NodeId> {
        match self {
            Self::AddRow { table, .. }
            | Self::DropRow { table, .. }
            | Self::AddColumn { table, .. }
            | Self::DropColumn { table, .. } => Some(*table),
            Self::Remove { .. } => None,
        }
    }
}
/// What a continuation was made to continue: its own cell, and the row and
/// cell where that vertical merge starts. The table reader decides where the
/// statement holds.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct MergeBasis {
    origin: String,
    start: String,
    cell: String,
}

fn cell_columns(doc: &Document, row: NodeId) -> Vec<(NodeId, usize, usize)> {
    doc.row_geometry(row)
        .into_iter()
        .filter(|s| matches!(doc.n(s.owner).kind, NodeKind::Cell(_)))
        .map(|s| (s.owner, s.start, s.span))
        .collect()
}
/// The cells of a row, each with a name that outlives grid changes: an
/// editable cell's origin, or a wrapper's origin and the cell's place in it.
fn pieces<'a>(
    doc: &'a Document,
    row: NodeId,
    shape: &Shape,
) -> Vec<(String, crate::tables::CellSlot<'a>)> {
    let mut inside: HashMap<NodeId, usize> = HashMap::new();
    doc.row_geometry(row)
        .into_iter()
        .map(|cell| {
            let origin =
                (shape.origins.get(&cell.owner).cloned()).unwrap_or_else(|| cell.owner.to_string());
            if matches!(doc.n(cell.owner).kind, NodeKind::Cell(_)) {
                return (origin, cell);
            }
            let at = inside.entry(cell.owner).or_default();
            *at += 1;
            (format!("{origin}#{}", *at - 1), cell)
        })
        .collect()
}
/// Each continuation in `row`, with the start reached through the merged
/// cells directly above it. A continuation under anything else names none.
fn merge_basis(doc: &Document, table: NodeId, row: NodeId, shape: &Shape) -> Vec<MergeBasis> {
    pieces(doc, row, shape)
        .into_iter()
        .filter(|(_, cell)| cell.merge == Some("continue"))
        .filter_map(|(origin, cell)| {
            let start = doc.merge_start_row(table, row, &cell)?;
            let (id, _) = pieces(doc, start, shape)
                .into_iter()
                .find(|(_, c)| c.start == cell.start && c.span == cell.span)
                .expect("the merge start has the continuation's coverage");
            Some(MergeBasis {
                origin,
                start: shape.row(start),
                cell: id,
            })
        })
        .collect()
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct CellBasis {
    pub properties: String,
    pub numbering: Option<ListState>,
}
#[derive(Default)]
pub(super) struct Shape {
    pub document: Option<Document>,
    rows: HashMap<NodeId, String>,
    cols: HashMap<NodeId, Vec<String>>,
    // View addresses map to immutable source-cell or column-creation origins.
    // Row copies carry the origin; widening/narrowing never changes it.
    origins: HashMap<NodeId, String>,
    pub cells: HashMap<String, (NodeId, Option<NodeId>)>,
    /// Each created cell's first paragraph as the table writer left it,
    /// before anything typed in the cell is written into it.
    pub cell_paragraphs: HashMap<NodeId, crate::ir::Node>,
    pub ignored: Vec<Resolution>,
    suppressed: HashSet<String>,
    hidden: HashSet<String>,
}
fn rows(doc: &Document, t: NodeId) -> Vec<NodeId> {
    doc.rows(t).collect()
}
fn cols(doc: &Document, t: NodeId) -> usize {
    if let NodeKind::Table(info) = &doc.node(t).unwrap().kind {
        info.grid.len()
    } else {
        0
    }
}
pub(super) fn attached(doc: &Document, mut id: NodeId) -> bool {
    loop {
        let Some(n) = doc.node(id) else { return false };
        if matches!(n.kind, NodeKind::Story { .. }) {
            return true;
        }
        let Some(p) = n.parent else { return false };
        if !doc.node(p).unwrap().children.contains(&id) {
            return false;
        }
        id = p;
    }
}
impl Shape {
    fn seeded(base: &Document) -> Self {
        Self {
            origins: base
                .stories()
                .flat_map(|s| base.descendants(s.id))
                .filter(|id| {
                    matches!(base.n(*id).kind, NodeKind::Cell(_))
                        || base.n(*id).preserved_cells.is_some()
                })
                .map(|id| (id, id.to_string()))
                .collect(),
            ..Self::default()
        }
    }
    fn row(&self, id: NodeId) -> String {
        self.rows
            .get(&id)
            .cloned()
            .unwrap_or_else(|| id.to_string())
    }
    fn columns(&self, doc: &Document, t: NodeId) -> Vec<String> {
        self.cols
            .get(&t)
            .cloned()
            .unwrap_or_else(|| (0..cols(doc, t)).map(|i| i.to_string()).collect())
    }
    pub fn build(base: &Document, store: &State) -> Result<Self, SessionError> {
        let mut ops = Vec::new();
        for (key, fact) in store.facts() {
            if let Some(id) = key.strip_prefix(STRUCTURE_PAYLOAD) {
                let op: Op = serde_json::from_slice(&fact.bytes)
                    .map_err(|_| corrupt("invalid table operation".into()))?;
                ops.push((fact.turn, id.to_owned(), op));
            }
        }
        if ops.is_empty() {
            return Ok(Self::seeded(base));
        }
        // A removed row atom belongs to its table, including atoms that only
        // obstruct column edits. Other source blocks retain the block interpreter.
        let table_of = |op: &Op| match op {
            Op::Remove { node } => base
                .node(*node)
                .filter(|n| n.preserved_cells.is_some())
                .and_then(|n| n.parent)
                .and_then(|r| base.n(r).parent),
            _ => op.table(),
        };
        ops.sort_by(|a, b| {
            (table_of(&a.2).map(std::cmp::Reverse), a.0, &a.1).cmp(&(
                table_of(&b.2).map(std::cmp::Reverse),
                b.0,
                &b.1,
            ))
        });
        let mut shape = Self::seeded(base);
        let mut doc = base.clone();
        let active = |id: &str| {
            store
                .facts()
                .contains_key(&format!("{STRUCTURE_ACTIVE}{id}"))
        };
        let mut messages = Vec::new();
        for group in ops.chunk_by(|a, b| table_of(&a.2) == table_of(&b.2)) {
            if let Some(table) = table_of(&group[0].2) {
                if attached(&doc, table) {
                    let mut grid = grid::Table::import(&doc, table, &shape);
                    for (_, id, op) in group {
                        grid.apply(op, active(id))?;
                    }
                    let decision = grid.decide()?;
                    decision.write(&mut doc, table, &mut shape)?;
                    messages.push((table, decision.reasons));
                }
            } else {
                for (_, id, op) in group {
                    let Op::Remove { node } = op else {
                        unreachable!()
                    };
                    if active(id) && attached(&doc, *node) {
                        if matches!(doc.n(*node).kind, NodeKind::Table(_)) {
                            // Preparation checked the user's current content, including
                            // bookmark edits. The source tree predates those edits.
                            let parent = doc.n(*node).parent.expect("attached table");
                            let children = doc
                                .n(parent)
                                .children
                                .iter()
                                .copied()
                                .filter(|id| id != node)
                                .collect::<Vec<_>>();
                            doc.arrange(parent, &children).map_err(unbuilt)?;
                        } else {
                            doc.remove_node(*node).map_err(unbuilt)?;
                        }
                    }
                }
            }
        }
        for (table, reasons) in messages {
            if attached(&doc, table) {
                shape.ignored.extend(reasons);
            }
        }
        let tables: Vec<_> = doc
            .stories()
            .flat_map(|s| doc.descendants(s.id))
            .filter(|id| matches!(doc.node(*id).unwrap().kind, NodeKind::Table(_)))
            .collect();
        for table in tables {
            for row in rows(&doc, table) {
                let rk = shape.row(row);
                for (cell, _, _) in cell_columns(&doc, row) {
                    let origin = &shape.origins[&cell];
                    if (rk.starts_with('+') || origin.starts_with('+')) && !doc.is_vacant_cell(cell)
                    {
                        let key = format!("{CELL}{table}/{rk}/{origin}");
                        if let Some(p) =
                            doc.node(cell).unwrap().children.iter().copied().find(|p| {
                                matches!(doc.node(*p).unwrap().kind, NodeKind::Paragraph(_))
                            })
                        {
                            shape.cells.insert(key, (p, doc.like(p)));
                            shape.cell_paragraphs.insert(p, doc.n(p).clone());
                        }
                    }
                }
            }
        }
        for strand in store.strands() {
            if let Ok(p) = strand.key.parse::<NodeId>() {
                if !attached(&doc, p) {
                    shape.hidden.insert(strand.key.clone());
                }
            }
        }
        for strand in store.strands() {
            if strand.prefix.is_none() {
                continue;
            }
            let Ok(p) = strand.key.parse::<NodeId>() else {
                continue;
            };
            if !attached(&doc, p) {
                continue;
            }
            let Some(parent) = doc.node(p).and_then(|n| n.parent) else {
                continue;
            };
            let children = &doc.node(parent).unwrap().children;
            if let Some(previous) = children
                .iter()
                .position(|id| *id == p)
                .and_then(|i| i.checked_sub(1))
                .map(|i| children[i])
            {
                let n = doc.node(previous).unwrap();
                if matches!(n.kind, NodeKind::Paragraph(_)) && n.support == Support::Editable {
                    shape.suppressed.insert(strand.key.clone());
                }
            }
        }
        shape.ignored.sort();
        shape.ignored.dedup();
        shape.document = Some(doc);
        Ok(shape)
    }
    pub fn write(
        &self,
        base: &Document,
        doc: &Document,
        store: &mut State,
        command: &Command,
        lists: &HashMap<String, ListBasis>,
    ) -> Result<(), SessionError> {
        // Table policy is checked before recording any fact. The resolver
        // reruns this explicitly ordered structural intent, never text edits.
        if let Command::RemoveNode { node } = command {
            if base.node(*node).is_none() && !self.rows.contains_key(node) {
                return Err(SessionError::Unsupported {
                    command: command.name(),
                    reason: "this item has no source identity to remove",
                });
            }
        }
        let (client, clock) = store.next_identity();
        let id = format!("{client}.{clock}");
        let key = format!("+{id}");
        let mut captured = Vec::new();
        let mut capture = |table: NodeId,
                           row: &str,
                           origin: &str,
                           template: NodeId|
         -> Result<(), SessionError> {
            let p = doc
                .n(template)
                .children
                .iter()
                .copied()
                .find(|p| matches!(doc.n(*p).kind, NodeKind::Paragraph(_)));
            let properties = match p {
                Some(p) => doc.paragraph_like(p, false)?.1,
                None => String::new(),
            };
            let numbering = p.and_then(|p| doc.list_ref(p)).map(|n| ListState {
                basis: lists
                    .get(&n.num_id)
                    .cloned()
                    .unwrap_or_else(|| ListBasis::Source(n.num_id.clone())),
                level: n.level,
            });
            captured.push((
                format!("{CELL_BASIS}{CELL}{table}/{row}/{origin}"),
                CellBasis {
                    properties,
                    numbering,
                },
            ));
            Ok(())
        };
        let op = match command {
            Command::InsertRow { row, below, like } => {
                let like = like.unwrap_or(*row);
                let table = doc.check_row_copy(*row, like)?;
                let rows = rows(doc, table);
                let at = rows.iter().position(|r| r == row).unwrap() + usize::from(*below);
                let before = rows.get(at).copied();
                let templates = pieces(doc, like, self);
                let mut merges = Vec::new();
                if let Some(next) = before {
                    let next_pieces = pieces(doc, next, self);
                    for mut dependency in merge_basis(doc, table, next, self) {
                        let (_, cell) = next_pieces
                            .iter()
                            .find(|(origin, _)| origin == &dependency.origin)
                            .unwrap();
                        let Some((origin, _)) = templates.iter().find(|(_, template)| {
                            template.start == cell.start && template.span == cell.span
                        }) else {
                            return Err(crate::EditError::Refused { node: like, reason: "a merged cell runs across this place, and the copied row has no cell with its columns" }.into());
                        };
                        dependency.origin.clone_from(origin);
                        merges.push(dependency);
                    }
                }
                for (origin, template) in templates {
                    capture(table, &key, &origin, template.owner)?;
                }
                Op::AddRow {
                    table,
                    key,
                    before: before.map(|r| self.row(r)),
                    like: self.row(like),
                    merges,
                }
            }
            Command::InsertColumn { table, index, like } => {
                doc.check_column(*table, *index, Some(*like))?;
                let cols = self.columns(doc, *table);
                for row in rows(doc, *table) {
                    let cells = doc.row_geometry(row);
                    if cells
                        .iter()
                        .any(|c| c.start < *index && *index < c.start + c.span)
                    {
                        continue;
                    }
                    let donor = cells
                        .iter()
                        .find(|c| c.start <= *like && *like < c.start + c.span)
                        .expect("validated row fills the grid");
                    capture(*table, &self.row(row), &key, donor.owner)?;
                }
                Op::AddColumn {
                    table: *table,
                    key,
                    before: cols.get(*index).cloned(),
                    left: index.checked_sub(1).map(|i| cols[i].clone()),
                    like: cols[*like].clone(),
                }
            }
            Command::RemoveColumn { table, index } => {
                doc.check_column(*table, *index, None)?;
                Op::DropColumn {
                    table: *table,
                    key: self.columns(doc, *table)[*index].clone(),
                }
            }
            Command::RemoveNode { node } => {
                doc.removal_nodes(*node)?;
                if matches!(doc.n(*node).kind, NodeKind::Row(_)) {
                    Op::DropRow {
                        table: doc.parent_of(*node)?,
                        key: self.row(*node),
                    }
                } else {
                    Op::Remove { node: *node }
                }
            }
            _ => unreachable!(),
        };
        store.set_fact(
            format!("{STRUCTURE_PAYLOAD}{id}"),
            serde_json::to_vec(&op).unwrap(),
            true,
        )?;
        store.set_fact(format!("{STRUCTURE_ACTIVE}{id}"), vec![1], false)?;
        for (key, value) in captured {
            store.set_fact(key, serde_json::to_vec(&value).unwrap(), true)?;
        }
        Ok(())
    }
    /// Gives each cell this shape creates a strand to type in. A cell nobody
    /// has typed in has none stored; its strand is declared when first written.
    pub(super) fn place_cells(&self, store: &mut State) {
        let mut cells: Vec<_> = self.cells.iter().collect();
        cells.sort_by_key(|(k, _)| *k);
        for (key, (_, like)) in cells {
            let name = serde_json::to_string(&(key, like)).unwrap();
            let flow = Flow::seeded(
                store.next_identity().0,
                &[Seeded::Item {
                    kind: MARK,
                    name: &format!("={name}"),
                }],
            );
            store.ensure(Strand {
                key: key.clone(),
                order: 1u64 << 63,
                prefix: Some(Slot::Unit(Content::Item {
                    kind: SCOPE.into(),
                    name: key.clone(),
                })),
                flow: Arc::new(flow),
                declared: false,
            });
        }
    }
}

/// A removed block's paragraphs are hidden, and so is the strand of a
/// created cell this shape does not show.
impl crate::state::Layout for Shape {
    fn hides(&self, key: &str) -> bool {
        self.hidden.contains(key) || (key.starts_with(CELL) && !self.cells.contains_key(key))
    }
    fn joins(&self, key: &str) -> bool {
        self.suppressed.contains(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unexpected_codec_refusal_is_not_a_merge_resolution() {
        let doc = Document::open(include_bytes!(
            "../../../fixtures/generated/simple-paragraphs.docx"
        ))
        .unwrap();
        let node = doc
            .descendants(doc.paragraphs()[0])
            .into_iter()
            .find(|id| matches!(doc.node(*id).unwrap().kind, NodeKind::Text { .. }))
            .unwrap();
        let mut state = seed(1, &doc).unwrap();
        // Supported edits cannot record this action. A damaged payload must
        // fail visibly, rather than letting the reader silently drop it.
        state
            .set_fact(
                "structure/payload/1.bad".into(),
                serde_json::to_vec(&Op::Remove { node }).unwrap(),
                true,
            )
            .unwrap();
        state
            .set_fact("structure/active/1.bad".into(), vec![1], false)
            .unwrap();
        assert!(matches!(
            Shape::build(&doc, &state),
            Err(SessionError::Unbuilt { .. })
        ));
    }
}
