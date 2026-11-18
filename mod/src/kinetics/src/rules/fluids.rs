//! Create's fluids and the items that carry them: what a bucket or a bottle holds, and the
//! fluid tags. No game types.
//!
//! Ported from Create `AllFluids` (the fluids and their numbers), `content/fluids/transfer/
//! GenericItemEmptying` and `GenericItemFilling` (what an item gives and takes), the datagen
//! recipes `emptying/honey_bottle`, `emptying/builders_tea`, `filling/honey_bottle`,
//! `filling/builders_tea`, and `foundation/fluid/FluidHelper` (the sounds).
//!
//! Known differences from Create: potions (`create:potion` and its `PotionFluidHandler`) and
//! the fluid containers other mods add are not handled; only the vanilla and Create containers
//! below.

/// `AllFluids.HONEY` and `CHOCOLATE`: `(id, viscosity, tint)`; both density 1400,
/// `levelDecreasePerBlock(2)`, `tickRate(25)`, `slopeFindDistance(3)`, `explosionResistance(100)`.
pub const HONEY: &str = "create:honey";
pub const CHOCOLATE: &str = "create:chocolate";
pub const TEA: &str = "create:tea";
pub const POTION: &str = "create:potion";
/// NeoForge's milk, as the basin takes it.
pub const MILK: &str = "neoforge:milk";
pub const WATER: &str = "minecraft:water";
pub const LAVA: &str = "minecraft:lava";

pub const HONEY_VISCOSITY: i32 = 2000;
pub const CHOCOLATE_VISCOSITY: i32 = 1500;
pub const DENSITY: i32 = 1400;
pub const LEVEL_DECREASE_PER_BLOCK: u32 = 2;
pub const TICK_RATE: u32 = 25;
pub const SLOPE_FIND_DISTANCE: u32 = 3;
pub const EXPLOSION_RESISTANCE: f32 = 100.0;

/// A bucket's worth, in mB.
pub const BUCKET: i32 = 1000;
/// A bottle's worth: the amount of `emptying/honey_bottle` and `emptying/builders_tea`.
pub const BOTTLE: i32 = 250;

/// An item that holds fluid: what it is full, what it is empty, and how much.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Container {
    pub full: &'static str,
    pub empty: &'static str,
    pub fluid: &'static str,
    pub amount: i32,
}

/// What `GenericItemEmptying` and `GenericItemFilling` know of: the vanilla buckets
/// (`FluidBucketWrapper`), Create's, and the two bottles of the datagen recipes.
pub const CONTAINERS: [Container; 7] = [
    Container { full: "minecraft:water_bucket", empty: "minecraft:bucket", fluid: WATER, amount: BUCKET },
    Container { full: "minecraft:lava_bucket", empty: "minecraft:bucket", fluid: LAVA, amount: BUCKET },
    Container { full: "minecraft:milk_bucket", empty: "minecraft:bucket", fluid: MILK, amount: BUCKET },
    Container { full: "create:honey_bucket", empty: "minecraft:bucket", fluid: HONEY, amount: BUCKET },
    Container { full: "create:chocolate_bucket", empty: "minecraft:bucket", fluid: CHOCOLATE, amount: BUCKET },
    Container { full: "minecraft:honey_bottle", empty: "minecraft:glass_bottle", fluid: HONEY, amount: BOTTLE },
    Container { full: "create:builders_tea", empty: "minecraft:glass_bottle", fluid: TEA, amount: BOTTLE },
];

/// `GenericItemEmptying.canItemBeEmptied` and `emptyItem`: what a held item gives.
pub fn emptied(item: &str) -> Option<&'static Container> {
    CONTAINERS.iter().find(|c| c.full == item)
}

/// `GenericItemFilling.getRequiredAmountForItem`: what filling a held item with `fluid` takes
/// (the fluid holds at least that much). Milk buckets are never filled (`canItemBeFilled`).
pub fn filled(item: &str, fluid: &str) -> Option<&'static Container> {
    CONTAINERS
        .iter()
        .find(|c| c.empty == item && c.fluid == fluid && c.full != "minecraft:milk_bucket")
}

/// Whether the item is one of the containers, full or empty (`canItemBeEmptied` or
/// `canItemBeFilled`): a click with it is the tank's, whether or not it did anything.
pub fn is_container(item: &str) -> bool {
    CONTAINERS.iter().any(|c| c.full == item || c.empty == item)
}

/// `FluidHelper.getEmptySound`: the sound of a fluid put into a tank.
pub fn empty_sound(fluid: &str) -> &'static str {
    if fluid == LAVA { "minecraft:item.bucket.empty_lava" } else { "minecraft:item.bucket.empty" }
}

/// `FluidHelper.getFillSound`: the sound of a fluid taken out of a tank.
pub fn fill_sound(fluid: &str) -> &'static str {
    if fluid == LAVA { "minecraft:item.bucket.fill_lava" } else { "minecraft:item.bucket.fill" }
}

/// The pitch of the sound a tank makes when a bucket is emptied into it or filled from it:
/// lower the fuller it is (`FluidTankBlock.useItemOn`), with `jitter` (-0.5 to 0.5) as its
/// random.
pub fn tank_pitch(amount: i32, capacity_per_block: i32, jitter: f32) -> f32 {
    let pitch = (1.0 - amount as f32 / (capacity_per_block as f32 * 16.0)).clamp(0.0, 1.0);
    pitch / 1.5 + 0.5 + jitter / 4.0
}

/// The fluid tags of Create's datagen and NeoForge's, and what they hold here
/// (`data/c/tags/fluid/honey.json`, `chocolate`, `tea`).
pub fn tag_holds(tag: &str, fluid: &str) -> bool {
    match tag {
        "c:honey" => fluid == HONEY,
        "c:chocolate" => fluid == CHOCOLATE,
        "c:tea" => fluid == TEA,
        "c:milk" => fluid == MILK,
        "c:water" | "minecraft:water" => fluid == WATER,
        "c:lava" | "minecraft:lava" => fluid == LAVA,
        _ => false,
    }
}

/// The colour a tank draws a fluid with when Horizon has no sprite of its own for it
/// (`AllFluids`: honey `0xEAAE2F`, chocolate `0x622020`; tea and milk are the colours of the
/// items), as RGB.
pub fn tint(fluid: &str) -> u32 {
    match fluid {
        HONEY => 0xEAAE2F,
        CHOCOLATE => 0x622020,
        TEA => 0x8C5B2E,
        MILK => 0xF4F4F0,
        POTION => 0x385DC6,
        _ => 0x7F7FFF,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bucket_of_honey_gives_a_bucket_and_a_bottle_a_quarter() {
        let bucket = emptied("create:honey_bucket").unwrap();
        assert_eq!((bucket.fluid, bucket.amount, bucket.empty), (HONEY, 1000, "minecraft:bucket"));
        let bottle = emptied("minecraft:honey_bottle").unwrap();
        assert_eq!((bottle.fluid, bottle.amount, bottle.empty), (HONEY, 250, "minecraft:glass_bottle"));
        assert!(emptied("minecraft:stick").is_none());
    }

    #[test]
    fn an_empty_container_is_filled_with_the_fluid_it_holds_when_full() {
        assert_eq!(filled("minecraft:bucket", WATER).unwrap().full, "minecraft:water_bucket");
        assert_eq!(filled("minecraft:glass_bottle", TEA).unwrap().full, "create:builders_tea");
        assert!(filled("minecraft:bucket", TEA).is_none());
        // `GenericItemFilling.canItemBeFilled`: a milk bucket is never filled.
        assert!(filled("minecraft:bucket", MILK).is_none());
    }

    #[test]
    fn every_container_is_found_by_either_form() {
        for c in CONTAINERS {
            assert!(is_container(c.full) && is_container(c.empty));
        }
        assert!(!is_container("minecraft:iron_ingot"));
    }

    #[test]
    fn fluid_tags_hold_create_fluids() {
        assert!(tag_holds("c:honey", HONEY));
        assert!(!tag_holds("c:honey", CHOCOLATE));
        assert!(tag_holds("c:tea", TEA));
    }

    #[test]
    fn the_tank_sounds_lower_as_it_fills() {
        assert!(tank_pitch(0, 8000, 0.0) > tank_pitch(64000, 8000, 0.0));
    }
}
