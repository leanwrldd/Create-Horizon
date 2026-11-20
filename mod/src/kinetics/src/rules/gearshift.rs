//! Gearshift and clutch: the rules of `GearshiftBlock`, `ClutchBlock` and the block entities
//! under them (`GearshiftBlockEntity`, `ClutchBlockEntity`, `SplitShaftBlockEntity`). How a
//! powered one connects its neighbours is `create_kinetics::Kind::gearshift` and `Kind::clutch`.
//!
//! Known differences from Create, for the README:
//! - Create cuts the network and re-attaches it a tick later when a gearshift changes
//!   (`detachKinetics`); here the network is solved again in the next tick, which looks the same.
//! - A powered clutch with a generator on each side keeps both sides turning (Create keeps the
//!   side the clutch was already attached to and may let the other go still).
//! - The redstone signal is read from the block entity's tick, but the world is asked only after a
//!   neighbour update (`mod/src/signals.rs`) or on a 20-tick heartbeat.

use crate::{Axis, Dir};

/// What a redstone signal does to a gearshift or clutch (`neighborChanged`): powered by any
/// signal at all (`hasNeighborSignal`).
pub fn powered(signal: i32) -> bool {
    signal > 0
}

/// The two kinds of split shaft.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Split {
    Gearshift,
    Clutch,
}

/// The two ends of a split shaft on `axis`, negative first.
pub fn ends(axis: Axis) -> [Dir; 2] {
    match axis {
        Axis::X => [Dir::West, Dir::East],
        Axis::Y => [Dir::Down, Dir::Up],
        Axis::Z => [Dir::North, Dir::South],
    }
}

/// The speed the half on `face` turns at, as `SplitShaftRenderer` draws it (`be.getSpeed()` times
/// `getRotationSpeedModifier(face)`). `speed` is the block's, which for a powered gearshift is
/// the positive end's (`create_kinetics::face_sign`); `input` is the end a powered clutch is
/// taking its rotation from.
pub fn half_speed(split: Split, powered: bool, speed: f32, face: Dir, input: Option<Dir>) -> f32 {
    match (split, powered) {
        (_, false) => speed,
        (Split::Gearshift, true) => speed * face.sign() as f32,
        (Split::Clutch, true) => {
            if input == Some(face) {
                speed
            } else {
                0.0
            }
        }
    }
}

/// Which end of a powered clutch its rotation comes in by: the end whose neighbour turns at the
/// clutch's own speed (`neighbour(face)` is that neighbour's speed, when it has a shaft end
/// pointing at the clutch). The end it came in by last time wins a tie, as the clutch stays
/// attached to its source.
pub fn clutch_input(
    axis: Axis,
    speed: f32,
    previous: Option<Dir>,
    neighbour: impl Fn(Dir) -> Option<f32>,
) -> Option<Dir> {
    if speed == 0.0 {
        return None;
    }
    let matches = |face: Dir| {
        neighbour(face).is_some_and(|other| (other - speed).abs() <= 1e-3 * speed.abs().max(1.0))
    };
    previous
        .filter(|face| face.axis() == axis && matches(*face))
        .or_else(|| ends(axis).into_iter().find(|face| matches(*face)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_signal_powers_it() {
        assert!(!powered(0));
        assert!(powered(1));
    }

    #[test]
    fn a_powered_gearshift_turns_its_halves_against_each_other() {
        let half = |face, powered| half_speed(Split::Gearshift, powered, 16.0, face, None);
        assert_eq!(half(Dir::East, false), 16.0);
        assert_eq!(half(Dir::West, false), 16.0);
        assert_eq!(half(Dir::East, true), 16.0);
        assert_eq!(half(Dir::West, true), -16.0);
        assert_eq!(half(Dir::Down, true), -16.0);
        assert_eq!(half(Dir::Up, true), 16.0);
    }

    #[test]
    fn a_powered_clutch_turns_only_the_half_it_takes_rotation_from() {
        let half = |face, input| half_speed(Split::Clutch, true, 16.0, face, input);
        assert_eq!(half(Dir::West, Some(Dir::West)), 16.0);
        assert_eq!(half(Dir::East, Some(Dir::West)), 0.0);
        assert_eq!(half(Dir::East, None), 0.0);
        assert_eq!(half_speed(Split::Clutch, false, 16.0, Dir::East, None), 16.0);
    }

    #[test]
    fn the_input_is_the_side_that_turns_with_the_clutch() {
        let beside = |west: Option<f32>, east: Option<f32>| {
            move |face: Dir| match face {
                Dir::West => west,
                Dir::East => east,
                _ => None,
            }
        };
        assert_eq!(
            clutch_input(Axis::X, 16.0, None, beside(Some(16.0), Some(0.0))),
            Some(Dir::West)
        );
        assert_eq!(
            clutch_input(Axis::X, -32.0, None, beside(None, Some(-32.0))),
            Some(Dir::East)
        );
        // Both sides turn at that speed: the end it already used.
        assert_eq!(
            clutch_input(Axis::X, 16.0, Some(Dir::East), beside(Some(16.0), Some(16.0))),
            Some(Dir::East)
        );
        assert_eq!(
            clutch_input(Axis::X, 16.0, None, beside(Some(16.0), Some(16.0))),
            Some(Dir::West)
        );
        // Standing still, or nothing turning with it.
        assert_eq!(clutch_input(Axis::X, 0.0, None, beside(Some(0.0), None)), None);
        assert_eq!(
            clutch_input(Axis::X, 16.0, None, beside(Some(8.0), None)),
            None
        );
    }
}
