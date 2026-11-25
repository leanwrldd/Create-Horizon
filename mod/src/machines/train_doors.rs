//! The doors of a carriage (`SlidingDoorMovementBehaviour`): the Train Doors in a carriage's structure open when the train
//! stands at a station whose door control (`DoorControlBehaviour`, `/station doors`) lets them, and close when it leaves.
//! The decisions are `create_kinetics::rules::trains::train_doors`; `trains.rs` calls [`tick`] for each carriage after its
//! anchor is put where its wheels are, and applies the structure it changes.
//!
//! Each door half is an actor of the structure: it keeps what Create keeps in the actor's data (`Open`, `firstMovement`) in
//! the carriage's runtime (not saved: after a load every door starts as if it had just been assembled, as a
//! `MovementContext` read back does). The click on a door in a carriage is `structure_click.rs`
//! (`Interaction::SlidingDoor`).
//!
//! Differences from Create, for the README:
//! - The doors swing (they are vanilla-style doors here), so there is no `SlidingDoorBlockEntity` animation: the open sound is
//!   played when the door opens, and the close sound when it closes, not when an animation settles.
//! - The door of the platform that the carriage's door opens onto (`db.setOpen(null, ..)`) is any vanilla-style door whose
//!   state has `open` and a `facing` along the same axis; both halves are set, and its own sound isn't played. Create's
//!   own sliding doors there also open their double-door neighbour (`SlidingDoorBlock.setOpen`): not done.
//! - An Elevator cabin's doors (`getElevatorDoorControl`) are [`tick_elevator`]: they open when the cabin has arrived, by the
//!   door control of the contact at the column's target floor (`/elevator doors <mode>` on the last Elevator Contact clicked).
//! - `context.disabled` is read from the structure's saved disabled filters (the Contraption Controls' clicks); the start
//!   state the Controls give a freshly assembled structure isn't applied to a carriage.
//!
//! Type-checked and unit-tested only (in the rules): not played.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use create_kinetics::Pos;
use create_kinetics::rules::actor::is_disabled;
use create_kinetics::rules::contraption::State;
use create_kinetics::rules::trains::carriage_pose::CarriagePose;
use create_kinetics::rules::trains::train_doors::{
    DOOR_SOUND_VOLUME, DoorControl, DoorData, bounds_centre, door_facing, is_sliding_door, should_open,
    should_open_in_elevator, tick_open, world_door_matches,
};

use crate::compat::{Hooks, Host, ModBlockEntity, Tag};

use super::contraption::{self as engine, CONTRAPTION};
use super::portable_interface_motion::Pose;
use super::trains::{CarriageRt, structure_pose};

struct Doors;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Doors)
}

impl Hooks for Doors {}

/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;

fn state_at(rt: &CarriageRt, local: Pos) -> Option<State> {
    rt.structure.blocks.iter().find(|b| b.local == local).map(|b| b.state.clone())
}

/// The door in the world that a carriage's door faces (`inWorldDoor`): the block next to the carriage's door, along its
/// facing. Both halves are set to `open`.
fn open_world_door(host: &mut dyn Host, centre: [f64; 3], facing: create_kinetics::Dir, open: bool) {
    let n = facing.normal();
    let cell = [centre[0].floor() as i32 + n[0], centre[1].floor() as i32 + n[1], centre[2].floor() as i32 + n[2]];
    let name = host.block(cell);
    if !name.ends_with("_door") {
        return;
    }
    let mut state = State::new(&name);
    for key in ["open", "facing", "half"] {
        if let Some(value) = host.block_property(cell, key) {
            state.set(key, &value);
        }
    }
    if !world_door_matches(&state, facing) || state.flag("open") == open {
        return;
    }
    let value = if open { "true" } else { "false" };
    host.set_block_property(cell, "open", value);
    let other = [cell[0], cell[1] + if state.get("half") == Some("upper") { -1 } else { 1 }, cell[2]];
    if host.block(other) == name {
        host.set_block_property(other, "open", value);
    }
}

/// A carriage's doors tick (`SlidingDoorMovementBehaviour.tick` on the server, for every Train Door half in the structure).
/// `speed` is the train's speed in blocks a tick while the carriage moves (0 while it stands), `mode` the door control of
/// the station the train is at. True when a door changed (the structure must be sent again).
pub fn tick(
    host: &mut dyn Host,
    dimension: &str,
    rt: &mut CarriageRt,
    pose: &CarriagePose,
    initial_yaw: f32,
    speed: f64,
    mode: Option<DoorControl>,
) -> bool {
    let _ = dimension;
    let doors: Vec<Pos> = rt.structure.blocks.iter().filter(|b| is_sliding_door(&b.state.name)).map(|b| b.local).collect();
    if doors.is_empty() {
        return false;
    }
    let centre = bounds_centre(rt.structure.blocks.iter().map(|b| b.local));
    let at = structure_pose(pose, initial_yaw);
    // `context.rotation.apply(v)`: a direction vector turned as the structure is.
    let origin = at.to_global_vector([0.5, 0.5, 0.5], 1.0, false);
    let rotate = |v: [f64; 3]| {
        let p = at.to_global_vector([0.5 + v[0], 0.5 + v[1], 0.5 + v[2]], 1.0, false);
        [p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]]
    };
    let disabled_filters = rt.structure.disabled.clone().unwrap_or_default();
    let mut changed = false;
    for local in doors {
        let Some(state) = state_at(rt, local) else { continue };
        let Some(state_facing) = state.dir("facing") else { continue };
        let facing = door_facing(state_facing, local, centre, &rotate);
        let disabled = is_disabled(&disabled_filters, &state.name);
        let wanted = should_open(disabled, speed, mode, facing);
        let other_at = [local[0], local[1] + if state.get("half") == Some("upper") { -1 } else { 1 }, local[2]];
        let other = state_at(rt, other_at);
        let data = rt.doors.entry(local).or_default();
        let toggle = tick_open(data, &state, local, other.as_ref(), wanted, facing);
        data.ticked();
        let Some(toggle) = toggle else { continue };
        for (pos, new) in &toggle.changes {
            if let Some(block) = rt.structure.blocks.iter_mut().find(|b| b.local == *pos) {
                block.state = new.clone();
            }
        }
        changed = true;
        let world = at.to_global_vector([f64::from(local[0]) + 0.5, f64::from(local[1]) + 0.5, f64::from(local[2]) + 0.5], 1.0, false);
        open_world_door(host, world, toggle.world_door.0, toggle.world_door.1);
        host.play_sound(toggle.sound, world, BLOCKS, DOOR_SOUND_VOLUME, 1.0);
    }
    changed
}

// --- the doors of an Elevator cabin ----------------------------------------------------------------------------------

/// What each cabin's door actors remember (`context.data.Open`, `firstMovement`), by dimension and controller. Not saved: a
/// reload starts every door afresh, as `startMoving` does.
static CABINS: LazyLock<Mutex<HashMap<(String, Pos), HashMap<Pos, DoorData>>>> = LazyLock::new(Mutex::default);

/// The saved key of `DoorControlBehaviour` (`NBTHelper.writeEnum(nbt, "DoorControl", mode)`).
pub const DOOR_CONTROL: &str = "DoorControl";

/// The door control a block entity saved (`ALL` until one is set).
pub fn saved_mode(entity: &ModBlockEntity) -> DoorControl {
    let saved = match crate::util::get(&entity.nbt, DOOR_CONTROL) {
        Some(Tag::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    };
    DoorControl::read(saved.as_deref())
}

/// The door control of the contact at `at`, `None` when there is no block entity there (`BlockEntityBehaviour.get` is null).
pub fn mode_at(host: &dyn Host, at: Pos) -> Option<DoorControl> {
    host.mod_block_entity(at).map(|e| saved_mode(&e))
}

/// A cabin's doors tick (`SlidingDoorMovementBehaviour.tick` on the server for each Train Door half of the cabin).
/// `speed` is the cabin's speed in blocks a tick, `stalled` whether an actor holds it, `arrived` `ElevatorContraption.arrived`
/// and `mode` the door control of the contact at the column's target floor (`train_doors::elevator_door_contact`). The structure is
/// written into `entity` when a door changed. True when one did.
#[allow(clippy::too_many_arguments)]
pub fn tick_elevator(
    host: &mut dyn Host,
    controller: Pos,
    entity: &mut ModBlockEntity,
    pose: &Pose<'_>,
    now: i64,
    speed: f64,
    stalled: bool,
    arrived: bool,
    mode: Option<DoorControl>,
) -> bool {
    let dimension = host.dimension().to_owned();
    let Some(live) = engine::live(&dimension, controller, &entity.nbt) else { return false };
    let doors: Vec<Pos> = live.moving.blocks.iter().filter(|b| is_sliding_door(&b.state.name)).map(|b| b.local).collect();
    if doors.is_empty() {
        CABINS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&(dimension, controller));
        return false;
    }
    let mut moving = live.moving.clone();
    let centre = bounds_centre(moving.blocks.iter().map(|b| b.local));
    let t = now as f64;
    let rotate = |v: [f64; 3]| pose.rotate(t, v);
    let disabled_filters = moving.disabled.clone().unwrap_or_default();
    let mut data = CABINS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&(dimension.clone(), controller))
        .unwrap_or_default();
    let mut changed = false;
    for local in doors {
        let Some(state) = state_of(&moving, local) else { continue };
        let Some(state_facing) = state.dir("facing") else { continue };
        let facing = door_facing(state_facing, local, centre, &rotate);
        let disabled = is_disabled(&disabled_filters, &state.name);
        let wanted = should_open_in_elevator(disabled, speed, stalled, arrived, mode, facing);
        let other_at = [local[0], local[1] + if state.get("half") == Some("upper") { -1 } else { 1 }, local[2]];
        let other = state_of(&moving, other_at);
        let door = data.entry(local).or_default();
        let toggle = tick_open(door, &state, local, other.as_ref(), wanted, facing);
        door.ticked();
        let Some(toggle) = toggle else { continue };
        for (pos, new) in &toggle.changes {
            if let Some(block) = moving.blocks.iter_mut().find(|b| b.local == *pos) {
                block.state = new.clone();
            }
        }
        changed = true;
        let world = pose.centre(t, local);
        open_world_door(host, world, toggle.world_door.0, toggle.world_door.1);
        host.play_sound(toggle.sound, world, BLOCKS, DOOR_SOUND_VOLUME, 1.0);
    }
    CABINS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert((dimension.clone(), controller), data);
    if changed {
        entity.nbt.insert(CONTRAPTION.into(), Tag::Compound(moving.to_nbt(false)));
        engine::set_live(&dimension, controller, moving);
    }
    changed
}

fn state_of(moving: &super::contraption::Moving, local: Pos) -> Option<State> {
    moving.blocks.iter().find(|b| b.local == local).map(|b| b.state.clone())
}

/// A cabin that was set down forgets its doors.
pub fn forget_elevator(dimension: &str, controller: Pos) {
    CABINS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&(dimension.to_owned(), controller));
}
