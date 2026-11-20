//! Which of vanilla's recipes Create's machines take, and the config defaults that decide it
//! (`CRecipes`, `AllRecipeTypes.CAN_BE_AUTOMATED`, `MechanicalPressBlockEntity.canCompress`,
//! `MechanicalMixerBlockEntity.matchStaticFilters`, `RecipeGridHandler.isRecipeAllowed`). No game
//! types: the glue reads the recipes through the new mod API (`LevelRef::recipe_for`,
//! `Host::recipes_of_type`) and asks these.

// --- CRecipes defaults (infrastructure/config/CRecipes.java) -------------------------------------------

/// `allowShapelessInMixer`: shapeless crafting recipes run in a Mechanical Mixer + Basin.
pub const ALLOW_SHAPELESS_IN_MIXER: bool = true;
/// `allowShapedSquareInPress`: single-ingredient 2x2 and 3x3 crafting recipes run in a Mechanical
/// Press + Basin.
pub const ALLOW_SHAPED_SQUARE_IN_PRESS: bool = true;
/// `allowRegularCraftingInCrafter`: standard crafting recipes run in Mechanical Crafters.
pub const ALLOW_REGULAR_CRAFTING_IN_CRAFTER: bool = true;
/// `maxFireworkIngredientsInCrafter`.
pub const MAX_FIREWORK_INGREDIENTS_IN_CRAFTER: usize = 9;
/// `allowStonecuttingOnSaw`.
pub const ALLOW_STONECUTTING_ON_SAW: bool = true;

/// `AllRecipeTypes.CAN_BE_AUTOMATED`: a recipe whose path ends in `_manual_only` is left to the
/// player. (`shouldIgnoreInAutomation` also reads the `create:automation_ignore` serializer tag,
/// which Create fills only with Occultism's spirit trade.)
pub fn can_be_automated(recipe_id: &str) -> bool {
    let path = recipe_id.split_once(':').map_or(recipe_id, |(_, path)| path);
    !path.ends_with("_manual_only")
}

/// `ItemHelper.matchIngredients`: two ingredients list the same items in the same order. An empty
/// cell is an ingredient with no items.
pub fn match_ingredients(a: &[String], b: &[String]) -> bool {
    a == b
}

/// `ItemHelper.matchAllIngredients`: every ingredient matches the first.
pub fn match_all_ingredients(ingredients: &[Vec<String>]) -> bool {
    ingredients.len() <= 1
        || ingredients[1..].iter().all(|ingredient| match_ingredients(&ingredients[0], ingredient))
}

/// `MechanicalPressBlockEntity.canCompress` for a crafting recipe: its `getIngredients()` (a shaped
/// recipe's cells, empty ones included; a shapeless recipe's list) number four or nine and all
/// match.
pub fn can_compress(ingredients: &[Vec<String>]) -> bool {
    ALLOW_SHAPED_SQUARE_IN_PRESS
        && (ingredients.len() == 4 || ingredients.len() == 9)
        && match_all_ingredients(ingredients)
}

/// `MechanicalMixerBlockEntity.matchStaticFilters` for a crafting recipe: a shapeless one (not a
/// `ShapedRecipe`) with more than one ingredient that the press doesn't compress, and not left to
/// the player.
pub fn mixer_takes(shapeless: bool, ingredients: &[Vec<String>], recipe_id: &str) -> bool {
    shapeless
        && ALLOW_SHAPELESS_IN_MIXER
        && ingredients.len() > 1
        && !can_compress(ingredients)
        && can_be_automated(recipe_id)
}

/// `MechanicalPressBlockEntity.matchStaticFilters` for a crafting recipe (`MechanicalCraftingRecipe`
/// is a Create type, never a vanilla one).
pub fn press_takes(ingredients: &[Vec<String>], recipe_id: &str) -> bool {
    can_compress(ingredients) && can_be_automated(recipe_id)
}

/// `RecipeGridHandler.isRecipeAllowed`: a firework rocket takes at most
/// [`MAX_FIREWORK_INGREDIENTS_IN_CRAFTER`] items, and a recipe not for automation is refused.
pub fn crafter_allows(recipe_id: &str, occupied_slots: usize) -> bool {
    if recipe_id == "minecraft:firework_rocket" && occupied_slots > MAX_FIREWORK_INGREDIENTS_IN_CRAFTER {
        return false;
    }
    can_be_automated(recipe_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(items: &[&str]) -> Vec<String> {
        items.iter().map(|i| (*i).to_owned()).collect()
    }

    #[test]
    fn manual_only_recipes_are_not_automated() {
        assert!(can_be_automated("minecraft:stick"));
        assert!(can_be_automated("create:crafting/kinetics/shaft"));
        assert!(!can_be_automated("create:crafting/foo_manual_only"));
        assert!(!can_be_automated("foo_manual_only"));
        assert!(can_be_automated("create:manual_only_not_at_the_end/x"));
    }

    #[test]
    fn a_square_of_one_ingredient_compresses_and_stays_out_of_the_mixer() {
        let iron = one(&["minecraft:iron_ingot"]);
        let block = vec![iron.clone(); 9];
        assert!(can_compress(&block));
        assert!(press_takes(&block, "minecraft:iron_block"));
        assert!(!mixer_takes(true, &block, "minecraft:iron_block"));
        // 2x2 of one ingredient, e.g. packed ice shapes aside: four match.
        assert!(can_compress(&vec![iron.clone(); 4]));
    }

    #[test]
    fn sizes_and_mixed_ingredients_do_not_compress() {
        let iron = one(&["minecraft:iron_ingot"]);
        assert!(!can_compress(&vec![iron.clone(); 3]));
        assert!(!can_compress(&vec![iron.clone(); 8]));
        let mut mixed = vec![iron.clone(); 9];
        mixed[4] = one(&["minecraft:gold_ingot"]);
        assert!(!can_compress(&mixed));
        // An empty cell is an ingredient with no items: a ring of eight doesn't compress.
        let mut ring = vec![iron.clone(); 9];
        ring[4] = Vec::new();
        assert!(!can_compress(&ring));
        // The same items in another order are another ingredient.
        assert!(!match_ingredients(&one(&["a", "b"]), &one(&["b", "a"])));
        assert!(match_ingredients(&one(&["a", "b"]), &one(&["a", "b"])));
        assert!(match_all_ingredients(&[]));
        assert!(match_all_ingredients(&[iron]));
    }

    #[test]
    fn the_mixer_takes_shapeless_recipes_of_two_or_more() {
        let a = one(&["minecraft:a"]);
        let b = one(&["minecraft:b"]);
        assert!(mixer_takes(true, &[a.clone(), b.clone()], "minecraft:mix"));
        assert!(!mixer_takes(true, &[a.clone()], "minecraft:one"));
        assert!(!mixer_takes(false, &[a.clone(), b.clone()], "minecraft:shaped"));
        assert!(!mixer_takes(true, &[a, b], "minecraft:x_manual_only"));
    }

    #[test]
    fn the_crafter_limits_fireworks() {
        assert!(crafter_allows("minecraft:firework_rocket", 9));
        assert!(!crafter_allows("minecraft:firework_rocket", 10));
        assert!(crafter_allows("minecraft:stick", 12));
        assert!(!crafter_allows("minecraft:stick_manual_only", 1));
    }
}
