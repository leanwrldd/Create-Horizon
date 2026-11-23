//! A train as an NBT tree: `Train.write` / `read`, `Carriage.write` / `read`, `CarriageBogey.write` / `read`,
//! `TravellingPoint.write` / `read`, `Navigation.write` / `read`, `ScheduleRuntime.write` / `read` and
//! `TrackNodeLocation.write` (content/trains/entity/, content/trains/schedule/ScheduleRuntime.java), the form the
//! level keeps trains in (Create's `RailwaySavedData` holds the same tags).
//!
//! The tree is [`crate::rules::snbt::Value`]; the glue turns it into the host's `Tag`. A train is read against the
//! graphs of the dimension: its wheels name nodes by their locations, which the graph a train is read into
//! must have again (the graphs are rebuilt from the track blocks after a load, so the glue reads a train once its
//! track is back, [`read_train`] says `None` until then). Dimensions are written by name (`Dim`), the glue gives
//! the numbering the graphs use.
//!
//! Differences from Java: the cargo (`TrainCargoManager`) and the carriages' entities and passengers are the
//! glue's, `bogeyData` is opaque bytes that the standard bogeys leave empty, and the carriage entity positioning
//! (`EntityPositioning`) is recomputed from the wheels on the first tick. A navigation whose destination station isn't
//! on the graph yet is dropped, as in Java (`getPoint` is null); the schedule runtime restarts it.

use super::bogey::BogeyType;
use super::carriage::{Carriage, CarriageBogey};
use super::graph::{EdgePointType, TrackNodeLocation, Uuid};
use super::navigation::Navigation;
use super::network::{RailNetworks, TrackGraph};
use super::railways::Railways;
use super::schedule::{ConditionContext, ScheduleRuntime, State};
use super::schedule_nbt::{read_schedule, write_schedule};
use super::train::{Train, TrainMigration};
use super::travelling_point::TravellingPoint;
use crate::rules::snbt::Value;
use std::collections::{BTreeMap, BTreeSet};

type Tag = Vec<(String, Value)>;

fn entry(key: &str, value: Value) -> (String, Value) {
    (key.to_owned(), value)
}

fn get<'a>(tag: &'a [(String, Value)], key: &str) -> Option<&'a Value> {
    tag.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn compound<'a>(tag: &'a [(String, Value)], key: &str) -> Option<&'a [(String, Value)]> {
    match get(tag, key) {
        Some(Value::Compound(c)) => Some(c),
        _ => None,
    }
}

fn list<'a>(tag: &'a [(String, Value)], key: &str) -> &'a [Value] {
    match get(tag, key) {
        Some(Value::List(l)) => l,
        _ => &[],
    }
}

fn int(tag: &[(String, Value)], key: &str) -> i32 {
    match get(tag, key) {
        Some(Value::Int(v)) => *v,
        Some(Value::Byte(v)) => i32::from(*v),
        Some(Value::Short(v)) => i32::from(*v),
        Some(Value::Long(v)) => *v as i32,
        _ => 0,
    }
}

fn boolean(tag: &[(String, Value)], key: &str) -> bool {
    int(tag, key) != 0
}

fn double(tag: &[(String, Value)], key: &str) -> f64 {
    match get(tag, key) {
        Some(Value::Double(v)) => *v,
        Some(Value::Float(v)) => f64::from(*v),
        Some(Value::Int(v)) => f64::from(*v),
        _ => 0.0,
    }
}

fn text(tag: &[(String, Value)], key: &str) -> String {
    match get(tag, key) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

fn uuid_value(id: Uuid) -> Value {
    Value::IntArray(vec![(id >> 96) as i32, (id >> 64) as i32, (id >> 32) as i32, id as i32])
}

fn uuid_of(tag: &[(String, Value)], key: &str) -> Option<Uuid> {
    match get(tag, key) {
        Some(Value::IntArray(v)) if v.len() == 4 => Some(v.iter().fold(0u128, |acc, part| (acc << 32) | u128::from(*part as u32))),
        _ => None,
    }
}

fn has(tag: &[(String, Value)], key: &str) -> bool {
    get(tag, key).is_some()
}

// --- a location on the graph -------------------------------------------------------------------------------------------

/// `TrackNodeLocation.write(dimensions)`: the half-block coordinates, the dimension by name and the tilt.
pub fn write_location(loc: &TrackNodeLocation, dimension: &dyn Fn(u32) -> String) -> Value {
    Value::Compound(vec![
        entry("X", Value::Int(loc.x)),
        entry("Y", Value::Int(loc.y)),
        entry("Z", Value::Int(loc.z)),
        entry("Dim", Value::Str(dimension(loc.dimension))),
        entry("YOffset", Value::Int(loc.y_offset_pixels)),
    ])
}

pub fn read_location(tag: &[(String, Value)], dimension: &dyn Fn(&str) -> u32) -> TrackNodeLocation {
    TrackNodeLocation {
        x: int(tag, "X"),
        y: int(tag, "Y"),
        z: int(tag, "Z"),
        dimension: dimension(&text(tag, "Dim")),
        y_offset_pixels: int(tag, "YOffset"),
    }
}

fn read_location_value(value: &Value, dimension: &dyn Fn(&str) -> u32) -> Option<TrackNodeLocation> {
    match value {
        Value::Compound(c) => Some(read_location(c, dimension)),
        _ => None,
    }
}

// --- the wheels ---------------------------------------------------------------------------------------------------------

/// `TravellingPoint.write`: an empty tag for a point on no edge.
pub fn write_point(point: &TravellingPoint, graph: &TrackGraph, dimension: &dyn Fn(u32) -> String) -> Value {
    let nodes = [point.node1, point.node2].map(|n| n.and_then(|n| graph.get_node(n)).map(|n| n.location));
    let (Some(a), Some(b)) = (nodes[0], nodes[1]) else { return Value::Compound(Vec::new()) };
    Value::Compound(vec![
        entry("Nodes", Value::List(vec![write_location(&a, dimension), write_location(&b, dimension)])),
        entry("Position", Value::Double(point.position)),
        entry("UpsideDown", Value::Byte(i8::from(point.upside_down))),
    ])
}

/// `TravellingPoint.read`: a point on nothing when a node isn't in the graph.
pub fn read_point(tag: &[(String, Value)], graph: &TrackGraph, dimension: &dyn Fn(&str) -> u32) -> TravellingPoint {
    let nodes = list(tag, "Nodes");
    if nodes.len() != 2 {
        return TravellingPoint::empty();
    }
    let ids = nodes.iter().map(|n| read_location_value(n, dimension).and_then(|l| graph.locate_node(&l)).map(|n| n.net_id)).collect::<Vec<_>>();
    let (Some(Some(a)), Some(Some(b))) = (ids.first().copied(), ids.get(1).copied()) else { return TravellingPoint::empty() };
    if graph.edge(a, b).is_none() {
        return TravellingPoint::empty();
    }
    TravellingPoint::new(a, b, double(tag, "Position"), boolean(tag, "UpsideDown"))
}

/// The locations of the nodes a point stands between, when it stands on an edge: where a train is found again.
pub fn point_locations(tag: &[(String, Value)], dimension: &dyn Fn(&str) -> u32) -> Option<[TrackNodeLocation; 2]> {
    let nodes = list(tag, "Nodes");
    if nodes.len() != 2 {
        return None;
    }
    Some([read_location_value(&nodes[0], dimension)?, read_location_value(&nodes[1], dimension)?])
}

// --- bogeys and carriages ----------------------------------------------------------------------------------------------

/// The bogey block a `BogeyType` is.
fn bogey_of(id: &str) -> BogeyType {
    if id == "create:large_bogey" { BogeyType::large() } else { BogeyType::small() }
}

/// `CarriageBogey.write`.
pub fn write_bogey(bogey: &CarriageBogey, graph: &TrackGraph, dimension: &dyn Fn(u32) -> String) -> Value {
    Value::Compound(vec![
        entry("Type", Value::Str(bogey.ty.id.clone())),
        entry("Points", Value::List(bogey.points.iter().map(|p| write_point(p, graph, dimension)).collect())),
        entry("UpsideDown", Value::Byte(i8::from(bogey.is_upside_down()))),
        entry("DerailAngle", Value::Int(bogey.derail_angle)),
        entry("BogeyData", Value::ByteArray(bogey.bogey_data.iter().map(|b| *b as i8).collect())),
    ])
}

/// `CarriageBogey.read`.
pub fn read_bogey(tag: &[(String, Value)], graph: &TrackGraph, dimension: &dyn Fn(&str) -> u32) -> CarriageBogey {
    let points = list(tag, "Points");
    let point = |i: usize| match points.get(i) {
        Some(Value::Compound(c)) => read_point(c, graph, dimension),
        _ => TravellingPoint::empty(),
    };
    let data = match get(tag, "BogeyData") {
        Some(Value::ByteArray(b)) => b.iter().map(|b| *b as u8).collect(),
        _ => Vec::new(),
    };
    CarriageBogey::new(bogey_of(&text(tag, "Type")), boolean(tag, "UpsideDown"), data, point(0), point(1), int(tag, "DerailAngle"))
}

/// `Carriage.write`.
pub fn write_carriage(carriage: &Carriage, graph: &TrackGraph, dimension: &dyn Fn(u32) -> String) -> Value {
    let mut tag: Tag = vec![
        entry("Id", Value::Int(carriage.id)),
        entry("FirstBogey", write_bogey(&carriage.bogeys.0, graph, dimension)),
    ];
    if let Some(second) = &carriage.bogeys.1 {
        tag.push(entry("SecondBogey", write_bogey(second, graph, dimension)));
    }
    tag.push(entry("Spacing", Value::Int(carriage.bogey_spacing)));
    tag.push(entry("FrontConductor", Value::Byte(i8::from(carriage.present_conductors[0]))));
    tag.push(entry("BackConductor", Value::Byte(i8::from(carriage.present_conductors[1]))));
    tag.push(entry("Stalled", Value::Byte(i8::from(carriage.stalled))));
    Value::Compound(tag)
}

/// `Carriage.read`.
pub fn read_carriage(tag: &[(String, Value)], graph: &TrackGraph, dimension: &dyn Fn(&str) -> u32) -> Carriage {
    let bogey = |key: &str| compound(tag, key).map(|c| read_bogey(c, graph, dimension));
    let first = bogey("FirstBogey").unwrap_or_else(|| CarriageBogey::new(BogeyType::small(), false, Vec::new(), TravellingPoint::empty(), TravellingPoint::empty(), 0));
    let mut carriage = Carriage::new(int(tag, "Id"), first, bogey("SecondBogey"), int(tag, "Spacing"));
    carriage.stalled = boolean(tag, "Stalled");
    carriage.present_conductors = [boolean(tag, "FrontConductor"), boolean(tag, "BackConductor")];
    carriage
}

// --- navigation and the schedule runtime ---------------------------------------------------------------------------------

/// `Navigation.write`.
pub fn write_navigation(nav: &Navigation, graph: &TrackGraph, dimension: &dyn Fn(u32) -> String) -> Value {
    let Some(destination) = nav.destination else { return Value::Compound(Vec::new()) };
    let path = nav.current_path.iter().filter_map(|(a, b)| {
        let (a, b) = (graph.get_node(*a)?.location, graph.get_node(*b)?.location);
        Some(Value::Compound(vec![entry("Nodes", Value::List(vec![write_location(&a, dimension), write_location(&b, dimension)]))]))
    });
    let mut tag: Tag = vec![
        entry("Destination", uuid_value(destination)),
        entry("DistanceToDestination", Value::Double(nav.distance_to_destination)),
        entry("DistanceStartedAt", Value::Double(nav.distance_started_at)),
        entry("BehindTrain", Value::Byte(i8::from(nav.destination_behind_train))),
        entry("AnnounceArrival", Value::Byte(i8::from(nav.announce_arrival))),
        entry("Path", Value::List(path.collect())),
    ];
    if let Some((signal, side)) = nav.waiting_for_signal {
        tag.push(entry("BlockingSignal", uuid_value(signal)));
        tag.push(entry("BlockingSignalSide", Value::Byte(i8::from(side))));
        tag.push(entry("DistanceToSignal", Value::Double(nav.distance_to_signal)));
        tag.push(entry("TicksWaitingForSignal", Value::Int(nav.ticks_waiting_for_signal)));
    }
    Value::Compound(tag)
}

/// `Navigation.read`: nothing when the destination isn't a station of the graph.
pub fn read_navigation(tag: &[(String, Value)], graph: &TrackGraph, dimension: &dyn Fn(&str) -> u32) -> Navigation {
    let mut nav = Navigation::new();
    let Some(destination) = uuid_of(tag, "Destination") else { return nav };
    if graph.point(EdgePointType::Station, destination).is_none() {
        return nav;
    }
    nav.destination = Some(destination);
    nav.distance_to_destination = double(tag, "DistanceToDestination");
    nav.distance_started_at = double(tag, "DistanceStartedAt");
    nav.destination_behind_train = boolean(tag, "BehindTrain");
    nav.announce_arrival = boolean(tag, "AnnounceArrival");
    for step in list(tag, "Path") {
        let Value::Compound(step) = step else { continue };
        let nodes = list(step, "Nodes");
        let ids = nodes.iter().map(|n| read_location_value(n, dimension).and_then(|l| graph.locate_node(&l)).map(|n| n.net_id)).collect::<Vec<_>>();
        // `removeBrokenPathEntries`: a pair whose nodes or edge are gone is dropped.
        if let [Some(a), Some(b)] = ids[..] {
            if graph.edge(a, b).is_some() {
                nav.current_path.push((a, b));
            }
        }
    }
    if let Some(signal) = uuid_of(tag, "BlockingSignal") {
        nav.waiting_for_signal = Some((signal, boolean(tag, "BlockingSignalSide")));
        nav.distance_to_signal = double(tag, "DistanceToSignal");
        nav.ticks_waiting_for_signal = int(tag, "TicksWaitingForSignal");
    }
    nav
}

fn state_name(state: State) -> &'static str {
    match state {
        State::PreTransit => "PRE_TRANSIT",
        State::InTransit => "IN_TRANSIT",
        State::PostTransit => "POST_TRANSIT",
    }
}

fn state_of(name: &str) -> State {
    match name {
        "IN_TRANSIT" => State::InTransit,
        "POST_TRANSIT" => State::PostTransit,
        _ => State::PreTransit,
    }
}

/// `ScheduleRuntime.write`.
pub fn write_runtime(runtime: &ScheduleRuntime) -> Value {
    let mut tag: Tag = vec![
        entry("CurrentEntry", Value::Int(runtime.current_entry)),
        entry("AutoSchedule", Value::Byte(i8::from(runtime.is_auto_schedule))),
        entry("Paused", Value::Byte(i8::from(runtime.paused))),
        entry("Completed", Value::Byte(i8::from(runtime.completed))),
    ];
    if let Some(schedule) = &runtime.schedule {
        tag.push(entry("Schedule", write_schedule(schedule)));
    }
    tag.push(entry("State", Value::Str(state_name(runtime.state).to_owned())));
    tag.push(entry("ConditionProgress", Value::IntArray(runtime.condition_progress.clone())));
    let contexts = runtime.condition_context.iter().map(|c| {
        let mut ctx: Tag = vec![
            entry("Time", Value::Int(c.time)),
            entry("StatusVersion", Value::Int(c.status_version)),
            entry("PrevPlayerCount", Value::Int(c.prev_player_count)),
        ];
        if let Some(v) = c.last_checked {
            ctx.push(entry("LastChecked", Value::Int(v)));
        }
        if let Some(v) = c.current_display {
            ctx.push(entry("CurrentDisplay", Value::Int(v)));
        }
        Value::Compound(ctx)
    });
    tag.push(entry("ConditionContext", Value::List(contexts.collect())));
    tag.push(entry("TransitTimes", Value::IntArray(runtime.prediction_ticks.clone())));
    Value::Compound(tag)
}

/// `ScheduleRuntime.read`.
pub fn read_runtime(tag: &[(String, Value)]) -> ScheduleRuntime {
    let mut runtime = ScheduleRuntime::new();
    runtime.paused = boolean(tag, "Paused");
    runtime.completed = boolean(tag, "Completed");
    runtime.is_auto_schedule = boolean(tag, "AutoSchedule");
    runtime.current_entry = int(tag, "CurrentEntry").max(0);
    if let Some(schedule) = compound(tag, "Schedule") {
        runtime.schedule = Some(read_schedule(schedule));
    }
    runtime.state = state_of(&text(tag, "State"));
    if let Some(Value::IntArray(progress)) = get(tag, "ConditionProgress") {
        runtime.condition_progress = progress.clone();
    }
    for context in list(tag, "ConditionContext") {
        let Value::Compound(c) = context else { continue };
        runtime.condition_context.push(ConditionContext {
            time: int(c, "Time"),
            status_version: int(c, "StatusVersion"),
            last_checked: has(c, "LastChecked").then(|| int(c, "LastChecked")),
            current_display: has(c, "CurrentDisplay").then(|| int(c, "CurrentDisplay")),
            prev_player_count: int(c, "PrevPlayerCount"),
        });
    }
    if let (Some(schedule), Some(Value::IntArray(times))) = (&runtime.schedule, get(tag, "TransitTimes")) {
        // `predictionTicks` starts as `TBD` for each entry and takes the saved times when the counts match.
        runtime.prediction_ticks = vec![super::schedule::TBD; schedule.entries.len()];
        if times.len() == schedule.entries.len() {
            runtime.prediction_ticks = times.clone();
        }
    }
    runtime
}

// --- the train ------------------------------------------------------------------------------------------------------------

fn uuid_list(ids: impl Iterator<Item = (Uuid, Option<Uuid>)>) -> Value {
    Value::List(
        ids.map(|(id, boundary)| {
            let mut c: Tag = vec![entry("Id", uuid_value(id))];
            if let Some(b) = boundary {
                c.push(entry("Boundary", uuid_value(b)));
            }
            Value::Compound(c)
        })
        .collect(),
    )
}

/// `Train.write`. The graph is the one the train's wheels are on (`train.graph`, or the one they last named).
pub fn write_train(train: &Train, graph: Option<&TrackGraph>, dimension: &dyn Fn(u32) -> String) -> Value {
    let empty = TrackGraph::new(0, 0);
    let g = graph.unwrap_or(&empty);
    let mut tag: Tag = vec![entry("Id", uuid_value(train.id))];
    if let Some(owner) = train.owner {
        tag.push(entry("Owner", uuid_value(owner)));
    }
    tag.push(entry("Carriages", Value::List(train.carriages.iter().map(|c| write_carriage(c, g, dimension)).collect())));
    tag.push(entry("CarriageSpacing", Value::IntArray(train.carriage_spacing.clone())));
    tag.push(entry("DoubleEnded", Value::Byte(i8::from(train.double_ended))));
    tag.push(entry("Speed", Value::Double(train.speed)));
    tag.push(entry("Throttle", Value::Double(train.throttle)));
    if let Some(speed) = train.speed_before_stall {
        tag.push(entry("SpeedBeforeStall", Value::Double(speed)));
    }
    tag.push(entry("Fuel", Value::Int(train.fuel_ticks)));
    tag.push(entry("TargetSpeed", Value::Double(train.target_speed)));
    tag.push(entry("IconType", Value::Str(train.icon.clone())));
    tag.push(entry("MapColorIndex", Value::Int(train.map_color_index)));
    tag.push(entry("Name", Value::Str(train.name.clone())));
    if let Some(station) = train.current_station {
        tag.push(entry("Station", uuid_value(station)));
    }
    tag.push(entry("Backwards", Value::Byte(i8::from(train.currently_backwards))));
    tag.push(entry("Derailed", Value::Byte(i8::from(train.derailed))));
    tag.push(entry("UpdateSignals", Value::Byte(i8::from(train.update_signal_blocks))));
    tag.push(entry("SignalBlocks", uuid_list(train.occupied_signal_blocks.iter().map(|(k, v)| (*k, *v)))));
    tag.push(entry("ReservedSignalBlocks", uuid_list(train.reserved_signal_blocks.iter().map(|k| (*k, None)))));
    tag.push(entry("OccupiedObservers", uuid_list(train.occupied_observers.iter().map(|k| (*k, None)))));
    let migrating = train.migrating_points.iter().map(|m| {
        Value::Compound(vec![
            entry("Locations", Value::List(m.locations.iter().map(|l| write_location(l, dimension)).collect())),
            entry("Position", Value::Double(m.position_on_old_edge)),
            entry("Curve", Value::Byte(i8::from(m.curve))),
            entry("Fallback", Value::List(m.fallback.iter().map(|v| Value::Double(*v)).collect())),
        ])
    });
    tag.push(entry("MigratingPoints", Value::List(migrating.collect())));
    tag.push(entry("Runtime", write_runtime(&train.runtime)));
    tag.push(entry("Navigation", write_navigation(&train.navigation, g, dimension)));
    Value::Compound(tag)
}

/// Where a saved train's wheels were: the node locations of every wheel that was on an edge.
pub fn wheel_locations(tag: &[(String, Value)], dimension: &dyn Fn(&str) -> u32) -> Vec<[TrackNodeLocation; 2]> {
    let mut out = Vec::new();
    for carriage in list(tag, "Carriages") {
        let Value::Compound(carriage) = carriage else { continue };
        for key in ["FirstBogey", "SecondBogey"] {
            let Some(bogey) = compound(carriage, key) else { continue };
            for point in list(bogey, "Points") {
                if let Value::Compound(point) = point {
                    out.extend(point_locations(point, dimension));
                }
            }
        }
    }
    out
}

/// What reading a train found: the train and the graph it stands on (none while derailed).
#[derive(Debug)]
pub struct Loaded {
    pub train: Train,
    pub graph: Option<Uuid>,
}

/// `Train.read`. `None` while the graph the wheels name isn't there yet (a train that was saved with no wheel on a
/// track reads at once, derailed and detached). The glue tries again as the track comes back.
pub fn read_train(tag: &[(String, Value)], nets: &RailNetworks, dimension: &dyn Fn(&str) -> u32) -> Option<Loaded> {
    let id = uuid_of(tag, "Id")?;
    let wheels = wheel_locations(tag, dimension);
    // The graph the wheels are on: all of them must be in one.
    let mut graph_id = None;
    for [a, b] in &wheels {
        let found = nets.get_graph(a)?;
        if nets.get_graph(b)? != found || graph_id.is_some_and(|g| g != found) {
            return None;
        }
        graph_id = Some(found);
    }
    let graph = graph_id.and_then(|g| nets.graphs.get(&g));
    let empty = TrackGraph::new(0, 0);
    let g = graph.unwrap_or(&empty);
    let carriages: Vec<Carriage> = list(tag, "Carriages")
        .iter()
        .filter_map(|c| if let Value::Compound(c) = c { Some(read_carriage(c, g, dimension)) } else { None })
        .collect();
    if carriages.is_empty() {
        return None;
    }
    // Every wheel that was on an edge must be again: the track the wheel stood on must be whole.
    let expected = wheels.len();
    let present = carriages
        .iter()
        .flat_map(|c| [Some(&c.bogeys.0), c.bogeys.1.as_ref()])
        .flatten()
        .flat_map(|b| b.points.iter())
        .filter(|p| p.node1.is_some())
        .count();
    if present != expected {
        return None;
    }
    let spacing = match get(tag, "CarriageSpacing") {
        Some(Value::IntArray(v)) => v.clone(),
        _ => Vec::new(),
    };
    let mut train = Train::new(
        id,
        uuid_of(tag, "Owner"),
        graph_id,
        carriages,
        spacing,
        boolean(tag, "DoubleEnded"),
        &text(tag, "Name"),
        &text(tag, "IconType"),
        int(tag, "MapColorIndex"),
    );
    train.speed = double(tag, "Speed");
    train.throttle = double(tag, "Throttle");
    if has(tag, "SpeedBeforeStall") {
        train.speed_before_stall = Some(double(tag, "SpeedBeforeStall"));
    }
    train.target_speed = double(tag, "TargetSpeed");
    train.current_station = uuid_of(tag, "Station");
    train.currently_backwards = boolean(tag, "Backwards");
    train.derailed = boolean(tag, "Derailed");
    train.update_signal_blocks = boolean(tag, "UpdateSignals");
    train.fuel_ticks = int(tag, "Fuel");
    for c in list(tag, "SignalBlocks") {
        if let Value::Compound(c) = c {
            if let Some(id) = uuid_of(c, "Id") {
                train.occupied_signal_blocks.insert(id, uuid_of(c, "Boundary"));
            }
        }
    }
    train.reserved_signal_blocks = list(tag, "ReservedSignalBlocks").iter().filter_map(|c| if let Value::Compound(c) = c { uuid_of(c, "Id") } else { None }).collect::<BTreeSet<_>>();
    train.occupied_observers = list(tag, "OccupiedObservers").iter().filter_map(|c| if let Value::Compound(c) = c { uuid_of(c, "Id") } else { None }).collect::<BTreeSet<_>>();
    for m in list(tag, "MigratingPoints") {
        let Value::Compound(m) = m else { continue };
        let locations = list(m, "Locations").iter().filter_map(|l| read_location_value(l, dimension)).collect::<Vec<_>>();
        let [a, b] = locations[..] else { continue };
        let fallback = match list(m, "Fallback") {
            [Value::Double(x), Value::Double(y), Value::Double(z)] => [*x, *y, *z],
            _ => [0.0; 3],
        };
        train.migrating_points.push(TrainMigration { locations: [a, b], position_on_old_edge: double(m, "Position"), curve: boolean(m, "Curve"), fallback });
    }
    if let Some(runtime) = compound(tag, "Runtime") {
        train.runtime = read_runtime(runtime);
    }
    if let Some(nav) = compound(tag, "Navigation") {
        train.navigation = read_navigation(nav, g, dimension);
    }
    // A train that stood at a station on a graph the station isn't on any more has nowhere to be.
    if let Some(station) = train.current_station {
        if g.point(EdgePointType::Station, station).is_none() {
            train.current_station = None;
        }
    }
    Some(Loaded { train, graph: graph_id })
}

impl Railways {
    /// A train read from the save joins the manager: the station it stood at is reserved for it again
    /// (`Train.read`'s `reserveFor`) and it goes on the moving list. The occupied signal blocks it saved stay.
    pub fn add_loaded_train(&mut self, train: Train) {
        let station = train.current_station;
        if let (Some(s), Some(g)) = (station, train.graph) {
            let (id, distance) = (train.id, train.navigation.distance_to_destination);
            let view = super::train::TrainView { others: &self.trains, me: Some((id, station, train.navigation.is_active(), distance)) };
            if let Some(st) = self.nets.graphs.get_mut(&g).and_then(|g| g.edge_points.get_mut(EdgePointType::Station, s)).and_then(|p| p.station_mut()) {
                st.reserve_for(id, distance, &view);
            }
        }
        self.add_train(train);
    }
}

/// The map type some callers want for ids: kept here so the glue doesn't need `BTreeMap` imports for it.
pub type SignalBlocks = BTreeMap<Uuid, Option<Uuid>>;

#[cfg(test)]
mod tests {
    use super::super::network::tests::{line, loc};
    use super::*;

    fn dim_name(d: u32) -> String {
        format!("test:dim{d}")
    }

    fn dim_id(name: &str) -> u32 {
        name.trim_start_matches("test:dim").parse().unwrap_or(0)
    }

    fn train_on(nets: &RailNetworks, g: Uuid) -> Train {
        let node = |x: f64| nets.graphs[&g].locate_node(&loc(x, 0.0)).unwrap().net_id;
        let (a, b) = (node(0.0), node(100.0));
        let pt = |p: f64| TravellingPoint::new(a, b, p, false);
        let b1 = CarriageBogey::new(BogeyType::small(), false, vec![1, 2], pt(20.0), pt(18.0), 7);
        let b2 = CarriageBogey::new(BogeyType::large(), false, vec![], pt(10.0), pt(8.0), -3);
        let c1 = Carriage::new(5, b1, None, 0);
        let mut c2 = Carriage::new(6, b2, None, 0);
        c2.present_conductors = [true, false];
        let mut t = Train::new(42, Some(9), Some(g), vec![c1, c2], vec![6], true, "Express", "create:traditional", 3);
        t.speed = 0.5;
        t.throttle = 0.75;
        t.target_speed = 0.6;
        t.fuel_ticks = 12;
        t.currently_backwards = true;
        t.occupied_signal_blocks.insert(100, Some(101));
        t.reserved_signal_blocks.insert(102);
        t.occupied_observers.insert(103);
        t.runtime.paused = false;
        t.runtime.current_entry = 2;
        t.runtime.state = State::InTransit;
        t.runtime.condition_progress = vec![1, 2];
        t
    }

    #[test]
    fn a_train_survives_its_nbt() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 100.0]);
        let train = train_on(&nets, g);
        let tag = write_train(&train, nets.graphs.get(&g), &dim_name);
        let Value::Compound(tag) = tag else { panic!("not a compound") };
        let back = read_train(&tag, &nets, &dim_id).expect("read").train;
        assert_eq!(back.id, 42);
        assert_eq!(back.owner, Some(9));
        assert_eq!(back.name, "Express");
        assert_eq!(back.map_color_index, 3);
        assert!(back.double_ended && back.currently_backwards);
        assert_eq!((back.speed, back.throttle, back.target_speed, back.fuel_ticks), (0.5, 0.75, 0.6, 12));
        assert_eq!(back.carriage_spacing, vec![6]);
        assert_eq!(back.carriages.len(), 2);
        assert_eq!(back.carriages[0].id, 5);
        assert_eq!(back.carriages[0].bogeys.0.points[0].position, 20.0);
        assert_eq!(back.carriages[0].bogeys.0.bogey_data, vec![1, 2]);
        assert_eq!(back.carriages[0].bogeys.0.derail_angle, 7);
        assert_eq!(back.carriages[1].bogeys.0.ty.id, "create:large_bogey");
        assert_eq!(back.carriages[1].present_conductors, [true, false]);
        assert_eq!(back.occupied_signal_blocks.get(&100), Some(&Some(101)));
        assert!(back.reserved_signal_blocks.contains(&102) && back.occupied_observers.contains(&103));
        assert_eq!((back.runtime.paused, back.runtime.current_entry, back.runtime.state), (false, 2, State::InTransit));
        assert_eq!(back.runtime.condition_progress, vec![1, 2]);
        assert_eq!(back.graph, Some(g));
    }

    #[test]
    fn a_train_waits_for_its_track() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 100.0]);
        let train = train_on(&nets, g);
        let Value::Compound(tag) = write_train(&train, nets.graphs.get(&g), &dim_name) else { panic!() };
        // The track isn't there yet.
        assert!(read_train(&tag, &RailNetworks::new(), &dim_id).is_none());
    }

    #[test]
    fn a_navigation_to_a_missing_station_is_dropped() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 100.0]);
        let mut train = train_on(&nets, g);
        train.navigation.destination = Some(777);
        train.navigation.distance_to_destination = 33.0;
        let Value::Compound(tag) = write_train(&train, nets.graphs.get(&g), &dim_name) else { panic!() };
        let back = read_train(&tag, &nets, &dim_id).expect("read").train;
        assert_eq!(back.navigation.destination, None);
    }

    #[test]
    fn a_derailed_train_with_no_wheels_on_track_reads_at_once() {
        let nets = RailNetworks::new();
        let b = CarriageBogey::new(BogeyType::small(), false, vec![], TravellingPoint::empty(), TravellingPoint::empty(), 0);
        let mut train = Train::new(8, None, None, vec![Carriage::new(1, b, None, 0)], vec![], false, "Lost", "create:traditional", 0);
        train.derailed = true;
        let Value::Compound(tag) = write_train(&train, None, &dim_name) else { panic!() };
        let loaded = read_train(&tag, &nets, &dim_id).expect("read");
        assert!(loaded.train.derailed);
        assert_eq!(loaded.graph, None);
    }

    #[test]
    fn a_location_keeps_its_dimension() {
        let l = TrackNodeLocation { x: 4, y: 10, z: -6, dimension: 2, y_offset_pixels: 3 };
        let Value::Compound(tag) = write_location(&l, &dim_name) else { panic!() };
        assert_eq!(read_location(&tag, &dim_id), l);
    }

    #[test]
    fn a_runtime_survives_its_nbt() {
        let mut rt = ScheduleRuntime::new();
        rt.is_auto_schedule = true;
        rt.condition_context.push(ConditionContext { time: 4, status_version: 2, last_checked: Some(5), current_display: None, prev_player_count: 1 });
        let Value::Compound(tag) = write_runtime(&rt) else { panic!() };
        let back = read_runtime(&tag);
        assert!(back.is_auto_schedule);
        assert_eq!(back.condition_context, rt.condition_context);
    }
}
