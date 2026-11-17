//! Tree Fertilizer's rules (`TreeFertilizerItem`): which tree a clicked sapling grows into
//! (vanilla's `TreeGrower`, which `performBonemeal` ends in) and which saplings it takes. No game
//! types: the game glue is `machines/tree_fertilizer.rs`.
//!
//! Known differences from Create, for the README:
//! - Create grows the tree in a simulated level (`TreesDreamWorld`: soil under the sapling, nothing
//!   in the way) and copies the blocks over the real ones that aren't solid. Horizon has no
//!   simulated level and no call to place a tree feature in a scratch level, so the tree is
//!   placed with the game's own `/place feature` where the sapling stood: the usual space checks
//!   apply (a tree under a low ceiling is cut short or doesn't grow), and when it doesn't grow the
//!   sapling is put back and the item dropped.
//! - No bees in the tree: a sapling's tree grows with a bee nest where flowers are near, which
//!   the simulated level never had either.

/// A tree feature and the chance of growing it, as vanilla's `TreeGrower` picks between two.
pub type Choice = (&'static str, f32);

/// What a sapling grows: vanilla's `TreeGrower`. `mega` is tried first where four saplings of the
/// kind stand in a square.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grower {
    pub mega: &'static [Choice],
    pub normal: &'static [Choice],
}

const NONE: &[Choice] = &[];

/// `TreeGrower.OAK`, `SPRUCE`, `MANGROVE`, `AZALEA`, `BIRCH`, `ACACIA`, `CHERRY`, `JUNGLE`,
/// `DARK_OAK` and `PALE_OAK` for the sapling blocks of `BlockTags.SAPLINGS`. A chance is the
/// probability the first feature is taken when the list has two (`random.nextInt(10) == 0` for the
/// fancy oak, `nextBoolean` for the two mega spruces, `nextFloat() < 0.85` for the tall mangrove).
pub fn grower(sapling: &str) -> Option<Grower> {
    Some(match sapling {
        "minecraft:oak_sapling" => Grower {
            mega: NONE,
            normal: &[("minecraft:fancy_oak", 0.1), ("minecraft:oak", 0.9)],
        },
        "minecraft:spruce_sapling" => Grower {
            mega: &[("minecraft:mega_spruce", 0.5), ("minecraft:mega_pine", 0.5)],
            normal: &[("minecraft:spruce", 1.0)],
        },
        "minecraft:mangrove_propagule" => Grower {
            mega: NONE,
            normal: &[("minecraft:tall_mangrove", 0.85), ("minecraft:mangrove", 0.15)],
        },
        "minecraft:azalea" | "minecraft:flowering_azalea" => Grower {
            mega: NONE,
            normal: &[("minecraft:azalea_tree", 1.0)],
        },
        "minecraft:birch_sapling" => Grower { mega: NONE, normal: &[("minecraft:birch", 1.0)] },
        "minecraft:acacia_sapling" => Grower { mega: NONE, normal: &[("minecraft:acacia", 1.0)] },
        "minecraft:cherry_sapling" => Grower { mega: NONE, normal: &[("minecraft:cherry", 1.0)] },
        "minecraft:jungle_sapling" => Grower {
            mega: &[("minecraft:mega_jungle_tree", 1.0)],
            normal: &[("minecraft:jungle_tree_no_vine", 1.0)],
        },
        // The dark and pale oaks grow in a square of four only.
        "minecraft:dark_oak_sapling" => Grower { mega: &[("minecraft:dark_oak", 1.0)], normal: NONE },
        "minecraft:pale_oak_sapling" => {
            Grower { mega: &[("minecraft:pale_oak_bonemeal", 1.0)], normal: NONE }
        }
        _ => return None,
    })
}

/// Picks one of the choices for a roll in 0..1: the chances are in order, so a roll under the first
/// chance takes the first.
pub fn pick(choices: &[Choice], roll: f32) -> Option<&'static str> {
    let mut covered = 0.0;
    for (feature, chance) in choices {
        covered += chance;
        if roll < covered {
            return Some(feature);
        }
    }
    choices.last().map(|(feature, _)| *feature)
}

/// What `TreeGrower.growTree` does for a sapling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The tree feature to place.
    pub feature: &'static str,
    /// Where the feature is placed, relative to the clicked sapling (the north-west one of a square).
    pub origin: [i32; 2],
    /// The saplings that make way (one, or four of a square), relative to the clicked sapling.
    pub saplings: Vec<[i32; 2]>,
}

/// `TreeGrower.growTree`: `same(dx, dz)` says whether the block at that offset from the clicked
/// sapling is the same sapling block. The two rolls are for the mega and the normal choice.
pub fn plan(
    grower: &Grower,
    mega_roll: f32,
    normal_roll: f32,
    same: impl Fn(i32, i32) -> bool,
) -> Option<Plan> {
    if let Some(feature) = pick(grower.mega, mega_roll) {
        // `for (i = 0; i >= -1; --i) for (j = 0; j >= -1; --j) if (isTwoByTwoSapling(.., i, j))`.
        for x in [0, -1] {
            for z in [0, -1] {
                let square = [[x, z], [x + 1, z], [x, z + 1], [x + 1, z + 1]];
                if square.iter().all(|[dx, dz]| same(*dx, *dz)) {
                    return Some(Plan { feature, origin: [x, z], saplings: square.to_vec() });
                }
            }
        }
    }
    let feature = pick(grower.normal, normal_roll)?;
    Some(Plan { feature, origin: [0, 0], saplings: vec![[0, 0]] })
}

/// `MangrovePropaguleBlock.HANGING`: a propagule still hanging from a leaf can't be fertilised.
pub fn can_fertilise(hanging: bool) -> bool {
    !hanging
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_oak_grows_a_fancy_one_one_time_in_ten() {
        let oak = grower("minecraft:oak_sapling").unwrap();
        assert_eq!(pick(oak.normal, 0.05), Some("minecraft:fancy_oak"));
        assert_eq!(pick(oak.normal, 0.5), Some("minecraft:oak"));
        assert_eq!(pick(oak.normal, 0.999), Some("minecraft:oak"));
        assert_eq!(pick(NONE, 0.5), None);
    }

    #[test]
    fn a_lone_dark_oak_does_not_grow_and_a_square_of_four_does() {
        let dark = grower("minecraft:dark_oak_sapling").unwrap();
        assert_eq!(plan(&dark, 0.0, 0.0, |dx, dz| (dx, dz) == (0, 0)), None);
        // The clicked sapling is the south-east of a square: it starts one block north-west.
        let square = |dx: i32, dz: i32| (-1..=0).contains(&dx) && (-1..=0).contains(&dz);
        let plan = plan(&dark, 0.0, 0.0, square).unwrap();
        assert_eq!(plan.feature, "minecraft:dark_oak");
        assert_eq!(plan.origin, [-1, -1]);
        assert_eq!(plan.saplings.len(), 4);
    }

    #[test]
    fn a_spruce_grows_the_mega_tree_only_in_a_square() {
        let spruce = grower("minecraft:spruce_sapling").unwrap();
        let lone = plan(&spruce, 0.0, 0.0, |dx, dz| (dx, dz) == (0, 0)).unwrap();
        assert_eq!((lone.feature, lone.saplings.len()), ("minecraft:spruce", 1));
        let all = plan(&spruce, 0.2, 0.0, |dx, dz| (0..=1).contains(&dx) && (0..=1).contains(&dz)).unwrap();
        assert_eq!((all.feature, all.origin), ("minecraft:mega_spruce", [0, 0]));
        let other = plan(&spruce, 0.7, 0.0, |dx, dz| (0..=1).contains(&dx) && (0..=1).contains(&dz)).unwrap();
        assert_eq!(other.feature, "minecraft:mega_pine");
    }

    #[test]
    fn the_first_square_found_wins_in_vanillas_order() {
        // Every offset is a sapling: the scan starts at (0, 0), not the north-west one.
        let jungle = grower("minecraft:jungle_sapling").unwrap();
        let plan = plan(&jungle, 0.0, 0.0, |_, _| true).unwrap();
        assert_eq!(plan.origin, [0, 0]);
    }

    #[test]
    fn only_saplings_have_a_grower_and_a_hanging_propagule_is_refused() {
        assert!(grower("minecraft:stone").is_none());
        assert!(grower("minecraft:mangrove_propagule").is_some());
        assert!(grower("minecraft:flowering_azalea").is_some());
        assert!(can_fertilise(false));
        assert!(!can_fertilise(true));
    }
}
