//! `Train` and `TrainStatus` (content/trains/entity/Train.java, TrainStatus.java, TrainMigration.java):
//! the train as a whole, ticked each game tick by [`super::railways::Railways`].
//!
//! `Train` implements [`NavTrain`] (so [`Navigation`] drives it) and, through [`ScheduleCtx`],
//! [`ScheduleTrain`] (so [`ScheduleRuntime`] does). Navigation and the runtime are taken out of the
//! train for the length of a call into them (Rust cannot lend a train to its own field), so what they
//! ask of the train that Java answers from the runtime comes from a copy ([`Train::paused`]) and what
//! they ask it to do to the runtime is queued (`pending_transit_interrupted`) and done after.
//! Whatever Java does to the world (explosions, mail transfers, entity management, fuel items used up)
//! is pushed as a [`TrainEvent`] for the glue.
//!
//! [`Train::tick`]: the passive slowdown, the carriages' stress (`stress[]` from the anchors),
//! stalls, then each carriage travelling by the speed plus the stress that pushes it, blocked
//! (`end_of_track`) or over-stressed (`coupling_stress`, derail) checks, then the navigation
//! target refresh. Collisions with other trains ([`find_colliding_train`]) crash both.
//!
//! Deviations from Java (all noted at the place): when a train arrives at its destination while
//! travelling, `runtime.destinationReached()` runs right after that carriage's travel rather than
//! inside the listener; `manageEntities` is an event; collisions are applied after the graph is back
//! in the networks; `forEachTravellingPointBackwards` keeps Java's reuse of the trailing bogey's
//! leading point for "inside 2nd bogey".
//!
//! Left out: NBT/packets, `Train.read/write`, honking and steam (`honk`, `lowHonk`, `honkPitch`,
//! `accumulatedSteamRelease`), advancements except the red-signal one as an event, `getOwner`,
//! `determineHonk`, `distanceToLocationSqr`, `backwardsDriver`, `CarriageParticles`, sounds.

use super::carriage::{CargoState, Carriage, CarriageKind, TravelCtx};
use super::graph::{EdgePointType, NodeId, TrackNodeLocation, Uuid};
use super::navigation::{
    ConductorStatus, DiscoveredPath, NavTrain, Navigation, OtherTrain, SearchEnv, TRACK_TYPE_STANDARD, carriage_valid_types,
    navigation_penalty,
};
use super::network::{RailNetworks, TrackGraph};
use super::schedule::{
    ConditionWorld, PortInfo, ScheduleRuntime, ScheduleTrain, State, StationInfo, TrainStatusEvent,
};
use super::station::TrainStates;
use super::travelling_point::{Candidate, EdgePointHit, Listeners, SteerDirection, TrackGraphLocation, TrackSelector, TravellingPoint, steer_select};
use super::vec::{Vec3, add, distance_to, equal, intersect, intersect_sphere, length, normalize, scale, sub, Plane, UP};
use crate::Pos;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

// --- config ------------------------------------------------------------------------------------

/// `CTrains`: the numbers of the trains config (defaults in Create 6.0.10).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrainConfig {
    pub trains_cause_damage: bool,
    pub max_assembly_length: i32,
    pub max_bogey_count: i32,
    pub manual_train_speed_modifier: f32,
    /// Blocks per second.
    pub train_top_speed: f32,
    pub train_turning_top_speed: f32,
    /// Blocks per second squared.
    pub train_acceleration: f32,
    pub powered_train_top_speed: f32,
    pub powered_train_turning_top_speed: f32,
    pub powered_train_acceleration: f32,
}

impl Default for TrainConfig {
    fn default() -> Self {
        TrainConfig {
            trains_cause_damage: true,
            max_assembly_length: 128,
            max_bogey_count: 20,
            manual_train_speed_modifier: 0.75,
            train_top_speed: 28.0,
            train_turning_top_speed: 14.0,
            train_acceleration: 3.0,
            powered_train_top_speed: 40.0,
            powered_train_turning_top_speed: 20.0,
            powered_train_acceleration: 3.0,
        }
    }
}

// --- status ------------------------------------------------------------------------------------

/// One line the train tells its owner (`TrainStatus.StatusMessage`): a lang key under
/// `train.status.` and whether it is good news, or the crash report with where each present
/// dimension's carriage is.
#[derive(Clone, Debug, PartialEq)]
pub enum StatusMessage {
    /// `displayInformation(key, itsAGoodThing, args)`.
    Info { key: String, good: bool, args: Vec<String> },
    /// `crash()`: `train.status.collision` and a `train.status.collision.where` per dimension.
    Crash { locations: Vec<(u32, Pos)> },
}

/// `TrainStatus`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrainStatus {
    pub navigation: bool,
    pub track: bool,
    pub conductor: bool,
    /// `queued`: at most 3, the oldest dropped.
    pub queued: Vec<StatusMessage>,
}

impl TrainStatus {
    fn info(&mut self, key: &str, good: bool, args: Vec<String>) {
        self.add_message(StatusMessage::Info { key: key.to_string(), good, args });
    }

    /// `addMessage(message)`.
    pub fn add_message(&mut self, message: StatusMessage) {
        self.queued.push(message);
        if self.queued.len() > 3 {
            self.queued.remove(0);
        }
    }

    /// `tick(level)`'s other half: the owner is online, the glue shows these and empties the queue.
    pub fn take_messages(&mut self) -> Vec<StatusMessage> {
        std::mem::take(&mut self.queued)
    }

    pub fn failed_navigation(&mut self) {
        if self.navigation {
            return;
        }
        self.info("no_path", false, vec![]);
        self.navigation = true;
    }

    pub fn failed_navigation_no_target(&mut self, filter: &str) {
        if self.navigation {
            return;
        }
        self.info("no_match", false, vec![filter.to_string()]);
        self.navigation = true;
    }

    pub fn failed_package_no_target(&mut self, address: &str) {
        if self.navigation {
            return;
        }
        self.info("no_package_target", false, vec![address.to_string()]);
        self.navigation = true;
    }

    pub fn successful_navigation(&mut self) {
        if !self.navigation {
            return;
        }
        self.info("navigation_success", true, vec![]);
        self.navigation = false;
    }

    pub fn found_conductor(&mut self) {
        if !self.conductor {
            return;
        }
        self.info("found_driver", true, vec![]);
        self.conductor = false;
    }

    pub fn missing_conductor(&mut self) {
        if self.conductor {
            return;
        }
        self.info("missing_driver", false, vec![]);
        self.conductor = true;
    }

    pub fn missing_correct_conductor(&mut self) {
        if self.conductor {
            return;
        }
        self.info("opposite_driver", false, vec![]);
        self.conductor = true;
    }

    pub fn manual_controls(&mut self) {
        self.info("paused_for_manual", true, vec![]);
    }

    pub fn failed_migration(&mut self) {
        if self.track {
            return;
        }
        self.info("track_missing", false, vec![]);
        self.track = true;
    }

    pub fn high_stress(&mut self) {
        if self.track {
            return;
        }
        self.info("coupling_stress", false, vec![]);
        self.track = true;
    }

    pub fn double_portal(&mut self) {
        if self.track {
            return;
        }
        self.info("double_portal", false, vec![]);
        self.track = true;
    }

    pub fn end_of_track(&mut self) {
        if self.track {
            return;
        }
        self.info("end_of_track", false, vec![]);
        self.track = true;
    }

    /// `crash()`, with where each present dimension's carriage is.
    pub fn crash(&mut self, locations: Vec<(u32, Pos)>) {
        self.add_message(StatusMessage::Crash { locations });
    }

    pub fn successful_migration(&mut self) {
        if !self.track {
            return;
        }
        self.info("back_on_track", true, vec![]);
        self.track = false;
    }

    pub fn track_ok(&mut self) {
        self.track = false;
    }

    /// `newSchedule()`.
    pub fn new_schedule(&mut self) {
        self.navigation = false;
        self.conductor = false;
    }
}

// --- world and events ----------------------------------------------------------------------------

/// What a train asks of the world around it. The glue implements it.
pub trait TrainWorld {
    /// `level.getDayTime()`.
    fn day_time(&self) -> i64;
    /// `REDSTONE_LINK_NETWORK_HANDLER.globalPowerVersion`.
    fn redstone_power_version(&self) -> i32;
    /// `hasAnyLoadedPower(frequency)`.
    fn has_any_loaded_power(&self, frequency: &[String; 2]) -> bool;
    /// The station's block has a neighbour signal (false where it isn't loaded).
    fn station_powered(&self, station: Uuid) -> bool;
    /// The station is not entity ticking.
    fn station_unloaded(&self, station: Uuid) -> bool;
    /// `server.getLevel(dimension) != null`.
    fn dimension_loaded(&self, dimension: u32) -> bool;
    /// The addresses of the packages waiting at a station's package port, slot by slot.
    fn port_packages(&self, port: Pos, dimension: u32) -> Vec<String>;
    /// `train.countPlayerPassengers()`.
    fn count_player_passengers(&self, train: Uuid) -> i32;
    /// Does the filter (a `FilterItemStack` as the glue keys it) accept the item or fluid?
    fn item_matches(&self, filter: &str, id: &str) -> bool {
        filter == id
    }
    /// `entity.checkConductors()` of the carriage: forward and backward driver present; `None` when
    /// `updateConductors` leaves the carriage alone (no entity, in several dimensions, or carrying
    /// serialised passengers).
    fn check_conductors(&self, train: Uuid, carriage: i32) -> Option<[bool; 2]>;
}

/// What a train did that the world must be told, for the glue to drain.
#[derive(Clone, Debug, PartialEq)]
pub enum TrainEvent {
    /// `level.explode(null, x, y, z, power, NONE)`.
    Explosion { pos: Vec3, power: f32 },
    Crashed { train: Uuid },
    /// `station.runMailTransfer()`.
    MailTransfer { train: Uuid, station: Uuid },
    /// A train left the manager (`invalid`, or disassembled): `RemoveTrainPacket`.
    TrainRemoved { train: Uuid },
    /// `carriage.manageEntities(level)`: create, align or discard the carriage's entities for the
    /// dimensions its wheels traverse (`Carriage::traversed_dimensions`).
    ManageEntities { train: Uuid, carriage: i32 },
    /// `burnFuel` took one item out of a carriage's fuel storage (the glue removes it, returning a
    /// container item if it has one).
    FuelConsumed { train: Uuid, carriage: i32, slot: usize },
    /// `AllAdvancements.RED_SIGNAL` for a train without a running schedule stopped at a red signal.
    RedSignal { train: Uuid },
}

// --- migration ---------------------------------------------------------------------------------

/// `TrainMigration`: where a travelling point was when its graph went, to find its place again.
#[derive(Clone, Debug, PartialEq)]
pub struct TrainMigration {
    pub locations: [TrackNodeLocation; 2],
    pub position_on_old_edge: f64,
    pub curve: bool,
    pub fallback: Vec3,
}

impl TrainMigration {
    /// `new TrainMigration(point)`, with the edge it stood on (`None` when it stood on none).
    pub fn new(point: &TravellingPoint, graph: &TrackGraph) -> Option<TrainMigration> {
        let edge = point.edge(graph)?;
        Some(TrainMigration::from_edge(point, edge, graph))
    }

    /// The same, from an edge that is no longer in the graph.
    pub fn from_edge(point: &TravellingPoint, edge: &super::graph::TrackEdge, graph: &TrackGraph) -> TrainMigration {
        let t = point.position / edge.length();
        let loc = |n: Option<NodeId>, fallback: TrackNodeLocation| n.and_then(|n| graph.get_node(n)).map_or(fallback, |n| n.location);
        TrainMigration {
            fallback: edge.position(None, t),
            curve: edge.is_turn(),
            position_on_old_edge: point.position,
            locations: [loc(point.node1, edge.loc1), loc(point.node2, edge.loc2)],
        }
    }

    /// `tryMigratingTo(graph)`: the same edge if the graph has it, else a straight edge whose line
    /// passes the old place (within 1/64 of a block) in the same direction.
    pub fn try_migrating_to(&self, graph_id: Uuid, graph: &TrackGraph) -> Option<TrackGraphLocation> {
        let n1 = graph.locate_node(&self.locations[0]);
        let n2 = graph.locate_node(&self.locations[1]);
        if let (Some(n1), Some(n2)) = (n1, n2) {
            if graph.edge(n1.net_id, n2.net_id).is_some() {
                return Some(TrackGraphLocation { graph: graph_id, edge: self.locations, position: self.position_on_old_edge });
            }
        }
        if self.curve {
            return None;
        }
        let prev_direction = normalize(sub(self.locations[1].location(), self.locations[0].location()));
        for node in graph.nodes() {
            let loc = node.location;
            let node_vec = loc.location();
            let d = distance_to(node_vec, self.fallback);
            if d * d > 32.0 * 32.0 {
                continue;
            }
            let Some(conns) = graph.connections_from(node.net_id) else { continue };
            for (other, edge) in conns {
                if edge.is_turn() {
                    continue;
                }
                let radius = f64::from(1.0_f32 / 64.0);
                let direction = edge.direction(true);
                if !equal(super::vec::dot(direction, prev_direction), 1.0) {
                    continue;
                }
                let Some(hit) = intersect_sphere(node_vec, direction, self.fallback, radius) else { continue };
                if !equal(super::vec::dot(direction, normalize(sub(hit, node_vec))), 1.0) {
                    continue;
                }
                let position = distance_to(hit, node_vec) - radius;
                if position.is_nan() || position < 0.0 || position > edge.length() {
                    continue;
                }
                let Some(other_node) = graph.get_node(*other) else { continue };
                return Some(TrackGraphLocation { graph: graph_id, edge: [loc, other_node.location], position });
            }
        }
        None
    }
}

// --- the train ---------------------------------------------------------------------------------

/// What a signal listener and the navigation selector change while a carriage travels.
struct Run<'a> {
    train_id: Uuid,
    nets: &'a mut RailNetworks,
    nav: &'a mut Navigation,
    status: &'a mut TrainStatus,
    speed: &'a mut f64,
    current_station: &'a mut Option<Uuid>,
    occupied: &'a mut BTreeMap<Uuid, Option<Uuid>>,
    reserved: &'a mut BTreeSet<Uuid>,
    observers: &'a mut BTreeSet<Uuid>,
    cached_observer_filtering: &'a mut BTreeMap<Uuid, (i32, bool)>,
    pending_arrival: &'a mut Option<Uuid>,
    /// `runtime.getSchedule() == null || runtime.paused`.
    schedule_idle: bool,
    events: &'a mut Vec<TrainEvent>,
}

/// `occupy(groupId, boundaryId)`.
fn occupy(occupied: &mut BTreeMap<Uuid, Option<Uuid>>, reserved: &mut BTreeSet<Uuid>, group: Uuid, boundary: Option<Uuid>) -> bool {
    reserved.remove(&group);
    if let Some(b) = boundary {
        if let Some(Some(current)) = occupied.get(&group) {
            if *current == b {
                return false;
            }
        }
    }
    // `put(...) == null`: no previous entry, or one with a null value.
    !matches!(occupied.insert(group, boundary), Some(Some(_)))
}

/// `Train`.
#[derive(Clone, Debug)]
pub struct Train {
    pub id: Uuid,
    pub owner: Option<Uuid>,
    /// `graph`: none while derailed or detached.
    pub graph: Option<Uuid>,
    /// The graph the wheels' node ids belong to, kept when `graph` goes (a crashed train's points
    /// still name nodes).
    pub point_graph: Option<Uuid>,
    pub navigation: Navigation,
    pub runtime: ScheduleRuntime,
    pub icon: String,
    pub map_color_index: i32,
    pub name: String,
    pub status: TrainStatus,
    pub invalid: bool,

    pub speed: f64,
    pub target_speed: f64,
    pub speed_before_stall: Option<f64>,
    /// `carriageWaitingForChunks`: a carriage id or -1 (the glue sets it).
    pub carriage_waiting_for_chunks: i32,
    pub throttle: f64,

    pub manual_steer: SteerDirection,
    pub manual_tick: bool,
    pub current_station: Option<Uuid>,
    pub currently_backwards: bool,
    pub double_ended: bool,
    pub carriages: Vec<Carriage>,
    pub carriage_spacing: Vec<i32>,

    pub update_signal_blocks: bool,
    pub occupied_signal_blocks: BTreeMap<Uuid, Option<Uuid>>,
    pub reserved_signal_blocks: BTreeSet<Uuid>,
    pub occupied_observers: BTreeSet<Uuid>,
    /// Observer id to the cargo version it was last checked at and whether it matched.
    pub cached_observer_filtering: BTreeMap<Uuid, (i32, bool)>,

    pub migrating_points: Vec<TrainMigration>,
    pub migration_cooldown: i32,
    pub derailed: bool,
    pub fuel_ticks: i32,
    pub config: TrainConfig,

    ticks_since_last_mail_transfer: i32,
    stress: Vec<f64>,
    /// Copy of `runtime.paused` for what Navigation asks while the runtime is out.
    runtime_paused: bool,
    pending_transit_interrupted: bool,
    pending_leave_station: bool,
    pending_arrival: Option<Uuid>,
    pending_events: Vec<TrainEvent>,
}

/// What a train answers for `TrainStates`: the others, and this one as it was when the call began.
pub struct TrainView<'a> {
    pub others: &'a BTreeMap<Uuid, Train>,
    /// `(id, current station, navigating, distance to destination)` of the train being ticked.
    pub me: Option<(Uuid, Option<Uuid>, bool, f64)>,
}

impl TrainStates for TrainView<'_> {
    fn exists(&self, train: Uuid) -> bool {
        self.others.contains_key(&train) || self.me.is_some_and(|m| m.0 == train)
    }
    fn current_station(&self, train: Uuid) -> Option<Uuid> {
        if let Some(m) = self.me {
            if m.0 == train {
                return m.1;
            }
        }
        self.others.get(&train).and_then(|t| t.current_station_id())
    }
    fn navigation_active(&self, train: Uuid) -> bool {
        if let Some(m) = self.me {
            if m.0 == train {
                return m.2;
            }
        }
        self.others.get(&train).is_some_and(|t| t.navigation.is_active())
    }
    fn distance_to_destination(&self, train: Uuid) -> f64 {
        if let Some(m) = self.me {
            if m.0 == train {
                return m.3;
            }
        }
        self.others.get(&train).map_or(0.0, |t| t.navigation.distance_to_destination)
    }
}

impl TrainStates for BTreeMap<Uuid, Train> {
    fn exists(&self, train: Uuid) -> bool {
        self.contains_key(&train)
    }
    fn current_station(&self, train: Uuid) -> Option<Uuid> {
        self.get(&train).and_then(|t| t.current_station_id())
    }
    fn navigation_active(&self, train: Uuid) -> bool {
        self.get(&train).is_some_and(|t| t.navigation.is_active())
    }
    fn distance_to_destination(&self, train: Uuid) -> f64 {
        self.get(&train).map_or(0.0, |t| t.navigation.distance_to_destination)
    }
}

/// `Mth.equal(float, float)`.
fn mth_equal_f32(a: f32, b: f32) -> bool {
    (b - a).abs() < 1.0e-5
}

impl Train {
    /// `new Train(id, owner, graph, carriages, carriageSpacing, doubleEnded, name, icon, mapColorIndex)`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: Uuid,
        owner: Option<Uuid>,
        graph: Option<Uuid>,
        carriages: Vec<Carriage>,
        carriage_spacing: Vec<i32>,
        double_ended: bool,
        name: &str,
        icon: &str,
        map_color_index: i32,
    ) -> Train {
        let stress = vec![0.0; carriage_spacing.len()];
        Train {
            id,
            owner,
            graph,
            point_graph: graph,
            navigation: Navigation::new(),
            runtime: ScheduleRuntime::new(),
            icon: icon.to_string(),
            map_color_index,
            name: name.to_string(),
            status: TrainStatus::default(),
            invalid: false,
            speed: 0.0,
            target_speed: 0.0,
            speed_before_stall: None,
            carriage_waiting_for_chunks: -1,
            throttle: 1.0,
            manual_steer: SteerDirection::None,
            manual_tick: false,
            current_station: None,
            currently_backwards: false,
            double_ended,
            carriages,
            carriage_spacing,
            update_signal_blocks: false,
            occupied_signal_blocks: BTreeMap::new(),
            reserved_signal_blocks: BTreeSet::new(),
            occupied_observers: BTreeSet::new(),
            cached_observer_filtering: BTreeMap::new(),
            migrating_points: Vec::new(),
            migration_cooldown: 0,
            derailed: false,
            fuel_ticks: 0,
            config: TrainConfig::default(),
            ticks_since_last_mail_transfer: 0,
            stress,
            runtime_paused: false,
            pending_transit_interrupted: false,
            pending_leave_station: false,
            pending_arrival: None,
            pending_events: Vec::new(),
        }
    }

    // --- numbers ---

    /// `maxSpeed()`: blocks per tick.
    pub fn max_speed(&self) -> f64 {
        let c = &self.config;
        f64::from((if self.fuel_ticks > 0 { c.powered_train_top_speed } else { c.train_top_speed }) / 20.0)
    }

    /// `maxTurnSpeed()`.
    pub fn max_turn_speed(&self) -> f64 {
        let c = &self.config;
        f64::from((if self.fuel_ticks > 0 { c.powered_train_turning_top_speed } else { c.train_turning_top_speed }) / 20.0)
    }

    /// `acceleration()`.
    pub fn acceleration(&self) -> f64 {
        let c = &self.config;
        f64::from((if self.fuel_ticks > 0 { c.powered_train_acceleration } else { c.train_acceleration }) / 400.0)
    }

    /// `getCurrentStation()`'s id (Java also needs the graph and the station in it; see
    /// [`Train::current_station_in`]).
    pub fn current_station_id(&self) -> Option<Uuid> {
        if self.graph.is_none() {
            return None;
        }
        self.current_station
    }

    /// `getCurrentStation()`: the station point in the train's graph.
    pub fn current_station_in<'a>(&self, nets: &'a RailNetworks) -> Option<&'a super::graph::TrackEdgePoint> {
        let id = self.current_station?;
        nets.graphs.get(&self.graph?)?.point(EdgePointType::Station, id)
    }

    /// `hasForwardConductor()`.
    pub fn has_forward_conductor(&self) -> bool {
        self.carriages.iter().any(|c| c.present_conductors[0])
    }

    /// `hasBackwardConductor()`.
    pub fn has_backward_conductor(&self) -> bool {
        self.carriages.iter().any(|c| c.present_conductors[1])
    }

    /// `getTotalLength()`.
    pub fn total_length(&self) -> i32 {
        let mut length = 0;
        let n = self.carriages.len();
        for (i, c) in self.carriages.iter().enumerate() {
            if i == 0 {
                length += (c.leading_bogey().ty.wheel_point_spacing / 2.0) as i32;
            }
            if i == n - 1 {
                length += (c.trailing_bogey().ty.wheel_point_spacing / 2.0) as i32;
            }
            length += c.bogey_spacing;
            if i < self.carriage_spacing.len() {
                length += self.carriage_spacing[i];
            }
        }
        length
    }

    /// The leading wheel's position on its edge.
    pub fn leading_x(&self) -> f64 {
        self.carriages[0].leading_point().position
    }

    /// `getNavigationPenalty()`.
    pub fn navigation_penalty(&self) -> i32 {
        let paused = self.runtime_paused_now();
        navigation_penalty(&self.navigation.penalty_input(self.manual_tick, self.runtime.schedule().is_some(), paused))
    }

    fn runtime_paused_now(&self) -> bool {
        self.runtime.paused
    }

    /// `getEndpointEdges()`: the nodes of the edge under the leading point and under the trailing one.
    pub fn endpoint_edges(&self) -> [(Option<NodeId>, Option<NodeId>); 2] {
        let (Some(first), Some(last)) = (self.carriages.first(), self.carriages.last()) else { return [(None, None); 2] };
        let l = first.leading_point();
        let t = last.trailing_point();
        [(l.node1, l.node2), (t.node1, t.node2)]
    }

    /// This train as another train's search sees it (none without a graph).
    pub fn as_other(&self) -> Option<OtherTrain> {
        Some(OtherTrain {
            id: self.id,
            graph: self.graph?,
            navigation_penalty: self.navigation_penalty(),
            endpoint_edges: self.endpoint_edges(),
        })
    }

    /// `forEachTravellingPoint`: every wheel, carriage by carriage (the trailing bogey only when
    /// there are two).
    pub fn travelling_points(&self) -> Vec<&TravellingPoint> {
        let mut v = Vec::new();
        for c in &self.carriages {
            v.extend(c.bogeys.0.points.iter());
            if let Some(b) = &c.bogeys.1 {
                v.extend(b.points.iter());
            }
        }
        v
    }

    fn travelling_points_mut(&mut self) -> Vec<&mut TravellingPoint> {
        let mut v = Vec::new();
        for c in &mut self.carriages {
            let (a, b) = (&mut c.bogeys.0, &mut c.bogeys.1);
            v.extend(a.points.iter_mut());
            if let Some(b) = b {
                v.extend(b.points.iter_mut());
            }
        }
        v
    }

    /// `isTravellingOn(node)`.
    pub fn is_travelling_on(&self, node: NodeId) -> bool {
        self.travelling_points().iter().any(|tp| tp.node1 == Some(node) || tp.node2 == Some(node))
    }

    /// `forEachTravellingPointBackwards`: each wheel's point with the distance to travel to reach it
    /// from the one before, from the trailing end forward. Java's "inside 2nd bogey" reuses
    /// `trailingBogey.leading()` (not the leading bogey's): kept.
    pub fn travelling_points_backwards(&self) -> Vec<(TravellingPoint, f64)> {
        let mut out = Vec::new();
        let mut last_wheel_offset = 0.0;
        let n = self.carriages.len();
        for i in 0..n {
            let index = n - i - 1;
            let carriage = &self.carriages[index];
            let trailing_bogey = carriage.trailing_bogey();
            let trail_spacing = trailing_bogey.ty.wheel_point_spacing;

            // trailing point
            let gap = if i == 0 { 0.0 } else { f64::from(self.carriage_spacing[index]) - last_wheel_offset - trail_spacing / 2.0 };
            out.push((trailing_bogey.trailing().clone(), gap));
            // inside 1st bogey
            out.push((trailing_bogey.leading().clone(), trail_spacing));

            last_wheel_offset = trail_spacing / 2.0;

            if !carriage.is_on_two_bogeys() {
                continue;
            }

            let leading_bogey = carriage.leading_bogey();
            let lead_spacing = leading_bogey.ty.wheel_point_spacing;

            // between bogeys
            out.push((leading_bogey.trailing().clone(), f64::from(carriage.bogey_spacing) - last_wheel_offset - lead_spacing / 2.0));
            // inside 2nd bogey
            out.push((trailing_bogey.leading().clone(), lead_spacing));

            last_wheel_offset = lead_spacing / 2.0;
        }
        out
    }

    // --- disassembly ---

    /// `canDisassemble()`. `entity_of(carriage)` is the carriage's entity as `(pitch, yaw)`, none
    /// without one.
    pub fn can_disassemble(&self, entity_of: &dyn Fn(&Carriage) -> Option<(f32, f32)>) -> bool {
        for carriage in &self.carriages {
            if carriage.present_in_multiple_dimensions() {
                return false;
            }
            let Some((pitch, yaw)) = entity_of(carriage) else { return false };
            if !mth_equal_f32(pitch, 0.0) {
                return false;
            }
            if !mth_equal_f32(((yaw % 90.0) + 360.0) % 90.0, 0.0) {
                return false;
            }
        }
        true
    }

    /// `disassemble(assemblyDirection, pos)`'s placing: where each carriage's entity goes, as
    /// `(carriage index, block position)`, `pos` being the block above the station's track and
    /// `direction` its assembly direction's normal.
    pub fn disassembly_positions(&self, direction: [i32; 3], pos: Pos) -> Vec<(usize, Pos)> {
        let n = self.carriages.len();
        let mut offset = 1;
        let backwards = self.currently_backwards;
        let mut out = Vec::new();
        for i in 0..n {
            let index = if backwards { n - i - 1 } else { i };
            let carriage = &self.carriages[index];
            let steps = if backwards { offset + carriage.bogey_spacing } else { offset };
            let below = if carriage.leading_bogey().is_upside_down() { 2 } else { 0 };
            out.push((index, [pos[0] + direction[0] * steps, pos[1] + direction[1] * steps - below, pos[2] + direction[2] * steps]));
            offset += carriage.bogey_spacing;
            if i < self.carriage_spacing.len() {
                let si = if backwards { self.carriage_spacing.len() - i - 1 } else { i };
                offset += self.carriage_spacing[si];
            }
        }
        out
    }

    // --- stations ---

    /// `setCurrentStation(station)`.
    pub fn set_current_station(&mut self, station: Uuid) {
        self.current_station = Some(station);
    }

    /// `leaveStation()`.
    pub fn leave_station(&mut self, nets: &mut RailNetworks) {
        if let (Some(g), Some(s)) = (self.graph, self.current_station) {
            if let Some(station) = nets.graphs.get_mut(&g).and_then(|g| g.edge_points.get_mut(EdgePointType::Station, s)).and_then(|p| p.station_mut()) {
                station.train_departed(self.id);
            }
        }
        self.current_station = None;
    }

    // --- running the runtime and navigation ---

    fn with_navigation<R>(&mut self, f: impl FnOnce(&mut Navigation, &mut Train) -> R) -> R {
        let mut nav = std::mem::take(&mut self.navigation);
        let r = f(&mut nav, self);
        self.navigation = nav;
        r
    }

    /// Does what the runtime and navigation queued while the other was out.
    fn flush(&mut self, nets: &mut RailNetworks) {
        if self.pending_transit_interrupted {
            self.pending_transit_interrupted = false;
            self.runtime.transit_interrupted();
        }
        if self.pending_leave_station {
            self.pending_leave_station = false;
            self.leave_station(nets);
        }
    }

    fn view_of<'a>(&self, others: &'a BTreeMap<Uuid, Train>) -> TrainView<'a> {
        TrainView {
            others,
            me: Some((self.id, self.current_station_id(), self.navigation.is_active(), self.navigation.distance_to_destination)),
        }
    }

    /// `navigation.cancelNavigation()`.
    pub fn cancel_navigation(&mut self, nets: &mut RailNetworks) {
        self.with_navigation(|nav, t| nav.cancel_navigation(nets, t));
        self.flush(nets);
    }

    /// `navigation.startNavigation(path)`.
    pub fn start_navigation(&mut self, nets: &mut RailNetworks, path: Option<DiscoveredPath>) -> f64 {
        let r = self.with_navigation(|nav, t| nav.start_navigation(nets, t, path));
        self.flush(nets);
        r
    }

    fn with_runtime<R>(
        &mut self,
        nets: &mut RailNetworks,
        others: &BTreeMap<Uuid, Train>,
        world: &dyn TrainWorld,
        f: impl FnOnce(&mut ScheduleRuntime, &mut ScheduleCtx) -> R,
    ) -> R {
        let mut runtime = std::mem::take(&mut self.runtime);
        self.runtime_paused = runtime.paused;
        let mut nav = std::mem::take(&mut self.navigation);
        let r = {
            let mut ctx = ScheduleCtx { train: &mut *self, nav: &mut nav, nets, others, world };
            f(&mut runtime, &mut ctx)
        };
        self.navigation = nav;
        self.runtime = runtime;
        self.runtime_paused = self.runtime.paused;
        self.flush(nets);
        r
    }

    /// `runtime.startCurrentInstruction(level)`: the path the current instruction wants, if any.
    pub fn start_current_instruction(
        &mut self,
        nets: &mut RailNetworks,
        others: &BTreeMap<Uuid, Train>,
        world: &dyn TrainWorld,
    ) -> Option<DiscoveredPath> {
        self.with_runtime(nets, others, world, |rt, ctx| rt.start_current_instruction(ctx))
    }

    /// `navigation.findPathTo(station, Double.MAX_VALUE)`.
    pub fn find_path_to(&mut self, nets: &mut RailNetworks, others: &BTreeMap<Uuid, Train>, station: Uuid) -> Option<DiscoveredPath> {
        let mut nav = std::mem::take(&mut self.navigation);
        let r = {
            let other_views: Vec<OtherTrain> = others.iter().filter(|(id, _)| **id != self.id).filter_map(|(_, t)| t.as_other()).collect();
            let view = self.view_of(others);
            let track_type_of = |_: Option<&str>| TRACK_TYPE_STANDARD.to_string();
            let env = SearchEnv { nets: &*nets, trains: &view, others: &other_views, track_type_of: &track_type_of };
            nav.find_path_to(&env, &*self, station, f64::MAX)
        };
        self.navigation = nav;
        r
    }

    /// `runtime.setSchedule(schedule, auto)`.
    pub fn set_schedule(
        &mut self,
        nets: &mut RailNetworks,
        others: &BTreeMap<Uuid, Train>,
        world: &dyn TrainWorld,
        schedule: super::schedule::Schedule,
        auto: bool,
    ) {
        self.with_runtime(nets, others, world, |rt, ctx| rt.set_schedule(schedule, auto, ctx));
    }

    /// `runtime.discardSchedule()`.
    pub fn discard_schedule(&mut self, nets: &mut RailNetworks, others: &BTreeMap<Uuid, Train>, world: &dyn TrainWorld) {
        self.with_runtime(nets, others, world, |rt, ctx| rt.discard_schedule(ctx));
    }

    /// `runtime.returnSchedule()`.
    pub fn return_schedule(
        &mut self,
        nets: &mut RailNetworks,
        others: &BTreeMap<Uuid, Train>,
        world: &dyn TrainWorld,
    ) -> Option<super::schedule::Schedule> {
        self.with_runtime(nets, others, world, |rt, ctx| rt.return_schedule(ctx))
    }

    /// `arriveAt(station)`: sets the station, drops the reserved blocks, tells the runtime.
    pub fn arrive_at(&mut self, nets: &mut RailNetworks, others: &BTreeMap<Uuid, Train>, world: &dyn TrainWorld, station: Uuid, events: &mut Vec<TrainEvent>) {
        self.set_current_station(station);
        self.reserved_signal_blocks.clear();
        self.with_runtime(nets, others, world, |rt, ctx| rt.destination_reached(ctx));
        events.push(TrainEvent::MailTransfer { train: self.id, station });
        self.ticks_since_last_mail_transfer = 0;
    }

    // --- speed ---

    fn tick_passive_slowdown(&mut self) {
        if !self.manual_tick && self.navigation.destination.is_none() && self.speed != 0.0 {
            let acceleration = self.acceleration();
            if self.speed > 0.0 {
                self.speed = (self.speed - acceleration).max(0.0);
            } else {
                self.speed = (self.speed + acceleration).min(0.0);
            }
        }
        self.manual_tick = false;
    }

    fn tick_derailed_slowdown(&mut self) {
        self.speed /= 3.0;
        if equal(self.speed, 0.0) {
            self.speed = 0.0;
        }
    }

    /// `cancelStall()`: the stalled carriages are let go (the glue cancels the actors' stalls).
    pub fn cancel_stall(&mut self) {
        self.speed_before_stall = None;
        for c in &mut self.carriages {
            c.stalled = false;
        }
    }

    /// `crash()`: cancels the navigation, flips the speed, derails; the train leaves the graph.
    pub fn crash(&mut self, nets: &mut RailNetworks, events: &mut Vec<TrainEvent>) {
        self.cancel_navigation(nets);
        if self.derailed {
            return;
        }
        self.speed = -self.speed.clamp(-0.5, 0.5);
        self.derailed = true;
        if self.graph.is_some() {
            self.point_graph = self.graph;
        }
        self.graph = None;
        let locations: Vec<(u32, Pos)> = self
            .carriages
            .iter()
            .flat_map(|c| c.dimensions.iter().filter_map(|(d, dce)| dce.position_anchor.map(|p| (*d, super::vec::block_containing(p)))))
            .fold(Vec::new(), |mut acc, x| {
                if !acc.iter().any(|(d, _)| *d == x.0) {
                    acc.push(x);
                }
                acc
            });
        self.status.crash(locations);
        events.push(TrainEvent::Crashed { train: self.id });
    }

    // --- signals ---

    /// `collectInitiallyOccupiedSignalBlocks()`.
    pub fn collect_initially_occupied_signal_blocks(&mut self, nets: &RailNetworks) {
        let Some(gid) = self.graph else { return };
        let Some(graph) = nets.graphs.get(&gid) else { return };
        let Some(last) = self.carriages.last() else { return };
        let trailing = last.trailing_point().clone();
        let (Some(node1), Some(node2)) = (trailing.node1, trailing.node2) else { return };
        let Some(edge) = graph.edge(node1, node2) else { return };
        let position = trailing.position;
        let node2_loc = graph.get_node(node2).map(|n| n.location);
        let node1_loc = graph.get_node(node1).map(|n| n.location);

        self.occupied_signal_blocks.clear();
        self.reserved_signal_blocks.clear();
        self.occupied_observers.clear();
        self.cached_observer_filtering.clear();

        let mut scout = TravellingPoint { node1: Some(node1), node2: Some(node2), position, blocked: false, upside_down: false };
        let mut prev_group: Option<Uuid> = None;
        let groups = &nets.signal_edge_groups;
        let group_of = |id: Uuid, loc: Option<TrackNodeLocation>| -> Option<Uuid> {
            let p = graph.point(EdgePointType::Signal, id)?;
            p.signal()?.group(p.is_primary(&loc?))
        };

        let data = &edge.edge_data;
        if data.has_signal_boundaries() {
            match data.next_of(EdgePointType::Signal, position) {
                None => {
                    let mut prev: Option<Uuid> = None;
                    let mut current = data.next_of(EdgePointType::Signal, 0.0);
                    while let Some(c) = current {
                        prev = Some(c.id);
                        current = data.next_of(EdgePointType::Signal, c.location_on);
                    }
                    if let Some(p) = prev {
                        if let Some(group) = group_of(p, node2_loc) {
                            if groups.contains_key(&group) {
                                occupy(&mut self.occupied_signal_blocks, &mut self.reserved_signal_blocks, group, None);
                                prev_group = Some(group);
                            }
                        }
                    }
                }
                Some(next) => {
                    if let Some(group) = group_of(next.id, node1_loc) {
                        if groups.contains_key(&group) {
                            occupy(&mut self.occupied_signal_blocks, &mut self.reserved_signal_blocks, group, None);
                            prev_group = Some(group);
                        }
                    }
                }
            }
        } else if let Some(group) = data.effective_edge_group_id(gid) {
            if groups.contains_key(&group) {
                occupy(&mut self.occupied_signal_blocks, &mut self.reserved_signal_blocks, group, None);
                prev_group = Some(group);
            }
        }

        let steps = self.travelling_points_backwards();
        let occupied = &mut self.occupied_signal_blocks;
        let reserved = &mut self.reserved_signal_blocks;
        let observers = &mut self.occupied_observers;
        for (tp, d) in steps {
            let mut selector = TrackSelector::follow(&tp);
            let mut on_point = |_distance: f64, hit: &EdgePointHit| -> bool {
                if hit.point.ty == EdgePointType::Observer {
                    observers.insert(hit.point.id);
                    return false;
                }
                let Some(signal) = hit.point.signal() else { return false };
                for loc in [hit.nodes.0, hit.nodes.1] {
                    let Some(l) = graph.get_node(loc).map(|n| n.location) else { continue };
                    let Some(id) = signal.group(hit.point.is_primary(&l)) else { continue };
                    if !groups.contains_key(&id) {
                        continue;
                    }
                    if prev_group == Some(id) {
                        continue;
                    }
                    occupy(occupied, reserved, id, None);
                    prev_group = Some(id);
                }
                false
            };
            let mut listeners = Listeners { edge_point: Some(&mut on_point), turn: None, portal: None };
            scout.travel(graph, d, &mut selector, &mut listeners);
        }
    }

    /// `earlyTick(level)`: before any train moves each game tick. `world` answers the observers'
    /// filters.
    pub fn early_tick(&mut self, nets: &mut RailNetworks, world: &dyn TrainWorld) {
        if self.graph.is_none() && !self.migrating_points.is_empty() {
            self.reattach_to_tracks(nets);
        }
        if self.graph.is_none() {
            add_to_signal_groups_map(&mut nets.signal_edge_groups, &mut self.occupied_signal_blocks, self.id);
            return;
        }

        if self.update_signal_blocks {
            self.update_signal_blocks = false;
            self.collect_initially_occupied_signal_blocks(nets);
        }

        add_to_signal_groups_map(&mut nets.signal_edge_groups, &mut self.occupied_signal_blocks, self.id);
        add_to_signal_groups_set(&mut nets.signal_edge_groups, &mut self.reserved_signal_blocks, self.id);

        if self.occupied_observers.is_empty() {
            return;
        }
        self.tick_occupied_observers(nets, world);
    }

    fn tick_occupied_observers(&mut self, nets: &mut RailNetworks, world: &dyn TrainWorld) {
        let Some(gid) = self.graph else { return };
        let storage_version: i32 = self.carriages.iter().map(|c| c.storage.version).sum();
        let ids: Vec<Uuid> = self.occupied_observers.iter().copied().collect();
        for uuid in ids {
            let Some(graph) = nets.graphs.get_mut(&gid) else { return };
            let Some(observer) = graph.edge_points.get_mut(EdgePointType::Observer, uuid).and_then(|p| p.observer_mut()) else {
                continue;
            };
            let Some(filter) = observer.filter.clone() else {
                observer.keep_alive(self.id);
                continue;
            };

            let cached = *self.cached_observer_filtering.entry(uuid).or_insert((-1, false));
            let mut should_activate = cached.1;
            if cached.0 == storage_version {
                if should_activate {
                    observer.keep_alive(self.id);
                }
                continue;
            }

            should_activate = false;
            for carriage in &self.carriages {
                if should_activate {
                    break;
                }
                for item in &carriage.storage.items {
                    if should_activate {
                        break;
                    }
                    if item.count <= 0 {
                        continue;
                    }
                    should_activate |= world.item_matches(&filter, &item.id);
                }
                if let Some((fluid, _)) = carriage.storage.fluids.iter().find(|(_, amount)| *amount > 0) {
                    // `drain(1, SIMULATE)` drains the same fluid for every tank, so one test.
                    if !should_activate {
                        should_activate |= world.item_matches(&filter, fluid);
                    }
                }
            }

            self.cached_observer_filtering.insert(uuid, (storage_version, should_activate));
            if should_activate {
                observer.keep_alive(self.id);
            }
        }
    }

    // --- migration ---

    /// `detachFromTracks()`: remembers where every wheel was and lets go of the graph.
    /// Cancel the navigation first (it needs the networks).
    pub fn detach_from_tracks(&mut self, graph: &TrackGraph, removed_edges: &[super::graph::TrackEdge]) {
        self.migrating_points.clear();
        let mut migrations = Vec::new();
        for tp in self.travelling_points() {
            let (Some(a), Some(b)) = (tp.node1, tp.node2) else { continue };
            let removed = removed_edges.iter().find(|e| e.node1 == a && e.node2 == b);
            match (removed, tp.edge(graph)) {
                (_, Some(e)) => migrations.push(TrainMigration::from_edge(tp, e, graph)),
                (Some(e), None) => migrations.push(TrainMigration::from_edge(tp, e, graph)),
                _ => {}
            }
        }
        self.migrating_points = migrations;
        if self.graph.is_some() {
            self.point_graph = self.graph;
        }
        self.graph = None;
    }

    /// `reattachToTracks(level)`.
    pub fn reattach_to_tracks(&mut self, nets: &mut RailNetworks) {
        if self.migration_cooldown > 0 {
            self.migration_cooldown -= 1;
            return;
        }

        let mut graphs: Vec<Uuid> = nets.graphs.keys().copied().collect();
        let mut successful: BTreeMap<Uuid, Vec<TrackGraphLocation>> = BTreeMap::new();
        for md in &self.migrating_points {
            graphs.retain(|gid| match md.try_migrating_to(*gid, &nets.graphs[gid]) {
                None => false,
                Some(gl) => {
                    successful.entry(*gid).or_default().push(gl);
                    true
                }
            });
        }

        if graphs.is_empty() {
            self.migration_cooldown = 40;
            self.status.failed_migration();
            self.derailed = true;
            return;
        }

        let gid = graphs[0];
        self.graph = Some(gid);
        self.point_graph = Some(gid);
        let locations = successful.remove(&gid).unwrap_or_default();
        {
            let graph = &nets.graphs[&gid];
            let mut it = locations.iter();
            for tp in self.travelling_points_mut() {
                if let Some(loc) = it.next() {
                    tp.migrate_to(graph, loc);
                }
            }
        }
        self.migrating_points.clear();
        if self.derailed {
            self.status.successful_migration();
        }
        self.derailed = false;
        if self.runtime.schedule().is_some() && self.runtime.state == State::InTransit {
            self.runtime.state = State::PreTransit;
        }
        if let Some(s) = self.current_station {
            let (id, distance) = (self.id, self.navigation.distance_to_destination);
            let empty = BTreeMap::new();
            let view = TrainView { others: &empty, me: Some((id, self.current_station_id(), self.navigation.is_active(), distance)) };
            if let Some(station) = nets.graphs.get_mut(&gid).and_then(|g| g.edge_points.get_mut(EdgePointType::Station, s)).and_then(|p| p.station_mut()) {
                station.reserve_for(id, distance, &view);
            }
        }
        self.update_signal_blocks = true;
        self.migration_cooldown = 0;
    }

    // --- conductors ---

    fn update_conductors(&mut self, world: &dyn TrainWorld) {
        let id = self.id;
        for c in &mut self.carriages {
            if let Some(found) = world.check_conductors(id, c.id) {
                c.present_conductors = found;
            }
        }
    }

    // --- tick ---

    /// `tick(level)`.
    pub fn tick(
        &mut self,
        nets: &mut RailNetworks,
        others: &mut BTreeMap<Uuid, Train>,
        world: &dyn TrainWorld,
        events: &mut Vec<TrainEvent>,
    ) {
        nets.mark_tracks_dirty();

        let Some(gid) = self.graph else {
            for c in &self.carriages {
                events.push(TrainEvent::ManageEntities { train: self.id, carriage: c.id });
            }
            self.update_conductors(world);
            return;
        };
        self.point_graph = Some(gid);

        if self.current_station_in(nets).is_some() {
            self.ticks_since_last_mail_transfer += 1;
            if self.ticks_since_last_mail_transfer > 20 {
                if let Some(s) = self.current_station {
                    events.push(TrainEvent::MailTransfer { train: self.id, station: s });
                }
                self.ticks_since_last_mail_transfer = 0;
            }
        }

        self.update_conductors(world);
        self.with_runtime(nets, others, world, |rt, ctx| {
            let cond = ConditionView::new(ctx.world, ctx.train, rt);
            rt.tick(ctx, &cond)
        });
        {
            let view = self.view_of(others);
            let mut nav = std::mem::take(&mut self.navigation);
            nav.tick(nets, self, &view);
            self.navigation = nav;
            self.flush(nets);
            events.append(&mut self.pending_events);
        }

        self.tick_passive_slowdown();
        if self.derailed {
            self.tick_derailed_slowdown();
        }

        let mut distance = self.speed;
        let carriage_count = self.carriages.len();
        let mut stalled = false;
        let mut max_stress: f64 = 0.0;

        if self.carriage_waiting_for_chunks != -1 {
            distance = 0.0;
        }

        // Stress between carriages, from their anchors.
        let graph_ref = nets.graphs.get(&gid);
        for i in 0..carriage_count {
            if i > 0 {
                let target = self.carriage_spacing[i - 1];
                let mut actual = f64::from(target);
                let carriage = &self.carriages[i];
                let previous = &self.carriages[i - 1];
                let leading_point = carriage.leading_point();
                let trailing_point = previous.trailing_point();

                let mut entries = 0;
                let mut total = 0.0;

                if let (Some(g), Some(n1), Some(n2)) = (graph_ref, leading_point.node1, trailing_point.node1) {
                    let d1 = g.get_node(n1).map(|n| n.location.dimension);
                    let d2 = g.get_node(n2).map(|n| n.location.dimension);
                    if let (Some(d1), Some(d2)) = (d1, d2) {
                        for b in [true, false] {
                            let d = if b { d1 } else { d2 };
                            if !b && d1 == d2 {
                                continue;
                            }
                            if d1 != d2 {
                                continue;
                            }
                            let (Some(dimensional), Some(dimensional2)) = (carriage.dimensional_if_present(d), previous.dimensional_if_present(d)) else {
                                continue;
                            };
                            let (Some(leading_anchor), Some(trailing_anchor)) =
                                (dimensional.leading_anchor(carriage.is_on_two_bogeys()), dimensional2.trailing_anchor(previous.is_on_two_bogeys()))
                            else {
                                continue;
                            };
                            let d_sqr = {
                                let dd = distance_to(leading_anchor, trailing_anchor);
                                dd * dd
                            };
                            let distance_to_ = if carriage.leading_bogey().is_upside_down() != previous.trailing_bogey().is_upside_down() {
                                (d_sqr - 4.0).sqrt()
                            } else {
                                d_sqr.sqrt()
                            };
                            total += distance_to_;
                            entries += 1;
                        }
                    }
                }

                if entries > 0 {
                    actual = total / f64::from(entries);
                }

                self.stress[i - 1] = f64::from(target) - actual;
                max_stress = max_stress.max((f64::from(target) - actual).abs());
            }

            if self.carriages[i].stalled {
                if self.speed_before_stall.is_none() {
                    self.speed_before_stall = Some(self.speed);
                }
                distance = 0.0;
                self.speed = 0.0;
                stalled = true;
            }
        }

        if !stalled {
            if let Some(before) = self.speed_before_stall.take() {
                self.speed = before.clamp(-1.0, 1.0);
            }
        }

        // positive stress: carriages should move apart; negative: closer
        let approaching_station = self.navigation.distance_to_destination < 5.0;
        let leading_modifier = if approaching_station { 0.75 } else { 0.5 };
        let trailing_modifier = if approaching_station { 0.0 } else { 0.125 };

        let mut blocked = false;
        let iterate_from_back = self.speed < 0.0;
        let mut crash_with: Option<(Uuid, Vec3, f64)> = None;
        let max_len = f64::from(self.config.max_assembly_length);
        let mut arrivals: Vec<Uuid> = Vec::new();

        // The graph is out of the networks while the wheels move, so the signal groups in them can
        // change under the listeners.
        let Some(graph) = nets.graphs.remove(&gid) else {
            return;
        };

        {
            let Train {
                id,
                carriages,
                navigation,
                status,
                speed,
                current_station,
                occupied_signal_blocks,
                reserved_signal_blocks,
                occupied_observers,
                cached_observer_filtering,
                pending_arrival,
                derailed,
                manual_steer,
                stress,
                runtime,
                ..
            } = self;
            let derailed = *derailed;
            let manual_steer = *manual_steer;
            let train_id = *id;
            let max_assembly_length = max_len;
            let schedule_idle = runtime.schedule().is_none() || runtime.paused;
            let run = RefCell::new(Run {
                train_id,
                nets: &mut *nets,
                nav: navigation,
                status,
                speed,
                current_station,
                occupied: occupied_signal_blocks,
                reserved: reserved_signal_blocks,
                observers: occupied_observers,
                cached_observer_filtering,
                pending_arrival,
                schedule_idle,
                events: &mut *events,
            });

            for index in 0..carriage_count {
                let i = if iterate_from_back { carriage_count - 1 - index } else { index };
                let leading_stress = if i == 0 { 0.0 } else { stress[i - 1] * -(if iterate_from_back { trailing_modifier } else { leading_modifier }) };
                let trailing_stress = if i == stress.len() { 0.0 } else { stress[i] * (if iterate_from_back { leading_modifier } else { trailing_modifier }) };

                let to_follow_forward = if i == 0 { None } else { Some(carriages[i - 1].trailing_point().clone()) };
                let to_follow_backward = if i == carriage_count - 1 { None } else { Some(carriages[i + 1].leading_point().clone()) };

                let total_stress = if derailed { 0.0 } else { leading_stress + trailing_stress };
                let kind = CarriageKind::of(i, carriage_count);

                let mut control = |g: &TrackGraph, tp: &TravellingPoint, _forward: bool, cands: &[Candidate]| -> usize {
                    let mut r = run.borrow_mut();
                    if r.nav.destination.is_none() {
                        steer_select(g, tp, manual_steer, UP, cands)
                    } else {
                        r.nav.select_track(g, cands)
                    }
                };
                let mut front = |_d: f64, hit: &EdgePointHit| -> bool { front_signal(&run, &graph, hit) };
                let mut back = |_d: f64, hit: &EdgePointHit| -> bool { back_signal(&run, &graph, hit) };
                let mut double_portal = || run.borrow_mut().status.double_portal();
                let mut ctx = TravelCtx { derailed, control: &mut control, front: &mut front, back: &mut back, double_portal: &mut double_portal };

                let actual_distance =
                    carriages[i].travel(&graph, distance + total_stress, to_follow_forward, to_follow_backward, kind, &mut ctx);
                events_push_manage(&run, train_id, carriages[i].id);
                let incompatible = carriages[i].is_on_incompatible_track(&graph, &|_| TRACK_TYPE_STANDARD.to_string());
                blocked |= carriages[i].blocked || incompatible;

                let on_two_bogeys = carriages[i].is_on_two_bogeys();
                max_stress = max_stress.max(if on_two_bogeys { f64::from(carriages[i].bogey_spacing) - carriages[i].anchor_diff(&graph) } else { 0.0 });
                max_stress = max_stress.max(carriages[i].leading_bogey().stress(&graph, derailed));
                if on_two_bogeys {
                    max_stress = max_stress.max(carriages[i].trailing_bogey().stress(&graph, derailed));
                }

                if let Some(s) = run.borrow_mut().pending_arrival.take() {
                    arrivals.push(s);
                }

                if index == 0 {
                    distance = actual_distance;
                    let speed_now = *run.borrow().speed;
                    if !derailed {
                        crash_with = collide_with_other_trains(
                            train_id,
                            Some(gid),
                            &graph,
                            &carriages[i],
                            speed_now,
                            others,
                            max_assembly_length,
                        );
                    }
                    if crash_with.is_some() {
                        break;
                    }
                }
            }
        }
        nets.graphs.insert(gid, graph);

        for s in arrivals {
            self.arrive_at(nets, others, world, s, events);
        }

        if let Some((other_id, pos, combined)) = crash_with {
            if combined > f64::from(0.2_f32) {
                events.push(TrainEvent::Explosion { pos, power: (3.0 * combined).min(5.0) as f32 });
            }
            self.crash(nets, events);
            if let Some(other) = others.get_mut(&other_id) {
                other.crash(nets, events);
            }
            return;
        }

        if blocked {
            self.speed = 0.0;
            self.cancel_navigation(nets);
            self.with_runtime(nets, others, world, |rt, ctx| {
                let cond = ConditionView::new(ctx.world, ctx.train, rt);
                rt.tick(ctx, &cond)
            });
            self.status.end_of_track();
        } else if max_stress > 4.0 {
            self.speed = 0.0;
            self.cancel_navigation(nets);
            self.with_runtime(nets, others, world, |rt, ctx| {
                let cond = ConditionView::new(ctx.world, ctx.train, rt);
                rt.tick(ctx, &cond)
            });
            self.derailed = true;
            self.status.high_stress();
        } else if self.speed != 0.0 {
            self.status.track_ok();
        }

        self.update_navigation_target(nets, others, world, distance);
    }

    /// `updateNavigationTarget(level, distance)`.
    fn update_navigation_target(&mut self, nets: &mut RailNetworks, others: &BTreeMap<Uuid, Train>, world: &dyn TrainWorld, distance: f64) {
        if self.navigation.destination.is_none() {
            return;
        }

        let blocking_signal = self.navigation.waiting_for_signal;
        let mut full_refresh = self.navigation.distance_to_destination > 100.0 && self.navigation.distance_to_destination % 100.0 > 20.0;
        let mut signal_refresh = blocking_signal.is_some() && self.navigation.distance_to_signal % 50.0 > 5.0;
        let mut partial_refresh = self.navigation.distance_to_destination < 100.0 && self.navigation.distance_to_destination % 50.0 > 5.0;

        let to_subtract = if self.navigation.destination_behind_train { -distance } else { distance };
        let navigating_manually = self.runtime.paused;

        self.navigation.distance_to_destination -= to_subtract;
        if blocking_signal.is_some() {
            self.navigation.distance_to_signal -= to_subtract;
            signal_refresh &= self.navigation.distance_to_signal % 50.0 < 5.0;
        }

        full_refresh &= self.navigation.distance_to_destination % 100.0 <= 20.0;
        partial_refresh &= self.navigation.distance_to_destination % 50.0 <= 5.0;

        if let Some((signal_id, side)) = blocking_signal {
            if self.navigation.ticks_waiting_for_signal % 100 == 50 {
                let cross = self
                    .graph
                    .and_then(|g| nets.graphs.get(&g))
                    .and_then(|g| g.point(EdgePointType::Signal, signal_id))
                    .and_then(|p| p.signal())
                    .is_some_and(|s| s.types[if side { 0 } else { 1 }] == super::signal::SignalType::CrossSignal);
                full_refresh |= cross;
            }
        }

        if signal_refresh {
            self.navigation.waiting_for_signal = None;
        }
        if !full_refresh && !partial_refresh {
            return;
        }
        if !self.reserved_signal_blocks.is_empty() {
            return;
        }

        if !navigating_manually && full_refresh {
            let preferred = self.with_runtime(nets, others, world, |rt, ctx| rt.start_current_instruction(ctx));
            if let Some(path) = preferred {
                self.start_navigation(nets, Some(path));
            }
        }
    }
}

fn events_push_manage(run: &RefCell<Run>, train: Uuid, carriage: i32) {
    run.borrow_mut().events.push(TrainEvent::ManageEntities { train, carriage });
}

/// `addToSignalGroups` for the occupied map: groups that are gone are forgotten, the rest list the train.
fn add_to_signal_groups_map(groups: &mut BTreeMap<Uuid, super::signal::SignalEdgeGroup>, ids: &mut BTreeMap<Uuid, Option<Uuid>>, train: Uuid) {
    ids.retain(|k, _| groups.contains_key(k));
    for k in ids.keys() {
        if let Some(g) = groups.get_mut(k) {
            g.trains.insert(train);
        }
    }
}

/// `addToSignalGroups` for the reserved set.
fn add_to_signal_groups_set(groups: &mut BTreeMap<Uuid, super::signal::SignalEdgeGroup>, ids: &mut BTreeSet<Uuid>, train: Uuid) {
    ids.retain(|k| groups.contains_key(k));
    for k in ids.iter() {
        if let Some(g) = groups.get_mut(k) {
            g.trains.insert(train);
        }
    }
}

/// `frontSignalListener()`.
fn front_signal(run: &RefCell<Run>, graph: &TrackGraph, hit: &EdgePointHit) -> bool {
    let mut r = run.borrow_mut();
    let r = &mut *r;
    let point = hit.point;
    let Some(node2_loc) = graph.get_node(hit.nodes.1).map(|n| n.location) else { return false };

    if point.ty == EdgePointType::Station {
        if !point.can_approach_from(&node2_loc) || r.nav.destination != Some(point.id) {
            return false;
        }
        *r.speed = 0.0;
        r.nav.distance_to_destination = 0.0;
        r.nav.current_path.clear();
        // `arriveAt(destination)`: the station and the reservation here, the runtime afterwards.
        *r.current_station = Some(point.id);
        r.reserved.clear();
        *r.pending_arrival = Some(point.id);
        r.nav.destination = None;
        return true;
    }

    if point.ty == EdgePointType::Observer {
        r.observers.insert(point.id);
        return false;
    }

    let Some(signal) = point.signal() else { return false };
    if let Some((id, _)) = r.nav.waiting_for_signal {
        if id == point.id {
            *r.speed = 0.0;
            r.nav.distance_to_signal = 0.0;
            return true;
        }
    }

    let Some(group_id) = signal.group(point.is_primary(&node2_loc)) else { return false };
    if !r.nets.signal_edge_groups.contains_key(&group_id) {
        return false;
    }

    if r.schedule_idle && r.nets.is_group_occupied_unless_train(group_id, r.train_id) {
        let id = r.train_id;
        r.events.push(TrainEvent::RedSignal { train: id });
    }

    if let Some(g) = r.nets.signal_edge_groups.get_mut(&group_id) {
        g.reserved = Some(point.id);
    }
    occupy(r.occupied, r.reserved, group_id, Some(point.id));
    false
}

/// `backSignalListener()`.
fn back_signal(run: &RefCell<Run>, graph: &TrackGraph, hit: &EdgePointHit) -> bool {
    let mut r = run.borrow_mut();
    let point = hit.point;
    if point.ty == EdgePointType::Observer {
        r.observers.remove(&point.id);
        r.cached_observer_filtering.remove(&point.id);
        return false;
    }
    let Some(signal) = point.signal() else { return false };
    let Some(node1_loc) = graph.get_node(hit.nodes.0).map(|n| n.location) else { return false };
    if let Some(group_id) = signal.group(point.is_primary(&node1_loc)) {
        r.occupied.remove(&group_id);
    }
    false
}

// --- collisions ----------------------------------------------------------------------------------

/// `collideWithOtherTrains(level, carriage)`: the train it hits, where, and the two trains'
/// combined speed (the caller explodes above 0.2 and crashes both).
pub fn collide_with_other_trains(
    me: Uuid,
    graph_id: Option<Uuid>,
    graph: &TrackGraph,
    carriage: &Carriage,
    speed: f64,
    others: &BTreeMap<Uuid, Train>,
    max_assembly_length: f64,
) -> Option<(Uuid, Vec3, f64)> {
    let trailing = carriage.trailing_point();
    let leading = carriage.leading_point();
    let (Some(ln), Some(tn)) = (leading.node1, trailing.node1) else { return None };
    if leading.edge(graph).is_none() || trailing.edge(graph).is_none() {
        return None;
    }
    let dimension = graph.get_node(ln)?.location.dimension;
    if graph.get_node(tn)?.location.dimension != dimension {
        return None;
    }

    let start = (if speed < 0.0 { trailing } else { leading }).get_position(Some(graph), false)?;
    let end = (if speed < 0.0 { leading } else { trailing }).get_position(Some(graph), false)?;

    let (other_id, v) = find_colliding_train(me, graph_id, graph, start, end, dimension, others, max_assembly_length, None)?;
    let other_speed = others.get(&other_id).map_or(0.0, |t| t.speed);
    Some((other_id, v, speed.abs() + other_speed.abs()))
}

/// `findCollidingTrain(level, start, end, dimension)`: the first other train with a carriage (or
/// the gap between two) that the segment `start..end` crosses, and the crossing point. A train
/// without a graph is looked up in `graphs` (the rails the others' wheels stand on) via
/// `other_graphs`, defaulting to `graph`.
#[allow(clippy::too_many_arguments)]
pub fn find_colliding_train(
    me: Uuid,
    graph_id: Option<Uuid>,
    graph: &TrackGraph,
    start: Vec3,
    end: Vec3,
    dimension: u32,
    others: &BTreeMap<Uuid, Train>,
    max_assembly_length: f64,
    other_graphs: Option<&BTreeMap<Uuid, TrackGraph>>,
) -> Option<(Uuid, Vec3)> {
    let diff = sub(end, start);
    let max_distance_sqr = max_assembly_length.powi(2);

    'trains: for (id, train) in others {
        if *id == me {
            continue;
        }
        if train.graph.is_some() && train.graph != graph_id {
            continue;
        }
        let tgraph: &TrackGraph = match (train.graph.or(train.point_graph), other_graphs) {
            (Some(g), Some(m)) if m.contains_key(&g) => &m[&g],
            _ => graph,
        };

        let mut last_point: Option<Vec3> = None;

        for other_carriage in &train.carriages {
            for between_bits in [true, false] {
                if between_bits && last_point.is_none() {
                    continue;
                }
                let other_leading = other_carriage.leading_point();
                let other_trailing = other_carriage.trailing_point();
                if other_leading.edge(tgraph).is_none() || other_trailing.edge(tgraph).is_none() {
                    continue;
                }
                let (Some(d1), Some(d2)) = (
                    other_leading.node1.and_then(|n| tgraph.get_node(n)).map(|n| n.location.dimension),
                    other_trailing.node1.and_then(|n| tgraph.get_node(n)).map(|n| n.location.dimension),
                ) else {
                    continue;
                };
                if d1 != d2 || d1 != dimension {
                    continue;
                }

                let (Some(mut start2), Some(mut end2)) =
                    (other_leading.get_position(Some(tgraph), false), other_trailing.get_position(Some(tgraph), false))
                else {
                    continue;
                };

                let dd = |a: Vec3, b: Vec3| {
                    let d = distance_to(a, b);
                    d * d
                };
                if dd(start2, start).min(dd(end2, start)) > max_distance_sqr {
                    continue 'trains;
                }

                if between_bits {
                    end2 = start2;
                    start2 = last_point.unwrap();
                }

                last_point = Some(end2);

                if (end[1] < end2[1] - 3.0 || end2[1] < end[1] - 3.0) && (start[1] < start2[1] - 3.0 || start2[1] < start[1] - 3.0) {
                    continue;
                }

                let diff2 = sub(end2, start2);
                let normed_diff = normalize(diff);
                let normed_diff2 = normalize(diff2);
                let mut hit = intersect(start, start2, normed_diff, normed_diff2, Plane::Y);

                if hit.is_none() {
                    let Some(sphere) = intersect_sphere(start2, normed_diff2, start, 0.125_f32 as f64) else { continue };
                    if !equal(super::vec::dot(normed_diff2, normalize(sub(sphere, start2))), 1.0) {
                        continue;
                    }
                    hit = Some([distance_to(sphere, start) - 0.125, distance_to(sphere, start2) - 0.125]);
                }
                let hit = hit.unwrap();

                if hit[0] > length(diff) {
                    continue;
                }
                if hit[1] > length(diff2) {
                    continue;
                }
                if hit[0] < 0.0 {
                    continue;
                }
                if hit[1] < 0.0 {
                    continue;
                }

                return Some((*id, add(start, scale(normed_diff, hit[0]))));
            }
        }
    }
    None
}

// --- NavTrain ------------------------------------------------------------------------------------

impl NavTrain for Train {
    fn id(&self) -> Uuid {
        self.id
    }
    fn graph(&self) -> Option<Uuid> {
        self.graph
    }
    fn leading_point(&self) -> &TravellingPoint {
        self.carriages[0].leading_point()
    }
    fn trailing_point(&self) -> &TravellingPoint {
        self.carriages[self.carriages.len() - 1].trailing_point()
    }
    fn valid_track_types(&self) -> BTreeSet<String> {
        let cs: Vec<(Vec<String>, Option<Vec<String>>)> = self
            .carriages
            .iter()
            .map(|c| (c.leading_bogey().ty.valid_pathfinding_types(), c.bogeys.1.as_ref().map(|b| b.ty.valid_pathfinding_types())))
            .collect();
        carriage_valid_types(&cs)
    }
    fn has_forward_conductor(&self) -> bool {
        Train::has_forward_conductor(self)
    }
    fn has_backward_conductor(&self) -> bool {
        Train::has_backward_conductor(self)
    }
    fn double_ended(&self) -> bool {
        self.double_ended
    }
    fn paused(&self) -> bool {
        self.runtime_paused || self.runtime.paused
    }
    fn manual_tick(&self) -> bool {
        self.manual_tick
    }
    fn current_station(&self) -> Option<Uuid> {
        self.current_station_id()
    }
    fn clear_current_station(&mut self) {
        self.current_station = None;
    }
    fn speed(&self) -> f64 {
        self.speed
    }
    fn set_speed(&mut self, speed: f64) {
        self.speed = speed;
    }
    fn throttle(&self) -> f64 {
        self.throttle
    }
    fn acceleration(&self) -> f64 {
        Train::acceleration(self)
    }
    fn max_speed(&self) -> f64 {
        Train::max_speed(self)
    }
    fn max_turn_speed(&self) -> f64 {
        Train::max_turn_speed(self)
    }
    fn set_target_speed(&mut self, speed: f64) {
        self.target_speed = speed;
    }
    fn approach_target_speed(&mut self, acceleration_mod: f64) {
        let actual_target = self.target_speed;
        if equal(actual_target, self.speed) {
            return;
        }
        if self.manual_tick {
            // `leaveStation()`: needs the networks, done when the call returns.
            self.pending_leave_station = true;
        }
        let acceleration = Train::acceleration(self);
        if self.speed < actual_target {
            self.speed = (self.speed + acceleration * acceleration_mod).min(actual_target);
        } else if self.speed > actual_target {
            self.speed = (self.speed - acceleration * acceleration_mod).max(actual_target);
        }
    }
    fn burn_fuel(&mut self) {
        if self.fuel_ticks > 0 {
            self.fuel_ticks -= 1;
            return;
        }
        let iterate_from_back = self.speed < 0.0;
        let n = self.carriages.len();
        let id = self.id;
        for index in 0..n {
            let i = if iterate_from_back { n - 1 - index } else { index };
            let carriage = &mut self.carriages[i];
            let cid = carriage.id;
            for (slot, fuel) in carriage.storage.fuel.iter_mut().enumerate() {
                if fuel.count <= 0 || fuel.burn_time <= 0 {
                    continue;
                }
                fuel.count -= 1;
                self.fuel_ticks += fuel.burn_time;
                carriage.storage.change_detected();
                self.pending_events.push(TrainEvent::FuelConsumed { train: id, carriage: cid, slot });
                return;
            }
        }
    }
    fn set_currently_backwards(&mut self, backwards: bool) {
        self.currently_backwards = backwards;
    }
    fn report_conductors(&mut self, status: ConductorStatus) {
        match status {
            ConductorStatus::MissingConductor => self.status.missing_conductor(),
            ConductorStatus::MissingCorrectConductor => self.status.missing_correct_conductor(),
            ConductorStatus::FoundConductor => self.status.found_conductor(),
        }
    }
    fn reserved_signal_blocks(&mut self) -> &mut BTreeSet<Uuid> {
        &mut self.reserved_signal_blocks
    }
    fn transit_interrupted(&mut self) {
        self.pending_transit_interrupted = true;
    }
}

// --- ScheduleTrain -------------------------------------------------------------------------------

/// A train as the schedule runtime sees it: the train (without its runtime and navigation, which are
/// out), the navigation, the networks and the other trains.
pub struct ScheduleCtx<'a> {
    pub train: &'a mut Train,
    pub nav: &'a mut Navigation,
    pub nets: &'a mut RailNetworks,
    pub others: &'a BTreeMap<Uuid, Train>,
    pub world: &'a dyn TrainWorld,
}

impl ScheduleCtx<'_> {
    fn station_name(&self, id: Uuid) -> Option<String> {
        let g = self.train.graph?;
        self.nets.graphs.get(&g)?.point(EdgePointType::Station, id)?.station().map(|s| s.name.clone())
    }
}

impl ScheduleTrain for ScheduleCtx<'_> {
    fn derailed(&self) -> bool {
        self.train.derailed
    }
    fn has_forward_conductor(&self) -> bool {
        self.train.has_forward_conductor()
    }
    fn has_backward_conductor(&self) -> bool {
        self.train.has_backward_conductor()
    }
    fn navigating(&self) -> bool {
        self.nav.destination.is_some()
    }
    fn current_station(&self) -> Option<Uuid> {
        self.train.current_station_id()
    }
    fn stations(&self) -> Vec<StationInfo> {
        let Some(g) = self.train.graph.and_then(|g| self.nets.graphs.get(&g)) else { return Vec::new() };
        g.points(EdgePointType::Station)
            .into_iter()
            .filter_map(|p| {
                let s = p.station()?;
                Some(StationInfo {
                    id: p.id,
                    name: s.name.clone(),
                    dimension_loaded: s.block_entity_dimension.is_some_and(|d| self.world.dimension_loaded(d)),
                    ports: s
                        .connected_ports
                        .iter()
                        .map(|(pos, port)| PortInfo {
                            address: port.address.clone(),
                            waiting_packages: self.world.port_packages(*pos, s.block_entity_dimension.unwrap_or(0)),
                        })
                        .collect(),
                })
            })
            .collect()
    }
    fn carried_package_addresses(&self) -> Vec<String> {
        self.train
            .carriages
            .iter()
            .flat_map(|c| c.storage.items.iter().filter_map(|i| i.package_address.clone()))
            .collect()
    }
    fn find_path_to(&mut self, destinations: &[Uuid]) -> Option<DiscoveredPath> {
        let others: Vec<OtherTrain> = self.others.iter().filter(|(id, _)| **id != self.train.id).filter_map(|(_, t)| t.as_other()).collect();
        let view = TrainView {
            others: self.others,
            me: Some((self.train.id, self.train.current_station_id(), self.nav.is_active(), self.nav.distance_to_destination)),
        };
        let track_type_of = |_: Option<&str>| TRACK_TYPE_STANDARD.to_string();
        let env = SearchEnv { nets: &*self.nets, trains: &view, others: &others, track_type_of: &track_type_of };
        self.nav.find_path_to_any(&env, &*self.train, destinations, f64::MAX)
    }
    fn start_navigation(&mut self, path: DiscoveredPath) -> f64 {
        self.nav.start_navigation(self.nets, &mut *self.train, Some(path))
    }
    fn cancel_navigation(&mut self) {
        self.nav.cancel_navigation(self.nets, &mut *self.train);
    }
    fn report(&mut self, event: TrainStatusEvent) {
        let s = &mut self.train.status;
        match event {
            TrainStatusEvent::MissingConductor => s.missing_conductor(),
            TrainStatusEvent::FailedNavigation => s.failed_navigation(),
            TrainStatusEvent::FailedNavigationNoTarget(f) => s.failed_navigation_no_target(&f),
            TrainStatusEvent::FailedPackageNoTarget(a) => s.failed_package_no_target(&a),
            TrainStatusEvent::SuccessfulNavigation => s.successful_navigation(),
            TrainStatusEvent::NewSchedule => s.new_schedule(),
        }
    }
    fn set_throttle(&mut self, throttle: f32) {
        self.train.throttle = f64::from(throttle);
    }
    fn reset_idle_cargo_trackers(&mut self) {
        for c in &mut self.train.carriages {
            c.storage.reset_idle_cargo_tracker();
        }
    }
    fn tick_idle_cargo_trackers(&mut self) {
        for c in &mut self.train.carriages {
            c.storage.tick_idle_cargo_tracker();
        }
    }
    fn current_station_name(&self) -> Option<String> {
        self.station_name(self.train.current_station_id()?)
    }
    fn destination_name(&self) -> Option<String> {
        self.station_name(self.nav.destination?)
    }
    fn throttle(&self) -> f64 {
        self.train.throttle
    }
    fn max_speed(&self) -> f64 {
        self.train.max_speed()
    }
    fn max_turn_speed(&self) -> f64 {
        self.train.max_turn_speed()
    }
    fn distance_to_destination(&self) -> f64 {
        self.nav.distance_to_destination
    }
    fn distance_started_at(&self) -> f64 {
        self.nav.distance_started_at
    }
}

// --- ConditionWorld ----------------------------------------------------------------------------

/// What the wait conditions read: the train's cargo as it was when the tick began, and the world.
pub struct ConditionView<'a> {
    world: &'a dyn TrainWorld,
    train: Uuid,
    station: Option<Uuid>,
    cargo: Vec<CargoState>,
}

impl<'a> ConditionView<'a> {
    /// The cargo is copied only while the runtime waits at a station (`PostTransit`), the one
    /// state that reads it.
    pub fn new(world: &'a dyn TrainWorld, train: &Train, runtime: &ScheduleRuntime) -> ConditionView<'a> {
        ConditionView {
            world,
            train: train.id,
            station: train.current_station_id(),
            cargo: if runtime.state == State::PostTransit { train.carriages.iter().map(|c| c.storage.clone()).collect() } else { Vec::new() },
        }
    }
}

impl ConditionWorld for ConditionView<'_> {
    fn day_time(&self) -> i64 {
        self.world.day_time()
    }
    fn cargo_version(&self) -> i32 {
        self.cargo.iter().map(|c| c.version).sum()
    }
    fn min_ticks_since_last_exchange(&self) -> i32 {
        self.cargo.iter().map(|c| c.ticks_since_last_exchange).min().unwrap_or(i32::MAX)
    }
    fn matching_item_slots(&self, filter: Option<&str>) -> Vec<(i32, i32)> {
        self.cargo
            .iter()
            .flat_map(|c| c.items.iter())
            .filter(|i| filter.is_none_or(|f| self.world.item_matches(f, &i.id)))
            .map(|i| (i.count, i.max_stack))
            .collect()
    }
    fn matching_fluid_amounts(&self, filter: Option<&str>) -> Vec<i32> {
        self.cargo
            .iter()
            .flat_map(|c| c.fluids.iter())
            .filter(|(id, _)| filter.is_none_or(|f| self.world.item_matches(f, id)))
            .map(|(_, amount)| *amount)
            .collect()
    }
    fn redstone_power_version(&self) -> i32 {
        self.world.redstone_power_version()
    }
    fn has_any_loaded_power(&self, frequency: &[String; 2]) -> bool {
        self.world.has_any_loaded_power(frequency)
    }
    fn count_player_passengers(&self) -> i32 {
        self.world.count_player_passengers(self.train)
    }
    fn station_powered(&self) -> bool {
        self.station.is_some_and(|s| self.world.station_powered(s))
    }
    fn station_unloaded(&self) -> bool {
        self.station.is_some_and(|s| self.world.station_unloaded(s))
    }
}

#[cfg(test)]
mod tests {
    use super::super::bogey::BogeyType;
    use super::super::carriage::{CarriageBogey, FuelSlot};
    use super::super::graph::TrackEdgePoint;
    use super::super::network::tests::{line, loc};
    use super::super::network::RailEvent;
    use super::super::railways::Railways;
    use super::*;

    struct W;
    impl TrainWorld for W {
        fn day_time(&self) -> i64 {
            0
        }
        fn redstone_power_version(&self) -> i32 {
            0
        }
        fn has_any_loaded_power(&self, _: &[String; 2]) -> bool {
            false
        }
        fn station_powered(&self, _: Uuid) -> bool {
            false
        }
        fn station_unloaded(&self, _: Uuid) -> bool {
            false
        }
        fn dimension_loaded(&self, _: u32) -> bool {
            true
        }
        fn port_packages(&self, _: Pos, _: u32) -> Vec<String> {
            Vec::new()
        }
        fn count_player_passengers(&self, _: Uuid) -> i32 {
            0
        }
        fn check_conductors(&self, _: Uuid, _: i32) -> Option<[bool; 2]> {
            Some([true, true])
        }
    }

    fn node(nets: &RailNetworks, g: Uuid, x: f64) -> NodeId {
        nets.graphs[&g].locate_node(&loc(x, 0.0)).unwrap().net_id
    }

    /// A one-bogey train on the line between x = 0 and x = `end`, its leading wheel at `x` (facing
    /// +x) or its leading wheel `x` from the near end, facing -x.
    fn mk_train(nets: &RailNetworks, g: Uuid, id: Uuid, end: f64, x: f64, forward: bool) -> Train {
        let (a, b) = (node(nets, g, 0.0), node(nets, g, end));
        let pt = |p: f64| if forward { TravellingPoint::new(a, b, p, false) } else { TravellingPoint::new(b, a, end - p, false) };
        let (l, t) = if forward { (x, x - 2.0) } else { (x, x + 2.0) };
        let bogey = CarriageBogey::new(BogeyType::small(), false, vec![], pt(l), pt(t), 0);
        let mut c = Carriage::new(1, bogey, None, 0);
        c.present_conductors = [true, true];
        Train::new(id, None, Some(g), vec![c], vec![], false, "Train", "create:traditional", 0)
    }

    fn two_carriage_train(nets: &RailNetworks, g: Uuid, id: Uuid, end: f64, x: f64) -> Train {
        let (a, b) = (node(nets, g, 0.0), node(nets, g, end));
        let pt = |p: f64| TravellingPoint::new(a, b, p, false);
        let b1 = CarriageBogey::new(BogeyType::small(), false, vec![], pt(x), pt(x - 2.0), 0);
        let b2 = CarriageBogey::new(BogeyType::small(), false, vec![], pt(x - 6.0), pt(x - 8.0), 0);
        let c1 = Carriage::new(1, b1, None, 0);
        let c2 = Carriage::new(2, b2, None, 0);
        Train::new(id, None, Some(g), vec![c1, c2], vec![6], false, "Two", "create:traditional", 0)
    }

    fn setup(end: f64) -> (Railways, Uuid) {
        let mut rw = Railways::new();
        let g = line(&mut rw.nets, &[0.0, end]);
        (rw, g)
    }

    fn manual_ticks(rw: &mut Railways, id: Uuid, n: usize) {
        for _ in 0..n {
            if let Some(t) = rw.trains.get_mut(&id) {
                t.manual_tick = true;
            }
            rw.tick(&W);
        }
    }

    #[test]
    fn config_numbers() {
        let (rw, g) = setup(100.0);
        let mut t = mk_train(&rw.nets, g, 1, 100.0, 10.0, true);
        assert_eq!(t.max_speed(), f64::from(28.0_f32 / 20.0));
        assert_eq!(t.max_turn_speed(), f64::from(14.0_f32 / 20.0));
        assert_eq!(t.acceleration(), f64::from(3.0_f32 / 400.0));
        t.fuel_ticks = 5;
        assert_eq!(t.max_speed(), 2.0);
        assert_eq!(t.max_turn_speed(), 1.0);
    }

    #[test]
    fn passive_slowdown_to_a_stop() {
        let (mut rw, g) = setup(300.0);
        let mut t = mk_train(&rw.nets, g, 1, 300.0, 10.0, true);
        t.speed = 0.02;
        rw.add_train(t);
        rw.tick(&W);
        let s = rw.trains[&1].speed;
        assert!((s - (0.02 - f64::from(3.0_f32 / 400.0))).abs() < 1e-9);
        for _ in 0..10 {
            rw.tick(&W);
        }
        assert_eq!(rw.trains[&1].speed, 0.0);
        assert!(rw.trains[&1].carriages[0].leading_point().position > 10.0);
    }

    #[test]
    fn end_of_track_blocks_and_reports() {
        let (mut rw, g) = setup(30.0);
        let mut t = mk_train(&rw.nets, g, 1, 30.0, 26.0, true);
        t.speed = 0.5;
        rw.add_train(t);
        manual_ticks(&mut rw, 1, 20);
        let t = &rw.trains[&1];
        assert_eq!(t.speed, 0.0);
        assert!(t.status.track);
        assert!(t.status.queued.iter().any(|m| matches!(m, StatusMessage::Info { key, .. } if key == "end_of_track")));
        assert!((t.carriages[0].leading_point().position - 30.0).abs() < 1e-3);
    }

    #[test]
    fn navigates_to_a_station_and_stops() {
        let (mut rw, g) = setup(200.0);
        let station = rw.nets.new_uuid();
        let (a, b) = (loc(0.0, 0.0), loc(200.0, 0.0));
        rw.nets.add_point(g, TrackEdgePoint::new_station(station, [a, b], 120.0));
        let t = mk_train(&rw.nets, g, 1, 200.0, 11.0, true);
        rw.add_train(t);
        let path = DiscoveredPath { path: vec![], destination: station, distance: 109.0, cost: 109.0 };
        let mut nets = std::mem::take(&mut rw.nets);
        let cost = rw.trains.get_mut(&1).unwrap().start_navigation(&mut nets, Some(path));
        rw.nets = nets;
        assert_eq!(cost, 109.0);
        let mut arrived = false;
        for _ in 0..1200 {
            rw.tick(&W);
            if rw.trains[&1].current_station == Some(station) {
                arrived = true;
                break;
            }
        }
        assert!(arrived);
        let t = &rw.trains[&1];
        assert_eq!(t.speed, 0.0);
        assert!(t.navigation.destination.is_none());
        let p = t.carriages[0].leading_point().position;
        assert!((p - 120.0).abs() < 1.0, "stopped at {p}");
        assert!(rw.events.iter().any(|e| matches!(e, TrainEvent::MailTransfer { .. })));
    }

    #[test]
    fn head_on_trains_crash() {
        let (mut rw, g) = setup(200.0);
        let mut a = mk_train(&rw.nets, g, 1, 200.0, 50.0, true);
        a.speed = 0.5;
        let b = mk_train(&rw.nets, g, 2, 200.0, 62.0, false);
        rw.add_train(a);
        rw.add_train(b);
        let mut crashed = false;
        for _ in 0..60 {
            manual_ticks(&mut rw, 1, 1);
            if rw.trains[&1].derailed {
                crashed = true;
                break;
            }
        }
        assert!(crashed);
        assert!(rw.trains[&2].derailed);
        assert!(rw.trains[&1].graph.is_none() && rw.trains[&2].graph.is_none());
        assert!(rw.events.iter().any(|e| matches!(e, TrainEvent::Explosion { .. })));
        assert_eq!(rw.events.iter().filter(|e| matches!(e, TrainEvent::Crashed { .. })).count(), 2);
        assert!(rw.trains[&1].speed <= 0.0);
        assert!(rw.trains[&1].status.queued.iter().any(|m| matches!(m, StatusMessage::Crash { .. })));
    }

    #[test]
    fn occupying_a_group_lists_the_train() {
        let (mut rw, g) = setup(100.0);
        let mut t = mk_train(&rw.nets, g, 1, 100.0, 10.0, true);
        t.update_signal_blocks = true;
        rw.add_train(t);
        rw.tick(&W);
        let group = &rw.nets.signal_edge_groups[&g];
        assert!(group.trains.contains(&1));
        assert!(rw.trains[&1].occupied_signal_blocks.contains_key(&g));
    }

    #[test]
    fn stall_holds_the_speed() {
        let (mut rw, g) = setup(300.0);
        let mut t = mk_train(&rw.nets, g, 1, 300.0, 10.0, true);
        t.speed = 0.4;
        t.carriages[0].stalled = true;
        rw.add_train(t);
        manual_ticks(&mut rw, 1, 1);
        assert_eq!(rw.trains[&1].speed, 0.0);
        assert_eq!(rw.trains[&1].speed_before_stall, Some(0.4));
        let p = rw.trains[&1].carriages[0].leading_point().position;
        assert!((p - 10.0).abs() < 1e-6);
        rw.trains.get_mut(&1).unwrap().carriages[0].stalled = false;
        manual_ticks(&mut rw, 1, 1);
        assert_eq!(rw.trains[&1].speed_before_stall, None);
        assert!(rw.trains[&1].speed > 0.0);
    }

    #[test]
    fn carriages_follow_each_other() {
        let (mut rw, g) = setup(300.0);
        let mut t = two_carriage_train(&rw.nets, g, 1, 300.0, 20.0);
        t.speed = 0.4;
        rw.add_train(t);
        manual_ticks(&mut rw, 1, 30);
        let t = &rw.trains[&1];
        assert!(!t.derailed);
        let lead = t.carriages[0].leading_point().position;
        let trail = t.carriages[1].trailing_point().position;
        assert!(lead > 30.0);
        assert!((lead - trail - 8.0).abs() < 1.5, "lead {lead} trail {trail}");
        assert!(t.stress.iter().all(|s| s.abs() < 1.5));
    }

    #[test]
    fn total_length_and_backwards_points() {
        let (rw, g) = setup(300.0);
        let t = two_carriage_train(&rw.nets, g, 1, 300.0, 20.0);
        // 2/2 + 2/2 + bogey spacings 0 + carriage spacing 6
        assert_eq!(t.total_length(), 1 + 1 + 6);
        let back = t.travelling_points_backwards();
        assert_eq!(back.len(), 2 * 2);
        assert_eq!(back[0].1, 0.0);
        assert_eq!(back[1].1, 2.0);
        // The first carriage's trailing end: spacing 6 - last offset 1 - 2/2.
        assert_eq!(back[2].1, 6.0 - 1.0 - 1.0);
    }

    #[test]
    fn disassembly_places_carriages_along_the_direction() {
        let (rw, g) = setup(300.0);
        let mut t = two_carriage_train(&rw.nets, g, 1, 300.0, 20.0);
        let places = t.disassembly_positions([1, 0, 0], [10, 5, 10]);
        assert_eq!(places, vec![(0, [11, 5, 10]), (1, [17, 5, 10])]);
        t.currently_backwards = true;
        let places = t.disassembly_positions([1, 0, 0], [10, 5, 10]);
        assert_eq!(places[0].0, 1);
        assert_eq!(places[1].0, 0);
        assert_eq!(places[0].1, [11, 5, 10]);
        assert_eq!(places[1].1, [17, 5, 10]);
        let upright = |_: &Carriage| Some((0.0f32, 180.0f32));
        assert!(t.can_disassemble(&upright));
        let tilted = |_: &Carriage| Some((10.0f32, 0.0f32));
        assert!(!t.can_disassemble(&tilted));
        let skew = |_: &Carriage| Some((0.0f32, 45.0f32));
        assert!(!t.can_disassemble(&skew));
        let none = |_: &Carriage| None;
        assert!(!t.can_disassemble(&none));
    }

    #[test]
    fn railways_disassemble_removes_the_train() {
        let (mut rw, g) = setup(300.0);
        let t = two_carriage_train(&rw.nets, g, 7, 300.0, 20.0);
        rw.add_train(t);
        let tilted = |_: &Carriage| Some((10.0f32, 0.0f32));
        assert!(rw.disassemble_train(7, [1, 0, 0], [0, 0, 0], &tilted).is_none());
        assert!(rw.trains.contains_key(&7));
        let ok = |_: &Carriage| Some((0.0f32, 0.0f32));
        let plan = rw.disassemble_train(7, [1, 0, 0], [0, 0, 0], &ok).unwrap();
        assert_eq!(plan.placements.len(), 2);
        assert_eq!(plan.train_name, "Two");
        assert!(rw.trains.is_empty());
        assert!(rw.events.contains(&TrainEvent::TrainRemoved { train: 7 }));
    }

    #[test]
    fn invalid_trains_are_dropped() {
        let (mut rw, g) = setup(300.0);
        let mut t = mk_train(&rw.nets, g, 1, 300.0, 10.0, true);
        t.invalid = true;
        rw.add_train(t);
        let mut u = mk_train(&rw.nets, g, 2, 300.0, 100.0, true);
        u.navigation.waiting_for_signal = Some((5, true));
        rw.add_train(u);
        rw.tick(&W);
        assert!(!rw.trains.contains_key(&1));
        assert!(rw.events.contains(&TrainEvent::TrainRemoved { train: 1 }));
        assert!(rw.trains.contains_key(&2));
        assert!(rw.is_waiting(2));
        // Once the wait is over it goes back to the moving list.
        rw.trains.get_mut(&2).unwrap().navigation.waiting_for_signal = None;
        rw.tick(&W);
        assert!(!rw.is_waiting(2));
    }

    #[test]
    fn status_queue_keeps_three() {
        let mut s = TrainStatus::default();
        s.failed_navigation();
        s.failed_navigation(); // already set: no second message
        assert_eq!(s.queued.len(), 1);
        s.manual_controls();
        s.manual_controls();
        s.manual_controls();
        assert_eq!(s.queued.len(), 3);
        assert!(matches!(&s.queued[0], StatusMessage::Info { key, .. } if key == "paused_for_manual"));
        s.successful_navigation();
        assert!(!s.navigation);
        s.missing_conductor();
        assert!(s.conductor);
        s.found_conductor();
        assert!(!s.conductor);
        let taken = s.take_messages();
        assert!(!taken.is_empty() && s.queued.is_empty());
    }

    #[test]
    fn navigation_penalty_follows_state() {
        let (rw, g) = setup(300.0);
        let mut t = mk_train(&rw.nets, g, 1, 300.0, 10.0, true);
        assert_eq!(t.navigation_penalty(), 700);
        t.manual_tick = true;
        assert_eq!(t.navigation_penalty(), 200);
    }

    #[test]
    fn burning_fuel_takes_an_item() {
        let (rw, g) = setup(300.0);
        let mut t = mk_train(&rw.nets, g, 1, 300.0, 10.0, true);
        t.carriages[0].storage.fuel = vec![FuelSlot { count: 2, burn_time: 1600 }];
        NavTrain::burn_fuel(&mut t);
        assert_eq!(t.fuel_ticks, 1600);
        assert_eq!(t.carriages[0].storage.fuel[0].count, 1);
        assert_eq!(t.carriages[0].storage.version, 1);
        NavTrain::burn_fuel(&mut t);
        assert_eq!(t.fuel_ticks, 1599);
        assert_eq!(t.carriages[0].storage.fuel[0].count, 1);
    }

    #[test]
    fn detach_and_reattach_a_train() {
        let (mut rw, g) = setup(300.0);
        let t = mk_train(&rw.nets, g, 1, 300.0, 50.0, true);
        rw.add_train(t);
        let graph = rw.nets.graphs[&g].clone();
        rw.trains.get_mut(&1).unwrap().detach_from_tracks(&graph, &[]);
        assert!(rw.trains[&1].graph.is_none());
        assert_eq!(rw.trains[&1].migrating_points.len(), 2);
        rw.tick(&W);
        let t = &rw.trains[&1];
        assert_eq!(t.graph, Some(g));
        assert!(t.migrating_points.is_empty());
        assert!(!t.derailed);
        assert!((t.carriages[0].leading_point().position - 50.0).abs() < 1e-6);
    }

    #[test]
    fn migration_without_a_graph_derails_with_a_cooldown() {
        let (mut rw, g) = setup(300.0);
        let t = mk_train(&rw.nets, g, 1, 300.0, 50.0, true);
        rw.add_train(t);
        let graph = rw.nets.graphs[&g].clone();
        rw.trains.get_mut(&1).unwrap().detach_from_tracks(&graph, &[]);
        rw.nets.graphs.clear();
        rw.tick(&W);
        let t = &rw.trains[&1];
        assert!(t.derailed);
        assert_eq!(t.migration_cooldown, 40);
        assert!(t.status.track);
    }

    #[test]
    fn graph_events_move_trains() {
        let (mut rw, g) = setup(300.0);
        let t = mk_train(&rw.nets, g, 1, 300.0, 50.0, true);
        rw.add_train(t);
        rw.apply_rail_events(&[RailEvent::GraphTransferred { from: g, to: 99 }]);
        assert_eq!(rw.trains[&1].graph, Some(99));
        rw.trains.get_mut(&1).unwrap().graph = Some(g);
        let edge = (node(&rw.nets, g, 0.0), node(&rw.nets, g, 300.0));
        rw.apply_rail_events(&[RailEvent::SignalBlocksStale { graph: g, edges: vec![edge] }]);
        assert!(rw.trains[&1].update_signal_blocks);
    }

    #[test]
    fn observers_stay_active_while_a_train_is_over_them() {
        let (mut rw, g) = setup(300.0);
        let ob = rw.nets.new_uuid();
        let (a, b) = (loc(0.0, 0.0), loc(300.0, 0.0));
        rw.nets.add_point(g, TrackEdgePoint::new_observer(ob, [a, b], 50.0));
        let mut t = mk_train(&rw.nets, g, 1, 300.0, 49.0, true);
        t.speed = 0.4;
        rw.add_train(t);
        let mut seen = false;
        for _ in 0..40 {
            manual_ticks(&mut rw, 1, 1);
            let o = rw.nets.graphs[&g].point(EdgePointType::Observer, ob).unwrap().observer().unwrap();
            if o.is_activated() {
                seen = true;
                assert_eq!(o.current_train(), Some(1));
            }
        }
        assert!(seen);
    }
}
