//! The Fluid Valve and the valve handles: how the valve's pointer opens and closes it, and how a
//! handle turns a given angle. No game types.
//!
//! Ported from Create `content/fluids/pipes/valve/FluidValveBlockEntity` (`onSpeedChanged`,
//! `getChaseSpeed`, `tick`), `FluidValveBlock` (`getPipeAxis`, in `fluid_pipe.rs`) and
//! `content/kinetics/crank/ValveHandleBlockEntity` / `HandCrankBlockEntity` (`activate`,
//! `getGeneratedSpeed`, `tick`), `ValveHandleBlock` (`getRotationSpeed` 32), `AllBlocks`
//! (`CStress.setCapacity(8.0)`).
//!
//! Known differences from Create:
//! - The handle's angle (`ScrollValueBehaviour`, -180 to 180 in steps of 45, 45 by default) has no
//!   value box here: a Wrench click steps it through [`ANGLES`] (the Creative Motor does the same
//!   with its speed).
//! - The valve's pointer is chased from the current speed on every tick, where Create's
//!   `LerpedFloat` is told once when the speed changes; the result is the same unless a speed
//!   changes while the pointer is moving.

use crate::{Axis, Dir};

/// The faces the valve's shaft ends on (`DirectionalAxisKineticBlock.hasShaftTowards`: both ends
/// of the rotation axis).
pub fn shaft_faces(facing: Dir, along_first: bool) -> [Dir; 2] {
    match super::fluid_pipe::valve_rotation_axis(facing, along_first) {
        Axis::X => [Dir::West, Dir::East],
        Axis::Y => [Dir::Down, Dir::Up],
        Axis::Z => [Dir::North, Dir::South],
    }
}

/// `Direction.fromYRot`: the horizontal direction a player turned `yaw` degrees looks.
pub fn horizontal_direction(yaw: f32) -> Dir {
    match (((yaw / 90.0 + 0.5).floor() as i32) & 3) as u8 {
        0 => Dir::South,
        1 => Dir::West,
        2 => Dir::North,
        _ => Dir::East,
    }
}

/// `DirectionalAxisKineticBlock.getStateForPlacement` without the shaft beside it it would
/// prefer: `(facing, axis_along_first)`. `look` is `getNearestLookingDirection`, `horizontal`
/// `getHorizontalDirection`.
pub fn placement(look: Dir, horizontal: Dir, sneaking: bool) -> (Dir, bool) {
    let mut facing = look.opposite();
    if sneaking {
        facing = facing.opposite();
    }
    let along_first = if facing.axis() == Axis::Y {
        horizontal.axis() == Axis::X
    } else {
        facing.axis() == Axis::Z
    };
    (facing, along_first)
}

/// `Direction.getClockWise(Axis)`: `facing` a quarter turn clockwise seen from the positive end of
/// `axis`; unchanged when it lies along the axis.
fn clockwise(facing: Dir, axis: Axis) -> Dir {
    match axis {
        Axis::Y => match facing {
            Dir::North => Dir::East,
            Dir::East => Dir::South,
            Dir::South => Dir::West,
            Dir::West => Dir::North,
            other => other,
        },
        Axis::X => match facing {
            Dir::Up => Dir::South,
            Dir::North => Dir::Up,
            Dir::South => Dir::Down,
            Dir::Down => Dir::North,
            other => other,
        },
        Axis::Z => match facing {
            Dir::Up => Dir::East,
            Dir::East => Dir::Down,
            Dir::Down => Dir::West,
            Dir::West => Dir::Up,
            other => other,
        },
    }
}

/// `IWrenchable.getRotatedBlockState` for a `DirectionalAxisKineticBlock` clicked on `clicked`:
/// `(facing, axis_along_first)` after the turn.
pub fn wrench(facing: Dir, along_first: bool, clicked: Dir) -> (Dir, bool) {
    if facing.axis() == clicked.axis() {
        return (facing, !along_first);
    }
    let (mut facing, mut along_first) = (facing, along_first);
    loop {
        facing = clockwise(facing, clicked.axis());
        if clicked.axis() == Axis::Y {
            along_first = !along_first;
        }
        if facing.axis() != clicked.axis() {
            return (facing, along_first);
        }
    }
}

/// `ValveHandleBlock.getRotationSpeed` (and `HandCrankBlock`'s): the RPM a handle turns its shaft at.
pub const ROTATION_SPEED: f32 = 32.0;
/// Stress units per RPM (`AllBlocks.COPPER_VALVE_HANDLE`: `CStress.setCapacity(8.0)`).
pub const CAPACITY: f32 = 8.0;
/// Ticks before a handle can be turned again (`ValveHandleBlockEntity.activate`: `cooldown = 4`).
pub const COOLDOWN: i32 = 4;
/// `ValveHandleScrollValueBehaviour.between(-180, 180)`, the default `angleInput.setValue(45)`.
pub const DEFAULT_ANGLE: i32 = 45;
/// What a Wrench click steps the angle through (the value box's 45-degree steps, both ways).
pub const ANGLES: [i32; 8] = [45, 90, 135, 180, -45, -90, -135, -180];

/// The angle after a Wrench click.
pub fn next_angle(angle: i32) -> i32 {
    let at = ANGLES.iter().position(|a| *a == angle);
    ANGLES[at.map_or(0, |i| (i + 1) % ANGLES.len())]
}

/// `FluidValveBlockEntity.getChaseSpeed`: how far the pointer moves each tick.
pub fn chase_speed(rpm: f32) -> f32 {
    (rpm.abs() / 16.0 / 20.0).clamp(0.0, 1.0)
}

/// The pointer (0 closed, 1 open) after one tick at `rpm` (`onSpeedChanged` starts a linear chase
/// to 1 for a positive speed and to 0 otherwise; at rest the chase speed is 0).
pub fn pointer_step(pointer: f32, rpm: f32) -> f32 {
    let target = if rpm > 0.0 { 1.0 } else { 0.0 };
    let step = chase_speed(rpm);
    // `Chaser.LINEAR`: lands on the target once it is within a step of it.
    if (target - pointer).abs() <= step {
        return if step > 0.0 { target } else { pointer };
    }
    pointer + (target - pointer).clamp(-step, step)
}

/// `FluidValveBlockEntity.tick`: the `enabled` state the block switches to, once the pointer has
/// reached an end; `None` while it still agrees.
pub fn switch_enabled(pointer: f32, enabled: bool) -> Option<bool> {
    if enabled && pointer == 0.0 {
        Some(false)
    } else if !enabled && pointer == 1.0 {
        Some(true)
    } else {
        None
    }
}

/// A Valve Handle's state (`ValveHandleBlockEntity`, `HandCrankBlockEntity`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handle {
    /// `angleInput`: how far one use turns the shaft; negative is the other way.
    pub angle: i32,
    /// `inUse`: ticks the handle still turns its shaft.
    pub in_use: i32,
    pub cooldown: i32,
    pub total_use_ticks: i32,
    pub start_angle: i32,
    pub target_angle: i32,
    pub backwards: bool,
}

impl Default for Handle {
    fn default() -> Self {
        Handle {
            angle: DEFAULT_ANGLE,
            in_use: 0,
            cooldown: 0,
            total_use_ticks: 0,
            start_angle: 0,
            target_angle: 0,
            backwards: false,
        }
    }
}

/// `KineticBlockEntity.convertToAngular`: degrees a tick at `rpm`.
pub fn degrees_per_tick(rpm: f32) -> f32 {
    rpm * 360.0 / 60.0 / 20.0
}

impl Handle {
    /// `ValveHandleBlockEntity.clockwise`.
    pub fn clockwise(&self) -> bool {
        (self.angle < 0) ^ self.backwards
    }

    /// `HandCrankBlockEntity.getGeneratedSpeed`, before it is turned to the block's direction:
    /// 32 RPM while in use, negative when `clockwise`.
    pub fn generated(&self) -> f32 {
        match (self.in_use, self.clockwise()) {
            (0, _) => 0.0,
            (_, true) => -ROTATION_SPEED,
            (_, false) => ROTATION_SPEED,
        }
    }

    /// `ValveHandleBlockEntity.activate` (server side): `current_speed` is the shaft's speed now
    /// (`getTheoreticalSpeed`). True when the handle starts turning.
    pub fn activate(&mut self, current_speed: f32, sneak: bool) -> bool {
        if current_speed != 0.0 || self.in_use > 0 || self.cooldown > 0 {
            return false;
        }
        // Always overshoot, the target will stop early.
        let value = self.angle;
        let target = value.abs();
        self.in_use = (target as f32 / degrees_per_tick(ROTATION_SPEED)).ceil() as i32 + 2;
        // `startAngle = (int) ((independentAngle) % 90 + 360) % 90`, with the server's
        // `independentAngle` always 0.
        self.start_angle = 0;
        let turn = if target > 135 { 180 } else { 90 };
        self.target_angle = ((self.start_angle + turn * value.signum()) as f32 / 90.0).round() as i32 * 90;
        self.total_use_ticks = self.in_use;
        self.backwards = sneak;
        self.cooldown = COOLDOWN;
        true
    }

    /// The counters of one tick (`HandCrankBlockEntity.tick`, `ValveHandleBlockEntity.tick`),
    /// after the speed was read.
    pub fn tick(&mut self) {
        if self.in_use > 0 {
            self.in_use -= 1;
        }
        if self.in_use == 0 && self.cooldown > 0 {
            self.cooldown -= 1;
        }
    }

    /// `ValveHandleBlockEntity.getIndependentAngle` while the handle turns, or at rest: the
    /// handle's own angle in degrees, `step` being its facing's axis direction.
    pub fn visual_angle(&self, elapsed: f32, step: i32) -> f32 {
        let sign = if self.backwards { -1.0 } else { 1.0 };
        let angle = if self.in_use > 0 && self.total_use_ticks > 0 {
            let progress = (self.total_use_ticks as f32).min(elapsed) / self.total_use_ticks as f32;
            self.start_angle as f32 + (self.target_angle - self.start_angle) as f32 * progress
        } else {
            self.target_angle as f32
        };
        angle * sign * step as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_valve_opens_while_its_shaft_turns_forwards_and_closes_backwards() {
        // 32 RPM: 32 / 16 / 20 = 0.1 a tick, ten ticks to open (eleven where the sum rounds up).
        let mut pointer = 0.0;
        let mut ticks = 0;
        while pointer < 1.0 {
            pointer = pointer_step(pointer, 32.0);
            ticks += 1;
            assert!(ticks <= 11);
        }
        assert!(ticks >= 10);
        assert_eq!(pointer, 1.0);
        let mut ticks = 0;
        while pointer > 0.0 {
            pointer = pointer_step(pointer, -32.0);
            ticks += 1;
            assert!(ticks <= 11);
        }
        assert_eq!(pointer, 0.0);
    }

    #[test]
    fn a_valve_is_placed_facing_the_player_with_its_shaft_across() {
        // Looking south and level: faces north, `facing.axis == Z` so along the first coordinate.
        assert_eq!(placement(Dir::South, Dir::South, false), (Dir::North, true));
        assert_eq!(placement(Dir::South, Dir::South, true), (Dir::South, true));
        // Looking down: faces up, along first when the player's horizontal direction is on X.
        assert_eq!(placement(Dir::Down, Dir::East, false), (Dir::Up, true));
        assert_eq!(placement(Dir::Down, Dir::South, false), (Dir::Up, false));
        assert_eq!(horizontal_direction(-90.0), Dir::East);
    }

    #[test]
    fn the_shaft_ends_are_on_the_rotation_axis() {
        assert_eq!(shaft_faces(Dir::Up, true), [Dir::West, Dir::East]);
        assert_eq!(shaft_faces(Dir::North, false), [Dir::Down, Dir::Up]);
        assert_eq!(shaft_faces(Dir::North, true), [Dir::West, Dir::East]);
    }

    #[test]
    fn the_wrench_turns_the_valve_round_the_face_clicked() {
        // On the facing axis: only the shaft turns.
        assert_eq!(wrench(Dir::Up, false, Dir::Up), (Dir::Up, true));
        // From the top, a horizontal facing turns clockwise and the shaft with it.
        assert_eq!(wrench(Dir::North, true, Dir::Up), (Dir::East, false));
        // From a side, a facing across turns until it is off the axis.
        assert_ne!(wrench(Dir::North, true, Dir::East).0.axis(), Axis::X);
    }

    #[test]
    fn a_still_valve_keeps_its_pointer() {
        assert_eq!(pointer_step(0.4, 0.0), 0.4);
    }

    #[test]
    fn the_pointer_switches_the_state_at_either_end() {
        assert_eq!(switch_enabled(1.0, false), Some(true));
        assert_eq!(switch_enabled(0.0, true), Some(false));
        assert_eq!(switch_enabled(0.5, true), None);
        assert_eq!(switch_enabled(1.0, true), None);
    }

    #[test]
    fn a_handle_turns_for_as_long_as_its_angle_takes_plus_two_ticks() {
        // 32 RPM is 9.6 degrees a tick: 45 degrees is 4.69 -> 5, plus 2.
        let mut handle = Handle::default();
        assert!(handle.activate(0.0, false));
        assert_eq!((handle.in_use, handle.total_use_ticks), (7, 7));
        assert_eq!(handle.target_angle, 90);
        assert_eq!(handle.generated(), 32.0);
        let mut handle = Handle { angle: 180, ..Handle::default() };
        handle.activate(0.0, false);
        assert_eq!((handle.in_use, handle.target_angle), (21, 180));
        let mut handle = Handle { angle: -90, ..Handle::default() };
        handle.activate(0.0, false);
        assert_eq!((handle.in_use, handle.target_angle), (12, -90));
        assert_eq!(handle.generated(), -32.0);
    }

    #[test]
    fn sneaking_turns_the_other_way_and_a_busy_handle_ignores_a_click() {
        let mut handle = Handle::default();
        handle.activate(0.0, true);
        assert_eq!(handle.generated(), -32.0);
        assert!(!handle.activate(0.0, false));
        let mut driven = Handle::default();
        assert!(!driven.activate(16.0, false));
    }

    #[test]
    fn the_cooldown_runs_down_after_the_handle_stops() {
        let mut handle = Handle::default();
        handle.activate(0.0, false);
        for _ in 0..7 {
            handle.tick();
        }
        assert_eq!(handle.in_use, 0);
        assert_eq!(handle.generated(), 0.0);
        // The cooldown only counts while the handle rests.
        assert_eq!(handle.cooldown, COOLDOWN - 1);
        for _ in 0..4 {
            handle.tick();
        }
        assert_eq!(handle.cooldown, 0);
        assert!(handle.activate(0.0, false));
    }

    #[test]
    fn the_angle_steps_round_both_ways() {
        assert_eq!(next_angle(45), 90);
        assert_eq!(next_angle(180), -45);
        assert_eq!(next_angle(-180), 45);
        assert_eq!(next_angle(7), 45);
    }

    #[test]
    fn the_drawn_handle_turns_from_its_start_to_its_target() {
        let mut handle = Handle::default();
        handle.activate(0.0, false);
        assert_eq!(handle.visual_angle(0.0, 1), 0.0);
        assert_eq!(handle.visual_angle(3.5, 1), 45.0);
        assert_eq!(handle.visual_angle(99.0, 1), 90.0);
        assert_eq!(handle.visual_angle(3.5, -1), -45.0);
        handle.in_use = 0;
        assert_eq!(handle.visual_angle(0.0, 1), 90.0);
    }
}
