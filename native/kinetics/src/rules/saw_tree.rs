//! `TreeCutter`: what a Mechanical Saw fells when it breaks a log, a plant or a chorus plant.
//! Ported from `TreeCutter` (MIT, the Creators of Create), `mc1.21.1/dev` at 4148fd8, search
//! order included.
//!
//! No game types: the glue answers what each block is ([`Class`]) and breaks what is found.

use std::collections::HashSet;

use crate::Pos;

/// What `TreeCutter` asks of a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Class {
    /// `isLog`: `minecraft:logs`, `create:slimy_logs` or a mushroom stem.
    pub log: bool,
    /// `isRoot`: `create:roots`.
    pub root: bool,
    /// `isLeaf`: the block has an integer `distance` property (not scaffolding's), and its value.
    pub leaf: Option<i32>,
    /// `AllBlockTags.TREE_ATTACHMENTS`.
    pub attachment: bool,
    /// `nonDecayingLeafDistance`: how far from the cut a block that is no leaf still counts as
    /// the tree's (-1 for none).
    pub non_decaying: i32,
    /// `isVerticalPlant`: bamboo, cactus, sugar cane, kelp.
    pub vertical_plant: bool,
    /// `isChorus`: a chorus plant or flower.
    pub chorus: bool,
}

impl Class {
    pub const NOTHING: Class = Class {
        log: false,
        root: false,
        leaf: None,
        attachment: false,
        non_decaying: -1,
        vertical_plant: false,
        chorus: false,
    };
}

/// The blocks around the cut, as the glue reads them.
pub trait World {
    fn class(&mut self, pos: Pos) -> Class;
    /// `BlockGetter.getHeight`: how many blocks tall the world is.
    fn height(&self) -> i32;
}

/// `TreeCutter.Tree`: what to break, in `destroyBlocks`' order (attachments, logs, leaves).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tree {
    pub logs: Vec<Pos>,
    pub leaves: Vec<Pos>,
    pub attachments: Vec<Pos>,
}

impl Tree {
    /// Every block with the chance of a break effect `destroyBlocks` gives it
    /// (`makeCallbackFor(world, effectChance, ...)`), in order.
    pub fn in_order(&self) -> impl Iterator<Item = (Pos, f32)> + '_ {
        let all = self.attachments.iter().map(|p| (*p, 1.0 / 32.0));
        let logs = self.logs.iter().map(|p| (*p, 1.0 / 2.0));
        let leaves = self.leaves.iter().map(|p| (*p, 1.0 / 8.0));
        all.chain(logs).chain(leaves)
    }

    pub fn is_empty(&self) -> bool {
        self.logs.is_empty() && self.leaves.is_empty() && self.attachments.is_empty()
    }
}

const NO_TREE: Tree = Tree {
    logs: Vec::new(),
    leaves: Vec::new(),
    attachments: Vec::new(),
};

fn offset(pos: Pos, by: Pos) -> Pos {
    [pos[0] + by[0], pos[1] + by[1], pos[2] + by[2]]
}

/// `BlockPos.betweenClosed`: x runs fastest, then y, then z.
fn between(min: Pos, max: Pos) -> Vec<Pos> {
    let mut out = Vec::new();
    for z in min[2]..=max[2] {
        for y in min[1]..=max[1] {
            for x in min[0]..=max[0] {
                out.push([x, y, z]);
            }
        }
    }
    out
}

#[derive(Clone, Copy)]
enum Search {
    Up,
    Down,
    Both,
}

impl Search {
    fn y(self) -> (i32, i32) {
        match self {
            Search::Up => (0, 1),
            Search::Down => (-1, 0),
            Search::Both => (-1, 1),
        }
    }
}

/// `TreeCutter.forNeighbours`: the box round `pos`, in order, each position looked at only
/// when its turn comes (`acceptor` may mark later ones visited).
fn for_neighbours(
    pos: Pos,
    visited: &mut HashSet<Pos>,
    direction: Search,
    mut acceptor: impl FnMut(Pos, &mut HashSet<Pos>),
) {
    let (min_y, max_y) = direction.y();
    for p in between(offset(pos, [-1, min_y, -1]), offset(pos, [1, max_y, 1])) {
        if !visited.contains(&p) {
            acceptor(p, visited);
        }
    }
}

fn is_log_or_root(c: Class) -> bool {
    c.log || c.root
}

/// `TreeCutter.validateCut`: the layer above the cut must not stand on any more logs.
fn validate_cut(world: &mut impl World, pos: Pos) -> bool {
    let mut visited: HashSet<Pos> = HashSet::new();
    let mut frontier: Vec<Pos> = vec![pos, [pos[0], pos[1] + 1, pos[2]]];
    let pos_y = pos[1];
    let mut next = 0;
    while next < frontier.len() {
        let current = frontier[next];
        next += 1;
        let below = [current[0], current[1] - 1, current[2]];
        visited.insert(current);
        let lower_layer = current[1] == pos_y;
        let here = world.class(current);
        if !is_log_or_root(here) {
            continue;
        }
        if !lower_layer && pos != below && is_log_or_root(world.class(below)) {
            return false;
        }
        for dir in crate::Dir::ALL {
            if dir == crate::Dir::Down {
                continue;
            }
            if dir == crate::Dir::Up && !lower_layer {
                continue;
            }
            let at = offset(current, dir.normal());
            if visited.contains(&at) {
                continue;
            }
            frontier.push(at);
        }
    }
    true
}

/// `TreeCutter.findTree`: the tree a saw at the broken block `pos` (now air) fells, given what
/// the block was (`broken`).
pub fn find_tree(world: &mut impl World, pos: Pos, broken: Class) -> Tree {
    let above = [pos[0], pos[1] + 1, pos[2]];
    let state_above = world.class(above);

    // Bamboo, sugar cane, cactus, kelp.
    if broken.vertical_plant {
        if !state_above.vertical_plant {
            return NO_TREE;
        }
        let mut logs = vec![above];
        for i in 1..world.height() {
            let current = [pos[0], pos[1] + i, pos[2]];
            if !world.class(current).vertical_plant {
                break;
            }
            logs.push(current);
        }
        logs.reverse();
        return Tree {
            logs,
            ..Tree::default()
        };
    }

    // Chorus.
    if broken.chorus {
        if !state_above.chorus {
            return NO_TREE;
        }
        let mut logs = Vec::new();
        let mut visited: HashSet<Pos> = HashSet::new();
        let mut frontier = vec![above];
        let mut next = 0;
        while next < frontier.len() {
            let current = frontier[next];
            next += 1;
            visited.insert(current);
            logs.push(current);
            for dir in crate::Dir::ALL {
                let at = offset(current, dir.normal());
                if visited.contains(&at) {
                    continue;
                }
                if !world.class(at).chorus {
                    continue;
                }
                frontier.push(at);
            }
        }
        logs.reverse();
        return Tree {
            logs,
            ..Tree::default()
        };
    }

    // A regular tree.
    if !validate_cut(world, pos) {
        return NO_TREE;
    }
    let mut logs: Vec<Pos> = Vec::new();
    let mut leaves: Vec<Pos> = Vec::new();
    let mut attachments: Vec<Pos> = Vec::new();
    let mut visited: HashSet<Pos> = HashSet::new();
    visited.insert(pos);
    let mut frontier: std::collections::VecDeque<Pos> = between(
        offset(pos, [-1, 0, -1]),
        offset(pos, [1, 1, 1]),
    )
    .into();

    // All logs and roots.
    let mut has_roots = false;
    while let Some(current) = frontier.pop_front() {
        if !visited.insert(current) {
            continue;
        }
        let here = world.class(current);
        if here.root {
            has_roots = true;
        } else if !here.log {
            continue;
        }
        logs.push(current);
        for_neighbours(current, &mut visited, Search::Up, |p, _| frontier.push_back(p));
    }

    visited.clear();
    visited.extend(logs.iter().copied());
    frontier.extend(logs.iter().copied());

    if has_roots {
        let old: HashSet<Pos> = logs.iter().copied().collect();
        while let Some(current) = frontier.pop_front() {
            if !world.class(current).root {
                continue;
            }
            if !old.contains(&current) {
                logs.push(current);
            }
            for_neighbours(current, &mut visited, Search::Down, |p, visited| {
                if visited.insert(p) {
                    frontier.push_back(p);
                }
            });
        }
        visited.clear();
        visited.extend(logs.iter().copied());
        frontier.extend(logs.iter().copied());
    }

    // All leaves.
    while let Some(prev) = frontier.pop_front() {
        let prev_state = world.class(prev);
        let prev_distance = prev_state.leaf.unwrap_or(0);
        // `isLeaf(prevState) ? getLeafDistance(prevState) : 0`.
        for_neighbours(prev, &mut visited, Search::Both, |current, visited| {
            let state = world.class(current);
            if state.attachment {
                attachments.push(current);
                visited.insert(current);
                return;
            }
            let horizontal = (current[0] - pos[0]).abs().max((current[2] - pos[2]).abs());
            if horizontal <= state.non_decaying && visited.insert(current) {
                leaves.push(current);
                frontier.push_back(current);
                return;
            }
            if let Some(distance) = state.leaf
                && distance > prev_distance
                && visited.insert(current)
            {
                leaves.push(current);
                frontier.push_back(current);
            }
        });
    }
    Tree {
        logs,
        leaves,
        attachments,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const LOG: Class = Class {
        log: true,
        ..Class::NOTHING
    };
    const ROOT: Class = Class {
        root: true,
        ..Class::NOTHING
    };
    const BAMBOO: Class = Class {
        vertical_plant: true,
        ..Class::NOTHING
    };
    const VINE: Class = Class {
        attachment: true,
        ..Class::NOTHING
    };
    fn leaf(distance: i32) -> Class {
        Class {
            leaf: Some(distance),
            ..Class::NOTHING
        }
    }

    #[derive(Default)]
    struct Blocks(HashMap<Pos, Class>);

    impl World for Blocks {
        fn class(&mut self, pos: Pos) -> Class {
            self.0.get(&pos).copied().unwrap_or(Class::NOTHING)
        }
        fn height(&self) -> i32 {
            384
        }
    }

    impl Blocks {
        fn put(&mut self, pos: Pos, class: Class) -> &mut Self {
            self.0.insert(pos, class);
            self
        }
        /// A trunk of `height` logs from `base` up, with leaves one block round its top two
        /// logs, as a plain tree has them (distance 1 beside a log, 2 beyond).
        fn tree(base: Pos, height: i32) -> Self {
            let mut w = Blocks::default();
            for y in 0..height {
                w.put([base[0], base[1] + y, base[2]], LOG);
            }
            for y in height - 2..height + 1 {
                for dx in -2..=2 {
                    for dz in -2..=2 {
                        let at = [base[0] + dx, base[1] + y, base[2] + dz];
                        if w.0.contains_key(&at) {
                            continue;
                        }
                        let d = (dx.abs().max(dz.abs()) + (height - 1 - y).max(0)).max(1);
                        if d <= 4 && (dx.abs() + dz.abs()) < 4 {
                            w.put(at, leaf(d.min(6)));
                        }
                    }
                }
            }
            w
        }
    }

    #[test]
    fn a_tree_cut_at_its_foot_falls_whole() {
        let mut w = Blocks::tree([0, 0, 0], 5);
        // The saw broke the bottom log.
        w.0.remove(&[0, 0, 0]);
        let tree = find_tree(&mut w, [0, 0, 0], LOG);
        assert_eq!(tree.logs.len(), 4);
        assert!(tree.logs.contains(&[0, 4, 0]));
        assert!(!tree.leaves.is_empty());
        // Every leaf found is a leaf of the model.
        assert!(tree.leaves.iter().all(|p| w.0[p].leaf.is_some()));
        // Nothing found twice.
        let all: HashSet<_> = tree.logs.iter().chain(&tree.leaves).collect();
        assert_eq!(all.len(), tree.logs.len() + tree.leaves.len());
    }

    #[test]
    fn a_cut_with_more_trunk_beside_it_leaves_the_tree() {
        // Two logs side by side at the layer above the cut, with a log under one of them.
        let mut w = Blocks::default();
        w.put([0, 1, 0], LOG).put([1, 1, 0], LOG).put([1, 0, 0], LOG);
        assert_eq!(find_tree(&mut w, [0, 0, 0], LOG), Tree::default());
    }

    #[test]
    fn nothing_above_the_cut_is_no_tree() {
        let mut w = Blocks::default();
        w.put([5, 5, 5], leaf(1));
        let tree = find_tree(&mut w, [0, 0, 0], LOG);
        assert!(tree.logs.is_empty() && tree.leaves.is_empty());
    }

    #[test]
    fn a_log_found_only_beside_the_cut_counts() {
        // A diagonal trunk: the log above the cut sits one block over.
        let mut w = Blocks::default();
        w.put([1, 1, 0], LOG).put([1, 2, 0], LOG);
        let tree = find_tree(&mut w, [0, 0, 0], LOG);
        assert_eq!(tree.logs, vec![[1, 1, 0], [1, 2, 0]]);
    }

    #[test]
    fn leaves_further_from_the_trunk_than_their_neighbour_follow() {
        let mut w = Blocks::default();
        w.put([0, 1, 0], LOG);
        w.put([1, 1, 0], leaf(1)).put([2, 1, 0], leaf(2)).put([3, 1, 0], leaf(3));
        // A leaf closer to a log than the one before it is another tree's.
        w.put([4, 1, 0], leaf(1));
        let tree = find_tree(&mut w, [0, 0, 0], LOG);
        assert_eq!(tree.leaves, vec![[1, 1, 0], [2, 1, 0], [3, 1, 0]]);
    }

    #[test]
    fn attachments_are_taken_but_stop_the_search() {
        let mut w = Blocks::default();
        w.put([0, 1, 0], LOG);
        w.put([1, 1, 0], VINE).put([2, 1, 0], leaf(2));
        let tree = find_tree(&mut w, [0, 0, 0], LOG);
        assert_eq!(tree.attachments, vec![[1, 1, 0]]);
        // The leaf behind it is only found through the vine, which the search doesn't pass.
        assert!(tree.leaves.is_empty());
    }

    #[test]
    fn mushrooms_count_within_their_reach() {
        let mut w = Blocks::default();
        w.put([0, 1, 0], LOG);
        let cap = Class {
            non_decaying: 2,
            ..Class::NOTHING
        };
        w.put([1, 2, 0], cap).put([2, 2, 0], cap).put([3, 2, 0], cap);
        let tree = find_tree(&mut w, [0, 0, 0], LOG);
        // Within 2 blocks of the cut, sideways: the third is too far.
        assert_eq!(tree.leaves.len(), 2);
    }

    #[test]
    fn roots_below_the_trunk_are_followed_down() {
        let mut w = Blocks::default();
        w.put([0, 1, 0], LOG);
        // A root beside the cut, and one below that.
        w.put([1, 0, 0], ROOT).put([1, -1, 0], ROOT);
        let tree = find_tree(&mut w, [0, 0, 0], LOG);
        assert!(tree.logs.contains(&[1, 0, 0]));
        assert!(tree.logs.contains(&[1, -1, 0]));
    }

    #[test]
    fn a_plant_is_felled_from_the_top_down() {
        let mut w = Blocks::default();
        for y in 1..=4 {
            w.put([0, y, 0], BAMBOO);
        }
        let tree = find_tree(&mut w, [0, 0, 0], BAMBOO);
        // Create's loop adds the block above the cut twice; the tops come first.
        assert_eq!(
            tree.logs,
            vec![[0, 4, 0], [0, 3, 0], [0, 2, 0], [0, 1, 0], [0, 1, 0]]
        );
        // Nothing grows above: no tree.
        let mut lone = Blocks::default();
        assert_eq!(find_tree(&mut lone, [0, 0, 0], BAMBOO), Tree::default());
    }

    #[test]
    fn a_chorus_plant_branches_in_every_direction() {
        let chorus = Class {
            chorus: true,
            ..Class::NOTHING
        };
        let mut w = Blocks::default();
        w.put([0, 1, 0], chorus).put([0, 2, 0], chorus).put([1, 2, 0], chorus);
        w.put([0, 2, 1], chorus).put([5, 5, 5], chorus);
        let tree = find_tree(&mut w, [0, 0, 0], chorus);
        assert_eq!(tree.logs.len(), 4);
        assert!(!tree.logs.contains(&[5, 5, 5]));
        // Reversed: the first found is last.
        assert_eq!(*tree.logs.last().unwrap(), [0, 1, 0]);
    }

    #[test]
    fn destroy_order_is_attachments_then_logs_then_leaves() {
        let tree = Tree {
            logs: vec![[0, 1, 0]],
            leaves: vec![[1, 1, 0]],
            attachments: vec![[2, 1, 0]],
        };
        let order: Vec<_> = tree.in_order().collect();
        assert_eq!(
            order,
            vec![
                ([2, 1, 0], 1.0 / 32.0),
                ([0, 1, 0], 0.5),
                ([1, 1, 0], 0.125)
            ]
        );
    }

    #[test]
    fn neighbours_run_x_then_y_then_z() {
        let cells = between([0, 0, 0], [1, 1, 1]);
        assert_eq!(cells[0], [0, 0, 0]);
        assert_eq!(cells[1], [1, 0, 0]);
        assert_eq!(cells[2], [0, 1, 0]);
        assert_eq!(cells[4], [0, 0, 1]);
        assert_eq!(cells.len(), 8);
    }
}
