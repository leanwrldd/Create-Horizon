//! The Mechanical Arm in a Ponder scene: `world.instructArm(armPos, phase, heldItem, targetedPoint)` (Create 6.0.10,
//! `CreateSceneBuilder.WorldInstructions.instructArm`). No game types.
//!
//! The call rewrites the arm's saved data (`Phase`, `HeldItem`, `TargetPointIndex`, and `MovementProgress` back to 0)
//! and the scene's arm, a client block entity, goes on ticking: `ArmBlockEntity.read` (with a client packet) notes the
//! point it was at as `previousTarget` when the phase or the index changed, and `tickMovementProgress` moves the
//! progress by `min(256, |speed|) / 1024` a tick and sets the angles from it. Both are in [`super::arm`]
//! (`Motion::after`, `progress_at`, `pose`): this replays a scene's instructions through them. Nothing else of the
//! arm's work (picking items up and putting them down) runs on a client, so the scene's instructions are the only
//! thing that moves it.

use super::arm::{Angles, Motion, Phase, pose, progress_at, render_angles};
use super::arm_points::{Point, point_angles, split};
use crate::{Dir, Pos};

/// One `instructArm`: the tick it runs, the phase and the index written, and the arm's speed at that tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Instruction {
    pub at: i64,
    pub phase: Phase,
    pub index: i32,
    pub speed: f32,
}

/// The arm and what it can reach.
pub struct Arm<'a> {
    pub pos: Pos,
    pub ceiling: bool,
    pub points: &'a [Point],
    /// The `facing` of the block at a point (the Deployer and the funnels turn the claw by it).
    pub facing: &'a dyn Fn(Pos) -> Option<Dir>,
}

impl Arm<'_> {
    /// `getTargetedInteractionPoint`: an input in `MOVE_TO_INPUT`, an output in `MOVE_TO_OUTPUT`.
    fn targeted(&self, phase: Phase, index: i32) -> Option<Angles> {
        let (inputs, outputs) = split(self.points);
        let list = match phase {
            Phase::MoveToInput => inputs,
            Phase::MoveToOutput => outputs,
            _ => return None,
        };
        let point = self.points[*list.get(usize::try_from(index).ok()?)?];
        Some(point_angles(point.ty, point.pos, (self.facing)(point.pos), self.pos, self.ceiling))
    }

    /// The four angles `ArmRenderer` takes at scene time `now`, for instructions in the order they run.
    pub fn angles(&self, instructions: &[Instruction], now: f32) -> [f32; 4] {
        let mut motion = Motion::REST;
        let (mut phase, mut index) = (Phase::SearchInputs, -1);
        let (mut since, mut speed) = (0, 0.0);
        for step in instructions.iter().take_while(|step| step.at as f32 <= now) {
            let left = self.targeted(phase, index);
            let target = self.targeted(step.phase, step.index);
            motion = if (step.phase, step.index) == (phase, index) {
                Motion { target, ..motion }
            } else {
                motion.after(left, target)
            };
            (phase, index, since, speed) = (step.phase, step.index, step.at, step.speed);
        }
        let whole = now.floor();
        let progress = progress_at(0.0, since, speed, whole as i64, now - whole);
        render_angles(pose(progress, &motion))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::arm_points::{Mode, PointType};

    const ARM: Pos = [2, 1, 2];

    fn depot(pos: Pos, mode: Mode) -> Point {
        Point { ty: PointType::Depot, pos, mode }
    }

    fn move_to(at: i64, phase: Phase, index: i32) -> Instruction {
        Instruction { at, phase, index, speed: 256.0 }
    }

    fn angles(points: &[Point], steps: &[Instruction], now: f32) -> [f32; 4] {
        Arm { pos: ARM, ceiling: false, points, facing: &|_| None }.angles(steps, now)
    }

    #[test]
    fn an_arm_nobody_instructed_rests_folded() {
        assert_eq!(angles(&[], &[], 50.0), render_angles(pose(0.0, &Motion::REST)));
    }

    #[test]
    fn moving_to_an_input_ends_at_the_points_angles_after_four_ticks_at_256_rpm() {
        let points = [depot([4, 0, 2], Mode::Take), depot([0, 0, 2], Mode::Deposit)];
        let steps = [move_to(10, Phase::MoveToInput, 0)];
        let target = point_angles(PointType::Depot, [4, 0, 2], None, ARM, false);
        // 256 RPM is 0.25 of the way a tick: there at tick 14.
        assert_eq!(angles(&points, &steps, 14.0), render_angles(pose(1.0, &Motion { target: Some(target), ..Motion::REST })));
        // Halfway the arm has folded back to rest.
        let half = angles(&points, &steps, 12.0);
        assert!((half[1] - (135.0 - 135.0)).abs() < 1e-3);
    }

    #[test]
    fn the_next_move_starts_from_the_point_the_arm_left() {
        let points = [depot([4, 0, 2], Mode::Take), depot([0, 0, 2], Mode::Deposit)];
        let steps = [
            move_to(0, Phase::MoveToInput, 0),
            Instruction { at: 20, phase: Phase::SearchOutputs, index: -1, speed: 256.0 },
            move_to(30, Phase::MoveToOutput, 0),
        ];
        let first = point_angles(PointType::Depot, [4, 0, 2], None, ARM, false);
        let second = point_angles(PointType::Depot, [0, 0, 2], None, ARM, false);
        // Just after the second move starts the base is still at the first point.
        assert!((angles(&points, &steps, 30.0)[0] - first.base).abs() < 1e-3);
        // And it ends at the second.
        assert!((angles(&points, &steps, 40.0)[0] - second.base).abs() < 1e-3);
    }

    #[test]
    fn an_index_with_no_point_is_no_target() {
        let steps = [move_to(0, Phase::MoveToInput, 3)];
        assert_eq!(angles(&[], &steps, 20.0), render_angles(pose(1.0, &Motion::REST)));
    }
}
