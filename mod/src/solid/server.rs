//! The server half: reporting each running structure after its controller's tick, and
//! `ContraptionCollider.collideEntities` for every entity but a player at the end of the level tick.

use std::sync::Arc;

use create_kinetics::Pos;
use create_kinetics::rules::collider::{ColliderFrame, Colliders, EntityBody, broad_phase_area, can_collide_with, collide_entities};
use create_kinetics::rules::collider_obb::{Aabb, CollisionList};
use create_kinetics::rules::solid::{collide_bounding_box, entity_facts};
use mod_api::{EntityRef, Host, LevelRef, ServerHost};

use super::registry::{self, Entry, Side};
use super::{Sample, build_solid, shapes};
use crate::compat::{Adapter, ModBlockEntity};
use crate::machines::contraption as engine;
use crate::util;

/// The most entities one structure collides in a tick.
const MAX_ENTITIES: usize = 256;

/// A controller's tick has run: report where its structure is, so the level tick can collide
/// entities with it. `entity` is the block entity after its tick.
pub fn observe(host: &mut dyn ServerHost, pos: Pos, entity: &ModBlockEntity) {
    if !super::is_controller(&entity.id) {
        return;
    }
    let dimension = host.dimension();
    if !util::boolean(&entity.nbt, engine::RUNNING) {
        registry::forget(Side::Server, dimension, pos);
        return;
    }
    let Some(live) = engine::live(dimension, pos, &entity.nbt) else {
        registry::forget(Side::Server, dimension, pos);
        return;
    };
    let Some(level) = host.level(dimension) else { return };
    let Some(now) = level.game_time(&*host) else { return };
    let anchor = live.moving.anchor;
    let sample = {
        let adapter = Adapter::Server(&mut *host);
        super::sample_of(&adapter, pos, entity, anchor, now as f64)
    };
    let Some(sample) = sample else {
        registry::forget(Side::Server, dimension, pos);
        return;
    };
    let turn_axis = match sample {
        Sample::Turn { axis, .. } => Some(axis),
        Sample::Slide { .. } => None,
    };
    let token_of = |version: u64| (Arc::as_ptr(&live) as usize as u64) ^ version.rotate_left(32);
    let mut token = token_of(shapes::version());
    let solid = match registry::token(Side::Server, dimension, pos) {
        Some((known, solid)) if known == token => solid,
        _ => {
            // The game's shapes of the structure's states: asked once for each, put in the update
            // tag (a client can't ask), so it is sent again when any was new.
            if shapes::fill(&*host, level, pos, live.moving.blocks.iter().map(|b| &b.state)) {
                host.block_entity_sync(pos);
            }
            token = token_of(shapes::version());
            Arc::new(build_solid(&live.moving, turn_axis, &shapes::boxes_of))
        }
    };
    registry::put(Side::Server, dimension, pos, Entry { solid, anchor, sample, seen: now, token });
}

/// `level_tick_post` on the server: every structure of the level, against the entities near it.
pub fn level_tick(host: &mut dyn ServerHost, level: LevelRef) {
    let dimension = host.dimension();
    let Some(now) = level.game_time(&*host) else { return };
    for (_, entry) in registry::active(Side::Server, dimension, now) {
        if !entry.solid.is_empty() {
            pass(host, level, &entry, now);
        }
    }
}

/// An entity as `collideEntities` takes it, or `None` for one Create leaves alone
/// (`canCollideWith`, a player on the server).
fn candidate(host: &dyn Host, entity: EntityRef) -> Option<EntityBody> {
    let kind = entity.kind(host)?.id(host);
    let facts = entity_facts(&kind);
    if facts.is_player {
        return None;
    }
    let mut collide = facts.collide;
    collide.no_physics = entity.no_physics(host).unwrap_or(false);
    collide.has_vehicle = entity.vehicle(host).is_some();
    if !can_collide_with(&collide, false) {
        return None;
    }
    let position = entity.position(host)?;
    let [min, max] = entity.bounding_box(host)?;
    let mut body = EntityBody::new(position, 0.0, 0.0);
    body.bounding_box = Aabb::new(min, max);
    body.motion = entity.velocity(host).unwrap_or([0.0; 3]);
    body.on_ground = entity.on_ground(host).unwrap_or(false);
    // `Entity.maxUpStep`: the step height attribute for a living entity, 0 for the rest.
    body.max_up_step = if host.living(entity).is_some() { 0.6 } else { 0.0 };
    body.is_item = facts.is_item;
    Some(body)
}

/// `Entity.collideBoundingBox` against the world's blocks.
fn world_collide(host: &dyn Host, level: LevelRef, motion: [f64; 3], bb: &Aabb) -> [f64; 3] {
    let region = bb.expand_towards(motion);
    let area = [
        region.min[0] - 1.0E-3,
        region.min[1] - 1.0E-3,
        region.min[2] - 1.0E-3,
        region.max[0] + 1.0E-3,
        region.max[1] + 1.0E-3,
        region.max[2] + 1.0E-3,
    ];
    let boxes: Vec<Aabb> = level
        .collision_boxes(host, area)
        .unwrap_or_default()
        .iter()
        .map(|b| Aabb::new([b[0], b[1], b[2]], [b[3], b[4], b[5]]))
        .collect();
    collide_bounding_box(motion, bb, &boxes)
}

/// One structure's loop over the entities near it.
fn pass(host: &mut dyn ServerHost, level: LevelRef, entry: &Entry, now: i64) {
    let pose = entry.pose(now);
    let frame = ColliderFrame::new(&pose);
    let area = broad_phase_area(&entry.solid.world_bounds(&pose));
    let Some(found) = level.entities_in(&*host, area.min, area.max) else { return };
    let mut entities: Vec<EntityRef> = Vec::new();
    let mut bodies: Vec<EntityBody> = Vec::new();
    for entity in found {
        if bodies.len() >= MAX_ENTITIES {
            break;
        }
        if let Some(body) = candidate(&*host, entity) {
            entities.push(entity);
            bodies.push(body);
        }
    }
    if bodies.is_empty() {
        return;
    }
    let before = bodies.clone();
    let mut scratch = CollisionList::new();
    let outcomes = {
        let reader: &dyn Host = &*host;
        collide_entities(
            &frame,
            &Colliders::Simplified(&entry.solid.list),
            &mut scratch,
            &|p| entry.solid.props_at(p),
            &mut bodies,
            &mut |_, motion, bb| world_collide(reader, level, motion, bb),
        )
    };
    for (((entity, was), now_body), outcome) in entities.iter().zip(&before).zip(&bodies).zip(&outcomes) {
        if outcome.skipped || !outcome.collided {
            continue;
        }
        // Create sets the position (`setPos`) and the motion; the host's teleport clears the
        // velocity, so it is set after.
        let moved = (0..3).any(|i| (now_body.position[i] - was.position[i]).abs() > 1.0E-6);
        if moved {
            entity.teleport_to(host, now_body.position, None);
        }
        if moved || now_body.motion != was.motion {
            entity.set_velocity(host, now_body.motion);
        }
        if outcome.fall_distance_reset {
            entity.reset_fall_distance(host);
        }
    }
}
