//! The Elevator Pulley and the Elevator Contact (`ElevatorPulleyBlock`, `ElevatorPulleyBlockEntity`,
//! `ElevatorContactBlock`, `ElevatorContactBlockEntity`, `ElevatorColumn`, `ElevatorContraption`,
//! `ElevatorPulleyRenderer`) on the contraption engine (`contraption.rs`, where the design and the
//! differences common to every contraption are written; the rules, with the list of what differs
//! from Create, are `create_kinetics::rules::elevator`).
//!
//! An empty hand on a turning Elevator Pulley takes the cabin under it (what hangs from the first
//! solid block below, with one horizontal contact) and makes the column of contacts in front of
//! that contact its floors. Redstone on a floor's contact calls the cabin there: it speeds up,
//! brakes onto the floor and its contact powers while it stands there. An empty hand again sets
//! the cabin down where it is. Floors are the Elevator Contacts, placed in a line facing the cabin.
//!
//! The columns (floors, target, whether a cabin runs) are kept in memory by the contacts' ticks and
//! saved in each contact (`ColumnTarget`, `ColumnActive`), as Create's `ElevatorColumn` is. The cabin's
//! position is replayed from a saved snapshot (`Offset`, `Speed`, `SyncTime`, `Target`) on the
//! server and the clients alike, so nothing is written while it moves.
//!
//! Actors on the cabin (`actor.rs`) and the Portable Interfaces run on it, each tick it runs, with
//! the cabin's `Pose` (it moves up and down from its anchor, `toPosition`). While one stalls the
//! cabin (`Stalled`, `psi_motion`) it keeps its offset, and its speed goes on building as Create's
//! `prevSpeed` does (`replay_held`): the snapshot is moved to now whenever that changes. The
//! cabin is a `PulleyContraption` (the Roller sleeps) and its Controls start no actor disabled.
//! A cabin that turns round doesn't stop its actors (the elevator's `onSpeedChanged` doesn't call
//! `super`), and setting it down stops them first.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use create_kinetics::rules::contraption::{self as rules, Transform, World, add};
use create_kinetics::rules::elevator::{self as elevator, Column, ColumnCoords, Snapshot};
use create_kinetics::rules::pulley as pulley_rules;
use create_kinetics::{Axis, Dir, Kind, Node, Pos, looking};
use crate::compat::{
    BlockEntityEvent, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent, BlockEntityTickEvent,
    BlockEntityTicksEvent, BlockEntityUpdateTagEvent, BlockSignalEvent, BlockStateId, Compound,
    DestroyBlockEvent, Flow, Hooks, Host, IsSignalSourceEvent, LevelHost, ModBlockEntity,
    RightClickBlockEvent, StandaloneModelsEvent, StateForPlacementEvent, Tag, UseItemOnEvent,
};

use super::actor;
use super::actor_draw;
use super::contraption::{
    self as engine, ASSEMBLE, CONTRAPTION, HostWorld, LAST_EXCEPTION, RUNNING, SYNC_TIME,
};
use super::diodes_signal as signal;
use super::portable_interface_motion as psi_motion;
use create_kinetics::rules::actor::{Carrier, Structure};
use crate::{kinetic, util};

const PULLEY: &str = elevator::PULLEY;
const CONTACT: &str = elevator::CONTACT;
/// `AllPartialModels.ELEVATOR_COIL`, `ELEVATOR_BELT`, `ELEVATOR_BELT_HALF`, `ELEVATOR_MAGNET`.
const COIL: &str = "create:block/elevator_pulley/rope_coil";
const BELT: &str = "create:block/elevator_pulley/rope";
const BELT_HALF: &str = "create:block/elevator_pulley/rope_half";
const MAGNET: &str = "create:block/elevator_pulley/pulley_magnet";

// Saved as `LinearActuatorBlockEntity`, `ElevatorContraption` and `ElevatorContactBlockEntity` name them.
const OFFSET: &str = "Offset";
const SPEED: &str = "CabinSpeed";
const TARGET: &str = "OffsetTarget";
const INITIAL_OFFSET: &str = "InitialOffset";
const CONTACT_Y: &str = "ContactY";
const MAX_CONTACT_Y: &str = "MaxContactY";
const MIN_CONTACT_Y: &str = "MinContactY";
const COLUMN: &str = "Column";
const DISASSEMBLE: &str = "Disassemble";
const SHORT_NAME: &str = "ShortName";
const LONG_NAME: &str = "LongName";
const COLUMN_TARGET: &str = "ColumnTarget";
const COLUMN_ACTIVE: &str = "ColumnActive";

struct Elevators;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Elevators)
}

fn flag(value: bool) -> Tag {
    Tag::Byte(i8::from(value))
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn put(entity: &mut ModBlockEntity, key: &str, value: Tag) {
    if util::differs(&entity.nbt, key, &value) {
        entity.nbt.insert(key.into(), value);
    }
}

// --- the columns --------------------------------------------------------------------------------------

type Key = (String, ColumnCoords);

static COLUMNS: LazyLock<Mutex<HashMap<Key, Column>>> = LazyLock::new(Mutex::default);
/// When each contact last ticked: a contact that stops ticking (broken, unloaded) leaves its column.
static SEEN: LazyLock<Mutex<HashMap<(String, Pos), i64>>> = LazyLock::new(Mutex::default);
/// The floor the cabin of each column stands at, once it has arrived: that contact powers.
static ARRIVED: LazyLock<Mutex<HashMap<Key, i32>>> = LazyLock::new(Mutex::default);

/// `ElevatorColumn.get(level, coords) != null`: some Elevator Contact has registered a floor in the
/// column (the Redstone Contact's Wrench uses it).
pub(super) fn column_exists(dimension: &str, coords: ColumnCoords) -> bool {
    lock(&COLUMNS)
        .get(&(dimension.to_owned(), coords))
        .is_some_and(|column| !column.floors().is_empty())
}

/// How long a contact may go without ticking before its column forgets it.
const STALE: i64 = 60;

/// Writes the column's target and whether a cabin runs into each of its contacts
/// (`ElevatorContactBlockEntity.write`, `markDirty`).
fn save_column(host: &mut dyn Host, coords: ColumnCoords, column: &Column) {
    for y in column.floors() {
        let at = coords.contact_at(y);
        let (target, active) = (column.target_value(), column.is_active());
        host.change_mod_block_entity(at, &mut |entity| {
            put(entity, COLUMN_TARGET, Tag::Int(target));
            put(entity, COLUMN_ACTIVE, flag(active));
        });
    }
}

fn contact_state(host: &dyn Host, pos: Pos) -> Option<(Dir, bool, bool, bool)> {
    if host.block(pos) != CONTACT {
        return None;
    }
    let facing = util::facing(host, pos, "facing")?;
    let is = |key: &str| host.block_property(pos, key).as_deref() == Some("true");
    Some((facing, is("powered"), is("calling"), is("powering")))
}

fn tick_contact(host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
    let pos = event.pos;
    let Some((facing, powered, calling, powering)) = contact_state(&*host, pos) else {
        return;
    };
    let dimension = host.dimension().to_owned();
    let now = util::game_time(&*host);
    let coords = elevator::column_of(pos, facing);
    let key = (dimension.clone(), coords);
    // Register the floor, and take back what the saved contact remembers of the column.
    let restored = {
        let mut columns = lock(&COLUMNS);
        let column = columns.entry(key.clone()).or_default();
        column.add(pos[1]);
        let mut restored = false;
        if column.targeted().is_none() && util::has(&event.entity.nbt, COLUMN_TARGET) {
            column.target(util::int(&event.entity.nbt, COLUMN_TARGET));
            restored = true;
        }
        if util::boolean(&event.entity.nbt, COLUMN_ACTIVE) && !column.is_active() {
            column.set_active(true);
        }
        restored
    };
    let _ = restored;
    {
        let mut seen = lock(&SEEN);
        seen.insert((dimension.clone(), pos), now);
        if now % 100 == 0 {
            let stale: Vec<(String, Pos)> = seen
                .iter()
                .filter(|(_, last)| now - **last > STALE)
                .map(|(k, _)| k.clone())
                .collect();
            let mut columns = lock(&COLUMNS);
            for gone in stale {
                seen.remove(&gone);
                for ((dim, _), column) in columns.iter_mut() {
                    if *dim == gone.0 {
                        column.remove(gone.1[1]);
                    }
                }
            }
            columns.retain(|_, column| !column.is_empty());
        }
    }

    // `neighborChanged`, polled: a signal on the contact.
    let has_signal = host.neighbor_signal(&dimension, pos) > 0;
    let reaction = elevator::neighbour_changed(powered, has_signal, calling);
    if let Some(on) = reaction.powered {
        host.set_block_property(pos, "powered", if on { "true" } else { "false" });
    }
    if reaction.call {
        // `callToContactAndUpdate`.
        let (others, snapshot) = {
            let mut columns = lock(&COLUMNS);
            let column = columns.entry(key.clone()).or_default();
            (elevator::call(column, pos[1]), column.clone())
        };
        for y in others {
            let other = coords.contact_at(y);
            if host.block(other) == CONTACT {
                host.set_block_property(other, "calling", "false");
            }
        }
        host.set_block_property(pos, "calling", "true");
        host.update_signal_neighbors(&dimension, pos);
        save_column(host, coords, &snapshot);
    }

    // The contact the cabin stands at powers (`RedstoneContactBlock.hasValidContact`).
    let should_power = lock(&ARRIVED).get(&key) == Some(&pos[1]);
    if powering != should_power {
        host.set_block_property(pos, "powering", if should_power { "true" } else { "false" });
        host.update_signal_neighbors(&dimension, pos);
    }
}

// --- the cabin ------------------------------------------------------------------------------------------------

struct Saved {
    running: bool,
    snapshot: Snapshot,
    initial: i32,
    contact_y: i32,
    max_contact_y: i32,
    min_contact_y: i32,
    column: ColumnCoords,
    assemble: i32,
}

fn dir_index(dir: Dir) -> i32 {
    Dir::ALL.iter().position(|d| *d == dir).unwrap_or(0) as i32
}

fn column_tag(coords: ColumnCoords) -> Tag {
    let mut tag = Compound::new();
    tag.insert("X".into(), Tag::Int(coords.x));
    tag.insert("Z".into(), Tag::Int(coords.z));
    tag.insert("Side".into(), Tag::Int(dir_index(coords.side)));
    Tag::Compound(tag)
}

/// Where the cabin is at game time `t` and a tick before (`solid`): `toPosition` with the cabin's
/// offset replayed from its snapshot, as the tick and the drawing do.
pub(crate) fn solid_sample(
    _host: &dyn Host,
    _pos: Pos,
    entity: &ModBlockEntity,
    anchor: Pos,
    t: f64,
) -> Option<crate::solid::Sample> {
    if entity.id != PULLEY {
        return None;
    }
    let state = saved(entity);
    if !state.running {
        return None;
    }
    let rpm = kinetic::speed(entity);
    let held = psi_motion::is_stalled(&entity.nbt);
    let at = |t: f64| {
        let whole = t.floor();
        let (cabin, speed, _) = elevator::replay_as(state.snapshot, rpm, whole as i64 - state.snapshot.time, held);
        let cabin = if held { cabin } else { cabin + speed * (t - whole) as f32 };
        [
            f64::from(anchor[0]),
            f64::from(anchor[1]) + f64::from(pulley_rules::rise(state.initial, cabin)),
            f64::from(anchor[2]),
        ]
    };
    Some(crate::solid::Sample::Slide { now: at(t), prev: at(t - 1.0) })
}

fn saved(entity: &ModBlockEntity) -> Saved {
    let column = match util::get(&entity.nbt, COLUMN) {
        Some(Tag::Compound(c)) => ColumnCoords {
            x: util::int(c, "X"),
            z: util::int(c, "Z"),
            side: usize::try_from(util::int(c, "Side")).ok().and_then(|i| Dir::ALL.get(i).copied()).unwrap_or(Dir::North),
        },
        _ => ColumnCoords { x: 0, z: 0, side: Dir::North },
    };
    Saved {
        running: util::boolean(&entity.nbt, RUNNING),
        snapshot: Snapshot {
            offset: util::float(&entity.nbt, OFFSET).unwrap_or(0.0),
            speed: util::float(&entity.nbt, SPEED).unwrap_or(0.0),
            time: util::long(&entity.nbt, SYNC_TIME),
            target: util::int(&entity.nbt, TARGET),
        },
        initial: util::int(&entity.nbt, INITIAL_OFFSET),
        contact_y: util::int(&entity.nbt, CONTACT_Y),
        max_contact_y: util::int(&entity.nbt, MAX_CONTACT_Y),
        min_contact_y: util::int(&entity.nbt, MIN_CONTACT_Y),
        column,
        assemble: util::int(&entity.nbt, ASSEMBLE),
    }
}

fn new_pulley_entity() -> Compound {
    let mut nbt = kinetic::new_entity();
    nbt.insert(RUNNING.into(), flag(false));
    nbt.insert(ASSEMBLE.into(), flag(false));
    nbt.insert(SYNC_TIME.into(), Tag::Long(0));
    nbt.insert(TARGET.into(), Tag::Int(0));
    nbt.insert(INITIAL_OFFSET.into(), Tag::Int(0));
    nbt.insert(LAST_EXCEPTION.into(), Tag::String("".into()));
    util::set_float(&mut nbt, OFFSET, 0.0);
    util::set_float(&mut nbt, SPEED, 0.0);
    nbt
}

fn below(pos: Pos, i: i32) -> Pos {
    [pos[0], pos[1] - i, pos[2]]
}

/// The pulley's rotation axis: across its facing (`getRotationAxis`).
fn rotation_axis(facing: Dir) -> Axis {
    facing.clockwise_about(Axis::Y).axis()
}

fn axis_ends(axis: Axis) -> [Dir; 2] {
    match axis {
        Axis::X => [Dir::West, Dir::East],
        Axis::Y => [Dir::Down, Dir::Up],
        Axis::Z => [Dir::North, Dir::South],
    }
}

/// `ElevatorPulleyBlockEntity.assemble`. Returns whether the cabin runs now.
fn assemble(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity, now: i64, manual: bool) -> bool {
    let dimension = host.dimension().to_owned();
    engine::prepare(host);
    // The first block below that has collision and can't be replaced is the cabin's top.
    let offset = {
        let Some(world) = HostWorld::new(&*host) else { return false };
        let mut i = 1;
        while i <= pulley_rules::MAX_ROPE_LENGTH {
            let at = below(pos, i);
            let Some(state) = world.state(at) else { break };
            if !world.collision_empty(at) && !rules::replaceable(&state) {
                break;
            }
            i += 1;
        }
        i - 1
    };
    let anchor = pulley_rules::anchor(pos, offset);
    let checked = {
        let Some(world) = HostWorld::new(&*host) else { return false };
        pulley_rules::assemble(&world, anchor, offset).and_then(|found| {
            if found.blocks.is_empty() {
                return Ok(None);
            }
            let contacts: Vec<(Pos, Dir)> = found
                .blocks
                .iter()
                .filter(|(_, s)| elevator::is_contact(s))
                .filter_map(|(p, s)| s.dir("facing").map(|f| (rules::sub(*p, anchor), f)))
                .collect();
            let (column, contact_y) = elevator::cabin_column(&contacts)?;
            let column = column.relative(anchor);
            if lock(&COLUMNS).get(&(dimension.clone(), column)).is_some_and(Column::is_active) {
                return Err(rules::Fail::ColumnConflict);
            }
            Ok(Some((found, column, contact_y)))
        })
    };
    let (found, column, contact_y) = match checked {
        Ok(Some(done)) => done,
        Ok(None) => return false,
        Err(fail) => {
            let changed = engine::store_exception(entity, Some(&fail));
            if manual {
                engine::tell_near(host, pos, &fail);
            }
            if changed {
                host.block_entity_sync(pos);
            }
            return false;
        }
    };
    let moving = engine::capture(host, anchor, &found.blocks, &[], &|p| rules::sub(p, anchor));
    engine::remove_from_world(host, &moving, &|local| add(local, anchor), &mut |_, _, _| false);
    engine::set_live(&dimension, pos, moving.clone());
    let max_contact_y = pos[1] + contact_y - 1;
    // `column.target(...)`, `setActive(true)`: the cabin stands at the floor it started at.
    let target_y = max_contact_y - offset;
    let snapshot = {
        let mut columns = lock(&COLUMNS);
        let entry = columns.entry((dimension.clone(), column)).or_default();
        entry.target(target_y);
        entry.set_active(true);
        entry.clone()
    };
    lock(&ARRIVED).insert((dimension, column), target_y);
    save_column(host, column, &snapshot);
    entity.nbt.insert(CONTRAPTION.into(), Tag::Compound(moving.to_nbt(false)));
    entity.nbt.insert(RUNNING.into(), flag(true));
    entity.nbt.insert(INITIAL_OFFSET.into(), Tag::Int(offset));
    entity.nbt.insert(CONTACT_Y.into(), Tag::Int(contact_y));
    entity.nbt.insert(MAX_CONTACT_Y.into(), Tag::Int(max_contact_y));
    entity.nbt.insert(MIN_CONTACT_Y.into(), Tag::Int(max_contact_y - pulley_rules::MAX_ROPE_LENGTH));
    entity.nbt.insert(COLUMN.into(), column_tag(column));
    entity.nbt.insert(TARGET.into(), Tag::Int(offset));
    util::set_float(&mut entity.nbt, OFFSET, offset as f32);
    util::set_float(&mut entity.nbt, SPEED, 0.0);
    entity.nbt.insert(SYNC_TIME.into(), Tag::Long(now));
    engine::store_exception(entity, None);
    host.block_entity_sync(pos);
    true
}

/// `ElevatorPulleyBlockEntity.disassemble`: the cabin down where it is, the column let go.
fn set_down(host: &mut dyn Host, pos: Pos, nbt: &Compound, offset: f32, initial: i32, column: ColumnCoords) {
    let dimension = host.dimension().to_owned();
    // `Contraption.stop`: the actors on the cabin stop first.
    actor::stop(host, pos, nbt);
    let grid = (offset + 0.5).floor().max(0.0) as i32;
    if let Some(live) = engine::live(&dimension, pos, nbt) {
        let anchor = live.moving.anchor;
        let transform = Transform::translation([anchor[0], anchor[1] + initial - grid, anchor[2]]);
        engine::place_in_world(host, &live.moving, &transform, &mut |_, _, _| false);
    }
    engine::forget_live(&dimension, pos);
    let snapshot = {
        let mut columns = lock(&COLUMNS);
        columns.get_mut(&(dimension.clone(), column)).map(|c| {
            c.set_active(false);
            c.clone()
        })
    };
    lock(&ARRIVED).remove(&(dimension, column));
    if let Some(snapshot) = snapshot {
        save_column(host, column, &snapshot);
    }
}

fn stop(entity: &mut ModBlockEntity, now: i64) {
    entity.nbt.insert(RUNNING.into(), flag(false));
    entity.nbt.remove(&psi_motion::STALLED.into());
    entity.nbt.remove(&CONTRAPTION.into());
    util::set_float(&mut entity.nbt, OFFSET, 0.0);
    util::set_float(&mut entity.nbt, SPEED, 0.0);
    entity.nbt.insert(TARGET.into(), Tag::Int(0));
    entity.nbt.insert(INITIAL_OFFSET.into(), Tag::Int(0));
    entity.nbt.insert(SYNC_TIME.into(), Tag::Long(now));
    put(entity, ASSEMBLE, flag(false));
    put(entity, DISASSEMBLE, flag(false));
}

fn tick_pulley(host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
    let pos = event.pos;
    let Some(facing) = util::facing(&*host, pos, "facing") else {
        return;
    };
    let node = Node {
        impact: pulley_rules::IMPACT,
        ..Node::passive(Kind::shafts(&axis_ends(rotation_axis(facing))))
    };
    let previous = kinetic::speed(event.entity);
    let Some((rpm, changed)) = kinetic::tick(host, pos, event.entity, node) else {
        return; // The network jammed and the block broke.
    };
    let now = util::game_time(&*host);
    let dimension = host.dimension().to_owned();
    let entity = &mut *event.entity;
    let mut state = saved(entity);
    let manual = state.assemble == i32::from(engine::ASKED);

    if !state.running {
        if state.assemble != 0 {
            put(entity, ASSEMBLE, flag(false));
            // `ElevatorPulleyBlockEntity.assemble` needs the pulley turning.
            if rpm != 0.0 {
                assemble(host, pos, entity, now, manual);
            }
        }
        return;
    }

    let key = (dimension.clone(), state.column);
    // A cabin held by an actor or an interface (`Contraption.stalled`) doesn't move.
    let held_now = psi_motion::is_stalled(&entity.nbt);
    let site = |cell: &Snapshot| elevator::replay_as(*cell, previous, now - cell.time, held_now);
    // The column's call: where the cabin is sent.
    let wanted = lock(&COLUMNS)
        .get(&key)
        .and_then(Column::targeted)
        .filter(|y| !elevator::target_unreachable(*y, state.min_contact_y, state.max_contact_y))
        .map(|y| elevator::target_offset(pos[1], y, state.contact_y));
    let target = wanted.unwrap_or(state.snapshot.target);
    if changed || target != state.snapshot.target {
        // A new target or speed: the cabin up to now at the old ones, then from here.
        let (offset, speed, _) = site(&state.snapshot);
        let snapshot = Snapshot { offset, speed, time: now, target };
        util::set_float(&mut entity.nbt, OFFSET, snapshot.offset);
        util::set_float(&mut entity.nbt, SPEED, snapshot.speed);
        entity.nbt.insert(SYNC_TIME.into(), Tag::Long(now));
        entity.nbt.insert(TARGET.into(), Tag::Int(target));
        state.snapshot = snapshot;
        host.block_entity_sync(pos);
    }

    // A click on a running elevator sets the cabin down.
    let (offset, _, _) = elevator::replay_as(state.snapshot, rpm, now - state.snapshot.time, held_now);
    if util::boolean(&entity.nbt, DISASSEMBLE) {
        set_down(host, pos, &entity.nbt.clone(), offset, state.initial, state.column);
        stop(entity, now);
        host.block_entity_sync(pos);
        return;
    }
    // `tickActors` and the Portable Interfaces: the drills, saws... on the cabin may hold it
    // still. The snapshot is moved to now when that changes.
    if let Some(live) = engine::live(&dimension, pos, &entity.nbt) {
        let snapshot = state.snapshot;
        let (anchor, initial) = (live.moving.anchor, state.initial);
        // `toPosition`, with the cabin's offset of the game time (replayed once a tick).
        let memo: Cell<Option<(f64, [f64; 3])>> = Cell::new(None);
        let origin = |t: f64| {
            if let Some((at, out)) = memo.get() {
                if at == t {
                    return out;
                }
            }
            let whole = t.floor();
            let (cabin, speed, _) = elevator::replay_as(snapshot, rpm, whole as i64 - snapshot.time, held_now);
            let cabin = if held_now { cabin } else { cabin + speed * (t - whole) as f32 };
            let out = [
                f64::from(anchor[0]),
                f64::from(anchor[1]) + f64::from(pulley_rules::rise(initial, cabin)),
                f64::from(anchor[2]),
            ];
            memo.set(Some((t, out)));
            out
        };
        let pose = psi_motion::Pose::Translating { origin: &origin };
        let acted = actor::tick(host, pos, entity, &pose, now, Carrier::Controlled, Structure::Elevator);
        let want = psi_motion::tick(host, pos, &live.moving, &pose, now) | acted;
        if want != held_now {
            let (at_now, speed, _) = elevator::replay_as(snapshot, rpm, now - snapshot.time, held_now);
            let moved = Snapshot { offset: at_now, speed, time: now, target: snapshot.target };
            util::set_float(&mut entity.nbt, OFFSET, moved.offset);
            util::set_float(&mut entity.nbt, SPEED, moved.speed);
            entity.nbt.insert(SYNC_TIME.into(), Tag::Long(now));
            psi_motion::set_stalled(entity, want);
            state.snapshot = moved;
            host.block_entity_sync(pos);
        }
    }
    let held_after = psi_motion::is_stalled(&entity.nbt);
    let (_, _, arrived) = elevator::replay_as(state.snapshot, rpm, now - state.snapshot.time, held_after);
    // `triggerContact`: the contact at the floor the cabin has reached powers.
    let floor = state.max_contact_y - (target);
    let reached = if arrived { Some(floor) } else { None };
    let mut arrivals = lock(&ARRIVED);
    match reached {
        Some(y) => {
            if arrivals.insert(key.clone(), y) != Some(y) {
                drop(arrivals);
                host.play_sound("minecraft:block.piston.contract", util::centre(below(pos, offset as i32)), 4, 0.75, 0.8);
            }
        }
        None => {
            arrivals.remove(&key);
        }
    }
}

impl Hooks for Elevators {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        for model in [COIL, BELT, BELT_HALF, MAGNET] {
            event.models.push(model.to_owned());
        }
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        match event.block {
            PULLEY => Some((PULLEY.to_owned(), new_pulley_entity())),
            CONTACT => {
                let mut nbt = Compound::new();
                nbt.insert(SHORT_NAME.into(), Tag::String("".into()));
                nbt.insert(LONG_NAME.into(), Tag::String("".into()));
                Some((CONTACT.to_owned(), nbt))
            }
            _ => None,
        }
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == PULLEY || event.block == CONTACT {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        match event.entity.id.as_str() {
            PULLEY => tick_pulley(host, event),
            CONTACT => tick_contact(host, event),
            _ => {}
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        let entity = event.entity;
        if entity.id != PULLEY {
            return None;
        }
        let state = saved(entity);
        let mut tag = kinetic::update_tag(entity);
        tag.insert(RUNNING.into(), flag(state.running));
        util::set_float(&mut tag, OFFSET, state.snapshot.offset);
        util::set_float(&mut tag, SPEED, state.snapshot.speed);
        tag.insert(SYNC_TIME.into(), Tag::Long(state.snapshot.time));
        tag.insert(TARGET.into(), Tag::Int(state.snapshot.target));
        tag.insert(INITIAL_OFFSET.into(), Tag::Int(state.initial));
        let line = match util::get(&entity.nbt, LAST_EXCEPTION) {
            Some(Tag::String(s)) => s.to_string_lossy(),
            _ => String::new(),
        };
        tag.insert(LAST_EXCEPTION.into(), Tag::String(line.as_str().into()));
        tag.insert(CONTRAPTION.into(), engine::client_contraption(entity));
        tag.insert(psi_motion::STALLED.into(), psi_motion::tag_value(&entity.nbt));
        Some(tag)
    }

    /// `ElevatorPulleyRenderer`: the shaft, the coil, the magnet and the belt under it, and the cabin.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != PULLEY {
            return;
        }
        let Some(facing) = host.block_state_property(event.state, "facing").and_then(Dir::from_name) else {
            return;
        };
        let rpm = kinetic::speed(event.entity);
        let state = saved(event.entity);
        let draw = &mut *event.draw;
        kinetic::draw_shaft(host, draw, rotation_axis(facing), rpm);
        // The block's own turn about Y: south 0, west 90, north 180, east 270 (`horizontalAngle`).
        let angle = match facing {
            Dir::South => 0.0,
            Dir::West => 90.0,
            Dir::North => 180.0,
            _ => 270.0,
        };
        draw.push();
        draw.rotate_about([0.0, 1.0, 0.0], -angle, [0.5; 3]);
        host.draw_model(draw, COIL, 0xFF_FFFF);
        draw.pop();
        let now = draw.game_time as f64 + f64::from(draw.partial);
        // A held cabin (an actor or an interface stalls it) stands still; its speed, for the
        // heads of the actors on it, is what it has built up.
        let held = psi_motion::is_stalled(&event.entity.nbt);
        let mut cabin_speed = 0.0;
        let offset = if state.running {
            let whole = now.floor();
            let (offset, speed, _) =
                elevator::replay_as(state.snapshot, rpm, whole as i64 - state.snapshot.time, held);
            cabin_speed = speed;
            if held { offset } else { offset + speed * (now - whole) as f32 }
        } else {
            0.0
        };
        let turned = |draw: &mut crate::compat::BlockEntityDraw, y: f32, model: &str| {
            draw.push();
            draw.translate([0.0, -y, 0.0]);
            draw.rotate_about([0.0, 1.0, 0.0], -(180.0 + angle), [0.5; 3]);
            host.draw_model(draw, model, 0xFF_FFFF);
            draw.pop();
        };
        if state.running || offset == 0.0 {
            turned(draw, offset, MAGNET);
        }
        if offset != 0.0 {
            let f = offset % 1.0;
            if f < 0.25 || f > 0.75 {
                turned(draw, if f > 0.75 { f - 1.0 } else { f }, BELT_HALF);
            }
            if state.running {
                let mut i = 0;
                while (i as f32) < offset - 0.25 {
                    turned(draw, offset - i as f32, BELT);
                    i += 1;
                }
            }
        }
        let Some(parsed) = engine::parsed(event.pos, event.entity) else {
            if !state.running {
                draw.keep();
            }
            return;
        };
        let anchor = parsed.moving.anchor;
        let base = [
            (anchor[0] - event.pos[0]) as f32,
            (anchor[1] - event.pos[1]) as f32 + pulley_rules::rise(state.initial, offset),
            (anchor[2] - event.pos[2]) as f32,
        ];
        // How it moves, for the actors' heads.
        actor_draw::set_motion(if held {
            actor_draw::Motion::Stalled
        } else {
            actor_draw::Motion::Slide(cabin_speed.abs())
        });
        engine::draw_blocks(host, draw, &parsed, base);
    }

    fn block_entity_render_bounding_box(&self, _: &dyn Host, event: &mut BlockEntityRenderBoundingBoxEvent<'_>) {
        if event.block != PULLEY {
            return;
        }
        let state = saved(event.entity);
        if !state.running {
            return;
        }
        let extent = engine::parsed(event.pos, event.entity).map_or(0, |p| {
            let e = p.extent;
            (0..6).map(|i| e[i].abs()).max().unwrap_or(0)
        }) as f32;
        // The whole shaft down to the lowest floor the cabin can reach.
        let reach = (state.max_contact_y - state.min_contact_y).max(state.initial) as f32 + 3.0;
        event.bounds[1] = event.bounds[1].min(-reach - extent);
        for i in [0, 2] {
            event.bounds[i] = event.bounds[i].min(-extent - 1.0);
            event.bounds[i + 3] = event.bounds[i + 3].max(extent + 2.0);
        }
    }

    /// `HorizontalKineticBlock.getStateForPlacement` and `WrenchableDirectionalBlock`'s: the pulley
    /// faces the player; the contact faces the side clicked, away from the wall.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        let item: &str = &context.item;
        if item == PULLEY {
            let facing = create_kinetics::rules::saw::horizontal_direction(context.yaw).opposite();
            return level.state_with_property(event.state, "facing", facing.name()).map(Some);
        }
        if item == CONTACT {
            // `WrenchableDirectionalBlock`: the nearest looking direction.
            let facing = looking(context.yaw, context.pitch);
            return level.state_with_property(event.state, "facing", facing.name()).map(Some);
        }
        None
    }

    /// `ElevatorContactBlock.getSignal`: 15 from a powering contact into anything but a contact.
    fn block_signal(&self, level: &dyn LevelHost, event: &mut BlockSignalEvent) {
        let own = event.state;
        if level.state_with_property(own, "calling", "true").is_some() {
            if !event.direct && level.state_with_property(own, "powering", "true") == Some(own) {
                let n = util::dir(event.face).normal();
                let reader = level.block_state([event.pos.x + n[0], event.pos.y + n[1], event.pos.z + n[2]]);
                let reader_is_contact = level.state_with_property(reader, "calling", "true").is_some();
                event.signal = i32::from(elevator::signal(true, reader_is_contact));
            }
        }
        signal::mask(event);
    }

    /// `ElevatorContactBlock.isSignalSource`: while it powers.
    fn is_signal_source(&self, level: &dyn LevelHost, event: &mut IsSignalSourceEvent) {
        let own = event.state;
        if level.state_with_property(own, "calling", "true").is_some()
            && level.state_with_property(own, "powering", "true") == Some(own)
        {
            event.source = true;
        }
    }

    /// The Wrench turns a pulley or a contact; a running pulley is set down first.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let block = host.block(event.pos);
        if &*event.stack.item != util::WRENCH || (block != PULLEY && block != CONTACT) {
            return Flow::Continue;
        }
        let (pos, clicked) = (event.pos, Dir::from_normal(event.face));
        util::wrench(host, pos, |host| {
            let Some(clicked) = clicked else { return };
            let Some(facing) = util::facing(host, pos, "facing") else { return };
            if block == PULLEY {
                let mut running = false;
                host.change_mod_block_entity(pos, &mut |entity| {
                    running = util::boolean(&entity.nbt, RUNNING);
                    if running {
                        put(entity, DISASSEMBLE, flag(true));
                    }
                });
                if running {
                    return;
                }
                // `HorizontalKineticBlock`: a quarter turn about the face, kept horizontal.
                let next = facing.clockwise_about(Axis::Y);
                host.set_block_property(pos, "facing", next.name());
                return;
            }
            // `WrenchableDirectionalBlock.getRotatedBlockState`.
            let next = if facing.axis() == clicked.axis() { facing } else { facing.clockwise_about(clicked.axis()) };
            if next != facing {
                host.set_block_property(pos, "facing", next.name());
            }
        })
    }

    /// `ElevatorPulleyBlock.useItemOn`: an empty hand starts the cabin or sets it down.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != PULLEY {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.sneaking || player.game_mode == util::SPECTATOR || player.game_mode == 2 {
            return Flow::Continue;
        }
        if host.held_stack(player.uuid).is_some_and(|stack| stack.count > 0) {
            return Flow::Continue;
        }
        if !util::first_click(&*host, player.uuid, pos) {
            return Flow::Cancel;
        }
        host.change_mod_block_entity(pos, &mut |entity| {
            if util::boolean(&entity.nbt, RUNNING) {
                put(entity, DISASSEMBLE, flag(true));
            } else {
                entity.nbt.insert(ASSEMBLE.into(), Tag::Byte(engine::ASKED));
            }
        });
        Flow::Cancel
    }

    /// A pulley broken while its cabin runs sets it down first (`remove`).
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != PULLEY {
            return Flow::Continue;
        }
        let Some(entity) = host.mod_block_entity(pos) else {
            return Flow::Continue;
        };
        let state = saved(&entity);
        if !state.running {
            return Flow::Continue;
        }
        let now = util::game_time(&*host);
        let rpm = kinetic::speed(&entity);
        let held = psi_motion::is_stalled(&entity.nbt);
        let (offset, _, _) = elevator::replay_as(state.snapshot, rpm, now - state.snapshot.time, held);
        set_down(host, pos, &entity.nbt, offset, state.initial, state.column);
        Flow::Continue
    }
}
