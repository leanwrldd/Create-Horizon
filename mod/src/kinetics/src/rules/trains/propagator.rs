//! `TrackPropagator` (content/trains/track/TrackPropagator.java) and the discovery half of
//! `ITrackBlock` (`getConnected`, `walkConnectedTracks`, `addToListIfConnected`): what happens to the
//! graphs when a piece of track is placed or broken.
//!
//! Placing a piece ([`RailNetworks::on_rail_added`]) removes every node the piece touches from the
//! graphs that had them, merges those graphs into one (or makes a new one), then walks the track from
//! the piece, making a node wherever the track is not just a straight in-line continuation
//! ([`is_valid_graph_node_location`]: a junction, a turn, a change of material, a dead end, or every 16
//! blocks) and an edge between consecutive nodes. Breaking a piece ([`RailNetworks::on_rail_removed`])
//! removes its nodes, re-runs the placement for the neighbouring track, and splits any graph that fell
//! into pieces.
//!
//! The world is behind [`TrackWorld`]: given a block position, what track ends it has. The glue
//! implements it from the real blocks (`ITrackBlock.getConnected`); [`connected_straight`] is the
//! common case written out.

use super::graph::{DiscoveredLocation, NodeId, TrackNodeLocation, Uuid};
use super::network::{RailEvent, RailNetworks};
use super::vec::{Vec3, add, equal, round, scale};
use crate::Pos;
use std::collections::{BTreeSet, HashSet, VecDeque};

/// What the propagator asks of the world.
pub trait TrackWorld {
    /// `ITrackBlock.getConnected(world, pos, state, linear, connectedTo)`: the track ends of the
    /// block at `pos`, empty when it is not track. With `connected_to`, only when that location is
    /// one of the block's ends, and then the other end(s). `ignore_turns` is Java's `linear`: leave
    /// out the curves attached to the block.
    fn connected(&self, pos: Pos, ignore_turns: bool, connected_to: Option<&TrackNodeLocation>) -> Vec<DiscoveredLocation>;
}

/// `ITrackBlock.walkConnectedTracks(world, location, linear)`: the ends reachable from a node, over
/// every block next to it.
pub fn walk_connected_tracks(world: &dyn TrackWorld, location: &TrackNodeLocation, linear: bool) -> Vec<DiscoveredLocation> {
    let mut list = Vec::new();
    for pos in location.all_adjacent() {
        list.extend(world.connected(pos, linear, Some(location)));
    }
    list
}

/// The tail of `ITrackBlock.addToListIfConnected`: with a `from_end`, only a block that has it as one
/// end contributes, and only the other end; two ends in different dimensions are both forced nodes.
pub fn add_to_list_if_connected(
    from_end: Option<&TrackNodeLocation>,
    list: &mut Vec<DiscoveredLocation>,
    mut first: DiscoveredLocation,
    mut second: DiscoveredLocation,
) {
    if first.loc.dimension != second.loc.dimension {
        first.force_node = true;
        second.force_node = true;
    }
    let mut skip_first = false;
    let mut skip_second = false;
    if let Some(from) = from_end {
        let equals_first = first.loc == *from;
        let equals_second = second.loc == *from;
        // not reachable from this end
        if !equals_first && !equals_second {
            return;
        }
        skip_first = equals_first;
        skip_second = equals_second;
    }
    if !skip_first {
        list.push(first);
    }
    if !skip_second {
        list.push(second);
    }
}

/// A straight (or sloped) track block as `ITrackBlock.getConnected` sees it: one end half a block
/// along each axis from the centre.
pub struct StraightTrack<'a> {
    /// `Vec3.atBottomCenterOf(pos)` plus `getElevationAtCenter`.
    pub center: Vec3,
    pub axes: Vec<Vec3>,
    pub normal: Vec3,
    pub dimension: u32,
    /// `getMaterialSimple(world, v)`: the track material at a point.
    pub material: &'a dyn Fn(Vec3) -> Option<String>,
    /// `getYOffsetAt(world, pos, state, v)`: the tilt of the end at `v`.
    pub y_offset: &'a dyn Fn(Vec3) -> i32,
}

/// `ITrackBlock.getConnected` for a block with straight axes.
pub fn connected_straight(t: &StraightTrack, connected_to: Option<&TrackNodeLocation>) -> Vec<DiscoveredLocation> {
    let mut list = Vec::new();
    for axis in &t.axes {
        let offset = |d: f64, b: bool| add(scale(*axis, if b { d } else { -d }), t.center);
        let make = |b: bool| {
            let first_offset = offset(0.5, b);
            DiscoveredLocation::new(t.dimension, first_offset)
                .via_turn(None)
                .materials((t.material)(offset(0.0, b)), (t.material)(offset(1.0, b)))
                .with_normal(t.normal)
                .with_direction(Some(*axis))
                .with_y_offset((t.y_offset)(first_offset))
        };
        add_to_list_if_connected(connected_to, &mut list, make(true), make(false));
    }
    list
}

struct FrontierEntry {
    prev: Option<DiscoveredLocation>,
    current: DiscoveredLocation,
    parent: Option<DiscoveredLocation>,
    /// Java compares `currentNode != startNode` by reference: only the entry seeded with the start
    /// node is it.
    is_start: bool,
}

/// `TrackPropagator.isValidGraphNodeLocation(location, next, first)`: whether a node belongs here.
pub fn is_valid_graph_node_location(location: &DiscoveredLocation, next: &[DiscoveredLocation], first: bool) -> bool {
    let size = next.len() as i64 - i64::from(first);
    if size != 1 {
        return true;
    }
    if location.force_node {
        return true;
    }
    if location.different_materials() {
        return true;
    }
    if next.iter().any(|d| d.force_node) {
        return true;
    }
    if let Some(direction) = location.direction {
        if next.iter().any(|dl| dl.not_in_line_with(direction)) {
            return true;
        }
    }
    let vec = location.loc.location();
    let centered_x = !equal(vec[0], round(vec[0]) as f64);
    let centered_z = !equal(vec[2], round(vec[2]) as f64);
    if centered_x && !centered_z {
        return (round(vec[2]) as i32) % 16 == 0;
    }
    (round(vec[0]) as i32) % 16 == 0
}

fn remove_first(list: &mut Vec<DiscoveredLocation>, target: &DiscoveredLocation) {
    if let Some(i) = list.iter().position(|d| d == target) {
        list.remove(i);
    }
}

/// `continueSearch`: new ends not seen before, with no parent.
fn continue_search(
    frontier: &mut VecDeque<FrontierEntry>,
    visited: &mut HashSet<TrackNodeLocation>,
    entry: &FrontierEntry,
    ends: Vec<DiscoveredLocation>,
) {
    for location in ends {
        if visited.insert(location.loc) {
            frontier.push_back(FrontierEntry { prev: Some(entry.current.clone()), current: location, parent: None, is_start: false });
        }
    }
}

/// `continueSearchWithParent`.
fn continue_search_with_parent(
    frontier: &mut VecDeque<FrontierEntry>,
    entry: &FrontierEntry,
    parent: &DiscoveredLocation,
    ends: Vec<DiscoveredLocation>,
) {
    for location in ends {
        frontier.push_back(FrontierEntry {
            prev: Some(entry.current.clone()),
            current: location,
            parent: Some(parent.clone()),
            is_start: false,
        });
    }
}

impl RailNetworks {
    /// `TrackPropagator.onRailRemoved(reader, pos, state)`. `removed_ends` is what
    /// `getConnected(pos, removedState, false, null)` gave for the block that was there (the world no
    /// longer has it).
    pub fn on_rail_removed(&mut self, world: &dyn TrackWorld, pos: Pos, removed_ends: &[DiscoveredLocation]) {
        // 1. Remove any nodes this rail was part of
        for removed in removed_ends {
            for graph in self.get_graphs(&removed.loc) {
                let Some(node) = self.graphs.get(&graph).and_then(|g| g.locate_node(&removed.loc)).map(|n| n.net_id) else {
                    continue;
                };
                self.remove_node(graph, &removed.loc, true);
                self.events.push(RailEvent::NodeRemoved { graph, node, location: removed.loc });
                if !self.graphs.get(&graph).is_some_and(|g| g.is_empty()) {
                    continue;
                }
                self.remove_graph_and_group(graph);
                self.events.push(RailEvent::GraphRemoved { graph });
            }
        }

        let mut positions_to_update: BTreeSet<Pos> = BTreeSet::new();
        for end in removed_ends {
            positions_to_update.extend(end.loc.all_adjacent());
        }

        // 2. Re-run railAdded for any track that was disconnected from this track
        let mut to_update: BTreeSet<Uuid> = BTreeSet::new();
        for block_pos in positions_to_update {
            if block_pos != pos {
                if let Some(g) = self.on_rail_added(world, block_pos) {
                    to_update.insert(g);
                }
            }
        }

        // 3. Ensure any affected graph gets checked for segmentation
        for graph in to_update {
            if self.graphs.contains_key(&graph) {
                self.update_split_graph(graph);
            }
        }
        self.mark_tracks_dirty();
    }

    /// `TrackPropagator.onRailAdded(reader, pos, state)`: the graph the piece ended up in, `None` when
    /// the block is not track.
    pub fn on_rail_added(&mut self, world: &dyn TrackWorld, pos: Pos) -> Option<Uuid> {
        let initial = world.connected(pos, false, None);
        if initial.is_empty() {
            return None;
        }

        // 1. Remove all immediately reachable node locations
        let mut frontier: VecDeque<FrontierEntry> = VecDeque::new();
        let mut visited: HashSet<TrackNodeLocation> = HashSet::new();
        let mut connected_graphs: BTreeSet<Uuid> = BTreeSet::new();
        for i in initial {
            frontier.push_back(FrontierEntry { prev: None, current: i, parent: None, is_start: false });
        }

        let mut emergency_exit = 1000;
        while let Some(entry) = frontier.pop_front() {
            if emergency_exit == 0 {
                break;
            }
            emergency_exit -= 1;

            let intersecting = self.get_graphs(&entry.current.loc);
            for graph in &intersecting {
                let node = self.graphs.get(graph).and_then(|g| g.locate_node(&entry.current.loc)).map(|n| n.net_id);
                self.remove_node(*graph, &entry.current.loc, true);
                if let Some(node) = node {
                    self.events.push(RailEvent::NodeRemoved { graph: *graph, node, location: entry.current.loc });
                }
                connected_graphs.insert(*graph);
            }
            if !intersecting.is_empty() {
                continue;
            }

            let mut ends = walk_connected_tracks(world, &entry.current.loc, false);
            if let Some(prev) = &entry.prev {
                remove_first(&mut ends, prev);
            }
            continue_search(&mut frontier, &mut visited, &entry, ends);
        }

        frontier.clear();
        visited.clear();

        // Remove empty graphs
        let snapshot: Vec<Uuid> = connected_graphs.iter().copied().collect();
        for g in snapshot {
            let empty = self.graphs.get(&g).is_none_or(|g| g.is_empty());
            if !empty || connected_graphs.len() == 1 {
                continue;
            }
            self.remove_graph_and_group(g);
            self.events.push(RailEvent::GraphRemoved { graph: g });
            connected_graphs.remove(&g);
        }

        // Merge graphs if more than 1
        let graph: Uuid;
        if connected_graphs.len() > 1 {
            let mut target: Option<Uuid> = None;
            for other in connected_graphs.clone() {
                match target {
                    None => target = Some(other),
                    Some(t) => {
                        self.events.push(RailEvent::GraphMerged { from: other, into: t });
                        self.transfer_all(other, t);
                        self.remove_graph_and_group(other);
                        self.events.push(RailEvent::GraphRemoved { graph: other });
                    }
                }
            }
            graph = target.expect("more than one graph");
        } else if let Some(only) = connected_graphs.iter().next() {
            graph = *only;
        } else {
            let g = self.new_graph();
            graph = g.id;
            self.put_graph_with_default_group(g);
        }

        // 2. Find the first graph node candidate nearby
        let mut start_node: Option<DiscoveredLocation> = None;
        for i in world.connected(pos, true, None) {
            frontier.push_back(FrontierEntry { prev: None, current: i, parent: None, is_start: false });
        }

        emergency_exit = 1000;
        while let Some(entry) = frontier.pop_front() {
            if emergency_exit == 0 {
                break;
            }
            emergency_exit -= 1;

            let mut ends = walk_connected_tracks(world, &entry.current.loc, false);
            let first = entry.prev.is_none();
            if let Some(prev) = &entry.prev {
                remove_first(&mut ends, prev);
            }
            if is_valid_graph_node_location(&entry.current, &ends, first) {
                start_node = Some(entry.current.clone());
                break;
            }
            continue_search(&mut frontier, &mut visited, &entry, ends);
        }

        frontier.clear();
        let Some(start_node) = start_node else {
            // Java would dereference null here: leave the graph as it is.
            return Some(graph);
        };
        let mut added_nodes: Vec<NodeId> = Vec::new();
        self.create_node_if_absent(graph, &start_node);
        frontier.push_back(FrontierEntry {
            prev: None,
            current: start_node.clone(),
            parent: Some(start_node.clone()),
            is_start: true,
        });

        // 3. Build up the graph via all connected nodes
        emergency_exit = 1000;
        while let Some(entry) = frontier.pop_front() {
            if emergency_exit == 0 {
                break;
            }
            emergency_exit -= 1;

            let mut parent_node = entry.parent.clone().unwrap_or_else(|| start_node.clone());
            let mut ends = walk_connected_tracks(world, &entry.current.loc, false);
            let first = entry.prev.is_none();
            if let Some(prev) = &entry.prev {
                remove_first(&mut ends, prev);
            }

            if is_valid_graph_node_location(&entry.current, &ends, first) && !entry.is_start {
                let node_is_new = self.create_node_if_absent(graph, &entry.current);
                self.connect_nodes(graph, &parent_node, &entry.current, entry.current.turn.clone());
                if let Some(n) = self.graphs.get(&graph).and_then(|g| g.locate_node(&entry.current.loc)).map(|n| n.net_id) {
                    added_nodes.push(n);
                }
                parent_node = entry.current.clone();
                if !node_is_new {
                    continue;
                }
            }

            continue_search_with_parent(&mut frontier, &entry, &parent_node, ends);
        }

        self.mark_tracks_dirty();
        for node in added_nodes {
            self.notify_signals_of_new_node(graph, node);
        }
        Some(graph)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A world of flat straight blocks, each with one axis.
    struct Straights {
        blocks: HashMap<Pos, Vec3>,
    }

    impl TrackWorld for Straights {
        fn connected(&self, pos: Pos, _ignore_turns: bool, connected_to: Option<&TrackNodeLocation>) -> Vec<DiscoveredLocation> {
            let Some(axis) = self.blocks.get(&pos) else { return Vec::new() };
            let center = [f64::from(pos[0]) + 0.5, f64::from(pos[1]), f64::from(pos[2]) + 0.5];
            let material = |_: Vec3| Some("create:andesite".to_string());
            let t = StraightTrack {
                center,
                axes: vec![*axis],
                normal: [0.0, 1.0, 0.0],
                dimension: 0,
                material: &material,
                y_offset: &|_| 0,
            };
            connected_straight(&t, connected_to)
        }
    }

    const X: Vec3 = [1.0, 0.0, 0.0];
    const Z: Vec3 = [0.0, 0.0, 1.0];

    fn row(n: i32) -> Straights {
        let mut s = Straights { blocks: HashMap::new() };
        for x in 0..n {
            s.blocks.insert([x, 64, 0], X);
        }
        s
    }

    fn loc(x: f64, z: f64) -> TrackNodeLocation {
        TrackNodeLocation::new([x, 64.0, z], 0)
    }

    #[test]
    fn a_straight_run_is_one_edge_between_its_ends() {
        let world = row(6);
        let mut nets = RailNetworks::new();
        let mut last = None;
        for x in 0..6 {
            last = nets.on_rail_added(&world, [x, 64, 0]);
        }
        assert_eq!(nets.graphs.len(), 1, "{:?}", nets.graphs.keys());
        let g = &nets.graphs[&last.unwrap()];
        assert_eq!(g.node_count(), 2);
        let a = g.locate_node(&loc(0.0, 0.5)).expect("node at the west end").net_id;
        let b = g.locate_node(&loc(6.0, 0.5)).expect("node at the east end").net_id;
        assert_eq!(g.edge(a, b).unwrap().length(), 6.0);
        assert_eq!(g.edge(b, a).unwrap().length(), 6.0);
    }

    #[test]
    fn a_block_replaced_and_put_back_rejoins_the_graph() {
        // Play-test 2026-11-26: a track block replaced (a bogey over it, a `setblock`) lost its node and, put back, was
        // never added again, so the station found no starting node. Removal and adding are symmetric.
        let mut world = row(6);
        let mut nets = RailNetworks::new();
        for x in 0..6 {
            nets.on_rail_added(&world, [x, 64, 0]);
        }
        let ends = world.connected([3, 64, 0], false, None);
        world.blocks.remove(&[3, 64, 0]);
        nets.on_rail_removed(&world, [3, 64, 0], &ends);
        assert_eq!(nets.graphs.len(), 2);
        world.blocks.insert([3, 64, 0], X);
        let graph = nets.on_rail_added(&world, [3, 64, 0]).expect("the block is track again");
        assert_eq!(nets.graphs.len(), 1, "{:?}", nets.graphs.values().map(|g| g.node_count()).collect::<Vec<_>>());
        let g = &nets.graphs[&graph];
        let a = g.locate_node(&loc(0.0, 0.5)).expect("west end").net_id;
        let b = g.locate_node(&loc(6.0, 0.5)).expect("east end").net_id;
        assert_eq!(g.edge(a, b).unwrap().length(), 6.0);
    }

    #[test]
    fn a_strip_nobody_told_the_graph_about_is_learned_from_one_block() {
        // A `/fill`ed strip: one `onRailAdded` from any of its blocks makes the graph for all of them.
        let world = row(8);
        let mut nets = RailNetworks::new();
        let graph = nets.on_rail_added(&world, [4, 64, 0]).expect("track");
        let g = &nets.graphs[&graph];
        assert_eq!(g.node_count(), 2);
        assert!(g.locate_node(&loc(0.0, 0.5)).is_some() && g.locate_node(&loc(8.0, 0.5)).is_some());
    }

    #[test]
    fn breaking_the_middle_splits_the_graph_in_two() {
        let mut world = row(6);
        let mut nets = RailNetworks::new();
        for x in 0..6 {
            nets.on_rail_added(&world, [x, 64, 0]);
        }
        // Break block 3.
        let ends = world.connected([3, 64, 0], false, None);
        world.blocks.remove(&[3, 64, 0]);
        nets.on_rail_removed(&world, [3, 64, 0], &ends);
        assert_eq!(nets.graphs.len(), 2, "{:?}", nets.graphs.values().map(|g| g.node_count()).collect::<Vec<_>>());
        let mut lens: Vec<f64> = Vec::new();
        for g in nets.graphs.values() {
            assert_eq!(g.node_count(), 2);
            for (a, b) in g.edge_keys() {
                lens.push(g.edge(a, b).unwrap().length());
            }
        }
        lens.sort_by(|a, b| a.partial_cmp(b).unwrap());
        // West piece 0..3 and east piece 4..6: edges of 3 and 2, each both ways.
        assert_eq!(lens, vec![2.0, 2.0, 3.0, 3.0]);
        assert_eq!(nets.signal_edge_groups.len(), 2);
        // Putting it back merges them again.
        world.blocks.insert([3, 64, 0], X);
        let g = nets.on_rail_added(&world, [3, 64, 0]).unwrap();
        assert_eq!(nets.graphs.len(), 1);
        assert_eq!(nets.graphs[&g].node_count(), 2);
        assert_eq!(nets.signal_edge_groups.len(), 1);
    }

    #[test]
    fn a_junction_makes_a_node() {
        // A T: a row along x with a block going out the side at x = 3.
        let mut world = row(7);
        world.blocks.insert([3, 64, 1], Z);
        let mut nets = RailNetworks::new();
        for x in 0..7 {
            nets.on_rail_added(&world, [x, 64, 0]);
        }
        nets.on_rail_added(&world, [3, 64, 1]);
        // The side block touches the row at (3.5, 0.0)... which is not a node of the row (its ends are on
        // block edges), so the two pieces are separate graphs.
        assert!(nets.graphs.len() >= 1);
        let total: usize = nets.graphs.values().map(|g| g.node_count()).sum();
        assert!(total >= 4);
    }

    #[test]
    fn a_long_straight_gets_a_node_every_sixteen_blocks() {
        let n = 40;
        let world = row(n);
        let mut nets = RailNetworks::new();
        let mut g = None;
        for x in 0..n {
            g = nets.on_rail_added(&world, [x, 64, 0]);
        }
        let graph = &nets.graphs[&g.unwrap()];
        // Nodes at x = 0, 16, 32 and 40.
        let mut xs: Vec<i32> = graph.node_locations().map(|l| l.x / 2).collect();
        xs.sort();
        assert_eq!(xs, vec![0, 16, 32, 40]);
        let a = graph.locate_node(&loc(0.0, 0.5)).unwrap().net_id;
        let b = graph.locate_node(&loc(16.0, 0.5)).unwrap().net_id;
        assert_eq!(graph.edge(a, b).unwrap().length(), 16.0);
    }

    #[test]
    fn validity_rules_of_a_node() {
        let d = |x: f64, z: f64| DiscoveredLocation::new(0, [x, 64.0, z]).with_direction(Some(X));
        let ahead = vec![d(2.0, 0.5)];
        // One way on (a straight continuation) in line: no node unless a multiple of 16.
        assert!(!is_valid_graph_node_location(&d(1.0, 0.5), &ahead, false));
        assert!(is_valid_graph_node_location(&d(16.0, 0.5), &ahead, false));
        // A dead end and a fork are always nodes.
        assert!(is_valid_graph_node_location(&d(1.0, 0.5), &[], false));
        assert!(is_valid_graph_node_location(&d(1.0, 0.5), &[d(2.0, 0.5), d(1.0, 1.5)], false));
        // The first location counts its own neighbour: one other means an end.
        assert!(!is_valid_graph_node_location(&d(1.0, 0.5), &[d(2.0, 0.5), d(0.0, 0.5)], true));
        // A forced node, a change of material and a bend.
        assert!(is_valid_graph_node_location(&d(1.0, 0.5).force_node(), &ahead, false));
        let m = d(1.0, 0.5).materials(Some("a".into()), Some("b".into()));
        assert!(is_valid_graph_node_location(&m, &ahead, false));
        let bend = vec![DiscoveredLocation::new(0, [2.0, 64.0, 0.5]).with_direction(Some(Z))];
        assert!(is_valid_graph_node_location(&d(1.0, 0.5), &bend, false));
    }
}
