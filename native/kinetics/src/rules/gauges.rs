//! Speedometer and stressometer: the rules of `GaugeBlock` (`DirectionalAxisKineticBlock`),
//! `SpeedGaugeBlockEntity`, `StressGaugeBlockEntity` and `GaugeRenderer`.
//!
//! Known differences from Create, for the README:
//! - No goggles tooltip, no comparator output, no dust particles in the dial's colour.
//! - The dial eases towards its value on the server's clock as Create's does, but without the
//!   random flutter past 1 (`GaugeBlockEntity.tick`).
//! - A head is drawn on every face Create could draw one on, whether or not a neighbour hides
//!   it (`Block.shouldRenderFace`); a neighbour that covers it hides it all the same.
//! - A gauge placed on the end of a shaft finds that shaft from the block entities that have
//!   ticked, so a shaft placed the same tick isn't seen.

use crate::{Axis, Dir};

/// Create's `kinetics.mediumSpeed`, `fastSpeed` and `maxRotationSpeed` defaults (`CKinetics`).
pub const MEDIUM_SPEED: f32 = 30.0;
pub const FAST_SPEED: f32 = 100.0;
pub const MAX_SPEED: f32 = 256.0;
/// The dial's value for an overstressed network (`StressGaugeBlockEntity.updateFromNetwork`).
pub const OVERSTRESSED: f32 = 1.125;

fn lerp(delta: f32, start: f32, end: f32) -> f32 {
    start + delta * (end - start)
}

/// `SpeedGaugeBlockEntity.getDialTarget`.
pub fn speed_dial(rpm: f32) -> f32 {
    let speed = rpm.abs();
    if speed == 0.0 {
        0.0
    } else if speed < MEDIUM_SPEED {
        lerp(speed / MEDIUM_SPEED, 0.0, 0.45)
    } else if speed < FAST_SPEED {
        lerp((speed - MEDIUM_SPEED) / (FAST_SPEED - MEDIUM_SPEED), 0.45, 0.75)
    } else {
        lerp((speed - FAST_SPEED) / (MAX_SPEED - FAST_SPEED), 0.75, 1.125)
    }
}

/// `StressGaugeBlockEntity.updateFromNetwork` and `onSpeedChanged`: no rotation reads zero.
pub fn stress_dial(stress: f32, capacity: f32, overstressed: bool, rpm: f32) -> f32 {
    if overstressed {
        OVERSTRESSED
    } else if capacity == 0.0 || rpm == 0.0 {
        0.0
    } else {
        stress / capacity
    }
}

/// `GaugeBlockEntity.tick`: each tick the dial covers an eighth of the way to its target.
pub fn dial_after(from: f32, target: f32, ticks: f32) -> f32 {
    target - (target - from) * 0.875f32.powf(ticks.max(0.0))
}

/// `DirectionalAxisKineticBlock.getRotationAxis`: the axis of the shaft through a gauge.
pub fn rotation_axis(facing: Dir, along_first: bool) -> Axis {
    match (facing.axis(), along_first) {
        (Axis::X, true) => Axis::Y,
        (Axis::X, false) => Axis::Z,
        (Axis::Y, true) => Axis::X,
        (Axis::Y, false) => Axis::Z,
        (Axis::Z, true) => Axis::X,
        (Axis::Z, false) => Axis::Y,
    }
}

/// The faces `GaugeBlock.shouldRenderHeadOnFace` gives a head and a dial, leaving out the
/// check against a neighbour that covers the face.
pub fn head_faces(facing: Dir, along_first: bool) -> Vec<Dir> {
    let rotation = rotation_axis(facing, along_first);
    Dir::ALL
        .into_iter()
        .filter(|face| {
            face.axis() != Axis::Y
                && *face != facing.opposite()
                && face.axis() != rotation
                && (rotation != Axis::Y || *face == facing)
        })
        .collect()
}

/// The turn about +Y that takes a model facing east to `face` (`GaugeRenderer.rotateBufferTowards`:
/// `(-yRot - 90)` degrees): south -90, west 180, north 90.
pub fn head_turn(face: Dir) -> f32 {
    match face {
        Dir::East => 0.0,
        Dir::South => -90.0,
        Dir::West => 180.0,
        _ => 90.0,
    }
}

/// `Direction.fromYRot`: the horizontal direction a yaw faces.
pub fn horizontal(yaw: f32) -> Dir {
    match ((yaw / 90.0 + 0.5).floor() as i32).rem_euclid(4) {
        0 => Dir::South,
        1 => Dir::West,
        2 => Dir::North,
        _ => Dir::East,
    }
}

/// `Direction.getClockWise(axis)` (`Dir::clockwise_about`: X up to south, not north).
pub fn clockwise(dir: Dir, about: Axis) -> Dir {
    dir.clockwise_about(about)
}

/// A gauge's state: the face it looks out of and which way its shaft lies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placing {
    pub facing: Dir,
    pub along_first: bool,
}

/// What `GaugeBlock.getStateForPlacement` (and `DirectionalAxisKineticBlock`'s) sees of the
/// placement.
#[derive(Clone, Copy, Debug)]
pub struct Context {
    /// The face clicked.
    pub face: Dir,
    /// `getHorizontalDirection`: the way the player faces, flat.
    pub horizontal: Dir,
    /// `getNearestLookingDirection`.
    pub looking: Dir,
}

/// `GaugeBlock.getStateForPlacement`. `on_shaft` is whether the block clicked has a shaft end
/// through the face clicked; `shaft_beside(d)` whether the block on side `d` of the gauge has
/// one pointing at it (`prefersConnectionTo`).
pub fn placement(
    context: Context,
    on_shaft: bool,
    shaft_beside: impl Fn(Dir) -> bool,
) -> Placing {
    let Context {
        face,
        horizontal,
        looking,
    } = context;
    if on_shaft {
        let look_positive = looking.sign() > 0;
        return match face.axis() {
            Axis::X => Placing {
                facing: if look_positive { Dir::North } else { Dir::South },
                along_first: true,
            },
            Axis::Y => Placing {
                facing: horizontal.opposite(),
                along_first: horizontal.axis() == Axis::X,
            },
            Axis::Z => Placing {
                facing: if look_positive { Dir::West } else { Dir::East },
                along_first: false,
            },
        };
    }

    // `DirectionalAxisKineticBlock.getStateForPlacement`, whose facing is the face clicked.
    let facing = face;
    let face_axis = facing.axis();
    let along_first = if face_axis != Axis::Y {
        let flat = face_axis == Axis::Z;
        if shaft_beside(Dir::Up) || shaft_beside(Dir::Down) {
            face_axis == Axis::X
        } else {
            flat
        }
    } else {
        let mut preferred: Option<Dir> = None;
        for side in [Dir::North, Dir::East, Dir::South, Dir::West] {
            if !shaft_beside(side) {
                continue;
            }
            if preferred.is_some_and(|p| p.axis() != side.axis()) {
                preferred = None;
                break;
            }
            preferred = Some(side);
        }
        match preferred {
            Some(side) => side.axis() == Axis::X,
            None => horizontal.axis() != Axis::X,
        }
    };
    Placing {
        facing,
        along_first,
    }
}

/// `IWrenchable.getRotatedBlockState` for a gauge: a quarter turn about the clicked face's axis.
pub fn wrenched(state: Placing, face_axis: Axis) -> Placing {
    if state.facing.axis() == face_axis {
        return Placing {
            along_first: !state.along_first,
            ..state
        };
    }
    Placing {
        facing: clockwise(state.facing, face_axis),
        along_first: if face_axis == Axis::Y {
            !state.along_first
        } else {
            state.along_first
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_speed_dial_runs_in_three_stretches() {
        assert_eq!(speed_dial(0.0), 0.0);
        assert!((speed_dial(15.0) - 0.225).abs() < 1e-6);
        assert!((speed_dial(-30.0) - 0.45).abs() < 1e-6);
        assert!((speed_dial(65.0) - 0.6).abs() < 1e-6);
        assert!((speed_dial(100.0) - 0.75).abs() < 1e-6);
        assert!((speed_dial(256.0) - 1.125).abs() < 1e-6);
    }

    #[test]
    fn the_stress_dial_shows_the_share_used_and_stops_for_no_rotation() {
        assert_eq!(stress_dial(512.0, 1024.0, false, 16.0), 0.5);
        assert_eq!(stress_dial(2048.0, 1024.0, true, 0.0), OVERSTRESSED);
        assert_eq!(stress_dial(0.0, 0.0, false, 0.0), 0.0);
        assert_eq!(stress_dial(512.0, 1024.0, false, 0.0), 0.0);
    }

    #[test]
    fn the_dial_closes_an_eighth_of_the_way_each_tick() {
        assert_eq!(dial_after(0.0, 1.0, 0.0), 0.0);
        assert!((dial_after(0.0, 1.0, 1.0) - 0.125).abs() < 1e-6);
        assert!((dial_after(0.0, 1.0, 2.0) - (0.125 + 0.875 * 0.125)).abs() < 1e-6);
        assert!(dial_after(1.0, 0.0, 200.0) < 1e-6);
        assert_eq!(dial_after(0.3, 0.3, 10.0), 0.3);
    }

    #[test]
    fn the_shaft_lies_where_the_facing_and_the_flag_say() {
        assert_eq!(rotation_axis(Dir::Up, false), Axis::Z);
        assert_eq!(rotation_axis(Dir::Up, true), Axis::X);
        assert_eq!(rotation_axis(Dir::East, true), Axis::Y);
        assert_eq!(rotation_axis(Dir::East, false), Axis::Z);
        assert_eq!(rotation_axis(Dir::North, true), Axis::X);
        assert_eq!(rotation_axis(Dir::North, false), Axis::Y);
    }

    #[test]
    fn heads_sit_on_the_horizontal_faces_across_the_shaft() {
        // On the floor with its shaft along z: faces to the east and west.
        let mut faces = head_faces(Dir::Up, false);
        faces.sort();
        assert_eq!(faces, vec![Dir::West, Dir::East]);
        // On a north wall (facing north) with a vertical shaft: only the face it looks out of.
        assert_eq!(head_faces(Dir::North, false), vec![Dir::North]);
        // Facing north with its shaft along x: the north face, not the back.
        assert_eq!(head_faces(Dir::North, true), vec![Dir::North]);
        // Facing east with its shaft along z (flat): east only, never the plate.
        assert_eq!(head_faces(Dir::East, false), vec![Dir::East]);
        // On the ceiling: also faces across the shaft.
        assert_eq!(head_faces(Dir::Down, true).len(), 2);
    }

    #[test]
    fn heads_turn_from_east_as_create_does() {
        assert_eq!(head_turn(Dir::East), 0.0);
        assert_eq!(head_turn(Dir::South), -90.0);
        assert_eq!(head_turn(Dir::West), 180.0);
        assert_eq!(head_turn(Dir::North), 90.0);
    }

    #[test]
    fn a_yaw_faces_a_side() {
        assert_eq!(horizontal(0.0), Dir::South);
        assert_eq!(horizontal(90.0), Dir::West);
        assert_eq!(horizontal(180.0), Dir::North);
        assert_eq!(horizontal(-90.0), Dir::East);
        assert_eq!(horizontal(270.0), Dir::East);
        assert_eq!(horizontal(44.0), Dir::South);
        assert_eq!(horizontal(46.0), Dir::West);
    }

    fn context(face: Dir, horizontal: Dir, looking: Dir) -> Context {
        Context {
            face,
            horizontal,
            looking,
        }
    }

    #[test]
    fn a_gauge_looks_out_of_the_face_clicked() {
        let flat = |_| false;
        // On a wall facing north (a z face): along the first coordinate, shaft along x.
        let placed = placement(context(Dir::North, Dir::North, Dir::North), false, flat);
        assert_eq!(placed.facing, Dir::North);
        assert!(placed.along_first);
        // On a wall facing east: shaft along z.
        let placed = placement(context(Dir::East, Dir::East, Dir::East), false, flat);
        assert_eq!((placed.facing, placed.along_first), (Dir::East, false));
        // On the floor, the player facing east: shaft along z (the other axis).
        let placed = placement(context(Dir::Up, Dir::East, Dir::Down), false, flat);
        assert_eq!((placed.facing, placed.along_first), (Dir::Up, false));
        let placed = placement(context(Dir::Up, Dir::South, Dir::Down), false, flat);
        assert_eq!((placed.facing, placed.along_first), (Dir::Up, true));
    }

    #[test]
    fn a_gauge_lines_its_shaft_up_with_the_shafts_beside_it() {
        // A shaft above or below a wall gauge: its shaft goes vertical.
        let placed = placement(
            context(Dir::North, Dir::North, Dir::North),
            false,
            |side| side == Dir::Up,
        );
        assert_eq!(rotation_axis(placed.facing, placed.along_first), Axis::Y);
        // On the floor with a shaft to the east: the shaft lies along x.
        let placed = placement(context(Dir::Up, Dir::South, Dir::Down), false, |side| {
            side == Dir::East
        });
        assert_eq!(rotation_axis(placed.facing, placed.along_first), Axis::X);
        // Shafts on both axes: no preference.
        let placed = placement(context(Dir::Up, Dir::South, Dir::Down), false, |side| {
            side == Dir::East || side == Dir::North
        });
        assert!(placed.along_first);
    }

    #[test]
    fn a_gauge_clicked_on_a_shaft_end_takes_that_shaft() {
        // Shaft along x, gauge on its east end (a click on the east face).
        let placed = placement(context(Dir::East, Dir::East, Dir::East), true, |_| false);
        assert_eq!(rotation_axis(placed.facing, placed.along_first), Axis::X);
        // Shaft along z, clicked on its south end.
        let placed = placement(context(Dir::South, Dir::South, Dir::South), true, |_| false);
        assert_eq!(rotation_axis(placed.facing, placed.along_first), Axis::Z);
        // Shaft along y, clicked on top: the shaft stays vertical?
        let placed = placement(context(Dir::Up, Dir::West, Dir::Down), true, |_| false);
        assert_eq!(placed.facing, Dir::East);
        assert_eq!(rotation_axis(placed.facing, placed.along_first), Axis::Y);
    }

    #[test]
    fn a_wrench_turns_a_gauge_about_the_face_clicked() {
        let state = Placing {
            facing: Dir::North,
            along_first: true,
        };
        assert_eq!(
            wrenched(state, Axis::Y),
            Placing {
                facing: Dir::East,
                along_first: false
            }
        );
        // Clicked on the face it looks out of: the shaft flips.
        assert_eq!(
            wrenched(state, Axis::Z),
            Placing {
                facing: Dir::North,
                along_first: false
            }
        );
        // `getClockWiseX`: north to up (this once said down, the mirror of vanilla's).
        assert_eq!(wrenched(state, Axis::X).facing, Dir::Up);
        assert_eq!(clockwise(Dir::Up, Axis::Z), Dir::East);
    }

    #[test]
    fn a_wrench_click_on_the_x_faces_turns_up_to_south_as_vanilla_does() {
        // `IWrenchable.getRotatedBlockState` -> `Direction.getClockWise(X)` = `getClockWiseX`:
        // up, south, down, north.
        let up = Placing { facing: Dir::Up, along_first: true };
        assert_eq!(wrenched(up, Axis::X).facing, Dir::South);
        let mut d = Dir::Up;
        let seen: Vec<Dir> = (0..4)
            .map(|_| {
                d = clockwise(d, Axis::X);
                d
            })
            .collect();
        assert_eq!(seen, [Dir::South, Dir::Down, Dir::North, Dir::Up]);
        // The same table as the drill's, the wheel's and the fan's.
        for facing in Dir::ALL {
            for axis in Axis::ALL {
                assert_eq!(clockwise(facing, axis), crate::rules::drill::clockwise(facing, axis));
                if facing.axis() != axis {
                    assert_eq!(clockwise(facing, axis), crate::rules::large_water_wheel::clockwise(facing, axis));
                }
            }
        }
    }
}
