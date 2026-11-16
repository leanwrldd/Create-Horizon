//! The Andesite and Brass Tunnels over a belt: which shape a tunnel takes from its neighbours,
//! which sides it has and where its flaps hang, what an item does as it crosses into one, and how
//! the flaps swing. No game types.
//!
//! Ported from Create 6.0.10 (MIT, the Creators of Create): `BeltTunnelBlock` (the shape states
//! and `getTunnelState`), `BeltTunnelBlockEntity.updateTunnelConnections`,
//! `BeltTunnelInteractionHandler` (`flapTunnelsAndCheckIfStuck`, `stuckAtTunnel`),
//! `BeltTunnelShapes` (the shapes are declared in `tools/content/tunnel.py`), `FlapStuffs` and
//! `BrassTunnelBlockEntity.canInsert` / `hasDistributionBehaviour`. The distribution of a Brass
//! Tunnel is `tunnel_brass`.

use crate::{Axis, Dir};

use super::funnel;

/// `FlapStuffs.TUNNEL_PIVOT`, `voxelSpace(0, 10, 1)`, in blocks.
pub const FLAP_PIVOT: [f32; 3] = [0.0, 10.0 / 16.0, 1.0 / 16.0];
/// `FlapStuffs.FLAP_COUNT`, `X_OFFSET` and `SEGMENT_STEP` are the funnel's.
pub const FLAP_COUNT: usize = funnel::FLAP_COUNT;
pub const FLAP_X_OFFSET: f32 = funnel::FLAP_X_OFFSET;
pub const FLAP_SEGMENT_STEP: f32 = funnel::FLAP_SEGMENT_STEP;
/// How long clients keep drawing a flap settling (`flap_value` is nearly 0 by then).
pub const FLAP_TICKS: f32 = 120.0;

/// `Iterate.horizontalDirections`, in the order Create walks them.
pub const HORIZONTALS: [Dir; 4] = [Dir::North, Dir::South, Dir::West, Dir::East];

/// `BeltTunnelBlock.Shape`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Straight,
    Window,
    Closed,
    TLeft,
    TRight,
    Cross,
}

impl Shape {
    pub const ALL: [Shape; 6] = [
        Shape::Straight,
        Shape::Window,
        Shape::Closed,
        Shape::TLeft,
        Shape::TRight,
        Shape::Cross,
    ];

    /// `getSerializedName`: the state's `shape` value.
    pub const fn name(self) -> &'static str {
        match self {
            Shape::Straight => "straight",
            Shape::Window => "window",
            Shape::Closed => "closed",
            Shape::TLeft => "t_left",
            Shape::TRight => "t_right",
            Shape::Cross => "cross",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }

    /// `BeltTunnelBlock.hasWindow`.
    pub const fn has_window(self) -> bool {
        matches!(self, Shape::Window | Shape::Closed)
    }

    /// `BeltTunnelBlock.isStraight`.
    pub const fn is_straight(self) -> bool {
        self.has_window() || matches!(self, Shape::Straight)
    }

    /// `BeltTunnelBlock.isJunction`.
    pub const fn is_junction(self) -> bool {
        matches!(self, Shape::Cross | Shape::TLeft | Shape::TRight)
    }
}

/// `Direction.get(AxisDirection.POSITIVE, axis)` for a horizontal axis.
pub const fn positive(axis: Axis) -> Dir {
    match axis {
        Axis::X => Dir::East,
        Axis::Y => Dir::Up,
        Axis::Z => Dir::South,
    }
}

/// `getTunnelState`'s `left`: the positive direction of the axis turned clockwise.
pub const fn left(axis: Axis) -> Dir {
    positive(axis).clockwise_about(Axis::Y)
}

/// What stands beside a tunnel along its axis (`canHaveWindow`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Beside {
    /// Another tunnel of either kind.
    Tunnel,
    /// A belt funnel that is `EXTENDED` and faces the tunnel.
    Funnel,
    Other,
}

/// `BeltTunnelBlock.canHaveWindow`: a tunnel with a tunnel or a funnel on each end, but not a
/// funnel on both.
pub fn can_have_window(front: Beside, back: Beside) -> bool {
    let valid = |b: Beside| b != Beside::Other;
    valid(front) && valid(back) && !(front == Beside::Funnel && back == Beside::Funnel)
}

/// `getTunnelState`: the shape from what takes items on the left and the right of the belt
/// below, and whether a window fits when it would be straight.
pub fn shape_for(on_left: bool, on_right: bool, window_fits: bool) -> Shape {
    let shape = match (on_left, on_right) {
        (true, true) => Shape::Cross,
        (true, false) => Shape::TLeft,
        (false, true) => Shape::TRight,
        (false, false) => Shape::Straight,
    };
    if shape == Shape::Straight && window_fits {
        Shape::Window
    } else {
        shape
    }
}

/// What the tunnel's state becomes when its neighbours say it should be `computed`
/// (`BeltTunnelBlock.updateTunnel` and `updateShape`): a tunnel closed with the wrench stays
/// closed while a window would still fit. Create's `updateTunnel` reopens it whenever a block
/// beside the belt changes; Horizon tells a mod no such event, so the check runs on a timer and
/// must not undo the wrench.
pub fn settle(axis: Axis, shape: Shape, computed: (Axis, Shape)) -> (Axis, Shape) {
    if shape == Shape::Closed && computed.0 == axis && computed.1 == Shape::Window {
        (axis, shape)
    } else {
        computed
    }
}

/// `BeltTunnelBlock.onWrenched`: a tunnel with a window closes it or opens it again; others
/// do nothing.
pub fn wrenched(shape: Shape) -> Option<Shape> {
    match shape {
        Shape::Window => Some(Shape::Closed),
        Shape::Closed => Some(Shape::Window),
        _ => None,
    }
}

/// `BeltTunnelBlockEntity.updateTunnelConnections`, the first half: the sides a tunnel opens on.
/// Both ends along its axis, and across it the side or sides the shape has a T or a cross for.
pub fn sides(axis: Axis, shape: Shape) -> Vec<Dir> {
    let mut out = Vec::new();
    for d in HORIZONTALS {
        if d.axis() != axis {
            // `direction.getAxisDirection() == POSITIVE ^ direction.getAxis() == Z`
            let positive = (d.sign() > 0) ^ (d.axis() == Axis::Z);
            if shape.is_straight()
                || (positive && shape == Shape::TLeft)
                || (!positive && shape == Shape::TRight)
            {
                continue;
            }
        }
        out.push(d);
    }
    out
}

/// What is on a side of a tunnel, for its flap (`updateTunnelConnections`, the second half).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Occluder {
    /// Open air, a wall, a belt: the flap hangs there.
    None,
    /// A tunnel or an extended belt funnel facing the tunnel covers the opening.
    Covered,
}

/// The sides with a flap: those of [`sides`] nothing covers.
pub fn flaps(sides: &[Dir], mut beside: impl FnMut(Dir) -> Occluder) -> Vec<Dir> {
    sides
        .iter()
        .copied()
        .filter(|&d| beside(d) == Occluder::None)
        .collect()
}

/// `BrassTunnelBlockEntity.hasDistributionBehaviour` (a brass tunnel's): it distributes when it
/// has a flap, and either a neighbour is connected or a flap is across its axis.
pub fn distributes(flaps: &[Dir], connected: bool, axis: Axis) -> bool {
    if flaps.is_empty() {
        return false;
    }
    connected || flaps.iter().any(|d| d.axis() != axis)
}

/// What a Brass Tunnel over the next segment says to an item about to cross into it
/// (`stuckAtTunnel`, `canInsert`, `canTakeItems`), from what the glue read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrassAhead {
    /// The filter on the side the item comes in by takes it (`filtering.test(side, stack)`).
    pub entry_allowed: bool,
    /// [`distributes`].
    pub distributes: bool,
    /// It holds a stack it hasn't given out yet.
    pub holding: bool,
    /// `canTakeItems`: it holds none and no synchronised output is waiting.
    pub can_take: bool,
}

/// What a tunnel over the next segment does to an item crossing into it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Goes through, flapping.
    Pass,
    /// Waits at the tunnel's door, a hair before it (`stuckAtTunnel`).
    Stuck,
    /// Waits where it is (`!canTakeItems`).
    Wait,
    /// The tunnel takes it to distribute.
    Take,
}

/// `BeltTunnelInteractionHandler.flapTunnelsAndCheckIfStuck` for a Brass Tunnel.
pub fn enter_brass(ahead: &BrassAhead) -> Verdict {
    // `stuckAtTunnel`: `!canInsert`.
    if !ahead.entry_allowed || (ahead.distributes && ahead.holding) {
        return Verdict::Stuck;
    }
    if !ahead.distributes {
        return Verdict::Pass;
    }
    if ahead.can_take {
        Verdict::Take
    } else {
        Verdict::Wait
    }
}

/// Where an item stuck at a tunnel waits (`current.beltPosition = currentSegment + (.99 | .01)`).
pub fn stuck_position(segment: i32, positive: bool) -> f32 {
    segment as f32 + if positive { 0.99 } else { 0.01 }
}

/// Whether an Andesite Tunnel over the next segment splits the stack as it enters: more than
/// one item, a junction, and the belt runs along the tunnel.
pub fn splits(count: u32, shape: Shape, belt_axis: Axis, tunnel_axis: Axis) -> bool {
    count > 1 && shape.is_junction() && belt_axis == tunnel_axis
}

/// What one side output answered to an item offered to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offer {
    /// No input there, or it won't take it from this side.
    Skip,
    /// One item went in.
    Gave,
    /// The chunk isn't loaded or the item was refused after all: the stack waits.
    Wait,
}

/// What came of splitting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Split {
    /// The sides an item was given to, in order.
    pub given: Vec<Dir>,
    /// The stack has to wait at the tunnel.
    pub wait: bool,
}

/// `BeltTunnelInteractionHandler`'s loop over the sides of an Andesite Tunnel junction: each
/// side across the belt that has a flap takes one item off the stack, until one is left.
pub fn split(
    count: u32,
    belt_axis: Axis,
    flaps: &[Dir],
    mut offer: impl FnMut(Dir) -> Offer,
) -> Split {
    let mut left = count;
    let mut out = Split { given: Vec::new(), wait: false };
    for d in HORIZONTALS {
        if d.axis() == belt_axis || !flaps.contains(&d) {
            continue;
        }
        match offer(d) {
            Offer::Skip => continue,
            Offer::Wait => {
                out.wait = true;
                return out;
            }
            Offer::Gave => {}
        }
        out.given.push(d);
        left -= 1;
        if left <= 1 {
            break;
        }
    }
    out
}

// --- the flaps (`FlapStuffs`) -------------------------------------------------------------------

/// `AngleHelper.horizontalAngle`: `toYRot` (south 0, west 90, north 180, east 270), negated for
/// an X facing, so south 0, east 90, north 180, west 270. A flap on a side is turned by the angle
/// of the side's opposite.
pub fn horizontal_angle(facing: Dir) -> f32 {
    match facing {
        Dir::East => 90.0,
        Dir::North => 180.0,
        Dir::West => 270.0,
        _ => 0.0,
    }
}

/// `FlapStuffs.flapAngle`, the funnel's own.
pub fn flap_angle(flapness: f32, segment: usize) -> f32 {
    funnel::flap_angle(flapness, segment)
}

/// `BeltTunnelBlockEntity.flap` then `createChasingFlap`: the flap is at -1 (`inward`) or 1 and
/// settles to 0. `ticks` after it moved.
pub fn flap_value(inward: bool, ticks: f32) -> f32 {
    funnel::flap_value(inward, ticks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_left_side_is_the_positive_end_turned_clockwise() {
        // Along z, positive is south and its clockwise turn is west.
        assert_eq!(left(Axis::Z), Dir::West);
        assert_eq!(left(Axis::X), Dir::South);
    }

    #[test]
    fn the_shape_follows_the_outputs_beside_the_belt() {
        assert_eq!(shape_for(true, true, true), Shape::Cross);
        assert_eq!(shape_for(true, false, true), Shape::TLeft);
        assert_eq!(shape_for(false, true, false), Shape::TRight);
        assert_eq!(shape_for(false, false, false), Shape::Straight);
        assert_eq!(shape_for(false, false, true), Shape::Window);
    }

    #[test]
    fn a_window_needs_a_neighbour_on_each_end_but_not_two_funnels() {
        assert!(can_have_window(Beside::Tunnel, Beside::Tunnel));
        assert!(can_have_window(Beside::Funnel, Beside::Tunnel));
        assert!(!can_have_window(Beside::Funnel, Beside::Funnel));
        assert!(!can_have_window(Beside::Tunnel, Beside::Other));
        assert!(!can_have_window(Beside::Other, Beside::Other));
    }

    #[test]
    fn the_wrench_toggles_a_window_only() {
        assert_eq!(wrenched(Shape::Window), Some(Shape::Closed));
        assert_eq!(wrenched(Shape::Closed), Some(Shape::Window));
        assert_eq!(wrenched(Shape::Straight), None);
        assert_eq!(wrenched(Shape::Cross), None);
    }

    #[test]
    fn a_closed_window_stays_closed_until_something_else_changes() {
        let x = Axis::X;
        assert_eq!(settle(x, Shape::Closed, (x, Shape::Window)), (x, Shape::Closed));
        assert_eq!(settle(x, Shape::Closed, (x, Shape::TLeft)), (x, Shape::TLeft));
        assert_eq!(settle(x, Shape::Closed, (Axis::Z, Shape::Window)), (Axis::Z, Shape::Window));
        assert_eq!(settle(x, Shape::Window, (x, Shape::Straight)), (x, Shape::Straight));
    }

    #[test]
    fn a_straight_tunnel_opens_on_both_ends_of_its_axis() {
        assert_eq!(sides(Axis::Z, Shape::Straight), vec![Dir::North, Dir::South]);
        assert_eq!(sides(Axis::X, Shape::Window), vec![Dir::West, Dir::East]);
    }

    #[test]
    fn a_t_opens_towards_its_side_and_a_cross_both() {
        // Along z, left is west: a left T opens west, a right T east.
        assert_eq!(
            sides(Axis::Z, Shape::TLeft),
            vec![Dir::North, Dir::South, Dir::West]
        );
        assert_eq!(
            sides(Axis::Z, Shape::TRight),
            vec![Dir::North, Dir::South, Dir::East]
        );
        // Along x, left is south.
        assert_eq!(
            sides(Axis::X, Shape::TLeft),
            vec![Dir::South, Dir::West, Dir::East]
        );
        assert_eq!(
            sides(Axis::X, Shape::TRight),
            vec![Dir::North, Dir::West, Dir::East]
        );
        assert_eq!(sides(Axis::Z, Shape::Cross).len(), 4);
        // The left of `left(axis)` is the side the left T opens to.
        for axis in [Axis::X, Axis::Z] {
            assert!(sides(axis, Shape::TLeft).contains(&left(axis)));
            assert!(!sides(axis, Shape::TRight).contains(&left(axis)));
        }
    }

    #[test]
    fn a_covered_side_has_no_flap() {
        let all = sides(Axis::Z, Shape::Cross);
        let hung = flaps(&all, |d| if d == Dir::North { Occluder::Covered } else { Occluder::None });
        assert_eq!(hung, vec![Dir::South, Dir::West, Dir::East]);
    }

    #[test]
    fn a_brass_tunnel_distributes_with_a_flap_across_or_a_neighbour() {
        assert!(!distributes(&[], true, Axis::Z));
        assert!(!distributes(&[Dir::North, Dir::South], false, Axis::Z));
        assert!(distributes(&[Dir::North, Dir::South], true, Axis::Z));
        assert!(distributes(&[Dir::North, Dir::West], false, Axis::Z));
    }

    #[test]
    fn items_stuck_at_a_brass_tunnel_wait_for_the_filter_and_for_room() {
        let open = BrassAhead { entry_allowed: true, distributes: true, holding: false, can_take: true };
        assert_eq!(enter_brass(&open), Verdict::Take);
        assert_eq!(enter_brass(&BrassAhead { entry_allowed: false, ..open }), Verdict::Stuck);
        assert_eq!(enter_brass(&BrassAhead { holding: true, can_take: false, ..open }), Verdict::Stuck);
        assert_eq!(enter_brass(&BrassAhead { can_take: false, ..open }), Verdict::Wait);
        assert_eq!(enter_brass(&BrassAhead { distributes: false, ..open }), Verdict::Pass);
        // Without distribution the stack it holds doesn't matter.
        assert_eq!(
            enter_brass(&BrassAhead { distributes: false, holding: true, can_take: false, ..open }),
            Verdict::Pass
        );
    }

    #[test]
    fn a_stuck_item_waits_just_before_the_tunnel() {
        assert_eq!(stuck_position(3, true), 3.99);
        assert_eq!(stuck_position(3, false), 3.01);
    }

    #[test]
    fn only_a_junction_along_the_belt_splits_a_stack() {
        assert!(splits(2, Shape::Cross, Axis::Z, Axis::Z));
        assert!(splits(5, Shape::TLeft, Axis::X, Axis::X));
        assert!(!splits(1, Shape::Cross, Axis::Z, Axis::Z));
        assert!(!splits(5, Shape::Straight, Axis::Z, Axis::Z));
        assert!(!splits(5, Shape::Window, Axis::Z, Axis::Z));
        assert!(!splits(5, Shape::Cross, Axis::X, Axis::Z));
    }

    #[test]
    fn a_split_gives_one_item_to_each_side_and_keeps_one() {
        let flaps = [Dir::North, Dir::South, Dir::West, Dir::East];
        let mut asked = Vec::new();
        let out = split(5, Axis::Z, &flaps, |d| {
            asked.push(d);
            Offer::Gave
        });
        // Along z, the sides are west then east.
        assert_eq!(out.given, vec![Dir::West, Dir::East]);
        assert!(!out.wait);
        assert_eq!(asked, vec![Dir::West, Dir::East]);
    }

    #[test]
    fn a_stack_of_two_gives_one_item_and_keeps_one() {
        let flaps = [Dir::North, Dir::South, Dir::West, Dir::East];
        let out = split(2, Axis::Z, &flaps, |_| Offer::Gave);
        assert_eq!(out.given, vec![Dir::West]);
    }

    #[test]
    fn a_side_without_a_flap_or_an_input_is_skipped() {
        // The west side has no flap; the east one has no input.
        let flaps = [Dir::North, Dir::South, Dir::East];
        let out = split(9, Axis::Z, &flaps, |_| Offer::Skip);
        assert!(out.given.is_empty());
        assert!(!out.wait);
    }

    #[test]
    fn a_refused_item_makes_the_stack_wait() {
        let flaps = [Dir::West, Dir::East];
        let out = split(9, Axis::Z, &flaps, |d| {
            if d == Dir::West { Offer::Gave } else { Offer::Wait }
        });
        assert_eq!(out.given, vec![Dir::West]);
        assert!(out.wait);
    }

    #[test]
    fn the_tunnel_flaps_settle_like_the_funnels() {
        assert_eq!(flap_value(true, 0.0), -1.0);
        assert!(flap_value(false, 100.0) < 0.01);
        assert!(flap_angle(0.5, 0) > 0.0);
        assert_eq!(FLAP_PIVOT, [0.0, 0.625, 0.0625]);
    }
}
