//! Tab/split tree — a port of `lib/reducers/term-groups.ts` semantics:
//! a tab is a root group; splitting in the parent's direction appends a
//! sibling (proportional rebalance), splitting across promotes the leaf
//! into a new split; removal rebalances and merges single-child parents.

use hyper_term::SessionId;
use slotmap::{new_key_type, SecondaryMap, SlotMap};

pub const MIN_SIZE: f32 = 0.05;

new_key_type! { pub struct NodeId; }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDir {
    /// Children side by side (pane:splitRight).
    Horizontal,
    /// Children stacked (pane:splitDown).
    Vertical,
}

#[derive(Debug, Clone)]
pub enum PaneNode {
    Leaf {
        session: SessionId,
    },
    Split {
        dir: SplitDir,
        children: Vec<NodeId>,
        /// Fractions along `dir`, always summing to ~1.0.
        sizes: Vec<f32>,
    },
}

pub struct Tab {
    pub root: NodeId,
    pub active_leaf: NodeId,
}

#[derive(Default)]
pub struct Workspace {
    pub nodes: SlotMap<NodeId, PaneNode>,
    parent: SecondaryMap<NodeId, NodeId>,
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
}

/// Rebalance for insertion: newcomer gets 1/(n+1), others scale down
/// proportionally (port of `insertRebalance`).
fn insert_rebalance(sizes: &mut Vec<f32>, insert_at: usize) {
    let n = sizes.len() as f32;
    let new_size = 1.0 / (n + 1.0);
    for s in sizes.iter_mut() {
        *s *= 1.0 - new_size;
    }
    sizes.insert(insert_at, new_size);
}

/// Rebalance for removal: spread the removed size evenly (port of
/// `removalRebalance`).
fn removal_rebalance(sizes: &mut Vec<f32>, removed_at: usize) {
    let removed = sizes.remove(removed_at);
    if sizes.is_empty() {
        return;
    }
    let share = removed / sizes.len() as f32;
    for s in sizes.iter_mut() {
        *s += share;
    }
}

impl Workspace {
    /// Create a new tab rooted at a fresh leaf; returns its index.
    pub fn new_tab(&mut self, session: SessionId) -> usize {
        let leaf = self.nodes.insert(PaneNode::Leaf { session });
        self.tabs.push(Tab {
            root: leaf,
            active_leaf: leaf,
        });
        self.active_tab = self.tabs.len() - 1;
        self.active_tab
    }

    pub fn active_session(&self) -> Option<SessionId> {
        let tab = self.tabs.get(self.active_tab)?;
        match self.nodes.get(tab.active_leaf)? {
            PaneNode::Leaf { session } => Some(*session),
            PaneNode::Split { .. } => None,
        }
    }

    pub fn find_leaf(&self, session: SessionId) -> Option<NodeId> {
        self.nodes.iter().find_map(|(id, node)| match node {
            PaneNode::Leaf { session: s } if *s == session => Some(id),
            _ => None,
        })
    }

    /// Tab index containing the given node (walks to the root).
    pub fn tab_of(&self, mut node: NodeId) -> Option<usize> {
        while let Some(parent) = self.parent.get(node) {
            node = *parent;
        }
        self.tabs.iter().position(|t| t.root == node)
    }

    /// Split `at_leaf`, placing `new_session` after it. Ports `splitGroup`.
    pub fn split(&mut self, at_leaf: NodeId, dir: SplitDir, new_session: SessionId) {
        let new_leaf = self.nodes.insert(PaneNode::Leaf {
            session: new_session,
        });

        let parent_id = self.parent.get(at_leaf).copied();
        let same_dir_parent = parent_id.and_then(|pid| match &self.nodes[pid] {
            PaneNode::Split { dir: d, .. } if *d == dir => Some(pid),
            _ => None,
        });

        if let Some(pid) = same_dir_parent {
            // Append as sibling right after the active leaf.
            if let PaneNode::Split {
                children, sizes, ..
            } = &mut self.nodes[pid]
            {
                let idx = children.iter().position(|c| *c == at_leaf).unwrap_or(0);
                children.insert(idx + 1, new_leaf);
                insert_rebalance(sizes, idx + 1);
            }
            self.parent.insert(new_leaf, pid);
        } else {
            // Promote: the leaf node becomes a Split in place (so the
            // parent's child list and tab roots stay valid) and its former
            // payload moves into a fresh child leaf.
            let old_payload = self.nodes[at_leaf].clone();
            let moved = self.nodes.insert(old_payload);
            // If the moved node was itself a split, reparent its children.
            if let PaneNode::Split { children, .. } = &self.nodes[moved] {
                for child in children.clone() {
                    self.parent.insert(child, moved);
                }
            }
            self.nodes[at_leaf] = PaneNode::Split {
                dir,
                children: vec![moved, new_leaf],
                sizes: vec![0.5, 0.5],
            };
            self.parent.insert(moved, at_leaf);
            self.parent.insert(new_leaf, at_leaf);

            // If the promoted node was a tab's active leaf, point at the
            // moved copy (a Split can't be active).
            for tab in &mut self.tabs {
                if tab.active_leaf == at_leaf {
                    tab.active_leaf = moved;
                }
            }
        }

        if let Some(tab_idx) = self.tab_of(new_leaf) {
            self.tabs[tab_idx].active_leaf = new_leaf;
        }
    }

    /// Remove a leaf; returns true if its whole tab was closed.
    /// Ports `removeGroup` + `replaceParent`.
    pub fn remove(&mut self, leaf: NodeId) -> bool {
        let Some(parent_id) = self.parent.get(leaf).copied() else {
            // Root leaf: close the tab.
            if let Some(idx) = self.tabs.iter().position(|t| t.root == leaf) {
                self.nodes.remove(leaf);
                self.tabs.remove(idx);
                self.retarget_active_tab(idx);
            }
            return true;
        };

        // The pane to focus if this one was active: its previous sibling, or
        // the next one when it was first (`findPrevious` in
        // `lib/actions/term-groups.ts`), descending leftmost afterwards.
        let mut focus_target = None;
        let mut merge_child: Option<NodeId> = None;
        if let PaneNode::Split {
            children, sizes, ..
        } = &mut self.nodes[parent_id]
        {
            if let Some(idx) = children.iter().position(|c| *c == leaf) {
                focus_target = if idx == 0 {
                    children.get(1).copied()
                } else {
                    children.get(idx - 1).copied()
                };
                children.remove(idx);
                removal_rebalance(sizes, idx);
            }
            if children.len() == 1 {
                merge_child = Some(children[0]);
            }
        }
        self.nodes.remove(leaf);
        self.parent.remove(leaf);

        // Single child left: the parent takes the child's payload.
        if let Some(child_id) = merge_child {
            let child_payload = self.nodes[child_id].clone();
            if let PaneNode::Split { children, .. } = &child_payload {
                for grandchild in children.clone() {
                    self.parent.insert(grandchild, parent_id);
                }
            }
            self.nodes[parent_id] = child_payload;
            self.nodes.remove(child_id);
            self.parent.remove(child_id);
            for tab in &mut self.tabs {
                if tab.active_leaf == child_id {
                    tab.active_leaf = parent_id;
                }
            }
            // The sibling we meant to focus just moved into the parent slot.
            if focus_target == Some(child_id) {
                focus_target = Some(parent_id);
            }
        }

        // Fix up the active leaf only if it pointed at something that no
        // longer exists — closing an unfocused pane must not move focus.
        if let Some(tab_idx) = self.tab_of(parent_id) {
            let active = self.tabs[tab_idx].active_leaf;
            let stale = !self.nodes.contains_key(active)
                || matches!(self.nodes[active], PaneNode::Split { .. });
            if stale {
                let target = focus_target
                    .filter(|id| self.nodes.contains_key(*id))
                    .unwrap_or(self.tabs[tab_idx].root);
                if let Some(&first) = self.leaves_of(target).first() {
                    self.tabs[tab_idx].active_leaf = first;
                }
            }
        }
        false
    }

    /// Keep focus on the same tab after the one at `removed_idx` closed.
    /// The original tracked the active tab by uid, so closing another tab
    /// never moved focus; closing the active one focused the tab *before* it
    /// (`findPrevious`), or the new first tab when the first was closed.
    fn retarget_active_tab(&mut self, removed_idx: usize) {
        if self.tabs.is_empty() {
            self.active_tab = 0;
            return;
        }
        if removed_idx < self.active_tab {
            self.active_tab -= 1;
        } else if removed_idx == self.active_tab {
            self.active_tab = removed_idx.saturating_sub(1);
        }
        self.active_tab = self.active_tab.min(self.tabs.len() - 1);
    }

    /// Adjust two adjacent children across a separator by `delta` fraction,
    /// clamped so both stay ≥ MIN_SIZE. Ports `TERM_GROUP_RESIZE`.
    pub fn resize_split(&mut self, split: NodeId, sep_index: usize, delta: f32) {
        if let Some(PaneNode::Split { sizes, .. }) = self.nodes.get_mut(split) {
            if sep_index + 1 >= sizes.len() {
                return;
            }
            let delta = delta
                .max(MIN_SIZE - sizes[sep_index])
                .min(sizes[sep_index + 1] - MIN_SIZE);
            sizes[sep_index] += delta;
            sizes[sep_index + 1] -= delta;
        }
    }

    /// Split the space of the two children either side of a separator evenly
    /// between them, leaving every other child alone. Ports `handleAutoResize`
    /// (double-click on a divider).
    pub fn equalize_split(&mut self, split: NodeId, sep_index: usize) {
        if let Some(PaneNode::Split { sizes, .. }) = self.nodes.get_mut(split) {
            if sep_index + 1 >= sizes.len() {
                return;
            }
            let shared = sizes[sep_index] + sizes[sep_index + 1];
            let half = shared / 2.0;
            sizes[sep_index] = half;
            sizes[sep_index + 1] = half;
        }
    }

    /// Leaves under `node` in DFS (visual) order.
    pub fn leaves_of(&self, node: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack = vec![node];
        while let Some(id) = stack.pop() {
            match &self.nodes[id] {
                PaneNode::Leaf { .. } => out.push(id),
                PaneNode::Split { children, .. } => {
                    for child in children.iter().rev() {
                        stack.push(*child);
                    }
                }
            }
        }
        out
    }

    pub fn sessions_of_tab(&self, tab_idx: usize) -> Vec<SessionId> {
        let Some(tab) = self.tabs.get(tab_idx) else {
            return Vec::new();
        };
        self.leaves_of(tab.root)
            .into_iter()
            .filter_map(|id| match &self.nodes[id] {
                PaneNode::Leaf { session } => Some(*session),
                _ => None,
            })
            .collect()
    }

    /// Move active-pane focus forward/backward in DFS order, wrapping.
    pub fn cycle_pane(&mut self, forward: bool) {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return;
        };
        let leaves = {
            let root = tab.root;
            let mut out = Vec::new();
            let mut stack = vec![root];
            while let Some(id) = stack.pop() {
                match &self.nodes[id] {
                    PaneNode::Leaf { .. } => out.push(id),
                    PaneNode::Split { children, .. } => {
                        for child in children.iter().rev() {
                            stack.push(*child);
                        }
                    }
                }
            }
            out
        };
        if leaves.len() < 2 {
            return;
        }
        let pos = leaves
            .iter()
            .position(|id| *id == tab.active_leaf)
            .unwrap_or(0);
        let next = if forward {
            (pos + 1) % leaves.len()
        } else {
            (pos + leaves.len() - 1) % leaves.len()
        };
        tab.active_leaf = leaves[next];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sid(n: u64) -> SessionId {
        SessionId(n)
    }

    #[test]
    fn same_direction_split_appends_sibling() {
        let mut ws = Workspace::default();
        ws.new_tab(sid(1));
        let leaf1 = ws.tabs[0].active_leaf;
        ws.split(leaf1, SplitDir::Horizontal, sid(2));
        let leaf2 = ws.tabs[0].active_leaf;
        ws.split(leaf2, SplitDir::Horizontal, sid(3));

        let root = ws.tabs[0].root;
        match &ws.nodes[root] {
            PaneNode::Split {
                dir,
                children,
                sizes,
            } => {
                assert_eq!(*dir, SplitDir::Horizontal);
                assert_eq!(children.len(), 3);
                assert!((sizes.iter().sum::<f32>() - 1.0).abs() < 1e-4);
                assert!((sizes[2] - 1.0 / 3.0).abs() < 1e-4);
            }
            _ => panic!("root should be a split"),
        }
        assert_eq!(ws.sessions_of_tab(0), vec![sid(1), sid(2), sid(3)]);
    }

    #[test]
    fn cross_direction_split_promotes() {
        let mut ws = Workspace::default();
        ws.new_tab(sid(1));
        let leaf1 = ws.tabs[0].active_leaf;
        ws.split(leaf1, SplitDir::Horizontal, sid(2));
        // Split the second pane vertically → it becomes a nested split.
        let leaf2 = ws.tabs[0].active_leaf;
        ws.split(leaf2, SplitDir::Vertical, sid(3));

        let root = ws.tabs[0].root;
        let PaneNode::Split { children, dir, .. } = &ws.nodes[root] else {
            panic!("root split");
        };
        assert_eq!(*dir, SplitDir::Horizontal);
        assert_eq!(children.len(), 2);
        let PaneNode::Split {
            dir: inner_dir,
            children: inner,
            sizes,
        } = &ws.nodes[children[1]]
        else {
            panic!("nested split expected");
        };
        assert_eq!(*inner_dir, SplitDir::Vertical);
        assert_eq!(inner.len(), 2);
        assert_eq!(sizes, &vec![0.5, 0.5]);
        assert_eq!(ws.sessions_of_tab(0), vec![sid(1), sid(2), sid(3)]);
    }

    #[test]
    fn removal_rebalances_and_merges_single_child_parent() {
        let mut ws = Workspace::default();
        ws.new_tab(sid(1));
        let leaf1 = ws.tabs[0].active_leaf;
        ws.split(leaf1, SplitDir::Horizontal, sid(2));
        let leaf2 = ws.tabs[0].active_leaf;
        ws.split(leaf2, SplitDir::Horizontal, sid(3));
        let leaf3 = ws.tabs[0].active_leaf;

        // Remove middle-ish pane; sizes redistribute, still 2 children.
        assert!(!ws.remove(leaf3));
        let root = ws.tabs[0].root;
        let PaneNode::Split {
            children, sizes, ..
        } = &ws.nodes[root]
        else {
            panic!()
        };
        assert_eq!(children.len(), 2);
        assert!((sizes.iter().sum::<f32>() - 1.0).abs() < 1e-4);

        // Remove one more: single child left → parent merges into leaf.
        let survivors = ws.leaves_of(root);
        assert!(!ws.remove(survivors[1]));
        assert!(matches!(ws.nodes[ws.tabs[0].root], PaneNode::Leaf { .. }));
        assert_eq!(ws.sessions_of_tab(0), vec![sid(1)]);

        // Removing the last leaf closes the tab.
        assert!(ws.remove(ws.tabs[0].root));
        assert!(ws.tabs.is_empty());
    }

    #[test]
    fn merge_of_nested_split_reparents_grandchildren() {
        let mut ws = Workspace::default();
        ws.new_tab(sid(1));
        let leaf1 = ws.tabs[0].active_leaf;
        ws.split(leaf1, SplitDir::Horizontal, sid(2));
        let leaf2 = ws.tabs[0].active_leaf;
        ws.split(leaf2, SplitDir::Vertical, sid(3));
        // Root: H[1, V[2,3]]. Remove 1 → root should become V[2,3].
        let root = ws.tabs[0].root;
        let first_leaf = ws.leaves_of(root)[0];
        ws.remove(first_leaf);
        let PaneNode::Split { dir, children, .. } = &ws.nodes[ws.tabs[0].root] else {
            panic!("root should still be a split");
        };
        assert_eq!(*dir, SplitDir::Vertical);
        assert_eq!(children.len(), 2);
        assert_eq!(ws.sessions_of_tab(0), vec![sid(2), sid(3)]);
        // Grandchildren must be reparented for future removals to work.
        let leaves = ws.leaves_of(ws.tabs[0].root);
        ws.remove(leaves[0]);
        assert_eq!(ws.sessions_of_tab(0), vec![sid(3)]);
    }

    /// Double-clicking a divider evens up the two panes it separates and
    /// leaves the rest of the row untouched.
    #[test]
    fn equalize_only_touches_the_two_panes_either_side() {
        let mut ws = Workspace::default();
        ws.new_tab(sid(1));
        let leaf1 = ws.tabs[0].active_leaf;
        ws.split(leaf1, SplitDir::Horizontal, sid(2));
        let leaf2 = ws.tabs[0].active_leaf;
        ws.split(leaf2, SplitDir::Horizontal, sid(3));
        let root = ws.tabs[0].root;

        let PaneNode::Split { sizes, .. } = &mut ws.nodes[root] else {
            panic!("expected a split");
        };
        *sizes = vec![0.6, 0.1, 0.3];

        ws.equalize_split(root, 0);
        let PaneNode::Split { sizes, .. } = &ws.nodes[root] else {
            panic!("expected a split");
        };
        // The first two share their combined 0.7; the third is untouched, and
        // the row still sums to 1.
        for (got, want) in sizes.iter().zip([0.35, 0.35, 0.3]) {
            assert!((got - want).abs() < 1e-5, "got {sizes:?}");
        }
        assert!((sizes.iter().sum::<f32>() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn resize_clamps_at_min_size() {
        let mut ws = Workspace::default();
        ws.new_tab(sid(1));
        let leaf1 = ws.tabs[0].active_leaf;
        ws.split(leaf1, SplitDir::Horizontal, sid(2));
        let root = ws.tabs[0].root;
        ws.resize_split(root, 0, 0.9); // would push child 2 below MIN_SIZE
        let PaneNode::Split { sizes, .. } = &ws.nodes[root] else {
            panic!()
        };
        assert!((sizes[1] - MIN_SIZE).abs() < 1e-4);
        assert!((sizes.iter().sum::<f32>() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn pane_cycling_wraps() {
        let mut ws = Workspace::default();
        ws.new_tab(sid(1));
        let leaf1 = ws.tabs[0].active_leaf;
        ws.split(leaf1, SplitDir::Horizontal, sid(2));
        assert_eq!(ws.active_session(), Some(sid(2)));
        ws.cycle_pane(true);
        assert_eq!(ws.active_session(), Some(sid(1)));
        ws.cycle_pane(false);
        assert_eq!(ws.active_session(), Some(sid(2)));
    }

    /// Three tabs of three sessions each, focus on the middle one.
    fn three_tabs() -> Workspace {
        let mut ws = Workspace::default();
        for n in 1..=3 {
            ws.new_tab(sid(n));
        }
        ws.active_tab = 1;
        ws
    }

    /// Closing another tab must not move focus (the original tracked the
    /// active tab by uid, not by index).
    #[test]
    fn closing_an_earlier_tab_keeps_focus() {
        let mut ws = three_tabs();
        let root = ws.tabs[0].root;
        assert!(ws.remove(root));
        assert_eq!(ws.tabs.len(), 2);
        assert_eq!(
            ws.active_session(),
            Some(sid(2)),
            "focus should stay on the same tab, not slide to the next"
        );
    }

    #[test]
    fn closing_a_later_tab_keeps_focus() {
        let mut ws = three_tabs();
        let root = ws.tabs[2].root;
        assert!(ws.remove(root));
        assert_eq!(ws.active_session(), Some(sid(2)));
    }

    /// Closing the active tab focuses the previous one (`findPrevious`).
    #[test]
    fn closing_the_active_tab_focuses_the_previous() {
        let mut ws = three_tabs();
        let root = ws.tabs[1].root;
        assert!(ws.remove(root));
        assert_eq!(ws.active_session(), Some(sid(1)));
    }

    /// …except when it was the first tab, where the next one takes over.
    #[test]
    fn closing_the_first_active_tab_focuses_the_new_first() {
        let mut ws = three_tabs();
        ws.active_tab = 0;
        let root = ws.tabs[0].root;
        assert!(ws.remove(root));
        assert_eq!(ws.active_session(), Some(sid(2)));
    }

    /// Closing the focused pane focuses its previous sibling, not the tab's
    /// first pane.
    #[test]
    fn closing_the_focused_pane_focuses_the_previous_sibling() {
        let mut ws = Workspace::default();
        ws.new_tab(sid(1));
        let leaf1 = ws.tabs[0].active_leaf;
        ws.split(leaf1, SplitDir::Horizontal, sid(2));
        let leaf2 = ws.tabs[0].active_leaf;
        ws.split(leaf2, SplitDir::Horizontal, sid(3));
        // A|B|C with C focused.
        assert_eq!(ws.active_session(), Some(sid(3)));
        let leaf3 = ws.tabs[0].active_leaf;
        ws.remove(leaf3);
        assert_eq!(ws.active_session(), Some(sid(2)), "previous sibling, not A");
    }

    /// Closing the *first* pane focuses the one that follows it.
    #[test]
    fn closing_the_first_focused_pane_focuses_the_next() {
        let mut ws = Workspace::default();
        ws.new_tab(sid(1));
        let leaf1 = ws.tabs[0].active_leaf;
        ws.split(leaf1, SplitDir::Horizontal, sid(2));
        ws.split(ws.tabs[0].active_leaf, SplitDir::Horizontal, sid(3));
        let first = ws.leaves_of(ws.tabs[0].root)[0];
        ws.tabs[0].active_leaf = first;
        ws.remove(first);
        assert_eq!(ws.active_session(), Some(sid(2)));
    }

    /// Closing an unfocused pane leaves focus where it was.
    #[test]
    fn closing_an_unfocused_pane_keeps_focus() {
        let mut ws = Workspace::default();
        ws.new_tab(sid(1));
        let leaf1 = ws.tabs[0].active_leaf;
        ws.split(leaf1, SplitDir::Horizontal, sid(2));
        ws.split(ws.tabs[0].active_leaf, SplitDir::Horizontal, sid(3));
        // Focus B, close A.
        let leaves = ws.leaves_of(ws.tabs[0].root);
        ws.tabs[0].active_leaf = leaves[1];
        ws.remove(leaves[0]);
        assert_eq!(ws.active_session(), Some(sid(2)));
    }

    /// After a merge, focus must land inside the surviving sibling's subtree.
    #[test]
    fn focus_survives_a_single_child_merge() {
        let mut ws = Workspace::default();
        ws.new_tab(sid(1));
        let leaf1 = ws.tabs[0].active_leaf;
        ws.split(leaf1, SplitDir::Horizontal, sid(2));
        let leaf2 = ws.tabs[0].active_leaf;
        ws.split(leaf2, SplitDir::Vertical, sid(3));
        // Root: H[1, V[2,3]] with 3 focused. Removing 1 merges V into root.
        let first = ws.leaves_of(ws.tabs[0].root)[0];
        ws.remove(first);
        assert_eq!(ws.sessions_of_tab(0), vec![sid(2), sid(3)]);
        assert_eq!(
            ws.active_session(),
            Some(sid(3)),
            "the focused pane is untouched by an unrelated merge"
        );
    }
}
