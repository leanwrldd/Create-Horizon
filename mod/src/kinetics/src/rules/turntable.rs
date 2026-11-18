//! The Turntable's numbers (`TurntableBlock.entityInside`, `TurntableHandler`), from Create
//! `mc1.21.1/dev` at 4148fd8 (MIT, the Creators of Create). No game types.

/// Stress units per RPM (`AllBlocks.TURNTABLE`: `CStress.setImpact(4.0)`).
pub const IMPACT: f32 = 4.0;

/// Degrees an entity is turned each tick for a speed in RPM (`getSpeed() * 3 / 10`): the same
/// as a shaft's `rpm * 0.3` degrees a tick.
pub fn degrees_per_tick(rpm: f32) -> f32 {
    rpm * 3.0 / 10.0
}

/// `Mth.clamp(speed, -16, 16)`: the most a player is carried round in one tick.
pub fn carry_degrees(per_tick: f32) -> f32 {
    per_tick.clamp(-16.0, 16.0)
}

/// `VecHelper.rotate(vec, angle, Axis.Y)`: `x' = x cos + z sin`, `z' = z cos - x sin`.
pub fn rotate_y(v: [f64; 3], degrees: f64) -> [f64; 3] {
    let (sin, cos) = degrees.to_radians().sin_cos();
    [v[0] * cos + v[2] * sin, v[1], v[2] * cos - v[0] * sin]
}

/// What `TurntableBlock.entityInside` adds to a player's motion (it runs on the player's own
/// client in Create): the offset from the block's centre turned by [`carry_degrees`], less
/// where it was.
pub fn carry(offset_from_centre: [f64; 3], per_tick: f32) -> [f64; 3] {
    let turned = rotate_y(offset_from_centre, f64::from(carry_degrees(per_tick)));
    [
        turned[0] - offset_from_centre[0],
        turned[1] - offset_from_centre[1],
        turned[2] - offset_from_centre[2],
    ]
}

/// `entityInside`'s first three checks: standing on it (`onGround`), not rising
/// (`getDeltaMovement().y > 0`) and at or above the middle of the block (`getY() < pos.getY() + .5`).
pub fn stands_on(on_ground: bool, velocity_y: f64, entity_y: f64, block_y: i32) -> bool {
    on_ground && velocity_y <= 0.0 && entity_y >= f64::from(block_y) + 0.5
}

/// The turn a mob gets: its head and body to `head - speed`, its yaw to `yaw - speed`
/// (`livingEntity.setYBodyRot(diff)`, `setYHeadRot(diff)`, `e.setYRot(e.getYRot() - speed)`).
pub fn turned_yaws(head_yaw: f32, yaw: f32, per_tick: f32) -> (f32, f32) {
    (head_yaw - per_tick, yaw - per_tick)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turntable_turns_what_a_shaft_does() {
        assert!((degrees_per_tick(256.0) - 76.8).abs() < 1e-4);
        assert_eq!(degrees_per_tick(0.0), 0.0);
        assert!((degrees_per_tick(-16.0) + 4.8).abs() < 1e-5);
    }

    #[test]
    fn a_player_is_carried_at_most_sixteen_degrees_a_tick() {
        assert_eq!(carry_degrees(76.8), 16.0);
        assert_eq!(carry_degrees(-76.8), -16.0);
        assert_eq!(carry_degrees(4.8), 4.8);
    }

    #[test]
    fn rotating_about_y_follows_vec_helper() {
        // At 90 degrees: (x, y, z) -> (z, y, -x).
        let r = rotate_y([1.0, 0.5, 0.0], 90.0);
        assert!(r[0].abs() < 1e-12 && (r[1] - 0.5).abs() < 1e-12 && (r[2] + 1.0).abs() < 1e-12);
        let r = rotate_y([0.0, 0.0, 1.0], 90.0);
        assert!((r[0] - 1.0).abs() < 1e-12 && r[2].abs() < 1e-12);
    }

    #[test]
    fn a_player_at_the_centre_goes_nowhere() {
        assert_eq!(carry([0.0, 0.0, 0.0], 12.0), [0.0; 3]);
        let step = carry([0.4, 0.0, 0.0], 4.8);
        // The move is across the offset (about 0.4 * sin 4.8 degrees), never along it much.
        assert!(step[2] < 0.0 && step[0] < 0.0 && step[0].abs() < 0.01);
    }

    #[test]
    fn only_something_standing_on_the_middle_or_above_turns() {
        assert!(stands_on(true, 0.0, 65.0, 64));
        assert!(stands_on(true, -0.08, 64.5, 64));
        assert!(!stands_on(false, 0.0, 65.0, 64));
        assert!(!stands_on(true, 0.2, 65.0, 64));
        assert!(!stands_on(true, 0.0, 64.4, 64));
    }

    #[test]
    fn a_mob_is_turned_the_other_way_round_the_block() {
        assert_eq!(turned_yaws(90.0, 80.0, 4.8), (85.2, 75.2));
    }
}
