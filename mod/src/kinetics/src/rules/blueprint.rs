//! The Crafting Blueprint's rules (`BlueprintEntity`, `BlueprintItem`): a recipe held as ghost
//! items, and crafting it from what a player carries. No game types: the game glue is
//! `machines/blueprint.rs`.
//!
//! Known differences from Create, for the README:
//! - **A block stands in for the entity.** Horizon has no mod entity type, so the Crafting
//!   Blueprint is a thin one-block plate (`create:crafting_blueprint`, placed on a wall, floor or
//!   ceiling like the Placard) with a block entity; it has no 2x2 or 3x3 sizes, only Create's size 1
//!   (one recipe).
//! - **No screen.** Create opens a crafting menu to set the 3x3 ghost items and the result. Here
//!   a sneaking click with an item sets the recipe that makes it (as Create's "assign recipe" button
//!   does for a recipe viewer): tags become Attribute Filters, several items a List Filter, a single
//!   item itself; clicking again with the same item moves on to the next recipe for it.
//! - Recipes come from the baked crafting tables the Mechanical Crafter reads
//!   (`create:crafter_baked_shaped`/`shapeless`), because a mod can't read vanilla's crafting
//!   recipes on this Horizon build: the same table to drop when the project moves to the new
//!   mod API's `recipe_for`. A data pack's crafting recipes are not seen.
//! - Crafting takes the ingredients with `Host::take_items`, which takes plain stacks only (no
//!   components) from the carried inventory: an ingredient with components isn't used.
//! - There is no overlay (`BlueprintOverlayRenderer`) listing what the player has; the action bar
//!   names the first missing ingredient.

/// The ghost slots of one recipe: a 3x3 grid and the result (`BlueprintSection`).
pub const GRID: usize = 9;
pub const RESULT_SLOT: usize = 9;

/// `BlueprintEntity.interactAt`: the crafts of one click add up to at most this many items.
pub const MAX_CRAFTED: u32 = 64;

/// What a ghost slot holds, before it is turned into a stack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ghost {
    Empty,
    Item(String),
    /// An ingredient that takes a whole tag: an Attribute Filter on `in_tag`.
    Tag(String),
    /// An ingredient that takes several: a List Filter of these (at most 18).
    List(Vec<Ghost>),
}

/// `BlueprintItem.convertIngredientToFilter`: the ghost for the entries an ingredient accepts
/// (`"minecraft:oak_planks"` is an item, `"#minecraft:planks"` a tag). No entry, or more than 18,
/// leaves the slot empty.
pub fn ghost_for(accepted: &[String]) -> Ghost {
    let one = |entry: &String| match entry.strip_prefix('#') {
        Some(tag) => Ghost::Tag(tag.to_owned()),
        None => Ghost::Item(entry.clone()),
    };
    match accepted {
        [] => Ghost::Empty,
        [only] => one(only),
        many if many.len() > 18 => Ghost::Empty,
        many => Ghost::List(many.iter().map(one).collect()),
    }
}

/// `BlueprintItem.assignCompleteRecipe` for a shaped recipe: the pattern's cells go to the grid
/// from the top-left corner, row by row, three to a row. Cells are `None` for a blank.
pub fn shaped_slots<T: Clone>(rows: &[Vec<Option<T>>]) -> [Option<T>; GRID] {
    let mut slots: [Option<T>; GRID] = Default::default();
    for (row, cells) in rows.iter().enumerate().take(3) {
        for (col, cell) in cells.iter().enumerate().take(3) {
            slots[row * 3 + col] = cell.clone();
        }
    }
    slots
}

/// `BlueprintItem.assignCompleteRecipe` for a shapeless recipe: one slot for each ingredient.
pub fn shapeless_slots<T: Clone>(ingredients: &[T]) -> [Option<T>; GRID] {
    let mut slots: [Option<T>; GRID] = Default::default();
    for (slot, item) in ingredients.iter().enumerate().take(GRID) {
        slots[slot] = Some(item.clone());
    }
    slots
}

/// Which of `count` recipes for an item a click shows: the first, then on for each click with the
/// same item.
pub fn next_variant(variant: i32, count: usize) -> i32 {
    if count == 0 { 0 } else { (variant + 1).rem_euclid(count as i32) }
}

/// One carried slot a recipe's ingredients are looked for in: the stack and how many it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Carried<S> {
    pub stack: S,
    pub count: u32,
}

/// What a click's crafting did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session<S> {
    /// The ingredients of each craft, in grid order.
    pub grids: Vec<[Option<S>; GRID]>,
    /// What was taken: the carried slot (as listed) and how many.
    pub taken: Vec<(usize, u32)>,
    /// How many items the crafts made.
    pub crafted: u32,
    /// The first ghost slot nothing carried fills, when the last attempt stopped for that.
    pub missing: Option<usize>,
}

/// `BlueprintEntity.interactAt`'s crafting: for each ghost slot in turn the first carried slot that
/// passes `fits` gives one item; `make` says how many items the grid makes (none if it matches no
/// recipe). With `repeat` (sneaking) it goes on until something fails or 64 items are made. Slots
/// are listed in the order the inventory is searched.
pub fn craft<S: Clone, R>(
    requests: &[Option<R>; GRID],
    carried: &[Carried<S>],
    repeat: bool,
    fits: &mut dyn FnMut(&R, &S) -> bool,
    make: &mut dyn FnMut(&[Option<S>; GRID]) -> Option<u32>,
) -> Session<S> {
    let mut left: Vec<u32> = carried.iter().map(|c| c.count).collect();
    let mut session = Session { grids: Vec::new(), taken: Vec::new(), crafted: 0, missing: None };
    loop {
        let mut grid: [Option<S>; GRID] = Default::default();
        let mut took: Vec<(usize, u32)> = Vec::new();
        let mut ok = true;
        for (slot, request) in requests.iter().enumerate() {
            let Some(request) = request else {
                continue;
            };
            let found = carried
                .iter()
                .enumerate()
                .find(|(index, c)| left[*index] > 0 && fits(request, &c.stack));
            let Some((index, c)) = found else {
                session.missing = Some(slot);
                ok = false;
                break;
            };
            left[index] -= 1;
            took.push((index, 1));
            grid[slot] = Some(c.stack.clone());
        }
        if ok {
            match make(&grid) {
                Some(made) if session.crafted + made <= MAX_CRAFTED => {
                    session.crafted += made;
                    session.grids.push(grid);
                    for (index, n) in took.iter().copied() {
                        match session.taken.iter_mut().find(|(i, _)| *i == index) {
                            Some((_, have)) => *have += n,
                            None => session.taken.push((index, n)),
                        }
                    }
                    session.missing = None;
                    if repeat {
                        continue;
                    }
                }
                _ => ok = false,
            }
        }
        if !ok {
            // What this attempt took goes back: the counts are only worked out, nothing was removed.
            for (index, n) in took {
                left[index] += n;
            }
        }
        return session;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ingredient_becomes_the_right_ghost() {
        assert_eq!(ghost_for(&[]), Ghost::Empty);
        assert_eq!(ghost_for(&["minecraft:stick".into()]), Ghost::Item("minecraft:stick".into()));
        assert_eq!(ghost_for(&["#minecraft:planks".into()]), Ghost::Tag("minecraft:planks".into()));
        assert_eq!(
            ghost_for(&["minecraft:a".into(), "#minecraft:b".into()]),
            Ghost::List(vec![Ghost::Item("minecraft:a".into()), Ghost::Tag("minecraft:b".into())])
        );
        let many: Vec<String> = (0..19).map(|n| format!("minecraft:i{n}")).collect();
        assert_eq!(ghost_for(&many), Ghost::Empty);
    }

    #[test]
    fn a_shaped_recipe_fills_the_grid_from_the_top_left() {
        let pickaxe = vec![
            vec![Some('P'), Some('P'), Some('P')],
            vec![None, Some('S'), None],
            vec![None, Some('S'), None],
        ];
        let slots = shaped_slots(&pickaxe);
        assert_eq!(slots[0], Some('P'));
        assert_eq!(slots[3], None);
        assert_eq!(slots[4], Some('S'));
        assert_eq!(slots[7], Some('S'));
        // A 2 x 1 recipe sits in the first two cells of the top row.
        let two = shaped_slots(&[vec![Some(1), Some(2)]]);
        assert_eq!(two[..3], [Some(1), Some(2), None]);
        // Shapeless: one slot each.
        let loose = shapeless_slots(&["a", "b", "c"]);
        assert_eq!(loose[..4], [Some("a"), Some("b"), Some("c"), None]);
    }

    #[test]
    fn the_variant_cycles_through_the_recipes() {
        assert_eq!(next_variant(0, 3), 1);
        assert_eq!(next_variant(2, 3), 0);
        assert_eq!(next_variant(5, 0), 0);
        assert_eq!(next_variant(-1, 3), 0);
    }

    fn requests(items: [Option<&'static str>; GRID]) -> [Option<&'static str>; GRID] {
        items
    }

    fn carried(list: &[(&'static str, u32)]) -> Vec<Carried<&'static str>> {
        list.iter().map(|(stack, count)| Carried { stack: *stack, count: *count }).collect()
    }

    #[test]
    fn one_craft_takes_one_of_each_ingredient() {
        let wanted = requests([Some("plank"), None, None, Some("plank"), None, None, None, None, None]);
        let inventory = carried(&[("dirt", 5), ("plank", 3)]);
        let session = craft(&wanted, &inventory, false, &mut |want, have| want == have, &mut |grid| {
            (grid.iter().flatten().count() == 2).then_some(4)
        });
        assert_eq!(session.crafted, 4);
        assert_eq!(session.taken, vec![(1, 2)]);
        assert_eq!(session.grids.len(), 1);
        assert_eq!(session.missing, None);
    }

    #[test]
    fn sneaking_crafts_until_the_ingredients_run_out() {
        let wanted = requests([Some("plank"), Some("plank"), None, None, None, None, None, None, None]);
        let inventory = carried(&[("plank", 5)]);
        let session = craft(&wanted, &inventory, true, &mut |a, b| a == b, &mut |_| Some(1));
        // Two planks a craft: two crafts, the fifth plank is left over.
        assert_eq!(session.crafted, 2);
        assert_eq!(session.taken, vec![(0, 4)]);
        assert_eq!(session.missing, Some(1));
    }

    #[test]
    fn a_missing_ingredient_takes_nothing_and_says_which() {
        let wanted = requests([Some("plank"), Some("stick"), None, None, None, None, None, None, None]);
        let inventory = carried(&[("plank", 9)]);
        let session = craft(&wanted, &inventory, false, &mut |a, b| a == b, &mut |_| Some(1));
        assert_eq!(session.crafted, 0);
        assert!(session.taken.is_empty());
        assert_eq!(session.missing, Some(1));
    }

    #[test]
    fn a_grid_that_makes_nothing_takes_nothing() {
        let wanted = requests([Some("plank"), None, None, None, None, None, None, None, None]);
        let inventory = carried(&[("plank", 9)]);
        let session = craft(&wanted, &inventory, false, &mut |a, b| a == b, &mut |_| None);
        assert_eq!(session.crafted, 0);
        assert!(session.taken.is_empty());
    }

    #[test]
    fn sixty_four_items_at_most_come_out_of_one_click() {
        let wanted = requests([Some("log"), None, None, None, None, None, None, None, None]);
        let inventory = carried(&[("log", 100)]);
        let session = craft(&wanted, &inventory, true, &mut |a, b| a == b, &mut |_| Some(4));
        assert_eq!(session.crafted, 64);
        assert_eq!(session.grids.len(), 16);
        assert_eq!(session.taken, vec![(0, 16)]);
    }
}
