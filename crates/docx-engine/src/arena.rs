//! Copy-on-write pages for the derived tree. Preparing a command shares
//! untouched nodes with the displayed document; only written pages copy.
use crate::ir::{Node, NodeId};
use std::{
    ops::{Index, IndexMut},
    sync::Arc,
};
const PAGE: usize = 64;
#[derive(Clone, Default)]
pub(crate) struct Nodes {
    pages: Vec<Arc<Vec<Node>>>,
    len: usize,
    vacant: Vec<NodeId>,
}
impl Nodes {
    pub fn iter(&self) -> impl Iterator<Item = &Node> {
        self.pages.iter().flat_map(|p| p.iter())
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn get(&self, index: usize) -> Option<&Node> {
        self.pages.get(index / PAGE)?.get(index % PAGE)
    }
    pub fn next_id(&self) -> NodeId {
        self.vacant
            .last()
            .copied()
            .unwrap_or(NodeId(self.len as u32))
    }
    /// The materializer has removed this derived node from the visible tree.
    pub fn release(&mut self, id: NodeId) {
        self[id.index()].parent = None;
        self[id.index()].children.clear();
        self.vacant.push(id);
    }
    pub fn push(&mut self, node: Node) {
        if let Some(id) = self.vacant.pop() {
            debug_assert_eq!(id, node.id);
            self[id.index()] = node;
            return;
        }
        if self.len.is_multiple_of(PAGE) {
            self.pages.push(Arc::new(Vec::with_capacity(PAGE)));
        }
        Arc::make_mut(self.pages.last_mut().unwrap()).push(node);
        self.len += 1;
    }
}
impl Index<usize> for Nodes {
    type Output = Node;
    fn index(&self, index: usize) -> &Node {
        self.get(index).expect("node index")
    }
}
impl IndexMut<usize> for Nodes {
    fn index_mut(&mut self, index: usize) -> &mut Node {
        &mut Arc::make_mut(&mut self.pages[index / PAGE])[index % PAGE]
    }
}
impl<'a> IntoIterator for &'a Nodes {
    type Item = &'a Node;
    type IntoIter = std::iter::FlatMap<
        std::slice::Iter<'a, Arc<Vec<Node>>>,
        std::slice::Iter<'a, Node>,
        fn(&'a Arc<Vec<Node>>) -> std::slice::Iter<'a, Node>,
    >;
    fn into_iter(self) -> Self::IntoIter {
        self.pages
            .iter()
            .flat_map((|p: &'a Arc<Vec<Node>>| p.iter()) as fn(_) -> _)
    }
}
