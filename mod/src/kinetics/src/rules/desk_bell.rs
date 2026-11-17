//! The Desk Bell's rules (`DeskBellBlock`, `DeskBellBlockEntity`, `DeskBellRenderer`): a click
//! rings it, powers it for 20 ticks and moves its plunger and bell. No game types: the game glue
//! is `machines/desk_bell.rs`.
//!
//! Known differences from Create, for the README:
//! - Create's `BellMovementBehaviour` rings a Desk Bell riding a contraption when an elevator
//!   arrives; contraptions here don't run movement behaviours for it.
//! - The pulse's timer is saved (`Timer`); Create's isn't, so a bell whose chunk unloads while
//!   it is powered stays powered there.

use crate::Dir;

/// `DeskBellBlockEntity.ding`: `blockStateTimer = 20`.
pub const POWERED_TICKS: i32 = 20;

/// `DeskBellBlockEntity.tick`: counts the timer down; true on the tick it reaches 0 (the bell is
/// let go, `unPress`). Answers the new timer.
pub fn tick(timer: i32) -> (i32, bool) {
    if timer <= 0 {
        return (0, false);
    }
    let next = timer - 1;
    (next, next == 0)
}

/// `DeskBellBlock.getSignal` and `getDirectSignal`: 15 in every direction while powered, and into
/// the block it stands on for a direct signal (`getConnectedDirection` is its `facing`).
pub fn signal(powered: bool, direct: bool, towards_support: bool) -> i32 {
    if powered && (!direct || towards_support) { 15 } else { 0 }
}

/// `WrenchableDirectionalBlock.getRotatedBlockState`: a quarter turn about the axis of the face
/// the Wrench was used on; a bell on that axis stays.
pub fn rotated(facing: Dir, targeted_face: Dir) -> Dir {
    if facing.axis() == targeted_face.axis() {
        facing
    } else {
        facing.clockwise_about(targeted_face.axis())
    }
}

/// `DeskBellBlockEntity.animation`: `startWithValue(1)` then `chase(0, 0.05, LINEAR)`: from 1 to 0
/// in 20 ticks. `age` is ticks since the ring, with the partial tick.
pub fn progress(age: f32) -> f32 {
    (1.0 - 0.05 * age).clamp(0.0, 1.0)
}

/// The renderer skips a bell this still that isn't powered (`p < 0.004`).
pub fn is_resting(p: f32, powered: bool) -> bool {
    p < 0.004 && !powered
}

/// What `DeskBellRenderer` moves, from the animation value `p`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    /// The plunger's drop, in blocks (negative: down).
    pub plunger: f32,
    /// The bell's tilt about x and about z, in degrees.
    pub tilt_x: f32,
    pub tilt_z: f32,
}

/// `DeskBellRenderer.renderSafe`. `offset` is the ring's random phase (`animationOffset`).
pub fn pose(p: f32, offset: f32) -> Pose {
    let f = 1.0 - 4.0 * ((p - 0.5).max(0.0) - 0.5).powi(2);
    let f2 = p.powf(1.25);
    let phase = p * std::f32::consts::PI * 4.0 + offset;
    Pose {
        plunger: f * -0.75 / 16.0,
        tilt_x: f2 * 8.0 * phase.sin(),
        tilt_z: f2 * 8.0 * phase.cos(),
    }
}

/// `animationOffset = random.nextFloat() * 2 * PI`, made from the game time of the ring so the
/// server, every client and a later reload agree on it.
pub fn animation_offset(since: i64) -> f32 {
    let mut x = (since as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 29;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 32;
    (x & 0xFF_FFFF) as f32 / 16_777_216.0 * 2.0 * std::f32::consts::PI
}

/// `AngleHelper.verticalAngle(facing)` plus the 90 the renderer adds: the partial models are
/// drawn for a bell standing up.
pub fn vertical_turn(facing: Dir) -> f32 {
    let vertical = match facing {
        Dir::Up => -90.0,
        Dir::Down => 90.0,
        _ => 0.0,
    };
    vertical + 90.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bell_stays_powered_for_twenty_ticks() {
        let mut timer = POWERED_TICKS;
        let mut ticks = 0;
        loop {
            let (next, release) = tick(timer);
            ticks += 1;
            timer = next;
            if release {
                break;
            }
        }
        assert_eq!(ticks, 20);
        assert_eq!(tick(0), (0, false));
    }

    #[test]
    fn it_gives_its_signal_like_a_lever() {
        assert_eq!(signal(true, false, false), 15);
        assert_eq!(signal(true, true, false), 0);
        assert_eq!(signal(true, true, true), 15);
        assert_eq!(signal(false, false, false), 0);
    }

    #[test]
    fn the_wrench_turns_about_the_face_it_is_used_on() {
        assert_eq!(rotated(Dir::Up, Dir::Up), Dir::Up);
        assert_eq!(rotated(Dir::Up, Dir::North), Dir::East);
        assert_eq!(rotated(Dir::North, Dir::Up), Dir::East);
    }

    #[test]
    fn the_animation_runs_down_over_twenty_ticks() {
        assert_eq!(progress(0.0), 1.0);
        assert!((progress(10.0) - 0.5).abs() < 1e-6);
        assert_eq!(progress(20.0), 0.0);
        assert_eq!(progress(30.0), 0.0);
        assert!(is_resting(0.0, false));
        assert!(!is_resting(0.0, true));
        assert!(!is_resting(0.5, false));
    }

    #[test]
    fn the_plunger_dips_and_returns() {
        // The plunger is down at the ring (p = 1), back up by half way, and stays up.
        let down = pose(1.0, 0.0).plunger;
        assert!((down + 0.75 / 16.0).abs() < 1e-6);
        assert!(pose(0.5, 0.0).plunger.abs() < 1e-6);
        assert!(pose(0.0, 0.0).plunger.abs() < 1e-6);
        // The bell stops swinging as it settles.
        assert_eq!(pose(0.0, 1.0).tilt_x, 0.0);
        assert!((pose(1.0, 0.0).tilt_z - 8.0).abs() < 1e-4);
    }

    #[test]
    fn the_offset_is_stable_and_within_a_turn() {
        assert_eq!(animation_offset(100), animation_offset(100));
        assert_ne!(animation_offset(100), animation_offset(101));
        for since in 0..50 {
            assert!((0.0..2.0 * std::f32::consts::PI).contains(&animation_offset(since)));
        }
    }

    #[test]
    fn the_partial_models_stand_up() {
        assert_eq!(vertical_turn(Dir::Up), 0.0);
        assert_eq!(vertical_turn(Dir::North), 90.0);
        assert_eq!(vertical_turn(Dir::Down), 180.0);
    }
}
