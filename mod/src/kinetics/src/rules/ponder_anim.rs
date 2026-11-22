//! Ponder's scripted block entity animations: `world.rotateBearing`, `world.movePulley` and
//! `world.moveDeployer` (Create 6.0.10, `CreateSceneBuilder.WorldInstructions` and
//! `AnimateBlockEntityInstruction`, `foundation/ponder/instruction/`). No game types.
//!
//! Each call adds an instruction that moves one number (a bearing's angle, a pulley's rope offset, a
//! deployer's hand offset) by `totalDelta` over `ticks` scene ticks. The instruction is a
//! `TickingInstruction` (a non-blocking one): it is ticked once in the tick the timeline reaches it,
//! and once in every tick after, until its `remainingTicks` runs out. At its first tick it reads the
//! number (`firstTick`: `target = getter + totalDelta`); every tick it writes `current +
//! deltaPerTick`, and on the last (`remainingTicks == 0`) it writes the target itself. A duration of
//! 0 writes the target in the very tick it starts. Several instructions in flight on one block each
//! write in turn, so the replay below walks the ticks and applies them in the order they started,
//! as the scene does.
//!
//! The block entity draws the number a tick behind the clock, interpolated by the partial tick
//! (`getInterpolatedAngle`, `animatedOffset.getValue(partialTicks)`, `Mth.lerp(partialTicks,
//! prevAnimatedOffset, offset)`: all blend the last tick's value with this tick's).

/// One `rotateBearing`, `movePulley` or `moveDeployer` call: its start tick, `totalDelta` and `ticks`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Instruction {
    pub start: u32,
    pub delta: f32,
    pub duration: u32,
}

/// The number after the scene tick `tick` has run, for instructions sorted by `start`: 0 before the
/// first of them starts.
pub fn value_after(instructions: &[Instruction], tick: i64) -> f32 {
    let Some(first) = instructions.first() else {
        return 0.0;
    };
    let mut value = 0.0_f32;
    let mut targets = vec![0.0_f32; instructions.len()];
    let mut now = i64::from(first.start);
    while now <= tick {
        let mut busy = false;
        for (index, step) in instructions.iter().enumerate() {
            let start = i64::from(step.start);
            if start > now {
                break;
            }
            let ticks = i64::from(step.duration);
            // The tick it is on: 1 at its first tick.
            let k = now - start + 1;
            if k > ticks.max(1) {
                continue;
            }
            busy = true;
            if now == start {
                targets[index] = value + step.delta;
            }
            let per_tick = f64::from(step.delta) * (1.0 / ticks.max(1) as f64);
            value = if ticks == 0 || k >= ticks {
                targets[index]
            } else {
                (f64::from(value) + per_tick) as f32
            };
        }
        if busy {
            now += 1;
        } else {
            // Nothing in flight: jump to the next instruction's start (or past the end).
            now = instructions
                .iter()
                .map(|step| i64::from(step.start))
                .find(|&start| start > now)
                .unwrap_or(tick + 1);
        }
    }
    value
}

/// What the block draws at the (fractional) scene time `time`: the value after the previous tick
/// blended towards the value after this one by the partial tick.
pub fn value_at(instructions: &[Instruction], time: f32) -> f32 {
    let tick = time.floor();
    let partial = time - tick;
    let tick = tick as i64;
    let previous = value_after(instructions, tick - 1);
    let current = value_after(instructions, tick);
    (current - previous).mul_add(partial, previous)
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn go(start: u32, delta: f32, duration: u32) -> Instruction {
        Instruction { start, delta, duration }
    }

    #[test]
    fn it_moves_by_delta_over_its_ticks_and_lands_exactly_on_the_target() {
        // `rotateBearing(pos, 360, 200)`: the first step is taken in the tick it starts.
        let spin = [go(10, 360.0, 200)];
        assert_eq!(value_after(&spin, 9), 0.0);
        assert!((value_after(&spin, 10) - 1.8).abs() < 1e-4);
        assert!((value_after(&spin, 109) - 180.0).abs() < 1e-3);
        assert_eq!(value_after(&spin, 209), 360.0);
        assert_eq!(value_after(&spin, 5000), 360.0);
    }

    #[test]
    fn a_duration_of_zero_sets_it_at_once() {
        // `rotateBearing(windmill, -45, 0)`.
        assert_eq!(value_after(&[go(7, -45.0, 0)], 6), 0.0);
        assert_eq!(value_after(&[go(7, -45.0, 0)], 7), -45.0);
    }

    #[test]
    fn a_later_instruction_starts_from_where_the_earlier_one_left_off() {
        // `moveDeployer(pos, 1, 25)` then `moveDeployer(pos, -1, 25)`.
        let stroke = [go(0, 1.0, 25), go(26, -1.0, 25)];
        assert_eq!(value_after(&stroke, 24), 1.0);
        assert_eq!(value_after(&stroke, 25), 1.0);
        assert!(value_after(&stroke, 30) < 1.0 && value_after(&stroke, 30) > 0.0);
        assert_eq!(value_after(&stroke, 50), 0.0);
    }

    #[test]
    fn instructions_in_flight_together_each_write_in_turn() {
        let both = [go(0, 10.0, 10), go(0, 10.0, 10)];
        // The second reads the number after the first has stepped (1), so its target is 11, and
        // its last write is the one that stays.
        assert_eq!(value_after(&both, 9), 11.0);
    }

    #[test]
    fn the_drawn_value_blends_the_last_tick_with_this_one() {
        let spin = [go(0, 100.0, 100)];
        let a = value_after(&spin, 19);
        let b = value_after(&spin, 20);
        assert!((value_at(&spin, 20.5) - (a + b) * 0.5).abs() < 1e-3);
        assert_eq!(value_at(&spin, 0.0), 0.0);
    }
}
