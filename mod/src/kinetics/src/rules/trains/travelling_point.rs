//! `TravellingPoint` (content/trains/entity/TravellingPoint.java): a point that moves along the track
//! graph, on an edge at a distance from its first node, choosing a way at each junction.
//!
//! A carriage's bogeys are travelling points; so is the head of a train's navigation. [`travel`] moves
//! the point by a distance (forward or back), crossing nodes as it reaches them. Where more than one
//! edge continues, a [`TrackSelector`] decides: [`TrackSelector::Steer`] (the player's lever),
//! [`TrackSelector::Follow`] (stay with another point, the way a trailing bogey follows the leading
//! one), [`TrackSelector::Random`] or one the caller supplies. Edge points passed on the way are
//! reported to a listener, which can stop the point there (a red signal); curves and portals the same.
//!
//! The point names its edge by its two node ids; `edge` in Java is looked up in the graph here.
//! Left out: `write`/`read` (NBT), `migrateTo` (see [`TrackGraphLocation`] for its data).

use super::graph::{NodeId, TrackEdge, TrackEdgePoint, TrackNodeLocation};
use super::network::TrackGraph;
use super::vec::{Vec3, add, cross, dot, equal, scale};
use std::collections::HashSet;

/// `TrackGraphLocation`: a spot on a graph, as the edge's node locations and a distance along it.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackGraphLocation {
    pub graph: super::graph::Uuid,
    pub edge: [TrackNodeLocation; 2],
    pub position: f64,
}

/// `TravellingPoint.SteerDirection`: the dot product with the way each branch leaves it aims for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SteerDirection {
    None,
    Left,
    Right,
}

impl SteerDirection {
    pub fn target_dot(self) -> f64 {
        match self {
            SteerDirection::None => 0.0,
            SteerDirection::Left => -1.0,
            SteerDirection::Right => 1.0,
        }
    }
}

/// One way on at a junction: the node the edge leads to, and the node it leaves from (the entry's
/// value in Java is the edge `from` to `node`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub from: NodeId,
    pub node: NodeId,
}

/// `ITrackSelector`: picks one of the ways on at a junction, by index into the candidates.
pub enum TrackSelector<'a> {
    /// `random()`: the closure gives an index below its argument.
    Random(&'a mut dyn FnMut(usize) -> usize),
    /// `follow(other, success)`: the way that leads to where `other` is. `success` is set to whether
    /// one was found (within 20 edges of searching).
    Follow { node1: Option<NodeId>, node2: Option<NodeId>, success: Option<bool> },
    /// `steer(direction, upNormal)`.
    Steer { direction: SteerDirection, up_normal: Vec3 },
    /// Anything else.
    Custom(&'a mut dyn FnMut(&TrackGraph, &TravellingPoint, bool, &[Candidate]) -> usize),
}

impl<'a> TrackSelector<'a> {
    /// `follow(other)`.
    pub fn follow(other: &TravellingPoint) -> TrackSelector<'a> {
        TrackSelector::Follow { node1: other.node1, node2: other.node2, success: None }
    }

    /// `steer(direction, upNormal)`.
    pub fn steer(direction: SteerDirection, up_normal: Vec3) -> TrackSelector<'a> {
        TrackSelector::Steer { direction, up_normal }
    }

    fn select(&mut self, graph: &TrackGraph, tp: &TravellingPoint, forward: bool, targets: &[Candidate]) -> usize {
        match self {
            TrackSelector::Random(rng) => rng(targets.len()),
            TrackSelector::Custom(f) => f(graph, tp, forward, targets),
            TrackSelector::Steer { direction, up_normal } => steer_select(graph, tp, *direction, *up_normal, targets),
            TrackSelector::Follow { node1, node2, success } => {
                let (idx, found) = follow_select(graph, forward, targets, *node1, *node2);
                *success = Some(found);
                idx
            }
        }
    }
}

pub fn steer_select(graph: &TrackGraph, tp: &TravellingPoint, direction: SteerDirection, up: Vec3, targets: &[Candidate]) -> usize {
    let mut closest = f64::MAX;
    let mut best: Option<usize> = None;
    let Some(edge) = tp.edge(graph) else { return 0 };
    for (i, c) in targets.iter().enumerate() {
        let trajectory = edge.direction(false);
        let Some(entry) = graph.edge(c.from, c.node) else { continue };
        let entry_trajectory = entry.direction(true);
        let normal = cross(trajectory, up);
        let dot = dot(normal, entry_trajectory);
        let diff = (direction.target_dot() - dot).abs();
        if diff > closest {
            continue;
        }
        closest = diff;
        best = Some(i);
    }
    // "Couldn't find steer target, choosing first".
    best.unwrap_or(0)
}

/// `follow`'s search. `node1`/`node2` are the followed point's nodes. A candidate that is one of them
/// wins at once; otherwise the candidates are searched breadth first in step, up to 20 rounds, for the
/// one whose way reaches the followed point's leading node.
fn follow_select(graph: &TrackGraph, forward: bool, targets: &[Candidate], node1: Option<NodeId>, node2: Option<NodeId>) -> (usize, bool) {
    let target = if forward { node1 } else { node2 };
    let secondary = if forward { node2 } else { node1 };

    for (i, c) in targets.iter().enumerate() {
        if Some(c.node) == target || Some(c.node) == secondary {
            return (i, true);
        }
    }

    let mut frontiers: Vec<Vec<(NodeId, NodeId)>> = targets.iter().map(|c| vec![(c.from, c.node)]).collect();
    let mut visiteds: Vec<HashSet<(NodeId, NodeId)>> = targets.iter().map(|c| HashSet::from([(c.from, c.node)])).collect();

    for _ in 0..20 {
        for j in 0..targets.len() {
            if frontiers[j].is_empty() {
                continue;
            }
            let (cur_from, cur_to) = frontiers[j].remove(0);
            let Some(cur_edge) = graph.edge(cur_from, cur_to) else { continue };
            let Some(nexts) = graph.connections_from(cur_to) else { continue };
            for (next_node, next_edge) in nexts {
                if !visiteds[j].insert((cur_to, *next_node)) {
                    continue;
                }
                if !cur_edge.can_travel_to(next_edge) {
                    continue;
                }
                if Some(*next_node) == target {
                    return (j, true);
                }
                frontiers[j].push((cur_to, *next_node));
            }
        }
    }
    (0, false)
}

/// What an edge point listener is told: the point reached, and the two nodes of the edge in the
/// direction travelled.
pub struct EdgePointHit<'a> {
    pub point: &'a TrackEdgePoint,
    pub nodes: (NodeId, NodeId),
}

/// The listeners `travel` calls. Each defaults to ignoring (`ignoreEdgePoints`, `ignoreTurns`,
/// `ignorePortals`).
#[derive(Default)]
pub struct Listeners<'a> {
    /// `IEdgePointListener`: called with the distance travelled so far at the point; true stops the
    /// point there.
    pub edge_point: Option<&'a mut dyn FnMut(f64, &EdgePointHit) -> bool>,
    /// `ITurnListener`: a curve entered, with the distance to it.
    pub turn: Option<&'a mut dyn FnMut(f64, &TrackEdge)>,
    /// `IPortalListener`: a zero-length edge between dimensions, as its two node locations; true
    /// blocks the point.
    pub portal: Option<&'a mut dyn FnMut([TrackNodeLocation; 2]) -> bool>,
}

/// `TravellingPoint`.
#[derive(Clone, Debug, PartialEq)]
pub struct TravellingPoint {
    pub node1: Option<NodeId>,
    pub node2: Option<NodeId>,
    pub position: f64,
    pub blocked: bool,
    pub upside_down: bool,
}

impl TravellingPoint {
    /// `new TravellingPoint()`: on nothing.
    pub fn empty() -> TravellingPoint {
        TravellingPoint { node1: None, node2: None, position: 0.0, blocked: false, upside_down: false }
    }

    pub fn new(node1: NodeId, node2: NodeId, position: f64, upside_down: bool) -> TravellingPoint {
        TravellingPoint { node1: Some(node1), node2: Some(node2), position, blocked: false, upside_down }
    }

    /// `edge`: the edge the point is on.
    pub fn edge<'g>(&self, graph: &'g TrackGraph) -> Option<&'g TrackEdge> {
        graph.edge(self.node1?, self.node2?)
    }

    /// `reverse(graph)`: face the other way on the same edge.
    pub fn reverse(&mut self, graph: &TrackGraph) {
        let (Some(n1), Some(n2)) = (self.node1, self.node2) else { return };
        let length = self.edge(graph).map_or(0.0, |e| e.length());
        self.node1 = Some(n2);
        self.node2 = Some(n1);
        self.position = length - self.position;
    }

    /// `getPosition(graph, flipUpsideDown)`.
    pub fn get_position(&self, graph: Option<&TrackGraph>, flip_upside_down: bool) -> Option<Vec3> {
        self.position_with_offset(graph, 0.0, flip_upside_down)
    }

    /// `getPositionWithOffset(graph, offset, flipUpsideDown)`: the point on the rail, lifted one
    /// block along the edge's normal (down when upside down).
    pub fn position_with_offset(&self, graph: Option<&TrackGraph>, offset: f64, flip_upside_down: bool) -> Option<Vec3> {
        let g = graph?;
        let edge = self.edge(g)?;
        let t = (self.position + offset) / edge.length();
        let sign = if self.upside_down ^ flip_upside_down { -1.0 } else { 1.0 };
        Some(add(edge.position(graph, t), scale(edge.normal(graph, t), sign)))
    }

    /// `travel(graph, distance, selector, listeners)`: moves by `distance` (negative is backward) and
    /// returns how far it got, less than asked when blocked: `blocked` says so.
    pub fn travel(&mut self, graph: &TrackGraph, distance: f64, selector: &mut TrackSelector, listeners: &mut Listeners) -> f64 {
        self.blocked = false;
        let (Some(mut n1), Some(mut n2)) = (self.node1, self.node2) else { return 0.0 };
        let Some(edge0) = graph.edge(n1, n2) else { return 0.0 };
        let mut edge_length = edge0.length();
        if equal(distance, 0.0) {
            return 0.0;
        }

        let prev_pos = self.position;
        let mut traveled = distance;
        let current_t = if edge_length == 0.0 { 0.0 } else { self.position / edge_length };
        let increment_t = edge0.increment_t(current_t, distance);
        self.position = increment_t * edge_length;

        let forward = distance > 0.0;
        let mut collected_distance = if forward { -prev_pos } else { -edge_length + prev_pos };

        let blocked_location =
            self.edge_traversed_from(graph, (n1, n2), forward, listeners, prev_pos, collected_distance);
        if let Some(loc) = blocked_location {
            self.position = loc;
            traveled = self.position - prev_pos;
            return traveled;
        }

        if forward {
            // Moving forward
            while self.position > edge_length {
                let Some(edge) = graph.edge(n1, n2) else { break };
                let mut valid: Vec<Candidate> = Vec::new();
                if let Some(conns) = graph.connections_from(n2) {
                    for (new_node, new_edge) in conns {
                        if *new_node == n1 {
                            continue;
                        }
                        if !edge.can_travel_to(new_edge) {
                            continue;
                        }
                        valid.push(Candidate { from: n2, node: *new_node });
                    }
                }

                if valid.is_empty() {
                    traveled -= self.position - edge_length;
                    self.position = edge_length;
                    self.blocked = true;
                    break;
                }

                let choice = if valid.len() == 1 {
                    valid[0]
                } else {
                    let tp = TravellingPoint { node1: Some(n1), node2: Some(n2), ..self.clone() };
                    valid[selector.select(graph, &tp, true, &valid)]
                };
                let Some(entry_edge) = graph.edge(choice.from, choice.node) else { break };

                if entry_edge.length() == 0.0 {
                    if let (Some(a), Some(b)) = (graph.get_node(n2), graph.get_node(choice.node)) {
                        if let Some(portal) = listeners.portal.as_deref_mut() {
                            if portal([a.location, b.location]) {
                                traveled -= self.position - edge_length;
                                self.position = edge_length;
                                self.blocked = true;
                                break;
                            }
                        }
                    }
                }

                n1 = n2;
                n2 = choice.node;
                self.node1 = Some(n1);
                self.node2 = Some(n2);
                self.position -= edge_length;

                collected_distance += edge_length;
                if entry_edge.is_turn() {
                    if let Some(turn) = listeners.turn.as_deref_mut() {
                        turn(collected_distance, entry_edge);
                    }
                }

                let blocked_location = self.edge_traversed_from(graph, (n1, n2), forward, listeners, 0.0, collected_distance);
                if let Some(loc) = blocked_location {
                    traveled -= self.position;
                    self.position = loc;
                    traveled += self.position;
                    break;
                }

                edge_length = entry_edge.length();
            }
        } else {
            // Moving backwards
            while self.position < 0.0 {
                let Some(edge) = graph.edge(n1, n2) else { break };
                let mut valid: Vec<Candidate> = Vec::new();
                if let Some(conns) = graph.connections_from(n1) {
                    for (new_node, _entry) in conns {
                        if *new_node == n2 {
                            continue;
                        }
                        let Some(back) = graph.edge(*new_node, n1) else { continue };
                        if !back.can_travel_to(edge) {
                            continue;
                        }
                        valid.push(Candidate { from: n1, node: *new_node });
                    }
                }

                if valid.is_empty() {
                    traveled -= self.position;
                    self.position = 0.0;
                    self.blocked = true;
                    break;
                }

                let choice = if valid.len() == 1 {
                    valid[0]
                } else {
                    let tp = TravellingPoint { node1: Some(n1), node2: Some(n2), ..self.clone() };
                    valid[selector.select(graph, &tp, false, &valid)]
                };
                let Some(entry_edge) = graph.edge(choice.from, choice.node) else { break };

                if entry_edge.length() == 0.0 {
                    if let (Some(a), Some(b)) = (graph.get_node(choice.node), graph.get_node(n1)) {
                        if let Some(portal) = listeners.portal.as_deref_mut() {
                            if portal([a.location, b.location]) {
                                traveled -= self.position;
                                self.position = 0.0;
                                self.blocked = true;
                                break;
                            }
                        }
                    }
                }

                n2 = n1;
                n1 = choice.node;
                self.node1 = Some(n1);
                self.node2 = Some(n2);
                let Some(new_edge) = graph.edge(n1, n2) else { break };
                collected_distance += edge_length;
                edge_length = new_edge.length();
                self.position += edge_length;

                let blocked_location =
                    self.edge_traversed_from(graph, (n1, n2), forward, listeners, edge_length, collected_distance);
                if let Some(loc) = blocked_location {
                    traveled -= self.position;
                    self.position = loc;
                    traveled += self.position;
                    break;
                }
            }
        }

        traveled
    }

    /// `edgeTraversedFrom`: the edge points on the stretch just crossed, offered to the listener in the
    /// order met; the position of the first it stops at.
    fn edge_traversed_from(
        &self,
        graph: &TrackGraph,
        edge_nodes: (NodeId, NodeId),
        forward: bool,
        listeners: &mut Listeners,
        prev_pos: f64,
        total_distance: f64,
    ) -> Option<f64> {
        let edge = graph.edge(edge_nodes.0, edge_nodes.1)?;
        if edge.is_turn() {
            if let Some(turn) = listeners.turn.as_deref_mut() {
                turn(total_distance.max(0.0), edge);
            }
        }

        let from = if forward { prev_pos } else { self.position };
        let to = if forward { self.position } else { prev_pos };

        let points = &edge.edge_data.points;
        let length = edge.length();
        for i in 0..points.len() {
            let index = if forward { i } else { points.len() - i - 1 };
            let r = points[index];
            let location_on = r.location_on;
            let distance = if forward { location_on } else { length - location_on };
            let skip = if forward { location_on < from || location_on >= to } else { location_on <= from || location_on > to };
            if skip {
                continue;
            }
            let Some(point) = graph.edge_points.get(r.ty, r.id) else { continue };
            let nodes = if forward { edge_nodes } else { (edge_nodes.1, edge_nodes.0) };
            if let Some(listener) = listeners.edge_point.as_deref_mut() {
                if listener(total_distance + distance, &EdgePointHit { point, nodes }) {
                    return Some(location_on);
                }
            }
        }
        None
    }

    /// `migrateTo(locations)`'s data: the node ids and position at a graph location.
    pub fn migrate_to(&mut self, graph: &TrackGraph, location: &TrackGraphLocation) {
        self.node1 = graph.locate_node(&location.edge[0]).map(|n| n.net_id);
        self.node2 = graph.locate_node(&location.edge[1]).map(|n| n.net_id);
        self.position = location.position;
    }
}

#[cfg(test)]
mod tests {
    use super::super::graph::{DiscoveredLocation, EdgePointType, TrackEdgePoint};
    use super::super::network::RailNetworks;
    use super::*;

    fn loc(x: f64, z: f64) -> TrackNodeLocation {
        TrackNodeLocation::new([x, 64.0, z], 0)
    }
    fn disc(x: f64, z: f64) -> DiscoveredLocation {
        DiscoveredLocation::new(0, [x, 64.0, z]).with_normal([0.0, 1.0, 0.0])
    }
    fn id(nets: &RailNetworks, g: super::super::graph::Uuid, x: f64, z: f64) -> NodeId {
        nets.graphs[&g].locate_node(&loc(x, z)).unwrap().net_id
    }

    /// A line 0..30 in steps of 10 with a fork at x = 20: one branch carries on to x = 30, the other
    /// leaves slightly to the side to (30, 1) (within the 7/8 dot limit: a gentle bend).
    fn fork() -> (RailNetworks, super::super::graph::Uuid) {
        let mut nets = RailNetworks::new();
        let g = nets.new_graph();
        let gid = g.id;
        nets.put_graph_with_default_group(g);
        for (x, z) in [(0.0, 0.0), (10.0, 0.0), (20.0, 0.0), (30.0, 0.0), (30.0, 4.0)] {
            nets.create_node_if_absent(gid, &disc(x, z));
        }
        for (a, b) in [((0.0, 0.0), (10.0, 0.0)), ((10.0, 0.0), (20.0, 0.0)), ((20.0, 0.0), (30.0, 0.0)), ((20.0, 0.0), (30.0, 4.0))] {
            nets.connect_nodes(gid, &disc(a.0, a.1), &disc(b.0, b.1), None);
        }
        (nets, gid)
    }

    fn no_listeners<'a>() -> Listeners<'a> {
        Listeners::default()
    }

    #[test]
    fn travels_along_an_edge_and_over_a_node() {
        let (nets, g) = fork();
        let graph = &nets.graphs[&g];
        let (a, b, c) = (id(&nets, g, 0.0, 0.0), id(&nets, g, 10.0, 0.0), id(&nets, g, 20.0, 0.0));
        let mut tp = TravellingPoint::new(a, b, 2.0, false);
        let mut sel = TrackSelector::Custom(&mut |_, _, _, _| 0);
        let got = tp.travel(graph, 5.0, &mut sel, &mut no_listeners());
        assert_eq!(got, 5.0);
        assert!((tp.position - 7.0).abs() < 1e-9);
        // Over the node: 7 + 6 = 13, 3 past b onto b -> c.
        let got = tp.travel(graph, 6.0, &mut sel, &mut no_listeners());
        assert_eq!(got, 6.0);
        assert_eq!((tp.node1, tp.node2), (Some(b), Some(c)));
        assert!((tp.position - 3.0).abs() < 1e-9);
        assert!(!tp.blocked);
        // And back again.
        let got = tp.travel(graph, -6.0, &mut sel, &mut no_listeners());
        assert_eq!(got, -6.0);
        assert_eq!((tp.node1, tp.node2), (Some(a), Some(b)));
        assert!((tp.position - 7.0).abs() < 1e-9);
    }

    #[test]
    fn the_end_of_the_line_blocks() {
        let (nets, g) = fork();
        let graph = &nets.graphs[&g];
        let (a, b) = (id(&nets, g, 0.0, 0.0), id(&nets, g, 10.0, 0.0));
        let mut tp = TravellingPoint::new(a, b, 2.0, false);
        let mut sel = TrackSelector::Custom(&mut |_, _, _, _| 0);
        // Back off the west end: stops at 0, having moved only 2.
        let got = tp.travel(graph, -5.0, &mut sel, &mut no_listeners());
        assert!(tp.blocked);
        assert_eq!(tp.position, 0.0);
        assert!((got + 2.0).abs() < 1e-9, "{got}");
        // Forward past the east end of the whole line (the two last nodes) stops at the end.
        let (c, d) = (id(&nets, g, 20.0, 0.0), id(&nets, g, 30.0, 0.0));
        let mut tp = TravellingPoint::new(c, d, 8.0, false);
        let got = tp.travel(graph, 5.0, &mut sel, &mut no_listeners());
        assert!(tp.blocked);
        assert!((tp.position - 10.0).abs() < 1e-9);
        assert!((got - 2.0).abs() < 1e-9, "{got}");
    }

    #[test]
    fn a_junction_asks_the_selector() {
        let (nets, g) = fork();
        let graph = &nets.graphs[&g];
        let (b, c, straight, bent) =
            (id(&nets, g, 10.0, 0.0), id(&nets, g, 20.0, 0.0), id(&nets, g, 30.0, 0.0), id(&nets, g, 30.0, 4.0));
        let at = || TravellingPoint::new(b, c, 8.0, false);

        let mut asked = 0;
        let mut first = TrackSelector::Custom(&mut |_, _, forward, c| {
            asked += 1;
            assert!(forward && c.len() == 2);
            0
        });
        let mut tp = at();
        tp.travel(graph, 4.0, &mut first, &mut no_listeners());
        assert_eq!(asked, 1);
        let chose_first = tp.node2.unwrap();
        assert!(chose_first == straight || chose_first == bent);

        // Steering: the straight edge leaves at dot 0 against the point's normal, the bend off to a side.
        let mut tp = at();
        tp.travel(graph, 4.0, &mut TrackSelector::steer(SteerDirection::None, [0.0, 1.0, 0.0]), &mut no_listeners());
        assert_eq!(tp.node2, Some(straight));
        let mut tp = at();
        tp.travel(graph, 4.0, &mut TrackSelector::steer(SteerDirection::Right, [0.0, 1.0, 0.0]), &mut no_listeners());
        let right = tp.node2.unwrap();
        let mut tp = at();
        tp.travel(graph, 4.0, &mut TrackSelector::steer(SteerDirection::Left, [0.0, 1.0, 0.0]), &mut no_listeners());
        let left = tp.node2.unwrap();
        assert_ne!(left, right);
        assert!(left == bent || right == bent);

        // Following a point already on the bent branch takes the bent branch.
        let leader = TravellingPoint::new(c, bent, 1.0, false);
        let mut tp = at();
        let mut follow = TrackSelector::follow(&leader);
        tp.travel(graph, 4.0, &mut follow, &mut no_listeners());
        assert_eq!(tp.node2, Some(bent));
        assert!(matches!(follow, TrackSelector::Follow { success: Some(true), .. }));
    }

    #[test]
    fn edge_points_are_reported_and_can_stop_the_point() {
        let (mut nets, g) = fork();
        let uuid = nets.new_uuid();
        nets.add_point(g, TrackEdgePoint::new_plain(EdgePointType::Station, uuid, [loc(0.0, 0.0), loc(10.0, 0.0)], 6.0));
        // `position` 6 is measured from the second location's side: on a -> b it is 6 along.
        let graph = &nets.graphs[&g];
        let (a, b) = (id(&nets, g, 0.0, 0.0), id(&nets, g, 10.0, 0.0));
        let mut tp = TravellingPoint::new(a, b, 1.0, false);
        let mut sel = TrackSelector::Custom(&mut |_, _, _, _| 0);

        let mut seen: Vec<(f64, (NodeId, NodeId))> = Vec::new();
        let mut listener = |d: f64, hit: &EdgePointHit| {
            seen.push((d, hit.nodes));
            false
        };
        let mut l = Listeners { edge_point: Some(&mut listener), ..Default::default() };
        tp.travel(graph, 8.0, &mut sel, &mut l);
        assert_eq!(seen, vec![(5.0, (a, b))]);
        // The point moved the full 8.
        assert!((tp.position - 9.0).abs() < 1e-9);

        // Stopping there: the point halts on it, having travelled 5.
        let mut tp = TravellingPoint::new(a, b, 1.0, false);
        let mut stop = |_: f64, _: &EdgePointHit| true;
        let mut l = Listeners { edge_point: Some(&mut stop), ..Default::default() };
        let got = tp.travel(graph, 8.0, &mut sel, &mut l);
        assert_eq!(tp.position, 6.0);
        assert!((got - 5.0).abs() < 1e-9, "{got}");

        // Coming the other way (b -> a), the same point is 10 - 6 = 4 along, 3 from the start's 1.
        let mut tp = TravellingPoint::new(b, a, 1.0, false);
        let mut seen2: Vec<f64> = Vec::new();
        let mut listener2 = |d: f64, _: &EdgePointHit| {
            seen2.push(d);
            false
        };
        let mut l = Listeners { edge_point: Some(&mut listener2), ..Default::default() };
        tp.travel(graph, 8.0, &mut sel, &mut l);
        assert_eq!(seen2, vec![3.0]);
    }

    #[test]
    fn reverse_flips_the_edge() {
        let (nets, g) = fork();
        let graph = &nets.graphs[&g];
        let (a, b) = (id(&nets, g, 0.0, 0.0), id(&nets, g, 10.0, 0.0));
        let mut tp = TravellingPoint::new(a, b, 3.0, false);
        tp.reverse(graph);
        assert_eq!((tp.node1, tp.node2, tp.position), (Some(b), Some(a), 7.0));
        // The rail point is one block above the track along the edge normal.
        let p = tp.get_position(Some(graph), false).unwrap();
        assert!((p[0] - 3.0).abs() < 1e-6 && (p[1] - 65.0).abs() < 1e-9, "{p:?}");
        let p = tp.get_position(Some(graph), true).unwrap();
        assert!((p[1] - 63.0).abs() < 1e-9, "{p:?}");
    }
}
