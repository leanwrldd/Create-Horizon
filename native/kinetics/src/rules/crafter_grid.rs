//! What a chain of Mechanical Crafters does with its items: `RecipeGridHandler` (`GroupedItems`,
//! `getAllCraftersOfChainIf`, `getTargetingCrafter`, `getPrecedingCrafters`) and the state machine
//! of `MechanicalCrafterBlockEntity` (`Phase`, `tick`, `begin`, `tryInsert`, `eject`), from Create
//! `mc1.21.1/dev` at 4148fd8 (MIT, the Creators of Create).
//!
//! No game types. The glue reads the crafters of a chain into a [`World`] (a snapshot, by
//! position), calls these functions with an [`Env`] for the few questions only the game can
//! answer (which recipe the grid makes, where the result may go), and writes back the crafters
//! the [`World`] says it changed. `S` is an item stack; an empty stack is `None` (a covered or
//! empty crafter's cell in a grid is `None`).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::crafter::{Pointing, count_down_speed, target_direction};
use crate::{Dir, Pos};

/// `MechanicalCrafterBlockEntity.Phase`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    Accepting,
    Assembling,
    Exporting,
    Waiting,
    Crafting,
    Inserting,
}

impl Phase {
    pub const ALL: [Phase; 7] = [
        Phase::Idle,
        Phase::Accepting,
        Phase::Assembling,
        Phase::Exporting,
        Phase::Waiting,
        Phase::Crafting,
        Phase::Inserting,
    ];

    /// `Enum.name`, as saved (`Phase`).
    pub const fn name(self) -> &'static str {
        match self {
            Phase::Idle => "IDLE",
            Phase::Accepting => "ACCEPTING",
            Phase::Assembling => "ASSEMBLING",
            Phase::Exporting => "EXPORTING",
            Phase::Waiting => "WAITING",
            Phase::Crafting => "CRAFTING",
            Phase::Inserting => "INSERTING",
        }
    }

    /// `read`: an unknown name is `IDLE`.
    pub fn from_name(name: &str) -> Self {
        Self::ALL.into_iter().find(|p| p.name() == name).unwrap_or_default()
    }
}

/// `RecipeGridHandler.GroupedItems`: the items a crafter holds for the grid, each in a cell
/// `(x, y)` relative to the crafter (x to the right, y upwards, seen from in front).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grid<S> {
    pub cells: BTreeMap<(i32, i32), Option<S>>,
}

impl<S> Default for Grid<S> {
    fn default() -> Self {
        Grid { cells: BTreeMap::new() }
    }
}

/// `calcStats`: the grid's bounds and size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
    pub width: i32,
    pub height: i32,
}

impl<S: Clone> Grid<S> {
    /// `new GroupedItems(stack)`: one cell at the origin.
    pub fn single(stack: Option<S>) -> Self {
        Grid { cells: BTreeMap::from([((0, 0), stack)]) }
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// `mergeOnto`: this grid's cells moved by `pointing`'s offset into `other`.
    pub fn merge_onto(&self, other: &mut Grid<S>, pointing: Pointing) {
        let (dx, dy) = pointing.merge_offset();
        for ((x, y), stack) in &self.cells {
            other.cells.insert((x + dx, y + dy), stack.clone());
        }
    }

    /// `onlyEmptyItems`.
    pub fn only_empty(&self) -> bool {
        self.cells.values().all(Option::is_none)
    }

    /// `calcStats`: the bounds always include the origin (they start at 0).
    pub fn stats(&self) -> Stats {
        let mut s = Stats::default();
        for &(x, y) in self.cells.keys() {
            s.min_x = s.min_x.min(x);
            s.min_y = s.min_y.min(y);
            s.max_x = s.max_x.max(x);
            s.max_y = s.max_y.max(y);
        }
        s.width = s.max_x - s.min_x + 1;
        s.height = s.max_y - s.min_y + 1;
        s
    }

    /// `MechanicalCraftingInput.of`: the grid cut to the box round its non-empty cells, row by
    /// row from the top (the highest y first): width, height and the cells.
    pub fn input(&self) -> (usize, usize, Vec<Option<S>>) {
        let st = self.stats();
        let mut bounds: Option<(i32, i32, i32, i32)> = None;
        for y in 0..st.height {
            for x in 0..st.width {
                let (xp, yp) = (x + st.min_x, y + st.min_y);
                if self.cells.get(&(xp, yp)).is_some_and(Option::is_some) {
                    let b = bounds.get_or_insert((xp, xp, yp, yp));
                    b.0 = b.0.min(xp);
                    b.1 = b.1.max(xp);
                    b.2 = b.2.min(yp);
                    b.3 = b.3.max(yp);
                }
            }
        }
        // Create's min/max start at the extremes, so an all-empty grid is `1 - (-1)`: a
        // nonsense size nothing matches.
        let Some((min_x, max_x, min_y, max_y)) = bounds else {
            return (0, 0, Vec::new());
        };
        let (w, h) = (max_x - min_x + 1, max_y - min_y + 1);
        let mut list = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                list.push(self.cells.get(&(x + min_x, max_y - y)).cloned().flatten());
            }
        }
        (w as usize, h as usize, list)
    }
}

/// One crafter, as the chain sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Crafter<S> {
    pub facing: Dir,
    pub pointing: Pointing,
    pub phase: Phase,
    pub count_down: i32,
    pub covered: bool,
    /// The one slot of its inventory (`inventory.getItem(0)`).
    pub slot: Option<S>,
    /// `groupedItems`.
    pub grid: Grid<S>,
    /// `groupedItemsBeforeCraft`: the grid as it was when it began to craft.
    pub before_craft: Grid<S>,
    /// The kinetic speed, in RPM.
    pub speed: f32,
    /// `wasPoweredBefore`.
    pub was_powered: bool,
}

impl<S> Crafter<S> {
    pub fn new(facing: Dir, pointing: Pointing) -> Self {
        Crafter {
            facing,
            pointing,
            phase: Phase::Idle,
            count_down: 0,
            covered: false,
            slot: None,
            grid: Grid::default(),
            before_craft: Grid::default(),
            speed: 0.0,
            // "Does not get serialized due to active checking in tick".
            was_powered: true,
        }
    }

    pub fn target_direction(&self) -> Dir {
        target_direction(self.facing, self.pointing)
    }

    /// `craftingItemPresent`.
    pub fn item_present(&self) -> bool {
        self.slot.is_some()
    }

    /// `craftingItemOrCoverPresent`.
    pub fn item_or_cover_present(&self) -> bool {
        self.slot.is_some() || self.covered
    }
}

/// A sound a crafter makes (`AllSoundEvents.CRAFTER_CLICK`, `CRAFTER_CRAFT`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sound {
    /// `CRAFTER_CLICK` at this pitch.
    Click(f32),
    Craft,
}

/// What a step does to the world besides the crafters' own state.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect<S> {
    /// `dropItem` at the front of the crafter at this position (`eject`).
    Drop(Pos, S),
    Sound(Pos, Sound),
}

/// The questions only the game answers.
pub trait Env<S> {
    /// `RecipeGridHandler.tryToApplyRecipe`: what the grid crafts, if anything.
    fn apply_recipe(&mut self, grid: &Grid<S>) -> Option<S>;
    /// `stack.getCraftingRemainingItem()`, for a stack that has one.
    fn remainder(&mut self, stack: &S) -> Option<S>;
    /// `inserting.hasInventory()`: whether the block the crafter at `pos` points at holds
    /// items (a chest, a basin, a crafter...).
    fn has_inventory(&mut self, pos: Pos) -> bool;
    /// `isTargetingBelt()`: whether the block pointed at takes items from a belt-like input.
    fn targeting_belt(&mut self, pos: Pos) -> bool;
    /// `behaviour.handleInsertion` / `inserting.insert`: gives `stack` to what the crafter at
    /// `pos` points at; what is left of it (`None`: all of it went in).
    fn insert(&mut self, pos: Pos, stack: S) -> Option<S>;
}

/// A snapshot of crafters by position. `changed` says which were modified, for the glue to
/// write back.
#[derive(Clone, Debug, Default)]
pub struct World<S> {
    pub crafters: BTreeMap<Pos, Crafter<S>>,
    pub changed: BTreeSet<Pos>,
}

fn step(pos: Pos, dir: Dir) -> Pos {
    let n = dir.normal();
    [pos[0] + n[0], pos[1] + n[1], pos[2] + n[2]]
}

impl<S: Clone> World<S> {
    pub fn new(crafters: BTreeMap<Pos, Crafter<S>>) -> Self {
        World { crafters, changed: BTreeSet::new() }
    }

    fn edit(&mut self, pos: Pos) -> Option<&mut Crafter<S>> {
        let crafter = self.crafters.get_mut(&pos)?;
        self.changed.insert(pos);
        Some(crafter)
    }

    /// `getTargetingCrafter`: the crafter this one passes its items to: the one in the direction
    /// it points, if it faces the same way.
    pub fn target_of(&self, pos: Pos) -> Option<Pos> {
        let crafter = self.crafters.get(&pos)?;
        let target = step(pos, crafter.target_direction());
        let other = self.crafters.get(&target)?;
        (other.facing == crafter.facing).then_some(target)
    }

    /// `getPrecedingCrafters`: the crafters that pass their items to this one: each beside it
    /// (not along its facing axis, not where it points) that faces the same way and points at it.
    pub fn preceding(&self, pos: Pos) -> Vec<Pos> {
        let Some(crafter) = self.crafters.get(&pos) else {
            return Vec::new();
        };
        let pointing = crafter.target_direction();
        Dir::ALL
            .into_iter()
            .filter(|d| d.axis() != crafter.facing.axis() && *d != pointing)
            .filter_map(|d| {
                let neighbour = step(pos, d);
                let other = self.crafters.get(&neighbour)?;
                (other.target_direction() == d.opposite() && other.facing == crafter.facing)
                    .then_some(neighbour)
            })
            .collect()
    }

    /// `getAllCraftersOfChainIf`: every crafter of the chain `root` is in, nearest first, or
    /// `None` when the chain loops, when some crafter fails `test` (unless `powered_start`), or
    /// when none passes it.
    pub fn chain_if(
        &self,
        root: Pos,
        test: &dyn Fn(&Crafter<S>) -> bool,
        powered_start: bool,
    ) -> Option<Vec<Pos>> {
        let mut crafters = Vec::new();
        let mut frontier: VecDeque<(Pos, Option<Pos>)> = VecDeque::from([(root, None)]);
        let mut visited = BTreeSet::new();
        let mut empty = false;
        let mut all_empty = true;
        while let Some((current, last)) = frontier.pop_front() {
            if visited.contains(&current) {
                return None;
            }
            let crafter = self.crafters.get(&current)?;
            if test(crafter) {
                all_empty = false;
            } else {
                empty = true;
            }
            crafters.push(current);
            visited.insert(current);
            let target = self.target_of(current);
            if target != last && let Some(target) = target {
                frontier.push_back((target, Some(current)));
            }
            for preceding in self.preceding(current) {
                if Some(preceding) != last {
                    frontier.push_back((preceding, Some(current)));
                }
            }
        }
        if (empty && !powered_start) || all_empty { None } else { Some(crafters) }
    }

    /// `getAllCraftersOfChain`.
    pub fn chain(&self, root: Pos) -> Option<Vec<Pos>> {
        self.chain_if(root, &|_| true, false)
    }

    /// `begin`: the crafter takes what its slot holds into its grid and waits for the ones before
    /// it (or starts, if none).
    fn begin(&mut self, pos: Pos) {
        let first = self.preceding(pos).is_empty();
        let Some(crafter) = self.edit(pos) else { return };
        crafter.phase = Phase::Accepting;
        crafter.grid = Grid::single(crafter.slot.take());
        if first {
            crafter.phase = Phase::Assembling;
            crafter.count_down = 1;
        }
    }

    /// `checkCompletedRecipe`: when every crafter of the chain holds an item or a cover (or, for
    /// a pulse of redstone, any does), they all begin. Whether they did. Does nothing for a
    /// crafter that isn't turning.
    pub fn check_completed_recipe(&mut self, root: Pos, powered_start: bool) -> bool {
        let Some(crafter) = self.crafters.get(&root) else {
            return false;
        };
        if crafter.speed == 0.0 {
            return false;
        }
        let chain = if powered_start {
            self.chain_if(root, &Crafter::item_present, true)
        } else {
            self.chain_if(root, &Crafter::item_or_cover_present, false)
        };
        let Some(chain) = chain else { return false };
        for pos in chain {
            self.begin(pos);
        }
        true
    }

    /// `continueIfAllPrecedingFinished`: once every crafter before this one has handed its items
    /// on (`WAITING`), it assembles.
    fn continue_if_all_preceding_finished(&mut self, pos: Pos) {
        let preceding = self.preceding(pos);
        if preceding.iter().any(|p| self.crafters.get(p).map(|c| c.phase) != Some(Phase::Waiting)) {
            return;
        }
        if let Some(crafter) = self.edit(pos) {
            crafter.phase = Phase::Assembling;
            crafter.count_down = 1;
        }
    }

    /// `eject` of one crafter: what it holds drops, and it is idle again.
    fn eject(&mut self, pos: Pos, effects: &mut Vec<Effect<S>>) {
        let Some(crafter) = self.edit(pos) else { return };
        for stack in crafter.grid.cells.values().flatten() {
            effects.push(Effect::Drop(pos, stack.clone()));
        }
        if let Some(stack) = crafter.slot.take() {
            effects.push(Effect::Drop(pos, stack));
        }
        crafter.phase = Phase::Idle;
        crafter.grid = Grid::default();
    }

    /// `ejectWholeGrid`: every crafter of the chain drops what it holds.
    pub fn eject_whole_grid(&mut self, pos: Pos) -> Vec<Effect<S>> {
        let mut effects = Vec::new();
        if let Some(chain) = self.chain(pos) {
            for member in chain {
                self.eject(member, &mut effects);
            }
        }
        effects
    }

    /// `tryInsert`: the crafter hands what it holds to what it points at, stack by stack; the
    /// rest waits (`INSERTING`), and it ejects everything when nothing is there to take it.
    fn try_insert(&mut self, pos: Pos, env: &mut dyn Env<S>, effects: &mut Vec<Effect<S>>) {
        if !env.has_inventory(pos) && !env.targeting_belt(pos) {
            effects.extend(self.eject_whole_grid(pos));
            return;
        }
        let Some(crafter) = self.crafters.get(&pos) else { return };
        let cells: Vec<((i32, i32), Option<S>)> =
            crafter.grid.cells.iter().map(|(k, v)| (*k, v.clone())).collect();
        let mut left: BTreeMap<(i32, i32), S> = BTreeMap::new();
        let mut inserted = Vec::new();
        for (cell, stack) in cells {
            match stack {
                // An empty stack goes in as nothing.
                None => inserted.push(cell),
                Some(stack) => match env.insert(pos, stack) {
                    Some(rest) => {
                        left.insert(cell, rest);
                    }
                    None => inserted.push(cell),
                },
            }
        }
        let Some(crafter) = self.edit(pos) else { return };
        for cell in &inserted {
            crafter.grid.cells.remove(cell);
        }
        for (cell, rest) in left {
            crafter.grid.cells.insert(cell, Some(rest));
        }
        if crafter.grid.is_empty() {
            effects.extend(self.eject_whole_grid(pos));
        } else {
            crafter.phase = Phase::Inserting;
        }
    }

    /// `lazyTick` (every 20 ticks): an idle crafter holding an item looks again whether the
    /// chain is full; one that couldn't hand its items on tries again.
    pub fn lazy_tick(&mut self, pos: Pos, env: &mut dyn Env<S>) -> Vec<Effect<S>> {
        let mut effects = Vec::new();
        let Some(crafter) = self.crafters.get(&pos) else {
            return effects;
        };
        let (phase, present) = (crafter.phase, crafter.item_present());
        if phase == Phase::Idle && present {
            self.check_completed_recipe(pos, false);
        }
        if phase == Phase::Inserting {
            self.try_insert(pos, env, &mut effects);
        }
        effects
    }

    /// `MechanicalCrafterBlockEntity.tick` for the crafter at `pos` (after the kinetic tick):
    /// `powered` is whether redstone reaches it (`hasNeighborSignal`). Only the crafter at `pos`
    /// and the ones it passes items to or takes from change; the glue reads those.
    pub fn tick(&mut self, pos: Pos, powered: bool, env: &mut dyn Env<S>) -> Vec<Effect<S>> {
        let mut effects = Vec::new();
        let Some(crafter) = self.crafters.get(&pos) else {
            return effects;
        };
        if crafter.phase == Phase::Accepting {
            return effects;
        }
        if crafter.was_powered != powered {
            if let Some(crafter) = self.edit(pos) {
                crafter.was_powered = powered;
            }
            if powered {
                self.check_completed_recipe(pos, true);
            }
        }
        let speed = count_down_speed(self.crafters[&pos].speed);

        if self.crafters[&pos].phase == Phase::Assembling {
            let finished = self.count_down(pos, speed);
            if finished {
                if self.target_of(pos).is_some() {
                    let empty = self.crafters[&pos].grid.only_empty();
                    if let Some(c) = self.edit(pos) {
                        c.phase = Phase::Exporting;
                        c.count_down = if empty { 0 } else { 1000 };
                    }
                    return effects;
                }
                let grid = self.crafters[&pos].grid.clone();
                if let Some(result) = env.apply_recipe(&grid) {
                    let mut containers = Vec::new();
                    for stack in grid.cells.values().flatten() {
                        if let Some(container) = env.remainder(stack) {
                            containers.push(container);
                        }
                    }
                    let mut made = Grid::single(Some(result));
                    for (i, container) in containers.into_iter().enumerate() {
                        let mut one = Grid::default();
                        one.cells.insert((i as i32, 0), Some(container));
                        one.merge_onto(&mut made, Pointing::Left);
                    }
                    if let Some(c) = self.edit(pos) {
                        c.before_craft = grid;
                        c.grid = made;
                        c.phase = Phase::Crafting;
                        c.count_down = 2000;
                    }
                    return effects;
                }
                effects.extend(self.eject_whole_grid(pos));
                return effects;
            }
        }

        if self.crafters[&pos].phase == Phase::Exporting {
            let finished = self.count_down(pos, speed);
            if finished {
                let Some(target) = self.target_of(pos) else {
                    effects.extend(self.eject_whole_grid(pos));
                    return effects;
                };
                let me = &self.crafters[&pos];
                let (empty, pointing, grid) = (me.grid.only_empty(), me.pointing, me.grid.clone());
                let size = if let Some(t) = self.edit(target) {
                    grid.merge_onto(&mut t.grid, pointing);
                    t.grid.cells.len()
                } else {
                    0
                };
                if let Some(c) = self.edit(pos) {
                    c.grid = Grid::default();
                    c.phase = Phase::Waiting;
                    c.count_down = 0;
                }
                if !empty {
                    effects.push(Effect::Sound(pos, Sound::Click(size as f32 / 16.0 + 0.5)));
                }
                self.continue_if_all_preceding_finished(target);
                return effects;
            }
        }

        if self.crafters[&pos].phase == Phase::Crafting {
            let before = self.crafters[&pos].count_down;
            let finished = self.count_down(pos, speed);
            let now = self.crafters[&pos].count_down;
            if now < 1000 && before >= 1000 {
                effects.push(Effect::Sound(pos, Sound::Click(2.0)));
                effects.push(Effect::Sound(pos, Sound::Craft));
            }
            if finished {
                self.try_insert(pos, env, &mut effects);
                return effects;
            }
        }

        if self.crafters[&pos].phase == Phase::Inserting && env.targeting_belt(pos) {
            self.try_insert(pos, env, &mut effects);
        }
        effects
    }

    /// `countDown -= getCountDownSpeed()`; when it went below 0 it is set to 0 and the step is
    /// done (true).
    fn count_down(&mut self, pos: Pos, speed: i32) -> bool {
        let Some(c) = self.edit(pos) else { return false };
        c.count_down -= speed;
        if c.count_down < 0 {
            c.count_down = 0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Stack = &'static str;

    /// A test game: a recipe that makes "result" of exactly two cells ("a" over "b"), and a
    /// target that takes everything up to `room` stacks.
    struct Game {
        recipe_calls: usize,
        room: usize,
        inserted: Vec<Stack>,
        inventory: bool,
    }

    impl Game {
        fn new() -> Self {
            Game { recipe_calls: 0, room: 99, inserted: Vec::new(), inventory: true }
        }
    }

    impl Env<Stack> for Game {
        fn apply_recipe(&mut self, grid: &Grid<Stack>) -> Option<Stack> {
            self.recipe_calls += 1;
            let (w, h, cells) = grid.input();
            (w == 1 && h == 2 && cells == vec![Some("a"), Some("b")]).then_some("result")
        }
        fn remainder(&mut self, stack: &Stack) -> Option<Stack> {
            (*stack == "milk").then_some("bucket")
        }
        fn has_inventory(&mut self, _: Pos) -> bool {
            self.inventory
        }
        fn targeting_belt(&mut self, _: Pos) -> bool {
            false
        }
        fn insert(&mut self, _: Pos, stack: Stack) -> Option<Stack> {
            if self.inserted.len() < self.room {
                self.inserted.push(stack);
                None
            } else {
                Some(stack)
            }
        }
    }

    /// Two crafters facing south, the lower one (y 0) pointing up at the upper (y 1), which
    /// points at nothing (a chest behind it, east).
    fn pair() -> World<Stack> {
        let mut lower = Crafter::new(Dir::South, Pointing::Up);
        lower.speed = 16.0;
        let mut upper = Crafter::new(Dir::South, Pointing::Right);
        upper.speed = 16.0;
        World::new(BTreeMap::from([([0, 0, 0], lower), ([0, 1, 0], upper)]))
    }

    fn run(world: &mut World<Stack>, env: &mut Game, ticks: usize) -> Vec<Effect<Stack>> {
        let mut out = Vec::new();
        for _ in 0..ticks {
            for pos in [[0, 0, 0], [0, 1, 0]] {
                out.extend(world.tick(pos, false, env));
            }
        }
        out
    }

    #[test]
    fn the_chain_is_found_from_either_end() {
        let w = pair();
        assert_eq!(w.target_of([0, 0, 0]), Some([0, 1, 0]));
        assert_eq!(w.target_of([0, 1, 0]), None);
        assert_eq!(w.preceding([0, 1, 0]), vec![[0, 0, 0]]);
        assert!(w.preceding([0, 0, 0]).is_empty());
        let mut chain = w.chain([0, 1, 0]).unwrap();
        chain.sort();
        assert_eq!(chain, vec![[0, 0, 0], [0, 1, 0]]);
        assert_eq!(w.chain([0, 0, 0]).unwrap().len(), 2);
    }

    #[test]
    fn a_crafter_facing_another_way_is_not_in_the_chain() {
        let mut w = pair();
        w.crafters.get_mut(&[0, 1, 0]).unwrap().facing = Dir::East;
        assert_eq!(w.target_of([0, 0, 0]), None);
        assert!(w.preceding([0, 1, 0]).is_empty());
    }

    #[test]
    fn nothing_starts_until_every_crafter_holds_an_item() {
        let mut w = pair();
        w.crafters.get_mut(&[0, 1, 0]).unwrap().slot = Some("a");
        assert!(!w.check_completed_recipe([0, 1, 0], false));
        w.crafters.get_mut(&[0, 0, 0]).unwrap().slot = Some("b");
        assert!(w.check_completed_recipe([0, 1, 0], false));
        // The one with no one before it assembles; the other waits to accept.
        assert_eq!(w.crafters[&[0, 0, 0]].phase, Phase::Assembling);
        assert_eq!(w.crafters[&[0, 1, 0]].phase, Phase::Accepting);
        assert_eq!(w.crafters[&[0, 0, 0]].count_down, 1);
        assert!(w.crafters[&[0, 0, 0]].slot.is_none());
    }

    #[test]
    fn a_covered_crafter_counts_as_full_and_contributes_nothing() {
        let mut w = pair();
        w.crafters.get_mut(&[0, 1, 0]).unwrap().slot = Some("a");
        w.crafters.get_mut(&[0, 0, 0]).unwrap().covered = true;
        assert!(w.check_completed_recipe([0, 1, 0], false));
        assert_eq!(w.crafters[&[0, 0, 0]].grid.cells[&(0, 0)], None);
    }

    #[test]
    fn a_pulse_starts_a_partly_filled_grid() {
        let mut w = pair();
        w.crafters.get_mut(&[0, 1, 0]).unwrap().slot = Some("a");
        assert!(w.check_completed_recipe([0, 1, 0], true));
        // But not an empty one, and not a stopped one.
        let mut w = pair();
        assert!(!w.check_completed_recipe([0, 1, 0], true));
        let mut w = pair();
        w.crafters.get_mut(&[0, 1, 0]).unwrap().slot = Some("a");
        w.crafters.get_mut(&[0, 1, 0]).unwrap().speed = 0.0;
        assert!(!w.check_completed_recipe([0, 1, 0], true));
    }

    #[test]
    fn a_full_grid_passes_its_items_along_crafts_and_hands_the_result_on() {
        let mut w = pair();
        w.crafters.get_mut(&[0, 1, 0]).unwrap().slot = Some("a");
        w.crafters.get_mut(&[0, 0, 0]).unwrap().slot = Some("b");
        assert!(w.check_completed_recipe([0, 1, 0], false));
        let mut game = Game::new();
        let effects = run(&mut w, &mut game, 400);
        // The lower crafter's "b" went to the upper at (0, -1): below "a".
        assert_eq!(game.recipe_calls, 1);
        assert_eq!(game.inserted, vec!["result"]);
        assert!(effects.iter().any(|e| matches!(e, Effect::Sound(_, Sound::Craft))));
        assert!(effects.iter().any(|e| matches!(e, Effect::Sound(_, Sound::Click(_)))));
        assert!(effects.iter().all(|e| !matches!(e, Effect::Drop(..))));
        for c in w.crafters.values() {
            assert_eq!(c.phase, Phase::Idle, "ejected the whole grid, so idle again");
            assert!(c.grid.is_empty());
        }
    }

    #[test]
    fn a_wrong_grid_is_thrown_back() {
        let mut w = pair();
        w.crafters.get_mut(&[0, 1, 0]).unwrap().slot = Some("b");
        w.crafters.get_mut(&[0, 0, 0]).unwrap().slot = Some("a");
        w.check_completed_recipe([0, 1, 0], false);
        let mut game = Game::new();
        let effects = run(&mut w, &mut game, 400);
        let mut dropped: Vec<Stack> = effects
            .iter()
            .filter_map(|e| match e {
                Effect::Drop(_, s) => Some(*s),
                _ => None,
            })
            .collect();
        dropped.sort_unstable();
        assert_eq!(dropped, vec!["a", "b"]);
        assert!(game.inserted.is_empty());
    }

    #[test]
    fn what_has_a_container_leaves_it_beside_the_result() {
        struct Single;
        impl Env<Stack> for Single {
            fn apply_recipe(&mut self, _: &Grid<Stack>) -> Option<Stack> {
                Some("cake")
            }
            fn remainder(&mut self, s: &Stack) -> Option<Stack> {
                (*s == "milk").then_some("bucket")
            }
            fn has_inventory(&mut self, _: Pos) -> bool {
                true
            }
            fn targeting_belt(&mut self, _: Pos) -> bool {
                false
            }
            fn insert(&mut self, _: Pos, _: Stack) -> Option<Stack> {
                None
            }
        }
        let mut w = World::new(BTreeMap::from([(
            [0, 0, 0],
            Crafter { speed: 16.0, slot: Some("milk"), ..Crafter::new(Dir::South, Pointing::Right) },
        )]));
        assert!(w.check_completed_recipe([0, 0, 0], false));
        w.tick([0, 0, 0], false, &mut Single);
        let c = &w.crafters[&[0, 0, 0]];
        assert_eq!(c.phase, Phase::Crafting);
        assert_eq!(c.grid.cells[&(0, 0)], Some("cake"));
        assert_eq!(c.grid.cells[&(1, 0)], Some("bucket"));
        assert_eq!(c.before_craft.cells[&(0, 0)], Some("milk"));
        assert_eq!(c.count_down, 2000);
    }

    #[test]
    fn a_full_target_keeps_the_rest_and_waits_inserting() {
        let mut w = pair();
        w.crafters.get_mut(&[0, 1, 0]).unwrap().slot = Some("a");
        w.crafters.get_mut(&[0, 0, 0]).unwrap().slot = Some("b");
        w.check_completed_recipe([0, 1, 0], false);
        let mut game = Game::new();
        game.room = 0;
        run(&mut w, &mut game, 400);
        let upper = &w.crafters[&[0, 1, 0]];
        assert_eq!(upper.phase, Phase::Inserting);
        assert_eq!(upper.grid.cells[&(0, 0)], Some("result"));
        // Room appears: the lazy tick hands it on and the grid is idle again.
        game.room = 9;
        w.lazy_tick([0, 1, 0], &mut game);
        assert_eq!(game.inserted, vec!["result"]);
        assert_eq!(w.crafters[&[0, 1, 0]].phase, Phase::Idle);
    }

    #[test]
    fn with_nothing_to_take_the_result_everything_drops() {
        let mut w = pair();
        w.crafters.get_mut(&[0, 1, 0]).unwrap().slot = Some("a");
        w.crafters.get_mut(&[0, 0, 0]).unwrap().slot = Some("b");
        w.check_completed_recipe([0, 1, 0], false);
        let mut game = Game::new();
        game.inventory = false;
        let effects = run(&mut w, &mut game, 400);
        assert!(effects.contains(&Effect::Drop([0, 1, 0], "result")));
        assert_eq!(w.crafters[&[0, 1, 0]].phase, Phase::Idle);
    }

    #[test]
    fn a_stopped_crafter_does_not_count_down() {
        let mut w = pair();
        w.crafters.get_mut(&[0, 1, 0]).unwrap().slot = Some("a");
        w.crafters.get_mut(&[0, 0, 0]).unwrap().slot = Some("b");
        w.check_completed_recipe([0, 1, 0], false);
        w.crafters.get_mut(&[0, 0, 0]).unwrap().speed = 0.0;
        let mut game = Game::new();
        w.tick([0, 0, 0], false, &mut game);
        // `countDown` was 1 and is not reduced by 0: not below 0, so still assembling.
        assert_eq!(w.crafters[&[0, 0, 0]].phase, Phase::Assembling);
    }

    #[test]
    fn a_pulse_of_redstone_starts_the_chain() {
        let mut w = pair();
        w.crafters.get_mut(&[0, 1, 0]).unwrap().slot = Some("a");
        let mut game = Game::new();
        // `wasPoweredBefore` starts true: an unpowered tick only notes it is off.
        w.tick([0, 1, 0], false, &mut game);
        assert_eq!(w.crafters[&[0, 1, 0]].phase, Phase::Idle);
        assert!(!w.crafters[&[0, 1, 0]].was_powered);
        w.tick([0, 1, 0], true, &mut game);
        assert_eq!(w.crafters[&[0, 1, 0]].phase, Phase::Accepting);
        assert_eq!(w.crafters[&[0, 0, 0]].phase, Phase::Assembling);
    }

    #[test]
    fn the_grid_is_cut_to_its_items_row_by_row_from_the_top() {
        let mut g: Grid<Stack> = Grid::default();
        g.cells.insert((0, 0), Some("bottom"));
        g.cells.insert((0, 1), Some("top"));
        g.cells.insert((1, 1), None);
        let (w, h, cells) = g.input();
        assert_eq!((w, h), (1, 2));
        assert_eq!(cells, vec![Some("top"), Some("bottom")]);
        let mut empty: Grid<Stack> = Grid::default();
        empty.cells.insert((0, 0), None);
        assert_eq!(empty.input().2.len(), 0);
    }

    #[test]
    fn merging_shifts_by_the_pointing() {
        let mut a: Grid<Stack> = Grid::single(Some("x"));
        let mut b: Grid<Stack> = Grid::default();
        a.merge_onto(&mut b, Pointing::Up);
        assert_eq!(b.cells[&(0, -1)], Some("x"));
        let mut c = Grid::default();
        a.merge_onto(&mut c, Pointing::Left);
        assert_eq!(c.cells[&(1, 0)], Some("x"));
        a.cells.clear();
    }
}
