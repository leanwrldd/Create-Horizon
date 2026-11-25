//! `TrackGraphHelper` (content/trains/graph/TrackGraphHelper.java) and the overlap test of
//! `TrackTargetingBlockItem.withGraphLocation` (content/trains/track/TrackTargetingBlockItem.java): where on the
//! track graph a track block and a direction land, and whether an edge point may go there.
//!
//! `TrackTargetingBehaviour.determineGraphLocation` and the targeting item both ask for a [`TrackGraphLocation`]
//! from a track block, the direction the point faces and, on a curve, the segment. The glue answers what the
//! blocks say through [`TrackQuery`] (the axes of a block, where its centre sits, the curve it holds toward a
//! position); the walk and the lookups in the graph are here.
//!
//! A curve's end that is tilted (`bc.smoothing`) is looked up with its offset, as `getBezierGraphLocationAt` does.

use super::bezier::BezierConnection;
use super::graph::{EdgePointType, TrackNodeLocation};
use super::network::RailNetworks;
use super::propagator::{TrackWorld, walk_connected_tracks};
use super::targeting::BezierTrackPointLocation;
use super::travelling_point::TrackGraphLocation;
use super::vec::{Vec3, distance_to, length, normalize, scale, sub};
use crate::Pos;

/// 1 / 4096, as `1 / 4096f` in Java.
const SQR_TOLERANCE: f64 = (1.0_f32 / 4096.0_f32) as f64;

/// What `TrackGraphHelper` asks of the track blocks (`ITrackBlock`): their axes, their centre and their curves.
pub trait TrackQuery: TrackWorld {
    /// `getTrackAxes(level, pos, state)`; empty when the block is not track.
    fn axes(&self, pos: Pos) -> Vec<Vec3>;
    /// `getElevationAtCenter(level, pos, state)`: half a block up for a slope, otherwise 0.
    fn elevation_at_center(&self, pos: Pos) -> f64;
    /// `TrackBlockEntity.getConnections().get(target)`: the curve the block at `pos` holds toward `target`.
    fn curve(&self, pos: Pos, target: Pos) -> Option<BezierConnection>;
    /// The dimension of the level, as the index the graph's node locations use.
    fn dimension(&self) -> u32;
}

/// `Vec3.atBottomCenterOf(pos).add(0, elevation, 0)`.
fn centre_at(pos: Pos, elevation: f64) -> Vec3 {
    [f64::from(pos[0]) + 0.5, f64::from(pos[1]) + elevation, f64::from(pos[2]) + 0.5]
}

fn flat(v: Vec3, x: f64, z: f64) -> Vec3 {
    [v[0] * x, 0.0, v[2] * z]
}

fn distance_sqr(a: Vec3, b: Vec3) -> f64 {
    let d = distance_to(a, b);
    d * d
}

/// `getGraphLocationAt(level, pos, targetDirection, targetAxis)`; `direction_step` is the direction's step (1 for
/// positive, -1 for negative).
pub fn graph_location_at(
    world: &dyn TrackQuery,
    nets: &RailNetworks,
    pos: Pos,
    direction_step: i32,
    target_axis: Vec3,
) -> Option<TrackGraphLocation> {
    if world.axes(pos).is_empty() {
        return None;
    }
    let axis = scale(target_axis, f64::from(direction_step));
    let axis_length = length(axis);
    let elevation = world.elevation_at_center(pos);
    let dimension = world.dimension();
    let mut graph = None;

    // Case 1: the centre of the block lies on a node.
    let centre_location = TrackNodeLocation::new(centre_at(pos, elevation), dimension);
    if let Some(found) = nets.get_graph(&centre_location) {
        graph = Some(found);
        let g = nets.graphs.get(&found)?;
        if let Some(node) = g.locate_node(&centre_location) {
            for (back, edge) in g.connections_from(node.net_id).into_iter().flatten() {
                let direction = edge.direction(true);
                if distance_sqr(scale(direction, axis_length), scale(axis, -1.0)) > SQR_TOLERANCE {
                    continue;
                }
                let back_node = g.get_node(*back)?;
                return Some(TrackGraphLocation { graph: found, edge: [node.location, back_node.location], position: 0.0 });
            }
        }
    }

    // Case 2: the centre of the block is between two nodes.
    let ends = world.connected(pos, true, None);
    let start = centre_at(pos, elevation);
    let mut front_node: Option<TrackNodeLocation> = None;
    let mut back_node: Option<TrackNodeLocation> = None;
    let mut position = 0.0;
    let mut single_track_piece = true;
    for end in ends {
        let offset = scale(normalize(sub(end.loc.location(), start)), axis_length);
        let compare_offset = normalize(flat(offset, 1.0, 1.0));
        let forward = distance_sqr(compare_offset, normalize(flat(axis, -1.0, -1.0))) < SQR_TOLERANCE;
        let backwards = distance_sqr(compare_offset, normalize(flat(axis, 1.0, 1.0))) < SQR_TOLERANCE;
        if !forward && !backwards {
            continue;
        }
        let mut current = end;
        let mut previous: Option<TrackNodeLocation> = None;
        let mut distance = 0.0;
        let mut i = 0;
        while i < 100 && distance < 32.0 {
            i += 1;
            let loc = current.clone();
            if graph.is_none() {
                graph = nets.get_graph(&loc.loc);
            }
            let node = graph.and_then(|g| nets.graphs.get(&g)).and_then(|g| g.locate_node(&loc.loc));
            let Some(node) = node else {
                single_track_piece = false;
                for next in walk_connected_tracks(world, &loc.loc, true) {
                    if previous == Some(next.loc) {
                        continue;
                    }
                    let diff = sub(next.loc.location(), loc.loc.location());
                    let wanted = if forward { scale(axis, -1.0) } else { axis };
                    if distance_sqr(wanted, scale(normalize(diff), axis_length)) > SQR_TOLERANCE {
                        continue;
                    }
                    previous = Some(current.loc);
                    distance += length(diff);
                    current = next;
                    break;
                }
                continue;
            };
            if forward {
                front_node = Some(node.location);
            }
            if backwards {
                back_node = Some(node.location);
                position = distance + axis_length / 2.0;
            }
            break;
        }
    }
    let (front, back) = (front_node?, back_node?);
    if single_track_piece {
        position = distance_to(front.location(), back.location()) / 2.0;
    }
    Some(TrackGraphLocation { graph: graph?, edge: [back, front], position })
}

/// `getBezierGraphLocationAt(level, pos, targetDirection, targetBezier)`.
pub fn bezier_graph_location_at(
    world: &dyn TrackQuery,
    nets: &RailNetworks,
    pos: Pos,
    positive: bool,
    target: BezierTrackPointLocation,
) -> Option<TrackGraphLocation> {
    if world.axes(pos).is_empty() {
        return None;
    }
    let bc = world.curve(pos, target.curve_target)?;
    if !bc.is_primary() {
        return None;
    }
    let mut target_location = TrackNodeLocation::new(bc.starts[1], world.dimension());
    if let Some(smoothing) = bc.smoothing {
        target_location.y_offset_pixels = smoothing[1];
    }
    for location in world.connected(pos, true, None) {
        let Some(graph_id) = nets.get_graph(&location.loc) else { continue };
        let graph = nets.graphs.get(&graph_id)?;
        let Some(target_node) = graph.locate_node(&target_location) else { continue };
        let Some(node) = graph.locate_node(&location.loc) else { continue };
        let Some(edge) = graph.edge(node.net_id, target_node.net_id) else { continue };
        let mut found = TrackGraphLocation {
            graph: graph_id,
            edge: [location.loc, target_location],
            position: f64::from((target.segment + 1) as f32 / 2.0),
        };
        if positive {
            found.edge.swap(0, 1);
            found.position = edge.length() - found.position;
        }
        return Some(found);
    }
    None
}

/// `TrackTargetingBlockItem.OverlapResult`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlapResult {
    Valid,
    Occupied,
    Junction,
    NoTrack,
}

impl OverlapResult {
    /// The lang key of the feedback (`feedback`), none for a valid spot.
    pub fn feedback(self) -> Option<&'static str> {
        match self {
            OverlapResult::Valid => None,
            OverlapResult::Occupied => Some("create.track_target.occupied"),
            OverlapResult::Junction => Some("create.track_target.no_junctions"),
            OverlapResult::NoTrack => Some("create.track_target.invalid"),
        }
    }
}

/// `withGraphLocation(level, pos, front, targetBezier, type, callback)`: the location (when there is one) and how it
/// compares with the points already on its edge. `front` is the direction the point faces (positive).
pub fn with_graph_location(
    world: &dyn TrackQuery,
    nets: &RailNetworks,
    pos: Pos,
    front: bool,
    target_bezier: Option<BezierTrackPointLocation>,
    ty: EdgePointType,
) -> (OverlapResult, Option<TrackGraphLocation>) {
    let axes = world.axes(pos);
    if axes.is_empty() {
        return (OverlapResult::NoTrack, None);
    }
    if target_bezier.is_none() && axes.len() > 1 {
        return (OverlapResult::Junction, None);
    }
    let location = match target_bezier {
        Some(bezier) => bezier_graph_location_at(world, nets, pos, front, bezier),
        None => graph_location_at(world, nets, pos, if front { 1 } else { -1 }, axes[0]),
    };
    let Some(location) = location else { return (OverlapResult::NoTrack, None) };
    let Some(graph) = nets.graphs.get(&location.graph) else { return (OverlapResult::NoTrack, None) };
    let (Some(node1), Some(node2)) = (graph.locate_node(&location.edge[0]), graph.locate_node(&location.edge[1])) else {
        return (OverlapResult::NoTrack, None);
    };
    // Java returns without calling back when there is no edge; the item then reads a null result.
    let Some(edge) = graph.edge(node1.net_id, node2.net_id) else { return (OverlapResult::NoTrack, None) };
    for point in &edge.edge_data.points {
        let distance = (location.position - point.location_on).abs();
        if distance > 0.75 {
            continue;
        }
        // `canCoexistWith(type, front)`: a signal sits with another signal; `front` doesn't matter to any type.
        if point.ty.can_coexist_with(ty) && distance < 0.25 {
            continue;
        }
        return (OverlapResult::Occupied, Some(location));
    }
    (OverlapResult::Valid, Some(location))
}

#[cfg(test)]
mod tests {
    use super::super::graph::{DiscoveredLocation, Uuid};
        use super::super::propagator::{StraightTrack, connected_straight};
    use super::*;

    /// A row of straight blocks along x at y 64, z 0, from `from` to `to` (inclusive).
    struct Row {
        from: i32,
        to: i32,
    }

    impl Row {
        fn has(&self, pos: Pos) -> bool {
            pos[1] == 64 && pos[2] == 0 && pos[0] >= self.from && pos[0] <= self.to
        }
    }

    impl TrackWorld for Row {
        fn connected(&self, pos: Pos, _: bool, connected_to: Option<&TrackNodeLocation>) -> Vec<DiscoveredLocation> {
            if !self.has(pos) {
                return Vec::new();
            }
            let material = |_: Vec3| None;
            let y_offset = |_: Vec3| 0;
            let t = StraightTrack {
                center: centre_at(pos, 0.0),
                axes: vec![[1.0, 0.0, 0.0]],
                normal: [0.0, 1.0, 0.0],
                dimension: 0,
                material: &material,
                y_offset: &y_offset,
            };
            connected_straight(&t, connected_to)
        }
    }

    impl TrackQuery for Row {
        fn axes(&self, pos: Pos) -> Vec<Vec3> {
            if self.has(pos) { vec![[1.0, 0.0, 0.0]] } else { Vec::new() }
        }
        fn elevation_at_center(&self, _: Pos) -> f64 {
            0.0
        }
        fn curve(&self, _: Pos, _: Pos) -> Option<BezierConnection> {
            None
        }
        fn dimension(&self) -> u32 {
            0
        }
    }

    /// A graph with nodes at the west end of block 0 and the east end of block 4 (x = 0 and 5, on the row's
    /// z = 0.5 line), joined by one edge.
    fn graph_with_row() -> (RailNetworks, Uuid) {
        let mut nets = RailNetworks::new();
        let graph = nets.new_graph();
        let id = graph.id;
        nets.put_graph_with_default_group(graph);
        let disc = |x: f64| DiscoveredLocation::new(0, [x, 64.0, 0.5]).with_normal([0.0, 1.0, 0.0]);
        for x in [0.0, 5.0] {
            assert!(nets.create_node_if_absent(id, &disc(x)));
        }
        nets.connect_nodes(id, &disc(0.0), &disc(5.0), None);
        (nets, id)
    }

    fn node(x: f64) -> TrackNodeLocation {
        TrackNodeLocation::new([x, 64.0, 0.5], 0)
    }

    #[test]
    fn a_block_between_two_nodes_is_located_on_the_edge_between_them() {
        let (nets, g) = graph_with_row();
        let row = Row { from: 0, to: 4 };
        // Facing east (+x) the front node is the one behind the axis, the west one; the edge runs from the back
        // node (east) to the front, and the position is measured from the back: two blocks walked plus half one.
        let at = graph_location_at(&row, &nets, [2, 64, 0], 1, [1.0, 0.0, 0.0]).expect("a location");
        assert_eq!(at.graph, g);
        assert_eq!(at.edge, [node(5.0), node(0.0)]);
        assert!((at.position - 2.5).abs() < 1e-9, "{}", at.position);
        // The other direction swaps the edge's ends.
        let at = graph_location_at(&row, &nets, [2, 64, 0], -1, [1.0, 0.0, 0.0]).expect("a location");
        assert_eq!(at.edge, [node(0.0), node(5.0)]);
        assert!((at.position - 2.5).abs() < 1e-9, "{}", at.position);
    }

    #[test]
    fn a_non_track_block_has_no_location() {
        let (nets, _) = graph_with_row();
        let row = Row { from: 0, to: 4 };
        assert!(graph_location_at(&row, &nets, [9, 64, 0], 1, [1.0, 0.0, 0.0]).is_none());
        let (result, location) = with_graph_location(&row, &nets, [9, 64, 0], true, None, EdgePointType::Station);
        assert_eq!(result, OverlapResult::NoTrack);
        assert!(location.is_none());
    }

    #[test]
    fn a_point_near_another_is_occupied_unless_signals_share_a_spot() {
        let (mut nets, g) = graph_with_row();
        let row = Row { from: 0, to: 4 };
        let (result, location) = with_graph_location(&row, &nets, [2, 64, 0], true, None, EdgePointType::Station);
        assert_eq!(result, OverlapResult::Valid);
        let location = location.unwrap();
        // A station where the item would go: occupied.
        let point = super::super::graph::TrackEdgePoint::new_station(7, location.edge, location.position);
        nets.add_point(g, point);
        let (result, _) = with_graph_location(&row, &nets, [2, 64, 0], true, None, EdgePointType::Station);
        assert_eq!(result, OverlapResult::Occupied);
        let (result, _) = with_graph_location(&row, &nets, [2, 64, 0], true, None, EdgePointType::Signal);
        assert_eq!(result, OverlapResult::Occupied);
        // A block further away (more than three quarters of a block) is free.
        let (result, _) = with_graph_location(&row, &nets, [4, 64, 0], true, None, EdgePointType::Station);
        assert_eq!(result, OverlapResult::Valid);
    }

    #[test]
    fn feedback_keys() {
        assert_eq!(OverlapResult::Valid.feedback(), None);
        assert_eq!(OverlapResult::Occupied.feedback(), Some("create.track_target.occupied"));
        assert_eq!(OverlapResult::Junction.feedback(), Some("create.track_target.no_junctions"));
        assert_eq!(OverlapResult::NoTrack.feedback(), Some("create.track_target.invalid"));
    }
}
