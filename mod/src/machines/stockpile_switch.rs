//! The Threshold Switch (`create:stockpile_switch`: `ThresholdSwitchBlock`,
//! `ThresholdSwitchBlockEntity`, `ThresholdSwitchGenerator`, `DirectedDirectionalBlock`): a block
//! that looks at the inventory or the tank it is attached to, reads how full it is, and gives a
//! redstone signal on every side but the one it looks at: on once the contents reach the upper
//! threshold, off again once they fall to the lower one. The glass shows how full the block is
//! (`level` 0 to 5). The rules are `create_kinetics::rules::stockpile_switch`; the glue it shares
//! with the Smart Observer is `content_observer_target.rs`.
//!
//! Known differences from Create:
//! - There is no screen. As a stand-in, right-click the switch (with an empty hand or with any
//!   item but the Wrench and the Redstone Torch) to step the thresholds through Create's defaults
//!   (turn on at 128, off at 64), then quarter-to-half, half-to-three-quarters and
//!   three-quarters-to-full of what the block holds (whole stacks of items, whole buckets of
//!   fluid), then "on with anything, off when empty"; the action bar says where it is and how
//!   full the block is. A Redstone Torch inverts the signal (the screen's flip button). The filter
//!   is Create's filter slot (a value box, `machines/value_box.rs`; a bucket of a fluid filters that
//!   fluid). The Wrench turns the switch, as Create's does,
//!   and sneaking with it picks the switch up. The saved keys are Create's (`OnAboveAmount`,
//!   `OffBelowAmount`, `Inverted`, `CurrentAmount`, `CurrentMinAmount`, `CurrentMaxAmount`,
//!   `Powered`, `PoweredAfterDelay`, `Filter`); the thresholds are saved as ints, not floats.
//! - Not read: a belt or a depot (Create reads them as one-slot inventories), the rope pulley's
//!   depth (`ThresholdSwitchObservable`), and the storage mods Create has compatibility with.
//!   There is no Display Link.
//! - It looks at its block every 10 ticks (`setLazyTickRate(10)`) as Create does, and straight
//!   away when it is placed; the signal changes two ticks after the state does
//!   (`scheduleBlockTick`), counted in the block entity's tick, not by the level's scheduler.
//! - What the switch reads, the redstone it gives (a hook can't read a block entity, so a table
//!   the block entity fills) and placing it are as the Smart Observer's; see
//!   `content_observer.rs` and `content_observer_target.rs`. `canConnectRedstone` is not done.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{LazyLock, Mutex};

use create_kinetics::rules::stockpile_switch::{self as rules, Kind, Level};
use create_kinetics::{Dir, Pos};
use crate::compat::{
    BlockEntityEvent, BlockEntityTickEvent, BlockEntityTicksEvent, BlockSignalEvent, BlockStateId,
    Compound, EquipmentSlot, Flow, Hooks, Host, IsSignalSourceEvent, LevelHost, ModBlockEntity,
    PlaceBlockEvent, RightClickBlockEvent, ServerTickEvent, ServerTickPhase,
    StateForPlacementEvent, Tag, UseItemOnEvent,
};

use super::chute_inv::{self as inv, Filter};
use super::content_observer_target::{self as target, SWITCH, Which};
use super::diodes_signal as signal;
use crate::{int, util};

const ON_ABOVE: &str = "OnAboveAmount";
const OFF_BELOW: &str = "OffBelowAmount";
const INVERTED: &str = "Inverted";
const CURRENT: &str = "CurrentAmount";
const CURRENT_MIN: &str = "CurrentMinAmount";
const CURRENT_MAX: &str = "CurrentMaxAmount";
const POWERED: &str = "Powered";
const POWERED_AFTER_DELAY: &str = "PoweredAfterDelay";
/// Not Create's: ticks until `updatePowerAfterDelay` (the level's scheduled tick), and whether the
/// block has yet to look at what it is placed against (`onPlace`).
const POWER_DELAY: &str = "PowerDelay";
const FRESH: &str = "Fresh";
const REDSTONE_TORCH: &str = "minecraft:redstone_torch";
/// Ticks a block may miss before its table entry counts as gone.
const MAX_AGE: i64 = 3;

struct Switches;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Switches)
}

fn flag(value: bool) -> Tag {
    Tag::Byte(i8::from(value))
}

fn new_entity() -> Compound {
    let mut nbt = Compound::new();
    inv::write_filter(&mut nbt, &Filter { item: None, amount: Default::default() });
    nbt.insert(ON_ABOVE.into(), Tag::Int(rules::ON_ABOVE));
    nbt.insert(OFF_BELOW.into(), Tag::Int(rules::OFF_BELOW));
    nbt.insert(INVERTED.into(), flag(false));
    nbt.insert(CURRENT.into(), Tag::Int(-1));
    nbt.insert(CURRENT_MIN.into(), Tag::Int(0));
    nbt.insert(CURRENT_MAX.into(), Tag::Int(0));
    nbt.insert(POWERED.into(), flag(false));
    nbt.insert(POWERED_AFTER_DELAY.into(), flag(false));
    nbt.insert(POWER_DELAY.into(), Tag::Int(0));
    nbt.insert(FRESH.into(), flag(true));
    nbt
}

// --- the signal table ---------------------------------------------------------------------------------

/// What each ticking switch gives (`poweredAfterDelay`) and when it last ticked, by level and
/// position: a signal hook can read only the level's blocks, not a block entity.
static SIGNALS: LazyLock<Mutex<HashMap<(u8, Pos), (bool, i64)>>> = LazyLock::new(Default::default);
/// A server tick counter, for pruning.
static NOW: AtomicI64 = AtomicI64::new(0);

/// Publishes a switch's signal. True when the table had no entry for it (the switch is new, or
/// has just loaded).
fn publish(level: u8, pos: Pos, powered: bool) -> bool {
    let now = NOW.load(Ordering::Relaxed);
    match SIGNALS.lock() {
        Ok(mut table) => table.insert((level, pos), (powered, now)).is_none(),
        Err(_) => false,
    }
}

// --- reading what the switch looks at --------------------------------------------------------------

/// What the switch at `pos` looks at holds: `ThresholdSwitchBlockEntity.updateCurrentLevel`'s
/// sums over the slots and tanks, and which kind of block it is. `None` where there is nothing
/// to read.
fn read_level(host: &mut dyn Host, at: Pos, filter: &Filter) -> Option<(Level, Kind)> {
    let slots = target::read_slots(host, at);
    let tanks = target::read_tanks(&*host, at);
    if slots.is_none() && tanks.is_none() {
        return None;
    }
    let (mut current, mut max) = (0, 0);
    let kind = if slots.is_some() { Kind::Items } else { Kind::Fluids };
    if let Some(slots) = slots {
        for slot in &slots {
            let size = slot.as_ref().map(|s| host.max_stack_size(&s.item).unwrap_or(64));
            let space = rules::slot_space(size, 64);
            if space == 0 {
                continue;
            }
            max += space;
            if let Some(stack) = slot
                && filter.test(host, stack)
            {
                current += stack.count as i32;
            }
        }
    }
    if let Some(tanks) = tanks {
        for tank in &tanks {
            if tank.capacity == 0 {
                continue;
            }
            max += tank.capacity;
            if let Some(fluid) = &tank.fluid
                && target::passes_fluid(filter, fluid)
            {
                current += tank.amount;
            }
        }
    }
    Some((Level { min: 0, current, max }, kind))
}

// --- the tick -----------------------------------------------------------------------------------------

/// `ThresholdSwitchBlockEntity.updatePowerAfterDelay`.
fn update_power(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity) {
    let powered = rules::should_be_powered(
        util::boolean(&entity.nbt, INVERTED),
        util::boolean(&entity.nbt, POWERED),
    );
    if util::boolean(&entity.nbt, POWERED_AFTER_DELAY) != powered {
        entity.nbt.insert(POWERED_AFTER_DELAY.into(), flag(powered));
    }
    publish(signal::host_level(host), pos, powered);
    host.update_signal_neighbors(host.dimension(), pos);
}

fn schedule(entity: &mut ModBlockEntity) {
    if int(&entity.nbt, POWER_DELAY) == 0 {
        entity.nbt.insert(POWER_DELAY.into(), Tag::Int(rules::POWER_DELAY));
    }
}

/// `ThresholdSwitchBlockEntity.updateCurrentLevel`.
fn update_level(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity) {
    let Some((_, tgt, facing)) = target::view(&*host, pos) else {
        return;
    };
    let (at, _) = target::target_pos(pos, tgt, facing);
    let filter = inv::filter_of(&entity.nbt);
    let previous = int(&entity.nbt, CURRENT);

    let Some((level, _)) = read_level(host, at, &filter) else {
        // No compatible inventory found.
        if int(&entity.nbt, CURRENT_MIN) != -1 {
            entity.nbt.insert(CURRENT_MIN.into(), Tag::Int(-1));
        }
        if int(&entity.nbt, CURRENT_MAX) != -1 {
            entity.nbt.insert(CURRENT_MAX.into(), Tag::Int(-1));
        }
        if previous == -1 {
            return;
        }
        if host.block_property(pos, "level").as_deref() != Some("0") {
            host.set_block_property(pos, "level", "0");
        }
        entity.nbt.insert(CURRENT.into(), Tag::Int(-1));
        entity.nbt.insert(POWERED.into(), flag(false));
        schedule(entity);
        return;
    };

    let current = rules::clamp(level.current, level.min, level.max);
    let level = Level { current, ..level };
    let was = util::boolean(&entity.nbt, POWERED);
    let now = rules::next_state(
        was,
        current,
        int(&entity.nbt, ON_ABOVE),
        int(&entity.nbt, OFF_BELOW),
    );
    for (key, value) in [(CURRENT, current), (CURRENT_MIN, level.min), (CURRENT_MAX, level.max)] {
        if int(&entity.nbt, key) != value || !util::has(&entity.nbt, key) {
            entity.nbt.insert(key.into(), Tag::Int(value));
        }
    }
    if now != was {
        entity.nbt.insert(POWERED.into(), flag(now));
        schedule(entity);
    }
    let shown = rules::display_level(level).to_string();
    if host.block_property(pos, "level").as_deref() != Some(shown.as_str()) {
        host.set_block_property(pos, "level", &shown);
    }
}

fn tick(host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
    let pos = event.pos;
    let level = signal::host_level(host);
    let now = util::game_time(host);
    let powered = util::boolean(&event.entity.nbt, POWERED_AFTER_DELAY);
    // A switch that has just loaded tells its neighbours what it gives.
    if publish(level, pos, powered) && powered {
        host.update_signal_neighbors(host.dimension(), pos);
    }

    // The level's scheduled tick.
    let delay = int(&event.entity.nbt, POWER_DELAY);
    if delay > 0 {
        event.entity.nbt.insert(POWER_DELAY.into(), Tag::Int(delay - 1));
        if delay == 1 {
            update_power(host, pos, event.entity);
        }
    }

    // `onPlace`, then `lazyTick`.
    let salt = i64::from(pos[0] ^ pos[2]) + i64::from(pos[1]);
    let fresh = util::boolean(&event.entity.nbt, FRESH);
    if fresh || (now + salt).rem_euclid(rules::LAZY_TICK) == 0 {
        if fresh {
            event.entity.nbt.insert(FRESH.into(), flag(false));
        }
        update_level(host, pos, event.entity);
    }
}

// --- the stand-in for the screen ------------------------------------------------------------------------

/// `ThresholdSwitchScreen` as a line: how full the block is, and the two thresholds.
fn status(nbt: &Compound, kind: Kind) -> String {
    let (on, off) = (int(nbt, ON_ABOVE), int(nbt, OFF_BELOW));
    let (current, max) = (int(nbt, CURRENT), int(nbt, CURRENT_MAX));
    let invert = if util::boolean(nbt, INVERTED) { ", inverted" } else { "" };
    let show = |value: i32| match kind {
        Kind::Fluids => format!("{} buckets", f64::from(value) / f64::from(rules::BUCKET)),
        _ => format!("{value} items"),
    };
    match kind {
        Kind::Unsupported => "Threshold Switch: not attached to a block it can read".to_owned(),
        _ => format!(
            "Threshold Switch: {} of {} | on at {}, off at {}{invert}",
            show(current.max(0)),
            show(max.max(0)),
            show(on),
            show(off),
        ),
    }
}

/// What the switch at `pos` looks at, for the stand-in.
fn kind_at(host: &mut dyn Host, pos: Pos) -> (Kind, i32, i32) {
    let Some((_, tgt, facing)) = target::view(&*host, pos) else {
        return (Kind::Unsupported, 0, 0);
    };
    let (at, _) = target::target_pos(pos, tgt, facing);
    let filter = host
        .mod_block_entity(pos)
        .map_or(Filter { item: None, amount: Default::default() }, |e| inv::filter_of(&e.nbt));
    match read_level(host, at, &filter) {
        Some((level, kind)) => (kind, level.min, level.max),
        None => (Kind::Unsupported, 0, 0),
    }
}

/// A click that opens Create's screen: steps the thresholds, and says where they are.
fn step(host: &mut dyn Host, pos: Pos) {
    let (kind, min, max) = kind_at(host, pos);
    if kind != Kind::Unsupported {
        let list = rules::presets(min, max, kind);
        host.change_mod_block_entity(pos, &mut |entity| {
            let (on, off) = rules::next_preset(
                &list,
                int(&entity.nbt, ON_ABOVE),
                int(&entity.nbt, OFF_BELOW),
            );
            entity.nbt.insert(ON_ABOVE.into(), Tag::Int(on));
            entity.nbt.insert(OFF_BELOW.into(), Tag::Int(off));
        });
    }
    if let Some(entity) = host.mod_block_entity(pos) {
        let line = status(&entity.nbt, kind);
        inv::tell(host, &line);
    }
}

/// The screen's flip button (`setInverted`): the signal turns over.
fn invert(host: &mut dyn Host, pos: Pos) {
    let (kind, _, _) = kind_at(host, pos);
    let mut powered = false;
    host.change_mod_block_entity(pos, &mut |entity| {
        let inverted = !util::boolean(&entity.nbt, INVERTED);
        entity.nbt.insert(INVERTED.into(), flag(inverted));
        powered = rules::should_be_powered(inverted, util::boolean(&entity.nbt, POWERED));
        entity.nbt.insert(POWERED_AFTER_DELAY.into(), flag(powered));
    });
    publish(signal::host_level(&*host), pos, powered);
    host.update_signal_neighbors(host.dimension(), pos);
    if let Some(entity) = host.mod_block_entity(pos) {
        let line = status(&entity.nbt, kind);
        inv::tell(host, &line);
    }
}

impl Hooks for Switches {
    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == SWITCH).then(|| (SWITCH.to_owned(), new_entity()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == SWITCH {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id == SWITCH {
            tick(host, event);
        }
    }

    /// Prunes the switches that stopped ticking (their chunk unloaded, or the block went).
    fn server_tick(&self, _: &mut dyn Host, event: &mut ServerTickEvent<'_>) {
        if let ServerTickPhase::ServerTickStart { world_running: true } = event.phase {
            let now = NOW.fetch_add(1, Ordering::Relaxed) + 1;
            if let Ok(mut table) = SIGNALS.lock() {
                table.retain(|_, (_, seen)| now - *seen < MAX_AGE);
            }
        }
    }

    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        target::placed_state(level, event, SWITCH)
    }

    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if &*event.stack.item == SWITCH && host.block(event.pos) == SWITCH {
            target::orient(host, event.pos);
        }
    }

    /// The Wrench turns the switch (`DirectedDirectionalBlock.getRotatedBlockState`).
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        if &*event.stack.item != util::WRENCH || host.block(pos) != SWITCH {
            return Flow::Continue;
        }
        let clicked = Dir::from_normal(event.face);
        util::wrench(host, pos, |host| {
            if let Some(clicked) = clicked {
                target::wrench_turn(host, pos, clicked);
            }
        })
    }

    /// `ThresholdSwitchBlock.useItemOn`, where Create opens its screen for any item but the
    /// Wrench: here it steps the thresholds and the Redstone Torch inverts the signal.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != SWITCH {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        let held = host.player_equipment(player.uuid, EquipmentSlot::MainHand);
        if held.as_ref().is_some_and(|stack| &*stack.item == util::WRENCH) {
            return Flow::Continue;
        }
        // A click reaches `right_click_block` for each hand, and a step must happen once.
        if !util::first_click(&*host, player.uuid, pos) {
            return Flow::Cancel;
        }
        if held.as_ref().is_some_and(|stack| &*stack.item == REDSTONE_TORCH) {
            invert(host, pos);
        } else {
            step(host, pos);
        }
        Flow::Cancel
    }

    /// `ThresholdSwitchBlock.getSignal`: 15 on every side but the one it looks at, while on
    /// (`poweredAfterDelay`); never a direct (strong) signal.
    fn block_signal(&self, level: &dyn LevelHost, event: &mut BlockSignalEvent) {
        if let Some(looks) = target::looks(level, event.state)
            && looks.which == Which::Switch
            && !event.direct
        {
            let at = [event.pos.x, event.pos.y, event.pos.z];
            let lit = SIGNALS
                .lock()
                .ok()
                .and_then(|table| {
                    table.get(&(signal::level_index(event.dimension), at)).map(|&(on, _)| on)
                })
                .unwrap_or(false);
            event.signal = create_kinetics::rules::content_observer_target::signal_towards(
                util::dir(event.face),
                looks.direction(),
                lit,
            );
        }
        signal::mask(event);
    }

    /// `ThresholdSwitchBlock.isSignalSource`: always.
    fn is_signal_source(&self, level: &dyn LevelHost, event: &mut IsSignalSourceEvent) {
        if target::looks(level, event.state).is_some_and(|looks| looks.which == Which::Switch) {
            event.source = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_entry_is_new_once_and_the_signal_is_kept() {
        let pos = [3, 4, 5];
        assert!(publish(7, pos, true));
        assert!(!publish(7, pos, false));
        assert_eq!(SIGNALS.lock().unwrap().get(&(7, pos)).map(|e| e.0), Some(false));
    }

    #[test]
    fn the_status_line_gives_items_or_buckets_and_the_thresholds() {
        let mut nbt = new_entity();
        nbt.insert(CURRENT.into(), Tag::Int(300));
        nbt.insert(CURRENT_MAX.into(), Tag::Int(1728));
        assert_eq!(
            status(&nbt, Kind::Items),
            "Threshold Switch: 300 items of 1728 items | on at 128 items, off at 64 items"
        );
        nbt.insert(INVERTED.into(), flag(true));
        nbt.insert(ON_ABOVE.into(), Tag::Int(4000));
        nbt.insert(OFF_BELOW.into(), Tag::Int(2000));
        nbt.insert(CURRENT.into(), Tag::Int(2500));
        nbt.insert(CURRENT_MAX.into(), Tag::Int(8000));
        assert_eq!(
            status(&nbt, Kind::Fluids),
            "Threshold Switch: 2.5 buckets of 8 buckets | on at 4 buckets, off at 2 buckets, inverted"
        );
        assert!(status(&nbt, Kind::Unsupported).contains("not attached"));
    }
}
