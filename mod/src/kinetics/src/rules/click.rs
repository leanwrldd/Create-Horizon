//! One right click that reaches a block more than once. No game types.
//!
//! On Horizon's `right_click_block` hook a click is heard once for each hand the game tries (the
//! main hand, and the off hand when the main hand's turn consumed nothing, which a cancelled
//! click doesn't). The v1 event doesn't say which hand it is for, so a block that must act once
//! per click remembers the last click it took and drops the repeat: same player, same block,
//! within a tick of it. The glue keeps the memory (per player); this is the decision.
//!
//! No Create counterpart: Create's click is consumed on the first hand.

use crate::Pos;

/// Ticks after a taken click in which the same click, heard again, is its repeat. The two calls
/// of one click arrive together; the client's own repeat while the button is held is four ticks.
pub const WINDOW: i64 = 1;

/// A click a block took: when (game time) and where.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Click {
    pub tick: i64,
    pub pos: Pos,
}

/// Whether a click at `tick` on `pos` is the rest of `last`, the click this player's last block
/// took (none if they haven't taken one). A game time that went back (another world, a reload)
/// is a new click.
pub fn is_repeat(last: Option<Click>, tick: i64, pos: Pos) -> bool {
    last.is_some_and(|last| last.pos == pos && within(last.tick, tick))
}

/// Whether anything happening at `tick` is the rest of `last`, wherever it was: the item use the
/// client sends after a click a block took (a bucket's own use) has no position.
pub fn is_repeat_anywhere(last: Option<Click>, tick: i64) -> bool {
    last.is_some_and(|last| within(last.tick, tick))
}

fn within(taken: i64, now: i64) -> bool {
    (0..=WINDOW).contains(&(now - taken))
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: Pos = [4, 70, -3];

    fn taken(tick: i64) -> Option<Click> {
        Some(Click { tick, pos: AT })
    }

    #[test]
    fn the_first_click_is_new() {
        assert!(!is_repeat(None, 100, AT));
        assert!(!is_repeat_anywhere(None, 100));
    }

    #[test]
    fn the_off_hands_call_is_a_repeat() {
        // Same tick, and the tick after: the packets of one click may straddle a tick.
        assert!(is_repeat(taken(100), 100, AT));
        assert!(is_repeat(taken(100), 101, AT));
    }

    #[test]
    fn a_later_click_is_new() {
        // The client repeats a held button every four ticks; even two ticks on is new.
        assert!(!is_repeat(taken(100), 102, AT));
        assert!(!is_repeat(taken(100), 104, AT));
    }

    #[test]
    fn another_block_is_new() {
        assert!(!is_repeat(taken(100), 100, [4, 70, -2]));
        assert!(!is_repeat(taken(100), 100, [5, 70, -3]));
        assert!(!is_repeat(taken(100), 100, [4, 71, -3]));
    }

    #[test]
    fn time_going_back_is_new() {
        assert!(!is_repeat(taken(100), 99, AT));
        assert!(!is_repeat(taken(100), 0, AT));
        assert!(!is_repeat_anywhere(taken(100), 40));
    }

    #[test]
    fn the_item_use_after_a_click_has_no_position() {
        assert!(is_repeat_anywhere(taken(100), 100));
        assert!(is_repeat_anywhere(taken(100), 101));
        assert!(!is_repeat_anywhere(taken(100), 102));
    }

    #[test]
    fn a_repeat_does_not_extend_the_window() {
        // The memory is only written for a new click: this is how the glue uses it. A third call
        // at tick 101 against a click taken at 100 is still a repeat; one at 102 is new.
        let last = taken(100);
        assert!(is_repeat(last, 101, AT));
        assert!(!is_repeat(last, 102, AT));
    }
}
