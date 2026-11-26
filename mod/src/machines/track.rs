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
//! - The tilt (`TrackBlockEntityTilt`) is kept and applied as Create does (the angle, each curve's `Smoothing`, the graph's
//!   y offsets), but the track's model is not tilted (a chunk model can't be), and no portal tracks, no fake track markers,
//!   no wrench pick-up of a curve's items (`onSneakWrenched`), no `getRequiredItems` for schematics.
//! - A curve's items drop for a survival player breaking an end, not for explosions.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::ops::Bound::{Excluded, Unbounded};
use std::sync::{LazyLock, Mutex};

use create_kinetics::rules::contraption::{State, replaceable as vanilla_replaceable};
use create_kinetics::rules::trains::bezier::BezierConnection;
use create_kinetics::rules::trains::discovery::{self, Registration, Sync};
use create_kinetics::rules::trains::graph::{DiscoveredLocation, TrackNodeLocation};
use create_kinetics::rules::trains::network::RailNetworks;
use create_kinetics::rules::trains::railways::Railways;
use create_kinetics::rules::trains::propagator::TrackWorld;
use create_kinetics::rules::trains::track_block::{self as rules, CONNECTIONS, MATERIAL, TRACK, TrackView};
use create_kinetics::rules::trains::track_shape::TrackShape;
use create_kinetics::rules::trains::track_tilt;
use create_kinetics::rules::trains::vec::{Vec3, block_containing};
use create_kinetics::{Dir, Pos};
use crate::compat::{
    AttachmentCodec, AttachmentType, BlockEntityEvent, BlockEntityTickEvent, BlockEntityTicksEvent, BlockStateId, Compound, DestroyBlockEvent, Flow, Holder, Hooks,
    Host, ItemStack, LevelHost, LevelTickPostEvent, PlaceBlockEvent, StateForPlacementEvent, Tag,
};

use crate::util;

struct Tracks;

/// The components an item's selection can leave in a block entity placed from it (`BlockItem.updateBlockEntityComponents` copies
/// the stack's whole patch): the track item's `TRACK_CONNECTING_FROM` and the targeting items' selection, and the glint that shows
/// either. Create's block entities hold none of them.
pub(crate) const LEAKED: [&str; 3] =
    [rules::COMPONENT, "create:track_targeting_item_selection", "minecraft:enchantment_glint_override"];

/// Whether a block entity's data holds any of [`LEAKED`] in its `components`.
pub(crate) fn leaked_components(nbt: &Compound) -> bool {
    matches!(util::get(nbt, "components"), Some(Tag::Compound(c)) if LEAKED.iter().any(|k| {
        c.contains_key(&(*k).into()) || c.contains_key(&format!("!{k}").as_str().into())
    }))
}

/// Removes [`LEAKED`] from a block entity's `components` (and the compound itself once it is empty).
pub(crate) fn strip_components(nbt: &mut Compound) {
    let empty = match nbt.get_mut(&"components".into()) {
        Some(Tag::Compound(c)) => {
            for key in LEAKED {
                c.remove(&key.into());
                c.remove(&format!("!{key}").as_str().into());
            }
            c.is_empty()
        }
        _ => false,
    };
    if empty {
        util::remove(nbt, "components");
    }
}

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
    let mut bc = BezierConnection::new(positions, starts, axes, normals, flag(tag, "Primary"), flag(tag, "Girder"), material);
    // `NBTHelper.intFromCompound`: `{V: int}`.
    bc.smoothing = pair(tag, "Smoothing", |t| match t.get("V") {
        Some(Tag::Int(v)) => Some(*v),
        _ => None,
    });
    Some(bc)
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
    if let Some(smoothing) = bc.smoothing {
        let ints = smoothing.map(|v| {
            let mut entry = Compound::new();
            entry.insert("V".into(), Tag::Int(v));
            Tag::Compound(entry)
        });
        c.insert("Smoothing".into(), Tag::List(ints.to_vec()));
    }
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
    if curves.iter().any(|c| c.key() == target) && is_tilted(&*host, pos) {
        // `captureSmoothingHandles`, and `tick`'s `undoSmoothing` a tick later.
        let dimension = host.dimension();
        lock(&PREVIOUS).insert((dimension, pos), track_tilt::capture_handles(&curves));
        lock(&UNDO).push((dimension, pos));
    }
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

// --- the tilt (`TrackBlockEntityTilt`) -----------------------------------------------------------------------------

/// `TrackBlockEntity.write`'s `Smoothing`: the angle of a tilted block, in degrees.
pub const SMOOTHING: &str = "Smoothing";

/// `TrackBlockEntityTilt.previousSmoothingHandles`, kept by level and position from `removeConnection` until `undoSmoothing`.
static PREVIOUS: LazyLock<Mutex<HashMap<(&'static str, Pos), Vec<(Vec3, i32)>>>> = LazyLock::new(Mutex::default);
/// Tilted blocks that lost a curve: `undoSmoothing` looks at them at the next tick (Create runs it every tick on every one).
static UNDO: Mutex<Vec<(&'static str, Pos)>> = Mutex::new(Vec::new());

/// `smoothingAngle` of a block entity's data.
pub fn tilt_angle(nbt: &Compound) -> Option<f64> {
    match util::get(nbt, SMOOTHING) {
        Some(Tag::Double(bits)) => Some(f64::from_bits(*bits)),
        _ => None,
    }
}

/// `TrackBlockEntity.isTilted`.
pub fn is_tilted(host: &dyn Host, pos: Pos) -> bool {
    host.mod_block_entity(pos).is_some_and(|e| tilt_angle(&e.nbt).is_some())
}

/// `TrackBlockEntityTilt.tryApplySmoothing` for the block at `pos`: a flat block between two curves, one falling away and one
/// climbing, is tilted (`track_tilt::try_apply_smoothing` says how); its curves are replaced by the two raised ones and the graph
/// is made again there.
pub fn apply_tilt(host: &mut dyn Host, pos: Pos) {
    if is_tilted(&*host, pos) {
        return;
    }
    let Some(shape) = shape_at(&*host, pos) else { return };
    let curves = connections_at(&*host, pos);
    let Some(smoothed) = track_tilt::try_apply_smoothing(&shape.axes(), false, &curves) else { return };
    // `removeInboundConnections(false)`, `connections.clear()`.
    cut_curves(host, pos, false);
    host.change_mod_block_entity(pos, &mut |e| {
        write_connections(&mut e.nbt, pos, &[]);
        e.nbt.insert(SMOOTHING.into(), Tag::Double(smoothed.angle.to_bits()));
    });
    let dimension = host.dimension();
    for bc in &smoothed.curves {
        let other = bc.key();
        if host.block(other) != TRACK {
            continue;
        }
        add_connection(host, pos, bc);
        add_connection(host, other, &bc.secondary());
        queue_retrack(dimension, other);
    }
    host.block_entity_sync(pos);
    queue_retrack(dimension, pos);
}

/// `TrackBlockEntityTilt.undoSmoothing`: a tilted block that lost a curve (and no longer has two) goes flat again, its curves
/// restored to the original ends.
fn undo_tilt(host: &mut dyn Host, pos: Pos) {
    let dimension = host.dimension();
    let curves = connections_at(&*host, pos);
    let has_previous = lock(&PREVIOUS).contains_key(&(dimension, pos));
    if host.block(pos) != TRACK || !track_tilt::should_undo(is_tilted(&*host, pos), has_previous, curves.len()) {
        return;
    }
    let valid: Vec<BezierConnection> = curves
        .into_iter()
        .filter(|bc| host.block(bc.key()) == TRACK && connections_at(&*host, bc.key()).iter().any(|c| c.key() == pos))
        .collect();
    cut_curves(host, pos, false);
    host.change_mod_block_entity(pos, &mut |e| {
        write_connections(&mut e.nbt, pos, &[]);
        util::remove(&mut e.nbt, SMOOTHING);
    });
    for bc in &valid {
        let restored = track_tilt::restore_to_original_curve(bc);
        add_connection(host, pos, &restored);
        add_connection(host, restored.key(), &restored.secondary());
        queue_retrack(dimension, restored.key());
    }
    host.block_entity_sync(pos);
    lock(&PREVIOUS).remove(&(dimension, pos));
    queue_retrack(dimension, pos);
}

/// The blocks that lost a curve while tilted, looked at once a tick.
fn run_undos(host: &mut dyn Host) {
    let dimension = host.dimension();
    let mine: Vec<Pos> = {
        let mut list = lock(&UNDO);
        let (mine, rest): (Vec<_>, Vec<_>) = list.drain(..).partition(|(d, _)| *d == dimension);
        *list = rest;
        mine.into_iter().map(|(_, pos)| pos).collect()
    };
    for pos in mine {
        undo_tilt(host, pos);
    }
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
        // `getYOffsetAt`: only a tilted block offsets its ends, by its curves' smoothing (or what it kept of a curve that left).
        let tilted = has_be && is_tilted(host, pos);
        let all = if tilted { connections_at(host, pos) } else { Vec::new() };
        let previous = if tilted { lock(&PREVIOUS).get(&(host.dimension(), pos)).cloned().unwrap_or_default() } else { Vec::new() };
        let y_offset = |end: Vec3| track_tilt::y_offset_for_axis_end(tilted, &all, &previous, end);
        let material = |v: Vec3, default: &str| {
            if host.block(block_containing(v)) == TRACK { MATERIAL.to_owned() } else { default.to_owned() }
        };
        let view = TrackView { pos, shape, has_be, dimension: self.dimension, curves: &curves, material: &material, y_offset: &y_offset };
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
const SWEEP: usize = 64;
/// Queued changes applied in one tick (a `/fill` of a long strip is in the graph within a few ticks).
const BUDGET: usize = 256;

impl DimState {
    fn push_added(&mut self, pos: Pos) {
        if self.queued.insert(pos) {
            self.added.push_back(pos);
        }
    }

    fn pop_added(&mut self) -> Option<Pos> {
        let pos = self.added.pop_front()?;
        self.queued.remove(&pos);
        Some(pos)
    }
}

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
    /// What `added` holds, so a block entity that reports its block every tick doesn't search the queue.
    queued: HashSet<Pos>,
    removed: VecDeque<Pos>,
    /// Blocks whose ends changed under them (a tilt): taken out of the graphs by what they had, then added again.
    retrack: VecDeque<Pos>,
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
    states.entry(dimension.to_owned()).or_default().push_added(pos);
}

/// A track block's block entity ticked: if the graph has never heard of the block (a `setblock`, a `/fill`, a structure, a piston,
/// a block put back where track was) it is queued as added; `TrackBlock.onPlace`'s scheduled tick and `onRailAdded`.
fn note_present(dimension: &str, pos: Pos) {
    let mut states = lock(&STATE);
    let st = states.entry(dimension.to_owned()).or_default();
    if !st.tracks.contains(&pos) {
        st.push_added(pos);
    }
}

/// Puts the block at `pos` in the graph now if it is not there yet, so the click that needs it (a Train Station's first click)
/// doesn't wait for the next tick. Only once the dimension's graphs exist (the first tick makes them).
pub fn register_now(host: &dyn Host, pos: Pos) {
    let dimension = host.dimension();
    let mut states = lock(&STATE);
    let Some(st) = states.get_mut(dimension) else { return };
    if st.token == 0 || st.tracks.contains(&pos) || host.block(pos) != TRACK {
        return;
    }
    let world = HostTracks { host, dimension: dimension_id(dimension) };
    add_now(&world, st, pos);
    let events = st.rails.nets.drain_events();
    st.rails.apply_rail_events(&events);
}

/// A track block's ends moved (a tilt was applied or let go): its graph nodes are taken out and made again at the next tick
/// (`onRailRemoved` then `onRailAdded`).
pub fn queue_retrack(dimension: &str, pos: Pos) {
    let mut states = lock(&STATE);
    let st = states.entry(dimension.to_owned()).or_default();
    if !st.retrack.contains(&pos) {
        st.retrack.push_back(pos);
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
    // The graphs in memory are dropped only for another world. An attachment that reads back empty used to reset them here,
    // every second, which wiped every node the glue had learned.
    match discovery::sync(stored_token, st.token, now < st.time) {
        Sync::Keep => {}
        Sync::Resave => st.dirty = true,
        Sync::Adopt(token) => {
            *st = DimState::default();
            st.token = token;
            if let Some(Tag::IntArray(v)) = stored.as_ref().and_then(|c| util::get(c, POSITIONS)) {
                st.rebuild = v.chunks_exact(3).map(|p| [p[0], p[1], p[2]]).collect();
            }
        }
        Sync::Fresh => {
            *st = DimState::default();
            st.token = new_token();
            st.dirty = true;
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
    let (added, removed, retrack) = (std::mem::take(&mut st.added), std::mem::take(&mut st.removed), std::mem::take(&mut st.retrack));
    st.queued.clear();
    if st.token == 0 || now < st.time || now % 20 == 0 {
        sync(host, &mut st, now);
    }
    for pos in added {
        st.push_added(pos);
    }
    st.removed.extend(removed);
    st.retrack.extend(retrack);
    run(&*host, &mut st, id, now);
    let mut states = lock(&STATE);
    // Anything queued meanwhile (by a hook the work called) joins what is there.
    if let Some(late) = states.remove(&dimension) {
        for pos in late.added {
            st.push_added(pos);
        }
        st.removed.extend(late.removed);
        st.retrack.extend(late.retrack);
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
        let Some(pos) = st.retrack.pop_front() else { break };
        if host.block(pos) != TRACK {
            continue;
        }
        let ends = st.known.remove(&pos).unwrap_or_default();
        st.rails.nets.on_rail_removed(&world, pos, &ends);
        add_now(&world, st, pos);
    }
    for _ in 0..BUDGET {
        let Some(pos) = st.pop_added() else { break };
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
    sweep(host, st, level, &world);
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
fn sweep(host: &dyn Host, st: &mut DimState, level: Option<crate::compat::LevelRef>, world: &HostTracks<'_>) {
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
        if !host.level_is_loaded(level, pos) {
            continue;
        }
        if host.block(pos) != TRACK {
            if !st.removed.contains(&pos) {
                st.removed.push_back(pos);
            }
            continue;
        }
        // A block replaced in place (`setblock` of another shape over it) keeps its place in the set but not its ends:
        // `onRailRemoved` with what it had, then `onRailAdded` (`discovery::registration`).
        let current = world.connected(pos, false, None);
        if discovery::registration(true, st.known.get(&pos).map(Vec::as_slice), &current) != Registration::Keep && !st.retrack.contains(&pos) {
            st.retrack.push_back(pos);
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

    /// `TrackBlock.onPlace` schedules a tick and `tick` runs `onRailAdded`: here the block entity's own tick reports a block the
    /// graph doesn't have (a `setblock`, `fill`, structure or piston never reaches `place_block`). It also drops what the placing
    /// item leaked into the entity.
    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == TRACK {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != TRACK {
            return;
        }
        if leaked_components(&event.entity.nbt) {
            strip_components(&mut event.entity.nbt);
            host.block_entity_sync(event.pos);
        }
        note_present(host.dimension(), event.pos);
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
        // A selection still on the stack is spent by a placement (the next inventory tick takes it off), and the new block
        // entity must not keep it among its components.
        if event.stack.data().is_some_and(|d| d.other(rules::COMPONENT).is_some())
            && let Some(player) = host.acting_player()
        {
            super::track_item::forget(player.uuid);
        }
        if host.mod_block_entity(pos).is_some_and(|e| leaked_components(&e.nbt)) {
            host.change_mod_block_entity(pos, &mut |e| strip_components(&mut e.nbt));
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
        run_undos(host);
    }
}
