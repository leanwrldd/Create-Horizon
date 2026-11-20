//! Slot numbers of a player's inventory, in the two numberings the game API uses, and the tally of a carried
//! inventory. Pure numbers: the compat layer reads and writes the slots.

/// Inventory numbers: 0 to 35 the main inventory (0 to 8 the hotbar), 36 feet, 37 legs, 38 chest, 39 head, 40 the off hand.
/// `inventory_tick` numbers (the way the inventory saves them): 0 to 35, 100 feet to 103 head, -106 the off hand.
/// Returns the first for the second; `None` for a number that is neither.
pub fn inventory_number(tick_slot: i32) -> Option<u32> {
    match tick_slot {
        0..=35 => Some(tick_slot as u32),
        100..=103 => Some(36 + (tick_slot - 100) as u32),
        -106 => Some(40),
        _ => None,
    }
}

/// The reverse of [`inventory_number`]: the `inventory_tick` number for an inventory number (41 and 42 have none).
pub fn tick_slot(inventory_number: u32) -> Option<i32> {
    match inventory_number {
        0..=35 => Some(inventory_number as i32),
        36..=39 => Some(100 + (inventory_number - 36) as i32),
        40 => Some(-106),
        _ => None,
    }
}

/// `(item, count)` of each plain stack summed by item, in the order an item first appears
/// (what the old `carried_items` told: only stacks with no components and no damage can be taken as a plain item).
pub fn tally<'a>(stacks: impl IntoIterator<Item = (&'a str, u32)>) -> Vec<(String, u32)> {
    let mut out: Vec<(String, u32)> = Vec::new();
    for (item, count) in stacks {
        if count == 0 {
            continue;
        }
        match out.iter_mut().find(|(have, _)| have == item) {
            Some((_, total)) => *total += count,
            None => out.push((item.to_owned(), count)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_numberings_meet() {
        assert_eq!(inventory_number(0), Some(0));
        assert_eq!(inventory_number(35), Some(35));
        assert_eq!(inventory_number(100), Some(36));
        assert_eq!(inventory_number(103), Some(39));
        assert_eq!(inventory_number(-106), Some(40));
        assert_eq!(inventory_number(36), None);
        assert_eq!(inventory_number(104), None);
        assert_eq!(inventory_number(-1), None);
    }

    #[test]
    fn they_round_trip() {
        for slot in (0..=35).chain(100..=103).chain([-106]) {
            assert_eq!(tick_slot(inventory_number(slot).unwrap()), Some(slot));
        }
        assert_eq!(tick_slot(41), None);
        assert_eq!(tick_slot(42), None);
    }

    #[test]
    fn the_tally_sums_by_item_in_first_seen_order() {
        let t = tally([("a:b", 3), ("c:d", 1), ("a:b", 5), ("e:f", 0)]);
        assert_eq!(t, vec![("a:b".to_owned(), 8), ("c:d".to_owned(), 1)]);
    }
}
