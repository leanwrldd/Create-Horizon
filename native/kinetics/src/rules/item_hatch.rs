//! The Item Hatch (`ItemHatchBlock`, `ItemHatchBlockEntity`, `ItemHatchHandler`): a use on it puts
//! the item in the player's hand (or, sneaking, everything in their inventory outside the
//! hotbar) into the inventory it faces, as much as fits, through a filter. Shulker boxes and
//! other things that can't go in a box are left alone, except packages.
//!
//! No game types: `K` is an item with its components. The inventory behind it and the player's
//! slots are plain input ([`Slot`]); what it takes and puts is plain output ([`Deposit`]).
//!
//! Known differences from Create, for the README:
//! - `ItemHandlerHelper.insertItemStacked` on an arbitrary handler is [`insert_stacked`] on a
//!   list of slots with their limits; a handler that refuses an item for other reasons
//!   (`isItemValid`) isn't modelled.
//! - `ItemHatchHandler` (a use on the hatch ignoring sneaking: `setUseBlock(TRUE)`) and the block
//!   state (`OPEN` for `CLOSE_DELAY` ticks, waterlogging, the shape) are the glue's.

use super::package::Big;
use crate::Dir;

/// `level.scheduleTick(pos, this, 10)`: the hatch stays open this many ticks.
pub const CLOSE_DELAY: i64 = 10;
/// `Inventory.items.size()`.
pub const PLAYER_SLOTS: usize = 36;
/// `Inventory.isHotbarSlot(i)`: the first nine slots.
pub const HOTBAR_SLOTS: usize = 9;

/// A slot of the inventory the hatch faces: its stack and the slot's limit (`getSlotLimit`).
#[derive(Clone, Debug, PartialEq)]
pub struct Slot<K> {
    pub stack: Option<Big<K>>,
    pub limit: u32,
}

/// `ItemHatchBlock.getStateForPlacement`: not on the top or bottom of a block; it faces the
/// block it is put against.
pub fn placement_facing(clicked_face: Dir) -> Option<Dir> {
    if matches!(clicked_face, Dir::Up | Dir::Down) { None } else { Some(clicked_face.opposite()) }
}

/// `ItemHandlerHelper.insertItemStacked` (simulated or not): onto stacks of the same item first
/// (unless the item doesn't stack), then into empty slots; each slot takes up to its limit and
/// the item's stack size. Returns what didn't fit.
pub fn insert_stacked<K: PartialEq + Clone>(
    slots: &mut [Slot<K>],
    key: &K,
    count: u32,
    max_stack: u32,
    simulate: bool,
) -> u32 {
    let mut left = count;
    let mut working: Vec<Slot<K>> = if simulate { slots.to_vec() } else { Vec::new() };
    let target: &mut [Slot<K>] = if simulate { &mut working } else { slots };
    if max_stack > 1 {
        for slot in target.iter_mut() {
            if left == 0 {
                return 0;
            }
            if let Some((held, have)) = &mut slot.stack
                && held == key
            {
                let moved = slot.limit.min(max_stack).saturating_sub(*have).min(left);
                *have += moved;
                left -= moved;
            }
        }
    }
    for slot in target.iter_mut() {
        if left == 0 {
            break;
        }
        if slot.stack.is_none() {
            let moved = slot.limit.min(max_stack).min(left);
            if moved > 0 {
                slot.stack = Some((key.clone(), moved));
                left -= moved;
            }
        }
    }
    left
}

/// What a use on the hatch decided.
#[derive(Clone, Debug, PartialEq)]
pub enum HatchUse<K> {
    /// The use fails (no inventory behind it, or no filter behaviour): `ItemInteractionResult.FAIL`.
    Fail,
    /// The default block interaction: sneaking with a wrench (which picks the hatch up).
    PassToDefault,
    /// The use ran; see [`Deposit`].
    Used(Deposit<K>),
}

/// What a deposit did.
#[derive(Clone, Debug, PartialEq)]
pub struct Deposit<K> {
    /// `(slot, count)` taken from the player's inventory, in slot order.
    pub removed: Vec<(usize, u32)>,
    /// The inventory behind the hatch afterwards.
    pub target: Vec<Slot<K>>,
    /// What the handler gave back after it said it would take it (`placeItemBackInInventory`);
    /// empty unless the handler misbehaved.
    pub failed: Vec<Big<K>>,
    /// Something went in: the hatch sound plays, the hatch opens for [`CLOSE_DELAY`] ticks and the
    /// message below shows. Nothing in means nothing happens.
    pub any_inserted: bool,
    /// "item_hatch.deposit_item" (the item in hand) or "item_hatch.deposit_inventory".
    pub deposit_item_in_hand: bool,
}

/// The hatch's filter as `use_hatch` reads it: none without a filter behaviour, `Some(None)` with the
/// slot empty, `Some(Some(test))` with a filter set.
pub type FilterRead<'a, K> = Option<Option<&'a dyn Fn(&K) -> bool>>;

/// What the hatch reads of the player.
pub struct Player<'a, K> {
    /// The 36 main slots (`inventory.items`).
    pub items: &'a [Option<Big<K>>],
    /// `inventory.selected`.
    pub selected: usize,
    pub sneaking: bool,
    /// The held item is a wrench (`Items.TOOLS_WRENCH`).
    pub holding_wrench: bool,
}

/// `ItemHatchBlock.useItemOn`, on the server: `target` is the inventory the hatch faces (none
/// when there is no block entity or item handler there), `filter` the hatch's filter (none when
/// it has no filter behaviour, `Some(None)` with the slot empty, `Some(Some(test))` with a filter
/// set), `max_stack` an item's stack size, `fits_in_boxes` whether it can go in a box
/// (`canFitInsideContainerItems`) or is a package.
pub fn use_hatch<K: PartialEq + Clone>(
    player: &Player<'_, K>,
    target: Option<&[Slot<K>]>,
    filter: FilterRead<'_, K>,
    max_stack: &dyn Fn(&K) -> u32,
    fits_in_boxes: &dyn Fn(&K) -> bool,
) -> HatchUse<K> {
    let Some(target) = target else {
        return HatchUse::Fail;
    };
    let Some(filter) = filter else {
        return HatchUse::Fail;
    };

    let deposit_item_in_hand = !player.sneaking;
    if !deposit_item_in_hand && player.holding_wrench {
        return HatchUse::PassToDefault;
    }

    let mut inventory: Vec<Slot<K>> = target.to_vec();
    let mut removed: Vec<(usize, u32)> = Vec::new();
    let mut failed: Vec<Big<K>> = Vec::new();
    let mut any_inserted = false;

    for (i, slot) in player.items.iter().enumerate().take(PLAYER_SLOTS) {
        if (i < HOTBAR_SLOTS) != deposit_item_in_hand {
            continue;
        }
        if deposit_item_in_hand && i != player.selected {
            continue;
        }
        let Some((key, count)) = slot else {
            continue;
        };
        if !fits_in_boxes(key) {
            continue;
        }
        if let Some(test) = filter
            && !test(key)
        {
            continue;
        }

        let remainder = insert_stacked(&mut inventory, key, *count, max_stack(key), true);
        if remainder == *count {
            continue;
        }

        let extracted = count - remainder;
        removed.push((i, extracted));
        let remainder = insert_stacked(&mut inventory, key, extracted, max_stack(key), false);
        any_inserted = true;

        // "remainder might not be empty in itemhandler edge cases".
        if remainder > 0 {
            failed.push((key.clone(), remainder));
        }
    }

    HatchUse::Used(Deposit { removed, target: inventory, failed, any_inserted, deposit_item_in_hand })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chest(slots: usize) -> Vec<Slot<&'static str>> {
        (0..slots).map(|_| Slot { stack: None, limit: 64 }).collect()
    }

    fn max(key: &&str) -> u32 {
        match *key {
            "egg" => 16,
            "shulker" | "package" => 1,
            _ => 64,
        }
    }

    fn fits(key: &&str) -> bool {
        *key != "shulker"
    }

    fn player(items: &[(usize, &'static str, u32)]) -> Vec<Option<Big<&'static str>>> {
        let mut slots: Vec<Option<Big<&'static str>>> = (0..PLAYER_SLOTS).map(|_| None).collect();
        for &(slot, key, count) in items {
            slots[slot] = Some((key, count));
        }
        slots
    }

    fn deposit(
        items: &[Option<Big<&'static str>>],
        selected: usize,
        sneaking: bool,
        target: &[Slot<&'static str>],
        filter: Option<&dyn Fn(&&'static str) -> bool>,
    ) -> Deposit<&'static str> {
        let player = Player { items, selected, sneaking, holding_wrench: false };
        match use_hatch(&player, Some(target), Some(filter), &max, &fits) {
            HatchUse::Used(d) => d,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_hand_puts_the_selected_stack_in_the_inventory() {
        // Slot 2 selected: 40 iron goes in; the iron in slot 3 and the dirt in slot 12 stay.
        let items = player(&[(2, "iron", 40), (3, "iron", 7), (12, "dirt", 5)]);
        let target = chest(3);
        let d = deposit(&items, 2, false, &target, None);
        assert!(d.deposit_item_in_hand && d.any_inserted);
        assert_eq!(d.removed, vec![(2, 40)]);
        assert_eq!(d.target[0].stack, Some(("iron", 40)));
        assert!(d.failed.is_empty());
    }

    #[test]
    fn sneaking_fills_the_inventory_below_from_the_players_inventory_with_a_filter() {
        // Hotbar slots are left alone; the rest goes, through the filter: only iron.
        let items = player(&[(0, "iron", 10), (9, "iron", 64), (10, "dirt", 30), (20, "iron", 30), (35, "iron", 5)]);
        let mut target = chest(2);
        target[0].stack = Some(("iron", 60));
        let only_iron = |k: &&str| *k == "iron";
        let d = deposit(&items, 0, true, &target, Some(&only_iron));
        assert!(!d.deposit_item_in_hand);
        // Computed from `insertItemStacked` (onto existing stacks first, then empty slots):
        // slot 9's 64 iron: 4 onto the 60, 60 into the empty slot, all taken. Slot 20's 30 iron:
        // the second slot has 4 of room left, so 4 are taken and 26 stay. Slot 35's 5 find no room.
        assert_eq!(d.removed, vec![(9, 64), (20, 4)]);
        assert_eq!(d.target[0].stack, Some(("iron", 64)));
        assert_eq!(d.target[1].stack, Some(("iron", 64)));
        // The filter held back the dirt, and the hotbar iron stayed.
        assert!(d.removed.iter().all(|(slot, _)| *slot != 10 && *slot != 0));
        // The full inventory takes nothing more: nothing is inserted, nothing happens.
        let full = d.target.clone();
        let again = deposit(&items, 0, true, &full, Some(&only_iron));
        assert!(!again.any_inserted && again.removed.is_empty());
    }

    #[test]
    fn without_a_filter_everything_goes_but_things_that_cant_go_in_a_box() {
        let items = player(&[(9, "dirt", 20), (10, "shulker", 1), (11, "package", 1), (12, "egg", 20)]);
        let d = deposit(&items, 0, true, &chest(5), None);
        // The shulker box stays; the package goes in; eggs stack to 16 (two slots).
        assert_eq!(d.removed, vec![(9, 20), (11, 1), (12, 20)]);
        let held: Vec<_> = d.target.iter().map(|s| s.stack).collect();
        assert_eq!(held, vec![Some(("dirt", 20)), Some(("package", 1)), Some(("egg", 16)), Some(("egg", 4)), None]);
    }

    #[test]
    fn nothing_behind_it_fails_and_a_wrench_picks_it_up_while_sneaking() {
        let items = player(&[(0, "iron", 1)]);
        let p = Player { items: &items, selected: 0, sneaking: false, holding_wrench: false };
        assert_eq!(use_hatch(&p, None, Some(None), &max, &fits), HatchUse::Fail);
        assert_eq!(use_hatch(&p, Some(&chest(1)), None, &max, &fits), HatchUse::Fail);
        let wrench = Player { sneaking: true, holding_wrench: true, ..Player { items: &items, selected: 0, sneaking: false, holding_wrench: false } };
        assert_eq!(use_hatch(&wrench, Some(&chest(1)), Some(None), &max, &fits), HatchUse::PassToDefault);
        // A wrench in hand but not sneaking deposits as usual (a wrench isn't filtered out).
        let plain = Player { holding_wrench: true, ..Player { items: &items, selected: 0, sneaking: false, holding_wrench: false } };
        assert!(matches!(use_hatch(&plain, Some(&chest(1)), Some(None), &max, &fits), HatchUse::Used(_)));
    }

    #[test]
    fn a_hatch_is_not_placed_on_a_top_or_bottom_and_faces_the_block() {
        assert_eq!(placement_facing(Dir::Up), None);
        assert_eq!(placement_facing(Dir::Down), None);
        assert_eq!(placement_facing(Dir::North), Some(Dir::South));
    }

    #[test]
    fn slot_limits_cap_what_goes_in() {
        // A slot that holds only 10 (a limited handler): 25 items take three slots.
        let mut target: Vec<Slot<&'static str>> = (0..3).map(|_| Slot { stack: None, limit: 10 }).collect();
        assert_eq!(insert_stacked(&mut target, &"iron", 25, 64, true), 0);
        assert!(target.iter().all(|s| s.stack.is_none()), "a simulation changes nothing");
        assert_eq!(insert_stacked(&mut target, &"iron", 25, 64, false), 0);
        let held: Vec<_> = target.iter().map(|s| s.stack).collect();
        assert_eq!(held, vec![Some(("iron", 10)), Some(("iron", 10)), Some(("iron", 5))]);
        assert_eq!(insert_stacked(&mut target, &"iron", 10, 64, false), 5);
    }
}
