//! Sequenced assembly: `SequencedAssemblyRecipe`, `SequencedRecipe`, `SequencedAssemblyItem` and
//! the `create:sequenced_assembly` recipes of Create's datagen. An item in the middle of an
//! assembly is the recipe's transitional item (`create:incomplete_precision_mechanism`) carrying
//! the component `create:sequenced_assembly` (`id`, `step`, `progress`). The Deployer, the
//! Mechanical Press, the Mechanical Saw and the Spout each ask [`find`] for a sequenced step
//! before looking at their own recipes, as `getRecipe` does (priority 100 in the Deployer), and
//! the item that comes out is [`Step::roll`]: the next transitional item, or, after the last
//! loop, one of the recipe's results picked by weight.
//!
//! The decisions are `create_kinetics::rules::sequenced`; this reads the recipe files, the
//! component and the world's random, and registers the component, the progress bar and the
//! tooltip of the transitional items.
//!
//! Known differences from Create:
//! - Only the four kinds of step this port has a machine for run: deploying, pressing, cutting
//!   and filling. A recipe with a step of another kind (none of Create's has) is never found.
//! - The `create:track` recipe is not declared: there are no Track and Sleepers yet.
//! - The tooltip's lines come from Create's language file (`create.recipe.assembly.*`), as the
//!   goggle overlay's do. A Deployer step is described by the first item its held ingredient
//!   lists, a filling step by its fluid's name (`IAssemblyRecipe.getDescriptionForAssembly`).
//! - The item's component is a registered Horizon component (`Hooks::component_types`), not a
//!   NeoForge `DataComponentType`; it carries the same three values. (A client shows the
//!   progress bar and tooltip only if the server sends the component: untested.)
//! - Step recipes aren't matched by `SequencedRecipe.initFromSequencedAssembly`'s rewritten first
//!   ingredient: the transitional item or the recipe's ingredient is checked by `applies`
//!   instead, which is the same test.

use create_kinetics::rules::sequenced::{self as rules, Advance, Carried, Machine};
use crate::compat::serde_json::Value;
use crate::compat::{
    ComponentTypesEvent, Compound, Hooks, Host, ItemBarEvent, ItemStack, RandomSource, Tag, Text,
    TooltipEvent,
};

use crate::recipes::{self, Output, Processing};

/// The recipe type of Create's datagen.
pub const TYPE: &str = "create:sequenced_assembly";
/// `AllItems.INCOMPLETE_PRECISION_MECHANISM`, `INCOMPLETE_REINFORCED_SHEET` and `INCOMPLETE_TRACK`:
/// the `SequencedAssemblyItem`s, which show a progress bar.
const ITEMS: [&str; 3] = [
    "create:incomplete_precision_mechanism",
    "create:unprocessed_obsidian_sheet",
    "create:incomplete_track",
];

struct Sequenced;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Sequenced)
}

// --- the item's component -------------------------------------------------------------------------

/// What `stack` says of its assembly (`AllDataComponents.SEQUENCED_ASSEMBLY`): the recipe id, the
/// steps done and the progress.
pub(crate) fn carried(stack: &ItemStack) -> Option<(String, i32, f32)> {
    let tag = stack.data()?.other(rules::COMPONENT)?;
    let id = match tag.get("id") {
        Some(Tag::String(id)) => id.to_string_lossy(),
        _ => return None,
    };
    let step = match tag.get("step") {
        Some(Tag::Int(step)) => *step,
        _ => 0,
    };
    let progress = match tag.get("progress") {
        Some(Tag::Float(bits)) => f32::from_bits(*bits),
        _ => 0.0,
    };
    Some((id, step, progress))
}

fn set_carried(host: &dyn Host, stack: &mut ItemStack, id: &str, step: i32, progress: f32) {
    let mut tag = Compound::new();
    tag.insert("id".into(), Tag::String(id.into()));
    tag.insert("step".into(), Tag::Int(step));
    tag.insert("progress".into(), Tag::Float(progress.to_bits()));
    // A refusal (a component the server doesn't carry) leaves the item as it was.
    let _ = host.set_component(stack, rules::COMPONENT, Tag::Compound(tag));
}

// --- finding the next step ----------------------------------------------------------------------

/// One step of an assembly that an item is up to.
#[derive(Clone)]
pub(crate) struct Step {
    /// The assembly recipe's id (`create:sequenced_assembly/precision_mechanism`).
    pub assembly: String,
    /// Steps done so far (`getStep`).
    pub step: i32,
    pub len: usize,
    pub loops: i32,
    /// The step's own recipe as the file writes it: its held ingredient or its fluid, its time.
    pub recipe: Processing,
    /// `ItemApplicationRecipe.shouldKeepHeldItem` of a deploying step (`keep_held_item`).
    pub keep_held: bool,
    transitional: ItemStack,
    /// `resultPool`: the weighted results of the whole assembly.
    pool: Vec<Output>,
}

impl Step {
    /// `ProcessingRecipe.forcedResult` (`SequencedAssemblyRecipe.advance`): what the item becomes.
    /// A result that rolls nothing is `None` (Create's empty stack).
    pub fn forced(&self, host: &mut dyn Host) -> Option<ItemStack> {
        match rules::advance(self.step, self.len, self.loops) {
            Advance::Next { step, progress } => {
                let mut next = self.transitional.clone();
                next.count = 1;
                set_carried(&*host, &mut next, &self.assembly, step, progress);
                Some(next)
            }
            Advance::Finished => {
                let weights: Vec<f32> = self.pool.iter().map(|output| output.chance).collect();
                let dimension = host.dimension();
                let unit = host.level_random(dimension).map_or(0.0, |random| random.next_float());
                rules::pick(&weights, unit).map(|index| self.pool[index].stack.clone())
            }
        }
    }

    /// `ProcessingRecipe.rollResults` of the step: the first result is the forced one, the others
    /// (a step has no others in Create's recipes) roll by their chance.
    pub fn roll(&self, host: &mut dyn Host) -> Vec<ItemStack> {
        let mut made = Vec::new();
        for (index, output) in self.recipe.results.iter().enumerate() {
            let stack = if index == 0 {
                self.forced(host)
            } else {
                let dimension = host.dimension();
                recipes::roll(host, dimension, std::slice::from_ref(output)).into_iter().next()
            };
            made.extend(stack.filter(|stack| stack.count > 0));
        }
        made
    }
}

fn ingredient_takes(host: &dyn Host, json: &Value, stack: &ItemStack) -> bool {
    json.get("ingredient")
        .and_then(|ingredient| host.recipe_ingredient(&recipes::modern_ingredient(ingredient)))
        .is_some_and(|ingredient| ingredient.test(stack))
}

/// `SequencedAssemblyRecipe.getRecipes` for one kind of machine: the step an item is up to, in
/// the first assembly that applies to it whose next step is of `machine`'s kind and that `accept`
/// takes (a Deployer's held item, a Spout's fluid). `None` when no assembly has a step for it.
pub(crate) fn find(
    host: &dyn Host,
    stack: &ItemStack,
    machine: Machine,
    accept: &dyn Fn(&Step) -> bool,
) -> Option<Step> {
    let have = carried(stack);
    let kinds = host.recipes_of_type(TYPE);
    if kinds.is_empty() {
        return None;
    }
    for recipe in kinds {
        let json = &*recipe.json;
        let transitional = json
            .get("transitional_item")
            .and_then(|item| item.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let carried = have.as_ref().map(|(id, step, _)| Carried { id, step: *step });
        if !rules::applies(carried, &recipe.id, transitional, &stack.item, || {
            ingredient_takes(host, json, stack)
        }) {
            continue;
        }
        let Some(sequence) = json.get("sequence").and_then(Value::as_array) else {
            continue;
        };
        let step = rules::step_of(carried);
        let Some(next) = sequence.get(rules::next_index(step, sequence.len())) else {
            continue;
        };
        if next.get("type").and_then(Value::as_str).and_then(Machine::of_type) != Some(machine) {
            continue;
        }
        let Some(processing) = recipes::parse(host, &recipe.id, next) else {
            continue;
        };
        let Some(transitional) = json.get("transitional_item").and_then(|t| host.recipe_result(t)) else {
            continue;
        };
        let pool: Vec<Output> = json
            .get("results")
            .and_then(Value::as_array)
            .map(|results| results.iter().filter_map(|r| recipes::output(host, r)).collect())
            .unwrap_or_default();
        let found = Step {
            assembly: recipe.id.clone(),
            step,
            len: sequence.len(),
            loops: json
                .get("loops")
                .and_then(Value::as_i64)
                .map_or(rules::DEFAULT_LOOPS, |loops| loops as i32),
            recipe: processing,
            keep_held: next.get("keep_held_item").and_then(Value::as_bool).unwrap_or(false),
            transitional,
            pool,
        };
        if accept(&found) {
            return Some(found);
        }
    }
    None
}

// --- the hooks -------------------------------------------------------------------------------------

/// `AllItemTags`-style test for the items that carry a progress bar.
fn is_assembly_item(id: &str) -> bool {
    ITEMS.contains(&id)
}

fn description(lang: &create_kinetics::rules::goggles::Lang, host: &dyn Host, assembly: &str, index: usize) -> Option<String> {
    let recipe = host.recipes_of_type(TYPE).iter().find(|r| r.id == assembly)?;
    let step = recipe.json.get("sequence")?.get(index)?;
    let machine = Machine::of_type(step.get("type")?.as_str()?)?;
    let key = machine.description_key();
    Some(match machine {
        Machine::Deploying => {
            // `ingredients.get(1).getItems()[0]`.
            let held = step.get("ingredients")?.get(1)?;
            let first = host.recipe_ingredient(&recipes::modern_ingredient(held))?.order.first()?.clone();
            lang.tr(key, &[lang.item_name(&first)])
        }
        Machine::Filling => {
            let fluid = step.get("ingredients")?.as_array()?.iter().find(|i| i.get("amount").is_some())?;
            let name = match fluid.get("fluid").and_then(Value::as_str) {
                Some(id) => fluid_name(lang, id),
                None => "Invalid".to_owned(),
            };
            lang.tr(key, &[name])
        }
        Machine::Pressing | Machine::Cutting => lang.tr(key, &[]),
    })
}

/// `FluidStack.getHoverName` of a fluid id: `block.minecraft.water`, `fluid.create.honey`.
fn fluid_name(lang: &create_kinetics::rules::goggles::Lang, id: &str) -> String {
    let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
    [format!("block.{namespace}.{path}"), format!("fluid.{namespace}.{path}")]
        .iter()
        .find_map(|key| lang.get(key).map(str::to_owned))
        .unwrap_or_else(|| path.replace('_', " "))
}

impl Hooks for Sequenced {
    /// `AllDataComponents.SEQUENCED_ASSEMBLY`.
    fn component_types(&self, event: &mut ComponentTypesEvent<'_>) {
        if !event.types.iter().any(|t| t == rules::COMPONENT) {
            event.types.push(rules::COMPONENT.to_owned());
        }
    }

    /// `SequencedAssemblyItem.isBarVisible`, `getBarWidth`, `getBarColor`.
    fn item_bar(&self, _host: &dyn Host, event: &mut ItemBarEvent<'_>) {
        if !is_assembly_item(&event.stack.item) {
            return;
        }
        let progress = carried(event.stack).map_or(0.0, |(_, _, progress)| progress);
        *event.bar = Some((rules::bar_width(progress), rules::bar_colour(progress)));
    }

    /// `SequencedAssemblyRecipe.addToTooltip`.
    fn tooltip(&self, host: &dyn Host, event: &mut TooltipEvent<'_>) {
        let Some((assembly, step, _)) = carried(event.stack) else {
            return;
        };
        let Some(recipe) = host.recipes_of_type(TYPE).iter().find(|r| r.id == assembly) else {
            return;
        };
        let len = recipe.json.get("sequence").and_then(Value::as_array).map_or(0, Vec::len);
        let loops = recipe
            .json
            .get("loops")
            .and_then(Value::as_i64)
            .map_or(rules::DEFAULT_LOOPS, |loops| loops as i32);
        if len == 0 {
            return;
        }
        let lang = super::goggles::lang(host);
        let outline = rules::outline(step, len, loops);
        // `ChatFormatting.GRAY`, `DARK_GRAY`, `AQUA`, `DARK_AQUA`.
        event.lines.push((Text::literal(String::new()), u32::MAX));
        event.lines.push((Text::literal(lang.tr("recipe.sequenced_assembly", &[])), 0xFFAA_AAAA));
        event.lines.push((
            Text::literal(lang.tr(
                "recipe.assembly.progress",
                &[outline.step.to_string(), outline.total.to_string()],
            )),
            0xFF55_5555,
        ));
        for (i, index) in outline.next.iter().enumerate() {
            let Some(text) = description(&lang, host, &assembly, *index) else {
                continue;
            };
            if i == 0 {
                event.lines.push((Text::literal(lang.tr("recipe.assembly.next", &[text])), 0xFF55_FFFF));
            } else {
                event.lines.push((Text::literal(format!("-> {text}")), 0xFF00_AAAA));
            }
        }
    }
}
