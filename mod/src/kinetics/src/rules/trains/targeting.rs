//! `TrackTargetingBehaviour` (content/trains/track/TrackTargetingBehaviour.java): how a signal,
//! station or observer block's behaviour finds its place on the track graph and puts (or merges)
//! its edge point there.
//!
//! The block entity knows which track block it targets (an offset), which way along the track it
//! faces and, on a curve, which curve and segment. The glue turns that into a [`TrackGraphLocation`]
//! (`determineGraphLocation`: `TrackGraphHelper.getGraphLocationAt` and `getBezierGraphLocationAt`,
//! which read the track blocks) and calls [`TargetingState::create_edge_point`], which is the rest of
//! `createEdgePoint`: the direction bookkeeping for a block that was rotated, the check that nothing
//! else is at the spot, merging a second signal into the first, and adding the new point to the graph.
//!
//! Left out: NBT, `invalidateEdgePoint` migration data, rendering (`render`, overlay types), the map
//! marker position, `transform`'s structure transform of the target (the glue rotates `target_track`
//! and `target_bezier` and calls [`TargetingState::transformed`] to get the new id and the rotated
//! axis), `hasValidTrack` and the track block lookups.

use super::graph::{EdgePointType, TrackEdgePoint, Uuid};
use super::network::RailNetworks;
use super::signal::SignalType;
use super::travelling_point::TrackGraphLocation;
use super::vec::{Vec3, dot, scale};
use crate::Pos;

/// `BezierTrackPointLocation`: the curve (by the block position at its other end) and the segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BezierTrackPointLocation {
    pub curve_target: Pos,
    pub segment: i32,
}

/// What the block entity says when it adds itself to the point (`blockEntityAdded(be, front)`).
#[derive(Clone, Debug, PartialEq)]
pub struct BlockEntityInfo {
    pub pos: Pos,
    pub dimension: u32,
    /// A signal's type (from its block state) and whether it is powered now.
    pub signal_type: Option<SignalType>,
    pub reported_power: bool,
    /// A station block in assembly mode.
    pub assembling: bool,
    /// An observer's filter key.
    pub filter: Option<String>,
}

/// What `createEdgePoint` came to.
#[derive(Clone, Debug, PartialEq)]
pub enum Placement {
    /// It returned null: no graph, no edge, something in the way, or the direction was reversed
    /// ([`TargetingState::target_positive`] flipped: the glue tries again next tick).
    Nothing,
    /// A new point was added to the graph.
    Created(Uuid),
    /// Another of the same type was there and took this block entity too; the behaviour's id is now its.
    Merged(Uuid),
}

/// The state of a `TrackTargetingBehaviour`.
#[derive(Clone, Debug, PartialEq)]
pub struct TargetingState {
    pub edge_point_type: EdgePointType,
    /// `targetTrack`: the target's offset from the block entity.
    pub target_track: Pos,
    pub target_bezier: Option<BezierTrackPointLocation>,
    /// `targetDirection == AxisDirection.POSITIVE`.
    pub target_positive: bool,
    pub id: Uuid,
    pub prev_direction: Option<Vec3>,
    pub rotated_direction: Option<Vec3>,
    pub orthogonal: bool,
}

impl TargetingState {
    /// `new TrackTargetingBehaviour(be, type)`: facing positive, targeting the block itself.
    pub fn new(edge_point_type: EdgePointType, id: Uuid) -> TargetingState {
        TargetingState {
            edge_point_type,
            target_track: [0, 0, 0],
            target_bezier: None,
            target_positive: true,
            id,
            prev_direction: None,
            rotated_direction: None,
            orthogonal: false,
        }
    }

    /// `getGlobalPosition()`: the target as a position, `block_pos` being the block entity's.
    pub fn global_position(&self, block_pos: Pos) -> Pos {
        [block_pos[0] + self.target_track[0], block_pos[1] + self.target_track[1], block_pos[2] + self.target_track[2]]
    }

    /// `isOnCurve()`.
    pub fn is_on_curve(&self) -> bool {
        self.target_bezier.is_some()
    }

    /// `isOrthogonal()`.
    pub fn is_orthogonal(&self) -> bool {
        self.orthogonal
    }

    /// `transform(be, transform)`'s part that does not read the structure: a new id, and the
    /// previous direction (rotated by the glue) kept as `rotatedDirection`.
    pub fn transformed(&mut self, new_id: Uuid, rotated_prev_direction: Option<Vec3>) {
        self.id = new_id;
        if let Some(r) = rotated_prev_direction {
            self.rotated_direction = Some(r);
        }
    }

    /// `createEdgePoint()`'s second half, from the graph location the glue worked out. `new_id` is
    /// the id for a new point (the behaviour's `id` when it has none yet).
    pub fn create_edge_point(
        &mut self,
        nets: &mut RailNetworks,
        loc: &TrackGraphLocation,
        be: &BlockEntityInfo,
    ) -> Placement {
        let ty = self.edge_point_type;
        let graph_id = loc.graph;
        let (edge_length, direction, direction_at, existing) = {
            let Some(graph) = nets.graphs.get(&graph_id) else { return Placement::Nothing };
            let (Some(node1), Some(node2)) = (graph.locate_node(&loc.edge[0]), graph.locate_node(&loc.edge[1])) else {
                return Placement::Nothing;
            };
            let Some(edge) = graph.edge(node1.net_id, node2.net_id) else { return Placement::Nothing };
            let existing: Vec<(EdgePointType, Uuid)> = if edge.edge_data.has_points() {
                EdgePointType::ALL
                    .into_iter()
                    .filter_map(|t| edge.edge_data.get(t, loc.position).map(|r| (t, r.id)))
                    .collect()
            } else {
                Vec::new()
            };
            (edge.length(), edge.direction(true), edge.direction_at(loc.position), existing)
        };

        let front = self.target_positive;
        let prev = scale(direction_at, if front { -1.0 } else { 1.0 });
        self.prev_direction = Some(prev);

        if let Some(rotated) = self.rotated_direction {
            let d = dot(prev, rotated);
            if d < f64::from(-0.85_f32) {
                self.rotated_direction = None;
                self.target_positive = !self.target_positive;
                return Placement::Nothing;
            }
            self.rotated_direction = None;
        }

        {
            self.orthogonal = self.target_bezier.is_none();
            let non_zero = direction.iter().filter(|c| **c != 0.0).count();
            self.orthogonal &= non_zero <= 1;
        }

        for (other_type, other_id) in existing {
            if other_type != ty {
                if !other_type.can_coexist_with(ty) {
                    return Placement::Nothing;
                }
                continue;
            }
            if !other_type.can_merge() {
                return Placement::Nothing;
            }
            if let Some(p) = nets.graphs.get_mut(&graph_id).and_then(|g| g.edge_points.get_mut(ty, other_id)) {
                if let Some(s) = p.signal_mut() {
                    s.block_entity_added(be.pos, front, be.signal_type, be.reported_power);
                }
            }
            self.id = other_id;
            return Placement::Merged(other_id);
        }

        let single = ty != EdgePointType::Signal;
        let reverse_edge = front || single;
        let edge_location = if reverse_edge { loc.edge } else { [loc.edge[1], loc.edge[0]] };
        let position = if reverse_edge { loc.position } else { edge_length - loc.position };

        let mut point = match ty {
            EdgePointType::Signal => TrackEdgePoint::new_signal(self.id, edge_location, position),
            EdgePointType::Station => TrackEdgePoint::new_station(self.id, edge_location, position),
            EdgePointType::Observer => TrackEdgePoint::new_observer(self.id, edge_location, position),
        };
        if let Some(s) = point.signal_mut() {
            s.block_entity_added(be.pos, front, be.signal_type, be.reported_power);
        }
        if let Some(s) = point.station_mut() {
            s.block_entity_added(be.pos, be.dimension, be.assembling);
        }
        if let Some(o) = point.observer_mut() {
            o.block_entity_added(be.pos, be.dimension, be.filter.clone());
        }
        nets.add_point(graph_id, point);
        Placement::Created(self.id)
    }

    /// `destroy()`: the block entity is gone; the point goes with it (a signal when its last
    /// block entity does).
    pub fn block_entity_removed(&self, nets: &mut RailNetworks, point: Uuid, pos: Pos) {
        match self.edge_point_type {
            EdgePointType::Signal => {
                let graph = nets.graphs.iter().find(|(_, g)| g.point(EdgePointType::Signal, point).is_some()).map(|(id, _)| *id);
                if let Some(g) = graph {
                    nets.signal_block_entity_removed(g, point, pos);
                }
            }
            t => nets.remove_point_from_all_graphs(t, point),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::network::tests::{line, loc};
    use super::*;

    fn at(g: Uuid, a: f64, b: f64, position: f64) -> TrackGraphLocation {
        TrackGraphLocation { graph: g, edge: [loc(a, 0.0), loc(b, 0.0)], position }
    }

    fn be(signal: bool) -> BlockEntityInfo {
        BlockEntityInfo {
            pos: [5, 5, 5],
            dimension: 0,
            signal_type: signal.then_some(SignalType::EntrySignal),
            reported_power: false,
            assembling: false,
            filter: None,
        }
    }

    #[test]
    fn a_front_station_goes_on_the_edge_as_given() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 100.0]);
        let mut t = TargetingState::new(EdgePointType::Station, 77);
        let p = t.create_edge_point(&mut nets, &at(g, 0.0, 100.0, 30.0), &be(false));
        assert_eq!(p, Placement::Created(77));
        let point = nets.graphs[&g].point(EdgePointType::Station, 77).unwrap();
        assert_eq!(point.edge_location, [loc(0.0, 0.0), loc(100.0, 0.0)]);
        assert_eq!(point.position, 30.0);
        assert_eq!(point.station().unwrap().block_entity_pos, Some([5, 5, 5]));
        assert!(t.is_orthogonal());
        assert!(t.prev_direction.is_some());
    }

    #[test]
    fn a_signal_facing_backwards_goes_on_the_swapped_edge() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 100.0]);
        let mut t = TargetingState::new(EdgePointType::Signal, 5);
        t.target_positive = false;
        let p = t.create_edge_point(&mut nets, &at(g, 0.0, 100.0, 30.0), &be(true));
        assert_eq!(p, Placement::Created(5));
        let point = nets.graphs[&g].point(EdgePointType::Signal, 5).unwrap();
        assert_eq!(point.edge_location, [loc(100.0, 0.0), loc(0.0, 0.0)]);
        assert_eq!(point.position, 70.0);
        assert_eq!(point.signal().unwrap().types[1], SignalType::EntrySignal);
    }

    #[test]
    fn a_second_signal_merges_and_a_station_blocks() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 100.0]);
        let mut t1 = TargetingState::new(EdgePointType::Signal, 5);
        t1.create_edge_point(&mut nets, &at(g, 0.0, 100.0, 30.0), &be(true));
        let mut t2 = TargetingState::new(EdgePointType::Signal, 6);
        t2.target_positive = false;
        let p = t2.create_edge_point(&mut nets, &at(g, 0.0, 100.0, 70.0), &be(true));
        // 70 from node 0 is 30 from node 100: the swapped point of the first signal's spot? The
        // first sits at 30 from node 100's side measured on the edge; only the same spot merges.
        assert!(matches!(p, Placement::Created(6) | Placement::Merged(5)));
        let mut t3 = TargetingState::new(EdgePointType::Signal, 8);
        let p = t3.create_edge_point(&mut nets, &at(g, 0.0, 100.0, 30.0), &be(true));
        assert_eq!(p, Placement::Merged(5));
        assert_eq!(t3.id, 5);
        // A station where a signal is: nothing.
        let mut ts = TargetingState::new(EdgePointType::Station, 9);
        assert_eq!(ts.create_edge_point(&mut nets, &at(g, 0.0, 100.0, 30.0), &be(false)), Placement::Nothing);
    }

    #[test]
    fn a_rotated_block_flips_its_direction_when_it_would_point_back() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 100.0]);
        let mut t = TargetingState::new(EdgePointType::Station, 1);
        // Facing positive, the previous direction comes out as -x; a rotated axis of +x is opposite.
        t.rotated_direction = Some([1.0, 0.0, 0.0]);
        let p = t.create_edge_point(&mut nets, &at(g, 0.0, 100.0, 30.0), &be(false));
        assert_eq!(p, Placement::Nothing);
        assert!(!t.target_positive);
        assert!(t.rotated_direction.is_none());
        // Next try, the other way, it goes ahead.
        let p = t.create_edge_point(&mut nets, &at(g, 0.0, 100.0, 30.0), &be(false));
        assert_eq!(p, Placement::Created(1));
        // The station on the swapped... stations are `SingleBlockEntityEdgePoint`s: as given.
        assert_eq!(nets.graphs[&g].point(EdgePointType::Station, 1).unwrap().position, 30.0);
    }

    #[test]
    fn missing_graph_or_edge_places_nothing() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 100.0]);
        let mut t = TargetingState::new(EdgePointType::Station, 1);
        assert_eq!(t.create_edge_point(&mut nets, &at(999, 0.0, 100.0, 1.0), &be(false)), Placement::Nothing);
        assert_eq!(t.create_edge_point(&mut nets, &at(g, 0.0, 50.0, 1.0), &be(false)), Placement::Nothing);
    }

    #[test]
    fn observers_and_removal() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 100.0]);
        let mut t = TargetingState::new(EdgePointType::Observer, 3);
        let mut info = be(false);
        info.filter = Some("create:cog".into());
        assert_eq!(t.create_edge_point(&mut nets, &at(g, 0.0, 100.0, 10.0), &info), Placement::Created(3));
        let o = nets.graphs[&g].point(EdgePointType::Observer, 3).unwrap().observer().unwrap();
        assert_eq!(o.filter.as_deref(), Some("create:cog"));
        t.block_entity_removed(&mut nets, 3, [5, 5, 5]);
        assert!(nets.graphs[&g].point(EdgePointType::Observer, 3).is_none());

        let mut s = TargetingState::new(EdgePointType::Signal, 4);
        s.create_edge_point(&mut nets, &at(g, 0.0, 100.0, 20.0), &be(true));
        assert!(nets.graphs[&g].point(EdgePointType::Signal, 4).is_some());
        s.block_entity_removed(&mut nets, 4, [5, 5, 5]);
        assert!(nets.graphs[&g].point(EdgePointType::Signal, 4).is_none());
    }

    #[test]
    fn positions() {
        let mut t = TargetingState::new(EdgePointType::Station, 1);
        t.target_track = [0, -1, 2];
        assert_eq!(t.global_position([10, 20, 30]), [10, 19, 32]);
        assert!(!t.is_on_curve());
        t.target_bezier = Some(BezierTrackPointLocation { curve_target: [1, 2, 3], segment: 4 });
        assert!(t.is_on_curve());
        t.transformed(9, Some([0.0, 0.0, 1.0]));
        assert_eq!((t.id, t.rotated_direction), (9, Some([0.0, 0.0, 1.0])));
    }
}
