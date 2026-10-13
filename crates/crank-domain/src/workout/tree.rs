use super::{MAX_NESTING_DEPTH, WorkoutNode};

/// Failure of a workout-tree helper on a tree built directly through the API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeError {
    /// Repeats nest deeper than [`MAX_NESTING_DEPTH`]; nothing was copied or generated.
    NestingTooDeep,
}
impl std::fmt::Display for TreeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for TreeError {}

/// Greatest number of repeats enclosing any node, measured without recursion.
fn repeat_depth(nodes: &[WorkoutNode]) -> usize {
    let mut deepest = 0;
    let mut stack: Vec<(&[WorkoutNode], usize)> = vec![(nodes, 0)];
    while let Some((level, enclosing)) = stack.pop() {
        for node in level {
            if let WorkoutNode::Repeat(r) = node {
                deepest = deepest.max(enclosing + 1);
                stack.push((&r.children, enclosing + 1));
            }
        }
    }
    deepest
}

fn copy_with_fresh_ids(
    nodes: &[WorkoutNode],
    ids: &mut impl FnMut() -> String,
) -> Vec<WorkoutNode> {
    nodes
        .iter()
        .map(|node| match node {
            WorkoutNode::Step(s) => {
                let mut s = s.clone();
                s.id = ids();
                WorkoutNode::Step(s)
            }
            WorkoutNode::Repeat(r) => {
                let id = ids();
                let mut copy = r.clone();
                copy.id = id;
                copy.children = copy_with_fresh_ids(&r.children, ids);
                WorkoutNode::Repeat(copy)
            }
        })
        .collect()
}

/// Deep copy of `nodes` in which every node (steps and repeats, at any depth) gets an
/// identity drawn from the caller-owned `ids` source; everything else is preserved exactly.
///
/// Mirrors the reference `reId`: IDs are requested in pre-order (a repeat's ID before its
/// children's, siblings in order), so a deterministic source yields reproducible IDs. This
/// function never checks the new IDs for uniqueness or collisions with the original or any
/// surrounding tree; use a source that is fresh across the whole workout.
///
/// # Errors
/// [`TreeError::NestingTooDeep`] when repeats nest deeper than [`MAX_NESTING_DEPTH`] (the
/// bound the parser and compiler enforce); the depth is checked iteratively before copying,
/// so no IDs are consumed and recursion is bounded.
pub fn re_id(
    nodes: &[WorkoutNode],
    ids: &mut impl FnMut() -> String,
) -> Result<Vec<WorkoutNode>, TreeError> {
    if repeat_depth(nodes) > MAX_NESTING_DEPTH {
        return Err(TreeError::NestingTooDeep);
    }
    Ok(copy_with_fresh_ids(nodes, ids))
}

/// Depth-first, pre-order lookup of the first node whose ID equals `id`.
///
/// Mirrors the reference `findNode`: with duplicate IDs the first match in depth-first order
/// wins (a repeat is tested before its children, siblings in order); an absent ID yields
/// `None`. The search is iterative, so it is safe on trees of any depth.
#[must_use]
pub fn find_node<'a>(nodes: &'a [WorkoutNode], id: &str) -> Option<&'a WorkoutNode> {
    let mut stack: Vec<std::slice::Iter<'a, WorkoutNode>> = vec![nodes.iter()];
    while let Some(level) = stack.last_mut() {
        let Some(node) = level.next() else {
            stack.pop();
            continue;
        };
        let (WorkoutNode::Step(super::StepNode { id: node_id, .. })
        | WorkoutNode::Repeat(super::RepeatNode { id: node_id, .. })) = node;
        if node_id == id {
            return Some(node);
        }
        if let WorkoutNode::Repeat(r) = node {
            stack.push(r.children.iter());
        }
    }
    None
}
