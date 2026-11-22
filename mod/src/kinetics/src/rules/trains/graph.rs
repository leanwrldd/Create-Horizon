//! The pieces a track graph is made of (content/trains/graph/): `TrackNodeLocation` with Create's
//! rounding, `DiscoveredLocation`, `TrackNode`, `TrackEdge` (length, direction, position along it,
//! `canTravelTo`, edge crossings), `EdgeData` (the edge points on an edge, its signal group, its
//! crossings), `TrackEdgeIntersection`, `EdgePointType`, `TrackEdgePoint` and `EdgePointStorage`.
//!
//! Where Java keeps object references, ids stand in: a node is a [`NodeId`] (its `netId`), an edge
//! is the pair of its two node ids, an edge point is its `(type, uuid)`. A UUID is a `u128`; the
//! rules crate has no random source, so [`super::network::RailNetworks`] hands them out.
//!
//! Left out: NBT and packet reading and writing, `DimensionPalette`, `TrackGraphBounds`,
//! `TrackGraphVisualizer`, `TrackGraphSync` (the network layer reports changes as
//! [`super::network::RailEvent`]s instead).

use super::bezier::BezierConnection;
use super::network::TrackGraph;
use super::signal::SignalBoundary;
use super::station::GlobalStation;
use super::vec::{
    Plane, Vec3, add, aabb_intersects, aabb_intersects_segment, block_containing, dot, equal, floor, intersect_ranged,
    lerp, normalize, round, scale, sub,
};
use crate::Pos;
use std::collections::{BTreeMap, BTreeSet};

/// A `UUID`.
pub type Uuid = u128;
/// A `TrackNode.netId`.
pub type NodeId = i32;
/// `EdgeData.passiveGroup`: the all-zero UUID, an edge with no signal boundary on it.
pub const PASSIVE_GROUP: Uuid = 0;

/// `TrackNodeLocation`: a point on the half-block grid in a dimension, plus a vertical offset in
/// sixteenths from track tilt. The x and z are in half blocks, the y in half blocks of the floor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TrackNodeLocation {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// `ResourceKey<Level>`, as an index the caller keeps.
    pub dimension: u32,
    pub y_offset_pixels: i32,
}

impl TrackNodeLocation {
    /// `new TrackNodeLocation(x, y, z).in(dimension)`: x and z round to the nearest half block, y
    /// floors to a block and is doubled.
    pub fn new(v: Vec3, dimension: u32) -> TrackNodeLocation {
        TrackNodeLocation {
            x: floor(round(v[0] * 2.0) as f64),
            y: floor(v[1]) * 2,
            z: floor(round(v[2] * 2.0) as f64),
            dimension,
            y_offset_pixels: 0,
        }
    }

    pub fn with_y_offset(mut self, y_offset_pixels: i32) -> TrackNodeLocation {
        self.y_offset_pixels = y_offset_pixels;
        self
    }

    /// `getLocation()`.
    pub fn location(&self) -> Vec3 {
        [
            f64::from(self.x) / 2.0,
            f64::from(self.y) / 2.0 + f64::from(self.y_offset_pixels) / 16.0,
            f64::from(self.z) / 2.0,
        ]
    }

    /// `equalsIgnoreDim`.
    pub fn equals_ignore_dim(&self, other: &TrackNodeLocation) -> bool {
        self.x == other.x && self.y == other.y && self.z == other.z && self.y_offset_pixels == other.y_offset_pixels
    }

    /// `allAdjacent()`: the blocks within an eighth of a block of the node, the node's own tilt removed.
    pub fn all_adjacent(&self) -> BTreeSet<Pos> {
        let mut set = BTreeSet::new();
        let loc = self.location();
        let v = [loc[0], loc[1] - f64::from(self.y_offset_pixels) / 16.0, loc[2]];
        let step = 0.125;
        for x in [1.0, -1.0] {
            for y in [1.0, -1.0] {
                for z in [1.0, -1.0] {
                    set.insert(block_containing(add(v, [x * step, y * step, z * step])));
                }
            }
        }
        set
    }
}

/// `TrackNodeLocation.DiscoveredLocation`: a node location found while walking track, with what the
/// walk knows about it.
#[derive(Clone, Debug)]
pub struct DiscoveredLocation {
    pub loc: TrackNodeLocation,
    pub turn: Option<BezierConnection>,
    pub force_node: bool,
    pub direction: Option<Vec3>,
    pub normal: Option<Vec3>,
    pub material_a: Option<String>,
    pub material_b: Option<String>,
}

impl PartialEq for DiscoveredLocation {
    fn eq(&self, other: &Self) -> bool {
        self.loc == other.loc
    }
}

impl DiscoveredLocation {
    pub fn new(dimension: u32, v: Vec3) -> DiscoveredLocation {
        DiscoveredLocation {
            loc: TrackNodeLocation::new(v, dimension),
            turn: None,
            force_node: false,
            direction: None,
            normal: None,
            material_a: None,
            material_b: None,
        }
    }
    pub fn materials(mut self, a: Option<String>, b: Option<String>) -> Self {
        self.material_a = a;
        self.material_b = b;
        self
    }
    /// `viaTurn`: a turn forces a node.
    pub fn via_turn(mut self, turn: Option<BezierConnection>) -> Self {
        if turn.is_some() {
            self.force_node = true;
        }
        self.turn = turn;
        self
    }
    pub fn force_node(mut self) -> Self {
        self.force_node = true;
        self
    }
    pub fn with_normal(mut self, normal: Vec3) -> Self {
        self.normal = Some(normal);
        self
    }
    pub fn with_y_offset(mut self, y_offset_pixels: i32) -> Self {
        self.loc.y_offset_pixels = y_offset_pixels;
        self
    }
    pub fn with_direction(mut self, direction: Option<Vec3>) -> Self {
        self.direction = direction.map(normalize);
        self
    }
    pub fn connected_via_turn(&self) -> bool {
        self.turn.is_some()
    }
    pub fn different_materials(&self) -> bool {
        self.material_a != self.material_b
    }
    /// `notInLineWith(direction)`.
    pub fn not_in_line_with(&self, direction: Vec3) -> bool {
        match self.direction {
            Some(d) => dot(direction, d).max(dot(direction, scale(d, -1.0))) < f64::from(7.0_f32 / 8.0),
            None => false,
        }
    }
}

/// `TrackNode`.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackNode {
    pub location: TrackNodeLocation,
    pub net_id: NodeId,
    pub normal: Vec3,
}

// --- edge points -----------------------------------------------------------------------------

/// `EdgePointType`: the three registered kinds (`create:signal`, `create:station`, `create:observer`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EdgePointType {
    Signal,
    Station,
    Observer,
}

impl EdgePointType {
    pub const ALL: [EdgePointType; 3] = [EdgePointType::Signal, EdgePointType::Station, EdgePointType::Observer];

    pub fn id(self) -> &'static str {
        match self {
            EdgePointType::Signal => "create:signal",
            EdgePointType::Station => "create:station",
            EdgePointType::Observer => "create:observer",
        }
    }

    pub fn from_id(id: &str) -> Option<EdgePointType> {
        Self::ALL.into_iter().find(|t| t.id() == id)
    }

    /// `canMerge()`: only signals take a second block entity; stations and observers extend
    /// `SingleBlockEntityEdgePoint`, which says no.
    pub fn can_merge(self) -> bool {
        self == EdgePointType::Signal
    }

    /// `canCoexistWith(otherType, front)`: a signal sits with another signal; the others with nothing.
    pub fn can_coexist_with(self, other: EdgePointType) -> bool {
        self == EdgePointType::Signal && other == EdgePointType::Signal
    }
}

/// What differs between kinds of edge point. Observers are `Plain` until their own port gives them
/// state.
#[derive(Clone, Debug, PartialEq)]
pub enum PointData {
    Signal(SignalBoundary),
    Station(GlobalStation),
    Plain,
}

/// `TrackEdgePoint`: something at a position on an edge. `edge_location` and `position` are measured
/// from the second node of `edge_location` (the "primary" direction).
#[derive(Clone, Debug, PartialEq)]
pub struct TrackEdgePoint {
    pub id: Uuid,
    pub ty: EdgePointType,
    pub edge_location: [TrackNodeLocation; 2],
    pub position: f64,
    pub data: PointData,
}

impl TrackEdgePoint {
    pub fn new_signal(id: Uuid, edge_location: [TrackNodeLocation; 2], position: f64) -> TrackEdgePoint {
        TrackEdgePoint {
            id,
            ty: EdgePointType::Signal,
            edge_location,
            position,
            data: PointData::Signal(SignalBoundary::new()),
        }
    }

    /// `new GlobalStation()` placed on an edge.
    pub fn new_station(id: Uuid, edge_location: [TrackNodeLocation; 2], position: f64) -> TrackEdgePoint {
        TrackEdgePoint {
            id,
            ty: EdgePointType::Station,
            edge_location,
            position,
            data: PointData::Station(GlobalStation::new()),
        }
    }

    pub fn new_plain(ty: EdgePointType, id: Uuid, edge_location: [TrackNodeLocation; 2], position: f64) -> TrackEdgePoint {
        TrackEdgePoint { id, ty, edge_location, position, data: PointData::Plain }
    }

    /// `isPrimary(node1)`: `edgeLocation.second` is this edge's first node.
    pub fn is_primary(&self, node1: &TrackNodeLocation) -> bool {
        self.edge_location[1] == *node1
    }

    /// `getLocationOn(edge)`: how far along `edge` (from its first node) the point is.
    pub fn location_on(&self, edge: &TrackEdge) -> f64 {
        if self.is_primary(&edge.loc1) { edge.length() - self.position } else { self.position }
    }

    pub fn signal(&self) -> Option<&SignalBoundary> {
        match &self.data {
            PointData::Signal(s) => Some(s),
            _ => None,
        }
    }

    pub fn station(&self) -> Option<&GlobalStation> {
        match &self.data {
            PointData::Station(s) => Some(s),
            _ => None,
        }
    }

    pub fn station_mut(&mut self) -> Option<&mut GlobalStation> {
        match &mut self.data {
            PointData::Station(s) => Some(s),
            _ => None,
        }
    }

    /// `GlobalStation.canApproachFrom(side)`: the primary side only, and not while assembling.
    /// False for anything that is not a station.
    pub fn can_approach_from(&self, side: &TrackNodeLocation) -> bool {
        match &self.data {
            PointData::Station(s) => s.can_approach_from(self.is_primary(side)),
            _ => false,
        }
    }

    pub fn signal_mut(&mut self) -> Option<&mut SignalBoundary> {
        match &mut self.data {
            PointData::Signal(s) => Some(s),
            _ => None,
        }
    }

    /// `canNavigateVia(side)`: a signal needs a block entity on that side; a station is closed while
    /// assembling.
    pub fn can_navigate_via(&self, side: &TrackNodeLocation) -> bool {
        match &self.data {
            PointData::Signal(s) => s.can_navigate_via(self.is_primary(side)),
            PointData::Station(s) => s.can_navigate_via(),
            PointData::Plain => true,
        }
    }
}

/// `EdgePointStorage`: the points of one graph by type and id.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EdgePointStorage {
    points: BTreeMap<EdgePointType, BTreeMap<Uuid, TrackEdgePoint>>,
}

impl EdgePointStorage {
    pub fn put(&mut self, point: TrackEdgePoint) {
        self.points.entry(point.ty).or_default().insert(point.id, point);
    }
    pub fn get(&self, ty: EdgePointType, id: Uuid) -> Option<&TrackEdgePoint> {
        self.points.get(&ty)?.get(&id)
    }
    pub fn get_mut(&mut self, ty: EdgePointType, id: Uuid) -> Option<&mut TrackEdgePoint> {
        self.points.get_mut(&ty)?.get_mut(&id)
    }
    pub fn remove(&mut self, ty: EdgePointType, id: Uuid) -> Option<TrackEdgePoint> {
        self.points.get_mut(&ty)?.remove(&id)
    }
    pub fn values(&self, ty: EdgePointType) -> Vec<&TrackEdgePoint> {
        self.points.get(&ty).map(|m| m.values().collect()).unwrap_or_default()
    }
    pub fn ids(&self, ty: EdgePointType) -> Vec<Uuid> {
        self.points.get(&ty).map(|m| m.keys().copied().collect()).unwrap_or_default()
    }
    pub fn is_empty(&self) -> bool {
        self.points.values().all(|m| m.is_empty())
    }
    /// `transferAll(target, other)`: everything moves to `other`.
    pub fn transfer_all(&mut self, other: &mut EdgePointStorage) {
        for (ty, map) in std::mem::take(&mut self.points) {
            other.points.entry(ty).or_default().extend(map);
        }
    }
}

// --- edge data -------------------------------------------------------------------------------

/// `TrackEdgeIntersection`: where this edge crosses another one.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackEdgeIntersection {
    pub id: Uuid,
    pub location: f64,
    pub target: [TrackNodeLocation; 2],
    pub target_location: f64,
    pub group_id: Option<Uuid>,
}

impl TrackEdgeIntersection {
    /// `isNear(location)`: within 1/32.
    pub fn is_near(&self, location: f64) -> bool {
        (location - self.location).abs() < f64::from(1.0_f32 / 32.0)
    }

    /// `targets(a, b)`: crosses the edge between these two nodes, either way round.
    pub fn targets(&self, a: &TrackNodeLocation, b: &TrackNodeLocation) -> bool {
        (*a == self.target[0] && *b == self.target[1]) || (*a == self.target[1] && *b == self.target[0])
    }
}

/// An edge point on an edge, as the edge's own list holds it: which one, and where along this edge
/// (`getLocationOn(edge)`, fixed when it was added).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgePointRef {
    pub ty: EdgePointType,
    pub id: Uuid,
    pub location_on: f64,
}

/// `EdgeData`: the edge points sorted by position, the crossings, and the signal group of the whole
/// edge when it has no signal on it. `single_signal_group` is `None` once a signal boundary is on the
/// edge (`hasSignalBoundaries`), [`PASSIVE_GROUP`] while no group has been worked out.
#[derive(Clone, Debug, PartialEq)]
pub struct EdgeData {
    pub single_signal_group: Option<Uuid>,
    pub points: Vec<EdgePointRef>,
    pub intersections: Vec<TrackEdgeIntersection>,
}

impl Default for EdgeData {
    fn default() -> Self {
        EdgeData { single_signal_group: Some(PASSIVE_GROUP), points: Vec::new(), intersections: Vec::new() }
    }
}

impl EdgeData {
    pub fn has_signal_boundaries(&self) -> bool {
        self.single_signal_group.is_none()
    }
    pub fn has_points(&self) -> bool {
        !self.points.is_empty()
    }
    pub fn has_intersections(&self) -> bool {
        !self.intersections.is_empty()
    }

    /// `getEffectiveEdgeGroupId(graph)`: the passive group stands for the graph's own id.
    pub fn effective_edge_group_id(&self, graph_id: Uuid) -> Option<Uuid> {
        match self.single_signal_group {
            None => None,
            Some(g) if g == PASSIVE_GROUP => Some(graph_id),
            Some(g) => Some(g),
        }
    }

    /// `next(type, minPosition)`: the first point of the type strictly after `min_position`.
    pub fn next_of(&self, ty: EdgePointType, min_position: f64) -> Option<EdgePointRef> {
        self.points.iter().find(|p| p.ty == ty && p.location_on > min_position).copied()
    }

    /// `next(minPosition)`: the first point of any type.
    pub fn next(&self, min_position: f64) -> Option<EdgePointRef> {
        self.points.iter().find(|p| p.location_on > min_position).copied()
    }

    /// `get(type, exactPosition)`.
    pub fn get(&self, ty: EdgePointType, exact_position: f64) -> Option<EdgePointRef> {
        let next = self.next_of(ty, exact_position - f64::from(0.5_f32))?;
        if equal(next.location_on, exact_position) { Some(next) } else { None }
    }

    /// `getGroupAtPosition(graph, position)`: the signal group the stretch of this edge at
    /// `position` belongs to. `node1` and `node2` are the edge's end locations.
    pub fn group_at_position(
        &self,
        graph_id: Uuid,
        points: &EdgePointStorage,
        node1: &TrackNodeLocation,
        node2: &TrackNodeLocation,
        position: f64,
    ) -> Option<Uuid> {
        if !self.has_signal_boundaries() {
            return self.effective_edge_group_id(graph_id);
        }
        let first = self.next_of(EdgePointType::Signal, 0.0)?;
        let sb = points.get(EdgePointType::Signal, first.id)?;
        let mut current = sb.signal()?.group(sb.is_primary(node1));

        for r in &self.points {
            if r.ty != EdgePointType::Signal {
                continue;
            }
            let Some(p) = points.get(r.ty, r.id) else { continue };
            let Some(s) = p.signal() else { continue };
            if r.location_on >= position {
                return current;
            }
            current = s.group(p.is_primary(node2));
        }
        current
    }

    /// The insertion `addPoint` does: sorted by position, after any equal one.
    pub fn insert_point(&mut self, r: EdgePointRef) {
        let i = self.points.iter().position(|p| p.location_on > r.location_on).unwrap_or(self.points.len());
        self.points.insert(i, r);
    }
}

// --- edges -----------------------------------------------------------------------------------

/// `TrackEdge`: a directed edge between two nodes, straight or a curve. Java holds the node objects;
/// here the ids, the two locations (for length and position) and the first node's normal.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackEdge {
    pub node1: NodeId,
    pub node2: NodeId,
    pub loc1: TrackNodeLocation,
    pub loc2: TrackNodeLocation,
    /// `node1.getNormal()`.
    pub normal1: Vec3,
    pub turn: Option<BezierConnection>,
    pub edge_data: EdgeData,
    pub inter_dimensional: bool,
    pub material: Option<String>,
    /// Stands in for Java's object identity: unique per edge object a network creates.
    pub serial: u64,
}

impl TrackEdge {
    /// `new TrackEdge(node1, node2, turn, material)`.
    pub fn new(
        node1: &TrackNode,
        node2: &TrackNode,
        turn: Option<BezierConnection>,
        material: Option<String>,
        serial: u64,
    ) -> TrackEdge {
        TrackEdge {
            node1: node1.net_id,
            node2: node2.net_id,
            loc1: node1.location,
            loc2: node2.location,
            normal1: node1.normal,
            turn,
            edge_data: EdgeData::default(),
            inter_dimensional: node1.location.dimension != node2.location.dimension,
            material,
            serial,
        }
    }

    pub fn is_turn(&self) -> bool {
        self.turn.is_some()
    }
    pub fn is_inter_dimensional(&self) -> bool {
        self.inter_dimensional
    }

    /// `getLength()`: 0 between dimensions, the curve's length for a turn, else the straight distance.
    pub fn length(&self) -> f64 {
        if self.inter_dimensional {
            0.0
        } else if let Some(turn) = &self.turn {
            turn.length()
        } else {
            super::vec::distance_to(self.loc1.location(), self.loc2.location())
        }
    }

    /// `getDirection(fromFirst)`.
    pub fn direction(&self, from_first: bool) -> Vec3 {
        normalize(sub(
            self.position(None, if from_first { 0.25 } else { 1.0 }),
            self.position(None, if from_first { 0.0 } else { 0.75 }),
        ))
    }

    /// `getDirectionAt(t)`, `t` a distance along the edge.
    pub fn direction_at(&self, t: f64) -> Vec3 {
        let length = self.length();
        let step = f64::from(0.5_f32) / length;
        let t = t / length;
        let ahead = self.position(None, (t + step).min(1.0));
        let behind = self.position(None, (t - step).max(0.0));
        normalize(sub(ahead, behind))
    }

    /// `canTravelTo(other)`: a train goes from here onto `other` when `other` leaves within about 29
    /// degrees of how this one arrives (dot over 7/8); any edge across dimensions is allowed.
    pub fn can_travel_to(&self, other: &TrackEdge) -> bool {
        if self.inter_dimensional || other.inter_dimensional {
            return true;
        }
        let new_direction = other.direction(true);
        dot(self.direction(false), new_direction) > f64::from(7.0_f32 / 8.0)
    }

    /// `incrementT(currentT, distance)`: past 5 blocks the curve's own stepping is skipped.
    pub fn increment_t(&self, current_t: f64, distance: f64) -> f64 {
        let too_far = distance.abs() > 5.0;
        let length = self.length();
        let distance = distance / if length == 0.0 { 1.0 } else { length };
        match &self.turn {
            Some(turn) if !too_far => turn.increment_t(current_t, distance),
            _ => current_t + distance,
        }
    }

    /// `getPosition(graph, t)`: `t` runs 0 to 1 along the edge.
    pub fn position(&self, graph: Option<&TrackGraph>, t: f64) -> Vec3 {
        if let Some(turn) = &self.turn {
            return turn.position(t.clamp(0.0, 1.0));
        }
        if let Some(g) = graph {
            if self.loc1.y_offset_pixels != 0 || self.loc2.y_offset_pixels != 0 {
                if let Some(p) = self.position_smoothed(g, t) {
                    return p;
                }
            }
        }
        lerp(t as f32, self.loc1.location(), self.loc2.location())
    }

    /// `getNormal(graph, t)`.
    pub fn normal(&self, graph: Option<&TrackGraph>, t: f64) -> Vec3 {
        if let Some(turn) = &self.turn {
            return turn.normal(t.clamp(0.0, 1.0));
        }
        if let Some(g) = graph {
            if self.loc1.y_offset_pixels != 0 || self.loc2.y_offset_pixels != 0 {
                if let Some(n) = self.normal_smoothed(g, t) {
                    return n;
                }
            }
        }
        self.normal1
    }

    /// `getPositionSmoothed`: between the start of the curve leaving each end.
    pub fn position_smoothed(&self, graph: &TrackGraph, t: f64) -> Option<Vec3> {
        let mut a = None;
        let mut b = None;
        for e in graph.connections_from(self.node1)?.values() {
            if e.is_turn() {
                a = Some(e.position(Some(graph), 0.0));
            }
        }
        for e in graph.connections_from(self.node2)?.values() {
            if e.is_turn() {
                b = Some(e.position(Some(graph), 0.0));
            }
        }
        Some(lerp(t as f32, a?, b?))
    }

    /// `getNormalSmoothed`.
    pub fn normal_smoothed(&self, graph: &TrackGraph, _t: f64) -> Option<Vec3> {
        let mut a = None;
        let mut b = None;
        for e in graph.connections_from(self.node1)?.values() {
            if e.is_turn() {
                a = Some(e.normal(Some(graph), 0.0));
            }
        }
        for e in graph.connections_from(self.node2)?.values() {
            if e.is_turn() {
                b = Some(e.normal(Some(graph), 0.0));
            }
        }
        Some(lerp(0.5, a?, b?))
    }

    /// `getIntersection(node1, node2, other, other1, other2)`: where the two edges cross in the
    /// horizontal plane, as pairs of distances along this edge and along `other`. Nothing across
    /// dimensions or between edges at different heights.
    pub fn intersections_with(&self, other: &TrackEdge) -> Vec<[f64; 2]> {
        let v1 = self.loc1.location();
        let v2 = self.loc2.location();
        let w1 = other.loc1.location();
        let w2 = other.loc2.location();

        if self.is_inter_dimensional() || other.is_inter_dimensional() {
            return Vec::new();
        }
        if v1[1] != v2[1] || v1[1] != w1[1] || v1[1] != w2[1] {
            return Vec::new();
        }

        let Some(turn) = &self.turn else {
            if other.turn.is_none() {
                return intersect_ranged(v1, w1, v2, w2, Plane::Y).into_iter().collect();
            }
            return other.intersections_with(self).into_iter().map(|a| [a[1], a[0]]).collect();
        };

        let bb = turn.bounds();
        let seg_count = turn.segment_count();

        let Some(other_turn) = &other.turn else {
            if !aabb_intersects_segment(&bb, w1, w2) {
                return Vec::new();
            }
            let mut seg1 = v1;
            let mut t = 0.0;
            let mut out = Vec::new();
            for _ in 0..seg_count {
                let t_offset = t;
                t += 0.5;
                let seg2 = self.position(None, t / self.length());
                let hit = intersect_ranged(seg1, w1, seg2, w2, Plane::Y);
                seg1 = seg2;
                if let Some(mut h) = hit {
                    h[0] += t_offset;
                    out.push(h);
                }
            }
            return out;
        };

        if !aabb_intersects(&bb, &other_turn.bounds()) {
            return Vec::new();
        }
        let mut seg1 = v1;
        let mut t = 0.0;
        let mut out = Vec::new();
        for _ in 0..seg_count {
            let t_offset = t;
            t += 0.5;
            let seg2 = self.position(None, t / self.length());

            let mut other_seg1 = w1;
            let mut u = 0.0;
            for _ in 0..other_turn.segment_count() {
                let u_offset = u;
                u += 0.5;
                let other_seg2 = other.position(None, u / other.length());
                let hit = intersect_ranged(seg1, other_seg1, seg2, other_seg2, Plane::Y);
                other_seg1 = other_seg2;
                if let Some(mut h) = hit {
                    h[0] += t_offset;
                    h[1] += u_offset;
                    out.push(h);
                }
            }
            seg1 = seg2;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: NodeId, v: Vec3) -> TrackNode {
        TrackNode { location: TrackNodeLocation::new(v, 0), net_id: id, normal: [0.0, 1.0, 0.0] }
    }

    #[test]
    fn node_locations_round_to_half_blocks() {
        let l = TrackNodeLocation::new([10.5, 64.0, -3.0], 0);
        assert_eq!((l.x, l.y, l.z), (21, 128, -6));
        // x and z round to the nearest half block; y floors to a block.
        let l = TrackNodeLocation::new([10.26, 64.9, 0.74], 0);
        assert_eq!((l.x, l.y, l.z), (21, 128, 1));
        assert_eq!(l.location(), [10.5, 64.0, 0.5]);
        // The tilt offset is in sixteenths.
        let t = TrackNodeLocation::new([0.0, 64.0, 0.0], 0).with_y_offset(8);
        assert_eq!(t.location()[1], 64.5);
        // Dimension and tilt are part of the identity.
        assert_ne!(TrackNodeLocation::new([0.0; 3], 0), TrackNodeLocation::new([0.0; 3], 1));
        assert!(TrackNodeLocation::new([0.0; 3], 0).equals_ignore_dim(&TrackNodeLocation::new([0.0; 3], 1)));
        assert!(!t.equals_ignore_dim(&TrackNodeLocation::new([0.0, 64.0, 0.0], 0)));
    }

    #[test]
    fn adjacent_blocks_of_a_node_on_a_corner() {
        // A node at a block's edge touches the blocks either side.
        let l = TrackNodeLocation::new([1.0, 64.0, 0.5], 0);
        let adj = l.all_adjacent();
        assert!(adj.contains(&[0, 64, 0]) && adj.contains(&[1, 64, 0]));
        assert!(adj.contains(&[0, 63, 0]));
        assert_eq!(adj.len(), 4);
    }

    #[test]
    fn straight_edge_length_and_direction() {
        let (a, b) = (node(1, [0.0, 64.0, 0.0]), node(2, [4.0, 64.0, 0.0]));
        let e = TrackEdge::new(&a, &b, None, None, 1);
        assert_eq!(e.length(), 4.0);
        assert_eq!(e.direction(true), [1.0, 0.0, 0.0]);
        assert_eq!(e.position(None, 0.5), [2.0, 64.0, 0.0]);
        assert_eq!(e.direction_at(2.0), [1.0, 0.0, 0.0]);
        assert_eq!(e.increment_t(0.0, 2.0), 0.5);
        let back = TrackEdge::new(&b, &a, None, None, 2);
        assert!(!e.can_travel_to(&back));
        let c = node(3, [8.0, 64.0, 0.0]);
        let next = TrackEdge::new(&b, &c, None, None, 3);
        assert!(e.can_travel_to(&next));
        // A sharp bend refuses.
        let d = node(4, [4.0, 64.0, 4.0]);
        let turn = TrackEdge::new(&b, &d, None, None, 4);
        assert!(!e.can_travel_to(&turn));
    }

    #[test]
    fn inter_dimensional_edges_have_no_length_and_connect_to_anything() {
        let a = node(1, [0.0; 3]);
        let mut b = node(2, [10.0, 0.0, 0.0]);
        b.location.dimension = 1;
        let e = TrackEdge::new(&a, &b, None, None, 1);
        assert!(e.is_inter_dimensional());
        assert_eq!(e.length(), 0.0);
        assert_eq!(e.increment_t(0.0, 3.0), 3.0);
        let c = node(3, [0.0, 10.0, 7.0]);
        let other = TrackEdge::new(&a, &c, None, None, 2);
        assert!(e.can_travel_to(&other) && other.can_travel_to(&e));
    }

    #[test]
    fn crossing_straight_edges_report_distances() {
        let e = TrackEdge::new(&node(1, [0.0, 64.0, 0.0]), &node(2, [4.0, 64.0, 0.0]), None, None, 1);
        let f = TrackEdge::new(&node(3, [2.0, 64.0, -1.0]), &node(4, [2.0, 64.0, 1.0]), None, None, 2);
        let hits = e.intersections_with(&f);
        assert_eq!(hits.len(), 1);
        assert!((hits[0][0] - 2.0).abs() < 1e-9 && (hits[0][1] - 1.0).abs() < 1e-9);
        // Different heights never cross.
        let g = TrackEdge::new(&node(5, [2.0, 66.0, -1.0]), &node(6, [2.0, 66.0, 1.0]), None, None, 3);
        assert!(e.intersections_with(&g).is_empty());
    }

    #[test]
    fn edge_points_sort_by_position_and_are_found() {
        let mut d = EdgeData::default();
        assert!(!d.has_signal_boundaries());
        d.insert_point(EdgePointRef { ty: EdgePointType::Signal, id: 2, location_on: 5.0 });
        d.insert_point(EdgePointRef { ty: EdgePointType::Station, id: 1, location_on: 2.0 });
        d.insert_point(EdgePointRef { ty: EdgePointType::Signal, id: 3, location_on: 7.0 });
        assert_eq!(d.points.iter().map(|p| p.id).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert_eq!(d.next(0.0).unwrap().id, 1);
        assert_eq!(d.next_of(EdgePointType::Signal, 0.0).unwrap().id, 2);
        assert_eq!(d.next_of(EdgePointType::Signal, 5.0).unwrap().id, 3);
        assert!(d.next_of(EdgePointType::Signal, 7.0).is_none());
        assert_eq!(d.get(EdgePointType::Signal, 7.0).unwrap().id, 3);
        assert!(d.get(EdgePointType::Signal, 6.0).is_none());
        assert_eq!(d.effective_edge_group_id(9), Some(9));
        d.single_signal_group = None;
        assert_eq!(d.effective_edge_group_id(9), None);
    }

    #[test]
    fn point_types() {
        assert_eq!(EdgePointType::from_id("create:station"), Some(EdgePointType::Station));
        assert!(EdgePointType::Signal.can_merge() && !EdgePointType::Station.can_merge());
        assert!(EdgePointType::Signal.can_coexist_with(EdgePointType::Signal));
        assert!(!EdgePointType::Signal.can_coexist_with(EdgePointType::Station));
        let loc = |x| TrackNodeLocation::new([x, 0.0, 0.0], 0);
        let e = TrackEdge::new(
            &TrackNode { location: loc(0.0), net_id: 1, normal: [0.0, 1.0, 0.0] },
            &TrackNode { location: loc(10.0), net_id: 2, normal: [0.0, 1.0, 0.0] },
            None,
            None,
            1,
        );
        // Placed from the far side: 3 from the second location.
        let p = TrackEdgePoint::new_plain(EdgePointType::Station, 1, [loc(0.0), loc(10.0)], 3.0);
        assert!(!p.is_primary(&loc(0.0)) && p.is_primary(&loc(10.0)));
        assert_eq!(p.location_on(&e), 3.0);
        let reverse = TrackEdge::new(
            &TrackNode { location: loc(10.0), net_id: 2, normal: [0.0, 1.0, 0.0] },
            &TrackNode { location: loc(0.0), net_id: 1, normal: [0.0, 1.0, 0.0] },
            None,
            None,
            2,
        );
        assert_eq!(p.location_on(&reverse), 7.0);
    }
}
