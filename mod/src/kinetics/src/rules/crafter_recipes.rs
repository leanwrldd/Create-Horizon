//! Which recipes a Mechanical Crafter's grid matches, from Create `mc1.21.1/dev` at 4148fd8 (MIT,
//! the Creators of Create): `MechanicalCraftingRecipe.matches` (and `matchesSpecific`),
//! `ShapedRecipe`/`ShapedRecipePattern.matches` and `ShapelessRecipe.matches` of the vanilla
//! crafting recipes the crafter also accepts (`allowRegularCraftingInCrafter`).
//!
//! No game types: an ingredient is `I`, a stack `S`, and `test` says whether an ingredient takes
//! a stack. The input is `MechanicalCraftingInput`, `Grid::input`: the grid cut to its items,
//! row by row from the top.

/// A shaped pattern: `width` by `height`, row by row from the top; `None` is a space.
#[derive(Clone, Debug, PartialEq)]
pub struct Shaped<I> {
    pub width: usize,
    pub height: usize,
    pub cells: Vec<Option<I>>,
}

impl<I> Shaped<I> {
    /// `ShapedRecipePattern.shrink`: the pattern without its empty outer rows and columns.
    pub fn trimmed(rows: Vec<Vec<Option<I>>>) -> Self {
        let width = rows.iter().map(Vec::len).max().unwrap_or(0);
        let used_row = |r: &Vec<Option<I>>| r.iter().any(Option::is_some);
        let top = rows.iter().position(used_row);
        let bottom = rows.iter().rposition(used_row);
        let (Some(top), Some(bottom)) = (top, bottom) else {
            return Shaped { width: 0, height: 0, cells: Vec::new() };
        };
        let left = (0..width)
            .find(|c| rows.iter().any(|r| r.get(*c).is_some_and(Option::is_some)))
            .unwrap_or(0);
        let right = (0..width)
            .rfind(|c| rows.iter().any(|r| r.get(*c).is_some_and(Option::is_some)))
            .unwrap_or(0);
        let mut cells = Vec::new();
        for row in rows.into_iter().skip(top).take(bottom - top + 1) {
            let mut row: Vec<Option<I>> = row.into_iter().collect();
            row.resize_with(width, || None);
            cells.extend(row.into_iter().skip(left).take(right - left + 1));
        }
        Shaped { width: right - left + 1, height: bottom - top + 1, cells }
    }
}

/// How a shaped recipe is matched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// A vanilla recipe, or a mechanical one with `accept_mirrored`: the same size, also
    /// mirrored left to right (`ShapedRecipe.matches`).
    Mirrored,
    /// A mechanical recipe without it: as written, anywhere in the input where the rest of the
    /// input is empty (`MechanicalCraftingRecipe.matches`).
    AsWritten,
}

/// Whether the cell of a pattern takes the cell of the input: a space takes only an empty cell.
fn cell_matches<I, S>(pattern: Option<&I>, input: Option<&S>, test: &dyn Fn(&I, &S) -> bool) -> bool {
    match (pattern, input) {
        (None, None) => true,
        (Some(ingredient), Some(stack)) => test(ingredient, stack),
        _ => false,
    }
}

/// `ShapedRecipe.matches` / `MechanicalCraftingRecipe.matches` for an input `width` by `height`.
pub fn shaped_matches<I, S>(
    pattern: &Shaped<I>,
    mode: Mode,
    width: usize,
    height: usize,
    input: &[Option<S>],
    test: &dyn Fn(&I, &S) -> bool,
) -> bool {
    if pattern.width == 0 || pattern.height == 0 || input.len() != width * height {
        return false;
    }
    match mode {
        Mode::Mirrored => {
            if (width, height) != (pattern.width, pattern.height) {
                return false;
            }
            let fits = |mirror: bool| {
                (0..height).all(|y| {
                    (0..width).all(|x| {
                        let px = if mirror { width - 1 - x } else { x };
                        cell_matches(
                            pattern.cells[y * width + px].as_ref(),
                            input[y * width + x].as_ref(),
                            test,
                        )
                    })
                })
            };
            fits(false) || fits(true)
        }
        Mode::AsWritten => {
            if width < pattern.width || height < pattern.height {
                return false;
            }
            (0..=width - pattern.width).any(|i| {
                (0..=height - pattern.height).any(|j| {
                    // `matchesSpecific`: every input cell against the pattern cell over it, a
                    // space outside the pattern.
                    (0..width).all(|x| {
                        (0..height).all(|y| {
                            let (k, l) = (x as isize - i as isize, y as isize - j as isize);
                            let inside = k >= 0
                                && l >= 0
                                && (k as usize) < pattern.width
                                && (l as usize) < pattern.height;
                            let cell = inside
                                .then(|| pattern.cells[l as usize * pattern.width + k as usize].as_ref())
                                .flatten();
                            cell_matches(cell, input[y * width + x].as_ref(), test)
                        })
                    })
                })
            })
        }
    }
}

/// `ShapelessRecipe.matches`: as many items as ingredients, and every ingredient takes a
/// different item (a bipartite match, as `StackedContents.canCraft`).
pub fn shapeless_matches<I, S>(
    ingredients: &[I],
    input: &[Option<S>],
    test: &dyn Fn(&I, &S) -> bool,
) -> bool {
    let items: Vec<&S> = input.iter().flatten().collect();
    if items.len() != ingredients.len() || items.is_empty() {
        return false;
    }
    if items.len() == 1 {
        return test(&ingredients[0], items[0]);
    }
    // Kuhn's augmenting paths: ingredients to items.
    let mut owner: Vec<Option<usize>> = vec![None; items.len()];
    fn augment<I, S>(
        i: usize,
        ingredients: &[I],
        items: &[&S],
        test: &dyn Fn(&I, &S) -> bool,
        seen: &mut Vec<bool>,
        owner: &mut Vec<Option<usize>>,
    ) -> bool {
        for (j, item) in items.iter().enumerate() {
            if seen[j] || !test(&ingredients[i], item) {
                continue;
            }
            seen[j] = true;
            if owner[j].is_none_or(|prev| augment(prev, ingredients, items, test, seen, owner)) {
                owner[j] = Some(i);
                return true;
            }
        }
        false
    }
    (0..ingredients.len()).all(|i| {
        let mut seen = vec![false; items.len()];
        augment(i, ingredients, &items, test, &mut seen, &mut owner)
    })
}

/// `Item.Properties.craftRemainder` of the vanilla items that have one
/// (`horizon_items::CRAFT_REMAINDERS`, which Horizon's own recipes use): what a stack of the
/// item leaves in the grid. Create asks `stack.getCraftingRemainingItem()`.
pub fn crafting_remainder(item: &str) -> Option<&'static str> {
    match item {
        "minecraft:water_bucket" | "minecraft:lava_bucket" | "minecraft:milk_bucket" => {
            Some("minecraft:bucket")
        }
        "minecraft:dragon_breath" | "minecraft:honey_bottle" => Some("minecraft:glass_bottle"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pat(rows: &[&str]) -> Shaped<char> {
        Shaped::trimmed(
            rows.iter()
                .map(|r| r.chars().map(|c| (c != ' ').then_some(c)).collect())
                .collect(),
        )
    }

    fn input(rows: &[&str]) -> (usize, usize, Vec<Option<char>>) {
        let w = rows[0].len();
        let cells = rows
            .iter()
            .flat_map(|r| r.chars().map(|c| (c != ' ').then_some(c)))
            .collect();
        (w, rows.len(), cells)
    }

    fn same(i: &char, s: &char) -> bool {
        i == s
    }

    #[test]
    fn a_pattern_is_trimmed_to_its_items() {
        let p = pat(&["   ", " a ", " b "]);
        assert_eq!((p.width, p.height), (1, 2));
        assert_eq!(p.cells, vec![Some('a'), Some('b')]);
        assert_eq!(pat(&["a b"]).width, 3);
        assert_eq!(pat(&["  "]).width, 0);
    }

    #[test]
    fn a_vanilla_shaped_recipe_must_match_the_size_and_may_be_mirrored() {
        let p = pat(&["ab", "c "]);
        let (w, h, i) = input(&["ab", "c "]);
        assert!(shaped_matches(&p, Mode::Mirrored, w, h, &i, &same));
        let (w, h, i) = input(&["ba", " c"]);
        assert!(shaped_matches(&p, Mode::Mirrored, w, h, &i, &same));
        let (w, h, i) = input(&["ab", " c"]);
        assert!(!shaped_matches(&p, Mode::Mirrored, w, h, &i, &same));
        let (w, h, i) = input(&["abc"]);
        assert!(!shaped_matches(&p, Mode::Mirrored, w, h, &i, &same));
    }

    #[test]
    fn a_mechanical_recipe_without_mirroring_matches_as_written_only() {
        let p = pat(&["ab", "c "]);
        let (w, h, i) = input(&["ab", "c "]);
        assert!(shaped_matches(&p, Mode::AsWritten, w, h, &i, &same));
        let (w, h, i) = input(&["ba", " c"]);
        assert!(!shaped_matches(&p, Mode::AsWritten, w, h, &i, &same));
        // A hole in the input where the pattern has a space is no match, and a bigger input
        // matches only where everything else is empty.
        let (w, h, i) = input(&["ab", "cd"]);
        assert!(!shaped_matches(&p, Mode::AsWritten, w, h, &i, &same));
    }

    #[test]
    fn a_pattern_with_holes_wants_the_holes_empty() {
        let p = pat(&["a a", " b "]);
        let (w, h, i) = input(&["a a", " b "]);
        assert!(shaped_matches(&p, Mode::AsWritten, w, h, &i, &same));
        let (w, h, i) = input(&["aaa", " b "]);
        assert!(!shaped_matches(&p, Mode::AsWritten, w, h, &i, &same));
    }

    #[test]
    fn shapeless_recipes_take_the_items_in_any_order() {
        let ing = ['a', 'b', 'c'];
        let (_, _, i) = input(&["cab"]);
        assert!(shapeless_matches(&ing, &i, &same));
        let (_, _, i) = input(&["ca "]);
        assert!(!shapeless_matches(&ing, &i, &same));
        let (_, _, i) = input(&["caba"]);
        assert!(!shapeless_matches(&ing, &i, &same));
        assert!(!shapeless_matches::<char, char>(&[], &[], &same));
    }

    #[test]
    fn shapeless_ingredients_that_overlap_still_find_their_items() {
        // Ingredient 0 takes 'x' or 'y', ingredient 1 takes only 'x': a greedy match would fail.
        let ing = ["xy", "x"];
        let test = |i: &&str, s: &char| i.contains(*s);
        assert!(shapeless_matches(&ing, &[Some('x'), Some('y')], &test));
        assert!(!shapeless_matches(&ing, &[Some('y'), Some('y')], &test));
    }

    #[test]
    fn vanilla_craft_remainders_are_buckets_and_bottles() {
        assert_eq!(crafting_remainder("minecraft:milk_bucket"), Some("minecraft:bucket"));
        assert_eq!(crafting_remainder("minecraft:honey_bottle"), Some("minecraft:glass_bottle"));
        assert_eq!(crafting_remainder("minecraft:iron_ingot"), None);
    }
}
