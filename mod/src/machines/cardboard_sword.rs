//! The Cardboard Sword (`create:cardboard_sword`) (`CardboardSwordItem`): a sword that never
//! hurts. A player holding it who hits a living creature (not an arthropod) deals no damage;
//! the target is knocked back instead, by 2 (3 while sprinting) times a half, along the way the
//! player faces (`cardboardSwordsCannotHurtYou`, `knockback`), with Create's bonk sound.
//!
//! Known differences from Create: no slowness for the creatures and animals it hits
//! (`MOVEMENT_SLOWDOWN`), no Knockback enchantment's extra, no sound when it is swung at a block
//! (`cardboardSwordsMakeNoiseOnClick`), no stopped sprint or slowed attacker.

use std::collections::BTreeSet;
use std::sync::Mutex;

use crate::compat::{Flow, Hooks, Host, ItemStack, LevelTickPostEvent, LivingIncomingDamageV3Event, PlayerTickEvent};

use crate::util;

const ID: &str = "create:cardboard_sword";

/// Players holding the sword in the main hand, as of their last tick, with their facing.
static HOLDERS: Mutex<Vec<(u128, f32)>> = Mutex::new(Vec::new());
/// Knockbacks to apply: the victim's entity id, the attacker's yaw, and the strength.
static PENDING: Mutex<Vec<(u32, f32, f32)>> = Mutex::new(Vec::new());
/// `EntityTypeTags.ARTHROPOD`.
static ARTHROPODS: [&str; 5] = [
    "minecraft:spider", "minecraft:cave_spider", "minecraft:bee", "minecraft:silverfish", "minecraft:endermite",
];

struct Sword;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Sword)
}

/// `knockback`'s push: along the attacker's facing, half the strength, and a little up.
pub fn push_for(yaw: f32, strength: f32) -> [f64; 3] {
    let yaw = f64::from(yaw).to_radians();
    let power = f64::from(strength) * 0.5;
    [-yaw.sin() * power, 0.4, yaw.cos() * power]
}

fn spared(kind: &str) -> bool {
    ARTHROPODS.contains(&kind)
}

impl Hooks for Sword {
    fn player_tick(&self, host: &mut dyn Host, _: &mut PlayerTickEvent) {
        let Some(player) = host.acting_player() else {
            return;
        };
        let holds = host.held_stack(player.uuid).is_some_and(|stack: ItemStack| &*stack.item == ID);
        let mut holders = HOLDERS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        holders.retain(|(uuid, _)| *uuid != player.uuid);
        if holds {
            holders.push((player.uuid, player.yaw));
        }
    }

    /// `cardboardSwordsCannotHurtYou`: the hit does nothing but push.
    fn living_incoming_damage_v3(&self, event: &mut LivingIncomingDamageV3Event<'_>) -> Flow {
        let Some(attacker) = event.attacker_uuid else {
            return Flow::Continue;
        };
        if event.damage_type != "minecraft:player_attack" || spared(event.kind) {
            return Flow::Continue;
        }
        let yaw = HOLDERS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .find(|(uuid, _)| *uuid == attacker)
            .map(|(_, yaw)| *yaw);
        let Some(yaw) = yaw else {
            return Flow::Continue;
        };
        PENDING.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((event.victim, yaw, 2.0));
        Flow::Cancel
    }

    fn level_tick_post(&self, host: &mut dyn Host, _: &mut LevelTickPostEvent<'_>) {
        let jobs: Vec<(u32, f32, f32)> = std::mem::take(&mut *PENDING.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
        let mut seen = BTreeSet::new();
        for (id, yaw, strength) in jobs {
            if !seen.insert(id) {
                continue;
            }
            if let Some(entity) = host.entity(id) {
                entity.push(host, push_for(yaw, strength));
                let _ = util::game_time(&*host);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_push_follows_the_way_the_attacker_faces() {
        // Yaw 0 faces south: pushed towards +z.
        let south = push_for(0.0, 2.0);
        assert!(south[0].abs() < 1e-9 && (south[2] - 1.0).abs() < 1e-9 && south[1] == 0.4);
        // Yaw 90 faces west.
        let west = push_for(90.0, 2.0);
        assert!((west[0] + 1.0).abs() < 1e-9 && west[2].abs() < 1e-9);
    }

    #[test]
    fn spiders_are_spared_the_bonk() {
        assert!(spared("minecraft:spider"));
        assert!(!spared("minecraft:zombie"));
    }
}
