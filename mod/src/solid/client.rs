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
    ColliderFrame, Colliders, EntityBody, PlayerType, StructurePose, broad_phase, collide_entity,
    save_client_player_from_clipping,
};
use create_kinetics::rules::collider_obb::{Aabb, CollisionList};
use create_kinetics::rules::contraption::State;
use create_kinetics::rules::solid::{Colliding, moved_on_disassembly, ray_boxes};
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
        // The structure was set down (the entry is there only if it ran): the packet
        // `ContraptionDisassemblyPacket` makes the client run `moveCollidedEntitiesOnDisassembly`.
        if let Some(last) = registry::take(Side::Client, dimension, pos)
            && let Some(now) = host.level(dimension).and_then(|level| level.game_time(&*host))
        {
            set_down(dimension, pos, &last, id == "create:windmill_bearing", now);
        }
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
            let shapes = palette_shapes(contraption, "Shape");
            remember_blocks(dimension, pos, id == "create:elevator_pulley", &moving, &palette_shapes(contraption, "Outline"));
            let turn_axis = match sample {
                Sample::Turn { axis, .. } => Some(axis),
                Sample::Slide { .. } => None,
                // `expandBoundsAroundAxis(Axis.Y)` of an assembled carriage.
                Sample::Oriented { .. } => Some(create_kinetics::Axis::Y),
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

/// What a click's ray meets of a structure, by its dimension and controller: each block's name and
/// outline (`Contraption.getBlocks()` and `BlockState.getShape`, as the update tag carries them).
type ClickBlocks = Arc<Vec<(Pos, String, Arc<[Aabb]>)>>;

static CLICK_BLOCKS: Mutex<Option<HashMap<(String, Pos), (bool, ClickBlocks)>>> = Mutex::new(None);

fn remember_blocks(dimension: &str, controller: Pos, elevator: bool, moving: &Moving, outlines: &HashMap<String, Arc<[Aabb]>>) {
    let blocks: Vec<_> = moving
        .blocks
        .iter()
        .map(|block| {
            let boxes = match outlines.get(&shapes::text(&block.state)) {
                Some(boxes) => boxes.clone(),
                // The server hadn't asked yet: the collision boxes, or a cube.
                None => {
                    let collision = shapes::boxes_of(&block.state);
                    if collision.is_empty() && !block.state.is_air() {
                        Arc::from(vec![Aabb::new([0.0; 3], [1.0; 3])])
                    } else {
                        collision
                    }
                }
            };
            (block.local, block.state.name.clone(), boxes)
        })
        .collect();
    CLICK_BLOCKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .insert((dimension.to_owned(), controller), (elevator, Arc::new(blocks)));
}

/// The block of a structure a client ray met.
#[derive(Clone, Debug)]
pub struct ClientHit {
    /// How far from the eye, in blocks.
    pub distance: f64,
    pub controller: Pos,
    pub local: Pos,
    pub name: String,
    /// The structure is an elevator's cabin.
    pub elevator: bool,
}

/// `ContraptionHandlerClient.rayTraceContraption` for every structure of the client: the nearest block the
/// ray from `eye` along `look` meets within `reach` (the player's reach, or the distance to the real block
/// aimed at when that is nearer: `getRayInputs`).
pub fn click_ray(eye: [f64; 3], look: [f64; 3], reach: f64) -> Option<ClientHit> {
    use create_kinetics::rules::contraption_click::{ClickBlock, ray_trace};
    let now = registry::client_now()?;
    let dimension = DIMENSION.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    let target = [0, 1, 2].map(|i| eye[i] + look[i] * reach);
    let mut best: Option<ClientHit> = None;
    for (controller, entry) in registry::active(Side::Client, &dimension, now) {
        let bounds = entry.solid.world_bounds(&entry.pose(now));
        if (0..3).any(|i| eye[i].max(target[i]) + 1.0 < bounds.min[i] || eye[i].min(target[i]) - 1.0 > bounds.max[i]) {
            continue;
        }
        let Some((elevator, blocks)) = CLICK_BLOCKS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .and_then(|map| map.get(&(dimension.clone(), controller)).cloned())
        else {
            continue;
        };
        let pose = entry.pose(now);
        let from = pose.to_local_vector(eye, 1.0, false);
        let to = pose.to_local_vector(target, 1.0, false);
        let clicks = blocks.iter().map(|(local, _, boxes)| ClickBlock { local: *local, boxes });
        let Some(hit) = ray_trace(clicks, from, to) else { continue };
        let world = pose.to_global_vector(hit.point, 1.0, false);
        let distance = (0..3).map(|i| (eye[i] - world[i]).powi(2)).sum::<f64>().sqrt();
        if best.as_ref().is_none_or(|b| distance < b.distance) {
            let name = blocks.iter().find(|(local, ..)| *local == hit.local).map(|(_, n, _)| n.clone()).unwrap_or_default();
            best = Some(ClientHit { distance, controller, local: hit.local, name, elevator });
        }
    }
    best
}

/// The palette's `key` shape (`Shape` or `Outline`) of each entry of the structure, by the state's text.
fn palette_shapes(contraption: &Compound, key: &str) -> HashMap<String, Arc<[Aabb]>> {
    let mut out = HashMap::new();
    let Some(Tag::Compound(blocks)) = util::get(contraption, "Blocks") else { return out };
    let Some(Tag::List(palette)) = util::get(blocks, "Palette") else { return out };
    for entry in palette {
        let Tag::Compound(entry) = entry else { continue };
        let Some(Tag::String(name)) = util::get(entry, "Name") else { continue };
        let Some(shape) = util::get(entry, key).and_then(shapes::from_tag) else { continue };
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
    // A structure set down since the last tick first: the shift the blocks' snap asks of the player.
    let shifted = apply_set_downs(player);
    collide_local_player(player, world) || shifted
}

/// `AbstractContraptionEntity.collidingEntities` for the local player, the only entity the client
/// moves: the game time it was last registered by each structure (dimension, controller).
static COLLIDING: Mutex<Option<HashMap<(String, Pos), Colliding<()>>>> = Mutex::new(None);

/// A structure the local player stood on, set down: the pose it was last in and whether it was a windmill.
struct SetDown {
    pose: StructurePose,
    windmill: bool,
}

static SET_DOWNS: Mutex<Vec<SetDown>> = Mutex::new(Vec::new());

fn colliding() -> std::sync::MutexGuard<'static, Option<HashMap<(String, Pos), Colliding<()>>>> {
    COLLIDING.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A controller's structure was set down (its client copy stopped running): if the local player
/// stood on it in the last three ticks, remember the shift for [`local_player_pass`] to make once.
/// The server doesn't move players (`PlayerType.SERVER` is skipped), so this is the player's only
/// owner: the game's own teleport of the player never happens, and the shift is not made twice.
fn set_down(dimension: &str, pos: Pos, last: &Entry, windmill: bool, now: i64) {
    let key = (dimension.to_owned(), pos);
    let stood = {
        let mut guard = colliding();
        let Some(set) = guard.as_mut().and_then(|map| map.remove(&key)) else { return };
        !set.alive(now).is_empty()
    };
    if stood {
        let pose = last.pose(last.seen);
        SET_DOWNS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(SetDown { pose, windmill });
    }
}

/// `entity.setPos(transformed.x, transformed.y + 1 / 16f, transformed.z)` for each structure set down.
fn apply_set_downs(player: &mut LocalPlayer) -> bool {
    let pending = std::mem::take(&mut *SET_DOWNS.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
    for done in &pending {
        player.position = moved_on_disassembly(&done.pose, done.windmill, player.position);
    }
    !pending.is_empty()
}

fn collide_local_player(player: &mut LocalPlayer, world: &mut dyn FnMut([f64; 3], [[f64; 3]; 2]) -> [f64; 3]) -> bool {
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
        if outcome.registered_colliding {
            // `contraptionEntity.registerColliding(entity)`.
            colliding().get_or_insert_with(HashMap::new).entry((dimension.clone(), *pos)).or_default().register((), now);
        }
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
