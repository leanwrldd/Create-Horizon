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

/// One tank of a block as another block sees it from a face.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TankView {
    /// The tank's index in the block's declared list.
    pub tank: usize,
    pub fluid: Option<String>,
    pub amount: i32,
    pub capacity: i32,
    pub can_fill: bool,
    pub can_drain: bool,
}

/// The tanks a block entity declares (`ModFluidTanks`: capacities, the tanks each face reaches, the ones that take a
/// fill and the ones that give a drain) as seen from `face` (0 down, 1 up, then north, south, west, east), filled
/// with what it saved: `saved[i]` is `(fluid, amount)` of tank `i`. A face that reaches no tank gives an empty list
/// (the Spout's below, the drain's above), which is not "no handler".
pub fn saved_tank_views(
    capacities: &[i32],
    faces: &[Vec<usize>; 6],
    inputs: &[usize],
    outputs: &[usize],
    face: usize,
    saved: &[(Option<String>, i32)],
) -> Vec<TankView> {
    faces
        .get(face)
        .map(|reached| {
            reached
                .iter()
                .filter(|tank| **tank < capacities.len())
                .map(|&tank| {
                    let (fluid, amount) = saved.get(tank).cloned().unwrap_or((None, 0));
                    let empty = amount <= 0 || fluid.as_deref().is_none_or(|f| f == "minecraft:empty");
                    TankView {
                        tank,
                        fluid: if empty { None } else { fluid },
                        amount: if empty { 0 } else { amount },
                        capacity: capacities[tank],
                        can_fill: inputs.contains(&tank),
                        can_drain: outputs.contains(&tank),
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `FluidPropagator.hasFluidCapability`: the block has a block entity (`blockEntity == null` is false) and the
/// level's fluid-handler capability answers on that face. A block without a block entity, such as the vanilla
/// cauldron, is not a handler for a pipe or a pump.
pub fn has_capability(has_block_entity: bool, has_handler: bool) -> bool {
    has_block_entity && has_handler
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handler_needs_a_block_entity() {
        assert!(has_capability(true, true));
        assert!(!has_capability(true, false));
        assert!(!has_capability(false, true), "the cauldron has no block entity");
        assert!(!has_capability(false, false));
    }

    #[test]
    fn a_face_sees_only_its_tanks_with_what_they_saved() {
        let faces = [vec![0], vec![], vec![0], vec![0], vec![0], vec![0]];
        let saved = [(Some("minecraft:water".to_string()), 250)];
        let side = saved_tank_views(&[1000], &faces, &[0], &[0], 2, &saved);
        assert_eq!(side.len(), 1);
        assert_eq!(side[0].fluid.as_deref(), Some("minecraft:water"));
        assert_eq!((side[0].amount, side[0].capacity), (250, 1000));
        assert!(side[0].can_fill && side[0].can_drain);
        // A face with no tank is an empty list, not a missing handler.
        assert!(saved_tank_views(&[1000], &faces, &[0], &[0], 1, &saved).is_empty());
    }

    #[test]
    fn empty_tanks_have_no_fluid_and_drain_only_ones_take_no_fill() {
        let faces: [Vec<usize>; 6] = [(); 6].map(|()| vec![0, 1]);
        let saved = [(Some("minecraft:empty".to_string()), 5), (Some("minecraft:lava".to_string()), 0)];
        let views = saved_tank_views(&[1000, 1000], &faces, &[0], &[0, 1], 0, &saved);
        assert!(views.iter().all(|v| v.fluid.is_none() && v.amount == 0));
        assert!(views[0].can_fill && !views[1].can_fill && views[1].can_drain);
        // A face past the six, or a tank past the capacities, is dropped.
        assert!(saved_tank_views(&[1000], &faces, &[], &[], 9, &[]).is_empty());
        assert_eq!(saved_tank_views(&[1000], &faces, &[], &[], 0, &[]).len(), 1);
    }

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
