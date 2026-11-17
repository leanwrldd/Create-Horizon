//! Create's processing recipes (`ProcessingRecipe`: pressing, milling, crushing, mixing...), read
//! from the data files as Create's datagen writes them for Minecraft 1.21.1, so the same JSON
//! works here and a data pack can add more (`Host::recipes_of_type`).
//!
//! ```json
//! { "type": "create:milling", "processing_time": 50,
//!   "ingredients": [{"item": "minecraft:wheat"}],
//!   "results": [{"id": "create:wheat_flour"}, {"id": "minecraft:wheat_seeds", "chance": 0.25}] }
//! ```
//!
//! Fluid ingredients (`{"type": "neoforge:single", "fluid": ..., "amount": ...}` or
//! `neoforge:tag` with a `tag`) and fluid results (an `amount` and no `count`) are kept as raw
//! JSON. Create's datagen also writes recipes for other mods' items behind `neoforge:conditions`:
//! leave those out of `content/`.

// Shared with the machine modules, which don't all use every helper.
#![allow(dead_code, unused_imports)]

use crate::compat::serde_json::{Value, json};
use crate::compat::{Host, Ingredient, ItemStack, RandomSource};

/// One of a recipe's item results (`ProcessingOutput`).
#[derive(Clone)]
pub struct Output {
    pub stack: ItemStack,
    pub chance: f32,
}

/// `HeatCondition`, a recipe's `heat_requirement`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Heat {
    None,
    Heated,
    Superheated,
}

#[derive(Clone)]
pub struct Processing {
    /// `create:milling/wheat`.
    pub id: String,
    pub ingredients: Vec<Ingredient>,
    /// Fluid ingredients, as written.
    pub fluid_ingredients: Vec<Value>,
    pub results: Vec<Output>,
    /// Fluid results, as written (`{"id": "minecraft:water", "amount": 250}`).
    pub fluid_results: Vec<Value>,
    /// `processing_time`; 0 when the recipe names none.
    pub processing_time: i32,
    pub heat: Heat,
}

/// An ingredient as a recipe for Minecraft 1.21.1 writes it (`{"tag": "c:ingots/iron"}`,
/// `{"item": ...}`), turned into the form Horizon reads (`"#c:ingots/iron"`); lists element by
/// element. Anything else is passed on as it is.
pub fn modern_ingredient(json: &Value) -> Value {
    match json {
        Value::Object(o) if o.len() == 1 => match (o.get("tag"), o.get("item")) {
            (Some(Value::String(tag)), _) => json!(format!("#{tag}")),
            (_, Some(Value::String(item))) => json!(item),
            _ => json.clone(),
        },
        Value::Array(a) => Value::Array(a.iter().map(modern_ingredient).collect()),
        _ => json.clone(),
    }
}

/// A fluid ingredient names an `amount` (`{"type": "neoforge:tag", "amount": 250, "tag": "c:milk"}`,
/// `{"type": "neoforge:single", "amount": 250, "fluid": "minecraft:water"}`); an item never does.
fn is_fluid_ingredient(json: &Value) -> bool {
    json.get("amount").is_some()
}

/// One result as an [`Output`]. `recipe_result` reads only `id`, `count` and `components`:
/// `chance` is Create's.
pub fn output(host: &dyn Host, result: &Value) -> Option<Output> {
    let mut plain = result.as_object()?.clone();
    let chance = plain
        .remove("chance")
        .and_then(|c| c.as_f64())
        .unwrap_or(1.0) as f32;
    let stack = host.recipe_result(&Value::Object(plain))?;
    Some(Output { stack, chance })
}

/// A recipe file of a processing type, parsed. `None` when an item ingredient can't be read.
pub fn parse(host: &dyn Host, id: &str, json: &Value) -> Option<Processing> {
    let mut ingredients = Vec::new();
    let mut fluid_ingredients = Vec::new();
    for ingredient in json.get("ingredients")?.as_array()? {
        if is_fluid_ingredient(ingredient) {
            fluid_ingredients.push(ingredient.clone());
        } else {
            ingredients.push(host.recipe_ingredient(&modern_ingredient(ingredient))?);
        }
    }
    let mut results = Vec::new();
    let mut fluid_results = Vec::new();
    for result in json
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if result.get("amount").is_some() {
            fluid_results.push(result.clone());
        } else if let Some(output) = output(host, result) {
            results.push(output);
        }
    }
    let heat = match json.get("heat_requirement").and_then(Value::as_str) {
        Some("heated") => Heat::Heated,
        Some("superheated") => Heat::Superheated,
        _ => Heat::None,
    };
    Some(Processing {
        id: id.to_owned(),
        ingredients,
        fluid_ingredients,
        results,
        fluid_results,
        processing_time: json
            .get("processing_time")
            .and_then(Value::as_i64)
            .unwrap_or(0) as i32,
        heat,
    })
}

/// Every recipe of `kind` (`create:milling`), in the order the game holds them.
pub fn all(host: &dyn Host, kind: &str) -> Vec<Processing> {
    host.recipes_of_type(kind)
        .iter()
        .filter_map(|recipe| parse(host, &recipe.id, &recipe.json))
        .collect()
}

/// The first recipe of `kind` with exactly one item ingredient, which `stack` fits
/// (`AllRecipeTypes.X.find` for a single-input machine: a press, a millstone, a saw, a fan).
pub fn for_stack(host: &dyn Host, kind: &str, stack: &ItemStack) -> Option<Processing> {
    host.recipes_of_type(kind).iter().find_map(|recipe| {
        let first = recipe.json.get("ingredients")?.get(0)?;
        if is_fluid_ingredient(first) {
            return None;
        }
        let ingredient = host.recipe_ingredient(&modern_ingredient(first))?;
        if !ingredient.test(stack) {
            return None;
        }
        parse(host, &recipe.id, &recipe.json)
    })
}

/// One roll of a recipe's results (`ProcessingRecipe.rollResults`): each stack that came up.
pub fn roll(host: &mut dyn Host, dimension: &str, results: &[Output]) -> Vec<ItemStack> {
    results
        .iter()
        .filter(|output| {
            output.chance >= 1.0
                || host
                    .level_random(dimension)
                    // `ProcessingOutput.rollOutput`: kept when `nextFloat() <= chance`.
                    .is_some_and(|random| random.next_float() <= output.chance)
        })
        .map(|output| output.stack.clone())
        .collect()
}
