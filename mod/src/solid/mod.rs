//! Moving structures made solid: Create's `ContraptionCollider.collideEntities`, on the new mod API.
//!
//! A structure that moves (the piston, the rope and elevator pulleys, the gantry, the bearing and
//! so the windmill) is a controller's block entity, not an entity: Horizon has no entity type a mod
//! can give a collider. So the collider reads the structure from the controller and runs the
//! entities against it each tick:
//!
//! - **where the structure is** is a function of game time that each controller already knows for
//!   drawing (`solid_sample` in `piston.rs`, `pulley.rs`, `gantry.rs`, `bearing.rs`, `elevator.rs`);
//!   this module asks it for now and a tick ago ([`Sample`]) and keeps the answer in a table
//!   ([`registry`]). The server asks after each controller's tick ([`server::observe`], called by the
//!   block entity part), a client from its own copy of the block entity's tick ([`client::observe`]):
//!   a client sees the structure where it draws it, at the same game time.
//! - **what is solid** is the collision shape of each block of the structure ([`shapes`]): the
//!   server asks the game for the shape of each distinct state (`LevelRef::block_shape`) and puts it
//!   in the controller's update tag, a client can't ask for it.
//! - **the server** ([`server::level_tick`], `level_tick_post`) runs Create's loop for every entity
//!   but players (a server player is not collided: `PlayerType.SERVER`): the entities near each
//!   structure go through `collider::collide_entities`, and what comes out is applied with the host:
//!   a teleport for the position Create sets, `set_velocity` for the motion, `reset_fall_distance`.
//!   It is applied after the tick rather than inside `entity_move`, which takes no host and is told
//!   neither the entity's position nor its box, only its id and the move it asks for. Create sets
//!   the entity's position at the end of the level tick too.
//! - **a client** collides its own player ([`client::local_player_pass`], called from a mixin on the
//!   end of the local player's simulation tick, `mixins/local_player_solid.rs`): the client moves
//!   its player itself and the server doesn't predict it, so this is the only place the structure
//!   can push, carry and hold up the player. The clipping rescue (`safetyLock`) is there too.
//!
//! The rules are `create_kinetics::rules::{collider, collider_obb, solid}`; this is glue.

pub mod client;
pub(crate) mod registry;
pub(crate) mod server;
pub(crate) mod shapes;

use create_kinetics::rules::collider_obb::Aabb;
use create_kinetics::rules::solid::{Solid, SolidBlock};
use create_kinetics::{Axis, Pos};

/// Where a structure is at a game time and a tick before it, as a controller reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sample {
    /// A structure that slides (piston, pulleys, gantry): the world position of the anchor cell's
    /// minimum corner, now and a tick ago.
    Slide { now: [f64; 3], prev: [f64; 3] },
    /// A structure that turns (bearing): right-handed about `axis` through the centre of the anchor
    /// cell by `now` degrees, `prev` a tick ago.
    Turn { axis: Axis, now: f32, prev: f32 },
    /// A carriage of a train (`OrientedContraptionEntity`): the entity's position (the bogey's block's bottom middle)
    /// and its yaw and pitch (`yaw`, `pitch` as the entity keeps them), now and a tick ago.
    Oriented { now: [f64; 3], prev: [f64; 3], initial_yaw: f32, yaw: f32, prev_yaw: f32, pitch: f32, prev_pitch: f32 },
}

/// The block entities that are controllers of a structure, by id.
pub fn is_controller(id: &str) -> bool {
    matches!(
        id,
        "create:mechanical_piston"
            | "create:sticky_mechanical_piston"
            | "create:rope_pulley"
            | "create:elevator_pulley"
            | "create:gantry_shaft"
            | "create:mechanical_bearing"
            | "create:windmill_bearing"
            | "create:carriage_anchor"
    )
}

/// Ticks a structure stays solid after its controller last reported it: a controller reports each
/// tick it runs, and a structure at rest for a few ticks (the network is solved again after a load)
/// is still there.
pub(crate) const STAY: i64 = 12;

/// Where the structure of the controller at `pos` is at game time `t` and a tick before, asked of
/// the machine whose block entity it is. `entity` holds the saved scalars (the structure itself is
/// not read), `anchor` is `Contraption.anchor`.
pub(crate) fn sample_of(
    host: &dyn crate::compat::Host,
    pos: Pos,
    entity: &crate::compat::ModBlockEntity,
    anchor: Pos,
    t: f64,
) -> Option<Sample> {
    use crate::machines::{bearing, elevator, gantry, piston, pulley};
    piston::solid_sample(host, pos, entity, anchor, t)
        .or_else(|| pulley::solid_sample(host, pos, entity, anchor, t))
        .or_else(|| gantry::solid_sample(host, pos, entity, anchor, t))
        .or_else(|| bearing::solid_sample(host, pos, entity, anchor, t))
        .or_else(|| elevator::solid_sample(host, pos, entity, anchor, t))
        .or_else(|| crate::machines::train_draw::solid_sample(entity, t))
}

/// The entity's data without the structure (`Contraption`, the blocks): what a `solid_sample` reads.
pub(crate) fn light_copy(id: &str, nbt: &crate::compat::Compound) -> crate::compat::ModBlockEntity {
    let mut out = crate::compat::Compound::new();
    for (key, value) in nbt {
        if matches!(value, crate::compat::Tag::Compound(_)) && key.to_string_lossy().starts_with("Contraption") {
            continue;
        }
        out.insert(key.clone(), value.clone());
    }
    crate::compat::ModBlockEntity { id: id.to_owned(), nbt: out, items: Vec::new(), data: Vec::new() }
}

/// The solid of a structure: its blocks' boxes by `shape`, widened round the axis when it turns.
pub(crate) fn build_solid(
    moving: &crate::machines::contraption::Moving,
    turn_axis: Option<Axis>,
    shape: &dyn Fn(&create_kinetics::rules::contraption::State) -> std::sync::Arc<[Aabb]>,
) -> Solid {
    let boxes: Vec<std::sync::Arc<[Aabb]>> = moving.blocks.iter().map(|b| shape(&b.state)).collect();
    let blocks: Vec<SolidBlock<'_>> = moving
        .blocks
        .iter()
        .zip(&boxes)
        .map(|(b, boxes)| SolidBlock { local: b.local, name: &b.state.name, boxes })
        .collect();
    Solid::build(&blocks, turn_axis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_controller_ids_are_the_machines_and_the_carriage_anchor() {
        for id in [
            "create:mechanical_piston",
            "create:sticky_mechanical_piston",
            "create:rope_pulley",
            "create:elevator_pulley",
            "create:gantry_shaft",
            "create:mechanical_bearing",
            "create:windmill_bearing",
        ] {
            assert!(is_controller(id), "{id}");
        }
        assert!(is_controller("create:carriage_anchor"));
        assert!(!is_controller("create:shaft"));
        assert!(!is_controller("create:clockwork_bearing"));
    }
}
