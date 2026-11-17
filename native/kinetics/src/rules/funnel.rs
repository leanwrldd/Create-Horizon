//! The Andesite and Brass Funnel's rules: what a funnel is doing, how a funnel on a belt changes
//! shape, when an item on a belt reaches a funnel, and what a funnel takes or puts out. Ported
//! from Create's `FunnelBlock`, `BeltFunnelBlock`, `FunnelBlockEntity`,
//! `BeltFunnelInteractionHandler` and `FlapStuffs` (MIT, the Creators of Create), checked
//! against `mc1.21.1/dev` at 4148fd8.
//!
//! No game types: the glue (`native/api/src/machines/funnel.rs`) owns the world and the stacks.
//! The filter and the extraction amounts are shared with the chutes (`rules::chute`).

use super::chute::{Amount, Mode};
use crate::{Axis, Dir};

/// `CLogistics.defaultExtractionTimer`: ticks a funnel waits between transfers.
pub const EXTRACTION_TIMER: i32 = 8;
/// `FunnelBlockEntity.createChasingFlap`: how fast the flap settles (`chase(0, .05f, EXP)`).
pub const FLAP_CHASE: f32 = 0.05;
/// `FlapStuffs.FLAP_COUNT`.
pub const FLAP_COUNT: usize = 4;
/// `FlapStuffs.X_OFFSET`, in blocks.
pub const FLAP_X_OFFSET: f32 = 0.075 / 16.0;
/// `FlapStuffs.SEGMENT_STEP`, in blocks.
pub const FLAP_SEGMENT_STEP: f32 = -3.05 / 16.0;
/// `FlapStuffs.FUNNEL_PIVOT`, `voxelSpace(0, 10, 9.5)`, in blocks.
pub const FLAP_PIVOT: [f32; 3] = [0.0, 10.0 / 16.0, 9.5 / 16.0];
/// Ticks between a funnel's looks at the item entities in its block (`entityInside` is told of
/// them as they move).
pub const COLLECT_SCAN: i64 = 2;

/// `BeltFunnelBlock.Shape`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Retracted,
    Extended,
    Pushing,
    Pulling,
}

impl Shape {
    pub const fn name(self) -> &'static str {
        match self {
            Shape::Retracted => "retracted",
            Shape::Extended => "extended",
            Shape::Pushing => "pushing",
            Shape::Pulling => "pulling",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [Shape::Retracted, Shape::Extended, Shape::Pushing, Shape::Pulling]
            .into_iter()
            .find(|s| s.name() == name)
    }
}

/// `FunnelBlockEntity.Mode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    Invalid,
    Paused,
    Collect,
    PushingToBelt,
    TakingFromBelt,
    Extract,
}

/// What a funnel's state and the belt under it say (`FunnelBlockEntity.determineCurrentMode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A funnel on a wall, floor or ceiling (`FunnelBlock`): `extracting` is its state.
    Plain { extracting: bool },
    /// A funnel on a belt or a depot (`BeltFunnelBlock`). `belt_moves_towards_facing` is, for a
    /// funnel on a belt block, whether the belt's top moves the way the funnel faces, and none
    /// where the block below is no belt.
    Belt { shape: Shape, belt_moves_towards_facing: Option<bool> },
}

pub fn determine_activity(kind: Kind, powered: bool) -> Activity {
    if powered {
        return Activity::Paused;
    }
    match kind {
        Kind::Belt { shape: Shape::Pulling, .. } => Activity::TakingFromBelt,
        Kind::Belt { shape: Shape::Pushing, .. } => Activity::PushingToBelt,
        Kind::Belt { belt_moves_towards_facing, .. } => match belt_moves_towards_facing {
            Some(true) => Activity::PushingToBelt,
            Some(false) => Activity::TakingFromBelt,
            None => Activity::Invalid,
        },
        Kind::Plain { extracting: true } => Activity::Extract,
        Kind::Plain { extracting: false } => Activity::Collect,
    }
}

/// `BeltFunnelBlock.getShapeForPosition`: what the block below makes of a horizontal funnel.
/// `belt_facing` is the facing of the belt block below, none where the block below is no belt.
pub fn shape_for_position(belt_facing: Option<Dir>, facing: Dir, extracting: bool) -> Shape {
    let perpendicular = if extracting { Shape::Pushing } else { Shape::Pulling };
    match belt_facing {
        None => perpendicular,
        Some(belt) if belt.axis() != facing.axis() => perpendicular,
        Some(_) => Shape::Retracted,
    }
}

/// `BeltFunnelBlock.updateShape` with a valid belt below: the shape the block takes, a wrenched
/// shape never reverting.
pub fn updated_shape(current: Shape, wanted: Shape) -> Shape {
    if wanted == current
        || (wanted == Shape::Pushing && current == Shape::Pulling)
        || (wanted == Shape::Retracted && current == Shape::Extended)
    {
        return current;
    }
    wanted
}

/// `BeltFunnelBlock.onWrenched`: the next shape. `sloped_belt` is whether the belt below isn't
/// horizontal (a retracted funnel then stays retracted).
pub fn wrenched(shape: Shape, sloped_belt: bool) -> Shape {
    match shape {
        Shape::Pulling => Shape::Pushing,
        Shape::Pushing => Shape::Pulling,
        Shape::Extended => Shape::Retracted,
        Shape::Retracted => {
            if sloped_belt {
                Shape::Retracted
            } else {
                Shape::Extended
            }
        }
    }
}

/// `BeltFunnelBlock.updateShape` without a belt: the funnel it falls back to is extracting when
/// it was pushing.
pub fn falls_back_extracting(shape: Shape) -> bool {
    shape == Shape::Pushing
}

/// `FunnelBlockEntity.supportsAmountOnFilter`: an extracting funnel, or a belt funnel that
/// pushes (or sits on a belt block).
pub fn supports_amount(kind: Kind, belt_below: bool) -> bool {
    match kind {
        Kind::Plain { extracting } => extracting,
        Kind::Belt { shape, .. } => shape == Shape::Pushing || belt_below,
    }
}

/// `FunnelBlockEntity.getAmountToExtract`: `-1` is the unlimited the filter's "any" would give.
pub fn amount_to_extract(supports_amount: bool, filter_active: bool, filter: Amount) -> u32 {
    if !supports_amount {
        return 64;
    }
    if !filter_active {
        return 1;
    }
    filter.count.max(1) as u32
}

/// `FunnelBlockEntity.getModeToExtract`.
pub fn mode_to_extract(supports_amount: bool, filter_active: bool, filter: Amount) -> Mode {
    if !supports_amount || !filter_active || filter.up_to {
        Mode::UpTo
    } else {
        Mode::Exactly
    }
}

/// `FunnelBlockEntity.supportsDirectBeltInput`: a belt can end at a funnel that isn't a belt
/// funnel, isn't extracting and opens upwards.
pub fn supports_direct_belt_input(plain: bool, extracting: bool, facing: Dir) -> bool {
    plain && !extracting && facing == Dir::Up
}

/// `FunnelBlock.canInsertIntoFunnel`: hands and falling items can put things in.
pub fn can_insert_into(powered: bool, extracting: bool) -> bool {
    !powered && !extracting
}

/// `AbstractFunnelBlock.canSurvive`: not against another funnel.
pub fn can_survive(block_behind_is_funnel: bool) -> bool {
    !block_behind_is_funnel
}

/// `Direction.orderedByNearest` of a look: the six directions, the one most like the look
/// first (`getNearestLookingDirections`).
pub fn nearest_looking(yaw: f32, pitch: f32) -> [Dir; 6] {
    let (yaw, pitch) = (yaw.to_radians(), pitch.to_radians());
    let look = [
        -yaw.sin() * pitch.cos(),
        -pitch.sin(),
        yaw.cos() * pitch.cos(),
    ];
    let mut order = Dir::ALL;
    let dot = |d: Dir| {
        let n = d.normal();
        look[0] * n[0] as f32 + look[1] * n[1] as f32 + look[2] * n[2] as f32
    };
    order.sort_by(|a, b| dot(*b).total_cmp(&dot(*a)));
    order
}

/// `FunnelBlock.getStateForPlacement`: the first direction in the order the player looks whose
/// opposite is a facing the funnel can survive at, `survives(facing)` asked of each.
pub fn placement_facing(order: [Dir; 6], mut survives: impl FnMut(Dir) -> bool) -> Option<Dir> {
    order.into_iter().map(Dir::opposite).find(|facing| survives(*facing))
}

/// `FunnelBlockEntity.activateExtractor`: where the extracted stack appears, relative to the
/// funnel's centre, and how it moves.
pub fn output_of(facing: Dir) -> ([f64; 3], [f64; 3]) {
    let vertical = facing.axis() == Axis::Y;
    let up = facing == Dir::Up;
    let reach = if vertical {
        if up { 0.15 } else { 0.5 }
    } else {
        0.25
    };
    let n = facing.normal();
    let mut at = [f64::from(n[0]) * reach, f64::from(n[1]) * reach, f64::from(n[2]) * reach];
    if !vertical {
        at[1] -= 0.45;
    }
    let motion = if up { [0.0, 4.0 / 16.0, 0.0] } else { [0.0; 3] };
    (at, motion)
}

/// `FunnelBlockEntity.getEntityOverflowScanningArea`: the box that must be free of items before
/// the funnel puts another out, relative to the funnel's block (`min`, `max`).
pub fn overflow_area(facing: Dir) -> ([f64; 3], [f64; 3]) {
    let min = [0.0, if facing == Dir::Up { 0.0 } else { -1.0 }, 0.0];
    (min, [1.0, 1.0, 1.0])
}

/// `FunnelBlock.entityInside`: an item entity at `offset` from the funnel's centre is on the
/// open side of the funnel (its opening is `0.25` back from the face for items, `0.125` for
/// anything else).
pub fn on_open_side(facing: Dir, offset: [f64; 3], item: bool) -> bool {
    let n = facing.normal();
    let back = if item { -0.25 } else { -0.125 };
    let axis = facing.axis().index();
    let projected = offset[axis] - f64::from(n[axis]) * back;
    // `projectedDiff < 0 == (axisDirection == POSITIVE)` returns.
    !((projected < 0.0) == (facing.sign() > 0))
}

// --- belt funnels and the items on the belt (`BeltFunnelInteractionHandler`) -----------------

/// What a belt funnel does to an item the belt brings (`checkForFunnels`'s first tests).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reaction {
    /// The funnel faces the way the belt moves, or pushes: the item isn't its business.
    Ignore,
    /// The item passes if the funnel doesn't take it.
    Passing,
    /// The funnel faces the oncoming items (`funnelFacing == movementFacing.getOpposite()`): an
    /// item it doesn't take waits at its entry.
    Blocking,
}

pub fn reaction(funnel_facing: Dir, movement: Dir, shape: Shape) -> Reaction {
    if funnel_facing == movement || shape == Shape::Pushing {
        Reaction::Ignore
    } else if funnel_facing == movement.opposite() {
        Reaction::Blocking
    } else {
        Reaction::Passing
    }
}

/// `checkForFunnels`: whether an item at `pos` that will be at `next` has crossed the entry of
/// the funnel over belt block `segment` (of a belt `length` long), and where the entry is.
pub fn belt_entry(
    pos: f32,
    next: f32,
    segment: i32,
    length: i32,
    positive: bool,
    extended: bool,
) -> Option<f32> {
    let first = (pos.floor() as i32).clamp(0, (length - 1).max(0));
    let in_loop = if positive {
        segment >= first && segment as f32 <= next
    } else {
        segment <= first && (segment + 1) as f32 >= next
    };
    if !in_loop {
        return None;
    }
    let mut entry = segment as f32 + 0.5;
    if extended {
        entry += 0.499 * if positive { -1.0 } else { 1.0 };
    }
    let crossed = if positive { next > entry } else { next < entry };
    crossed.then_some(entry)
}

/// What the funnel does with an item stack that reached it: how many of `count` to offer, and
/// whether the offer must be checked first (`simulate`), or none at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offer {
    /// `amountToExtract > count` with a mode that needs the whole amount.
    Refuse,
    /// Offer `count` items (`simulate`: they must all fit or the item is not taken).
    Take { count: u32, simulate: bool },
}

pub fn belt_offer(amount: u32, mode: Mode, count: u32) -> Offer {
    if amount > count && mode != Mode::UpTo {
        return Offer::Refuse;
    }
    if mode != Mode::UpTo {
        Offer::Take { count: amount.min(count), simulate: true }
    } else {
        Offer::Take { count, simulate: false }
    }
}

// --- the flaps (`FlapStuffs`) -----------------------------------------------------------------

/// `FunnelBlockEntity.getFlapOffset`, in blocks.
pub fn flap_offset(kind: Kind) -> f32 {
    match kind {
        Kind::Plain { .. } => -1.0 / 16.0,
        Kind::Belt { shape: Shape::Extended, .. } => 8.0 / 16.0,
        Kind::Belt { shape: Shape::Pulling | Shape::Pushing, .. } => -2.0 / 16.0,
        Kind::Belt { .. } => 0.0,
    }
}

/// `FlapStuffs.flapAngle`: degrees one of the four flaps is turned at `flapness` (-1 to 1).
pub fn flap_angle(flapness: f32, segment: usize) -> f32 {
    let intensity = if segment == 3 { 1.5 } else { (segment + 1) as f32 };
    let abs = flapness.abs();
    let mut angle = ((1.0 - abs) * std::f32::consts::PI * intensity).sin() * 30.0 * flapness;
    if flapness < 0.0 {
        angle *= 0.5;
    }
    angle
}

/// `FunnelBlockEntity.flap`: `flap.setValue(inward ? -1 : 1)`, then it chases 0 by
/// [`FLAP_CHASE`] of the way each tick. The value `ticks` ticks (and a partial one) later.
pub fn flap_value(inward: bool, ticks: f32) -> f32 {
    let start = if inward { -1.0 } else { 1.0 };
    start * (1.0 - FLAP_CHASE).powf(ticks.max(0.0))
}

/// Whether a click clears a brass funnel's filter (the stand-in for taking the item out of Create's filter slot):
/// sneaking with nothing in either hand. Any item is another handler's (setting the filter, the Wrench, placing).
pub fn clears_filter(brass: bool, sneaking: bool, main_hand_empty: bool, off_hand_empty: bool) -> bool {
    brass && sneaking && main_hand_empty && off_hand_empty
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_sneaking_empty_handed_click_on_a_brass_funnel_clears_its_filter() {
        assert!(clears_filter(true, true, true, true));
        assert!(!clears_filter(false, true, true, true), "an Andesite Funnel has no filter");
        assert!(!clears_filter(true, false, true, true));
        assert!(!clears_filter(true, true, false, true));
        assert!(!clears_filter(true, true, true, false));
    }

    #[test]
    fn a_funnels_activity_follows_its_state_and_redstone() {
        let plain = |extracting| Kind::Plain { extracting };
        assert_eq!(determine_activity(plain(true), false), Activity::Extract);
        assert_eq!(determine_activity(plain(false), false), Activity::Collect);
        assert_eq!(determine_activity(plain(true), true), Activity::Paused);
        let belt = |shape, moves| Kind::Belt { shape, belt_moves_towards_facing: moves };
        assert_eq!(determine_activity(belt(Shape::Pulling, None), false), Activity::TakingFromBelt);
        assert_eq!(determine_activity(belt(Shape::Pushing, None), false), Activity::PushingToBelt);
        assert_eq!(determine_activity(belt(Shape::Pushing, None), true), Activity::Paused);
        // Retracted and extended read the belt: it moving the way the funnel faces pushes onto it.
        assert_eq!(determine_activity(belt(Shape::Retracted, Some(true)), false), Activity::PushingToBelt);
        assert_eq!(determine_activity(belt(Shape::Extended, Some(false)), false), Activity::TakingFromBelt);
        assert_eq!(determine_activity(belt(Shape::Retracted, None), false), Activity::Invalid);
    }

    #[test]
    fn a_funnel_beside_a_belt_lies_along_it_and_across_it_hangs_over() {
        // A belt block facing north runs along z; a funnel facing north or south is along it.
        assert_eq!(shape_for_position(Some(Dir::North), Dir::South, true), Shape::Retracted);
        assert_eq!(shape_for_position(Some(Dir::North), Dir::East, true), Shape::Pushing);
        assert_eq!(shape_for_position(Some(Dir::North), Dir::East, false), Shape::Pulling);
        // No belt (a depot): always across.
        assert_eq!(shape_for_position(None, Dir::East, true), Shape::Pushing);
        assert_eq!(shape_for_position(None, Dir::East, false), Shape::Pulling);
    }

    #[test]
    fn a_wrenched_shape_is_not_undone_by_the_next_update() {
        assert_eq!(updated_shape(Shape::Pulling, Shape::Pushing), Shape::Pulling);
        assert_eq!(updated_shape(Shape::Extended, Shape::Retracted), Shape::Extended);
        assert_eq!(updated_shape(Shape::Pushing, Shape::Retracted), Shape::Retracted);
        assert_eq!(updated_shape(Shape::Retracted, Shape::Pulling), Shape::Pulling);
        assert_eq!(updated_shape(Shape::Pulling, Shape::Pulling), Shape::Pulling);
    }

    #[test]
    fn the_wrench_flips_pull_and_push_and_extends_a_funnel_on_a_flat_belt() {
        assert_eq!(wrenched(Shape::Pulling, false), Shape::Pushing);
        assert_eq!(wrenched(Shape::Pushing, false), Shape::Pulling);
        assert_eq!(wrenched(Shape::Retracted, false), Shape::Extended);
        assert_eq!(wrenched(Shape::Retracted, true), Shape::Retracted);
        assert_eq!(wrenched(Shape::Extended, false), Shape::Retracted);
        assert!(falls_back_extracting(Shape::Pushing));
        assert!(!falls_back_extracting(Shape::Pulling));
    }

    #[test]
    fn andesite_funnels_move_one_item_and_brass_ones_what_the_filter_says() {
        let any = Amount::default();
        assert_eq!(amount_to_extract(true, false, any), 1);
        assert_eq!(amount_to_extract(true, true, any), 64);
        assert_eq!(amount_to_extract(true, true, Amount { count: 8, up_to: false }), 8);
        assert_eq!(amount_to_extract(false, true, Amount { count: 8, up_to: false }), 64);
        assert_eq!(mode_to_extract(true, true, Amount { count: 8, up_to: false }), Mode::Exactly);
        assert_eq!(mode_to_extract(true, false, Amount { count: 8, up_to: false }), Mode::UpTo);
        assert_eq!(mode_to_extract(false, true, Amount { count: 8, up_to: false }), Mode::UpTo);
        assert!(supports_amount(Kind::Plain { extracting: true }, false));
        assert!(!supports_amount(Kind::Plain { extracting: false }, false));
        assert!(supports_amount(Kind::Belt { shape: Shape::Pushing, belt_moves_towards_facing: None }, false));
        assert!(supports_amount(Kind::Belt { shape: Shape::Retracted, belt_moves_towards_facing: Some(true) }, true));
        assert!(!supports_amount(Kind::Belt { shape: Shape::Pulling, belt_moves_towards_facing: None }, false));
    }

    #[test]
    fn only_an_open_upward_funnel_takes_items_from_a_belt_end() {
        assert!(supports_direct_belt_input(true, false, Dir::Up));
        assert!(!supports_direct_belt_input(true, true, Dir::Up));
        assert!(!supports_direct_belt_input(true, false, Dir::North));
        assert!(!supports_direct_belt_input(false, false, Dir::Up));
        assert!(can_insert_into(false, false));
        assert!(!can_insert_into(true, false));
        assert!(!can_insert_into(false, true));
    }

    #[test]
    fn a_funnel_is_placed_facing_the_player_unless_it_cant_survive_there() {
        // Looking north (yaw 180): nearest is north, so the funnel faces south.
        let order = nearest_looking(180.0, 0.0);
        assert_eq!(order[0], Dir::North);
        assert_eq!(order[5], Dir::South);
        assert_eq!(placement_facing(order, |_| true), Some(Dir::South));
        // Another funnel behind (north of) it: the next nearest direction is tried.
        let next = placement_facing(order, |facing| facing != Dir::South);
        assert!(next.is_some());
        assert_ne!(next, Some(Dir::South));
        assert_eq!(placement_facing(order, |_| false), None);
        // Looking straight down: the funnel sits on a floor, facing up.
        assert_eq!(placement_facing(nearest_looking(0.0, 90.0), |_| true), Some(Dir::Up));
    }

    #[test]
    fn an_extracted_stack_appears_in_front_of_the_funnel() {
        let (at, motion) = output_of(Dir::Up);
        assert_eq!(at, [0.0, 0.15, 0.0]);
        assert_eq!(motion, [0.0, 0.25, 0.0]);
        let (at, motion) = output_of(Dir::Down);
        assert_eq!(at, [0.0, -0.5, 0.0]);
        assert_eq!(motion, [0.0; 3]);
        let (at, _) = output_of(Dir::East);
        assert!((at[0] - 0.25).abs() < 1e-9 && (at[1] + 0.45).abs() < 1e-9 && at[2] == 0.0);
        assert_eq!(overflow_area(Dir::Up).0, [0.0; 3]);
        assert_eq!(overflow_area(Dir::North).0, [0.0, -1.0, 0.0]);
    }

    #[test]
    fn an_item_is_caught_on_the_open_side_of_the_funnel() {
        // A funnel facing up on the floor: an item above its opening is caught, one below it isn't.
        assert!(on_open_side(Dir::Up, [0.0, 0.0, 0.0], true));
        assert!(!on_open_side(Dir::Up, [0.0, -0.4, 0.0], true));
        // Facing down it is the other way round.
        assert!(on_open_side(Dir::Down, [0.0, 0.0, 0.0], true));
        assert!(!on_open_side(Dir::Down, [0.0, 0.4, 0.0], true));
        // A funnel facing north takes items north of the plate.
        assert!(on_open_side(Dir::North, [0.0, 0.0, -0.1], true));
        assert!(!on_open_side(Dir::North, [0.0, 0.0, 0.4], true));
    }

    #[test]
    fn belt_funnels_react_to_the_way_the_belt_moves() {
        assert_eq!(reaction(Dir::East, Dir::East, Shape::Retracted), Reaction::Ignore);
        assert_eq!(reaction(Dir::West, Dir::East, Shape::Retracted), Reaction::Blocking);
        assert_eq!(reaction(Dir::North, Dir::East, Shape::Pulling), Reaction::Passing);
        assert_eq!(reaction(Dir::West, Dir::East, Shape::Pushing), Reaction::Ignore);
    }

    #[test]
    fn an_item_reaches_a_funnel_once_it_passes_the_middle_of_its_block() {
        // Funnel over block 2 of a belt 5 long, items moving up the belt.
        assert_eq!(belt_entry(2.2, 2.4, 2, 5, true, false), None);
        assert_eq!(belt_entry(2.4, 2.6, 2, 5, true, false), Some(2.5));
        // Still on its way there, a block before.
        assert_eq!(belt_entry(1.2, 1.4, 2, 5, true, false), None);
        // Already past it.
        assert_eq!(belt_entry(3.2, 3.4, 2, 5, true, false), None);
        // An extended funnel reaches back to nearly the start of the block.
        let entry = belt_entry(2.0, 2.1, 2, 5, true, true).expect("crossed");
        assert!((entry - 2.001).abs() < 1e-4);
        // Moving down the belt (negative): crossed once below the middle.
        assert_eq!(belt_entry(2.7, 2.4, 2, 5, false, false), Some(2.5));
        assert_eq!(belt_entry(2.9, 2.6, 2, 5, false, false), None);
    }

    #[test]
    fn a_belt_funnel_offers_what_its_amount_allows() {
        assert_eq!(belt_offer(64, Mode::UpTo, 10), Offer::Take { count: 10, simulate: false });
        assert_eq!(belt_offer(8, Mode::Exactly, 10), Offer::Take { count: 8, simulate: true });
        assert_eq!(belt_offer(8, Mode::Exactly, 4), Offer::Refuse);
    }

    #[test]
    fn the_flaps_swing_out_and_settle() {
        assert_eq!(flap_angle(0.0, 0), 0.0);
        // Pushed outward (positive) is a full swing at the start; inward (negative) is half.
        assert!(flap_angle(0.5, 0) > 0.0);
        assert!(flap_angle(-0.5, 0) < 0.0);
        assert!((flap_angle(-0.5, 0) * -2.0 - flap_angle(0.5, 0)).abs() < 1e-4);
        assert_eq!(flap_value(true, 0.0), -1.0);
        assert!(flap_value(false, 40.0) < 0.15);
        assert_eq!(flap_offset(Kind::Plain { extracting: true }), -1.0 / 16.0);
        assert_eq!(flap_offset(Kind::Belt { shape: Shape::Extended, belt_moves_towards_facing: None }), 0.5);
    }
}
