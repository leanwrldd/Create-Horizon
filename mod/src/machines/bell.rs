//! The Peculiar Bell and the Haunted Bell (`PeculiarBellBlock`, `HauntedBellBlock`, `AbstractBellBlock`,
//! `HauntedBellPulser`, `SoulPulseEffect`): bells made from vanilla's bell block (it rings when hit
//! or powered) with Create's models. A peculiar bell placed over soul fire or a soul campfire becomes
//! a haunted one, and a player holding a haunted bell sees where a mob could spawn nearby. The rules
//! are `create_kinetics::rules::bell`; this is the glue.
//!
//! Known differences from Create, for the README:
//! - **Not played.** The blocks are declared on vanilla's `minecraft:bell` template with
//!   Create's models in the chunk mesh; whether the template's own bell block entity (vanilla draws its
//!   body as an entity) draws over Create's model, and whether it rings and swings, is not known.
//!   Create's own ring (`AbstractBellBlockEntity`, a swing with a particle) and its sounds
//!   (`peculiar_bell_use`, `haunted_bell_use`) are vanilla's here.
//! - The pulse (`SoulPulseEffect`) is sent to the holder only, as dust at each dark spot (a free
//!   spot with a solid floor and no block light) every 8 ticks within 3 blocks; Create draws
//!   soul particles and a perimeter outline in layers outward.
//! - A bell becomes haunted when it is placed over soul fire or a soul campfire: not when soul
//!   fire is lit under one later (`updateShape`), and the Haunted Bell has no `HauntedBellMovementBehaviour`
//!   on contraptions.
//! - The haunted bell's own block entity effect (`effectTicks`, the particles it throws when ringing) is
//!   not done.

use std::collections::BTreeMap;
use std::sync::Mutex;

use create_kinetics::rules::bell::{self as rules, Pulser, Spot};
use create_kinetics::rules::drill::is_air;
use crate::compat::{
    EquipmentSlot, Hooks, Host, ParticleOptions, PlaceBlockEvent, PlayerTickEvent,
};

use crate::util;

struct Bells;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Bells)
}

/// Each player holding a haunted bell, and where its warm-up is.
static PULSERS: Mutex<BTreeMap<u128, (Pulser, i64)>> = Mutex::new(BTreeMap::new());

fn lock<T>(m: &'static Mutex<T>) -> std::sync::MutexGuard<'static, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn solid(block: &str) -> bool {
    !is_air(block) && !block.ends_with("water") && !block.ends_with("lava")
}

impl Hooks for Bells {
    /// `PeculiarBellBlock.getStateForPlacement` and `tryConvert`.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if &*event.stack.item != rules::PECULIAR_BELL {
            return;
        }
        let pos = event.pos;
        let below = [pos[0], pos[1] - 1, pos[2]];
        if !rules::haunts(&host.block(below)) {
            return;
        }
        let (Some(facing), Some(attachment), Some(powered)) = (
            host.block_property(pos, "facing"),
            host.block_property(pos, "attachment"),
            host.block_property(pos, "powered"),
        ) else {
            return;
        };
        let Some(state) = host.default_state(rules::HAUNTED_BELL) else {
            return;
        };
        let dimension = host.dimension();
        if !host.set_block(dimension, pos, state, util::UPDATE_ALL) {
            return;
        }
        host.set_block_property(pos, "facing", &facing);
        host.set_block_property(pos, "attachment", &attachment);
        host.set_block_property(pos, "powered", &powered);
        let at = util::centre(pos);
        host.play_sound("create:haunted_bell_convert", at, 4, 1.0, 1.0);
        for _ in 0..16 {
            host.send_particles(&ParticleOptions::dust([0.3, 0.9, 1.0], 0.8), None, at, [0.3; 3], 0.05, 1);
        }
    }

    /// `HauntedBellPulser.hauntedBellCreatesPulse`, seen only by the one holding it.
    fn player_tick(&self, host: &mut dyn Host, _: &mut PlayerTickEvent) {
        let Some(player) = host.acting_player() else {
            return;
        };
        let uuid = player.uuid;
        let holding = [host.held_stack(uuid), host.player_equipment(uuid, EquipmentSlot::OffHand)]
            .into_iter()
            .flatten()
            .any(|stack| &*stack.item == rules::HAUNTED_BELL);
        if !holding || player.game_mode == util::SPECTATOR {
            lock(&PULSERS).remove(&uuid);
            return;
        }
        let now = util::game_time(&*host);
        let due = {
            let mut pulsers = lock(&PULSERS);
            let entry = pulsers.entry(uuid).or_insert_with(|| (Pulser::default(), now));
            entry.1 = now;
            entry.0.tick(now) && now % 4 == 0
        };
        if !due {
            return;
        }
        let base = [
            player.position[0].floor() as i32,
            player.position[1].floor() as i32,
            player.position[2].floor() as i32,
        ];
        for offset in rules::pulse_offsets(rules::DISTANCE) {
            let pos = [base[0] + offset[0], base[1] + offset[1], base[2] + offset[2]];
            let free = is_air(&host.block(pos));
            let above_free = free && is_air(&host.block([pos[0], pos[1] + 1, pos[2]]));
            let below_solid = free && solid(&host.block([pos[0], pos[1] - 1, pos[2]]));
            let light = if free && above_free && below_solid { host.light(pos).0 } else { 0 };
            if rules::spot(free, above_free, below_solid, light) == Spot::Dark {
                let at = [f64::from(pos[0]) + 0.5, f64::from(pos[1]) + 0.1, f64::from(pos[2]) + 0.5];
                host.send_particles_to(uuid, &ParticleOptions::dust([0.3, 0.9, 1.0], 0.6), None, at, [0.0; 3], 0.0, 1, false);
            }
        }
    }
}
