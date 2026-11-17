//! The Mechanical Harvester on a structure: which plants it reaps and what it leaves standing
//! (`HarvesterMovementBehaviour.visitNewPosition`, `isValidCrop`, `isValidOther`, `cutCrop`;
//! `CKinetics.harvestPartiallyGrown`, `harvesterReplants`). Plain values: the glue reads the block
//! and its state, asks this, and applies the answer.

use super::actor_facts as facts;

/// The two switches of `CKinetics` (defaults: `harvestPartiallyGrown` false, `harvesterReplants`
/// true).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub partial: bool,
    pub replants: bool,
}

pub const DEFAULTS: Config = Config { partial: false, replants: true };

/// A block as the harvester sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plant<'a> {
    pub block: &'a str,
    /// The value of its `age` property.
    pub age: Option<i32>,
    /// `getCollisionShape().isEmpty()`.
    pub collision_empty: bool,
    /// `BlockTags.LEAVES`.
    pub leaves: bool,
    /// `AllBlockTags.SUGAR_CANE_VARIANTS`.
    pub cane_variant: bool,
}

/// `IntegerProperty.getPossibleValues().size()` of the `age` property.
pub fn age_values(block: &str) -> Option<i32> {
    facts::AGE_VALUES.iter().find(|(name, _)| *name == block).map(|(_, n)| *n)
}

fn is_crop(block: &str) -> bool {
    facts::CROP.contains(&block)
}

fn is_cocoa(block: &str) -> bool {
    facts::COCOA.contains(&block)
}

/// `isValidCrop`: a crop worth breaking (grown, unless partial harvesting is on).
pub fn is_valid_crop(config: Config, plant: &Plant<'_>) -> bool {
    let age = plant.age;
    if is_crop(plant.block) {
        let (Some(age), Some(values)) = (age, age_values(plant.block)) else { return false };
        if config.partial {
            // `state != crop.getStateForAge(0) || !replant`.
            return age != 0 || !config.replants;
        }
        return age == values - 1;
    }
    if plant.collision_empty || is_cocoa(plant.block) {
        let (Some(age), Some(values)) = (age, age_values(plant.block)) else { return false };
        if facts::SWEET_BERRY.contains(&plant.block) && age <= 1 && config.replants {
            return false;
        }
        if (age == 0 && config.replants) || (!config.partial && values - 1 != age) {
            return false;
        }
        return true;
    }
    false
}

/// `isValidOther`: something that is cut though it isn't a crop.
pub fn is_valid_other(plant: &Plant<'_>) -> bool {
    if is_crop(plant.block) {
        return false;
    }
    if facts::SUGAR_CANE.contains(&plant.block) || plant.leaves {
        return true;
    }
    if is_cocoa(plant.block) {
        return plant.age == Some(2);
    }
    if plant.collision_empty {
        if facts::GROWING_PLANT.contains(&plant.block) {
            return true;
        }
        // A block with an `age` property that isn't a crop above is left alone.
        if age_values(plant.block).is_some() {
            return false;
        }
        // `BushBlock` (the mushroom's Farmer's Delight case is another mod's), `SpecialPlantable`.
        if facts::VEGETATION.contains(&plant.block) || facts::SPECIAL_PLANTABLE.contains(&plant.block) {
            return true;
        }
    }
    false
}

/// What the harvester does at a visited block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reap {
    /// Nothing there for it.
    Skip,
    /// Break it: with shears for leaves (`effectChance` .45), the drops rolled as the
    /// tool's. `cuttable` is `notCropButCuttable`: its drops keep the seed.
    Cut { shears: bool, effect_chance: f32, cuttable: bool },
}

/// `visitNewPosition`'s decision: air and `AllBlockTags.NON_HARVESTABLE` are passed over.
pub fn assess(config: Config, plant: &Plant<'_>, air: bool, non_harvestable: bool) -> Reap {
    if air || non_harvestable {
        return Reap::Skip;
    }
    let mut cuttable = false;
    if !is_valid_crop(config, plant) {
        if !is_valid_other(plant) {
            return Reap::Skip;
        }
        cuttable = true;
    }
    if plant.leaves {
        return Reap::Cut { shears: true, effect_chance: 0.45, cuttable };
    }
    Reap::Cut { shears: false, effect_chance: 1.0, cuttable }
}

/// What stands where the plant was (`cutCrop`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Left {
    /// Nothing but the block's fluid (`getFluidState().createLegacyBlock()`), air when dry.
    Nothing,
    /// The same block with its `age` set.
    Aged(i32),
}

/// `cutCrop`.
pub fn cut_crop(config: Config, plant: &Plant<'_>) -> Left {
    if !config.replants {
        return Left::Nothing;
    }
    if is_crop(plant.block) {
        // `crop.getStateForAge(0)`: the same block at age 0 for every crop of the game.
        return Left::Aged(0);
    }
    if facts::SWEET_BERRY.contains(&plant.block) {
        return Left::Aged(1);
    }
    if plant.cane_variant || facts::GROWING_PLANT.contains(&plant.block) {
        return Left::Nothing;
    }
    if (plant.collision_empty || is_cocoa(plant.block)) && age_values(plant.block).is_some() {
        return Left::Aged(0);
    }
    Left::Nothing
}

/// `ItemHelper.sameItem(stack, new ItemStack(state.getBlock()))`: the item a block stands for.
pub fn block_item(block: &str) -> &str {
    facts::BLOCK_ITEM.iter().find(|(name, _)| *name == block).map_or(block, |(_, item)| item)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plant<'a>(block: &'a str, age: Option<i32>) -> Plant<'a> {
        Plant { block, age, collision_empty: true, leaves: false, cane_variant: block == "minecraft:sugar_cane" }
    }

    #[test]
    fn only_a_grown_crop_is_reaped_by_default() {
        assert!(is_valid_crop(DEFAULTS, &plant("minecraft:wheat", Some(7))));
        assert!(!is_valid_crop(DEFAULTS, &plant("minecraft:wheat", Some(6))));
        assert!(!is_valid_crop(DEFAULTS, &plant("minecraft:wheat", Some(0))));
        // Beetroots grow to 3.
        assert!(is_valid_crop(DEFAULTS, &plant("minecraft:beetroots", Some(3))));
        assert!(!is_valid_crop(DEFAULTS, &plant("minecraft:beetroots", Some(2))));
    }

    #[test]
    fn partial_harvesting_takes_anything_but_a_seedling() {
        let partial = Config { partial: true, replants: true };
        assert!(is_valid_crop(partial, &plant("minecraft:wheat", Some(3))));
        assert!(!is_valid_crop(partial, &plant("minecraft:wheat", Some(0))));
        let no_replant = Config { partial: true, replants: false };
        assert!(is_valid_crop(no_replant, &plant("minecraft:wheat", Some(0))));
    }

    #[test]
    fn berries_and_wart_grow_up_before_they_are_taken() {
        assert!(is_valid_crop(DEFAULTS, &plant("minecraft:sweet_berry_bush", Some(3))));
        assert!(!is_valid_crop(DEFAULTS, &plant("minecraft:sweet_berry_bush", Some(1))));
        assert!(is_valid_crop(DEFAULTS, &plant("minecraft:nether_wart", Some(3))));
        assert!(!is_valid_crop(DEFAULTS, &plant("minecraft:nether_wart", Some(2))));
        let mut cocoa = plant("minecraft:cocoa", Some(2));
        cocoa.collision_empty = false;
        assert!(is_valid_crop(DEFAULTS, &cocoa));
        cocoa.age = Some(1);
        assert!(!is_valid_crop(DEFAULTS, &cocoa));
    }

    #[test]
    fn cane_leaves_and_bushes_are_cut_without_a_seed_kept() {
        let cane = plant("minecraft:sugar_cane", Some(0));
        assert_eq!(assess(DEFAULTS, &cane, false, false), Reap::Cut { shears: false, effect_chance: 1.0, cuttable: true });
        let mut leaves = plant("minecraft:oak_leaves", None);
        leaves.collision_empty = false;
        leaves.leaves = true;
        assert_eq!(assess(DEFAULTS, &leaves, false, false), Reap::Cut { shears: true, effect_chance: 0.45, cuttable: true });
        assert_eq!(
            assess(DEFAULTS, &plant("minecraft:dandelion", None), false, false),
            Reap::Cut { shears: false, effect_chance: 1.0, cuttable: true }
        );
        // A young sapling: a VegetationBlock without an age.
        assert!(is_valid_other(&plant("minecraft:oak_sapling", None)));
    }

    #[test]
    fn fire_air_and_solid_things_are_skipped() {
        assert_eq!(assess(DEFAULTS, &plant("minecraft:air", None), true, false), Reap::Skip);
        assert_eq!(assess(DEFAULTS, &plant("minecraft:fire", None), false, true), Reap::Skip);
        let mut stone = plant("minecraft:stone", None);
        stone.collision_empty = false;
        assert_eq!(assess(DEFAULTS, &stone, false, false), Reap::Skip);
        // A crop that isn't grown isn't cut either.
        assert_eq!(assess(DEFAULTS, &plant("minecraft:wheat", Some(2)), false, false), Reap::Skip);
    }

    #[test]
    fn what_is_left_standing() {
        assert_eq!(cut_crop(DEFAULTS, &plant("minecraft:wheat", Some(7))), Left::Aged(0));
        assert_eq!(cut_crop(DEFAULTS, &plant("minecraft:sweet_berry_bush", Some(3))), Left::Aged(1));
        assert_eq!(cut_crop(DEFAULTS, &plant("minecraft:sugar_cane", Some(0))), Left::Nothing);
        assert_eq!(cut_crop(DEFAULTS, &plant("minecraft:kelp", Some(1))), Left::Nothing);
        assert_eq!(cut_crop(DEFAULTS, &plant("minecraft:nether_wart", Some(3))), Left::Aged(0));
        let mut cocoa = plant("minecraft:cocoa", Some(2));
        cocoa.collision_empty = false;
        assert_eq!(cut_crop(DEFAULTS, &cocoa), Left::Aged(0));
        let no_replant = Config { partial: false, replants: false };
        assert_eq!(cut_crop(no_replant, &plant("minecraft:wheat", Some(7))), Left::Nothing);
    }

    #[test]
    fn the_seed_a_crop_drops_is_the_item_it_stands_for() {
        assert_eq!(block_item("minecraft:wheat"), "minecraft:wheat_seeds");
        assert_eq!(block_item("minecraft:carrots"), "minecraft:carrot");
        assert_eq!(block_item("minecraft:nether_wart"), "minecraft:nether_wart");
    }
}
