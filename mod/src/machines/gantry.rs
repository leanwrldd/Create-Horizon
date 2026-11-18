//! The Gantry Shaft and the Gantry Carriage (`GantryShaftBlock`, `GantryShaftBlockEntity`,
//! `GantryCarriageBlock`, `GantryCarriageBlockEntity`, `GantryContraption`,
//! `GantryContraptionEntity`) on the contraption engine (`contraption.rs`, where the design and
//! the differences common to every contraption are written; the rules are
//! `create_kinetics::rules::gantry`, whose header lists what differs from Create).
//!
//! A carriage on a gantry shaft, with the shaft turning (and not powered), takes the blocks it
//! holds on to (what is in front of it, what is attached to those) and runs along the shaft at the
//! shaft's speed, until the shaft ends, something is in the way, the shaft turns off, or
//! redstone powers it. A powered gantry shaft is the other way round: the carriage turns it.
//!
//! The structure is kept in the block entity of the gantry shaft the carriage started on (the
//! controller): `Running`, `Offset` (blocks along the shaft's facing, a snapshot taken at
//! `SyncTime`), `Facing` (the carriage's) and `Contraption`, as the piston keeps its own.
//!
//! Actors on the structure (`actor.rs`) and the Portable Interfaces run on it as on a piston's:
//! the controller gives them the structure's `Pose` (it slides along the shaft's facing) and holds
//! the structure still while one stalls it (`Stalled`; the shaft under it is still checked at the
//! shaft's own speed, `checkPinionShaft`). A carriage that turns round stops its actors
//! (`GantryContraptionEntity.tickContraption`), and setting the structure down stops them first.
//! The drills' heads turn with the structure's speed.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use create_kinetics::rules::contraption::{self as rules, Transform, add, relative};
use create_kinetics::rules::drill as drill_rules;
use create_kinetics::rules::gantry::{self as gantry, Part, ShaftState};
use create_kinetics::rules::piston as piston_rules;
use create_kinetics::{Dir, Kind, Node, Pos, looking};
use crate::compat::{
    BlockEntityEvent, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent, BlockEntityTickEvent,
    BlockEntityTicksEvent, BlockEntityUpdateTagEvent, BlockStateId, Compound, DestroyBlockEvent, Flow,
    Hooks, Host, LevelHost, LevelTicksHost, ModBlockEntity, RightClickBlockEvent, StateForPlacementEvent,
    Tag, UpdateShapeEvent, UseItemOnEvent,
};

use super::actor;
use super::actor_draw;
use super::contraption::{
    self as engine, ASSEMBLE, CONTRAPTION, HostWorld, LAST_EXCEPTION, RUNNING, SYNC_TIME,
};
use super::portable_interface_motion as psi_motion;
use create_kinetics::rules::actor::{Carrier, Structure, speed_reversed};
use crate::{kinetic, util};

const SHAFT: &str = gantry::SHAFT;
const CARRIAGE: &str = gantry::CARRIAGE;
const OFFSET: &str = "Offset";
/// The carriage's facing, `GantryContraption.facing` (`Direction.get3DDataValue`).
const FACING: &str = "Facing";
/// The longest line of shafts read for power.
const LINE_LIMIT: usize = 64;

struct Gantries;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Gantries)
}

fn flag(value: bool) -> Tag {
    Tag::Byte(i8::from(value))
}

fn put(entity: &mut ModBlockEntity, key: &str, value: Tag) {
    if util::differs(&entity.nbt, key, &value) {
        entity.nbt.insert(key.into(), value);
    }
}

/// Where the structure is at game time `t` and a tick before (`solid`): the offset along the shaft's
/// facing follows its snapshot and the pinion's speed, as the tick and the drawing do.
pub(crate) fn solid_sample(
    host: &dyn Host,
    pos: Pos,
    entity: &ModBlockEntity,
    anchor: Pos,
    t: f64,
) -> Option<crate::solid::Sample> {
    if entity.id != SHAFT {
        return None;
    }
    let state = saved(entity);
    if !state.running {
        return None;
    }
    let facing = util::facing(host, pos, "facing")?;
    let speed = gantry::pinion_speed(psi_motion::moving_rpm(entity, kinetic::speed(entity)));
    let at = |t: f64| piston_rules::position(anchor, facing, gantry::offset_at(state.offset, state.time, speed, t));
    Some(crate::solid::Sample::Slide { now: at(t), prev: at(t - 1.0) })
}

fn dir_of_index(index: i32) -> Dir {
    usize::try_from(index).ok().and_then(|i| Dir::ALL.get(i).copied()).unwrap_or(Dir::North)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

// --- reading the blocks ------------------------------------------------------------------------------

/// The gantry shaft at `pos`: its facing, part and whether it is powered.
fn shaft_at(host: &dyn Host, pos: Pos) -> Option<(Dir, Part, bool)> {
    if host.block(pos) != SHAFT {
        return None;
    }
    let facing = util::facing(host, pos, "facing")?;
    let part = Part::from_name(&host.block_property(pos, "part")?)?;
    let powered = host.block_property(pos, "powered").as_deref() == Some("true");
    Some((facing, part, powered))
}

/// A gantry shaft's facing from a state id (the `part` property says it is one).
fn shaft_facing(level: &dyn LevelHost, id: BlockStateId) -> Option<Dir> {
    level.state_with_property(id, "part", "single")?;
    Dir::ALL
        .into_iter()
        .find(|d| level.state_with_property(id, "facing", d.name()) == Some(id))
}

fn shaft_part(level: &dyn LevelHost, id: BlockStateId) -> Option<Part> {
    [Part::Start, Part::Middle, Part::End, Part::Single]
        .into_iter()
        .find(|p| level.state_with_property(id, "part", p.name()) == Some(id))
}

// --- redstone (`neighborChanged`) -------------------------------------------------------------------------

/// Each shaft's last own signal: the line is read only when it changes.
static LAST_SIGNAL: LazyLock<Mutex<HashMap<(String, Pos), bool>>> = LazyLock::new(Mutex::default);

/// `GantryShaftBlock.neighborChanged`, polled: when this shaft's own signal changed, or a powered
/// neighbour of its line should power it, the line is read and the shafts that change are set.
fn poll_power(host: &mut dyn Host, pos: Pos, facing: Dir, powered: bool) {
    let dimension = host.dimension().to_owned();
    let signal = host.neighbor_signal(&dimension, pos) > 0;
    let key = (dimension.clone(), pos);
    let changed = {
        let mut last = lock(&LAST_SIGNAL);
        if last.len() > 4096 {
            last.clear();
        }
        last.insert(key, signal) != Some(signal)
    };
    let neighbour_powered = facing.axis_dirs().iter().any(|d| {
        shaft_at(&*host, relative(pos, *d, 1)).is_some_and(|(f, _, p)| f == facing && p)
    });
    // A powered shaft with no signal of its own is looked at now and then: whatever powered it
    // may be gone.
    let now = util::game_time(&*host);
    let walk = changed
        || (!powered && !signal && neighbour_powered)
        || (powered && !signal && now.rem_euclid(20) == 0);
    if !walk {
        return;
    }
    let mut line: HashMap<Pos, ShaftState> = HashMap::new();
    line.insert(pos, ShaftState { facing, powered });
    for d in facing.axis_dirs() {
        let mut current = relative(pos, d, 1);
        for _ in 0..LINE_LIMIT {
            match shaft_at(&*host, current) {
                Some((f, _, p)) if f == facing => {
                    line.insert(current, ShaftState { facing: f, powered: p });
                    current = relative(current, d, 1);
                }
                _ => break,
            }
        }
    }
    let mut signals: HashMap<Pos, bool> = HashMap::new();
    for cell in line.keys().copied().collect::<Vec<_>>() {
        signals.insert(cell, cell == pos && signal || (cell != pos && host.neighbor_signal(&dimension, cell) > 0));
    }
    let changes = gantry::power_changes(
        ShaftState { facing, powered },
        pos,
        &|p| line.get(&p).copied(),
        &|p| signals.get(&p).copied().unwrap_or(false),
    );
    for (cell, value) in changes {
        host.set_block_property(cell, "powered", if value { "true" } else { "false" });
    }
}

// --- the structure ----------------------------------------------------------------------------------------

struct Saved {
    running: bool,
    offset: f32,
    time: i64,
    assemble: i32,
    carriage: Dir,
}

fn saved(entity: &ModBlockEntity) -> Saved {
    Saved {
        running: util::boolean(&entity.nbt, RUNNING),
        offset: util::float(&entity.nbt, OFFSET).unwrap_or(0.0),
        time: util::long(&entity.nbt, SYNC_TIME),
        assemble: util::int(&entity.nbt, ASSEMBLE),
        carriage: dir_of_index(util::int(&entity.nbt, FACING)),
    }
}

fn new_shaft_entity() -> Compound {
    let mut nbt = kinetic::new_entity();
    nbt.insert(RUNNING.into(), flag(false));
    nbt.insert(ASSEMBLE.into(), flag(false));
    nbt.insert(FACING.into(), Tag::Int(0));
    nbt.insert(SYNC_TIME.into(), Tag::Long(0));
    nbt.insert(LAST_EXCEPTION.into(), Tag::String("".into()));
    util::set_float(&mut nbt, OFFSET, 0.0);
    nbt
}

/// The shaft a carriage at the structure's anchor `anchor`, `cell` blocks along from where it
/// started, rides on: behind the carriage.
fn shaft_cell(anchor: Pos, dir: Dir, cell: i32, carriage: Dir) -> Pos {
    relative(relative(anchor, dir, cell), carriage, -1)
}

/// `GantryCarriageBlockEntity.tryAssemble` for the carriages beside the shaft at `pos`. Returns
/// whether one was taken.
fn try_assemble(
    host: &mut dyn Host,
    pos: Pos,
    entity: &mut ModBlockEntity,
    facing: Dir,
    part: Part,
    rpm: f32,
    now: i64,
    manual: bool,
) -> bool {
    let dimension = host.dimension().to_owned();
    let speed = gantry::pinion_speed(rpm);
    if !gantry::can_assemble_on(false, part, speed) {
        return false;
    }
    for d in Dir::ALL {
        if d.axis() == facing.axis() {
            continue;
        }
        let carriage = relative(pos, d, 1);
        if host.block(carriage) != CARRIAGE || util::facing(&*host, carriage, "facing") != Some(d) {
            continue;
        }
        engine::prepare(host);
        let checked = {
            let Some(world) = HostWorld::new(&*host) else {
                return false;
            };
            gantry::assemble(&world, carriage, d).map(|found| {
                // `isCollidingWithWorld` for the first step.
                let movement = if speed > 0.0 { facing } else { facing.opposite() };
                let mut states = std::collections::BTreeMap::new();
                for (p, state) in &found.blocks {
                    states.insert(rules::sub(*p, carriage), state.clone());
                }
                let first = relative(carriage, movement, 1);
                let blocked = rules::colliders(&states, movement, &engine::solid)
                    .iter()
                    .any(|c| rules::blocked_by(&world, add(first, *c)));
                (found, blocked)
            })
        };
        let (found, blocked) = match checked {
            Ok(done) => done,
            Err(fail) => {
                let changed = engine::store_exception(entity, Some(&fail));
                if manual {
                    engine::tell_near(host, pos, &fail);
                }
                if changed {
                    host.block_entity_sync(pos);
                }
                continue;
            }
        };
        if blocked || found.blocks.is_empty() {
            continue;
        }
        let moving = engine::capture(host, carriage, &found.blocks, &[], &|p| rules::sub(p, carriage));
        engine::remove_from_world(host, &moving, &|local| add(local, carriage), &mut |_, _, _| false);
        engine::set_live(&dimension, pos, moving.clone());
        entity.nbt.insert(CONTRAPTION.into(), Tag::Compound(moving.to_nbt(false)));
        entity.nbt.insert(RUNNING.into(), flag(true));
        entity.nbt.insert(FACING.into(), Tag::Int(Dir::ALL.iter().position(|x| *x == d).unwrap_or(0) as i32));
        util::set_float(&mut entity.nbt, OFFSET, 0.0);
        entity.nbt.insert(SYNC_TIME.into(), Tag::Long(now));
        engine::store_exception(entity, None);
        host.block_entity_sync(pos);
        return true;
    }
    false
}

/// Sets the structure of the shaft at `pos` down `cell` blocks along from where it started.
fn set_down(host: &mut dyn Host, pos: Pos, nbt: &Compound, facing: Dir, cell: i32) {
    let dimension = host.dimension().to_owned();
    // `Contraption.stop`: the actors on the structure stop first.
    actor::stop(host, pos, nbt);
    if let Some(live) = engine::live(&dimension, pos, nbt) {
        let transform = Transform::translation(relative(live.moving.anchor, facing, cell));
        engine::place_in_world(host, &live.moving, &transform, &mut |_, _, _| false);
    }
    engine::forget_live(&dimension, pos);
}

fn stop(entity: &mut ModBlockEntity, now: i64) {
    entity.nbt.insert(RUNNING.into(), flag(false));
    entity.nbt.remove(&psi_motion::STALLED.into());
    entity.nbt.remove(&CONTRAPTION.into());
    util::set_float(&mut entity.nbt, OFFSET, 0.0);
    entity.nbt.insert(SYNC_TIME.into(), Tag::Long(now));
    put(entity, ASSEMBLE, flag(false));
}

/// `GantryContraptionEntity.checkPinionShaft`, once a tick: where the structure is and whether
/// the shaft under it lets it go on. The shaft's own speed decides that (`rpm`, even while an
/// actor holds the structure still); the structure follows `held`, the pinion speed it moves at
/// (none while it is held). Returns the cell to set it down in when it has to stop.
fn check_shaft(
    host: &mut dyn Host,
    pos: Pos,
    entity: &ModBlockEntity,
    facing: Dir,
    rpm: f32,
    held: f32,
    now: i64,
) -> Option<i32> {
    let state = saved(entity);
    let dimension = host.dimension().to_owned();
    let live = engine::live(&dimension, pos, &entity.nbt)?;
    let speed = gantry::pinion_speed(rpm);
    let offset = gantry::offset_at(state.offset, state.time, held, now as f64);
    let cell = gantry::grid_offset(offset);
    let anchor = live.moving.anchor;
    // The shaft under the carriage is gone, powered, or nothing turns it.
    match shaft_at(&*host, shaft_cell(anchor, facing, cell, state.carriage)) {
        Some((_, _, false)) if speed != 0.0 => {}
        _ => return Some(cell),
    }
    // The crossing test: the shaft at the cell it is about to leave must let it on.
    if let Some(k) = gantry::shaft_to_check(offset, speed) {
        match shaft_at(&*host, shaft_cell(anchor, facing, k, state.carriage)) {
            Some((_, part, powered)) if gantry::can_assemble_on(powered, part, speed) => {}
            _ => return Some(k),
        }
    }
    None
}

/// `ContraptionCollider.collideBlocks`, once a tick: whether something is in the structure's way
/// as it moves at `held` (a structure held still isn't moving into anything). Returns the cell
/// to set it down in when it has to stop.
fn check_collision(
    host: &mut dyn Host,
    pos: Pos,
    entity: &ModBlockEntity,
    facing: Dir,
    held: f32,
    now: i64,
) -> Option<i32> {
    if held == 0.0 {
        return None;
    }
    let state = saved(entity);
    let dimension = host.dimension().to_owned();
    let live = engine::live(&dimension, pos, &entity.nbt)?;
    let offset = gantry::offset_at(state.offset, state.time, held, now as f64);
    let cell = gantry::grid_offset(offset);
    let anchor = live.moving.anchor;
    let movement = if held > 0.0 { facing } else { facing.opposite() };
    let next = gantry::offset_at(state.offset, state.time, held, (now + 1) as f64);
    let at = piston_rules::collision_anchor(piston_rules::position(anchor, facing, next), movement);
    let colliders = live.colliders(movement);
    let blocked = HostWorld::new(&*host)
        .is_some_and(|world| colliders.iter().any(|c| rules::blocked_by(&world, add(*c, at))));
    blocked.then_some(cell)
}

/// `GantryContraptionEntity.tickContraption` after `checkPinionShaft`: the actors (`tickActors`)
/// and the Portable Interfaces on the structure, which may hold it still (`Contraption.stalled`:
/// `!isStalled()` is what lets it `move`), then the collision. `from` and `at` are the snapshot the
/// structure follows. Returns the cell to set it down in when something is in its way.
#[allow(clippy::too_many_arguments)]
fn tick_structure(
    host: &mut dyn Host,
    pos: Pos,
    entity: &mut ModBlockEntity,
    facing: Dir,
    rpm: f32,
    now: i64,
    from: f32,
    at: i64,
) -> Option<i32> {
    let dimension = host.dimension().to_owned();
    let live = engine::live(&dimension, pos, &entity.nbt)?;
    let held = psi_motion::is_stalled(&entity.nbt);
    let speed = gantry::pinion_speed(psi_motion::moving_rpm(entity, rpm));
    let anchor = live.moving.anchor;
    let origin = |t: f64| piston_rules::position(anchor, facing, gantry::offset_at(from, at, speed, t));
    let pose = psi_motion::Pose::Translating { origin: &origin };
    let acted = actor::tick(host, pos, entity, &pose, now, Carrier::Controlled, Structure::Other);
    let want = psi_motion::tick(host, pos, &live.moving, &pose, now) | acted;
    if want != held {
        // The snapshot moves to now, then the structure follows no speed (or its own again).
        let offset = gantry::offset_at(from, at, speed, now as f64);
        util::set_float(&mut entity.nbt, OFFSET, offset);
        entity.nbt.insert(SYNC_TIME.into(), Tag::Long(now));
        psi_motion::set_stalled(entity, want);
        host.block_entity_sync(pos);
    }
    let moved = gantry::pinion_speed(psi_motion::moving_rpm(entity, rpm));
    check_collision(host, pos, entity, facing, moved, now)
}

impl Hooks for Gantries {
    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        match event.block {
            SHAFT => Some((SHAFT.to_owned(), new_shaft_entity())),
            CARRIAGE => Some((CARRIAGE.to_owned(), kinetic::new_entity())),
            _ => None,
        }
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == SHAFT || event.block == CARRIAGE {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        match event.entity.id.as_str() {
            SHAFT => tick_shaft(host, event),
            CARRIAGE => tick_carriage(host, event),
            _ => {}
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        let entity = event.entity;
        match entity.id.as_str() {
            CARRIAGE => Some(kinetic::update_tag(entity)),
            SHAFT => {
                let state = saved(entity);
                let mut tag = kinetic::update_tag(entity);
                tag.insert(RUNNING.into(), flag(state.running));
                util::set_float(&mut tag, OFFSET, state.offset);
                tag.insert(SYNC_TIME.into(), Tag::Long(state.time));
                tag.insert(FACING.into(), Tag::Int(Dir::ALL.iter().position(|d| *d == state.carriage).unwrap_or(0) as i32));
                let line = match util::get(&entity.nbt, LAST_EXCEPTION) {
                    Some(Tag::String(s)) => s.to_string_lossy(),
                    _ => String::new(),
                };
                tag.insert(LAST_EXCEPTION.into(), Tag::String(line.as_str().into()));
                tag.insert(CONTRAPTION.into(), engine::client_contraption(entity));
                tag.insert(psi_motion::STALLED.into(), psi_motion::tag_value(&entity.nbt));
                Some(tag)
            }
            _ => None,
        }
    }

    /// The shaft the gantry shaft is (turning with its speed) and the structure it carries; the
    /// carriage's shaft.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        let rpm = kinetic::speed(event.entity);
        if event.block == CARRIAGE {
            let (Some(facing), along_first) = (
                host.block_state_property(event.state, "facing").and_then(Dir::from_name),
                host.block_state_property(event.state, "axis_along_first") == Some("true"),
            ) else {
                return;
            };
            kinetic::draw_shaft(host, &mut *event.draw, piston_rules::rotation_axis(facing, along_first), rpm);
            event.draw.keep();
            return;
        }
        if event.block != SHAFT {
            return;
        }
        let (Some(facing), Some(part), powered) = (
            host.block_state_property(event.state, "facing").and_then(Dir::from_name),
            host.block_state_property(event.state, "part"),
            host.block_state_property(event.state, "powered") == Some("true"),
        ) else {
            return;
        };
        let draw = &mut *event.draw;
        // The block's own model, turning about its axis (`KineticBlockEntityRenderer`).
        draw.spin(facing.axis().unit(), [0.5; 3], rpm * 0.3, 0.0);
        host.draw_block(
            draw,
            SHAFT,
            &[("facing", facing.name()), ("part", part), ("powered", if powered { "true" } else { "false" })],
        );
        draw.still();
        let state = saved(event.entity);
        let Some(parsed) = engine::parsed(event.pos, event.entity) else {
            draw.keep();
            return;
        };
        let now = draw.game_time as f64 + f64::from(draw.partial);
        // A held structure (an actor or an interface stalls it) follows no speed.
        let speed = gantry::pinion_speed(psi_motion::moving_rpm(event.entity, rpm));
        let offset = gantry::offset_at(state.offset, state.time, speed, now);
        let n = facing.normal();
        let anchor = parsed.moving.anchor;
        let base = [0, 1, 2].map(|i| (anchor[i] - event.pos[i]) as f32 + n[i] as f32 * offset);
        // How it moves, for the actors' heads.
        actor_draw::set_motion(if psi_motion::is_stalled(&event.entity.nbt) {
            actor_draw::Motion::Stalled
        } else {
            actor_draw::Motion::Slide(speed.abs())
        });
        engine::draw_blocks(host, draw, &parsed, base);
    }

    /// Where the structure draws: along the shaft, as far as a line of shafts goes.
    fn block_entity_render_bounding_box(&self, host: &dyn Host, event: &mut BlockEntityRenderBoundingBoxEvent<'_>) {
        if event.block != SHAFT {
            return;
        }
        let Some(parsed) = engine::parsed(event.pos, event.entity) else {
            return;
        };
        let Some(facing) = host.block_state_property(event.state, "facing").and_then(Dir::from_name) else {
            return;
        };
        let (e, n) = (parsed.extent, facing.normal());
        for i in 0..3 {
            let rel = parsed.moving.anchor[i] - event.pos[i];
            let travel = 64 * n[i].abs();
            event.bounds[i] = event.bounds[i].min((rel + e[i] - travel - 1) as f32);
            event.bounds[i + 3] = event.bounds[i + 3].max((rel + e[i + 3] + travel + 2) as f32);
        }
    }

    /// `GantryShaftBlock.getStateForPlacement` and `GantryCarriageBlock.getStateForPlacement`.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        let item: &str = &context.item;
        let pos = [context.pos.x, context.pos.y, context.pos.z];
        let read = |p: Pos| shaft_facing(level, level.block_state(p));
        if item == SHAFT {
            let base = drill_rules::placement_facing(looking(context.yaw, context.pitch), context.sneaking);
            let face = util::dir(context.face);
            let behind = read(relative(pos, base, -1));
            let clicked_on = read(relative(pos, face, -1));
            let facing = gantry::shaft_placement(base, context.sneaking, behind, clicked_on);
            return level.state_with_property(event.state, "facing", facing.name()).map(Some);
        }
        if item == CARRIAGE {
            let clicked = util::dir(context.face);
            let horizontal = create_kinetics::rules::saw::horizontal_direction(context.yaw);
            let (facing, along_first) = gantry::carriage_placement(clicked, horizontal, read(relative(pos, clicked, -1)));
            let state = level.state_with_property(event.state, "facing", facing.name())?;
            return level
                .state_with_property(state, "axis_along_first", if along_first { "true" } else { "false" })
                .map(Some);
        }
        None
    }

    /// `GantryShaftBlock.updateShape`: shafts side by side join into start, middle and end.
    fn update_shape(&self, level: &mut dyn LevelTicksHost, event: &mut UpdateShapeEvent) -> Option<BlockStateId> {
        let level: &dyn LevelHost = &*level;
        let own = event.state;
        let facing = shaft_facing(level, own)?;
        let part = shaft_part(level, own)?;
        let towards = util::dir(event.direction);
        let connect = shaft_facing(level, event.neighbour) == Some(facing);
        let new = gantry::update_part(part, facing, towards, connect);
        (new != part).then(|| level.state_with_property(own, "part", new.name())).flatten()
    }

    /// An empty hand on the carriage asks the shaft behind it to take it on
    /// (`GantryCarriageBlock.useItemOn`).
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != CARRIAGE {
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
        let Some(facing) = util::facing(&*host, pos, "facing") else {
            return Flow::Continue;
        };
        if !util::first_click(&*host, player.uuid, pos) {
            return Flow::Cancel;
        }
        let shaft = relative(pos, facing, -1);
        if host.block(shaft) == SHAFT {
            host.change_mod_block_entity(shaft, &mut |entity| {
                entity.nbt.insert(ASSEMBLE.into(), Tag::Byte(engine::ASKED));
            });
        }
        Flow::Cancel
    }

    /// The Wrench turns a carriage (`DirectionalAxisKineticBlock`) and a gantry shaft
    /// (`DirectionalKineticBlock`); a running gantry is set down first.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let block = host.block(event.pos);
        if &*event.stack.item != util::WRENCH || (block != CARRIAGE && block != SHAFT) {
            return Flow::Continue;
        }
        let (pos, clicked) = (event.pos, Dir::from_normal(event.face));
        util::wrench(host, pos, |host| {
            let Some(clicked) = clicked else { return };
            if block == SHAFT {
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
                let Some(facing) = util::facing(host, pos, "facing") else { return };
                let next = drill_rules::wrench_facing(facing, clicked);
                if next != facing {
                    host.set_block_property(pos, "facing", next.name());
                }
                return;
            }
            let Some(facing) = util::facing(host, pos, "facing") else { return };
            let along_first = host.block_property(pos, "axis_along_first").as_deref() == Some("true");
            let (next_facing, next_along) = piston_rules::wrench(facing, along_first, clicked);
            if next_facing != facing {
                host.set_block_property(pos, "facing", next_facing.name());
            }
            if next_along != along_first {
                host.set_block_property(pos, "axis_along_first", if next_along { "true" } else { "false" });
            }
        })
    }

    /// A gantry shaft broken while its structure runs sets the structure down first.
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != SHAFT {
            return Flow::Continue;
        }
        let Some(entity) = host.mod_block_entity(pos) else {
            return Flow::Continue;
        };
        let state = saved(&entity);
        if !state.running {
            return Flow::Continue;
        }
        let Some(facing) = util::facing(&*host, pos, "facing") else {
            return Flow::Continue;
        };
        let now = util::game_time(&*host);
        let speed = gantry::pinion_speed(psi_motion::moving_rpm(&entity, kinetic::speed(&entity)));
        let cell = gantry::grid_offset(gantry::offset_at(state.offset, state.time, speed, now as f64));
        set_down(host, pos, &entity.nbt, facing, cell);
        Flow::Continue
    }
}

/// A click of the Wrench on a running gantry: set the structure down at the next tick.
const DISASSEMBLE: &str = "Disassemble";

fn tick_carriage(host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
    let pos = event.pos;
    let (Some(facing), along_first) = (
        util::facing(&*host, pos, "facing"),
        host.block_property(pos, "axis_along_first").as_deref() == Some("true"),
    ) else {
        return;
    };
    let node = Node::passive(Kind::pinion(piston_rules::rotation_axis(facing, along_first), facing));
    let Some((_, changed)) = kinetic::tick(host, pos, event.entity, node) else {
        return; // The network jammed and the block broke.
    };
    if changed {
        host.block_entity_sync(pos);
    }
    // `GantryCarriageBlock.canSurvive`, polled: a carriage whose shaft is gone breaks.
    let behind = relative(pos, facing, -1);
    let dimension = host.dimension().to_owned();
    if host.chunk_is_loaded(&dimension, [behind[0] >> 4, behind[2] >> 4])
        && !gantry::carriage_can_survive(facing, shaft_at(&*host, behind).map(|(f, _, _)| f))
    {
        host.destroy_block(pos, None, false);
    }
}

fn tick_shaft(host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
    let pos = event.pos;
    let Some((facing, part, powered)) = shaft_at(&*host, pos) else {
        return;
    };
    poll_power(host, pos, facing, powered);
    // A powered shaft turns with the carriage beside it; an unpowered one is a plain shaft.
    let node = Node::passive(if powered { Kind::gantry_shaft(facing) } else { Kind::Shaft(facing.axis()) });
    let previous = kinetic::speed(event.entity);
    let Some((rpm, changed)) = kinetic::tick(host, pos, event.entity, node) else {
        return; // The network jammed and the block broke.
    };
    let now = util::game_time(&*host);
    let entity = &mut *event.entity;
    let mut state = saved(entity);
    let manual = state.assemble == i32::from(engine::ASKED);

    if changed {
        if state.running {
            // `GantryContraptionEntity.tickContraption`: a carriage that turns round (or stops)
            // stops its actors (`Contraption.stop`).
            if speed_reversed(gantry::pinion_speed(previous), gantry::pinion_speed(rpm)) {
                actor::restart(host, pos, &entity.nbt);
            }
            // `onSpeedChanged`: the structure follows the old speed up to now.
            let moved = gantry::pinion_speed(psi_motion::moving_rpm(entity, previous));
            let offset = gantry::offset_at(state.offset, state.time, moved, now as f64);
            util::set_float(&mut entity.nbt, OFFSET, offset);
            entity.nbt.insert(SYNC_TIME.into(), Tag::Long(now));
            state.offset = offset;
            state.time = now;
        } else if state.assemble == 0 {
            // `checkAttachedCarriageBlocks`: a change of speed asks the carriages to assemble.
            entity.nbt.insert(ASSEMBLE.into(), flag(true));
            state.assemble = 1;
        }
        host.block_entity_sync(pos);
    }

    if !state.running {
        if state.assemble != 0 {
            put(entity, ASSEMBLE, flag(false));
            try_assemble(host, pos, entity, facing, part, rpm, now, manual);
        }
        return;
    }

    // A click of the Wrench, or the structure's end.
    let held = gantry::pinion_speed(psi_motion::moving_rpm(entity, rpm));
    let cell_now = gantry::grid_offset(gantry::offset_at(state.offset, state.time, held, now as f64));
    let asked_to_stop = util::boolean(&entity.nbt, DISASSEMBLE);
    let mut stop_at = if asked_to_stop {
        Some(cell_now)
    } else {
        check_shaft(host, pos, entity, facing, rpm, held, now)
    };
    if stop_at.is_none() {
        stop_at = tick_structure(host, pos, entity, facing, rpm, now, state.offset, state.time);
    }
    if let Some(cell) = stop_at {
        set_down(host, pos, &entity.nbt.clone(), facing, cell);
        stop(entity, now);
        put(entity, DISASSEMBLE, flag(false));
        host.block_entity_sync(pos);
    }
}
