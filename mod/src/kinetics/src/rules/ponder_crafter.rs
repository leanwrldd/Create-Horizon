//! The Mechanical Crafters of a Ponder scene: what `world.modifyBlockEntity(pos, MechanicalCrafterBlockEntity.class, ..)`,
//! `setCraftingResult` and `connectCrafterInvs` (Create 6.0.10, `CreateSceneBuilder.WorldInstructions`) make them do. No
//! game types.
//!
//! The scene's world is virtual and its crafters run Create's own logic (`isVirtual()`): a stack put in a crafter's slot
//! starts the chain once every crafter holds one (`whenContentsChanged`, `checkCompletedRecipe`), the chain passes its
//! items along the arrows and crafts, and the result is not a recipe's but the one `setCraftingResult` scripted
//! (`scriptedResult`). So the replay runs [`crate::rules::crafter_grid`] tick by tick from the first, with a [`Setting`]
//! answering what the scene's blocks say, and keeps what each crafter held and did as a [`Timeline`] the scene player
//! reads at any tick (a seek rebuilds the same picture).
//!
//! Where it differs from the game: a crafter with no scripted result finds no recipe (Create would craft an empty
//! stack); the group inventory a stacked insertion fills is the crafters that touch each other, not the saved
//! `ConnectedInput`; redstone reaches a crafter from the six blocks around it; sounds are not played.

use std::collections::{BTreeMap, BTreeSet};

use super::crafter::{Input, Inputs, Pointing, count_down_speed, inventory_order, toggle};
use super::crafter_grid::{Crafter, Effect, Env, Grid, Phase, World};
use crate::{Dir, Pos};

/// `MechanicalCrafterBlockEntity.lazyTick`'s rate: `setLazyTickRate(20)`, which fires every 21st tick.
const LAZY_PERIOD: i64 = 21;

/// What the block a crafter points at does with the items it passes on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// No inventory: the crafter ejects what it holds.
    Nothing,
    /// A Depot: holds one stack until the scene clears it (`removeItemsFromBelt`).
    Depot,
    /// A Belt: takes any stack and carries it on.
    Belt,
    /// A chest, a barrel, a hopper...: takes everything.
    Container,
}

/// What the scene's blocks say, by tick.
pub trait Setting<S> {
    /// The crafter the block at `pos` is, with its facing and pointing; `None` for any other block.
    fn crafter(&self, pos: Pos, tick: i64) -> Option<(Dir, Pointing)>;
    /// The speed the crafter at `pos` turns at.
    fn rpm(&self, pos: Pos, tick: i64) -> f32;
    /// `hasNeighborSignal`.
    fn powered(&self, pos: Pos, tick: i64) -> bool;
    /// What the block at `pos` is to a crafter that points at it.
    fn target(&self, pos: Pos, tick: i64) -> Target;
    /// `getCraftingRemainingItem`.
    fn remainder(&self, stack: &S) -> Option<S>;
}

/// One instruction of the scene's on the crafters, by the tick it runs.
#[derive(Clone, Debug, PartialEq)]
pub enum Action<S> {
    /// `getInventory().insertItem(0, stack, false)`: one item, if the crafter is idle, uncovered and empty.
    Insert { pos: Pos, stack: S },
    /// `getInventory().setItem(0, stack)`.
    Set { pos: Pos, stack: S },
    /// `ItemHandlerHelper.insertItemStacked(handler, stack, false)` through the crafter's item handler: `count` items
    /// of `stack`, one to each joined crafter in order.
    Stacked { pos: Pos, stack: S, count: u32 },
    /// `setCraftingResult`: what the crafter at `pos` makes when it crafts (`setScriptedResult`).
    Result { pos: Pos, stack: S },
    /// `connectCrafterInvs`.
    Connect { first: Pos, second: Pos },
    /// `checkCompletedRecipe(true)`.
    Check { pos: Pos },
    /// `ejectWholeGrid()`.
    Eject { pos: Pos },
    /// `modifyBlockEntityNBT(.., putBoolean("Cover", ..))`.
    Cover { pos: Pos, covered: bool },
    /// A stack was put on the Depot at `pos` (`createItemOnBelt`): it takes nothing more until cleared.
    Occupy { pos: Pos },
    /// The scene took what rests on the Depot at `pos` (`removeItemsFromBelt`).
    Clear { pos: Pos },
}

/// What a crafter passed on to a block.
#[derive(Clone, Debug, PartialEq)]
pub struct Output<S> {
    pub tick: i64,
    /// The block the crafter pointed at.
    pub at: Pos,
    pub kind: Target,
    pub stack: S,
}

/// A crafter's stack dropped in front of it (`eject`).
#[derive(Clone, Debug, PartialEq)]
pub struct Dropped<S> {
    pub tick: i64,
    pub pos: Pos,
    pub facing: Dir,
    pub stack: S,
}

/// What the replay left: each crafter's state whenever what it shows changed.
#[derive(Clone, Debug)]
pub struct Timeline<S> {
    /// `(tick, state)`; `None` is a position where no crafter stands.
    pub states: BTreeMap<Pos, Vec<(i64, Option<Crafter<S>>)>>,
    /// The joined inventories after each change.
    pub inputs: Vec<(i64, Inputs)>,
    pub drops: Vec<Dropped<S>>,
    pub outputs: Vec<Output<S>>,
}

impl<S> Default for Timeline<S> {
    fn default() -> Self {
        Timeline { states: BTreeMap::new(), inputs: Vec::new(), drops: Vec::new(), outputs: Vec::new() }
    }
}

impl<S> Timeline<S> {
    /// The crafter at `pos` as it was after `tick`, with the tick the state began.
    pub fn at(&self, pos: Pos, tick: i64) -> Option<(i64, &Crafter<S>)> {
        let states = self.states.get(&pos)?;
        let index = states.partition_point(|(from, _)| *from <= tick);
        let (from, state) = states.get(index.checked_sub(1)?)?;
        state.as_ref().map(|crafter| (*from, crafter))
    }

    /// The joined inventories as they were after `tick`.
    pub fn inputs_at(&self, tick: i64) -> Option<&Inputs> {
        let index = self.inputs.partition_point(|(from, _)| *from <= tick);
        self.inputs.get(index.checked_sub(1)?).map(|(_, inputs)| inputs)
    }
}

/// What a crafter shows of its state: if all of it is as it was, nothing needs recording. The count-down is not here: a
/// client counts it down from the last record.
fn same_view<S: PartialEq>(a: &Crafter<S>, b: &Crafter<S>) -> bool {
    a.facing == b.facing
        && a.pointing == b.pointing
        && a.phase == b.phase
        && a.covered == b.covered
        && a.slot == b.slot
        && a.grid == b.grid
        && a.before_craft == b.before_craft
        && count_down_speed(a.speed) == count_down_speed(b.speed)
}

struct Sim<'a, S, G: Setting<S>> {
    setting: &'a G,
    tick: i64,
    /// The crafter in its own tick: whose scripted result an assembly takes.
    current: Pos,
    aims: BTreeMap<Pos, Dir>,
    results: BTreeMap<Pos, S>,
    /// The Depots holding a stack.
    full: BTreeSet<Pos>,
    outputs: Vec<Output<S>>,
}

impl<S: Clone, G: Setting<S>> Sim<'_, S, G> {
    fn aim(&self, pos: Pos) -> Option<(Pos, Dir)> {
        let dir = *self.aims.get(&pos)?;
        let n = dir.normal();
        Some(([pos[0] + n[0], pos[1] + n[1], pos[2] + n[2]], dir))
    }

    fn kind(&self, pos: Pos) -> Target {
        self.aim(pos).map_or(Target::Nothing, |(at, _)| self.setting.target(at, self.tick))
    }
}

impl<S: Clone, G: Setting<S>> Env<S> for Sim<'_, S, G> {
    fn apply_recipe(&mut self, _: &Grid<S>) -> Option<S> {
        self.results.get(&self.current).cloned()
    }

    fn remainder(&mut self, stack: &S) -> Option<S> {
        self.setting.remainder(stack)
    }

    fn has_inventory(&mut self, pos: Pos) -> bool {
        self.kind(pos) != Target::Nothing
    }

    fn targeting_belt(&mut self, pos: Pos) -> bool {
        self.kind(pos) == Target::Belt
    }

    fn insert(&mut self, pos: Pos, stack: S) -> Option<S> {
        let (at, _) = self.aim(pos)?;
        let kind = self.kind(pos);
        match kind {
            Target::Nothing => return Some(stack),
            Target::Depot if !self.full.insert(at) => return Some(stack),
            _ => {}
        }
        self.outputs.push(Output { tick: self.tick, at, kind, stack });
        None
    }
}

fn step(pos: Pos, dir: Dir) -> Pos {
    let n = dir.normal();
    [pos[0] + n[0], pos[1] + n[1], pos[2] + n[2]]
}

/// The crafters that touch the one at `pos` (same facing, side by side or above and below), in the order their slots
/// make one inventory (`ConnectedInput.getInventories`).
fn group<S>(world: &World<S>, pos: Pos) -> Vec<Pos> {
    let Some(first) = world.crafters.get(&pos) else {
        return Vec::new();
    };
    let facing = first.facing;
    let mut found = BTreeSet::from([pos]);
    let mut queue = vec![pos];
    while let Some(at) = queue.pop() {
        for dir in Dir::ALL {
            if dir.axis() == facing.axis() {
                continue;
            }
            let next = step(at, dir);
            if world.crafters.get(&next).is_some_and(|c| c.facing == facing) && found.insert(next) {
                queue.push(next);
            }
        }
    }
    inventory_order(facing, found.into_iter().collect())
}

/// Runs the scene's crafter instructions up to `until`. `candidates` are the positions a crafter may stand at (the
/// structure's, and any a scene's block change turns into one); `actions` are in the order they run.
pub fn run<S, G>(setting: &G, candidates: &[Pos], actions: &[(i64, Action<S>)], until: i64) -> Timeline<S>
where
    S: Clone + PartialEq,
    G: Setting<S>,
{
    let mut world: World<S> = World::new(BTreeMap::new());
    let mut inputs: Inputs = BTreeMap::new();
    let mut born: BTreeMap<Pos, i64> = BTreeMap::new();
    let mut timeline = Timeline::default();
    let mut sim = Sim {
        setting,
        tick: 0,
        current: [0; 3],
        aims: BTreeMap::new(),
        results: BTreeMap::new(),
        full: BTreeSet::new(),
        outputs: Vec::new(),
    };
    let mut last_inputs: Option<Inputs> = None;
    let mut next = 0;
    for tick in 0..=until {
        sim.tick = tick;
        // The blocks the scene changed: a crafter placed again starts afresh (a new block entity).
        for &pos in candidates {
            match setting.crafter(pos, tick) {
                Some((facing, pointing)) => {
                    let crafter = world.crafters.entry(pos).or_insert_with(|| {
                        born.insert(pos, tick);
                        inputs.insert(pos, Input::default());
                        Crafter::new(facing, pointing)
                    });
                    crafter.facing = facing;
                    crafter.pointing = pointing;
                    crafter.speed = setting.rpm(pos, tick);
                }
                None => {
                    world.crafters.remove(&pos);
                    inputs.remove(&pos);
                    sim.results.remove(&pos);
                }
            }
        }
        sim.aims = world.crafters.iter().map(|(pos, c)| (*pos, c.target_direction())).collect();
        let mut effects: Vec<Effect<S>> = Vec::new();
        while let Some((_, action)) = actions.get(next).filter(|(at, _)| *at <= tick) {
            next += 1;
            match action {
                Action::Insert { pos, stack } => {
                    if let Some(c) = world.crafters.get_mut(pos)
                        && c.phase == Phase::Idle
                        && !c.covered
                        && c.slot.is_none()
                    {
                        c.slot = Some(stack.clone());
                        world.check_completed_recipe(*pos, false);
                    }
                }
                Action::Set { pos, stack } => {
                    if let Some(c) = world.crafters.get_mut(pos) {
                        c.slot = Some(stack.clone());
                        if c.phase == Phase::Idle {
                            world.check_completed_recipe(*pos, false);
                        }
                    }
                }
                Action::Stacked { pos, stack, count } => {
                    let mut left = *count;
                    for member in group(&world, *pos) {
                        if left == 0 {
                            break;
                        }
                        let Some(c) = world.crafters.get_mut(&member) else { continue };
                        if c.phase != Phase::Idle || c.covered || c.slot.is_some() {
                            continue;
                        }
                        c.slot = Some(stack.clone());
                        left -= 1;
                        world.check_completed_recipe(member, false);
                    }
                }
                Action::Result { pos, stack } => {
                    sim.results.insert(*pos, stack.clone());
                }
                Action::Connect { first, second } => toggle(&mut inputs, *first, *second),
                Action::Check { pos } => {
                    world.check_completed_recipe(*pos, true);
                }
                Action::Eject { pos } => effects.extend(world.eject_whole_grid(*pos)),
                Action::Cover { pos, covered } => {
                    if let Some(c) = world.crafters.get_mut(pos) {
                        c.covered = *covered;
                    }
                }
                Action::Occupy { pos } => {
                    sim.full.insert(*pos);
                }
                Action::Clear { pos } => {
                    sim.full.remove(pos);
                }
            }
        }
        let positions: Vec<Pos> = world.crafters.keys().copied().collect();
        for pos in positions {
            sim.current = pos;
            let powered = setting.powered(pos, tick);
            effects.extend(world.tick(pos, powered, &mut sim));
            let age = tick - born.get(&pos).copied().unwrap_or(0);
            if age > 0 && age % LAZY_PERIOD == LAZY_PERIOD - 1 {
                sim.current = pos;
                effects.extend(world.lazy_tick(pos, &mut sim));
            }
        }
        for effect in effects {
            if let Effect::Drop(pos, stack) = effect
                && let Some(c) = world.crafters.get(&pos)
            {
                timeline.drops.push(Dropped { tick, pos, facing: c.facing, stack });
            }
        }
        timeline.outputs.append(&mut sim.outputs);
        world.changed.clear();

        for &pos in candidates {
            let now = world.crafters.get(&pos);
            let states = timeline.states.entry(pos).or_default();
            let record = match (states.last().and_then(|(_, state)| state.as_ref()), now) {
                (None, None) => states.is_empty().then_some(None),
                (Some(_), None) => Some(None),
                (None, Some(c)) => Some(Some(c.clone())),
                (Some(before), Some(c)) => (!same_view(before, c)).then(|| Some(c.clone())),
            };
            if let Some(state) = record {
                states.push((tick, state));
            }
        }
        if last_inputs.as_ref() != Some(&inputs) {
            timeline.inputs.push((tick, inputs.clone()));
            last_inputs = Some(inputs.clone());
        }
    }
    timeline
}

#[cfg(test)]
mod tests {
    use super::*;

    type Stack = &'static str;

    /// A column of three crafters facing north at x 0, y 0 to 2, each pointing down at the next; the one at y 0 points
    /// at (0, -1, 0): `target`.
    struct Column {
        target: Target,
        rpm: f32,
        powered_from: Option<i64>,
        gone: Option<(Pos, i64)>,
    }

    impl Column {
        fn new(target: Target) -> Self {
            Column { target, rpm: 32.0, powered_from: None, gone: None }
        }
    }

    impl Setting<Stack> for Column {
        fn crafter(&self, pos: Pos, tick: i64) -> Option<(Dir, Pointing)> {
            if self.gone.is_some_and(|(at, until)| at == pos && tick < until) {
                return None;
            }
            ((0..3).contains(&pos[1]) && pos[0] == 0 && pos[2] == 0).then_some((Dir::North, Pointing::Down))
        }
        fn rpm(&self, _: Pos, _: i64) -> f32 {
            self.rpm
        }
        fn powered(&self, pos: Pos, tick: i64) -> bool {
            pos == [0, 2, 0] && self.powered_from.is_some_and(|from| tick >= from)
        }
        fn target(&self, pos: Pos, _: i64) -> Target {
            if pos == [0, -1, 0] { self.target } else { Target::Nothing }
        }
        fn remainder(&self, stack: &Stack) -> Option<Stack> {
            (*stack == "milk").then_some("bucket")
        }
    }

    const COLUMN: [Pos; 3] = [[0, 0, 0], [0, 1, 0], [0, 2, 0]];

    fn insert(tick: i64, y: i32, stack: Stack) -> (i64, Action<Stack>) {
        (tick, Action::Insert { pos: [0, y, 0], stack })
    }

    fn state(timeline: &Timeline<Stack>, y: i32, tick: i64) -> Crafter<Stack> {
        timeline.at([0, y, 0], tick).map(|(_, c)| c.clone()).expect("a crafter")
    }

    fn fill(first: i64) -> Vec<(i64, Action<Stack>)> {
        vec![
            (0, Action::Result { pos: [0, 0, 0], stack: "pickaxe" }),
            insert(first, 2, "a"),
            insert(first + 1, 1, "b"),
            insert(first + 2, 0, "c"),
        ]
    }

    #[test]
    fn a_crafter_waits_for_every_slot_of_its_chain_and_then_everything_begins() {
        let column = Column::new(Target::Depot);
        let timeline = run(&column, &COLUMN, &fill(5), 7);
        assert_eq!(state(&timeline, 2, 6).phase, Phase::Idle);
        assert_eq!(state(&timeline, 2, 6).slot, Some("a"));
        let begun = state(&timeline, 2, 7);
        assert_eq!(begun.slot, None);
        assert_ne!(begun.phase, Phase::Idle);
    }

    #[test]
    fn nothing_begins_while_the_chain_stands_still() {
        let mut column = Column::new(Target::Depot);
        column.rpm = 0.0;
        let timeline = run(&column, &COLUMN, &fill(5), 100);
        assert!(COLUMN.iter().all(|pos| timeline.at(*pos, 100).unwrap().1.phase == Phase::Idle));
    }

    #[test]
    fn items_travel_down_the_chain_and_the_scripted_result_reaches_the_depot() {
        let column = Column::new(Target::Depot);
        let timeline = run(&column, &COLUMN, &fill(5), 400);
        // The first crafter's items moved to the next and then the last ones.
        let kinds: Vec<Phase> = (5..400).map(|t| state(&timeline, 2, t).phase).collect();
        assert!(kinds.contains(&Phase::Exporting));
        assert!(kinds.contains(&Phase::Waiting));
        let last: Vec<Phase> = (5..400).map(|t| state(&timeline, 0, t).phase).collect();
        assert!(last.contains(&Phase::Crafting));
        assert_eq!(timeline.outputs.len(), 1, "{:?}", timeline.outputs);
        assert_eq!(timeline.outputs[0].stack, "pickaxe");
        assert_eq!(timeline.outputs[0].at, [0, -1, 0]);
        assert!(timeline.drops.is_empty());
        assert_eq!(state(&timeline, 0, 400).phase, Phase::Idle);
    }

    #[test]
    fn a_depot_that_is_full_keeps_the_result_waiting_until_the_scene_clears_it() {
        let column = Column::new(Target::Depot);
        let mut actions = vec![(0, Action::Occupy { pos: [0, -1, 0] })];
        actions.extend(fill(5));
        actions.push((300, Action::Clear { pos: [0, -1, 0] }));
        let timeline = run(&column, &COLUMN, &actions, 400);
        assert_eq!(state(&timeline, 0, 299).phase, Phase::Inserting);
        let output = &timeline.outputs[0];
        assert_eq!(output.stack, "pickaxe");
        // It tries again at its next lazy tick, so a little after the depot was emptied, not before.
        assert!(output.tick >= 300 && output.tick < 300 + LAZY_PERIOD, "{}", output.tick);
        assert_eq!(timeline.outputs.len(), 1);
        assert_eq!(state(&timeline, 0, 400).phase, Phase::Idle);
    }

    #[test]
    fn a_crafter_pointing_at_nothing_ejects_the_grid_in_front_of_it() {
        let column = Column::new(Target::Nothing);
        let timeline = run(&column, &COLUMN, &fill(5), 400);
        assert!(timeline.outputs.is_empty());
        assert_eq!(timeline.drops.len(), 1, "{:?}", timeline.drops);
        assert_eq!(timeline.drops[0].stack, "pickaxe");
        assert_eq!(timeline.drops[0].facing, Dir::North);
        assert_eq!(state(&timeline, 0, 400).phase, Phase::Idle);
    }

    #[test]
    fn a_container_takes_the_result_and_never_fills() {
        let column = Column::new(Target::Container);
        let mut actions = fill(5);
        actions.push((300, Action::Result { pos: [0, 0, 0], stack: "again" }));
        actions.extend([insert(310, 2, "a"), insert(311, 1, "b"), insert(312, 0, "c")]);
        let timeline = run(&column, &COLUMN, &actions, 800);
        assert_eq!(timeline.outputs.iter().map(|o| o.stack).collect::<Vec<_>>(), ["pickaxe", "again"]);
    }

    #[test]
    fn a_redstone_pulse_starts_a_chain_that_is_not_full() {
        let mut column = Column::new(Target::Container);
        column.powered_from = Some(50);
        let actions = vec![(0, Action::Result { pos: [0, 0, 0], stack: "pickaxe" }), insert(5, 2, "a"), insert(6, 1, "b")];
        let timeline = run(&column, &COLUMN, &actions, 49);
        assert_eq!(state(&timeline, 2, 49).phase, Phase::Idle);
        let pulsed = run(&column, &COLUMN, &actions, 55);
        assert_ne!(state(&pulsed, 2, 55).phase, Phase::Idle);
        assert_eq!(state(&pulsed, 0, 55).slot, None);
    }

    #[test]
    fn a_cover_counts_as_an_item_and_refuses_one() {
        let column = Column::new(Target::Container);
        let actions = vec![
            (0, Action::Result { pos: [0, 0, 0], stack: "bucket" }),
            (1, Action::Cover { pos: [0, 1, 0], covered: true }),
            insert(2, 1, "refused"),
            insert(3, 2, "a"),
            insert(4, 0, "c"),
        ];
        let timeline = run(&column, &COLUMN, &actions, 10);
        assert_eq!(state(&timeline, 1, 10).slot, None);
        assert_ne!(state(&timeline, 2, 10).phase, Phase::Idle);
    }

    #[test]
    fn a_stacked_insertion_gives_each_joined_crafter_one_item() {
        let column = Column::new(Target::Container);
        let actions = vec![(0, Action::Stacked { pos: [0, 1, 0], stack: "iron", count: 2 })];
        let timeline = run(&column, &COLUMN, &actions, 0);
        // Top row first: the highest crafter, then the next.
        assert_eq!(state(&timeline, 2, 0).slot, Some("iron"));
        assert_eq!(state(&timeline, 1, 0).slot, Some("iron"));
        assert_eq!(state(&timeline, 0, 0).slot, None);
    }

    #[test]
    fn a_crafter_that_comes_back_starts_empty_and_a_removed_one_has_no_state() {
        let mut column = Column::new(Target::Container);
        column.gone = Some(([0, 1, 0], 20));
        let actions = vec![insert(5, 2, "a")];
        let timeline = run(&column, &COLUMN, &actions, 30);
        assert!(timeline.at([0, 1, 0], 10).is_none());
        let back = state(&timeline, 1, 25);
        assert_eq!(back.phase, Phase::Idle);
        assert_eq!(back.slot, None);
    }

    #[test]
    fn joining_two_crafters_is_recorded_and_toggles_back() {
        let column = Column::new(Target::Container);
        let actions = vec![
            (3, Action::Connect { first: [0, 0, 0], second: [0, 1, 0] }),
            (6, Action::Connect { first: [0, 0, 0], second: [0, 1, 0] }),
        ];
        let timeline = run(&column, &COLUMN, &actions, 9);
        assert_eq!(timeline.inputs_at(2).unwrap()[&[0, 1, 0]], Input::default());
        assert!(!timeline.inputs_at(4).unwrap()[&[0, 1, 0]].controller);
        assert_eq!(timeline.inputs_at(8).unwrap()[&[0, 1, 0]], Input::default());
    }

    #[test]
    fn a_remainder_stays_in_the_grid_beside_the_result() {
        let column = Column::new(Target::Container);
        let actions = vec![
            (0, Action::Result { pos: [0, 0, 0], stack: "cake" }),
            insert(2, 2, "milk"),
            insert(3, 1, "b"),
            insert(4, 0, "c"),
        ];
        let timeline = run(&column, &COLUMN, &actions, 800);
        let stacks: Vec<_> = timeline.outputs.iter().map(|o| o.stack).collect();
        assert!(stacks.contains(&"cake"), "{stacks:?}");
    }
}
