//! The Item Hatch (`create:item_hatch`) (`ItemHatchBlock`, `ItemHatchBlockEntity`, `ItemHatchHandler`, `AllShapes.ITEM_HATCH`): a hatch
//! in a wall that puts what a player hands it into the inventory behind it. The decision is `rules::item_hatch::use_hatch`; this
//! reads the player and the inventory, and moves the items.
//!
//! How it works here:
//! - Right-click it with an item: the whole stack goes into the inventory it faces, as much as fits. Right-click it sneaking
//!   (a wrench aside): everything in the main inventory outside the hotbar goes in. Shulker boxes stay with the player; packages go in.
//! - The hatch opens for ten ticks (`level.scheduleTick(pos, this, 10)`) and the chat line says what was deposited.
//! - It can't be placed on the top or bottom of a block, and faces the block it was put against.
//!
//! Known differences from Create:
//! - The filter slot (`HatchFilterSlot`, a value box) is not offered: the hatch takes everything.
//! - The player's stack is removed by item name (`Host::take_items`), from any slot that holds the item, so with the same item in the
//!   hotbar and the inventory the hotbar one may go first; a stack with components goes in only from the hand, and one package at a
//!   time. The inventory behind it must be one the packager glue knows the size of (chests, barrels, shulker boxes, hoppers, droppers,
//!   the Item Vault).
//! - No waterlogging, and the Wrench only picks it up.

use create_kinetics::Pos;
use create_kinetics::rules::item_hatch::{self as rules, HatchUse, Player, Slot};
use crate::compat::{
    BlockEntityEvent, BlockEntityTickEvent, BlockEntityTicksEvent, BlockStateId, Compound, Flow, Hooks, Host, ItemStack, LevelHost,
    RightClickBlockEvent, StateForPlacementEvent, Tag, Text, UseItemOnEvent,
};

use super::chute_inv as inv;
use super::package as items;
use super::packager_inv as target_inv;
use crate::util;

pub const ID: &str = "create:item_hatch";
/// The game time the hatch opened at (this port's own: Create schedules a block tick).
const OPENED_AT: &str = "OpenedAt";
/// `ItemHandler`'s slot limit, as `packager_inv` counts it.
const SLOT_LIMIT: u32 = 64;

struct Hatches;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Hatches)
}

/// The slots of the inventory at `target`, as the rules see them (`None` where its size isn't known).
fn slots_of(host: &mut dyn Host, target: Pos) -> Option<Vec<Slot<ItemStack>>> {
    let capacity = target_inv::capacity(&*host, target)?;
    let mut slots: Vec<Slot<ItemStack>> = (0..capacity).map(|_| Slot { stack: None, limit: SLOT_LIMIT }).collect();
    for (slot, stack) in inv::items_of(host, target)? {
        if let Some(place) = slots.get_mut(slot) {
            place.stack = Some((items::key(&stack), stack.count));
        }
    }
    Some(slots)
}

/// `ItemHatchBlock.useItemOn`, on the server.
fn deposit(host: &mut dyn Host, pos: Pos, uuid: u128, sneaking: bool, holding_wrench: bool) -> Flow {
    let Some(facing) = util::facing(&*host, pos, "facing") else {
        return Flow::Continue;
    };
    let target = util::add(pos, facing.normal());
    let Some(held) = slots_of(host, target) else {
        // `ItemInteractionResult.FAIL`: no inventory behind it.
        return Flow::Cancel;
    };
    let stacks = host.player_stacks(uuid).unwrap_or_default();
    let in_hand = host.held_stack(uuid);
    // `inventory.selected`: the hotbar slot that holds what the hand holds.
    let selected = in_hand
        .as_ref()
        .and_then(|hand| stacks.iter().take(rules::HOTBAR_SLOTS).position(|slot| slot.as_ref() == Some(hand)))
        .unwrap_or(usize::MAX);
    let player_items: Vec<Option<(ItemStack, u32)>> = stacks
        .iter()
        .enumerate()
        .map(|(index, slot)| {
            let stack = slot.as_ref()?;
            // A stack with components can be taken from the hand only.
            if stack.extra.is_some() && index != selected {
                return None;
            }
            Some((items::key(stack), stack.count))
        })
        .collect();
    let max = |key: &ItemStack| host.max_stack_size(&key.item).unwrap_or(64);
    let fits = |key: &ItemStack| !key.item.ends_with("shulker_box");
    let player = Player { items: &player_items, selected, sneaking, holding_wrench };
    let used = rules::use_hatch(&player, Some(&held), Some(None), &max, &fits);
    let HatchUse::Used(result) = used else {
        return if matches!(used, HatchUse::PassToDefault) { Flow::Continue } else { Flow::Cancel };
    };
    let face = facing.opposite();
    let mut inserted_any = false;
    for (slot, count) in &result.removed {
        let Some(Some(stack)) = stacks.get(*slot) else {
            continue;
        };
        let mut offered = stack.clone();
        offered.count = *count;
        let rest = inv::insert(host, target, face, &offered, false).map_or(*count, |rest| rest.count);
        let moved = count - rest.min(*count);
        if moved == 0 {
            continue;
        }
        inserted_any = true;
        if stack.extra.is_some() {
            // A package from the hand, which stacks to one.
            for _ in 0..moved {
                items::shrink_held(uuid, &stack.item);
            }
        } else {
            host.take_items(&stack.item, moved);
        }
    }
    if !inserted_any {
        return Flow::Cancel;
    }
    let now = util::game_time(&*host);
    host.play_sound("create:item_hatch", util::centre(pos), 2, 1.0, 1.0);
    host.set_block_property(pos, "open", "true");
    host.change_mod_block_entity(pos, &mut |entity| {
        util::put(&mut entity.nbt, OPENED_AT, Tag::Long(now));
    });
    host.action_bar(
        uuid,
        Text::literal(if result.deposit_item_in_hand { "Held item deposited" } else { "Inventory items deposited" }.to_owned()),
    );
    Flow::Cancel
}

impl Hooks for Hatches {
    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        (event.block == ID).then(|| (event.block.to_owned(), Compound::new()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    /// `ItemHatchBlock.getStateForPlacement`: not on the top or bottom of a block; it faces the block it is put against.
    fn state_for_placement(&self, level: &dyn LevelHost, event: &mut StateForPlacementEvent<'_>) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != ID {
            return None;
        }
        let clicked = util::dir(context.face);
        let Some(facing) = rules::placement_facing(clicked) else {
            return Some(None);
        };
        let state = level.state_with_property(event.state, "facing", facing.name())?;
        Some(level.state_with_property(state, "open", "false"))
    }

    /// `ItemHatchBlock.tick`: after ten ticks the hatch closes.
    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        let pos = event.pos;
        if host.block_property(pos, "open").as_deref() != Some("true") {
            return;
        }
        let now = util::game_time(&*host);
        if now - util::long(&event.entity.nbt, OPENED_AT) >= rules::CLOSE_DELAY {
            host.set_block_property(pos, "open", "false");
        }
    }

    /// An empty hand: `useItemOn` is called with the empty stack, so a sneaking click deposits the inventory.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR || host.held_stack(player.uuid).is_some() {
            return Flow::Continue;
        }
        if !util::first_click(&*host, player.uuid, pos) {
            return Flow::Cancel;
        }
        deposit(host, pos, player.uuid, player.sneaking, false)
    }

    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        let wrench = &*event.stack.item == util::WRENCH;
        if wrench && player.sneaking {
            return util::wrench(host, pos, |_| {});
        }
        if !util::first_click(&*host, player.uuid, pos) {
            return Flow::Cancel;
        }
        deposit(host, pos, player.uuid, player.sneaking, wrench)
    }
}
