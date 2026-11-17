//! What a click on a value box does, before any value is set: Create's right-click handling
//! (`ValueSettingsInputHandler`, `ValueSettingsClient`) and what a short click does to a filter slot
//! (`FilteringBehaviour.onShortInteract`).
//!
//! A box that takes a value board (a scroll value, an option, a filter with an amount) waits for the click to be
//! held: let go in under six ticks and it is a short click; hold on and the board opens. A filter slot with no
//! amount has no board, so its short click happens at once.

use crate::rules::value_box_board::Values;

/// Ticks a click is held before the board opens (`ValueSettingsClient.tick`: `interactHeldTicks++ < 5`).
pub const HOLD_TICKS: i32 = 5;

/// What the held right click on a box does this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Held {
    /// Still waiting.
    Waiting,
    /// The button came up before the board opened: a short click (`onShortInteract`).
    Short,
    /// Held long enough: open the board.
    Open,
    /// The player looked away, put the item away or the box is gone: nothing happens.
    Cancelled,
}

/// `ValueSettingsClient`'s `interactHeldTicks`: the click being held on a box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hold {
    ticks: i32,
}

impl Hold {
    /// `startInteractionWith`.
    pub fn start() -> Self {
        Self { ticks: 0 }
    }

    /// `tick`: `valid` is whether the player can still interact and still points at the same box,
    /// `down` whether the use button is still held.
    pub fn tick(&mut self, valid: bool, down: bool) -> Held {
        if !valid {
            return Held::Cancelled;
        }
        if !down {
            return Held::Short;
        }
        if self.ticks < HOLD_TICKS {
            self.ticks += 1;
            return Held::Waiting;
        }
        Held::Open
    }
}

/// What the player holds, as a short click sees it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hand {
    pub empty: bool,
    /// `AllItems.WRENCH.isIn`.
    pub wrench: bool,
    /// `AllBlocks.MECHANICAL_ARM.isIn`: the arm's item is placed to point at the slot's inventory, not set as a filter.
    pub arm: bool,
    /// A Filter item (`create:filter`, `create:attribute_filter`, the package filter): the slot takes it whole.
    pub filter_item: bool,
}

/// What a short click does to the filter (`FilteringBehaviour.onShortInteract`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Short {
    /// Nothing: `canShortInteract` is false (a Wrench or the arm's item). The click is still taken.
    Nothing,
    /// The filter becomes what the hand holds (nothing, for an empty hand).
    Set {
        /// The old Filter item goes back to the player's inventory (`placeItemBackInInventory`): a plain item set
        /// as a filter is only a copy of the item, nothing to give back.
        refund: bool,
        /// A survival player gives up the Filter item they set (one of the stack).
        consume: bool,
    },
}

/// `onShortInteract`: `old_is_filter_item` says the slot holds a Filter item, `creative` is the player's mode and
/// `creative_copy` whether a creative player has another of that filter in their inventory (then there is
/// nothing to give back: `!player.isCreative() || extract(...).isEmpty()`).
pub fn short_interact(hand: Hand, old_is_filter_item: bool, creative: bool, creative_copy: bool) -> Short {
    if hand.wrench || hand.arm {
        return Short::Nothing;
    }
    Short::Set {
        refund: old_is_filter_item && (!creative || !creative_copy),
        consume: !creative && hand.filter_item && !hand.empty,
    }
}

/// `FilteringBehaviour.setFilter(stack)` on the saved values: a new item keeps the amount, but an exact one is
/// cut to the new item's stack size (`if (!upTo && !stack.isEmpty()) count = min(count, maxStack)`).
pub fn after_set(old: &Values, new_max_stack: Option<i32>) -> Values {
    let mut next = *old;
    if let Some(max) = new_max_stack {
        next.max_stack = max;
        if !old.up_to {
            next.count = old.count.min(max);
        }
    } else {
        next.max_stack = 64;
    }
    next
}

// --- what a client tells the server ---------------------------------------------------------------------

/// The channel the messages travel on.
pub const CHANNEL: &str = "create:value_box";

/// What the client says about a box (`ValueSettingsPacket`): a short click that released before the board
/// opened (`interactHand` set), or the setting the board ended on. `side` is the face looked at
/// (`Direction.get3DDataValue`: down, up, north, south, west, east).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {
    Short { pos: [i32; 3], net_id: u8, side: u8 },
    Set { pos: [i32; 3], net_id: u8, side: u8, row: i32, value: i32 },
}

impl Message {
    pub fn encode(&self) -> Vec<u8> {
        let (tag, pos, net_id, side, row, value) = match *self {
            Message::Short { pos, net_id, side } => (0u8, pos, net_id, side, 0, 0),
            Message::Set { pos, net_id, side, row, value } => (1u8, pos, net_id, side, row, value),
        };
        let mut out = vec![tag, net_id, side];
        for number in [pos[0], pos[1], pos[2], row, value] {
            out.extend(number.to_be_bytes());
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != 3 + 5 * 4 {
            return None;
        }
        let number = |i: usize| i32::from_be_bytes(bytes[3 + i * 4..7 + i * 4].try_into().expect("four bytes"));
        let (net_id, side) = (bytes[1], bytes[2]);
        if side > 5 {
            return None;
        }
        let pos = [number(0), number(1), number(2)];
        match bytes[0] {
            0 => Some(Message::Short { pos, net_id, side }),
            1 => Some(Message::Set { pos, net_id, side, row: number(3), value: number(4) }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_survives_the_wire() {
        let short = Message::Short { pos: [-3, 64, 1_000_000], net_id: 1, side: 4 };
        assert_eq!(Message::decode(&short.encode()), Some(short));
        let set = Message::Set { pos: [5, -60, 7], net_id: 0, side: 1, row: 1, value: 256 };
        assert_eq!(Message::decode(&set.encode()), Some(set));
        assert_eq!(Message::decode(&[]), None);
        assert_eq!(Message::decode(&set.encode()[..10]), None);
        let mut bad = set.encode();
        bad[2] = 9;
        assert_eq!(Message::decode(&bad), None);
        let mut bad = set.encode();
        bad[0] = 7;
        assert_eq!(Message::decode(&bad), None);
    }

    #[test]
    fn a_short_click_is_released_before_the_sixth_tick() {
        let mut hold = Hold::start();
        for _ in 0..HOLD_TICKS {
            assert_eq!(hold.tick(true, true), Held::Waiting);
        }
        assert_eq!(hold.tick(true, true), Held::Open);
        let mut hold = Hold::start();
        assert_eq!(hold.tick(true, true), Held::Waiting);
        assert_eq!(hold.tick(true, false), Held::Short);
    }

    #[test]
    fn looking_away_cancels_the_click() {
        let mut hold = Hold::start();
        assert_eq!(hold.tick(false, true), Held::Cancelled);
        // Even a released button doesn't count as a short click then.
        assert_eq!(Hold::start().tick(false, false), Held::Cancelled);
    }

    #[test]
    fn the_wrench_and_the_arm_item_are_not_filters() {
        let wrench = Hand { wrench: true, ..Hand::default() };
        let arm = Hand { arm: true, ..Hand::default() };
        assert_eq!(short_interact(wrench, false, false, false), Short::Nothing);
        assert_eq!(short_interact(arm, true, false, false), Short::Nothing);
    }

    #[test]
    fn an_item_sets_and_an_empty_hand_clears() {
        let item = Hand::default();
        assert_eq!(short_interact(item, false, false, false), Short::Set { refund: false, consume: false });
        let empty = Hand { empty: true, ..Hand::default() };
        assert_eq!(short_interact(empty, false, false, false), Short::Set { refund: false, consume: false });
    }

    #[test]
    fn a_filter_item_is_taken_from_a_survival_hand_and_the_old_one_given_back() {
        let filter = Hand { filter_item: true, ..Hand::default() };
        assert_eq!(short_interact(filter, true, false, false), Short::Set { refund: true, consume: true });
        // Creative keeps the stack, and gives the old filter back only if they have no other of it.
        assert_eq!(short_interact(filter, true, true, false), Short::Set { refund: true, consume: false });
        assert_eq!(short_interact(filter, true, true, true), Short::Set { refund: false, consume: false });
        // Clearing the slot with an empty hand gives a Filter item back.
        let empty = Hand { empty: true, ..Hand::default() };
        assert_eq!(short_interact(empty, true, false, false), Short::Set { refund: true, consume: false });
    }

    #[test]
    fn a_new_filter_item_cuts_an_exact_amount() {
        let old = Values { count: 40, up_to: false, max_stack: 64, scroll: 0 };
        let next = after_set(&old, Some(16));
        assert_eq!((next.count, next.max_stack), (16, 16));
        // An up-to amount is left alone.
        let up_to = Values { up_to: true, ..old };
        assert_eq!(after_set(&up_to, Some(16)).count, 40);
        assert_eq!(after_set(&old, None).max_stack, 64);
    }
}
