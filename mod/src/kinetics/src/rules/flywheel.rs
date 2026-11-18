//! The Flywheel's numbers (`FlywheelBlockEntity`, `FlywheelRenderer`): a decorative wheel that
//! turns with its shaft and carries on after the power goes, as a heavy wheel does.
//!
//! Create keeps a client-side `visualSpeed` (a `LerpedFloat`) that chases the block's speed
//! each tick (`Chaser.EXP` at 1/64: it closes 1/64 of the gap) and an angle that adds
//! `visualSpeed * 3 / 10` degrees a tick (`visualSpeed.tickChaser(); angle += ...`). A client
//! here can't tick a block entity, so the same two sums are written in closed form from where
//! they stood when the target speed last changed (a [`Wheel`], saved by the server and sent to
//! clients): every viewer and the server agree at any game time, and nothing is written between
//! speed changes.
//!
//! The Flywheel has no stress impact (`CStress.setNoImpact`) and no capacity: it only draws.
//!
//! Known differences from Create, for the README:
//! - Create's chase starts from the speed the client first read (`getGeneratedSpeed()`, 0 here),
//!   so a wheel that was already turning when a client arrived starts from a stand still in
//!   Create and from where it really is here.
//! - The part of the turn between two ticks is the next tick's step taken in proportion, where
//!   Create interpolates `visualSpeed` itself: the same to the eye.

/// `Chaser.EXP` speed in `FlywheelBlockEntity.read` / `tick`: `1 / 64` of the gap each tick.
pub const CHASE: f64 = 1.0 / 64.0;
/// `visualSpeed * 3 / 10` degrees a tick, as every Create shaft turns (`rpm * 0.3`).
pub const DEGREES_PER_RPM_TICK: f64 = 3.0 / 10.0;
/// The gap to the target below which the wheel counts as at speed. The remaining difference
/// is under 0.01 RPM, a turn of 0.003 degrees a tick.
pub const SETTLED: f64 = 0.01;

/// The wheel's visual state at one moment, and the speed it is chasing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wheel {
    /// `visualSpeed` at game time `since`, in RPM.
    pub speed: f32,
    /// The angle at `since`, in degrees.
    pub angle: f32,
    /// The game time the two above were true at.
    pub since: i64,
    /// The speed it chases: the block's own, in RPM (`getSpeed()`).
    pub target: f32,
}

impl Default for Wheel {
    fn default() -> Self {
        Wheel {
            speed: 0.0,
            angle: 0.0,
            since: 0,
            target: 0.0,
        }
    }
}

impl Wheel {
    /// The visual speed after `ticks` whole ticks: `v += (target - v) / 64` each.
    pub fn speed_after(self, ticks: f64) -> f64 {
        let target = f64::from(self.target);
        let q = 1.0 - CHASE;
        target + (f64::from(self.speed) - target) * q.powf(ticks.max(0.0))
    }

    /// The angle after `ticks` whole ticks: the sum of the visual speeds of each (the speed is
    /// stepped before the angle in `FlywheelBlockEntity.tick`), in degrees, kept in 0 to 360.
    pub fn angle_after(self, ticks: f64) -> f64 {
        let ticks = ticks.max(0.0);
        let target = f64::from(self.target);
        let gap = f64::from(self.speed) - target;
        let q = 1.0 - CHASE;
        // sum_{k=1..n} (target + gap * q^k)
        let sum = target * ticks + gap * q * (1.0 - q.powf(ticks)) / (1.0 - q);
        (f64::from(self.angle) + sum * DEGREES_PER_RPM_TICK).rem_euclid(360.0)
    }

    /// The speed and angle `now - since` ticks on, with the part of a tick taken in proportion:
    /// what a frame draws (`partial` is the part of the tick past, 0 to 1).
    pub fn frame(self, now: i64, partial: f32) -> (f32, f32) {
        let whole = (now - self.since).max(0) as f64;
        let speed = self.speed_after(whole);
        let next = self.speed_after(whole + 1.0);
        let angle = self.angle_after(whole) + f64::from(partial) * next * DEGREES_PER_RPM_TICK;
        (speed as f32, angle.rem_euclid(360.0) as f32)
    }

    /// The same wheel chasing `target` from game time `now`: where it stood then is kept.
    pub fn retarget(self, now: i64, target: f32) -> Wheel {
        let whole = (now - self.since).max(0) as f64;
        Wheel {
            speed: self.speed_after(whole) as f32,
            angle: self.angle_after(whole) as f32,
            since: now,
            target,
        }
    }

    /// Ticks after `since` when the visual speed is within [`SETTLED`] of the target.
    pub fn settles_after(self) -> i64 {
        let gap = (f64::from(self.speed) - f64::from(self.target)).abs();
        if gap <= SETTLED {
            return 0;
        }
        ((SETTLED / gap).ln() / (1.0 - CHASE).ln()).ceil() as i64
    }

    /// Whether the wheel is at its target speed at game time `now`, turning steadily.
    pub fn is_settled(self, now: i64) -> bool {
        now - self.since >= self.settles_after()
    }

    /// For a steady turn once settled: the speed in degrees a tick and the phase that
    /// `BlockEntityDraw::spin` needs (`angle = game_time * degrees + phase`).
    pub fn steady(self) -> (f32, f32) {
        let n = self.settles_after();
        let angle = self.angle_after(n as f64);
        let degrees = f64::from(self.target) * DEGREES_PER_RPM_TICK;
        let at = (self.since + n) as f64;
        // Kept small: the game time is large and the angle is only needed modulo 360.
        let phase = (angle - degrees * at).rem_euclid(360.0);
        (degrees as f32, phase as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_closes_a_sixty_fourth_of_the_gap_each_tick() {
        let wheel = Wheel {
            target: 64.0,
            ..Wheel::default()
        };
        assert!((wheel.speed_after(0.0) - 0.0).abs() < 1e-9);
        assert!((wheel.speed_after(1.0) - 1.0).abs() < 1e-9);
        // 63/64 of the gap is left after one tick, so after two: 64 * (1 - (63/64)^2).
        let two = 64.0 * (1.0 - (63.0f64 / 64.0).powi(2));
        assert!((wheel.speed_after(2.0) - two).abs() < 1e-9);
    }

    #[test]
    fn the_angle_adds_the_speed_each_tick_as_create_does() {
        // Step it the way FlywheelBlockEntity.tick does and compare with the closed form.
        let wheel = Wheel {
            speed: 10.0,
            angle: 30.0,
            since: 100,
            target: -40.0,
        };
        let (mut v, mut angle) = (10.0f64, 30.0f64);
        for tick in 1..=200 {
            v += (-40.0 - v) / 64.0;
            angle = (angle + v * 3.0 / 10.0) % 360.0;
            if angle < 0.0 {
                angle += 360.0;
            }
            if tick % 40 == 0 {
                assert!((wheel.speed_after(f64::from(tick)) - v).abs() < 1e-6, "{tick}");
                let got = wheel.angle_after(f64::from(tick));
                let diff = (got - angle).abs();
                assert!(diff.min(360.0 - diff) < 1e-6, "{tick}: {got} vs {angle}");
            }
        }
    }

    #[test]
    fn it_keeps_turning_after_the_power_goes() {
        let running = Wheel {
            speed: 128.0,
            angle: 0.0,
            since: 0,
            target: 128.0,
        };
        let stopped = running.retarget(1000, 0.0);
        assert_eq!(stopped.since, 1000);
        assert!((f64::from(stopped.speed) - 128.0).abs() < 1e-3);
        // Half the speed is left after 64 * ln 2 ticks, about 44.
        let (speed, _) = stopped.frame(1044, 0.0);
        assert!(speed > 60.0 && speed < 70.0, "{speed}");
        // And it has to coast a long way before it is still.
        assert!(!stopped.is_settled(1300));
        assert!(stopped.is_settled(1000 + stopped.settles_after()));
    }

    #[test]
    fn retargeting_keeps_the_angle_continuous() {
        let wheel = Wheel {
            speed: 20.0,
            angle: 5.0,
            since: 0,
            target: 80.0,
        };
        let before = wheel.angle_after(77.0);
        let later = wheel.retarget(77, -30.0);
        assert!((f64::from(later.angle) - before).abs() < 1e-3);
        assert!((f64::from(later.speed) - wheel.speed_after(77.0)).abs() < 1e-3);
    }

    #[test]
    fn a_steady_wheel_turns_at_its_speed() {
        let wheel = Wheel {
            speed: 64.0,
            angle: 12.0,
            since: 50,
            target: 64.0,
        };
        assert_eq!(wheel.settles_after(), 0);
        let (degrees, phase) = wheel.steady();
        assert!((degrees - 19.2).abs() < 1e-4);
        // spin: angle(t) = t * degrees + phase must agree with the closed form at t = 50.
        let spin = (50.0 * f64::from(degrees) + f64::from(phase)).rem_euclid(360.0);
        assert!((spin - 12.0).abs() < 1e-2, "{spin}");
    }

    #[test]
    fn a_frame_between_ticks_lands_between_them() {
        let wheel = Wheel {
            speed: 0.0,
            angle: 0.0,
            since: 0,
            target: 100.0,
        };
        let (_, here) = wheel.frame(10, 0.0);
        let (_, half) = wheel.frame(10, 0.5);
        let (_, next) = wheel.frame(11, 0.0);
        assert!(here < half && half < next);
    }
}
