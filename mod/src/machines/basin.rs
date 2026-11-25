//! The Basin (`BasinBlock`, `BasinBlockEntity`, `BasinInventory`, `BasinRecipe`, `BasinRenderer`):
//! nine input and nine output slots, four fluid tanks, the recipe matching the Mechanical Mixer
//! and the Mechanical Press share, and the spout that sends the output to the side it faces. The
//! rules (slot policy, taking ingredients, heat, the spout's choice) are
//! `create_kinetics::rules::basin`; this is the glue.
//!
//! What another module calls (the press for `create:compacting`, the mixer for `create:mixing`):
//! [`matching`], [`matches`], [`apply`], [`can_continue`], [`notify_removed`], [`signature`] and
//! [`input_stacks`], all `pub(crate)` below.
//!
//! Known differences from Create (for the README):
//! - The slots are the block entity's own item slots (`Items`: 0 to 8 the inputs, 9 to 17 the
//!   outputs), saved as Horizon saves them, not Create's `InputItems` / `OutputItems` compounds, so
//!   hoppers (`block_entity_container`) and drops on breaking work. The fluids are Horizon's tank
//!   list (`fluids`: tanks 0 and 1 are `inputTank`, 2 and 3 `outputTank`, 1000 mB each). Create's
//!   `PreferredSpoutput`, `DisabledSpoutput`, `Overflow` and `FluidOverflow` keys are kept.
//! - Items dropped on top are taken in by a scan every two ticks, not when they land
//!   (`updateEntityAfterFallOn`).
//! - Output goes to the side the basin faces when the block beside it is empty and the block
//!   beside and below is a Depot, a Belt, a Basin or a vanilla chest, barrel or hopper (Create asks
//!   for a `DirectBeltInputBehaviour`, which funnels, tunnels and chutes also have: none exist here
//!   yet). A Belt gets the item as an item entity dropped on it, a Depot has it put on it.
//! - No recipe filter (the value box), no output animation, no fluid or item particles, no `canSurvive` check against a mixer above, no analog output.
//! - Fluids are Horizon tanks: water, lava, milk (`neoforge:milk`, as Create's `c:milk` holds it),
//!   `create:honey`, `create:chocolate` and `create:tea` hold and match. Only water and lava are
//!   drawn with their own texture; any other fluid is drawn as a tinted box. Buckets of water,
//!   lava and milk fill and empty the basin (the other fluid containers of `GenericItemFilling`
//!   are not ported); a sponge drains it.
//! - Items with a crafting remainder leave nothing behind (`getRemainingItems`).
//! - Recipes: the `create:mixing` and `create:compacting` files, and vanilla's crafting recipes
//!   (`Host::recipes_of_type("minecraft:crafting")`, data packs included) the way
//!   `MechanicalMixerBlockEntity.matchStaticFilters` and `MechanicalPressBlockEntity.matchStaticFilters`
//!   take them: shapeless ones of two or more ingredients to the mixer, shaped or shapeless ones of
//!   four or nine identical ingredients to the press (`rules::vanilla_recipes`), each as a
//!   `BasinRecipe.convertShapeless` (one ingredient per slot, one result). Brewing in the mixer
//!   (`allowBrewingInMixer`) is missing.

// Shared with the mixer, which doesn't use every helper.
#![allow(dead_code)]

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use create_kinetics::belt::movement_facing;
use create_kinetics::rules::basin::{
    self as rules, HeatCondition, HeatLevel, INPUT_SLOTS, SLOTS, SLOT_LIMIT, Spout, TANKS,
    TANK_CAPACITY,
};
use create_kinetics::rules::basin_output as rules_output;
use create_kinetics::rules::potion_mixing::{self, Brew};
use create_kinetics::rules::vanilla_recipes;
use create_kinetics::Dir;
use crate::compat::serde_json::Value;
use crate::compat::{
    BlockEntityContainerEvent, BlockEntityEvent, BlockEntityFluidsEvent, BlockEntitySubmitEvent,
    BlockEntityTickEvent, BlockEntityTicksEvent, BlockEntityUpdateTagEvent, Compound,
    ContainerAcceptsEvent, CubeFace, DestroyBlockEvent, Direction, DisplayContext, EquipmentSlot,
    Flow, Hooks, Host, Ingredient, ItemStack, ModBlockEntity, ModContainer, ModFluidTanks,
    NativeFluidRenderOptions, NativeFluidStack, RandomSource, RightClickBlockEvent, Tag,
    UseItemOnEvent,
};

use crate::util;

pub(crate) const ID: &str = "create:basin";
const MIXING: &str = "create:mixing";
/// Vanilla's crafting recipes, which the mixer and the press take some of.
const CRAFTING: &str = "minecraft:crafting";
const BREWING: &str = "minecraft:brewing";
const COMPACTING: &str = "create:compacting";

/// Horizon's own key for a block entity's tanks (`native_block_entity_fluids`).
const FLUIDS: &str = "fluids";
// Create's keys (`BasinBlockEntity.write`).
const PREFERRED: &str = "PreferredSpoutput";
const DISABLED: &str = "DisabledSpoutput";
const OVERFLOW: &str = "Overflow";
const FLUID_OVERFLOW: &str = "FluidOverflow";
/// Whether the mixer above is whirling the contents (`areFluidsMoving`), sent to clients.
const MOVING: &str = "Moving";
/// What clients draw: the non-empty stacks, as clients have no item slots.
const DRAWN: &str = "DrawItems";
/// The stacks the basin just gave out (`visualizedOutputItems`) and the tick they were given at. A Ponder scene sets them
/// (`modifyBlockEntityNBT`); the game's own basin does not send them yet.
const VISUALIZED: &str = "VisualizedItems";
const VISUALIZED_SINCE: &str = "VisualizedSince";
/// Set on a mixer or press two blocks up when the basin was emptied (`basinRemoved`); the
/// operator clears it. Any `BasinOperatingBlockEntity` in this mod honours it.
pub(crate) const OPERATOR_FLAG: &str = "BasinRemoved";
const OPERATORS: [&str; 2] = ["create:mechanical_mixer", "create:mechanical_press"];

pub(crate) type Pos = [i32; 3];

/// What clients were last sent for each basin, so a change a hopper made is sent too.
static SENT: Mutex<Option<HashMap<Pos, u64>>> = Mutex::new(None);

struct Basin;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Basin)
}

// --- the contents ---------------------------------------------------------------------------------

/// One of the four tanks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Tank {
    pub fluid: Option<String>,
    pub amount: i32,
}

/// Everything a recipe can take from or give to a basin.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Contents {
    pub items: Vec<Option<ItemStack>>,
    pub tanks: Vec<Tank>,
    /// `spoutputBuffer`.
    pub overflow: Vec<ItemStack>,
    /// `spoutputFluidBuffer`.
    pub fluid_overflow: Vec<(String, i32)>,
}

fn tag_string(tag: &Tag) -> Option<String> {
    match tag {
        Tag::String(s) => Some(s.to_string_lossy()),
        _ => None,
    }
}

fn read_tanks(nbt: &Compound) -> Vec<Tank> {
    let mut tanks: Vec<Tank> = match util::get(nbt, FLUIDS) {
        Some(Tag::List(list)) => list
            .iter()
            .take(TANKS)
            .map(|tag| {
                let fluid = tag
                    .get("Fluid")
                    .and_then(tag_string)
                    .filter(|f| f != "minecraft:empty");
                let amount = match tag.get("Amount") {
                    Some(Tag::Int(n)) => *n,
                    _ => 0,
                };
                Tank { fluid, amount }
            })
            .collect(),
        _ => Vec::new(),
    };
    tanks.resize(TANKS, Tank::default());
    tanks
}

/// Horizon's `write_tanks`: an empty tank as `minecraft:empty` so the numbers stay in place.
fn tanks_tag(tanks: &[Tank]) -> Tag {
    Tag::List(
        tanks
            .iter()
            .map(|tank| {
                let mut map = Compound::new();
                let fluid = match (&tank.fluid, tank.amount > 0) {
                    (Some(fluid), true) => fluid.as_str(),
                    _ => "minecraft:empty",
                };
                map.insert("Fluid".into(), Tag::String(fluid.into()));
                map.insert("Amount".into(), Tag::Int(tank.amount.max(0)));
                Tag::Compound(map)
            })
            .collect(),
    )
}

fn fluid_stacks_of(nbt: &Compound) -> Vec<(String, i32)> {
    match util::get(nbt, FLUID_OVERFLOW) {
        Some(Tag::List(list)) => list
            .iter()
            .filter_map(|tag| {
                let id = tag.get("id").and_then(tag_string)?;
                let amount = match tag.get("amount") {
                    Some(Tag::Int(n)) => *n,
                    _ => return None,
                };
                (amount > 0).then_some((id, amount))
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn fluid_stacks_tag(stacks: &[(String, i32)]) -> Tag {
    Tag::List(
        stacks
            .iter()
            .map(|(id, amount)| {
                let mut map = Compound::new();
                map.insert("id".into(), Tag::String(id.as_str().into()));
                map.insert("amount".into(), Tag::Int(*amount));
                Tag::Compound(map)
            })
            .collect(),
    )
}

impl Contents {
    pub(crate) fn from_entity(entity: &ModBlockEntity) -> Self {
        let mut items = entity.items.clone();
        items.resize(SLOTS, None);
        Contents {
            items,
            tanks: read_tanks(&entity.nbt),
            overflow: util::stacks_of(&entity.nbt, OVERFLOW),
            fluid_overflow: fluid_stacks_of(&entity.nbt),
        }
    }

    /// `BasinBlockEntity.isEmpty`.
    pub(crate) fn is_empty(&self) -> bool {
        self.items.iter().all(Option::is_none)
            && self.tanks.iter().all(|t| t.amount <= 0 || t.fluid.is_none())
    }

    /// `canContinueProcessing`: nothing is waiting to leave by the spout.
    pub(crate) fn can_continue(&self) -> bool {
        self.overflow.is_empty() && self.fluid_overflow.is_empty()
    }

    fn write_to(&self, entity: &mut ModBlockEntity) {
        if entity.items != self.items {
            entity.items = self.items.clone();
        }
        let tanks = tanks_tag(&self.tanks);
        if util::differs(&entity.nbt, FLUIDS, &tanks) {
            entity.nbt.insert(FLUIDS.into(), tanks);
        }
        let overflow = util::stacks_tag(&self.overflow);
        if util::differs(&entity.nbt, OVERFLOW, &overflow) {
            entity.nbt.insert(OVERFLOW.into(), overflow);
        }
        let fluid_overflow = fluid_stacks_tag(&self.fluid_overflow);
        if util::differs(&entity.nbt, FLUID_OVERFLOW, &fluid_overflow) {
            entity.nbt.insert(FLUID_OVERFLOW.into(), fluid_overflow);
        }
    }
}

/// A hash of what a basin holds, for "did the contents change" (`contentsChanged`).
fn signature_of(contents: &Contents, heat: HeatLevel) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for stack in contents.items.iter().flatten() {
        (&*stack.item, stack.count, stack.damage).hash(&mut hasher);
    }
    for (slot, stack) in contents.items.iter().enumerate() {
        (slot, stack.is_some()).hash(&mut hasher);
    }
    for tank in &contents.tanks {
        (&tank.fluid, tank.amount).hash(&mut hasher);
    }
    (contents.overflow.len(), contents.fluid_overflow.len(), heat as u8).hash(&mut hasher);
    hasher.finish()
}

// --- reading a basin from outside ---------------------------------------------------------------------

fn add(a: Pos, b: Pos) -> Pos {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub(crate) fn is_basin(host: &dyn Host, pos: Pos) -> bool {
    host.block(pos) == ID
}

/// The basin at `pos`: its contents. Not the one ticking now (it is out of the world there).
pub(crate) fn contents(host: &dyn Host, pos: Pos) -> Option<Contents> {
    if !is_basin(host, pos) {
        return None;
    }
    let entity = host.mod_block_entity(pos)?;
    (entity.id == ID).then(|| Contents::from_entity(&entity))
}

/// Every non-empty stack in the input slots (the press's particles).
pub(crate) fn input_stacks(host: &dyn Host, pos: Pos) -> Vec<ItemStack> {
    contents(host, pos)
        .map(|c| c.items[..INPUT_SLOTS].iter().flatten().cloned().collect())
        .unwrap_or_default()
}

/// `BasinBlockEntity.canContinueProcessing` of the basin at `pos`; false without one.
pub(crate) fn can_continue(host: &dyn Host, pos: Pos) -> bool {
    contents(host, pos).is_some_and(|c| c.can_continue())
}

/// `BasinBlockEntity.getHeatLevel`: the `blaze` state of the block below, or a lit passive heater
/// (`data/create/tags/block/passive_boiler_heaters`: blaze burners, magma, lava, campfires, fire)
/// as smouldering.
pub(crate) fn heat_level(host: &dyn Host, basin: Pos) -> HeatLevel {
    let below = add(basin, [0, -1, 0]);
    let name = host.block(below);
    if let Some(level) = host
        .block_property(below, "blaze")
        .as_deref()
        .and_then(HeatLevel::from_name)
    {
        return level;
    }
    let campfire = name == "minecraft:campfire" || name == "minecraft:soul_campfire";
    let heater = campfire
        || matches!(
            name.as_str(),
            "create:blaze_burner"
                | "create:lit_blaze_burner"
                | "minecraft:magma_block"
                | "minecraft:lava"
                | "minecraft:fire"
                | "minecraft:soul_fire"
        );
    // `BlockHelper.isNotUnheated`: an unlit campfire doesn't heat.
    let unheated = campfire && host.block_property(below, "lit").as_deref() == Some("false");
    rules::passive_heat(heater, unheated)
}

/// A hash of the contents and the heat below, to tell an operator the basin changed.
pub(crate) fn signature(host: &dyn Host, pos: Pos) -> Option<u64> {
    let contents = contents(host, pos)?;
    Some(signature_of(&contents, heat_level(host, pos)))
}

/// Tells the mixer or press two blocks above that the basin was emptied or removed
/// (`BasinBlockEntity.onEmptied`: `basinRemoved = true`).
pub(crate) fn notify_removed(host: &mut dyn Host, basin: Pos) {
    let operator = add(basin, [0, 2, 0]);
    let is_operator = host
        .mod_block_entity(operator)
        .is_some_and(|e| OPERATORS.contains(&e.id.as_str()));
    if is_operator {
        host.change_mod_block_entity(operator, &mut |e| {
            e.nbt.insert(OPERATOR_FLAG.into(), Tag::Byte(1));
        });
    }
}

// --- recipes ---------------------------------------------------------------------------------------

/// Which recipe type an operator looks for: the mixer mixes, the press compacts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Mixing,
    Compacting,
}

impl Kind {
    fn recipe_type(self) -> &'static str {
        match self {
            Kind::Mixing => MIXING,
            Kind::Compacting => COMPACTING,
        }
    }
}

/// An item ingredient: an `Ingredient`, or NeoForge's `neoforge:block_tag` (an item whose block is
/// in a block tag: Create's mud recipe).
#[derive(Clone)]
pub(crate) enum ItemIngredient {
    Plain(Ingredient),
    BlockTag(String),
}

impl ItemIngredient {
    fn test(&self, host: &dyn Host, stack: &ItemStack) -> bool {
        match self {
            ItemIngredient::Plain(ingredient) => ingredient.test(stack),
            ItemIngredient::BlockTag(tag) => host.block_name_in_tag(&stack.item, tag).unwrap_or(false),
        }
    }
}

/// `SizedFluidIngredient`: one fluid or a fluid tag, and how much.
#[derive(Clone)]
pub(crate) enum FluidIngredient {
    Single(String, i32),
    Tag(String, i32),
}

/// What Create's fluid tags hold here: `c:milk` is NeoForge's milk, `c:honey` Create's honey.
fn tag_holds(tag: &str, fluid: &str) -> bool {
    match tag {
        "c:milk" => fluid == "neoforge:milk",
        "c:honey" => fluid == "create:honey",
        "c:chocolate" => fluid == "create:chocolate",
        "c:tea" => fluid == "create:tea",
        "minecraft:water" | "c:water" => fluid == "minecraft:water",
        "c:lava" => fluid == "minecraft:lava",
        _ => false,
    }
}

impl FluidIngredient {
    fn amount(&self) -> i32 {
        match self {
            FluidIngredient::Single(_, amount) | FluidIngredient::Tag(_, amount) => *amount,
        }
    }

    fn test(&self, fluid: &str) -> bool {
        match self {
            FluidIngredient::Single(id, _) => id == fluid,
            FluidIngredient::Tag(tag, _) => tag_holds(tag, fluid),
        }
    }
}

/// A result stack and its `chance` (`ProcessingOutput`).
#[derive(Clone)]
pub(crate) struct Output {
    pub stack: ItemStack,
    pub chance: f32,
}

/// A `create:mixing` or `create:compacting` recipe (`BasinRecipe`).
#[derive(Clone)]
pub(crate) struct Recipe {
    /// `create:mixing/brass_ingot`.
    pub id: String,
    pub kind: Kind,
    pub ingredients: Vec<ItemIngredient>,
    pub fluids: Vec<FluidIngredient>,
    pub results: Vec<Output>,
    pub fluid_results: Vec<(String, i32)>,
    /// `processing_time`; 0 when the file names none.
    pub processing_time: i32,
    pub heat: HeatCondition,
}

fn parse_item_ingredient(host: &dyn Host, json: &Value) -> Option<ItemIngredient> {
    if json.get("type").and_then(Value::as_str) == Some("neoforge:block_tag") {
        return Some(ItemIngredient::BlockTag(json.get("tag")?.as_str()?.to_owned()));
    }
    host.recipe_ingredient(&crate::recipes::modern_ingredient(json))
        .map(ItemIngredient::Plain)
}

fn parse_fluid_ingredient(json: &Value) -> Option<FluidIngredient> {
    let amount = json.get("amount")?.as_i64()? as i32;
    match json.get("type").and_then(Value::as_str)? {
        "neoforge:single" => Some(FluidIngredient::Single(json.get("fluid")?.as_str()?.to_owned(), amount)),
        "neoforge:tag" => Some(FluidIngredient::Tag(json.get("tag")?.as_str()?.to_owned(), amount)),
        _ => None,
    }
}

/// A recipe file as Create's datagen writes it for 1.21.1. `None` when an ingredient can't be read
/// (a tag nothing declares), which leaves the recipe unusable.
fn parse(host: &dyn Host, id: &str, kind: Kind, json: &Value) -> Option<Recipe> {
    let mut ingredients = Vec::new();
    let mut fluids = Vec::new();
    for entry in json.get("ingredients")?.as_array()? {
        if entry.get("amount").is_some() {
            fluids.push(parse_fluid_ingredient(entry)?);
        } else {
            ingredients.push(parse_item_ingredient(host, entry)?);
        }
    }
    let mut results = Vec::new();
    let mut fluid_results = Vec::new();
    for result in json.get("results").and_then(Value::as_array).into_iter().flatten() {
        if let Some(amount) = result.get("amount").and_then(Value::as_i64) {
            fluid_results.push((result.get("id")?.as_str()?.to_owned(), amount as i32));
            continue;
        }
        // `recipe_result` reads only `id`, `count` and `components`: `chance` is Create's.
        let mut plain = result.as_object()?.clone();
        let chance = plain.remove("chance").and_then(|c| c.as_f64()).unwrap_or(1.0) as f32;
        let stack = host.recipe_result(&Value::Object(plain))?;
        results.push(Output { stack, chance });
    }
    Some(Recipe {
        id: id.to_owned(),
        kind,
        ingredients,
        fluids,
        results,
        fluid_results,
        processing_time: json.get("processing_time").and_then(Value::as_i64).unwrap_or(0) as i32,
        heat: HeatCondition::from_name(json.get("heat_requirement").and_then(Value::as_str).unwrap_or("")),
    })
}

/// The items an ingredient lists, in order (`Ingredient.getItems`); none for an empty cell.
fn item_list(ingredient: &Ingredient) -> Vec<String> {
    let inner = &ingredient.0;
    if !inner.order.is_empty() {
        return inner.order.clone();
    }
    let mut items: Vec<String> = inner.items.iter().cloned().collect();
    items.sort();
    items
}

/// A vanilla crafting recipe as a basin recipe, if the mixer or the press takes it
/// (`rules::vanilla_recipes`): `BasinRecipe.convertShapeless`, one ingredient per cell and the
/// result item.
fn from_crafting(host: &dyn Host, id: &str, json: &Value) -> Option<Recipe> {
    let shapeless = match json.get("type")?.as_str()? {
        "minecraft:crafting_shapeless" => true,
        "minecraft:crafting_shaped" => false,
        _ => return None,
    };
    // `Recipe.getIngredients`: a shapeless recipe's list, a shaped one's cells (empty ones too).
    let mut cells: Vec<Option<Ingredient>> = Vec::new();
    if shapeless {
        for entry in json.get("ingredients")?.as_array()? {
            cells.push(Some(host.recipe_ingredient(&crate::recipes::modern_ingredient(entry))?));
        }
    } else {
        let key = json.get("key")?.as_object()?;
        for row in json.get("pattern")?.as_array()? {
            for c in row.as_str()?.chars() {
                cells.push(match c {
                    ' ' => None,
                    c => Some(
                        host.recipe_ingredient(&crate::recipes::modern_ingredient(key.get(&c.to_string())?))?,
                    ),
                });
            }
        }
    }
    let lists: Vec<Vec<String>> =
        cells.iter().map(|cell| cell.as_ref().map(item_list).unwrap_or_default()).collect();
    let kind = if vanilla_recipes::press_takes(&lists, id) {
        Kind::Compacting
    } else if vanilla_recipes::mixer_takes(shapeless, &lists, id) {
        Kind::Mixing
    } else {
        return None;
    };
    let stack = host.recipe_result(json.get("result")?)?;
    Some(Recipe {
        id: id.to_owned(),
        kind,
        ingredients: cells.into_iter().flatten().map(ItemIngredient::Plain).collect(),
        fluids: Vec::new(),
        results: vec![Output { stack, chance: 1.0 }],
        fluid_results: Vec::new(),
        processing_time: 0,
        heat: HeatCondition::None,
    })
}

type CraftingCache = Option<(usize, usize, Arc<Vec<Recipe>>)>;
static CRAFTING_RECIPES: Mutex<CraftingCache> = Mutex::new(None);

/// The vanilla crafting recipes the mixer and the press take, found once: the game hands out the
/// same table until a reload, so the answer is kept while it is the same table.
fn crafting_recipes(host: &dyn Host) -> Arc<Vec<Recipe>> {
    let table = host.recipes_of_type(CRAFTING);
    let key = (table.as_ptr() as usize, table.len());
    let mut cache = CRAFTING_RECIPES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((ptr, len, found)) = cache.as_ref()
        && (*ptr, *len) == key
    {
        return Arc::clone(found);
    }
    let found: Arc<Vec<Recipe>> =
        Arc::new(table.iter().filter_map(|recipe| from_crafting(host, &recipe.id, &recipe.json)).collect());
    *cache = Some((key.0, key.1, Arc::clone(&found)));
    found
}

type PotionCache = Option<(usize, usize, Arc<Vec<Recipe>>)>;
static POTION_RECIPES: Mutex<PotionCache> = Mutex::new(None);

/// A string or a list of strings of a recipe file, as a list.
fn strings(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(one)) => vec![one.clone()],
        Some(Value::Array(many)) => many.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect(),
        _ => Vec::new(),
    }
}

/// The items a brewing recipe's `item` names (items and `#tags`), resolved and in order.
fn brew_items(host: &dyn Host, names: Vec<String>) -> Vec<String> {
    if names.is_empty() {
        return Vec::new();
    }
    let json = Value::Array(names.into_iter().map(Value::String).collect());
    host.recipe_ingredient(&json).map(|ingredient| item_list(&ingredient)).unwrap_or_default()
}

/// A `minecraft:potion_contents` of a recipe: a potion id, or an object with `potion` (an output) or `potions` (an
/// input's filter).
fn contents_potions(value: Option<&Value>) -> Option<Vec<String>> {
    let value = value?;
    if let Some(one) = value.as_str() {
        return Some(vec![one.to_owned()]);
    }
    let list = strings(value.get("potions").or_else(|| value.get("potion")));
    (!list.is_empty()).then_some(list)
}

/// One `minecraft:brewing` recipe file as `rules::potion_mixing` reads it.
fn read_brew(host: &dyn Host, json: &Value) -> Option<Brew> {
    let input = json.get("input")?;
    let output = json.get("output")?;
    let potion_contents = |value: &Value| {
        contents_potions(value.get("potion_contents").or_else(|| value.get("components")?.get("minecraft:potion_contents")))
    };
    Some(Brew {
        input_items: brew_items(host, strings(input.get("item"))),
        input_potions: potion_contents(input),
        reagent: brew_items(host, strings(json.get("reagent")?.get("item"))),
        output_item: output.get("id")?.as_str()?.to_owned(),
        output_potion: potion_contents(output).and_then(|list| list.into_iter().next()),
    })
}

/// Create's `PotionMixingRecipes.createRecipes`: the brewing recipes as mixing recipes (a potion fluid and an ingredient
/// over heat), found once while the game hands out the same brewing table.
fn potion_recipes(host: &dyn Host) -> Arc<Vec<Recipe>> {
    let table = host.recipes_of_type(BREWING);
    let key = (table.as_ptr() as usize, table.len());
    let mut cache = POTION_RECIPES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((ptr, len, found)) = cache.as_ref()
        && (*ptr, *len) == key
    {
        return Arc::clone(found);
    }
    let brews: Vec<Brew> = table.iter().filter_map(|recipe| read_brew(host, &recipe.json)).collect();
    let found: Arc<Vec<Recipe>> = Arc::new(
        potion_mixing::mixes(&brews)
            .into_iter()
            .filter_map(|mix| {
                let json = Value::Array(mix.reagent.iter().cloned().map(Value::String).collect());
                let ingredient = host.recipe_ingredient(&json)?;
                Some(Recipe {
                    id: mix.id,
                    kind: Kind::Mixing,
                    ingredients: vec![ItemIngredient::Plain(ingredient)],
                    fluids: vec![FluidIngredient::Single(mix.from, potion_mixing::AMOUNT)],
                    results: Vec::new(),
                    fluid_results: vec![(mix.to, potion_mixing::AMOUNT)],
                    processing_time: 0,
                    heat: HeatCondition::Heated,
                })
            })
            .collect(),
    );
    *cache = Some((key.0, key.1, Arc::clone(&found)));
    found
}

/// Every recipe of `kind`, in the order the game holds them (Create's own, then vanilla's crafting
/// recipes it takes).
pub(crate) fn recipes(host: &dyn Host, kind: Kind) -> Vec<Recipe> {
    let mut all: Vec<Recipe> = host
        .recipes_of_type(kind.recipe_type())
        .iter()
        .filter_map(|recipe| parse(host, &recipe.id, kind, &recipe.json))
        .collect();
    all.extend(crafting_recipes(host).iter().filter(|recipe| recipe.kind == kind).cloned());
    // `MechanicalMixerBlockEntity.getMatchingRecipes`: the brewing mixes come after, for the mixer only.
    if kind == Kind::Mixing {
        all.extend(potion_recipes(host).iter().cloned());
    }
    all
}

/// The recipe called `id` (`create:mixing/brass_ingot`), whichever of the two types it is.
pub(crate) fn recipe_by_id(host: &dyn Host, id: &str) -> Option<Recipe> {
    [Kind::Mixing, Kind::Compacting]
        .into_iter()
        .find_map(|kind| {
            host.recipes_of_type(kind.recipe_type())
                .iter()
                .find(|recipe| recipe.id == id)
                .and_then(|recipe| parse(host, &recipe.id, kind, &recipe.json))
        })
        .or_else(|| crafting_recipes(host).iter().find(|recipe| recipe.id == id).cloned())
        .or_else(|| potion_recipes(host).iter().find(|recipe| recipe.id == id).cloned())
}

fn max_stack(host: &dyn Host, stack: &ItemStack) -> u32 {
    host.max_stack_size(&stack.item).unwrap_or(64)
}

/// `ItemHandlerHelper.insertItemStacked` into the output slots of `items`, as the glue's stacks.
fn insert_outputs(host: &dyn Host, items: &mut [Option<ItemStack>], stack: ItemStack) -> bool {
    let limit = SLOT_LIMIT.min(max_stack(host, &stack));
    rules::insert_stacked(
        &mut items[INPUT_SLOTS..],
        stack,
        |s| s.count,
        |s, n| s.count = n,
        util::same_item,
        limit,
    )
    .is_none()
}

/// `SmartFluidTankBehaviour.InternalFluidHandler.forceFill`: into the tanks holding the fluid,
/// then the empty ones, among tanks `range`. True when it all fit.
fn force_fill(tanks: &mut [Tank], range: std::ops::Range<usize>, fluid: &str, amount: i32) -> bool {
    let mut left = amount;
    for pass in 0..2 {
        for tank in &mut tanks[range.clone()] {
            if left <= 0 {
                break;
            }
            let fits = if pass == 0 {
                tank.fluid.as_deref() == Some(fluid) && tank.amount > 0
            } else {
                tank.fluid.is_none() || tank.amount <= 0
            };
            if !fits {
                continue;
            }
            let room = TANK_CAPACITY - tank.amount.max(0);
            let add = room.min(left);
            if add <= 0 {
                continue;
            }
            if pass == 1 {
                tank.amount = 0;
            }
            tank.amount += add;
            tank.fluid = Some(fluid.to_owned());
            left -= add;
        }
    }
    left <= 0
}

/// `BasinBlockEntity.acceptOutputs` on a working copy of the contents: results go into the output
/// slots and tanks when the basin faces down, or into the spout's buffers when it faces a side.
/// False when they do not fit.
fn accept_outputs(
    host: &dyn Host,
    contents: &mut Contents,
    facing: Dir,
    items: &[ItemStack],
    fluids: &[(String, i32)],
    target_has_tank: bool,
) -> bool {
    if facing != Dir::Down {
        // The buffer takes any item; a fluid goes to the buffer only where a tank takes it, else
        // into the basin's own output tanks (`acceptFluidOutputsIntoBasin`).
        for stack in items {
            contents.overflow.push(stack.clone());
        }
        for (fluid, amount) in fluids {
            if target_has_tank {
                contents.fluid_overflow.push((fluid.clone(), *amount));
            } else if !force_fill(&mut contents.tanks, 2..TANKS, fluid, *amount) {
                return false;
            }
        }
        return true;
    }
    for stack in items {
        if !insert_outputs(host, &mut contents.items, stack.clone()) {
            return false;
        }
    }
    for (fluid, amount) in fluids {
        if !force_fill(&mut contents.tanks, 2..TANKS, fluid, *amount) {
            return false;
        }
    }
    true
}

/// `BasinRecipe.apply` on a copy of `contents`: the heat, the ingredients and room for the
/// results. `rolled` are the results to hand out (Create rolls them once per application) or
/// `None` to test with every result. The changed copy, or `None` where the recipe doesn't apply.
fn plan(
    host: &dyn Host,
    contents: &Contents,
    recipe: &Recipe,
    heat: HeatLevel,
    facing: Dir,
    target_has_tank: bool,
    rolled: Option<Vec<ItemStack>>,
) -> Option<Contents> {
    if !recipe.heat.test(heat) {
        return None;
    }
    let mut next = contents.clone();
    // Items.
    let counts: Vec<u32> = next.items.iter().map(|s| s.as_ref().map_or(0, |s| s.count)).collect();
    let matrix: Vec<Vec<bool>> = recipe
        .ingredients
        .iter()
        .map(|ingredient| {
            next.items
                .iter()
                .map(|slot| slot.as_ref().is_some_and(|stack| ingredient.test(host, stack)))
                .collect()
        })
        .collect();
    let taken = rules::assign_items(&counts, &matrix)?;
    for (slot, n) in taken.iter().enumerate() {
        if *n == 0 {
            continue;
        }
        if let Some(stack) = &mut next.items[slot] {
            stack.count -= n;
            if stack.count == 0 {
                next.items[slot] = None;
            }
        }
    }
    // Fluids.
    if !recipe.fluids.is_empty() {
        let mut amounts: Vec<i32> = next
            .tanks
            .iter()
            .map(|t| if t.fluid.is_some() { t.amount } else { 0 })
            .collect();
        let matrix: Vec<Vec<bool>> = recipe
            .fluids
            .iter()
            .map(|ing| {
                next.tanks
                    .iter()
                    .map(|t| t.fluid.as_deref().is_some_and(|f| ing.test(f)))
                    .collect()
            })
            .collect();
        let required: Vec<i32> = recipe.fluids.iter().map(FluidIngredient::amount).collect();
        // Both passes of Create's loop: the simulation, then the real drain.
        if !rules::drain_fluids(&mut amounts.clone(), &matrix, &required, true) {
            return None;
        }
        rules::drain_fluids(&mut amounts, &matrix, &required, false);
        for (tank, amount) in next.tanks.iter_mut().zip(amounts) {
            tank.amount = amount;
            if amount <= 0 {
                tank.fluid = None;
            }
        }
    }
    // Results.
    let items: Vec<ItemStack> = rolled.unwrap_or_else(|| recipe.results.iter().map(|o| o.stack.clone()).collect());
    if !accept_outputs(host, &mut next, facing, &items, &recipe.fluid_results, target_has_tank) {
        return None;
    }
    Some(next)
}

/// What the spout needs to know to take an output: the side and whether a tank is there.
fn spout_of(host: &dyn Host, basin: Pos) -> (Dir, bool) {
    let facing = util::facing(host, basin, "facing").unwrap_or(Dir::Down);
    if facing == Dir::Down {
        return (facing, false);
    }
    let target = add(add(basin, [0, -1, 0]), facing.normal());
    let has_tank = host
        .fluid_tanks(host.dimension(), target, util::direction(facing.opposite()))
        .is_some();
    (facing, has_tank)
}

/// `BasinRecipe.match` without the filter: the recipe's heat, ingredients and room for results,
/// as the basin at `basin` is now.
pub(crate) fn matches(host: &dyn Host, basin: Pos, recipe: &Recipe) -> bool {
    let Some(contents) = contents(host, basin) else {
        return false;
    };
    let (facing, has_tank) = spout_of(host, basin);
    let heat = heat_level(host, basin);
    plan(host, &contents, recipe, heat, facing, has_tank, None).is_some()
}

/// `BasinOperatingBlockEntity.getMatchingRecipes` for a type: the recipes that match the basin
/// at `basin`, the ones with most ingredients first (`list.sort`). Empty for an empty basin.
pub(crate) fn matching(host: &dyn Host, basin: Pos, kind: Kind) -> Vec<Recipe> {
    let Some(contents) = contents(host, basin) else {
        return Vec::new();
    };
    if contents.is_empty() {
        return Vec::new();
    }
    let (facing, has_tank) = spout_of(host, basin);
    let heat = heat_level(host, basin);
    let mut list: Vec<Recipe> = recipes(host, kind)
        .into_iter()
        .filter(|recipe| plan(host, &contents, recipe, heat, facing, has_tank, None).is_some())
        .collect();
    list.sort_by_key(|recipe| std::cmp::Reverse(recipe.ingredients.len()));
    list
}

/// `BasinRecipe.apply`: takes the ingredients out of the basin at `basin` and puts the results in
/// (or out by its spout). False, with nothing changed, where the recipe doesn't apply.
pub(crate) fn apply(host: &mut dyn Host, basin: Pos, recipe: &Recipe) -> bool {
    let Some(contents) = contents(host, basin) else {
        return false;
    };
    let (facing, has_tank) = spout_of(host, basin);
    let heat = heat_level(host, basin);
    // `rollResults`, once.
    let dimension = host.dimension();
    let mut rolled = Vec::new();
    for output in &recipe.results {
        let kept = output.chance >= 1.0
            || host
                .level_random(dimension)
                // `ProcessingOutput.rollOutput`: kept when `nextFloat() <= chance`.
                .is_some_and(|random| random.next_float() <= output.chance);
        if kept {
            rolled.push(output.stack.clone());
        }
    }
    let Some(next) = plan(host, &contents, recipe, heat, facing, has_tank, Some(rolled)) else {
        return false;
    };
    host.change_mod_block_entity(basin, &mut |entity| next.write_to(entity));
    host.block_entity_sync(basin);
    true
}

// --- the spout -----------------------------------------------------------------------------------------

/// Vanilla blocks that take an item from the spout (an inventory where Create asks for a belt
/// input behaviour).
const INVENTORIES: [&str; 6] = [
    "minecraft:chest",
    "minecraft:trapped_chest",
    "minecraft:barrel",
    "minecraft:hopper",
    "minecraft:dropper",
    "minecraft:dispenser",
];

/// `BasinBlock.canOutputTo`: the block beside the basin lets the spout through and the block
/// below that one takes items. A belt turned the other way, or one that runs towards the basin's
/// side, doesn't (`belt.getSpeed() == 0 || belt.getMovementFacing() != direction.getOpposite()`).
fn can_output_to(host: &dyn Host, basin: Pos, direction: Dir) -> bool {
    let neighbour = add(basin, direction.normal());
    if !host.block_collision(neighbour).is_empty() {
        return false;
    }
    let output = add(neighbour, [0, -1, 0]);
    let name = host.block(output);
    match name.as_str() {
        "create:depot" | ID => true,
        "create:belt" => {
            let rpm = host
                .mod_block_entity(output)
                .and_then(|e| crate::float(&e.nbt, crate::SPEED))
                .unwrap_or(0.0);
            let facing = util::facing(host, output, "facing").unwrap_or(Dir::North);
            rpm == 0.0 || movement_facing(rpm, facing) != direction.opposite()
        }
        other => INVENTORIES.contains(&other),
    }
}

fn read_spout(nbt: &Compound) -> Spout {
    let dir_of = |tag: &Tag| tag_string(tag).and_then(|s| Dir::from_name(&s.to_ascii_lowercase()));
    Spout {
        preferred: util::get(nbt, PREFERRED).and_then(dir_of),
        disabled: match util::get(nbt, DISABLED) {
            Some(Tag::List(list)) => list.iter().filter_map(dir_of).collect(),
            _ => Vec::new(),
        },
    }
}

fn write_spout(nbt: &mut Compound, spout: &Spout) {
    let upper = |d: Dir| Tag::String(d.name().to_ascii_uppercase().as_str().into());
    match spout.preferred {
        Some(dir) => nbt.insert(PREFERRED.into(), upper(dir)),
        None => util::remove(nbt, PREFERRED),
    };
    nbt.insert(DISABLED.into(), Tag::List(spout.disabled.iter().map(|d| upper(*d)).collect()));
}

/// `BasinBlockEntity.updateSpoutput`: picks the side and, when it is a side, sends what the output
/// slots hold out through the spout. Answers whether anything changed.
fn update_spout(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity) -> bool {
    let current = util::facing(host, pos, "facing").unwrap_or(Dir::Down);
    let spout = read_spout(&entity.nbt);
    let facing = spout.facing(|d| can_output_to(host, pos, d));
    if facing == current {
        return false;
    }
    host.set_block_property(pos, "facing", facing.name());
    if facing == Dir::Down {
        return true;
    }
    let mut contents = Contents::from_entity(entity);
    for slot in INPUT_SLOTS..SLOTS {
        if let Some(stack) = contents.items[slot].take() {
            contents.overflow.push(stack);
        }
    }
    let target = add(add(pos, [0, -1, 0]), facing.normal());
    let dimension = host.dimension();
    let has_tank = host.fluid_tanks_mut(dimension, target, util::direction(facing.opposite())).is_some();
    if has_tank {
        for tank in &mut contents.tanks[2..TANKS] {
            if let (Some(fluid), true) = (tank.fluid.take(), tank.amount > 0) {
                contents.fluid_overflow.push((fluid, tank.amount));
            }
            tank.amount = 0;
        }
    }
    contents.write_to(entity);
    true
}

/// A stack put on a Depot (`DepotBehaviour.handleBeltInput`'s result): into its one slot if that
/// is empty; the Depot's own tick settles it in the centre.
fn put_on_depot(host: &mut dyn Host, depot: Pos, stack: &ItemStack) -> bool {
    let free = host
        .mod_block_entity(depot)
        .is_some_and(|e| e.items.first().is_none_or(Option::is_none));
    if !free {
        return false;
    }
    let mut put = Some(stack.clone());
    host.change_mod_block_entity(depot, &mut |entity| {
        if entity.items.is_empty() {
            entity.items.push(None);
        }
        entity.items[0] = put.take();
    });
    host.block_entity_sync(depot);
    true
}

/// `tryClearingSpoutputOverflow` for the items: into what the spout faces. Answers the stack that
/// is left (the whole of it when nothing took any).
fn deliver_item(host: &mut dyn Host, basin: Pos, facing: Dir, stack: ItemStack) -> ItemStack {
    let target = add(add(basin, [0, -1, 0]), facing.normal());
    match host.block(target).as_str() {
        "create:depot" => {
            if put_on_depot(host, target, &stack) {
                let mut rest = stack;
                rest.count = 0;
                return rest;
            }
            stack
        }
        "create:belt" => {
            // The belt carries an item that lands on it; the neighbour's block is above it.
            let [x, y, z] = target;
            let at = [f64::from(x) + 0.5, f64::from(y) + 1.0, f64::from(z) + 0.5];
            util::spawn_item(host, at, &stack, [0.0; 3], 10);
            let mut rest = stack;
            rest.count = 0;
            rest
        }
        _ => match host.insert_item(target, util::direction(facing.opposite()), stack.clone()) {
            Some(rest) => rest,
            None => {
                let mut rest = stack;
                rest.count = 0;
                rest
            }
        },
    }
}

/// `tryClearingSpoutputOverflow`: hands the buffers on, every tick, until they are empty.
fn clear_overflow(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity) -> bool {
    let overflow = util::stacks_of(&entity.nbt, OVERFLOW);
    let fluid_overflow = fluid_stacks_of(&entity.nbt);
    if overflow.is_empty() && fluid_overflow.is_empty() {
        return false;
    }
    let facing = util::facing(host, pos, "facing").unwrap_or(Dir::Down);
    let mut update = false;
    let mut kept_items = Vec::new();
    for stack in overflow {
        if facing == Dir::Down {
            // `Block.popResource`.
            let [x, y, z] = pos;
            let at = [f64::from(x) + 0.5, f64::from(y) + 0.5, f64::from(z) + 0.5];
            util::spawn_item(host, at, &stack, [0.0; 3], 10);
            update = true;
            continue;
        }
        let before = stack.count;
        let rest = deliver_item(host, pos, facing, stack);
        if rest.count != before {
            update = true;
        }
        if rest.count > 0 {
            kept_items.push(rest);
        }
    }
    let mut kept_fluids = Vec::new();
    let target = add(add(pos, [0, -1, 0]), facing.normal());
    let face = util::direction(facing.opposite());
    for (fluid, amount) in fluid_overflow {
        if facing == Dir::Down {
            update = true;
            continue;
        }
        // Only a whole fluid stack goes (`fill != amount` breaks out).
        let dimension = host.dimension();
        let room: i32 = host
            .fluid_tanks_mut(dimension, target, face)
            .map(|tanks| {
                tanks
                    .iter()
                    .filter(|t| t.can_fill && t.fluid.as_deref().is_none_or(|f| f == fluid))
                    .map(|t| (t.capacity - t.amount).max(0))
                    .sum()
            })
            .unwrap_or(0);
        if room >= amount && host.fluid_insert(target, face, &fluid, amount) == 0 {
            update = true;
            continue;
        }
        kept_fluids.push((fluid, amount));
    }
    if update {
        let items = util::stacks_tag(&kept_items);
        entity.nbt.insert(OVERFLOW.into(), items);
        entity.nbt.insert(FLUID_OVERFLOW.into(), fluid_stacks_tag(&kept_fluids));
    }
    update
}

// --- items from above ---------------------------------------------------------------------------------

/// `ItemHandlerHelper.insertItem` into the input slots with `BasinInventory.insertItem`'s policy.
fn insert_input(host: &dyn Host, items: &mut [Option<ItemStack>], mut stack: ItemStack) -> ItemStack {
    for slot in 0..INPUT_SLOTS {
        if stack.count == 0 {
            break;
        }
        let empty: Vec<bool> = items[..INPUT_SLOTS].iter().map(Option::is_none).collect();
        let same_elsewhere = items[..INPUT_SLOTS]
            .iter()
            .enumerate()
            .any(|(i, s)| i != slot && s.as_ref().is_some_and(|s| util::same_item(s, &stack)));
        if !rules::may_insert_input(slot, &empty, same_elsewhere) {
            continue;
        }
        let limit = SLOT_LIMIT.min(max_stack(host, &stack));
        match &mut items[slot] {
            Some(have) => {
                if !util::same_item(have, &stack) {
                    continue;
                }
                let moved = limit.saturating_sub(have.count).min(stack.count);
                have.count += moved;
                stack.count -= moved;
            }
            empty_slot => {
                let moved = limit.min(stack.count);
                let mut put = stack.clone();
                put.count = moved;
                stack.count -= moved;
                *empty_slot = Some(put);
            }
        }
    }
    stack
}

/// `BasinBlock.updateEntityAfterFallOn` as a scan: an item lying in the basin goes into the
/// input slots, what does not fit stays. Answers whether the slots changed.
fn pick_up(host: &mut dyn Host, pos: Pos, items: &mut [Option<ItemStack>]) -> bool {
    let [x, y, z] = pos.map(f64::from);
    let dimension = host.dimension();
    let mut changed = false;
    for item in host.item_entities(dimension, [x, y, z], [x + 1.0, y + 1.0, z + 1.0]) {
        let [px, py, pz] = item.position;
        if !(x..x + 1.0).contains(&px) || !(z..z + 1.0).contains(&pz) || !(y..y + 1.0).contains(&py) {
            continue;
        }
        let on_ground = host
            .entity(item.id)
            .and_then(|entity| host.entity_on_ground(entity))
            .unwrap_or(false);
        if !on_ground {
            continue;
        }
        let rest = insert_input(host, items, item.stack.clone());
        if rest.count != item.stack.count {
            host.set_item_entity(dimension, item.id, rest);
            changed = true;
        }
    }
    changed
}

// --- fluid items -------------------------------------------------------------------------------------

/// `GenericItemEmptying` / `Filling` for the three buckets: full bucket, fluid.
const BUCKETS: [(&str, &str); 5] = [
    ("minecraft:water_bucket", "minecraft:water"),
    ("minecraft:lava_bucket", "minecraft:lava"),
    ("minecraft:milk_bucket", "neoforge:milk"),
    ("create:honey_bucket", "create:honey"),
    ("create:chocolate_bucket", "create:chocolate"),
];
const BUCKET: i32 = 1000;

fn room_for(host: &mut dyn Host, pos: Pos, fluid: &str) -> i32 {
    let dimension = host.dimension();
    host.fluid_tanks_mut(dimension, pos, Direction::Up)
        .map(|tanks| {
            tanks
                .iter()
                .filter(|t| t.can_fill && t.fluid.as_deref().is_none_or(|f| f == fluid))
                .map(|t| (t.capacity - t.amount).max(0))
                .sum()
        })
        .unwrap_or(0)
}

/// `FluidHelper.tryEmptyItemIntoBE` / `tryFillItemFromBE` for buckets; true when it did something.
fn use_bucket(host: &mut dyn Host, pos: Pos, stack: &ItemStack, creative: bool) -> bool {
    let item = &*stack.item;
    if let Some((_, fluid)) = BUCKETS.iter().find(|(bucket, _)| *bucket == item) {
        if room_for(host, pos, fluid) < BUCKET || host.fluid_insert(pos, Direction::Up, fluid, BUCKET) != 0 {
            return false;
        }
        if !creative && stack.extra.is_none() && host.take_items(item, 1) == 1 {
            host.give_items("minecraft:bucket", 1);
        }
        host.play_sound(
            if *fluid == "minecraft:lava" {
                "minecraft:item.bucket.empty_lava"
            } else {
                "minecraft:item.bucket.empty"
            },
            util::centre(pos),
            4,
            1.0,
            1.0,
        );
        return true;
    }
    if item == "minecraft:bucket" {
        let dimension = host.dimension();
        let Some(tanks) = host.fluid_tanks_mut(dimension, pos, Direction::Up) else {
            return false;
        };
        let Some(first) = tanks.iter().find(|t| t.can_drain && t.fluid.is_some() && t.amount > 0) else {
            return false;
        };
        let fluid = first.fluid.clone().unwrap_or_default();
        let total: i32 = tanks
            .iter()
            .filter(|t| t.can_drain && t.fluid.as_deref() == Some(fluid.as_str()))
            .map(|t| t.amount)
            .sum();
        let Some((full, _)) = BUCKETS.iter().find(|(_, f)| *f == fluid) else {
            return false;
        };
        if total < BUCKET {
            return false;
        }
        match host.fluid_extract(dimension, pos, Direction::Up, BUCKET) {
            Some((_, amount)) if amount == BUCKET => {}
            Some((drained, amount)) => {
                host.fluid_insert(pos, Direction::Up, &drained, amount);
                return false;
            }
            None => return false,
        }
        if !creative && stack.extra.is_none() && host.take_items(item, 1) == 1 {
            host.give_items(full, 1);
        }
        host.play_sound("minecraft:item.bucket.fill", util::centre(pos), 4, 1.0, 1.0);
        return true;
    }
    false
}

/// A sponge drains the whole basin (`fluidHandler.drain(Integer.MAX_VALUE)`).
fn drain_all(host: &mut dyn Host, pos: Pos) -> bool {
    let mut drained = false;
    host.change_mod_block_entity(pos, &mut |entity| {
        let mut tanks = read_tanks(&entity.nbt);
        if tanks.iter().all(|t| t.amount <= 0) {
            return;
        }
        for tank in &mut tanks {
            *tank = Tank::default();
        }
        entity.nbt.insert(FLUIDS.into(), tanks_tag(&tanks));
        drained = true;
    });
    drained
}

// --- the hooks -------------------------------------------------------------------------------------------

fn container() -> ModContainer {
    let all: Vec<usize> = (0..SLOTS).collect();
    ModContainer {
        slots: SLOTS,
        faces: [(); 6].map(|()| all.clone()),
        // `CombinedInvWrapper(inputInventory, outputInventory)`: put into the inputs, take from any.
        inputs: (0..INPUT_SLOTS).collect(),
        outputs: all,
        menu: None,
    }
}

fn tanks() -> ModFluidTanks {
    let all: Vec<usize> = (0..TANKS).collect();
    ModFluidTanks {
        tanks: vec![TANK_CAPACITY; TANKS],
        faces: [(); 6].map(|()| all.clone()),
        // `outputTank.forbidInsertion()`: fills reach the two input tanks, drains any.
        inputs: (0..2).collect(),
        outputs: all,
    }
}

fn new_entity() -> Compound {
    let mut nbt = Compound::new();
    nbt.insert(FLUIDS.into(), tanks_tag(&vec![Tank::default(); TANKS]));
    nbt
}

fn drawn_items(entity: &ModBlockEntity) -> Vec<ItemStack> {
    entity.items.iter().flatten().cloned().collect()
}

/// The fluid colours for what Horizon can't draw itself: ARGB-free RGB tints of a white texture.
fn fluid_tint(fluid: &str) -> u32 {
    match fluid {
        "neoforge:milk" => 0xF4F4F0,
        "create:honey" => 0xE3A227,
        "create:chocolate" => 0x4B2A17,
        "create:tea" => 0x8C5B2E,
        _ => 0x7F7FFF,
    }
}

/// `BasinRenderer.renderFluids`: the fluids as boxes side by side along x, level with the total.
/// Answers the height of the top (`yMax`), 0 without fluid.
fn draw_fluids(host: &dyn Host, draw: &mut crate::compat::BlockEntityDraw, tanks: &[Tank]) -> f32 {
    let total: i32 = tanks
        .iter()
        .filter(|t| t.fluid.is_some() && t.amount >= 1)
        .map(|t| t.amount)
        .sum();
    if total < 1 {
        return 0.0;
    }
    let fill = rules::fluid_fill(total as f32);
    let (y_min, z_min, z_max) = (2.0 / 16.0, 2.0 / 16.0, 14.0 / 16.0);
    let y_max = y_min + 12.0 / 16.0 * fill;
    let mut x_min = 2.0 / 16.0;
    let mut x_max = 2.0 / 16.0;
    for tank in tanks {
        let Some(fluid) = &tank.fluid else { continue };
        if tank.amount < 1 {
            continue;
        }
        x_max += (tank.amount as f32 / total as f32).clamp(0.0, 1.0) * 12.0 / 16.0;
        let bounds = [x_min, y_min, z_min, x_max, y_max, z_max];
        let stack = NativeFluidStack {
            fluid: fluid.clone(),
            amount: tank.amount,
            components: Compound::new(),
        };
        let options = NativeFluidRenderOptions {
            argb: None,
            render_bottom: false,
            uv_shrink_ratio: 4.0 / 1024.0,
        };
        if !host.draw_fluid_box(draw, &stack, bounds, options) {
            // A fluid Horizon has no still sprite for: a tinted white box.
            let face = Some(CubeFace {
                sprite: "block/white_concrete",
                uv: None,
            });
            let [x0, y0, z0, x1, y1, z1] = bounds.map(|v| v * 16.0);
            host.draw_cube(draw, [x0, y0, z0], [x1, y1, z1], [face; 6], fluid_tint(fluid));
        }
        x_min = x_max;
    }
    y_max
}

/// `BasinRenderer.renderSafe`'s `visualizedOutputItems`: each stack falls out of the spout for ten ticks. Whether
/// anything was drawn (the geometry then reads the time and must not be kept).
fn draw_output(
    host: &dyn Host,
    draw: &mut crate::compat::BlockEntityDraw,
    state: crate::compat::BlockStateId,
    nbt: &Compound,
    pos: [i32; 3],
) -> bool {
    let stacks = util::stacks_of(nbt, VISUALIZED);
    let Some(facing) = host.block_state_property(state, "facing").as_deref().and_then(Dir::from_name) else {
        return false;
    };
    if stacks.is_empty() {
        return false;
    }
    // A scene counts its own ticks (`Since` is one of them).
    let scene = crate::float(nbt, crate::ponder::TICK);
    let now = scene.unwrap_or(draw.game_time as f32 + draw.partial);
    let Some(progress) = rules_output::progress(now - util::long(nbt, VISUALIZED_SINCE) as f32) else {
        return false;
    };
    let facing_pos = util::add(pos, facing.normal());
    let to_basin = scene.is_none() && host.block(facing_pos) == ID;
    let Some(flight) = rules_output::flight(facing, progress, to_basin) else {
        // Still drawing nothing now, but the progress will change.
        return true;
    };
    for stack in &stacks {
        draw.push();
        draw.translate(flight.mouth);
        draw.translate([0.0, flight.fall, 0.0]);
        draw.translate(flight.out);
        draw.rotate_about([0.0, 1.0, 0.0], flight.yaw, [0.0; 3]);
        draw.rotate_about([1.0, 0.0, 0.0], flight.roll, [0.0; 3]);
        host.draw_item(draw, stack, DisplayContext::Ground, None, 0);
        draw.pop();
    }
    true
}

/// `BasinRenderer.renderSafe`'s ingredients: in a ring a little above the floor, each stack tilted
/// and a copy more for every eight items.
fn draw_items(
    host: &dyn Host,
    draw: &mut crate::compat::BlockEntityDraw,
    items: &[ItemStack],
    fluid_top: f32,
    seed: u32,
    moving: bool,
) {
    if items.is_empty() {
        return;
    }
    let level = (fluid_top - 0.3).clamp(0.125, 0.6);
    let mut random = seed.wrapping_mul(2_654_435_761) | 1;
    let mut next = || {
        // xorshift: a stable scatter for one basin, standing in for `RandomSource.create(pos.hashCode())`.
        random ^= random << 13;
        random ^= random >> 17;
        random ^= random << 5;
        random as f32 / u32::MAX as f32
    };
    draw.push();
    draw.translate([0.5, 0.2, 0.5]);
    if moving {
        // The ring whirls (`ingredientRotation`, up to 20 degrees a tick).
        draw.spin([0.0, 1.0, 0.0], [0.5, 0.2, 0.5], 20.0, 0.0);
    }
    let count = items.len();
    let partition = 360.0 / count as f32;
    for (index, stack) in items.iter().enumerate() {
        // Create counts `itemCount` down from the number of stacks.
        let n = (count - index) as f32;
        let angle = partition * n;
        let base = if count == 1 { [0.0, level, 0.0] } else { [0.125, level, 0.0] };
        let (sin, cos) = angle.to_radians().sin_cos();
        let at = [base[0] * cos + base[2] * sin, base[1], -base[0] * sin + base[2] * cos];
        draw.push();
        draw.translate(at);
        draw.rotate_about([0.0, 1.0, 0.0], angle + 35.0, [0.0; 3]);
        draw.rotate_about([1.0, 0.0, 0.0], 65.0, [0.0; 3]);
        for _ in 0..=(stack.count / 8) {
            draw.push();
            let jitter = [(); 3].map(|()| (next() - 0.5) * 2.0 / 16.0);
            draw.translate(jitter);
            host.draw_item(draw, stack, DisplayContext::Ground, None, 0);
            draw.pop();
        }
        draw.pop();
    }
    if moving {
        draw.still();
    }
    draw.pop();
}

impl Hooks for Basin {
    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == ID).then(|| (ID.to_owned(), new_entity()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    fn block_entity_container(
        &self,
        _: &dyn Host,
        event: &mut BlockEntityContainerEvent<'_>,
    ) -> Option<ModContainer> {
        (event.block == ID).then(container)
    }

    fn block_entity_fluids(
        &self,
        _: &dyn Host,
        event: &mut BlockEntityFluidsEvent<'_>,
    ) -> Option<ModFluidTanks> {
        (event.block == ID).then(tanks)
    }

    /// `BasinInventory.insertItem` for a hopper: one stack per item, a new one only in the first
    /// free input slot, and nothing into the outputs.
    fn container_accepts(&self, _: &dyn Host, event: &mut ContainerAcceptsEvent<'_>) -> Flow {
        if event.entity.id != ID {
            return Flow::Continue;
        }
        let mut items = event.entity.items.clone();
        items.resize(SLOTS, None);
        let empty: Vec<bool> = items[..INPUT_SLOTS].iter().map(Option::is_none).collect();
        let same_elsewhere = items[..INPUT_SLOTS]
            .iter()
            .enumerate()
            .any(|(i, s)| i != event.slot && s.as_ref().is_some_and(|s| util::same_item(s, event.stack)));
        if rules::may_insert_input(event.slot, &empty, same_elsewhere) {
            Flow::Continue
        } else {
            Flow::Cancel
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        let pos = event.pos;
        let now = util::game_time(host);
        let mut sync = false;

        // Items lying in it (`updateEntityAfterFallOn`).
        if now.rem_euclid(2) == 0 {
            let mut items = event.entity.items.clone();
            items.resize(SLOTS, None);
            if pick_up(host, pos, &mut items) {
                event.entity.items = items;
            }
        }
        // The spout (`lazyTick`: every 10 ticks in Create; looked at more often here).
        if now.rem_euclid(5) == 0 && update_spout(host, pos, event.entity) {
            sync = true;
        }
        // What waits to leave by the spout.
        if clear_overflow(host, pos, event.entity) {
            sync = true;
        }
        // Whether the mixer above whirls the contents (`areFluidsMoving`).
        if now.rem_euclid(5) == 0 {
            let moving = host
                .mod_block_entity(add(pos, [0, 2, 0]))
                .filter(|e| e.id == "create:mechanical_mixer")
                .is_some_and(|mixer| {
                    util::boolean(&mixer.nbt, "Running") && util::int(&mixer.nbt, "Ticks") <= 20
                });
            if util::boolean(&event.entity.nbt, MOVING) != moving {
                event.entity.nbt.insert(MOVING.into(), Tag::Byte(i8::from(moving)));
                sync = true;
            }
        }
        // Clients are told whenever what they draw changed, whoever changed it (a hopper, say).
        let drawn = {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            signature_of(&Contents::from_entity(event.entity), HeatLevel::None).hash(&mut hasher);
            util::boolean(&event.entity.nbt, MOVING).hash(&mut hasher);
            hasher.finish()
        };
        let mut sent = SENT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let map = sent.get_or_insert_with(HashMap::new);
        if map.get(&pos) != Some(&drawn) {
            map.insert(pos, drawn);
            sync = true;
        }
        drop(sent);
        if sync {
            host.block_entity_sync(pos);
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        if event.entity.id != ID {
            return None;
        }
        let entity = event.entity;
        let mut tag = Compound::new();
        tag.insert(FLUIDS.into(), tanks_tag(&read_tanks(&entity.nbt)));
        tag.insert(MOVING.into(), Tag::Byte(i8::from(util::boolean(&entity.nbt, MOVING))));
        tag.insert(DRAWN.into(), util::stacks_tag(&drawn_items(entity)));
        Some(tag)
    }

    /// The bowl is in the chunk mesh; this draws what is in it.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let tanks = read_tanks(&event.entity.nbt);
        let items = util::stacks_of(&event.entity.nbt, DRAWN);
        let moving = util::boolean(&event.entity.nbt, MOVING);
        let [x, y, z] = event.pos;
        let seed = (x.wrapping_mul(31).wrapping_add(y).wrapping_mul(31).wrapping_add(z)) as u32;
        let draw = &mut *event.draw;
        let top = draw_fluids(host, draw, &tanks);
        draw_items(host, draw, &items, top, seed, moving);
        let flying = draw_output(host, draw, event.state, &event.entity.nbt, event.pos);
        // Nothing here reads the partial tick or the game time but the stacks flying out: the whirl turns through `spin`.
        if !flying {
            draw.keep();
        }
    }

    /// `BasinBlock.useItemOn`: a bucket fills or empties it, a sponge drains it, and an empty hand
    /// takes everything out.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        let uuid = player.uuid;
        let main = host.player_equipment(uuid, EquipmentSlot::MainHand);
        // Sneaking with anything in a hand skips the block's own use.
        if player.sneaking
            && (main.is_some() || host.player_equipment(uuid, EquipmentSlot::OffHand).is_some())
        {
            return Flow::Continue;
        }
        // The off hand's turn of a click the basin took (a filled bucket would be emptied again).
        if util::click_repeated(host, uuid, pos) {
            return Flow::Cancel;
        }
        if let Some(stack) = main {
            let creative = player.game_mode == util::CREATIVE;
            if use_bucket(host, pos, &stack, creative) {
                util::click_taken(host, uuid, pos);
                host.block_entity_sync(pos);
                return Flow::Cancel;
            }
            if &*stack.item == "minecraft:sponge" && drain_all(host, pos) {
                util::click_taken(host, uuid, pos);
                host.block_entity_sync(pos);
                return Flow::Cancel;
            }
            // `PASS_TO_DEFAULT_BLOCK_INTERACTION`.
            return Flow::Continue;
        }
        util::click_taken(host, uuid, pos);
        let mut taken: Vec<ItemStack> = Vec::new();
        host.change_mod_block_entity(pos, &mut |entity| {
            let mut slots = entity.items.clone();
            slots.resize(SLOTS, None);
            taken = slots.iter_mut().filter_map(Option::take).collect();
            if !taken.is_empty() {
                entity.items = slots;
            }
        });
        let success = !taken.is_empty();
        for stack in taken {
            // `Inventory.placeItemBackInInventory`: what does not fit drops at the player.
            if let Some(rest) = host.give_stack(uuid, stack) {
                util::spawn_item(host, player.position, &rest, [0.0; 3], 0);
            }
        }
        if success {
            host.play_sound("minecraft:entity.item.pickup", util::centre(pos), 7, 0.2, 1.0 + (util::random() % 1000) as f32 / 1000.0);
        }
        notify_removed(host, pos);
        host.block_entity_sync(pos);
        Flow::Cancel
    }

    /// The Wrench: sneak-click picks the basin up; a click sets the spout side
    /// (`BasinBlock.onWrenched`, `BasinBlockEntity.onWrenched`).
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != ID {
            return Flow::Continue;
        }
        let pos = event.pos;
        let Some(face) = Dir::from_normal(event.face) else {
            return Flow::Cancel;
        };
        let facing = util::facing(host, pos, "facing").unwrap_or(Dir::Down);
        util::wrench(host, pos, |host| {
            host.change_mod_block_entity(pos, &mut |entity| {
                let mut spout = read_spout(&entity.nbt);
                spout.wrenched(facing, face);
                write_spout(&mut entity.nbt, &spout);
            });
        })
    }

    /// `BasinBlockEntity.destroy`: what waits in the spout's buffer drops with the contents.
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        if let Some(contents) = contents(host, pos) {
            for stack in &contents.overflow {
                let [x, y, z] = pos;
                let at = [f64::from(x) + 0.5, f64::from(y) + 0.5, f64::from(z) + 0.5];
                util::spawn_item(host, at, stack, [0.0; 3], 10);
            }
        }
        notify_removed(host, pos);
        Flow::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tanks_round_trip_through_their_list() {
        let tanks = vec![
            Tank { fluid: Some("minecraft:water".into()), amount: 250 },
            Tank::default(),
            Tank { fluid: Some("create:honey".into()), amount: 1000 },
            Tank::default(),
        ];
        let mut nbt = Compound::new();
        nbt.insert(FLUIDS.into(), tanks_tag(&tanks));
        assert_eq!(read_tanks(&nbt), tanks);
    }

    #[test]
    fn forced_fills_use_the_same_fluid_first_and_refuse_overflow() {
        let mut tanks = vec![Tank::default(); TANKS];
        assert!(force_fill(&mut tanks, 2..TANKS, "minecraft:water", 600));
        assert!(force_fill(&mut tanks, 2..TANKS, "minecraft:water", 600));
        assert_eq!((tanks[2].amount, tanks[3].amount), (1000, 200));
        assert!(!force_fill(&mut tanks, 2..TANKS, "minecraft:lava", 900));
        assert!(force_fill(&mut tanks, 2..TANKS, "minecraft:water", 800));
        assert_eq!(tanks[3].amount, 1000);
    }

    #[test]
    fn fluid_tags_hold_create_s_fluids() {
        assert!(tag_holds("c:milk", "neoforge:milk"));
        assert!(!tag_holds("c:milk", "minecraft:water"));
        assert!(FluidIngredient::Tag("c:honey".into(), 1000).test("create:honey"));
        assert!(FluidIngredient::Single("minecraft:water".into(), 250).test("minecraft:water"));
    }

    #[test]
    fn spout_state_round_trips() {
        let spout = Spout {
            preferred: Some(Dir::East),
            disabled: vec![Dir::North, Dir::West],
        };
        let mut nbt = Compound::new();
        write_spout(&mut nbt, &spout);
        assert_eq!(read_spout(&nbt), spout);
        write_spout(&mut nbt, &Spout::default());
        assert_eq!(read_spout(&nbt), Spout::default());
    }

    #[test]
    fn an_empty_basin_is_empty() {
        let c = Contents {
            items: vec![None; SLOTS],
            tanks: vec![Tank::default(); TANKS],
            ..Contents::default()
        };
        assert!(c.is_empty());
        assert!(c.can_continue());
    }
}
