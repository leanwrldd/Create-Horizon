//! Where the blocks of a belt are on every slope, how a Belt Connector lays one, and where an item
//! on it is in the world (Belts III).
//!
//! Ported from Create 6.0.10 (`mc1.21.1/dev` at `4148fd8`, MIT): `BeltBlock.nextSegmentPosition`,
//! `BeltHelper` (`getPositionForOffset`, `getVectorForOffset`), `BeltConnectorItem` (`canConnect`,
//! `createBelts`, `getFacingFromTo`, `getSlopeBetween`, `getBeltChainBetween`),
//! `BeltBlockEntity.getMovementDirection` and the item part of `BeltRenderer.renderItem`.
//!
//! A belt is a chain of blocks from its start (the controller, index 0) to its end. On a flat or
//! sideways belt each block is one along the facing; on a slope one along the facing and one up
//! (`UPWARD`) or down (`DOWNWARD`); on a vertical belt one up or down, by the sign of the facing.
//! An item's position counts blocks along the chain, as in [`crate::belt`].

use crate::belt::{Part, Slope};
use crate::{Axis, Dir, Pos};

fn add(a: Pos, b: Pos) -> Pos {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: Pos, by: i32) -> Pos {
    [a[0] * by, a[1] * by, a[2] * by]
}

/// `BeltHelper`'s `verticality`: how far up each block of a diagonal belt is from the one before.
pub const fn verticality(slope: Slope) -> i32 {
    match slope {
        Slope::Upward => 1,
        Slope::Downward => -1,
        _ => 0,
    }
}

/// The step from one block of the chain to the next one towards the end
/// (`BeltBlock.nextSegmentPosition` with `forward`).
pub const fn step(facing: Dir, slope: Slope) -> Pos {
    if matches!(slope, Slope::Vertical) {
        return [0, facing.sign(), 0];
    }
    let n = facing.normal();
    [n[0], verticality(slope), n[2]]
}

/// `BeltBlock.nextSegmentPosition`: the next block of the chain, or the previous one, or none
/// past either end.
pub fn next_segment(pos: Pos, facing: Dir, slope: Slope, part: Part, forward: bool) -> Option<Pos> {
    if (part == Part::End && forward) || (part == Part::Start && !forward) {
        return None;
    }
    Some(add(pos, scale(step(facing, slope), if forward { 1 } else { -1 })))
}

/// Where the blocks of one belt are (`BeltHelper.getPositionForOffset`): its controller, the way
/// it faces, its slope and the number of blocks it has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chain {
    pub controller: Pos,
    pub facing: Dir,
    pub slope: Slope,
    pub length: i32,
}

impl Chain {
    /// The block at `offset` from the controller. Past either end a diagonal belt goes on along the
    /// facing at the height of its last block, as Create's does (`Mth.clamp(offset, 0, beltLength - 1)`):
    /// that is where its items go on to, level with the end.
    pub fn at(self, offset: i32) -> Pos {
        if self.slope == Slope::Vertical {
            return add(self.controller, [0, offset * self.facing.sign(), 0]);
        }
        let n = self.facing.normal();
        let up = offset.clamp(0, (self.length - 1).max(0)) * verticality(self.slope);
        add(self.controller, [offset * n[0], up, offset * n[2]])
    }

    /// The controller of a belt one of whose blocks, at `index` from it, is at `pos`.
    pub fn controller_of(pos: Pos, facing: Dir, slope: Slope, index: i32) -> Pos {
        add(pos, scale(step(facing, slope), -index))
    }

    /// Whether an item can ride a belt of this slope (`BeltBlock.canTransportObjects`).
    pub fn transports(self) -> bool {
        self.slope.transports()
    }
}

/// `BeltHelper.getVectorForOffset`: the world position of the belt's surface line (the middle of
/// the block at the height of its centre) at `offset` blocks along the chain.
pub fn vector_for_offset(chain: Chain, offset: f32) -> [f64; 3] {
    let verticality = verticality(chain.slope) as f32;
    let mut vertical = verticality;
    if offset < 0.5 {
        vertical = 0.0;
    }
    vertical *= offset.min(chain.length as f32 - 0.5) - 0.5;
    let n = chain.facing.normal();
    let along = if chain.slope == Slope::Vertical { 0.0 } else { offset - 0.5 };
    [
        f64::from(chain.controller[0]) + 0.5 + f64::from(n[0] as f32 * along),
        f64::from(chain.controller[1]) + 0.5 + f64::from(vertical),
        f64::from(chain.controller[2]) + 0.5 + f64::from(n[2] as f32 * along),
    ]
}

/// `BeltBlockEntity.getMovementDirection(firstHalf, ignoreHalves)` for a belt whose controller
/// turns at `rpm`: the way an item or entity at that half of the block moves, in blocks.
/// `ignore_halves` is `getBeltChainDirection`, the way items fly off the end.
pub fn movement_direction(
    rpm: f32,
    facing: Dir,
    slope: Slope,
    part: Part,
    first_half: bool,
    ignore_halves: bool,
) -> Pos {
    if rpm == 0.0 {
        return [0, 0, 0];
    }
    let axis = facing.axis();
    let mut movement = if axis == Axis::X { Dir::West } else { Dir::South };
    if rpm < 0.0 {
        movement = movement.opposite();
    }
    let n = movement.normal();
    let slope_before_half = (part == Part::End) == (facing.sign() > 0);
    let on_slope =
        slope != Slope::Horizontal && (part == Part::Middle || slope_before_half == first_half || ignore_halves);
    if !on_slope {
        return n;
    }
    let up_slope = if movement == facing { Slope::Upward } else { Slope::Downward };
    [n[0], if slope == up_slope { 1 } else { -1 }, n[2]]
}

/// Why two shafts can't be joined (`BeltConnectorItem.canConnect` refuses).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Not closer than [`crate::belt::MAX_LENGTH`].
    TooFar,
    /// The same shaft twice.
    Same,
    /// Not in a line or a diagonal that crosses both shafts' axis, or the axes differ.
    NotInLine,
}

/// A belt a Belt Connector would lay between two shafts (`BeltConnectorItem.createBelts`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connection {
    /// The way it faces, from the start (the first shaft) towards the end.
    pub facing: Dir,
    pub slope: Slope,
    /// Every block of it in order, the two shafts' blocks included.
    pub chain: Vec<Pos>,
}

/// Where a belt between the shafts at `first` and `second`, both turning about `axis`, goes, as
/// `BeltConnectorItem.canConnect` and `createBelts` lay it. Only the geometry: the game still
/// checks the blocks between and the shafts' speeds.
///
/// Two shafts side by side across their axis make a flat belt; one a block up for every block
/// along makes a diagonal one; one straight above the other, a vertical one; and two upright
/// shafts (axis Y) a sideways one.
pub fn connect(first: Pos, second: Pos, axis: Axis, second_axis: Axis) -> Result<Connection, Refusal> {
    let diff = [second[0] - first[0], second[1] - first[1], second[2] - first[2]];
    if diff == [0, 0, 0] {
        return Err(Refusal::Same);
    }
    // `Vec3i.closerThan`: distSqr < maxLength².
    let distance = diff.iter().map(|c| c * c).sum::<i32>();
    if distance >= crate::belt::MAX_LENGTH * crate::belt::MAX_LENGTH {
        return Err(Refusal::TooFar);
    }
    let [x, y, z] = diff.map(i32::abs);
    let sames = i32::from(x == y) + i32::from(y == z) + i32::from(z == x);
    if diff[axis.index()] != 0 || sames != 1 || axis != second_axis {
        return Err(Refusal::NotInLine);
    }
    // Upright shafts can't be joined on a diagonal.
    if axis == Axis::Y && diff[0] != 0 && diff[2] != 0 {
        return Err(Refusal::NotInLine);
    }
    // `getSlopeBetween`.
    let slope = if diff[1] != 0 {
        if diff[0] != 0 || diff[2] != 0 {
            if diff[1] > 0 { Slope::Upward } else { Slope::Downward }
        } else {
            Slope::Vertical
        }
    } else {
        Slope::Horizontal
    };
    // `getFacingFromTo`: along x unless the two share an x; straight up or down by the way up.
    let belt_axis = if first[0] == second[0] { Axis::Z } else { Axis::X };
    let positive = if diff[0] == 0 && diff[2] == 0 {
        diff[1] > 0
    } else {
        diff[belt_axis.index()] > 0
    };
    let mut facing = match (belt_axis, positive) {
        (Axis::X, true) => Dir::East,
        (Axis::X, false) => Dir::West,
        (_, true) => Dir::South,
        (_, false) => Dir::North,
    };
    // `if (diff.getX() == diff.getZ())`: a vertical belt faces across the shafts' axis.
    if diff[0] == diff[2] {
        let across = if axis == Axis::X { Axis::Z } else { Axis::X };
        facing = match (across, facing.sign() > 0) {
            (Axis::X, true) => Dir::East,
            (Axis::X, false) => Dir::West,
            (_, true) => Dir::South,
            (_, false) => Dir::North,
        };
    }
    // `getBeltChainBetween` (with the slope as it is before the shafts make it sideways).
    let mut chain = vec![first];
    let mut current = first;
    let each = step(facing, slope);
    for _ in 0..1000 {
        current = add(current, each);
        if current == second {
            break;
        }
        chain.push(current);
    }
    chain.push(second);
    // `pulley && shaftState.getValue(AXIS) == Axis.Y`: shafts standing upright lay it sideways.
    let slope = if axis == Axis::Y { Slope::Sideways } else { slope };
    Ok(Connection { facing, slope, chain })
}

/// Where an item rides, as `BeltRenderer.renderItem` puts it: from the block `controller` corner,
/// the item is moved `offset` along the facing, `vertical` up, then tilted `tilt` degrees about
/// `tilt_axis` and lifted `lift`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemPose {
    /// `verticalMovement`: how far up the slope the item is.
    pub vertical: f32,
    /// The item lies on the sloped part (`onSlope`), not an end half block.
    pub on_slope: bool,
    /// `slopeAngle`: -45, 45, or 0 off the slope.
    pub tilt: f32,
    /// The axis it tilts about: `slopeAlongX ? Z : X`.
    pub tilt_axis: Axis,
    /// `slopeOffset`: 1/8 on the slope.
    pub lift: f32,
}

pub fn item_pose(facing: Dir, slope: Slope, length: i32, offset: f32) -> ItemPose {
    let verticality = verticality(slope) as f32;
    let vertical = if offset < 0.5 {
        0.0
    } else {
        verticality * (offset.min(length as f32 - 0.5) - 0.5)
    };
    let on_slope = slope != Slope::Horizontal
        && offset.clamp(0.5, (length as f32 - 0.5).max(0.5)) == offset;
    let tilt_forward = ((slope == Slope::Downward) ^ (facing.sign() > 0)) == (facing.axis() == Axis::Z);
    ItemPose {
        vertical,
        on_slope,
        tilt: if on_slope {
            if tilt_forward { -45.0 } else { 45.0 }
        } else {
            0.0
        },
        tilt_axis: if facing.axis() == Axis::X { Axis::Z } else { Axis::X },
        lift: if on_slope { 1.0 / 8.0 } else { 0.0 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(first: Pos, second: Pos, axis: Axis) -> Connection {
        connect(first, second, axis, axis).unwrap()
    }

    #[test]
    fn a_chain_steps_one_along_the_facing_and_one_up_or_down_on_a_slope() {
        assert_eq!(step(Dir::East, Slope::Horizontal), [1, 0, 0]);
        assert_eq!(step(Dir::East, Slope::Sideways), [1, 0, 0]);
        assert_eq!(step(Dir::East, Slope::Upward), [1, 1, 0]);
        assert_eq!(step(Dir::North, Slope::Downward), [0, -1, -1]);
        // A vertical belt goes up or down with the sign of its facing.
        assert_eq!(step(Dir::South, Slope::Vertical), [0, 1, 0]);
        assert_eq!(step(Dir::North, Slope::Vertical), [0, -1, 0]);
        assert_eq!(step(Dir::West, Slope::Vertical), [0, -1, 0]);
        assert_eq!(step(Dir::East, Slope::Vertical), [0, 1, 0]);
    }

    #[test]
    fn next_segment_follows_the_slope_and_stops_at_the_ends() {
        let at = [4, 10, 4];
        assert_eq!(next_segment(at, Dir::East, Slope::Upward, Part::Middle, true), Some([5, 11, 4]));
        assert_eq!(next_segment(at, Dir::East, Slope::Upward, Part::Middle, false), Some([3, 9, 4]));
        assert_eq!(next_segment(at, Dir::East, Slope::Downward, Part::Start, true), Some([5, 9, 4]));
        assert_eq!(next_segment(at, Dir::South, Slope::Vertical, Part::Middle, true), Some([4, 11, 4]));
        assert_eq!(next_segment(at, Dir::North, Slope::Vertical, Part::Pulley, true), Some([4, 9, 4]));
        assert_eq!(next_segment(at, Dir::East, Slope::Upward, Part::End, true), None);
        assert_eq!(next_segment(at, Dir::East, Slope::Upward, Part::Start, false), None);
    }

    #[test]
    fn positions_for_an_offset_climb_with_the_slope_and_level_off_past_the_ends() {
        let chain = Chain { controller: [0, 5, 0], facing: Dir::South, slope: Slope::Upward, length: 4 };
        assert_eq!(chain.at(0), [0, 5, 0]);
        assert_eq!(chain.at(3), [0, 8, 3]);
        // Past either end: along the facing, at the height of the last block (`Mth.clamp`).
        assert_eq!(chain.at(4), [0, 8, 4]);
        assert_eq!(chain.at(-1), [0, 5, -1]);
        let down = Chain { slope: Slope::Downward, ..chain };
        assert_eq!(down.at(2), [0, 3, 2]);
        assert_eq!(down.at(4), [0, 2, 4]);
        // Inverse: the controller from any block.
        for index in 0..4 {
            assert_eq!(Chain::controller_of(chain.at(index), Dir::South, Slope::Upward, index), [0, 5, 0]);
            assert_eq!(Chain::controller_of(down.at(index), Dir::South, Slope::Downward, index), [0, 5, 0]);
        }
        let vertical = Chain { slope: Slope::Vertical, ..chain };
        assert_eq!(vertical.at(2), [0, 7, 0]);
        assert!(!vertical.transports());
        assert!(chain.transports());
    }

    #[test]
    fn a_horizontal_belt_is_unchanged_by_the_slope_rules() {
        // The old horizontal chain: `controller + offset * facing`.
        let chain = Chain { controller: [10, 3, 5], facing: Dir::West, slope: Slope::Horizontal, length: 5 };
        for offset in -2..8 {
            assert_eq!(chain.at(offset), [10 - offset, 3, 5]);
        }
    }

    #[test]
    fn the_surface_line_rises_with_the_slope_between_the_half_blocks() {
        let chain = Chain { controller: [0, 0, 0], facing: Dir::South, slope: Slope::Upward, length: 4 };
        // Before the middle of the first block: level; then one block up for each block along.
        assert_eq!(vector_for_offset(chain, 0.25), [0.5, 0.5, 0.25 + 0.0]);
        assert_eq!(vector_for_offset(chain, 0.5), [0.5, 0.5, 0.5 + 0.0 + 0.0]);
        let v = vector_for_offset(chain, 2.5);
        assert!((v[1] - 2.5).abs() < 1e-6 && (v[2] - 2.5).abs() < 1e-6, "{v:?}");
        // Past the last block's middle it stops rising.
        let end = vector_for_offset(chain, 4.0);
        assert!((end[1] - 3.5).abs() < 1e-6, "{end:?}");
        let down = Chain { slope: Slope::Downward, ..chain };
        assert!((vector_for_offset(down, 2.5)[1] + 1.5).abs() < 1e-6);
        // Horizontal: never up.
        let flat = Chain { slope: Slope::Horizontal, ..chain };
        assert_eq!(vector_for_offset(flat, 3.5)[1], 0.5);
    }

    #[test]
    fn items_fly_off_a_slope_along_it() {
        // `getBeltChainDirection`: up the slope going with the facing, down going against it.
        // Axis Z: the belt top moves south (positive) at a positive speed.
        assert_eq!(movement_direction(32.0, Dir::South, Slope::Upward, Part::Middle, true, true), [0, 1, 1]);
        assert_eq!(movement_direction(-32.0, Dir::South, Slope::Upward, Part::Middle, true, true), [0, -1, -1]);
        assert_eq!(movement_direction(32.0, Dir::North, Slope::Upward, Part::End, true, true), [0, -1, 1]);
        // Along x the positive speed moves west.
        assert_eq!(movement_direction(32.0, Dir::East, Slope::Upward, Part::Middle, true, true), [-1, -1, 0]);
        assert_eq!(movement_direction(32.0, Dir::West, Slope::Upward, Part::Middle, true, true), [-1, 1, 0]);
        // A flat belt never rises, and a stopped one doesn't move.
        assert_eq!(movement_direction(32.0, Dir::South, Slope::Horizontal, Part::Middle, true, true), [0, 0, 1]);
        assert_eq!(movement_direction(0.0, Dir::South, Slope::Upward, Part::Middle, true, true), [0, 0, 0]);
        // An end half of a slope block is flat on one half: the flat half doesn't rise.
        // On the end block, the half before the end is the sloped one.
        let slope_half = movement_direction(32.0, Dir::South, Slope::Upward, Part::End, true, false);
        let flat_half = movement_direction(32.0, Dir::South, Slope::Upward, Part::End, false, false);
        assert_eq!(slope_half, [0, 1, 1]);
        assert_eq!(flat_half, [0, 0, 1]);
    }

    #[test]
    fn a_flat_connection_runs_between_the_shafts() {
        let c = ok([0, 0, 0], [3, 0, 0], Axis::Z);
        assert_eq!((c.facing, c.slope), (Dir::East, Slope::Horizontal));
        assert_eq!(c.chain, vec![[0, 0, 0], [1, 0, 0], [2, 0, 0], [3, 0, 0]]);
        let c = ok([0, 0, 0], [0, 0, -2], Axis::X);
        assert_eq!((c.facing, c.slope), (Dir::North, Slope::Horizontal));
        assert_eq!(c.chain.len(), 3);
    }

    #[test]
    fn a_diagonal_connection_climbs_a_block_for_every_block_along() {
        // Shafts about z, three along x and three up.
        let c = ok([0, 0, 0], [3, 3, 0], Axis::Z);
        assert_eq!((c.facing, c.slope), (Dir::East, Slope::Upward));
        assert_eq!(c.chain, vec![[0, 0, 0], [1, 1, 0], [2, 2, 0], [3, 3, 0]]);
        // Down and the other way.
        let c = ok([3, 3, 0], [0, 0, 0], Axis::Z);
        assert_eq!((c.facing, c.slope), (Dir::West, Slope::Downward));
        assert_eq!(c.chain, vec![[3, 3, 0], [2, 2, 0], [1, 1, 0], [0, 0, 0]]);
        // Shafts about x, along z.
        let c = ok([5, 1, 5], [5, 4, 2], Axis::X);
        assert_eq!((c.facing, c.slope), (Dir::North, Slope::Upward));
        assert_eq!(c.chain, vec![[5, 1, 5], [5, 2, 4], [5, 3, 3], [5, 4, 2]]);
        let c = ok([5, 4, 2], [5, 1, 5], Axis::X);
        assert_eq!((c.facing, c.slope), (Dir::South, Slope::Downward));
        // One block of height for two along isn't a belt, and a diagonal in the shafts' own
        // plane isn't either.
        assert_eq!(connect([0, 0, 0], [4, 2, 0], Axis::Z, Axis::Z), Err(Refusal::NotInLine));
        assert_eq!(connect([0, 0, 0], [2, 2, 0], Axis::X, Axis::X), Err(Refusal::NotInLine));
        // Too far: the distance counts the height too.
        assert!(connect([0, 0, 0], [14, 14, 0], Axis::Z, Axis::Z).is_ok());
        assert_eq!(connect([0, 0, 0], [15, 15, 0], Axis::Z, Axis::Z), Err(Refusal::TooFar));
    }

    #[test]
    fn a_vertical_connection_faces_across_the_shafts_and_goes_up_with_its_sign() {
        let c = ok([0, 0, 0], [0, 4, 0], Axis::X);
        assert_eq!((c.facing, c.slope), (Dir::South, Slope::Vertical));
        assert_eq!(c.chain, vec![[0, 0, 0], [0, 1, 0], [0, 2, 0], [0, 3, 0], [0, 4, 0]]);
        let c = ok([0, 4, 0], [0, 0, 0], Axis::X);
        assert_eq!((c.facing, c.slope), (Dir::North, Slope::Vertical));
        assert_eq!(c.chain.first(), Some(&[0, 4, 0]));
        // Shafts about z: it faces along x.
        let c = ok([0, 0, 0], [0, 3, 0], Axis::Z);
        assert_eq!((c.facing, c.slope), (Dir::East, Slope::Vertical));
        let c = ok([0, 3, 0], [0, 0, 0], Axis::Z);
        assert_eq!((c.facing, c.slope), (Dir::West, Slope::Vertical));
        // Whichever way it faces, `nextSegmentPosition` goes the way the chain was laid.
        let c = ok([2, 9, 2], [2, 5, 2], Axis::Z);
        assert_eq!(next_segment(c.chain[0], c.facing, c.slope, Part::Start, true), Some(c.chain[1]));
    }

    #[test]
    fn upright_shafts_make_a_sideways_belt() {
        let c = ok([0, 0, 0], [4, 0, 0], Axis::Y);
        assert_eq!((c.facing, c.slope), (Dir::East, Slope::Sideways));
        assert_eq!(c.chain.len(), 5);
        let c = ok([0, 0, 3], [0, 0, 0], Axis::Y);
        assert_eq!((c.facing, c.slope), (Dir::North, Slope::Sideways));
        // Upright shafts can't be on a diagonal in the ground plane, nor at different heights.
        assert_eq!(connect([0, 0, 0], [2, 0, 2], Axis::Y, Axis::Y), Err(Refusal::NotInLine));
        assert_eq!(connect([0, 0, 0], [2, 2, 0], Axis::Y, Axis::Y), Err(Refusal::NotInLine));
        assert_eq!(connect([0, 0, 0], [0, 3, 0], Axis::Y, Axis::Y), Err(Refusal::NotInLine));
    }

    #[test]
    fn every_laid_chain_is_walked_by_next_segment() {
        for (first, second, axis) in [
            ([0, 0, 0], [5, 0, 0], Axis::Z),
            ([0, 0, 0], [5, 5, 0], Axis::Z),
            ([5, 5, 0], [0, 0, 0], Axis::Z),
            ([0, 0, 0], [0, 4, 4], Axis::X),
            ([0, 4, 4], [0, 0, 0], Axis::X),
            ([0, 0, 0], [0, 6, 0], Axis::X),
            ([0, 6, 0], [0, 0, 0], Axis::Z),
            ([0, 0, 0], [0, 0, 6], Axis::Y),
        ] {
            let c = ok(first, second, axis);
            let last = c.chain.len() - 1;
            for (i, pair) in c.chain.windows(2).enumerate() {
                let part = if i == 0 { Part::Start } else { Part::Middle };
                assert_eq!(next_segment(pair[0], c.facing, c.slope, part, true), Some(pair[1]), "{first:?} {second:?}");
                let part = if i + 1 == last { Part::End } else { Part::Middle };
                assert_eq!(next_segment(pair[1], c.facing, c.slope, part, false), Some(pair[0]));
            }
        }
    }

    #[test]
    fn items_tilt_on_the_slope_only() {
        // South, going up: the belt rises along z, so the item tilts about x.
        let pose = item_pose(Dir::South, Slope::Upward, 4, 2.5);
        assert!(pose.on_slope);
        assert_eq!(pose.tilt_axis, Axis::X);
        assert_eq!(pose.tilt.abs(), 45.0);
        assert_eq!(pose.lift, 0.125);
        assert!((pose.vertical - 2.0).abs() < 1e-6);
        // Along x it tilts about z.
        assert_eq!(item_pose(Dir::East, Slope::Upward, 4, 2.5).tilt_axis, Axis::Z);
        // Going up and going down tilt opposite ways.
        let up = item_pose(Dir::South, Slope::Upward, 4, 2.5).tilt;
        let down = item_pose(Dir::South, Slope::Downward, 4, 2.5).tilt;
        assert_eq!(up, -down);
        // Facing the other way round flips it back.
        assert_eq!(item_pose(Dir::North, Slope::Upward, 4, 2.5).tilt, down);
        // On the first or last half block, or on a flat belt, nothing.
        for offset in [0.2, 3.8] {
            let p = item_pose(Dir::South, Slope::Upward, 4, offset);
            assert!(!p.on_slope && p.tilt == 0.0 && p.lift == 0.0);
        }
        let flat = item_pose(Dir::South, Slope::Horizontal, 4, 2.5);
        assert!(!flat.on_slope && flat.vertical == 0.0);
    }
}
