//! Table geometry, command validation, and property updates used by the grid reader.
//!
//! Columns are the table grid's (`w:tblGrid`); a cell covers as many as its
//! `w:gridSpan` says. A new column halves the width of the column it copies
//! and a removed column gives its width to a neighbour, so the table keeps
//! its width and every cell's `w:tcW` stays the sum of its columns.

use crate::error::EditError;
use crate::format::{rewrite, without};
use crate::import::{cell_geometry, cell_width, grid_columns};
use crate::ir::{CellInfo, Made, NodeId, NodeKind, ParagraphInfo, RowInfo, Support};
use crate::structure::refuse;
use crate::Document;

const TCPR_ORDER: [&str; 18] = [
    "cnfStyle",
    "tcW",
    "gridSpan",
    "hMerge",
    "vMerge",
    "tcBorders",
    "shd",
    "noWrap",
    "tcMar",
    "textDirection",
    "tcFitText",
    "vAlign",
    "hideMark",
    "headers",
    "cellIns",
    "cellDel",
    "cellMerge",
    "tcPrChange",
];

/// The `w:tcPr` children a cell step may change.
const CELL: [&str; 3] = ["tcW", "gridSpan", "vMerge"];
/// Tracked changes, and Word's cache of the conditional formatting that
/// applies, which a copy in another place must not carry.
pub(crate) const REVISIONS: [&str; 11] = [
    "ins",
    "del",
    "moveFrom",
    "moveTo",
    "rPrChange",
    "pPrChange",
    "trPrChange",
    "tcPrChange",
    "cellIns",
    "cellDel",
    "cellMerge",
];

/// A cell in a table whose rows each fill its grid.
#[derive(Debug, Clone, Copy)]
struct Slot {
    cell: NodeId,
    start: usize,
    span: usize,
}

/// A cell's grid geometry, including cells preserved inside an opaque owner.
#[derive(Clone, Copy)]
pub(crate) struct CellSlot<'a> {
    pub owner: NodeId,
    pub start: usize,
    pub span: usize,
    pub merge: Option<&'a str>,
}

const VACANT_SLOT: Support = Support::ReadOnly("this table slot is vacant");

impl Document {
    pub(crate) fn check_row_copy(&self, row: NodeId, like: NodeId) -> Result<NodeId, EditError> {
        let table = self.parent_of(row)?;
        self.expect_kind(row, matches!(self.n(row).kind, NodeKind::Row(_)), "a row")?;
        self.attached(like)?;
        self.expect_kind(like, matches!(self.n(like).kind, NodeKind::Row(_)), "a row")?;
        if self.n(like).parent != Some(table) {
            return refuse(like, "it is a row of another table");
        }
        self.check_editable(table)?;
        self.check_copyable_row(like)?;
        Ok(table)
    }

    pub(crate) fn check_column(
        &self,
        table: NodeId,
        index: usize,
        donor: Option<usize>,
    ) -> Result<(), EditError> {
        let (rows, widths) = self.grid_of(table)?;
        let len = widths.len();
        match donor {
            Some(like) if index > len || like >= len => {
                return refuse(table, "the table grid has no such column")
            }
            None if index >= len => return refuse(table, "the table grid has no such column"),
            None if len == 1 => {
                return refuse(
                    table,
                    "a table keeps at least one column; remove the table instead",
                )
            }
            None => {
                let removed = rows
                    .iter()
                    .flat_map(|(_, cells)| cells)
                    .filter(|c| c.start == index && c.span == 1)
                    .map(|c| c.cell)
                    .collect::<Vec<_>>();
                self.check_contained(&removed)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Refuses removing a row that leaves its table without rows, or whose
    /// cell starts a vertical merge that continues below it.
    pub(crate) fn check_row_removal(&self, row: NodeId) -> Result<(), EditError> {
        let table = self.parent_of(row)?;
        let rows: Vec<NodeId> = self.rows(table).collect();
        if rows.len() == 1 {
            return refuse(
                row,
                "a table keeps at least one row; remove the table instead",
            );
        }
        self.check_merge_start_removal(row, None)
    }

    /// Whether a cell is part of a vertical merge: its start, or a
    /// continuation that reaches one through merged cells of the same
    /// columns directly above it. A continuation mark with no start above
    /// it merges nothing, as when a wrapper keeps its mark after a cut.
    pub(crate) fn merge_start_row(
        &self,
        table: NodeId,
        row: NodeId,
        cell: &CellSlot<'_>,
    ) -> Option<NodeId> {
        match cell.merge {
            Some("restart") => return Some(row),
            Some("continue") => {}
            _ => return None,
        }
        let children = &self.n(table).children;
        let at = children.iter().position(|&id| id == row)?;
        for &above in children[..at].iter().rev() {
            if !matches!(self.n(above).kind, NodeKind::Row(_)) {
                continue;
            }
            let covering = self
                .row_geometry(above)
                .into_iter()
                .find(|c| c.start == cell.start && c.span == cell.span)?;
            match covering.merge {
                Some("restart") => return Some(above),
                Some("continue") => {}
                _ => return None,
            }
        }
        None
    }

    pub(crate) fn check_merge_start_removal(
        &self,
        row: NodeId,
        owner: Option<NodeId>,
    ) -> Result<(), EditError> {
        let table = self.parent_of(row)?;
        let Some(next) = self.rows(table).skip_while(|r| *r != row).nth(1) else {
            return Ok(());
        };
        let continued = self.row_geometry(next);
        for cell in self.row_geometry(row) {
            let below = continued.iter().any(|c| {
                c.start == cell.start && c.span == cell.span && c.merge == Some("continue")
            });
            if !below
                || owner.is_some_and(|n| n != cell.owner)
                || self.merge_start_row(table, row, &cell).is_none()
            {
                continue;
            }
            // Removing a whole row closes the gap it leaves. Removing a
            // wrapper leaves its slots in place, so a merge passing through
            // it would be cut as well.
            match (cell.merge, owner) {
                (Some("restart"), _) => {
                    return refuse(
                        cell.owner,
                        "a merged cell starts in this row and continues below it",
                    )
                }
                (Some("continue"), Some(_)) => {
                    return refuse(
                        cell.owner,
                        "a merged cell runs through this element and continues below it",
                    )
                }
                _ => (),
            }
        }
        Ok(())
    }

    fn compose_cell_properties(
        &mut self,
        id: NodeId,
        properties: &str,
    ) -> Result<String, EditError> {
        let parsed =
            self.read_markup(id, properties, |p| (cell_geometry(Some(p)), cell_width(p)))?;
        let (geometry, width) = parsed.unwrap_or_else(|| (cell_geometry(None), None));
        let current = self.replace_properties(
            id,
            "tcPr",
            properties,
            |scope, e| scope.local(e).is_some_and(|n| CELL.contains(&n)),
            "only a cell's width, grid span and vertical merge can change this way",
        )?;
        if let NodeKind::Cell(info) = &mut self.nodes[id.index()].kind {
            info.grid_span = geometry.grid_span;
            info.vertical_merge = geometry.vertical_merge;
            info.width = width;
        }
        Ok(current)
    }

    pub(crate) fn set_table_grid(&mut self, id: NodeId, grid: &str) -> Result<(), EditError> {
        let table = matches!(self.n(id).kind, NodeKind::Table(_));
        self.expect_kind(id, table, "a table")?;
        self.check_editable(id)?;
        let columns = self
            .read_markup(id, grid, grid_columns)?
            .unwrap_or_default();
        self.replace_properties(
            id,
            "tblGrid",
            grid,
            |scope, e| scope.is(e, "gridCol"),
            "only a table's grid columns can change this way",
        )?;
        if let NodeKind::Table(info) = &mut self.nodes[id.index()].kind {
            info.grid = columns;
        }
        Ok(())
    }

    /// Refuses a node that is detached, not editable, or not `expected`.
    fn expect_kind(&self, id: NodeId, is: bool, expected: &'static str) -> Result<(), EditError> {
        self.attached(id)?;
        if !is {
            return Err(EditError::WrongKind {
                node: id,
                kind: self.n(id).kind.name(),
                expected,
            });
        }
        Ok(())
    }

    /// Refuses a row a copy could not repeat: one with a content control or
    /// other element around its cells.
    pub(crate) fn check_copyable_row(&self, row: NodeId) -> Result<(), EditError> {
        for &c in &self.n(row).children {
            if !matches!(self.n(c).kind, NodeKind::Cell(_) | NodeKind::Marker { .. }) {
                return refuse(
                    c,
                    "the row holds a content control or other element around its cells",
                );
            }
        }
        Ok(())
    }

    pub(crate) fn rows(&self, table: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.n(table)
            .children
            .iter()
            .copied()
            .filter(|&c| matches!(self.n(c).kind, NodeKind::Row(_)))
    }

    /// One geometry reading for command checks and semantic materialization.
    pub(crate) fn row_geometry(&self, row: NodeId) -> Vec<CellSlot<'_>> {
        let NodeKind::Row(info) = &self.n(row).kind else {
            return Vec::new();
        };
        let mut start = info.grid_before as usize;
        let mut out = Vec::new();
        for &owner in &self.n(row).children {
            let node = self.n(owner);
            if let NodeKind::Cell(cell) = &node.kind {
                let span = cell.grid_span.max(1) as usize;
                out.push(CellSlot {
                    owner,
                    start,
                    span,
                    merge: cell.vertical_merge.as_deref(),
                });
                start += span;
            } else if let Some(cells) = &node.preserved_cells {
                for cell in cells.iter() {
                    let span = cell.grid_span.max(1) as usize;
                    out.push(CellSlot {
                        owner,
                        start,
                        span,
                        merge: cell.vertical_merge.as_deref(),
                    });
                    start += span;
                }
            }
        }
        out
    }

    /// Constructible cells only; callers that modify them check row policy.
    fn row_slots(&self, row: NodeId) -> Vec<Slot> {
        self.row_geometry(row)
            .into_iter()
            .filter(|s| matches!(self.n(s.owner).kind, NodeKind::Cell(_)))
            .map(|s| Slot {
                cell: s.owner,
                start: s.start,
                span: s.span,
            })
            .collect()
    }

    /// Each row with its cells, and the grid, for a table whose columns a
    /// command can change: every row fills the grid, and nothing in it is a
    /// tracked change to the grid or a cell's properties.
    #[allow(clippy::type_complexity)]
    fn grid_of(&self, table: NodeId) -> Result<(Vec<(NodeId, Vec<Slot>)>, Vec<u32>), EditError> {
        let NodeKind::Table(info) = &self.attached(table)?.kind else {
            return Err(EditError::WrongKind {
                node: table,
                kind: self.n(table).kind.name(),
                expected: "a table",
            });
        };
        self.check_editable(table)?;
        let grid = info.grid.clone();
        if grid.is_empty() {
            return refuse(table, "the table has no grid");
        }
        let name = self.w_name(table, "tblGrid");
        let prefix = name.strip_suffix("tblGrid").unwrap_or_default();
        let head = self.head(table);
        if head.contains(&format!("<{prefix}tblGridChange")) {
            return refuse(
                table,
                "the table grid has a tracked change; accept or reject it first",
            );
        }
        let mut rows = Vec::new();
        for &c in &self.n(table).children {
            match &self.n(c).kind {
                NodeKind::Row(_) => {}
                NodeKind::Marker { .. } => continue,
                _ => {
                    return refuse(
                        c,
                        "the table holds a content control or other element around its rows",
                    )
                }
            }
            self.check_copyable_row(c)?;
            let slots = self.row_slots(c);
            let filled = slots.last().map_or(0, |s| s.start + s.span);
            let NodeKind::Row(row) = &self.n(c).kind else {
                unreachable!("a row");
            };
            if row.grid_before > 0 || row.grid_after > 0 || filled != grid.len() {
                return refuse(c, "a row does not fill the table grid");
            }
            for s in &slots {
                self.check_editable(s.cell)?;
                let tcpr = self.properties(s.cell, "tcPr");
                if REVISIONS
                    .iter()
                    .any(|r| tcpr.contains(&format!("<{prefix}{r}")))
                {
                    return refuse(
                        s.cell,
                        "a cell has a tracked change; accept or reject it first",
                    );
                }
            }
            rows.push((c, slots));
        }
        Ok((rows, grid))
    }

    /// A cell's `w:tcPr` with a new grid span, width or vertical merge.
    /// `None` leaves that property as it is.
    fn cell_markup(
        &self,
        cell: NodeId,
        span: Option<usize>,
        width: Option<u32>,
        merge: Option<&str>,
    ) -> String {
        let name = self.w_name(cell, "tcPr");
        let prefix = name.strip_suffix("tcPr").unwrap_or_default().to_string();
        let val = self.w_attr_name(cell, "val");
        let scope = &self.scope(cell);
        let mut tcpr = self.properties(cell, "tcPr");
        if let Some(width) = width {
            let w = self.w_attr_name(cell, "w");
            // An unprefixed cell declares w on the first attribute.
            let attr_prefix = if prefix.is_empty() { "w:" } else { &prefix };
            let set = vec![format!(
                "<{prefix}tcW {w}=\"{width}\" {attr_prefix}type=\"dxa\"/>"
            )];
            tcpr = rewrite(&tcpr, scope, &name, &TCPR_ORDER, &["tcW"], set);
        }
        if let Some(span) = span {
            let set = (span > 1).then(|| vec![format!("<{prefix}gridSpan {val}=\"{span}\"/>")]);
            tcpr = rewrite(
                &tcpr,
                scope,
                &name,
                &TCPR_ORDER,
                &["gridSpan"],
                set.unwrap_or_default(),
            );
        }
        if let Some(merge) = merge {
            let set = match merge {
                "restart" => vec![format!("<{prefix}vMerge {val}=\"restart\"/>")],
                "none" => Vec::new(),
                _ => vec![format!("<{prefix}vMerge/>")],
            };
            tcpr = rewrite(&tcpr, scope, &name, &TCPR_ORDER, &["vMerge"], set);
        }
        tcpr
    }

    /// A table's `w:tblGrid` with these column widths.
    fn grid_markup(&self, table: NodeId, widths: &[u32]) -> String {
        let name = self.w_name(table, "tblGrid");
        let prefix = name.strip_suffix("tblGrid").unwrap_or_default();
        let w = self.w_attr_name(table, "w");
        let columns: Vec<String> = widths
            .iter()
            .map(|width| format!("<{prefix}gridCol {w}=\"{width}\"/>"))
            .collect();
        let current = self.properties(table, "tblGrid");
        rewrite(
            &current,
            &self.scope(table),
            &name,
            &["gridCol"],
            &["gridCol"],
            columns,
        )
    }

    pub(crate) fn empty_row_like(&mut self, like: NodeId, info: RowInfo) -> NodeId {
        let story = self.n(like).story;
        let name = self.element_name(like);
        let tail = format!("</{name}>");
        let exceptions = self.properties(like, "tblPrEx");
        let trpr = self.properties(like, "trPr");
        let mut stripped = REVISIONS.to_vec();
        stripped.extend(["cnfStyle", "tblHeader"]);
        let scope = self.scope(like);
        let trpr = without(&trpr, &scope, &stripped);
        let head = format!(
            "<{name}{}>{exceptions}{trpr}",
            scope.detached_declarations()
        );
        let info = RowInfo {
            revision: None,
            header: false,
            ..info
        };
        self.create(
            NodeKind::Row(info),
            Support::Editable,
            Made {
                head,
                tail,
                like: None,
            },
            story,
        )
    }

    /// A new, detached, empty copy of a row or cell, built without the
    /// journal: only attaching it is a change an editor draws.
    pub(crate) fn empty_copy(&mut self, like: NodeId) -> NodeId {
        let story = self.n(like).story;
        let name = self.element_name(like);
        let tail = format!("</{name}>");
        match self.n(like).kind.clone() {
            NodeKind::Row(info) => {
                let row = self.empty_row_like(like, info);
                let cells: Vec<NodeId> = self.n(like).children.clone();
                for cell in cells {
                    if matches!(self.n(cell).kind, NodeKind::Cell(_)) {
                        let copy = self.empty_copy(cell);
                        self.adopt(row, copy);
                    }
                }
                row
            }
            NodeKind::Cell(info) => {
                let tcpr = self.properties(like, "tcPr");
                let mut stripped = REVISIONS.to_vec();
                stripped.extend(["cnfStyle", "vMerge"]);
                let scope = self.scope(like);
                let tcpr = without(&tcpr, &scope, &stripped);
                let head = format!("<{name}{}>{tcpr}", scope.detached_declarations());
                let info = CellInfo {
                    vertical_merge: None,
                    ..info
                };
                let cell = self.create(
                    NodeKind::Cell(info),
                    Support::Editable,
                    Made {
                        head,
                        tail,
                        like: None,
                    },
                    story,
                );
                let first = self
                    .n(like)
                    .children
                    .iter()
                    .copied()
                    .find(|&c| matches!(self.n(c).kind, NodeKind::Paragraph(_)));
                let paragraph = self.empty_paragraph(first, cell);
                self.adopt(cell, paragraph);
                cell
            }
            _ => unreachable!("copy_node copies rows and cells"),
        }
    }

    pub(crate) fn compose_grid(&mut self, table: NodeId, widths: &[u32]) -> Result<(), EditError> {
        let grid = self.grid_markup(table, widths);
        self.set_table_grid(table, &grid)?;
        Ok(())
    }

    /// Change only omitted grid slots, preserving every other row property.
    pub(crate) fn compose_row_gaps(
        &mut self,
        row: NodeId,
        before: usize,
        after: usize,
    ) -> Result<(), EditError> {
        let NodeKind::Row(info) = &self.n(row).kind else {
            unreachable!()
        };
        if info.grid_before as usize == before && info.grid_after as usize == after {
            return Ok(());
        }
        let name = self.w_name(row, "trPr");
        let prefix = name.strip_suffix("trPr").unwrap();
        let val = self.w_attr_name(row, "val");
        let set = [("gridBefore", before), ("gridAfter", after)]
            .into_iter()
            .filter(|(_, n)| *n != 0)
            .map(|(tag, n)| format!("<{prefix}{tag} {val}=\"{n}\"/>"))
            .collect();
        let order = [
            "cnfStyle",
            "divId",
            "gridBefore",
            "gridAfter",
            "wBefore",
            "wAfter",
            "cantSplit",
            "trHeight",
            "tblHeader",
            "tblCellSpacing",
            "jc",
            "hidden",
            "ins",
            "del",
            "trPrChange",
        ];
        let props = rewrite(
            &self.properties(row, "trPr"),
            &self.scope(row),
            &name,
            &order,
            &["gridBefore", "gridAfter"],
            set,
        );
        self.replace_properties(
            row,
            "trPr",
            &props,
            |scope, e| matches!(scope.local(e), Some("gridBefore" | "gridAfter")),
            "only the omitted grid slots may change",
        )?;
        if let NodeKind::Row(info) = &mut self.nodes[row.index()].kind {
            info.grid_before = before as u32;
            info.grid_after = after as u32;
        }
        Ok(())
    }

    pub(crate) fn bare_cell(&mut self, row: NodeId) -> NodeId {
        let name = self.w_name(row, "tc");
        let cell = self.create(
            NodeKind::Cell(CellInfo {
                grid_span: 1,
                vertical_merge: None,
                width: None,
                props: Default::default(),
            }),
            Support::Editable,
            Made {
                head: format!("<{name}>"),
                tail: format!("</{name}>"),
                like: None,
            },
            self.n(row).story,
        );
        let p = self.empty_paragraph(None, cell);
        self.adopt(cell, p);
        cell
    }

    pub(crate) fn vacant_cell(&mut self, row: NodeId) -> NodeId {
        let cell = self.bare_cell(row);
        let p = self.n(cell).children[0];
        self.nodes[p.index()].support = VACANT_SLOT;
        cell
    }
    /// The atom is indivisible. Its removed cells leave the same grid spans,
    /// including on source tables whose grid omits explicit column widths.
    pub(crate) fn wrapper_vacancies(
        &mut self,
        row: NodeId,
        atom: NodeId,
    ) -> Result<Vec<NodeId>, EditError> {
        let spans: Vec<_> = self
            .n(atom)
            .preserved_cells
            .iter()
            .flat_map(|cs| cs.iter().map(|c| c.grid_span.max(1)))
            .collect();
        let mut blanks = Vec::new();
        for span in spans {
            let blank = self.vacant_cell(row);
            self.write_grid_cell(blank, span as usize, 0, None, false)?;
            blanks.push(blank);
        }
        Ok(blanks)
    }
    pub(crate) fn is_vacant_cell(&self, id: NodeId) -> bool {
        self.n(id)
            .children
            .first()
            .is_some_and(|p| self.n(*p).support == VACANT_SLOT)
    }
    /// `continued` says whether the cell continues a vertical merge, or
    /// leaves that as the cell has it.
    pub(crate) fn write_grid_cell(
        &mut self,
        id: NodeId,
        span: usize,
        width: u32,
        continued: Option<bool>,
        width_changed: bool,
    ) -> Result<(), EditError> {
        let NodeKind::Cell(cell) = &self.n(id).kind else {
            unreachable!()
        };
        let span = (cell.grid_span.max(1) as usize != span).then_some(span);
        let width = cell
            .width
            .filter(|old| width_changed && *old != width)
            .map(|_| width);
        let merge = match (continued, cell.vertical_merge.as_deref()) {
            (Some(true), Some("continue")) | (Some(false), None | Some("restart")) | (None, _) => {
                None
            }
            (Some(true), _) => Some("continue"),
            (Some(false), _) => Some("none"),
        };
        if span.is_some() || width.is_some() || merge.is_some() {
            let props = self.cell_markup(id, span, width, merge);
            self.compose_cell_properties(id, &props)?;
        }
        Ok(())
    }
    /// An empty paragraph with the properties of `like`, without its
    /// tracked changes, or a bare one.
    pub(crate) fn empty_paragraph(&mut self, like: Option<NodeId>, near: NodeId) -> NodeId {
        let story = self.n(near).story;
        let name = self.w_name(like.unwrap_or(near), "p");
        let open = format!("<{name}>");
        let (info, head) = match like {
            Some(like) => {
                let (info, props) = self.paragraph_like(like, false);
                // Later edits can reuse the template's raw paragraph properties.
                // Bind their prefixes on the paragraph that receives them.
                let open = self.scope(like).carried_to(&self.scope(near), &open);
                (info, format!("{open}{props}"))
            }
            None => (ParagraphInfo::default(), open),
        };
        let made = Made {
            head,
            tail: format!("</{name}>"),
            like: like.and_then(|like| self.like(like)),
        };
        self.create(
            NodeKind::Paragraph(Box::new(info)),
            Support::Editable,
            made,
            story,
        )
    }

    pub(crate) fn adopt(&mut self, parent: NodeId, child: NodeId) {
        self.nodes[parent.index()].children.push(child);
        self.nodes[child.index()].parent = Some(parent);
    }
}
