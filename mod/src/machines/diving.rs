//! The Copper and Netherite Diving Helmets and Boots (`DivingHelmetItem`, `DivingBootsItem`,
//! `NetheriteDivingHandler`): the helmet lets a player breathe under water on the air of a worn
//! backtank; a netherite helmet also drains a netherite tank in lava, and a full netherite set (helmet,
//! netherite backtank with air, leggings, boots) makes the player immune to fire and lava; the boots
//! pull a player down in water. The rules are `create_kinetics::rules::diving`; this is the glue.
//!
//! Known differences from Create, for the README:
//! - The helmets and boots are made from iron (copper) and netherite armour: they look like them
//!   worn, with Create's armour values and durability, and the copper pieces repair with copper
//!   ingots. No Aqua Affinity from the helmet (`DivingHelmetItem.supportsEnchantment`): it can't be
//!   faked without an enchantment of its own.
//! - Horizon's `living_breathe` hook has no host, so the player's gear is read in their own tick and
//!   the hook answers from what was read (one tick old). The tank's air is taken a second later in
//!   the level tick, because a player's slots can't be changed inside their own hook.
//! - Fire immunity is the cancelling of fire, lava and hot-floor damage for a player in the full set
//!   (`setFireImmune`), so a player may still burn visually and can't swim in lava any faster.
//! - The boots only pull the player down (`motion.add(0, -0.05, 0)`) from the client's tick, in water
//!   that is not swimming (the swimming pose can't be told): no jump boost from the sea floor, no
//!   horizontal speed-up (`getMovementMultiplier`), no lava walking.
//! - No air overlay (`RemainingAirOverlay`): the low and depleted warnings show on the action bar (with Create's
//!   deny and steam sounds), not as a title.

use std::collections::BTreeMap;
use std::sync::Mutex;

use create_kinetics::rules::backtank as tank;
use create_kinetics::rules::diving::{self as rules, Breath};
use crate::compat::{
    ClientTickPostEvent, EquipmentSlot, Flow, Hooks, Host, ItemStack, LevelTickPostEvent,
    LivingBreatheEvent, LivingIncomingDamageV3Event, NativeInventoryKind, PlayerTickEvent, Tag, Text,
};

use super::backtank::air_of;
use crate::util;

/// The chest slot in `NativeInventoryKind::Player`'s numbering (36 feet, 37 legs, 38 chest, 39 head).
const CHEST: usize = 38;

struct Diving;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Diving)
}

/// What a player wore at their last tick.
#[derive(Clone, Default)]
struct Worn {
    entity_id: u32,
    helmet: Option<String>,
    /// The worn chest item and its air.
    chest: Option<(String, i32)>,
    chest_stack: Option<ItemStack>,
    legs: Option<String>,
    feet: Option<String>,
    in_lava: bool,
    full_set: bool,
    seen: i64,
    /// Ticks the helmet has been breathing for them, and ticks in lava.
    breathing: u32,
    lava: u32,
    /// Air to take from the chest tank in the next level tick.
    owed: i32,
    dimension: &'static str,
    /// Where the player stood at their last tick: the warnings' sounds play there.
    position: [f64; 3],
}

static WORN: Mutex<BTreeMap<u128, Worn>> = Mutex::new(BTreeMap::new());

fn worn() -> std::sync::MutexGuard<'static, BTreeMap<u128, Worn>> {
    WORN.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn id_of(host: &dyn Host, uuid: u128, slot: EquipmentSlot) -> Option<ItemStack> {
    host.player_equipment(uuid, slot)
}

/// `BacktankUtil.canAbsorbDamage` for another module (the Extendo Grip): the worn tank pays `n` air if
/// it has any, and the answer says whether it did.
pub(crate) fn owe_air(uuid: u128, n: i32) -> bool {
    let mut all = worn();
    let Some(entry) = all.get_mut(&uuid) else {
        return false;
    };
    let has_air = entry.chest.as_ref().is_some_and(|(id, air)| tank::is_backtank(id) && *air - entry.owed > 0);
    if has_air {
        entry.owed += n;
    }
    has_air
}

impl Hooks for Diving {
    /// Reads the player's gear and what it asks for.
    fn player_tick(&self, host: &mut dyn Host, _: &mut PlayerTickEvent) {
        let Some(player) = host.acting_player() else {
            return;
        };
        let uuid = player.uuid;
        let helmet = id_of(&*host, uuid, EquipmentSlot::Head);
        let chest = id_of(&*host, uuid, EquipmentSlot::Chest);
        let legs = id_of(&*host, uuid, EquipmentSlot::Legs);
        let feet = id_of(&*host, uuid, EquipmentSlot::Feet);
        let helmet_id = helmet.as_ref().map(|s| s.item.to_string());
        let chest_air = chest.as_ref().map(|s| (s.item.to_string(), if tank::is_backtank(&s.item) { air_of(s) } else { 0 }));
        let legs_id = legs.as_ref().map(|s| s.item.to_string());
        let feet_id = feet.as_ref().map(|s| s.item.to_string());
        let relevant = helmet_id.as_deref().is_some_and(rules::is_diving_helmet)
            || chest_air.as_ref().is_some_and(|(id, _)| tank::is_backtank(id));
        if !relevant {
            worn().remove(&uuid);
            return;
        }
        let eye = [
            player.position[0].floor() as i32,
            (player.position[1] + f64::from(player.eye_height)).floor() as i32,
            player.position[2].floor() as i32,
        ];
        let feet_pos = [eye[0], player.position[1].floor() as i32, eye[2]];
        let in_lava = host.block(eye) == "minecraft:lava" || host.block(feet_pos) == "minecraft:lava";
        let chest_ref = chest_air.as_ref().map(|(id, air)| (id.as_str(), *air));
        let full_set = rules::full_netherite_set(helmet_id.as_deref(), chest_ref, legs_id.as_deref(), feet_id.as_deref());
        let now = util::game_time(&*host);
        let dimension = host.dimension();
        let mut all = worn();
        let entry = all.entry(uuid).or_default();
        entry.entity_id = player.entity_id as u32;
        entry.helmet = helmet_id;
        entry.chest = chest_air;
        entry.chest_stack = chest;
        entry.legs = legs_id;
        entry.feet = feet_id;
        entry.in_lava = in_lava;
        entry.full_set = full_set;
        entry.seen = now;
        entry.dimension = dimension;
        entry.position = player.position;
        // `breatheUnderwater` in lava: a netherite helmet and tank give a second of air for a second.
        if in_lava {
            entry.lava += 1;
            let chest_ref = entry.chest.as_ref().map(|(id, air)| (id.as_str(), *air));
            if rules::lava_drain(entry.helmet.as_deref(), chest_ref, true, entry.lava) {
                entry.owed += 1;
            }
        } else {
            entry.lava = 0;
        }
    }

    /// `DivingHelmetItem.breatheUnderwater` for water.
    fn living_breathe(&self, event: &mut LivingBreatheEvent<'_>) {
        if event.kind != "minecraft:player" {
            return;
        }
        let mut all = worn();
        let Some(entry) = all.values_mut().find(|w| w.entity_id == event.entity) else {
            return;
        };
        let helmet = entry.helmet.as_deref().is_some_and(rules::is_diving_helmet);
        let air = entry.chest.as_ref().map_or(0, |(_, air)| *air) - entry.owed;
        let Breath { can_breathe, refill, uses_air } = rules::breathe(event.can_breathe, helmet, air, entry.breathing);
        if refill.is_none() {
            entry.breathing = 0;
            return;
        }
        entry.breathing += 1;
        event.can_breathe = can_breathe;
        if let Some(refill) = refill {
            event.refill = refill;
        }
        if uses_air {
            entry.owed += 1;
        }
    }

    /// `setFireImmune`: a fully dressed player takes no fire damage.
    fn living_incoming_damage_v3(&self, event: &mut LivingIncomingDamageV3Event<'_>) -> Flow {
        if !rules::is_fire_damage(event.damage_type) {
            return Flow::Continue;
        }
        let Some(uuid) = event.victim_uuid else {
            return Flow::Continue;
        };
        if worn().get(&uuid).is_some_and(|w| w.full_set) { Flow::Cancel } else { Flow::Continue }
    }

    /// `BacktankUtil.consumeAir`: the air a worn tank owes comes out of its stack, with the warnings.
    fn level_tick_post(&self, host: &mut dyn Host, event: &mut LevelTickPostEvent<'_>) {
        let now = util::game_time(&*host);
        let jobs: Vec<(u128, i32, ItemStack, [f64; 3])> = worn()
            .iter()
            .filter(|(_, w)| w.owed > 0 && w.dimension == event.dimension && now - w.seen <= 3)
            .filter_map(|(uuid, w)| Some((*uuid, w.owed, w.chest_stack.clone()?, w.position)))
            .collect();
        for (uuid, owed, stack, position) in jobs {
            if !tank::is_backtank(&stack.item) {
                if let Some(w) = worn().get_mut(&uuid) {
                    w.owed = 0;
                }
                continue;
            }
            let max = tank::max_air(0);
            let air = air_of(&stack);
            let new_air = tank::consume(air, max, owed);
            let mut new_stack = stack.clone();
            if host.set_component(&mut new_stack, tank::COMPONENT, Tag::Int(new_air)).is_err() {
                continue;
            }
            let done = host
                .set_inventory_slot_typed(uuid, NativeInventoryKind::Player, CHEST, Some(&stack), Some(new_stack.clone()))
                .is_ok();
            if !done {
                continue;
            }
            if let Some(w) = worn().get_mut(&uuid) {
                w.owed = 0;
                w.chest_stack = Some(new_stack);
                if let Some((_, a)) = &mut w.chest {
                    *a = new_air;
                }
            }
            for warning in tank::warnings(air, new_air, max) {
                let line = match warning {
                    tank::Warning::Low => "Warning: backtank low on air",
                    tank::Warning::Depleted => "Warning: backtank depleted",
                };
                host.action_bar(uuid, Text::literal(line.to_owned()));
                // `BacktankUtil.sendWarning`: `DENY.play(.., 1, 1.25f)` and `STEAM.play(.., .5f, .5f)` at the player's block.
                let at = position.map(|v| v.floor() + 0.5);
                util::create_sound(host, "deny", at, 1.0, 1.25);
                util::create_sound(host, "steam", at, 0.5, 0.5);
            }
        }
        // Forget players that stopped ticking.
        worn().retain(|_, w| now - w.seen <= 40);
    }

    /// `DivingBootsItem.accelerateDescentUnderwater`: a push down each tick, on the client that moves the player.
    fn client_tick_post(&self, host: &mut dyn Host, _: &mut ClientTickPostEvent) {
        if !host.window() {
            return;
        }
        let Some(boots) = host.equipment(EquipmentSlot::Feet) else {
            return;
        };
        if !rules::is_diving_boots(&boots.item) {
            return;
        }
        let Some(player) = host.player() else {
            return;
        };
        let at = [player.x.floor() as i32, (player.y + 0.1).floor() as i32, player.z.floor() as i32];
        if rules::sinks(true, host.block(at) == "minecraft:water", false, false) {
            host.push_player([0.0, -rules::SINK, 0.0]);
        }
    }
}
