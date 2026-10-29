//! Version 3 stores a bounded, topologically ordered immutable value graph.
//! Shapes belong to references, so reshaped aliases share one decoded allocation.
use super::*;
use crate::{
    containers::{CellArray, SLOT},
    script_containers::MAX_CELL_SLOTS,
};
pub(super) const MAX_NODES: usize = MAX_CELL_SLOTS + VARIABLES;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Graph {
    pub roots: BTreeMap<String, Edge>,
    pub nodes: Vec<Node>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graphics: Option<graphics::Table>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edge {
    pub node: usize,
    pub rows: usize,
    pub cols: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum Node {
    Leaf { name: String },
    Cell { children: Vec<Edge> },
    Graphics { handles: Vec<usize> },
}

pub(super) fn character_len(rows: usize, cols: usize) -> Result<usize> {
    crate::CharacterRow::checked_len(rows, cols)
}
pub(super) fn canonical_character_shape(rows: usize, cols: usize) -> bool {
    rows == usize::from(cols != 0)
}
#[derive(Clone, Copy)]
struct Cost {
    bytes: usize,
    slots: usize,
    depth: usize,
    len: usize,
    kind: u8,
}
fn edge_cost(edge: &Edge, costs: &[Cost]) -> Result<Cost> {
    let cost = *costs
        .get(edge.node)
        .ok_or("Missing or forward workspace graph reference")?;
    let len = match cost.kind {
        2 => crate::ComplexMatrix::checked_len(edge.rows, edge.cols)?,
        3 => character_len(edge.rows, edge.cols)?,
        4 | 5 => {
            let count = edge
                .rows
                .checked_mul(edge.cols)
                .ok_or("Cell recovery shape overflow")?;
            if edge.rows > MAX_CELL_SLOTS || edge.cols > MAX_CELL_SLOTS || count > MAX_CELL_SLOTS {
                return Err("Cell recovery shape exceeds limit".into());
            }
            count
        }
        _ => Matrix::checked_len(edge.rows, edge.cols)?,
    };
    if len != cost.len {
        return Err("Workspace graph alias shape changes element count".into());
    }
    Ok(cost)
}
impl Graph {
    // Metadata only: no UTF16/numeric payload scans or shape-dependent allocation.
    pub(super) fn shaped_characters(
        &self,
        is_character: impl Fn(&str) -> bool,
        cancel: &AtomicBool,
    ) -> Result<bool> {
        let check_edge = |edge: &Edge| -> Result<bool> {
            let node = self
                .nodes
                .get(edge.node)
                .ok_or("Missing character shape node")?;
            Ok(matches!(node, Node::Leaf { name } if is_character(name))
                && !canonical_character_shape(edge.rows, edge.cols))
        };
        for edge in self.roots.values() {
            checkpoint(cancel)?;
            if check_edge(edge)? {
                return Ok(true);
            }
        }
        for node in &self.nodes {
            checkpoint(cancel)?;
            if let Node::Cell { children } = node {
                for edge in children {
                    checkpoint(cancel)?;
                    if check_edge(edge)? {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }
    pub(super) fn validate(
        &self,
        leaves: &BTreeMap<String, Reference>,
        cancel: &AtomicBool,
    ) -> Result<()> {
        if self.roots.len() > VARIABLES || self.nodes.len() > MAX_NODES {
            return Err("Workspace graph count exceeds limit".into());
        }
        if let Some(table) = &self.graphics {
            table.validate(cancel)?;
        }
        let mut used_handles = vec![false; self.graphics.as_ref().map_or(0, graphics::Table::len)];
        let mut graphics_slots = 0usize;
        let mut has_graphics = false;
        let mut costs = Vec::new();
        costs
            .try_reserve_exact(self.nodes.len())
            .map_err(|_| "Cannot allocate recovery validation metadata")?;
        let mut used_leaves = BTreeSet::new();
        let mut edges = 0usize;
        for node in &self.nodes {
            checkpoint(cancel)?;
            let cost = match node {
                Node::Leaf { name } => {
                    let r = leaves.get(name).ok_or("Missing workspace graph leaf")?;
                    if !used_leaves.insert(name) {
                        return Err("Duplicate workspace graph leaf node".into());
                    }
                    Cost {
                        bytes: r.bytes,
                        slots: 0,
                        depth: 0,
                        len: r.rows * r.cols,
                        kind: if r.character {
                            3
                        } else if r.complex {
                            2
                        } else {
                            u8::from(r.logical)
                        },
                    }
                }
                Node::Graphics { handles } => {
                    has_graphics = true;
                    if self.graphics.is_none() || handles.len() > MAX_CELL_SLOTS {
                        return Err("Invalid graphics recovery node".into());
                    }
                    graphics_slots = graphics_slots
                        .checked_add(handles.len())
                        .ok_or("Graphics reference overflow")?;
                    self.graphics
                        .as_ref()
                        .unwrap()
                        .validate_budget(graphics_slots)?;
                    for &id in handles {
                        checkpoint(cancel)?;
                        *used_handles
                            .get_mut(id)
                            .ok_or("Missing graphics handle record")? = true;
                    }
                    Cost {
                        bytes: handles.len() * graphics::HANDLE_BYTES,
                        slots: 0,
                        depth: 0,
                        len: handles.len(),
                        kind: 5,
                    }
                }
                Node::Cell { children } => {
                    edges = edges
                        .checked_add(children.len())
                        .ok_or("Workspace graph edge overflow")?;
                    if edges > MAX_CELL_SLOTS {
                        return Err("Workspace graph exceeds 65536 unique child references".into());
                    }
                    let mut cost = Cost {
                        bytes: children.len() * SLOT,
                        slots: children.len(),
                        depth: 1,
                        len: children.len(),
                        kind: 4,
                    };
                    for (i, child) in children.iter().enumerate() {
                        if i.is_multiple_of(1024) {
                            checkpoint(cancel)?;
                        }
                        let child = edge_cost(child, &costs)?;
                        cost.bytes = cost
                            .bytes
                            .checked_add(child.bytes)
                            .ok_or("Workspace graph byte overflow")?;
                        cost.slots = cost
                            .slots
                            .checked_add(child.slots)
                            .ok_or("Workspace graph slot overflow")?;
                        cost.depth = cost.depth.max(child.depth + 1);
                        if cost.bytes > LIMIT || cost.slots > MAX_CELL_SLOTS || cost.depth > 16 {
                            return Err(
                                "Cell recovery exceeds logical byte, slot or depth limit".into()
                            );
                        }
                    }
                    cost
                }
            };
            costs.push(cost);
        }
        if has_graphics != self.graphics.is_some() || used_handles.iter().any(|used| !*used) {
            return Err("Missing or unreferenced graphics table".into());
        }
        if used_leaves.len() != leaves.len() {
            return Err("Unreferenced workspace graph leaf".into());
        }
        let mut total = 0usize;
        let mut reachable = vec![false; self.nodes.len()];
        for (name, edge) in &self.roots {
            checkpoint(cancel)?;
            validate_name(name)?;
            let cost = edge_cost(edge, &costs)?;
            total = total
                .checked_add(cost.bytes)
                .ok_or("Workspace recovery size overflow")?;
            if total > LIMIT {
                return Err("Workspace recovery exceeds 256 MiB logical storage".into());
            }
            reachable[edge.node] = true;
        }
        // Edges are strictly backward: one reverse traversal reaches every descendant.
        for (i, node) in self.nodes.iter().enumerate().rev() {
            checkpoint(cancel)?;
            if reachable[i]
                && let Node::Cell { children } = node
            {
                for (j, child) in children.iter().enumerate() {
                    if j.is_multiple_of(1024) {
                        checkpoint(cancel)?;
                    }
                    reachable[child.node] = true;
                }
            }
        }
        if reachable.iter().any(|b| !b) {
            return Err("Unreachable workspace graph node".into());
        }
        check_cancel(cancel)
    }

    pub(super) fn restore(
        &self,
        mut leaves: Variables,
        handles: &[Arc<crate::GraphicsHandle>],
        cancel: &AtomicBool,
    ) -> Result<Variables> {
        let mut values = Vec::new();
        values
            .try_reserve_exact(self.nodes.len())
            .map_err(|_| "Cannot allocate recovery graph")?;
        for node in &self.nodes {
            checkpoint(cancel)?;
            let value = match node {
                Node::Leaf { name } => {
                    leaves.remove(name).ok_or("Missing decoded recovery leaf")?
                }
                Node::Graphics { handles: indices } => {
                    let mut slots = Vec::new();
                    slots
                        .try_reserve_exact(indices.len())
                        .map_err(|_| "Cannot allocate recovered graphics slots")?;
                    for &index in indices {
                        checkpoint(cancel)?;
                        slots.push(handles[index].clone());
                    }
                    crate::GraphicsArray::from_shared(1, indices.len(), slots, cancel)?.into()
                }
                Node::Cell { children } => {
                    let mut slots = Vec::new();
                    slots
                        .try_reserve_exact(children.len())
                        .map_err(|_| "Cannot allocate recovered cell slots")?;
                    for (i, child) in children.iter().enumerate() {
                        if i.is_multiple_of(1024) {
                            checkpoint(cancel)?;
                        }
                        slots.push(Arc::new(reshape(&values[child.node], child, cancel)?));
                    }
                    CellArray::from_shared(1, children.len(), slots, cancel)?.into()
                }
            };
            values.push(value);
        }
        let mut result = BTreeMap::new();
        for (name, edge) in &self.roots {
            checkpoint(cancel)?;
            result.insert(name.clone(), reshape(&values[edge.node], edge, cancel)?);
        }
        check_cancel(cancel)?;
        Ok(result)
    }
}

fn reshape(value: &Array, edge: &Edge, cancel: &AtomicBool) -> Result<Array> {
    check_cancel(cancel)?;
    Ok(match value {
        Array::Numeric(a) => a.reshape(edge.rows, edge.cols)?.into(),
        Array::Logical(a) => a.reshape(edge.rows, edge.cols)?.into(),
        Array::Complex(a) => a.reshape(edge.rows, edge.cols, cancel)?.into(),
        Array::Character(a) => a.reshape(edge.rows, edge.cols, cancel)?.into(),
        Array::Cell(a) => a.reshape(edge.rows, edge.cols, cancel)?.into(),
        Array::Graphics(a) => a.reshape_recovery(edge.rows, edge.cols, cancel)?.into(),
        Array::Struct(a) => a.reshape(edge.rows, edge.cols, cancel)?.into(),
        Array::Handle(_) => {
            return Err("Session recovery of function handles is not supported yet".into());
        }
        Array::Typed(a) => a.reshaped(edge.rows, edge.cols)?.into(),
    })
}

pub(super) fn plan(
    values: Variables,
    figures: Option<(
        &super::super::figures::FigureSession,
        &super::super::figures::FigureManifest,
    )>,
    cancel: &AtomicBool,
) -> Result<(Variables, Option<Graph>)> {
    // No identity index is built unless traversal encounters an actual Graphics node.
    let mut needs_graph = false;
    for value in values.values() {
        check_cancel(cancel)?;
        if !value.is_numeric_array() {
            needs_graph = true;
        }
    }
    if !needs_graph {
        return Ok((values, None));
    }
    struct Planner<'a> {
        graph: Graph,
        leaves: Variables,
        identities: BTreeMap<(u8, usize), usize>,
        edges: usize,
        figures: Option<(
            &'a super::super::figures::FigureSession,
            &'a super::super::figures::FigureManifest,
        )>,
        graphics: Option<graphics::Planner>,
    }
    impl Planner<'_> {
        fn visit(&mut self, value: &Array, depth: usize, cancel: &AtomicBool) -> Result<Edge> {
            checkpoint(cancel)?;
            if depth > 16 {
                return Err("Recovery cell nesting exceeds 16 levels".into());
            }
            let key = match value {
                Array::Cell(a) => (4, a.weak_storage().as_ptr() as usize),
                Array::Numeric(_)
                | Array::Logical(_)
                | Array::Complex(_)
                | Array::Character(_)
                | Array::Typed(_) => WeakArray::new(value).key(),
                Array::Graphics(a) => (5, a.recovery_key()),
                Array::Struct(_) => {
                    return Err("Workspace recovery of struct values is not supported yet".into());
                }
                Array::Handle(_) => {
                    return Err("Session recovery of function handles is not supported yet".into());
                }
            };
            let node = if let Some(&node) = self.identities.get(&key) {
                node
            } else {
                let node = if let Array::Graphics(a) = value {
                    if self.graphics.is_none() {
                        let (figures, manifest) = self.figures.ok_or(
                            "Recovering graphics handles requires a complete figure snapshot",
                        )?;
                        self.graphics = Some(graphics::Planner::new(figures, manifest, cancel)?);
                    }
                    self.graphics.as_mut().unwrap().add_slots(a.len())?;
                    let mut handles = Vec::new();
                    handles
                        .try_reserve_exact(a.len())
                        .map_err(|_| "Cannot allocate graphics graph node")?;
                    for i in 0..a.len() {
                        handles.push(
                            self.graphics
                                .as_mut()
                                .unwrap()
                                .handle(a.get_linear(i)?, cancel)?,
                        );
                    }
                    Node::Graphics { handles }
                } else if let Array::Cell(a) = value {
                    self.edges = self
                        .edges
                        .checked_add(a.len())
                        .ok_or("Workspace graph edge overflow")?;
                    if self.edges > MAX_CELL_SLOTS {
                        return Err("Workspace graph exceeds 65536 unique child references".into());
                    }
                    let mut children = Vec::new();
                    children
                        .try_reserve_exact(a.len())
                        .map_err(|_| "Cannot allocate recovery plan")?;
                    for i in 0..a.len() {
                        children.push(self.visit(a.get_linear(i)?, depth + 1, cancel)?);
                    }
                    Node::Cell { children }
                } else {
                    let name = format!("leaf_{}", self.leaves.len());
                    self.leaves.insert(name.clone(), value.clone());
                    Node::Leaf { name }
                };
                if self.graph.nodes.len() == MAX_NODES {
                    return Err("Workspace graph exceeds node limit".into());
                }
                let index = self.graph.nodes.len();
                self.graph.nodes.push(node);
                self.identities.insert(key, index);
                index
            };
            Ok(Edge {
                node,
                rows: value.rows(),
                cols: value.cols(),
            })
        }
    }
    let mut plan = Planner {
        graph: Graph {
            roots: BTreeMap::new(),
            nodes: Vec::new(),
            graphics: None,
        },
        leaves: BTreeMap::new(),
        identities: BTreeMap::new(),
        edges: 0,
        figures,
        graphics: None,
    };
    for (name, value) in &values {
        validate_name(name)?;
        let edge = plan.visit(value, 0, cancel)?;
        plan.graph.roots.insert(name.clone(), edge);
    }
    check_cancel(cancel)?;
    plan.graph.graphics = plan.graphics.map(|p| p.table);
    Ok((plan.leaves, Some(plan.graph)))
}
