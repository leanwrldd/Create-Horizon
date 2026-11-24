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
//! - Elevator cabins' door control (`getElevatorDoorControl`) is not wired.
//! - `context.disabled` is read from the structure's saved disabled filters (the Contraption Controls' clicks); the start
//!   state the Controls give a freshly assembled structure isn't applied to a carriage.
//!
//! Type-checked and unit-tested only (in the rules): not played.

use create_kinetics::Pos;
use create_kinetics::rules::actor::is_disabled;
use create_kinetics::rules::contraption::State;
use create_kinetics::rules::trains::carriage_pose::CarriagePose;
use create_kinetics::rules::trains::train_doors::{
    DOOR_SOUND_VOLUME, DoorControl, bounds_centre, door_facing, is_sliding_door, should_open, tick_open,
    world_door_matches,
};

use crate::compat::{Hooks, Host};

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
