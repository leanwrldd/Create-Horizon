//! `LevelRef::recipe_for` for the old `Host`: what the engine's recipe matcher makes of an input
//! (vanilla crafting, smelting, smoking, blasting, campfire cooking, stonecutting and smithing,
//! data packs included), without consuming it. Replaces the baked copies of vanilla's recipes.
use super::*;
use std::sync::{Mutex, PoisonError};

/// What goes into the matcher.
pub enum Input<'a> {
    /// `SingleRecipeInput`: cooking and stonecutting.
    Single(&'a ItemStack),
    /// `CraftingInput`: `width` x `height` slots, row by row, `None` for an empty slot.
    Crafting { width: usize, height: usize, slots: &'a [Option<ItemStack>] },
}

/// A match: `RecipeMatch` with the old stacks.
pub struct Found {
    /// The recipe's id (`minecraft:iron_ingot_from_smelting_raw_iron`).
    pub id: String,
    /// `Recipe.assemble` (a cooking recipe's result item).
    pub output: ItemStack,
    /// Per input slot, what is left after one of each occupied slot is used (buckets, bottles).
    pub remainders: Vec<Option<ItemStack>>,
    /// The cooking recipe's experience (0 for other types).
    pub experience: f32,
    /// The recipe file's cooking time (0 for other types).
    pub processing_time: u32,
}

/// Why the engine could not answer, as opposed to answering "no recipe takes it". A caller that
/// destroys or refuses on "no recipe" (the fan's lava) must not do so on this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unavailable {
    /// No level for the dimension (a client, a parked or stale level).
    Level,
    /// The stack could not be turned into a new-API stack (an item the registry lacks).
    Input,
    /// `LevelRef::recipe_for` refused (`UnavailableLevel`, `InvalidItem`, `NotMatchable`...).
    Engine(mod_api::RecipeLookupError),
    /// The match's output could not be read back as an old stack.
    Output,
}

/// Say once per cause that a lookup could not be answered (the game log is where it is found).
fn refused(why: Unavailable) {
    static SEEN: Mutex<Vec<Unavailable>> = Mutex::new(Vec::new());
    let mut seen = SEEN.lock().unwrap_or_else(PoisonError::into_inner);
    if !seen.contains(&why) {
        seen.push(why);
        eprintln!("create: recipe_for could not answer ({why:?}); machines leave the item alone");
    }
}

/// The first recipe of `recipe_type` (`minecraft:smelting`, `minecraft:crafting`...) the matcher takes
/// `input` for, as `RecipeManager.getRecipeFor` would, asked in `dimension` (the level the machine
/// is in; `Host::dimension` is only the level in place). `Ok(None)` is "no recipe takes it";
/// `Err` is "the engine could not say".
pub fn lookup(
    host: &dyn Host,
    dimension: &str,
    recipe_type: &str,
    input: &Input,
) -> Result<Option<Found>, Unavailable> {
    let fail = |why| {
        refused(why);
        Err(why)
    };
    let new = host.new_host();
    let Some(level) = host.level(dimension) else { return fail(Unavailable::Level) };
    let Ok(kind) = Identifier::parse(recipe_type) else {
        return fail(Unavailable::Engine(mod_api::RecipeLookupError::InvalidRecipe));
    };
    let wire = match input {
        Input::Single(stack) => {
            if stack.count == 0 {
                return Ok(None);
            }
            let mut one = (*stack).clone();
            one.count = 1;
            let Some(stack) = events::stack_new(new, &one) else { return fail(Unavailable::Input) };
            RecipeInput::Single(stack)
        }
        Input::Crafting { width, height, slots } => {
            let (Ok(width), Ok(height)) = (u32::try_from(*width), u32::try_from(*height)) else {
                return fail(Unavailable::Engine(mod_api::RecipeLookupError::InvalidInput));
            };
            RecipeInput::Crafting { width, height, slots: events::items_new(new, slots) }
        }
    };
    let found = match level.recipe_for(new, &kind, &wire) {
        Ok(found) => found,
        Err(error) => return fail(Unavailable::Engine(error)),
    };
    let Some(found) = found else { return Ok(None) };
    let Some(output) = events::stack_old(new, &found.output) else { return fail(Unavailable::Output) };
    Ok(Some(Found {
        id: found.recipe.to_string(),
        output,
        remainders: events::items_old(new, &found.remainders),
        experience: found.experience,
        processing_time: found.processing_time,
    }))
}

/// `lookup` in the level in place, with "could not say" read as "no recipe": for the callers
/// that only offer something when a recipe is found (crafter, attribute filter).
pub fn recipe_for(host: &dyn Host, recipe_type: &str, input: &Input) -> Option<Found> {
    lookup(host, host.dimension(), recipe_type, input).ok().flatten()
}
