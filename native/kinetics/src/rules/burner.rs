//! The Blaze Burner's fuel and heat. Ported from Create's `BlazeBurnerBlockEntity`,
//! `BlazeBurnerBlock`, `LitBlazeBurnerBlock` and `BlazeBurnerHandler` (MIT, the Creators of
//! Create), checked against `mc1.21.1/dev` at 4148fd8.
//!
//! No game types: the glue (`native/api/src/machines/burner.rs`) reads the stacks and sets the
//! block's `blaze` state.

use super::basin::HeatLevel;

/// `BlazeBurnerBlockEntity.MAX_HEAT_CAPACITY`.
pub const MAX_HEAT_CAPACITY: i32 = 10_000;
/// `BlazeBurnerBlockEntity.INSERTION_THRESHOLD`: fuel of the kind burning is only taken once the
/// time left has fallen to this.
pub const INSERTION_THRESHOLD: i32 = 500;
/// `AllItemTags.BLAZE_BURNER_FUEL_SPECIAL` (and `create:superheated_blaze_burner_fuels` for the
/// Blaze Cake): 3200 ticks of superheated burning.
pub const SPECIAL_BURN_TIME: i32 = 3200;
/// What a regular fuel in `AllItemTags.BLAZE_BURNER_FUEL_REGULAR` burns for ("same as coal").
pub const REGULAR_TAG_BURN_TIME: i32 = 1600;
/// `BlazeBurnerHandler.thrownEggsGetEatenByBurner`: an egg adds 80 ticks.
pub const EGG_BURN_TIME: i32 = 80;

/// `BlazeBurnerBlockEntity.FuelType`; the ordinal is saved as `fuelLevel`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FuelType {
    None,
    Normal,
    Special,
}

impl FuelType {
    pub fn ordinal(self) -> i32 {
        self as i32
    }

    /// `FuelType.values()[n]`; anything out of range reads as none.
    pub fn from_ordinal(n: i32) -> Self {
        match n {
            1 => FuelType::Normal,
            2 => FuelType::Special,
            _ => FuelType::None,
        }
    }
}

/// The burner block entity's fuel state: `fuelLevel`, `burnTimeRemaining` and `isCreative`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Burner {
    pub fuel: FuelType,
    pub remaining: i32,
    pub creative: bool,
}

impl Default for Burner {
    fn default() -> Self {
        Burner {
            fuel: FuelType::None,
            remaining: 0,
            creative: false,
        }
    }
}

impl Burner {
    /// `getHeatLevel`: smouldering with no fuel, seething on special fuel, and kindled on normal
    /// fuel until under 1.25 percent of the capacity is left, when it is fading.
    pub fn heat_level(&self) -> HeatLevel {
        match self.fuel {
            FuelType::Special => HeatLevel::Seething,
            FuelType::Normal => {
                if (self.remaining as f64) / (MAX_HEAT_CAPACITY as f64) < 0.0125 {
                    HeatLevel::Fading
                } else {
                    HeatLevel::Kindled
                }
            }
            FuelType::None => HeatLevel::Smouldering,
        }
    }

    /// One server tick (`BlazeBurnerBlockEntity.tick`). Answers the heat the block's state should
    /// take now (`updateBlockState`), or `None` when the tick leaves it alone.
    pub fn tick(&mut self) -> Option<HeatLevel> {
        if self.creative {
            return None;
        }
        if self.remaining > 0 {
            self.remaining -= 1;
        }
        if self.fuel == FuelType::Normal && self.remaining > 0 {
            return Some(self.heat_level());
        }
        if self.remaining > 0 {
            return None;
        }
        if self.fuel == FuelType::Special {
            self.fuel = FuelType::Normal;
            self.remaining = MAX_HEAT_CAPACITY / 2;
        } else {
            self.fuel = FuelType::None;
        }
        Some(self.heat_level())
    }

    /// `tryUpdateFuel`: whether the burner takes an item of `fuel` kind that burns `burn_time`
    /// ticks (and so consumes it). `force_overflow` lets normal fuel top the heat up to the
    /// capacity (a player's use, not a fake player's). Unless `simulate`, the fuel is taken.
    pub fn try_update_fuel(
        &mut self,
        fuel: FuelType,
        burn_time: i32,
        force_overflow: bool,
        simulate: bool,
    ) -> bool {
        if self.creative || fuel == FuelType::None {
            return false;
        }
        if fuel < self.fuel {
            return false;
        }
        let mut new_time = burn_time;
        if fuel == self.fuel {
            if self.remaining <= INSERTION_THRESHOLD {
                new_time += self.remaining;
            } else if force_overflow && fuel == FuelType::Normal {
                new_time = if self.remaining < MAX_HEAT_CAPACITY {
                    (self.remaining + new_time).min(MAX_HEAT_CAPACITY)
                } else {
                    self.remaining
                };
            } else {
                return false;
            }
        }
        if simulate {
            return true;
        }
        self.fuel = fuel;
        self.remaining = new_time;
        true
    }

    /// `applyCreativeFuel`: a Creative Blaze Cake makes the burner endless and steps its heat up
    /// (skipping fading). Answers the heat the block takes, from the one it has.
    pub fn apply_creative_fuel(&mut self, current: HeatLevel) -> HeatLevel {
        self.fuel = FuelType::None;
        self.remaining = 0;
        self.creative = true;
        let mut next = current.next_active_level();
        if next == HeatLevel::Fading {
            next = next.next_active_level();
        }
        next
    }

    /// `thrownEggsGetEatenByBurner`: an egg adds 80 ticks of normal fuel, unless special fuel
    /// burns or the burner is creative.
    pub fn eat_egg(&mut self) {
        if self.creative || self.fuel == FuelType::Special {
            return;
        }
        self.fuel = FuelType::Normal;
        self.remaining = (self.remaining + EGG_BURN_TIME).clamp(0, MAX_HEAT_CAPACITY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heat_follows_the_fuel() {
        let mut b = Burner::default();
        assert_eq!(b.heat_level(), HeatLevel::Smouldering);
        b.fuel = FuelType::Normal;
        b.remaining = 1600;
        assert_eq!(b.heat_level(), HeatLevel::Kindled);
        b.remaining = 124; // 1.24 percent
        assert_eq!(b.heat_level(), HeatLevel::Fading);
        b.remaining = 125;
        assert_eq!(b.heat_level(), HeatLevel::Kindled);
        b.fuel = FuelType::Special;
        assert_eq!(b.heat_level(), HeatLevel::Seething);
    }

    #[test]
    fn coal_into_an_empty_burner_burns_1600_ticks() {
        let mut b = Burner::default();
        assert!(b.try_update_fuel(FuelType::Normal, 1600, true, false));
        assert_eq!((b.fuel, b.remaining), (FuelType::Normal, 1600));
        // More of the same is refused until it is nearly out, unless overflowing.
        assert!(!b.try_update_fuel(FuelType::Normal, 1600, false, true));
        assert!(b.try_update_fuel(FuelType::Normal, 1600, true, false));
        assert_eq!(b.remaining, 3200);
        // Overflow stops at the capacity.
        b.remaining = 9000;
        assert!(b.try_update_fuel(FuelType::Normal, 1600, true, false));
        assert_eq!(b.remaining, MAX_HEAT_CAPACITY);
        assert!(b.try_update_fuel(FuelType::Normal, 1600, true, false));
        assert_eq!(b.remaining, MAX_HEAT_CAPACITY);
    }

    #[test]
    fn nearly_out_fuel_adds_up() {
        let mut b = Burner {
            fuel: FuelType::Normal,
            remaining: 400,
            creative: false,
        };
        assert!(b.try_update_fuel(FuelType::Normal, 1600, false, false));
        assert_eq!(b.remaining, 2000);
    }

    #[test]
    fn special_fuel_replaces_normal_and_normal_cannot_replace_special() {
        let mut b = Burner {
            fuel: FuelType::Normal,
            remaining: 5000,
            creative: false,
        };
        assert!(b.try_update_fuel(FuelType::Special, 3200, true, false));
        assert_eq!((b.fuel, b.remaining), (FuelType::Special, 3200));
        assert!(!b.try_update_fuel(FuelType::Normal, 1600, true, false));
        // Special on special without overflow, above the threshold: refused.
        assert!(!b.try_update_fuel(FuelType::Special, 3200, true, false));
    }

    #[test]
    fn a_simulation_changes_nothing() {
        let mut b = Burner::default();
        assert!(b.try_update_fuel(FuelType::Normal, 1600, true, true));
        assert_eq!(b, Burner::default());
    }

    #[test]
    fn special_fuel_runs_out_into_half_a_tank_of_normal_and_then_into_nothing() {
        let mut b = Burner {
            fuel: FuelType::Special,
            remaining: 1,
            creative: false,
        };
        assert_eq!(b.tick(), Some(HeatLevel::Kindled));
        assert_eq!((b.fuel, b.remaining), (FuelType::Normal, 5000));
        b.remaining = 1;
        assert_eq!(b.tick(), Some(HeatLevel::Smouldering));
        assert_eq!(b.fuel, FuelType::None);
        assert_eq!(b.tick(), Some(HeatLevel::Smouldering));
    }

    #[test]
    fn normal_fuel_counts_down_and_reports_its_heat() {
        let mut b = Burner {
            fuel: FuelType::Normal,
            remaining: 200,
            creative: false,
        };
        assert_eq!(b.tick(), Some(HeatLevel::Kindled));
        assert_eq!(b.remaining, 199);
        let mut idle = Burner::default();
        assert_eq!(idle.tick(), Some(HeatLevel::Smouldering));
    }

    #[test]
    fn a_creative_burner_never_burns_down() {
        let mut b = Burner::default();
        assert_eq!(b.apply_creative_fuel(HeatLevel::Smouldering), HeatLevel::Kindled);
        assert!(b.creative);
        assert_eq!(b.tick(), None);
        assert!(!b.try_update_fuel(FuelType::Normal, 1600, true, false));
        assert_eq!(b.apply_creative_fuel(HeatLevel::Kindled), HeatLevel::Seething);
    }

    #[test]
    fn eggs_feed_a_burner_but_not_one_on_special_fuel() {
        let mut b = Burner::default();
        b.eat_egg();
        assert_eq!((b.fuel, b.remaining), (FuelType::Normal, 80));
        let mut special = Burner {
            fuel: FuelType::Special,
            remaining: 100,
            creative: false,
        };
        special.eat_egg();
        assert_eq!(special.remaining, 100);
    }

    #[test]
    fn fuel_types_round_trip_through_their_ordinal() {
        for fuel in [FuelType::None, FuelType::Normal, FuelType::Special] {
            assert_eq!(FuelType::from_ordinal(fuel.ordinal()), fuel);
        }
        assert_eq!(FuelType::from_ordinal(9), FuelType::None);
    }
}
