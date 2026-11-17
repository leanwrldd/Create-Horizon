//! What an item takes and gives when a Spout fills it or an Item Drain empties it: Create's
//! `create:filling` and `create:emptying` recipes, then the generic containers
//! (`FillingBySpout`, `GenericItemFilling`, `GenericItemEmptying`, `PotionFluidHandler`), shared
//! by the `spout` and `item_drain` modules. No hooks of its own.
//!
//! `FillingBySpout` asks the sequenced assembly's next filling step first (`machines/sequenced.rs`:
//! the Sturdy Sheet's lava), then the `create:filling` recipes.
//!
//! Known differences from Create:
//! - Containers are the ones `rules/fluids.rs` knows (the vanilla and Create buckets, the honey
//!   bottle, Builder's Tea), the glass bottle with water and the bottle of water, and the potions
//!   (`rules/potion_fluid.rs`: the potion and the bottle type go in the fluid's id): Horizon's
//!   `Host` has no item fluid handler to ask for other mods' containers.
//! - A potion with custom effects (`custom_effects`, `custom_color`, `custom_name`) can't be
//!   emptied: the fluid carries only the potion's id.

use create_kinetics::rules::fluids as fluid_rules;
use create_kinetics::rules::potion_fluid as potion_rules;
use create_kinetics::rules::sequenced::Machine;
use crate::compat::serde_json::{Value, json};
use crate::compat::{Hooks, Host, ItemStack, Tag};

use super::sequenced;
use crate::recipes::{self, Processing};

/// A module of shared code: no hooks of its own.
struct Shared;

impl Hooks for Shared {}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

const FILLING: &str = "create:filling";
const EMPTYING: &str = "create:emptying";
const GLASS_BOTTLE: &str = "minecraft:glass_bottle";
const POTION: &str = "minecraft:potion";
const WATER: &str = fluid_rules::WATER;

/// `FluidIngredient.test`: whether the recipe's fluid ingredient takes `fluid`. A `neoforge:components`
/// ingredient names `create:potion` and the bottle type and potion it must carry (`rules/potion_fluid.rs`).
fn fluid_matches(ingredient: &Value, fluid: &str) -> bool {
    match ingredient.get("type").and_then(Value::as_str) {
        Some("neoforge:single") => ingredient.get("fluid").and_then(Value::as_str) == Some(fluid),
        Some("neoforge:tag") => ingredient
            .get("tag")
            .and_then(Value::as_str)
            .is_some_and(|tag| fluid_rules::tag_holds(tag, fluid)),
        Some("neoforge:components") => {
            let components = ingredient.get("components");
            let bottle = components
                .and_then(|c| c.get(potion_rules::BOTTLE_TYPE_COMPONENT))
                .and_then(Value::as_str);
            // `minecraft:potion_contents` is `{"potion": id}` (or the bare id).
            let potion = components.and_then(|c| c.get(potion_rules::POTION_CONTENTS)).and_then(|contents| {
                contents.as_str().or_else(|| contents.get("potion").and_then(Value::as_str))
            });
            ingredient
                .get("fluids")
                .and_then(Value::as_str)
                .is_some_and(|fluids| potion_rules::ingredient_matches(fluid, fluids, bottle, potion))
        }
        _ => false,
    }
}

fn fluid_amount(ingredient: &Value) -> i32 {
    ingredient.get("amount").and_then(Value::as_i64).unwrap_or(0) as i32
}

/// The recipes of `kind` whose single item ingredient takes `stack`.
fn matching(host: &dyn Host, kind: &str, stack: &ItemStack) -> Vec<Processing> {
    recipes::all(host, kind)
        .into_iter()
        .filter(|recipe| recipe.ingredients.first().is_some_and(|i| i.test(stack)))
        .collect()
}

/// What a potion item holds: its bottle type and its `minecraft:potion_contents` potion (`None` for empty contents).
/// `None` for an item that isn't a potion, and for contents Horizon's fluid can't carry (custom effects, colour or
/// name).
fn potion_of(stack: &ItemStack) -> Option<(potion_rules::Bottle, Option<String>)> {
    let bottle = potion_rules::Bottle::of_item(&stack.item)?;
    let potion = match stack.data().and_then(|data| data.other(potion_rules::POTION_CONTENTS)) {
        None => None,
        Some(Tag::String(potion)) => Some(potion.to_string_lossy()),
        Some(Tag::Compound(contents)) => {
            if ["custom_effects", "custom_color", "custom_name"]
                .iter()
                .any(|key| contents.contains_key(&(*key).into()))
            {
                return None;
            }
            match contents.get(&"potion".into()) {
                Some(Tag::String(potion)) => Some(potion.to_string_lossy()),
                _ => None,
            }
        }
        _ => return None,
    };
    Some((bottle, potion))
}

/// `PotionFluidHandler.isPotionItem` and `getFluidFromPotionItem`: the fluid a potion bottle holds and its amount.
fn potion_fluid(stack: &ItemStack) -> Option<(String, i32)> {
    let (bottle, potion) = potion_of(stack)?;
    potion_rules::from_item(bottle.item(), potion.as_deref())
}

/// A bottle of a potion: `PotionFluidHandler.fillBottle`, built through the recipe reader (the 26.x form of the
/// component, as `water_potion`'s).
fn potion_bottle(host: &dyn Host, fluid: &str) -> Option<ItemStack> {
    let (item, potion) = potion_rules::filled_bottle(fluid)?;
    let mut result = json!({ "id": item, "count": 1 });
    if let Some(potion) = potion {
        result["components"] = json!({ potion_rules::POTION_CONTENTS: potion });
    }
    host.recipe_result(&result)
}

/// A bottle of water: `PotionContents.createItemStack(Items.POTION, Potions.WATER)`. `None` when
/// the game doesn't read the component as written here.
fn water_potion(host: &dyn Host) -> Option<ItemStack> {
    host.recipe_result(&json!({
        "id": POTION,
        "count": 1,
        "components": { "minecraft:potion_contents": "minecraft:water" },
    }))
}

/// `FillingBySpout.canItemBeFilled`.
pub fn can_be_filled(host: &dyn Host, stack: &ItemStack) -> bool {
    if sequenced::find(host, stack, Machine::Filling, &|_| true).is_some() {
        return true;
    }
    if !matching(host, FILLING, stack).is_empty() {
        return true;
    }
    // `GenericItemFilling.canItemBeFilled`.
    let item: &str = &stack.item;
    item == GLASS_BOTTLE
        || fluid_rules::CONTAINERS
            .iter()
            .any(|c| c.empty == item && c.full != "minecraft:milk_bucket")
}

/// `FillingBySpout.getRequiredAmountForItem`: what filling `stack` with `fluid` takes, if it can be.
pub fn required_amount(host: &dyn Host, stack: &ItemStack, fluid: &str) -> Option<i32> {
    // `matchItemAndFluid`: the assembly's step if its fluid takes `fluid`.
    let step = sequenced::find(host, stack, Machine::Filling, &|step| {
        step.recipe.fluid_ingredients.first().is_some_and(|i| fluid_matches(i, fluid))
    });
    if let Some(ingredient) = step.and_then(|step| step.recipe.fluid_ingredients.first().cloned()) {
        return Some(fluid_amount(&ingredient));
    }
    for recipe in matching(host, FILLING, stack) {
        if let Some(ingredient) = recipe.fluid_ingredients.first()
            && fluid_matches(ingredient, fluid)
        {
            return Some(fluid_amount(ingredient));
        }
    }
    // `GenericItemFilling.getRequiredAmountForItem`: a glass bottle takes water and potions
    // (`canFillGlassBottleInternally`).
    let item: &str = &stack.item;
    if item == GLASS_BOTTLE && (fluid == WATER || potion_rules::is_potion_fluid(fluid)) {
        return Some(potion_rules::BOTTLE_AMOUNT);
    }
    fluid_rules::filled(item, fluid).map(|c| c.amount)
}

/// `FillingBySpout.fillItem` for one item of `stack`, with `fluid` of at least `required`: what
/// it becomes. `None` when nothing comes of it (Create's empty stack).
pub fn fill_item(host: &mut dyn Host, stack: &ItemStack, fluid: &str, required: i32) -> Option<ItemStack> {
    let step = sequenced::find(&*host, stack, Machine::Filling, &|step| {
        step.recipe
            .fluid_ingredients
            .first()
            .is_some_and(|i| fluid_matches(i, fluid) && fluid_amount(i) <= required)
    });
    if let Some(step) = step {
        // `rollResults(random).get(0)`: the item's next form, or the assembly's result.
        return step.roll(host).into_iter().next();
    }
    for recipe in matching(host, FILLING, stack) {
        let Some(ingredient) = recipe.fluid_ingredients.first() else {
            continue;
        };
        if fluid_matches(ingredient, fluid) && fluid_amount(ingredient) <= required {
            let dimension = host.dimension().to_owned();
            return recipes::roll(host, &dimension, &recipe.results).into_iter().next();
        }
    }
    // `GenericItemFilling.fillItem`.
    let item: &str = &stack.item;
    if item == GLASS_BOTTLE && fluid == WATER {
        return water_potion(host);
    }
    if item == GLASS_BOTTLE && potion_rules::is_potion_fluid(fluid) {
        return potion_bottle(host, fluid);
    }
    fluid_rules::filled(item, fluid).map(|c| ItemStack::new(c.full, 1))
}

/// What emptying one item gives.
#[derive(Clone, Debug, PartialEq)]
pub struct Emptied {
    pub fluid: String,
    pub amount: i32,
    /// What is left of the item (the bucket, the bottle).
    pub item: Option<ItemStack>,
}

/// `GenericItemEmptying.canItemBeEmptied`.
pub fn can_be_emptied(host: &dyn Host, stack: &ItemStack) -> bool {
    potion_fluid(stack).is_some()
        || !matching(host, EMPTYING, stack).is_empty()
        || fluid_rules::emptied(&stack.item).is_some()
}

/// `GenericItemEmptying.emptyItem` for one item of `stack`, simulated: the fluid it holds and what
/// is left of it.
pub fn empty_item(host: &dyn Host, stack: &ItemStack) -> Option<Emptied> {
    if let Some((fluid, amount)) = potion_fluid(stack) {
        // `PotionFluidHandler.emptyPotion`: a potion is 250 mB of its fluid (water for a bottle of water) in a
        // glass bottle.
        return Some(Emptied { fluid, amount, item: Some(ItemStack::new(GLASS_BOTTLE, 1)) });
    }
    if let Some(recipe) = matching(host, EMPTYING, stack).into_iter().next() {
        let fluid = recipe.fluid_results.first()?;
        return Some(Emptied {
            fluid: fluid.get("id")?.as_str()?.to_owned(),
            amount: fluid_amount(fluid),
            item: recipe.results.first().map(|output| output.stack.clone()),
        });
    }
    let container = fluid_rules::emptied(&stack.item)?;
    Some(Emptied {
        fluid: container.fluid.to_owned(),
        amount: container.amount,
        item: Some(ItemStack::new(container.empty, 1)),
    })
}

// --- the one tank of a Spout or an Item Drain ---------------------------------------------------

/// Horizon's own key for a block entity's tanks (`native_block_entity_fluids`).
pub const FLUIDS: &str = "fluids";

/// The fluid and amount in the first tank of an entity's saved data (`None` for an empty one).
pub fn tank_of(nbt: &crate::compat::Compound) -> (Option<String>, i32) {
    let Some(Tag::List(list)) = crate::util::get(nbt, FLUIDS) else {
        return (None, 0);
    };
    let Some(first) = list.first() else {
        return (None, 0);
    };
    let amount = match first.get("Amount") {
        Some(Tag::Int(n)) => *n,
        _ => 0,
    };
    let fluid = match first.get("Fluid") {
        Some(Tag::String(id)) => Some(id.to_string_lossy()).filter(|f| f != "minecraft:empty"),
        _ => None,
    };
    if amount > 0 { (fluid, amount) } else { (None, 0) }
}

/// Horizon's `write_tanks` for one tank: an empty one as `minecraft:empty`.
pub fn tank_tag(fluid: Option<&str>, amount: i32) -> Tag {
    let mut tank = crate::compat::Compound::new();
    let fluid = match (fluid, amount > 0) {
        (Some(fluid), true) => fluid,
        _ => "minecraft:empty",
    };
    tank.insert("Fluid".into(), Tag::String(fluid.into()));
    tank.insert("Amount".into(), Tag::Int(amount.max(0)));
    Tag::List(vec![Tag::Compound(tank)])
}

/// Writes the tank into the entity's data, only when it differs.
pub fn set_tank(nbt: &mut crate::compat::Compound, fluid: Option<&str>, amount: i32) {
    let tag = tank_tag(fluid, amount);
    if crate::util::differs(nbt, FLUIDS, &tag) {
        nbt.insert(FLUIDS.into(), tag);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tank_is_read_back_as_it_was_written() {
        let mut nbt = crate::compat::Compound::new();
        assert_eq!(tank_of(&nbt), (None, 0));
        set_tank(&mut nbt, Some("create:honey"), 250);
        assert_eq!(tank_of(&nbt), (Some("create:honey".to_owned()), 250));
        set_tank(&mut nbt, Some("create:honey"), 0);
        assert_eq!(tank_of(&nbt), (None, 0));
    }

    #[test]
    fn fluid_ingredients_match_by_fluid_or_tag() {
        let single = json!({"type": "neoforge:single", "amount": 500, "fluid": "minecraft:water"});
        assert!(fluid_matches(&single, "minecraft:water"));
        assert!(!fluid_matches(&single, "minecraft:lava"));
        assert_eq!(fluid_amount(&single), 500);
        let tag = json!({"type": "neoforge:tag", "amount": 250, "tag": "c:honey"});
        assert!(fluid_matches(&tag, "create:honey"));
        assert!(!fluid_matches(&tag, "create:chocolate"));
        // The datagen's `filling/glowstone`: a regular potion of night vision.
        let potion = json!({
            "type": "neoforge:components", "amount": 25, "fluids": "create:potion",
            "components": {
                "create:potion_fluid_bottle_type": "regular",
                "minecraft:potion_contents": {"potion": "minecraft:night_vision"},
            },
        });
        assert!(fluid_matches(&potion, "create:potion[bottle=regular,potion=minecraft:night_vision]"));
        assert!(!fluid_matches(&potion, "create:potion[bottle=regular,potion=minecraft:strength]"));
        assert!(!fluid_matches(&potion, "create:potion[bottle=splash,potion=minecraft:night_vision]"));
        assert!(!fluid_matches(&potion, "minecraft:water"));
        assert_eq!(fluid_amount(&potion), 25);
    }
}
