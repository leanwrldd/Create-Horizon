//! The Large Water Wheel's rules: the twelve blocks it takes the current from, the speed it makes,
//! the eight structure blocks it fills around itself and where its master is. Ported from Create
//! `mc1.21.1/dev` at 4148fd8 (MIT, the Creators of Create): `WaterWheelBlockEntity`
//! (`LARGE_OFFSETS`, `determineAndApplyFlowScore`, `getGeneratedSpeed`),
//! `LargeWaterWheelBlockEntity.getSize`, `LargeWaterWheelBlock` (`tick`, `getStateForPlacement`),
//! `WaterWheelStructuralBlock` (`getMaster`, `stillValid`) and `AllBlocks.LARGE_WATER_WHEEL`
//! (`CStress.setCapacity(128.0)`, `setGeneratorSpeed(4)`). No game types.

use crate::{Axis, Dir, Pos};

/// Stress units per RPM (`AllBlocks.LARGE_WATER_WHEEL`: `CStress.setCapacity(128.0)`).
pub const CAPACITY: f32 = 128.0;
/// `LargeWaterWheelBlockEntity.getSize()`.
pub const SIZE: i32 = 2;

/// RPM the wheel makes for a flow score: `Mth.clamp(flowScore, -1, 1) * 8 / getSize()`, half the
/// small wheel's.
pub fn speed(score: i32) -> f32 {
    score.clamp(-1, 1) as f32 * 8.0 / SIZE as f32
}

/// `LARGE_OFFSETS` for `axis`: for each side not on the axis, the block two away and the two
/// blocks beside that one in the wheel's plane. Twelve blocks, in Create's order.
pub fn offsets(axis: Axis) -> Vec<Pos> {
    let mut out: Vec<Pos> = Vec::with_capacity(12);
    for d in Dir::ALL {
        if d.axis() == axis {
            continue;
        }
        let central = d.normal().map(|v| v * 2);
        let push = |out: &mut Vec<Pos>, p: Pos| {
            if !out.contains(&p) {
                out.push(p);
            }
        };
        push(&mut out, central);
        for d2 in Dir::ALL {
            if d2.axis() == axis || d2.axis() == d.axis() {
                continue;
            }
            let n = d2.normal();
            push(&mut out, [central[0] + n[0], central[1] + n[1], central[2] + n[2]]);
        }
    }
    out
}

/// `VecHelper.rotate(normal, 90, axis)`: where the rim moves at that offset when the wheel turns
/// the positive way. X: (x, -z, y); Y: (z, y, -x); Z: (-y, x, z).
fn rim(n: [f64; 3], axis: Axis) -> [f64; 3] {
    match axis {
        Axis::X => [n[0], -n[2], n[1]],
        Axis::Y => [n[2], n[1], -n[0]],
        Axis::Z => [-n[1], n[0], n[2]],
    }
}

/// Create's flow score (`determineAndApplyFlowScore`) over the twelve blocks: each whose current,
/// cut to the wheel's plane, runs along the rim counts 1 for the positive way round and -1 for the
/// other. Unlike the small wheel's, an offset is a normalised direction, not a unit step.
pub fn score(axis: Axis, mut flow: impl FnMut(Pos) -> [f64; 3]) -> i32 {
    let mut score = 0;
    for offset in offsets(axis) {
        let mut current = flow(offset);
        current[axis.index()] = 0.0;
        let length = current.iter().map(|v| v * v).sum::<f64>().sqrt();
        if length == 0.0 {
            continue;
        }
        let n = offset.map(f64::from);
        let nl = n.iter().map(|v| v * v).sum::<f64>().sqrt();
        let normal = [n[0] / nl, n[1] / nl, n[2] / nl];
        let motion = rim(normal, axis);
        let dot: f64 = (0..3).map(|i| current[i] / length * motion[i]).sum();
        if dot.abs() > 0.5 {
            score += dot.signum() as i32;
        }
    }
    score
}

/// The eight blocks the wheel fills around itself (`LargeWaterWheelBlock.tick`): for each side
/// not on the axis, the block beside the master and the block beside the one clockwise of it
/// (`secondary`), each with the way it faces back at the master (`targetSide.getOpposite()`, the
/// `facing` of `water_wheel_structure`).
pub fn structure(axis: Axis) -> Vec<(Pos, Dir)> {
    let mut out = Vec::with_capacity(8);
    for side in Dir::ALL {
        if side.axis() == axis {
            continue;
        }
        for secondary in [false, true] {
            let target = if secondary { clockwise(side, axis) } else { side };
            let base = if secondary { side.normal() } else { [0; 3] };
            let n = target.normal();
            out.push(([base[0] + n[0], base[1] + n[1], base[2] + n[2]], target.opposite()));
        }
    }
    out
}

/// `Direction.getClockWise(axis)`: turning `side` a quarter clockwise seen from the positive end of
/// `axis`.
pub fn clockwise(side: Dir, axis: Axis) -> Dir {
    // Seen from +Y looking down: north -> east -> south -> west. From +X: up -> south -> down ->
    // north. From +Z: up -> east -> down -> west (`Dir::clockwise_about`). A side along the axis
    // never turns here (the wheel skips those).
    side.clockwise_about(axis)
}

/// The nine blocks a wheel at the origin fills, apart from the master (the placement's `canBeReplaced`
/// scan): every offset in the plane of the axis within one block.
pub fn footprint(axis: Axis) -> Vec<Pos> {
    let mut out = Vec::with_capacity(8);
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                let p = [x, y, z];
                if p[axis.index()] != 0 || p == [0, 0, 0] {
                    continue;
                }
                out.push(p);
            }
        }
    }
    out
}

/// `WaterWheelStructuralBlock.getMaster`, one step: the block a structure block faces.
pub fn master_step(pos: Pos, facing: Dir) -> Pos {
    let n = facing.normal();
    [pos[0] + n[0], pos[1] + n[1], pos[2] + n[2]]
}

/// `WaterWheelStructuralBlock.stillValid` for a structure block whose target is a wheel of
/// `target_axis`: the target must be a large wheel turning about an axis other than the one the
/// structure block faces along.
pub fn faces_wheel(facing: Dir, target_axis: Axis) -> bool {
    target_axis != facing.axis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_makes_half_the_small_wheels_speed() {
        assert_eq!(speed(1), 4.0);
        assert_eq!(speed(-3), -4.0);
        assert_eq!(speed(0), 0.0);
    }

    #[test]
    fn twelve_blocks_ring_the_wheel_in_its_plane() {
        for axis in Axis::ALL {
            let around = offsets(axis);
            assert_eq!(around.len(), 12, "{axis:?}");
            assert!(around.iter().all(|p| p[axis.index()] == 0));
            // Every one is two out in one direction and at most one across in the other.
            for p in &around {
                let along: Vec<i32> = (0..3).filter(|i| *i != axis.index()).map(|i| p[i].abs()).collect();
                assert!(along.contains(&2) && along.iter().all(|v| *v <= 2), "{p:?}");
            }
        }
    }

    #[test]
    fn water_falling_beside_the_rim_turns_it() {
        // Axle along x, falling water at the south (+z) side, two out: the rim there moves down
        // when the wheel turns the positive way about +x.
        let south = |p: Pos| if p == [0, 0, 2] { [0.0, -1.0, 0.0] } else { [0.0; 3] };
        assert_eq!(score(Axis::X, south), 1);
        let north = |p: Pos| if p == [0, 0, -2] { [0.0, -1.0, 0.0] } else { [0.0; 3] };
        assert_eq!(score(Axis::X, north), -1);
        assert_eq!(speed(score(Axis::X, north)), -4.0);
    }

    #[test]
    fn a_corner_block_counts_by_its_diagonal_direction() {
        // The corner (0, 1, 2): its normal is (0, 1, 2) / sqrt(5), the rim there moves along
        // (0, -2, 1) / sqrt(5) for the positive way. Straight down is only 0.89 along it: counts.
        let corner = |p: Pos| if p == [0, 1, 2] { [0.0, -1.0, 0.0] } else { [0.0; 3] };
        assert_eq!(score(Axis::X, corner), 1);
        // A current along the diagonal's own normal pushes no rim.
        let radial = |p: Pos| if p == [0, 1, 2] { [0.0, 1.0, 2.0] } else { [0.0; 3] };
        assert_eq!(score(Axis::X, radial), 0);
    }

    #[test]
    fn still_water_or_water_along_the_axle_does_nothing() {
        assert_eq!(score(Axis::Y, |_| [0.0; 3]), 0);
        assert_eq!(score(Axis::Y, |_| [0.0, 1.0, 0.0]), 0);
    }

    #[test]
    fn the_structure_fills_the_eight_blocks_round_the_master() {
        for axis in Axis::ALL {
            let blocks = structure(axis);
            assert_eq!(blocks.len(), 8);
            let mut at: Vec<Pos> = blocks.iter().map(|(p, _)| *p).collect();
            let mut want = footprint(axis);
            at.sort();
            want.sort();
            assert_eq!(at, want, "{axis:?}");
            // Each faces back: a step along its facing leads towards the master, directly or
            // through the block beside it.
            for (pos, facing) in &blocks {
                let next = master_step(*pos, *facing);
                assert!(next == [0, 0, 0] || blocks.iter().any(|(p, _)| *p == next), "{pos:?} {facing:?}");
            }
        }
    }

    #[test]
    fn clockwise_is_the_next_side_seen_from_the_positive_end() {
        assert_eq!(clockwise(Dir::North, Axis::Y), Dir::East);
        assert_eq!(clockwise(Dir::West, Axis::Y), Dir::North);
        assert_eq!(clockwise(Dir::Up, Axis::X), Dir::South);
        assert_eq!(clockwise(Dir::Up, Axis::Z), Dir::East);
        // Four turns come back.
        let mut d = Dir::Up;
        for _ in 0..4 {
            d = clockwise(d, Axis::X);
        }
        assert_eq!(d, Dir::Up);
    }

    #[test]
    fn a_structure_block_is_valid_only_across_the_wheels_axis() {
        assert!(faces_wheel(Dir::East, Axis::Y));
        assert!(!faces_wheel(Dir::East, Axis::X));
    }
}
