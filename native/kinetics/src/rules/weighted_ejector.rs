//! The Weighted Ejector's rules: the arc it throws along, how long it takes to charge, the lid, the
//! flight of what it launched and how a target is chosen. Ported from Create `mc1.21.1/dev` at
//! 4148fd8 (MIT, the Creators of Create): `EntityLauncher`, `EjectorBlockEntity` (`tick`,
//! `getWindUpSpeed`, `getLaunchedItemLocation`, `getTargetOpenInv`), `EjectorTargetHandler`
//! (`getValidTargetDirection`, `flushSettings`), `AllBlocks.WEIGHTED_EJECTOR`
//! (`CStress.setImpact(2.0)`) and `CKinetics` (`maxEjectorDistance` 32, `ejectorScanInterval` 120).
//! No game types.

use crate::{Dir, Pos};

/// Stress units per RPM (`AllBlocks.WEIGHTED_EJECTOR`: `CStress.setImpact(2.0)`).
pub const IMPACT: f32 = 2.0;
/// `CKinetics.maxEjectorDistance`: the furthest a target may be to either side of the ejector.
pub const MAX_DISTANCE: i32 = 32;
/// `CKinetics.ejectorScanInterval`: ticks between one launched item being scanned for obstacles
/// and the next.
pub const SCAN_INTERVAL: i32 = 120;

/// `EjectorBlockEntity.State`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Charged,
    Launching,
    Retracting,
}

impl State {
    /// The names `NBTHelper.writeEnum` saves (`State`).
    pub fn name(self) -> &'static str {
        match self {
            State::Charged => "CHARGED",
            State::Launching => "LAUNCHING",
            State::Retracting => "RETRACTING",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [State::Charged, State::Launching, State::Retracting].into_iter().find(|s| s.name() == name)
    }
}

/// `AngleHelper.horizontalAngle`: south 0, east 90, north 180, west 270.
pub fn horizontal_angle(facing: Dir) -> f32 {
    match facing {
        Dir::East => 90.0,
        Dir::North => 180.0,
        Dir::West => 270.0,
        _ => 0.0,
    }
}

/// `VecHelper.rotate(vec, degrees, Axis.Y)`.
fn rotate_y(v: [f64; 3], degrees: f64) -> [f64; 3] {
    let (sin, cos) = degrees.to_radians().sin_cos();
    [v[0] * cos + v[2] * sin, v[1], v[2] * cos - v[0] * sin]
}

/// `EntityLauncher`: the arc to a target `horizontal` blocks ahead and `vertical` blocks up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Launcher {
    horizontal: i32,
    vertical: i32,
    y_motion: f64,
    x_motion: f64,
    total_flying_ticks: f64,
}

impl Launcher {
    pub fn new(horizontal: i32, vertical: i32) -> Self {
        let mut launcher = Self {
            horizontal: 0,
            vertical: 0,
            y_motion: 0.0,
            x_motion: 0.0,
            total_flying_ticks: 0.0,
        };
        launcher.set(horizontal, vertical);
        launcher
    }

    pub fn horizontal(&self) -> i32 {
        self.horizontal
    }

    pub fn vertical(&self) -> i32 {
        self.vertical
    }

    pub fn total_flying_ticks(&self) -> f64 {
        self.total_flying_ticks
    }

    /// `clamp(max)`.
    pub fn clamp(&mut self, max: i32) {
        self.set(
            self.horizontal.min(max),
            self.vertical.signum() * self.vertical.abs().min(max),
        );
    }

    /// `set`, which recalculates the trajectory.
    pub fn set(&mut self, horizontal: i32, vertical: i32) {
        self.horizontal = horizontal;
        self.vertical = vertical;
        self.recalculate();
    }

    /// `applyMotion`: the velocity an entity leaves with, facing `facing`
    /// (`motion.x * .91, motion.y * .98, motion.z * .91`).
    pub fn motion(&self, facing: Dir) -> [f64; 3] {
        let v = rotate_y([0.0, self.y_motion, self.x_motion], f64::from(horizontal_angle(facing)));
        [v[0] * 0.91, v[1] * 0.98, v[2] * 0.91]
    }

    /// `getGlobalPos`: where something launched at `launcher` (the block's position) is `t` ticks
    /// later. `towards` is the ejector's own facing (Create passes `getFacing().getOpposite()` and
    /// turns by 180 more).
    pub fn global_pos(&self, t: f64, towards: Dir, launcher: Pos) -> [f64; 3] {
        let start = launcher.map(|v| f64::from(v) + 0.5);
        let xt = f64::from(self.x(t));
        let yt = f64::from(self.y(t));
        let progress = (t / self.total_flying_ticks).clamp(0.0, 1.0);
        let correction = progress.powi(3);
        let vec = [
            0.0,
            yt + (f64::from(self.vertical) - yt) * correction * 0.5,
            xt + (f64::from(self.horizontal) - xt) * correction,
        ];
        let v = rotate_y(vec, f64::from(horizontal_angle(towards)));
        [v[0] + start[0], v[1] + start[1], v[2] + start[2]]
    }

    /// `getGlobalVelocity`.
    pub fn global_velocity(&self, t: f64, towards: Dir) -> [f64; 3] {
        rotate_y(
            [0.0, f64::from(self.dy(t)), f64::from(self.dx(t))],
            f64::from(horizontal_angle(towards)),
        )
    }

    /// `x(t)`: how far ahead after `t` ticks.
    pub fn x(&self, t: f64) -> f32 {
        (self.x_motion * -10.6033 * (-1.0 + 0.91_f64.powf(t))) as f32
    }

    /// `y(t)`: how high after `t` ticks.
    pub fn y(&self, t: f64) -> f32 {
        let f = 0.98_f64.powf(t);
        (self.y_motion * -49.4983 * f + 49.4983 * self.y_motion - 194.033 * f - 3.92 * t + 194.033) as f32
    }

    pub fn dx(&self, t: f64) -> f32 {
        (self.x_motion * 0.91_f64.powf(t)) as f32
    }

    pub fn dy(&self, t: f64) -> f32 {
        let f = 0.98_f64.powf(t);
        (self.y_motion * f + ((f - 1.0) / (0.98 - 1.0)) * -0.0784) as f32
    }

    /// `recalculateTrajectory`.
    fn recalculate(&mut self) {
        let x_target = f64::from(self.horizontal);
        let y_target = f64::from(self.vertical);

        // Interpolated correction from observed inaccuracies in-game (Create's comment).
        let x_error = (-0.0007 * (x_target + 0.5).powi(2) + 0.484)
            - (y_target.min(5.0) / 5.0) * (0.076 * x_target - 0.0014 * x_target * x_target).min(1.0);

        let y_peak = (y_target + (x_target + 0.5) / 8.0).max(0.0) + if x_target <= 1.0 { 1.0 } else { 4.0 };
        self.y_motion = (2.0 * y_peak / 13.0).sqrt() + 0.015 * y_peak;
        let t_peak = (98.0 / (25.0 * self.y_motion + 98.0)).ln()
            / (2.0 * 7.0_f64.ln() - 2.0 * 5.0_f64.ln() - 2.0_f64.ln());
        self.total_flying_ticks =
            (y_peak - self.y_motion).sqrt() * 4.87 + 0.115 * (y_peak - self.y_motion) + t_peak;
        self.x_motion = (x_target - x_error + 0.5) / (-10.6033 * (-1.0 + 0.91_f64.powf(self.total_flying_ticks)));
    }
}

/// `EjectorBlockEntity.getWindUpSpeed`: how much of the lid's turn each tick winds up, from the
/// speed in RPM. The further the target, the slower.
pub fn wind_up_speed(rpm: f32, horizontal: i32, vertical: i32) -> f32 {
    let speed_factor = rpm.abs() / 256.0;
    let distance_factor = if horizontal == 0 && vertical == 0 {
        1.0
    } else {
        ((horizontal * horizontal + vertical * vertical) as f32).sqrt()
    };
    speed_factor / distance_factor
}

/// The lid at game time `now`, `0` shut and charged and `1` open, for an ejector that entered `state` at
/// `start` with the lid at `lid0` and turns at `rpm`: wound up each tick by [`wind_up_speed`] while
/// retracting, thrown open by [`launch_chase`] while launching.
pub fn lid_at(state: State, lid0: f32, start: i64, now: i64, rpm: f32, horizontal: i32, vertical: i32) -> f32 {
    let elapsed = (now - start).max(0);
    match state {
        State::Charged => 0.0,
        State::Launching => {
            let mut lid = lid0;
            for _ in 0..elapsed.min(16) {
                lid = launch_chase(lid);
            }
            lid
        }
        State::Retracting => {
            (lid0 - wind_up_speed(rpm, horizontal, vertical) * elapsed as f32).clamp(0.0, 1.0)
        }
    }
}

/// One tick of a retracting lid: `Mth.clamp(lid - getWindUpSpeed(), 0, 1)`.
pub fn wind_up(lid: f32, speed: f32) -> f32 {
    (lid - speed).clamp(0.0, 1.0)
}

/// One tick of a launching lid: `lidProgress.chase(1, .8f, Chaser.EXP)` moves it a share of the
/// way to 1 (`Chaser.EXP` snaps when closer than 1 / 4096).
pub fn launch_chase(lid: f32) -> f32 {
    let next = lid + (1.0 - lid) * 0.8;
    if (1.0 - next).abs() < 1.0 / 4096.0 { 1.0 } else { next }
}

/// The lid is out far enough to start retracting (`lid > 1 - 1 / 16`).
pub fn lid_is_open(lid: f32) -> bool {
    lid > 1.0 - 1.0 / 16.0
}

/// How many ticks a launched item flies: `Math.max(3, totalFlyingTicks)`.
pub fn total_time(launcher: &Launcher) -> f32 {
    (launcher.total_flying_ticks() as f32).max(3.0)
}

/// `IntAttached.exceeds`: an item that has flown `elapsed` ticks lands once that is more than
/// `max_time` (cut to a whole number of ticks).
pub fn has_landed(elapsed: i32, max_time: f32) -> bool {
    elapsed > max_time as i32
}

/// `EjectorTargetHandler.getValidTargetDirection`: which way an ejector placed at `placed` must
/// face to throw to `selected`, or `None` if the two are in one column, further than `max` apart
/// or not in line along an axis.
pub fn valid_direction(placed: Pos, selected: Pos, max: i32) -> Option<Dir> {
    let x = selected[0] - placed[0];
    let z = selected[2] - placed[2];
    // `VecHelper.onSameAxis(pos, selection, Axis.Y)`: the same column.
    if x == 0 && z == 0 {
        return None;
    }
    if x.abs() > max || z.abs() > max {
        return None;
    }
    if x == 0 {
        return Some(if z < 0 { Dir::North } else { Dir::South });
    }
    if z == 0 {
        return Some(if x < 0 { Dir::West } else { Dir::East });
    }
    None
}

/// `EjectorTargetHandler.flushSettings`: the horizontal and vertical distance to `selected` from an
/// ejector placed at `placed`: `h = |dx + dz|` and `v = -dy` of the placed position minus the
/// selection.
pub fn distances(placed: Pos, selected: Pos) -> (i32, i32) {
    let diff = [placed[0] - selected[0], placed[1] - selected[1], placed[2] - selected[2]];
    ((diff[0] + diff[2]).abs(), -diff[1])
}

/// `EjectorBlockEntity.setTarget`: `Math.max(1, horizontalDistance)`.
pub fn target_distances(horizontal: i32, vertical: i32) -> (i32, i32) {
    (horizontal.max(1), vertical)
}

/// The block a launch ends at (`getTargetPosition` and `getTargetOpenInv`): `horizontal` ahead and
/// `vertical` up, at least one ahead.
pub fn target_position(pos: Pos, facing: Dir, horizontal: i32, vertical: i32) -> Pos {
    let n = facing.normal();
    let ahead = horizontal.max(1);
    [pos[0] + n[0] * ahead, pos[1] + vertical, pos[2] + n[2] * ahead]
}

/// `ejectIfTriggered`'s conditions that read only numbers: whether the held stack is big enough
/// and has settled in the middle (`beltPosition >= .49`).
pub fn ready(present: u32, stack_size: i32, belt_position: Option<f32>) -> bool {
    if present == 0 {
        return false;
    }
    if (present as i32) < stack_size {
        return false;
    }
    !belt_position.is_some_and(|p| p < 0.49)
}

/// `DepotBehaviour.getRemainingSpace` for a merging ejector: how many more of a stack's items the
/// ejector takes, given the stack size setting (0 means up to 64) and the item's stack limit.
pub fn remaining_space(setting: i32, item_max: u32, present: u32) -> i32 {
    let from_setting = if setting == 0 { 64 } else { setting };
    from_setting.min(item_max as i32) - present as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_launcher_for_the_default_target_is_a_short_hop() {
        // The constructor's `new EntityLauncher(1, 0)`.
        let l = Launcher::new(1, 0);
        assert!(l.total_flying_ticks() > 3.0 && l.total_flying_ticks() < 20.0, "{}", l.total_flying_ticks());
        assert!(l.y_motion > 0.0 && l.x_motion > 0.0);
    }

    #[test]
    fn the_arc_ends_over_the_target() {
        // At the end of the flight the correction pulls the item onto the target block's
        // centre (`correctionStrength` 1).
        for (h, v) in [(1, 0), (3, 0), (8, 2), (16, -4), (20, 5), (32, 10)] {
            let l = Launcher::new(h, v);
            let end = l.global_pos(l.total_flying_ticks(), Dir::South, [0, 0, 0]);
            // Facing south: +z. The item is `h` ahead and `v` up from the block's centre.
            assert!((end[2] - (0.5 + f64::from(h))).abs() < 1e-3, "{h},{v}: {end:?}");
            // Create's approximation of the height of the arc (`Simi's approximation`) lands
            // within a few blocks of the target's: the correction closes half the gap, so far throws end short.
            assert!((end[1] - (0.5 + f64::from(v))).abs() < 5.0, "{h},{v}: {end:?}");
            assert!((end[0] - 0.5).abs() < 1e-6);
        }
    }

    #[test]
    fn it_throws_the_way_the_ejector_faces() {
        let l = Launcher::new(5, 1);
        let east = l.global_pos(4.0, Dir::East, [10, 64, 10]);
        let south = l.global_pos(4.0, Dir::South, [10, 64, 10]);
        assert!(east[0] > 10.5 && (east[2] - 10.5).abs() < 1e-9);
        assert!(south[2] > 10.5 && (south[0] - 10.5).abs() < 1e-9);
        let west = l.global_pos(4.0, Dir::West, [10, 64, 10]);
        assert!(west[0] < 10.5);
        let north = l.global_pos(4.0, Dir::North, [10, 64, 10]);
        assert!(north[2] < 10.5);
        // The entity's launch velocity points the same way.
        assert!(l.motion(Dir::East)[0] > 0.0 && l.motion(Dir::West)[0] < 0.0);
        assert!(l.motion(Dir::South)[2] > 0.0 && l.motion(Dir::North)[2] < 0.0);
        assert!(l.motion(Dir::South)[1] > 0.0);
    }

    #[test]
    fn a_launched_entity_rises_then_falls() {
        let l = Launcher::new(6, 0);
        assert!(l.dy(0.0) > 0.0);
        assert!(l.dy(l.total_flying_ticks()) < 0.0);
        assert!(l.global_velocity(0.0, Dir::South)[2] > 0.0);
    }

    #[test]
    fn a_target_is_clamped_to_the_maximum_range() {
        let mut l = Launcher::new(50, -60);
        l.clamp(MAX_DISTANCE);
        assert_eq!((l.horizontal(), l.vertical()), (32, -32));
        let mut l = Launcher::new(3, 5);
        l.clamp(MAX_DISTANCE);
        assert_eq!((l.horizontal(), l.vertical()), (3, 5));
    }

    #[test]
    fn farther_targets_wind_up_more_slowly() {
        assert_eq!(wind_up_speed(256.0, 0, 0), 1.0);
        assert!((wind_up_speed(256.0, 3, 4) - 0.2).abs() < 1e-6);
        assert!((wind_up_speed(-128.0, 1, 0) - 0.5).abs() < 1e-6);
        assert_eq!(wind_up_speed(0.0, 5, 5), 0.0);
        assert_eq!(wind_up(0.3, 0.5), 0.0);
        assert_eq!(wind_up(1.0, 0.0), 1.0);
    }

    #[test]
    fn the_lid_flies_open_and_then_retracts() {
        let mut lid = 0.0;
        let mut ticks = 0;
        while !lid_is_open(lid) {
            lid = launch_chase(lid);
            ticks += 1;
        }
        // 1 - 0.2^n passes 15/16 on the second tick.
        assert_eq!(ticks, 2);
    }

    #[test]
    fn items_land_after_their_flight() {
        let l = Launcher::new(4, 0);
        let max = total_time(&l);
        assert!(!has_landed(0, max));
        assert!(has_landed(max as i32 + 1, max));
        assert!(!has_landed(max as i32, max));
    }

    #[test]
    fn a_target_must_be_in_line_with_the_ejector() {
        let at = [10, 64, 10];
        assert_eq!(valid_direction(at, [10, 64, 15], 32), Some(Dir::South));
        assert_eq!(valid_direction(at, [10, 70, 5], 32), Some(Dir::North));
        assert_eq!(valid_direction(at, [20, 60, 10], 32), Some(Dir::East));
        assert_eq!(valid_direction(at, [3, 64, 10], 32), Some(Dir::West));
        // Off to a side, in the same column, or too far.
        assert_eq!(valid_direction(at, [12, 64, 15], 32), None);
        assert_eq!(valid_direction(at, [10, 80, 10], 32), None);
        assert_eq!(valid_direction(at, [10, 64, 60], 32), None);
        assert_eq!(valid_direction(at, [10, 64, 42], 32), Some(Dir::South));
    }

    #[test]
    fn the_distances_are_those_the_placement_packet_sends() {
        // Placed at 10,64,10 to throw to 10,70,15: 5 ahead and 6 up.
        assert_eq!(distances([10, 64, 10], [10, 70, 15]), (5, 6));
        assert_eq!(distances([10, 64, 10], [3, 60, 10]), (7, -4));
        assert_eq!(target_distances(0, 2), (1, 2));
        assert_eq!(target_position([10, 64, 10], Dir::East, 4, 2), [14, 66, 10]);
        assert_eq!(target_position([10, 64, 10], Dir::North, 0, 0), [10, 64, 9]);
    }

    #[test]
    fn it_fires_once_the_stack_is_big_enough_and_settled() {
        assert!(!ready(0, 0, None));
        assert!(ready(1, 0, Some(0.5)));
        assert!(!ready(1, 0, Some(0.3)));
        assert!(!ready(3, 4, Some(0.5)));
        assert!(ready(4, 4, None));
    }

    #[test]
    fn the_ejector_merges_up_to_its_stack_size_or_the_items_limit() {
        assert_eq!(remaining_space(0, 64, 10), 54);
        assert_eq!(remaining_space(16, 64, 10), 6);
        assert_eq!(remaining_space(0, 16, 10), 6);
        assert_eq!(remaining_space(8, 64, 8), 0);
    }

    #[test]
    fn states_round_trip_their_saved_names() {
        for s in [State::Charged, State::Launching, State::Retracting] {
            assert_eq!(State::from_name(s.name()), Some(s));
        }
    }

    #[test]
    fn a_retracting_lid_winds_up_with_the_speed() {
        // 256 RPM over 4 blocks: a quarter of the lid a tick.
        let lid = |now, rpm| lid_at(State::Retracting, 1.0, 100, now, rpm, 4, 0);
        assert!((lid(100, 256.0) - 1.0).abs() < 1e-6);
        assert!((lid(102, 256.0) - 0.5).abs() < 1e-6);
        assert_eq!(lid(110, 256.0), 0.0);
        // Not turning: the lid stays where it was.
        assert_eq!(lid(500, 0.0), 1.0);
    }

    #[test]
    fn a_launching_lid_flies_open_and_a_charged_one_is_shut() {
        let lid = |now| lid_at(State::Launching, 0.0, 10, now, 64.0, 1, 0);
        assert_eq!(lid(10), 0.0);
        assert!(lid(11) > 0.79 && lid(12) > 0.95);
        assert_eq!(lid_at(State::Charged, 0.0, 10, 99, 64.0, 1, 0), 0.0);
    }
}
