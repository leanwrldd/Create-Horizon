//! What the Hand Crank and the Water Wheel make. Ported from Create's `HandCrankBlock`,
//! `HandCrankBlockEntity` and `WaterWheelBlockEntity` (MIT, the Creators of Create), checked
//! against Create `mc1.21.1/dev` at 4148fd8 (`AllBlocks`, `CKinetics`, `VecHelper.rotate`).

use crate::{Axis, Dir, Kind, Pos};

/// RPM a Hand Crank makes while turned (`HandCrankBlock.getRotationSpeed`).
pub const CRANK_SPEED: f32 = 32.0;
/// Stress units per RPM (`AllBlocks.HAND_CRANK`: `CStress.setCapacity(8.0)`).
pub const CRANK_CAPACITY: f32 = 8.0;
/// Ticks one click keeps a crank turning (`HandCrankBlockEntity.turn` sets `inUse = 10`).
pub const CRANK_TICKS: i32 = 10;
/// Food exhaustion per click: `getRotationSpeed() * crankHungerMultiplier` (default 0.01).
pub const CRANK_EXHAUSTION: f32 = CRANK_SPEED * 0.01;

/// RPM a small Water Wheel makes in a current (`getGeneratedSpeed`: 8 over its size, 1).
pub const WHEEL_SPEED: f32 = 8.0;
/// Stress units per RPM (`AllBlocks.WATER_WHEEL`: `CStress.setCapacity(32)`).
pub const WHEEL_CAPACITY: f32 = 32.0;

/// The crank's speed: 32 RPM while turned, the other way when the player sneaked, and
/// flipped for a handle facing a negative direction (`HandCrankBlockEntity.getGeneratedSpeed`
/// through `KineticBlockEntity.convertToDirection`).
pub fn crank_speed(ticks_left: i32, backwards: bool, facing: Dir) -> f32 {
    let turned = match (ticks_left > 0, backwards) {
        (false, _) => 0.0,
        (true, false) => CRANK_SPEED,
        (true, true) => -CRANK_SPEED,
    };
    turned * facing.sign() as f32
}

/// The four blocks around a small wheel, in the plane it turns in.
pub fn wheel_around(axis: Axis) -> impl Iterator<Item = Pos> {
    crate::Dir::ALL
        .into_iter()
        .filter(move |face| face.axis() != axis)
        .map(|face| face.normal())
}

/// Create's flow score (`determineAndApplyFlowScore`): each block around the wheel whose
/// current runs along the rim counts 1 for pushing it the positive way round its axis and -1
/// for the other. `flow` gives the current at an offset from the wheel: zero where there is no
/// moving fluid, and straight up or down in a bubble column (`getFlowVectorAtPosition`).
pub fn wheel_score(axis: Axis, mut flow: impl FnMut(Pos) -> [f64; 3]) -> i32 {
    let mut score = 0;
    for offset in wheel_around(axis) {
        let mut current = flow(offset);
        current[axis.index()] = 0.0;
        let length = current.iter().map(|v| v * v).sum::<f64>().sqrt();
        if length == 0.0 {
            continue;
        }
        // Where the rim moves at that offset when the wheel turns the positive way: the
        // offset turned 90 degrees about the axis, as `VecHelper.rotate(normal, 90, axis)`.
        let n = offset.map(f64::from);
        let rim = match axis {
            Axis::X => [0.0, -n[2], n[1]],
            Axis::Y => [n[2], 0.0, -n[0]],
            Axis::Z => [-n[1], n[0], 0.0],
        };
        let along: f64 = (0..3).map(|i| current[i] / length * rim[i]).sum();
        if along.abs() > 0.5 {
            score += along.signum() as i32;
        }
    }
    score
}

/// The wheel's speed for a flow score: one current or many, it makes the same.
pub fn wheel_speed(score: i32) -> f32 {
    score.clamp(-1, 1) as f32 * WHEEL_SPEED
}

/// Create's `DirectionalKineticBlock.getPreferredFacing` drops the preference
/// when shaft ends point in from different axes.
pub fn preferred_shaft(mut neighbor: impl FnMut(Dir) -> Option<Kind>) -> Option<Dir> {
    let mut sides = Dir::ALL
        .into_iter()
        .filter(|&side| neighbor(side).is_some_and(|kind| kind.has_shaft(side.opposite())));
    let first = sides.next()?;
    let mut preferred = first;
    for side in sides {
        if side.axis() != first.axis() {
            return None;
        }
        preferred = side;
    }
    Some(preferred)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_only_shaft_ends_facing_the_placement() {
        let preferred = preferred_shaft(|side| match side {
            Dir::North => Some(Kind::Shaft(Axis::X)),
            Dir::East => Some(Kind::Motor(Dir::East)),
            Dir::West => Some(Kind::Shaft(Axis::X)),
            _ => None,
        });
        assert_eq!(preferred, Some(Dir::West));
    }

    #[test]
    fn opposite_shaft_ends_share_an_axis_but_perpendicular_ends_conflict() {
        let along_x = |side| match side {
            Dir::East | Dir::West => Some(Kind::Shaft(Axis::X)),
            _ => None,
        };
        assert_eq!(preferred_shaft(along_x), Some(Dir::East));
        assert_eq!(
            preferred_shaft(|side| if side == Dir::Up {
                Some(Kind::Shaft(Axis::Y))
            } else {
                along_x(side)
            }),
            None
        );
    }

    #[test]
    fn a_crank_turns_only_while_held_and_backwards_when_sneaking() {
        assert_eq!(crank_speed(0, false, Dir::Up), 0.0);
        assert_eq!(crank_speed(0, true, Dir::Up), 0.0);
        assert_eq!(crank_speed(CRANK_TICKS, false, Dir::Up), 32.0);
        assert_eq!(crank_speed(1, true, Dir::Up), -32.0);
    }

    #[test]
    fn a_crank_facing_a_negative_direction_turns_the_other_way() {
        // So a crank turns the same way seen from its handle whichever way it faces.
        assert_eq!(crank_speed(5, false, Dir::Down), -32.0);
        assert_eq!(crank_speed(5, false, Dir::North), -32.0);
        assert_eq!(crank_speed(5, false, Dir::East), 32.0);
        assert_eq!(crank_speed(5, true, Dir::West), 32.0);
    }

    #[test]
    fn the_rim_direction_is_createss_vec_helper_rotate_by_90() {
        // VecHelper.rotate(v, 90, axis) with cos 90 = 0, sin 90 = 1:
        // X: (x, -z, y); Y: (z, y, -x); Z: (-y, x, z).
        let rotate = |v: [f64; 3], axis: Axis| match axis {
            Axis::X => [v[0], -v[2], v[1]],
            Axis::Y => [v[2], v[1], -v[0]],
            Axis::Z => [-v[1], v[0], v[2]],
        };
        for axis in Axis::ALL {
            for offset in wheel_around(axis) {
                let motion = rotate(offset.map(f64::from), axis);
                // A current along that motion at that one offset scores +1.
                let score = wheel_score(axis, |p| if p == offset { motion } else { [0.0; 3] });
                assert_eq!(score, 1, "{axis:?} {offset:?}");
            }
        }
    }

    #[test]
    fn a_wheel_samples_the_four_blocks_in_its_plane() {
        for axis in Axis::ALL {
            let around: Vec<Pos> = wheel_around(axis).collect();
            assert_eq!(around.len(), 4);
            assert!(around.iter().all(|p| p[axis.index()] == 0));
        }
    }

    #[test]
    fn still_water_or_water_along_the_axis_does_nothing() {
        assert_eq!(wheel_score(Axis::X, |_| [0.0; 3]), 0);
        // Flowing along the axle pushes no rim.
        assert_eq!(wheel_score(Axis::X, |_| [1.0, 0.0, 0.0]), 0);
        // The same current on every side pushes opposite rims opposite ways.
        assert_eq!(wheel_score(Axis::X, |_| [0.0, -1.0, 0.0]), 0);
    }

    #[test]
    fn water_falling_on_one_side_turns_the_wheel() {
        // Axle along x, water falling on the south (+z) side: the rim there moves down
        // when the wheel turns the positive way about +x.
        let south = |p: Pos| if p[2] > 0 { [0.0, -1.0, 0.0] } else { [0.0; 3] };
        assert_eq!(wheel_score(Axis::X, south), 1);
        assert_eq!(wheel_speed(wheel_score(Axis::X, south)), 8.0);
        // On the north side it turns the other way.
        let north = |p: Pos| if p[2] < 0 { [0.0, -1.0, 0.0] } else { [0.0; 3] };
        assert_eq!(wheel_speed(wheel_score(Axis::X, north)), -8.0);
    }

    #[test]
    fn more_water_does_not_turn_it_faster() {
        // Over the top going north and under the bottom going south: both push the same way.
        let both = |p: Pos| match p {
            [0, 1, 0] => [0.0, 0.0, -1.0],
            [0, -1, 0] => [0.0, 0.0, 1.0],
            _ => [0.0; 3],
        };
        assert_eq!(wheel_score(Axis::X, both), -2);
        assert_eq!(wheel_speed(-2), -8.0);
    }

    #[test]
    fn a_diagonal_current_counts_only_if_it_mostly_follows_the_rim() {
        let slant = |p: Pos| if p[2] > 0 { [0.0, -1.0, 1.0] } else { [0.0; 3] };
        // 45 degrees off the rim: cos 45 is above 0.5, so it counts.
        assert_eq!(wheel_score(Axis::X, slant), 1);
        let shallow = |p: Pos| if p[2] > 0 { [0.0, -0.4, 1.0] } else { [0.0; 3] };
        assert_eq!(wheel_score(Axis::X, shallow), 0);
    }
}
