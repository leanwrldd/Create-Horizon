//! The Extendo Grip (`ExtendoGripItem`): a reach bonus while held (3 blocks for one, 5 for two, one in
//! each hand), more knockback, and wear that a worn Backtank can take instead. No game types.

pub const EXTENDO_GRIP: &str = "create:extendo_grip";
/// `ExtendoGripItem.MAX_DAMAGE`.
pub const MAX_DAMAGE: u32 = 200;
/// `singleRangeAttributeModifier` and `doubleRangeAttributeModifier`: `ADD_VALUE` to
/// `Attributes.BLOCK_INTERACTION_RANGE`.
pub const SINGLE_RANGE: f64 = 3.0;
pub const DOUBLE_RANGE: f64 = 5.0;
/// `Attributes.BLOCK_INTERACTION_RANGE`'s base for a player.
pub const BASE_REACH: f64 = 4.5;
/// `attacksByExtendoGripHaveMoreKnockback`: `event.setStrength(strength + 2)`.
pub const EXTRA_KNOCKBACK: f64 = 2.0;
/// `CEquipment.maxExtendoGripActions`: free actions a full Copper Backtank gives.
pub const MAX_ACTIONS: i32 = 1000;

/// How many grips are held: `holdingExtendo` is one hand xor the other, `holdingDualExtendo` both.
pub fn grips(main: bool, off: bool) -> u8 {
    u8::from(main) + u8::from(off)
}

/// The reach base a player holding that many grips should have.
pub fn wanted_base(grips: u8) -> f64 {
    match grips {
        0 => BASE_REACH,
        1 => BASE_REACH + SINGLE_RANGE,
        _ => BASE_REACH + DOUBLE_RANGE,
    }
}

/// What to set the reach base to, when it is one this item makes or the normal one (any other base is
/// something else's, and left alone): none when it is right already.
pub fn needs_update(base: f64, grips: u8) -> Option<f64> {
    let wanted = wanted_base(grips);
    let ours = [0u8, 1, 2].iter().any(|g| (base - wanted_base(*g)).abs() < 1e-6);
    (ours && (base - wanted).abs() > 1e-6).then_some(wanted)
}

/// `BacktankUtil.canAbsorbDamage`'s cost: `max(airInBacktank / usesPerTank, 1)`; free with no limit.
pub fn backtank_cost(max_air: i32, uses_per_tank: i32) -> Option<i32> {
    (uses_per_tank != 0).then(|| (max_air / uses_per_tank).max(1))
}

/// What one more use does to the grip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wear {
    Damaged(u32),
    Broken,
}

pub fn wear(damage: u32, uses: u32) -> Wear {
    let total = damage.saturating_add(uses);
    if total >= MAX_DAMAGE { Wear::Broken } else { Wear::Damaged(total) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_grip_adds_three_and_two_add_five() {
        assert_eq!(grips(true, false), 1);
        assert_eq!(grips(false, true), 1);
        assert_eq!(grips(true, true), 2);
        assert_eq!(grips(false, false), 0);
        assert_eq!(wanted_base(0), 4.5);
        assert_eq!(wanted_base(1), 7.5);
        assert_eq!(wanted_base(2), 9.5);
    }

    #[test]
    fn the_base_is_only_changed_when_it_is_ours_and_wrong() {
        assert_eq!(needs_update(4.5, 1), Some(7.5));
        assert_eq!(needs_update(7.5, 2), Some(9.5));
        assert_eq!(needs_update(9.5, 0), Some(4.5));
        assert_eq!(needs_update(7.5, 1), None);
        assert_eq!(needs_update(4.5, 0), None);
        // A base something else set stays.
        assert_eq!(needs_update(6.0, 1), None);
    }

    #[test]
    fn a_backtank_takes_the_use_at_a_share_of_its_air() {
        assert_eq!(backtank_cost(900, 1000), Some(1));
        assert_eq!(backtank_cost(900, 10), Some(90));
        assert_eq!(backtank_cost(900, 0), None);
    }

    #[test]
    fn the_grip_breaks_after_two_hundred_uses() {
        assert_eq!(wear(0, 1), Wear::Damaged(1));
        assert_eq!(wear(198, 1), Wear::Damaged(199));
        assert_eq!(wear(199, 1), Wear::Broken);
        assert_eq!(wear(150, 100), Wear::Broken);
    }
}
