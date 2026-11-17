//! The Item Drain's timing: how the held item slides in, when it empties, and what happens at the
//! end. No game types.
//!
//! Ported from Create `content/fluids/drain/ItemDrainBlockEntity` (`FILLING_TIME`, `tick`,
//! `continueProcessing`, `tryInsertingFromSide`, `internalTank` of 1500), `ItemDrainItemHandler`
//! and `ItemDrainRenderer` (the stream's width).

/// `ItemDrainBlockEntity.FILLING_TIME`.
pub const FILLING_TIME: i32 = 20;
/// `SmartFluidTankBehaviour.single(this, 1500)`.
pub const CAPACITY: i32 = 1500;
/// `itemMovementPerTick`.
pub const MOVEMENT: f32 = 1.0 / 8.0;

/// `tryInsertingFromSide` / `ItemDrainItemHandler.insertItem`: of a stack of `count` offered, how
/// many go in and how many come back. An item that can be emptied goes in one at a time; any other
/// stack goes in whole (the handler returns `limitCountToMaxStackSize`, which the glue applies).
pub fn accept(count: u32, can_be_emptied: bool) -> (u32, u32) {
    if count > 1 && can_be_emptied { (1, count - 1) } else { (count, 0) }
}

/// Which way an item that a hopper, a chute or a dropper puts in by `face` (the drain's side that touches the giver)
/// moves, if the drain takes it: `ItemDrainBlockEntity.registerCapabilities` gives the item handler to horizontal
/// sides only, and `ItemDrainItemHandler.insertItem` sets the item moving `side.getOpposite()`.
///
/// Create gives none for the top or the bottom, so a hopper or chute straight above never fed a drain there. Here
/// the top takes it too (moving north, as an item that falls straight onto the drain does,
/// `ItemDrainBlock.updateEntityAfterFallOn`), which is the natural place for a chute to end: a known difference.
pub fn handler_motion(face: Option<crate::Dir>) -> Option<crate::Dir> {
    match face? {
        crate::Dir::Down => None,
        crate::Dir::Up => Some(crate::Dir::North),
        side => Some(side.opposite()),
    }
}

/// `DirectBeltInputBehaviour.tryExportingToBeltFunnel`: whether the funnel above the drain takes the item that has
/// reached the far end. It must be a belt funnel in its pulling shape that faces `side` (the way the item came from).
/// `None` when it doesn't apply and the item goes on as usual.
pub fn belt_funnel_applies(is_belt_funnel: bool, pulling: bool, funnel_facing: crate::Dir, side: crate::Dir) -> bool {
    is_belt_funnel && pulling && funnel_facing == side
}

/// What the drain does with the funnel's answer (`tick`): `rest` is what the funnel gave back of a stack of `held`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Export {
    /// All of it went in.
    Gone,
    /// Some went in; this many are left.
    Left(u32),
    /// Nothing went and the funnel is there: the item waits (and is not thrown out or passed on).
    Wait,
}

pub fn export_result(held: u32, rest: u32) -> Export {
    if rest != held {
        if rest == 0 { Export::Gone } else { Export::Left(rest) }
    } else {
        Export::Wait
    }
}

/// What a tick of a drain holding an item does while it is not being emptied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Slide {
    /// Moved along: nothing else yet.
    Moving { pos: f32 },
    /// Reached the middle with an item that can be emptied: processing starts (20 ticks).
    Start,
    /// Reached the far end (`beltPosition > 1`, now 1): the item tries to leave.
    AtEnd,
}

/// The part of `tick` after the processing branch: `pos` is the item's `beltPosition` before the step.
pub fn slide(pos: f32, can_be_emptied_at_middle: bool) -> Slide {
    let next = pos + MOVEMENT;
    if next > 1.0 {
        return Slide::AtEnd;
    }
    if pos < 0.5 && next >= 0.5 && can_be_emptied_at_middle {
        return Slide::Start;
    }
    Slide::Moving { pos: next }
}

/// What `continueProcessing` does at `processing_ticks` (after the tick counted down).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Processing {
    /// The item can't be emptied any more: processing stops (`processingTicks = 0`).
    Stop,
    /// Nothing yet.
    Wait,
    /// The tank has no room: back to the start of the wait (`processingTicks = FILLING_TIME`).
    Restart,
    /// Empty the item now (`processingTicks == 5`).
    Empty,
}

/// `continueProcessing`: `can_be_emptied` for the held item, `fits` whether the fluid it gives
/// fits the tank (`fill(SIMULATE) == amount`).
pub fn continue_processing(processing_ticks: i32, can_be_emptied: bool, fits: bool) -> Processing {
    if processing_ticks < 5 {
        return Processing::Wait;
    }
    if !can_be_emptied {
        return Processing::Stop;
    }
    if processing_ticks > 5 {
        return if fits { Processing::Wait } else { Processing::Restart };
    }
    Processing::Empty
}

/// `ItemDrainRenderer.renderFluid`: the `radius` the stream above the held item is `inflate`d by
/// (`radius / 32` each way; a negative one is a stream `|radius| / 16` wide, as `AABB` sorts its
/// corners), for `processingTicks` and `partial` ticks into a tick. `None` when nothing is
/// processing (`processingTicks == -1`).
pub fn stream(processing_ticks: i32, partial: f32) -> Option<f32> {
    if processing_ticks == -1 {
        return None;
    }
    let pt = processing_ticks as f32 - partial;
    let progress = (1.0 - (pt - 5.0) / 10.0).clamp(0.0, 1.0);
    Some((2.0 * progress - 1.0).powi(2) - 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Dir;

    #[test]
    fn a_giver_beside_the_drain_sets_the_item_moving_across_it() {
        assert_eq!(handler_motion(Some(Dir::West)), Some(Dir::East));
        assert_eq!(handler_motion(Some(Dir::North)), Some(Dir::South));
        // The top takes it too (an extension), moving north; the bottom never.
        assert_eq!(handler_motion(Some(Dir::Up)), Some(Dir::North));
        assert_eq!(handler_motion(Some(Dir::Down)), None);
        assert_eq!(handler_motion(None), None);
    }

    #[test]
    fn a_pulling_belt_funnel_facing_the_arrival_side_takes_the_output() {
        assert!(belt_funnel_applies(true, true, Dir::West, Dir::West));
        assert!(!belt_funnel_applies(true, false, Dir::West, Dir::West), "an extended or retracted funnel pulls nothing");
        assert!(!belt_funnel_applies(true, true, Dir::East, Dir::West));
        assert!(!belt_funnel_applies(false, true, Dir::West, Dir::West), "a plain funnel isn't a belt funnel");
    }

    #[test]
    fn the_funnels_answer_decides_what_the_drain_does() {
        assert_eq!(export_result(1, 0), Export::Gone);
        assert_eq!(export_result(8, 3), Export::Left(3));
        assert_eq!(export_result(8, 8), Export::Wait);
    }

    #[test]
    fn an_emptiable_stack_goes_in_one_at_a_time() {
        assert_eq!(accept(16, true), (1, 15));
        assert_eq!(accept(1, true), (1, 0));
        assert_eq!(accept(16, false), (16, 0));
    }

    #[test]
    fn an_item_slides_in_eighths_and_stops_at_the_middle_to_be_emptied() {
        let mut pos = 0.0;
        let mut steps = 0;
        loop {
            match slide(pos, true) {
                Slide::Moving { pos: next } => pos = next,
                Slide::Start => break,
                Slide::AtEnd => panic!("it should have stopped"),
            }
            steps += 1;
        }
        // 0, .125, .25, .375 move; the step from .375 to .5 starts the wait.
        assert_eq!(steps, 3);
    }

    #[test]
    fn an_item_that_cannot_be_emptied_slides_through_to_the_end() {
        let mut pos = 0.0;
        let mut steps = 0;
        while let Slide::Moving { pos: next } = slide(pos, false) {
            pos = next;
            steps += 1;
            assert!(steps < 20);
        }
        // From 0 in eighths, 1.0 is reached after 8 steps and the ninth goes past it.
        assert_eq!(steps, 8);
        assert_eq!(slide(pos, false), Slide::AtEnd);
    }

    #[test]
    fn emptying_happens_at_five_and_waits_for_room_before_that() {
        assert_eq!(continue_processing(19, true, true), Processing::Wait);
        assert_eq!(continue_processing(19, true, false), Processing::Restart);
        assert_eq!(continue_processing(5, true, false), Processing::Empty);
        assert_eq!(continue_processing(5, false, true), Processing::Stop);
        assert_eq!(continue_processing(4, false, false), Processing::Wait);
        assert_eq!(continue_processing(0, true, true), Processing::Wait);
    }

    #[test]
    fn the_stream_swells_mid_wait() {
        assert_eq!(stream(-1, 0.0), None);
        assert_eq!(stream(20, 0.0), Some(0.0));
        assert_eq!(stream(10, 0.0), Some(-1.0));
        assert_eq!(stream(5, 0.0), Some(0.0));
    }
}
