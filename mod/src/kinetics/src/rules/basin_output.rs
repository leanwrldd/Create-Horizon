//! What a Basin shows of the stacks it just gave out (`BasinRenderer.renderSafe`'s `visualizedOutputItems`, Create 6.0.10):
//! each flies out of the spout side for `OUTPUT_ANIMATION_TIME` ticks, falling and turning over. No game types.

use super::crafter::horizontal_angle;
use crate::Dir;

/// `BasinBlockEntity.OUTPUT_ANIMATION_TIME`.
pub const ANIMATION_TICKS: f32 = 10.0;

/// A stack's place and turn on its way out: translations in order (the spout's mouth, the fall, the push out), then the
/// turn about the vertical axis (`horizontalAngle(facing)`) and about the horizontal one (`progress * 180`), in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flight {
    pub mouth: [f32; 3],
    pub fall: f32,
    pub out: [f32; 3],
    pub yaw: f32,
    pub roll: f32,
}

/// How far along an animation that began `age` ticks ago the stack is, 0 to 1 (`1 - (remaining - partial) / 10`); `None`
/// once it is over or before it began.
pub fn progress(age: f32) -> Option<f32> {
    let progress = age / ANIMATION_TICKS;
    (0.0..1.0).contains(&progress).then_some(progress)
}

/// Where the stack is at `progress`, for a Basin whose spout faces `facing`; `None` for a Basin that empties downwards
/// (it draws nothing) and, unless `to_basin` (the block there is a Basin too), once the stack is past `.35`.
pub fn flight(facing: Dir, progress: f32, to_basin: bool) -> Option<Flight> {
    if facing == Dir::Down || facing == Dir::Up || (!to_basin && progress > 0.35) {
        return None;
    }
    let normal = facing.normal().map(|v| v as f32);
    Some(Flight {
        mouth: [0.5 + normal[0] * 0.55, 0.0, 0.5 + normal[2] * 0.55],
        fall: (-(progress * progress * 2.0)).max(-0.55),
        out: normal.map(|v| v * progress * 0.5),
        yaw: horizontal_angle(facing) as f32,
        roll: progress * 180.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_animation_lasts_ten_ticks() {
        assert_eq!(progress(-1.0), None);
        assert_eq!(progress(0.0), Some(0.0));
        assert_eq!(progress(5.0), Some(0.5));
        assert_eq!(progress(10.0), None);
    }

    #[test]
    fn a_stack_falls_out_of_the_spout_and_a_downward_basin_shows_nothing() {
        let start = flight(Dir::North, 0.0, false).unwrap();
        assert_eq!(start.mouth, [0.5, 0.0, 0.5 - 0.55]);
        assert_eq!(start.fall, 0.0);
        assert_eq!(start.yaw, 180.0);
        let later = flight(Dir::North, 0.3, false).unwrap();
        assert!((later.fall + 0.18).abs() < 1e-6);
        assert!((later.out[2] + 0.15).abs() < 1e-6);
        assert!((later.roll - 54.0).abs() < 1e-3);
        assert_eq!(flight(Dir::Down, 0.1, true), None);
    }

    #[test]
    fn only_a_stack_going_into_another_basin_is_seen_past_a_third_of_the_way() {
        assert_eq!(flight(Dir::East, 0.4, false), None);
        assert!(flight(Dir::East, 0.4, true).is_some());
        // The fall stops at `-.55`.
        assert_eq!(flight(Dir::East, 0.9, true).unwrap().fall, -0.55);
    }
}
