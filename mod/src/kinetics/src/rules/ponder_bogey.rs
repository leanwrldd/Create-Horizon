//! A bogey a Ponder scene animates: `world.animateBogey(pos, distance, duration)` (Create 6.0.10,
//! `CreateSceneBuilder.WorldInstructions.animateBogey`, `AnimateBlockEntityInstruction.bogey`). No game types.
//!
//! The instruction runs for `duration + 1` ticks and calls `AbstractBogeyBlockEntity.animate(distance /
//! (duration + 1))` on each of them but the last, which calls it with 0. So the bogey ends up
//! `distance * duration / (duration + 1)` along, not `distance`: the generator emits that as an ordinary
//! scripted number (`ponder_anim`) and the glue turns the number into the wheels' angle with [`wheel_angle`].
//!
//! `animate` turns the wheels by `360 * moved / (2 pi * wheelRadius)` backwards (`virtualAnimation = (value
//! - angleDiff) % 360`), a bogey's `getWheelRadius` being its size's (`BogeySizes.SMALL` 6.5 / 16,
//! `LARGE` 12.5 / 16).

use super::trains::bogey::{BogeySize, LARGE, SMALL};

/// `AbstractBogeyBlockEntity.animate(moved)` applied for `distance` blocks in all: the wheels' angle in degrees, 0 to 360.
pub fn wheel_angle(distance: f32, size: BogeySize) -> f32 {
    let turned = 360.0 * f64::from(distance) / (std::f64::consts::TAU * f64::from(size.wheel_radius));
    (-turned).rem_euclid(360.0) as f32
}

/// The size of the bogey block `block` (`create:small_bogey`, `create:large_bogey`), `None` for any other block.
pub fn size_of(block: &str) -> Option<BogeySize> {
    match block {
        "create:small_bogey" => Some(SMALL),
        "create:large_bogey" => Some(LARGE),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bogey_that_has_not_moved_has_its_wheels_where_they_began() {
        assert_eq!(wheel_angle(0.0, SMALL), 0.0);
    }

    #[test]
    fn one_wheel_circumference_is_a_full_turn_backwards() {
        let circumference = std::f32::consts::TAU * SMALL.wheel_radius;
        assert!(wheel_angle(circumference, SMALL).min(360.0 - wheel_angle(circumference, SMALL)) < 0.01);
        // A quarter of it turns the wheels 90 degrees the other way round: 270.
        assert!((wheel_angle(circumference / 4.0, SMALL) - 270.0).abs() < 0.01);
    }

    #[test]
    fn the_large_bogey_turns_less_for_the_same_distance() {
        assert!(wheel_angle(0.5, SMALL) != wheel_angle(0.5, LARGE));
        let small = 360.0 - wheel_angle(0.5, SMALL);
        let large = 360.0 - wheel_angle(0.5, LARGE);
        assert!(large < small);
    }

    #[test]
    fn a_negative_distance_turns_the_wheels_forwards() {
        let circumference = std::f32::consts::TAU * LARGE.wheel_radius;
        assert!((wheel_angle(-circumference / 4.0, LARGE) - 90.0).abs() < 0.01);
    }

    #[test]
    fn only_the_two_bogey_blocks_have_a_size() {
        assert_eq!(size_of("create:small_bogey"), Some(SMALL));
        assert_eq!(size_of("create:large_bogey"), Some(LARGE));
        assert_eq!(size_of("create:shaft"), None);
    }
}
