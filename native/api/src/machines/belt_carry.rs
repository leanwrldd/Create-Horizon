//! Players and mobs riding the Mechanical Belt (`BeltBlock.entityInside`,
//! `BeltMovementHandler.transportEntity`). The rules are `create_kinetics::rules::belt_carry`; this
//! reads the world, asks them how far Create would move the entity this tick, and pushes.
//!
//! Create moves the entity itself every tick (`Entity.move`), and for a player on their own
//! client. Horizon gives a server mod no such call, only velocity: `entity_push` for a mob, and for
//! a player a message their client answers with `push_player` (as the Encased Fan does). The push is
//! worked out so the entity, with the ground's drag, travels what Create would move it each tick
//! (`belt_carry::push`).
//!
//! Known differences from Create, for the README:
//! - Pushed, not moved: an entity eases up to the belt's speed over a few ticks and eases off after
//!   leaving it, and a player is moved by a message to their client one tick late.
//! - Up and down a slope nothing lifts the entity: the belt's stair-stepped collision does it, as it
//!   would a person walking.
//! - No pull towards the belt's line (a mod can't tell a player or mob that walks from one that
//!   stands), no Slowness on mobs (the `Host` gives effects to players only), no check for entities
//!   in the way, no stepping height change for mobs, and the cardboard boots and diving boots
//!   exceptions are not there.
//! - Only players and the mobs, minecarts and boats that Horizon ticks are carried; item entities
//!   join the belt as items instead.

use create_kinetics::rules::belt_carry::{self, GROUND_FRICTION};
use horizon_client::mod_api::{
    EntityTickListenedEvent, EntityTickPostEvent, Hooks, Host, NetFromServerEvent, PlayerTickEvent,
    ServerTickEvent, ServerTickPhase,
};

use crate::belt;

/// The channel the server tells a player's client to push them on.
const CHANNEL: &str = "create:belt_push";
/// `GameType.SPECTATOR`.
const SPECTATOR: i32 = 3;

struct BeltCarry;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(BeltCarry)
}

/// The push for an entity with its feet at `feet`, `on_ground` or not: the first block its feet
/// are in or the one below that is a belt carrying it (`BeltBlock.entityInside`,
/// `updateEntityAfterFallOn`). None where nothing carries it.
fn push_at(host: &dyn Host, feet: [f64; 3], on_ground: bool) -> Option<[f64; 3]> {
    for cell in belt_carry::cells_under(feet) {
        let Some(surface) = belt::surface(host, cell) else {
            continue;
        };
        if feet[1] - f64::from(cell[1]) > belt_carry::MAX_HEIGHT {
            continue;
        }
        let Some(carry) = belt_carry::carry(&surface, feet, false) else {
            continue;
        };
        // Stepping down a slope the entity is in the air between steps: keep pushing.
        if !on_ground && !carry.on_slope {
            continue;
        }
        return Some(belt_carry::push(&carry, GROUND_FRICTION));
    }
    None
}

impl Hooks for BeltCarry {
    /// Counts server ticks, for what only matters while a belt moves (`belt::recently_moving`).
    fn server_tick(&self, _: &mut dyn Host, event: &mut ServerTickEvent<'_>) {
        if let ServerTickPhase::ServerTickStart { world_running: true } = event.phase {
            belt::next_tick();
        }
    }

    /// Mobs are told of only while a belt that carries things has turned lately: the game keeps no
    /// list of ticking entities otherwise.
    fn entity_tick_listened(&self, _: &mut dyn Host, event: &mut EntityTickListenedEvent) {
        if belt::recently_moving() {
            event.listened = true;
        }
    }

    /// A mob, minecart or boat that ticked: if it stands on a moving belt, push it.
    fn entity_tick_post(&self, host: &mut dyn Host, event: &mut EntityTickPostEvent<'_>) {
        if !belt::recently_moving() || event.kind == "minecraft:item" || event.kind == "minecraft:player" {
            return;
        }
        let Some(entity) = host.entity(event.entity) else {
            return;
        };
        let Some(feet) = host.entity_position(entity) else {
            return;
        };
        // Cheap first: a block lookup for the feet and the one below.
        let sneaking = host.entity_is_shift_key_down(entity).unwrap_or(false);
        if !belt_carry::can_ride(sneaking, false) {
            return;
        }
        let on_ground = host.entity_on_ground(entity).unwrap_or(false);
        if let Some(push) = push_at(&*host, feet, on_ground) {
            host.entity_push(entity, push);
        }
    }

    /// A player on a moving belt: their client is told how much to push them.
    fn player_tick(&self, host: &mut dyn Host, _: &mut PlayerTickEvent) {
        if !belt::recently_moving() {
            return;
        }
        let Some(player) = host.acting_player() else {
            return;
        };
        if player.game_mode == SPECTATOR || !belt_carry::can_ride(player.sneaking, player.flying) {
            return;
        }
        let Some(push) = push_at(&*host, player.position, player.on_ground) else {
            return;
        };
        let mut data = Vec::with_capacity(12);
        for v in push {
            data.extend_from_slice(&(v as f32).to_le_bytes());
        }
        let _ = host.net_send_to_player(player.uuid, CHANNEL, &data);
    }

    /// The client half: the server's push for this player, added to their velocity.
    fn net_from_server(&self, host: &mut dyn Host, event: &mut NetFromServerEvent<'_>) {
        if event.channel != CHANNEL || event.data.len() != 12 {
            return;
        }
        let mut push = [0.0_f32; 3];
        for (i, chunk) in event.data.chunks_exact(4).enumerate() {
            push[i] = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        if push.iter().all(|v| v.is_finite()) {
            host.push_player(push);
        }
    }
}
