//! What the Deployer (and a player) applies to items and blocks: Create's `create:deploying` and
//! `create:item_application` recipes (`DeployerApplicationRecipe`, `ItemApplicationRecipe`,
//! `DeployerBlockEntity.getRecipe`, `RecipeApplier`) and the manual application of the latter to
//! a block with a right-click (`ManualApplicationRecipe`: a stripped log and an Andesite Alloy
//! make an Andesite Casing). The Deployer's own stroke is `deployer.rs`; what it does in the
//! world is `deployer_use.rs`.
//!
//! `getRecipe` asks, in Create's order: the sandpaper polishing recipe when the hand holds a
//! `SandPaperItem` (`create:sandpaper_polishing`), the sequenced assembly's next deploying step
//! (`SequencedAssemblyRecipe.getRecipe`, priority 100, `sequenced.rs`), then the `create:deploying`
//! and `create:item_application` recipes (priority 50).
//!
//! Known differences from Create:
//! - A `DeployerRecipeSearchEvent` (another mod adding a recipe) has no equivalent.
//! - A recipe lookup is remembered by the two item ids (`stack`, `held`), so a recipe whose
//!   ingredients ask for components isn't told apart; none of Create's do. It is dropped when the
//!   number of recipes changes.
//! - The remainder of a used-up held item (`getCraftingRemainingItem`) is not returned, as in
//!   the saw (Horizon's `Host` has no way to ask an item for it), and a held item with the
//!   `unbreakable` component still wears.
//! - A player's manual application works for stackable plain items only (`take_items`): inside a
//!   player's own hook the held stack can't be worn or replaced, so a damageable or component-
//!   carrying item is not applied by hand (none of Create's `item_application` recipes holds one).
//! - Advancements are not given (`AllAdvancements.ANDESITE_CASING` and the others).

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use create_kinetics::rules::deployer as rules;
use create_kinetics::rules::drill;
use create_kinetics::rules::sandpaper;
use create_kinetics::rules::sequenced::Machine;
use crate::compat::serde_json::Value;
use crate::compat::{Flow, Hooks, Host, Ingredient, ItemStack, RightClickBlockEvent};

use super::sand_paper::POLISHING;
use super::sequenced;
use crate::recipes::{self, Processing};
use crate::util;

pub const DEPLOYING: &str = "create:deploying";
pub const ITEM_APPLICATION: &str = "create:item_application";
/// `SoundSource.PLAYERS`.
const PLAYERS: usize = 7;

struct Apply;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Apply)
}

type Found = Option<(&'static str, String)>;

/// What was looked up, by the item ids it was asked for, and how many recipes there were then.
struct Cache {
    recipes: usize,
    deploying: HashMap<(String, String), Found>,
    manual: HashMap<(String, String), Option<String>>,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

fn cache(host: &dyn Host) -> MutexGuard<'static, Option<Cache>> {
    let mut guard = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    let count = host.recipes_of_type(DEPLOYING).len()
        + host.recipes_of_type(ITEM_APPLICATION).len()
        + host.recipes_of_type(POLISHING).len();
    if guard.as_ref().is_none_or(|c| c.recipes != count) {
        *guard = Some(Cache { recipes: count, deploying: HashMap::new(), manual: HashMap::new() });
    }
    guard
}

/// The two item ingredients of a recipe of either type (`ingredients.get(0)` is what is worked on,
/// `get(1)` what is held), as Create's datagen writes them.
fn ingredients(host: &dyn Host, json: &Value) -> Option<(Ingredient, Ingredient)> {
    let list = json.get("ingredients")?.as_array()?;
    let first = host.recipe_ingredient(&recipes::modern_ingredient(list.first()?))?;
    let second = host.recipe_ingredient(&recipes::modern_ingredient(list.get(1)?))?;
    Some((first, second))
}

/// `DeployerBlockEntity.getRecipe` for the item `stack` on a belt or depot and the held `held`:
/// the first `create:deploying` recipe that matches, else the first `create:item_application`
/// one (both are priority 50 and a later one is only taken if its priority is higher), leaving
/// out those whose id ends in `_manual_only` (`AllRecipeTypes.CAN_BE_AUTOMATED`).
fn search(host: &dyn Host, stack: &ItemStack, held: &ItemStack) -> Found {
    // `heldItemMainhand.getItem() instanceof SandPaperItem`: the polishing recipes come first.
    let sand_paper = sandpaper::is_sand_paper(&held.item);
    let kinds: &[&'static str] = if sand_paper {
        &[POLISHING, DEPLOYING, ITEM_APPLICATION]
    } else {
        &[DEPLOYING, ITEM_APPLICATION]
    };
    for &kind in kinds {
        for recipe in host.recipes_of_type(kind) {
            if recipe.id.ends_with("_manual_only") {
                continue;
            }
            if kind == POLISHING {
                // `SandPaperPolishingRecipe.matches`: its one ingredient.
                let first = recipe.json.get("ingredients").and_then(|i| i.get(0));
                let takes = first
                    .and_then(|i| host.recipe_ingredient(&recipes::modern_ingredient(i)))
                    .is_some_and(|i| i.test(stack));
                if takes {
                    return Some((kind, recipe.id.clone()));
                }
                continue;
            }
            let Some((processed, tool)) = ingredients(host, &recipe.json) else {
                continue;
            };
            if processed.test(stack) && tool.test(held) {
                return Some((kind, recipe.id.clone()));
            }
        }
    }
    None
}

/// `SequencedAssemblyRecipe.getRecipe(.., DEPLOYING, DeployerApplicationRecipe.class)`: the
/// assembly's next step if it is a deploying one that the held item fits (`matches`: the second
/// ingredient). The sandpaper's polishing outranks it.
fn sequenced_step(host: &dyn Host, stack: &ItemStack, held: &ItemStack) -> Option<sequenced::Step> {
    sequenced::find(host, stack, Machine::Deploying, &|step| {
        step.recipe.ingredients.get(1).is_some_and(|tool| tool.test(held))
    })
}

fn lookup(host: &dyn Host, stack: &ItemStack, held: &ItemStack) -> Found {
    let key = (stack.item.to_string(), held.item.to_string());
    if let Some(found) = cache(host).as_ref().and_then(|c| c.deploying.get(&key)) {
        return found.clone();
    }
    let found = search(host, stack, held);
    if let Some(c) = cache(host).as_mut() {
        c.deploying.insert(key, found.clone());
    }
    found
}

/// Whether the Deployer holding `held` has a recipe for `stack` (`getRecipe(stack) != null`).
pub(super) fn has_recipe(host: &dyn Host, stack: &ItemStack, held: &ItemStack) -> bool {
    sequenced_step(host, stack, held).is_some() || lookup(host, stack, held).is_some()
}

/// What applying a recipe to one item made.
pub(super) struct Applied {
    pub results: Vec<ItemStack>,
    /// `ItemApplicationRecipe.shouldKeepHeldItem` (`keep_held_item`).
    pub keep_held: bool,
    /// The recipe was a `SandPaperPolishingRecipe`: the Deployer sands (`BeltDeployerCallbacks`).
    pub polishing: bool,
}

/// `RecipeApplier.applyRecipeOn(level, stack.copyWithCount(1), recipe, true)` for the recipe
/// `getRecipe` finds: one roll of its results, equal stacks merged. `None` where there is none.
pub(super) fn apply(host: &mut dyn Host, stack: &ItemStack, held: &ItemStack) -> Option<Applied> {
    let found = lookup(&*host, stack, held);
    let polishing = found.as_ref().is_some_and(|(kind, _)| *kind == POLISHING);
    if !polishing && let Some(step) = sequenced_step(&*host, stack, held) {
        // `RecipeApplier.applyRecipeOn`: one roll of the step, equal stacks merged.
        let mut results = Vec::new();
        for out in step.roll(host) {
            util::merge(&*host, &mut results, out);
        }
        return Some(Applied { results, keep_held: step.keep_held, polishing: false });
    }
    let (kind, id) = found?;
    let (processing, keep_held) = resolve(&*host, kind, &id)?;
    let dimension = host.dimension();
    let rolled = recipes::roll(host, dimension, &processing.results);
    let mut results = Vec::new();
    for out in rolled {
        util::merge(&*host, &mut results, out);
    }
    Some(Applied { results, keep_held, polishing: kind == POLISHING })
}

fn resolve(host: &dyn Host, kind: &str, id: &str) -> Option<(Processing, bool)> {
    let recipe = host.recipes_of_type(kind).iter().find(|r| r.id == id)?;
    let keep = recipe.json.get("keep_held_item").and_then(Value::as_bool).unwrap_or(false);
    Some((recipes::parse(host, &recipe.id, &recipe.json)?, keep))
}

// --- by hand: `ManualApplicationRecipe` ----------------------------------------------------------

/// A `create:item_application` recipe found for a block and a held item.
pub(super) struct Manual {
    /// The block the recipe makes in its place, if its main result is a block item.
    pub block: Option<String>,
    /// What pops out beside it (`getRollableResultsExceptBlock`, rolled).
    pub pops: Vec<ItemStack>,
    pub keep_held: bool,
}

/// `ManualApplicationRecipe.testBlock` and the held ingredient: the first recipe whose processed
/// item fits `block`'s item and whose second ingredient fits `held`. Rolled once found.
pub(super) fn manual_for(host: &mut dyn Host, block: &str, held: &ItemStack) -> Option<Manual> {
    if drill::is_air(block) {
        return None;
    }
    let key = (block.to_owned(), held.item.to_string());
    let remembered = cache(&*host).as_ref().and_then(|c| c.manual.get(&key).cloned());
    let id = match remembered {
        Some(id) => id,
        None => {
            let as_item = ItemStack { item: block.into(), count: 1, damage: 0, extra: None };
            let found = host.recipes_of_type(ITEM_APPLICATION).iter().find_map(|recipe| {
                let (processed, tool) = ingredients(&*host, &recipe.json)?;
                (processed.test(&as_item) && tool.test(held)).then(|| recipe.id.clone())
            });
            if let Some(c) = cache(&*host).as_mut() {
                c.manual.insert(key, found.clone());
            }
            found
        }
    }?;
    let (processing, keep_held) = resolve(&*host, ITEM_APPLICATION, &id)?;
    let dimension = host.dimension();
    // `transformBlock`: the main result is rolled, and only a block item makes a block.
    let main = processing.results.first()?;
    let rolled = recipes::roll(host, dimension, std::slice::from_ref(main));
    let main_is_block = host.default_state(&main.stack.item).is_some();
    let block = rolled
        .first()
        .filter(|stack| host.default_state(&stack.item).is_some())
        .map(|stack| stack.item.to_string());
    // `rollResults(getRollableResultsExceptBlock)`.
    let rest = if main_is_block { &processing.results[1..] } else { &processing.results[..] };
    let pops = recipes::roll(host, dimension, rest);
    Some(Manual { block, pops, keep_held })
}

/// `manualApplicationRecipesApplyInWorld` once the recipe is found and the held item paid for:
/// the sound, the block broken without drops and replaced, the other results popped out.
pub(super) fn apply_manual(host: &mut dyn Host, pos: [i32; 3], manual: &Manual) {
    host.play_sound(
        rules::MANUAL_SOUND.0,
        util::centre(pos),
        PLAYERS,
        rules::MANUAL_SOUND.1,
        rules::MANUAL_SOUND.2,
    );
    // `BlockHelper.copyProperties`: the properties the two blocks share. A log's axis is the
    // one a casing could have.
    let axis = host.block_property(pos, "axis");
    // `level.destroyBlock(pos, false)`: nothing dropped.
    host.destroy_block(pos, None, true);
    let dimension = host.dimension();
    if let Some(state) = manual.block.as_deref().and_then(|b| host.default_state(b)) {
        host.set_block(dimension, pos, state, util::UPDATE_ALL);
        if let Some(axis) = axis
            && host.block_property(pos, "axis").is_some()
        {
            host.set_block_property(pos, "axis", &axis);
        }
    }
    for stack in &manual.pops {
        // `Block.popResource`.
        let jitter = |n: u64| (n % 1000) as f64 / 2000.0 - 0.25;
        let at = [
            f64::from(pos[0]) + 0.5 + jitter(util::random()),
            f64::from(pos[1]) + 0.5 + jitter(util::random()) - 0.125,
            f64::from(pos[2]) + 0.5 + jitter(util::random()),
        ];
        util::spawn_item(host, at, stack, [0.0; 3], 10);
    }
}

impl Hooks for Apply {
    /// `ManualApplicationRecipe.manualApplicationRecipesApplyInWorld` for a player: a held item
    /// used on a block that a `create:item_application` recipe takes.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        // A Deployer's own fake player is handled by the Deployer, where it can wear what it holds.
        if player.fake || player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        let Some(held) = host.held_stack(player.uuid) else {
            return Flow::Continue;
        };
        let block = host.block(event.pos);
        let Some(manual) = manual_for(host, &block, &held) else {
            return Flow::Continue;
        };
        // The off hand's turn of a click that already applied it (a stack is used up once more).
        if util::click_repeated(host, player.uuid, event.pos) {
            return Flow::Cancel;
        }
        let keep = manual.keep_held || player.game_mode == util::CREATIVE;
        if !keep {
            // `heldItem.shrink(1)`, for what `take_items` can take (a plain stack).
            if held.extra.is_some() || rules::max_damage(&held.item) > 0 || host.take_items(&held.item, 1) == 0 {
                return Flow::Continue;
            }
        }
        util::click_taken(host, player.uuid, event.pos);
        apply_manual(host, event.pos, &manual);
        Flow::Cancel
    }
}
