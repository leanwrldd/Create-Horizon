//! The client half: reporting each structure from the client's copy of its controller, and
//! colliding the local player with them (`ContraptionCollider.collideEntities` for
//! `PlayerType.CLIENT`, with `saveClientPlayerFromClipping`).
//!
//! The client moves its own player and the server doesn't predict it, so the player has to be
//! collided here. [`local_player_pass`] is called by the mixin `mixins/local_player_solid.rs` right
//! after the local player's simulation tick (Create runs it at the end of the level tick, after the
//! player has moved), with the player's state as plain numbers and the game's own block collision
//! for the part of Create's `collide` that keeps a pushed player out of the world's blocks.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use create_kinetics::Pos;
use create_kinetics::rules::collider::{
    ColliderFrame, Colliders, EntityBody, PlayerType, broad_phase, collide_entity, save_client_player_from_clipping,
};
use create_kinetics::rules::collider_obb::{Aabb, CollisionList};
use create_kinetics::rules::contraption::State;
use create_kinetics::rules::solid::ray_boxes;
use mod_api::{BlockEntityClientTickEvent, ClientHost, Compound, Tag};

use super::registry::{self, Entry, Side};
use super::{Sample, build_solid, shapes};
use crate::compat::Adapter;
use crate::machines::contraption::Moving;
use crate::util;

/// A controller's client tick: report where its structure is (at the game time the client draws
/// it) so the local player can be collided with it.
pub fn observe(host: &mut dyn ClientHost, event: &BlockEntityClientTickEvent<'_>) {
    let entity = event.entity;
    let id = entity.id();
    if !super::is_controller(id) {
        return;
    }
    let pos = event.pos;
    let dimension = host.dimension();
    let nbt = &entity.nbt;
    if !util::boolean(nbt, "Running") {
        registry::forget(Side::Client, dimension, pos);
        return;
    }
    let Some(contraption) = contraption_of(nbt) else {
        registry::forget(Side::Client, dimension, pos);
        return;
    };
    let Some(anchor) = anchor_of(contraption) else { return };
    let Some(now) = host.level(dimension).and_then(|level| level.game_time(&*host)) else { return };
    let light = super::light_copy(id, nbt);
    let sample = {
        let adapter = Adapter::Client(&*host);
        super::sample_of(&adapter, pos, &light, anchor, now as f64)
    };
    let Some(sample) = sample else {
        registry::forget(Side::Client, dimension, pos);
        return;
    };
    // The structure is read again when the block entity's data changed (the client keeps a
    // revision for each copy; an update tag that brings nothing new keeps it).
    let revision = entity.revision();
    let token = revision.identity().wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ revision.changes();
    let solid = match registry::token(Side::Client, dimension, pos) {
        Some((known, solid)) if known == token => solid,
        _ => {
            let Some(moving) = Moving::from_nbt(contraption) else { return };
            let shapes = palette_shapes(contraption);
            let turn_axis = match sample {
                Sample::Turn { axis, .. } => Some(axis),
                Sample::Slide { .. } => None,
            };
            Arc::new(build_solid(&moving, turn_axis, &|state: &State| match shapes.get(&shapes::text(state)) {
                Some(boxes) => boxes.clone(),
                None => shapes::boxes_of(state),
            }))
        }
    };
    registry::put(Side::Client, dimension, pos, Entry { solid, anchor, sample, seen: now, token });
    *DIMENSION.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = dimension.to_owned();
}

/// The dimension the client's structures are in.
static DIMENSION: Mutex<String> = Mutex::new(String::new());

fn contraption_of(nbt: &Compound) -> Option<&Compound> {
    match util::get(nbt, "Contraption")? {
        Tag::Compound(c) if !c.is_empty() => Some(c),
        _ => None,
    }
}

fn anchor_of(contraption: &Compound) -> Option<Pos> {
    match util::get(contraption, "Anchor")? {
        Tag::IntArray(v) => <Pos>::try_from(v.as_slice()).ok(),
        _ => None,
    }
}

/// The `Shape` each palette entry of the structure carries, by the state's text.
fn palette_shapes(contraption: &Compound) -> HashMap<String, Arc<[Aabb]>> {
    let mut out = HashMap::new();
    let Some(Tag::Compound(blocks)) = util::get(contraption, "Blocks") else { return out };
    let Some(Tag::List(palette)) = util::get(blocks, "Palette") else { return out };
    for entry in palette {
        let Tag::Compound(entry) = entry else { continue };
        let Some(Tag::String(name)) = util::get(entry, "Name") else { continue };
        let Some(shape) = util::get(entry, "Shape").and_then(shapes::from_tag) else { continue };
        let mut state = State::new(&name.to_string_lossy());
        if let Some(Tag::Compound(props)) = util::get(entry, "Properties") {
            for (key, value) in props {
                if let Tag::String(value) = value {
                    state.props.push((key.to_string_lossy(), value.to_string_lossy()));
                }
            }
        }
        out.insert(shapes::text(&state), Arc::from(shape));
    }
    out
}

// --- the local player -------------------------------------------------------------------------------

/// The local player as the mixin hands it over: plain numbers, changed in place.
pub struct LocalPlayer {
    pub position: [f64; 3],
    pub velocity: [f64; 3],
    pub width: f64,
    pub height: f64,
    /// `Entity.maxUpStep` (the step height attribute).
    pub max_up_step: f32,
    pub on_ground: bool,
    pub fall_distance: f64,
    /// `isPassenger()`: Create's rescue leaves a rider alone, and a rider isn't collided.
    pub passenger: bool,
}

/// `ContraptionCollider.safetyLock`: the structure the player stood on (dimension, controller) and
/// how far above its position the player was (`entity.getY() - contraptionEntity.getY()`).
static LOCK: Mutex<Option<(String, Pos, f64)>> = Mutex::new(None);
/// `packetCooldown`: the rescue counts it down only in multiplayer, for the packet we don't send.
static COOLDOWN: Mutex<i32> = Mutex::new(0);

/// `collideEntities` for the local player over every structure of the client, in Create's order,
/// then `saveClientPlayerFromClipping` on the one the lock is on. `world(motion, box)` is
/// `Entity.collideBoundingBox` against the world's blocks. Whether the player changed.
pub fn local_player_pass(player: &mut LocalPlayer, world: &mut dyn FnMut([f64; 3], [[f64; 3]; 2]) -> [f64; 3]) -> bool {
    if player.passenger || !registry::client_any() {
        return false;
    }
    let Some(now) = registry::client_now() else { return false };
    let dimension = DIMENSION.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    let entries = registry::active(Side::Client, &dimension, now);
    if entries.is_empty() {
        return false;
    }
    let mut body = EntityBody::new(player.position, player.width, player.height);
    body.player_type = PlayerType::Client;
    body.motion = player.velocity;
    body.max_up_step = player.max_up_step;
    body.on_ground = player.on_ground;
    body.fall_distance = player.fall_distance;
    let start = body;
    let mut scratch = CollisionList::new();
    let mut lock = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    for (pos, entry) in &entries {
        if entry.solid.is_empty() {
            continue;
        }
        let pose = entry.pose(now);
        let frame = ColliderFrame::new(&pose);
        // The rescue first, for the structure the player last stood on.
        if let Some((lock_dimension, lock_pos, offset)) = lock.clone()
            && lock_dimension == dimension
            && lock_pos == *pos
        {
            let mut cooldown = COOLDOWN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut ray = |from: [f64; 3], to: [f64; 3]| {
                ray_boxes(&entry.solid.list, pose.to_local_vector(from, 1.0, false), pose.to_local_vector(to, 1.0, false))
            };
            let decision = save_client_player_from_clipping(
                &pose,
                frame.contraption_motion,
                &body.bounding_box,
                body.position,
                body.motion,
                body.max_up_step,
                player.passenger,
                offset,
                false,
                &mut cooldown,
                &mut ray,
            );
            if let Some(y) = decision.new_y {
                body.set_pos([body.position[0], y, body.position[2]]);
            }
            if decision.clear_lock {
                *lock = None;
            }
        }
        if !broad_phase(&entry.solid.world_bounds(&pose), &body.bounding_box) {
            continue;
        }
        let outcome = collide_entity(
            &frame,
            &Colliders::Simplified(&entry.solid.list),
            &mut scratch,
            &|p| entry.solid.props_at(p),
            &mut body,
            &mut |motion, bb| world(motion, [bb.min, bb.max]),
        );
        if let Some(offset) = outcome.safety_lock {
            *lock = Some((dimension.clone(), *pos, offset));
        }
    }
    let changed = body.position != start.position
        || body.motion != start.motion
        || body.on_ground != start.on_ground
        || body.fall_distance != start.fall_distance;
    if changed {
        player.position = body.position;
        player.velocity = body.motion;
        player.on_ground = body.on_ground;
        player.fall_distance = body.fall_distance;
    }
    changed
}
