//! The Train Track in the world (`TrackBlock`, `TrackBlockEntity`, `TrackPropagator`'s callers): the block, the curves
//! its block entity holds, and the track graphs that follow the blocks. The decisions are
//! `create_kinetics::rules::trains::{track_block, placement, propagator, network}`; this reads the world, asks them and
//! writes what they say. The click flow that places a curve is `track_item.rs`; the curves are drawn by `track_draw.rs`.
//!
//! # How the track graph is kept
//!
//! `TrackWorld` is implemented over the host ([`HostTracks`]: a block's `shape` and `turn` properties and its block
//! entity's `Connections`). Each dimension keeps one `RailNetworks` in a static ([`with_networks`]). Placing and
//! breaking are not hooks the graph can hang on, so every change is queued and applied at the level's tick:
//! the placement hooks (`place_block`, the curve flow) queue a block as added, the break hook queues it as removed
//! after cutting its curves, and a slow sweep over the known track queues whatever vanished by other means
//! (explosions, pistons, `setblock`); `onPlace`'s own scheduled tick is replaced by the queue.
//!
//! # What is saved
//!
//! Create saves the graphs (`RailwaySavedData`, not ported). Here a level attachment (`create:track_network`) keeps a
//! token that names the world and the positions of every known track block; after a load the graphs are rebuilt by
//! running `onRailAdded` over those positions, as chunks come in. What a graph holds besides track (signals, stations,
//! trains) will need real saving with its own parts.
//!
//! Known differences from Create, for the README:
//! - Every track block has a block entity (Horizon can't ask for one only when `turn` is set); `turn` is kept as
//!   Create's blockstate expects it.
//! - A block placed by `setblock` or a structure joins the graph only once something next to it is built through the
//!   item, or never: `onPlace` is not a hook.
//! - No tilt/smoothing (`TrackBlockEntityTilt`), no portal tracks, no girder on curves, no fake track markers,
//!   no wrench pick-up of a curve's items (`onSneakWrenched`), no `getRequiredItems` for schematics.
//! - A curve's items drop for a survival player breaking an end, not for explosions.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::ops::Bound::{Excluded, Unbounded};
use std::sync::{LazyLock, Mutex};

use create_kinetics::rules::contraption::{State, replaceable as vanilla_replaceable};
use create_kinetics::rules::trains::bezier::BezierConnection;
use create_kinetics::rules::trains::graph::{DiscoveredLocation, TrackNodeLocation};
use create_kinetics::rules::trains::network::RailNetworks;
use create_kinetics::rules::trains::railways::Railways;
use create_kinetics::rules::trains::propagator::TrackWorld;
use create_kinetics::rules::trains::track_block::{self as rules, CONNECTIONS, MATERIAL, TRACK, TrackView};
use create_kinetics::rules::trains::track_shape::TrackShape;
use create_kinetics::rules::trains::vec::{Vec3, block_containing};
use create_kinetics::{Dir, Pos};
use crate::compat::{
    AttachmentCodec, AttachmentType, BlockEntityEvent, BlockStateId, Compound, DestroyBlockEvent, Flow, Holder, Hooks,
    Host, ItemStack, LevelHost, LevelTickPostEvent, PlaceBlockEvent, StateForPlacementEvent, Tag,
};

use crate::util;

struct Tracks;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Tracks)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

// --- the block's properties ---------------------------------------------------------------------------------

/// `TrackShape` from a `shape` property value (its serialized name).
pub fn shape_named(name: &str) -> Option<TrackShape> {
    TrackShape::ALL.into_iter().find(|s| s.serialized_name() == name)
}

/// The shape of the track block at `pos`, `None` when it is not one.
pub fn shape_at(host: &dyn Host, pos: Pos) -> Option<TrackShape> {
    if host.block(pos) != TRACK {
        return None;
    }
    Some(host.block_property(pos, "shape").as_deref().and_then(shape_named).unwrap_or(TrackShape::Zo))
}

fn has_turn(host: &dyn Host, pos: Pos) -> bool {
    host.block_property(pos, "turn").as_deref() == Some("true")
}

// --- the curves, as Create saves them (`BezierConnection.write` and its constructor) -----------------------------

fn double(tag: &Tag, key: &str) -> Option<f64> {
    match tag.get(key) {
        Some(Tag::Double(bits)) => Some(f64::from_bits(*bits)),
        _ => None,
    }
}

pub(crate) fn vec_tag(v: Vec3) -> Tag {
    let mut c = Compound::new();
    for (key, value) in [("X", v[0]), ("Y", v[1]), ("Z", v[2])] {
        c.insert(key.into(), Tag::Double(value.to_bits()));
    }
    Tag::Compound(c)
}

pub(crate) fn vec_of(tag: &Tag) -> Option<Vec3> {
    Some([double(tag, "X")?, double(tag, "Y")?, double(tag, "Z")?])
}

fn pair<T>(tag: &Tag, key: &str, read: impl Fn(&Tag) -> Option<T>) -> Option<[T; 2]> {
    let Some(Tag::List(list)) = tag.get(key) else { return None };
    let mut it = list.iter();
    Some([read(it.next()?)?, read(it.next()?)?])
}

fn flag(tag: &Tag, key: &str) -> bool {
    matches!(tag.get(key), Some(Tag::Byte(b)) if *b != 0)
}

/// `new BezierConnection(compound, localTo)`.
pub fn read_connection(tag: &Tag, local_to: Pos) -> Option<BezierConnection> {
    let positions = pair(tag, "Positions", |t| match t.get("Pos") {
        Some(Tag::IntArray(v)) if v.len() == 3 => Some([v[0] + local_to[0], v[1] + local_to[1], v[2] + local_to[2]]),
        _ => None,
    })?;
    let corner = [f64::from(local_to[0]), f64::from(local_to[1]), f64::from(local_to[2])];
    let starts = pair(tag, "Starts", vec_of)?.map(|v| [v[0] + corner[0], v[1] + corner[1], v[2] + corner[2]]);
    let axes = pair(tag, "Axes", vec_of)?;
    let normals = pair(tag, "Normals", vec_of)?;
    let material = match tag.get("Material") {
        Some(Tag::String(m)) => m.to_string_lossy(),
        _ => MATERIAL.to_owned(),
    };
    Some(BezierConnection::new(positions, starts, axes, normals, flag(tag, "Primary"), flag(tag, "Girder"), material))
}

/// `BezierConnection.write(localTo)`.
pub fn write_connection(bc: &BezierConnection, local_to: Pos) -> Tag {
    let corner = [f64::from(local_to[0]), f64::from(local_to[1]), f64::from(local_to[2])];
    let mut c = Compound::new();
    c.insert("Girder".into(), Tag::Byte(i8::from(bc.has_girder)));
    c.insert("Primary".into(), Tag::Byte(i8::from(bc.primary)));
    let positions = bc.be_positions.iter().map(|p| {
        let mut entry = Compound::new();
        entry.insert("Pos".into(), Tag::IntArray(vec![p[0] - local_to[0], p[1] - local_to[1], p[2] - local_to[2]]));
        Tag::Compound(entry)
    });
    c.insert("Positions".into(), Tag::List(positions.collect()));
    let starts = bc.starts.iter().map(|v| vec_tag([v[0] - corner[0], v[1] - corner[1], v[2] - corner[2]]));
    c.insert("Starts".into(), Tag::List(starts.collect()));
    c.insert("Axes".into(), Tag::List(bc.axes.iter().map(|v| vec_tag(*v)).collect()));
    c.insert("Normals".into(), Tag::List(bc.normals.iter().map(|v| vec_tag(*v)).collect()));
    c.insert("Material".into(), Tag::String(bc.material.as_str().into()));
    Tag::Compound(c)
}

/// The curves a block entity's data holds (`TrackBlockEntity.connections`), each as seen from the block at `pos`.
pub fn connections_in(nbt: &Compound, pos: Pos) -> Vec<BezierConnection> {
    let Some(Tag::List(list)) = util::get(nbt, CONNECTIONS) else { return Vec::new() };
    list.iter().filter_map(|t| read_connection(t, pos)).collect()
}

fn write_connections(nbt: &mut Compound, pos: Pos, curves: &[BezierConnection]) {
    let list = curves.iter().map(|bc| write_connection(bc, pos)).collect();
    nbt.insert(CONNECTIONS.into(), Tag::List(list));
}

/// The curves of the block at `pos`.
pub fn connections_at(host: &dyn Host, pos: Pos) -> Vec<BezierConnection> {
    host.mod_block_entity(pos).map(|e| connections_in(&e.nbt, pos)).unwrap_or_default()
}

/// `TrackBlockEntity.addConnection`: keeps an equal curve already there (whatever its material), else puts it in.
pub fn add_connection(host: &mut dyn Host, pos: Pos, bc: &BezierConnection) {
    let mut curves = connections_at(&*host, pos);
    let key = bc.key();
    if curves.iter().any(|c| c.key() == key && bc.equals_sans_material(c)) {
        return;
    }
    curves.retain(|c| c.key() != key);
    curves.push(bc.clone());
    host.change_mod_block_entity(pos, &mut |e| write_connections(&mut e.nbt, pos, &curves));
    host.set_block_property(pos, "turn", "true");
    host.block_entity_sync(pos);
}

/// `TrackBlockEntity.removeConnection`: drops the curve to `target`; the last one gone lets `turn` fall back.
pub fn remove_connection(host: &mut dyn Host, pos: Pos, target: Pos) {
    let mut curves = connections_at(&*host, pos);
    let before = curves.len();
    curves.retain(|c| c.key() != target);
    if curves.len() == before {
        return;
    }
    host.change_mod_block_entity(pos, &mut |e| write_connections(&mut e.nbt, pos, &curves));
    let shape = shape_at(&*host, pos).unwrap_or(TrackShape::Zo);
    if rules::drops_block_entity(curves.len(), shape) {
        host.set_block_property(pos, "turn", "false");
    }
    host.block_entity_sync(pos);
}

// --- TrackWorld over the host ----------------------------------------------------------------------------------

static DIMENSION_NAMES: LazyLock<Mutex<Vec<String>>> = LazyLock::new(Mutex::default);

/// The dimension index the rules number a dimension by (`ResourceKey<Level>` there).
pub fn dimension_id(name: &str) -> u32 {
    let mut names = lock(&DIMENSION_NAMES);
    if let Some(i) = names.iter().position(|n| n == name) {
        return i as u32;
    }
    names.push(name.to_owned());
    (names.len() - 1) as u32
}

/// The name of a dimension index (the inverse of [`dimension_id`]); an index never named is "minecraft:overworld".
pub fn dimension_name(id: u32) -> String {
    lock(&DIMENSION_NAMES).get(id as usize).cloned().unwrap_or_else(|| "minecraft:overworld".to_owned())
}

/// `ITrackBlock.getConnected` for the real blocks.
pub struct HostTracks<'a> {
    pub host: &'a dyn Host,
    pub dimension: u32,
}

impl TrackWorld for HostTracks<'_> {
    fn connected(&self, pos: Pos, ignore_turns: bool, connected_to: Option<&TrackNodeLocation>) -> Vec<DiscoveredLocation> {
        let Some(shape) = shape_at(self.host, pos) else { return Vec::new() };
        let has_be = has_turn(self.host, pos);
        let curves = if has_be && !ignore_turns { connections_at(self.host, pos) } else { Vec::new() };
        let host = self.host;
        let material = |v: Vec3, default: &str| {
            if host.block(block_containing(v)) == TRACK { MATERIAL.to_owned() } else { default.to_owned() }
        };
        let view = TrackView { pos, shape, has_be, dimension: self.dimension, curves: &curves, material: &material };
        rules::track_ends(&view, ignore_turns, connected_to)
    }
}

// --- the graphs of each dimension ---------------------------------------------------------------------------------

/// `AttachmentType` of the saved part: a token naming the world and the known track positions.
static NETWORK: AttachmentType<Compound> = AttachmentType {
    namespace: "create",
    name: "track_network",
    default: Compound::new,
    codec: Some(AttachmentCodec { encode: |data| Ok(data.clone()), decode: |data| Ok(data.clone()) }),
    copy_on_death: false,
    sync: false,
};

const TOKEN: &str = "Token";
const POSITIONS: &str = "Tracks";
/// Track positions looked at in one tick by the sweep for blocks that vanished.
const SWEEP: usize = 96;
/// Queued changes applied in one tick.
const BUDGET: usize = 96;

#[derive(Default)]
struct DimState {
    token: i64,
    time: i64,
    /// The graphs and the trains on them (`GlobalRailwayManager.sided(level)`); `trains.rs` ticks it.
    rails: Railways,
    tracks: BTreeSet<Pos>,
    /// The ends each track block had when last added, so its removal knows what to take out of the graphs.
    known: HashMap<Pos, Vec<DiscoveredLocation>>,
    /// The curve partners each block had when last added (to cut them if the block goes without the break hook).
    partners: HashMap<Pos, Vec<Pos>>,
    added: VecDeque<Pos>,
    removed: VecDeque<Pos>,
    /// Positions to add after a load, and those whose chunk wasn't there yet.
    rebuild: VecDeque<Pos>,
    waiting: Vec<Pos>,
    cursor: Option<Pos>,
    dirty: bool,
}

static STATE: LazyLock<Mutex<HashMap<String, DimState>>> = LazyLock::new(Mutex::default);

/// Runs `f` on a dimension's track graphs (`GlobalRailwayManager.sided(level)` for it), `None` before the dimension has
/// been ticked.
#[allow(dead_code)]
pub fn with_networks<R>(dimension: &str, f: impl FnOnce(&mut RailNetworks) -> R) -> Option<R> {
    lock(&STATE).get_mut(dimension).map(|st| f(&mut st.rails.nets))
}

/// Runs `f` on a dimension's graphs without waiting for the server's tick to let go of them: for the client's reads in a single-player
/// world (`None` when they are busy, when the dimension was never ticked here, or on a client that has no graphs).
pub fn try_with_networks<R>(dimension: &str, f: impl FnOnce(&RailNetworks) -> R) -> Option<R> {
    let states = STATE.try_lock().ok()?;
    states.get(dimension).map(|st| f(&st.rails.nets))
}

/// Runs `f` on a dimension's railways: the graphs and the trains (`trains.rs`).

pub fn with_railways<R>(dimension: &str, f: impl FnOnce(&mut Railways) -> R) -> Option<R> {
    lock(&STATE).get_mut(dimension).map(|st| f(&mut st.rails))
}

/// A track block was placed or changed: its graph is updated at the next tick.
pub fn queue_added(dimension: &str, pos: Pos) {
    let mut states = lock(&STATE);
    let st = states.entry(dimension.to_owned()).or_default();
    if !st.added.contains(&pos) {
        st.added.push_back(pos);
    }
}

fn queue_removed(dimension: &str, pos: Pos) {
    let mut states = lock(&STATE);
    let st = states.entry(dimension.to_owned()).or_default();
    if !st.removed.contains(&pos) {
        st.removed.push_back(pos);
    }
}

fn new_token() -> i64 {
    (util::random() as i64) | 1
}

fn save(st: &DimState) -> Compound {
    let mut c = Compound::new();
    c.insert(TOKEN.into(), Tag::Long(st.token));
    c.insert(POSITIONS.into(), Tag::IntArray(st.tracks.iter().flatten().copied().collect()));
    c
}

/// Reads the level's attachment: a new world (another token, or none) starts from nothing; a saved one queues its
/// positions to be added again.
fn sync(host: &mut dyn Host, st: &mut DimState, now: i64) {
    let stored = NETWORK.get_existing(host.new_server(), Holder::Level).ok().flatten();
    let stored_token = stored.as_ref().map(|c| util::long(c, TOKEN)).unwrap_or(0);
    if stored_token == 0 || stored_token != st.token {
        *st = DimState::default();
        match stored.as_ref().filter(|_| stored_token != 0) {
            Some(c) => {
                st.token = stored_token;
                if let Some(Tag::IntArray(v)) = util::get(c, POSITIONS) {
                    st.rebuild = v.chunks_exact(3).map(|p| [p[0], p[1], p[2]]).collect();
                }
            }
            None => {
                st.token = new_token();
                st.dirty = true;
            }
        }
    }
    st.time = now;
    if st.dirty {
        let _ = NETWORK.set(host.new_server(), Holder::Level, save(st));
        st.dirty = false;
    }
}

/// Everything the tracks do each tick of a level.
fn tick(host: &mut dyn Host) {
    let dimension = host.dimension().to_owned();
    let now = util::game_time(&*host);
    let id = dimension_id(&dimension);
    // The state is out of the map while it works, so nothing it calls can find the lock held.
    let queued = lock(&STATE).remove(&dimension);
    let mut st = queued.unwrap_or_default();
    // What was queued before this world's first sync is kept across the reset a new world makes.
    let (added, removed) = (std::mem::take(&mut st.added), std::mem::take(&mut st.removed));
    if st.token == 0 || now < st.time || now % 20 == 0 {
        sync(host, &mut st, now);
    }
    st.added.extend(added);
    st.removed.extend(removed);
    run(&*host, &mut st, id, now);
    let mut states = lock(&STATE);
    // Anything queued meanwhile (by a hook the work called) joins what is there.
    if let Some(late) = states.remove(&dimension) {
        st.added.extend(late.added);
        st.removed.extend(late.removed);
    }
    states.insert(dimension, st);
}

fn run(host: &dyn Host, st: &mut DimState, id: u32, now: i64) {
    let world = HostTracks { host, dimension: id };
    // removals first (a block put back in the same tick is then added again)
    for _ in 0..BUDGET {
        let Some(pos) = st.removed.pop_front() else { break };
        if host.block(pos) == TRACK {
            continue;
        }
        let ends = st.known.remove(&pos).unwrap_or_default();
        st.partners.remove(&pos);
        if st.tracks.remove(&pos) {
            st.dirty = true;
        }
        st.rails.nets.on_rail_removed(&world, pos, &ends);
    }
    for _ in 0..BUDGET {
        let Some(pos) = st.added.pop_front() else { break };
        add_now(&world, st, pos);
    }
    if now % 40 == 0 && !st.waiting.is_empty() {
        let back = std::mem::take(&mut st.waiting);
        st.rebuild.extend(back);
    }
    let level = host.level(host.dimension());
    for _ in 0..BUDGET {
        let Some(pos) = st.rebuild.pop_front() else { break };
        if !level.is_some_and(|level| host.level_is_loaded(level, pos)) {
            st.waiting.push(pos);
            continue;
        }
        if host.block(pos) == TRACK {
            add_now(&world, st, pos);
        } else if st.tracks.remove(&pos) {
            st.dirty = true;
        }
    }
    sweep(host, st, level);
    // What the track's changes do to the trains on it (`removeNode`, `transfer`, `notifyTrains`).
    let events = st.rails.nets.drain_events();
    st.rails.apply_rail_events(&events);
}

fn add_now(world: &HostTracks<'_>, st: &mut DimState, pos: Pos) {
    if world.host.block(pos) != TRACK {
        return;
    }
    if st.tracks.insert(pos) {
        st.dirty = true;
    }
    st.known.insert(pos, world.connected(pos, false, None));
    let partners = connections_at(world.host, pos).iter().map(|bc| bc.key()).collect();
    st.partners.insert(pos, partners);
    let _ = st.rails.nets.on_rail_added(world, pos);
}

/// A few known blocks each tick: any that is gone is queued as removed (and its curves cut).
fn sweep(host: &dyn Host, st: &mut DimState, level: Option<crate::compat::LevelRef>) {
    let Some(level) = level else { return };
    let from = st.cursor;
    let mut taken: Vec<Pos> = match from {
        Some(c) => st.tracks.range((Excluded(c), Unbounded)).take(SWEEP).copied().collect(),
        None => st.tracks.iter().take(SWEEP).copied().collect(),
    };
    if taken.len() < SWEEP && from.is_some() {
        taken.extend(st.tracks.iter().take(SWEEP - taken.len()).copied());
    }
    st.cursor = taken.last().copied();
    for pos in taken {
        if host.level_is_loaded(level, pos) && host.block(pos) != TRACK && !st.removed.contains(&pos) {
            st.removed.push_back(pos);
        }
    }
}

// --- breaking -----------------------------------------------------------------------------------------------------

/// `TrackBlockEntity.removeInboundConnections(dropAndDiscard)`: each curve of the block at `pos` is cut at its other
/// end, and its items drop unless `drop` is false (`cancelDrops`, a creative player).
fn cut_curves(host: &mut dyn Host, pos: Pos, drop: bool) {
    for bc in connections_at(&*host, pos) {
        let partner = bc.key();
        if host.block(partner) != TRACK {
            continue;
        }
        remove_connection(host, partner, pos);
        queue_added(&host.dimension().to_owned(), partner);
        if !drop {
            continue;
        }
        let origin = bc.be_positions[0];
        for (at, girders) in rules::drop_points(&bc) {
            let at = [at[0] + f64::from(origin[0]), at[1] + f64::from(origin[1]), at[2] + f64::from(origin[2])];
            let stack = ItemStack { item: TRACK.into(), count: 1, damage: 0, extra: None };
            util::spawn_item(host, at, &stack, [0.0; 3], 10);
            for _ in 0..girders {
                let girder = ItemStack { item: "create:metal_girder".into(), count: 1, damage: 0, extra: None };
                util::spawn_item(host, at, &girder, [0.0; 3], 10);
            }
        }
    }
}

// --- placing helpers shared with the item flow ---------------------------------------------------------------

/// `BlockState.canBeReplaced` or a flower (`placeTracks`' `canPlace`).
pub fn can_replace(host: &dyn Host, pos: Pos) -> bool {
    let name = host.block(pos);
    vanilla_replaceable(&State::new(&name)) || matches!(name.as_str(), "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air")
        || host.block_name_in_tag(&name, "minecraft:flowers").unwrap_or(false)
}

/// `isFaceSturdy(UP)` of the block at `pos`, as its collision says.
pub fn sturdy_top(host: &dyn Host, pos: Pos) -> bool {
    crate::belt::face_full(host, pos, Dir::Up)
}

/// `isFaceSturdy(DOWN)`.
pub fn sturdy_bottom(host: &dyn Host, pos: Pos) -> bool {
    crate::belt::face_full(host, pos, Dir::Down)
}

/// `TrackBlock.getStateForPlacement`'s shape for a block put at `pos`, looking along `look` with `yaw`.
pub fn placement_shape(host: &dyn Host, pos: Pos, look: Vec3, yaw: f32) -> TrackShape {
    let best = rules::shape_for_look(look, yaw);
    rules::slope_fix(best, pos, &|p| sturdy_top(host, p), &|p| sturdy_bottom(host, p))
}

/// The player's look (`getLookAngle`) from its yaw and pitch.
pub fn look_of(yaw: f32, pitch: f32) -> Vec3 {
    let (yaw, pitch) = (f64::from(yaw).to_radians(), f64::from(pitch).to_radians());
    [-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()]
}

// --- the hooks ----------------------------------------------------------------------------------------------------

impl Hooks for Tracks {
    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        (event.block == TRACK).then(|| (TRACK.to_owned(), Compound::new()))
    }

    /// `TrackBlock.getStateForPlacement`: the shape from the player's look; the slope fix is `place_block`'s (a
    /// placement hook can't read neighbours).
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if context.item != TRACK {
            return None;
        }
        let shape = rules::shape_for_look(look_of(context.yaw, context.pitch), context.yaw);
        level.state_with_property(event.state, "shape", shape.serialized_name()).map(Some)
    }

    /// The rest of `getStateForPlacement` (a ledge beside it) and `onPlace`: the block joins the graph.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if &*event.stack.item != TRACK || host.block(event.pos) != TRACK {
            return;
        }
        let pos = event.pos;
        if let Some(shape) = shape_at(&*host, pos) {
            let fixed = rules::slope_fix(shape, pos, &|p| sturdy_top(&*host, p), &|p| sturdy_bottom(&*host, p));
            if fixed != shape && matches!(shape, TrackShape::Xo | TrackShape::Zo) {
                host.set_block_property(pos, "shape", fixed.serialized_name());
            }
        }
        queue_added(&host.dimension().to_owned(), pos);
    }

    /// `TrackBlock.playerWillDestroy` and `onRemove`: the curves are cut, and the block leaves the graph.
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != TRACK {
            return Flow::Continue;
        }
        let creative = host.acting_player().is_some_and(|p| p.game_mode == util::CREATIVE);
        let dimension = host.dimension().to_owned();
        // What the block gave the graph, read while it is still there.
        let id = dimension_id(&dimension);
        let ends = HostTracks { host: &*host, dimension: id }.connected(pos, false, None);
        {
            let mut states = lock(&STATE);
            states.entry(dimension.clone()).or_default().known.insert(pos, ends);
        }
        cut_curves(host, pos, !creative);
        queue_removed(&dimension, pos);
        Flow::Continue
    }

    fn level_tick_post(&self, host: &mut dyn Host, _: &mut LevelTickPostEvent<'_>) {
        if host.window() {
            return;
        }
        tick(host);
    }
}
