//! The Clockwork Bearing: two structures turned as the hour and minute hands of the day time
//! (`ClockworkBearingBlockEntity`, `ClockworkContraption`). The angles are not a function of the
//! speed alone: each hand chases a target that follows the day time, at most as fast as the
//! bearing turns (`getHourArmSpeed`, `getMinuteArmSpeed`), so the position is worked out tick by
//! tick from a saved snapshot ([`Clock::angles_at`]). Plain values: the glue reads the block, the
//! day time and the dimension.

use std::collections::HashSet;

use crate::rules::arm::shortest_angle_diff;
use crate::rules::contraption::{self, Config, Fail, Found, World, relative};
use crate::{Dir, Pos};

pub const ID: &str = "create:clockwork_bearing";

/// `CStress.setImpact(4.0)` of `AllBlocks.CLOCKWORK_BEARING`.
pub const IMPACT: f32 = 4.0;

/// The number of ticks a snapshot is worked forward at most before the hands are taken to have
/// caught up with their targets.
pub const MAX_CATCH_UP: i64 = 4000;

/// `ClockworkBearingBlockEntity.ClockHands`, the scroll value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hands {
    HourFirst,
    MinuteFirst,
    HourFirst24,
}

impl Hands {
    pub fn from_value(value: i32) -> Hands {
        match value {
            1 => Hands::MinuteFirst,
            2 => Hands::HourFirst24,
            _ => Hands::HourFirst,
        }
    }

    pub const fn value(self) -> i32 {
        match self {
            Hands::HourFirst => 0,
            Hands::MinuteFirst => 1,
            Hands::HourFirst24 => 2,
        }
    }

    /// The next of the three (the stand-in for scrolling).
    pub const fn next(self) -> Hands {
        match self {
            Hands::HourFirst => Hands::MinuteFirst,
            Hands::MinuteFirst => Hands::HourFirst24,
            Hands::HourFirst24 => Hands::HourFirst,
        }
    }

    /// `create.contraptions.clockwork.*` and its English.
    pub const fn label(self) -> (&'static str, &'static str) {
        match self {
            Hands::HourFirst => ("create.contraptions.clockwork.hour_first", "Hour hand first"),
            Hands::MinuteFirst => ("create.contraptions.clockwork.minute_first", "Minute hand first"),
            Hands::HourFirst24 => ("create.contraptions.clockwork.hour_first_24", "24-Hour hand first"),
        }
    }
}

/// What the bearing's clock reads: its speed, the mode, which way along its axis it faces
/// (`getAxisDirection().getStep()`), whether the dimension is natural (`dimensionType().natural()`:
/// the day runs 24 times faster elsewhere) and the day time at game time 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clock {
    pub rpm: f32,
    pub hands: Hands,
    pub step: i32,
    pub natural: bool,
    pub day_offset: i64,
}

/// `KineticBlockEntity.getAngularSpeed` of the Clockwork: `-abs(speed * 3 / 10)`.
pub fn angular_speed(rpm: f32) -> f32 {
    -(rpm * 3.0 / 10.0).abs()
}

impl Clock {
    fn day_time(&self, tick: i64) -> i64 {
        let day = tick + self.day_offset;
        (day * if self.natural { 1 } else { 24 }).rem_euclid(24_000)
    }

    /// `getHourTarget`.
    pub fn hour_target(&self, tick: i64, cycle24: bool) -> f32 {
        let hours = (self.day_time(tick) / 1000 + 6) % 24;
        let around = if cycle24 { 24.0f32 } else { 12.0 };
        let shown = if cycle24 { hours % 24 } else { hours % 12 };
        (self.step * -360) as f32 / around * shown as f32
    }

    /// `getMinuteTarget`.
    pub fn minute_target(&self, tick: i64) -> f32 {
        let minutes = (self.day_time(tick) % 1000) * 60 / 1000;
        (self.step * -360) as f32 / 60.0 * minutes as f32
    }

    fn chase(speed: f32, angle: f32, target: f32) -> f32 {
        let diff = shortest_angle_diff(angle, target);
        if diff < 0.0 { speed.max(diff) } else { (-speed).min(diff) }
    }

    /// `getHourArmSpeed`, degrees this tick.
    pub fn hour_speed(&self, hour: f32, tick: i64) -> f32 {
        let speed = angular_speed(self.rpm) / 2.0;
        if speed == 0.0 {
            return speed;
        }
        let target = match self.hands {
            Hands::HourFirst => self.hour_target(tick, false),
            Hands::MinuteFirst => self.minute_target(tick),
            Hands::HourFirst24 => self.hour_target(tick, true),
        };
        Self::chase(speed, hour, target)
    }

    /// `getMinuteArmSpeed`, degrees this tick.
    pub fn minute_speed(&self, minute: f32, tick: i64) -> f32 {
        let speed = angular_speed(self.rpm);
        if speed == 0.0 {
            return speed;
        }
        let target = if self.hands == Hands::MinuteFirst {
            self.hour_target(tick, false)
        } else {
            self.minute_target(tick)
        };
        Self::chase(speed, minute, target)
    }

    /// One tick of `ClockworkBearingBlockEntity.tick`: both angles advance, `% 360`.
    pub fn step_tick(&self, angles: (f32, f32), tick: i64) -> (f32, f32) {
        let hour = (angles.0 + self.hour_speed(angles.0, tick)) % 360.0;
        let minute = (angles.1 + self.minute_speed(angles.1, tick)) % 360.0;
        (hour, minute)
    }

    /// The angles at game time `now` from the angles at `time`: a tick at a time, or, when the
    /// snapshot is older than [`MAX_CATCH_UP`], the hands where they would have long caught up to.
    pub fn angles_at(&self, snapshot: (f32, f32), time: i64, now: i64) -> (f32, f32) {
        let mut angles = snapshot;
        let mut from = time + 1;
        if now - time > MAX_CATCH_UP {
            from = now - MAX_CATCH_UP / 4;
        }
        for tick in from..=now {
            angles = self.step_tick(angles, tick);
        }
        angles
    }
}

/// `assembleClockworkAt`: the length of the hour hand along the axis from its anchor, which is
/// where the minute hand's structure starts: the first of 16 cells along the facing with no block
/// of the hour hand (0 when all 16 hold one). `has` says whether the hour hand has a block at a
/// position relative to its anchor.
pub fn hour_arm_length(has: &dyn Fn(Pos) -> bool, facing: [i32; 3]) -> i32 {
    for i in 0..16 {
        if has([facing[0] * i, facing[1] * i, facing[2] * i]) {
            continue;
        }
        return i;
    }
    0
}

/// One hand: the cell its structure is anchored at (the cell its rotation axis passes through)
/// and the blocks it takes.
pub struct Arm {
    pub anchor: Pos,
    pub found: Found,
}

/// The two hands of `ClockworkContraption.assembleClockworkAt`.
pub struct Arms {
    pub hour: Arm,
    /// The minute hand and its `offset`: where it starts, past the hour hand's line.
    pub minute: Option<(Arm, i32)>,
}

fn search_arm(world: &dyn World, bearing: Pos, start: Pos, ignore: &HashSet<Pos>) -> Result<Found, Fail> {
    let anchoring = move |p: Pos| p == bearing || ignore.contains(&p);
    let mut frontier = Vec::new();
    // `searchMovedStructure`: the first cell starts the search unless it is brittle.
    if let Some(state) = world.state(start) {
        if !contraption::is_brittle(world, &state) {
            frontier.push(start);
        }
    }
    let config = Config { forced: None, anchoring: &anchoring, frontier, preexisting: 0, known: HashSet::new() };
    contraption::search(world, &config)
}

/// `assembleClockworkAt`: the hour hand from the block in front of the bearing, then the minute hand
/// from the first cell in line beyond the hour hand's line, the hour hand's blocks left out of it.
pub fn assemble(world: &dyn World, pos: Pos, facing: Dir) -> Result<Arms, Fail> {
    let anchor = relative(pos, facing, 1);
    let hour = search_arm(world, pos, anchor, &HashSet::new())?;
    let taken: HashSet<Pos> = hour.blocks.iter().map(|(p, _)| *p).collect();
    let normal = facing.normal();
    let length = hour_arm_length(
        &|local| taken.contains(&[anchor[0] + local[0], anchor[1] + local[1], anchor[2] + local[2]]),
        normal,
    );
    let mut minute = None;
    if length > 0 {
        let start = relative(pos, facing, length + 1);
        let found = search_arm(world, pos, start, &taken)?;
        if !found.blocks.is_empty() {
            minute = Some((Arm { anchor: start, found }, length));
        }
    }
    Ok(Arms { hour: Arm { anchor, found: hour }, minute })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock(rpm: f32) -> Clock {
        Clock { rpm, hands: Hands::HourFirst, step: 1, natural: true, day_offset: 0 }
    }

    #[test]
    fn day_time_zero_is_six_in_the_morning() {
        // `(0 / 1000 + 6) % 24` = 6 o'clock: half way round a 12-hour dial.
        assert_eq!(clock(16.0).hour_target(0, false), -180.0);
        assert_eq!(clock(16.0).minute_target(0), 0.0);
        // 24-hour: a quarter.
        assert_eq!(clock(16.0).hour_target(0, true), -90.0);
        // A facing the other way turns the other way.
        let back = Clock { step: -1, ..clock(16.0) };
        assert_eq!(back.hour_target(0, false), 180.0);
    }

    #[test]
    fn a_thousand_ticks_is_an_hour_and_sixty_minutes() {
        let c = clock(16.0);
        // Seven o'clock.
        assert_eq!(c.hour_target(1000, false), -210.0);
        // 500 ticks: half past, 30 minutes of 6 degrees each.
        assert_eq!(c.minute_target(500), -180.0);
        assert_eq!(c.minute_target(999), -354.0);
    }

    #[test]
    fn the_other_dimensions_run_their_days_twenty_four_times_faster() {
        let nether = Clock { natural: false, ..clock(16.0) };
        // One tick is 24 ticks of the day: 24 / 1000 of an hour: no minute yet (24 * 60 / 1000 = 1).
        assert_eq!(nether.minute_target(1), -6.0);
        assert_eq!(nether.hour_target(1000, false), clock(16.0).hour_target(24_000 % 24_000 + 0, false));
    }

    #[test]
    fn the_hands_catch_up_with_the_time_and_stay_with_it() {
        let c = clock(16.0);
        // The hour hand moves 2.4 a tick, the minute hand 4.8; 180 degrees take at most 75 ticks.
        let (hour, minute) = c.angles_at((0.0, 0.0), 0, 200);
        assert!((shortest_angle_diff(hour, c.hour_target(200, false))).abs() < 0.1);
        assert!((shortest_angle_diff(minute, c.minute_target(200))).abs() < 5.0);
        // A hand never moves faster than the bearing.
        let (h, _) = c.step_tick((0.0, 0.0), 1);
        assert!(h.abs() <= 2.4 + 1e-6);
    }

    #[test]
    fn a_stopped_clock_stands_still() {
        let c = clock(0.0);
        assert_eq!(c.angles_at((12.0, 34.0), 0, 100), (12.0, 34.0));
    }

    #[test]
    fn minute_first_swaps_what_the_hands_follow() {
        let c = Clock { hands: Hands::MinuteFirst, ..clock(256.0) };
        // The first hand follows the minute target, the second the hour's.
        let (h, m) = c.angles_at((0.0, 0.0), 0, 50);
        assert!((shortest_angle_diff(h, c.minute_target(50))).abs() < 0.5);
        assert!((shortest_angle_diff(m, c.hour_target(50, false))).abs() < 0.5);
    }

    #[test]
    fn a_snapshot_worked_in_two_halves_agrees_with_one_run() {
        let c = clock(16.0);
        let whole = c.angles_at((0.0, 0.0), 0, 300);
        let first = c.angles_at((0.0, 0.0), 0, 120);
        let second = c.angles_at(first, 120, 300);
        assert_eq!(whole, second);
    }

    #[test]
    fn the_hour_hand_arm_ends_at_the_first_gap() {
        let blocks = [[0, 0, 0], [1, 0, 0], [2, 0, 0], [5, 0, 0]];
        let has = |p: Pos| blocks.contains(&p);
        assert_eq!(hour_arm_length(&has, [1, 0, 0]), 3);
        // Nothing at the anchor: 0.
        assert_eq!(hour_arm_length(&|_| false, [1, 0, 0]), 0);
        // Sixteen in a row: the loop finds no gap and the length stays 0.
        assert_eq!(hour_arm_length(&|_| true, [1, 0, 0]), 0);
    }

    #[test]
    fn the_minute_hand_starts_past_the_hour_hand() {
        use crate::rules::contraption::State;
        use std::collections::HashMap;
        struct Grid(HashMap<Pos, State>);
        impl World for Grid {
            fn state(&self, pos: Pos) -> Option<State> {
                Some(self.0.get(&pos).cloned().unwrap_or_else(|| State::new("minecraft:air")))
            }
            fn collision_empty(&self, pos: Pos) -> bool {
                self.0.get(&pos).is_none_or(State::is_air)
            }
        }
        let mut grid = Grid(HashMap::new());
        // The bearing at the origin facing east; an hour hand of a stone with slime above it, and
        // beside it, not stuck to it (stone sticks to nothing), the minute hand: a stone with slime
        // after it.
        grid.0.insert([1, 0, 0], State::new("minecraft:stone"));
        grid.0.insert([1, 1, 0], State::new("minecraft:slime_block"));
        grid.0.insert([2, 0, 0], State::new("minecraft:stone"));
        grid.0.insert([3, 0, 0], State::new("minecraft:slime_block"));
        let arms = assemble(&grid, [0, 0, 0], Dir::East).expect("assembles");
        assert_eq!(arms.hour.anchor, [1, 0, 0]);
        let hour: HashSet<Pos> = arms.hour.found.blocks.iter().map(|(p, _)| *p).collect();
        assert!(hour.contains(&[1, 0, 0]) && hour.contains(&[1, 1, 0]));
        assert!(!hour.contains(&[2, 0, 0]));
        let (minute, offset) = arms.minute.expect("a minute hand");
        assert_eq!(offset, 1);
        assert_eq!(minute.anchor, [2, 0, 0]);
        assert!(minute.found.blocks.iter().any(|(p, _)| *p == [3, 0, 0]));
        // The hour hand's blocks are left to the hour hand.
        assert!(minute.found.blocks.iter().all(|(p, _)| !hour.contains(p)));
    }

    #[test]
    fn the_modes_cycle() {
        assert_eq!(Hands::HourFirst.next().next().next(), Hands::HourFirst);
        assert_eq!(Hands::from_value(Hands::MinuteFirst.value()), Hands::MinuteFirst);
    }
}
