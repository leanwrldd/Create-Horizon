//! Trains in the world: the railways of each dimension ticked every level tick, the carriages' structures and their
//! anchors, the trains saved with the world, and `/train`. The decisions are
//! `create_kinetics::rules::trains::{railways, train, carriage_pose, carriage_contraption, train_nbt, train_controls}`;
//! this reads the world, calls them and applies what they say. The track and its graphs are `track.rs`
//! (`Railways` lives beside them, per dimension), assembling and disassembling at a station is `train_assemble.rs`, the
//! drawing is `train_draw.rs` and driving by hand `train_controls.rs`.
//!
//! # The tick (`GlobalRailwayManager.tick`, `CarriageContraptionEntity.tickContraption`)
//!
//! [`Railways::tick`] runs the signal groups and the points of every graph, then the trains. What the trains did that
//! the world must be told (`TrainEvent`) is applied here: a crash's explosion (a flash and a sound: Horizon has no
//! `explode`), a train that left the manager (its carriages' anchors are removed). Then each carriage is put where its
//! wheels say (`updateContraptionAnchors`, `alignEntity`): the pose goes into the carriage's anchor block entity.
//!
//! # Saving
//!
//! Create saves the trains with the graphs (`RailwaySavedData`). The graphs aren't saved here (`track.rs` rebuilds them
//! from the track blocks), so the trains are kept in a level attachment (`create:trains`) with the carriages'
//! structures: each train's NBT (`train_nbt`) and the structure of each carriage. After a load a train waits (`Pending`)
//! until the graph its wheels name is back, every wheel on its edge and the world has had a while for the stations and
//! signals to put their points on the graph; then it joins the railways again. Its anchors stay where the chunks saved
//! them, so a train that can't be restored is still seen where it stood.
//!
//! Differences from Create, for the README:
//! - No carriage entities: the seats' riders are what `seat.rs` keeps (a marker per seat, carried with the carriage by
//!   `carry_riders`), no sounds, particles, honking or couplings.
//! - Cargo (`TrainCargoManager`, `train_cargo.rs`): a carriage's storage is read from its structure's blocks every 20 ticks
//!   (and after a burn): fuel is burnt from its chests, barrels and shulker boxes, and cargo conditions read what they hold.
//! - Conductors (`train_riders.rs`): a mob that touches a seat of a carriage sits on it (`SeatInteractionBehaviour`), a mob on a
//!   seat in front of Train Controls conducts (`checkConductors`), and so does a Blaze Burner in front of them. The Schedule is
//!   handed to a seated mob or clicked into the burner; `/train conductor` still says a driver stands in for the train.
//! - Doors (`train_doors.rs`): the carriage's Train Doors open at a station by its door control (`/station doors`).
//! - A carriage in a chunk nobody loads still runs (Create stalls a train whose player rides it into unloaded chunks).
//! - Trains are not kept moving across dimensions (portals); the nether is never ticked with the overworld's trains.

use std::collections::{BTreeMap, HashMap};
use std::sync::{LazyLock, Mutex};

use create_kinetics::rules::contraption::State;
use create_kinetics::rules::snbt::Value;
use create_kinetics::rules::trains::carriage::{Carriage, CarriageBogey};
use create_kinetics::rules::trains::carriage_contraption as cc;
use create_kinetics::rules::collider::{Rotation, StructurePose};
use create_kinetics::rules::trains::carriage_pose::{self as pose, CarriagePose, Motion};
use create_kinetics::rules::trains::assembly::StationTrainState;
use create_kinetics::rules::trains::graph::{EdgePointType, Uuid};
use create_kinetics::rules::trains::station_screen::{self as screens, TrainEdit, TrainInfo};
use create_kinetics::rules::trains::network::TrackGraph;
use create_kinetics::rules::trains::railways::Railways;
use create_kinetics::rules::trains::schedule::Schedule;
use create_kinetics::rules::trains::train::{StatusMessage, TrainEvent, TrainWorld};
use create_kinetics::rules::trains::schedule_hand::RuntimeView;
use create_kinetics::rules::trains::train_controls as drive;
use create_kinetics::rules::trains::train_doors::{DoorControl, DoorData};
use create_kinetics::rules::trains::train_passengers::{self as passengers, Passenger};
use create_kinetics::rules::trains::train_nbt;
use create_kinetics::{Dir, Pos};
use crate::compat::{
    AttachmentCodec, AttachmentType, BlockEntityTickEvent, Compound, Flow, Holder, Hooks, Host, LevelTickPostEvent,
    NativeCommand, PerformCommandEvent, RegisterCommandsEvent, Tag, Text,
};

use super::contraption::{MovedBlock, Moving};
use super::track as world;
use super::train_draw::{self as draw, ANCHOR, ANCHOR_RADIUS, BogeyData};
use crate::util;

struct Trains;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Trains)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// `Block.UPDATE_CLIENTS | UPDATE_KNOWN_SHAPE | UPDATE_SUPPRESS_DROPS`: moving an anchor sets no neighbour off.
const ANCHOR_FLAGS: i32 = 2 | 16 | 32;
/// Ticks a pending train waits before its first try, for the points of the stations and signals to be back.
const SETTLE: i64 = 60;
/// Ticks between two tries.
const RETRY: i64 = 20;
/// Ticks between two saves of the trains.
const SAVE_EVERY: i64 = 100;
/// Ticks between pose news of a moving carriage.
const NEWS_EVERY: i64 = 2;
/// Ticks between rewrites of a moving carriage's anchor data (structure and pose, sent whole to clients): the pose news
/// in between goes as a message to the players near it (`train_draw::publish`).
const DATA_EVERY: i64 = 40;
/// Ticks an anchor with no train is left before it is removed.
const ORPHAN_TICKS: i64 = 400;

// --- a carriage's runtime ---------------------------------------------------------------------------------------------

/// What the glue keeps of a carriage besides what `Carriage` holds.
#[derive(Clone, Debug)]
pub struct CarriageRt {
    /// The carriage's blocks, local to its first bogey's block (`Contraption.blocks`, `anchor`).
    pub structure: Moving,
    /// The station's assembly direction when it was assembled (`CarriageContraption.assemblyDirection`).
    pub direction: Dir,
    pub forward_controls: bool,
    pub backward_controls: bool,
    /// Each Train Controls block: local position and facing.
    pub controls: Vec<(Pos, Dir)>,
    /// The block the anchor is in now.
    pub anchor: Option<Pos>,
    pub last_pose: Option<CarriagePose>,
    /// The last tick's pose, for the velocity.
    pub prev_pose: Option<CarriagePose>,
    /// Each bogey's wheel angle.
    pub wheel: [f64; 2],
    /// Game time of the last news and whether it said the carriage moved.
    pub news_at: i64,
    pub news_moving: bool,
    /// Game time the anchor's data was last rewritten (and so sent whole).
    pub data_at: i64,
    /// The bogeys' blocks changed or the structure is new: send everything.
    pub fresh: bool,
    /// `conductorSeats`: each seat (local position) and the `(forward, backward)` sides it conducts for.
    pub conductor_seats: BTreeMap<Pos, [bool; 2]>,
    /// `blockConductors`: a Blaze Burner in front of the controls, `(forward, backward)`.
    pub block_conductors: [bool; 2],
    /// What the doors' actors remember (`context.data.Open`, `firstMovement`), by local position. Not saved.
    pub doors: BTreeMap<Pos, DoorData>,
    /// The cargo is to be read again (the structure is new or changed).
    pub cargo_dirty: bool,
    /// Game time of the last look at the cargo, and whether the change since is counted already (`burnFuel` counts its own).
    pub cargo_at: i64,
    pub cargo_silent: bool,
}

impl CarriageRt {
    pub fn new(structure: Moving, direction: Dir, summary: &cc::Summary) -> CarriageRt {
        let controls = structure
            .blocks
            .iter()
            .filter(|b| b.state.is(cc::CONTROLS))
            .filter_map(|b| b.state.dir("facing").map(|d| (b.local, d)))
            .collect();
        // The conductors come from the blocks themselves (the positions are local, as the structure's are).
        let local_blocks: Vec<(Pos, State)> = structure.blocks.iter().map(|b| (b.local, b.state.clone())).collect();
        let found = cc::summarize(&local_blocks, direction);
        CarriageRt {
            structure,
            direction,
            forward_controls: summary.forward_controls,
            backward_controls: summary.backward_controls,
            controls,
            anchor: None,
            last_pose: None,
            prev_pose: None,
            wheel: [0.0; 2],
            news_at: i64::MIN,
            news_moving: false,
            data_at: i64::MIN,
            fresh: true,
            conductor_seats: found.conductor_seats,
            block_conductors: found.block_conductors,
            doors: BTreeMap::new(),
            cargo_dirty: true,
            cargo_at: i64::MIN / 2,
            cargo_silent: false,
        }
    }

    /// The `(forward, backward)` conductor sides a stand-in driver gives (`checkConductors`).
    fn conductor_sides(&self) -> [bool; 2] {
        [self.forward_controls, self.backward_controls]
    }
}

type Key = (Uuid, i32);

/// A saved train waiting for its track.
struct Pending {
    train: Vec<(String, Value)>,
    carriages: BTreeMap<i32, CarriageRt>,
    next_try: i64,
}

/// A player driving a carriage.
struct Drive {
    train: Uuid,
    carriage: i32,
    controls: Pos,
    keys: drive::HeldKeys,
    driver: drive::Driver,
}

#[derive(Default)]
struct Dim {
    token: i64,
    carriages: BTreeMap<Key, CarriageRt>,
    pending: Vec<Pending>,
    /// Trains a stand-in driver is aboard (`/train conductor`).
    conductors: std::collections::BTreeSet<Uuid>,
    drives: HashMap<u128, Drive>,
    saved_at: i64,
    dirty: bool,
    /// Stations whose block is powered / in an unloaded chunk, read every few ticks.
    powered: std::collections::BTreeSet<Uuid>,
    unloaded: std::collections::BTreeSet<Uuid>,
    /// Stations that had a train present at the last tick (`trainPresent`).
    present: BTreeMap<Uuid, Uuid>,
    /// Each station's door control (`doorControls.mode`), read every few ticks.
    doors: BTreeMap<Uuid, DoorControl>,
}

static DIMS: LazyLock<Mutex<HashMap<String, Dim>>> = LazyLock::new(Mutex::default);
/// Anchors whose carriage nobody knows, since when: `(dimension, pos)` to game time.
static ORPHANS: LazyLock<Mutex<HashMap<(String, Pos), i64>>> = LazyLock::new(Mutex::default);
/// Anchors to remove at the next level tick.
static REMOVE: LazyLock<Mutex<Vec<(String, Pos)>>> = LazyLock::new(Mutex::default);

/// Runs `f` on a dimension's trains data (the dimension's state is out of the map during a tick, so this is `None`
/// then: callers on the server thread outside the tick are the ones that use it).
pub fn with_dim<R>(dimension: &str, f: impl FnOnce(&mut DimHandle<'_>) -> R) -> Option<R> {
    let mut map = lock(&DIMS);
    let dim = map.get_mut(dimension)?;
    Some(f(&mut DimHandle { dim }))
}

/// The part of the dimension's trains data other modules may use.
pub struct DimHandle<'a> {
    dim: &'a mut Dim,
}

impl DimHandle<'_> {
    /// A carriage made by an assembly joins (`Carriage.setContraption`).
    pub fn add_carriage(&mut self, train: Uuid, carriage: i32, rt: CarriageRt) {
        self.dim.carriages.insert((train, carriage), rt);
        self.dim.dirty = true;
    }

    /// The carriage's runtime, taken out (a disassembly).
    pub fn take_carriage(&mut self, train: Uuid, carriage: i32) -> Option<CarriageRt> {
        self.dim.dirty = true;
        self.dim.carriages.remove(&(train, carriage))
    }

    pub fn dirty(&mut self) {
        self.dim.dirty = true;
    }
}

/// Whether the server knows a carriage (in the railways or waiting to be restored).
fn knows(dimension: &str, train: Uuid, carriage: i32) -> bool {
    let known = lock(&DIMS).get(dimension).is_some_and(|d| {
        d.carriages.contains_key(&(train, carriage))
            || d.pending.iter().any(|p| uuid_of(&p.train) == Some(train))
    });
    // The state is out of the map while it ticks: then it's the railways that answer.
    known || (!lock(&DIMS).contains_key(dimension) && world::with_railways(dimension, |r| r.trains.contains_key(&train)).unwrap_or(false))
}

fn uuid_of(tag: &[(String, Value)]) -> Option<Uuid> {
    match tag.iter().find(|(k, _)| k == "Id") {
        Some((_, Value::IntArray(v))) if v.len() == 4 => Some(v.iter().fold(0u128, |acc, part| (acc << 32) | u128::from(*part as u32))),
        _ => None,
    }
}

fn uuid_tag(id: Uuid) -> Tag {
    Tag::IntArray(vec![(id >> 96) as i32, (id >> 64) as i32, (id >> 32) as i32, id as i32])
}

fn tag_uuid(tag: &Tag) -> Option<Uuid> {
    match tag {
        Tag::IntArray(v) if v.len() == 4 => Some(v.iter().fold(0u128, |acc, part| (acc << 32) | u128::from(*part as u32))),
        _ => None,
    }
}

// --- the tree and the tag -----------------------------------------------------------------------------------------------

pub fn to_tag(value: &Value) -> Tag {
    match value {
        Value::Byte(v) => Tag::Byte(*v),
        Value::Short(v) => Tag::Short(*v),
        Value::Int(v) => Tag::Int(*v),
        Value::Long(v) => Tag::Long(*v),
        Value::Float(v) => Tag::Float(v.to_bits()),
        Value::Double(v) => Tag::Double(v.to_bits()),
        Value::Str(s) => Tag::String(s.as_str().into()),
        Value::ByteArray(v) => Tag::ByteArray(v.iter().map(|b| *b as u8).collect()),
        Value::IntArray(v) => Tag::IntArray(v.clone()),
        Value::LongArray(v) => Tag::LongArray(v.clone()),
        Value::List(items) => Tag::List(items.iter().map(to_tag).collect()),
        Value::Compound(fields) => {
            let mut out = Compound::new();
            for (key, value) in fields {
                out.insert(key.as_str().into(), to_tag(value));
            }
            Tag::Compound(out)
        }
    }
}

pub fn from_tag(tag: &Tag) -> Option<Value> {
    Some(match tag {
        Tag::Byte(v) => Value::Byte(*v),
        Tag::Short(v) => Value::Short(*v),
        Tag::Int(v) => Value::Int(*v),
        Tag::Long(v) => Value::Long(*v),
        Tag::Float(v) => Value::Float(f32::from_bits(*v)),
        Tag::Double(v) => Value::Double(f64::from_bits(*v)),
        Tag::String(s) => Value::Str(s.to_string_lossy()),
        Tag::ByteArray(v) => Value::ByteArray(v.iter().map(|b| *b as i8).collect()),
        Tag::IntArray(v) => Value::IntArray(v.clone()),
        Tag::LongArray(v) => Value::LongArray(v.clone()),
        Tag::List(items) => Value::List(items.iter().filter_map(from_tag).collect()),
        Tag::Compound(c) => Value::Compound(c.iter().filter_map(|(k, v)| Some((k.to_string_lossy(), from_tag(v)?))).collect()),
        Tag::End => return None,
    })
}

fn dim_name(id: u32) -> String {
    world::dimension_name(id)
}

fn dim_id(name: &str) -> u32 {
    world::dimension_id(name)
}

// --- the saved form of a carriage ---------------------------------------------------------------------------------------

fn dir_name(dir: Dir) -> &'static str {
    dir.name()
}

fn write_rt(id: i32, rt: &CarriageRt) -> Tag {
    let mut c = Compound::new();
    c.insert("Id".into(), Tag::Int(id));
    c.insert("Structure".into(), Tag::Compound(rt.structure.to_nbt(false)));
    c.insert("Direction".into(), Tag::String(dir_name(rt.direction).into()));
    c.insert("FrontControls".into(), Tag::Byte(i8::from(rt.forward_controls)));
    c.insert("BackControls".into(), Tag::Byte(i8::from(rt.backward_controls)));
    if let Some(a) = rt.anchor {
        c.insert("Anchor".into(), Tag::IntArray(a.to_vec()));
    }
    c.insert("Wheel".into(), Tag::List(rt.wheel.iter().map(|w| Tag::Double(w.to_bits())).collect()));
    Tag::Compound(c)
}

fn read_rt(tag: &Tag) -> Option<(i32, CarriageRt)> {
    let Tag::Compound(c) = tag else { return None };
    let Some(Tag::Compound(structure)) = util::get(c, "Structure") else { return None };
    let moving = Moving::from_nbt(structure)?;
    let direction = match util::get(c, "Direction") {
        Some(Tag::String(s)) => Dir::from_name(&s.to_string_lossy())?,
        _ => return None,
    };
    let summary = cc::Summary {
        forward_controls: util::boolean(c, "FrontControls"),
        backward_controls: util::boolean(c, "BackControls"),
        ..Default::default()
    };
    let mut rt = CarriageRt::new(moving, direction, &summary);
    if let Some(Tag::IntArray(a)) = util::get(c, "Anchor") {
        rt.anchor = <Pos>::try_from(a.as_slice()).ok();
    }
    if let Some(Tag::List(w)) = util::get(c, "Wheel") {
        for (i, t) in w.iter().take(2).enumerate() {
            if let Tag::Double(bits) = t {
                rt.wheel[i] = f64::from_bits(*bits);
            }
        }
    }
    Some((util::int(c, "Id"), rt))
}

// --- the level attachment ---------------------------------------------------------------------------------------------------

static ATTACHMENT: AttachmentType<Compound> = AttachmentType {
    namespace: "create",
    name: "trains",
    default: Compound::new,
    codec: Some(AttachmentCodec { encode: |data| Ok(data.clone()), decode: |data| Ok(data.clone()) }),
    copy_on_death: false,
    sync: false,
};

fn save(host: &mut dyn Host, dim: &Dim, rails: &Railways) {
    let mut trains = Vec::new();
    for (id, train) in &rails.trains {
        let graph = train.graph.or(train.point_graph).and_then(|g| rails.nets.graphs.get(&g));
        let mut entry = Compound::new();
        entry.insert("Train".into(), to_tag(&train_nbt::write_train(train, graph, &dim_name)));
        let carriages = train.carriages.iter().filter_map(|c| dim.carriages.get(&(*id, c.id)).map(|rt| write_rt(c.id, rt))).collect();
        entry.insert("Carriages".into(), Tag::List(carriages));
        trains.push(Tag::Compound(entry));
    }
    // Trains still waiting for their track are kept as they were read.
    for p in &dim.pending {
        let mut entry = Compound::new();
        entry.insert("Train".into(), to_tag(&Value::Compound(p.train.clone())));
        entry.insert("Carriages".into(), Tag::List(p.carriages.iter().map(|(id, rt)| write_rt(*id, rt)).collect()));
        trains.push(Tag::Compound(entry));
    }
    let mut out = Compound::new();
    out.insert("Token".into(), Tag::Long(dim.token));
    out.insert("Trains".into(), Tag::List(trains));
    let _ = ATTACHMENT.set(host.new_server(), Holder::Level, out);
}

/// Reads the level's attachment into the dimension's pending trains, or starts a new world (another token).
fn sync(host: &mut dyn Host, dim: &mut Dim, now: i64) {
    let stored = ATTACHMENT.get_existing(host.new_server(), Holder::Level).ok().flatten();
    let token = stored.as_ref().map(|c| util::long(c, "Token")).unwrap_or(0);
    if token != 0 && token == dim.token {
        return;
    }
    *dim = Dim::default();
    dim.saved_at = now;
    let Some(stored) = stored.filter(|_| token != 0) else {
        dim.token = (util::random() as i64) | 1;
        dim.dirty = true;
        return;
    };
    dim.token = token;
    let Some(Tag::List(trains)) = util::get(&stored, "Trains") else { return };
    for entry in trains {
        let Tag::Compound(entry) = entry else { continue };
        let Some(Tag::Compound(train)) = util::get(entry, "Train") else { continue };
        let Some(Value::Compound(train)) = from_tag(&Tag::Compound(train.clone())) else { continue };
        let mut carriages = BTreeMap::new();
        if let Some(Tag::List(list)) = util::get(entry, "Carriages") {
            for c in list {
                if let Some((id, rt)) = read_rt(c) {
                    carriages.insert(id, rt);
                }
            }
        }
        dim.pending.push(Pending { train, carriages, next_try: now + SETTLE });
    }
}

// --- what the trains ask of the world ---------------------------------------------------------------------------------------

struct World {
    day_time: i64,
    dimension: u32,
    conductors: BTreeMap<Key, [bool; 2]>,
    /// The players sitting on each train's seats (`countPlayerPassengers`).
    players: BTreeMap<Uuid, i32>,
    powered: std::collections::BTreeSet<Uuid>,
    unloaded: std::collections::BTreeSet<Uuid>,
}

impl TrainWorld for World {
    fn day_time(&self) -> i64 {
        self.day_time
    }
    fn redstone_power_version(&self) -> i32 {
        0
    }
    fn has_any_loaded_power(&self, _: &[String; 2]) -> bool {
        false
    }
    fn station_powered(&self, station: Uuid) -> bool {
        self.powered.contains(&station)
    }
    fn station_unloaded(&self, station: Uuid) -> bool {
        self.unloaded.contains(&station)
    }
    fn dimension_loaded(&self, dimension: u32) -> bool {
        dimension == self.dimension
    }
    fn port_packages(&self, port: Pos, dimension: u32) -> Vec<String> {
        super::train_mail::waiting(port, dimension)
    }
    fn count_player_passengers(&self, train: Uuid) -> i32 {
        self.players.get(&train).copied().unwrap_or(0)
    }
    fn check_conductors(&self, train: Uuid, carriage: i32) -> Option<[bool; 2]> {
        Some(self.conductors.get(&(train, carriage)).copied().unwrap_or([false, false]))
    }
}

/// Who sits on a carriage's seats (`seat.rs` keeps the markers and their riders).
pub fn riders_of(host: &dyn Host, dimension: &str, train: Uuid, carriage: i32) -> Vec<Passenger> {
    super::seat::passengers_of(host, dimension, seat_key(train, carriage))
        .into_iter()
        .map(|(seat, _, player)| Passenger { seat, player })
        .collect()
}

fn build_world(dim: &Dim, rails: &Railways, host: &dyn Host) -> World {
    let mut conductors = BTreeMap::new();
    let mut players: BTreeMap<Uuid, i32> = BTreeMap::new();
    let dimension = host.dimension();
    for ((train, id), rt) in &dim.carriages {
        // `checkConductors`: the burners in front of the controls and the mobs on the seats in front of them.
        let riders = riders_of(host, dimension, *train, *id);
        let mut sides = passengers::check_conductors(rt.block_conductors, &rt.conductor_seats, &riders);
        if dim.conductors.contains(train) {
            // `/train conductor`: a driver stands in for the train.
            let stand_in = rt.conductor_sides();
            sides = [sides[0] || stand_in[0], sides[1] || stand_in[1]];
        }
        conductors.insert((*train, *id), sides);
        *players.entry(*train).or_default() += passengers::count_player_passengers(&riders);
    }
    let _ = rails;
    World {
        day_time: host.day_time(),
        dimension: dim_id(host.dimension()),
        players,
        conductors,
        powered: dim.powered.clone(),
        unloaded: dim.unloaded.clone(),
    }
}

/// The stations' block states, read every few ticks: whether the block has a signal and whether its chunk is loaded.
fn read_stations(host: &mut dyn Host, dim: &mut Dim, rails: &Railways) {
    let dimension = host.dimension();
    let level = host.level(dimension);
    dim.powered.clear();
    dim.unloaded.clear();
    dim.doors.clear();
    for graph in rails.nets.graphs.values() {
        for point in graph.edge_points.values(EdgePointType::Station) {
            let Some(station) = point.station() else { continue };
            let Some(pos) = station.block_entity_pos else { continue };
            if !level.is_some_and(|l| host.level_is_loaded(l, pos)) {
                dim.unloaded.insert(point.id);
                continue;
            }
            if host.neighbor_signal(dimension, pos) > 0 {
                dim.powered.insert(point.id);
            }
            // `DoorControlBehaviour`: the station block entity's `DoorControl` (all when it has none saved).
            let saved = host.mod_block_entity(pos).and_then(|e| match util::get(&e.nbt, "DoorControl") {
                Some(Tag::String(s)) => Some(s.to_string_lossy()),
                _ => None,
            });
            dim.doors.insert(point.id, DoorControl::read(saved.as_deref()));
        }
    }
}

// --- the anchors ----------------------------------------------------------------------------------------------------------------

fn is_air(name: &str) -> bool {
    matches!(name, "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air")
}

/// The anchor block's cell for a carriage at `position`: the first air cell at or above the one it is in.
fn choose_cell(host: &dyn Host, position: [f64; 3]) -> Option<Pos> {
    let base = draw::cell(position);
    [1, 2, 3, 0].into_iter().map(|up| [base[0], base[1] + up, base[2]]).find(|c| is_air(&host.block(*c)))
}

fn anchor_is(host: &dyn Host, pos: Pos, train: Uuid, carriage: i32) -> bool {
    host.block(pos) == ANCHOR
        && host.mod_block_entity(pos).is_some_and(|e| {
            util::get(&e.nbt, draw::TRAIN).and_then(tag_uuid) == Some(train) && util::int(&e.nbt, draw::CARRIAGE) == carriage
        })
}

fn far_from(anchor: Pos, position: [f64; 3]) -> bool {
    let dx = f64::from(anchor[0]) + 0.5 - position[0];
    let dz = f64::from(anchor[2]) + 0.5 - position[2];
    let dy = f64::from(anchor[1]) + 0.5 - position[1];
    dx * dx + dz * dz > ANCHOR_RADIUS * ANCHOR_RADIUS || dy.abs() > 4.0
}

fn remove_anchor(host: &mut dyn Host, pos: Pos) {
    if host.block(pos) != ANCHOR {
        return;
    }
    // The structure kept for its collision goes with it, and its pose news.
    super::contraption::forget_live(host.dimension(), pos);
    draw::forget(host.dimension(), pos);
    if let Some(air) = host.default_state("minecraft:air") {
        let dimension = host.dimension();
        host.set_block(dimension, pos, air, ANCHOR_FLAGS);
    }
}

fn bogey_data(carriage: &Carriage, graph: &TrackGraph, pose: &CarriagePose, derailed: bool, rt: &CarriageRt, diff: [f64; 3]) -> Vec<BogeyData> {
    let mut out = Vec::new();
    let leading_ud = carriage.leading_bogey().is_upside_down();
    let _ = leading_ud;
    let list: Vec<(usize, &CarriageBogey)> = [Some(&carriage.bogeys.0), carriage.bogeys.1.as_ref()].into_iter().flatten().enumerate().collect();
    for (i, bogey) in list {
        let positions = [bogey.points[0].get_position(Some(graph), false), bogey.points[1].get_position(Some(graph), false)];
        let (yaw, pitch) = match (positions, derailed) {
            ([Some(a), Some(b)], false) if bogey.points[0].edge(graph).is_some() => pose::bogey_angles(a, b),
            _ => pose::bogey_angles_derailed(pose.yaw, Some(bogey.derail_angle)),
        };
        out.push(BogeyData {
            large: bogey.ty.id == "create:large_bogey",
            upside_down: bogey.is_upside_down(),
            leading: i == 0,
            yaw,
            pitch,
            wheel: rt.wheel[i.min(1)],
            wheel_rate: pose::wheel_rate(diff, pose.yaw, bogey.ty.wheel_radius),
        });
    }
    out
}

/// The whole data of an anchor: the structure for the client, whose train and carriage, and the pose.
fn anchor_data(train: Uuid, carriage: i32, rt: &CarriageRt, motion: &Motion, initial_yaw: f32, spacing: i32, bogeys: &[BogeyData], derailed: bool) -> Compound {
    let mut c = Compound::new();
    c.insert("Running".into(), Tag::Byte(1));
    c.insert("Contraption".into(), Tag::Compound(rt.structure.to_nbt(false)));
    c.insert(draw::TRAIN.into(), uuid_tag(train));
    c.insert(draw::CARRIAGE.into(), Tag::Int(carriage));
    draw::write_pose(&mut c, motion, initial_yaw, spacing, bogeys, derailed);
    c
}

/// A carriage's anchor follows it: placed where there is none, moved when the carriage has got far from it, and
/// told the pose.
#[allow(clippy::too_many_arguments)]
fn update_anchor(
    host: &mut dyn Host,
    dimension: &'static str,
    train_id: Uuid,
    carriage: &Carriage,
    graph: &TrackGraph,
    derailed: bool,
    spacing: i32,
    rt: &mut CarriageRt,
    pose_now: CarriagePose,
    now: i64,
) {
    let before = rt.last_pose;
    rt.prev_pose = before;
    rt.last_pose = Some(pose_now);
    let moved = before.is_none_or(|b| b != pose_now);
    let initial_yaw = pose::initial_yaw(horizontal_step(rt.direction));
    carry_riders(host, train_id, carriage.id, rt, &pose_now, initial_yaw);
    // The wheels turn with the distance moved this tick.
    let diff = before.map_or([0.0; 3], |b| [0, 1, 2].map(|i| pose_now.position[i] - b.position[i]));
    if let Some(b) = before {
        let (moved_by, _) = pose::distance_moved(diff, pose_now.yaw);
        let _ = b;
        let leading_radius = carriage.bogeys.0.ty.wheel_radius;
        rt.wheel[0] = pose::next_wheel_angle(rt.wheel[0], moved_by, leading_radius);
        if let Some(second) = &carriage.bogeys.1 {
            rt.wheel[1] = pose::next_wheel_angle(rt.wheel[1], moved_by, second.ty.wheel_radius);
        }
    }

    // Where the anchor should be.
    // The block is looked at every tick, whose block entity it holds (a copy of the structure) every two seconds.
    let valid = rt.anchor.is_some_and(|a| {
        host.block(a) == ANCHOR && !far_from(a, pose_now.position) && (now % 40 != 0 || anchor_is(&*host, a, train_id, carriage.id))
    });
    // What clicks changed in the structure (a door, a lever: `structure_click.rs` writes the anchor's copy) is taken back.
    if valid && now % 40 == 0 {
        if let Some(a) = rt.anchor {
            adopt_clicked_structure(&*host, a, rt);
        }
    }
    if !valid {
        let Some(cell) = choose_cell(&*host, pose_now.position) else { return };
        let Some(state) = host.default_state(ANCHOR) else { return };
        // The chunk must be there.
        let loaded = host.level(dimension).is_some_and(|l| host.level_is_loaded(l, cell));
        if !loaded {
            return;
        }
        super::contraption::forget_live(dimension, cell);
        host.set_block(dimension, cell, state, ANCHOR_FLAGS);
        let motion = Motion::new(&pose_now, &before.unwrap_or(pose_now), now);
        let bogeys = bogey_data(carriage, graph, &pose_now, derailed, rt, diff);
        let data = anchor_data(train_id, carriage.id, rt, &motion, initial_yaw, spacing, &bogeys, derailed);
        host.set_block_tag(dimension, cell, Tag::Compound(data));
        host.block_entity_sync(cell);
        if let Some(old) = rt.anchor.replace(cell) {
            if old != cell {
                remove_anchor(host, old);
            }
        }
        rt.news_at = now;
        rt.news_moving = moved;
        rt.data_at = now;
        rt.fresh = false;
        return;
    }

    // News of the pose: every other tick while it moves, and once more when it has stopped.
    let due = if moved { now - rt.news_at >= NEWS_EVERY } else { rt.news_moving || rt.fresh };
    if !due {
        return;
    }
    let Some(anchor) = rt.anchor else { return };
    let motion = if moved { Motion::new(&pose_now, &before.unwrap_or(pose_now), now) } else { Motion::new(&pose_now, &pose_now, now) };
    let bogeys = bogey_data(carriage, graph, &pose_now, derailed, rt, if moved { diff } else { [0.0; 3] });
    // The pose alone to the players near it; the whole data (the structure travels with it) when the carriage stops,
    // when something changed, and every `DATA_EVERY` ticks for players who come into view and for saving.
    draw::publish(dimension, anchor, &motion, &bogeys, derailed);
    if !moved || rt.fresh || now - rt.data_at >= DATA_EVERY {
        let wrote = host.change_mod_block_entity(anchor, &mut |e| {
            draw::write_pose(&mut e.nbt, &motion, initial_yaw, spacing, &bogeys, derailed);
        });
        if wrote {
            host.block_entity_sync(anchor);
        }
        rt.data_at = now;
    }
    rt.news_at = now;
    rt.news_moving = moved;
    rt.fresh = false;
}

/// What `seat.rs` knows a carriage's seats by, as a structure's are known by its controller block (a carriage's anchor
/// moves as it runs): a position no block can have.
pub fn seat_key(train: Uuid, carriage: i32) -> Pos {
    let folded = (train as u32) ^ ((train >> 32) as u32) ^ ((train >> 64) as u32) ^ ((train >> 96) as u32);
    [i32::MIN, folded as i32, carriage]
}

/// The seat key of the carriage an anchor block draws, if `anchor` is one.
pub fn seat_key_of_anchor(host: &dyn Host, anchor: Pos) -> Option<Pos> {
    if host.block(anchor) != ANCHOR {
        return None;
    }
    let entity = host.mod_block_entity(anchor)?;
    let train = util::get(&entity.nbt, draw::TRAIN).and_then(tag_uuid)?;
    Some(seat_key(train, util::int(&entity.nbt, draw::CARRIAGE)))
}

/// `AbstractContraptionEntity.positionRider` for a carriage: the markers of its seats' riders go where the seat
/// blocks are now, turned with the carriage as the solid module turns it (`OrientedContraptionEntity`).
fn carry_riders(host: &mut dyn Host, train: Uuid, carriage: i32, rt: &CarriageRt, at: &CarriagePose, initial_yaw: f32) {
    let seats: Vec<Pos> = rt
        .structure
        .blocks
        .iter()
        .filter(|b| create_kinetics::rules::seat::colour_of_block(&b.state.name).is_some())
        .map(|b| b.local)
        .collect();
    if seats.is_empty() {
        return;
    }
    let structure = structure_pose(at, initial_yaw);
    let lift = create_kinetics::rules::seat::carried_lift();
    let points: Vec<(Pos, [f64; 3])> = seats
        .iter()
        .map(|l| {
            let up = [f64::from(l[0]) + 0.5 + lift[0], f64::from(l[1]) + 0.5 + lift[1], f64::from(l[2]) + 0.5 + lift[2]];
            (*l, structure.to_global_vector(up, 1.0, false))
        })
        .collect();
    super::seat::carry_points(host, seat_key(train, carriage), None, &points);
    // Mobs that touch a seat sit on it (`SeatInteractionBehaviour.handleEntityCollision`).
    super::train_riders::sweep(host, seat_key(train, carriage), rt.structure.anchor, &points);
}

/// The carriage's pose as the solid module turns a structure (`OrientedContraptionEntity`).
pub fn structure_pose(at: &CarriagePose, initial_yaw: f32) -> StructurePose {
    StructurePose {
        position: at.position,
        prev_position: at.position,
        prev_pos_invalid: false,
        delta_movement: [0.0; 3],
        rotation: Rotation::Oriented { initial_yaw, pitch: at.pitch, yaw: at.yaw, prev_pitch: at.pitch, prev_yaw: at.yaw },
        translating: false,
    }
}

/// The carriage's structure changed (a door, the fuel): the anchor's copy, the colliders' copy and the clients get it.
pub fn push_structure(host: &mut dyn Host, dimension: &str, rt: &CarriageRt) {
    let Some(anchor) = rt.anchor else { return };
    let tag = Tag::Compound(rt.structure.to_nbt(false));
    host.change_mod_block_entity(anchor, &mut |e| {
        e.nbt.insert(super::contraption::CONTRAPTION.into(), tag.clone());
    });
    host.block_entity_sync(anchor);
    super::contraption::set_live(dimension, anchor, rt.structure.clone());
}

/// The anchor's structure, if clicks changed the states of its blocks, becomes the carriage's.
fn adopt_clicked_structure(host: &dyn Host, anchor: Pos, rt: &mut CarriageRt) {
    let Some(entity) = host.mod_block_entity(anchor) else { return };
    let Some(Tag::Compound(saved)) = util::get(&entity.nbt, "Contraption") else { return };
    let Some(moved) = Moving::from_nbt(saved) else { return };
    let same = moved.blocks.len() == rt.structure.blocks.len()
        && moved.blocks.iter().zip(&rt.structure.blocks).all(|(a, b)| a.local == b.local && a.state == b.state);
    if !same {
        rt.structure = moved;
    }
}

/// `(x, z)` of a horizontal direction.
pub fn horizontal_step(dir: Dir) -> [i32; 2] {
    let n = dir.normal();
    [n[0], n[2]]
}

// --- the tick -----------------------------------------------------------------------------------------------------------------------

fn level_tick(host: &mut dyn Host) {
    let dimension = host.dimension();
    let now = util::game_time(&*host);
    let mut dim = lock(&DIMS).remove(dimension).unwrap_or_default();
    // The orphans found by the anchors' own ticks.
    for (d, pos) in std::mem::take(&mut *lock(&REMOVE)) {
        if d == dimension {
            remove_anchor(host, pos);
        } else {
            lock(&REMOVE).push((d, pos));
        }
    }
    let had_state = world::with_railways(dimension, |_| ()).is_some();
    if had_state {
        sync(host, &mut dim, now);
        run(host, dimension, &mut dim, now);
    }
    lock(&DIMS).insert(dimension.to_owned(), dim);
}

fn run(host: &mut dyn Host, dimension: &'static str, dim: &mut Dim, now: i64) {
    // The railways come out of the track's state for the tick, so nothing they call can find a lock held.
    let Some(mut rails) = world::with_railways(dimension, std::mem::take) else { return };
    restore_pending(dim, &mut rails, now);
    if now % 5 == 0 {
        read_stations(host, dim, &rails);
    }
    let world_view = build_world(dim, &rails, &*host);
    rails.tick(&world_view);
    let events = std::mem::take(&mut rails.events);
    apply_events(host, dimension, dim, &mut rails, events);
    place_carriages(host, dimension, dim, &mut rails, now);
    stations(host, dim, &mut rails);
    drive_players(host, dimension, dim, &mut rails, now);
    tell_owners(host, &mut rails);
    if dim.dirty || (now - dim.saved_at >= SAVE_EVERY && !rails.trains.is_empty()) || now - dim.saved_at >= SAVE_EVERY * 6 {
        save(host, dim, &rails);
        dim.saved_at = now;
        dim.dirty = false;
    }
    // Put the railways back; whatever the track code queued meanwhile is kept by its own state.
    world::with_railways(dimension, |r| {
        let late_events = std::mem::take(&mut r.events);
        let _ = late_events;
        *r = rails;
    });
}

/// Trains saved with the world join the railways as their track comes back.
fn restore_pending(dim: &mut Dim, rails: &mut Railways, now: i64) {
    if dim.pending.is_empty() {
        return;
    }
    let pending = std::mem::take(&mut dim.pending);
    for mut p in pending {
        if now < p.next_try {
            dim.pending.push(p);
            continue;
        }
        match train_nbt::read_train(&p.train, &rails.nets, &dim_id) {
            Some(loaded) => {
                let train = loaded.train;
                let id = train.id;
                for c in &train.carriages {
                    if let Some(rt) = p.carriages.remove(&c.id) {
                        dim.carriages.insert((id, c.id), rt);
                    }
                }
                // A carriage the save has no structure for can't be drawn: the train is let go whole.
                rails.add_loaded_train(train);
                dim.dirty = true;
            }
            None => {
                p.next_try = now + RETRY;
                dim.pending.push(p);
            }
        }
    }
}

fn apply_events(host: &mut dyn Host, dimension: &'static str, dim: &mut Dim, rails: &mut Railways, events: Vec<TrainEvent>) {
    for event in events {
        match event {
            TrainEvent::Explosion { pos, power } => {
                // `level.explode(null, .., ExplosionInteraction.NONE)`: the flash and the sound.
                let _ = host.run_command(&format!(
                    "execute in {dimension} run particle minecraft:explosion_emitter {:.2} {:.2} {:.2}",
                    pos[0], pos[1], pos[2]
                ));
                host.play_sound("minecraft:entity.generic.explode", pos, 4, power.max(1.0), 1.0);
            }
            TrainEvent::TrainRemoved { train } => {
                let gone: Vec<Key> = dim.carriages.keys().filter(|(t, _)| *t == train).copied().collect();
                for key in gone {
                    if let Some(rt) = dim.carriages.remove(&key) {
                        if let Some(a) = rt.anchor {
                            remove_anchor(host, a);
                        }
                    }
                }
                dim.drives.retain(|_, d| d.train != train);
                dim.conductors.remove(&train);
                dim.dirty = true;
            }
            // `Train.burnFuel`: one item leaves the fuel slot (and what it leaves is put back).
            TrainEvent::FuelConsumed { train, carriage, slot } => {
                let Some(rt) = dim.carriages.get_mut(&(train, carriage)) else { continue };
                if super::train_cargo::burn(&*host, &mut rt.structure, slot) {
                    rt.cargo_dirty = true;
                    rt.cargo_silent = true;
                    push_structure(host, dimension, rt);
                    dim.dirty = true;
                }
            }
            // `GlobalStation.runMailTransfer`: packages between the train's carriages and the Postboxes of its station.
            TrainEvent::MailTransfer { train, station } => {
                let Some(ids) = rails.trains.get(&train).map(|t| t.carriages.iter().map(|c| c.id).collect::<Vec<_>>()) else { continue };
                let mut taken: Vec<(Key, CarriageRt)> =
                    ids.iter().filter_map(|id| dim.carriages.remove(&(train, *id)).map(|rt| ((train, *id), rt))).collect();
                let changed = {
                    let mut structures: Vec<&mut super::contraption::Moving> = taken.iter_mut().map(|(_, rt)| &mut rt.structure).collect();
                    super::train_mail::transfer(host, station, &mut structures)
                };
                for (index, (key, mut rt)) in taken.into_iter().enumerate() {
                    if changed.contains(&index) {
                        rt.cargo_dirty = true;
                        push_structure(host, dimension, &rt);
                        dim.dirty = true;
                    }
                    dim.carriages.insert(key, rt);
                }
            }
            _ => {}
        }
    }
    let _ = rails;
}

/// Every carriage is put where its wheels are (`updateContraptionAnchors`, `alignEntity`).
fn place_carriages(host: &mut dyn Host, dimension: &'static str, dim: &mut Dim, rails: &mut Railways, now: i64) {
    let did = dim_id(dimension);
    let Railways { nets, trains, .. } = rails;
    for (id, train) in trains.iter_mut() {
        let Some(graph_id) = train.graph.or(train.point_graph) else { continue };
        let Some(graph) = nets.graphs.get(&graph_id) else { continue };
        let derailed = train.derailed;
        // `shouldActorTrigger` scales a carriage's motion to the train's speed; the doors read the station the train is at.
        let speed = train.speed.abs();
        let door_mode = train.current_station_id().and_then(|s| dim.doors.get(&s).copied());
        let spacing_of: Vec<i32> = train.carriages.iter().map(|c| c.bogey_spacing).collect();
        for (i, carriage) in train.carriages.iter_mut().enumerate() {
            if !derailed {
                carriage.update_contraption_anchors(graph);
            }
            carriage.update_traversed_dimensions(graph);
            let Some(rt) = dim.carriages.get_mut(&(*id, carriage.id)) else { continue };
            let Some(dce) = carriage.dimensions.get(&did) else { continue };
            let (Some(position), [Some(a), Some(b)]) = (dce.position_anchor, dce.rotation_anchors) else { continue };
            let pose_now = pose::align(position, [a, b]);
            let carriage_copy = carriage.clone();
            update_anchor(host, dimension, *id, &carriage_copy, graph, derailed, spacing_of[i], rt, pose_now, now);
            // The cargo is read from the structure, and the doors (actors of the carriage's structure) tick.
            let force = rt.cargo_dirty;
            let silent = std::mem::take(&mut rt.cargo_silent);
            if super::train_cargo::sync(&*host, &rt.structure, &mut carriage.storage, &mut rt.cargo_at, now, force, silent) {
                rt.cargo_dirty = false;
            }
            let moved = rt.last_pose != rt.prev_pose;
            let initial_yaw = pose::initial_yaw(horizontal_step(rt.direction));
            if super::train_doors::tick(host, dimension, rt, &pose_now, initial_yaw, if moved { speed } else { 0.0 }, door_mode) {
                push_structure(host, dimension, rt);
                dim.dirty = true;
            }
        }
    }
}

/// The station blocks' view of the trains: the redstone output while a train stands there, and a schedule left at the
/// station given to the train that arrives (`StationBlockEntity.tick`).
fn stations(host: &mut dyn Host, dim: &mut Dim, rails: &mut Railways) {
    let entity_of = |c: &Carriage| -> Option<(f32, f32)> { dim_pose_of(dim, c) };
    let mut present: BTreeMap<Uuid, Uuid> = BTreeMap::new();
    let mut ready: Vec<(Uuid, Uuid)> = Vec::new();
    let station_ids: Vec<Uuid> = rails
        .nets
        .graphs
        .values()
        .flat_map(|g| g.edge_points.values(EdgePointType::Station).into_iter().map(|p| p.id).collect::<Vec<_>>())
        .collect();
    for station in station_ids {
        let state = rails.station_train_state(station, &entity_of);
        if state.train_present {
            if let Some(train) = state.imminent_train {
                present.insert(station, train);
                if dim.present.get(&station) != Some(&train) {
                    ready.push((station, train));
                }
            }
        }
    }
    dim.present = present;
    let _ = (host, ready);
}

/// A carriage's `(pitch, yaw)` for `canDisassemble`, from the pose it was last put in.
fn dim_pose_of(dim: &Dim, carriage: &Carriage) -> Option<(f32, f32)> {
    dim.carriages
        .iter()
        .find(|(k, _)| k.1 == carriage.id)
        .and_then(|(_, rt)| rt.last_pose)
        .map(|p| (p.pitch, p.yaw))
}

/// The trains' status lines (`TrainStatus.tick`) go to their owners.
fn tell_owners(host: &mut dyn Host, rails: &mut Railways) {
    let online: Vec<u128> = host.server_players().iter().map(|p| p.uuid).collect();
    for train in rails.trains.values_mut() {
        let Some(owner) = train.owner.filter(|o| online.contains(o)) else { continue };
        if train.status.queued.is_empty() {
            continue;
        }
        for message in train.status.take_messages() {
            let line = format!("[{}] {}", train.name, drive::status_text(&message, &dim_name));
            let good = matches!(&message, StatusMessage::Info { good: true, .. });
            let _ = good;
            host.tell(owner, Text::literal(line));
        }
    }
}

// --- driving ----------------------------------------------------------------------------------------------------------------------

/// What a client of a driving player sends (`ControlsInputPacket`): the keys and whether they are pressed.
pub const CONTROLS_CHANNEL: &str = "create:train_controls";

/// The keys a player sent, to be taken in the next tick.
pub static INBOX: LazyLock<Mutex<HashMap<u128, Vec<(Vec<i32>, bool)>>>> = LazyLock::new(Mutex::default);

fn drive_players(host: &mut dyn Host, dimension: &'static str, dim: &mut Dim, rails: &mut Railways, now: i64) {
    let _ = now;
    let players = host.server_players();
    let inbox = std::mem::take(&mut *lock(&INBOX));
    let mut stops: Vec<u128> = Vec::new();
    let uuids: Vec<u128> = dim.drives.keys().copied().collect();
    for uuid in uuids {
        let Some(d) = dim.drives.get_mut(&uuid) else { continue };
        for (keys, pressed) in inbox.get(&uuid).into_iter().flatten() {
            d.keys.receive(keys, *pressed);
        }
        let held = d.keys.tick();
        let Some(player) = players.iter().find(|p| p.uuid == uuid && p.dimension == dimension) else {
            stops.push(uuid);
            continue;
        };
        if player.game_mode == util::SPECTATOR {
            stops.push(uuid);
            continue;
        }
        let Some(rt) = dim.carriages.get(&(d.train, d.carriage)) else {
            stops.push(uuid);
            continue;
        };
        // `canInteractWithBlock(player, controls, 8)`: near the carriage's middle, allowing for its length.
        let near = rt.last_pose.is_some_and(|p| {
            let dx = p.position[0] - player.position[0];
            let dy = p.position[1] - player.position[1];
            let dz = p.position[2] - player.position[2];
            (dx * dx + dy * dy + dz * dz).sqrt() <= drive::RANGE + 6.0
        });
        if !near {
            stops.push(uuid);
            continue;
        }
        let facing = rt.controls.iter().find(|(p, _)| *p == d.controls).map(|(_, f)| *f);
        let side = drive::controls_side(facing.map_or([0, 0], horizontal_step), horizontal_step(rt.direction));
        let Railways { nets, trains, .. } = &mut *rails;
        let Some(mut train) = trains.remove(&d.train) else {
            stops.push(uuid);
            continue;
        };
        let index = train.carriages.iter().position(|c| c.id == d.carriage).unwrap_or(0);
        let on_turn = drive::on_turn(&train, index, nets);
        let outcome = drive::control(&mut train, nets, trains, &held, side, &mut d.driver, on_turn);
        trains.insert(d.train, train);
        if let Some(prompt) = &outcome.prompt {
            host.action_bar(uuid, Text::literal(prompt_text(prompt)));
        } else if outcome.clear_prompt {
            host.action_bar(uuid, Text::literal(String::new()));
        }
        if !outcome.keep {
            stops.push(uuid);
        }
    }
    for uuid in stops {
        stop_driving(host, dim, uuid);
    }
}

fn prompt_text(prompt: &drive::Prompt) -> String {
    match prompt {
        drive::Prompt::ArrivedAt(name) => format!("Arrived at {name}"),
        drive::Prompt::DepartingFrom(name) => format!("Departing from {name}"),
        drive::Prompt::ApproachStation(name) => format!("Hold [Space] to approach {name}"),
        drive::Prompt::Progress { progress, .. } => {
            let progress = (*progress).clamp(0, drive::PROGRESS_BARS) as usize;
            format!("{}{}", "|".repeat(drive::PROGRESS_BARS as usize - progress), "|".repeat(progress))
        }
    }
}

fn stop_driving(host: &mut dyn Host, dim: &mut Dim, uuid: u128) {
    if dim.drives.remove(&uuid).is_some() {
        let _ = host.net_send_to_player(uuid, CONTROLS_CHANNEL, &[0]);
        host.action_bar(uuid, Text::literal("Stopped controlling contraption".to_owned()));
    }
}

/// A player starts driving the nearest carriage with controls (`ControlsInteractionBehaviour`, `startControlling`).
pub fn start_driving(host: &mut dyn Host, uuid: u128) -> Result<String, String> {
    let dimension = host.dimension();
    let player = host.server_players().into_iter().find(|p| p.uuid == uuid).ok_or("No player")?;
    let mut dim = lock(&DIMS).remove(dimension).ok_or("No trains here")?;
    let result = (|| -> Result<String, String> {
        // The carriage nearest to the player that has controls, within a few blocks.
        let best = dim
            .carriages
            .iter()
            .filter(|(_, rt)| !rt.controls.is_empty())
            .filter_map(|(key, rt)| {
                let p = rt.last_pose?;
                let d = (0..3).map(|i| (p.position[i] - player.position[i]).powi(2)).sum::<f64>().sqrt();
                (d <= drive::RANGE + 4.0).then_some((d, *key))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .ok_or_else(|| "Stand within a few blocks of a carriage with Train Controls".to_owned())?;
        let key = best.1;
        let rt = dim.carriages.get(&key).ok_or("Carriage lost")?;
        let controls = rt.controls.first().map(|(p, _)| *p).ok_or("No controls")?;
        let name = world::with_railways(dimension, |rails| {
            let Railways { nets, trains, .. } = rails;
            let train = trains.get_mut(&key.0)?;
            if !drive::start_controlling(train, nets) {
                return None;
            }
            Some(train.name.clone())
        })
        .flatten()
        .ok_or_else(|| "The train can't be driven (it's derailed)".to_owned())?;
        dim.drives.retain(|_, d| d.train != key.0);
        dim.drives.insert(uuid, Drive { train: key.0, carriage: key.1, controls, keys: drive::HeldKeys::default(), driver: drive::Driver::default() });
        Ok(name)
    })();
    lock(&DIMS).insert(dimension.to_owned(), dim);
    let name = result?;
    let _ = host.net_send_to_player(uuid, CONTROLS_CHANNEL, &[1]);
    Ok(format!("Now controlling: {name}"))
}

/// A click on a carriage's Train Controls (`ControlsInteractionBehaviour`): the player starts driving it, or stops.
/// `controller` is the anchor's position.
pub fn click_controls(host: &mut dyn Host, dimension: &str, controller: Pos, uuid: u128) -> bool {
    let train = host.mod_block_entity(controller).and_then(|e| util::get(&e.nbt, draw::TRAIN).and_then(tag_uuid));
    let Some(train) = train else { return false };
    if is_driving(dimension, uuid) {
        if let Some(d) = lock(&DIMS).get_mut(dimension) {
            d.drives.remove(&uuid);
        }
        let _ = host.net_send_to_player(uuid, CONTROLS_CHANNEL, &[0]);
        host.action_bar(uuid, Text::literal("Stopped controlling contraption".to_owned()));
        return true;
    }
    let _ = train;
    match start_driving(host, uuid) {
        Ok(line) => host.action_bar(uuid, Text::literal(line)),
        Err(line) => host.action_bar(uuid, Text::literal(line)),
    };
    true
}

/// Whether the player is driving (the server's view).
pub fn is_driving(dimension: &str, uuid: u128) -> bool {
    lock(&DIMS).get(dimension).is_some_and(|d| d.drives.contains_key(&uuid))
}

// --- the anchors' own tick ------------------------------------------------------------------------------------------------------------

/// An anchor block entity ticks on the server: one whose carriage the server doesn't know is removed after a while.
pub fn anchor_tick(host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
    let now = util::game_time(&*host);
    if now % 20 != 0 {
        return;
    }
    let dimension = host.dimension();
    let train = util::get(&event.entity.nbt, draw::TRAIN).and_then(tag_uuid);
    let carriage = util::int(&event.entity.nbt, draw::CARRIAGE);
    let key = (dimension.to_owned(), event.pos);
    let alive = train.is_some_and(|t| knows(dimension, t, carriage));
    let mut orphans = lock(&ORPHANS);
    if alive {
        orphans.remove(&key);
        return;
    }
    let since = *orphans.entry(key.clone()).or_insert(now);
    if now - since >= ORPHAN_TICKS {
        orphans.remove(&key);
        drop(orphans);
        lock(&REMOVE).push((dimension.to_owned(), event.pos));
    }
}

/// `(pitch, yaw)` of each carriage of a train as it was last put (what `canDisassemble` reads of the entity).
pub fn carriage_poses(dimension: &str, train: Uuid) -> BTreeMap<i32, (f32, f32)> {
    lock(&DIMS)
        .get(dimension)
        .map(|d| d.carriages.iter().filter(|((t, _), _)| *t == train).filter_map(|((_, c), rt)| rt.last_pose.map(|p| (*c, (p.pitch, p.yaw)))).collect())
        .unwrap_or_default()
}

/// Takes an anchor block out of the world.
pub fn remove_anchor_block(host: &mut dyn Host, pos: Pos) {
    remove_anchor(host, pos);
}

/// A train that was disassembled is forgotten by everything that kept something of it.
pub fn forget_train(dimension: &str, train: Uuid) {
    if let Some(d) = lock(&DIMS).get_mut(dimension) {
        d.carriages.retain(|(t, _), _| *t != train);
        d.drives.retain(|_, drive| drive.train != train);
        d.conductors.remove(&train);
        d.present.retain(|_, t| *t != train);
        d.dirty = true;
    }
}

// --- what the riders and the doors ask of the trains ---------------------------------------------------------------------------------

/// The carriage the anchor block at `anchor` draws: its train and carriage id.
pub fn carriage_of_anchor(host: &dyn Host, anchor: Pos) -> Option<(Uuid, i32)> {
    if host.block(anchor) != ANCHOR {
        return None;
    }
    let entity = host.mod_block_entity(anchor)?;
    Some((util::get(&entity.nbt, draw::TRAIN).and_then(tag_uuid)?, util::int(&entity.nbt, draw::CARRIAGE)))
}

/// The carriage a rider sits on (`getRootVehicle() instanceof CarriageContraptionEntity`): its train, its id, and the
/// seat's local position.
pub fn carriage_of_rider(host: &dyn Host, dimension: &str, rider: u128) -> Option<(Uuid, i32, Pos)> {
    let keys: Vec<Key> = lock(&DIMS).get(dimension)?.carriages.keys().copied().collect();
    keys.into_iter().find_map(|(train, carriage)| {
        super::seat::passengers_of(host, dimension, seat_key(train, carriage))
            .into_iter()
            .find(|(_, uuid, _)| *uuid == rider)
            .map(|(seat, _, _)| (train, carriage, seat))
    })
}

/// What a carriage knows for a click on its seat or burner: whether the seat is in front of controls, and the blocks.
#[derive(Clone, Debug)]
pub struct CarriageInfo {
    pub direction: Dir,
    pub conductor_seat: bool,
    pub blocks: Vec<(Pos, State)>,
}

pub fn carriage_info(dimension: &str, train: Uuid, carriage: i32, seat: Option<Pos>) -> Option<CarriageInfo> {
    let map = lock(&DIMS);
    let rt = map.get(dimension)?.carriages.get(&(train, carriage))?;
    Some(CarriageInfo {
        direction: rt.direction,
        conductor_seat: seat.is_some_and(|s| rt.conductor_seats.contains_key(&s)),
        blocks: rt.structure.blocks.iter().map(|b| (b.local, b.state.clone())).collect(),
    })
}

/// The entity ids of everyone who wears the Train Hat now (`EntityHats.shouldRenderTrainHat`): the riders of a conductor seat, of a
/// carriage with a schedule (a player's needs none). The client draws the hat on these (`train_hat.rs`). The head slot isn't read.
pub fn hat_wearers(host: &dyn Host, dimension: &str) -> Vec<u32> {
    use create_kinetics::rules::trains::schedule_hand::{HatView, wears_train_hat};
    let keys: Vec<Key> = lock(&DIMS).get(dimension).map(|d| d.carriages.keys().copied().collect()).unwrap_or_default();
    let mut out = Vec::new();
    for (train, carriage) in keys {
        let has_schedule = runtime_view(dimension, train).is_some_and(|v| v.has_schedule);
        for (seat, id, player) in super::seat::riders_with_ids(host, dimension, seat_key(train, carriage)) {
            let conductor_seat = carriage_info(dimension, train, carriage, Some(seat)).is_some_and(|i| i.conductor_seat);
            let view = HatView { head_item: false, tagged: false, on_carriage: true, carriage_has_schedule: has_schedule, player, conductor_seat };
            if wears_train_hat(&view) {
                out.push(id);
            }
        }
    }
    out
}

/// `train.runtime` as the Schedule hand-over reads it.

pub fn runtime_view(dimension: &str, train: Uuid) -> Option<RuntimeView> {
    world::with_railways(dimension, |r| {
        r.trains.get(&train).map(|t| RuntimeView {
            has_schedule: t.runtime.schedule.is_some(),
            paused: t.runtime.paused,
            completed: t.runtime.completed,
            auto: t.runtime.is_auto_schedule,
        })
    })
    .flatten()
}

/// `train.runtime.paused = false`.
pub fn resume_schedule(dimension: &str, train: Uuid) -> bool {
    world::with_railways(dimension, |r| r.trains.get_mut(&train).map(|t| t.runtime.paused = false).is_some()).unwrap_or(false)
}

// --- the stations' questions -----------------------------------------------------------------------------------------------------------


/// What a station's screens know of its train (`getImminent()`): how it stands at the station, and what the screen shows of it.
pub fn station_train(dimension: &str, station: Uuid) -> Option<(StationTrainState, TrainInfo)> {
    let imminent = world::with_railways(dimension, |r| r.station_train_state(station, &|_| None).imminent_train).flatten()?;
    // `canDisassemble` reads each carriage's pose as it was last put.
    let poses: BTreeMap<i32, (f32, f32)> = carriage_poses(dimension, imminent);
    world::with_railways(dimension, |r| {
        let state = r.station_train_state(station, &|c: &Carriage| poses.get(&c.id).copied());
        let train = r.trains.get(&state.imminent_train?)?;
        let info = TrainInfo {
            id: train.id.to_string(),
            name: train.name.clone(),
            icon: train.icon.clone(),
            spacings: train.carriages.iter().map(|c| c.bogey_spacing).collect(),
            double_ended: train.double_ended,
            distance: train.navigation.distance_to_destination as f32,
            map_color: train.map_color_index,
        };
        Some((state, info))
    })
    .flatten()
}

/// `TrainEditPacket.handleSided`: the train's name, icon and map colour.
pub fn edit_train(dimension: &str, train: Uuid, edit: &TrainEdit) {
    let done = world::with_railways(dimension, |r| {
        r.trains
            .get_mut(&train)
            .map(|t| screens::apply_train_edit(&mut t.name, &mut t.icon, &mut t.map_color_index, edit))
            .is_some()
    })
    .unwrap_or(false);
    if done {
        with_dim(dimension, |d| d.dirty());
    }
}

/// `train.countPlayerPassengers()`: the players on the seats of the train's carriages.
pub fn count_passengers(host: &dyn Host, dimension: &str, train: Uuid) -> i32 {
    let carriages: Vec<i32> = lock(&DIMS).get(dimension).map(|d| d.carriages.keys().filter(|(t, _)| *t == train).map(|(_, c)| *c).collect()).unwrap_or_default();
    carriages.into_iter().map(|c| passengers::count_player_passengers(&riders_of(host, dimension, train, c))).sum()
}

/// The train standing at a station, if any.
pub fn train_at(dimension: &str, station: Uuid) -> Option<Uuid> {
    lock(&DIMS).get(dimension).and_then(|d| d.present.get(&station).copied())
}

// --- /train ---------------------------------------------------------------------------------------------------------------------------------

/// The train nearest to a point, within `range` blocks of one of its carriages.
fn nearest_train(dimension: &str, at: [f64; 3], range: f64) -> Option<Uuid> {
    let map = lock(&DIMS);
    let dim = map.get(dimension)?;
    dim.carriages
        .iter()
        .filter_map(|((train, _), rt)| {
            let p = rt.last_pose?;
            let d = (0..3).map(|i| (p.position[i] - at[i]).powi(2)).sum::<f64>().sqrt();
            (d <= range).then_some((d, *train))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, t)| t)
}

/// `ScheduleItem.handScheduleTo`: the train takes the schedule (`runtime.setSchedule`), automatically running it.
pub fn give_schedule(host: &mut dyn Host, train_id: Uuid, schedule: Schedule, auto: bool) -> bool {
    let dimension = host.dimension();
    let day_time = host.day_time();
    let Some(dim_world) = ({
        let map = lock(&DIMS);
        map.get(dimension).map(|d| (d.conductors.clone(), d.carriages.iter().map(|(k, rt)| (*k, rt.conductor_sides())).collect::<BTreeMap<Key, [bool; 2]>>()))
    }) else {
        return false;
    };
    let world_view = World {
        day_time,
        dimension: dim_id(dimension),
        players: BTreeMap::new(),
        conductors: dim_world.1.into_iter().filter(|(k, _)| dim_world.0.contains(&k.0)).collect(),
        powered: Default::default(),
        unloaded: Default::default(),
    };
    world::with_railways(dimension, |rails| {
        let Railways { nets, trains, .. } = rails;
        let Some(mut train) = trains.remove(&train_id) else { return false };
        train.set_schedule(nets, trains, &world_view, schedule, auto);
        trains.insert(train_id, train);
        true
    })
    .unwrap_or(false)
}

/// `Train.runtime.returnSchedule()`: the train lets go of its schedule, for the station's `dropSchedule`.
pub fn take_schedule(host: &mut dyn Host, train_id: Uuid) -> Option<Schedule> {
    let dimension = host.dimension();
    let day_time = host.day_time();
    let conductors = lock(&DIMS).get(dimension).map(|d| d.conductors.clone()).unwrap_or_default();
    let world_view = World { day_time, dimension: dim_id(dimension), conductors: BTreeMap::new(), players: BTreeMap::new(), powered: Default::default(), unloaded: Default::default() };
    let _ = conductors;
    world::with_railways(dimension, |rails| {
        let Railways { nets, trains, .. } = rails;
        let mut train = trains.remove(&train_id)?;
        let schedule = train.return_schedule(nets, trains, &world_view);
        trains.insert(train_id, train);
        schedule
    })
    .flatten()
}

fn say(host: &mut dyn Host, uuid: u128, line: impl Into<String>) {
    host.tell(uuid, Text::literal(line.into()));
}

fn command(host: &mut dyn Host, uuid: u128, rest: &str) {
    let dimension = host.dimension();
    let Some(player) = host.server_players().into_iter().find(|p| p.uuid == uuid) else { return };
    let mut words = rest.split_whitespace();
    match words.next().unwrap_or("info") {
        "drive" => match start_driving(host, uuid) {
            Ok(line) => say(host, uuid, line),
            Err(line) => say(host, uuid, line),
        },
        "info" => {
            let Some(id) = nearest_train(dimension, player.position, 24.0) else {
                say(host, uuid, "No train within 24 blocks");
                return;
            };
            let line = world::with_railways(dimension, |r| {
                r.trains.get(&id).map(|t| {
                    format!(
                        "{}: {} carriage{}, speed {:.1} blocks/s{}{}",
                        t.name,
                        t.carriages.len(),
                        if t.carriages.len() == 1 { "" } else { "s" },
                        t.speed * 20.0,
                        if t.derailed { ", derailed" } else { "" },
                        t.current_station_in(&r.nets).and_then(|p| p.station()).map_or(String::new(), |s| format!(", at {}", s.name))
                    )
                })
            })
            .flatten();
            say(host, uuid, line.unwrap_or_else(|| "The train is gone".to_owned()));
        }
        "conductor" => {
            let Some(id) = nearest_train(dimension, player.position, 24.0) else {
                say(host, uuid, "No train within 24 blocks");
                return;
            };
            let on = with_dim(dimension, |d| {
                if !d.dim.conductors.remove(&id) {
                    d.dim.conductors.insert(id);
                    true
                } else {
                    false
                }
            })
            .unwrap_or(false);
            say(host, uuid, if on { "A driver is aboard the train" } else { "The driver left the train" });
        }
        "name" => {
            let name: Vec<&str> = words.collect();
            let Some(id) = nearest_train(dimension, player.position, 24.0) else {
                say(host, uuid, "No train within 24 blocks");
                return;
            };
            if name.is_empty() {
                say(host, uuid, "/train name <text>");
                return;
            }
            let name = name.join(" ");
            world::with_railways(dimension, |r| {
                if let Some(t) = r.trains.get_mut(&id) {
                    t.name = name.clone();
                }
            });
            with_dim(dimension, |d| d.dirty());
            say(host, uuid, format!("The train is now called '{name}'"));
        }
        "icon" => {
            let Some(id) = nearest_train(dimension, player.position, 24.0) else {
                say(host, uuid, "No train within 24 blocks");
                return;
            };
            let word: Vec<&str> = words.collect();
            let Some(icon) = screens::icon_by_word(&word.join(" ")) else {
                let all: Vec<&str> = screens::ICON_TYPES.iter().map(|i| i.id.trim_start_matches("create:")).collect();
                say(host, uuid, format!("/train icon <{}>", all.join(" | ")));
                return;
            };
            // `AssemblyScreen`'s icon scroll: only the icon changes.
            let current = world::with_railways(dimension, |r| r.trains.get(&id).map(|t| (t.name.clone(), t.map_color_index))).flatten();
            if let Some((name, map_color)) = current {
                edit_train(dimension, id, &TrainEdit { id: id.to_string(), name, icon: icon.id.to_owned(), map_color });
                say(host, uuid, format!("The train's icon is now '{}'", icon.id.trim_start_matches("create:")));
            }
        }
        "schedule" => {
            let Some(id) = nearest_train(dimension, player.position, 24.0) else {
                say(host, uuid, "No train within 24 blocks");
                return;
            };
            match super::schedule::held_schedule(&*host, uuid) {
                Some(schedule) if !schedule.entries.is_empty() => {
                    if give_schedule(host, id, schedule, false) {
                        say(host, uuid, "Train is now following this Schedule");
                    } else {
                        say(host, uuid, "The train didn't take the Schedule");
                    }
                }
                Some(_) => say(host, uuid, "This Schedule does not have any Stops yet"),
                None => say(host, uuid, "Hold a Schedule"),
            }
        }
        _ => say(host, uuid, "/train [info | drive | conductor | name <text> | icon <type> | schedule]"),
    }
}

impl Hooks for Trains {
    fn level_tick_post(&self, host: &mut dyn Host, _: &mut LevelTickPostEvent<'_>) {
        if host.window() {
            return;
        }
        level_tick(host);
    }

    fn register_commands(&self, _: &mut dyn Host, event: &mut RegisterCommandsEvent<'_>) {
        event.commands.push(NativeCommand::new("train"));
        event.commands.push(NativeCommand::new("train <text:text>"));
    }

    fn perform_command(&self, host: &mut dyn Host, event: &mut PerformCommandEvent<'_>) -> Flow {
        let line = event.command.trim();
        let Some(rest) = (if line == "train" { Some("") } else { line.strip_prefix("train ") }) else {
            return Flow::Continue;
        };
        command(host, event.player.uuid, rest);
        Flow::Cancel
    }
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tree_survives_the_tag() {
        let tree = Value::Compound(vec![
            ("Id".into(), Value::IntArray(vec![1, 2, 3, 4])),
            ("Speed".into(), Value::Double(0.25)),
            ("Name".into(), Value::Str("Express".into())),
            ("Data".into(), Value::ByteArray(vec![1, -2, 3])),
            ("List".into(), Value::List(vec![Value::Int(1), Value::Int(2)])),
        ]);
        assert_eq!(from_tag(&to_tag(&tree)), Some(tree));
    }

    #[test]
    fn a_uuid_survives_the_tag() {
        let id: Uuid = 0x0123_4567_89ab_cdef_0011_2233_4455_6677;
        assert_eq!(tag_uuid(&uuid_tag(id)), Some(id));
    }

    #[test]
    fn the_horizontal_step_drops_the_height() {
        assert_eq!(horizontal_step(Dir::East), [1, 0]);
        assert_eq!(horizontal_step(Dir::North), [0, -1]);
    }

    #[test]
    fn a_carriage_survives_its_saved_form() {
        let mut moving = Moving::default();
        moving.anchor = [3, 4, 5];
        moving.blocks.push(MovedBlock { local: [0, 0, 0], state: State::parse("create:small_bogey[axis=x]"), data: None });
        moving.blocks.push(MovedBlock { local: [0, 1, 0], state: State::parse("create:controls[facing=east]"), data: None });
        let summary = cc::summarize(&moving.blocks.iter().map(|b| (b.local, b.state.clone())).collect::<Vec<_>>(), Dir::East);
        let mut rt = CarriageRt::new(moving, Dir::East, &summary);
        rt.anchor = Some([9, 9, 9]);
        rt.wheel = [12.5, -3.0];
        let (id, back) = read_rt(&write_rt(7, &rt)).expect("reads");
        assert_eq!(id, 7);
        assert_eq!(back.structure.blocks.len(), 2);
        assert_eq!(back.direction, Dir::East);
        assert!(back.forward_controls && !back.backward_controls);
        assert_eq!(back.controls, vec![([0, 1, 0], Dir::East)]);
        assert_eq!(back.anchor, Some([9, 9, 9]));
        assert_eq!(back.wheel, [12.5, -3.0]);
    }

    #[test]
    fn the_status_text_is_listed_for_the_trains_messages() {
        let m = StatusMessage::Info { key: "no_path".into(), good: false, args: vec![] };
        assert!(drive::status_text(&m, &dim_name).contains("No suitable path"));
    }
}
