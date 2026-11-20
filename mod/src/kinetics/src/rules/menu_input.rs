//! The clicks of a chest view with input slots (`Host::open_menu_with_input`'s stand-in), as a plain state machine.
//!
//! The new API's chest menu (`open_menu`) moves nothing on a click: each click reaches the mod (`menu_clicked`), which
//! has to do what the menu would. This is that, for a chest view of `size` slots followed by the player's 27 main and
//! 9 hotbar slots (`ChestMenu`'s order): the PICKUP, QUICK_MOVE, SWAP and PICKUP_ALL clicks, mirrored from Horizon's
//! `menu/click.rs` (`AbstractContainerMenu.doClick`, `moveItemStackTo`, `Slot.safeInsert`). No game types: a stack is
//! the Toolbox's [`Stack`], so the glue builds both from the real ones.
//!
//! Not done (the view is simply left as it was, and the server's next sync shows that): drag (QUICK_CRAFT), THROW and
//! CLONE clicks, a click outside the window (it would drop the cursor stack) and a swap with the off hand.

use std::ops::Range;

use super::toolbox::{Stack, SLOT_LIMIT};

/// `ClickType`'s ids as the click packet carries them.
pub const PICKUP: i32 = 0;
pub const QUICK_MOVE: i32 = 1;
pub const SWAP: i32 = 2;
pub const PICKUP_ALL: i32 = 6;

/// The player's inventory below the view: 27 main slots, then the 9 of the hotbar.
pub const PLAYER_SLOTS: usize = 36;
const HOTBAR: usize = 9;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Menu {
    /// The view's slots, then the player's 36.
    pub slots: Vec<Option<Stack>>,
    pub cursor: Option<Stack>,
    /// How many slots the view has of its own.
    pub size: usize,
    /// Which of them take items (`Slot.mayPlace`); the rest only give them up, as a result slot does.
    pub input: Range<usize>,
}

fn same(a: &Stack, b: &Stack) -> bool {
    super::toolbox::same_stack(&a.item, &b.item)
}

fn tidy(stack: &mut Option<Stack>) {
    if stack.as_ref().is_some_and(|s| s.count == 0) {
        *stack = None;
    }
}

impl Menu {
    pub fn new(view: Vec<Option<Stack>>, player: Vec<Option<Stack>>, input: Range<usize>) -> Self {
        let size = view.len();
        let mut slots = view;
        let mut player = player;
        player.resize(PLAYER_SLOTS, None);
        slots.extend(player);
        Self { slots, cursor: None, size, input }
    }

    pub fn view(&self) -> &[Option<Stack>] {
        &self.slots[..self.size]
    }

    fn may_place(&self, slot: usize) -> bool {
        slot >= self.size || self.input.contains(&slot)
    }

    /// `Slot.getMaxStackSize(stack)`: the container's 64, or the item's own smaller size.
    fn limit(stack: &Stack) -> u32 {
        SLOT_LIMIT.min(stack.item.max_stack)
    }

    /// `Slot.safeInsert(stack, amount)`: moves up to `amount` of `stack` into slot `s`, what fits.
    fn insert(&mut self, s: usize, stack: &mut Stack, amount: u32) {
        if !self.may_place(s) {
            return;
        }
        let limit = Self::limit(stack);
        let moved = match &mut self.slots[s] {
            Some(have) if same(have, stack) => {
                let room = limit.saturating_sub(have.count);
                let moved = amount.min(stack.count).min(room);
                have.count += moved;
                moved
            }
            Some(_) => 0,
            empty => {
                let moved = amount.min(stack.count).min(limit);
                if moved > 0 {
                    *empty = Some(Stack::new(stack.item.clone(), moved));
                }
                moved
            }
        };
        stack.count -= moved;
    }

    /// Takes up to `amount` from slot `s`, leaving the slot to keep the rest (`Slot.safeTake`).
    fn take(&mut self, s: usize, amount: u32) -> Option<Stack> {
        let have = self.slots[s].as_mut()?;
        let taken = amount.min(have.count);
        if taken == 0 {
            return None;
        }
        have.count -= taken;
        let out = Stack::new(have.item.clone(), taken);
        tidy(&mut self.slots[s]);
        Some(out)
    }

    /// `AbstractContainerMenu.clicked`: whether anything could have changed.
    pub fn click(&mut self, slot: i32, button: i32, mode: i32) {
        let Ok(s) = usize::try_from(slot) else { return };
        if s >= self.slots.len() {
            return;
        }
        match mode {
            PICKUP if button == 0 || button == 1 => self.pickup(s, button == 0),
            QUICK_MOVE if button == 0 || button == 1 => {
                // `while (!clicked.isEmpty() && ItemStack.isSameItem(slot, clicked))`: the same stack again.
                let mut moved = self.quick_move(s);
                while let Some(stack) = moved {
                    if !self.slots[s].as_ref().is_some_and(|now| same(now, &stack)) {
                        break;
                    }
                    moved = self.quick_move(s);
                }
            }
            SWAP if (0..9).contains(&button) => self.swap(s, button as usize),
            PICKUP_ALL => self.pickup_all(s, button),
            _ => {}
        }
    }

    /// The PICKUP branch of `doClick`.
    fn pickup(&mut self, s: usize, primary: bool) {
        let clicked = self.slots[s].clone();
        let carried = self.cursor.take();
        match (clicked, carried) {
            (None, None) => {}
            (None, Some(mut carried)) => {
                let amount = if primary { carried.count } else { 1 };
                self.insert(s, &mut carried, amount);
                self.cursor = Some(carried);
                tidy(&mut self.cursor);
            }
            (Some(clicked), None) => {
                let amount = if primary { clicked.count } else { clicked.count.div_ceil(2) };
                self.cursor = self.take(s, amount);
            }
            (Some(clicked), Some(mut carried)) => {
                if self.may_place(s) {
                    if same(&clicked, &carried) {
                        let amount = if primary { carried.count } else { 1 };
                        self.insert(s, &mut carried, amount);
                        self.cursor = Some(carried);
                        tidy(&mut self.cursor);
                    } else if carried.count <= Self::limit(&carried) {
                        // Swap: the slot's stack goes on the cursor.
                        self.cursor = Some(clicked);
                        self.slots[s] = Some(carried);
                    } else {
                        self.cursor = Some(carried);
                    }
                } else if same(&clicked, &carried) {
                    // A take-only slot adds to the cursor up to the item's stack size.
                    let room = carried.item.max_stack.saturating_sub(carried.count);
                    let took = self.take(s, room.min(clicked.count));
                    let mut carried = carried;
                    carried.count += took.map_or(0, |t| t.count);
                    self.cursor = Some(carried);
                } else {
                    self.cursor = Some(carried);
                }
            }
        }
    }

    /// `AbstractContainerMenu.moveItemStackTo`: first onto stacks of the same kind, then into empty slots, from
    /// `start` up (or `end` down when `backwards`). What is left stays in `stack`.
    fn move_to(&mut self, stack: &mut Stack, range: Range<usize>, backwards: bool) {
        let order = |from: &Range<usize>| -> Vec<usize> {
            if backwards { from.clone().rev().collect() } else { from.clone().collect() }
        };
        let slots = order(&range);
        if stack.item.stackable() {
            for &i in &slots {
                if stack.count == 0 {
                    break;
                }
                if self.may_place(i) && self.slots[i].as_ref().is_some_and(|have| same(have, stack)) {
                    let amount = stack.count;
                    self.insert(i, stack, amount);
                }
            }
        }
        for &i in &slots {
            if stack.count == 0 {
                break;
            }
            if self.slots[i].is_none() && self.may_place(i) {
                let amount = stack.count;
                self.insert(i, stack, amount);
            }
        }
    }

    /// `ChestMenu.quickMoveStack`: the view's slots go to the inventory (the last slot first), the inventory's into
    /// the view. The stack as it was when moving began, if anything moved.
    fn quick_move(&mut self, s: usize) -> Option<Stack> {
        let original = self.slots[s].clone()?;
        let mut stack = original.clone();
        let total = self.slots.len();
        if s < self.size {
            self.move_to(&mut stack, self.size..total, true);
        } else {
            self.move_to(&mut stack, 0..self.size, false);
        }
        if stack.count == original.count {
            return None;
        }
        self.slots[s] = if stack.count == 0 { None } else { Some(stack) };
        Some(original)
    }

    /// The SWAP branch of `doClick`: the slot and hotbar slot `button` exchange.
    fn swap(&mut self, target: usize, button: usize) {
        let hotbar = self.size + PLAYER_SLOTS - HOTBAR + button;
        if target == hotbar {
            return;
        }
        let source = self.slots[hotbar].clone();
        let there = self.slots[target].clone();
        match (source, there) {
            (None, None) => {}
            (None, Some(taken)) => {
                self.slots[hotbar] = Some(taken);
                self.slots[target] = None;
            }
            (Some(mut source), None) => {
                if self.may_place(target) {
                    let max = Self::limit(&source);
                    if source.count > max {
                        let part = Stack::new(source.item.clone(), max);
                        source.count -= max;
                        self.slots[hotbar] = Some(source);
                        self.slots[target] = Some(part);
                    } else {
                        self.slots[hotbar] = None;
                        self.slots[target] = Some(source);
                    }
                }
            }
            (Some(mut source), Some(taken)) => {
                if self.may_place(target) {
                    let max = Self::limit(&source);
                    if source.count > max {
                        // What doesn't fit stays in the hotbar slot; the old stack goes to the inventory's room.
                        let part = Stack::new(source.item.clone(), max);
                        source.count -= max;
                        self.slots[hotbar] = Some(source);
                        self.slots[target] = Some(part);
                        let mut taken = taken;
                        let end = self.slots.len();
                        self.move_to(&mut taken, self.size..end, true);
                    } else {
                        self.slots[hotbar] = Some(taken);
                        self.slots[target] = Some(source);
                    }
                }
            }
        }
    }

    /// The PICKUP_ALL branch of `doClick` (a double click): everything of the cursor's kind, partly filled stacks
    /// first, from the first slot (left button) or the last.
    fn pickup_all(&mut self, s: usize, button: i32) {
        let Some(carried) = self.cursor.clone() else { return };
        if self.slots[s].is_some() {
            return;
        }
        let total = self.slots.len();
        let max = carried.item.max_stack;
        let order: Vec<usize> = if button == 0 { (0..total).collect() } else { (0..total).rev().collect() };
        for pass in 0..2 {
            for &t in &order {
                let have = self.cursor.as_ref().map_or(0, |c| c.count);
                if have >= max {
                    break;
                }
                let Some(item) = self.slots[t].clone() else { continue };
                if !same(&item, &carried) || (pass == 0 && item.count == item.item.max_stack) {
                    continue;
                }
                if let Some(taken) = self.take(t, (max - have).min(item.count))
                    && let Some(cursor) = self.cursor.as_mut()
                {
                    cursor.count += taken.count;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::toolbox::Item;

    fn item(id: &str, max: u32) -> Item {
        Item { id: id.into(), components: String::new(), max_stack: max, damageable: false, fits: true }
    }
    fn stack(id: &str, max: u32, count: u32) -> Option<Stack> {
        Some(Stack::new(item(id, max), count))
    }
    fn menu(view: Vec<Option<Stack>>) -> Menu {
        let size = view.len();
        Menu::new(view, vec![], 0..size)
    }

    #[test]
    fn left_click_takes_a_stack_and_puts_it_down() {
        let mut m = menu(vec![stack("a", 64, 10), None, None]);
        m.click(0, 0, PICKUP);
        assert_eq!(m.cursor, stack("a", 64, 10));
        assert_eq!(m.slots[0], None);
        m.click(2, 0, PICKUP);
        assert_eq!(m.cursor, None);
        assert_eq!(m.slots[2], stack("a", 64, 10));
    }

    #[test]
    fn right_click_takes_half_rounded_up_and_places_one() {
        let mut m = menu(vec![stack("a", 64, 5), None]);
        m.click(0, 1, PICKUP);
        assert_eq!(m.cursor, stack("a", 64, 3));
        assert_eq!(m.slots[0], stack("a", 64, 2));
        m.click(1, 1, PICKUP);
        assert_eq!(m.slots[1], stack("a", 64, 1));
        assert_eq!(m.cursor, stack("a", 64, 2));
    }

    #[test]
    fn another_kind_swaps_and_the_same_kind_stacks_up_to_the_limit() {
        let mut m = menu(vec![stack("a", 64, 60), stack("b", 64, 1)]);
        m.cursor = stack("a", 64, 10);
        m.click(0, 0, PICKUP);
        assert_eq!(m.slots[0], stack("a", 64, 64));
        assert_eq!(m.cursor, stack("a", 64, 6));
        m.click(1, 0, PICKUP);
        assert_eq!(m.slots[1], stack("a", 64, 6));
        assert_eq!(m.cursor, stack("b", 64, 1));
    }

    #[test]
    fn an_unstackable_item_stays_at_one() {
        let mut m = menu(vec![stack("pick", 1, 1), None]);
        m.click(0, 0, PICKUP);
        m.click(1, 1, PICKUP);
        assert_eq!(m.slots[1], stack("pick", 1, 1));
        assert_eq!(m.cursor, None);
    }

    #[test]
    fn a_slot_outside_the_input_only_gives() {
        let mut m = Menu::new(vec![stack("a", 64, 3), None], vec![], 1..2);
        m.cursor = stack("a", 64, 2);
        m.click(0, 0, PICKUP);
        // Not placed on the output; it is taken from up to the cursor's room.
        assert_eq!(m.slots[0], None);
        assert_eq!(m.cursor, stack("a", 64, 5));
        m.click(1, 0, PICKUP);
        assert_eq!(m.slots[1], stack("a", 64, 5));
    }

    #[test]
    fn shift_click_moves_between_the_view_and_the_inventory() {
        let mut m = Menu::new(vec![stack("a", 64, 10), None], vec![stack("a", 64, 60)], 0..2);
        // From the inventory (menu slot 2): onto the same kind first, then the empty slot.
        m.click(2, 0, QUICK_MOVE);
        assert_eq!(m.slots[0], stack("a", 64, 64));
        assert_eq!(m.slots[1], stack("a", 64, 6));
        assert_eq!(m.slots[2], None);
        // From the view back: the last inventory slot first.
        m.click(1, 0, QUICK_MOVE);
        assert_eq!(m.slots[1], None);
        assert_eq!(m.slots[2 + PLAYER_SLOTS - 1], stack("a", 64, 6));
    }

    #[test]
    fn number_key_swaps_with_the_hotbar_slot() {
        let mut m = Menu::new(vec![stack("a", 64, 4), None], vec![], 0..2);
        let hotbar = 2 + PLAYER_SLOTS - 9;
        m.click(0, 0, SWAP);
        assert_eq!(m.slots[hotbar], stack("a", 64, 4));
        assert_eq!(m.slots[0], None);
        m.click(1, 0, SWAP);
        assert_eq!(m.slots[1], stack("a", 64, 4));
        assert_eq!(m.slots[hotbar], None);
    }

    #[test]
    fn double_click_gathers_the_kind_onto_the_cursor() {
        let mut m = menu(vec![None, stack("a", 64, 20), stack("a", 64, 64), stack("b", 64, 5)]);
        m.cursor = stack("a", 64, 1);
        m.click(0, 0, PICKUP_ALL);
        assert_eq!(m.cursor, stack("a", 64, 64));
        assert_eq!(m.slots[1], None);
        assert_eq!(m.slots[2], stack("a", 64, 21));
        assert_eq!(m.slots[3], stack("b", 64, 5));
    }
}
