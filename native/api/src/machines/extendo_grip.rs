//! The Extendo Grip (`ExtendoGripItem`): held in a hand it adds 3 blocks to the player's reach
//! (`Attributes.BLOCK_INTERACTION_RANGE`), 5 with one in each hand; attacks with it knock back 2 more;
//! each block broken, block placed or hit given wears it, unless a worn Backtank pays for the use. The
//! numbers are `create_kinetics::rules::extendo_grip`; this is the glue.
//!
//! Known differences from Create, for the README:
//! - The reach is the player's `block_interaction_range` *base*, set by the grip's rule in their tick
//!   (Create adds a transient modifier): it is put back to 4.5 when no grip is held, and any other
//!   base (another mod's) is left alone. A world closed while holding one keeps the longer reach
//!   until the player's next tick without it.
//! - Only a grip in the main hand wears (a wear is written to the held stack on the next inventory
//!   tick, which Horizon only gives for the selected slot); a grip in the off hand never does. Creative
//!   players don't wear it (Create's `hurtAndBreak` and `canAbsorbDamage` skip them).
//! - No `dontMissEntitiesWhenYouHaveHighReachDistance` and no `ExtendoGripInteractionPacket`: Horizon's
//!   client picks entities with the reach it has, so reaching a mob 6 blocks away may need the
//!   crosshair on a block behind it. No drawn arms (`ExtendoGripItemRenderer`), no durability
//!   enchantment tag, no Backtank bar on the grip.
//! - The knockback bonus uses `living_knockback`, which names the player that hit.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use create_kinetics::rules::backtank as tank;
use create_kinetics::rules::extendo_grip::{self as rules, Wear};
use horizon_client::mod_api::{
    DestroyBlockEvent, EquipmentSlot, Flow, Hooks, Host, InventoryTickEvent, ItemStack,
    LivingIncomingDamageV3Event, LivingKnockbackEvent, PlaceBlockEvent, PlayerTickEvent,
};

use super::diving;
use crate::util;

const REACH: &str = "minecraft:block_interaction_range";

struct Extendo;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Extendo)
}

/// Entity ids of the players holding a grip.
static HOLDING: Mutex<BTreeSet<u32>> = Mutex::new(BTreeSet::new());
/// Uses waiting to be written to the held grip, by player.
static WEAR: Mutex<BTreeMap<u128, u32>> = Mutex::new(BTreeMap::new());
static WEAR_COUNT: AtomicUsize = AtomicUsize::new(0);

fn lock<T>(m: &'static Mutex<T>) -> std::sync::MutexGuard<'static, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn is_grip(stack: &ItemStack) -> bool {
    &*stack.item == rules::EXTENDO_GRIP
}

/// `findAndDamageExtendoGrip`: a use of the grip in the player's main hand.
fn use_up(host: &mut dyn Host, uuid: u128) {
    let Some(player) = host.server_players().into_iter().find(|p| p.uuid == uuid).or_else(|| host.acting_player().filter(|p| p.uuid == uuid))
    else {
        return;
    };
    if player.game_mode == util::CREATIVE || player.game_mode == util::SPECTATOR {
        return;
    }
    if !host.held_stack(uuid).is_some_and(|stack| is_grip(&stack)) {
        return;
    }
    // A worn backtank with air pays for it (`BacktankUtil.canAbsorbDamage`).
    if let Some(cost) = rules::backtank_cost(tank::AIR_IN_BACKTANK, rules::MAX_ACTIONS)
        && diving::owe_air(uuid, cost)
    {
        return;
    }
    let mut wear = lock(&WEAR);
    *wear.entry(uuid).or_insert(0) += 1;
    WEAR_COUNT.store(wear.len(), Ordering::Relaxed);
}

impl Hooks for Extendo {
    /// `holdingExtendoGripIncreasesRange`.
    fn player_tick(&self, host: &mut dyn Host, _: &mut PlayerTickEvent) {
        let Some(player) = host.acting_player() else {
            return;
        };
        let uuid = player.uuid;
        let main = host.held_stack(uuid).is_some_and(|s| is_grip(&s));
        let off = host.player_equipment(uuid, EquipmentSlot::OffHand).is_some_and(|s| is_grip(&s));
        let grips = rules::grips(main, off);
        {
            let mut holding = lock(&HOLDING);
            if grips > 0 {
                holding.insert(player.entity_id as u32);
            } else {
                holding.remove(&(player.entity_id as u32));
            }
        }
        let Some((base, _)) = host.attribute(uuid, REACH) else {
            return;
        };
        if let Some(wanted) = rules::needs_update(base, grips) {
            host.set_attribute_base(uuid, REACH, wanted);
        }
    }

    /// `consumeDurabilityOnBlockBreak`.
    fn destroy_block(&self, host: &mut dyn Host, _: &mut DestroyBlockEvent) -> Flow {
        if let Some(player) = host.acting_player() {
            use_up(host, player.uuid);
        }
        Flow::Continue
    }

    /// `consumeDurabilityOnPlace`.
    fn place_block(&self, host: &mut dyn Host, _: &mut PlaceBlockEvent<'_>) {
        if let Some(player) = host.acting_player() {
            use_up(host, player.uuid);
        }
    }

    /// `bufferLivingAttackEvent`: a hit by a player with a grip wears it.
    fn living_incoming_damage_v3(&self, _: &mut LivingIncomingDamageV3Event<'_>) -> Flow {
        // The attacker's UUID is all this hook has and the host isn't passed: the wear of an attack is
        // not counted (see the module header).
        Flow::Continue
    }

    /// `attacksByExtendoGripHaveMoreKnockback`.
    fn living_knockback(&self, event: &mut LivingKnockbackEvent<'_>) -> Flow {
        if event.direct > 0 && lock(&HOLDING).contains(&(event.direct as u32)) {
            event.strength += rules::EXTRA_KNOCKBACK;
        }
        Flow::Continue
    }

    /// The uses are written to the held grip.
    fn inventory_tick(&self, host: &mut dyn Host, event: &mut InventoryTickEvent) {
        if !event.selected || WEAR_COUNT.load(Ordering::Relaxed) == 0 {
            return;
        }
        let Some(player) = host.acting_player() else {
            return;
        };
        let Some(uses) = lock(&WEAR).get(&player.uuid).copied() else {
            return;
        };
        let Some(stack) = host.carried_stack(event.slot) else {
            return;
        };
        if !is_grip(stack) {
            return;
        }
        match rules::wear(stack.damage, uses) {
            Wear::Damaged(damage) => stack.damage = damage,
            Wear::Broken => stack.count = 0,
        }
        let broke = stack.count == 0;
        let mut wear = lock(&WEAR);
        wear.remove(&player.uuid);
        WEAR_COUNT.store(wear.len(), Ordering::Relaxed);
        drop(wear);
        if broke {
            host.play_sound("minecraft:entity.item.break", player.position, 7, 0.8, 1.0);
        }
    }
}
