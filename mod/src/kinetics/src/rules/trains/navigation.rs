//! `Navigation` (content/trains/entity/Navigation.java) and `DiscoveredPath`
//! (content/trains/graph/DiscoveredPath.java): a train finding a way to a station over the track
//! graph, and driving it.
//!
//! [`search`] is Create's Dijkstra over edges. Each frontier entry has a distance and a penalty
//! (signals that are red, stations, other trains, see [`Penalties`]) and an octile-distance
//! estimate to the nearest destination; the frontier is ordered by their sum. Java's
//! `PriorityQueue` breaks ties by its heap layout, so [`JavaPriorityQueue`] reproduces it. Edges
//! are named by their `(from, to)` node ids; Java's identity maps over edges are maps over those.
//!
//! The train is not in this module. Everything Navigation asks of it is [`NavTrain`] (carriage
//! points, conductors, speed, status); the other trains on the graph come as [`OtherTrain`]s and
//! the station side as a [`TrainStates`]. [`Navigation::tick`] drives the train: it scouts the
//! signals ahead (reserving blocks, waiting at red), works out the braking and curve speeds, and
//! sets the train's target speed. Java's `travel` closures mutate fields of `Navigation`; here they
//! work on locals copied back afterwards, which gives the same result.
//!
//! Where Java walks a `HashMap` (a node's connections, other trains) the order is not defined; here
//! it is by node id.
//!
//! Left out: `write`/`read` (NBT), `Create.RAILWAYS.trains` (the caller lists the others),
//! `TrackMaterial.TrackType` beyond an id string.

use super::graph::{EdgePointType, NodeId, TrackEdge, TrackEdgePoint, TrackNodeLocation, Uuid};
use super::network::{RailNetworks, TrackGraph};
use super::signal::SignalType;
use super::station::TrainStates;
use super::travelling_point::{Candidate, EdgePointHit, Listeners, SteerDirection, TrackSelector, TravellingPoint};
use super::vec::{distance_to, scale};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

/// `Train.Penalties`.
pub struct Penalties;

impl Penalties {
    pub const STATION: i32 = 50;
    pub const STATION_WITH_TRAIN: i32 = 300;
    pub const MANUAL_TRAIN: i32 = 200;
    pub const IDLE_TRAIN: i32 = 700;
    pub const ARRIVING_TRAIN: i32 = 50;
    pub const WAITING_TRAIN: i32 = 50;
    pub const ANY_TRAIN: i32 = 25;
    pub const RED_SIGNAL: i32 = 25;
    pub const REDSTONE_RED_SIGNAL: i32 = 400;
}

/// `TrackMaterial.TrackType.STANDARD`'s id: every track Create adds is of it.
pub const TRACK_TYPE_STANDARD: &str = "create:standard";

/// The octile distance factors of the remaining distance estimate.
const OCTILE_MIN: f64 = 0.317837245195782;
const OCTILE_MID: f64 = 0.414213562373095;

/// `Train.getNavigationPenalty()`'s inputs.
#[derive(Clone, Copy, Debug, Default)]
pub struct PenaltyInput {
    pub manual_tick: bool,
    pub has_schedule: bool,
    pub paused: bool,
    pub waiting_for_signal: bool,
    pub ticks_waiting_for_signal: i32,
    pub has_destination: bool,
    pub distance_to_destination: f64,
    pub distance_to_signal: f64,
}

/// `Train.getNavigationPenalty()`: how much another train's search should avoid the edges this one
/// stands on.
pub fn navigation_penalty(i: &PenaltyInput) -> i32 {
    if i.manual_tick {
        return Penalties::MANUAL_TRAIN;
    }
    if !i.has_schedule || i.paused {
        return Penalties::IDLE_TRAIN;
    }
    if i.waiting_for_signal && i.ticks_waiting_for_signal > 0 {
        return Penalties::WAITING_TRAIN + (i.ticks_waiting_for_signal / 20).min(1000);
    }
    if i.has_destination && i.distance_to_destination < 50.0 || i.distance_to_signal < 20.0 {
        return Penalties::ARRIVING_TRAIN;
    }
    Penalties::ANY_TRAIN
}

/// The track types a train can travel on (`search`'s `validTypes`): the types every carriage's
/// leading bogey allows, and its trailing bogey too where it has two. Each carriage is
/// `(leading bogey's types, trailing bogey's types if on two bogeys)`.
pub fn carriage_valid_types(carriages: &[(Vec<String>, Option<Vec<String>>)]) -> BTreeSet<String> {
    let mut valid: BTreeSet<String> = BTreeSet::new();
    for (i, (leading, trailing)) in carriages.iter().enumerate() {
        if i == 0 {
            valid.extend(leading.iter().cloned());
        } else {
            valid.retain(|t| leading.contains(t));
        }
        if let Some(trailing) = trailing {
            valid.retain(|t| trailing.contains(t));
        }
    }
    valid
}

/// `DiscoveredPath`.
#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveredPath {
    /// The edges to take where the way forks, as `(node1, node2)`.
    pub path: Vec<(NodeId, NodeId)>,
    /// The station's edge point id.
    pub destination: Uuid,
    /// Negative when the destination is behind the train.
    pub distance: f64,
    pub cost: f64,
}

/// What the train's status reports (`train.status.*`) about its conductors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConductorStatus {
    MissingConductor,
    MissingCorrectConductor,
    FoundConductor,
}

/// What Navigation uses of `Train`.
pub trait NavTrain {
    fn id(&self) -> Uuid;
    /// `train.graph`.
    fn graph(&self) -> Option<Uuid>;
    /// `carriages.get(0).getLeadingPoint()`.
    fn leading_point(&self) -> &TravellingPoint;
    /// `carriages.get(last).getTrailingPoint()`.
    fn trailing_point(&self) -> &TravellingPoint;
    /// See [`carriage_valid_types`].
    fn valid_track_types(&self) -> BTreeSet<String>;
    fn has_forward_conductor(&self) -> bool;
    fn has_backward_conductor(&self) -> bool;
    fn double_ended(&self) -> bool;
    /// `runtime.paused`.
    fn paused(&self) -> bool;
    fn manual_tick(&self) -> bool;
    /// `getCurrentStation()`'s id.
    fn current_station(&self) -> Option<Uuid>;
    /// `currentStation = null` (`leaveStation` after telling the station).
    fn clear_current_station(&mut self);
    fn speed(&self) -> f64;
    fn set_speed(&mut self, speed: f64);
    fn throttle(&self) -> f64;
    fn acceleration(&self) -> f64;
    fn max_speed(&self) -> f64;
    fn max_turn_speed(&self) -> f64;
    fn set_target_speed(&mut self, speed: f64);
    /// `approachTargetSpeed(accelerationMod)`.
    fn approach_target_speed(&mut self, acceleration_mod: f64);
    fn burn_fuel(&mut self);
    fn set_currently_backwards(&mut self, backwards: bool);
    fn report_conductors(&mut self, status: ConductorStatus);
    /// `reservedSignalBlocks`.
    fn reserved_signal_blocks(&mut self) -> &mut BTreeSet<Uuid>;
    /// `runtime.transitInterrupted()`.
    fn transit_interrupted(&mut self);
}

/// Another train on the graph, as a search sees it for the penalties on edges.
#[derive(Clone, Debug)]
pub struct OtherTrain {
    pub id: Uuid,
    pub graph: Uuid,
    /// `getNavigationPenalty()`.
    pub navigation_penalty: i32,
    /// `getEndpointEdges()`: the nodes of the edge under the leading point and under the trailing point.
    pub endpoint_edges: [(Option<NodeId>, Option<NodeId>); 2],
}

/// What a search reads besides the train.
pub struct SearchEnv<'a> {
    pub nets: &'a RailNetworks,
    pub trains: &'a dyn TrainStates,
    pub others: &'a [OtherTrain],
    /// An edge's material id to its track type id (`getTrackMaterial().trackType.id`). Every
    /// material in Create is [`TRACK_TYPE_STANDARD`].
    pub track_type_of: &'a dyn Fn(Option<&str>) -> String,
}

/// `Navigation.StationTest`'s arguments.
pub struct StationHit<'a> {
    pub distance: f64,
    pub cost: f64,
    pub reached_via: &'a ReachedVia,
    /// The edge the station is on, as `(node1, node2)` in the direction travelled.
    pub edge: (NodeId, NodeId),
    pub station: &'a TrackEdgePoint,
}

/// `reachedVia`: for each edge reached, whether the way there passed a fork, and the edge before it.
pub type ReachedVia = BTreeMap<(NodeId, NodeId), (bool, (NodeId, NodeId))>;

// --- java.util.PriorityQueue ------------------------------------------------------------------

/// `java.util.PriorityQueue`'s heap (`offer`, `poll`), so entries with equal keys leave in the order
/// Java gives.
struct JavaPriorityQueue<T> {
    q: Vec<T>,
    cmp: fn(&T, &T) -> Ordering,
}

impl<T> JavaPriorityQueue<T> {
    fn new(cmp: fn(&T, &T) -> Ordering) -> Self {
        JavaPriorityQueue { q: Vec::new(), cmp }
    }

    fn is_empty(&self) -> bool {
        self.q.is_empty()
    }

    fn add(&mut self, x: T) {
        let mut k = self.q.len();
        self.q.push(x);
        // siftUp
        while k > 0 {
            let parent = (k - 1) >> 1;
            if (self.cmp)(&self.q[k], &self.q[parent]) != Ordering::Less {
                break;
            }
            self.q.swap(k, parent);
            k = parent;
        }
    }

    fn poll(&mut self) -> Option<T> {
        if self.q.is_empty() {
            return None;
        }
        let n = self.q.len() - 1;
        let result = self.q.swap_remove(0);
        // `swap_remove` moved the last element to the root: that is Java's `x`. siftDown it.
        if n > 1 {
            let mut k = 0;
            let half = n >> 1;
            while k < half {
                let mut child = 2 * k + 1;
                let right = child + 1;
                if right < n && (self.cmp)(&self.q[child], &self.q[right]) == Ordering::Greater {
                    child = right;
                }
                if (self.cmp)(&self.q[k], &self.q[child]) != Ordering::Greater {
                    break;
                }
                self.q.swap(k, child);
                k = child;
            }
        }
        Some(result)
    }
}

/// `FrontierEntry`.
struct FrontierEntry {
    distance: f64,
    penalty: i32,
    remaining: f64,
    has_destination: bool,
    node1: NodeId,
    node2: NodeId,
}

fn frontier_cmp(a: &FrontierEntry, b: &FrontierEntry) -> Ordering {
    (a.distance + f64::from(a.penalty) + a.remaining).total_cmp(&(b.distance + f64::from(b.penalty) + b.remaining))
}

// --- search -----------------------------------------------------------------------------------

/// `Mth.clamp(int, min, max)`.
fn clamp_i(v: i32, min: i32, max: i32) -> i32 {
    if v < min {
        min
    } else if v > max {
        max
    } else {
        v
    }
}

/// `Mth.clamp(double, min, max)` (unlike `f64::clamp`, no panic when `min > max`).
fn clamp_f(v: f64, min: f64, max: f64) -> f64 {
    if v < min {
        min
    } else if v > max {
        max
    } else {
        v
    }
}

/// `Navigation.search(maxDistance, maxCost, forward, destinations, stationTest)`: Dijkstra from the
/// train's leading point (`forward`) or trailing point over the edges, calling `station_test` at
/// each station that can be approached. It returns when the test says true. `maxCost` of -1 (or any
/// negative) ignores penalties from other trains and bounds nothing. `destinations` are station ids
/// the remaining-distance estimate aims at.
#[allow(clippy::too_many_arguments)]
pub fn search(
    env: &SearchEnv,
    train: &dyn NavTrain,
    ticks_waiting_for_signal: i32,
    max_distance: f64,
    max_cost: f64,
    forward: bool,
    destinations: Option<&[Uuid]>,
    station_test: &mut dyn FnMut(&StationHit) -> bool,
) {
    let Some(graph_id) = train.graph() else { return };
    let Some(graph) = env.nets.graphs.get(&graph_id) else { return };

    // The track types the train can travel on.
    let valid_types = train.valid_track_types();
    if valid_types.is_empty() {
        return; // no valid track types, a route can't be found
    }

    let mut penalties: BTreeMap<(NodeId, NodeId), i32> = BTreeMap::new();
    let cost_relevant = max_cost >= 0.0;
    if cost_relevant {
        for other in env.others {
            if other.graph != graph_id || other.id == train.id() {
                continue;
            }
            for nodes in other.endpoint_edges {
                let (Some(a), Some(b)) = nodes else { continue };
                for flip in [true, false] {
                    let key = if flip { (b, a) } else { (a, b) };
                    if graph.edge(key.0, key.1).is_none() {
                        continue;
                    }
                    *penalties.entry(key).or_insert(0) += other.navigation_penalty / 2;
                }
            }
        }
    }

    let starting_point = if forward { train.leading_point() } else { train.trailing_point() };

    let mut visited: BTreeSet<(NodeId, NodeId)> = BTreeSet::new();
    let mut reached_via: ReachedVia = BTreeMap::new();
    let mut frontier = JavaPriorityQueue::new(frontier_cmp);

    let (initial_node1, initial_node2) =
        if forward { (starting_point.node1, starting_point.node2) } else { (starting_point.node2, starting_point.node1) };
    let (Some(initial_node1), Some(initial_node2)) = (initial_node1, initial_node2) else { return };
    let Some(initial_edge) = graph.edge(initial_node1, initial_node2) else { return };

    let distance_to_node2 =
        if forward { initial_edge.length() - starting_point.position } else { starting_point.position };

    let mut signal_weight = clamp_i(ticks_waiting_for_signal.saturating_mul(2), Penalties::RED_SIGNAL, 200);

    let loc_of = |n: NodeId| -> Option<TrackNodeLocation> { graph.get_node(n).map(|n| n.location) };

    // Apply penalties to the initial edge.
    let mut initial_penalty = 0;
    if cost_relevant {
        initial_penalty += penalties.get(&(initial_node1, initial_node2)).copied().unwrap_or(0);
    }
    let Some(initial_node2_loc) = loc_of(initial_node2) else { return };

    if initial_edge.edge_data.has_points() {
        for r in &initial_edge.edge_data.points {
            let Some(point) = graph.edge_points.get(r.ty, r.id) else { continue };
            if point.location_on(initial_edge) < initial_edge.length() - distance_to_node2 {
                continue;
            }
            if cost_relevant && distance_to_node2 + f64::from(initial_penalty) > max_cost {
                return;
            }
            if !point.can_navigate_via(&initial_node2_loc) {
                return;
            }
            if let Some(signal) = point.signal() {
                let primary = point.is_primary(&initial_node2_loc);
                if signal.is_forced_red(primary) {
                    initial_penalty += Penalties::REDSTONE_RED_SIGNAL;
                    continue;
                }
                let Some(group) = signal.group(primary) else { continue };
                if !env.nets.signal_edge_groups.contains_key(&group) {
                    continue;
                }
                if env.nets.is_group_occupied_unless_boundary(group, point.id) {
                    initial_penalty += signal_weight;
                    signal_weight /= 2;
                }
            }
            if let Some(_station) = point.station() {
                let present = point_station_present(point, env);
                let is_own_station = present == Some(train.id());
                if present.is_some() && !is_own_station {
                    initial_penalty += Penalties::STATION_WITH_TRAIN;
                }
                if point.can_approach_from(&initial_node2_loc)
                    && station_test(&StationHit {
                        distance: distance_to_node2,
                        cost: distance_to_node2 + f64::from(initial_penalty),
                        reached_via: &reached_via,
                        edge: (initial_node1, initial_node2),
                        station: point,
                    })
                {
                    return;
                }
                if !is_own_station {
                    initial_penalty += Penalties::STATION;
                }
            }
        }
    }

    if cost_relevant && distance_to_node2 + f64::from(initial_penalty) > max_cost {
        return;
    }

    frontier.add(FrontierEntry {
        distance: distance_to_node2,
        penalty: initial_penalty,
        remaining: 0.0,
        has_destination: false,
        node1: initial_node1,
        node2: initial_node2,
    });

    while !frontier.is_empty() {
        let Some(entry) = frontier.poll() else { break };
        if !visited.insert((entry.node1, entry.node2)) {
            continue;
        }

        let distance = entry.distance;
        let penalty = entry.penalty;

        if distance > max_distance {
            continue;
        }

        let (node1, node2) = (entry.node1, entry.node2);
        let Some(edge) = graph.edge(node1, node2) else { continue };
        let Some(node2_loc) = loc_of(node2) else { continue };

        if entry.has_destination && edge.edge_data.has_points() {
            for r in &edge.edge_data.points {
                let Some(point) = graph.edge_points.get(r.ty, r.id) else { continue };
                if point.station().is_some()
                    && point.can_approach_from(&node2_loc)
                    && station_test(&StationHit {
                        distance,
                        cost: f64::from(penalty),
                        reached_via: &reached_via,
                        edge: (node1, node2),
                        station: point,
                    })
                {
                    return;
                }
            }
        }

        let mut valid_targets: Vec<(NodeId, &TrackEdge)> = Vec::new();
        if let Some(connections) = graph.connections_from(node2) {
            for (new_node, new_edge) in connections {
                if *new_node == node1 {
                    continue;
                }
                if edge.can_travel_to(new_edge) {
                    valid_targets.push((*new_node, new_edge));
                }
            }
        }

        if valid_targets.is_empty() {
            continue;
        }

        'search: for (new_node, new_edge) in &valid_targets {
            let new_node = *new_node;
            if !valid_types.contains(&(env.track_type_of)(new_edge.material.as_deref())) {
                continue;
            }
            let mut new_penalty = penalty;
            let edge_length = new_edge.length();
            let new_distance = distance + edge_length;

            if cost_relevant {
                new_penalty += penalties.get(&(node2, new_node)).copied().unwrap_or(0);
            }
            let Some(new_node_loc) = loc_of(new_node) else { continue };

            // Apply penalty to the next connected edge.
            let mut has_destination = false;
            if new_edge.edge_data.has_points() {
                for r in &new_edge.edge_data.points {
                    let Some(point) = graph.edge_points.get(r.ty, r.id) else { continue };
                    if node2 == initial_node1 && point.location_on(new_edge) < edge_length - distance_to_node2 {
                        continue;
                    }
                    if cost_relevant && new_distance + f64::from(new_penalty) > max_cost {
                        continue 'search;
                    }
                    if !point.can_navigate_via(&new_node_loc) {
                        continue 'search;
                    }
                    if let Some(signal) = point.signal() {
                        let primary = point.is_primary(&new_node_loc);
                        if signal.is_forced_red(primary) {
                            new_penalty += Penalties::REDSTONE_RED_SIGNAL;
                            continue;
                        }
                        let Some(group) = signal.group(primary) else { continue };
                        if !env.nets.signal_edge_groups.contains_key(&group) {
                            continue;
                        }
                        if env.nets.is_group_occupied_unless_boundary(group, point.id) {
                            new_penalty += signal_weight;
                            signal_weight /= 2;
                        }
                    }
                    if point.station().is_some() {
                        let present = point_station_present(point, env);
                        let is_own_station = present == Some(train.id());
                        if present.is_some() && !is_own_station {
                            new_penalty += Penalties::STATION_WITH_TRAIN;
                        }
                        if point.can_approach_from(&new_node_loc)
                            && station_test(&StationHit {
                                distance: new_distance,
                                cost: new_distance + f64::from(new_penalty),
                                reached_via: &reached_via,
                                edge: (node2, new_node),
                                station: point,
                            })
                        {
                            has_destination = true;
                            break;
                        }
                        if !is_own_station {
                            new_penalty += Penalties::STATION;
                        }
                    }
                }
            }

            if cost_relevant && new_distance + f64::from(new_penalty) > max_cost {
                continue;
            }

            let mut remaining_dist = 0.0;
            // The remaining distance estimate for the next connected edge.
            if let Some(destinations) = destinations.filter(|d| !d.is_empty()) {
                remaining_dist = f64::MAX;
                let new_node_position = new_node_loc.location();
                for dest_id in destinations {
                    let Some(destination) = graph.edge_points.get(EdgePointType::Station, *dest_id) else { continue };
                    let destination_node = destination.edge_location[0];
                    let dest_pos = destination_node.location();
                    let mut d_min = (new_node_position[0] - dest_pos[0]).abs();
                    let mut d_mid = (new_node_position[1] - dest_pos[1]).abs();
                    let mut d_max = (new_node_position[2] - dest_pos[2]).abs();
                    // Sort the distance vector in ascending order.
                    if d_min > d_mid {
                        std::mem::swap(&mut d_min, &mut d_mid);
                    }
                    if d_min > d_max {
                        std::mem::swap(&mut d_min, &mut d_max);
                    }
                    if d_mid > d_max {
                        std::mem::swap(&mut d_mid, &mut d_max);
                    }
                    // Octile distance from the new node to the station node.
                    let mut current_remaining = OCTILE_MIN * d_min + OCTILE_MID * d_mid + d_max + destination.position;
                    if node2_loc == destination_node {
                        current_remaining -= new_edge.length() * 2.0; // correct the estimate for the station's edge
                    }
                    remaining_dist = remaining_dist.min(current_remaining);
                }
            }

            reached_via.entry((node2, new_node)).or_insert((valid_targets.len() > 1, (node1, node2)));
            frontier.add(FrontierEntry {
                distance: new_distance,
                penalty: new_penalty,
                remaining: remaining_dist,
                has_destination,
                node1: node2,
                node2: new_node,
            });
        }
    }
}

/// `station.getPresentTrain()` for the station edge point `point`.
fn point_station_present(point: &TrackEdgePoint, env: &SearchEnv) -> Option<Uuid> {
    point.station()?.present_train(point.id, env.trains)
}

/// `findNearestApproachable`'s `search(maxDistance, forward, null, ...)`: no cost limit.
fn search_nearest(
    env: &SearchEnv,
    train: &dyn NavTrain,
    ticks: i32,
    max_distance: f64,
    forward: bool,
    test: &mut dyn FnMut(&StationHit) -> bool,
) {
    search(env, train, ticks, max_distance, -1.0, forward, None, test);
}

/// `navigateOptions(path, graph, options)`: at a fork, the option that is the next edge of the path
/// (taken off it); else the first.
pub fn navigate_options(path: &mut Vec<(NodeId, NodeId)>, graph: &TrackGraph, options: &[Candidate]) -> usize {
    if path.is_empty() {
        return 0;
    }
    let target = graph.edge(path[0].0, path[0].1).map(|e| e.serial);
    for (i, c) in options.iter().enumerate() {
        if graph.edge(c.from, c.node).map(|e| e.serial) != target {
            continue;
        }
        path.remove(0);
        return i;
    }
    0
}

// --- Navigation -------------------------------------------------------------------------------

/// `Navigation`'s state.
#[derive(Clone, Debug, PartialEq)]
pub struct Navigation {
    /// `destination`: a station's edge point id.
    pub destination: Option<Uuid>,
    pub distance_to_destination: f64,
    pub distance_started_at: f64,
    pub destination_behind_train: bool,
    pub announce_arrival: bool,
    pub current_path: Vec<(NodeId, NodeId)>,
    signal_scout: TravellingPoint,
    /// `waitingForSignal`: the boundary's id and the side (`true` is the primary one).
    pub waiting_for_signal: Option<(Uuid, bool)>,
    /// Group id to the boundary entering it and its side.
    waiting_for_chained_groups: BTreeMap<Uuid, (Uuid, bool)>,
    pub distance_to_signal: f64,
    pub ticks_waiting_for_signal: i32,
}

impl Default for Navigation {
    fn default() -> Self {
        Self::new()
    }
}

impl Navigation {
    pub fn new() -> Navigation {
        Navigation {
            destination: None,
            distance_to_destination: 0.0,
            distance_started_at: 0.0,
            destination_behind_train: false,
            announce_arrival: false,
            current_path: Vec::new(),
            signal_scout: TravellingPoint::empty(),
            waiting_for_signal: None,
            waiting_for_chained_groups: BTreeMap::new(),
            distance_to_signal: 0.0,
            ticks_waiting_for_signal: 0,
        }
    }

    /// `isActive()`.
    pub fn is_active(&self) -> bool {
        self.destination.is_some()
    }

    /// The selector for the train's own points (`control(mp)` with a destination): at a fork it takes
    /// the next edge of the path.
    pub fn select_track(&mut self, graph: &TrackGraph, options: &[Candidate]) -> usize {
        navigate_options(&mut self.current_path, graph, options)
    }

    /// `control(mp)` without a destination: steer by the player's lever.
    pub fn manual_selector<'a>(manual_steer: SteerDirection) -> TrackSelector<'a> {
        TrackSelector::steer(manual_steer, [0.0, 1.0, 0.0])
    }

    /// `getNavigationPenalty()`'s inputs for this navigation.
    pub fn penalty_input(&self, manual_tick: bool, has_schedule: bool, paused: bool) -> PenaltyInput {
        PenaltyInput {
            manual_tick,
            has_schedule,
            paused,
            waiting_for_signal: self.waiting_for_signal.is_some(),
            ticks_waiting_for_signal: self.ticks_waiting_for_signal,
            has_destination: self.destination.is_some(),
            distance_to_destination: self.distance_to_destination,
            distance_to_signal: self.distance_to_signal,
        }
    }

    // --- starting and stopping ---

    /// `cancelNavigation()` on a graph the caller has in hand.
    fn cancel_in(&mut self, graph: Option<&mut TrackGraph>, train: &mut dyn NavTrain) {
        self.distance_to_destination = 0.0;
        self.current_path.clear();
        let Some(dest) = self.destination else { return };
        if let Some(station) = graph
            .and_then(|g| g.edge_points.get_mut(EdgePointType::Station, dest))
            .and_then(|p| p.station_mut())
        {
            station.cancel_reservation(train.id());
        }
        self.destination = None;
        train.transit_interrupted();
        train.reserved_signal_blocks().clear();
    }

    /// `cancelNavigation()`.
    pub fn cancel_navigation(&mut self, nets: &mut RailNetworks, train: &mut dyn NavTrain) {
        let graph = train.graph();
        self.cancel_in(graph.and_then(|g| nets.graphs.get_mut(&g)), train);
    }

    /// `startNavigation(pathTo)`: returns the cost, 0 when already going there, -1 when there is no
    /// path or the train lacks the conductor to drive it.
    pub fn start_navigation(&mut self, nets: &mut RailNetworks, train: &mut dyn NavTrain, path_to: Option<DiscoveredPath>) -> f64 {
        let none_found = path_to.is_none();
        let distance = path_to.as_ref().map_or(-1.0, |p| p.distance.abs());
        let cost = path_to.as_ref().map_or(-1.0, |p| p.cost);

        self.distance_to_destination = distance;

        let Some(path_to) = path_to else {
            debug_assert!(none_found);
            self.distance_to_destination = 0.0;
            self.distance_started_at = 0.0;
            self.current_path = Vec::new();
            if self.destination.is_some() {
                self.cancel_navigation(nets, train);
            }
            return -1.0;
        };

        if self.distance_to_destination.abs() > 100.0 {
            self.announce_arrival = true;
        }

        self.current_path = path_to.path.clone();
        self.destination_behind_train = path_to.distance < 0.0;
        train.reserved_signal_blocks().clear();
        self.waiting_for_signal = None;

        if self.destination.is_none() {
            self.distance_started_at = distance;
        }

        if self.destination == Some(path_to.destination) {
            return 0.0;
        }

        if !train.paused() {
            let front_driver = train.has_forward_conductor();
            let back_driver = train.has_backward_conductor();
            if self.destination_behind_train && !back_driver {
                train.report_conductors(if front_driver {
                    ConductorStatus::MissingCorrectConductor
                } else {
                    ConductorStatus::MissingConductor
                });
                return -1.0;
            }

            if !self.destination_behind_train && !front_driver {
                train.report_conductors(if back_driver {
                    ConductorStatus::MissingCorrectConductor
                } else {
                    ConductorStatus::MissingConductor
                });
                return -1.0;
            }

            train.report_conductors(ConductorStatus::FoundConductor);
        }

        self.destination = Some(path_to.destination);
        cost
    }

    // --- searching ---

    /// `findPathTo(destination, maxCost)`.
    pub fn find_path_to(&self, env: &SearchEnv, train: &dyn NavTrain, destination: Uuid, max_cost: f64) -> Option<DiscoveredPath> {
        self.find_path_to_any(env, train, &[destination], max_cost)
    }

    /// `findPathTo(destinations, maxCost)`: the best path to any of the stations, forward or backward.
    pub fn find_path_to_any(
        &self,
        env: &SearchEnv,
        train: &dyn NavTrain,
        destinations: &[Uuid],
        max_cost: f64,
    ) -> Option<DiscoveredPath> {
        let graph = env.nets.graphs.get(&train.graph()?)?;

        let mut results: [Option<DiscoveredPath>; 2] = [None, None];
        for forward in [true, false] {
            // When updating destinations midtransit, avoid reversing out of path.
            if self.destination.is_some() && self.destination_behind_train == forward {
                continue;
            }

            let initial_point = if forward { train.leading_point() } else { train.trailing_point() };
            let initial_edge = if forward {
                initial_point.node1.zip(initial_point.node2)
            } else {
                initial_point.node2.zip(initial_point.node1)
            };

            let slot = if forward { 0 } else { 1 };
            let mut test = |hit: &StationHit| -> bool {
                for destination in destinations {
                    if hit.station.id != *destination {
                        continue;
                    }
                    let edge = hit.edge;
                    let mut current_path: Vec<(NodeId, NodeId)> = Vec::new();
                    let mut back_track = hit.reached_via.get(&edge);
                    let mut to_reach = edge;
                    let mut edge_reached = edge;
                    while let Some(bt) = back_track {
                        if Some(edge_reached) == initial_edge {
                            break;
                        }
                        if bt.0 {
                            current_path.insert(0, to_reach);
                        }
                        to_reach = bt.1;
                        edge_reached = to_reach;
                        back_track = hit.reached_via.get(&edge_reached);
                    }

                    let Some(e) = graph.edge(edge.0, edge.1) else { return false };
                    let position = e.length() - hit.station.location_on(e);
                    let distance_to_destination = hit.distance - position;
                    results[slot] = Some(DiscoveredPath {
                        path: current_path,
                        destination: *destination,
                        distance: (if forward { 1.0 } else { -1.0 }) * distance_to_destination,
                        cost: hit.cost,
                    });
                    return true;
                }
                false
            };
            search(env, train, self.ticks_waiting_for_signal, f64::MAX, max_cost, forward, Some(destinations), &mut test);
        }

        let [front, back] = results;

        let front_empty = front.is_none();
        let back_empty = back.is_none();
        let can_drive_forward = train.has_forward_conductor() || train.paused();
        let can_drive_backward = train.double_ended() && train.has_backward_conductor() || train.paused();

        if back_empty || !can_drive_backward {
            return if can_drive_forward { front } else { None };
        }
        if front_empty || !can_drive_forward {
            return if can_drive_backward { back } else { None };
        }

        let (front, back) = (front?, back?);
        let front_better = if max_cost == -1.0 { -back.distance > front.distance } else { back.cost > front.cost };
        Some(if front_better { front } else { back })
    }

    /// `findNearestApproachable(forward)`: the nearest station the train can still stop at, if it
    /// brakes now, that no other train is standing at.
    pub fn find_nearest_approachable(&self, env: &SearchEnv, train: &dyn NavTrain, forward: bool) -> Option<Uuid> {
        let graph = env.nets.graphs.get(&train.graph()?)?;

        let mut result: Option<Uuid> = None;
        let acceleration = train.acceleration();
        let speed = train.speed();
        let min_distance = f64::from(0.75_f32) * (speed * speed) / (2.0 * acceleration);
        let max_distance = f64::max(32.0, f64::from(1.5_f32) * (speed * speed) / (2.0 * acceleration));

        let mut test = |hit: &StationHit| -> bool {
            if hit.distance < min_distance {
                return false;
            }
            let Some(edge) = graph.edge(hit.edge.0, hit.edge.1) else { return false };
            let position = edge.length() - hit.station.location_on(edge);
            if hit.distance - position < min_distance {
                return false;
            }
            let present = point_station_present(hit.station, env);
            if present.is_some() && present != Some(train.id()) {
                return false;
            }
            result = Some(hit.station.id);
            true
        };
        search_nearest(env, train, self.ticks_waiting_for_signal, max_distance, forward, &mut test);
        result
    }

    // --- ticking ---

    /// `currentSignalResolved()`: has the signal the train waits at cleared? A cross signal clears
    /// when every group in its chain is free.
    fn current_signal_resolved(&mut self, nets: &RailNetworks, graph: &TrackGraph, train: &mut dyn NavTrain) -> bool {
        if train.manual_tick() {
            return true;
        }
        if self.distance_to_destination < f64::from(0.5_f32) {
            return true;
        }
        let Some((signal_id, side)) = self.waiting_for_signal else { return true };
        let Some(signal) = graph.point(EdgePointType::Signal, signal_id).and_then(|p| p.signal()) else { return true };

        let idx = |b: bool| if b { 0 } else { 1 };
        // Cross signal.
        if signal.types[idx(side)] == SignalType::CrossSignal {
            for (group_id, (boundary_id, boundary_side)) in &self.waiting_for_chained_groups {
                if !nets.signal_edge_groups.contains_key(group_id) {
                    // Migration: re-initialise the chain.
                    self.waiting_for_signal = Some((0, side));
                    return true;
                }
                let forced_red = graph
                    .point(EdgePointType::Signal, *boundary_id)
                    .and_then(|p| p.signal())
                    .is_some_and(|s| s.is_forced_red(*boundary_side));
                if forced_red {
                    train.reserved_signal_blocks().clear();
                    return false;
                }
                if nets.is_group_occupied_unless_train(*group_id, train.id()) {
                    return false;
                }
            }
            return true;
        }

        // Entry signal.
        let Some(group_id) = signal.groups[idx(side)] else { return true };
        if !nets.signal_edge_groups.contains_key(&group_id) {
            return true;
        }
        !nets.is_group_occupied_unless_train(group_id, train.id())
    }

    /// `leaveStation()`: the station forgets the train, the train forgets the station.
    fn leave_station(&self, graph: &mut TrackGraph, train: &mut dyn NavTrain) {
        if let Some(station_id) = train.current_station() {
            if let Some(station) = graph.edge_points.get_mut(EdgePointType::Station, station_id).and_then(|p| p.station_mut()) {
                station.train_departed(train.id());
            }
        }
        train.clear_current_station();
    }

    /// `tick(level)`: scouts the signals ahead and sets the train's target speed. `trains` answers
    /// the station's questions about the other trains (for `reserveFor`).
    pub fn tick(&mut self, nets: &mut RailNetworks, train: &mut dyn NavTrain, trains: &dyn TrainStates) {
        if self.destination.is_none() {
            return;
        }
        // The graph is taken out of the networks for the tick, so that the signal groups in `nets`
        // can change while the scouting point reads it.
        let Some(gid) = train.graph() else {
            self.cancel_in(None, train);
            return;
        };
        let Some(mut graph) = nets.graphs.remove(&gid) else {
            self.cancel_in(None, train);
            return;
        };
        self.tick_on(nets, &mut graph, train, trains);
        nets.graphs.insert(gid, graph);
    }

    fn tick_on(&mut self, nets: &mut RailNetworks, graph: &mut TrackGraph, train: &mut dyn NavTrain, trains: &dyn TrainStates) {
        let Some(destination) = self.destination else { return };

        if !train.paused() {
            let front_driver = train.has_forward_conductor();
            let back_driver = train.has_backward_conductor();
            if self.destination_behind_train && !back_driver {
                train.report_conductors(if front_driver {
                    ConductorStatus::MissingCorrectConductor
                } else {
                    ConductorStatus::MissingConductor
                });
                self.cancel_in(Some(graph), train);
                return;
            }

            if !self.destination_behind_train && !front_driver {
                train.report_conductors(ConductorStatus::MissingConductor);
                self.cancel_in(Some(graph), train);
                return;
            }

            train.report_conductors(ConductorStatus::FoundConductor);
        }

        if let Some(station) = graph.edge_points.get_mut(EdgePointType::Station, destination).and_then(|p| p.station_mut()) {
            station.reserve_for(train.id(), self.distance_to_destination, trains);
        }

        let acceleration = train.acceleration();
        let braking_distance = (train.speed() * train.speed()) / (2.0 * acceleration);
        let speed_mod = if self.destination_behind_train { -1.0 } else { 1.0 };
        let pre_departure_look_ahead = if train.current_station().is_some() { 4.5 } else { 0.0 };
        let mut distance_to_next_curve = -1.0;

        // Signals.
        if self.waiting_for_signal.is_some() && self.current_signal_resolved(nets, graph, train) {
            if let Some((signal_id, side)) = self.waiting_for_signal {
                let cross = graph
                    .point(EdgePointType::Signal, signal_id)
                    .and_then(|p| p.signal())
                    .is_some_and(|s| s.types[if side { 0 } else { 1 }] == SignalType::CrossSignal);
                if cross {
                    self.waiting_for_chained_groups.clear();
                }
            }
            self.waiting_for_signal = None;
        }

        let leading_point = if !self.destination_behind_train { train.leading_point() } else { train.trailing_point() }.clone();

        if self.waiting_for_signal.is_none() {
            self.distance_to_signal = f64::MAX;
            self.ticks_waiting_for_signal = 0;
        }

        if self.distance_to_signal > f64::from(1.0_f32 / 16.0) {
            let mut curve_distance_tracker: f64 = -1.0;

            self.signal_scout.node1 = leading_point.node1;
            self.signal_scout.node2 = leading_point.node2;
            self.signal_scout.position = leading_point.position;

            let braking_distance_no_flicker = braking_distance + 3.0 - (braking_distance % 3.0);
            let scan_distance = clamp_f(braking_distance_no_flicker, pre_departure_look_ahead, self.distance_to_destination);

            let mut cross_signal_distance_tracker: f64 = -1.0;
            let mut tracking_cross_signal: Option<(Uuid, bool)> = None;
            let mut chained: BTreeMap<Uuid, (Uuid, bool)> = BTreeMap::new();
            self.waiting_for_chained_groups.clear();

            let mut waiting_for_signal = self.waiting_for_signal;
            let mut distance_to_signal = self.distance_to_signal;
            let mut newly_reserved: BTreeSet<Uuid> = BTreeSet::new();

            let train_id = train.id();
            let manual_tick = train.manual_tick();
            let dest_id = destination;

            let mut path_copy = self.current_path.clone();
            let mut sel = |g: &TrackGraph, _tp: &TravellingPoint, _f: bool, c: &[Candidate]| navigate_options(&mut path_copy, g, c);
            let mut selector = TrackSelector::Custom(&mut sel);

            let reserve_chain = |chained: &mut BTreeMap<Uuid, (Uuid, bool)>,
                                 newly_reserved: &mut BTreeSet<Uuid>,
                                 nets: &mut RailNetworks| {
                newly_reserved.extend(chained.keys().copied());
                for (group_id, (boundary_id, _)) in chained.iter() {
                    if let Some(group) = nets.signal_edge_groups.get_mut(group_id) {
                        group.reserved = Some(*boundary_id);
                    }
                }
                chained.clear();
            };

            {
                let graph_ref: &TrackGraph = graph;
                let idx = |b: bool| if b { 0 } else { 1 };
                let mut on_point = |distance: f64, hit: &EdgePointHit| -> bool {
                    // > scanDistance and not following down a cross signal.
                    let cross_signal_tracked = tracking_cross_signal.is_some();
                    if !cross_signal_tracked && distance > scan_distance {
                        return true;
                    }

                    let boundary = hit.point;
                    let Some(node2_loc) = graph_ref.get_node(hit.nodes.1).map(|n| n.location) else { return false };
                    if boundary.id == dest_id && boundary.can_approach_from(&node2_loc) {
                        return true;
                    }
                    let Some(signal) = boundary.signal() else { return false };

                    let side_primary = boundary.is_primary(&node2_loc);
                    let Some(entering) = signal.group(side_primary) else { return false };
                    if !nets.signal_edge_groups.contains_key(&entering) {
                        return false;
                    }

                    let primary = signal.groups[0] == Some(entering);
                    let cross_signal = signal.types[idx(primary)] == SignalType::CrossSignal;
                    let occupied = !manual_tick
                        && (signal.is_forced_red(side_primary) || nets.is_group_occupied_unless_train(entering, train_id));

                    if !cross_signal_tracked {
                        if cross_signal {
                            // Now entering a cross signal path.
                            tracking_cross_signal = Some((boundary.id, primary));
                            cross_signal_distance_tracker = distance;
                            chained.insert(entering, (boundary.id, primary));
                        }
                        if occupied {
                            // Section is occupied.
                            waiting_for_signal = Some((boundary.id, primary));
                            distance_to_signal = distance;
                            if !cross_signal {
                                return true; // standard entry signal, do not collect any further segments
                            }
                        }
                        if !occupied
                            && !cross_signal
                            && distance < distance_to_signal + 0.25
                            && distance < braking_distance_no_flicker
                        {
                            // Reserve the group for traversal.
                            if let Some(group) = nets.signal_edge_groups.get_mut(&entering) {
                                group.reserved = Some(boundary.id);
                            }
                        }
                        return false;
                    }

                    // Following down a cross signal.
                    chained.insert(entering, (boundary.id, primary)); // add the group to the chain
                    if occupied {
                        // Section is occupied, but wait at the cross signal that started the chain.
                        waiting_for_signal = tracking_cross_signal;
                        distance_to_signal = cross_signal_distance_tracker;
                        if !cross_signal {
                            return true; // entry signals end a chain
                        }
                    }
                    if !cross_signal {
                        if distance < distance_to_signal + 0.25 {
                            // Collect and reset the signal chain because none were blocked.
                            tracking_cross_signal = None;
                            reserve_chain(&mut chained, &mut newly_reserved, nets);
                            return false;
                        } else {
                            return true; // end of a blocked signal chain
                        }
                    }

                    false
                };

                let mut on_turn = |distance: f64, edge: &TrackEdge| {
                    let Some(turn) = &edge.turn else { return };
                    let v_distance = (turn.starts[0][1] - turn.starts[1][1]).abs();

                    // Ignore a turn that is a straight and a mild slope.
                    if v_distance > f64::from(1.0_f32 / 16.0) {
                        let a = [turn.axes[0][0], 0.0, turn.axes[0][2]];
                        let b = scale([turn.axes[1][0], 0.0, turn.axes[1][2]], -1.0);
                        if distance_to(a, b) < f64::from(1.0_f32 / 64.0)
                            && v_distance / turn.length() < f64::from(0.225_f32)
                        {
                            return;
                        }
                    }

                    let current = curve_distance_tracker as f32 as f64;
                    if current == -1.0 || distance < current {
                        curve_distance_tracker = distance;
                    }
                };

                let mut listeners = Listeners { edge_point: Some(&mut on_point), turn: Some(&mut on_turn), portal: None };

                // Adding 50 to the distance due to unresolved inaccuracies in TravellingPoint::travel.
                self.signal_scout.travel(
                    graph_ref,
                    (self.distance_to_destination + 50.0) * speed_mod,
                    &mut selector,
                    &mut listeners,
                );
            }

            if tracking_cross_signal.is_some() && waiting_for_signal.is_none() {
                reserve_chain(&mut chained, &mut newly_reserved, nets);
            }

            self.waiting_for_signal = waiting_for_signal;
            self.distance_to_signal = distance_to_signal;
            self.waiting_for_chained_groups = chained;
            train.reserved_signal_blocks().extend(newly_reserved);

            distance_to_next_curve = curve_distance_tracker as f32 as f64;
        } else {
            self.ticks_waiting_for_signal += 1;
        }

        let mut target_distance = if self.waiting_for_signal.is_some() { self.distance_to_signal } else { self.distance_to_destination };

        // Always overshoot to ensure the travelling point crosses the target.
        target_distance += 0.25;

        // Don't leave until the light is green.
        if target_distance > f64::from(1.0_f32 / 32.0) && train.current_station().is_some() {
            if self.waiting_for_signal.is_some() && self.distance_to_signal < pre_departure_look_ahead {
                self.ticks_waiting_for_signal += 1;
                return;
            }
            self.leave_station(graph, train);
        }

        train.set_currently_backwards(self.destination_behind_train);

        if target_distance < -10.0 {
            self.cancel_in(Some(graph), train);
            return;
        }

        if target_distance - train.speed().abs() < f64::from(1.0_f32 / 32.0) {
            train.set_speed(f64::max(target_distance, f64::from(1.0_f32 / 32.0)) * speed_mod);
            return;
        }

        train.burn_fuel();

        let mut top_speed = train.max_speed();

        if target_distance < 10.0 {
            let max_approach_speed = top_speed * (target_distance / 10.0);
            let speed_relative_to_station = train.speed() * speed_mod;

            if speed_relative_to_station > max_approach_speed {
                let speed = train.speed() + (max_approach_speed - train.speed().abs()) * f64::from(0.5_f32) * speed_mod;
                train.set_speed(speed);
                return;
            }
        }

        top_speed *= train.throttle();
        let turn_top_speed = f64::min(top_speed, train.max_turn_speed());

        let mut target_speed = if target_distance > braking_distance { top_speed * speed_mod } else { 0.0 };

        if distance_to_next_curve != -1.0 {
            let slowing_distance = braking_distance - (turn_top_speed * turn_top_speed) / (2.0 * acceleration);
            let target_turn_speed =
                if distance_to_next_curve > slowing_distance { top_speed * speed_mod } else { turn_top_speed * speed_mod };
            if target_turn_speed.abs() < target_speed.abs() {
                target_speed = target_turn_speed;
            }
        }

        train.set_target_speed(target_speed);
        train.approach_target_speed(1.0);
    }
}


#[cfg(test)]
mod tests {
    use super::super::graph::TrackEdgePoint;
    use super::super::network::tests::{disc, line, loc};
    use super::*;

    struct NoTrains;
    impl TrainStates for NoTrains {
        fn exists(&self, _: Uuid) -> bool {
            false
        }
        fn current_station(&self, _: Uuid) -> Option<Uuid> {
            None
        }
        fn navigation_active(&self, _: Uuid) -> bool {
            false
        }
        fn distance_to_destination(&self, _: Uuid) -> f64 {
            0.0
        }
    }

    /// A train sitting at one place of a graph, recording what Navigation does to it.
    struct Mock {
        id: Uuid,
        graph: Uuid,
        leading: TravellingPoint,
        trailing: TravellingPoint,
        types: BTreeSet<String>,
        forward_conductor: bool,
        backward_conductor: bool,
        double_ended: bool,
        paused: bool,
        station: Option<Uuid>,
        speed: f64,
        target: Option<f64>,
        approached: u32,
        status: Vec<ConductorStatus>,
        reserved: BTreeSet<Uuid>,
        interrupted: u32,
    }

    impl NavTrain for Mock {
        fn id(&self) -> Uuid {
            self.id
        }
        fn graph(&self) -> Option<Uuid> {
            Some(self.graph)
        }
        fn leading_point(&self) -> &TravellingPoint {
            &self.leading
        }
        fn trailing_point(&self) -> &TravellingPoint {
            &self.trailing
        }
        fn valid_track_types(&self) -> BTreeSet<String> {
            self.types.clone()
        }
        fn has_forward_conductor(&self) -> bool {
            self.forward_conductor
        }
        fn has_backward_conductor(&self) -> bool {
            self.backward_conductor
        }
        fn double_ended(&self) -> bool {
            self.double_ended
        }
        fn paused(&self) -> bool {
            self.paused
        }
        fn manual_tick(&self) -> bool {
            false
        }
        fn current_station(&self) -> Option<Uuid> {
            self.station
        }
        fn clear_current_station(&mut self) {
            self.station = None;
        }
        fn speed(&self) -> f64 {
            self.speed
        }
        fn set_speed(&mut self, speed: f64) {
            self.speed = speed;
        }
        fn throttle(&self) -> f64 {
            1.0
        }
        fn acceleration(&self) -> f64 {
            0.1
        }
        fn max_speed(&self) -> f64 {
            3.0
        }
        fn max_turn_speed(&self) -> f64 {
            1.0
        }
        fn set_target_speed(&mut self, speed: f64) {
            self.target = Some(speed);
        }
        fn approach_target_speed(&mut self, _: f64) {
            self.approached += 1;
        }
        fn burn_fuel(&mut self) {}
        fn set_currently_backwards(&mut self, _: bool) {}
        fn report_conductors(&mut self, status: ConductorStatus) {
            self.status.push(status);
        }
        fn reserved_signal_blocks(&mut self) -> &mut BTreeSet<Uuid> {
            &mut self.reserved
        }
        fn transit_interrupted(&mut self) {
            self.interrupted += 1;
        }
    }

    fn std_type(_: Option<&str>) -> String {
        TRACK_TYPE_STANDARD.to_string()
    }

    fn env<'a>(nets: &'a RailNetworks, others: &'a [OtherTrain]) -> SearchEnv<'a> {
        SearchEnv { nets, trains: &NoTrains, others, track_type_of: &std_type }
    }

    fn node(nets: &RailNetworks, g: Uuid, x: f64) -> NodeId {
        nets.graphs[&g].locate_node(&loc(x, 0.0)).unwrap().net_id
    }

    /// A train whose leading point is `position` along the edge `from` to `to` (x coordinates), and
    /// whose trailing point is at the same place.
    fn train_at(nets: &RailNetworks, g: Uuid, from: f64, to: f64, position: f64) -> Mock {
        let (a, b) = (node(nets, g, from), node(nets, g, to));
        let point = TravellingPoint::new(a, b, position, false);
        Mock {
            id: 1000,
            graph: g,
            leading: point.clone(),
            trailing: point,
            types: BTreeSet::from([TRACK_TYPE_STANDARD.to_string()]),
            forward_conductor: true,
            backward_conductor: true,
            double_ended: true,
            paused: false,
            station: None,
            speed: 0.0,
            target: None,
            approached: 0,
            status: Vec::new(),
            reserved: BTreeSet::new(),
            interrupted: 0,
        }
    }

    /// A station on the edge `from` to `to`, `position` along it from `from`, approachable by trains
    /// heading for `to`.
    fn add_station(nets: &mut RailNetworks, g: Uuid, from: f64, to: f64, position: f64) -> Uuid {
        let id = nets.new_uuid();
        nets.add_point(g, TrackEdgePoint::new_station(id, [loc(from, 0.0), loc(to, 0.0)], position));
        id
    }

    fn add_signal(nets: &mut RailNetworks, g: Uuid, from: f64, to: f64, position: f64, powered: bool) -> Uuid {
        let p = nets.new_signal_boundary([loc(from, 0.0), loc(to, 0.0)], position, [0, 65, 0], true, SignalType::EntrySignal, powered);
        let id = p.id;
        nets.add_point(g, p);
        for _ in 0..4 {
            nets.tick_all_points(true);
        }
        id
    }

    #[test]
    fn penalties_by_state() {
        let i = PenaltyInput { has_schedule: true, distance_to_signal: 100.0, ..Default::default() };
        assert_eq!(navigation_penalty(&i), 25);
        assert_eq!(navigation_penalty(&PenaltyInput { manual_tick: true, ..i }), 200);
        assert_eq!(navigation_penalty(&PenaltyInput { has_schedule: false, ..i }), 700);
        assert_eq!(navigation_penalty(&PenaltyInput { paused: true, ..i }), 700);
        assert_eq!(navigation_penalty(&PenaltyInput { waiting_for_signal: true, ticks_waiting_for_signal: 400, ..i }), 70);
        assert_eq!(navigation_penalty(&PenaltyInput { waiting_for_signal: true, ticks_waiting_for_signal: 1_000_000, ..i }), 1050);
        // Waiting but no ticks yet: falls through.
        assert_eq!(navigation_penalty(&PenaltyInput { waiting_for_signal: true, ..i }), 25);
        assert_eq!(navigation_penalty(&PenaltyInput { has_destination: true, distance_to_destination: 49.0, ..i }), 50);
        assert_eq!(navigation_penalty(&PenaltyInput { has_destination: true, distance_to_destination: 50.0, ..i }), 25);
        assert_eq!(navigation_penalty(&PenaltyInput { distance_to_signal: 19.0, ..i }), 50);
    }

    #[test]
    fn valid_types_are_what_every_bogey_allows() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let t = carriage_valid_types(&[(s(&["a", "b"]), None), (s(&["b", "c"]), Some(s(&["b"])))]);
        assert_eq!(t, BTreeSet::from(["b".to_string()]));
        let t = carriage_valid_types(&[(s(&["a"]), Some(s(&["b"])))]);
        assert!(t.is_empty());
        assert!(carriage_valid_types(&[]).is_empty());
    }

    #[test]
    fn the_priority_queue_orders_like_java() {
        let mut q: JavaPriorityQueue<(f64, i32)> = JavaPriorityQueue::new(|a, b| a.0.total_cmp(&b.0));
        for (i, k) in [5.0, 1.0, 3.0, 1.0, 4.0, 2.0, 3.0].into_iter().enumerate() {
            q.add((k, i as i32));
        }
        let mut keys = Vec::new();
        while let Some((k, _)) = q.poll() {
            keys.push(k);
        }
        assert_eq!(keys, vec![1.0, 1.0, 2.0, 3.0, 3.0, 4.0, 5.0]);
    }

    #[test]
    fn finds_a_station_ahead() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 10.0, 20.0, 30.0, 40.0]);
        let st = add_station(&mut nets, g, 20.0, 30.0, 4.0);
        let train = train_at(&nets, g, 0.0, 10.0, 5.0);
        let nav = Navigation::new();
        let env = env(&nets, &[]);
        let p = nav.find_path_to(&env, &train, st, f64::MAX).unwrap();
        // 5 to the node, 10 and 4 along: no forks, so an empty path.
        assert_eq!(p.distance, 19.0);
        assert!(p.path.is_empty());
        assert_eq!(p.destination, st);
        // The cost of the second hit is the penalty alone (Java passes `penalty`): nothing here.
        assert_eq!(p.cost, 0.0);
        // The same with no cost limit.
        let p2 = nav.find_path_to(&env, &train, st, -1.0).unwrap();
        assert_eq!(p2.distance, 19.0);
    }

    #[test]
    fn finds_a_station_on_the_starting_edge() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 10.0, 20.0]);
        let st = add_station(&mut nets, g, 0.0, 10.0, 8.0);
        let train = train_at(&nets, g, 0.0, 10.0, 5.0);
        let env = env(&nets, &[]);
        let p = Navigation::new().find_path_to(&env, &train, st, f64::MAX).unwrap();
        assert_eq!(p.distance, 3.0);
    }

    #[test]
    fn a_station_behind_is_a_negative_distance_for_a_double_ended_train() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 10.0, 20.0, 30.0]);
        // Approachable by trains heading for 0: the edge 10 -> 0 is the one the station sits on.
        let id = nets.new_uuid();
        nets.add_point(g, TrackEdgePoint::new_station(id, [loc(10.0, 0.0), loc(0.0, 0.0)], 3.0));
        let train = train_at(&nets, g, 20.0, 30.0, 4.0);
        let env = env(&nets, &[]);
        let nav = Navigation::new();
        let p = nav.find_path_to(&env, &train, id, f64::MAX).unwrap();
        assert!(p.distance < 0.0);
        // 4 back to the node at 20, 10 to the node at 10 and 3 along the edge toward 0: 17.
        assert_eq!(p.distance, -17.0);
        // Without a back conductor (and not paused) the train can't take it.
        let mut single = train_at(&nets, g, 20.0, 30.0, 4.0);
        single.double_ended = false;
        assert!(nav.find_path_to(&env, &single, id, f64::MAX).is_none());
    }

    #[test]
    fn a_fork_puts_the_branch_in_the_path() {
        let mut nets = RailNetworks::new();
        // A(0,0) - J(10,0), then B(20,0) straight on and C(20,3) branching off.
        let g = nets.new_graph();
        let gid = g.id;
        nets.put_graph_with_default_group(g);
        for (x, z) in [(0.0, 0.0), (10.0, 0.0), (20.0, 0.0), (20.0, 3.0)] {
            assert!(nets.create_node_if_absent(gid, &disc(x, z)));
        }
        for (a, b) in [((0.0, 0.0), (10.0, 0.0)), ((10.0, 0.0), (20.0, 0.0)), ((10.0, 0.0), (20.0, 3.0))] {
            nets.connect_nodes(gid, &disc(a.0, a.1), &disc(b.0, b.1), None);
        }
        let n = |nets: &RailNetworks, x, z| nets.graphs[&gid].locate_node(&loc(x, z)).unwrap().net_id;
        let (a, j, c) = (n(&nets, 0.0, 0.0), n(&nets, 10.0, 0.0), n(&nets, 20.0, 3.0));
        let id = nets.new_uuid();
        nets.add_point(
            gid,
            TrackEdgePoint::new_station(id, [loc(10.0, 0.0), TrackNodeLocation::new([20.0, 64.0, 3.0], 0)], 2.0),
        );
        let mut train = train_at(&nets, gid, 0.0, 10.0, 0.0);
        train.leading = TravellingPoint::new(a, j, 0.0, false);
        train.trailing = train.leading.clone();
        let env = env(&nets, &[]);
        let p = Navigation::new().find_path_to(&env, &train, id, f64::MAX).unwrap();
        assert_eq!(p.path, vec![(j, c)]);
    }

    #[test]
    fn a_forced_red_signal_costs_400_and_other_trains_cost_half_their_penalty() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 10.0, 20.0, 30.0, 40.0]);
        add_signal(&mut nets, g, 10.0, 20.0, 4.0, true);
        let st = add_station(&mut nets, g, 30.0, 40.0, 4.0);
        let train = train_at(&nets, g, 0.0, 10.0, 0.0);
        let env0 = env(&nets, &[]);
        let nav = Navigation::new();
        let p = nav.find_path_to(&env0, &train, st, f64::MAX).unwrap();
        assert_eq!(p.cost, 400.0);

        // Another train under the 20 -> 30 edge with penalty 700: 350 on top. A third, of another
        // graph, and this train itself, count for nothing.
        let e2030 = (Some(node(&nets, g, 20.0)), Some(node(&nets, g, 30.0)));
        let others = [
            OtherTrain { id: 5, graph: g, navigation_penalty: 700, endpoint_edges: [e2030, (None, None)] },
            OtherTrain { id: 6, graph: g + 1, navigation_penalty: 700, endpoint_edges: [e2030; 2] },
            OtherTrain { id: 1000, graph: g, navigation_penalty: 700, endpoint_edges: [e2030; 2] },
        ];
        let env1 = env(&nets, &others);
        let p = nav.find_path_to(&env1, &train, st, f64::MAX).unwrap();
        assert_eq!(p.cost, 750.0);
        // With no cost limit (-1) other trains are not counted.
        let p = nav.find_path_to(&env1, &train, st, -1.0).unwrap();
        assert_eq!(p.cost, 400.0);
        // A cost limit below the penalty finds nothing.
        assert!(nav.find_path_to(&env1, &train, st, 100.0).is_none());
    }

    #[test]
    fn track_types_the_train_cannot_run_on_block_the_search() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 10.0, 20.0]);
        let st = add_station(&mut nets, g, 10.0, 20.0, 4.0);
        let mut train = train_at(&nets, g, 0.0, 10.0, 0.0);
        let nav = Navigation::new();
        let e = env(&nets, &[]);
        assert!(nav.find_path_to(&e, &train, st, f64::MAX).is_some());
        train.types = BTreeSet::from(["other:type".to_string()]);
        assert!(nav.find_path_to(&e, &train, st, f64::MAX).is_none());
        train.types.clear();
        assert!(nav.find_path_to(&e, &train, st, f64::MAX).is_none());
    }

    #[test]
    fn an_assembling_station_is_closed() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 10.0, 20.0]);
        let st = add_station(&mut nets, g, 10.0, 20.0, 4.0);
        nets.graphs.get_mut(&g).unwrap().edge_points.get_mut(EdgePointType::Station, st).unwrap().station_mut().unwrap().assembling = true;
        let train = train_at(&nets, g, 0.0, 10.0, 0.0);
        let e = env(&nets, &[]);
        assert!(Navigation::new().find_path_to(&e, &train, st, f64::MAX).is_none());
    }

    #[test]
    fn the_nearest_approachable_station_respects_braking() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0]);
        let near = add_station(&mut nets, g, 10.0, 20.0, 2.0);
        let far = add_station(&mut nets, g, 20.0, 30.0, 2.0);
        let mut train = train_at(&nets, g, 0.0, 10.0, 0.0);
        let nav = Navigation::new();
        let e = env(&nets, &[]);
        // At rest the first is 12 away, with a minimum distance of 0.
        assert_eq!(nav.find_nearest_approachable(&e, &train, true), Some(near));
        // Moving at 2 with acceleration 0.1: braking takes 20 blocks, the minimum distance is 15, so
        // the first station (12 away) is too close.
        train.speed = 2.0;
        assert_eq!(nav.find_nearest_approachable(&e, &train, true), Some(far));
    }

    #[test]
    fn navigate_options_takes_the_path_edge() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 10.0, 20.0]);
        let (a, b, c) = (node(&nets, g, 0.0), node(&nets, g, 10.0), node(&nets, g, 20.0));
        let graph = &nets.graphs[&g];
        let options = [Candidate { from: b, node: a }, Candidate { from: b, node: c }];
        let mut path = vec![(b, c), (c, b)];
        assert_eq!(navigate_options(&mut path, graph, &options), 1);
        assert_eq!(path, vec![(c, b)]);
        // Not an option: the first, and the path stays.
        assert_eq!(navigate_options(&mut path, graph, &options), 0);
        assert_eq!(path.len(), 1);
        // Empty path: the first.
        let mut none = Vec::new();
        assert_eq!(navigate_options(&mut none, graph, &options), 0);
    }

    #[test]
    fn starting_and_cancelling() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 10.0, 20.0, 30.0]);
        let st = add_station(&mut nets, g, 20.0, 30.0, 4.0);
        let mut train = train_at(&nets, g, 0.0, 10.0, 5.0);
        let mut nav = Navigation::new();
        let e = env(&nets, &[]);
        let path = nav.find_path_to(&e, &train, st, f64::MAX).unwrap();

        // No path: nothing starts.
        assert_eq!(nav.start_navigation(&mut nets, &mut train, None), -1.0);
        assert!(!nav.is_active());

        // Without a forward conductor: -1, nothing starts, the status says so.
        train.forward_conductor = false;
        train.backward_conductor = false;
        assert_eq!(nav.start_navigation(&mut nets, &mut train, Some(path.clone())), -1.0);
        assert!(!nav.is_active());
        assert_eq!(train.status, vec![ConductorStatus::MissingConductor]);
        // A backward conductor only: the right kind is missing.
        train.backward_conductor = true;
        train.status.clear();
        nav.start_navigation(&mut nets, &mut train, Some(path.clone()));
        assert_eq!(train.status, vec![ConductorStatus::MissingCorrectConductor]);

        // With one: the cost comes back, the navigation is on.
        train.forward_conductor = true;
        let cost = nav.start_navigation(&mut nets, &mut train, Some(path.clone()));
        assert_eq!(cost, path.cost);
        assert!(nav.is_active());
        assert_eq!(nav.distance_to_destination, 19.0);
        assert_eq!(nav.distance_started_at, 19.0);
        assert!(!nav.announce_arrival);
        // Starting again toward the same station is free.
        assert_eq!(nav.start_navigation(&mut nets, &mut train, Some(path)), 0.0);

        // A long way announces the arrival.
        let far = DiscoveredPath { path: vec![], destination: st + 1, distance: -250.0, cost: 3.0 };
        nav.cancel_navigation(&mut nets, &mut train);
        assert!(!nav.is_active());
        assert_eq!(train.interrupted, 1);
        nav.start_navigation(&mut nets, &mut train, Some(far));
        assert!(nav.announce_arrival);
        assert!(nav.destination_behind_train);
        nav.cancel_navigation(&mut nets, &mut train);

        // Cancelling releases the reservation.
        let mut nav = Navigation::new();
        nets.graphs.get_mut(&g).unwrap().edge_points.get_mut(EdgePointType::Station, st).unwrap().station_mut().unwrap().nearest_train = Some(1000);
        nav.destination = Some(st);
        nav.cancel_navigation(&mut nets, &mut train);
        assert_eq!(station_of(&nets, g, st).nearest_train, None);
        assert_eq!(nav.destination, None);
    }

    fn station_of(nets: &RailNetworks, g: Uuid, st: Uuid) -> &super::super::station::GlobalStation {
        nets.graphs[&g].point(EdgePointType::Station, st).unwrap().station().unwrap()
    }

    #[test]
    fn a_clear_run_drives_at_top_speed() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0]);
        let st = add_station(&mut nets, g, 50.0, 60.0, 4.0);
        let mut train = train_at(&nets, g, 0.0, 10.0, 0.0);
        let mut nav = Navigation::new();
        let e = env(&nets, &[]);
        let path = nav.find_path_to(&e, &train, st, f64::MAX).unwrap();
        nav.start_navigation(&mut nets, &mut train, Some(path));
        nav.tick(&mut nets, &mut train, &NoTrains);
        assert_eq!(train.target, Some(3.0));
        assert_eq!(train.approached, 1);
        // The station is reserved for the train.
        assert_eq!(station_of(&nets, g, st).nearest_train, Some(1000));
        assert_eq!(nav.waiting_for_signal, None);
        assert_eq!(nav.distance_to_signal, f64::MAX);
    }

    #[test]
    fn close_to_the_destination_the_train_brakes() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 10.0, 20.0]);
        let st = add_station(&mut nets, g, 10.0, 20.0, 8.0);
        let mut train = train_at(&nets, g, 10.0, 20.0, 5.0);
        train.speed = 1.0;
        let mut nav = Navigation::new();
        let e = env(&nets, &[]);
        let path = nav.find_path_to(&e, &train, st, f64::MAX).unwrap();
        nav.start_navigation(&mut nets, &mut train, Some(path));
        assert_eq!(nav.distance_to_destination, 3.0);
        nav.tick(&mut nets, &mut train, &NoTrains);
        // Within 10 blocks: the approach speed is top * (3.25 / 10) = 0.975, below the current speed
        // of 1, so the speed is pulled toward it by half the difference.
        assert!((train.speed - (1.0 + (0.975 - 1.0) * 0.5)).abs() < 1e-9);
        assert_eq!(train.target, None);
        // Nearly there and slow: the speed becomes the distance left, at least 1/32.
        train.speed = 0.1;
        nav.distance_to_destination = -0.24;
        nav.tick(&mut nets, &mut train, &NoTrains);
        assert_eq!(train.speed, 0.03125);
    }

    #[test]
    fn overshooting_the_destination_cancels() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 10.0, 20.0]);
        let st = add_station(&mut nets, g, 10.0, 20.0, 8.0);
        let mut train = train_at(&nets, g, 10.0, 20.0, 5.0);
        let mut nav = Navigation::new();
        nav.destination = Some(st);
        nav.distance_to_destination = -11.0;
        nav.tick(&mut nets, &mut train, &NoTrains);
        assert!(!nav.is_active());
        assert_eq!(train.interrupted, 1);
    }

    #[test]
    fn a_missing_conductor_cancels_the_navigation() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 10.0, 20.0]);
        let st = add_station(&mut nets, g, 10.0, 20.0, 8.0);
        let mut train = train_at(&nets, g, 10.0, 20.0, 5.0);
        train.forward_conductor = false;
        train.backward_conductor = false;
        let mut nav = Navigation::new();
        nav.destination = Some(st);
        nav.distance_to_destination = 3.0;
        nav.tick(&mut nets, &mut train, &NoTrains);
        assert!(!nav.is_active());
        assert_eq!(train.status, vec![ConductorStatus::MissingConductor]);
        // A paused schedule needs none.
        let mut train = train_at(&nets, g, 10.0, 20.0, 5.0);
        train.forward_conductor = false;
        train.paused = true;
        nav.destination = Some(st);
        nav.distance_to_destination = 3.0;
        nav.tick(&mut nets, &mut train, &NoTrains);
        assert!(nav.is_active());
        assert!(train.status.is_empty());
    }

    /// A line of nodes every 10 blocks to 90, a signal 4 along the edge 20 to 30, and a station at
    /// the far end. The group the signal faces is returned with it.
    fn signalled() -> (RailNetworks, Uuid, Uuid, Uuid, Uuid) {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0]);
        let signal = add_signal(&mut nets, g, 20.0, 30.0, 4.0, false);
        let st = add_station(&mut nets, g, 80.0, 90.0, 4.0);
        let ahead = nets.graphs[&g].point(EdgePointType::Signal, signal).unwrap().signal().unwrap().group(true).unwrap();
        (nets, g, signal, st, ahead)
    }

    #[test]
    fn a_free_block_ahead_is_reserved_and_the_train_runs() {
        let (mut nets, g, signal, st, ahead) = signalled();
        let mut train = train_at(&nets, g, 10.0, 20.0, 0.0);
        train.speed = 2.0;
        let mut nav = Navigation::new();
        let e = env(&nets, &[]);
        let path = nav.find_path_to(&e, &train, st, f64::MAX).unwrap();
        nav.start_navigation(&mut nets, &mut train, Some(path));
        nav.tick(&mut nets, &mut train, &NoTrains);
        assert_eq!(nav.waiting_for_signal, None);
        // The signal is 14 away, inside the 21 of braking distance (with the flicker margin): the
        // block it leads into is reserved.
        assert_eq!(nets.signal_edge_groups[&ahead].reserved, Some(signal));
        assert_eq!(train.target, Some(3.0));
    }

    #[test]
    fn an_occupied_block_stops_the_train_at_the_signal() {
        let (mut nets, g, signal, st, ahead) = signalled();
        nets.signal_edge_groups.get_mut(&ahead).unwrap().trains.insert(99);
        let mut train = train_at(&nets, g, 10.0, 20.0, 0.0);
        train.speed = 2.0;
        let mut nav = Navigation::new();
        let e = env(&nets, &[]);
        let path = nav.find_path_to(&e, &train, st, f64::MAX).unwrap();
        // The red signal is in the cost: 25 for the first.
        assert_eq!(path.cost, 25.0);
        nav.start_navigation(&mut nets, &mut train, Some(path));
        nav.tick(&mut nets, &mut train, &NoTrains);
        assert_eq!(nav.waiting_for_signal, Some((signal, true)));
        assert_eq!(nav.distance_to_signal, 14.0);
        assert_eq!(nets.signal_edge_groups[&ahead].reserved, None);
        // 14.25 left to the signal, braking distance 20: stop.
        assert_eq!(train.target, Some(0.0));

        // While it waits, the scouting is skipped.
        nav.tick(&mut nets, &mut train, &NoTrains);
        assert_eq!(nav.waiting_for_signal, Some((signal, true)));

        // The other train leaves: the next tick finds the signal resolved and goes on.
        nets.signal_edge_groups.get_mut(&ahead).unwrap().trains.clear();
        nav.tick(&mut nets, &mut train, &NoTrains);
        assert_eq!(nav.waiting_for_signal, None);
        assert_eq!(train.target, Some(3.0));
    }

    #[test]
    fn a_train_in_the_block_does_not_block_itself() {
        let (mut nets, g, _signal, st, ahead) = signalled();
        nets.signal_edge_groups.get_mut(&ahead).unwrap().trains.insert(1000);
        let mut train = train_at(&nets, g, 10.0, 20.0, 0.0);
        train.speed = 2.0;
        let mut nav = Navigation::new();
        let e = env(&nets, &[]);
        let path = nav.find_path_to(&e, &train, st, f64::MAX).unwrap();
        nav.start_navigation(&mut nets, &mut train, Some(path));
        nav.tick(&mut nets, &mut train, &NoTrains);
        assert_eq!(nav.waiting_for_signal, None);
    }

    #[test]
    fn a_standing_train_waits_for_green_before_leaving_the_station() {
        let (mut nets, g, signal, st, ahead) = signalled();
        nets.signal_edge_groups.get_mut(&ahead).unwrap().trains.insert(99);
        // At rest, 4 blocks from the signal, standing at a station of its own.
        let mut train = train_at(&nets, g, 20.0, 30.0, 0.0);
        train.station = Some(555);
        let mut nav = Navigation::new();
        let e = env(&nets, &[]);
        let path = nav.find_path_to(&e, &train, st, f64::MAX).unwrap();
        nav.start_navigation(&mut nets, &mut train, Some(path));
        nav.tick(&mut nets, &mut train, &NoTrains);
        // Within the 4.5 of look-ahead (the scan reaches the signal, 4 away): it stays.
        assert_eq!(nav.waiting_for_signal, Some((signal, true)));
        assert_eq!(train.station, Some(555));
        assert_eq!(nav.ticks_waiting_for_signal, 1);
        // Green: it leaves.
        nets.signal_edge_groups.get_mut(&ahead).unwrap().trains.clear();
        nav.tick(&mut nets, &mut train, &NoTrains);
        assert_eq!(train.station, None);
    }
}
