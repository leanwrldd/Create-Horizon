//! The Mechanical Mixer's cycle: the head comes down, turns while the basin's recipe runs, and goes
//! back up. Ported from Create's `MechanicalMixerBlockEntity`, `MechanicalMixerBlock`,
//! `BasinOperatingBlockEntity` and `MechanicalMixerRenderer` (MIT, the Creators of Create),
//! checked against `mc1.21.1/dev` at 4148fd8.
//!
//! No game types: the glue (`native/api/src/machines/mixer.rs`) looks the recipe up and applies it.

/// Stress units per RPM (`AllBlocks.MECHANICAL_MIXER`: `CStress.setImpact(4.0)`).
pub const IMPACT: f32 = 4.0;
/// `getMinimumRequiredSpeedLevel` is `MEDIUM`, whose speed is `CKinetics.mediumSpeed`, 30 RPM.
pub const MIN_SPEED: f32 = 30.0;
/// The head's whole travel takes 40 ticks: 20 down, 20 up.
pub const CYCLE: i32 = 40;
/// The tick at the bottom, where the recipe runs.
pub const BOTTOM: i32 = 20;
/// How many ticks apart the glue looks at the basin's contents when it is idle: Create is told by
/// the basin (`contentsChanged`) at once, which the glue approximates with this poll.
pub const POLL: i64 = 5;

/// `MechanicalMixerBlockEntity.tickAudio`: while the head sits at the bottom (`runningTicks == 20`) the `MIXING`
/// entry plays at 0.75 every tick, or every other tick (the odd ones) below 65 RPM. `ticks` is the game time.
pub fn mixing_sound_now(rpm: f32, at_bottom: bool, ticks: i64) -> bool {
    let slow = rpm.abs() < 65.0;
    if slow && ticks.rem_euclid(2) == 0 {
        return false;
    }
    at_bottom
}

/// `AllSoundEvents.MIXING.playAt(.., .75f, 1, ..)`: the volume it plays at.
pub const MIXING_VOLUME: f32 = 0.75;

/// `isSpeedRequirementFulfilled`: `abs(speed) >= MEDIUM`.
pub fn speed_fulfilled(rpm: f32) -> bool {
    rpm.abs() >= MIN_SPEED
}

/// `Mth.log2(int)`: the floor of the base 2 logarithm (-1 for 0, as Mojang's code answers).
pub fn log2(value: i32) -> i32 {
    if value <= 0 {
        -1
    } else {
        31 - value.leading_zeros() as i32
    }
}

/// The ticks the recipe runs for at the bottom, from `speed` (RPM, absolute) and the recipe's
/// `processing_time` (0 for none): `max(log2((int) (512 / speed)) * ceil(t / 100 * 15) + 1, 1)`.
pub fn processing_ticks(speed: f32, duration: i32) -> i32 {
    let recipe_speed = if duration != 0 {
        duration as f32 / 100.0
    } else {
        1.0
    };
    let steps = (recipe_speed * 15.0).ceil() as i32;
    (log2((512.0 / speed) as i32) * steps + 1).max(1)
}

/// What a tick of the mixer asks of the glue.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tick {
    /// `basinChecker.scheduleUpdate()`: the cycle ended, look at the basin again.
    pub recheck: bool,
    /// The recipe's time ran out: apply it (`applyBasinRecipe`) and tell clients.
    pub apply: bool,
    /// The recipe's time was just worked out: the basin has liquid in it, so the whirlpool sound
    /// plays (`BUBBLE_COLUMN_WHIRLPOOL_AMBIENT`).
    pub started: bool,
}

/// The mixer block entity's cycle state: `running`, `runningTicks` (saved as `Running` and
/// `Ticks`) and `processingTicks`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mixer {
    pub running: bool,
    pub running_ticks: i32,
    /// Counts down at the bottom; negative while it has not been worked out.
    pub processing_ticks: i32,
}

impl Default for Mixer {
    fn default() -> Self {
        Mixer {
            running: false,
            running_ticks: 0,
            processing_ticks: 0,
        }
    }
}

impl Mixer {
    /// `startProcessingBasin`: a cycle that is still on its way down is not restarted.
    pub fn start(&mut self) {
        if self.running && self.running_ticks <= BOTTOM {
            return;
        }
        self.running = true;
        self.running_ticks = 0;
    }

    /// `continueWithPreviousRecipe`: the recipe still matches after it ran, so the head stays down.
    pub fn continue_recipe(&mut self) {
        self.running_ticks = BOTTOM;
    }

    /// `onBasinRemoved`: the head goes back up.
    pub fn basin_removed(&mut self) {
        if !self.running {
            return;
        }
        self.running_ticks = CYCLE;
        self.running = false;
    }

    /// `MechanicalMixerBlockEntity.tick` on the server. `rpm` is the signed speed and
    /// `duration` the current recipe's `processing_time` (0 for none).
    pub fn tick(&mut self, rpm: f32, duration: i32) -> Tick {
        let mut out = Tick::default();
        if self.running_ticks >= CYCLE {
            self.running = false;
            self.running_ticks = 0;
            out.recheck = true;
            return out;
        }
        let speed = rpm.abs();
        if !self.running {
            return out;
        }
        if speed == 0.0 || !speed_fulfilled(rpm) {
            if self.running_ticks < BOTTOM {
                self.running_ticks = CYCLE - self.running_ticks;
            } else if self.running_ticks == BOTTOM {
                self.running_ticks += 1;
            }
        }
        if self.running_ticks == BOTTOM {
            if self.processing_ticks < 0 {
                self.processing_ticks = processing_ticks(speed, duration);
                out.started = true;
            } else {
                self.processing_ticks -= 1;
                if self.processing_ticks == 0 {
                    self.running_ticks += 1;
                    self.processing_ticks = -1;
                    out.apply = true;
                }
            }
        }
        if self.running_ticks != BOTTOM {
            self.running_ticks += 1;
        }
        out
    }
}

/// `getRenderedHeadOffset`, in blocks, from the running ticks and the partial tick.
pub fn head_offset(running: bool, running_ticks: i32, partial: f32) -> f32 {
    let mut offset = 0.0;
    if running {
        let smooth = |local: f32| {
            let num = local / 20.0;
            (2.0 - (num * std::f32::consts::PI).cos()) / 2.0 - 0.5
        };
        if running_ticks < BOTTOM {
            offset = smooth(running_ticks as f32 + partial);
        } else if running_ticks <= BOTTOM {
            offset = 1.0;
        } else {
            offset = smooth((CYCLE - running_ticks) as f32 - partial);
        }
    }
    offset + 7.0 / 16.0
}

/// `getRenderedHeadRotationSpeed`: the head turns at the shaft's speed, twice as fast for the
/// last five ticks of the descent and the bottom, and at half speed when idle.
pub fn head_rotation_speed(running: bool, running_ticks: i32, speed: f32) -> f32 {
    if running {
        if running_ticks < 15 {
            return speed;
        }
        if running_ticks <= BOTTOM {
            return speed * 2.0;
        }
        return speed;
    }
    speed / 2.0
}

/// The running ticks a client shows at game time `now` (ticks plus the partial tick), given the
/// pair the server last sent: `ticks` at game time `since`. The head waits at the bottom while the
/// server works (a client's own count stops at 20, as Create's does) and stops at the top.
pub fn client_ticks(ticks: i32, since: i64, now: f64) -> f32 {
    let cap = if ticks <= BOTTOM { BOTTOM } else { CYCLE } as f32;
    let start = (ticks as f32).min(cap);
    (((ticks as f64 + (now - since as f64)) as f32).max(start)).min(cap)
}

#[cfg(test)]
mod sound_tests {
    use super::*;

    #[test]
    fn the_mixing_sound_plays_at_the_bottom_every_tick_when_fast_and_every_other_when_slow() {
        assert!(mixing_sound_now(128.0, true, 10));
        assert!(mixing_sound_now(128.0, true, 11));
        assert!(!mixing_sound_now(32.0, true, 10));
        assert!(mixing_sound_now(32.0, true, 11));
        assert!(!mixing_sound_now(128.0, false, 11));
        assert!(!mixing_sound_now(-32.0, true, 12));
        assert!(mixing_sound_now(-64.0, true, 13));
        assert!(mixing_sound_now(-65.0, true, 14));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log2_floors() {
        assert_eq!(log2(1), 0);
        assert_eq!(log2(2), 1);
        assert_eq!(log2(3), 1);
        assert_eq!(log2(32), 5);
        assert_eq!(log2(0), -1);
    }

    #[test]
    fn processing_time_falls_with_speed() {
        // 100 ticks of recipe time is a factor of one: log2(512 / speed) * 15 + 1.
        assert_eq!(processing_ticks(32.0, 100), 4 * 15 + 1);
        assert_eq!(processing_ticks(256.0, 100), 15 + 1);
        assert_eq!(processing_ticks(64.0, 0), 3 * 15 + 1);
        // A longer recipe takes proportionally more steps: ceil(2 * 15) = 30.
        assert_eq!(processing_ticks(128.0, 200), 2 * 30 + 1);
        assert_eq!(processing_ticks(30.0, 50), 4 * 8 + 1); // 512 / 30 = 17, log2 = 4
        assert_eq!(processing_ticks(1.0e9, 100), 1);
    }

    #[test]
    fn speed_requirement_is_the_medium_speed() {
        assert!(!speed_fulfilled(29.9));
        assert!(speed_fulfilled(30.0));
        assert!(speed_fulfilled(-64.0));
    }

    /// Runs a cycle at `rpm`, applying after the recipe's time; answers the ticks it took.
    fn run(rpm: f32) -> (Mixer, i32, i32) {
        let mut m = Mixer::default();
        m.start();
        let (mut ticks, mut applied) = (0, 0);
        loop {
            let out = m.tick(rpm, 100);
            ticks += 1;
            if out.apply {
                applied += 1;
            }
            if out.recheck {
                return (m, ticks, applied);
            }
            assert!(ticks < 1000);
        }
    }

    #[test]
    fn a_cycle_goes_down_works_once_and_comes_back_up() {
        let (m, ticks, applied) = run(64.0);
        assert_eq!(applied, 1);
        assert!(!m.running);
        assert_eq!(m.running_ticks, 0);
        // 20 down, one tick to find the time (plus the wasted first), the time itself, 20 up.
        assert!(ticks > 40 + 3 * 15, "{ticks}");
    }

    #[test]
    fn losing_speed_on_the_way_down_turns_the_head_round() {
        let mut m = Mixer::default();
        m.start();
        for _ in 0..8 {
            m.tick(64.0, 100);
        }
        assert_eq!(m.running_ticks, 8);
        m.tick(0.0, 100);
        // `runningTicks = 40 - runningTicks`, then the tick counts on.
        assert_eq!(m.running_ticks, 33);
        let mut out = Tick::default();
        for _ in 0..8 {
            out = m.tick(0.0, 100);
        }
        assert!(out.recheck || !m.running);
    }

    #[test]
    fn losing_speed_at_the_bottom_skips_the_recipe() {
        let mut m = Mixer {
            running: true,
            running_ticks: BOTTOM,
            processing_ticks: 10,
        };
        let out = m.tick(0.0, 100);
        assert!(!out.apply);
        assert_eq!(m.running_ticks, BOTTOM + 2);
    }

    #[test]
    fn start_does_not_restart_a_cycle_on_its_way_down() {
        let mut m = Mixer {
            running: true,
            running_ticks: 7,
            processing_ticks: 0,
        };
        m.start();
        assert_eq!(m.running_ticks, 7);
        m.running_ticks = 30;
        m.start();
        assert_eq!(m.running_ticks, 0);
    }

    #[test]
    fn a_recipe_that_still_matches_keeps_the_head_down() {
        let mut m = Mixer {
            running: true,
            running_ticks: 21,
            processing_ticks: -1,
        };
        m.continue_recipe();
        assert_eq!(m.running_ticks, BOTTOM);
        m.basin_removed();
        assert_eq!((m.running, m.running_ticks), (false, CYCLE));
    }

    #[test]
    fn the_head_is_down_by_seven_sixteenths_at_rest_and_a_block_lower_at_the_bottom() {
        assert_eq!(head_offset(false, 0, 0.0), 7.0 / 16.0);
        assert!((head_offset(true, BOTTOM, 0.0) - (1.0 + 7.0 / 16.0)).abs() < 1e-6);
        assert!((head_offset(true, 0, 0.0) - (0.5 - 0.5 + 7.0 / 16.0)).abs() < 1e-6);
        assert!(head_offset(true, 10, 0.0) > head_offset(true, 5, 0.0));
        // Going up mirrors going down.
        let down = head_offset(true, 12, 0.0);
        let up = head_offset(true, CYCLE - 12, 0.0);
        assert!((down - up).abs() < 1e-5);
    }

    #[test]
    fn the_head_turns_faster_near_the_bottom_and_slower_idle() {
        assert_eq!(head_rotation_speed(false, 0, 64.0), 32.0);
        assert_eq!(head_rotation_speed(true, 10, 64.0), 64.0);
        assert_eq!(head_rotation_speed(true, 17, 64.0), 128.0);
        assert_eq!(head_rotation_speed(true, 30, 64.0), 64.0);
    }

    #[test]
    fn clients_wait_at_the_bottom_and_stop_at_the_top() {
        assert_eq!(client_ticks(0, 100, 105.5), 5.5);
        assert_eq!(client_ticks(0, 100, 150.0), 20.0);
        assert_eq!(client_ticks(21, 100, 110.0), 31.0);
        assert_eq!(client_ticks(21, 100, 400.0), 40.0);
        assert_eq!(client_ticks(20, 100, 400.0), 20.0);
    }
}
