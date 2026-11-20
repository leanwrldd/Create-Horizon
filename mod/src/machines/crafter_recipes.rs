//! Which recipe a Mechanical Crafter's grid makes (`RecipeGridHandler.tryToApplyRecipe`): the
//! crafting recipes first (`allowRegularCraftingInCrafter`), then `create:mechanical_crafting`.
//! The matching is `create_kinetics::rules::crafter_recipes`; this reads the recipes from the data.
//!
//! Known differences from Create, for `crafter.rs`'s list:
//! - Vanilla's crafting recipes (shaped, shapeless, special ones such as dyeing, fireworks, banners
//!   and maps, data packs included) match through the new mod API's `LevelRef::recipe_for`
//!   (`compat::recipe_lookup`), as `getRecipeFor(RecipeType.CRAFTING, ..)` does, and the first
//!   match is refused when `isRecipeAllowed` says so (a `_manual_only` recipe, a firework rocket
//!   of more items than `maxFireworkIngredientsInCrafter`): `rules::vanilla_recipes`. The engine's
//!   matcher takes grids up to 3x3, so a grid whose bounding box is larger finds no vanilla
//!   recipe (Create would still match a shapeless one spread over a large grid).
//! - `create:mechanical_crafting` is read as it is (`Host::recipes_of_type`), so a data pack can
//!   add to it.

use create_kinetics::rules::crafter_grid::Grid;
use create_kinetics::rules::crafter_recipes::{Mode, Shaped, shaped_matches};
use create_kinetics::rules::vanilla_recipes;
use crate::compat::serde_json::Value;
use crate::compat::{Hooks, Host, Ingredient, ItemStack};

use crate::compat::recipe_lookup::{self, Input};
use crate::recipes::modern_ingredient;

/// A module of shared code: no hooks of its own.
struct Shared;

impl Hooks for Shared {}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

/// `AllRecipeTypes.MECHANICAL_CRAFTING`.
const MECHANICAL: &str = "create:mechanical_crafting";
/// `RecipeType.CRAFTING`.
const CRAFTING: &str = "minecraft:crafting";

/// The pattern's characters, row by row (`" "` is a space), trimmed to its items.
fn pattern_of(json: &Value) -> Option<Shaped<char>> {
    let rows: Vec<Vec<Option<char>>> = json
        .get("pattern")?
        .as_array()?
        .iter()
        .map(|row| Some(row.as_str()?.chars().map(|c| (c != ' ').then_some(c)).collect()))
        .collect::<Option<_>>()?;
    Some(Shaped::trimmed(rows))
}

/// Whether a pattern could match an input of this size at all, so its ingredients are only
/// looked up for the few that could.
fn could_fit(pattern: &Shaped<char>, mode: Mode, width: usize, height: usize) -> bool {
    match mode {
        Mode::Mirrored => (pattern.width, pattern.height) == (width, height),
        Mode::AsWritten => pattern.width <= width && pattern.height <= height,
    }
}

/// An ingredient as a recipe's JSON writes it: Create's datagen form (`{"tag": ...}`) or
/// Horizon's (`"#tag"`), both read.
fn ingredient(host: &dyn Host, json: &Value) -> Option<Ingredient> {
    host.recipe_ingredient(&modern_ingredient(json))
}

/// The pattern with each character's ingredient; `None` when one can't be read.
fn resolve(host: &dyn Host, pattern: &Shaped<char>, json: &Value) -> Option<Shaped<Ingredient>> {
    let key = json.get("key")?;
    let mut cells = Vec::with_capacity(pattern.cells.len());
    for cell in &pattern.cells {
        cells.push(match cell {
            None => None,
            Some(c) => Some(ingredient(host, key.get(c.to_string())?)?),
        });
    }
    Some(Shaped { width: pattern.width, height: pattern.height, cells })
}

fn result(host: &dyn Host, json: &Value) -> Option<ItemStack> {
    host.recipe_result(json.get("result")?)
}

fn takes(ingredient: &Ingredient, stack: &ItemStack) -> bool {
    ingredient.test(stack)
}

/// What a shaped recipe makes of the input, read as `mode` says.
fn shaped(
    host: &dyn Host,
    json: &Value,
    mode: Mode,
    (width, height, cells): &(usize, usize, Vec<Option<ItemStack>>),
) -> Option<ItemStack> {
    let pattern = pattern_of(json)?;
    if !could_fit(&pattern, mode, *width, *height) {
        return None;
    }
    let resolved = resolve(host, &pattern, json)?;
    if !shaped_matches(&resolved, mode, *width, *height, cells, &takes) {
        return None;
    }
    result(host, json)
}

/// `RecipeGridHandler.tryToApplyRecipe`: what the grid makes, if anything.
pub fn apply(host: &dyn Host, grid: &Grid<ItemStack>) -> Option<ItemStack> {
    let input = grid.input();
    if input.0 == 0 || input.1 == 0 {
        return None;
    }
    // `allowRegularCraftingInCrafter`, then `getRecipeFor(RecipeType.CRAFTING, ..)` filtered by
    // `isRecipeAllowed` (the engine's matcher takes up to 3x3).
    if vanilla_recipes::ALLOW_REGULAR_CRAFTING_IN_CRAFTER && input.0 <= 3 && input.1 <= 3 {
        let slots = &input.2;
        let crafting = Input::Crafting { width: input.0, height: input.1, slots };
        if let Some(found) = recipe_lookup::recipe_for(host, CRAFTING, &crafting) {
            let occupied = slots.iter().flatten().count();
            if vanilla_recipes::crafter_allows(&found.id, occupied) {
                return Some(found.output);
            }
        }
    }
    for recipe in host.recipes_of_type(MECHANICAL) {
        let mirrored = recipe.json.get("accept_mirrored").and_then(Value::as_bool).unwrap_or(false);
        let mode = if mirrored { Mode::Mirrored } else { Mode::AsWritten };
        if let Some(made) = shaped(host, &recipe.json, mode, &input) {
            return Some(made);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compat::serde_json::json;

    #[test]
    fn a_recipes_pattern_is_trimmed_and_sized_before_its_ingredients_are_read() {
        let torch = json!({"pattern": ["X", "#"]});
        let p = pattern_of(&torch).unwrap();
        assert_eq!((p.width, p.height), (1, 2));
        assert!(could_fit(&p, Mode::Mirrored, 1, 2));
        assert!(!could_fit(&p, Mode::Mirrored, 2, 2));
        assert!(could_fit(&p, Mode::AsWritten, 2, 2));
        assert!(!could_fit(&p, Mode::AsWritten, 1, 1));
        let wheel = json!({"pattern": [" AAA ", "AAPAA", "APSPA", "AAPAA", " AAA "]});
        let p = pattern_of(&wheel).unwrap();
        assert_eq!((p.width, p.height), (5, 5));
        assert_eq!(p.cells[0], None);
        assert_eq!(p.cells[1], Some('A'));
        assert!(pattern_of(&json!({"pattern": 3})).is_none());
    }
}
