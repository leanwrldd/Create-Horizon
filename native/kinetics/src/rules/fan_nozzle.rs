//! The Nozzle: turns the current of the fan behind it into a push or pull in every direction.
//! Ported from Create's `NozzleBlock` and `NozzleBlockEntity` (MIT, the Creators of Create),
//! checked against Create `mc1.21.1/dev` at 4148fd8.
//!
//! Known differences from Create (for the README): a pull nozzle holding more than
//! [`MAX_PULLED`] entities discards them without the explosion, and players are pushed through
//! the client half (mod messages) like the fan's.

/// `NozzleBlockEntity.setLazyTickRate(5)`: ticks between looks for entities to push.
pub const LAZY_TICKS: i32 = 5;
/// `NozzleBlockEntity.lazyTick`: more pulled entities than this are removed.
pub const MAX_PULLED: usize = 256;
/// A pull leaves entities closer than this alone (`!pushing && distance < 1.5f`).
pub const PULL_DEAD_ZONE: f64 = 1.5;

/// `NozzleBlockEntity.calcRange`'s answer for a fan that isn't turning, or isn't there.
pub const NO_RANGE: f32 = 0.0;

/// Half the side of the box searched for entities (`new AABB(center, center).inflate(range / 2f)`).
pub fn search_half_extent(range: f32) -> f64 {
    f64::from(range / 2.0)
}

/// Whether an entity at `distance` from the nozzle's centre is let go
/// (`distance > range || entity.isShiftKeyDown() || isPlayerCreativeFlying`).
pub fn lets_go(distance: f64, range: f32, sneaking: bool, creative_flying: bool) -> bool {
    distance > f64::from(range) || sneaking || creative_flying
}

/// The change to an entity's velocity this tick, for one `diff` from the nozzle's centre to the
/// entity (`NozzleBlockEntity.tick`); `None` where the nozzle leaves it alone.
pub fn push(diff: [f64; 3], range: f32, pushing: bool, item: bool) -> Option<[f64; 3]> {
    let distance = (diff[0] * diff[0] + diff[1] * diff[1] + diff[2] * diff[2]).sqrt();
    if !pushing && distance < PULL_DEAD_ZONE as f64 {
        return None;
    }
    let factor = if item { 1.0 / 128.0 } else { 1.0 / 32.0 };
    // `diff.normalize()`: the zero vector stays zero.
    let scale = if distance > 0.0 {
        let sign = if pushing { 1.0 } else { -1.0 };
        (f64::from(range) - distance) * sign / distance
    } else {
        0.0
    };
    Some(diff.map(|v| v * scale * factor))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_push_is_strongest_at_the_nozzle() {
        let near = push([2.0, 0.0, 0.0], 10.0, true, false).unwrap();
        let far = push([8.0, 0.0, 0.0], 10.0, true, false).unwrap();
        assert!((near[0] - 8.0 / 32.0).abs() < 1e-12);
        assert!((far[0] - 2.0 / 32.0).abs() < 1e-12);
        assert_eq!(near[1], 0.0);
    }

    #[test]
    fn a_pull_goes_the_other_way_and_leaves_the_middle_alone() {
        let pull = push([0.0, 4.0, 0.0], 10.0, false, false).unwrap();
        assert!((pull[1] + 6.0 / 32.0).abs() < 1e-12);
        assert_eq!(push([0.0, 1.0, 0.0], 10.0, false, false), None);
        assert!(push([0.0, 1.0, 0.0], 10.0, true, false).is_some());
    }

    #[test]
    fn items_move_a_quarter_as_much() {
        let thing = push([4.0, 0.0, 0.0], 10.0, true, false).unwrap();
        let item = push([4.0, 0.0, 0.0], 10.0, true, true).unwrap();
        assert!((thing[0] / item[0] - 4.0).abs() < 1e-12);
    }

    #[test]
    fn entities_slip_away_out_of_range_or_sneaking() {
        assert!(lets_go(11.0, 10.0, false, false));
        assert!(lets_go(1.0, 10.0, true, false));
        assert!(lets_go(1.0, 10.0, false, true));
        assert!(!lets_go(10.0, 10.0, false, false));
    }

    #[test]
    fn a_push_at_the_exact_centre_is_zero() {
        assert_eq!(push([0.0; 3], 10.0, true, false), Some([0.0; 3]));
    }
}
