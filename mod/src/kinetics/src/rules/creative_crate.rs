//! The Creative Crate's endless supply (`BottomlessItemHandler`, `CreativeCrateBlock`,
//! `WrenchableDirectionalBlock`): a handler of two slots that gives full stacks of the item set
//! in its filter without end, and takes anything put in without keeping it. No game types.
//!
//! Known differences from Create: none in these rules; see `machines/creative_crate.rs`.

use crate::{Axis, Dir};

/// `BottomlessItemHandler.getSlots`: the supply and a slot that is always empty.
pub const SLOTS: usize = 2;
/// The slot that holds the supply (`getStackInSlot(0)`).
pub const SUPPLY: usize = 0;
/// The slot that is always empty, and so takes anything (`insertItem` voids what it is given).
pub const VOID: usize = 1;

/// `BottomlessItemHandler.getStackInSlot`: how many of the filter item slot `slot` shows
/// (`copyWithCount(getMaxStackSize())`), `None` for an empty slot or no filter.
pub fn shown(slot: usize, has_filter: bool, max_stack: u32) -> Option<u32> {
    (slot == SUPPLY && has_filter).then_some(max_stack)
}

/// `BottomlessItemHandler.extractItem`: how many a take of `amount` from `slot` gives
/// (`copyWithCount(Math.min(getMaxStackSize(), amount))`), 0 for the empty slot or no filter.
pub fn extracted(slot: usize, has_filter: bool, max_stack: u32, amount: u32) -> u32 {
    if slot == SUPPLY && has_filter { max_stack.min(amount) } else { 0 }
}

/// `BottomlessItemHandler.insertItem`: what is left of a stack put in: nothing, it is voided.
pub const fn inserted_leftover() -> u32 {
    0
}

/// `WrenchableDirectionalBlock.getStateForPlacement`: it faces the way the player looks
/// (`getNearestLookingDirection`), whichever that is.
pub fn placement_facing(look: Dir) -> Dir {
    look
}

/// `WrenchableDirectionalBlock.getRotatedBlockState`: a Wrench click on a face turns the crate a
/// quarter clockwise about that face's axis, unless it already points along it.
pub fn wrenched(facing: Dir, clicked: Axis) -> Dir {
    if facing.axis() == clicked { facing } else { crate::rules::drill::clockwise(facing, clicked) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_supply_is_a_full_stack_in_the_first_slot_only() {
        assert_eq!(shown(SUPPLY, true, 64), Some(64));
        assert_eq!(shown(SUPPLY, true, 16), Some(16));
        assert_eq!(shown(VOID, true, 64), None);
        assert_eq!(shown(SUPPLY, false, 64), None);
    }

    #[test]
    fn taking_gives_as_many_as_asked_up_to_a_stack_and_never_runs_out() {
        assert_eq!(extracted(SUPPLY, true, 64, 5), 5);
        assert_eq!(extracted(SUPPLY, true, 64, 200), 64);
        assert_eq!(extracted(SUPPLY, true, 1, 8), 1);
        assert_eq!(extracted(VOID, true, 64, 5), 0);
        assert_eq!(extracted(SUPPLY, false, 64, 5), 0);
        assert_eq!(inserted_leftover(), 0);
    }

    #[test]
    fn it_faces_where_the_player_looks_and_the_wrench_turns_it_about_the_clicked_face() {
        assert_eq!(placement_facing(Dir::Down), Dir::Down);
        assert_eq!(wrenched(Dir::North, Axis::Y), Dir::East);
        assert_eq!(wrenched(Dir::Up, Axis::Y), Dir::Up);
        assert_eq!(wrenched(Dir::Up, Axis::X), Dir::South);
        assert_eq!(wrenched(Dir::East, Axis::X), Dir::East);
    }
}
