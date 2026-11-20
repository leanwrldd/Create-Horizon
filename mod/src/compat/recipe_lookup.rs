//! `LevelRef::recipe_for` for the old `Host`: what the engine's recipe matcher makes of an input
//! (vanilla crafting, smelting, smoking, blasting, campfire cooking, stonecutting and smithing,
//! data packs included), without consuming it. Replaces the baked copies of vanilla's recipes.
use super::*;

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

/// The first recipe of `recipe_type` (`minecraft:smelting`, `minecraft:crafting`...) the matcher takes
/// `input` for, as `RecipeManager.getRecipeFor` would. `None` for no match, for a type the engine
/// can't match, or when no level is there.
pub fn recipe_for(host: &dyn Host, recipe_type: &str, input: &Input) -> Option<Found> {
    let new = host.new_host();
    let level = host.level(host.dimension())?;
    let kind = Identifier::parse(recipe_type).ok()?;
    let wire = match input {
        Input::Single(stack) => {
            let mut one = (*stack).clone();
            one.count = 1;
            RecipeInput::Single(events::stack_new(new, &one)?)
        }
        Input::Crafting { width, height, slots } => RecipeInput::Crafting {
            width: u32::try_from(*width).ok()?,
            height: u32::try_from(*height).ok()?,
            slots: events::items_new(new, slots),
        },
    };
    let found = level.recipe_for(new, &kind, &wire).ok()??;
    Some(Found {
        id: found.recipe.to_string(),
        output: events::stack_old(new, &found.output)?,
        remainders: events::items_old(new, &found.remainders),
        experience: found.experience,
        processing_time: found.processing_time,
    })
}
