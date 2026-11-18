//! The Creative Crate (`CreativeCrateBlock`, `CreativeCrateBlockEntity`, `BottomlessItemHandler`,
//! `WrenchableDirectionalBlock`): an endless supply of the item set in its filter. Anything
//! taking from it (a hopper, a chute, a funnel) gets full stacks without end, and anything put in
//! is voided. The rules are `create_kinetics::rules::creative_crate`; this is the glue.
//!
//! Horizon moves items only between real slots, so the crate keeps two (`rules::SLOTS`): the
//! first holds a full stack of the filter item and is topped up every tick, and the second takes
//! anything and is emptied every tick (a hopper pushing an item at it sees it vanish).
//!
//! Known differences from Create:
//! - There is no filter slot with its value box yet. As a stand-in, right-click the crate with an
//!   item to set that item as its supply, and right-click it with an empty hand to clear it.
//!   Sneak-right-click with an item in hand still places that item against the crate. The
//!   Wrench is left to its own jobs (turning the crate, sneaking to pick it up). The filter
//!   keeps Create's keys (`Filter`, `FilterAmount`, `UpTo`); a component-bearing item keeps its
//!   components.
//! - The crate's contents are real slots, so a comparator beside it reads them as it reads any
//!   container (Create's crate gives no signal), a hopper takes from the top slot as one item
//!   at a time like any other, and a crate broken by anything but a player (an explosion)
//!   drops a stack of its item. A player breaking it empties it first.
//! - Right-clicking the crate with a block in hand may briefly show that block placed on the
//!   client (the known client prediction gap, see the skill's notes).
//! - Create's crate also feeds the Schematicannon beside it; there is no Schematicannon here.
//! - No tooltip text, and no advancement.

use create_kinetics::rules::chute::Amount;
use create_kinetics::rules::creative_crate as rules;
use create_kinetics::{Dir, looking};
use crate::compat::{
    BlockEntityContainerEvent, BlockEntityEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockStateId, Compound, DestroyBlockEvent, EquipmentSlot, Flow, Hooks, Host, ItemStack,
    LevelHost, ModContainer, RightClickBlockEvent, StateForPlacementEvent,
    UseItemOnEvent,
};

use super::chute_inv::{self as inv, Filter};
use crate::util;

pub const ID: &str = "create:creative_crate";

struct Crates;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Crates)
}

/// Both slots reachable from every side; hoppers put only into the void slot and take only from
/// the supply (`BottomlessItemHandler.insertItem` and `extractItem`).
fn container() -> ModContainer {
    let both: Vec<usize> = (0..rules::SLOTS).collect();
    ModContainer {
        slots: rules::SLOTS,
        faces: [(); 6].map(|()| both.clone()),
        inputs: vec![rules::VOID],
        outputs: vec![rules::SUPPLY],
        menu: None,
    }
}

/// The two slots the crate should hold now: a full stack of the filter item, and nothing.
fn wanted(host: &dyn Host, filter: &Filter) -> Vec<Option<ItemStack>> {
    let supply = filter.item.as_ref().and_then(|item| {
        let max = host.max_stack_size(&item.item).unwrap_or(64);
        let count = rules::shown(rules::SUPPLY, true, max)?;
        let mut stack = item.clone();
        stack.count = count;
        Some(stack)
    });
    vec![supply, None]
}

fn new_entity() -> Compound {
    let mut nbt = Compound::new();
    inv::write_filter(&mut nbt, &Filter { item: None, amount: Amount::default() });
    nbt
}

/// Sets the filter of the crate at `pos` (`FilteringBehaviour.setFilter`), and tops up its slots.
fn set_filter(host: &mut dyn Host, pos: [i32; 3], item: Option<ItemStack>) -> bool {
    let Some(entity) = host.mod_block_entity(pos) else {
        return false;
    };
    let filter = Filter { item, amount: Amount::default() };
    if inv::filter_of(&entity.nbt) != filter {
        let items = wanted(&*host, &filter);
        host.change_mod_block_entity(pos, &mut |e| {
            inv::write_filter(&mut e.nbt, &filter);
            e.items = items.clone();
        });
    }
    true
}

/// What the stand-in for the filter slot says to the player.
fn describe(item: &Option<ItemStack>) -> String {
    match item {
        Some(stack) => format!("Infinite Supply: {}", stack.item),
        None => "Infinite Supply cleared".to_owned(),
    }
}

impl Hooks for Crates {
    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == ID).then(|| (ID.to_owned(), new_entity()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    fn block_entity_container(
        &self,
        _: &dyn Host,
        event: &mut BlockEntityContainerEvent<'_>,
    ) -> Option<ModContainer> {
        (event.block == ID).then(container)
    }

    /// Keeps the supply a full stack and the void slot empty.
    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        let filter = inv::filter_of(&event.entity.nbt);
        let mut items = wanted(&*host, &filter);
        let mut have = event.entity.items.clone();
        have.resize(rules::SLOTS, None);
        if have != items {
            std::mem::swap(&mut event.entity.items, &mut items);
        }
    }

    /// `CrateBlock` is a block that can be taken by a player: its supply must not drop.
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) == ID {
            host.change_mod_block_entity(pos, &mut |e| {
                if e.items.iter().any(Option::is_some) {
                    e.items = vec![None; rules::SLOTS];
                }
            });
        }
        Flow::Continue
    }

    /// The stand-in for the filter slot: an item sets the supply, an empty hand clears it.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
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
        let held = host.player_equipment(player.uuid, EquipmentSlot::MainHand);
        match held {
            // The Wrench turns the crate and picks it up (`FilteringBehaviour.canShortInteract`).
            Some(stack) if &*stack.item == util::WRENCH => Flow::Continue,
            // Sneaking with something in hand places it against the crate.
            Some(_) if player.sneaking => Flow::Continue,
            Some(mut stack) => {
                stack.count = 1;
                if util::first_click(host, player.uuid, pos)
                    && set_filter(host, pos, Some(stack.clone()))
                {
                    inv::tell(host, &describe(&Some(stack)));
                }
                Flow::Cancel
            }
            None => {
                // An item in the off hand is that hand's click: the main hand's turn is only
                // seen as empty.
                if host.player_equipment(player.uuid, EquipmentSlot::OffHand).is_some()
                    && player.sneaking
                {
                    return Flow::Continue;
                }
                if util::first_click(host, player.uuid, pos) && set_filter(host, pos, None) {
                    inv::tell(host, &describe(&None));
                }
                Flow::Cancel
            }
        }
    }

    /// `WrenchableDirectionalBlock.getRotatedBlockState`.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        if &*event.stack.item != util::WRENCH || host.block(pos) != ID {
            return Flow::Continue;
        }
        let clicked = Dir::from_normal(event.face).map(Dir::axis);
        util::wrench(host, pos, |host| {
            let (Some(facing), Some(clicked)) = (util::facing(&*host, pos, "facing"), clicked) else {
                return;
            };
            let next = rules::wrenched(facing, clicked);
            if next != facing {
                host.set_block_property(pos, "facing", next.name());
            }
        })
    }

    /// `WrenchableDirectionalBlock.getStateForPlacement`: facing the way the player looks.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != ID {
            return None;
        }
        let facing = rules::placement_facing(looking(context.yaw, context.pitch));
        level.state_with_property(event.state, "facing", facing.name()).map(Some)
    }
}
