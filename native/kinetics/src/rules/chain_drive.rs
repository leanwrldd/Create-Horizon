//! Encased chain drives and adjustable chain gearshifts: the state rules of `ChainDriveBlock`
//! (`updateShape`, `getStateForPlacement`, `updateAfterWrenched`) and the signal of
//! `ChainGearshiftBlockEntity`. How the drives turn together is `create_kinetics::Chain`.
//!
//! Known differences from Create, for the README:
//! - Create updates a drive's `part` and `axis_along_first` the moment a neighbour changes
//!   (`updateShape`); here the block entity checks its four neighbours every tick, so a drive
//!   can show its old chain for up to a tick.
//! - The adjustable chain gearshift reads its redstone signal every tick instead of on neighbour
//!   updates and every 40 ticks (`lazyTick`), and each change of signal re-solves the network
//!   in the next tick rather than detaching and reattaching it.

use crate::{Axis, ChainPart, Dir};

/// The block entity's NBT key for the signal (`ChainGearshiftBlockEntity.write`).
pub const SIGNAL: &str = "Signal";

/// The state properties of a chain drive that `updateShape` writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    pub axis: Axis,
    pub part: ChainPart,
    pub along_first: bool,
}

impl State {
    /// `defaultBlockState().setValue(AXIS, axis)`.
    pub fn alone(axis: Axis) -> Self {
        State {
            axis,
            part: ChainPart::Alone,
            along_first: false,
        }
    }

    /// `ChainDriveBlock.getConnectionAxis`.
    pub fn connection_axis(self) -> Axis {
        match (self.along_first, self.axis) {
            (true, Axis::X) => Axis::Y,
            (true, _) => Axis::X,
            (false, Axis::Z) => Axis::Y,
            (false, _) => Axis::Z,
        }
    }
}

/// `ChainDriveBlock.updateShape`: the state of a drive once its neighbour on `face` is
/// `neighbour` (`None` for a block that isn't a chain drive or gearshift).
pub fn update_shape(state: State, face: Dir, neighbour: Option<State>) -> State {
    let State {
        axis,
        mut part,
        along_first: mut connection_along_first,
    } = state;
    let connection_axis = state.connection_axis();
    let face_axis = face.axis();
    let facing_along_first = if axis == Axis::X {
        face_axis == Axis::Y
    } else {
        face_axis == Axis::X
    };
    let positive = face.sign() > 0;

    if axis == face_axis {
        return state;
    }

    let Some(other) = neighbour else {
        if facing_along_first != connection_along_first || part == ChainPart::Alone {
            return state;
        }
        return match part {
            ChainPart::Middle => State {
                part: if positive {
                    ChainPart::End
                } else {
                    ChainPart::Start
                },
                ..state
            },
            _ if (part == ChainPart::Start) == positive => State {
                part: ChainPart::Alone,
                ..state
            },
            _ => state,
        };
    };

    if other.axis == face_axis {
        return state;
    }
    if other.part != ChainPart::Alone && other.connection_axis() != face_axis {
        return state;
    }

    if part == ChainPart::Alone {
        part = if positive {
            ChainPart::Start
        } else {
            ChainPart::End
        };
        connection_along_first = facing_along_first;
    } else if connection_axis != face_axis {
        return state;
    }

    if (part == ChainPart::Start) != positive {
        part = ChainPart::Middle;
    }
    State {
        axis,
        part,
        along_first: connection_along_first,
    }
}

/// `ChainDriveBlock.getStateForPlacement` after the axis is chosen, and `updateAfterWrenched`:
/// a fresh drive on `axis`, told of each neighbour in `Iterate.directions` order.
/// `neighbour(face)` is the chain drive's state on that side, if it is one.
pub fn settle(axis: Axis, neighbour: impl Fn(Dir) -> Option<State>) -> State {
    Dir::ALL
        .into_iter()
        .filter(|face| face.axis() != axis)
        .fold(State::alone(axis), |state, face| {
            update_shape(state, face, neighbour(face))
        })
}

/// `ChainDriveBlock.getRotatedBlockState`: the axis a Wrench on `face_axis` turns a drive to. One
/// with a chain turns about the axis of its chain instead of the face's.
pub fn wrenched_axis(state: State, face_axis: Axis) -> Axis {
    let about = if state.part == ChainPart::Alone {
        face_axis
    } else {
        state.connection_axis()
    };
    super::encased::rotate_axis(state.axis, about)
}

/// `ChainGearshiftBlock.neighborChanged`: whether the block's `powered` is set for a signal.
pub fn powered(signal: i32) -> bool {
    signal > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drive(axis: Axis, part: ChainPart, along_first: bool) -> State {
        State {
            axis,
            part,
            along_first,
        }
    }

    #[test]
    fn the_connection_axis_follows_the_first_or_second_coordinate() {
        let axis = |a, first| drive(a, ChainPart::Start, first).connection_axis();
        assert_eq!(axis(Axis::Y, true), Axis::X);
        assert_eq!(axis(Axis::Y, false), Axis::Z);
        assert_eq!(axis(Axis::X, true), Axis::Y);
        assert_eq!(axis(Axis::X, false), Axis::Z);
        assert_eq!(axis(Axis::Z, true), Axis::X);
        assert_eq!(axis(Axis::Z, false), Axis::Y);
    }

    #[test]
    fn a_drive_beside_another_starts_a_chain_towards_it() {
        let beside = |faces: &'static [Dir]| {
            settle(Axis::Y, |face| {
                faces.contains(&face).then(|| State::alone(Axis::Y))
            })
        };
        // East of a y drive: it starts a chain along x, running to the east.
        assert_eq!(beside(&[Dir::East]), drive(Axis::Y, ChainPart::Start, true));
        // West of it: the chain ends there instead.
        assert_eq!(beside(&[Dir::West]), drive(Axis::Y, ChainPart::End, true));
        // North of it: along z.
        assert_eq!(beside(&[Dir::North]), drive(Axis::Y, ChainPart::End, false));
        // On both sides: the middle of a chain.
        assert_eq!(
            beside(&[Dir::West, Dir::East]),
            drive(Axis::Y, ChainPart::Middle, true)
        );
        // No neighbour: alone.
        assert_eq!(beside(&[]), State::alone(Axis::Y));
    }

    #[test]
    fn a_drive_whose_neighbour_goes_loses_that_end_of_its_chain() {
        let middle = drive(Axis::Y, ChainPart::Middle, true);
        assert_eq!(
            update_shape(middle, Dir::East, None),
            drive(Axis::Y, ChainPart::End, true)
        );
        assert_eq!(
            update_shape(middle, Dir::West, None),
            drive(Axis::Y, ChainPart::Start, true)
        );
        let start = drive(Axis::Y, ChainPart::Start, true);
        assert_eq!(update_shape(start, Dir::East, None).part, ChainPart::Alone);
        // The other end of a start is not its chain.
        assert_eq!(update_shape(start, Dir::West, None), start);
        // A side that isn't along the chain leaves it alone.
        assert_eq!(update_shape(start, Dir::North, None), start);
        // Nothing happens to a drive with no chain.
        let alone = State::alone(Axis::Y);
        assert_eq!(update_shape(alone, Dir::East, None), alone);
        // Nor for a neighbour on the drive's own axis.
        assert_eq!(update_shape(start, Dir::Up, None), start);
        assert_eq!(
            update_shape(alone, Dir::Up, Some(State::alone(Axis::Y))),
            alone
        );
    }

    #[test]
    fn drives_do_not_join_across_each_others_axes() {
        // A neighbour lying along the direction of the face never joins.
        let alone = State::alone(Axis::Y);
        assert_eq!(
            update_shape(alone, Dir::East, Some(State::alone(Axis::X))),
            alone
        );
        // A neighbour with a chain along another axis doesn't join.
        let other = drive(Axis::Y, ChainPart::Start, false);
        assert_eq!(update_shape(alone, Dir::East, Some(other)), alone);
        // But one whose chain points the right way does.
        let other = drive(Axis::Y, ChainPart::End, true);
        assert_eq!(
            update_shape(alone, Dir::East, Some(other)),
            drive(Axis::Y, ChainPart::Start, true)
        );
        // A drive with a chain along x doesn't take a neighbour to the north.
        let start = drive(Axis::Y, ChainPart::Start, true);
        assert_eq!(
            update_shape(start, Dir::North, Some(State::alone(Axis::Y))),
            start
        );
    }

    #[test]
    fn a_wrench_turns_a_chain_about_its_own_axis() {
        // Alone: about the face's axis, as any block with an axis.
        assert_eq!(wrenched_axis(State::alone(Axis::Y), Axis::X), Axis::Z);
        // With a chain along x: about x whatever face is clicked.
        let chained = drive(Axis::Y, ChainPart::Start, true);
        assert_eq!(wrenched_axis(chained, Axis::Y), Axis::Z);
        assert_eq!(wrenched_axis(chained, Axis::Z), Axis::Z);
    }

    #[test]
    fn a_signal_powers_the_block() {
        assert!(!powered(0));
        assert!(powered(1));
        assert!(powered(15));
    }
}
