//! `TrackGraph` (content/trains/graph/TrackGraph.java) and the track side of `GlobalRailwayManager`
//! (`trackNetworks`, `signalEdgeGroups`, `putGraphWithDefaultGroup`, `removeGraphAndGroup`,
//! `updateSplitGraph`, `getGraph(s)`), plus `EdgePointManager`.
//!
//! A [`TrackGraph`] is the nodes of one connected piece of track, the directed edges between them
//! and the edge points on those. [`RailNetworks`] is every graph of the world plus the signal edge
//! groups; every operation that in Java reaches into other graphs, the trains or the sync layer lives
//! here, because it needs them all:
//!
//! - adding and removing nodes and edges (`createNodeIfAbsent`, `removeNode`, `connectNodes`,
//!   `disconnectNodes`, `putConnection`), with the crossings between edges found when an edge is
//!   connected and cleaned up when a node goes;
//! - joining two graphs (`transferAll`) and cutting one apart (`findDisconnectedGraphs`, `transfer`);
//! - edge points added to and removed from a graph (`addPoint`, `removePoint`, `EdgePointManager`);
//! - `resolveIntersectingEdgeGroups`.
//!
//! What Java does to trains (detach from a removed node, follow their node into another graph, flag
//! their signal blocks stale) and to the network layer (`TrackGraphSync`) is not done here: each is
//! pushed as a [`RailEvent`] onto [`RailNetworks::events`], for the train port and the glue to drain.
//!
//! Known differences: nodes and edges are kept in ordered maps (Java's `HashMap`/`IdentityHashMap`
//! iterate in no defined order), so where Java picks "any" the lowest wins; UUIDs come from a counter.

use super::bezier::BezierConnection;
use super::graph::{
    DiscoveredLocation, EdgeData, EdgePointRef, EdgePointStorage, EdgePointType, NodeId, PASSIVE_GROUP, TrackEdge,
    TrackEdgeIntersection, TrackEdgePoint, TrackNode, TrackNodeLocation, Uuid,
};
use super::signal::SignalEdgeGroup;
use super::vec::{Vec3, distance_to};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// What changed, for the trains and the network layer (`TrackGraphSync`, `Create.RAILWAYS.trains`).
#[derive(Clone, Debug, PartialEq)]
pub enum RailEvent {
    NodeAdded { graph: Uuid, node: NodeId },
    NodeRemoved { graph: Uuid, node: NodeId, location: TrackNodeLocation },
    EdgeAdded { graph: Uuid, from: NodeId, to: NodeId },
    EdgeDataChanged { graph: Uuid, from: NodeId, to: NodeId },
    GraphRemoved { graph: Uuid },
    GraphSplit { from: Uuid, new: Vec<Uuid> },
    /// `TrackGraphMergeEvent(other, graph)`: `from` was folded into `into`.
    GraphMerged { from: Uuid, into: Uuid },
    PointAdded { graph: Uuid, ty: EdgePointType, id: Uuid },
    PointRemoved { graph: Uuid, ty: EdgePointType, id: Uuid },
    /// `point.invalidate(level)`: the block entities at these positions must drop their point.
    PointInvalidated { graph: Uuid, ty: EdgePointType, id: Uuid, block_entities: Vec<crate::Pos> },
    GroupCreated { id: Uuid, color: super::signal::EdgeGroupColor },
    GroupRemoved { id: Uuid },
    /// `train.isTravellingOn(node)` while its node is removed: `train.detachFromTracks()`.
    TrainsOnNodeRemoved { graph: Uuid, node: NodeId },
    /// A node moved to another graph: trains travelling on it take the new graph (`train.graph = target`).
    NodeTransferred { from: Uuid, to: Uuid, node: NodeId },
    /// A whole graph moved into another (`transferAll`).
    GraphTransferred { from: Uuid, to: Uuid },
    /// `SignalPropagator.notifyTrains`: trains travelling on these edges must rebuild their signal blocks.
    SignalBlocksStale { graph: Uuid, edges: Vec<(NodeId, NodeId)> },
}

/// What `removeNode` took out of a graph, for the cross-graph part.
struct RemovedNode {
    removed: TrackNode,
    /// The points that sat on the node's edges, taken out of the graph's storage.
    points: Vec<TrackEdgePoint>,
    /// `(intersection, other node)` of every crossing on the node's edges.
    intersections: Vec<(TrackEdgeIntersection, TrackNode)>,
}

/// `TrackGraph`.
#[derive(Clone, Debug)]
pub struct TrackGraph {
    pub id: Uuid,
    /// `netId`: how the client names the graph.
    pub net_id: i32,
    nodes: BTreeMap<TrackNodeLocation, TrackNode>,
    nodes_by_id: BTreeMap<NodeId, TrackNodeLocation>,
    connections: BTreeMap<NodeId, BTreeMap<NodeId, TrackEdge>>,
    pub edge_points: EdgePointStorage,
    /// `deferredIntersectionUpdates`: `(from, to, serial)`, the serial standing for object identity.
    deferred_intersection_updates: Vec<(NodeId, NodeId, u64)>,
    checksum: i32,
}

impl TrackGraph {
    pub fn new(id: Uuid, net_id: i32) -> TrackGraph {
        TrackGraph {
            id,
            net_id,
            nodes: BTreeMap::new(),
            nodes_by_id: BTreeMap::new(),
            connections: BTreeMap::new(),
            edge_points: EdgePointStorage::default(),
            deferred_intersection_updates: Vec::new(),
            checksum: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// `getNodes()`: the node locations.
    pub fn node_locations(&self) -> impl Iterator<Item = &TrackNodeLocation> {
        self.nodes.keys()
    }

    pub fn nodes(&self) -> impl Iterator<Item = &TrackNode> {
        self.nodes.values()
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// `locateNode(location)`.
    pub fn locate_node(&self, loc: &TrackNodeLocation) -> Option<&TrackNode> {
        self.nodes.get(loc)
    }

    /// `getNode(netId)`.
    pub fn get_node(&self, id: NodeId) -> Option<&TrackNode> {
        self.nodes.get(self.nodes_by_id.get(&id)?)
    }

    /// `getConnectionsFrom(node)`: the edges leaving a node, by the node they lead to.
    pub fn connections_from(&self, node: NodeId) -> Option<&BTreeMap<NodeId, TrackEdge>> {
        self.connections.get(&node)
    }

    /// `getConnection(nodes)`.
    pub fn edge(&self, from: NodeId, to: NodeId) -> Option<&TrackEdge> {
        self.connections.get(&from)?.get(&to)
    }

    pub fn edge_mut(&mut self, from: NodeId, to: NodeId) -> Option<&mut TrackEdge> {
        self.connections.get_mut(&from)?.get_mut(&to)
    }

    /// Every directed edge as `(from, to)`.
    pub fn edge_keys(&self) -> Vec<(NodeId, NodeId)> {
        self.connections.iter().flat_map(|(a, m)| m.keys().map(move |b| (*a, *b))).collect()
    }

    /// `getChecksum()`: the sum of the node ids, cached (0 means "not computed").
    pub fn checksum(&mut self) -> i32 {
        if self.checksum == 0 {
            self.checksum = self.nodes.values().fold(0_i32, |a, n| a.wrapping_add(n.net_id));
        }
        self.checksum
    }

    pub fn invalidate_bounds(&mut self) {
        self.checksum = 0;
    }

    /// `addNode(node)`: replaces a node already at the location.
    pub fn add_node(&mut self, node: TrackNode) {
        if self.nodes.contains_key(&node.location) {
            self.remove_node_local(&node.location);
        }
        self.nodes_by_id.insert(node.net_id, node.location);
        self.nodes.insert(node.location, node);
    }

    /// `addNodeIfAbsent(node)`.
    pub fn add_node_if_absent(&mut self, node: TrackNode) -> bool {
        if self.nodes.contains_key(&node.location) {
            return false;
        }
        self.nodes_by_id.insert(node.net_id, node.location);
        self.nodes.insert(node.location, node);
        true
    }

    /// `loadNode(location, netId, normal)`.
    pub fn load_node(&mut self, location: TrackNodeLocation, net_id: NodeId, normal: Vec3) {
        self.add_node(TrackNode { location, net_id, normal });
    }

    /// `disconnectNodes(node1, node2)`: drops the edge both ways.
    pub fn disconnect_nodes(&mut self, node1: NodeId, node2: NodeId) {
        if let Some(m) = self.connections.get_mut(&node1) {
            m.remove(&node2);
        }
        if let Some(m) = self.connections.get_mut(&node2) {
            m.remove(&node1);
        }
    }

    /// `putConnection(node1, node2, edge)`: an edge that already carries points is kept; true when
    /// there was no edge before.
    pub fn put_connection(&mut self, node1: NodeId, node2: NodeId, edge: TrackEdge) -> bool {
        let conns = self.connections.entry(node1).or_default();
        if conns.get(&node2).is_some_and(|e| e.edge_data.has_points()) {
            return false;
        }
        conns.insert(node2, edge).is_none()
    }

    /// `distanceToLocationSqr(level, location)`.
    pub fn distance_to_location_sqr(&self, dimension: u32, location: Vec3) -> f32 {
        let mut nearest = f32::MAX;
        for tnl in self.nodes.keys() {
            if tnl.dimension != dimension {
                continue;
            }
            let d = distance_to(tnl.location(), location);
            nearest = nearest.min((d * d) as f32);
        }
        nearest
    }

    /// `getPoint(type, id)`.
    pub fn point(&self, ty: EdgePointType, id: Uuid) -> Option<&TrackEdgePoint> {
        self.edge_points.get(ty, id)
    }

    /// `getPoints(type)`.
    pub fn points(&self, ty: EdgePointType) -> Vec<&TrackEdgePoint> {
        self.edge_points.values(ty)
    }

    /// `deferIntersectionUpdate(edge)`.
    fn defer_intersection_update(&mut self, from: NodeId, to: NodeId, serial: u64) {
        self.deferred_intersection_updates.push((from, to, serial));
    }

    /// The local half of `removeNode`: the node, its edges, the points on them. The caller
    /// (`RailNetworks::remove_node`) does what needs other graphs.
    fn remove_node_local(&mut self, location: &TrackNodeLocation) -> Option<RemovedNode> {
        let removed = self.nodes.remove(location)?;
        self.nodes_by_id.remove(&removed.net_id);
        self.invalidate_bounds();

        let mut out = RemovedNode { removed: removed.clone(), points: Vec::new(), intersections: Vec::new() };
        let Some(connections) = self.connections.remove(&removed.net_id) else {
            return Some(out);
        };
        for (other, edge) in &connections {
            for r in &edge.edge_data.points {
                if let Some(p) = self.edge_points.remove(r.ty, r.id) {
                    out.points.push(p);
                }
            }
            if let Some(other_node) = self.get_node(*other).cloned() {
                for i in &edge.edge_data.intersections {
                    out.intersections.push((i.clone(), other_node.clone()));
                }
            }
        }
        for other in connections.keys() {
            if let Some(m) = self.connections.get_mut(other) {
                m.remove(&removed.net_id);
            }
        }
        Some(out)
    }

    /// `transfer(level, node, target)`: moves a node, the edges leaving it and the points on them.
    /// Nodes are located by id, so edges leaving a node stay with the node.
    fn transfer_node(&mut self, node: NodeId, target: &mut TrackGraph) {
        let Some(loc) = self.nodes_by_id.get(&node).copied() else { return };
        let Some(n) = self.nodes.get(&loc).cloned() else { return };
        target.add_node(n);
        target.invalidate_bounds();

        if let Some(conns) = self.connections.get(&node) {
            if !conns.is_empty() {
                for e in conns.values() {
                    for r in &e.edge_data.points {
                        if let Some(p) = self.edge_points.remove(r.ty, r.id) {
                            target.edge_points.put(p);
                        }
                    }
                }
                let conns = self.connections.remove(&node).unwrap_or_default();
                target.connections.insert(node, conns);
            }
        }

        self.nodes.remove(&loc);
        self.nodes_by_id.remove(&node);
        self.connections.remove(&node);
        self.invalidate_bounds();
    }
}

/// `GlobalRailwayManager`'s track graphs and signal edge groups.
#[derive(Clone, Debug)]
pub struct RailNetworks {
    pub graphs: BTreeMap<Uuid, TrackGraph>,
    pub signal_edge_groups: BTreeMap<Uuid, SignalEdgeGroup>,
    pub events: Vec<RailEvent>,
    /// `markTracksDirty()`: the saved data needs writing.
    pub dirty: bool,
    node_net_id: i32,
    graph_net_id: i32,
    edge_serial: u64,
    uuid_counter: u128,
}

impl Default for RailNetworks {
    fn default() -> Self {
        Self::new()
    }
}

impl RailNetworks {
    pub fn new() -> RailNetworks {
        RailNetworks {
            graphs: BTreeMap::new(),
            signal_edge_groups: BTreeMap::new(),
            events: Vec::new(),
            dirty: false,
            node_net_id: 0,
            graph_net_id: 0,
            edge_serial: 0,
            uuid_counter: 0,
        }
    }

    /// A fresh UUID (never the passive group's zero).
    pub fn new_uuid(&mut self) -> Uuid {
        self.uuid_counter += 1;
        self.uuid_counter
    }

    /// `nextNodeId()`.
    pub fn next_node_id(&mut self) -> NodeId {
        self.node_net_id += 1;
        self.node_net_id
    }

    /// `nextGraphId()`.
    pub fn next_graph_id(&mut self) -> i32 {
        self.graph_net_id += 1;
        self.graph_net_id
    }

    pub(super) fn next_edge_serial(&mut self) -> u64 {
        self.edge_serial += 1;
        self.edge_serial
    }

    pub fn mark_tracks_dirty(&mut self) {
        self.dirty = true;
    }

    /// `new TrackGraph()`: not yet registered.
    pub fn new_graph(&mut self) -> TrackGraph {
        let id = self.new_uuid();
        let net = self.next_graph_id();
        TrackGraph::new(id, net)
    }

    /// `putGraphWithDefaultGroup(graph)`: the graph and the fallback signal edge group named by its id.
    pub fn put_graph_with_default_group(&mut self, graph: TrackGraph) {
        let group = SignalEdgeGroup::new(graph.id).as_fallback();
        self.events.push(RailEvent::GroupCreated { id: graph.id, color: group.color });
        self.signal_edge_groups.insert(graph.id, group);
        self.put_graph(graph);
    }

    /// `putGraph(graph)`.
    pub fn put_graph(&mut self, graph: TrackGraph) {
        self.graphs.insert(graph.id, graph);
        self.mark_tracks_dirty();
    }

    /// `removeGraphAndGroup(graph)`.
    pub fn remove_graph_and_group(&mut self, id: Uuid) {
        self.signal_edge_groups.remove(&id);
        self.events.push(RailEvent::GroupRemoved { id });
        self.remove_graph(id);
    }

    /// `removeGraph(graph)`.
    pub fn remove_graph(&mut self, id: Uuid) {
        self.graphs.remove(&id);
        self.mark_tracks_dirty();
    }

    /// `getGraph(level, vertex)`: the first graph with a node there.
    pub fn get_graph(&self, vertex: &TrackNodeLocation) -> Option<Uuid> {
        self.graphs.iter().find(|(_, g)| g.locate_node(vertex).is_some()).map(|(id, _)| *id)
    }

    /// `getGraphs(level, vertex)`: every graph with a node there.
    pub fn get_graphs(&self, vertex: &TrackNodeLocation) -> Vec<Uuid> {
        self.graphs.iter().filter(|(_, g)| g.locate_node(vertex).is_some()).map(|(id, _)| *id).collect()
    }

    // --- nodes ---------------------------------------------------------------------------

    /// `createNodeIfAbsent(location)`: adds a node with a fresh id; false if one is already there.
    pub fn create_node_if_absent(&mut self, graph: Uuid, location: &DiscoveredLocation) -> bool {
        let net_id = self.next_node_id();
        let Some(g) = self.graphs.get_mut(&graph) else { return false };
        let normal = location.normal.unwrap_or([0.0, 1.0, 0.0]);
        if !g.add_node_if_absent(TrackNode { location: location.loc, net_id, normal }) {
            // The id was not used; Java's counter has moved on, ours stays gapless.
            self.node_net_id -= 1;
            return false;
        }
        g.invalidate_bounds();
        self.events.push(RailEvent::NodeAdded { graph, node: net_id });
        self.mark_tracks_dirty();
        true
    }

    /// `removeNode(level, location)`. With `with_level` the points on its edges are invalidated and
    /// the crossings they had in other graphs are removed; without, they are just dropped.
    pub fn remove_node(&mut self, graph: Uuid, location: &TrackNodeLocation, with_level: bool) -> bool {
        let Some(g) = self.graphs.get_mut(&graph) else { return false };
        let Some(r) = g.remove_node_local(location) else { return false };
        self.events.push(RailEvent::TrainsOnNodeRemoved { graph, node: r.removed.net_id });

        if with_level {
            for p in &r.points {
                self.invalidate_point(graph, p);
            }
            for (intersection, other_node) in &r.intersections {
                let target = intersection.target;
                if let Some(other_graph) = self.get_graph(&target[0]) {
                    self.remove_intersection(other_graph, intersection, &r.removed, other_node);
                }
            }
        }
        true
    }

    /// `removeIntersection(intersection, targetNode1, targetNode2)` on the graph holding the target edge.
    fn remove_intersection(
        &mut self,
        graph: Uuid,
        intersection: &TrackEdgeIntersection,
        _target_node1: &TrackNode,
        _target_node2: &TrackNode,
    ) {
        let (n1, n2) = {
            let Some(g) = self.graphs.get(&graph) else { return };
            let (Some(a), Some(b)) = (g.locate_node(&intersection.target[0]), g.locate_node(&intersection.target[1]))
            else {
                return;
            };
            (a.net_id, b.net_id)
        };
        for (from, to) in [(n1, n2), (n2, n1)] {
            if self.graphs.get(&graph).and_then(|g| g.edge(from, to)).is_some() {
                self.edge_remove_intersection(graph, from, to, intersection.id);
            }
        }
    }

    // --- edges ---------------------------------------------------------------------------

    /// `connectNodes(reader, location, location2, turn)`: puts the edge both ways between two nodes
    /// of the graph, after finding where it crosses every curve already in any graph.
    pub fn connect_nodes(
        &mut self,
        graph: Uuid,
        location: &DiscoveredLocation,
        location2: &DiscoveredLocation,
        turn: Option<BezierConnection>,
    ) {
        let (node1, node2) = {
            let Some(g) = self.graphs.get(&graph) else { return };
            let (Some(a), Some(b)) = (g.locate_node(&location.loc), g.locate_node(&location2.loc)) else { return };
            (a.clone(), b.clone())
        };

        let bezier = turn.is_some();
        let material = match &turn {
            Some(t) => Some(t.material.clone()),
            None => location2.material_a.clone(),
        };
        let serial1 = self.next_edge_serial();
        let serial2 = self.next_edge_serial();
        let mut edge = TrackEdge::new(&node1, &node2, turn.clone(), material.clone(), serial1);
        let mut edge2 = TrackEdge::new(&node2, &node1, turn.as_ref().map(|t| t.secondary()), material, serial2);

        // Find the crossings first (read-only), then apply them.
        struct Crossing {
            graph: Uuid,
            other: (NodeId, NodeId),
            other_reverse: Option<(NodeId, NodeId)>,
            id: Uuid,
            s: f64,
            t: f64,
            other_length: f64,
            other_node1: TrackNodeLocation,
            other_node2: TrackNodeLocation,
        }
        let mut found: Vec<Crossing> = Vec::new();
        let graph_ids: Vec<Uuid> = self.graphs.keys().copied().collect();
        for gid in graph_ids {
            let mut work: Vec<(NodeId, NodeId, Vec<[f64; 2]>, f64, TrackNodeLocation, TrackNodeLocation, bool)> = Vec::new();
            {
                let g = &self.graphs[&gid];
                for (other1, conns) in &g.connections {
                    let Some(other_loc1) = g.get_node(*other1).map(|n| n.location) else { continue };
                    for (other2, other_edge) in conns {
                        if gid == graph && (*other1 == node1.net_id || *other2 == node1.net_id || *other1 == node2.net_id || *other2 == node2.net_id) {
                            continue;
                        }
                        if other_edge.is_inter_dimensional() || edge.is_inter_dimensional() {
                            continue;
                        }
                        if node1.location.dimension != other_loc1.dimension {
                            continue;
                        }
                        if !bezier && !other_edge.is_turn() {
                            continue;
                        }
                        if other_edge.turn.as_ref().is_some_and(|t| t.is_primary()) {
                            continue;
                        }
                        let hits = edge.intersections_with(other_edge);
                        let reverse = g.edge(*other2, *other1).map(|_| ());
                        work.push((
                            *other1,
                            *other2,
                            hits,
                            other_edge.length(),
                            other_edge.loc1,
                            other_edge.loc2,
                            reverse.is_some(),
                        ));
                    }
                }
            }
            for (o1, o2, hits, other_length, l1, l2, has_reverse) in work {
                let id = self.new_uuid();
                for [s, t] in hits {
                    found.push(Crossing {
                        graph: gid,
                        other: (o1, o2),
                        other_reverse: has_reverse.then_some((o2, o1)),
                        id,
                        s,
                        t,
                        other_length,
                        other_node1: l1,
                        other_node2: l2,
                    });
                }
            }
        }

        let mut deferred_here: Vec<(NodeId, NodeId, u64)> = Vec::new();
        for c in &found {
            // The new edge and its reverse.
            let target = [c.other_node1, c.other_node2];
            let len = edge.length();
            if add_intersection(&mut edge.edge_data, c.id, c.s, target, c.t) {
                deferred_here.push((edge.node1, edge.node2, edge.serial));
            }
            if add_intersection(&mut edge2.edge_data, c.id, len - c.s, target, c.t) {
                deferred_here.push((edge2.node1, edge2.node2, edge2.serial));
            }
            // The edge it crosses, and that edge's reverse.
            let mine = [node1.location, node2.location];
            if let Some(g) = self.graphs.get_mut(&c.graph) {
                if let Some(oe) = g.edge_mut(c.other.0, c.other.1) {
                    if add_intersection(&mut oe.edge_data, c.id, c.t, mine, c.s) {
                        let k = (oe.node1, oe.node2, oe.serial);
                        g.defer_intersection_update(k.0, k.1, k.2);
                    }
                }
                if let Some((a, b)) = c.other_reverse {
                    if let Some(oe) = g.edge_mut(a, b) {
                        if add_intersection(&mut oe.edge_data, c.id, c.other_length - c.t, mine, c.s) {
                            let k = (oe.node1, oe.node2, oe.serial);
                            g.defer_intersection_update(k.0, k.1, k.2);
                        }
                    }
                }
            }
        }

        let g = self.graphs.get_mut(&graph).expect("graph checked above");
        for (a, b, s) in deferred_here {
            g.defer_intersection_update(a, b, s);
        }
        g.put_connection(node1.net_id, node2.net_id, edge);
        g.put_connection(node2.net_id, node1.net_id, edge2);
        self.events.push(RailEvent::EdgeAdded { graph, from: node1.net_id, to: node2.net_id });
        self.events.push(RailEvent::EdgeAdded { graph, from: node2.net_id, to: node1.net_id });
        self.mark_tracks_dirty();
    }

    /// `resolveIntersectingEdgeGroups(level)`: ties the signal groups of crossing edges together.
    pub fn resolve_intersecting_edge_groups(&mut self, graph_id: Uuid) {
        let deferred = match self.graphs.get_mut(&graph_id) {
            Some(g) => std::mem::take(&mut g.deferred_intersection_updates),
            None => return,
        };
        for (from, to, serial) in deferred {
            let (edge_data, loc1, loc2) = {
                let g = &self.graphs[&graph_id];
                let Some(e) = g.edge(from, to) else { continue };
                if e.serial != serial {
                    continue;
                }
                (e.edge_data.clone(), e.loc1, e.loc2)
            };
            for intersection in &edge_data.intersections {
                let group_id = {
                    let g = &self.graphs[&graph_id];
                    edge_data.group_at_position(graph_id, &g.edge_points, &loc1, &loc2, intersection.location)
                };
                let target = intersection.target;
                let Some(other_graph) = self.get_graph(&target[0]) else { continue };
                let other_group_id = {
                    let og = &self.graphs[&other_graph];
                    let (Some(n1), Some(n2)) = (og.locate_node(&target[0]), og.locate_node(&target[1])) else {
                        continue;
                    };
                    let Some(oe) = og.edge(n1.net_id, n2.net_id) else { continue };
                    oe.edge_data.group_at_position(other_graph, &og.edge_points, &oe.loc1, &oe.loc2, intersection.target_location)
                };
                let (Some(gid), Some(ogid)) = (group_id, other_group_id) else { continue };
                if !self.signal_edge_groups.contains_key(&gid) || !self.signal_edge_groups.contains_key(&ogid) {
                    continue;
                }
                if let Some(e) = self.graphs.get_mut(&graph_id).and_then(|g| g.edge_mut(from, to)) {
                    if let Some(i) = e.edge_data.intersections.iter_mut().find(|i| i.id == intersection.id) {
                        i.group_id = Some(gid);
                    }
                }
                self.put_group_intersection(gid, intersection.id, ogid);
                self.put_group_intersection(ogid, intersection.id, gid);
            }
        }
    }

    // --- EdgeData operations that need the graph or the groups --------------------------------

    /// `EdgeData.setSingleSignalGroup(graph, group)`.
    pub fn edge_set_single_signal_group(&mut self, graph: Uuid, from: NodeId, to: NodeId, group: Option<Uuid>) {
        let changed = match self.graphs.get(&graph).and_then(|g| g.edge(from, to)) {
            Some(e) => e.edge_data.single_signal_group != group,
            None => return,
        };
        if changed {
            self.edge_refresh_intersecting_signal_groups(graph, from, to);
        }
        if let Some(e) = self.graphs.get_mut(&graph).and_then(|g| g.edge_mut(from, to)) {
            e.edge_data.single_signal_group = group;
        }
    }

    /// `EdgeData.refreshIntersectingSignalGroups(graph)`.
    pub fn edge_refresh_intersecting_signal_groups(&mut self, graph: Uuid, from: NodeId, to: NodeId) {
        let (intersections, serial, has) = match self.graphs.get(&graph).and_then(|g| g.edge(from, to)) {
            Some(e) => (e.edge_data.intersections.clone(), e.serial, e.edge_data.has_intersections()),
            None => return,
        };
        for i in &intersections {
            let Some(gid) = i.group_id else { continue };
            if self.signal_edge_groups.contains_key(&gid) {
                self.remove_group_intersection(gid, i.id);
            }
        }
        if has {
            if let Some(g) = self.graphs.get_mut(&graph) {
                g.defer_intersection_update(from, to, serial);
            }
        }
    }

    /// `EdgeData.removeIntersection(graph, id)`.
    pub fn edge_remove_intersection(&mut self, graph: Uuid, from: NodeId, to: NodeId, id: Uuid) {
        self.edge_refresh_intersecting_signal_groups(graph, from, to);
        if let Some(e) = self.graphs.get_mut(&graph).and_then(|g| g.edge_mut(from, to)) {
            e.edge_data.intersections.retain(|i| i.id != id);
        }
    }

    /// `EdgeData.addPoint(graph, point)`: a signal puts the edge in "has boundaries" mode.
    fn edge_add_point(&mut self, graph: Uuid, from: NodeId, to: NodeId, point: &TrackEdgePoint) {
        if point.ty == EdgePointType::Signal {
            self.edge_set_single_signal_group(graph, from, to, None);
        }
        if let Some(e) = self.graphs.get_mut(&graph).and_then(|g| g.edge_mut(from, to)) {
            let location_on = point.location_on(e);
            e.edge_data.insert_point(EdgePointRef { ty: point.ty, id: point.id, location_on });
        }
    }

    /// `EdgeData.removePoint(graph, point)`: with its last signal gone the edge is passive again.
    fn edge_remove_point(&mut self, graph: Uuid, from: NodeId, to: NodeId, ty: EdgePointType, id: Uuid) {
        let Some(e) = self.graphs.get_mut(&graph).and_then(|g| g.edge_mut(from, to)) else { return };
        e.edge_data.points.retain(|p| !(p.ty == ty && p.id == id));
        let no_signals_remaining = e.edge_data.next_of(EdgePointType::Signal, 0.0).is_none();
        if ty == EdgePointType::Signal {
            self.edge_set_single_signal_group(graph, from, to, if no_signals_remaining { Some(PASSIVE_GROUP) } else { None });
        }
    }

    // --- edge points -----------------------------------------------------------------------

    /// `addPoint(type, point)` with `EdgePointManager.onEdgePointAdded`: stores the point and lists it
    /// on both directions of its edge.
    pub fn add_point(&mut self, graph: Uuid, point: TrackEdgePoint) {
        let Some(g) = self.graphs.get_mut(&graph) else { return };
        let start_nodes: Vec<Option<NodeId>> =
            point.edge_location.iter().map(|l| g.locate_node(l).map(|n| n.net_id)).collect();
        g.edge_points.put(point.clone());
        if let [Some(a), Some(b)] = start_nodes[..] {
            for (from, to) in [(a, b), (b, a)] {
                self.edge_add_point(graph, from, to, &point);
                self.events.push(RailEvent::EdgeDataChanged { graph, from, to });
            }
        }
        self.events.push(RailEvent::PointAdded { graph, ty: point.ty, id: point.id });
        self.mark_tracks_dirty();
    }

    /// `removePoint(type, id)` with `EdgePointManager.onEdgePointRemoved`.
    pub fn remove_point(&mut self, graph: Uuid, ty: EdgePointType, id: Uuid) -> Option<TrackEdgePoint> {
        let removed = self.graphs.get_mut(&graph)?.edge_points.remove(ty, id)?;
        self.point_removed(graph, &removed);
        self.events.push(RailEvent::PointRemoved { graph, ty, id });
        self.mark_tracks_dirty();
        Some(removed)
    }

    fn point_removed(&mut self, graph: Uuid, point: &TrackEdgePoint) {
        // `point.onRemoved(graph)`.
        if point.ty == EdgePointType::Signal {
            self.on_signal_removed(graph, point);
        }
        let ids: Vec<Option<NodeId>> = {
            let Some(g) = self.graphs.get(&graph) else { return };
            point.edge_location.iter().map(|l| g.locate_node(l).map(|n| n.net_id)).collect()
        };
        if let [Some(a), Some(b)] = ids[..] {
            for (from, to) in [(a, b), (b, a)] {
                if self.graphs.get(&graph).and_then(|g| g.edge(from, to)).is_none() {
                    continue;
                }
                self.edge_remove_point(graph, from, to, point.ty, point.id);
                self.events.push(RailEvent::EdgeDataChanged { graph, from, to });
            }
        }
    }

    /// `TrackEdgePoint.removeFromAllGraphs`: the first graph that has it drops it.
    pub fn remove_point_from_all_graphs(&mut self, ty: EdgePointType, id: Uuid) {
        let ids: Vec<Uuid> = self.graphs.keys().copied().collect();
        for g in ids {
            if self.remove_point(g, ty, id).is_some() {
                return;
            }
        }
    }

    /// `point.invalidate(level)`.
    fn invalidate_point(&mut self, graph: Uuid, p: &TrackEdgePoint) {
        match p.signal() {
            Some(s) => {
                let mut block_entities: Vec<crate::Pos> = Vec::new();
                for side in &s.block_entities {
                    block_entities.extend(side.keys().copied());
                }
                self.events.push(RailEvent::PointInvalidated { graph, ty: p.ty, id: p.id, block_entities });
                for group in s.groups.iter().flatten() {
                    if self.signal_edge_groups.remove(group).is_some() {
                        self.events.push(RailEvent::GroupRemoved { id: *group });
                    }
                }
            }
            None => self.events.push(RailEvent::PointInvalidated {
                graph,
                ty: p.ty,
                id: p.id,
                block_entities: Vec::new(),
            }),
        }
    }

    // --- joining and splitting graphs -------------------------------------------------------

    /// `transferAll(toOther)`: every node, edge and point of `from` moves into `to`; `from` is left empty.
    pub fn transfer_all(&mut self, from: Uuid, to: Uuid) {
        let Some(mut source) = self.graphs.remove(&from) else { return };
        let Some(target) = self.graphs.get_mut(&to) else {
            self.graphs.insert(from, source);
            return;
        };
        let mut added: Vec<NodeId> = Vec::new();
        for node in source.nodes.values() {
            if target.add_node_if_absent(node.clone()) {
                added.push(node.net_id);
            }
        }
        let mut new_edges: Vec<(NodeId, NodeId)> = Vec::new();
        for (n1, map) in &source.connections {
            for (n2, edge) in map {
                let (Some(a), Some(b)) = (
                    target.locate_node(&source.nodes_by_id.get(n1).copied().unwrap_or_else(|| edge.loc1)).map(|n| n.net_id),
                    target.locate_node(&source.nodes_by_id.get(n2).copied().unwrap_or_else(|| edge.loc2)).map(|n| n.net_id),
                ) else {
                    continue;
                };
                if target.put_connection(a, b, edge.clone()) {
                    new_edges.push((a, b));
                }
            }
        }
        source.edge_points.transfer_all(&mut target.edge_points);
        source.nodes.clear();
        source.nodes_by_id.clear();
        source.connections.clear();
        target.invalidate_bounds();
        for node in added {
            self.events.push(RailEvent::NodeAdded { graph: to, node });
        }
        for (a, b) in new_edges {
            self.events.push(RailEvent::EdgeAdded { graph: to, from: a, to: b });
            self.events.push(RailEvent::EdgeDataChanged { graph: to, from: a, to: b });
        }
        self.events.push(RailEvent::GraphTransferred { from, to });
        self.graphs.insert(from, source);
    }

    /// `findDisconnectedGraphs(level, splitSubGraphs)`: cuts every connected piece but the first
    /// out of the graph into a new one, and returns the new graphs (not yet registered).
    /// `split_sub_graphs` maps a node id to the `(netId, id)` its new graph should take (the client
    /// replaying a server split).
    pub fn find_disconnected_graphs(
        &mut self,
        graph_id: Uuid,
        split_sub_graphs: Option<&BTreeMap<NodeId, (i32, Uuid)>>,
    ) -> Vec<TrackGraph> {
        let Some(mut graph) = self.graphs.remove(&graph_id) else { return Vec::new() };
        let mut discovered: Vec<TrackGraph> = Vec::new();
        let mut vertices: BTreeSet<TrackNodeLocation> = graph.nodes.keys().copied().collect();
        let mut first = true;

        while let Some(start) = vertices.iter().next().copied() {
            let mut target: Option<TrackGraph> = if first { None } else { Some(self.new_graph()) };
            first = false;
            let mut frontier: VecDeque<TrackNodeLocation> = VecDeque::new();
            frontier.push_back(start);
            vertices.remove(&start);

            while let Some(current) = frontier.pop_front() {
                let Some(current_node) = graph.locate_node(&current).map(|n| n.net_id) else { continue };
                let connected: Vec<NodeId> =
                    graph.connections_from(current_node).map(|m| m.keys().copied().collect()).unwrap_or_default();
                for c in connected {
                    if let Some(loc) = graph.get_node(c).map(|n| n.location) {
                        if vertices.remove(&loc) {
                            frontier.push_back(loc);
                        }
                    }
                }
                if let Some(t) = target.as_mut() {
                    if let Some(ids) = split_sub_graphs.and_then(|m| m.get(&current_node)) {
                        t.net_id = ids.0;
                        t.id = ids.1;
                    }
                    graph.transfer_node(current_node, t);
                    self.events.push(RailEvent::NodeTransferred { from: graph_id, to: t.id, node: current_node });
                }
            }
            if let Some(t) = target {
                discovered.push(t);
            }
        }
        self.graphs.insert(graph_id, graph);
        discovered
    }

    /// `updateSplitGraph(level, graph)`: registers the pieces a graph fell apart into.
    pub fn update_split_graph(&mut self, graph_id: Uuid) {
        let disconnected = self.find_disconnected_graphs(graph_id, None);
        let ids: Vec<Uuid> = disconnected.iter().map(|g| g.id).collect();
        for d in disconnected {
            self.put_graph_with_default_group(d);
        }
        if !ids.is_empty() {
            self.events.push(RailEvent::GraphSplit { from: graph_id, new: ids });
            self.mark_tracks_dirty();
        }
    }

    /// Takes the pending events.
    pub fn drain_events(&mut self) -> Vec<RailEvent> {
        std::mem::take(&mut self.events)
    }
}

/// `EdgeData.addIntersection`: ignored when one near the same place on the same target is there;
/// true when added (the caller defers the group update).
fn add_intersection(data: &mut EdgeData, id: Uuid, position: f64, target: [TrackNodeLocation; 2], target_position: f64) -> bool {
    for existing in &data.intersections {
        if existing.is_near(position) && existing.targets(&target[0], &target[1]) {
            return false;
        }
    }
    data.intersections.push(TrackEdgeIntersection {
        id,
        location: position,
        target,
        target_location: target_position,
        group_id: None,
    });
    true
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    pub fn loc(x: f64, z: f64) -> TrackNodeLocation {
        TrackNodeLocation::new([x, 64.0, z], 0)
    }

    pub fn disc(x: f64, z: f64) -> DiscoveredLocation {
        DiscoveredLocation::new(0, [x, 64.0, z]).with_normal([0.0, 1.0, 0.0])
    }

    /// A graph of nodes in a row along x, `step` apart, joined in a chain.
    pub fn line(nets: &mut RailNetworks, xs: &[f64]) -> Uuid {
        let g = nets.new_graph();
        let id = g.id;
        nets.put_graph_with_default_group(g);
        for x in xs {
            assert!(nets.create_node_if_absent(id, &disc(*x, 0.0)));
        }
        for w in xs.windows(2) {
            nets.connect_nodes(id, &disc(w[0], 0.0), &disc(w[1], 0.0), None);
        }
        id
    }

    #[test]
    fn nodes_and_edges_are_added_both_ways() {
        let mut nets = RailNetworks::new();
        let id = line(&mut nets, &[0.0, 5.0, 10.0]);
        let g = &nets.graphs[&id];
        assert_eq!(g.node_count(), 3);
        let a = g.locate_node(&loc(0.0, 0.0)).unwrap().net_id;
        let b = g.locate_node(&loc(5.0, 0.0)).unwrap().net_id;
        assert_eq!(g.edge(a, b).unwrap().length(), 5.0);
        assert_eq!(g.edge(b, a).unwrap().length(), 5.0);
        assert!(g.edge(a, g.locate_node(&loc(10.0, 0.0)).unwrap().net_id).is_none());
        assert_eq!(g.edge_keys().len(), 4);
        // Adding the same node again does nothing.
        assert!(!nets.create_node_if_absent(id, &disc(5.0, 0.0)));
        assert_eq!(nets.graphs[&id].node_count(), 3);
        // The fallback group exists under the graph's own id.
        assert!(nets.signal_edge_groups[&id].fallback_group);
        assert_eq!(nets.get_graph(&loc(5.0, 0.0)), Some(id));
        assert!(nets.get_graph(&loc(99.0, 0.0)).is_none());
        assert!(nets.dirty);
    }

    #[test]
    fn removing_a_node_drops_its_edges_and_a_split_makes_two_graphs() {
        let mut nets = RailNetworks::new();
        let id = line(&mut nets, &[0.0, 5.0, 10.0, 15.0, 20.0]);
        assert!(nets.remove_node(id, &loc(10.0, 0.0), true));
        assert!(!nets.remove_node(id, &loc(10.0, 0.0), true));
        assert_eq!(nets.graphs[&id].node_count(), 4);
        let removed_event = nets.events.iter().any(|e| matches!(e, RailEvent::TrainsOnNodeRemoved { .. }));
        assert!(removed_event);

        nets.update_split_graph(id);
        assert_eq!(nets.graphs.len(), 2);
        let sizes: Vec<usize> = nets.graphs.values().map(|g| g.node_count()).collect();
        assert_eq!(sizes.iter().sum::<usize>(), 4);
        assert!(sizes.iter().all(|s| *s == 2));
        // The piece cut off has its own fallback group.
        assert_eq!(nets.signal_edge_groups.len(), 2);
        let split = nets.events.iter().find_map(|e| match e {
            RailEvent::GraphSplit { from, new } => Some((*from, new.clone())),
            _ => None,
        });
        let (from, new) = split.expect("a split event");
        assert_eq!(from, id);
        assert_eq!(new.len(), 1);
        // Edges followed the nodes.
        for g in nets.graphs.values() {
            assert_eq!(g.edge_keys().len(), 2);
        }
    }

    #[test]
    fn transfer_all_joins_two_graphs() {
        let mut nets = RailNetworks::new();
        let a = line(&mut nets, &[0.0, 5.0]);
        let b = {
            let g = nets.new_graph();
            let id = g.id;
            nets.put_graph_with_default_group(g);
            for x in [20.0, 25.0] {
                nets.create_node_if_absent(id, &disc(x, 0.0));
            }
            nets.connect_nodes(id, &disc(20.0, 0.0), &disc(25.0, 0.0), None);
            id
        };
        nets.transfer_all(b, a);
        assert_eq!(nets.graphs[&a].node_count(), 4);
        assert_eq!(nets.graphs[&a].edge_keys().len(), 4);
        assert!(nets.graphs[&b].is_empty());
        nets.remove_graph_and_group(b);
        assert_eq!(nets.graphs.len(), 1);
        assert!(!nets.signal_edge_groups.contains_key(&b));
    }

    #[test]
    fn a_new_curve_registers_its_crossing_with_the_straight_edge() {
        let mut nets = RailNetworks::new();
        let g = nets.new_graph();
        let gid = g.id;
        nets.put_graph_with_default_group(g);
        // A straight edge from (0,0) to (10,0).
        nets.create_node_if_absent(gid, &disc(0.0, 0.0));
        nets.create_node_if_absent(gid, &disc(10.0, 0.0));
        nets.connect_nodes(gid, &disc(0.0, 0.0), &disc(10.0, 0.0), None);
        // A (here straight) curve from (5, -5) to (5, 5) across it.
        let curve = BezierConnection::new(
            [[5, 64, -5], [5, 64, 5]],
            [[5.0, 64.0, -5.0], [5.0, 64.0, 5.0]],
            [[0.0, 0.0, 1.0], [0.0, 0.0, -1.0]],
            [[0.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
            true,
            false,
            "create:andesite",
        );
        nets.create_node_if_absent(gid, &disc(5.0, -5.0));
        nets.create_node_if_absent(gid, &disc(5.0, 5.0));
        nets.connect_nodes(gid, &disc(5.0, -5.0), &disc(5.0, 5.0), Some(curve));
        let g = &nets.graphs[&gid];
        let a = g.locate_node(&loc(0.0, 0.0)).unwrap().net_id;
        let b = g.locate_node(&loc(10.0, 0.0)).unwrap().net_id;
        let c = g.locate_node(&loc(5.0, -5.0)).unwrap().net_id;
        let d = g.locate_node(&loc(5.0, 5.0)).unwrap().net_id;
        // Both directions of the straight edge got the crossing, 5 along each way.
        for (x, y) in [(a, b), (b, a)] {
            let i = &g.edge(x, y).unwrap().edge_data.intersections;
            assert_eq!(i.len(), 1, "{i:?}");
            assert!((i[0].location - 5.0).abs() < 0.6, "{}", i[0].location);
        }
        // The curve and its reverse hold it too, pointing back at the straight edge, with one id.
        let ci = &g.edge(c, d).unwrap().edge_data.intersections;
        let di = &g.edge(d, c).unwrap().edge_data.intersections;
        assert_eq!((ci.len(), di.len()), (1, 1));
        assert_eq!(ci[0].id, g.edge(a, b).unwrap().edge_data.intersections[0].id);
        assert!(ci[0].targets(&loc(0.0, 0.0), &loc(10.0, 0.0)));
        // Removing a node of the straight edge clears the crossing off the curve.
        nets.remove_node(gid, &loc(0.0, 0.0), true);
        let g = &nets.graphs[&gid];
        assert!(g.edge(c, d).unwrap().edge_data.intersections.is_empty());
    }

    #[test]
    fn points_are_listed_on_both_directions_of_their_edge() {
        let mut nets = RailNetworks::new();
        let id = line(&mut nets, &[0.0, 10.0]);
        let uuid = nets.new_uuid();
        let p = TrackEdgePoint::new_plain(EdgePointType::Station, uuid, [loc(0.0, 0.0), loc(10.0, 0.0)], 3.0);
        nets.add_point(id, p);
        let g = &nets.graphs[&id];
        let a = g.locate_node(&loc(0.0, 0.0)).unwrap().net_id;
        let b = g.locate_node(&loc(10.0, 0.0)).unwrap().net_id;
        assert_eq!(g.edge(a, b).unwrap().edge_data.points[0].location_on, 3.0);
        assert_eq!(g.edge(b, a).unwrap().edge_data.points[0].location_on, 7.0);
        assert!(g.point(EdgePointType::Station, uuid).is_some());
        assert_eq!(g.points(EdgePointType::Station).len(), 1);
        // A node's removal takes the points on its edges (invalidating them with a level).
        nets.events.clear();
        nets.remove_node(id, &loc(0.0, 0.0), true);
        assert!(nets.events.iter().any(|e| matches!(e, RailEvent::PointInvalidated { .. })));
        assert!(nets.graphs[&id].point(EdgePointType::Station, uuid).is_none());
    }
}
