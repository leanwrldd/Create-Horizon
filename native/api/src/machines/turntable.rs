//! The Turntable (`TurntableBlock`, `TurntableBlockEntity`, `TurntableHandler`): a block on a shaft
//! from below that turns what stands on it. The numbers are `create_kinetics::rules::turntable`;
//! this is the glue.
//!
//! Known differences from Create, for the README:
//! - Create turns mobs from `entityInside` (every tick, on the server) and players on their own
//!   client (`TurntableHandler`, each rendered frame, with a pull towards the centre the player
//!   isn't in). Horizon has no per-frame hook for a player's view, so a player standing on the
//!   turntable is turned by the server each tick (`entity_set_rotation`), at the same
//!   `rpm * 3 / 10` degrees a tick as a mob, fading out towards the edge as `TurntableHandler`
//!   does. The turn arrives with the server's tick, so it is less smooth than Create's.
//! - Create's `TurntableBlock.entityInside` carry of a player's position (the rotated offset added
//!   to their motion) only runs when the player is not inside the turntable's own block, which
//!   never holds for one standing on it, so it is not done here either.
//! - Entities are found by their feet being in the block's column, each tick the turntable turns,
//!   rather than as the game finds them inside the block.
//! - A mob keeps its `setNoActionTime(20)` and `hurtMarked` behaviour only as far as turning its
//!   body, head and yaw goes: Horizon has no call to hold its AI idle.

use create_kinetics::rules::turntable as rules;
use create_kinetics::{Dir, Kind, Node};
use horizon_client::mod_api::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, Compound, Flow, Hooks, Host, UseItemOnEvent,
};

use crate::{kinetic, util};

const ID: &str = "create:turntable";
const PLAYER: &str = "minecraft:player";
const CENTRE: [f32; 3] = [0.5; 3];

struct Turntable;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Turntable)
}

/// The turntable's shape from `AllShapes.TURNTABLE_SHAPE`: the plate at 4 to 8 and the foot below.
const TOP: f64 = 0.5;

/// What `TurntableBlock.entityInside` does to everything standing on the turntable at `pos`.
fn turn_entities(host: &mut dyn Host, pos: [i32; 3], rpm: f32) {
    let per_tick = rules::degrees_per_tick(rpm);
    if per_tick == 0.0 {
        return;
    }
    let dimension = host.dimension();
    let [x, y, z] = pos.map(f64::from);
    // `entities_in` looks at feet: reach a little past the block for those whose box overlaps it.
    let lo = [x - 0.5, y + TOP - 0.01, z - 0.5];
    let hi = [x + 1.5, y + 1.0, z + 1.5];
    for id in host.entities_in(dimension, lo, hi) {
        let Some(entity) = host.entity(id) else {
            continue;
        };
        let Some(kind) = host.entity_kind(entity) else {
            continue;
        };
        // An item entity's yaw is not drawn.
        if kind == "minecraft:item" {
            continue;
        }
        let Some(position) = host.entity_position(entity) else {
            continue;
        };
        let overlaps = match host.entity_bounding_box(entity) {
            Some([min, max]) => min[0] < x + 1.0 && max[0] > x && min[2] < z + 1.0 && max[2] > z,
            None => (x..x + 1.0).contains(&position[0]) && (z..z + 1.0).contains(&position[2]),
        };
        if !overlaps {
            continue;
        }
        let on_ground = host.entity_on_ground(entity).unwrap_or(false);
        let velocity_y = host.entity_velocity(entity).map_or(0.0, |v| v[1]);
        if !rules::stands_on(on_ground, velocity_y, position[1], pos[1]) {
            continue;
        }
        let Some([yaw, pitch]) = host.entity_rotation(entity) else {
            continue;
        };
        let mut speed = per_tick;
        if kind == PLAYER {
            // `TurntableHandler.gameRenderFrame`: full speed near the middle, fading to nothing a
            // half block out.
            let from_centre = ((position[0] - x - 0.5).powi(2) + (position[2] - z - 0.5).powi(2)).sqrt();
            if from_centre > 0.25 {
                speed *= ((0.5 - from_centre) * 2.0).clamp(0.0, 1.0) as f32;
            }
            if speed != 0.0 {
                host.entity_set_rotation(entity, yaw - speed, pitch);
            }
            continue;
        }
        if let Some(head) = host.entity_head_yaw(entity) {
            let (head, yaw) = rules::turned_yaws(head, yaw, speed);
            host.entity_set_head_yaw(entity, head);
            host.entity_set_rotation(entity, yaw, pitch);
        } else {
            let (_, yaw) = rules::turned_yaws(yaw, yaw, speed);
            host.entity_set_rotation(entity, yaw, pitch);
        }
    }
}

impl Hooks for Turntable {
    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == ID).then(|| (ID.to_owned(), kinetic::new_entity()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        // `TurntableBlock.hasShaftTowards`: the shaft comes out of the bottom.
        let node = Node {
            impact: rules::IMPACT,
            ..Node::passive(Kind::shafts(&[Dir::Down]))
        };
        let Some((rpm, changed)) = kinetic::tick(host, event.pos, event.entity, node) else {
            return; // The network jammed and the block broke.
        };
        if changed {
            host.block_entity_sync(event.pos);
        }
        turn_entities(host, event.pos, rpm);
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        (event.entity.id == ID).then(|| kinetic::update_tag(event.entity))
    }

    /// `KineticBlockEntityRenderer`: the whole block turns about the vertical with its shaft.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let rpm = kinetic::speed(event.entity);
        event.draw.spin([0.0, 1.0, 0.0], CENTRE, rpm * 0.3, 0.0);
        host.draw_block(event.draw, ID, &[]);
        event.draw.still();
        event.draw.keep();
    }

    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != ID {
            return Flow::Continue;
        }
        // `IWrenchable.onWrenched`: nothing to turn on a block with no state.
        util::wrench(host, event.pos, |_| {})
    }
}
