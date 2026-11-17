//! Vertical Gearbox: which way `VerticalGearboxItem.updateCustomBlockEntityTag` turns the Gearbox
//! it places.

use crate::{Axis, Dir, Kind};

/// The four horizontal directions (`Iterate.horizontalDirections`).
const HORIZONTAL: [Dir; 4] = [Dir::North, Dir::South, Dir::West, Dir::East];

/// The axis of the shaft ends that point into the new gearbox from its four sides: `None` when
/// there are none, or when two of them lie on different axes (Create sets `prefferedAxis` back
/// to null and stops there).
pub fn preferred_axis(mut neighbour: impl FnMut(Dir) -> Option<Kind>) -> Option<Axis> {
    let mut preferred = None;
    for side in HORIZONTAL {
        if !neighbour(side).is_some_and(|kind| kind.has_shaft(side.opposite())) {
            continue;
        }
        match preferred {
            Some(axis) if axis != side.axis() => return None,
            _ => preferred = Some(side.axis()),
        }
    }
    preferred
}

/// The gearbox's `axis` property: the axis the shaft ends beside it do not use (so they meet its
/// side shafts), or, with none, `player.getDirection().getClockWise().getAxis()`.
pub fn axis(preferred: Option<Axis>, player_facing: Dir) -> Axis {
    match preferred {
        None => player_facing.clockwise_about(Axis::Y).axis(),
        Some(Axis::X) => Axis::Z,
        Some(_) => Axis::X,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_neighbouring_shaft_decides_the_axis() {
        let along_x = |side: Dir| match side {
            Dir::East => Some(Kind::Shaft(Axis::X)),
            _ => None,
        };
        assert_eq!(preferred_axis(along_x), Some(Axis::X));
        assert_eq!(axis(Some(Axis::X), Dir::North), Axis::Z);
        let along_z = |side: Dir| (side == Dir::North).then_some(Kind::Shaft(Axis::Z));
        assert_eq!(axis(preferred_axis(along_z), Dir::North), Axis::X);
    }

    #[test]
    fn a_shaft_not_pointing_in_is_ignored() {
        // A shaft along x to the north has no end facing the new block.
        let side = |side: Dir| (side == Dir::North).then_some(Kind::Shaft(Axis::X));
        assert_eq!(preferred_axis(side), None);
    }

    #[test]
    fn shafts_from_two_axes_drop_the_preference() {
        let both = |side: Dir| match side {
            Dir::East => Some(Kind::Shaft(Axis::X)),
            Dir::North => Some(Kind::Shaft(Axis::Z)),
            _ => None,
        };
        assert_eq!(preferred_axis(both), None);
        let opposite = |side: Dir| match side {
            Dir::East | Dir::West => Some(Kind::Shaft(Axis::X)),
            _ => None,
        };
        assert_eq!(preferred_axis(opposite), Some(Axis::X));
    }

    #[test]
    fn without_a_shaft_it_follows_the_player() {
        // Facing north, the clockwise side is east: the x axis.
        assert_eq!(axis(None, Dir::North), Axis::X);
        assert_eq!(axis(None, Dir::East), Axis::Z);
        assert_eq!(axis(None, Dir::South), Axis::X);
        assert_eq!(axis(None, Dir::West), Axis::Z);
    }
}
