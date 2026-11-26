//! What the blocks that sit on an edge point share (`TrackTargetingBehaviour`, `GlobalRailwayManager.tick`'s points half):
//! the track graph questions the rules ask of the blocks ([`Query`]), a block entity's targeting data and its edge point
//! ([`load`], [`store`], [`ensure_point`]), the sweep that removes a point whose block entity is gone, and the tick of the
//! graphs' points. The decisions are `create_kinetics::rules::trains::{targeting, graph_helper, edge_blocks}`; the graphs
//! are kept by `track.rs`.
//!
//! Known differences from Create, for the README:
//! - A block entity ensures its edge point each tick (`TrackTargetingBehaviour.tick`), and a point whose block entity is gone
//!   is found by a sweep every few ticks (`destroy()` isn't a hook): a station, signal or observer the world loses by any means
//!   leaves the graph within a second.
//! - The graphs are not saved (`track.rs`), so the points are made again from the block entities' own saved data (their `Id`,
//!   their target) after a load; a station keeps its name in its block entity for that.
//! - The graphs' points are ticked by `trains.rs` (`Railways::tick`: `tickAllPoints` before and after the trains).

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};

use create_kinetics::rules::trains::bezier::BezierConnection;
use create_kinetics::rules::trains::graph::{DiscoveredLocation, EdgePointType, TrackEdgePoint, TrackNodeLocation, Uuid};
use create_kinetics::rules::trains::graph_helper::{TrackQuery, bezier_graph_location_at, graph_location_at};
use create_kinetics::rules::trains::network::RailNetworks;
use create_kinetics::rules::trains::propagator::TrackWorld;
use create_kinetics::rules::trains::targeting::{BezierTrackPointLocation, BlockEntityInfo, Placement, TargetingState};
use create_kinetics::rules::trains::vec::Vec3;
use create_kinetics::Pos;
use crate::compat::{Compound, Hooks, Host, LevelTickPostEvent, Tag, Text, TextExt};
use create_kinetics::rules::trains::edge_blocks::format_message;

use super::track::{self as world, HostTracks};
use crate::util;

pub fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// --- the track, as the rules ask ----------------------------------------------------------------------------------

/// `ITrackBlock` over the host's blocks (`track.rs`'s `HostTracks` and the block entity's curves).
pub struct Query<'a> {
    tracks: HostTracks<'a>,
}

pub fn query(host: &dyn Host) -> Query<'_> {
    Query { tracks: HostTracks { host, dimension: world::dimension_id(host.dimension()) } }
}

impl TrackWorld for Query<'_> {
    fn connected(&self, pos: Pos, ignore_turns: bool, connected_to: Option<&TrackNodeLocation>) -> Vec<DiscoveredLocation> {
        self.tracks.connected(pos, ignore_turns, connected_to)
    }
}

impl TrackQuery for Query<'_> {
    fn axes(&self, pos: Pos) -> Vec<Vec3> {
        world::shape_at(self.tracks.host, pos).map(|s| s.axes()).unwrap_or_default()
    }

    fn elevation_at_center(&self, pos: Pos) -> f64 {
        // `isSlope`: the first axis rises.
        if self.axes(pos).first().is_some_and(|a| a[1] != 0.0) { 0.5 } else { 0.0 }
    }

    fn curve(&self, pos: Pos, target: Pos) -> Option<BezierConnection> {
        world::connections_at(self.tracks.host, pos).into_iter().find(|bc| bc.key() == target)
    }

    fn dimension(&self) -> u32 {
        self.tracks.dimension
    }
}

/// Runs `f` on the dimension's graphs; `None` before the track code has ticked the dimension.
pub fn with_nets<R>(host: &dyn Host, f: impl FnOnce(&mut RailNetworks) -> R) -> Option<R> {
    world::with_networks(host.dimension(), f)
}

/// Runs `f` on the edge point of type `ty` with `id`, wherever its graph is.
pub fn with_point<R>(host: &dyn Host, ty: EdgePointType, id: Uuid, f: impl FnOnce(&mut TrackEdgePoint) -> R) -> Option<R> {
    with_nets(host, |nets| nets.graphs.values_mut().find_map(|g| g.edge_points.get_mut(ty, id)).map(f)).flatten()
}

// --- the targeting data of a block entity (`TrackTargetingBehaviour.write` and `read`) ----------------------------------------------------

const ID: &str = "Id";
const TARGET_TRACK: &str = "TargetTrack";
const ORTHO: &str = "Ortho";
const TARGET_DIRECTION: &str = "TargetDirection";
const ROTATED_AXIS: &str = "RotatedAxis";
const PREV_AXIS: &str = "PrevAxis";
const BEZIER: &str = "Bezier";

fn uuid_tag(id: Uuid) -> Tag {
    Tag::IntArray(vec![(id >> 96) as i32, (id >> 64) as i32, (id >> 32) as i32, id as i32])
}

fn uuid_of(tag: &Tag) -> Option<Uuid> {
    match tag {
        Tag::IntArray(v) if v.len() == 4 => Some(v.iter().fold(0u128, |acc, part| (acc << 32) | u128::from(*part as u32))),
        _ => None,
    }
}

fn pos_tag(p: Pos) -> Tag {
    Tag::IntArray(p.to_vec())
}

fn pos_of(tag: &Tag) -> Option<Pos> {
    match tag {
        Tag::IntArray(v) if v.len() == 3 => Some([v[0], v[1], v[2]]),
        _ => None,
    }
}

/// A new id: `UUID.randomUUID()`.
pub fn new_id() -> Uuid {
    (u128::from(util::random()) << 64 | u128::from(util::random())) | 1
}

/// `TrackTargetingBehaviour.read` for the block entity at `pos`; a missing `Id` is a new random one.
pub fn load(nbt: &Compound, ty: EdgePointType, pos: Pos) -> TargetingState {
    let id = util::get(nbt, ID).and_then(uuid_of).unwrap_or_else(new_id);
    let mut state = TargetingState::new(ty, id);
    state.target_track = util::get(nbt, TARGET_TRACK).and_then(pos_of).unwrap_or([0, 0, 0]);
    // `TargetDirection` defaults to positive in the constructor, but `read` takes `getBoolean`: absent is false.
    state.target_positive = util::has(nbt, TARGET_DIRECTION) && util::boolean(nbt, TARGET_DIRECTION);
    state.orthogonal = util::boolean(nbt, ORTHO);
    state.prev_direction = util::get(nbt, PREV_AXIS).and_then(world::vec_of);
    state.rotated_direction = util::get(nbt, ROTATED_AXIS).and_then(world::vec_of);
    if let Some(bezier) = util::get(nbt, BEZIER)
        && let Some(key) = bezier.get("Key").and_then(pos_of)
    {
        let segment = match bezier.get("Segment") {
            Some(Tag::Int(v)) => *v,
            _ => 0,
        };
        state.target_bezier = Some(BezierTrackPointLocation { curve_target: [key[0] + pos[0], key[1] + pos[1], key[2] + pos[2]], segment });
    }
    state
}

/// `TrackTargetingBehaviour.write`.
pub fn store(state: &TargetingState, nbt: &mut Compound, pos: Pos) {
    nbt.insert(ID.into(), uuid_tag(state.id));
    nbt.insert(TARGET_TRACK.into(), pos_tag(state.target_track));
    nbt.insert(ORTHO.into(), Tag::Byte(i8::from(state.orthogonal)));
    nbt.insert(TARGET_DIRECTION.into(), Tag::Byte(i8::from(state.target_positive)));
    match state.rotated_direction {
        Some(v) => nbt.insert(ROTATED_AXIS.into(), world::vec_tag(v)),
        None => util::remove(nbt, ROTATED_AXIS),
    };
    match state.prev_direction {
        Some(v) => nbt.insert(PREV_AXIS.into(), world::vec_tag(v)),
        None => util::remove(nbt, PREV_AXIS),
    };
    match state.target_bezier {
        Some(b) => {
            let mut c = Compound::new();
            c.insert("Segment".into(), Tag::Int(b.segment));
            let key = [b.curve_target[0] - pos[0], b.curve_target[1] - pos[1], b.curve_target[2] - pos[2]];
            c.insert("Key".into(), pos_tag(key));
            nbt.insert(BEZIER.into(), Tag::Compound(c))
        }
        None => util::remove(nbt, BEZIER),
    };
}

/// What the item stores in a new block's data (`TrackTargetingBlockItem.useOn`'s `blockEntityData`): where the track is
/// from the block, which way it faces and the curve.
pub fn item_data(selected: Pos, positive: bool, bezier: Option<BezierTrackPointLocation>, placed: Pos) -> Compound {
    let mut nbt = Compound::new();
    nbt.insert(TARGET_DIRECTION.into(), Tag::Byte(i8::from(positive)));
    let offset = [selected[0] - placed[0], selected[1] - placed[1], selected[2] - placed[2]];
    nbt.insert(TARGET_TRACK.into(), pos_tag(offset));
    if let Some(b) = bezier {
        let mut c = Compound::new();
        c.insert("Segment".into(), Tag::Int(b.segment));
        let key = [b.curve_target[0] - placed[0], b.curve_target[1] - placed[1], b.curve_target[2] - placed[2]];
        c.insert("Key".into(), pos_tag(key));
        nbt.insert(BEZIER.into(), Tag::Compound(c));
    }
    nbt
}

// --- the edge point of a block entity ---------------------------------------------------------------------------------

/// How an `ensure_point` went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ensured {
    /// The point is in a graph.
    Present,
    /// A point was made, or merged into one that was there; the targeting data changed (the id, the direction).
    Placed,
    /// Nothing yet: no graph, no track, something in the way, or a flipped direction to try again.
    Missing { state_changed: bool },
}

/// `TrackTargetingBehaviour.tick` and `createEdgePoint`: finds the point with the behaviour's id in a graph, or places it
/// from the block entity's data.
pub fn ensure_point(host: &dyn Host, state: &mut TargetingState, info: &BlockEntityInfo) -> Ensured {
    let ty = state.edge_point_type;
    let track = state.global_position(info.pos);
    let q = query(host);
    let positive = state.target_positive;
    let result = with_nets(host, |nets| {
        if nets.graphs.values().any(|g| g.point(ty, state.id).is_some()) {
            return Ensured::Present;
        }
        let axes = q.axes(track);
        if axes.is_empty() {
            return Ensured::Missing { state_changed: false };
        }
        let location = match state.target_bezier {
            Some(bezier) => bezier_graph_location_at(&q, nets, track, positive, bezier),
            None => graph_location_at(&q, nets, track, if positive { 1 } else { -1 }, axes[0]),
        };
        let Some(location) = location else { return Ensured::Missing { state_changed: false } };
        let before = state.clone();
        match state.create_edge_point(nets, &location, info) {
            Placement::Created(_) | Placement::Merged(_) => Ensured::Placed,
            Placement::Nothing => Ensured::Missing { state_changed: *state != before },
        }
    });
    result.unwrap_or(Ensured::Missing { state_changed: false })
}

// --- the sweep and the tick of the points ---------------------------------------------------------------------------------

/// Redstone outputs the blocks answer from (the signal's red, the station's train, the observer's power), by dimension and
/// position: what `getSignal` and `getAnalogOutputSignal` read in Java from the block entity.
static OUTPUTS: LazyLock<Mutex<HashMap<(u8, Pos), i32>>> = LazyLock::new(Mutex::default);

pub fn set_output(dimension: u8, pos: Pos, value: i32) {
    let mut table = lock(&OUTPUTS);
    if value == 0 {
        table.remove(&(dimension, pos));
    } else {
        table.insert((dimension, pos), value);
    }
}

pub fn output(dimension: u8, pos: Pos) -> i32 {
    lock(&OUTPUTS).get(&(dimension, pos)).copied().unwrap_or(0)
}

/// The power an observer gives (`TrackObserverBlock.getSignal`), by dimension and position.
static SOURCES: LazyLock<Mutex<HashMap<(u8, Pos), i32>>> = LazyLock::new(Mutex::default);

pub fn set_source(dimension: u8, pos: Pos, value: i32) {
    let mut table = lock(&SOURCES);
    if value == 0 {
        table.remove(&(dimension, pos));
    } else {
        table.insert((dimension, pos), value);
    }
}

pub fn source(dimension: u8, pos: Pos) -> i32 {
    lock(&SOURCES).get(&(dimension, pos)).copied().unwrap_or(0)
}

/// The block each kind of point's block entity stands in.
fn block_of(ty: EdgePointType) -> &'static str {
    match ty {
        EdgePointType::Signal => "create:track_signal",
        EdgePointType::Station => "create:track_station",
        EdgePointType::Observer => "create:track_observer",
    }
}

/// Every block entity position each point of the dimension knows, with the kind and the point's id.
fn known_block_entities(nets: &RailNetworks) -> Vec<(EdgePointType, Uuid, Pos)> {
    let mut out = Vec::new();
    for graph in nets.graphs.values() {
        for ty in EdgePointType::ALL {
            for point in graph.edge_points.values(ty) {
                if let Some(signal) = point.signal() {
                    out.extend(signal.block_entities.iter().flat_map(|side| side.keys()).map(|p| (ty, point.id, *p)));
                }
                if let Some(station) = point.station().and_then(|s| s.block_entity_pos) {
                    out.push((ty, point.id, station));
                }
                if let Some(observer) = point.observer().and_then(|o| o.block_entity_pos) {
                    out.push((ty, point.id, observer));
                }
            }
        }
    }
    out
}

/// The points whose block entity is not there any more go (`destroy()`).
fn sweep(host: &dyn Host) {
    let Some(known) = with_nets(host, |nets| known_block_entities(nets)) else { return };
    let level = host.level(host.dimension());
    let mut gone = Vec::new();
    for (ty, id, pos) in known {
        if !level.is_some_and(|level| host.level_is_loaded(level, pos)) {
            continue;
        }
        if host.block(pos) != block_of(ty) {
            gone.push((ty, id, pos));
        }
    }
    if gone.is_empty() {
        return;
    }
    with_nets(host, |nets| {
        for (ty, id, pos) in gone {
            let state = TargetingState::new(ty, id);
            state.block_entity_removed(nets, id, pos);
        }
    });
}

/// Every level tick: the points of lost block entities are swept.
pub fn level_tick(host: &mut dyn Host, _: &mut LevelTickPostEvent<'_>) {
    if host.window() {
        return;
    }
    // The points are ticked by `trains.rs` through `Railways::tick`, before and after the trains.
    let now = util::game_time(&*host);
    if now % 5 == 0 {
        sweep(&*host);
    }
}

// --- messages ---------------------------------------------------------------------------------------------------

/// A lang line in the action bar (`displayClientMessage(.., true)`).
pub fn say(host: &mut dyn Host, uuid: u128, key: &str, args: &[String]) {
    let text = format_message(key, args);
    host.action_bar(uuid, Text::translate_with_fallback(key, &text, &[]));
}

/// A lang line in the chat.
pub fn tell(host: &mut dyn Host, uuid: u128, line: impl Into<String>) {
    let line = line.into();
    // A chat line the host refuses (the player can't be reached for it right now) still has to reach the player: the action bar
    // is the fallback, so a command's answer is never dropped silently.
    if !host.tell(uuid, Text::literal(line.clone())) {
        host.action_bar(uuid, Text::literal(line));
    }
}

struct Net;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Net)
}

impl Hooks for Net {
    fn level_tick_post(&self, host: &mut dyn Host, event: &mut LevelTickPostEvent<'_>) {
        level_tick(host, event);
    }
}
