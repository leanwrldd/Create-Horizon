//! The Smart Observer (`SmartObserverBlock`, `SmartObserverBlockEntity`, `SmartObserverGenerator`,
//! `DirectedDirectionalBlock`): a block that looks at the block it is attached to and lights up,
//! giving a redstone signal on every side but the one it looks at, while that block holds
//! something that passes its filter. It reads an inventory (any item, or the filtered one), a belt
//! or a depot (an item passing the middle of a block), a fluid tank (any fluid, or the filtered
//! one) and the block itself (a filter that is that block's item). The rules are
//! `create_kinetics::rules::content_observer` and `content_observer_target`; the shared glue is
//! `content_observer_target.rs`.
//!
//! Known differences from Create:
//! - The filter slot is Create's value box (`machines/value_box.rs`). A bucket or bottle of a fluid is a filter
//!   for that fluid. The block entity keeps Create's keys (`Filter`, `FilterAmount`, `UpTo`, `TurnOff`).
//! - An inventory is looked at every other tick (Create waits for its contents to change,
//!   `VersionedInventoryTrackerBehaviour`), a belt or depot every tick. A belt that isn't turning
//!   is not read (the belt module's `edit_items` skips it).
//! - Not detected: fluid pipes (`FluidTransportBehaviour.getFlow`), packages looping on a chain
//!   conveyor, and an item passing through a funnel (`onFunnelTransfer`): the funnels and chutes
//!   here don't tell the observer, and there are no chain conveyors.
//! - The block the observer looks at counts as itself when the filter is the item of its block
//!   (`new ItemStack(block)`): the block's name is read as the item's, and a shulker box's
//!   contents are not added (only the item is compared, as Create does without NBT).
//! - Placing it turns it to what it can read just after it is placed (see
//!   `content_observer_target.rs`); `canConnectRedstone` (redstone dust not bending towards the
//!   face it looks at) is not done.
//! - The signal's timing is polled: the block lights on the tick it is found and goes dark the
//!   tick after its countdown ends, as Create's scheduled tick does, but neighbour updates come
//!   from Horizon's `set_block_property`.

use create_kinetics::rules::content_observer::{self as rules, BLOCK_DELAY, DEFAULT_DELAY};
use create_kinetics::{Dir, Pos};
use crate::compat::{
    BlockEntityEvent, BlockEntityTickEvent, BlockEntityTicksEvent, BlockSignalEvent, BlockStateId,
    Compound, Flow, Hooks, Host, IsSignalSourceEvent, LevelHost, PlaceBlockEvent,
    StateForPlacementEvent, Tag, UseItemOnEvent,
};

use super::chute_inv::Filter;
use super::content_observer_target::{self as target, OBSERVER, Which};
use super::diodes_signal as signal;
use crate::{belt, depot, float, int, util};

/// `SmartObserverBlockEntity.turnOffTicks`, saved as Create's `write` does.
const TURN_OFF: &str = "TurnOff";
/// Ticks between looks at an inventory or a tank.
const SCAN_RATE: i64 = 2;
/// `HeldItem` and `Pos` of a depot's saved item (`depot.rs`).
const HELD: &str = "HeldItem";
const POS: &str = "Pos";

struct Observers;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Observers)
}

fn new_entity() -> Compound {
    let mut nbt = Compound::new();
    super::chute_inv::write_filter(
        &mut nbt,
        &Filter { item: None, amount: create_kinetics::rules::chute::Amount::default() },
    );
    nbt.insert(TURN_OFF.into(), Tag::Int(0));
    nbt
}

/// What the observer at `pos` found this tick: the number of ticks to stay lit, if it found
/// something.
fn detect(host: &mut dyn Host, pos: Pos, now: i64, turn_off: i32, filter: &Filter) -> Option<i32> {
    let (_, kind, facing) = target::view(&*host, pos)?;
    let (at, _) = target::target_pos(pos, kind, facing);

    // The block itself (`filtering.test(new ItemStack(block))`).
    if let Some(wanted) = &filter.item {
        let block = host.block(at);
        if *wanted.item == *block {
            return Some(BLOCK_DELAY);
        }
    }

    // Items on a belt or a depot: an item in the middle of the block.
    let name = host.block(at);
    if name == belt::ID {
        let mut found = false;
        belt::edit_items(host, at, &mut |_, lane, items| {
            found = items
                .iter()
                .any(|item| rules::on_block(lane.index, item.pos) && filter.test_static(&item.stack));
            false
        });
        // `turnOffTicks == 6`: an item seen the tick the block lit does not light it again.
        return (found && turn_off != DEFAULT_DELAY).then_some(DEFAULT_DELAY);
    }
    if name == depot::ID {
        let seen = host.mod_block_entity(at).is_some_and(|entity| {
            let Some(stack) = entity.items.first().cloned().flatten() else {
                return false;
            };
            let along = match util::get(&entity.nbt, HELD) {
                Some(Tag::Compound(held)) => float(held, POS).unwrap_or(0.5),
                _ => 0.5,
            };
            rules::depot_in_reach(along) && filter.test(host, &stack)
        });
        return (seen && turn_off != DEFAULT_DELAY).then_some(DEFAULT_DELAY);
    }

    if now.rem_euclid(SCAN_RATE) != 0 {
        return None;
    }
    // An inventory, then a tank.
    if target::inventory_has(host, at, filter) == Some(true) {
        return Some(DEFAULT_DELAY);
    }
    if let Some(tanks) = target::read_tanks(&*host, at)
        && tanks.iter().any(|tank| {
            tank.amount > 0
                && tank.fluid.as_deref().is_some_and(|fluid| target::passes_fluid(&filter, fluid))
        })
    {
        return Some(DEFAULT_DELAY);
    }
    None
}

fn tick(host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
    let pos = event.pos;
    let powered = host.block_property(pos, "powered").as_deref() == Some("true");
    let mut turn_off = int(&event.entity.nbt, TURN_OFF);
    let now = util::game_time(host);

    // The level's scheduled tick, then this block entity's countdown.
    let (dark, next) = rules::step(powered, turn_off);
    let mut powered = powered;
    if dark {
        host.set_block_property(pos, "powered", "false");
        powered = false;
    }
    turn_off = next;

    let filter = super::chute_inv::filter_of(&event.entity.nbt);
    if let Some(ticks) = detect(host, pos, now, turn_off, &filter) {
        let (counter, light) = rules::activate(powered, ticks);
        turn_off = counter;
        if light {
            host.set_block_property(pos, "powered", "true");
        }
    }
    if int(&event.entity.nbt, TURN_OFF) != turn_off {
        event.entity.nbt.insert(TURN_OFF.into(), Tag::Int(turn_off));
    }
}

impl Hooks for Observers {
    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == OBSERVER).then(|| (OBSERVER.to_owned(), new_entity()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == OBSERVER {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id == OBSERVER {
            tick(host, event);
        }
    }

    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        target::placed_state(level, event, OBSERVER)
    }

    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if &*event.stack.item == OBSERVER && host.block(event.pos) == OBSERVER {
            target::orient(host, event.pos);
        }
    }

    /// The Wrench turns the observer (`DirectedDirectionalBlock.getRotatedBlockState`).
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        if host.block(pos) != OBSERVER {
            return Flow::Continue;
        }
        if &*event.stack.item == util::WRENCH {
            let clicked = Dir::from_normal(event.face);
            return util::wrench(host, pos, |host| {
                if let Some(clicked) = clicked {
                    target::wrench_turn(host, pos, clicked);
                }
            });
        }
        Flow::Continue
    }

    /// `SmartObserverBlock.getSignal`: 15 on every side but the one it looks at, while lit; never
    /// a direct (strong) signal.
    fn block_signal(&self, level: &dyn LevelHost, event: &mut BlockSignalEvent) {
        if let Some(looks) = target::looks(level, event.state)
            && looks.which == Which::Observer
            && !event.direct
        {
            event.signal = create_kinetics::rules::content_observer_target::signal_towards(
                util::dir(event.face),
                looks.direction(),
                looks.powered,
            );
        }
        signal::mask(event);
    }

    /// `SmartObserverBlock.isSignalSource`: while lit.
    fn is_signal_source(&self, level: &dyn LevelHost, event: &mut IsSignalSourceEvent) {
        if let Some(looks) = target::looks(level, event.state)
            && looks.which == Which::Observer
            && looks.powered
        {
            event.source = true;
        }
    }
}
