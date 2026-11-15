//! The Mechanical Saw (`SawBlock`, `SawBlockEntity`, `SawRenderer`, `CuttingRecipe`,
//! `TreeCutter`). Facing sideways it breaks what it can saw in front of it like the drill
//! (`BlockBreakingKineticBlockEntity`, shared through `machines::drill`) and fells whole trees;
//! facing up it cuts items that lie on it or that a hopper puts in, with `create:cutting`
//! recipes, moving them across the blade and out of the far side. The rules are
//! `create_kinetics::rules::{saw, saw_tree, drill}`; this is the glue.
//!
//! Known differences from Create:
//! - Stationary saws only: the saw on a contraption (`SawMovementBehaviour`) is a later slice.
//! - The recipe filter slot (`SawFilterSlot`) is out of scope: a stack with several recipes
//!   cycles through them, as an unfiltered saw does.
//! - Stonecutting recipes are baked into `create:cutting` recipes of 50 ticks
//!   (`_saw_recipes_gen.py`): `Host` cannot list the game's stonecutting recipes. A data pack that
//!   changes stonecutting is not seen. Woodcutting recipes of other mods are not cut.
//! - Items are taken from the world by a scan of what lies on the blade every 2 ticks, not when
//!   they land (`updateEntityAfterFallOn`). Belts and funnels do not hand items to the saw, and
//!   the saw does not hand its results to a belt, a funnel or a depot: it drops them
//!   (`DirectBeltInputBehaviour` is a later slice).
//! - One item at a time (`bulkCutting` is off); a package is cut like any other item.
//! - Cut items keep Create's `ProcessingTime`, `RecipeTime`, `AppliedRecipe` and `RecipeIndex`
//!   keys, but at the top of the block entity's data, and the stacks are the block entity's own
//!   item slots (`Items`: slot 0 is the input) as Horizon saves them, not an `Inventory` compound.
//! - A crafting remainder of the input (a bucket) is not returned with the results.
//! - No particles, no cutting sound, no Workshop's Most Feared advancement, and a broken saw does
//!   not drop what it holds.
//! - Felled trees show a break effect for every block, not by chance (`effectChance`).
//! - Dynamic Trees and Tinkers' slimy logs are not supported (the tag `create:slimy_logs` is
//!   empty).
//! - See `machines::drill` for the block breaking: tables of hardness, the unfelt neighbour
//!   updates, the scan for the blade's damage.

use std::collections::HashMap;
use std::sync::Mutex;

use create_kinetics::rules::drill as drill_rules;
use create_kinetics::rules::saw::{self as rules, Cut, SLOTS, Step, TOP_SCAN};
use create_kinetics::rules::saw_tree::{self as tree, Class, World};
use create_kinetics::{Dir, Kind, Node, Pos, looking};
use horizon_client::mod_api::{
    BlockEntityContainerEvent, BlockEntityDraw, BlockEntityEvent, BlockEntitySubmitEvent,
    BlockEntityTickEvent, BlockEntityTicksEvent, BlockEntityUpdateTagEvent, BlockStateId,
    Compound, ContainerAcceptsEvent, DisplayContext, EquipmentSlot, Flow, Hooks, Host, ItemStack,
    LevelHost, ModBlockEntity, ModContainer, RightClickBlockEvent, StandaloneModelsEvent,
    StateForPlacementEvent, Tag, UseItemOnEvent,
};

use super::drill::{self, Tool};
use crate::{belt, kinetic, recipes, util};

const ID: &str = "create:mechanical_saw";
/// `AllRecipeTypes.CUTTING`.
const RECIPE_TYPE: &str = "create:cutting";
/// `CreateDamageSources.saw`.
const DAMAGE_TYPE: &str = "create:mechanical_saw";
/// `AllPartialModels.SAW_BLADE_*`.
const BLADES: [&str; 6] = [
    "create:block/mechanical_saw/blade_horizontal_active",
    "create:block/mechanical_saw/blade_horizontal_inactive",
    "create:block/mechanical_saw/blade_horizontal_reversed",
    "create:block/mechanical_saw/blade_vertical_active",
    "create:block/mechanical_saw/blade_vertical_inactive",
    "create:block/mechanical_saw/blade_vertical_reversed",
];
const CENTRE: [f32; 3] = [0.5; 3];
const ORIGIN: [f32; 3] = [0.0; 3];
const UP: [f32; 3] = [0.0, 1.0, 0.0];
/// The slots kept: the input and the results (Create's 32 is far more than any recipe makes).
const STORED: usize = 9;

// Saved under the keys Create's `ProcessingInventory` and `SawBlockEntity.write` use.
const PROCESSING_TIME: &str = "ProcessingTime";
const RECIPE_TIME: &str = "RecipeTime";
const APPLIED: &str = "AppliedRecipe";
const RECIPE_INDEX: &str = "RecipeIndex";
/// The game time `ProcessingTime` was true at, for clients to run the cut on from.
const SINCE: &str = "Since";
/// What is on the blade, for the update tag: the input and the results.
const INPUT: &str = "Input";
const OUTPUT: &str = "Output";

struct Saw;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Saw)
}

// --- the block's state ----------------------------------------------------------------------------

/// `(facing, axis_along_first)` of the block at `pos`.
fn state_of(host: &dyn Host, pos: Pos) -> Option<(Dir, bool)> {
    let facing = util::facing(host, pos, "facing")?;
    let along_first = host.block_property(pos, "axis_along_first").as_deref() == Some("true");
    Some((facing, along_first))
}

fn node(facing: Dir, along_first: bool) -> Node {
    Node {
        impact: drill_rules::IMPACT,
        ..Node::passive(Kind::shafts(&rules::shaft_faces(facing, along_first)))
    }
}

// --- what it saws ----------------------------------------------------------------------------------

fn in_tag(host: &dyn Host, block: &str, tag: &str) -> bool {
    host.block_name_in_tag(block, tag).unwrap_or(false)
}

fn is_log(host: &dyn Host, block: &str) -> bool {
    // `TreeCutter.isLog`.
    in_tag(host, block, "minecraft:logs")
        || in_tag(host, block, "create:slimy_logs")
        || block == "minecraft:mushroom_stem"
}

impl Tool for Saw {
    /// `SawBlockEntity.isSawable`.
    fn allowed(&self, host: &dyn Host, _at: Pos, block: &str) -> bool {
        rules::is_sawable(
            block,
            in_tag(host, block, "minecraft:saplings"),
            is_log(host, block),
            in_tag(host, block, "minecraft:leaves"),
            in_tag(host, block, "create:roots"),
            in_tag(host, block, "create:solid_crops"),
        )
    }

    /// `SawBlockEntity.onBlockBroken`: the block breaks as a drill's does, then the tree it held up.
    fn broken(&self, host: &mut dyn Host, pos: Pos, breaking: Pos, block: &str) {
        drill::drop_block(host, breaking);
        let broken = Class {
            vertical_plant: rules::is_vertical_plant(block),
            chorus: rules::is_chorus(block),
            ..Class::NOTHING
        };
        let found = {
            let mut world = Blocks {
                host: &*host,
                seen: HashMap::new(),
            };
            tree::find_tree(&mut world, breaking, broken)
        };
        let normal = util::add(breaking, pos.map(|v| -v));
        for (at, _effect_chance) in found.in_order() {
            let Some(drops) = host.destroy_block(at, None, true) else {
                continue;
            };
            for stack in drops {
                drop_cut_item(host, at, breaking, normal, &stack);
            }
        }
    }
}

/// `SawBlockEntity.dropItemFromCutTree`: the item leaves where the block stood, thrown a little
/// towards the saw's front, the further the tree block was from the cut.
fn drop_cut_item(host: &mut dyn Host, at: Pos, breaking: Pos, towards: Pos, stack: &ItemStack) {
    let distance_squared: i32 = (0..3).map(|i| (at[i] - breaking[i]).pow(2)).sum();
    let distance = (distance_squared as f32).sqrt();
    let motion = towards.map(|v| f64::from(v as f32 * (distance / 20.0)));
    // `new ItemEntity` leaves the pickup delay at 0.
    util::spawn_item(host, util::centre(at), stack, motion, 0);
}

/// The blocks round a cut, as `TreeCutter` asks of them.
struct Blocks<'a> {
    host: &'a dyn Host,
    seen: HashMap<Pos, Class>,
}

impl World for Blocks<'_> {
    fn class(&mut self, pos: Pos) -> Class {
        if let Some(class) = self.seen.get(&pos) {
            return *class;
        }
        let host = self.host;
        let block = host.block(pos);
        let class = if drill_rules::is_air(&block) {
            Class::NOTHING
        } else {
            // `isLeaf`: an integer `distance` property, but not scaffolding's.
            let leaf = (block != "minecraft:scaffolding")
                .then(|| host.block_property(pos, "distance"))
                .flatten()
                .and_then(|value| value.parse::<i32>().ok());
            Class {
                log: is_log(host, &block),
                root: in_tag(host, &block, "create:roots"),
                leaf,
                attachment: in_tag(host, &block, "create:tree_attachments"),
                non_decaying: rules::non_decaying_distance(
                    &block,
                    in_tag(host, &block, "minecraft:wart_blocks"),
                ),
                vertical_plant: rules::is_vertical_plant(&block),
                chorus: rules::is_chorus(&block),
            }
        };
        self.seen.insert(pos, class);
        class
    }

    fn height(&self) -> i32 {
        self.host.world_height().1
    }
}

// --- the items on the blade ------------------------------------------------------------------------

fn slots_of(entity: &ModBlockEntity) -> Vec<Option<ItemStack>> {
    let mut slots = entity.items.clone();
    slots.resize(STORED, None);
    slots
}

/// Writes the slots back only when they differ: any `&mut` reach into the entity counts as a change.
fn store_slots(entity: &mut ModBlockEntity, slots: Vec<Option<ItemStack>>) -> bool {
    let mut have = entity.items.clone();
    have.resize(STORED, None);
    if slots == have {
        return false;
    }
    entity.items = slots;
    true
}

fn read_cut(entity: &ModBlockEntity) -> Cut {
    Cut {
        remaining: util::float(&entity.nbt, PROCESSING_TIME).unwrap_or(-1.0),
        duration: util::float(&entity.nbt, RECIPE_TIME).unwrap_or(0.0),
        applied: util::boolean(&entity.nbt, APPLIED),
    }
}

/// Writes what differs of the cut (and the time it holds at).
fn write_cut(entity: &mut ModBlockEntity, before: Cut, after: Cut, now: i64) {
    if after.remaining != before.remaining || util::float(&entity.nbt, PROCESSING_TIME).is_none() {
        util::set_float(&mut entity.nbt, PROCESSING_TIME, after.remaining);
    }
    if after.duration != before.duration {
        util::set_float(&mut entity.nbt, RECIPE_TIME, after.duration);
    }
    if after.applied != before.applied {
        entity.nbt.insert(APPLIED.into(), Tag::Byte(i8::from(after.applied)));
    }
    if after != before && util::long(&entity.nbt, SINCE) != now {
        entity.nbt.insert(SINCE.into(), Tag::Long(now));
    }
}

fn new_entity() -> Compound {
    let mut nbt = drill::new_breaker_entity();
    util::set_float(&mut nbt, PROCESSING_TIME, -1.0);
    util::set_float(&mut nbt, RECIPE_TIME, 0.0);
    nbt.insert(APPLIED.into(), Tag::Byte(0));
    nbt.insert(RECIPE_INDEX.into(), Tag::Int(0));
    nbt.insert(SINCE.into(), Tag::Long(0));
    nbt
}

/// The recipes that cut `input`, in the order the game holds them (`getRecipes`): `create:cutting`
/// recipes whose first ingredient takes it, left out when not for automation and when an item
/// they make is not in the game.
fn recipes_for(host: &dyn Host, input: &ItemStack) -> Vec<recipes::Processing> {
    let mut found = Vec::new();
    for recipe in host.recipes_of_type(RECIPE_TYPE) {
        if recipe.json.get("ignore_in_automation").and_then(|v| v.as_bool()) == Some(true) {
            continue;
        }
        let Some(first) = recipe.json.get("ingredients").and_then(|i| i.get(0)) else {
            continue;
        };
        if first.get("amount").is_some() {
            continue;
        }
        let Some(ingredient) = host.recipe_ingredient(&recipes::modern_ingredient(first)) else {
            continue;
        };
        if !ingredient.test(input) {
            continue;
        }
        let Some(parsed) = recipes::parse(host, &recipe.id, &recipe.json) else {
            continue;
        };
        let wanted = recipe
            .json
            .get("results")
            .and_then(|r| r.as_array())
            .map_or(0, Vec::len);
        // A recipe that makes something the game doesn't have would only swallow the input.
        if parsed.results.is_empty() || parsed.results.len() != wanted {
            continue;
        }
        found.push(parsed);
    }
    found
}

/// `SawBlockEntity.start`: a cut begins for the item in slot 0.
fn start(host: &dyn Host, input: &ItemStack, cut: &mut Cut, index: &mut usize) {
    let found = recipes_for(host, input);
    if found.is_empty() {
        cut.begin(rules::NO_RECIPE_TIME);
        return;
    }
    *index = rules::next_recipe(*index, found.len());
    let time = found[*index].processing_time;
    cut.begin(rules::start_time(Some(time), input.count));
}

/// `SawBlockEntity.applyRecipe`: the input is replaced by the recipe's results, one roll for each
/// item of the stack.
fn apply(host: &mut dyn Host, slots: &mut [Option<ItemStack>], index: &mut usize) {
    let Some(input) = slots[0].clone() else {
        return;
    };
    let found = recipes_for(host, &input);
    if found.is_empty() {
        return;
    }
    *index = rules::recipe_index(*index, found.len());
    let recipe = &found[*index];
    let dimension = host.dimension();
    let mut list: Vec<ItemStack> = Vec::new();
    for _ in 0..input.count {
        for made in recipes::roll(host, dimension, &recipe.results) {
            util::merge(host, &mut list, made);
        }
    }
    for slot in slots.iter_mut() {
        *slot = None;
    }
    for (i, stack) in list.into_iter().enumerate() {
        if i + 1 < SLOTS && i + 1 < slots.len() {
            slots[i + 1] = Some(stack);
        }
    }
}

/// The end of `SawBlockEntity.tick`: the results go on to the next belt or depot, or, with none,
/// leave over the far edge. Answers whether the cut is over (everything left); false waits for
/// room, as Create's does.
fn leave(host: &mut dyn Host, pos: Pos, slots: &mut [Option<ItemStack>], movement: [i32; 3]) -> bool {
    let face = rules::movement_face(movement);
    let next = util::add(pos, movement);
    // `DirectBeltInputBehaviour` at the next block.
    if let Some(open) = belt::can_insert_from_side(host, next, face) {
        if !open {
            return false;
        }
        for slot in slots.iter_mut() {
            let Some(stack) = slot.clone() else {
                continue;
            };
            // The belt's own item handler: a stack set down on the belt or depot at its start.
            let Some(rest) = belt::handler_insert(host, next, &stack, false) else {
                continue;
            };
            if rest.count == stack.count {
                continue;
            }
            *slot = (rest.count > 0).then_some(rest);
        }
        return slots.iter().all(Option::is_none);
    }
    // `Eject Items`: from the far edge, on top, moving away.
    let [mx, _, mz] = movement.map(f64::from);
    let centre = util::centre(pos);
    let at = [centre[0] + mx * 0.5, centre[1] + 0.5, centre[2] + mz * 0.5];
    let motion = [mx * 0.0625, 0.125, mz * 0.0625];
    for stack in slots.iter_mut().filter_map(Option::take) {
        util::spawn_item(host, at, &stack, motion, 0);
    }
    true
}

/// `DirectBeltInputBehaviour.handleInsertion` for a belt or funnel that hands the saw an item
/// (`ProcessingInventory.insertItem` into slot 0, one at a time): answers what was left.
/// Belts call it through `belt::handle_insertion`.
pub(crate) fn insert_from_side(
    host: &mut dyn Host,
    pos: Pos,
    stack: &ItemStack,
    simulate: bool,
) -> ItemStack {
    let upright = util::facing(host, pos, "facing") == Some(Dir::Up);
    let empty = host
        .mod_block_entity(pos)
        .is_some_and(|e| e.id == ID && e.items.iter().all(Option::is_none));
    if !upright || !empty || stack.count == 0 {
        return stack.clone();
    }
    let mut rest = stack.clone();
    rest.count -= 1;
    if !simulate {
        let mut put = stack.clone();
        put.count = 1;
        host.change_mod_block_entity(pos, &mut |entity| {
            let mut slots = slots_of(entity);
            slots[0] = Some(put.clone());
            store_slots(entity, slots);
        });
        host.block_entity_sync(pos);
    }
    rest
}

/// `SawBlock.updateEntityAfterFallOn` / `SawBlockEntity.insertItem`, as a scan of what lies on the
/// blade: an item entity goes in when the saw is empty, one at a time.
fn take_from_top(host: &mut dyn Host, pos: Pos, slots: &mut [Option<ItemStack>]) -> bool {
    if slots.iter().any(Option::is_some) {
        return false;
    }
    let [x, y, z] = pos.map(f64::from);
    let dimension = host.dimension();
    for item in host.item_entities(dimension, [x, y + 0.6, z], [x + 1.0, y + 1.2, z + 1.0]) {
        let [px, _, pz] = item.position;
        if !(x..x + 1.0).contains(&px) || !(z..z + 1.0).contains(&pz) {
            continue;
        }
        let on_ground = host
            .entity(item.id)
            .and_then(|entity| host.entity_on_ground(entity))
            .unwrap_or(false);
        if !on_ground {
            continue;
        }
        // The slot's limit is 1 (`ProcessingInventory.withSlotLimit`, `bulkCutting` off).
        let mut put = item.stack.clone();
        put.count = 1;
        slots[0] = Some(put);
        let mut rest = item.stack;
        rest.count -= 1;
        host.set_item_entity(dimension, item.id, rest);
        return true;
    }
    false
}

/// One server tick of an upright saw: `SawBlockEntity.tick` from `canProcess` on.
fn process(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity, rpm: f32, along_first: bool) -> bool {
    let now = util::game_time(host);
    let mut slots = slots_of(entity);
    let mut sync = false;
    if rpm != 0.0 && now.rem_euclid(TOP_SCAN) == 0 && take_from_top(host, pos, &mut slots) {
        sync = true;
    }
    let before = read_cut(entity);
    let mut cut = before;
    let mut index = util::int(&entity.nbt, RECIPE_INDEX).max(0) as usize;
    let index_before = index;
    let occupied = slots.iter().any(Option::is_some);
    // Whatever left by a belt took the cut with it (`inventory.clear()` on the next insertion).
    if !occupied && cut.remaining != -1.0 {
        cut.finish();
    }
    match cut.tick(rpm, occupied) {
        Step::Idle | Step::Counting => {}
        Step::Start => {
            if let Some(input) = &slots[0] {
                start(host, input, &mut cut, &mut index);
            }
            sync = true;
        }
        Step::Apply => {
            apply(host, &mut slots, &mut index);
            sync = true;
        }
        Step::Leave => {
            let movement = rules::item_movement(along_first, rpm);
            if leave(host, pos, &mut slots, movement) {
                cut.finish();
            }
            sync = true;
        }
    }
    if store_slots(entity, slots) {
        sync = true;
    }
    write_cut(entity, before, cut, now);
    // The time the counting `ProcessingTime` holds at moves each tick it counts.
    if cut.remaining > 0.0 && rpm != 0.0 && util::long(&entity.nbt, SINCE) != now {
        entity.nbt.insert(SINCE.into(), Tag::Long(now));
    }
    if index != index_before {
        entity.nbt.insert(RECIPE_INDEX.into(), Tag::Int(index as i32));
    }
    sync
}

// --- hooks -----------------------------------------------------------------------------------------

impl Hooks for Saw {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.extend(BLADES.map(str::to_owned));
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == ID).then(|| (ID.to_owned(), new_entity()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    /// `SawBlockEntity.registerCapabilities`: every side but the bottom takes an item into the
    /// one input slot, and nothing comes out (`extractItem` gives nothing).
    fn block_entity_container(
        &self,
        _: &dyn Host,
        event: &mut BlockEntityContainerEvent<'_>,
    ) -> Option<ModContainer> {
        (event.block == ID).then(|| ModContainer {
            slots: STORED,
            faces: [vec![], vec![0], vec![0], vec![0], vec![0], vec![0]],
            inputs: vec![0],
            outputs: vec![],
            menu: None,
        })
    }

    /// `ProcessingInventory.isItemValid`: slot 0 when empty, and only for an upright saw.
    fn container_accepts(&self, host: &dyn Host, event: &mut ContainerAcceptsEvent<'_>) -> Flow {
        if event.entity.id != ID {
            return Flow::Continue;
        }
        let upright = util::facing(host, event.pos, "facing") == Some(Dir::Up);
        let empty = event.entity.items.iter().all(Option::is_none);
        if event.slot != 0 || !upright || !empty {
            return Flow::Cancel;
        }
        Flow::Continue
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        let pos = event.pos;
        let Some((facing, along_first)) = state_of(host, pos) else {
            return;
        };
        let Some((rpm, changed)) =
            kinetic::tick(host, pos, event.entity, node(facing, along_first))
        else {
            return; // The network jammed and the block broke.
        };
        let mut sync = changed;
        if rules::is_horizontal(facing) {
            drill::run_breaker(host, pos, event.entity, rpm, facing, true, true, self);
        } else if facing == Dir::Up && process(host, pos, event.entity, rpm, along_first) {
            sync = true;
        }
        if sync {
            host.block_entity_sync(pos);
        }
        if rpm != 0.0 && drill::scan_due(host, pos) {
            drill::hurt_entities(host, pos, rpm, DAMAGE_TYPE);
        }
    }

    fn block_entity_update_tag(
        &self,
        event: &mut BlockEntityUpdateTagEvent<'_>,
    ) -> Option<Compound> {
        if event.entity.id != ID {
            return None;
        }
        let entity = event.entity;
        let mut tag = kinetic::update_tag(entity);
        // Lists even when empty: a client merges tags, and keeps what a missing key had.
        let stacks = |slots: &[Option<ItemStack>]| {
            Tag::List(slots.iter().flatten().map(util::stack_tag).collect())
        };
        let first = entity.items.first().cloned().into_iter().collect::<Vec<_>>();
        tag.insert(INPUT.into(), stacks(&first));
        tag.insert(OUTPUT.into(), stacks(entity.items.get(1..).unwrap_or(&[])));
        for key in [PROCESSING_TIME, RECIPE_TIME] {
            util::set_float(&mut tag, key, util::float(&entity.nbt, key).unwrap_or(0.0));
        }
        tag.insert(APPLIED.into(), Tag::Byte(i8::from(util::boolean(&entity.nbt, APPLIED))));
        tag.insert(SINCE.into(), Tag::Long(util::long(&entity.nbt, SINCE)));
        Some(tag)
    }

    /// `SawRenderer`: the blade in front or on top, the shaft, and the items crossing the blade.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let Some(facing) = host
            .block_state_property(event.state, "facing")
            .and_then(Dir::from_name)
        else {
            return;
        };
        let along_first = host.block_state_property(event.state, "axis_along_first") == Some("true");
        let rpm = kinetic::speed(event.entity);
        let draw = &mut *event.draw;

        // `renderBlade`: which model follows the sign of the speed.
        let horizontal = rules::is_horizontal(facing);
        let state = if rpm > 0.0 {
            0
        } else if rpm < 0.0 {
            2
        } else {
            1
        };
        let blade = BLADES[if horizontal { 0 } else { 3 } + state];
        draw.push();
        crate::draw::face_south(draw, facing);
        if !horizontal && along_first {
            draw.rotate_about(UP, 90.0, CENTRE);
        }
        host.draw_model(draw, blade, 0xFF_FFFF);
        draw.pop();

        // `renderShaft`: half a shaft out of the back, or a whole one through an upright saw.
        if horizontal {
            kinetic::draw_shaft_half(host, draw, facing.opposite(), rpm);
        } else {
            kinetic::draw_shaft(host, draw, rules::rotation_axis(facing, along_first), rpm);
        }

        let drew_items = facing == Dir::Up && draw_items(host, draw, along_first, rpm, event.entity);
        // The blade and the shaft turn or stand by themselves, as the speed's sign and the
        // update tag say; the items move with the frame.
        if !drew_items {
            draw.keep();
        }
    }

    /// `SawBlock.getStateForPlacement` (`DirectionalAxisKineticBlock`), without the shaft beside
    /// it that Create lines up with.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != ID {
            return None;
        }
        let look = looking(context.yaw, context.pitch);
        let horizontal = rules::horizontal_direction(context.yaw);
        let (facing, along_first, flipped) = rules::placement(look, horizontal, context.sneaking);
        let state = level.state_with_property(event.state, "facing", facing.name())?;
        let state =
            level.state_with_property(state, "axis_along_first", bool_name(along_first))?;
        level
            .state_with_property(state, "flipped", bool_name(flipped))
            .map(Some)
    }

    /// The Wrench turns the saw round the face clicked (`IWrenchable.getRotatedBlockState`).
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != ID {
            return Flow::Continue;
        }
        let (pos, clicked) = (event.pos, Dir::from_normal(event.face));
        util::wrench(host, pos, |host| {
            let (Some((facing, along_first)), Some(clicked)) = (state_of(host, pos), clicked)
            else {
                return;
            };
            let flipped = host.block_property(pos, "flipped").as_deref() == Some("true");
            let next = rules::wrench(facing, along_first, flipped, clicked);
            if next.0 != facing {
                host.set_block_property(pos, "facing", next.0.name());
            }
            if next.1 != along_first {
                host.set_block_property(pos, "axis_along_first", bool_name(next.1));
            }
            if next.2 != flipped {
                host.set_block_property(pos, "flipped", bool_name(next.2));
            }
        })
    }

    /// `SawBlock.useItemOn`: an empty hand on an upright saw takes everything it holds.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID || util::facing(host, pos, "facing") != Some(Dir::Up) {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        let uuid = player.uuid;
        if host.player_equipment(uuid, EquipmentSlot::MainHand).is_some() {
            return Flow::Continue;
        }
        // Sneaking with anything in a hand skips the block's own use.
        if player.sneaking && host.player_equipment(uuid, EquipmentSlot::OffHand).is_some() {
            return Flow::Continue;
        }
        let now = util::game_time(host);
        if repeated(uuid, pos, now) {
            return Flow::Cancel;
        }
        let mut taken: Vec<ItemStack> = Vec::new();
        host.change_mod_block_entity(pos, &mut |entity| {
            let mut slots = slots_of(entity);
            taken = slots.iter_mut().filter_map(Option::take).collect();
            store_slots(entity, slots);
            // `inventory.clear()`: the timer back to 0, nothing applied.
            let before = read_cut(entity);
            let mut cut = before;
            cut.clear();
            cut.remaining = -1.0;
            write_cut(entity, before, cut, now);
        });
        for stack in taken {
            // `Inventory.placeItemBackInInventory`: what does not fit drops at the player.
            if let Some(rest) = host.give_stack(uuid, stack) {
                util::spawn_item(host, player.position, &rest, [0.0; 3], 0);
            }
        }
        host.block_entity_sync(pos);
        Flow::Cancel
    }
}

fn bool_name(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

/// `SawRenderer.renderItems`: the input, or the results, moving over the blade. Answers whether
/// anything was drawn (the frame then moves with the time).
fn draw_items(
    host: &dyn Host,
    draw: &mut BlockEntityDraw,
    along_first: bool,
    rpm: f32,
    entity: &ModBlockEntity,
) -> bool {
    let input = util::stacks_of(&entity.nbt, INPUT);
    let output = util::stacks_of(&entity.nbt, OUTPUT);
    if input.is_empty() && output.is_empty() {
        return false;
    }
    let along_z = !along_first;
    let cut = Cut {
        remaining: util::float(&entity.nbt, PROCESSING_TIME).unwrap_or(-1.0),
        duration: util::float(&entity.nbt, RECIPE_TIME).unwrap_or(0.0),
        applied: util::boolean(&entity.nbt, APPLIED),
    };
    let since = (draw.game_time - util::long(&entity.nbt, SINCE)) as f32 + draw.partial;
    let moving = cut.duration != 0.0;
    let mut offset = if moving { cut.offset(rpm, since) } else { 0.0 };
    if rpm == 0.0 {
        offset = 0.5;
    }
    if (rpm < 0.0) ^ along_z {
        offset = 1.0 - offset;
    }
    let outputs = output.len();

    draw.push();
    if along_z {
        draw.rotate_about(UP, 90.0, ORIGIN);
    }
    draw.translate([if outputs <= 1 { 0.5 } else { 0.25 }, 0.0, offset]);
    draw.translate([if along_z { -1.0 } else { 0.0 }, 0.0, 0.0]);
    let mut rendered = 0;
    for (i, stack) in input.iter().chain(output.iter()).enumerate() {
        // `isGui3d`, as the depot has it: an item that is also a block.
        let block_item = host.default_state(&stack.item).is_some();
        draw.push();
        draw.translate([0.0, if block_item { 0.925 } else { 13.0 / 16.0 }, 0.0]);
        if i > 0 && outputs > 1 {
            draw.translate([(0.5 / (outputs - 1) as f32) * rendered as f32, 0.0, 0.0]);
        }
        draw.scale([0.5; 3]);
        draw.rotate_about([1.0, 0.0, 0.0], 90.0, ORIGIN);
        host.draw_item(draw, stack, DisplayContext::Fixed, None, 0);
        rendered += 1;
        draw.pop();
    }
    draw.pop();
    true
}

/// The last collection by a player: `(player, block, game time)`.
static LAST: Mutex<Option<(u128, Pos, i64)>> = Mutex::new(None);

/// Whether this player emptied this block a moment ago (the client may send the off hand's click
/// for the same use).
fn repeated(uuid: u128, pos: Pos, now: i64) -> bool {
    let mut last = LAST.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let again = matches!(*last, Some((u, p, t)) if u == uuid && p == pos && now - t <= 1);
    if !again {
        *last = Some((uuid, pos, now));
    }
    again
}
