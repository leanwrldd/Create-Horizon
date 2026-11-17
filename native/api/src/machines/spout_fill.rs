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
//!   bottle, Builder's Tea) plus the glass bottle with water and the bottle of water: Horizon's
//!   `Host` has no item fluid handler to ask for other mods' containers, and potions other
//!   than water (`create:potion`) are not carried (see `fluids.rs`).
//! - The three `create:filling` recipes that take a `create:potion` fluid
//!   (`filling/glowstone`, `gunpowder`, `redstone`) are not declared, as nothing here holds that
//!   fluid.

use create_kinetics::rules::fluids as fluid_rules;
use create_kinetics::rules::sequenced::Machine;
use horizon_client::mod_api::serde_json::{Value, json};
use horizon_client::mod_api::{Hooks, Host, ItemStack, Tag};

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
/// `PotionFluidHandler.getRequiredAmountForFilledBottle`, and a bottle of water's worth.
const BOTTLE: i32 = fluid_rules::BOTTLE;

/// `FluidIngredient.test`: whether the recipe's fluid ingredient takes `fluid`. A
/// `neoforge:components` ingredient names a `create:potion` and matches nothing here.
fn fluid_matches(ingredient: &Value, fluid: &str) -> bool {
    match ingredient.get("type").and_then(Value::as_str) {
        Some("neoforge:single") => ingredient.get("fluid").and_then(Value::as_str) == Some(fluid),
        Some("neoforge:tag") => ingredient
            .get("tag")
            .and_then(Value::as_str)
            .is_some_and(|tag| fluid_rules::tag_holds(tag, fluid)),
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

/// `PotionFluidHandler.isPotionItem` for a water potion: a `minecraft:potion` whose contents are
/// plain water (`PotionContents.createItemStack(Items.POTION, Potions.WATER)`).
fn is_water_potion(stack: &ItemStack) -> bool {
    if &*stack.item != POTION {
        return false;
    }
    match stack.data().and_then(|data| data.other("minecraft:potion_contents")) {
        Some(Tag::String(potion)) => potion.to_string_lossy() == "minecraft:water",
        Some(tag @ Tag::Compound(_)) => matches!(
            tag.get("potion"),
            Some(Tag::String(potion)) if potion.to_string_lossy() == "minecraft:water"
        ),
        _ => false,
    }
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
    // `GenericItemFilling.getRequiredAmountForItem`.
    let item: &str = &stack.item;
    if item == GLASS_BOTTLE && fluid == WATER {
        return Some(BOTTLE);
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
    is_water_potion(stack)
        || !matching(host, EMPTYING, stack).is_empty()
        || fluid_rules::emptied(&stack.item).is_some()
}

/// `GenericItemEmptying.emptyItem` for one item of `stack`, simulated: the fluid it holds and what
/// is left of it.
pub fn empty_item(host: &dyn Host, stack: &ItemStack) -> Option<Emptied> {
    if is_water_potion(stack) {
        // `PotionFluidHandler.emptyPotion`: a bottle of water is 250 mB of water in a glass bottle.
        return Some(Emptied {
            fluid: WATER.to_owned(),
            amount: BOTTLE,
            item: Some(ItemStack::new(GLASS_BOTTLE, 1)),
        });
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
pub fn tank_of(nbt: &horizon_client::mod_api::Compound) -> (Option<String>, i32) {
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
    let mut tank = horizon_client::mod_api::Compound::new();
    let fluid = match (fluid, amount > 0) {
        (Some(fluid), true) => fluid,
        _ => "minecraft:empty",
    };
    tank.insert("Fluid".into(), Tag::String(fluid.into()));
    tank.insert("Amount".into(), Tag::Int(amount.max(0)));
    Tag::List(vec![Tag::Compound(tank)])
}

/// Writes the tank into the entity's data, only when it differs.
pub fn set_tank(nbt: &mut horizon_client::mod_api::Compound, fluid: Option<&str>, amount: i32) {
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
        let mut nbt = horizon_client::mod_api::Compound::new();
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
        // A potion's data components are not carried here.
        let potion = json!({"type": "neoforge:components", "amount": 25, "fluids": "create:potion"});
        assert!(!fluid_matches(&potion, "create:potion"));
    }
}
