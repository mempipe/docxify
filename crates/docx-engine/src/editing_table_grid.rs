//! The immutable source supplies cell templates. Each column payload names
//! both sides of its insertion gap. Cell coverage can therefore be assigned
//! while either neighbouring column is hidden, without consulting a view.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, PartialEq, Eq)]
enum GridSlot {
    Before,
    After,
    // The place of one cell of a removed wrapper. It keeps that cell's name,
    // which a copy of the row carries on.
    Vacated(String),
    Cell(String),
    // A wrapper, and the name of its cell that holds this column.
    Atom(NodeId, String),
}
/// One cell of a row as imported: an editable cell, or one cell held inside
/// a wrapper. Rows outside the declared grid have cells with no column name,
/// so the source position is kept too.
#[derive(Clone)]
struct Piece {
    id: String,
    start: usize,
    span: usize,
    atom: Option<NodeId>,
}
#[derive(Clone)]
struct Row {
    template: NodeId,
    source: bool,
    within_grid: bool,
    // Includes hidden columns and cells, so future grants keep their basis.
    slots: BTreeMap<String, GridSlot>,
    geometry: Vec<Piece>,
    cells: BTreeMap<String, Option<NodeId>>,
    atoms: Vec<NodeId>,
    // What each continuation was made to continue. Whether it does so on a
    // given grid is decided for the whole table, in `Table::merged`.
    merges: Vec<MergeBasis>,
}
pub(super) struct Table {
    source: Vec<(String, u32)>,
    columns: Vec<(String, u32)>,
    column_order: Vec<String>,
    transferred: BTreeMap<String, String>,
    touched: BTreeSet<String>,
    row_order: Vec<String>,
    rows: BTreeMap<String, Row>,
    available: BTreeSet<String>,
    deleted: BTreeSet<String>,
    removed: BTreeSet<NodeId>,
    last_row: Option<String>,
    held_last_column: bool,
    edited: bool,
}
struct Selection {
    rows: BTreeSet<String>,
    retained: BTreeSet<NodeId>,
    source_grid: bool,
    pub reasons: Vec<Resolution>,
}
struct DecidedRow<'a> {
    key: &'a String,
    basis: &'a Row,
    // An original row on its source grid can preserve its exact XML and
    // replace only explicitly vacated atoms, including unmodelled geometry.
    // A row outside the declared grid has no slots to rebuild its cells from,
    // so it and its copies also keep their own cells.
    slots: Option<Vec<GridSlot>>,
    removed_atoms: BTreeSet<NodeId>,
    // The continuations of this row that are part of their merge.
    continued: BTreeSet<String>,
}
pub(super) struct Decision<'a> {
    rows: Vec<DecidedRow<'a>>,
    columns: &'a [(String, u32)],
    resized: Option<&'a BTreeSet<String>>,
    pub reasons: Vec<Resolution>,
}
impl Table {
    pub fn import(doc: &Document, table: NodeId, shape: &Shape) -> Self {
        let NodeKind::Table(info) = &doc.n(table).kind else {
            unreachable!()
        };
        let source: Vec<_> = info
            .grid
            .iter()
            .enumerate()
            .map(|(i, &w)| (i.to_string(), w))
            .collect();
        let mut rows_by_key = BTreeMap::new();
        let mut row_order = Vec::new();
        for r in rows(doc, table) {
            let key = shape.row(r);
            let mut slots = BTreeMap::new();
            let mut cells = BTreeMap::new();
            let geometry = pieces(doc, r, shape);
            let NodeKind::Row(info) = &doc.n(r).kind else {
                unreachable!()
            };
            for (i, (col, _)) in source.iter().enumerate() {
                slots.insert(
                    col.clone(),
                    if i < (info.grid_before as usize) {
                        GridSlot::Before
                    } else {
                        GridSlot::After
                    },
                );
            }
            let within_grid = geometry
                .iter()
                .all(|(_, s)| s.start.saturating_add(s.span) <= source.len());
            for (id, cell) in &geometry {
                let normal = matches!(doc.n(cell.owner).kind, NodeKind::Cell(_));
                if normal {
                    cells.insert(id.clone(), Some(cell.owner));
                }
                for (col, _) in source.iter().skip(cell.start).take(cell.span) {
                    slots.insert(
                        col.clone(),
                        if normal {
                            GridSlot::Cell(id.clone())
                        } else {
                            GridSlot::Atom(cell.owner, id.clone())
                        },
                    );
                }
            }
            let geometry = geometry
                .into_iter()
                .map(|(id, cell)| Piece {
                    id,
                    start: cell.start,
                    span: cell.span,
                    atom: doc
                        .n(cell.owner)
                        .preserved_cells
                        .as_ref()
                        .map(|_| cell.owner),
                })
                .collect();
            let atoms = doc
                .n(r)
                .children
                .iter()
                .copied()
                .filter(|id| {
                    !matches!(doc.n(*id).kind, NodeKind::Cell(_) | NodeKind::Marker { .. })
                })
                .collect();
            rows_by_key.insert(
                key.clone(),
                Row {
                    template: r,
                    source: true,
                    within_grid,
                    slots,
                    geometry,
                    cells,
                    atoms,
                    merges: merge_basis(doc, table, r, shape),
                },
            );
            row_order.push(key);
        }
        Self {
            columns: source.clone(),
            column_order: source.iter().map(|c| c.0.clone()).collect(),
            source,
            transferred: Default::default(),
            touched: Default::default(),
            available: row_order.iter().cloned().collect(),
            row_order,
            rows: rows_by_key,
            deleted: Default::default(),
            removed: Default::default(),
            last_row: None,
            held_last_column: false,
            edited: false,
        }
    }
    pub fn apply(&mut self, op: &Op, active: bool) -> Result<(), SessionError> {
        match op {
            Op::Remove { node } if active => {
                self.removed.insert(*node);
            }
            Op::DropRow { key, .. } if active => {
                self.deleted.insert(key.clone());
                self.last_row = Some(key.clone());
            }
            Op::AddRow {
                key,
                before,
                like,
                merges,
                ..
            } => {
                if self.rows.contains_key(key) {
                    return Err(corrupt("duplicate row identity".into()));
                }
                let mut row = self
                    .rows
                    .get(like)
                    .ok_or_else(|| corrupt("unknown row template".into()))?
                    .clone();
                if merges.iter().any(|m| !self.rows.contains_key(&m.start)) {
                    return Err(corrupt("a merge names an unknown start row".into()));
                }
                row.source = false;
                row.atoms.clear();
                row.merges = merges.clone();
                let i = position(&self.row_order, before)?;
                self.row_order.insert(i, key.clone());
                self.rows.insert(key.clone(), row);
                if active {
                    self.available.insert(key.clone());
                }
            }
            Op::AddColumn {
                key,
                before,
                left,
                like,
                ..
            } => {
                if self.column_order.contains(key)
                    || left
                        .as_ref()
                        .is_some_and(|k| !self.column_order.contains(k))
                {
                    return Err(corrupt(
                        "invalid column creation identity or left anchor".into(),
                    ));
                }
                let at = position(&self.column_order, before)?;
                for row in self.rows.values_mut() {
                    let model = match row.slots.get(like) {
                        Some(GridSlot::Cell(origin)) => row.cells.get(origin).copied().flatten(),
                        Some(_) => None,
                        None => return Err(corrupt("unknown column template".into())),
                    };
                    let l = left.as_ref().and_then(|c| row.slots.get(c));
                    let r = before.as_ref().and_then(|c| row.slots.get(c));
                    // A partial row has no cell at the copied column, and the
                    // new column extends its gap. A wrapper's cell has no
                    // template either, but it is a cell.
                    let omitted = !matches!(
                        row.slots.get(like),
                        Some(GridSlot::Cell(_) | GridSlot::Atom(..))
                    );
                    let slot = match (l, r) {
                        (Some(GridSlot::Cell(a)), Some(GridSlot::Cell(b))) if a == b => {
                            GridSlot::Cell(a.clone())
                        }
                        // A wrapper's cell widens the same way. It shows on
                        // an edited grid only as a vacated place or a copy.
                        (Some(GridSlot::Atom(a, p)), Some(GridSlot::Atom(b, q)))
                            if a == b && p == q =>
                        {
                            GridSlot::Atom(*a, p.clone())
                        }
                        (_, Some(GridSlot::Before)) => GridSlot::Before,
                        (None | Some(GridSlot::Before), _) if omitted => GridSlot::Before,
                        (Some(GridSlot::After), _) => GridSlot::After,
                        (_, None | Some(GridSlot::After)) if omitted => GridSlot::After,
                        _ => {
                            row.cells.insert(key.clone(), model);
                            GridSlot::Cell(key.clone())
                        }
                    };
                    row.slots.insert(key.clone(), slot);
                }
                let live_at = self.column_order[..at]
                    .iter()
                    .rev()
                    .find_map(|k| self.columns.iter().position(|c| &c.0 == k))
                    .map_or(0, |i| i + 1);
                self.column_order.insert(at, key.clone());
                if active {
                    self.edited = true;
                    let mut donor = like;
                    while let Some(next) = self.transferred.get(donor) {
                        donor = next;
                    }
                    let width = &mut self
                        .columns
                        .iter_mut()
                        .find(|c| &c.0 == donor)
                        .ok_or_else(|| corrupt("unknown column width donor".into()))?
                        .1;
                    let half = *width / 2;
                    *width -= half;
                    self.touched.insert(donor.clone());
                    self.columns.insert(live_at, (key.clone(), half));
                    if self.held_last_column {
                        self.remove_column(1 - live_at);
                        self.held_last_column = false;
                    }
                    self.touched.insert(key.clone());
                } else {
                    self.transferred.insert(key.clone(), like.clone());
                }
            }
            Op::DropColumn { key, .. } if active => {
                self.edited = true;
                if let Some(i) = self.columns.iter().position(|c| &c.0 == key) {
                    if self.columns.len() == 1 {
                        self.held_last_column = true;
                    } else {
                        self.remove_column(i);
                    }
                }
            }
            _ => (),
        }
        Ok(())
    }

    fn remove_column(&mut self, i: usize) {
        let (key, w) = self.columns.remove(i);
        let recipient = &mut self.columns[i.saturating_sub(1)];
        recipient.1 += w;
        self.touched.insert(recipient.0.clone());
        self.transferred.insert(key, recipient.0.clone());
    }
    fn covered(row: &Row, cols: &[(String, u32)]) -> bool {
        cols.iter().any(|(col, _)| {
            matches!(
                row.slots.get(col),
                Some(GridSlot::Cell(_) | GridSlot::Atom(..))
            )
        })
    }
    fn fits(row: &Row, cols: &[(String, u32)]) -> bool {
        let mut starts = BTreeSet::new();
        let mut last = None;
        for (col, _) in cols {
            let next = match row.slots.get(col) {
                Some(GridSlot::Cell(k) | GridSlot::Atom(_, k)) => Some(k),
                _ => None,
            };
            if next != last && next.is_some_and(|k| !starts.insert(k)) {
                return false;
            }
            last = next;
        }
        row.within_grid && Self::covered(row, cols)
    }
    /// The cells a row shows on a grid, each with the columns it holds. A
    /// wrapper's cells count while `kept` says the wrapper is shown. On the
    /// source grid the source positions name the columns, so a row outside
    /// the declared grid is read too.
    fn shown<'a>(
        row: &'a Row,
        source_grid: bool,
        cols: &[(String, u32)],
        kept: impl Fn(NodeId) -> bool,
    ) -> Vec<(&'a String, Vec<String>)> {
        // A created row holds an ordinary empty cell where its template held
        // a wrapper's cell.
        let here = |p: &&Piece| p.atom.is_none_or(|n| !row.source || kept(n));
        if source_grid {
            return row
                .geometry
                .iter()
                .filter(here)
                .map(|p| {
                    let columns = (p.start..p.start + p.span).map(|i| i.to_string());
                    (&p.id, columns.collect())
                })
                .collect();
        }
        let mut out: Vec<(&String, Vec<String>)> = Vec::new();
        for (col, _) in cols {
            let id = match row.slots.get(col) {
                Some(GridSlot::Cell(id)) => Some(id),
                Some(GridSlot::Atom(_, id)) => {
                    let piece = row.geometry.iter().find(|p| p.id == *id);
                    piece.filter(here).map(|p| &p.id)
                }
                _ => None,
            };
            let Some(id) = id else { continue };
            match out.iter_mut().find(|(known, _)| *known == id) {
                Some((_, columns)) => columns.push(col.clone()),
                None => out.push((id, vec![col.clone()])),
            }
        }
        out
    }
    /// Vertical merges, decided once for the whole table. Walking the shown
    /// rows downwards, a continuation is part of its merge only when the cell
    /// directly above it is that merge's start, or a continuation already
    /// found to be part of that merge, and both hold the same columns. A
    /// break anywhere leaves the cells below it ordinary until it closes, and
    /// a cell never joins a merge it was not made to continue.
    fn merged(
        &self,
        rows: &[&String],
        source_grid: bool,
        cols: &[(String, u32)],
        retained: &BTreeSet<NodeId>,
    ) -> BTreeMap<String, BTreeSet<String>> {
        let starts: BTreeSet<_> = (self.rows.values())
            .flat_map(|r| r.merges.iter().map(|m| (&m.start, &m.cell)))
            .collect();
        let kept = |n: NodeId| !self.removed.contains(&n) || retained.contains(&n);
        let mut out = BTreeMap::new();
        let mut above: Vec<(Vec<String>, (&String, &String))> = Vec::new();
        for &key in rows {
            let row = &self.rows[key];
            let mut here = Vec::new();
            let mut continued = BTreeSet::new();
            for (id, columns) in Self::shown(row, source_grid, cols, kept) {
                if starts.contains(&(key, id)) {
                    here.push((columns, (key, id)));
                } else if let Some(m) = row.merges.iter().find(|m| &m.origin == id) {
                    let merge = (&m.start, &m.cell);
                    if above
                        .iter()
                        .any(|(c, known)| *c == columns && *known == merge)
                    {
                        continued.insert(id.clone());
                        here.push((columns, merge));
                    }
                }
            }
            out.insert(key.clone(), continued);
            above = here;
        }
        out
    }
    /// The rows to show, given the rows nobody deleted. A deleted start row,
    /// or a removed wrapper that holds a start, stays while the next shown
    /// row below continues that start. Each answer depends only on the rows
    /// below it, so one pass upwards settles them all.
    fn choose(
        &self,
        initial: &BTreeSet<String>,
        source_grid: bool,
        cols: &[(String, u32)],
    ) -> Option<(BTreeSet<String>, BTreeSet<NodeId>)> {
        let mut rows = BTreeSet::new();
        let mut retained = BTreeSet::new();
        let mut below: Option<&String> = None;
        for key in self.row_order.iter().rev() {
            let row = &self.rows[key];
            let mut needed = Vec::new();
            if let Some(lower) = below.map(|k| &self.rows[k]) {
                let kept = |n: NodeId| !self.removed.contains(&n) || retained.contains(&n);
                let theirs = Self::shown(lower, source_grid, cols, kept);
                let ours = Self::shown(row, source_grid, cols, |_| true);
                for m in lower.merges.iter().filter(|m| &m.start == key) {
                    let own = theirs.iter().find(|(id, _)| **id == m.origin);
                    let start = ours.iter().find(|(id, _)| **id == m.cell);
                    if let (Some(own), Some(start)) = (own, start) {
                        if own.1 == start.1 {
                            let piece = row.geometry.iter().find(|p| p.id == m.cell);
                            needed.push(piece.and_then(|p| p.atom));
                        }
                    }
                }
            }
            if initial.contains(key) || (!needed.is_empty() && self.available.contains(key)) {
                let removed = needed.into_iter().flatten();
                retained.extend(removed.filter(|n| self.removed.contains(n)));
                rows.insert(key.clone());
                below = Some(key);
            }
        }
        let unfit = |key: &String| {
            let row = &self.rows[key];
            let wrapper = |n: &NodeId| !self.removed.contains(n) || retained.contains(n);
            !Self::fits(row, cols) || row.atoms.iter().any(wrapper)
        };
        if !source_grid && rows.iter().any(unfit) {
            return None;
        }
        Some((rows, retained))
    }
    fn select(&self, source_grid: bool) -> Option<Selection> {
        let cols = if source_grid {
            &self.source
        } else {
            &self.columns
        };
        let selected: BTreeSet<_> = self
            .available
            .difference(&self.deleted)
            .filter(|key| {
                source_grid || !self.rows[*key].within_grid || Self::covered(&self.rows[*key], cols)
            })
            .cloned()
            .collect();
        let mut reasons = Vec::new();
        let (expected, (rows, retained)) = if selected.is_empty() {
            let (key, found) = self
                .last_row
                .iter()
                .chain(self.row_order.iter().rev())
                .filter(|k| self.available.contains(*k))
                .find_map(|k| {
                    self.choose(&[k.clone()].into(), source_grid, cols)
                        .map(|f| (k, f))
                })?;
            reasons.push(Resolution::LastRow);
            ([key.clone()].into(), found)
        } else {
            let found = self.choose(&selected, source_grid, cols)?;
            (selected, found)
        };
        // One outcome, a merge start that is still needed, kept in one of
        // two ways. They are told apart so a person is told the right thing.
        if rows != expected {
            reasons.push(Resolution::MergeStartRow);
        }
        if !retained.is_empty() {
            reasons.push(Resolution::MergeStartWrapper);
        }
        Some(Selection {
            rows,
            retained,
            source_grid,
            reasons,
        })
    }
    fn selection(&self) -> Result<Selection, SessionError> {
        let mut source = self
            .select(true)
            .ok_or_else(|| corrupt("table has no available source row".into()))?;
        if !self.edited {
            return Ok(source);
        }
        let wrapped = source.rows.iter().any(|k| {
            self.rows[k]
                .atoms
                .iter()
                .any(|n| !self.removed.contains(n) || source.retained.contains(n))
        });
        if !wrapped {
            if let Some(mut d) = self.select(false) {
                if self.held_last_column {
                    d.reasons.push(Resolution::LastColumn);
                }
                return Ok(d);
            }
        }
        source.reasons.push(Resolution::SourceGrid);
        Ok(source)
    }
    pub fn decide(&self) -> Result<Decision<'_>, SessionError> {
        let selection = self.selection()?;
        let columns = if selection.source_grid {
            &self.source
        } else {
            &self.columns
        };
        let shown: Vec<_> = (self.row_order.iter())
            .filter(|k| selection.rows.contains(*k))
            .collect();
        let mut merged = self.merged(&shown, selection.source_grid, columns, &selection.retained);
        let mut rows = Vec::new();
        for key in shown {
            let basis = &self.rows[key];
            let removed_atoms: BTreeSet<_> = basis
                .atoms
                .iter()
                .copied()
                .filter(|id| self.removed.contains(id) && !selection.retained.contains(id))
                .collect();
            let own_cells = selection.source_grid && (basis.source || !basis.within_grid);
            let slots = (!own_cells).then(|| {
                columns
                    .iter()
                    .map(|(col, _)| match &basis.slots[col] {
                        GridSlot::Atom(n, name) if basis.source && removed_atoms.contains(n) => {
                            GridSlot::Vacated(name.clone())
                        }
                        GridSlot::Atom(..) if basis.source => basis.slots[col].clone(),
                        GridSlot::Atom(_, name) => GridSlot::Cell(name.clone()),
                        s => s.clone(),
                    })
                    .collect()
            });
            rows.push(DecidedRow {
                key,
                basis,
                slots,
                removed_atoms,
                continued: merged.remove(key).unwrap_or_default(),
            });
        }
        Ok(Decision {
            rows,
            columns,
            resized: (!selection.source_grid).then_some(&self.touched),
            reasons: selection.reasons,
        })
    }
}
fn position(order: &[String], before: &Option<String>) -> Result<usize, SessionError> {
    before.as_ref().map_or(Ok(order.len()), |k| {
        order
            .iter()
            .position(|c| c == k)
            .ok_or_else(|| corrupt("unknown insertion anchor".into()))
    })
}

// Insert preserved children at their source boundary. An inserted cell or
// row has no source boundary of its own. It cannot acquire a marker's identity.
fn with_extras(
    original: &[NodeId],
    primary: Vec<NodeId>,
    extras: &BTreeSet<NodeId>,
) -> Vec<NodeId> {
    let mut output = primary;
    let positions: BTreeSet<_> = output.iter().copied().collect();
    let mut anchor = None;
    for &id in original.iter().rev() {
        if positions.contains(&id) {
            anchor = Some(id);
        } else if extras.contains(&id) {
            let at = anchor
                .and_then(|n| output.iter().position(|&v| v == n))
                .unwrap_or(output.len());
            output.insert(at, id);
            anchor = Some(id);
        }
    }
    output
}

impl Decision<'_> {
    pub fn write(
        &self,
        doc: &mut Document,
        table: NodeId,
        shape: &mut Shape,
    ) -> Result<(), SessionError> {
        let cols = self.columns;
        let widths: Vec<_> = cols.iter().map(|c| c.1).collect();
        let original = doc.n(table).children.clone();
        let extras = original
            .iter()
            .copied()
            .filter(|c| !matches!(doc.n(*c).kind, NodeKind::Row(_)))
            .collect();
        let mut ids = BTreeMap::new();
        // Copies read the original cell templates before any source row is written.
        for copied in [true, false] {
            for decided in &self.rows {
                let key = decided.key;
                let row = decided.basis;
                if row.source == copied {
                    continue;
                }
                let id = if row.source {
                    row.template
                } else {
                    let NodeKind::Row(info) = doc.n(row.template).kind.clone() else {
                        unreachable!()
                    };
                    doc.empty_row_like(row.template, info)
                };
                if !row.source {
                    shape.rows.insert(id, key.clone());
                }
                ids.insert(key, id);
                let Some(slots) = &decided.slots else {
                    if !row.source {
                        let mut children = Vec::new();
                        // A copy has an ordinary empty cell for each cell of
                        // its template, also for one a wrapper holds.
                        for piece in &row.geometry {
                            let like = row.cells.get(&piece.id).copied().flatten();
                            let cell = match like {
                                Some(like) => doc.empty_copy(like),
                                None => doc.bare_cell(id),
                            };
                            let continued = Some(decided.continued.contains(&piece.id));
                            doc.write_grid_cell(cell, piece.span, 0, continued, false)
                                .map_err(unbuilt)?;
                            shape.origins.insert(cell, piece.id.clone());
                            children.push(cell);
                        }
                        doc.arrange(id, &children).map_err(unbuilt)?;
                    } else if !decided.removed_atoms.is_empty() {
                        let mut children = Vec::new();
                        for child in doc.n(id).children.clone() {
                            if decided.removed_atoms.contains(&child) {
                                for (i, blank) in doc
                                    .wrapper_vacancies(id, child)
                                    .map_err(unbuilt)?
                                    .into_iter()
                                    .enumerate()
                                {
                                    // The name `pieces` gives that cell of the wrapper.
                                    shape.origins.insert(blank, format!("{child}#{i}"));
                                    children.push(blank);
                                }
                            } else {
                                children.push(child);
                            }
                        }
                        doc.arrange(id, &children).map_err(unbuilt)?;
                    }
                    if row.source {
                        // The row keeps its own XML. Only an editable
                        // continuation cut off from its merge changes.
                        for m in &row.merges {
                            let cell = row.cells.get(&m.origin).copied().flatten();
                            if let Some(cell) =
                                cell.filter(|_| !decided.continued.contains(&m.origin))
                            {
                                let NodeKind::Cell(info) = &doc.n(cell).kind else {
                                    unreachable!()
                                };
                                let span = info.grid_span.max(1) as usize;
                                doc.write_grid_cell(cell, span, 0, Some(false), false)
                                    .map_err(unbuilt)?;
                            }
                        }
                    }
                    continue;
                };
                let children = doc.n(id).children.clone();
                let extras = children
                    .iter()
                    .copied()
                    .filter(|c| {
                        !decided.removed_atoms.contains(c)
                            && !matches!(doc.n(*c).kind, NodeKind::Cell(_))
                            && doc.n(*c).preserved_cells.is_none()
                    })
                    .collect();
                let before = slots
                    .iter()
                    .take_while(|s| matches!(s, GridSlot::Before))
                    .count();
                let after = slots[before..]
                    .iter()
                    .rev()
                    .take_while(|s| matches!(s, GridSlot::After))
                    .count();
                let mut cells = Vec::new();
                let mut i = before;
                while i < slots.len() - after {
                    let end = i + slots[i..].iter().take_while(|s| **s == slots[i]).count();
                    match &slots[i] {
                        GridSlot::Cell(origin) => {
                            let model = row.cells.get(origin).copied().flatten();
                            let source = model.filter(|n| row.source && n.to_string() == *origin);
                            let cell = source.unwrap_or_else(|| {
                                model
                                    .map(|n| doc.empty_copy(n))
                                    .unwrap_or_else(|| doc.bare_cell(id))
                            });
                            let stated = row.merges.iter().any(|m| m.origin == *origin);
                            let continued =
                                (!row.source || stated).then(|| decided.continued.contains(origin));
                            let resize = self.resized.is_some_and(|t| {
                                origin.starts_with('+')
                                    || cols[i..end].iter().any(|(c, _)| t.contains(c))
                            });
                            doc.write_grid_cell(
                                cell,
                                end - i,
                                widths[i..end].iter().sum(),
                                continued,
                                resize,
                            )
                            .map_err(unbuilt)?;
                            shape.origins.insert(cell, origin.clone());
                            cells.push(cell);
                        }
                        GridSlot::Atom(n, _) => {
                            if cells.last() != Some(n) {
                                cells.push(*n);
                            }
                        }
                        GridSlot::Vacated(name) => {
                            let cell = doc.vacant_cell(id);
                            doc.write_grid_cell(cell, end - i, 0, None, false)
                                .map_err(unbuilt)?;
                            shape.origins.insert(cell, name.clone());
                            cells.push(cell);
                        }
                        _ => {
                            for (col, _) in &cols[i..end] {
                                let cell = doc.vacant_cell(id);
                                shape.origins.insert(cell, format!("vacant/{col}"));
                                cells.push(cell);
                            }
                        }
                    }
                    i = end;
                }
                doc.arrange(id, &with_extras(&children, cells, &extras))
                    .map_err(unbuilt)?;
                doc.compose_row_gaps(id, before, after).map_err(unbuilt)?;
            }
        }
        let rs = self.rows.iter().map(|r| ids[r.key]).collect();
        doc.arrange(table, &with_extras(&original, rs, &extras))
            .map_err(unbuilt)?;
        if self.resized.is_some() {
            if matches!(&doc.n(table).kind,NodeKind::Table(info) if info.grid != widths) {
                doc.compose_grid(table, &widths).map_err(unbuilt)?;
            }
            shape
                .cols
                .insert(table, cols.iter().map(|c| c.0.clone()).collect());
        }
        Ok(())
    }
}
