//! The Copper and Netherite Backtanks: `BacktankUtil` (air in a worn tank), `BacktankItem` (its bar)
//! and `BacktankBlockEntity.tick` (a placed tank filling from rotation), with no game types.

pub const COPPER: &str = "create:copper_backtank";
pub const NETHERITE: &str = "create:netherite_backtank";
/// `AllDataComponents.BACKTANK_AIR`.
pub const COMPONENT: &str = "create:backtank_air";
/// `CEquipment.airInBacktank`: seconds of air in a full tank.
pub const AIR_IN_BACKTANK: i32 = 900;
/// `CEquipment.enchantedBacktankCapacity`: what a level of the Capacity enchantment adds.
pub const ENCHANTED_CAPACITY: i32 = 300;
/// `BacktankItem.BAR_COLOR`.
pub const BAR_COLOR: u32 = 0xEF_EFEF;
/// `BuilderTransformers.backtank`: `CStress.setImpact(4.0)`.
pub const IMPACT: f32 = 4.0;

pub fn is_backtank(id: &str) -> bool {
    id == COPPER || id == NETHERITE
}

/// `BacktankUtil.maxAir(int)`.
pub fn max_air(enchant_level: i32) -> i32 {
    AIR_IN_BACKTANK + ENCHANTED_CAPACITY * enchant_level
}

/// `BacktankUtil.getAir`: the saved air, never over the tank's capacity.
pub fn air_of(saved: i32, max: i32) -> i32 {
    saved.min(max)
}

/// `BacktankItem.getBarWidth`: 13 pixels of bar.
pub fn bar_width(air: i32, max: i32) -> i32 {
    (13.0 * (air as f32 / max as f32).clamp(0.0, 1.0)).round() as i32
}

/// `BacktankUtil.consumeAir`: the air left after using `n`.
pub fn consume(air: i32, max: i32, n: i32) -> i32 {
    (air.min(max) - n).max(0).min(max)
}

/// The two warnings `BacktankUtil.sendWarning` gives, at a tenth of the tank and at 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Warning {
    Low,
    Depleted,
}

/// What the air going from `air` to `new_air` warns of: crossing down to a tenth of the capacity
/// (`backtank.low`) or to 1 (`backtank.depleted`); both when one use does both.
pub fn warnings(air: i32, new_air: i32, max: i32) -> Vec<Warning> {
    let mut out = Vec::new();
    let low = max as f32 / 10.0;
    if new_air as f32 <= low && air as f32 > low {
        out.push(Warning::Low);
    }
    if new_air <= 1 && air > 1 {
        out.push(Warning::Depleted);
    }
    out
}

/// `BacktankBlockEntity.tick` for a turning tank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fill {
    pub air: i32,
    pub timer: i32,
}

/// One server tick of a placed tank: at `speed` RPM it adds 1 to 5 air every 0 to 20 ticks
/// (faster turning adds more, more often).
pub fn fill_tick(speed: f32, air: i32, timer: i32, max: i32) -> Fill {
    if speed == 0.0 {
        return Fill { air, timer };
    }
    if timer > 0 {
        return Fill { air, timer: timer - 1 };
    }
    if air == max {
        return Fill { air, timer };
    }
    let abs = speed.abs();
    let increment = (((abs as i32) - 100) / 20).clamp(1, 5);
    Fill {
        air: max.min(air + increment),
        timer: (((128.0 - abs / 5.0) as i32) - 108).clamp(0, 20),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_tank_holds_900_and_a_capacity_level_adds_300() {
        assert_eq!(max_air(0), 900);
        assert_eq!(max_air(3), 1800);
        assert_eq!(air_of(5000, 900), 900);
        assert_eq!(air_of(100, 900), 100);
    }

    #[test]
    fn the_bar_is_thirteen_pixels() {
        assert_eq!(bar_width(900, 900), 13);
        assert_eq!(bar_width(0, 900), 0);
        assert_eq!(bar_width(450, 900), 7);
        assert_eq!(bar_width(5000, 900), 13);
    }

    #[test]
    fn using_air_never_goes_below_nothing() {
        assert_eq!(consume(10, 900, 1), 9);
        assert_eq!(consume(0, 900, 5), 0);
        assert_eq!(consume(2000, 900, 1), 899);
    }

    #[test]
    fn it_warns_when_a_tenth_is_left_and_when_it_is_empty() {
        assert_eq!(warnings(91, 90, 900), vec![Warning::Low]);
        assert!(warnings(90, 89, 900).is_empty());
        assert_eq!(warnings(2, 1, 900), vec![Warning::Depleted]);
        // A tank of 10: 1 is both a tenth and empty.
        assert_eq!(warnings(2, 1, 10), vec![Warning::Low, Warning::Depleted]);
        assert!(warnings(1, 0, 900).is_empty());
    }

    #[test]
    fn a_still_tank_does_not_fill() {
        assert_eq!(fill_tick(0.0, 10, 0, 900), Fill { air: 10, timer: 0 });
    }

    #[test]
    fn a_slow_tank_adds_one_and_waits_and_a_fast_one_adds_five() {
        // 16 RPM: the increment clamps to 1; the timer is (int)(128 - 3.2) - 108 = 16.
        assert_eq!(fill_tick(16.0, 0, 0, 900), Fill { air: 1, timer: 16 });
        // 256 RPM: (256 - 100) / 20 = 7 clamps to 5; the timer is (int)(128 - 51.2) - 108 < 0, so 0.
        let fast = fill_tick(256.0, 0, 0, 900);
        assert_eq!(fast, Fill { air: 5, timer: 0 });
        // The direction doesn't matter.
        assert_eq!(fill_tick(-256.0, 0, 0, 900), fast);
    }

    #[test]
    fn the_timer_counts_down_before_the_next_fill() {
        assert_eq!(fill_tick(16.0, 5, 3, 900), Fill { air: 5, timer: 2 });
    }

    #[test]
    fn a_full_tank_stops_and_the_last_fill_stops_at_the_top() {
        assert_eq!(fill_tick(64.0, 900, 0, 900), Fill { air: 900, timer: 0 });
        assert_eq!(fill_tick(256.0, 898, 0, 900).air, 900);
    }

    #[test]
    fn the_two_tanks_are_tanks() {
        assert!(is_backtank(COPPER) && is_backtank(NETHERITE));
        assert!(!is_backtank("create:copper_diving_helmet"));
    }
}
