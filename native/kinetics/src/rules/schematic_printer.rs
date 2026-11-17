//! What a Schematicannon prints and in which order (`SchematicPrinter`, `SchematicLevel`'s bounds,
//! `ItemRequirement`, `BlockHelper.setZeroAge`): the blocks of a deployed schematic laid out about
//! its anchor, the walk through them (rows east, then south, then up; fragile blocks deferred to
//! the end), what each block costs and whether it is worth placing where the world stands. No game
//! types: the glue reads the world and answers [`Existing`].
//!
//! Known differences from Create, for the README:
//! - A block costs the item vanilla's data gives it (`Block.asItem`, baked in `schematic_facts`);
//!   Create's own special cases are kept for the blocks this mod has (encased shafts and cogwheels,
//!   belts' shafts, belt funnels, blaze burners, glass and encased pipes, nixie tubes), and the
//!   framed-blocks, copycat, bracket, backtank, clipboard and placard extras are not (those blocks
//!   cost their own item only).
//! - Belts are never printed: a belt is built by connecting two shafts, which a schematic can't
//!   do here, so the cannon leaves `create:belt` blocks out (their shafts print as shafts).
//! - Entities (item frames, armor stands, glue) are not printed.

use std::collections::{BTreeMap, VecDeque};

use crate::rules::contraption::{State, is_brittle_name};
use crate::rules::schematic_facts::{BLOCK_ENTITY, ITEM_OF, SOLID, UNBREAKABLE};
use crate::rules::schematic_nbt::Nbt;
use crate::rules::schematic_structure::{Pos, Template};
use crate::rules::schematic_transform::{Mirror, Rotation, rotate_size, transform_pos, transform_state};

fn has_name(list: &[&str], name: &str) -> bool {
    list.binary_search(&name).is_ok()
}

fn add(a: Pos, b: Pos) -> Pos {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: Pos, b: Pos) -> Pos {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

// --- blocks as the world knows them ---------------------------------------------------------------

/// `Block.isAir` of `AirBlock`: air, cave air and void air.
pub fn is_air(state: &State) -> bool {
    matches!(state.name.as_str(), "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air")
}

/// Whether the block has a block entity. Vanilla's are baked; `own` says whether the world holds
/// one for a block of a mod (the glue asks the host).
pub fn has_block_entity(state: &State, own: bool) -> bool {
    if state.name.starts_with("minecraft:") { has_name(BLOCK_ENTITY, &state.name) } else { own }
}

/// `getDestroySpeed == -1`.
pub fn unbreakable(state: &State) -> bool {
    has_name(UNBREAKABLE, &state.name)
}

/// `isRedstoneConductor`, as the block drawing a full opaque cube.
pub fn conductor(state: &State) -> bool {
    has_name(SOLID, &state.name)
}

/// `BlockHelper.setZeroAge`: a crop, a hive, a turtle egg, a sapling, a composter goes back to its
/// first stage, a cauldron is empty and a piston retracted.
pub fn zero_age(state: &State) -> State {
    let mut out = state.clone();
    for key in ["age", "honey_level", "hatch", "stage"] {
        if state.has(key) {
            out.set(key, "0");
            return out;
        }
    }
    if state.name.starts_with("minecraft:") && state.path().ends_with("cauldron") {
        return State::new("minecraft:cauldron");
    }
    if state.name == "minecraft:composter" && state.has("level") {
        out.set("level", "0");
    } else if state.has("extended") {
        out.set("extended", "false");
    }
    out
}

/// `BlockHelper.placeSchematicBlock`'s changes to the state before it is placed: no waterlogging,
/// pistons retracted, a cauldron empty, a composter empty, a chiseled bookshelf with no books
/// (`AllSchematicStateFilters`).
pub fn placement_state(state: &State) -> State {
    let mut out = state.clone();
    if out.has("extended") {
        out.set("extended", "false");
    }
    if out.has("waterlogged") {
        out.set("waterlogged", "false");
    }
    if out.name == "minecraft:composter" {
        return State::new("minecraft:composter");
    }
    if out.name.starts_with("minecraft:") && out.path().ends_with("cauldron") {
        return State::new("minecraft:cauldron");
    }
    if out.name == "minecraft:chiseled_bookshelf" {
        for slot in 0..6 {
            let key = format!("slot_{slot}_occupied");
            if out.has(&key) {
                out.set(&key, "false");
            }
        }
    }
    out
}

/// Two states are the same block as far as a cannon cares: the same name, and every property the
/// schematic's state lists is the same in the world's (a state from a file that leaves out a
/// property the game fills in matches).
pub fn same_state(wanted: &State, found: &State) -> bool {
    wanted.name == found.name && wanted.props.iter().all(|(key, value)| found.get(key) == Some(value.as_str()))
}

// --- what a block costs ---------------------------------------------------------------------------

/// `ItemRequirement`.
#[derive(Clone, Debug, PartialEq)]
pub enum Requirement {
    /// `ItemRequirement.NONE`: air, which costs nothing.
    None,
    /// `ItemRequirement.INVALID`: a block with no item, which a cannon can't print.
    Invalid,
    /// The items it consumes, each with its count.
    Items(Vec<(String, u32)>),
}

/// `ItemRequirement.of(state, be)`. `item_exists` says whether the game has an item of that id.
pub fn requirement(state: &State, item_exists: &dyn Fn(&str) -> bool) -> Requirement {
    let items = |list: &[(&str, u32)]| {
        Requirement::Items(list.iter().map(|(id, count)| ((*id).to_owned(), *count)).collect())
    };
    let name = state.name.as_str();
    // Create's own blocks that cost another item than their own (`getRequiredItems`).
    if let Some(path) = name.strip_prefix("create:") {
        match path {
            "andesite_encased_shaft" | "brass_encased_shaft" => return items(&[("create:shaft", 1)]),
            "andesite_encased_cogwheel" | "brass_encased_cogwheel" => return items(&[("create:cogwheel", 1)]),
            "andesite_encased_large_cogwheel" | "brass_encased_large_cogwheel" => {
                return items(&[("create:large_cogwheel", 1)]);
            }
            "belt" => {
                let part = state.get("part").unwrap_or("start");
                let mut list: Vec<(&str, u32)> = Vec::new();
                if part != "middle" {
                    list.push(("create:shaft", 1));
                }
                if part == "start" {
                    list.push(("create:belt_connector", 1));
                }
                return if list.is_empty() { Requirement::None } else { items(&list) };
            }
            "andesite_belt_funnel" => return items(&[("create:andesite_funnel", 1)]),
            "brass_belt_funnel" => return items(&[("create:brass_funnel", 1)]),
            "blaze_burner" => {
                let lit = state.get("blaze").is_some_and(|heat| heat != "none");
                return items(&[(if lit { "create:blaze_burner" } else { "create:empty_blaze_burner" }, 1)]);
            }
            "encased_fluid_pipe" | "glass_fluid_pipe" => return items(&[("create:fluid_pipe", 1)]),
            "white_nixie_tube" | "orange_nixie_tube" => return items(&[("create:orange_nixie_tube", 1)]),
            _ if path.ends_with("_nixie_tube") => return items(&[("create:orange_nixie_tube", 1)]),
            _ => {}
        }
    }
    if name == "minecraft:air" {
        return Requirement::None;
    }
    let item: &str = match ITEM_OF.binary_search_by(|(block, _)| (*block).cmp(name)) {
        Ok(at) => ITEM_OF[at].1,
        Err(_) => name,
    };
    if item.is_empty() || !item_exists(item) {
        return Requirement::Invalid;
    }
    let count = |key: &str| state.get(key).and_then(|v| v.parse::<u32>().ok()).unwrap_or(1);
    // A double slab needs two, as do turtle eggs, sea pickles and snow layers by their count.
    if state.path().ends_with("_slab") && state.get("type") == Some("double") {
        return items(&[(item, 2)]);
    }
    match name {
        "minecraft:turtle_egg" => return items(&[(item, count("eggs"))]),
        "minecraft:sea_pickle" => return items(&[(item, count("pickles"))]),
        "minecraft:snow" => return items(&[(item, count("layers"))]),
        // `FarmBlock` and `DirtPathBlock` cost dirt.
        "minecraft:farmland" | "minecraft:dirt_path" => return items(&[("minecraft:dirt", 1)]),
        _ => {}
    }
    items(&[(item, 1)])
}

/// `SchematicannonBlockEntity.shouldIgnoreBlockState`: blocks that are not placed on their own
/// (the other half of a door, a bed, a piston's head, the middle of a belt).
pub fn should_ignore(state: &State, item_exists: &dyn Fn(&str) -> bool) -> bool {
    if state.name == "minecraft:structure_void" {
        return true;
    }
    // Printing a belt is not possible here (see the module's differences), so none is attempted.
    if state.name == "create:belt" {
        return true;
    }
    match requirement(state, item_exists) {
        Requirement::None | Requirement::Invalid => return false,
        Requirement::Items(_) => {}
    }
    if state.get("half") == Some("upper") {
        return true;
    }
    if state.get("part") == Some("head") && state.has("occupied") {
        return true;
    }
    if state.name == "minecraft:piston_head" {
        return true;
    }
    false
}

/// A block with `DoubleBlockHalf` (doors, tall plants): its `half` is upper or lower.
fn is_double_block(state: &State) -> bool {
    matches!(state.get("half"), Some("upper" | "lower"))
}

/// `ItemRequirement` of the fragile blocks `SchematicPrinter.shouldDeferBlock` puts off: gantry
/// carriages, mechanical arms and what `BlockMovementChecks.isBrittle` says.
pub fn should_defer(state: &State) -> bool {
    state.name == "create:gantry_carriage" || state.name == "create:mechanical_arm" || is_brittle_name(state)
}

// --- the schematic laid out -----------------------------------------------------------------------

/// `BoundingBox`: both corners included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    pub min: Pos,
    pub max: Pos,
}

impl Bounds {
    pub fn around(pos: Pos) -> Self {
        Bounds { min: pos, max: pos }
    }

    /// `BBHelper.encapsulate`.
    pub fn encapsulate(self, pos: Pos) -> Self {
        Bounds {
            min: [self.min[0].min(pos[0]), self.min[1].min(pos[1]), self.min[2].min(pos[2])],
            max: [self.max[0].max(pos[0]), self.max[1].max(pos[1]), self.max[2].max(pos[2])],
        }
    }

    pub fn contains(self, pos: Pos) -> bool {
        (0..3).all(|i| pos[i] >= self.min[i] && pos[i] <= self.max[i])
    }

    /// `getXSpan`, `getYSpan`, `getZSpan`.
    pub fn span(self) -> Pos {
        [self.max[0] - self.min[0] + 1, self.max[1] - self.min[1] + 1, self.max[2] - self.min[2] + 1]
    }
}

/// `SchematicLevel` of a deployed schematic: its blocks by position relative to the anchor, as
/// `StructureTemplate.placeInWorld` places them (mirrored, turned), with the bounds the level
/// keeps.
#[derive(Clone, Debug)]
pub struct Layout {
    blocks: BTreeMap<Pos, (State, Option<Nbt>)>,
    pub bounds: Bounds,
}

impl Layout {
    /// `SchematicPrinter.loadSchematic`'s placement and `setBounds(encapsulate(extraBounds))`.
    pub fn new(template: &Template, rotation: Rotation, mirror: Mirror) -> Self {
        let mut blocks = BTreeMap::new();
        let mut bounds = Bounds::around([0, 0, 0]);
        for block in &template.blocks {
            let at = transform_pos(block.pos, mirror, rotation);
            bounds = bounds.encapsulate(at);
            let state = transform_state(template.state(block), mirror, rotation);
            blocks.insert(at, (state, block.nbt.clone()));
        }
        // `calculateRelativePosition(settings, size - 1)`.
        let far = transform_pos(sub(template.size, [1, 1, 1]), mirror, rotation);
        bounds = bounds.encapsulate(far);
        Layout { blocks, bounds }
    }

    /// The state at a position relative to the anchor: air where the schematic has no block.
    pub fn state(&self, rel: Pos) -> State {
        self.blocks.get(&rel).map_or_else(|| State::new("minecraft:air"), |(state, _)| state.clone())
    }

    pub fn nbt(&self, rel: Pos) -> Option<&Nbt> {
        self.blocks.get(&rel).and_then(|(_, nbt)| nbt.as_ref())
    }

    pub fn has_block(&self, rel: Pos) -> bool {
        self.blocks.contains_key(&rel)
    }

    /// `getAllPositions`.
    pub fn positions(&self) -> impl Iterator<Item = Pos> + '_ {
        self.blocks.keys().copied()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }
}

/// The size of the template turned (what the hologram's outline spans).
pub fn turned_size(template: &Template, rotation: Rotation) -> Pos {
    rotate_size(template.size, rotation)
}

/// `SchematicPrinter.PrintStage`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Blocks,
    DeferredBlocks,
    Entities,
}

/// What the world holds where a block would go: the glue answers it from the host.
#[derive(Clone, Debug, PartialEq)]
pub struct Existing {
    pub state: State,
    pub has_block_entity: bool,
}

impl Existing {
    pub fn air() -> Self {
        Existing { state: State::new("minecraft:air"), has_block_entity: false }
    }
}

/// `SchematicPrinter`: the walk through a layout.
#[derive(Clone, Debug)]
pub struct Printer {
    pub anchor: Pos,
    pub layout: Layout,
    /// `currentPos`, relative to the anchor.
    pub current: Pos,
    pub stage: Stage,
    deferred: VecDeque<Pos>,
}

impl Printer {
    pub fn new(anchor: Pos, layout: Layout) -> Self {
        let bounds = layout.bounds;
        Printer {
            anchor,
            current: [bounds.min[0] - 1, bounds.min[1], bounds.min[2]],
            layout,
            stage: Stage::Blocks,
            deferred: VecDeque::new(),
        }
    }

    /// `getCurrentTarget`: where the walk stands, in the world.
    pub fn target(&self) -> Pos {
        add(self.anchor, self.current)
    }

    /// `SchematicLevel.getBlockState(globalPos)`.
    pub fn state_at(&self, target: Pos) -> State {
        self.layout.state(sub(target, self.anchor))
    }

    /// `advanceCurrentPos`: the next position to print; false when everything has been.
    pub fn advance(&mut self) -> bool {
        loop {
            if self.stage == Stage::Blocks {
                while self.try_advance() {
                    self.deferred.push_back(self.current);
                }
            }
            if self.stage == Stage::DeferredBlocks {
                match self.deferred.pop_front() {
                    Some(next) => self.current = next,
                    None => self.stage = Stage::Entities,
                }
            }
            // No entities are printed: the walk ends with the blocks.
            if self.stage == Stage::Entities {
                return false;
            }
            if self.layout.bounds.contains(self.current) {
                return true;
            }
        }
    }

    /// `tryAdvanceCurrentPos`, quirks and all (it walks one row and one layer past the box, which
    /// `advance` skips): one step east, wrapping to the next row and layer; true when the block
    /// found is one to leave for later.
    fn try_advance(&mut self) -> bool {
        let bounds = self.layout.bounds;
        self.current[0] += 1;
        let span = bounds.span();
        let inside = sub(self.current, bounds.min);
        if inside[0] > span[0] {
            self.current = [bounds.min[0] - 1, self.current[1], self.current[2] + 1];
        }
        if inside[2] > span[2] {
            self.current = [self.current[0] - 1, self.current[1] + 1, bounds.min[2]];
        }
        if self.current[1] > span[1] {
            self.stage = Stage::DeferredBlocks;
            return false;
        }
        should_defer(&self.state_at(self.target()))
    }

    /// `getCurrentRequirement` for the block at the current target.
    pub fn current_requirement(&self, item_exists: &dyn Fn(&str) -> bool) -> Requirement {
        requirement(&zero_age(&self.state_at(self.target())), item_exists)
    }

    /// `shouldPlaceBlock`: whether the block at `target` is worth placing where the world stands.
    /// `loaded` says the chunk is loaded and inside the world border, `world` answers what is at a
    /// position, and `decide` is the cannon's own say (its replace mode).
    pub fn should_place(
        &self,
        target: Pos,
        loaded: bool,
        world: &mut dyn FnMut(Pos) -> Existing,
        decide: &dyn Fn(&Candidate) -> bool,
    ) -> bool {
        let state = zero_age(&self.state_at(target));
        let to_replace = world(target);
        let mut other: Option<Existing> = None;
        if state.get("part") == Some("foot") && state.has("occupied") {
            if let Some(facing) = state.get("facing").and_then(crate::Dir::from_name) {
                other = Some(world(add(target, facing.normal())));
            }
        }
        if state.get("half") == Some("lower") && is_double_block(&state) {
            other = Some(world([target[0], target[1] + 1, target[2]]));
        }
        if !loaded {
            return false;
        }
        if same_state(&state, &to_replace.state) {
            return false;
        }
        if unbreakable(&to_replace.state) || other.as_ref().is_some_and(|o| unbreakable(&o.state)) {
            return false;
        }
        let is_normal_cube = conductor(&state);
        decide(&Candidate { target, state, to_replace, to_replace_other: other, is_normal_cube })
    }

    /// `markAllBlockRequirements`: how many blocks would be placed and what they cost in all.
    pub fn count_requirements(
        &self,
        world: &mut dyn FnMut(Pos) -> Option<Existing>,
        decide: &dyn Fn(&Candidate) -> bool,
        item_exists: &dyn Fn(&str) -> bool,
        checklist: &mut Checklist,
    ) -> i32 {
        let mut to_place = 0;
        let positions: Vec<Pos> = self.layout.positions().collect();
        for rel in positions {
            let target = add(self.anchor, rel);
            let required = self.layout.state(rel);
            if world(target).is_none() {
                checklist.not_loaded = true;
                continue;
            }
            let mut ask = |at: Pos| world(at).unwrap_or_else(Existing::air);
            if !self.should_place(target, true, &mut ask, decide) {
                continue;
            }
            match requirement(&required, item_exists) {
                Requirement::None | Requirement::Invalid => continue,
                Requirement::Items(list) => {
                    for (item, count) in list {
                        checklist.require(&item, count);
                    }
                    to_place += 1;
                }
            }
        }
        to_place
    }
}

/// What `PlacementPredicate` is handed: the block to place and what stands there now.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub target: Pos,
    pub state: State,
    pub to_replace: Existing,
    pub to_replace_other: Option<Existing>,
    pub is_normal_cube: bool,
}

/// `MaterialChecklist`: what a schematic needs, and what the containers beside the cannon hold.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Checklist {
    pub required: BTreeMap<String, u32>,
    pub gathered: BTreeMap<String, u32>,
    pub not_loaded: bool,
}

impl Checklist {
    pub fn require(&mut self, item: &str, count: u32) {
        *self.required.entry(item.to_owned()).or_insert(0) += count;
    }

    pub fn collect(&mut self, item: &str, count: u32) {
        *self.gathered.entry(item.to_owned()).or_insert(0) += count;
    }

    /// The lines of the checklist: `item: have / need`, the ones that are short first.
    pub fn lines(&self) -> Vec<String> {
        let mut rows: Vec<(bool, String)> = self
            .required
            .iter()
            .map(|(item, need)| {
                let have = self.gathered.get(item).copied().unwrap_or(0);
                (have >= *need, format!("{item}: {have} / {need}"))
            })
            .collect();
        rows.sort_by_key(|(enough, _)| *enough);
        rows.into_iter().map(|(_, line)| line).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::schematic_structure::Block;

    fn template(size: Pos, blocks: &[(Pos, &str)]) -> Template {
        let mut template = Template { size, ..Template::default() };
        for (pos, text) in blocks {
            let state = State::parse(text);
            let index = template.palette.iter().position(|s| *s == state).unwrap_or_else(|| {
                template.palette.push(state.clone());
                template.palette.len() - 1
            });
            template.blocks.push(Block { pos: *pos, state: index, nbt: None });
        }
        template
    }

    fn exists(_: &str) -> bool {
        true
    }

    fn walk(printer: &mut Printer) -> Vec<Pos> {
        let mut out = Vec::new();
        while printer.advance() {
            out.push(printer.current);
        }
        out
    }

    #[test]
    fn the_walk_goes_east_then_south_then_up() {
        // A 2 x 2 x 2 box, every cell stone.
        let mut blocks = Vec::new();
        for y in 0..2 {
            for x in 0..2 {
                for z in 0..2 {
                    blocks.push(([x, y, z], "minecraft:stone"));
                }
            }
        }
        let template = template([2, 2, 2], &blocks);
        let layout = Layout::new(&template, Rotation::None, Mirror::None);
        assert_eq!(layout.bounds, Bounds { min: [0, 0, 0], max: [1, 1, 1] });
        let mut printer = Printer::new([10, 64, 10], layout);
        let order = walk(&mut printer);
        assert_eq!(
            order,
            [
                [0, 0, 0], [1, 0, 0], [0, 0, 1], [1, 0, 1],
                [0, 1, 0], [1, 1, 0], [0, 1, 1], [1, 1, 1],
            ]
        );
        assert_eq!(printer.stage, Stage::Entities);
        // The walk stays at its end.
        assert!(!printer.advance());
    }

    #[test]
    fn fragile_blocks_wait_until_the_end() {
        let template = template(
            [3, 1, 1],
            &[
                ([0, 0, 0], "minecraft:stone"),
                ([1, 0, 0], "minecraft:torch"),
                ([2, 0, 0], "minecraft:stone"),
            ],
        );
        let layout = Layout::new(&template, Rotation::None, Mirror::None);
        let mut printer = Printer::new([0, 0, 0], layout);
        assert_eq!(walk(&mut printer), [[0, 0, 0], [2, 0, 0], [1, 0, 0]]);
    }

    #[test]
    fn a_turned_schematic_walks_its_turned_box() {
        let template = template([3, 1, 2], &[([0, 0, 0], "minecraft:stone"), ([2, 0, 1], "minecraft:dirt")]);
        let layout = Layout::new(&template, Rotation::Clockwise90, Mirror::None);
        // Clockwise: (x, z) goes to (-z, x): the dirt lands at (-1, 0, 2).
        assert!(layout.has_block([-1, 0, 2]));
        assert_eq!(layout.bounds, Bounds { min: [-1, 0, 0], max: [0, 0, 2] });
        let mut printer = Printer::new([5, 5, 5], layout);
        // Every cell of the box is visited, air included, in the same order.
        let order = walk(&mut printer);
        assert_eq!(order.len(), 6);
        assert_eq!(order[0], [-1, 0, 0]);
        assert_eq!(*order.last().unwrap(), [0, 0, 2]);
        // The target is the walk's position about the anchor.
        let mut printer = Printer::new([5, 5, 5], Layout::new(&template, Rotation::Clockwise90, Mirror::None));
        printer.advance();
        assert_eq!(printer.target(), [4, 5, 5]);
    }

    #[test]
    fn requirements_follow_create() {
        let cost = |text: &str| requirement(&State::parse(text), &exists);
        assert_eq!(cost("minecraft:air"), Requirement::None);
        assert_eq!(cost("minecraft:stone"), Requirement::Items(vec![("minecraft:stone".into(), 1)]));
        assert_eq!(cost("minecraft:water[level=0]"), Requirement::Invalid);
        assert_eq!(
            cost("minecraft:oak_slab[type=double,waterlogged=false]"),
            Requirement::Items(vec![("minecraft:oak_slab".into(), 2)])
        );
        assert_eq!(cost("minecraft:snow[layers=3]"), Requirement::Items(vec![("minecraft:snow".into(), 3)]));
        assert_eq!(
            cost("minecraft:oak_wall_sign[facing=north]"),
            Requirement::Items(vec![("minecraft:oak_sign".into(), 1)])
        );
        assert_eq!(cost("minecraft:wheat[age=7]"), Requirement::Items(vec![("minecraft:wheat_seeds".into(), 1)]));
        assert_eq!(cost("minecraft:farmland[moisture=7]"), Requirement::Items(vec![("minecraft:dirt".into(), 1)]));
        assert_eq!(
            cost("create:andesite_encased_shaft[axis=x]"),
            Requirement::Items(vec![("create:shaft".into(), 1)])
        );
        assert_eq!(
            cost("create:belt[part=start,slope=horizontal]"),
            Requirement::Items(vec![("create:shaft".into(), 1), ("create:belt_connector".into(), 1)])
        );
        assert_eq!(cost("create:belt[part=middle]"), Requirement::None);
        // A block the game has no item for can't be printed.
        assert_eq!(requirement(&State::parse("mod:weird"), &|_| false), Requirement::Invalid);
    }

    #[test]
    fn halves_and_heads_are_not_placed_on_their_own() {
        let ignore = |text: &str| should_ignore(&State::parse(text), &exists);
        assert!(ignore("minecraft:oak_door[half=upper,hinge=left,facing=north,open=false]"));
        assert!(!ignore("minecraft:oak_door[half=lower,hinge=left,facing=north,open=false]"));
        assert!(ignore("minecraft:tall_grass[half=upper]"));
        assert!(ignore("minecraft:red_bed[part=head,facing=north,occupied=false]"));
        assert!(!ignore("minecraft:red_bed[part=foot,facing=north,occupied=false]"));
        // A piston head has no item (`Items.AIR`): invalid, so the cannon skips it all the same.
        assert_eq!(
            requirement(&State::parse("minecraft:piston_head[facing=up]"), &exists),
            Requirement::Invalid
        );
        assert!(ignore("create:belt[part=middle,slope=horizontal,facing=north]"));
        assert!(ignore("minecraft:structure_void"));
        // Air is placed (it clears).
        assert!(!ignore("minecraft:air"));
        // A stair's `half` is no double block.
        assert!(!ignore("minecraft:oak_stairs[half=top,facing=north,shape=straight]"));
    }

    #[test]
    fn what_is_worth_placing_depends_on_the_world() {
        let template = template([1, 1, 1], &[([0, 0, 0], "minecraft:stone")]);
        let printer = Printer::new([0, 0, 0], Layout::new(&template, Rotation::None, Mirror::None));
        let always = |_: &Candidate| true;
        let mut dirt = |_: Pos| Existing { state: State::new("minecraft:dirt"), has_block_entity: false };
        assert!(printer.should_place([0, 0, 0], true, &mut dirt, &always));
        // Already stone, not loaded, or bedrock: no.
        let mut stone = |_: Pos| Existing { state: State::new("minecraft:stone"), has_block_entity: false };
        assert!(!printer.should_place([0, 0, 0], true, &mut stone, &always));
        assert!(!printer.should_place([0, 0, 0], false, &mut dirt, &always));
        let mut bedrock = |_: Pos| Existing { state: State::new("minecraft:bedrock"), has_block_entity: false };
        assert!(!printer.should_place([0, 0, 0], true, &mut bedrock, &always));
    }

    #[test]
    fn states_are_compared_by_what_the_schematic_says() {
        let wanted = State::parse("minecraft:oak_stairs[facing=north]");
        let found = State::parse("minecraft:oak_stairs[half=bottom,facing=north,shape=straight]");
        assert!(same_state(&wanted, &found));
        assert!(!same_state(&State::parse("minecraft:oak_stairs[facing=south]"), &found));
    }

    #[test]
    fn what_is_placed_is_made_safe() {
        let wet = placement_state(&State::parse("minecraft:oak_fence[north=true,waterlogged=true]"));
        assert_eq!(wet.get("waterlogged"), Some("false"));
        assert_eq!(placement_state(&State::parse("minecraft:water_cauldron[level=2]")), State::new("minecraft:cauldron"));
        assert_eq!(zero_age(&State::parse("minecraft:wheat[age=7]")).get("age"), Some("0"));
        assert_eq!(zero_age(&State::parse("minecraft:composter[level=5]")).get("level"), Some("0"));
        assert_eq!(zero_age(&State::parse("minecraft:stone")), State::new("minecraft:stone"));
    }

    #[test]
    fn the_checklist_lists_the_short_items_first() {
        let mut list = Checklist::default();
        list.require("minecraft:stone", 10);
        list.require("minecraft:dirt", 4);
        list.collect("minecraft:stone", 64);
        list.collect("minecraft:dirt", 1);
        assert_eq!(list.lines(), ["minecraft:dirt: 1 / 4", "minecraft:stone: 64 / 10"]);
    }

    #[test]
    fn requirements_are_counted_over_the_whole_schematic() {
        let template = template(
            [2, 1, 1],
            &[([0, 0, 0], "minecraft:stone"), ([1, 0, 0], "minecraft:oak_slab[type=double]")],
        );
        let printer = Printer::new([0, 0, 0], Layout::new(&template, Rotation::None, Mirror::None));
        let mut list = Checklist::default();
        let mut world = |_: Pos| Some(Existing::air());
        let placed = printer.count_requirements(&mut world, &|_| true, &exists, &mut list);
        assert_eq!(placed, 2);
        assert_eq!(list.required.get("minecraft:oak_slab"), Some(&2));
        assert!(!list.not_loaded);
        let mut nothing = |_: Pos| None;
        let mut list = Checklist::default();
        assert_eq!(printer.count_requirements(&mut nothing, &|_| true, &exists, &mut list), 0);
        assert!(list.not_loaded);
    }
}
