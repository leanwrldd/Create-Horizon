//! The Smart Observer's timing (`SmartObserverBlockEntity`): how long a detection keeps the
//! block lit, and what counts as seen on a belt. No game types.
//!
//! Ported from Create `content/redstone/smartObserver/SmartObserverBlockEntity` and
//! `SmartObserverBlock` (`tick`, `getSignal`), `TransportedItemStackHandlerBehaviour` and
//! `BeltInventory.applyToEachWithin`.
//!
//! Known differences from Create: none in these rules; see `machines/content_observer.rs`.

/// `DEFAULT_DELAY`: ticks a detection keeps the block lit.
pub const DEFAULT_DELAY: i32 = 6;
/// What a filter that matches the block looked at keeps it lit for (`activate(3)`).
pub const BLOCK_DELAY: i32 = 3;
/// `handleCenteredProcessingOnAllItems(.45f, ...)`: how far from the middle of a belt block an
/// item still counts as on it.
pub const CENTRE: f32 = 0.45;

/// The block's life each tick (`SmartObserverBlockEntity.tick` and the `tick` the level calls on
/// the block a tick after the countdown ends): whether it goes dark now, and the new countdown.
/// A lit block whose countdown has run out goes dark at the start of the next tick, before the
/// block entity looks again.
pub fn step(powered: bool, turn_off: i32) -> (bool, i32) {
    let dark = powered && turn_off <= 0;
    (dark, (turn_off - 1).max(0))
}

/// `activate(ticks)`: the countdown after a detection, and whether the block lights up now (it
/// does only when it was dark; a lit block just has its countdown reset).
pub fn activate(powered: bool, ticks: i32) -> (i32, bool) {
    (ticks, !powered)
}

/// `BeltInventory.applyToEachWithin(index + .5, .45)`: whether an item at belt position `pos`
/// is on the block at `index`.
pub fn on_block(index: i32, pos: f32) -> bool {
    (index as f32 + 0.5 - pos).abs() < CENTRE
}

/// `DepotBehaviour.applyToAllItems`: a held item is looked at only once it has slid in towards the
/// middle (`.5 - beltPosition > .45` skips it).
pub fn depot_in_reach(pos: f32) -> bool {
    0.5 - pos <= CENTRE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_detection_lights_the_block_for_seven_ticks_and_a_sustained_one_keeps_it() {
        // Detected on tick 0.
        let (mut turn_off, lit) = activate(false, DEFAULT_DELAY);
        assert!(lit);
        let mut powered = true;
        let mut lit_ticks = 1;
        for _ in 0..20 {
            let (dark, next) = step(powered, turn_off);
            if dark {
                powered = false;
                break;
            }
            turn_off = next;
            lit_ticks += 1;
        }
        // Lit for the tick it was found and the six after it.
        assert_eq!(lit_ticks, 7);
        assert!(!powered);

        // Detected again every other tick, it never goes dark.
        let (mut turn_off, _) = activate(false, DEFAULT_DELAY);
        let powered = true;
        for tick in 0..40 {
            let (dark, next) = step(powered, turn_off);
            assert!(!dark);
            turn_off = next;
            if tick % 2 == 0 {
                turn_off = activate(powered, DEFAULT_DELAY).0;
            }
        }
    }

    #[test]
    fn a_lit_block_found_again_is_not_lit_again() {
        assert_eq!(activate(true, DEFAULT_DELAY), (DEFAULT_DELAY, false));
        assert_eq!(activate(false, BLOCK_DELAY), (BLOCK_DELAY, true));
    }

    #[test]
    fn a_dark_block_has_nothing_to_turn_off() {
        assert_eq!(step(false, 0), (false, 0));
        assert_eq!(step(true, 1), (false, 0));
        assert_eq!(step(true, 0), (true, 0));
    }

    #[test]
    fn items_count_when_within_just_under_half_a_block_of_the_middle() {
        assert!(on_block(2, 2.5));
        assert!(on_block(2, 2.94));
        assert!(!on_block(2, 2.96));
        assert!(!on_block(2, 1.9));
        assert!(depot_in_reach(0.5));
        assert!(depot_in_reach(0.06));
        assert!(!depot_in_reach(0.04));
    }
}
