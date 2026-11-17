//! The moving half of the Portable Storage and Fluid Interfaces on a structure
//! (`PortableStorageInterfaceMovement`), and the one change this makes to the contraption engine:
//! a structure can be **held still**.
//!
//! # What the engine gains
//!
//! A controller (`piston.rs`, `bearing.rs`) saves a snapshot of where its structure is and follows
//! the speed from it (`contraption.rs`). Create stalls a structure (`Contraption.stalled`) while a
//! moving interface works with a stationary one; here that is the controller's saved `Stalled` flag,
//! and **a stalled structure's speed is zero**: [`moving_rpm`] is the rpm the offset or the angle
//! follows (the real rpm still decides assembling and setting down). Setting or clearing the flag
//! moves the snapshot to the present first (the controller does that with [`tick`]'s answer), so
//! nothing jumps; the flag is in the controller's update tag, so a client holds the drawing still
//! too. It is cleared when the structure is set down.
//!
//! # The moving interface
//!
//! Each tick the controller calls [`tick`] with a [`Pose`] (where the structure is at any game
//! time). For each interface block of the structure it works out where the block's probe is
//! (1.85 blocks in front of its centre, turned with the structure), finds a stationary interface
//! facing it in its cell or the next when that cell changes (`findInterface`), links with it
//! (`startTransferringTo`), stalls the structure when it comes to its closest and holds the link
//! (`keepAlive`) until the stationary interface's timer is down (nothing more moved for the
//! timeout), and goes on. The decisions are `rules::portable_interface`.
//!
//! Known differences from Create: only the piston and the bearing (and so the windmill) stall a
//! structure; a gantry, a rope pulley and an elevator don't, and an interface on one never links.
//! The moving interface's own extension toward the stationary one isn't animated (it is drawn
//! at rest). `Carriage` contraptions (trains) aren't in this port.

use std::collections::HashMap;
use std::sync::Mutex;

use create_kinetics::rules::portable_interface::{self as rules, Actor, KEEP_ALIVE, Stationary, TIMEOUT};
use create_kinetics::{Axis, Dir, Pos};
use horizon_client::mod_api::{BlockEntityDraw, Compound, Hooks, Host, ModBlockEntity, Tag};

use create_kinetics::rules::contraption::State;

use super::contraption::Moving;
use super::portable_interface as stationary;
use crate::util;

struct Shared;

impl Hooks for Shared {}

/// A module of shared code: no hooks of its own.
pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

/// Saved on a controller while an interface holds its structure still (Create's
/// `Contraption.stalled`).
pub const STALLED: &str = "Stalled";

/// Whether the controller's structure is held still.
pub fn is_stalled(nbt: &Compound) -> bool {
    util::boolean(nbt, STALLED)
}

/// The rpm a structure follows: none while it is held still.
pub fn moving_rpm(entity: &ModBlockEntity, rpm: f32) -> f32 {
    rules::moving_speed(is_stalled(&entity.nbt), rpm)
}

/// Holds the structure still, or lets it go (only a write when that changes anything). Whether it did.
pub fn set_stalled(entity: &mut ModBlockEntity, on: bool) -> bool {
    if is_stalled(&entity.nbt) == on {
        return false;
    }
    entity.nbt.insert(STALLED.into(), Tag::Byte(i8::from(on)));
    true
}

/// What the controller's update tag carries (always sent, so a client drops it when it goes).
pub fn tag_value(nbt: &Compound) -> Tag {
    Tag::Byte(i8::from(is_stalled(nbt)))
}

/// Where a structure is at a game time, for a controller.
pub enum Pose<'a> {
    /// A structure that slides: the world position of the anchor cell's minimum corner at game time `t`.
    Translating { origin: &'a dyn Fn(f64) -> [f64; 3] },
    /// A structure that turns right-handed about the axis through the anchor cell's centre by
    /// `angle(t)` degrees.
    Turning { anchor: Pos, axis: Axis, angle: &'a dyn Fn(f64) -> f64 },
}

impl Pose<'_> {
    /// The world position of the centre of the structure's block at `local` (relative to the anchor).
    pub(crate) fn centre(&self, t: f64, local: Pos) -> [f64; 3] {
        let l = local.map(f64::from);
        match self {
            Pose::Translating { origin } => {
                let o = origin(t);
                [o[0] + l[0] + 0.5, o[1] + l[1] + 0.5, o[2] + l[2] + 0.5]
            }
            Pose::Turning { anchor, axis, angle } => {
                let turned = rules::rotate_vec(l, *axis, angle(t));
                let a = rules::centre(*anchor);
                [a[0] + turned[0], a[1] + turned[1], a[2] + turned[2]]
            }
        }
    }

    /// How a vector fixed in the structure is turned at `t`.
    pub(crate) fn rotate(&self, t: f64, v: [f64; 3]) -> [f64; 3] {
        match self {
            Pose::Translating { .. } => v,
            Pose::Turning { axis, angle, .. } => rules::rotate_vec(v, *axis, angle(t)),
        }
    }

    /// The other way: a world vector in the structure's own frame (`reverseRotation`), for the
    /// actors (`actor.rs`).
    pub(crate) fn unrotate(&self, t: f64, v: [f64; 3]) -> [f64; 3] {
        match self {
            Pose::Translating { .. } => v,
            Pose::Turning { axis, angle, .. } => rules::rotate_vec(v, *axis, -angle(t)),
        }
    }
}

// --- each moving interface's memory -----------------------------------------------------------------------------

struct Memory {
    actor: Actor,
    /// The grid cell the probe was in last (`shouldActorTrigger`).
    cell: Option<Pos>,
    seen: i64,
}

type Key = (String, Pos, Pos);

static ACTORS: Mutex<Option<HashMap<Key, Memory>>> = Mutex::new(None);

fn with_actors<R>(f: impl FnOnce(&mut HashMap<Key, Memory>) -> R) -> R {
    let mut guard = ACTORS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    f(guard.get_or_insert_with(HashMap::new))
}

/// How long a moving interface nobody ticked stays in the table.
const STALE: i64 = 400;

/// `getStationaryInterfaceAt` as the rules want it: the interface of `block` at `pos`.
fn stationary_at(host: &dyn Host, pos: Pos, block: &str) -> Option<Stationary> {
    if host.block(pos) != block {
        return None;
    }
    let facing = host.block_property(pos, "facing").as_deref().and_then(Dir::from_name)?;
    let powered = stationary::peek(host.dimension(), pos).is_some_and(|link| link.powered);
    Some(Stationary { facing, powered })
}

fn floor(v: [f64; 3]) -> Pos {
    v.map(|c| c.floor() as i32)
}

/// One tick of every interface on the structure of the controller at `controller`, `now` being
/// the game time. Answers whether the structure is to be held still.
pub fn tick(host: &mut dyn Host, controller: Pos, moving: &Moving, pose: &Pose<'_>, now: i64) -> bool {
    let dimension = host.dimension();
    let mut any = false;
    let mut stalled = false;
    for block in &moving.blocks {
        if !stationary::is_ours(&block.state.name) {
            continue;
        }
        any = true;
        let Some(facing) = block.state.dir("facing") else { continue };
        stalled |= tick_one(host, dimension, controller, pose, now, block.local, &block.state, facing);
    }
    if any && now.rem_euclid(200) == 0 {
        with_actors(|actors| actors.retain(|_, memory| now - memory.seen < STALE));
    }
    stalled
}

#[allow(clippy::too_many_arguments)]
fn tick_one(
    host: &mut dyn Host,
    dimension: &'static str,
    controller: Pos,
    pose: &Pose<'_>,
    now: i64,
    local: Pos,
    state: &State,
    facing: Dir,
) -> bool {
    let t = now as f64;
    // `actorPosition`: the block's centre plus the active area (1.85 along its facing), turned.
    let normal = facing.normal().map(f64::from);
    let reach = normal.map(|c| c * rules::ACTIVE_AREA);
    let probe_at = |time: f64| {
        let centre = pose.centre(time, local);
        let offset = pose.rotate(time, reach);
        [centre[0] + offset[0], centre[1] + offset[1], centre[2] + offset[2]]
    };
    let position = probe_at(t);
    let ahead = probe_at(t + 1.0);
    let motion = [ahead[0] - position[0], ahead[1] - position[1], ahead[2] - position[2]];
    let current = rules::current_facing(pose.rotate(t, normal));

    let key: Key = (dimension.to_owned(), controller, local);
    let mut memory = with_actors(|actors| actors.remove(&key)).unwrap_or(Memory {
        actor: Actor::default(),
        cell: None,
        seen: now,
    });
    memory.seen = now;

    // `visitNewPosition`: a new grid cell is a new look for a stationary interface.
    let cell = floor(position);
    if memory.cell != Some(cell) {
        memory.cell = Some(cell);
        if !memory.actor.stall {
            let found = current.and_then(|face| {
                let host_ref: &dyn Host = &*host;
                rules::find_interface(cell, face, &|pos| stationary_at(host_ref, pos, &state.name))
            });
            memory.actor.visit(found);
            if let (Some(found), Some(face)) = (found, current) {
                // `findInterface`: link, reaching `distance` out.
                let distance = rules::link_distance(found, position, face);
                let link = stationary::with_link(dimension, found, now, |link| {
                    link.start(controller, distance);
                    *link
                });
                stationary::publish(host, found, &link, now);
            }
        }
    }

    let actor = &mut memory.actor;
    let Some(working) = actor.working else {
        if actor.stall {
            actor.reset();
        }
        let stall = actor.stall;
        with_actors(|actors| actors.insert(key, memory));
        return stall;
    };
    actor.arrive(position, motion, rules::centre(working));
    let Some(face) = current else {
        actor.reset();
        with_actors(|actors| actors.insert(key, memory));
        return false;
    };
    let here = stationary_at(&*host, working, &state.name);
    if !rules::links_with(here, face) {
        actor.reset();
        with_actors(|actors| actors.insert(key, memory));
        return false;
    }
    // `startTransferringTo` again when the link let go, then `keepAlive = 2` and the timer.
    let (link, started) = stationary::with_link(dimension, working, now, |link| {
        let started = link.controller.is_none() && link.start(controller, link.distance);
        link.keep_alive = KEEP_ALIVE;
        (*link, started)
    });
    if started {
        stationary::publish(host, working, &link, now);
    }
    actor.release(link.timer);
    let stall = actor.stall;
    with_actors(|actors| actors.insert(key, memory));
    let _ = TIMEOUT;
    stall
}

/// What a controller's `block_entity_submit` leaves out of a structure block that is an interface:
/// its middle and top, at rest (Create's renderer draws them as the block's own parts in the
/// structure).
pub fn draw_moving(host: &dyn Host, draw: &mut BlockEntityDraw, state: &State) {
    if !stationary::is_ours(&state.name) {
        return;
    }
    if let Some(facing) = state.dir("facing") {
        stationary::draw_parts(host, draw, &state.name, facing, false, 0.0);
    }
}
