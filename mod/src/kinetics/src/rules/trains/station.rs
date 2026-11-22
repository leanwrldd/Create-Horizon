//! Train stations: `GlobalStation` (content/trains/station/GlobalStation.java, the edge point a
//! station block puts on the graph) and the pure rules of `StationBlockEntity`'s assembly area.
//!
//! A station keeps no train object, only the id of the nearest train (Java: a `WeakReference`).
//! What it asks of a train it asks of a [`TrainStates`], which the code that owns the trains
//! implements: whether the train still exists, which station it stands at, whether it is navigating
//! and how far it still has to go.
//!
//! Assembly (`StationBlockEntity.refreshAssemblyInfo`, `isValidBogeyOffset`, `getAssemblyDirection`,
//! `tryEnterAssemblyMode`, the first half of `assemble`): [`scan_assembly`] walks the assembly track
//! and lists the bogeys on it, [`bogey_point_offsets`] turns bogey places into wheel point offsets
//! along the track, [`place_travelling_points`] puts the wheel points on the graph, and every error
//! the assembler reports is an [`AssemblyError`] with Create's lang key.
//!
//! Left out: NBT and packet reading and writing, `runMailTransfer` and the package ports' offline
//! buffers (items), the second half of `assemble` (contraption assembly, carriages, `Train`), the
//! station block's states, `StationEditPacket`, `TrackTargetingBehaviour`.

use super::graph::{TrackNodeLocation, Uuid};
use super::network::TrackGraph;
use super::travelling_point::TravellingPoint;
use super::vec::{Vec3, add, dot, equal, normalize, scale};
use crate::Pos;
use std::collections::BTreeMap;

/// `CTrains.maxAssemblyLength` (default 128, range 5 to 512).
pub const MAX_ASSEMBLY_LENGTH: i32 = 128;
/// `CTrains.maxBogeyCount` (default 20, range 1 to 200).
pub const MAX_BOGEY_COUNT: usize = 20;
/// `GlobalStation()`'s default name.
pub const DEFAULT_STATION_NAME: &str = "Track Station";
/// How far (blocks) a train still has to go for `getImminentTrain` to count it.
pub const IMMINENT_DISTANCE: f64 = 30.0;

/// What a station needs to know about trains, which are not in this module.
pub trait TrainStates {
    /// Is the train still there (Java's weak reference has not been cleared)?
    fn exists(&self, train: Uuid) -> bool;
    /// `train.getCurrentStation()`'s id.
    fn current_station(&self, train: Uuid) -> Option<Uuid>;
    /// `train.navigation.isActive()`: it has a destination.
    fn navigation_active(&self, train: Uuid) -> bool;
    /// `train.navigation.distanceToDestination`.
    fn distance_to_destination(&self, train: Uuid) -> f64;
}

/// `GlobalPackagePort` without its offline item buffer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PackagePort {
    pub address: String,
    pub primed: bool,
}

/// `GlobalStation`.
#[derive(Clone, Debug, PartialEq)]
pub struct GlobalStation {
    pub name: String,
    /// `nearestTrain`.
    pub nearest_train: Option<Uuid>,
    pub assembling: bool,
    /// `SingleBlockEntityEdgePoint.blockEntityPos` and `blockEntityDimension` (an index the caller keeps).
    pub block_entity_pos: Option<Pos>,
    pub block_entity_dimension: Option<u32>,
    /// `connectedPorts`.
    pub connected_ports: BTreeMap<Pos, PackagePort>,
}

impl Default for GlobalStation {
    fn default() -> Self {
        Self::new()
    }
}

impl GlobalStation {
    pub fn new() -> GlobalStation {
        GlobalStation {
            name: DEFAULT_STATION_NAME.to_string(),
            nearest_train: None,
            assembling: false,
            block_entity_pos: None,
            block_entity_dimension: None,
            connected_ports: BTreeMap::new(),
        }
    }

    /// `blockEntityAdded(blockEntity, front)`: remembers the block entity's place, and whether its
    /// state says assembling.
    pub fn block_entity_added(&mut self, pos: Pos, dimension: u32, assembling_state: bool) {
        self.block_entity_pos = Some(pos);
        self.block_entity_dimension = Some(dimension);
        self.assembling = assembling_state;
    }

    /// `canApproachFrom(side)` with `isPrimary(side)` worked out: only the primary side, and not
    /// while assembling.
    pub fn can_approach_from(&self, is_primary: bool) -> bool {
        is_primary && !self.assembling
    }

    /// `canNavigateVia(side)`: `super` says yes; a station being assembled is closed.
    pub fn can_navigate_via(&self) -> bool {
        !self.assembling
    }

    /// `reserveFor(train)`: the train becomes the nearest unless another is nearer. `distance` is the
    /// train's own `navigation.distanceToDestination`.
    pub fn reserve_for(&mut self, train: Uuid, distance: f64, trains: &dyn TrainStates) {
        match self.nearest_train(trains) {
            Some(nearest) if trains.distance_to_destination(nearest) <= distance => {}
            _ => self.nearest_train = Some(train),
        }
    }

    /// `cancelReservation(train)`.
    pub fn cancel_reservation(&mut self, train: Uuid) {
        if self.nearest_train == Some(train) {
            self.nearest_train = None;
        }
    }

    /// `trainDeparted(train)`.
    pub fn train_departed(&mut self, train: Uuid) {
        self.cancel_reservation(train);
    }

    /// `getNearestTrain()`: a train that has gone is none (Java's cleared weak reference).
    pub fn nearest_train(&self, trains: &dyn TrainStates) -> Option<Uuid> {
        self.nearest_train.filter(|t| trains.exists(*t))
    }

    /// `getPresentTrain()`: the nearest train, if it stands at this station. `id` is the station's
    /// own edge point id.
    pub fn present_train(&self, id: Uuid, trains: &dyn TrainStates) -> Option<Uuid> {
        let nearest = self.nearest_train(trains)?;
        if trains.current_station(nearest) != Some(id) {
            return None;
        }
        Some(nearest)
    }

    /// `getImminentTrain()`: the nearest train if it is here, or navigating and within 30 blocks.
    pub fn imminent_train(&self, id: Uuid, trains: &dyn TrainStates) -> Option<Uuid> {
        let nearest = self.nearest_train(trains)?;
        if trains.current_station(nearest) == Some(id) {
            return Some(nearest);
        }
        if !trains.navigation_active(nearest) {
            return None;
        }
        if trains.distance_to_destination(nearest) > IMMINENT_DISTANCE {
            return None;
        }
        Some(nearest)
    }
}

// --- assembly --------------------------------------------------------------------------------

/// What can go wrong assembling a train, with Create's lang keys (`train_assembly.*`).
#[derive(Clone, Debug, PartialEq)]
pub enum AssemblyError {
    /// The first bogey is not on the station's own block.
    FrontmostBogeyAtStation,
    /// Bogeys `a` and `b` (1-based, as the lang string shows them) are less than 3 blocks apart.
    BogeysTooClose { a: usize, b: usize },
    NoBogeys,
    /// A carriage (1-based) had nothing attached to its bogey.
    NothingAttached { carriage: usize },
    NotConnectedInOrder,
    SingleBogeyCarriage,
    NoControls,
}

impl AssemblyError {
    /// The translation key.
    pub fn key(&self) -> &'static str {
        match self {
            AssemblyError::FrontmostBogeyAtStation => "train_assembly.frontmost_bogey_at_station",
            AssemblyError::BogeysTooClose { .. } => "train_assembly.bogeys_too_close",
            AssemblyError::NoBogeys => "train_assembly.no_bogeys",
            AssemblyError::NothingAttached { .. } => "train_assembly.nothing_attached",
            AssemblyError::NotConnectedInOrder => "train_assembly.not_connected_in_order",
            AssemblyError::SingleBogeyCarriage => "train_assembly.single_bogey_carriage",
            AssemblyError::NoControls => "train_assembly.no_controls",
        }
    }

    /// The numbers the lang string takes (`bogeys_too_close` takes two, `nothing_attached` one).
    pub fn args(&self) -> Vec<usize> {
        match self {
            AssemblyError::BogeysTooClose { a, b } => vec![*a, *b],
            AssemblyError::NothingAttached { carriage } => vec![*carriage],
            _ => Vec::new(),
        }
    }
}

/// What `refreshAssemblyInfo` found: how long the assembly track is, and the bogeys along it.
#[derive(Clone, Debug, PartialEq)]
pub struct AssemblyScan<B> {
    /// `assemblyLength`.
    pub length: i32,
    /// `bogeyLocations`: the step along the track of each bogey (only the ones found).
    pub bogey_locations: Vec<i32>,
    /// `bogeyTypes`.
    pub bogey_types: Vec<B>,
    /// `upsideDownBogeys`.
    pub upside_down: Vec<bool>,
}

impl<B> AssemblyScan<B> {
    /// `bogeyCount`.
    pub fn bogey_count(&self) -> usize {
        self.bogey_locations.len()
    }

    /// `isValidBogeyOffset(i)`: a bogey may go at the station's block, or at least 3 blocks along and
    /// not within 2 of another bogey.
    pub fn is_valid_bogey_offset(&self, i: i32) -> bool {
        is_valid_bogey_offset(i, &self.bogey_locations)
    }
}

/// `isValidBogeyOffset(i)`, `bogey_locations` holding the bogeys found so far.
pub fn is_valid_bogey_offset(i: i32, bogey_locations: &[i32]) -> bool {
    if (i < 3 || bogey_locations.is_empty()) && i != 0 {
        return false;
    }
    for j in bogey_locations {
        if i >= j - 2 && i <= j + 2 {
            return false;
        }
    }
    true
}

/// `refreshAssemblyInfo`'s walk down the assembly track, one block at a time from the one next to
/// the station.
///
/// `track_matches(i)`: is the block at step `i` the same track (`trackEquals`). `bogey_at(i,
/// upside_down_lookup)`: the bogey block at the step, looked up above the track (`false`) or below
/// it (`true`), with whether its state is upside down. Where the walk ends: at `max_length - 1`
/// steps, or at the first step that is not the same track (then `max(0, i - 1)`). Only `max_bogeys`
/// bogeys are kept.
pub fn scan_assembly<B>(
    max_length: i32,
    max_bogeys: usize,
    mut track_matches: impl FnMut(i32) -> bool,
    mut bogey_at: impl FnMut(i32, bool) -> Option<(B, bool)>,
) -> AssemblyScan<B> {
    let mut scan = AssemblyScan { length: 0, bogey_locations: Vec::new(), bogey_types: Vec::new(), upside_down: Vec::new() };
    for i in 0..max_length {
        if i == max_length - 1 {
            scan.length = i;
            break;
        }
        if !track_matches(i) {
            scan.length = (i - 1).max(0);
            break;
        }
        if scan.bogey_locations.len() < max_bogeys {
            if let Some((b, _)) = bogey_at(i, false).filter(|(_, ud)| !*ud) {
                scan.bogey_types.push(b);
                scan.bogey_locations.push(i);
                scan.upside_down.push(false);
            } else if let Some((b, _)) = bogey_at(i, true).filter(|(_, ud)| *ud) {
                scan.bogey_types.push(b);
                scan.bogey_locations.push(i);
                scan.upside_down.push(true);
            }
        }
    }
    scan
}

/// `tryEnterAssemblyMode`'s axis test: the track's first axis must lie along one block axis (at most
/// one component is not exactly 0).
pub fn track_axis_is_aligned(axis: Vec3) -> bool {
    axis.iter().filter(|c| **c != 0.0).count() <= 1
}

/// `net.minecraft.core.Direction`, in the order of `Direction.values()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

impl Direction {
    pub const ALL: [Direction; 6] =
        [Direction::Down, Direction::Up, Direction::North, Direction::South, Direction::West, Direction::East];

    /// `getNormal()`.
    pub fn normal(self) -> [i32; 3] {
        match self {
            Direction::Down => [0, -1, 0],
            Direction::Up => [0, 1, 0],
            Direction::North => [0, 0, -1],
            Direction::South => [0, 0, 1],
            Direction::West => [-1, 0, 0],
            Direction::East => [1, 0, 0],
        }
    }

    /// `Direction.getNearest(x, y, z)`: the first direction with the largest dot product.
    pub fn nearest(x: f64, y: f64, z: f64) -> Direction {
        let mut best = Direction::North;
        let mut best_dot = f64::from(f32::MIN);
        for d in Direction::ALL {
            let n = d.normal();
            let dot = x * f64::from(n[0]) + y * f64::from(n[1]) + z * f64::from(n[2]);
            if dot > best_dot {
                best_dot = dot;
                best = d;
            }
        }
        best
    }
}

/// `getAssemblyDirection()`: the track's first axis, normalised, pointed the way the edge point
/// faces (`axisDirection` is `getTargetDirection().getStep()`, 1 or -1), as the nearest direction.
pub fn assembly_direction(track_axis: Vec3, axis_direction_step: i32) -> Direction {
    let axis = scale(normalize(track_axis), f64::from(axis_direction_step));
    Direction::nearest(axis[0], axis[1], axis[2])
}

/// The offsets along the assembly track (in blocks from the station's block) of each wheel point,
/// two per bogey: `loc + .5 -/+ spacing / 2`. `wheel_spacing(i)` is bogey `i`'s `getWheelPointSpacing()`.
/// Fails when the first bogey is not at the station (`bogeyLocations[0] != 0`) or two bogeys are
/// closer than 3 blocks (the check `assemble` makes, starting from -100).
pub fn bogey_point_offsets(bogey_locations: &[i32], wheel_spacing: impl Fn(usize) -> f64) -> Result<Vec<f64>, AssemblyError> {
    if bogey_locations.first() != Some(&0) {
        return Err(AssemblyError::FrontmostBogeyAtStation);
    }
    let mut offsets = Vec::new();
    let mut previous = -100;
    for (i, loc) in bogey_locations.iter().enumerate() {
        if loc - previous < 3 {
            return Err(AssemblyError::BogeysTooClose { a: i, b: i + 1 });
        }
        let size = wheel_spacing(i);
        offsets.push(f64::from(*loc) + 0.5 - size / 2.0);
        offsets.push(f64::from(*loc) + 0.5 + size / 2.0);
        previous = *loc;
    }
    if offsets.is_empty() {
        return Err(AssemblyError::NoBogeys);
    }
    Ok(offsets)
}

/// Why the wheel points could not be put on the graph (Java logs a warning and returns).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlacePointsError {
    /// "Cannot assemble: No valid starting node found".
    NoStartingNode,
    /// "Cannot assemble: Missing graph edge".
    MissingEdge,
    /// "Cannot assemble: Not all Points created".
    NotAllPoints,
}

/// The loop in `assemble` that creates the wheel points: walks the graph from `start` (the end of
/// the station's track block in the assembly direction) in half-block steps along `direction`, and
/// as each point's offset is passed puts a [`TravellingPoint`] on the edge behind (`position` =
/// how far past its offset the walk is). At most `assembly_length * 2 + 40` steps.
pub fn place_travelling_points(
    graph: &TrackGraph,
    start: TrackNodeLocation,
    direction: Vec3,
    assembly_length: i32,
    point_offsets: &[f64],
) -> Result<Vec<TravellingPoint>, PlacePointsError> {
    let mut location = start;
    let mut points: Vec<TravellingPoint> = Vec::new();
    let mut second_node: Option<i32> = None;

    for j in 0..(assembly_length * 2 + 40) {
        let i = f64::from(j) / 2.0;
        if points.len() == point_offsets.len() {
            break;
        }

        let current_location = location;
        location = TrackNodeLocation::new(add(location.location(), scale(direction, 0.5)), location.dimension);

        let Some(node) = graph.locate_node(&current_location) else { continue };
        let node = node.net_id;

        let mut point_index = points.len();
        while point_index < point_offsets.len() {
            let offset = point_offsets[point_index];
            if offset > i {
                break;
            }
            let position_on_edge = i - offset;

            let connections = graph.connections_from(node);

            if second_node.is_none() {
                if let Some(connections) = connections {
                    for (other, edge) in connections {
                        if edge.is_turn() {
                            continue;
                        }
                        let edge_direction = edge.direction(true);
                        if equal(dot(normalize(edge_direction), direction), -1.0) {
                            second_node = Some(*other);
                        }
                    }
                }
            }

            let Some(second) = second_node else { return Err(PlacePointsError::NoStartingNode) };
            if connections.and_then(|c| c.get(&second)).is_none() {
                return Err(PlacePointsError::MissingEdge);
            }

            points.push(TravellingPoint::new(node, second, position_on_edge, false));
            point_index += 1;
        }

        second_node = Some(node);
    }

    if points.len() != point_offsets.len() {
        return Err(PlacePointsError::NotAllPoints);
    }
    Ok(points)
}

#[cfg(test)]
mod tests {
    use super::super::graph::TrackEdgePoint;
    use super::super::network::RailNetworks;
    use super::super::network::tests::{disc, line, loc};
    use super::*;

    struct Trains(BTreeMap<Uuid, (Option<Uuid>, bool, f64)>);
    impl TrainStates for Trains {
        fn exists(&self, train: Uuid) -> bool {
            self.0.contains_key(&train)
        }
        fn current_station(&self, train: Uuid) -> Option<Uuid> {
            self.0.get(&train).and_then(|t| t.0)
        }
        fn navigation_active(&self, train: Uuid) -> bool {
            self.0.get(&train).is_some_and(|t| t.1)
        }
        fn distance_to_destination(&self, train: Uuid) -> f64 {
            self.0.get(&train).map_or(0.0, |t| t.2)
        }
    }

    #[test]
    fn a_new_station_is_named_and_open() {
        let s = GlobalStation::new();
        assert_eq!(s.name, "Track Station");
        assert!(s.can_approach_from(true));
        assert!(!s.can_approach_from(false));
        assert!(s.can_navigate_via());
        let mut s = s;
        s.block_entity_added([1, 2, 3], 0, true);
        assert!(s.assembling);
        assert!(!s.can_approach_from(true));
        assert!(!s.can_navigate_via());
        assert_eq!(s.block_entity_pos, Some([1, 2, 3]));
    }

    #[test]
    fn the_nearest_train_wins_the_reservation() {
        let trains = Trains(BTreeMap::from([(1, (None, true, 40.0)), (2, (None, true, 10.0)), (3, (None, true, 10.0))]));
        let mut s = GlobalStation::new();
        s.reserve_for(1, 40.0, &trains);
        assert_eq!(s.nearest_train, Some(1));
        // A nearer train takes over; an equal one does not.
        s.reserve_for(2, 10.0, &trains);
        assert_eq!(s.nearest_train, Some(2));
        s.reserve_for(3, 10.0, &trains);
        assert_eq!(s.nearest_train, Some(2));
        // Only the holder can cancel.
        s.cancel_reservation(3);
        assert_eq!(s.nearest_train, Some(2));
        s.train_departed(2);
        assert_eq!(s.nearest_train, None);
    }

    #[test]
    fn a_train_that_is_gone_does_not_hold_the_station() {
        let trains = Trains(BTreeMap::from([(2, (None, true, 99.0))]));
        let mut s = GlobalStation::new();
        s.nearest_train = Some(1);
        assert_eq!(s.nearest_train(&trains), None);
        s.reserve_for(2, 99.0, &trains);
        assert_eq!(s.nearest_train, Some(2));
    }

    #[test]
    fn present_and_imminent_trains() {
        let id = 7;
        let mut t = Trains(BTreeMap::from([(1, (Some(id), false, 0.0))]));
        let mut s = GlobalStation::new();
        s.nearest_train = Some(1);
        assert_eq!(s.present_train(id, &t), Some(1));
        assert_eq!(s.imminent_train(id, &t), Some(1));
        // On its way: imminent only within 30, never present.
        t.0.insert(1, (None, true, 30.0));
        assert_eq!(s.present_train(id, &t), None);
        assert_eq!(s.imminent_train(id, &t), Some(1));
        t.0.insert(1, (None, true, 30.5));
        assert_eq!(s.imminent_train(id, &t), None);
        t.0.insert(1, (None, false, 5.0));
        assert_eq!(s.imminent_train(id, &t), None);
        // At another station.
        t.0.insert(1, (Some(99), false, 0.0));
        assert_eq!(s.present_train(id, &t), None);
    }

    #[test]
    fn edge_point_station_rules() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 5.0]);
        let id = nets.new_uuid();
        let p = TrackEdgePoint::new_station(id, [loc(0.0, 0.0), loc(5.0, 0.0)], 2.0);
        nets.add_point(g, p);
        let p = nets.graphs[&g].point(super::super::graph::EdgePointType::Station, id).unwrap();
        // The primary side is the edge's first node (`edgeLocation.second`).
        assert!(p.can_approach_from(&loc(5.0, 0.0)));
        assert!(!p.can_approach_from(&loc(0.0, 0.0)));
        assert!(p.can_navigate_via(&loc(0.0, 0.0)));
        nets.graphs.get_mut(&g).unwrap().edge_points.get_mut(super::super::graph::EdgePointType::Station, id).unwrap().station_mut().unwrap().assembling = true;
        let p = nets.graphs[&g].point(super::super::graph::EdgePointType::Station, id).unwrap();
        assert!(!p.can_approach_from(&loc(5.0, 0.0)));
        assert!(!p.can_navigate_via(&loc(0.0, 0.0)));
        let _ = disc(0.0, 0.0);
    }

    #[test]
    fn scanning_the_assembly_track_collects_bogeys() {
        // Track for 10 blocks; bogeys at 0, 5 (upright) and 8 (upside down).
        let scan = scan_assembly(
            128,
            20,
            |i| i < 10,
            |i, below| match (i, below) {
                (0, false) | (5, false) => Some(("std", false)),
                (8, true) => Some(("std", true)),
                _ => None,
            },
        );
        assert_eq!(scan.length, 9);
        assert_eq!(scan.bogey_locations, vec![0, 5, 8]);
        assert_eq!(scan.upside_down, vec![false, false, true]);
        assert_eq!(scan.bogey_count(), 3);
        assert_eq!(scan.bogey_types, vec!["std"; 3]);
    }

    #[test]
    fn scanning_stops_at_the_limits() {
        // Track never ends: stops one short of the maximum.
        let scan: AssemblyScan<()> = scan_assembly(128, 20, |_| true, |_, _| None);
        assert_eq!(scan.length, 127);
        // Track ends at the very start: length 0, not -1.
        let scan: AssemblyScan<()> = scan_assembly(128, 20, |_| false, |_, _| None);
        assert_eq!(scan.length, 0);
        // A bogey on the last step is not seen.
        let scan = scan_assembly(10, 20, |_| true, |i, b| (i == 9 && !b).then_some(((), false)));
        assert_eq!(scan.length, 9);
        assert!(scan.bogey_locations.is_empty());
        // At most max_bogeys are kept.
        let scan = scan_assembly(20, 2, |_| true, |i, b| (i % 3 == 0 && !b).then_some(((), false)));
        assert_eq!(scan.bogey_locations, vec![0, 3]);
        // An upright lookup that finds an upside down block falls through to the lower one.
        let scan = scan_assembly(5, 5, |_| true, |i, _| (i == 1).then_some(((), true)));
        assert_eq!(scan.upside_down, vec![true]);
    }

    #[test]
    fn valid_bogey_offsets() {
        // No bogeys yet: only the station's own block.
        assert!(is_valid_bogey_offset(0, &[]));
        assert!(!is_valid_bogey_offset(5, &[]));
        // With one at 0: nothing within 2 of it, nothing under 3.
        assert!(!is_valid_bogey_offset(0, &[0]));
        assert!(!is_valid_bogey_offset(2, &[0]));
        assert!(is_valid_bogey_offset(3, &[0]));
        assert!(is_valid_bogey_offset(10, &[0, 6]));
        assert!(!is_valid_bogey_offset(8, &[0, 6]));
        assert!(!is_valid_bogey_offset(4, &[0, 6]));
        assert!(!is_valid_bogey_offset(-1, &[0]));
    }

    #[test]
    fn axis_alignment() {
        assert!(track_axis_is_aligned([1.0, 0.0, 0.0]));
        assert!(track_axis_is_aligned([0.0, 0.0, -1.0]));
        assert!(track_axis_is_aligned([0.0, 0.0, 0.0]));
        assert!(!track_axis_is_aligned([1.0, 0.0, 1.0]));
        assert!(!track_axis_is_aligned([1.0, 0.25, 0.0]));
    }

    #[test]
    fn direction_nearest() {
        assert_eq!(Direction::nearest(1.0, 0.0, 0.0), Direction::East);
        assert_eq!(Direction::nearest(-0.2, 0.0, -0.9), Direction::North);
        assert_eq!(Direction::nearest(0.0, -3.0, 1.0), Direction::Down);
        // A tie keeps the first in `Direction.values()` order.
        assert_eq!(Direction::nearest(0.0, 0.0, 0.0), Direction::Down);
        assert_eq!(assembly_direction([2.0, 0.0, 0.0], -1), Direction::West);
        assert_eq!(assembly_direction([0.0, 0.0, 3.0], 1), Direction::South);
    }

    #[test]
    fn point_offsets_and_their_errors() {
        // Bogeys at 0 and 4, wheel spacing 2 and 3.
        let offsets = bogey_point_offsets(&[0, 4], |i| if i == 0 { 2.0 } else { 3.0 }).unwrap();
        assert_eq!(offsets, vec![-0.5, 1.5, 3.0, 6.0]);
        assert_eq!(bogey_point_offsets(&[1, 5], |_| 2.0), Err(AssemblyError::FrontmostBogeyAtStation));
        assert_eq!(bogey_point_offsets(&[], |_| 2.0), Err(AssemblyError::FrontmostBogeyAtStation));
        let too_close = bogey_point_offsets(&[0, 2], |_| 2.0).unwrap_err();
        assert_eq!(too_close, AssemblyError::BogeysTooClose { a: 1, b: 2 });
        assert_eq!(too_close.key(), "train_assembly.bogeys_too_close");
        assert_eq!(too_close.args(), vec![1, 2]);
        // Exactly 3 apart is fine.
        assert!(bogey_point_offsets(&[0, 3], |_| 2.0).is_ok());
    }

    #[test]
    fn error_keys() {
        assert_eq!(AssemblyError::NoBogeys.key(), "train_assembly.no_bogeys");
        assert_eq!(AssemblyError::NoControls.key(), "train_assembly.no_controls");
        assert_eq!(AssemblyError::NotConnectedInOrder.key(), "train_assembly.not_connected_in_order");
        assert_eq!(AssemblyError::SingleBogeyCarriage.key(), "train_assembly.single_bogey_carriage");
        assert_eq!(AssemblyError::FrontmostBogeyAtStation.key(), "train_assembly.frontmost_bogey_at_station");
        let e = AssemblyError::NothingAttached { carriage: 2 };
        assert_eq!(e.key(), "train_assembly.nothing_attached");
        assert_eq!(e.args(), vec![2]);
    }

    #[test]
    fn wheel_points_land_on_the_edge_behind_each_offset() {
        // A straight line of nodes every 1 block along +x, from x = 0 to 12.
        let mut nets = RailNetworks::new();
        let xs: Vec<f64> = (0..=12).map(|x| f64::from(x)).collect();
        let g = line(&mut nets, &xs);
        let graph = &nets.graphs[&g];
        // The assembly runs toward -x from the station at x = 10: the first node is at x = 10.
        let pts = place_travelling_points(graph, loc(10.0, 0.0), [-1.0, 0.0, 0.0], 8, &[0.0, 2.0]).unwrap();
        assert_eq!(pts.len(), 2);
        let n = |x: f64| graph.locate_node(&loc(x, 0.0)).unwrap().net_id;
        // The first point is at the start node, on the edge that leads back toward +x... which is
        // the edge whose direction runs against the assembly direction: from x=10 to x=11.
        assert_eq!(pts[0].node1, Some(n(10.0)));
        assert_eq!(pts[0].node2, Some(n(11.0)));
        assert_eq!(pts[0].position, 0.0);
        // The second is 2 blocks on: at the node at x = 8, on the edge from 8 to 9, at position 0.
        assert_eq!(pts[1].node1, Some(n(8.0)));
        assert_eq!(pts[1].node2, Some(n(9.0)));
        assert_eq!(pts[1].position, 0.0);
    }

    #[test]
    fn wheel_points_fail_without_track() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 1.0, 2.0]);
        let graph = &nets.graphs[&g];
        // The walk runs off the end of the graph before every point exists.
        let r = place_travelling_points(graph, loc(0.0, 0.0), [-1.0, 0.0, 0.0], 2, &[0.0, 50.0]);
        assert_eq!(r, Err(PlacePointsError::NotAllPoints));
        // A start that is not on the graph never finds a node.
        let r = place_travelling_points(graph, loc(100.0, 0.0), [-1.0, 0.0, 0.0], 2, &[0.0]);
        assert_eq!(r, Err(PlacePointsError::NotAllPoints));
        // A point on a lone node with no edge against the direction has no starting node.
        let mut nets = RailNetworks::new();
        let g = nets.new_graph();
        let gid = g.id;
        nets.put_graph_with_default_group(g);
        nets.create_node_if_absent(gid, &disc(0.0, 0.0));
        let r = place_travelling_points(&nets.graphs[&gid], loc(0.0, 0.0), [-1.0, 0.0, 0.0], 2, &[0.0]);
        assert_eq!(r, Err(PlacePointsError::NoStartingNode));
    }
}
