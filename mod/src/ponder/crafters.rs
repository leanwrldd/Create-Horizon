//! A scene's Mechanical Crafters: the glue between the scene's blocks and `rules::ponder_crafter`, and what a crafter's
//! block entity shows at a tick (the keys `machines::crafter_draw` reads).

use std::collections::BTreeMap;

use create_kinetics::rules::crafter::Pointing;
use create_kinetics::rules::crafter_grid::Crafter;
use create_kinetics::rules::crafter_recipes::crafting_remainder;
use create_kinetics::rules::ponder_crafter::{Action, Setting, Target, Timeline, run};
use create_kinetics::{Dir, Pos};

use crate::compat::{ItemStack, Tag};
use crate::machines::{crafter_draw, crafter_world as world};
use crate::util;

pub(super) type Actions = Vec<(i64, Action<ItemStack>)>;
type State = (String, BTreeMap<String, String>);

pub(super) const CRAFTER: &str = "create:mechanical_crafter";

/// What the scene's blocks say, as [`Setting`] reads it.
pub(super) struct Scenery<'a> {
    pub palette: &'a [State],
    /// The structure's blocks, as palette indices.
    pub blocks: &'a BTreeMap<Pos, usize>,
    /// Each position's block changes: the tick and the palette index (`None`: `restoreBlocks`).
    pub changes: BTreeMap<Pos, Vec<(i64, Option<usize>)>>,
    /// The ticks each position was toggled at (`toggleRedstonePower`).
    pub toggles: BTreeMap<Pos, Vec<i64>>,
    /// Each block's speed over time: from the tick, in rpm.
    pub speeds: &'a BTreeMap<Pos, Vec<(f32, f32)>>,
}

impl Scenery<'_> {
    fn state(&self, pos: Pos, tick: i64) -> Option<&State> {
        let changed = self.changes.get(&pos).and_then(|steps| {
            let known = steps.partition_point(|(at, _)| *at <= tick);
            known.checked_sub(1).map(|i| steps[i].1)
        });
        let index = match changed {
            Some(Some(index)) => index,
            _ => *self.blocks.get(&pos)?,
        };
        self.palette.get(index)
    }

    /// Where a crafter may stand: the structure's, and the positions a scene turns into one.
    pub fn candidates(&self) -> Vec<Pos> {
        let is_crafter = |index: usize| self.palette.get(index).is_some_and(|(id, _)| id == CRAFTER);
        let mut found: Vec<Pos> = self.blocks.iter().filter(|(_, index)| is_crafter(**index)).map(|(pos, _)| *pos).collect();
        for (pos, steps) in &self.changes {
            if steps.iter().any(|(_, index)| index.is_some_and(is_crafter)) && !found.contains(pos) {
                found.push(*pos);
            }
        }
        found.sort_unstable();
        found
    }
}

fn container(id: &str) -> bool {
    matches!(
        id,
        "minecraft:chest"
            | "minecraft:trapped_chest"
            | "minecraft:barrel"
            | "minecraft:hopper"
            | "minecraft:dropper"
            | "minecraft:dispenser"
            | "minecraft:furnace"
            | "minecraft:blast_furnace"
            | "minecraft:smoker"
            | "create:item_vault"
            | "create:basin"
            | "create:packager"
            | "create:repackager"
            | "create:chute"
            | "create:smart_chute"
            | "create:creative_crate"
            | "create:andesite_funnel"
            | "create:brass_funnel"
    )
}

impl Setting<ItemStack> for Scenery<'_> {
    fn crafter(&self, pos: Pos, tick: i64) -> Option<(Dir, Pointing)> {
        let (id, props) = self.state(pos, tick)?;
        (id == CRAFTER).then_some(())?;
        Some((Dir::from_name(props.get("facing")?)?, Pointing::from_name(props.get("pointing")?)?))
    }

    fn rpm(&self, pos: Pos, tick: i64) -> f32 {
        self.speeds
            .get(&pos)
            .and_then(|log| log.iter().rev().find(|(from, _)| *from <= tick as f32))
            .map_or(0.0, |&(_, rpm)| rpm)
    }

    /// `hasNeighborSignal`: a lever, a wire or a block touching the crafter that the scene turned on.
    fn powered(&self, pos: Pos, tick: i64) -> bool {
        Dir::ALL.into_iter().any(|dir| {
            let n = dir.normal();
            let neighbour = [pos[0] + n[0], pos[1] + n[1], pos[2] + n[2]];
            self.toggles.get(&neighbour).is_some_and(|ticks| ticks.iter().filter(|at| **at <= tick).count() % 2 == 1)
        })
    }

    fn target(&self, pos: Pos, tick: i64) -> Target {
        match self.state(pos, tick).map(|(id, _)| id.as_str()) {
            Some("create:depot") => Target::Depot,
            Some("create:belt") => Target::Belt,
            Some(id) if container(id) => Target::Container,
            _ => Target::Nothing,
        }
    }

    fn remainder(&self, stack: &ItemStack) -> Option<ItemStack> {
        crafting_remainder(&stack.item).map(|id| ItemStack::new(id, 1))
    }
}

/// Runs the scene's crafters; `None` when it has none to act on.
pub(super) fn replay(scenery: &Scenery<'_>, actions: &Actions, until: i64) -> Option<Timeline<ItemStack>> {
    let candidates = scenery.candidates();
    (!actions.is_empty() && !candidates.is_empty()).then(|| run(scenery, &candidates, actions, until))
}

/// The keys of a crafter's block entity at `tick`, as `machines::crafter_draw` reads them.
pub(super) fn tags(timeline: &Timeline<ItemStack>, pos: Pos, tick: i64) -> Vec<(String, Tag)> {
    let Some((since, crafter)) = timeline.at(pos, tick) else {
        return Vec::new();
    };
    let mut tags: Vec<(String, Tag)> = vec![
        (world::PHASE.to_owned(), Tag::String(crafter.phase.name().into())),
        (world::COVERED.to_owned(), Tag::Byte(i8::from(crafter.covered))),
        (world::ANCHOR.to_owned(), Tag::Int(crafter.count_down)),
        (world::SINCE.to_owned(), Tag::Long(since)),
        (world::GROUPED.to_owned(), world::grid_tag(&crafter.grid)),
        (world::BEFORE.to_owned(), world::grid_tag(&crafter.before_craft)),
        (crafter_draw::INVENTORY.to_owned(), slot_tag(crafter)),
    ];
    if let Some(input) = timeline.inputs_at(tick).and_then(|inputs| inputs.get(&pos)) {
        tags.push((world::INPUT.to_owned(), world::input_tag(input)));
    }
    tags
}

fn slot_tag(crafter: &Crafter<ItemStack>) -> Tag {
    crafter
        .slot
        .as_ref()
        .map_or_else(|| Tag::Compound(crate::Compound::new()), util::stack_tag)
}

/// Where a crafter's ejected stack appears: 0.75 out of its front, from its centre.
pub(super) fn eject_point(pos: Pos, facing: Dir) -> [f32; 3] {
    let front = facing.normal();
    std::array::from_fn(|axis| pos[axis] as f32 + 0.5 + front[axis] as f32 * 0.75)
}

#[cfg(test)]
mod tests {
    use super::*;
    use create_kinetics::rules::crafter_grid::Phase;

    fn crafter(facing: &str, pointing: &str) -> State {
        (
            CRAFTER.to_owned(),
            BTreeMap::from([("facing".to_owned(), facing.to_owned()), ("pointing".to_owned(), pointing.to_owned())]),
        )
    }

    fn plain(id: &str) -> State {
        (id.to_owned(), BTreeMap::new())
    }

    fn palette() -> Vec<State> {
        vec![
            crafter("north", "right"),
            crafter("north", "down"),
            plain("minecraft:air"),
            plain("create:depot"),
            plain("minecraft:hopper"),
            plain("create:belt"),
        ]
    }

    fn scenery<'a>(
        palette: &'a [State],
        blocks: &'a BTreeMap<Pos, usize>,
        speeds: &'a BTreeMap<Pos, Vec<(f32, f32)>>,
    ) -> Scenery<'a> {
        Scenery { palette, blocks, changes: BTreeMap::new(), toggles: BTreeMap::new(), speeds }
    }

    #[test]
    fn a_block_the_scene_changes_is_a_crafter_only_while_it_is_one() {
        let palette = palette();
        let blocks = BTreeMap::from([([1, 1, 2], 0)]);
        let speeds = BTreeMap::new();
        let mut scenery = scenery(&palette, &blocks, &speeds);
        scenery.changes.insert([1, 1, 2], vec![(10, Some(1)), (20, Some(2)), (30, None)]);
        scenery.changes.insert([5, 1, 2], vec![(12, Some(1))]);
        let at = |tick| scenery.crafter([1, 1, 2], tick);
        assert_eq!(at(0), Some((Dir::North, Pointing::Right)));
        assert_eq!(at(10), Some((Dir::North, Pointing::Down)));
        assert_eq!(at(25), None);
        // `restoreBlocks` puts the structure's block back.
        assert_eq!(at(30), Some((Dir::North, Pointing::Right)));
        assert_eq!(scenery.candidates(), vec![[1, 1, 2], [5, 1, 2]]);
        assert_eq!(scenery.crafter([5, 1, 2], 11), None);
        assert_eq!(scenery.crafter([5, 1, 2], 12), Some((Dir::North, Pointing::Down)));
    }

    #[test]
    fn redstone_reaches_a_crafter_from_a_block_beside_it_that_the_scene_turned_on() {
        let palette = palette();
        let blocks = BTreeMap::new();
        let speeds = BTreeMap::new();
        let mut scenery = scenery(&palette, &blocks, &speeds);
        scenery.toggles.insert([3, 1, 1], vec![5, 15]);
        assert!(!scenery.powered([3, 1, 2], 4));
        assert!(scenery.powered([3, 1, 2], 5));
        assert!(scenery.powered([3, 1, 2], 14));
        assert!(!scenery.powered([3, 1, 2], 15));
        assert!(!scenery.powered([4, 1, 4], 9));
    }

    #[test]
    fn the_speed_follows_the_scenes_log_and_the_targets_are_told_by_block() {
        let palette = palette();
        let blocks = BTreeMap::from([([0, 1, 2], 3), ([9, 1, 2], 4), ([8, 1, 2], 5), ([7, 1, 2], 2)]);
        let speeds = BTreeMap::from([([1, 1, 2], vec![(-1.0, 32.0), (40.0, -48.0)])]);
        let scenery = scenery(&palette, &blocks, &speeds);
        assert_eq!(scenery.rpm([1, 1, 2], 39), 32.0);
        assert_eq!(scenery.rpm([1, 1, 2], 40), -48.0);
        assert_eq!(scenery.rpm([2, 1, 2], 40), 0.0);
        assert_eq!(scenery.target([0, 1, 2], 0), Target::Depot);
        assert_eq!(scenery.target([9, 1, 2], 0), Target::Container);
        assert_eq!(scenery.target([8, 1, 2], 0), Target::Belt);
        assert_eq!(scenery.target([7, 1, 2], 0), Target::Nothing);
        assert_eq!(scenery.target([6, 6, 6], 0), Target::Nothing);
        let left = scenery.remainder(&ItemStack::new("minecraft:milk_bucket", 1));
        assert_eq!(left.map(|stack| stack.item.to_string()), Some("minecraft:bucket".to_owned()));
    }

    #[test]
    fn a_crafter_shows_what_the_replay_left_in_it_through_the_keys_its_renderer_reads() {
        let palette = palette();
        // A crafter turning at 32 RPM, pointing at a depot, and an item put in its slot at tick 3.
        let blocks = BTreeMap::from([([1, 1, 2], 0), ([0, 1, 2], 3)]);
        let speeds = BTreeMap::from([([1, 1, 2], vec![(-1.0, 32.0)])]);
        let scenery = scenery(&palette, &blocks, &speeds);
        let planks = ItemStack::new("minecraft:oak_planks", 1);
        let actions: Actions = vec![
            (0, Action::Result { pos: [1, 1, 2], stack: ItemStack::new("minecraft:piston", 1) }),
            (3, Action::Insert { pos: [1, 1, 2], stack: planks }),
        ];
        let timeline = replay(&scenery, &actions, 200).expect("a crafter replay");
        let key = |tags: &[(String, Tag)], name: &str| tags.iter().find(|(key, _)| key == name).map(|(_, tag)| tag.clone());
        let before = tags(&timeline, [1, 1, 2], 2);
        assert_eq!(key(&before, world::PHASE), Some(Tag::String("IDLE".into())));
        assert_eq!(key(&before, crafter_draw::INVENTORY), Some(Tag::Compound(crate::Compound::new())));
        // A lone crafter whose slot is full is the whole chain: it crafts and hands the piston to the depot.
        let ran: Vec<Phase> = (3..200).filter_map(|t| timeline.at([1, 1, 2], t).map(|(_, c)| c.phase)).collect();
        assert!(ran.contains(&Phase::Crafting), "{ran:?}");
        assert_eq!(timeline.outputs.len(), 1);
        assert_eq!(&*timeline.outputs[0].stack.item, "minecraft:piston");
        assert_eq!(timeline.outputs[0].at, [0, 1, 2]);
        let crafting = (3..200)
            .find(|t| timeline.at([1, 1, 2], *t).is_some_and(|(_, c)| c.phase == Phase::Crafting))
            .unwrap();
        let during = tags(&timeline, [1, 1, 2], crafting);
        assert_eq!(key(&during, world::PHASE), Some(Tag::String("CRAFTING".into())));
        assert!(matches!(key(&during, world::SINCE), Some(Tag::Long(since)) if since == crafting));
        assert!(matches!(key(&during, world::ANCHOR), Some(Tag::Int(n)) if n > 0 && n <= 2000));
        // No crafter stands where none is.
        assert!(tags(&timeline, [4, 4, 4], 10).is_empty());
    }

    #[test]
    fn a_scene_with_nothing_to_do_to_its_crafters_has_no_replay() {
        let palette = palette();
        let blocks = BTreeMap::from([([1, 1, 2], 0)]);
        let speeds = BTreeMap::new();
        assert!(replay(&scenery(&palette, &blocks, &speeds), &Vec::new(), 100).is_none());
    }

    #[test]
    fn an_ejected_stack_starts_in_front_of_the_crafter() {
        assert_eq!(eject_point([1, 1, 2], Dir::North), [1.5, 1.5, 2.5 - 0.75]);
    }
}
