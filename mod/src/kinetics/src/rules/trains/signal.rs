//! Signals: `SignalBoundary` (content/trains/signal/SignalBoundary.java), `SignalEdgeGroup`,
//! `EdgeGroupColor`, `SignalPropagator`, and the signal part of `GlobalRailwayManager.tick`.
//!
//! A signal boundary sits on an edge and splits the track into blocks. Each side of it faces one
//! block, a **signal edge group**: the stretch of track between this boundary and the next ones, found
//! by walking the graph from the boundary until it meets other boundaries ([`RailNetworks::propagate_signal_group`]).
//! A group is occupied when a train is on it ([`SignalEdgeGroup::trains`], filled each tick by the
//! trains) or when a station has reserved it ([`SignalEdgeGroup::reserved`]); crossing tracks tie their
//! groups together (`intersecting`), so occupying one occupies the other.
//!
//! [`RailNetworks::tick_points`] is `TrackEdgePoint.tick(graph, preTrains)` for signals: before the
//! trains run it re-propagates groups for sides marked for update; after, it recomputes what each
//! side shows (red when its block is occupied or a lever forces it, green otherwise, or for a
//! cross signal green/yellow/red by the signals ahead).
//!
//! Left out: the block entity side (`SignalBlockEntity`, the overlay rendering, `OverlayState` aside),
//! NBT and packets, and `SignalEdgeGroup`'s resolved-set cache (the walk is recomputed instead, which
//! gives the same answers because Java clears the cache on every change).
//! Java quirk kept: `onSignalRemoved` calls `signal.sidesToUpdate.map($ -> false)` and throws the
//! result away, so the removed signal's own pending sides still skip their walk.

use super::graph::{EdgeData, EdgePointType, NodeId, PASSIVE_GROUP, TrackEdgePoint, TrackNodeLocation, Uuid};
use super::network::{RailEvent, RailNetworks};
use crate::Pos;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// `SignalBlock.SignalType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SignalType {
    #[default]
    EntrySignal,
    CrossSignal,
}

/// `SignalBlockEntity.SignalState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SignalState {
    Red,
    Yellow,
    Green,
    #[default]
    Invalid,
}

/// `SignalBlockEntity.OverlayState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayState {
    Skip,
    Render,
    Dual,
}

/// `EdgeGroupColor`, in ordinal order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EdgeGroupColor {
    Yellow,
    Green,
    Blue,
    Orange,
    Lavender,
    Red,
    Cyan,
    Brown,
    White,
}

impl EdgeGroupColor {
    pub const ALL: [EdgeGroupColor; 9] = [
        EdgeGroupColor::Yellow,
        EdgeGroupColor::Green,
        EdgeGroupColor::Blue,
        EdgeGroupColor::Orange,
        EdgeGroupColor::Lavender,
        EdgeGroupColor::Red,
        EdgeGroupColor::Cyan,
        EdgeGroupColor::Brown,
        EdgeGroupColor::White,
    ];

    /// The colour as 0xRRGGBB.
    pub fn rgb(self) -> u32 {
        match self {
            EdgeGroupColor::Yellow => 0xEBC255,
            EdgeGroupColor::Green => 0x51C054,
            EdgeGroupColor::Blue => 0x5391E1,
            EdgeGroupColor::Orange => 0xE36E36,
            EdgeGroupColor::Lavender => 0xCB92BA,
            EdgeGroupColor::Red => 0xA43538,
            EdgeGroupColor::Cyan => 0x6EDAD9,
            EdgeGroupColor::Brown => 0xA17C58,
            EdgeGroupColor::White => 0xE5E1DC,
        }
    }

    /// `getDefault()`: the first.
    pub fn default_color() -> EdgeGroupColor {
        EdgeGroupColor::Yellow
    }

    /// `strikeFrom(mask)`: white is never struck.
    pub fn strike_from(self, mask: u32) -> u32 {
        if self == EdgeGroupColor::White { mask } else { mask | (1 << self as u32) }
    }

    /// `findNextAvailable(mask)`: the first colour whose bit is free, else white.
    pub fn find_next_available(mut mask: u32) -> EdgeGroupColor {
        for v in Self::ALL {
            if mask & 1 == 0 {
                return v;
            }
            mask >>= 1;
        }
        EdgeGroupColor::White
    }
}

fn idx(b: bool) -> usize {
    if b { 0 } else { 1 }
}

/// `SignalBoundary`. Index 0 is `Couple.first` (the front / primary side), 1 the second.
#[derive(Clone, Debug, PartialEq)]
pub struct SignalBoundary {
    /// The signal block entities facing each side, with whether each reports power (`forcedRed`).
    pub block_entities: [BTreeMap<Pos, bool>; 2],
    pub types: [SignalType; 2],
    /// The signal edge group each side faces.
    pub groups: [Option<Uuid>; 2],
    pub sides_to_update: [bool; 2],
    pub cached_states: [SignalState; 2],
    /// The signals ahead a cross signal waits for, with the side of each that faces it.
    chained_signals: [Option<BTreeMap<Uuid, bool>>; 2],
}

impl Default for SignalBoundary {
    fn default() -> Self {
        Self::new()
    }
}

impl SignalBoundary {
    pub fn new() -> SignalBoundary {
        SignalBoundary {
            block_entities: [BTreeMap::new(), BTreeMap::new()],
            types: [SignalType::EntrySignal; 2],
            groups: [None, None],
            sides_to_update: [true, true],
            cached_states: [SignalState::Invalid; 2],
            chained_signals: [None, None],
        }
    }

    /// The group a side faces (`getGroup(side)` with `primary = isPrimary(side)`).
    pub fn group(&self, primary: bool) -> Option<Uuid> {
        self.groups[idx(primary)]
    }

    /// `canNavigateVia(side)`: a train may pass only where a signal faces it.
    pub fn can_navigate_via(&self, primary: bool) -> bool {
        !self.block_entities[idx(primary)].is_empty()
    }

    /// `queueUpdate(side)`.
    pub fn queue_update(&mut self, primary: bool) {
        self.sides_to_update[idx(primary)] = true;
    }

    /// `isForcedRed(primary)`: any signal on the side reports power.
    pub fn is_forced_red(&self, primary: bool) -> bool {
        self.block_entities[idx(primary)].values().any(|b| *b)
    }

    /// `blockEntityAdded(blockEntity, front)`: the first signal on a side sets the side's type.
    pub fn block_entity_added(&mut self, pos: Pos, front: bool, signal_type: Option<SignalType>, reported_power: bool) {
        let side = &mut self.block_entities[idx(front)];
        if side.is_empty() {
            if let Some(t) = signal_type {
                self.types[idx(front)] = t;
            }
        }
        side.insert(pos, reported_power);
    }

    /// `updateBlockEntityPower`.
    pub fn update_block_entity_power(&mut self, pos: Pos, power: bool) {
        for side in &mut self.block_entities {
            if let Some(v) = side.get_mut(&pos) {
                *v = power;
            }
        }
    }

    /// `blockEntityRemoved(pos, front)`: true when none are left, so the point should go.
    pub fn block_entity_removed(&mut self, pos: Pos) -> bool {
        for side in &mut self.block_entities {
            side.remove(&pos);
        }
        self.block_entities.iter().all(|s| s.is_empty())
    }

    /// `getOverlayFor(blockEntity)`: as Java does, looks only at the first block entity of the first
    /// non-empty side.
    pub fn overlay_for(&self, pos: Pos) -> OverlayState {
        for first in [true, false] {
            let set = &self.block_entities[idx(first)];
            if let Some(p) = set.keys().next() {
                if *p == pos {
                    return if self.block_entities[idx(!first)].is_empty() { OverlayState::Render } else { OverlayState::Dual };
                }
                return OverlayState::Skip;
            }
        }
        OverlayState::Skip
    }

    /// `getTypeFor(blockEntity)`.
    pub fn type_for(&self, pos: Pos) -> SignalType {
        self.types[idx(self.block_entities[0].contains_key(&pos))]
    }

    /// `getStateFor(blockEntity)`.
    pub fn state_for(&self, pos: Pos) -> SignalState {
        for first in [true, false] {
            if self.block_entities[idx(first)].contains_key(&pos) {
                return self.cached_states[idx(first)];
            }
        }
        SignalState::Invalid
    }

    /// `cycleSignalType(pos)`: entry to cross and back, for the side holding the block entity.
    pub fn cycle_signal_type(&mut self, pos: Pos) {
        let side = idx(self.block_entities[0].contains_key(&pos));
        self.types[side] = match self.type_for(pos) {
            SignalType::EntrySignal => SignalType::CrossSignal,
            SignalType::CrossSignal => SignalType::EntrySignal,
        };
    }
}

/// `SignalEdgeGroup`: one block of track.
#[derive(Clone, Debug, PartialEq)]
pub struct SignalEdgeGroup {
    pub id: Uuid,
    pub color: EdgeGroupColor,
    /// The trains on the group this tick (by train id); the trains refill it every tick.
    pub trains: BTreeSet<Uuid>,
    /// The signal boundary a station reserved the group for.
    pub reserved: Option<Uuid>,
    /// Crossing id to the group on the other track.
    pub intersecting: BTreeMap<Uuid, Uuid>,
    pub adjacent: BTreeSet<Uuid>,
    pub fallback_group: bool,
}

impl SignalEdgeGroup {
    pub fn new(id: Uuid) -> SignalEdgeGroup {
        SignalEdgeGroup {
            id,
            color: EdgeGroupColor::default_color(),
            trains: BTreeSet::new(),
            reserved: None,
            intersecting: BTreeMap::new(),
            adjacent: BTreeSet::new(),
            fallback_group: false,
        }
    }

    pub fn as_fallback(mut self) -> SignalEdgeGroup {
        self.fallback_group = true;
        self
    }

    /// `isThisOccupiedUnless(train)`.
    fn is_this_occupied_unless_train(&self, train: Uuid) -> bool {
        self.reserved.is_some() || self.trains.len() > 1 || (!self.trains.contains(&train) && !self.trains.is_empty())
    }

    /// `isThisOccupiedUnless(boundary)`.
    fn is_this_occupied_unless_boundary(&self, boundary: Uuid) -> bool {
        !self.trains.is_empty() || self.reserved.is_some_and(|r| r != boundary)
    }

    pub fn put_adjacent(&mut self, adjacent: Uuid) {
        self.adjacent.insert(adjacent);
    }
    pub fn remove_adjacent(&mut self, adjacent: Uuid) {
        self.adjacent.remove(&adjacent);
    }
}

/// How `SignalPropagator.walkSignals` is called: what it does at each boundary and non-boundary edge.
enum Walk<'a> {
    /// `onSignalRemoved` and `notifySignalsOfNewNode`: queue every boundary reached for an update,
    /// reset the edges between them to the passive group.
    Requeue,
    /// `propagateSignalGroup`: give everything reached this group.
    Assign(Uuid),
    /// `collectChainedSignals`: note the boundaries reached.
    Collect(&'a mut BTreeMap<Uuid, bool>),
}

impl RailNetworks {
    // --- groups ------------------------------------------------------------------------------

    /// `walkIntersecting`: the group and every group tied to it through crossings.
    fn walk_intersecting(&self, start: Uuid) -> BTreeSet<Uuid> {
        let mut visited = BTreeSet::new();
        let mut stack = vec![start];
        while let Some(id) = stack.pop() {
            if !visited.insert(id) {
                continue;
            }
            if let Some(g) = self.signal_edge_groups.get(&id) {
                for t in g.intersecting.values() {
                    if self.signal_edge_groups.contains_key(t) {
                        stack.push(*t);
                    }
                }
            }
        }
        visited
    }

    /// `isOccupiedUnless(train)`: is the group, or one tied to it, taken by something other than this train?
    pub fn is_group_occupied_unless_train(&self, group: Uuid, train: Uuid) -> bool {
        self.walk_intersecting(group)
            .iter()
            .filter_map(|g| self.signal_edge_groups.get(g))
            .any(|g| g.is_this_occupied_unless_train(train))
    }

    /// `isOccupiedUnless(boundary)`.
    pub fn is_group_occupied_unless_boundary(&self, group: Uuid, boundary: Uuid) -> bool {
        self.walk_intersecting(group)
            .iter()
            .filter_map(|g| self.signal_edge_groups.get(g))
            .any(|g| g.is_this_occupied_unless_boundary(boundary))
    }

    /// `putIntersection(intersectionId, targetGroup)`.
    pub fn put_group_intersection(&mut self, group: Uuid, intersection_id: Uuid, target: Uuid) {
        if let Some(g) = self.signal_edge_groups.get_mut(&group) {
            g.intersecting.insert(intersection_id, target);
        }
        self.resolve_group_color(group);
    }

    /// `removeIntersection(intersectionId)`.
    pub fn remove_group_intersection(&mut self, group: Uuid, intersection_id: Uuid) {
        let removed = self.signal_edge_groups.get_mut(&group).and_then(|g| g.intersecting.remove(&intersection_id));
        if let Some(r) = removed {
            if let Some(other) = self.signal_edge_groups.get_mut(&r) {
                other.intersecting.remove(&intersection_id);
            }
        }
        self.resolve_group_color(group);
    }

    /// `resolveColor()`: the first colour no neighbouring block (beyond the crossing set) uses.
    pub fn resolve_group_color(&mut self, group: Uuid) {
        let resolved = self.walk_intersecting(group);
        let mut mask = 0_u32;
        for g in &resolved {
            let Some(grp) = self.signal_edge_groups.get(g) else { continue };
            for adj in &grp.adjacent {
                if resolved.contains(adj) {
                    continue;
                }
                if let Some(a) = self.signal_edge_groups.get(adj) {
                    mask = a.color.strike_from(mask);
                }
            }
        }
        let new_colour = EdgeGroupColor::find_next_available(mask);
        let Some(current) = self.signal_edge_groups.get(&group).map(|g| g.color) else { return };
        if new_colour == current {
            return;
        }
        for g in resolved {
            if let Some(grp) = self.signal_edge_groups.get_mut(&g) {
                grp.color = new_colour;
                self.events.push(RailEvent::GroupCreated { id: g, color: new_colour });
            }
        }
        self.mark_tracks_dirty();
    }

    /// The start of `GlobalRailwayManager.tick`: every group forgets its trains and reservation.
    pub fn clear_group_occupancy(&mut self) {
        for g in self.signal_edge_groups.values_mut() {
            g.trains.clear();
            g.reserved = None;
        }
    }

    // --- boundaries ---------------------------------------------------------------------------

    fn signal_mut(&mut self, graph: Uuid, id: Uuid) -> Option<&mut SignalBoundary> {
        self.graphs.get_mut(&graph)?.edge_points.get_mut(EdgePointType::Signal, id)?.signal_mut()
    }

    fn signal(&self, graph: Uuid, id: Uuid) -> Option<&SignalBoundary> {
        self.graphs.get(&graph)?.edge_points.get(EdgePointType::Signal, id)?.signal()
    }

    /// `SignalBoundary.setGroup(primary, groupId)`: also keeps the groups' `adjacent` sets.
    pub fn signal_set_group(&mut self, graph: Uuid, id: Uuid, primary: bool, group_id: Option<Uuid>) {
        let (previous, opposite) = {
            let Some(s) = self.signal_mut(graph, id) else { return };
            let previous = s.groups[idx(primary)];
            s.groups[idx(primary)] = group_id;
            (previous, s.groups[idx(!primary)])
        };
        if let Some(opp) = opposite {
            if let Some(og) = self.signal_edge_groups.get_mut(&opp) {
                if let Some(p) = previous {
                    og.remove_adjacent(p);
                }
                if let Some(g) = group_id {
                    og.put_adjacent(g);
                }
            }
        }
        if let Some(g) = group_id {
            if let Some(group) = self.signal_edge_groups.get_mut(&g) {
                if let Some(opp) = opposite {
                    group.put_adjacent(opp);
                }
            }
        }
    }

    /// `setGroupAndUpdate(side, groupId)`.
    fn signal_set_group_and_update(&mut self, graph: Uuid, id: Uuid, side: &TrackNodeLocation, group_id: Uuid) {
        let Some(primary) = self.graphs.get(&graph).and_then(|g| g.point(EdgePointType::Signal, id)).map(|p| p.is_primary(side))
        else {
            return;
        };
        self.signal_set_group(graph, id, primary, Some(group_id));
        if let Some(s) = self.signal_mut(graph, id) {
            s.sides_to_update[idx(primary)] = false;
            s.chained_signals[idx(primary)] = None;
        }
    }

    fn remove_group_with_event(&mut self, id: Uuid) {
        if self.signal_edge_groups.remove(&id).is_some() {
            self.events.push(RailEvent::GroupRemoved { id });
        }
    }

    // --- SignalPropagator ----------------------------------------------------------------------

    /// `SignalBoundary.onRemoved` -> `SignalPropagator.onSignalRemoved(graph, signal)`: the blocks on
    /// both sides merge into their neighbours, so every boundary reached is queued to recompute.
    pub(super) fn on_signal_removed(&mut self, graph: Uuid, signal: &TrackEdgePoint) {
        let Some(sb) = signal.signal() else { return };
        for front in [true, false] {
            // Java discards the result of `sidesToUpdate.map($ -> false)`, so the real flag decides.
            if sb.sides_to_update[idx(front)] {
                continue;
            }
            if let Some(id) = sb.groups[idx(front)] {
                self.remove_group_with_event(id);
            }
            self.walk_signals(graph, signal, front, &mut Walk::Requeue, false);
        }
    }

    /// `SignalPropagator.notifySignalsOfNewNode(graph, node)`.
    pub fn notify_signals_of_new_node(&mut self, graph: Uuid, node: NodeId) {
        let mut frontier: VecDeque<(NodeId, Option<NodeId>)> = VecDeque::new();
        frontier.push_back((node, None));
        self.walk_frontier(graph, frontier, &mut Walk::Requeue, false);
    }

    /// `SignalPropagator.propagateSignalGroup(graph, signal, front)`: a new group for this side, spread
    /// over the whole block up to the next boundaries.
    pub fn propagate_signal_group(&mut self, graph: Uuid, signal_id: Uuid, front: bool) {
        let group_id = self.new_uuid();
        let group = SignalEdgeGroup::new(group_id);
        self.signal_edge_groups.insert(group_id, group);
        self.signal_set_group(graph, signal_id, front, Some(group_id));
        self.events.push(RailEvent::PointAdded { graph, ty: EdgePointType::Signal, id: signal_id });

        let Some(signal) = self.graphs.get(&graph).and_then(|g| g.point(EdgePointType::Signal, signal_id)).cloned() else {
            return;
        };
        self.walk_signals(graph, &signal, front, &mut Walk::Assign(group_id), false);

        self.resolve_group_color(group_id);
        let color = self.signal_edge_groups.get(&group_id).map(|g| g.color).unwrap_or_else(EdgeGroupColor::default_color);
        self.events.push(RailEvent::GroupCreated { id: group_id, color });
    }

    /// `SignalPropagator.collectChainedSignals(graph, signal, front)`: the boundaries a cross signal
    /// waits on, each with the side that faces it.
    pub fn collect_chained_signals(&mut self, graph: Uuid, signal_id: Uuid, front: bool) -> BTreeMap<Uuid, bool> {
        let mut map = BTreeMap::new();
        if let Some(signal) = self.graphs.get(&graph).and_then(|g| g.point(EdgePointType::Signal, signal_id)).cloned() {
            self.walk_signals(graph, &signal, front, &mut Walk::Collect(&mut map), true);
        }
        map
    }

    fn walk_signals(&mut self, graph: Uuid, signal: &TrackEdgePoint, front: bool, mode: &mut Walk, for_collection: bool) {
        let (start_nodes, start_edge_nodes) = {
            let Some(g) = self.graphs.get(&graph) else { return };
            let n: Vec<Option<NodeId>> = signal.edge_location.iter().map(|l| g.locate_node(l).map(|n| n.net_id)).collect();
            let [Some(a), Some(b)] = n[..] else { return };
            ([a, b], [(a, b), (b, a)])
        };
        let node1 = start_nodes[idx(front)];
        let node2 = start_nodes[idx(!front)];
        let (se_from, se_to) = start_edge_nodes[idx(front)];
        let (oe_from, oe_to) = start_edge_nodes[idx(!front)];

        let immediate = {
            let Some(g) = self.graphs.get(&graph) else { return };
            let Some(start_edge) = g.edge(se_from, se_to) else { return };
            let loc_on = signal.location_on(start_edge);
            start_edge.edge_data.next_of(EdgePointType::Signal, loc_on)
        };

        if !for_collection {
            self.notify_trains(graph, &[(se_from, se_to), (oe_from, oe_to)]);
            self.edge_refresh_intersecting_signal_groups(graph, se_from, se_to);
            self.events.push(RailEvent::EdgeDataChanged { graph, from: node1, to: node2 });
        }

        // Check for signal on the same edge
        if let Some(r) = immediate {
            if self.walk_boundary_cb(graph, mode, node1, r.id) {
                self.edge_refresh_intersecting_signal_groups(graph, se_from, se_to);
            }
            return;
        }

        // Search for any connected signals
        let mut frontier: VecDeque<(NodeId, Option<NodeId>)> = VecDeque::new();
        frontier.push_back((node2, Some(node1)));
        self.walk_frontier(graph, frontier, mode, for_collection);
    }

    fn walk_frontier(
        &mut self,
        graph: Uuid,
        mut frontier: VecDeque<(NodeId, Option<NodeId>)>,
        mode: &mut Walk,
        for_collection: bool,
    ) {
        let mut visited: BTreeSet<(NodeId, NodeId)> = BTreeSet::new();
        while let Some((current, prev)) = frontier.pop_front() {
            let nexts: Vec<NodeId> = match self.graphs.get(&graph).and_then(|g| g.connections_from(current)) {
                Some(m) => m.keys().copied().collect(),
                None => Vec::new(),
            };
            'edge_walk: for next in nexts {
                if Some(next) == prev {
                    continue;
                }
                // already checked this edge
                if !visited.insert((current, next)) {
                    continue;
                }
                // chain signal: check if reachable
                if for_collection {
                    let Some(g) = self.graphs.get(&graph) else { return };
                    if let Some(p) = prev {
                        let (Some(from_prev), Some(edge)) = (g.edge(p, current), g.edge(current, next)) else {
                            continue;
                        };
                        if !from_prev.can_travel_to(edge) {
                            continue;
                        }
                    }
                }
                visited.insert((next, current));

                for flip in [false, true] {
                    let (from, to) = if flip { (next, current) } else { (current, next) };
                    let (has_boundaries, first_boundary) = {
                        let Some(g) = self.graphs.get(&graph) else { return };
                        let Some(e) = g.edge(from, to) else { continue };
                        (
                            e.edge_data.has_signal_boundaries(),
                            e.edge_data.next_of(EdgePointType::Signal, 0.0),
                        )
                    };

                    // no boundary- update group of edge
                    if !has_boundaries {
                        if self.walk_non_boundary_cb(graph, mode, from, to) {
                            self.notify_trains(graph, &[(from, to)]);
                            self.events.push(RailEvent::EdgeDataChanged { graph, from: current, to: next });
                        }
                        continue;
                    }

                    // other/own boundary found
                    let Some(b) = first_boundary else { continue };
                    if self.walk_boundary_cb(graph, mode, current, b.id) {
                        self.notify_trains(graph, &[(current, next), (next, current)]);
                        self.edge_refresh_intersecting_signal_groups(graph, from, to);
                        self.events.push(RailEvent::EdgeDataChanged { graph, from: current, to: next });
                    }
                    continue 'edge_walk;
                }

                frontier.push_back((next, Some(current)));
            }
        }
    }

    /// The boundary callback of each walk, for the boundary `id` reached from `node1`.
    fn walk_boundary_cb(&mut self, graph: Uuid, mode: &mut Walk, node1: NodeId, id: Uuid) -> bool {
        let Some(point) = self.graphs.get(&graph).and_then(|g| g.point(EdgePointType::Signal, id)).cloned() else {
            return false;
        };
        let Some(node_loc) = self.graphs.get(&graph).and_then(|g| g.get_node(node1)).map(|n| n.location) else {
            return false;
        };
        let primary = point.is_primary(&node_loc);
        match mode {
            Walk::Requeue => {
                if let Some(s) = self.signal_mut(graph, id) {
                    s.queue_update(primary);
                }
                false
            }
            Walk::Assign(group_id) => {
                let group_id = *group_id;
                if let Some(current) = point.signal().and_then(|s| s.group(primary)) {
                    self.remove_group_with_event(current);
                }
                self.signal_set_group_and_update(graph, id, &node_loc, group_id);
                self.events.push(RailEvent::PointAdded { graph, ty: EdgePointType::Signal, id });
                true
            }
            Walk::Collect(map) => {
                map.insert(id, !primary);
                false
            }
        }
    }

    /// The non-boundary callback: an edge with no boundary on it.
    fn walk_non_boundary_cb(&mut self, graph: Uuid, mode: &mut Walk, from: NodeId, to: NodeId) -> bool {
        match mode {
            Walk::Requeue => {
                let has = self.graphs.get(&graph).and_then(|g| g.edge(from, to)).is_some_and(|e| e.edge_data.has_signal_boundaries());
                if !has {
                    self.edge_set_single_signal_group(graph, from, to, Some(PASSIVE_GROUP));
                    return true;
                }
                false
            }
            Walk::Assign(group_id) => {
                let group_id = *group_id;
                let single = self.graphs.get(&graph).and_then(|g| g.edge(from, to)).and_then(|e| e.edge_data.single_signal_group);
                if let Some(s) = single {
                    self.remove_group_with_event(s);
                }
                self.edge_set_single_signal_group(graph, from, to, Some(group_id));
                true
            }
            Walk::Collect(_) => false,
        }
    }

    /// `SignalPropagator.notifyTrains(graph, edges)`.
    fn notify_trains(&mut self, graph: Uuid, edges: &[(NodeId, NodeId)]) {
        self.events.push(RailEvent::SignalBlocksStale { graph, edges: edges.to_vec() });
    }

    // --- ticking ------------------------------------------------------------------------------

    /// `TrackGraph.tickPoints(preTrains)` for every signal of the graph.
    pub fn tick_points(&mut self, graph: Uuid, pre_trains: bool) {
        let ids: Vec<Uuid> = match self.graphs.get(&graph) {
            Some(g) => g.edge_points.ids(EdgePointType::Signal),
            None => return,
        };
        for id in ids {
            if !pre_trains {
                self.tick_signal_state(graph, id);
                continue;
            }
            for front in [true, false] {
                let pending = self.signal(graph, id).is_some_and(|s| s.sides_to_update[idx(front)]);
                if !pending {
                    continue;
                }
                if let Some(s) = self.signal_mut(graph, id) {
                    s.sides_to_update[idx(front)] = false;
                }
                self.propagate_signal_group(graph, id, front);
                if let Some(s) = self.signal_mut(graph, id) {
                    s.chained_signals[idx(front)] = None;
                }
            }
        }
    }

    /// `GlobalRailwayManager.tick`, the points' half: before the trains, then after.
    pub fn tick_all_points(&mut self, pre_trains: bool) {
        let ids: Vec<Uuid> = self.graphs.keys().copied().collect();
        for g in ids {
            self.tick_points(g, pre_trains);
            if pre_trains {
                self.resolve_intersecting_edge_groups(g);
            }
        }
    }

    /// `SignalBoundary.tickState(graph)`.
    fn tick_signal_state(&mut self, graph: Uuid, id: Uuid) {
        let Some(sb) = self.signal(graph, id).cloned() else { return };
        for current in [true, false] {
            let i = idx(current);
            if sb.block_entities[i].is_empty() {
                continue;
            }
            let forced_red = sb.is_forced_red(current);
            let group = sb.groups[i];
            if group == sb.groups[idx(!current)] {
                self.set_cached_state(graph, id, current, SignalState::Invalid);
                continue;
            }
            let Some(group) = group.filter(|g| self.signal_edge_groups.contains_key(g)) else {
                self.set_cached_state(graph, id, current, SignalState::Invalid);
                continue;
            };
            let occupied = forced_red || self.is_group_occupied_unless_boundary(group, id);
            let state = if occupied { SignalState::Red } else { self.resolve_signal_chain(graph, id, current) };
            self.set_cached_state(graph, id, current, state);
        }
    }

    fn set_cached_state(&mut self, graph: Uuid, id: Uuid, side: bool, state: SignalState) {
        if let Some(s) = self.signal_mut(graph, id) {
            s.cached_states[idx(side)] = state;
        }
    }

    /// `resolveSignalChain(graph, side)`.
    fn resolve_signal_chain(&mut self, graph: Uuid, id: Uuid, side: bool) -> SignalState {
        let i = idx(side);
        let Some(ty) = self.signal(graph, id).map(|s| s.types[i]) else { return SignalState::Invalid };
        if ty != SignalType::CrossSignal {
            return SignalState::Green;
        }
        if self.signal(graph, id).is_some_and(|s| s.chained_signals[i].is_none()) {
            let collected = self.collect_chained_signals(graph, id, side);
            if let Some(s) = self.signal_mut(graph, id) {
                s.chained_signals[i] = Some(collected);
            }
        }
        let chained = self.signal(graph, id).and_then(|s| s.chained_signals[i].clone()).unwrap_or_default();

        let mut all_paths_free = true;
        let mut no_paths_free = true;
        let mut invalid = false;
        for (uuid, side_of_other) in chained {
            let Some(other) = self.signal(graph, uuid) else {
                invalid = true;
                break;
            };
            if other.block_entities[idx(side_of_other)].is_empty() {
                continue;
            }
            let other_state = other.cached_states[idx(side_of_other)];
            all_paths_free &= other_state == SignalState::Green || other_state == SignalState::Invalid;
            no_paths_free &= other_state == SignalState::Red;
        }
        if invalid {
            if let Some(s) = self.signal_mut(graph, id) {
                s.chained_signals[i] = None;
            }
            return SignalState::Invalid;
        }
        if all_paths_free {
            return SignalState::Green;
        }
        if no_paths_free {
            return SignalState::Red;
        }
        SignalState::Yellow
    }

    // --- boundary lifetime ---------------------------------------------------------------------

    /// The first signal block entity placed at a spot: makes the boundary (not yet on any graph) and
    /// gives it its first side. Place it with [`RailNetworks::add_point`].
    pub fn new_signal_boundary(
        &mut self,
        edge_location: [TrackNodeLocation; 2],
        position: f64,
        pos: Pos,
        front: bool,
        signal_type: SignalType,
        reported_power: bool,
    ) -> TrackEdgePoint {
        let id = self.new_uuid();
        let mut p = TrackEdgePoint::new_signal(id, edge_location, position);
        if let Some(s) = p.signal_mut() {
            s.block_entity_added(pos, front, Some(signal_type), reported_power);
        }
        p
    }

    /// `SignalBoundary.blockEntityRemoved(pos, front)`: when the last block entity goes the boundary
    /// is removed from its graph.
    pub fn signal_block_entity_removed(&mut self, graph: Uuid, id: Uuid, pos: Pos) {
        let gone = match self.signal_mut(graph, id) {
            Some(s) => s.block_entity_removed(pos),
            None => return,
        };
        if gone {
            self.remove_point_from_all_graphs(EdgePointType::Signal, id);
        }
    }

    /// `EdgeData`'s view for tests and the glue: an edge's current effective group.
    pub fn edge_group_at(&self, graph: Uuid, from: NodeId, to: NodeId, position: f64) -> Option<Uuid> {
        let g = self.graphs.get(&graph)?;
        let e = g.edge(from, to)?;
        let data: &EdgeData = &e.edge_data;
        data.group_at_position(graph, &g.edge_points, &e.loc1, &e.loc2, position)
    }
}

#[cfg(test)]
mod tests {
    use super::super::network::tests::{line, loc};
    use super::*;

    fn node(nets: &RailNetworks, g: Uuid, x: f64) -> NodeId {
        nets.graphs[&g].locate_node(&loc(x, 0.0)).unwrap().net_id
    }

    /// Places a signal boundary at `position` from `loc(from)` toward `loc(to)` and pumps the ticks.
    fn place_signal(nets: &mut RailNetworks, g: Uuid, from: f64, to: f64, position: f64, front: bool) -> Uuid {
        // `TrackTargetingBehaviour`: a point on edge (a -> b) at `position` along it, stored as
        // edge location [a, b] when `front`, else swapped with the position from the other end.
        let len = (to - from).abs();
        let (edge, pos) = if front { ([loc(from, 0.0), loc(to, 0.0)], position) } else { ([loc(to, 0.0), loc(from, 0.0)], len - position) };
        let p = nets.new_signal_boundary(edge, pos, [0, 65, 0], front, SignalType::EntrySignal, false);
        let id = p.id;
        nets.add_point(g, p);
        id
    }

    fn settle(nets: &mut RailNetworks) {
        // `GlobalRailwayManager.tick` points half: propagate until nothing is pending.
        for _ in 0..4 {
            nets.tick_all_points(true);
        }
    }

    #[test]
    fn colour_mask_picks_the_first_free_colour() {
        assert_eq!(EdgeGroupColor::find_next_available(0), EdgeGroupColor::Yellow);
        let m = EdgeGroupColor::Yellow.strike_from(0);
        assert_eq!(EdgeGroupColor::find_next_available(m), EdgeGroupColor::Green);
        assert_eq!(EdgeGroupColor::White.strike_from(m), m);
        assert_eq!(EdgeGroupColor::find_next_available(0xFF), EdgeGroupColor::White);
        assert_eq!(EdgeGroupColor::Blue.rgb(), 0x5391E1);
    }

    #[test]
    fn a_signal_splits_the_track_into_two_blocks() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 10.0, 20.0]);
        let (a, b, c) = (node(&nets, g, 0.0), node(&nets, g, 10.0), node(&nets, g, 20.0));
        let before = nets.graphs[&g].edge(a, b).unwrap().edge_data.effective_edge_group_id(g);
        assert_eq!(before, Some(g));

        // A signal 4 along the first edge, facing forward.
        let s = place_signal(&mut nets, g, 0.0, 10.0, 4.0, true);
        assert!(nets.graphs[&g].edge(a, b).unwrap().edge_data.has_signal_boundaries());
        settle(&mut nets);

        let sb = nets.graphs[&g].point(EdgePointType::Signal, s).unwrap().signal().unwrap().clone();
        let (front, back) = (sb.groups[0].unwrap(), sb.groups[1].unwrap());
        assert_ne!(front, back);
        assert!(nets.signal_edge_groups.contains_key(&front) && nets.signal_edge_groups.contains_key(&back));
        // The two blocks are neighbours, and so differently coloured.
        assert!(nets.signal_edge_groups[&front].adjacent.contains(&back));
        assert!(nets.signal_edge_groups[&back].adjacent.contains(&front));
        assert_ne!(nets.signal_edge_groups[&front].color, nets.signal_edge_groups[&back].color);

        // Before the signal (position 2 on a->b) is one block, after it (7) and past the node another.
        let at = |nets: &RailNetworks, f, t, pos| nets.edge_group_at(g, f, t, pos);
        let near = at(&nets, a, b, 2.0);
        let far = at(&nets, a, b, 7.0);
        assert_ne!(near, far);
        assert_eq!(at(&nets, b, c, 3.0), far);
        // Whichever side the signal faces forward is the group of the stretch after it.
        assert!(near == Some(front) || near == Some(back));
        // The stale-block event went out for the trains.
        assert!(nets.events.iter().any(|e| matches!(e, RailEvent::SignalBlocksStale { .. })));
    }

    #[test]
    fn occupancy_turns_the_signal_red() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 10.0, 20.0]);
        let s = place_signal(&mut nets, g, 0.0, 10.0, 4.0, true);
        settle(&mut nets);
        // A signal block entity on the front side makes it show something.
        let sb = nets.graphs[&g].point(EdgePointType::Signal, s).unwrap().signal().unwrap().clone();
        nets.tick_all_points(false);
        let st = nets.graphs[&g].point(EdgePointType::Signal, s).unwrap().signal().unwrap().cached_states;
        // The front side has the block entity; its block is free.
        assert_eq!(st[0], SignalState::Green);
        assert_eq!(st[1], SignalState::Invalid);

        // A train sitting in the block ahead of the signal (the group its front side faces) turns it red.
        let guarded = sb.groups[0].unwrap();
        nets.signal_edge_groups.get_mut(&guarded).unwrap().trains.insert(77);
        nets.tick_all_points(false);
        let st = nets.graphs[&g].point(EdgePointType::Signal, s).unwrap().signal().unwrap().cached_states;
        assert_eq!(st[0], SignalState::Red);
        // A reservation by the signal itself does not.
        nets.signal_edge_groups.get_mut(&guarded).unwrap().trains.clear();
        nets.signal_edge_groups.get_mut(&guarded).unwrap().reserved = Some(s);
        nets.tick_all_points(false);
        assert_eq!(nets.graphs[&g].point(EdgePointType::Signal, s).unwrap().signal().unwrap().cached_states[0], SignalState::Green);
        // Someone else's does.
        nets.signal_edge_groups.get_mut(&guarded).unwrap().reserved = Some(s + 1);
        nets.tick_all_points(false);
        assert_eq!(nets.graphs[&g].point(EdgePointType::Signal, s).unwrap().signal().unwrap().cached_states[0], SignalState::Red);
        // A lever forcing red does too.
        nets.signal_edge_groups.get_mut(&guarded).unwrap().reserved = None;
        nets.graphs.get_mut(&g).unwrap().edge_points.get_mut(EdgePointType::Signal, s).unwrap().signal_mut().unwrap().update_block_entity_power([0, 65, 0], true);
        nets.tick_all_points(false);
        assert_eq!(nets.graphs[&g].point(EdgePointType::Signal, s).unwrap().signal().unwrap().cached_states[0], SignalState::Red);
    }

    #[test]
    fn train_occupancy_rules() {
        let mut g = SignalEdgeGroup::new(1);
        assert!(!g.is_this_occupied_unless_train(5));
        g.trains.insert(5);
        assert!(!g.is_this_occupied_unless_train(5));
        assert!(g.is_this_occupied_unless_train(6));
        g.trains.insert(6);
        assert!(g.is_this_occupied_unless_train(5));
        g.trains.clear();
        g.reserved = Some(9);
        assert!(g.is_this_occupied_unless_train(5));
        assert!(g.is_this_occupied_unless_boundary(8));
        assert!(!g.is_this_occupied_unless_boundary(9));
    }

    #[test]
    fn crossing_groups_share_occupancy_and_colour() {
        let mut nets = RailNetworks::new();
        nets.signal_edge_groups.insert(1, SignalEdgeGroup::new(1));
        nets.signal_edge_groups.insert(2, SignalEdgeGroup::new(2));
        nets.signal_edge_groups.insert(3, SignalEdgeGroup::new(3));
        // 3 is next to 1; 1 and 2 cross.
        nets.signal_edge_groups.get_mut(&1).unwrap().adjacent.insert(3);
        nets.signal_edge_groups.get_mut(&3).unwrap().adjacent.insert(1);
        nets.put_group_intersection(1, 100, 2);
        nets.put_group_intersection(2, 100, 1);
        // Group 1's colour is yellow, as is 3's: they are adjacent, so 1 (and its crossing, 2) move on.
        assert_eq!(nets.signal_edge_groups[&3].color, EdgeGroupColor::Yellow);
        assert_eq!(nets.signal_edge_groups[&1].color, EdgeGroupColor::Green);
        assert_eq!(nets.signal_edge_groups[&2].color, EdgeGroupColor::Green);
        // A train on 2 occupies 1 too, unless it is that train.
        nets.signal_edge_groups.get_mut(&2).unwrap().trains.insert(42);
        assert!(nets.is_group_occupied_unless_train(1, 7));
        assert!(!nets.is_group_occupied_unless_train(1, 42));
        assert!(nets.is_group_occupied_unless_boundary(1, 5));
        // Dropping the crossing frees them.
        nets.remove_group_intersection(1, 100);
        assert!(!nets.is_group_occupied_unless_train(1, 7));
        assert!(nets.signal_edge_groups[&2].intersecting.is_empty());
    }

    #[test]
    fn removing_the_signal_makes_the_edges_passive_again() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 10.0, 20.0]);
        let (a, b) = (node(&nets, g, 0.0), node(&nets, g, 10.0));
        let s = place_signal(&mut nets, g, 0.0, 10.0, 4.0, true);
        settle(&mut nets);
        assert!(nets.graphs[&g].edge(a, b).unwrap().edge_data.has_signal_boundaries());
        let removed = nets.remove_point(g, EdgePointType::Signal, s);
        assert!(removed.is_some());
        let e = nets.graphs[&g].edge(a, b).unwrap();
        assert!(!e.edge_data.has_signal_boundaries());
        assert_eq!(e.edge_data.single_signal_group, Some(PASSIVE_GROUP));
        assert!(!e.edge_data.has_points());
    }

    #[test]
    fn a_cross_signal_reads_the_signals_ahead() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 10.0, 20.0, 30.0]);
        let s1 = place_signal(&mut nets, g, 0.0, 10.0, 4.0, true);
        let s2 = place_signal(&mut nets, g, 20.0, 30.0, 4.0, true);
        settle(&mut nets);
        // The first becomes a cross signal.
        nets.graphs.get_mut(&g).unwrap().edge_points.get_mut(EdgePointType::Signal, s1).unwrap().signal_mut().unwrap().types[0] = SignalType::CrossSignal;
        nets.tick_all_points(false);
        nets.tick_all_points(false);
        let state = |nets: &RailNetworks, id| nets.graphs[&g].point(EdgePointType::Signal, id).unwrap().signal().unwrap().cached_states[0];
        assert_eq!(state(&nets, s1), SignalState::Green);
        // The only signal ahead (s2) guards a block: occupy it and the cross signal waits.
        let guarded = nets.graphs[&g].point(EdgePointType::Signal, s2).unwrap().signal().unwrap().groups[0].unwrap();
        nets.signal_edge_groups.get_mut(&guarded).unwrap().trains.insert(5);
        nets.tick_all_points(false);
        nets.tick_all_points(false);
        assert_eq!(state(&nets, s2), SignalState::Red);
        assert_eq!(state(&nets, s1), SignalState::Red);
    }
}
