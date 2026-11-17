//! The Spout's timing: what it tells a belt about an item under it, when it fills, and how the
//! nozzle moves. No game types.
//!
//! Ported from Create `content/fluids/spout/SpoutBlockEntity` (`FILLING_TIME`, `onItemReceived`,
//! `whenItemHeld`, `tick`, `SmartFluidTankBehaviour.single(this, 1000)`) and `SpoutRenderer` (the
//! nozzle's stroke and the stream's width).
//!
//! Known differences from Create:
//! - Filling happens in the Spout's own tick, at the tick `processingTicks` reaches 5, where
//!   Create does it inside `whenItemHeld` when the belt asks at that tick. The held item, the
//!   times and what is taken are the same; [`Spout::held`] answers only Hold, Pass or Start.
//! - The blocks a Spout fills (`BlockSpoutingBehaviour`) are `spout_blocks`; a belt or a depot's
//!   items are this module's. Create has no behaviour for a basin, a bee nest or a campfire.

use crate::belt::Processing;

/// `SpoutBlockEntity.FILLING_TIME`.
pub const FILLING_TIME: i32 = 20;
/// `SmartFluidTankBehaviour.single(this, 1000)`: the Spout's tank, in mB.
pub const CAPACITY: i32 = 1000;
/// The tick (counting down) at which the item is filled: `processingTicks == 5`.
pub const FILL_AT: i32 = 5;

/// What the item the Spout looks at is to it: `FillingBySpout.canItemBeFilled` and
/// `getRequiredAmountForItem` against what the tank holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Item {
    pub can_be_filled: bool,
    pub tank_empty: bool,
    /// What filling it with the tank's fluid takes (`None`: Create's -1, it can't be filled with it).
    pub required: Option<i32>,
    /// What the tank holds.
    pub amount: i32,
}

/// `onItemReceived`: what the Spout says of an item that reached the middle of the block under it.
pub fn received(item: &Item) -> Processing {
    if !item.can_be_filled {
        return Processing::Pass;
    }
    if item.tank_empty {
        return Processing::Hold;
    }
    if item.required.is_none() {
        return Processing::Pass;
    }
    Processing::Hold
}

/// What `whenItemHeld` does for the held item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Held {
    /// Let it go on.
    Pass,
    /// Keep it where it is.
    Hold,
    /// Keep it and start the 20 ticks of filling (`processingTicks = FILLING_TIME`).
    Start,
}

/// `whenItemHeld` up to and including the start of a fill; the fill itself is at [`FILL_AT`]
/// ([`Spout::tick`]).
pub fn held(processing_ticks: i32, item: &Item) -> Held {
    if processing_ticks != -1 {
        return Held::Hold;
    }
    if !item.can_be_filled {
        return Held::Pass;
    }
    if item.tank_empty {
        return Held::Hold;
    }
    match item.required {
        None => Held::Pass,
        Some(required) if required > item.amount => Held::Hold,
        Some(_) => Held::Start,
    }
}

/// `processingTicks` of one Spout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spout {
    pub processing_ticks: i32,
}

impl Default for Spout {
    fn default() -> Self {
        Spout { processing_ticks: -1 }
    }
}

impl Spout {
    /// `SpoutBlockEntity.tick`: counts down while filling. True at the tick the item is filled.
    pub fn tick(&mut self) -> bool {
        if self.processing_ticks >= 0 {
            self.processing_ticks -= 1;
            return self.processing_ticks == FILL_AT;
        }
        false
    }

    /// The belt's `whenItemHeld` for the held item.
    pub fn held(&mut self, item: &Item) -> Held {
        let held = held(self.processing_ticks, item);
        if held == Held::Start {
            self.processing_ticks = FILLING_TIME;
        }
        held
    }
}

/// `SpoutRenderer`: `(radius, squeeze)` of the stream and the nozzle at `processing_ticks`,
/// `partial` ticks into a tick. The stream's box, a line down from the nozzle, is `inflate`d by
/// `radius / 32` (`AABB` sorts its corners, so a negative radius is a stream `|radius| / 16`
/// wide: thickest in the middle of the stroke); the nozzle's bits are moved `-3 * squeeze / 32`
/// each, one after the other.
pub fn stroke(processing_ticks: i32, partial: f32) -> (f32, f32) {
    let pt = processing_ticks as f32 - partial;
    let progress = (1.0 - (pt - 5.0) / 10.0).clamp(0.0, 1.0);
    let mut radius = 0.0;
    if processing_ticks != -1 {
        radius = (2.0 * progress - 1.0).powi(2) - 1.0;
    }
    let mut squeeze = radius;
    if pt < 0.0 {
        squeeze = 0.0;
    } else if pt < 2.0 {
        // `Mth.lerp(pt / 2, 0, -1)`.
        squeeze = -pt / 2.0;
    } else if pt < 10.0 {
        squeeze = -1.0;
    }
    (radius, squeeze)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(can: bool, empty: bool, required: Option<i32>, amount: i32) -> Item {
        Item { can_be_filled: can, tank_empty: empty, required, amount }
    }

    #[test]
    fn an_item_that_cannot_be_filled_passes_and_one_that_can_waits_for_fluid() {
        assert_eq!(received(&item(false, false, Some(250), 1000)), Processing::Pass);
        assert_eq!(received(&item(true, true, None, 0)), Processing::Hold);
        assert_eq!(received(&item(true, false, None, 500)), Processing::Pass);
        assert_eq!(received(&item(true, false, Some(1000), 500)), Processing::Hold);
    }

    #[test]
    fn filling_starts_when_the_tank_has_enough_and_fills_five_ticks_before_the_end() {
        let mut spout = Spout::default();
        let bottle = item(true, false, Some(250), 250);
        assert_eq!(spout.held(&bottle), Held::Start);
        assert_eq!(spout.processing_ticks, FILLING_TIME);
        // Holds while counting.
        assert_eq!(spout.held(&bottle), Held::Hold);
        let mut filled_at = None;
        for tick in 1..=25 {
            if spout.tick() {
                filled_at = Some(tick);
            }
        }
        // 20 -> 5 is fifteen ticks; it keeps counting to -1 and then rests.
        assert_eq!(filled_at, Some(15));
        assert_eq!(spout.processing_ticks, -1);
    }

    #[test]
    fn a_tank_with_too_little_holds_the_item_without_starting() {
        let mut spout = Spout::default();
        assert_eq!(spout.held(&item(true, false, Some(1000), 250)), Held::Hold);
        assert_eq!(spout.processing_ticks, -1);
        assert_eq!(spout.held(&item(true, true, None, 0)), Held::Hold);
        assert_eq!(spout.held(&item(false, false, None, 250)), Held::Pass);
        assert_eq!(spout.held(&item(true, false, None, 250)), Held::Pass);
    }

    #[test]
    fn the_stream_swells_mid_stroke_and_the_nozzle_squeezes() {
        // At rest nothing is drawn and the nozzle is out.
        assert_eq!(stroke(-1, 0.0), (0.0, 0.0));
        assert_eq!(stroke(20, 0.0).0, 0.0);
        assert_eq!(stroke(10, 0.0).0, -1.0);
        assert_eq!(stroke(5, 0.0).0, 0.0);
        // The nozzle is fully squeezed between ticks 10 and 2, and eases out after.
        assert_eq!(stroke(5, 0.0).1, -1.0);
        assert_eq!(stroke(1, 0.0).1, -0.5);
        assert_eq!(stroke(0, 1.0).1, 0.0);
    }
}
