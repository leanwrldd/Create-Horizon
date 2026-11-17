//! What the Smart Observer and the Threshold Switch share (`DirectedDirectionalBlock`,
//! `FilteringBehaviour`, `InvManipulationBehaviour`, `TankManipulationBehaviour`): the block
//! they look at, the filter that stands in for the filter slot, the way they turn when placed and
//! wrenched, and reading what the block they look at holds. The rules are
//! `create_kinetics::rules::content_observer_target`.
//!
//! Known differences from Create (the two blocks' own are in `content_observer.rs` and
//! `stockpile_switch.rs`):
//! - Horizon can't list another block's slots or ask whether it is an inventory, so an inventory
//!   is read from its saved `Items` (see `chute_inv`) and its slot count comes from a table of
//!   vanilla's containers (a chest 27, a double chest 54, a barrel or shulker box 27, a hopper 5,
//!   a dropper or dispenser 9, a furnace 3, a brewing stand 5...), Create's vault (20 a block, all
//!   of its blocks) and crate (2); another of Create's containers counts the slots up to its last
//!   stack.
//! - A fluid is read through `Host::fluid_tanks` (Create's tanks, the basin, a cauldron...), from
//!   any side.
//! - The filter is Create's filter slot (a value box, `machines/value_box.rs`). A bucket or bottle of a fluid is
//!   a filter for that fluid (`FilterItemStack.test` with `GenericItemEmptying`).
//! - When placed, the block turns to the first thing it can read in the way the player looks
//!   (`getStateForPlacement` reads the block entities around): that is done just after the
//!   placement, so the block first appears as it would with nothing to read.

use create_kinetics::rules::chute::Amount;
use create_kinetics::rules::content_observer_target::{self as rules, Target};
use create_kinetics::rules::fluids as fluid_rules;
use create_kinetics::{Dir, Pos, looking};
use horizon_client::mod_api::{
    BlockStateId, Hooks, Host, ItemStack, LevelHost, StateForPlacementEvent,
};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use super::chute_inv::{self as inv, Filter};
use super::diodes_signal as signal;
use crate::{belt, depot, util};

pub const OBSERVER: &str = "create:content_observer";
pub const SWITCH: &str = "create:stockpile_switch";

/// A module of shared code: no hooks of its own.
struct Shared;

impl Hooks for Shared {}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

// --- the block's state ----------------------------------------------------------------------------

/// Which of the two blocks a state is, and what it looks at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    Observer,
    Switch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Looks {
    pub which: Which,
    pub target: Target,
    pub facing: Dir,
    /// The observer's `powered`; false for a switch (its signal is its block entity's).
    pub powered: bool,
}

impl Looks {
    /// `DirectedDirectionalBlock.getTargetDirection`.
    pub fn direction(&self) -> Dir {
        rules::target_direction(self.target, self.facing)
    }
}

/// States already looked at: a state never changes.
static STATES: LazyLock<Mutex<HashMap<u32, Option<Looks>>>> = LazyLock::new(Default::default);

/// The block a state belongs to, from its properties alone (a hook with only a level can't name
/// it): `target` and a horizontal `facing`, with `powered` for the observer and `level` for the
/// switch.
pub fn looks(level: &dyn LevelHost, state: BlockStateId) -> Option<Looks> {
    if let Ok(cache) = STATES.lock()
        && let Some(known) = cache.get(&state.0)
    {
        return *known;
    }
    let found = looks_uncached(level, state);
    if let Ok(mut cache) = STATES.lock() {
        cache.insert(state.0, found);
    }
    found
}

fn looks_uncached(level: &dyn LevelHost, state: BlockStateId) -> Option<Looks> {
    let target = signal::choice(level, state, "target", &["floor", "wall", "ceiling"])
        .and_then(Target::from_name)?;
    let facing = signal::horizontal_facing(level, state)?;
    if let Some(powered) = signal::boolean(level, state, "powered") {
        if signal::takes(level, state, "level", "0") {
            return None;
        }
        return Some(Looks { which: Which::Observer, target, facing, powered });
    }
    signal::takes(level, state, "level", "5").then_some(Looks {
        which: Which::Switch,
        target,
        facing,
        powered: false,
    })
}

/// The block at `pos`, read as the world has it.
pub fn view(host: &dyn Host, pos: Pos) -> Option<(Which, Target, Dir)> {
    let which = match host.block(pos).as_str() {
        OBSERVER => Which::Observer,
        SWITCH => Which::Switch,
        _ => return None,
    };
    let target = host.block_property(pos, "target").as_deref().and_then(Target::from_name)?;
    let facing = host.block_property(pos, "facing").as_deref().and_then(Dir::from_name)?;
    Some((which, target, facing))
}

/// Where the block at `pos` looks: the block next to it on that side.
pub fn target_pos(pos: Pos, target: Target, facing: Dir) -> (Pos, Dir) {
    let direction = rules::target_direction(target, facing);
    (util::add(pos, direction.normal()), direction)
}

// --- placing and turning ------------------------------------------------------------------------------

/// `getStateForPlacement` as far as a level can tell: looking back at the player, or the way
/// they look when sneaking. The block turns to what it can read in [`orient`], after it is placed.
pub fn placed_state(
    level: &dyn LevelHost,
    event: &StateForPlacementEvent<'_>,
    id: &str,
) -> Option<Option<BlockStateId>> {
    let context = event.context;
    if &*context.item != id {
        return None;
    }
    let nearest = looking(context.yaw, context.pitch);
    let (target, facing) = rules::placement(None, nearest, rules::horizontal(context.yaw), context.sneaking);
    let state = level.state_with_property(event.state, "target", target.name())?;
    level.state_with_property(state, "facing", facing.name()).map(Some)
}

/// Whether the block at `at` is something a Smart Observer (`observer`) or a Threshold Switch
/// can read.
fn readable(host: &mut dyn Host, at: Pos, observer: bool) -> bool {
    let name = host.block(at);
    if name == belt::ID || name == depot::ID {
        return observer;
    }
    if slot_count(host, at).is_some() {
        return true;
    }
    host.fluid_tanks(host.dimension(), at, util::direction(Dir::Up)).is_some()
}

/// `getStateForPlacement`'s loop over `getNearestLookingDirections`: the block just placed at
/// `pos` turns to the first side that has something it can read. Called from `place_block`.
pub fn orient(host: &mut dyn Host, pos: Pos) {
    let Some((which, target, facing)) = view(&*host, pos) else {
        return;
    };
    let Some(player) = host.acting_player() else {
        return;
    };
    let sneaking = player.sneaking;
    let order = rules::ordered_by_nearest(player.yaw, player.pitch);
    let found = order
        .into_iter()
        .find(|side| readable(host, util::add(pos, side.normal()), which == Which::Observer));
    let Some(found) = found else {
        return;
    };
    let (want_target, want_facing) =
        rules::placement(Some(found), order[0], rules::horizontal(player.yaw), sneaking);
    if want_target != target {
        host.set_block_property(pos, "target", want_target.name());
    }
    if want_facing != facing {
        host.set_block_property(pos, "facing", want_facing.name());
    }
}

/// The Wrench on the block at `pos` (`DirectedDirectionalBlock.getRotatedBlockState`).
pub fn wrench_turn(host: &mut dyn Host, pos: Pos, clicked: Dir) {
    let Some((_, target, facing)) = view(&*host, pos) else {
        return;
    };
    let (next_target, next_facing) = rules::rotated(target, facing, clicked);
    if next_target != target {
        host.set_block_property(pos, "target", next_target.name());
    }
    if next_facing != facing {
        host.set_block_property(pos, "facing", next_facing.name());
    }
}

// --- the filter -------------------------------------------------------------------------------------

/// `FilteringBehaviour.test(FluidStack)` (`FilterItemStack.test`): no filter passes anything, a
/// filter that holds a fluid passes that fluid, any other only items.
pub fn passes_fluid(filter: &Filter, fluid: &str) -> bool {
    match &filter.item {
        None => true,
        Some(item) if super::filter::is_filter_item(&item.item) => {
            super::filter::matches_fluid(item, fluid)
        }
        Some(item) => fluid_rules::emptied(&item.item).is_some_and(|c| c.fluid == fluid),
    }
}

// --- reading what the block holds --------------------------------------------------------------------

const CHEST_SLOTS: usize = 27;

/// How many slots the inventory at `pos` has (`IItemHandler.getSlots`), `None` where there is no
/// inventory to read (a belt or a depot is not one here).
pub fn slot_count(host: &mut dyn Host, pos: Pos) -> Option<usize> {
    let name = host.block(pos);
    let each = match name.as_str() {
        n if n == belt::ID || n == depot::ID => return None,
        "minecraft:chest" | "minecraft:trapped_chest" => {
            return Some(inv::parts(&*host, pos).len() * CHEST_SLOTS);
        }
        inv::VAULT => return Some(inv::parts(&*host, pos).len() * inv::VAULT_SLOTS),
        "minecraft:barrel" => 27,
        n if n.ends_with("shulker_box") => 27,
        "minecraft:hopper" | "minecraft:brewing_stand" => 5,
        "minecraft:dropper" | "minecraft:dispenser" | "minecraft:crafter" => 9,
        "minecraft:furnace" | "minecraft:blast_furnace" | "minecraft:smoker" => 3,
        "minecraft:chiseled_bookshelf" => 6,
        "create:creative_crate" => 2,
        _ => {
            // Another of Create's containers, or a mod's: as many slots as its last stack's.
            if !inv::is_inventory(host, pos) {
                return None;
            }
            let items = inv::items_of(host, pos).unwrap_or_default();
            return Some(items.iter().map(|(slot, _)| slot + 1).max().unwrap_or(0));
        }
    };
    Some(each)
}

/// Every slot of the inventory at `pos`, the empty ones too, in order.
pub fn read_slots(host: &mut dyn Host, pos: Pos) -> Option<Vec<Option<ItemStack>>> {
    let count = slot_count(host, pos)?;
    let mut slots: Vec<Option<ItemStack>> = vec![None; count];
    for (slot, stack) in inv::items_of(host, pos).unwrap_or_default() {
        if slot >= slots.len() {
            slots.resize(slot + 1, None);
        }
        slots[slot] = Some(stack);
    }
    Some(slots)
}

/// Whether the inventory at `pos` holds a stack that passes `filter` (any stack when there is
/// none): `InvManipulationBehaviour.simulate().extract()` is not empty. `None` for a block that
/// is no inventory.
pub fn inventory_has(host: &mut dyn Host, pos: Pos, filter: &Filter) -> Option<bool> {
    let name = host.block(pos);
    if name == belt::ID || name == depot::ID || !inv::is_inventory(host, pos) {
        return None;
    }
    let items = inv::items_of(host, pos).unwrap_or_default();
    Some(items.iter().any(|(_, stack)| stack.count > 0 && filter.test(host, stack)))
}

/// A tank as the side reaches it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tank {
    pub fluid: Option<String>,
    pub amount: i32,
    pub capacity: i32,
}

/// The tanks of the block at `pos`, `None` where there are none (`TankManipulationBehaviour`).
pub fn read_tanks(host: &dyn Host, pos: Pos) -> Option<Vec<Tank>> {
    let tanks = host.fluid_tanks(host.dimension(), pos, util::direction(Dir::Up))?;
    Some(
        tanks
            .into_iter()
            .map(|t| Tank { fluid: t.fluid, amount: t.amount, capacity: t.capacity })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machines::diodes_signal::fake::Registry;

    const TARGETS: &[&str] = &["floor", "wall", "ceiling"];
    const FACING: &[&str] = &["north", "south", "west", "east"];
    const BOOL: &[&str] = &["false", "true"];
    const LEVELS: &[&str] = &["0", "1", "2", "3", "4", "5"];

    #[test]
    fn an_observer_has_powered_and_a_switch_has_a_level() {
        let mut level = Registry::default();
        level.block(&[("target", TARGETS), ("facing", FACING), ("powered", BOOL)]);
        level.block(&[("target", TARGETS), ("facing", FACING), ("level", LEVELS)]);
        // A block with a `facing` and a `powered` but no `target` (a lever, a button).
        level.block(&[("facing", FACING), ("powered", BOOL)]);

        let observer = level.find(&[("target", "ceiling"), ("facing", "east"), ("powered", "true")]);
        assert_eq!(
            looks(&level, observer),
            Some(Looks { which: Which::Observer, target: Target::Ceiling, facing: Dir::East, powered: true })
        );
        let switch = level.find(&[("target", "wall"), ("facing", "south"), ("level", "3")]);
        let seen = looks(&level, switch).expect("a switch");
        assert_eq!((seen.which, seen.direction()), (Which::Switch, Dir::South));
        let lever = level.find(&[("facing", "north"), ("powered", "false")]);
        assert_eq!(looks(&level, lever), None);
    }

    #[test]
    fn the_target_block_is_the_neighbour_on_the_looked_at_side() {
        assert_eq!(target_pos([5, 5, 5], Target::Wall, Dir::East), ([6, 5, 5], Dir::East));
        assert_eq!(target_pos([5, 5, 5], Target::Ceiling, Dir::East), ([5, 6, 5], Dir::Up));
        assert_eq!(target_pos([5, 5, 5], Target::Floor, Dir::North), ([5, 4, 5], Dir::Down));
    }

    #[test]
    fn a_bucket_filters_its_fluid_and_an_item_filter_no_fluid() {
        let stack = |item: &str| ItemStack { item: item.into(), count: 1, damage: 0, extra: None };
        let none = Filter { item: None, amount: Amount::default() };
        let water = Filter { item: Some(stack("minecraft:water_bucket")), amount: Amount::default() };
        let stone = Filter { item: Some(stack("minecraft:stone")), amount: Amount::default() };
        assert!(passes_fluid(&none, "minecraft:lava"));
        assert!(passes_fluid(&water, "minecraft:water"));
        assert!(!passes_fluid(&water, "minecraft:lava"));
        assert!(!passes_fluid(&stone, "minecraft:water"));
    }
}
