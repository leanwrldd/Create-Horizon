//! Sequenced assembly: the decisions of `SequencedAssemblyRecipe`, `SequencedRecipe` and
//! `SequencedAssemblyItem`, with no game types. The recipe's JSON, the stack and its component
//! are the glue's (`machines/sequenced.rs`); this takes plain values.
//!
//! An item carries `SequencedAssembly { id, step, progress }`. Each machine that works on it asks
//! which recipe of the assembly is next (`step % sequence.len()`), works that recipe, and the
//! item's result is `advance`: the next transitional item, or, once `loops` passes of the
//! sequence are done, one of the recipe's results picked by weight.
//!
//! Known differences from Create: none in the decisions. (Create's `SequencedRecipe` also
//! rewrites each step's first ingredient to `transitional item or the recipe's ingredient`; the
//! glue matches the same way, `applies`.)

use super::analog_lever::mix_colours;

/// `AllDataComponents.SEQUENCED_ASSEMBLY`: the saved component of an item in the middle of an
/// assembly (a compound of `id`, `step` and `progress`).
pub const COMPONENT: &str = "create:sequenced_assembly";

/// `SequencedAssemblyRecipeSerializer`: `loops` is optional in the file and 1 when it is missing.
/// (The class's own constructor says 5, but the codec writes the field over it.)
pub const DEFAULT_LOOPS: i32 = 1;

/// The kinds of recipe `IAssemblyRecipe` is implemented by that this port has a machine for:
/// `DeployerApplicationRecipe`, `PressingRecipe`, `CuttingRecipe` and `FillingRecipe`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Machine {
    Deploying,
    Pressing,
    Cutting,
    Filling,
}

impl Machine {
    pub const ALL: [Machine; 4] = [Machine::Deploying, Machine::Pressing, Machine::Cutting, Machine::Filling];

    /// The recipe type a step of this kind is written as (`AllRecipeTypes`).
    pub fn recipe_type(self) -> &'static str {
        match self {
            Machine::Deploying => "create:deploying",
            Machine::Pressing => "create:pressing",
            Machine::Cutting => "create:cutting",
            Machine::Filling => "create:filling",
        }
    }

    pub fn of_type(recipe_type: &str) -> Option<Machine> {
        Self::ALL.into_iter().find(|machine| machine.recipe_type() == recipe_type)
    }

    /// The language key of `IAssemblyRecipe.getDescriptionForAssembly`, in Create's namespace:
    /// `recipe.assembly.deploying_item` (with the held item's name), `pressing`, `cutting` and
    /// `spout_filling_fluid` (with the fluid's name).
    pub fn description_key(self) -> &'static str {
        match self {
            Machine::Deploying => "recipe.assembly.deploying_item",
            Machine::Pressing => "recipe.assembly.pressing",
            Machine::Cutting => "recipe.assembly.cutting",
            Machine::Filling => "recipe.assembly.spout_filling_fluid",
        }
    }
}

/// What an item says of its assembly: the recipe it belongs to and how many steps are done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Carried<'a> {
    pub id: &'a str,
    pub step: i32,
}

/// `SequencedAssemblyRecipe.appliesTo`: an item in the middle of an assembly belongs to that
/// recipe if its transitional item is this item; any other item starts the recipe if the
/// recipe's ingredient takes it (`ingredient_takes`, asked only then).
pub fn applies(
    carried: Option<Carried<'_>>,
    recipe_id: &str,
    transitional_item: &str,
    item: &str,
    ingredient_takes: impl FnOnce() -> bool,
) -> bool {
    match carried {
        Some(carried) => transitional_item == item && carried.id == recipe_id,
        None => ingredient_takes(),
    }
}

/// `SequencedAssemblyRecipe.getStep`: 0 for an item that carries nothing.
pub fn step_of(carried: Option<Carried<'_>>) -> i32 {
    carried.map_or(0, |carried| carried.step)
}

/// `getNextRecipe`: which of the `len` recipes of the sequence the item is up to.
pub fn next_index(step: i32, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    step.max(0) as usize % len
}

/// What `advance` makes of an item that has just had its step done.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Advance {
    /// The transitional item again, carrying this step and progress.
    Next { step: i32, progress: f32 },
    /// The loops are done: pick one of the results by weight (`pick`).
    Finished,
}

/// `SequencedAssemblyRecipe.advance` (the integer division is Java's): finished once
/// `(step + 1) / len >= loops`, else the next step with progress `(step + 1) / (len * loops)`.
pub fn advance(step: i32, len: usize, loops: i32) -> Advance {
    let len = len.max(1) as i32;
    if (step + 1) / len >= loops {
        return Advance::Finished;
    }
    Advance::Next { step: step + 1, progress: (step as f32 + 1.0) / (len * loops) as f32 }
}

/// `SequencedAssemblyRecipe.rollResult`: the index of the result a roll `unit` (`nextFloat()`,
/// 0 up to 1) picks, each result weighing its `chance`. `None` when nothing is picked (no
/// results, or no weight at all): Create's empty stack.
pub fn pick(weights: &[f32], unit: f32) -> Option<usize> {
    let total: f32 = weights.iter().sum();
    let mut number = unit * total;
    for (index, weight) in weights.iter().enumerate() {
        number -= weight;
        if number < 0.0 {
            return Some(index);
        }
    }
    None
}

/// `SequencedAssemblyRecipe.getOutputChance`: the chance of the first result.
pub fn output_chance(weights: &[f32]) -> f32 {
    let total: f32 = weights.iter().sum();
    weights.first().map_or(0.0, |first| first / total)
}

/// `SequencedAssemblyItem.getBarWidth`: `Math.round(progress * 13)`.
pub fn bar_width(progress: f32) -> i32 {
    (progress * 13.0 + 0.5).floor() as i32
}

/// `SequencedAssemblyItem.getBarColor`: `Color.mixColors(0xFFC074, 0x46FFE0, progress)`.
pub fn bar_colour(progress: f32) -> u32 {
    mix_colours(0xFF_C074, 0x46_FFE0, progress)
}

/// What `SequencedAssemblyRecipe.addToTooltip` lists for an item at `step` of a sequence of `len`
/// recipes done `loops` times.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outline {
    /// `Progress: step/total`.
    pub step: i32,
    pub total: i32,
    /// The indices into the sequence of what comes next, the first as `Next: ...` and the rest
    /// as `-> ...`; at most one pass of the sequence, and none past the last step.
    pub next: Vec<usize>,
}

pub fn outline(step: i32, len: usize, loops: i32) -> Outline {
    let total = len as i32 * loops;
    let remaining = total - step;
    let next = (0..len)
        .take_while(|&i| (i as i32) < remaining)
        .map(|i| (i + step.max(0) as usize) % len)
        .collect();
    Outline { step, total, next }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_item_belongs_to_the_recipe_whose_ingredient_takes_it() {
        assert!(applies(None, "a", "t", "plate", || true));
        assert!(!applies(None, "a", "t", "plate", || false));
    }

    #[test]
    fn an_item_in_the_middle_belongs_to_its_recipe_only() {
        let carried = Some(Carried { id: "a", step: 2 });
        // The ingredient isn't asked: the transitional item and the id decide.
        assert!(applies(carried, "a", "t", "t", || panic!("not asked")));
        assert!(!applies(carried, "b", "t", "t", || true));
        assert!(!applies(carried, "a", "t", "plate", || true));
    }

    #[test]
    fn the_next_recipe_goes_round_the_sequence() {
        assert_eq!((0..7).map(|s| next_index(s, 3)).collect::<Vec<_>>(), [0, 1, 2, 0, 1, 2, 0]);
        assert_eq!(next_index(4, 0), 0);
        assert_eq!(step_of(None), 0);
        assert_eq!(step_of(Some(Carried { id: "a", step: 4 })), 4);
    }

    #[test]
    fn precision_mechanism_takes_fifteen_steps() {
        // Three recipes, five loops: steps 0..=13 advance, the 15th step (index 14) finishes.
        for step in 0..14 {
            match advance(step, 3, 5) {
                Advance::Next { step: next, progress } => {
                    assert_eq!(next, step + 1);
                    assert!((progress - (step + 1) as f32 / 15.0).abs() < 1e-6);
                }
                Advance::Finished => panic!("finished early at {step}"),
            }
        }
        assert_eq!(advance(14, 3, 5), Advance::Finished);
    }

    #[test]
    fn one_loop_of_three_finishes_on_the_third_step() {
        // The Sturdy Sheet: filling, pressing, pressing.
        assert!(matches!(advance(0, 3, 1), Advance::Next { step: 1, .. }));
        assert!(matches!(advance(1, 3, 1), Advance::Next { step: 2, .. }));
        assert_eq!(advance(2, 3, 1), Advance::Finished);
    }

    #[test]
    fn no_loops_finish_at_once() {
        // `loops: 0` is allowed by the codec (`NON_NEGATIVE_INT`).
        assert_eq!(advance(0, 3, 0), Advance::Finished);
    }

    #[test]
    fn a_result_is_picked_by_weight() {
        let weights = [120.0, 8.0, 8.0, 5.0, 3.0, 2.0, 2.0, 1.0, 1.0];
        let total: f32 = weights.iter().sum();
        assert_eq!(total, 150.0);
        assert_eq!(pick(&weights, 0.0), Some(0));
        // 120 / 150 = 0.8: just under it is still the first result, just over the second.
        assert_eq!(pick(&weights, 0.79), Some(0));
        assert_eq!(pick(&weights, 0.81), Some(1));
        assert_eq!(pick(&weights, 0.999), Some(8));
        assert_eq!(pick(&[], 0.5), None);
        assert_eq!(pick(&[0.0], 0.5), None);
        assert!((output_chance(&weights) - 0.8).abs() < 1e-6);
    }

    #[test]
    fn the_bar_fills_with_progress() {
        assert_eq!(bar_width(0.0), 0);
        assert_eq!(bar_width(0.5), 7);
        assert_eq!(bar_width(1.0), 13);
        assert_eq!(bar_colour(0.0), 0xFF_C074);
        assert_eq!(bar_colour(1.0), 0x46_FFE0);
    }

    #[test]
    fn the_tooltip_lists_what_comes_next() {
        // Step 4 of 15 in a sequence of 3: the next three, starting at index 1.
        let outline = outline(4, 3, 5);
        assert_eq!((outline.step, outline.total), (4, 15));
        assert_eq!(outline.next, [1, 2, 0]);
        // One step from the end lists only that step.
        assert_eq!(super::outline(14, 3, 5).next, [2]);
        assert!(super::outline(15, 3, 5).next.is_empty());
    }

    #[test]
    fn machines_are_told_by_recipe_type() {
        for machine in Machine::ALL {
            assert_eq!(Machine::of_type(machine.recipe_type()), Some(machine));
        }
        assert_eq!(Machine::of_type("create:milling"), None);
    }
}
