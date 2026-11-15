//! The Millstone's rules: its timer and slot arithmetic. Ported from Create's
//! `MillstoneBlockEntity`, `MillstoneBlock` and `ItemHandlerHelper.insertItemStacked`
//! (MIT, the Creators of Create), checked against `mc1.21.1/dev` at 4148fd8.
//!
//! No game types: the glue (`native/api/src/machines/millstone.rs`) owns the item stacks.

/// Stress units per RPM (`AllBlocks.MILLSTONE`: `CStress.setImpact(4.0)`).
pub const IMPACT: f32 = 4.0;
/// `MillstoneBlockEntity`: `inputInv = new ItemStackHandler(1)`.
pub const INPUT_SLOTS: usize = 1;
/// `MillstoneBlockEntity`: `outputInv = new ItemStackHandler(9)`.
pub const OUTPUT_SLOTS: usize = 9;
/// `ItemStackHandler.getSlotLimit`.
pub const SLOT_LIMIT: u32 = 64;
/// The timer when no recipe matches the input (`tick`: `timer = 100`).
pub const NO_RECIPE_TIME: i32 = 100;
/// How many ticks apart the glue looks for items lying on top. Create reacts to an item
/// landing (`MillstoneBlock.updateEntityAfterFallOn`); Horizon has no such hook with a host.
pub const TOP_SCAN: i64 = 4;

/// `getProcessingSpeed`: `clamp((int) abs(speed / 16), 1, 512)`; the cast truncates.
pub fn processing_speed(rpm: f32) -> i32 {
    ((rpm.abs() / 16.0) as i32).clamp(1, 512)
}

/// What a tick of the millstone has to do next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Standing still, or an output slot is full: nothing happens (timer untouched).
    Stopped,
    /// The timer ran down but not out.
    Counting,
    /// The timer ran out: process one input item.
    Process,
    /// Nothing to grind.
    Idle,
    /// Timer at zero with an input: look the recipe up and set the timer
    /// ([`timer_for`]), which clients are told.
    Lookup,
}

/// One `MillstoneBlockEntity.tick`, on the server. `timer` is changed as Create changes it
/// (only the count-down; a lookup sets it afterwards through [`timer_for`]).
pub fn step(timer: &mut i32, rpm: f32, output_full: bool, input_empty: bool) -> Step {
    if rpm == 0.0 || output_full {
        return Step::Stopped;
    }
    if *timer > 0 {
        *timer -= processing_speed(rpm);
        return if *timer <= 0 {
            Step::Process
        } else {
            Step::Counting
        };
    }
    if input_empty {
        return Step::Idle;
    }
    Step::Lookup
}

/// The timer after a lookup: the recipe's `processing_time`, or [`NO_RECIPE_TIME`] without one.
pub fn timer_for(recipe_time: Option<i32>) -> i32 {
    recipe_time.unwrap_or(NO_RECIPE_TIME)
}

/// Whether an output slot is at its limit (`getCount() == getSlotLimit`). A stack that can
/// never reach 64 (its maximum is smaller) is full at its own maximum, where Create would
/// go on and lose what no longer fits.
pub fn slot_full(count: u32, max_stack: u32) -> bool {
    count >= SLOT_LIMIT.min(max_stack)
}

/// `ItemHandlerHelper.insertItemStacked`: tops up equal stacks first, then fills empty slots,
/// each up to `limit`. Answers what did not fit (None for all of it in). `same` says whether
/// two stacks are the same item with the same components.
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
        let room = limit.saturating_sub(count_of(have));
        let moved = room.min(count_of(&stack));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn processing_speed_is_a_sixteenth_of_the_rpm_truncated_and_clamped() {
        assert_eq!(processing_speed(0.0), 1);
        assert_eq!(processing_speed(8.0), 1);
        assert_eq!(processing_speed(31.0), 1);
        assert_eq!(processing_speed(32.0), 2);
        assert_eq!(processing_speed(-64.0), 4);
        assert_eq!(processing_speed(256.0), 16);
        assert_eq!(processing_speed(1_000_000.0), 512);
    }

    #[test]
    fn a_stopped_or_full_millstone_does_nothing() {
        let mut timer = 10;
        assert_eq!(step(&mut timer, 0.0, false, false), Step::Stopped);
        assert_eq!(step(&mut timer, 16.0, true, false), Step::Stopped);
        assert_eq!(timer, 10);
    }

    #[test]
    fn the_timer_counts_down_by_the_processing_speed_and_then_processes() {
        let mut timer = 50;
        assert_eq!(step(&mut timer, 64.0, false, false), Step::Counting);
        assert_eq!(timer, 46);
        timer = 3;
        assert_eq!(step(&mut timer, 64.0, false, false), Step::Process);
        assert_eq!(timer, -1);
    }

    #[test]
    fn at_zero_an_input_asks_for_a_recipe_and_an_empty_input_idles() {
        let mut timer = 0;
        assert_eq!(step(&mut timer, 16.0, false, true), Step::Idle);
        assert_eq!(step(&mut timer, 16.0, false, false), Step::Lookup);
        // After a processed item the timer is below zero, and counts as zero.
        timer = -3;
        assert_eq!(step(&mut timer, 16.0, false, false), Step::Lookup);
        assert_eq!(timer_for(Some(150)), 150);
        assert_eq!(timer_for(None), NO_RECIPE_TIME);
    }

    #[test]
    fn slots_are_full_at_the_limit_or_the_stack_maximum() {
        assert!(!slot_full(63, 64));
        assert!(slot_full(64, 64));
        assert!(slot_full(16, 16));
        assert!(!slot_full(15, 16));
    }

    fn put(slots: &mut [Option<(char, u32)>], item: char, count: u32) -> Option<(char, u32)> {
        insert_stacked(slots, (item, count), |s| s.1, |s, n| s.1 = n, |a, b| a.0 == b.0, 64)
    }

    #[test]
    fn insert_stacked_tops_up_equal_stacks_before_empty_slots() {
        let mut slots = [None, Some(('a', 60)), None];
        assert_eq!(put(&mut slots, 'a', 10), None);
        assert_eq!(slots, [Some(('a', 6)), Some(('a', 64)), None]);
    }

    #[test]
    fn insert_stacked_gives_back_what_does_not_fit() {
        let mut slots = [Some(('a', 64)), Some(('b', 1))];
        assert_eq!(put(&mut slots, 'a', 5), Some(('a', 5)));
        assert_eq!(slots, [Some(('a', 64)), Some(('b', 1))]);
        let mut slots = [None];
        assert_eq!(put(&mut slots, 'c', 70), Some(('c', 6)));
        assert_eq!(slots, [Some(('c', 64))]);
    }
}
