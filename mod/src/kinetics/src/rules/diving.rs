//! The Copper and Netherite Diving Helmets and Boots: `DivingHelmetItem.breatheUnderwater`,
//! `DivingBootsItem` and `NetheriteDivingHandler`, with no game types. What the helmet does with the
//! air of a backtank, who is fire immune, and the boots' sinking.

pub const COPPER_HELMET: &str = "create:copper_diving_helmet";
pub const NETHERITE_HELMET: &str = "create:netherite_diving_helmet";
pub const COPPER_BOOTS: &str = "create:copper_diving_boots";
pub const NETHERITE_BOOTS: &str = "create:netherite_diving_boots";
pub const NETHERITE_BACKTANK: &str = "create:netherite_backtank";
/// `Entity.getMaxAirSupply`: what a breath refills to.
pub const MAX_AIR_SUPPLY: i32 = 300;
/// `level.getGameTime() % 20 == 0`: a tank gives up one air a second.
pub const CONSUME_PERIOD: u32 = 20;
/// `DivingBootsItem.accelerateDescentUnderwater`: the extra pull down each tick, in blocks a tick.
pub const SINK: f32 = 0.05;

pub fn is_diving_helmet(id: &str) -> bool {
    id == COPPER_HELMET || id == NETHERITE_HELMET
}

pub fn is_diving_boots(id: &str) -> bool {
    id == COPPER_BOOTS || id == NETHERITE_BOOTS
}

/// `NetheriteDivingHandler.isNetheriteArmor`: armour with `FIRE_RESISTANT`, which in vanilla is
/// every netherite piece (and Create's).
pub fn is_netherite_armor(id: &str) -> bool {
    (id.starts_with("minecraft:netherite_") || id.starts_with("create:netherite_"))
        && ["_helmet", "_chestplate", "_leggings", "_boots", "_backtank"].iter().any(|s| id.ends_with(s))
}

/// `NetheriteDivingHandler`'s four bits: a netherite diving helmet, a netherite backtank with air, and
/// netherite leggings and boots. `chest` is the worn chest item and its air.
pub fn full_netherite_set(helmet: Option<&str>, chest: Option<(&str, i32)>, legs: Option<&str>, feet: Option<&str>) -> bool {
    helmet == Some(NETHERITE_HELMET)
        && chest.is_some_and(|(id, air)| id == NETHERITE_BACKTANK && air > 0)
        && legs.is_some_and(is_netherite_armor)
        && feet.is_some_and(is_netherite_armor)
}

/// The damage a fully dressed player doesn't take (`setFireImmune`: burning, lava, fire, hot floors).
pub fn is_fire_damage(damage_type: &str) -> bool {
    matches!(
        damage_type,
        "minecraft:in_fire" | "minecraft:on_fire" | "minecraft:lava" | "minecraft:hot_floor"
    )
}

/// What the helmet does about breathing (`breatheUnderwater` for water).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Breath {
    pub can_breathe: bool,
    /// How much air this tick refills (the maximum), when the helmet took over.
    pub refill: Option<i32>,
    /// Whether the tank gives one air this tick (every `CONSUME_PERIOD`th).
    pub uses_air: bool,
}

/// `can_breathe` is what the game found, `helmet` whether a diving helmet is worn, `tank_air` the air
/// of the tank that would be used (0 for none) and `counter` how many ticks the helmet has been
/// breathing for this player.
pub fn breathe(can_breathe: bool, helmet: bool, tank_air: i32, counter: u32) -> Breath {
    if !helmet || can_breathe || tank_air <= 0 {
        return Breath { can_breathe, refill: None, uses_air: false };
    }
    Breath { can_breathe: true, refill: Some(MAX_AIR_SUPPLY), uses_air: counter % CONSUME_PERIOD == 0 }
}

/// `breatheUnderwater` in lava: a netherite helmet and a netherite tank with air drain one air a
/// second (nothing refills).
pub fn lava_drain(helmet: Option<&str>, chest: Option<(&str, i32)>, in_lava: bool, counter: u32) -> bool {
    in_lava
        && helmet == Some(NETHERITE_HELMET)
        && chest.is_some_and(|(id, air)| id == NETHERITE_BACKTANK && air > 0)
        && counter % CONSUME_PERIOD == 0
}

/// `DivingBootsItem.affects`: the boots pull a player down while in water, not swimming or flying.
pub fn sinks(boots: bool, in_water: bool, swimming: bool, flying: bool) -> bool {
    boots && in_water && !swimming && !flying
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_helmet_lets_a_player_breathe_while_a_tank_has_air() {
        let b = breathe(false, true, 100, 1);
        assert!(b.can_breathe);
        assert_eq!(b.refill, Some(300));
        assert!(!b.uses_air);
        assert!(breathe(false, true, 100, 20).uses_air);
        assert!(breathe(false, true, 100, 0).uses_air);
    }

    #[test]
    fn nothing_changes_without_a_helmet_or_air_or_when_the_player_can_breathe() {
        for (can, helmet, air) in [(false, false, 100), (false, true, 0), (true, true, 100)] {
            let b = breathe(can, helmet, air, 0);
            assert_eq!(b, Breath { can_breathe: can, refill: None, uses_air: false });
        }
    }

    #[test]
    fn a_full_netherite_set_needs_all_four_pieces_and_air() {
        let full = |c: i32| {
            full_netherite_set(
                Some(NETHERITE_HELMET),
                Some((NETHERITE_BACKTANK, c)),
                Some("minecraft:netherite_leggings"),
                Some("create:netherite_diving_boots"),
            )
        };
        assert!(full(1));
        assert!(!full(0));
        let boots = Some("minecraft:netherite_boots");
        let legs = Some("minecraft:netherite_leggings");
        assert!(!full_netherite_set(Some(COPPER_HELMET), Some((NETHERITE_BACKTANK, 5)), legs, boots));
        assert!(!full_netherite_set(Some(NETHERITE_HELMET), Some(("create:copper_backtank", 5)), legs, boots));
        assert!(!full_netherite_set(Some(NETHERITE_HELMET), Some((NETHERITE_BACKTANK, 5)), Some("minecraft:diamond_leggings"), boots));
        assert!(!full_netherite_set(Some(NETHERITE_HELMET), Some((NETHERITE_BACKTANK, 5)), None, boots));
    }

    #[test]
    fn netherite_armour_is_by_name() {
        assert!(is_netherite_armor("minecraft:netherite_boots"));
        assert!(is_netherite_armor("create:netherite_backtank"));
        assert!(!is_netherite_armor("minecraft:netherite_sword"));
        assert!(!is_netherite_armor("create:copper_diving_boots"));
    }

    #[test]
    fn fire_damage_is_what_burns() {
        assert!(is_fire_damage("minecraft:lava"));
        assert!(is_fire_damage("minecraft:in_fire"));
        assert!(!is_fire_damage("minecraft:drown"));
    }

    #[test]
    fn lava_diving_drains_a_netherite_tank_each_second() {
        let tank = Some((NETHERITE_BACKTANK, 10));
        assert!(lava_drain(Some(NETHERITE_HELMET), tank, true, 20));
        assert!(!lava_drain(Some(NETHERITE_HELMET), tank, true, 7));
        assert!(!lava_drain(Some(NETHERITE_HELMET), tank, false, 20));
        assert!(!lava_drain(Some(COPPER_HELMET), tank, true, 20));
        assert!(!lava_drain(Some(NETHERITE_HELMET), Some(("create:copper_backtank", 10)), true, 20));
    }

    #[test]
    fn the_boots_sink_a_player_in_water_unless_swimming_or_flying() {
        assert!(sinks(true, true, false, false));
        assert!(!sinks(false, true, false, false));
        assert!(!sinks(true, false, false, false));
        assert!(!sinks(true, true, true, false));
        assert!(!sinks(true, true, false, true));
        assert!(is_diving_boots(COPPER_BOOTS) && is_diving_helmet(NETHERITE_HELMET));
    }
}
