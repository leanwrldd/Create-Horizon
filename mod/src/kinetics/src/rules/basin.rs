//! The Basin's rules and the numbers it shares with the mixer and the blaze burner: heat levels,
//! how a recipe takes its ingredients out of the slots and tanks, which side the output spills
//! to, and the slot policy of the input inventory. Ported from Create's `BasinBlock`,
//! `BasinBlockEntity`, `BasinInventory`, `BasinRecipe`, `HeatCondition` and
//! `BlazeBurnerBlock.HeatLevel` (MIT, the Creators of Create), checked against `mc1.21.1/dev`
//! at 4148fd8.
//!
//! No game types: the glue (`native/api/src/machines/basin.rs`) owns the item stacks, the tanks
//! and the world.

use crate::Dir;

/// `new BasinInventory(9, this)`.
pub const INPUT_SLOTS: usize = 9;
/// `new BasinInventory(9, this)` for the outputs.
pub const OUTPUT_SLOTS: usize = 9;
/// Every slot of `CombinedInvWrapper(inputInventory, outputInventory)`.
pub const SLOTS: usize = INPUT_SLOTS + OUTPUT_SLOTS;
/// `SmartInventory(slots, be, 64, true)` and `withMaxStackSize(64)`.
pub const SLOT_LIMIT: u32 = 64;
/// `new SmartFluidTankBehaviour(INPUT, this, 2, 1000, true)`: two tanks on each side.
pub const TANKS_PER_SIDE: usize = 2;
/// Input tanks first (0 and 1), then output tanks (2 and 3).
pub const TANKS: usize = 2 * TANKS_PER_SIDE;
/// A tank's capacity in millibuckets (`SmartFluidTankBehaviour(..., 1000, ...)`).
pub const TANK_CAPACITY: i32 = 1000;
/// `BasinBlockEntity.recipeBackupCheck = 20`, counted in lazy ticks of 10 game ticks
/// (`SmartBlockEntity.lazyTickRate`): a basin with contents asks its operator to look again
/// about every 200 ticks.
pub const BACKUP_CHECK_TICKS: i64 = 200;

// --- heat ----------------------------------------------------------------------------------------

/// `BlazeBurnerBlock.HeatLevel`, the burner's `blaze` state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HeatLevel {
    None,
    Smouldering,
    Fading,
    Kindled,
    Seething,
}

impl HeatLevel {
    pub const ALL: [HeatLevel; 5] = [
        HeatLevel::None,
        HeatLevel::Smouldering,
        HeatLevel::Fading,
        HeatLevel::Kindled,
        HeatLevel::Seething,
    ];

    /// `getSerializedName`: the state's `blaze` value.
    pub const fn name(self) -> &'static str {
        match self {
            HeatLevel::None => "none",
            HeatLevel::Smouldering => "smouldering",
            HeatLevel::Fading => "fading",
            HeatLevel::Kindled => "kindled",
            HeatLevel::Seething => "seething",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|level| level.name() == name)
    }

    /// `isAtLeast`.
    pub fn is_at_least(self, other: HeatLevel) -> bool {
        self >= other
    }

    /// `nextActiveLevel`: `byIndex(ordinal % (length - 1) + 1)`.
    pub fn next_active_level(self) -> HeatLevel {
        Self::ALL[self as usize % (Self::ALL.len() - 1) + 1]
    }

    /// `BlazeBurnerBlock.getLight`.
    pub fn light(self) -> u8 {
        match self {
            HeatLevel::None => 0,
            HeatLevel::Smouldering => 8,
            _ => 15,
        }
    }

    /// `getAnalogOutputSignal`: `Math.max(0, ordinal - 1)`.
    pub fn analog_output(self) -> u8 {
        (self as u8).saturating_sub(1)
    }
}

/// `HeatCondition`, a recipe's `heat_requirement`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeatCondition {
    None,
    Heated,
    Superheated,
}

impl HeatCondition {
    pub fn from_name(name: &str) -> Self {
        match name {
            "heated" => HeatCondition::Heated,
            "superheated" => HeatCondition::Superheated,
            _ => HeatCondition::None,
        }
    }

    /// `testBlazeBurner`.
    pub fn test(self, level: HeatLevel) -> bool {
        match self {
            HeatCondition::Superheated => level == HeatLevel::Seething,
            HeatCondition::Heated => !matches!(level, HeatLevel::None | HeatLevel::Smouldering),
            HeatCondition::None => true,
        }
    }
}

/// `BasinBlockEntity.getHeatLevelOf` for a block that has no `blaze` state: a passive heater
/// (`create:passive_boiler_heaters`) that is lit counts as smouldering. `heater` says the block is
/// in that tag and `unheated` that `BlockHelper.isNotUnheated` is false for it.
pub fn passive_heat(heater: bool, unheated: bool) -> HeatLevel {
    if heater && !unheated {
        HeatLevel::Smouldering
    } else {
        HeatLevel::None
    }
}

// --- the input inventory (`BasinInventory.insertItem`) ----------------------------------------------

/// Whether `BasinInventory.insertItem` lets a stack into input slot `slot`. `empty` says which
/// input slots are empty and `same_elsewhere` whether another slot already holds the same item
/// with the same components: one stack per item, and a new stack only in the first empty slot.
/// Output slots take nothing from outside (`forbidInsertion`).
pub fn may_insert_input(slot: usize, empty: &[bool], same_elsewhere: bool) -> bool {
    if slot >= INPUT_SLOTS || same_elsewhere {
        return false;
    }
    let first_free = empty.iter().position(|e| *e);
    !(empty.get(slot).copied().unwrap_or(false) && first_free != Some(slot))
}

// --- taking a recipe's ingredients (`BasinRecipe.apply`) -----------------------------------------

/// The item pass of `BasinRecipe.apply`: each ingredient, in order, takes one item from the first
/// slot it matches that still has one left. `counts` are the slots' counts and `matches[i][s]`
/// says whether ingredient `i` takes the item in slot `s`. Answers how many items come out of
/// each slot, or `None` when some ingredient finds nothing ("something wasn't found").
pub fn assign_items(counts: &[u32], matches: &[Vec<bool>]) -> Option<Vec<u32>> {
    let mut taken = vec![0u32; counts.len()];
    'ingredients: for ingredient in matches {
        for (slot, count) in counts.iter().enumerate() {
            if *count <= taken[slot] {
                continue;
            }
            if !ingredient.get(slot).copied().unwrap_or(false) {
                continue;
            }
            taken[slot] += 1;
            continue 'ingredients;
        }
        return None;
    }
    Some(taken)
}

/// The fluid pass of `BasinRecipe.apply`, with its quirks: an ingredient drains the tanks that
/// match it in order, `required[i]` millibuckets in all. In the simulation a tank is skipped once
/// the amount already planned out of it has reached what it holds, and that amount is only
/// recorded when the ingredient is met in full (so an ingredient split over two tanks records
/// the second only). With `simulate` false the tanks are drained for real. Answers whether every
/// fluid ingredient was found.
pub fn drain_fluids(
    amounts: &mut [i32],
    matches: &[Vec<bool>],
    required: &[i32],
    simulate: bool,
) -> bool {
    let mut planned = vec![0i32; amounts.len()];
    'ingredients: for (i, need) in required.iter().enumerate() {
        let mut left = *need;
        for tank in 0..amounts.len() {
            if simulate && amounts[tank] <= planned[tank] {
                continue;
            }
            if !matches[i].get(tank).copied().unwrap_or(false) {
                continue;
            }
            let drained = left.min(amounts[tank]);
            if !simulate {
                amounts[tank] -= drained;
            }
            left -= drained;
            if left != 0 {
                continue;
            }
            planned[tank] += drained;
            continue 'ingredients;
        }
        return false;
    }
    true
}

/// `ItemHandlerHelper.insertItemStacked` over `slots`: tops up equal stacks first, then fills
/// empty slots, each up to `limit`. Answers what did not fit (`None`: all of it went in). `same`
/// says whether two stacks are the same item with the same components.
pub fn insert_stacked<T: Clone>(
    slots: &mut [Option<T>],
    mut stack: T,
    count_of: impl Fn(&T) -> u32,
    set_count: impl Fn(&mut T, u32),
    same: impl Fn(&T, &T) -> bool,
    limit: u32,
) -> Option<T> {
    for slot in slots.iter_mut() {
        if count_of(&stack) == 0 {
            return None;
        }
        let Some(have) = slot else { continue };
        if !same(have, &stack) {
            continue;
        }
        let moved = limit.saturating_sub(count_of(have)).min(count_of(&stack));
        if moved > 0 {
            let total = count_of(have) + moved;
            set_count(have, total);
            let left = count_of(&stack) - moved;
            set_count(&mut stack, left);
        }
    }
    for slot in slots.iter_mut() {
        if count_of(&stack) == 0 {
            return None;
        }
        if slot.is_some() {
            continue;
        }
        let moved = limit.min(count_of(&stack));
        let mut put = stack.clone();
        set_count(&mut put, moved);
        *slot = Some(put);
        let left = count_of(&stack) - moved;
        set_count(&mut stack, left);
    }
    (count_of(&stack) > 0).then_some(stack)
}

// --- the spout (`BasinBlockEntity.updateSpoutput`, `onWrenched`) -------------------------------------

/// `Iterate.horizontalDirections`: `Direction.values()` filtered to the horizontal ones.
pub const HORIZONTALS: [Dir; 4] = [Dir::North, Dir::South, Dir::West, Dir::East];

/// What a basin has been told about its spout: the side a wrench set (`preferredSpoutput`) and the
/// sides it turned off (`disabledSpoutput`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Spout {
    pub preferred: Option<Dir>,
    pub disabled: Vec<Dir>,
}

impl Spout {
    /// `BasinBlockEntity.onWrenched(face)` for a basin currently `facing` a side.
    pub fn wrenched(&mut self, facing: Dir, face: Dir) {
        self.disabled.retain(|d| *d != face);
        if facing == face {
            if self.preferred == Some(face) {
                self.preferred = None;
            }
            self.disabled.push(face);
        } else {
            self.preferred = Some(face);
        }
    }

    /// The side `updateSpoutput` picks: the last horizontal side that can take the output and is
    /// not disabled, or down; the preferred side wins when it can take it.
    /// `can_output(dir)` is `BasinBlock.canOutputTo`.
    pub fn facing(&self, can_output: impl Fn(Dir) -> bool) -> Dir {
        let mut facing = Dir::Down;
        for side in HORIZONTALS {
            if can_output(side) && !self.disabled.contains(&side) {
                facing = side;
            }
        }
        if let Some(preferred) = self.preferred
            && can_output(preferred)
            && preferred != Dir::Up
        {
            facing = preferred;
        }
        facing
    }
}

/// The level of the fluids in a basin, as `BasinRenderer.renderFluids` raises it:
/// `1 - (1 - clamp(units / 2000)) ^ 2`.
pub fn fluid_fill(total_units: f32) -> f32 {
    let level = (total_units / 2000.0).clamp(0.0, 1.0);
    1.0 - (1.0 - level) * (1.0 - level)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heat_levels_follow_create() {
        assert_eq!(HeatLevel::None.next_active_level(), HeatLevel::Smouldering);
        assert_eq!(HeatLevel::Smouldering.next_active_level(), HeatLevel::Fading);
        assert_eq!(HeatLevel::Seething.next_active_level(), HeatLevel::Smouldering);
        assert_eq!(HeatLevel::from_name("kindled"), Some(HeatLevel::Kindled));
        assert_eq!(HeatLevel::None.light(), 0);
        assert_eq!(HeatLevel::Smouldering.light(), 8);
        assert_eq!(HeatLevel::Fading.light(), 15);
        assert_eq!(HeatLevel::None.analog_output(), 0);
        assert_eq!(HeatLevel::Seething.analog_output(), 3);
    }

    #[test]
    fn heat_conditions_test_the_burner() {
        use HeatLevel::*;
        assert!(HeatCondition::None.test(None));
        assert!(!HeatCondition::Heated.test(None));
        assert!(!HeatCondition::Heated.test(Smouldering));
        assert!(HeatCondition::Heated.test(Fading));
        assert!(HeatCondition::Heated.test(Seething));
        assert!(!HeatCondition::Superheated.test(Kindled));
        assert!(HeatCondition::Superheated.test(Seething));
    }

    #[test]
    fn a_lit_passive_heater_is_smouldering() {
        assert_eq!(passive_heat(true, false), HeatLevel::Smouldering);
        assert_eq!(passive_heat(true, true), HeatLevel::None);
        assert_eq!(passive_heat(false, false), HeatLevel::None);
    }

    #[test]
    fn input_slots_keep_one_stack_per_item_and_fill_in_order() {
        let empty = [false, true, true, false, true, true, true, true, true];
        assert!(may_insert_input(0, &empty, false), "topping up a held stack");
        assert!(may_insert_input(1, &empty, false), "the first free slot");
        assert!(!may_insert_input(2, &empty, false), "not the first free slot");
        assert!(!may_insert_input(0, &empty, true), "the item is elsewhere");
        assert!(!may_insert_input(9, &empty, false), "outputs take nothing");
    }

    #[test]
    fn ingredients_take_one_item_each_from_the_first_matching_slot() {
        // Two ingredients that both match slot 0 (3 items) and slot 1 (1 item).
        let counts = [1, 1];
        let both = vec![vec![true, true], vec![true, true]];
        assert_eq!(assign_items(&counts, &both), Some(vec![1, 1]));
        let three = vec![vec![true, true]; 3];
        assert_eq!(assign_items(&counts, &three), None);
        // A slot with several items serves several ingredients.
        assert_eq!(assign_items(&[3], &vec![vec![true]; 3]), Some(vec![3]));
        assert_eq!(assign_items(&[2], &vec![vec![true]; 3]), None);
        // An ingredient nothing matches.
        assert_eq!(assign_items(&[5], &[vec![false]]), None);
    }

    #[test]
    fn fluids_drain_in_order_and_need_the_whole_amount() {
        let mut tanks = [1000, 0, 0, 0];
        let m = vec![vec![true, true, true, true]];
        assert!(drain_fluids(&mut tanks, &m, &[250], true));
        assert_eq!(tanks, [1000, 0, 0, 0], "a simulation drains nothing");
        assert!(drain_fluids(&mut tanks, &m, &[250], false));
        assert_eq!(tanks, [750, 0, 0, 0]);
        assert!(!drain_fluids(&mut tanks, &m, &[800], true));
        // Split over two tanks: the real pass drains both.
        let mut split = [600, 600, 0, 0];
        assert!(drain_fluids(&mut split, &m, &[1000], true));
        assert!(drain_fluids(&mut split, &m, &[1000], false));
        assert_eq!(split, [0, 200, 0, 0]);
    }

    fn put(slots: &mut [Option<(char, u32)>], item: char, count: u32) -> Option<(char, u32)> {
        insert_stacked(slots, (item, count), |s| s.1, |s, n| s.1 = n, |a, b| a.0 == b.0, 64)
    }

    #[test]
    fn insert_stacked_tops_up_then_fills_empties_and_returns_the_rest() {
        let mut slots = [None, Some(('a', 60)), None];
        assert_eq!(put(&mut slots, 'a', 10), None);
        assert_eq!(slots, [Some(('a', 6)), Some(('a', 64)), None]);
        let mut full = [Some(('a', 64)), Some(('b', 1))];
        assert_eq!(put(&mut full, 'a', 5), Some(('a', 5)));
    }

    #[test]
    fn the_spout_goes_to_the_last_open_side_unless_one_is_preferred() {
        let mut spout = Spout::default();
        assert_eq!(spout.facing(|_| false), Dir::Down);
        assert_eq!(spout.facing(|d| d == Dir::North || d == Dir::West), Dir::West);
        // Wrenching a side that is not the spout prefers it.
        spout.wrenched(Dir::Down, Dir::North);
        assert_eq!(spout.preferred, Some(Dir::North));
        assert_eq!(spout.facing(|d| d == Dir::North || d == Dir::West), Dir::North);
        // Wrenching the side it faces turns that side off and drops the preference.
        spout.wrenched(Dir::North, Dir::North);
        assert_eq!(spout.preferred, None);
        assert_eq!(spout.disabled, vec![Dir::North]);
        assert_eq!(spout.facing(|d| d == Dir::North), Dir::Down);
        // Wrenching a disabled side that is not facing turns it on again, as the preference.
        spout.wrenched(Dir::Down, Dir::North);
        assert!(spout.disabled.is_empty());
        assert_eq!(spout.preferred, Some(Dir::North));
    }

    #[test]
    fn fluid_level_rises_quickly_then_slowly() {
        assert_eq!(fluid_fill(0.0), 0.0);
        assert_eq!(fluid_fill(2000.0), 1.0);
        assert!((fluid_fill(1000.0) - 0.75).abs() < 1e-6);
    }
}
