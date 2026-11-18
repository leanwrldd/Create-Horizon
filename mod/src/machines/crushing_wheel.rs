//! Crushing Wheels (`CrushingWheelBlock`, `CrushingWheelBlockEntity`, `CrushingWheelControllerBlock`,
//! `CrushingWheelControllerBlockEntity`): two wheels two blocks apart, turning against each other,
//! make a controller block between them; items and mobs that enter it are crushed with Create's
//! `create:crushing` recipes (falling back to `create:milling`) and the results leave through the
//! controller's facing. The rules (where the controller faces, the clock, the pull) are
//! `create_kinetics::rules::crushing_wheel`; this is the glue.
//!
//! Known differences from Create:
//! - Items lying inside the controller are taken in at once by a scan (every 2 ticks), not pulled
//!   to the middle first; mobs are found by a scan every 4 ticks (`entityInside` has no host here).
//! - The controller's collision is Create's fixed floor (`CRUSHING_WHEEL_CONTROLLER_COLLISION`); Create
//!   makes it empty for the entity being crushed. A mob is pushed along the facing (not a player,
//!   whom Horizon lets only their own client move) and hurt, and moved to the exit just before the
//!   killing blow so its drops land there.
//! - Only living entities are crushed (Create also hits arrows and the like).
//! - Output never goes into a belt or depot below (`DirectBeltInputBehaviour`): items are thrown
//!   out, and a belt or depot picks them up as it picks up any item. Belts, funnels and hoppers
//!   don't feed the controller either; a hopper above does (one item at a time).
//! - The entity being crushed is forgotten on a reload, and the wheels can't refuse a placement
//!   next to a controller (`canSurvive`). No particles, no crushing sound, no advancements.
//! - The wheel's recipe is Create's mechanical crafting one, which needs the Mechanical Crafter.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use create_kinetics::generators::preferred_shaft;
use create_kinetics::rules::crushing_wheel::{
    self as rules, Clock, Phase, SLOTS,
};
use create_kinetics::{Axis, Dir, Kind, Node, Pos, looking};
use crate::compat::{
    BlockEntityContainerEvent, BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent,
    BlockEntityTicksEvent, BlockEntityUpdateTagEvent, BlockStateId, Compound,
    ContainerAcceptsEvent, Flow, Hooks, Host, ItemStack, LevelHost, ModBlockEntity, ModContainer,
    PlaceBlockEvent, StandaloneModelsEvent, StateForPlacementEvent, Tag, UseItemOnEvent,
};

use crate::{kinetic, recipes, util};

const WHEEL: &str = "create:crushing_wheel";
const CONTROLLER: &str = "create:crushing_wheel_controller";
/// `AllPartialModels.CRUSHING_WHEEL`.
const WHEEL_MODEL: &str = "create:block/crushing_wheel/block";
/// `AllRecipeTypes.CRUSHING` and `MILLING`.
const CRUSHING: &str = "create:crushing";
const MILLING: &str = "create:milling";
/// `CreateDamageSources.crush`.
const CRUSH: &str = "create:crush";
const SPEED: &str = "Speed";
const INVENTORY: &str = "Inventory";
const PROCESSING_TIME: &str = "ProcessingTime";
const RECIPE_TIME: &str = "RecipeTime";
const APPLIED_RECIPE: &str = "AppliedRecipe";
const CENTRE: [f32; 3] = [0.5; 3];
/// How often a controller looks for items and for mobs in it, in ticks.
const ITEM_SCAN: i64 = 2;
const MOB_SCAN: i64 = 4;
/// How often a controller checks its two wheels are still there.
const SUPPORT_CHECK: i64 = 20;
/// Ticks an item the controller threw out is left alone (`BypassCrushingWheel`).
const BYPASS_TICKS: i64 = 40;

struct Crushing;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Crushing)
}

// --- what the controller remembers outside its saved data ---------------------------------

#[derive(Default)]
struct Runtime {
    /// `processingEntity`.
    entity: Option<u32>,
    /// Item entities it threw out, with the game time to forget them at.
    bypass: Vec<(u32, i64)>,
}

static RUNTIME: Mutex<Option<HashMap<(&'static str, Pos), Runtime>>> = Mutex::new(None);

/// Never hold it across a call into the game.
fn runtime() -> MutexGuard<'static, Option<HashMap<(&'static str, Pos), Runtime>>> {
    RUNTIME.lock().unwrap_or_else(PoisonError::into_inner)
}

fn with_runtime<T>(dimension: &'static str, pos: Pos, f: impl FnOnce(&mut Runtime) -> T) -> T {
    let mut guard = runtime();
    f(guard
        .get_or_insert_with(HashMap::new)
        .entry((dimension, pos))
        .or_default())
}

// --- helpers --------------------------------------------------------------------------------

fn axis_of(host: &dyn Host, pos: Pos) -> Option<Axis> {
    host.block_property(pos, "axis")
        .as_deref()
        .and_then(Axis::from_name)
}

fn offset(pos: Pos, dir: Dir, steps: i32) -> Pos {
    let n = dir.normal();
    [pos[0] + n[0] * steps, pos[1] + n[1] * steps, pos[2] + n[2] * steps]
}

/// The speed a wheel's block entity last stored (`getSpeed`), read from outside its tick.
fn wheel_speed(host: &dyn Host, pos: Pos) -> f32 {
    host.mod_block_entity(pos)
        .map_or(0.0, |entity| kinetic::speed(&entity))
}

/// What `BlockState.canBeReplaced` says of the blocks that matter here.
fn replaceable(block: &str) -> bool {
    matches!(
        block,
        "minecraft:air"
            | "minecraft:cave_air"
            | "minecraft:void_air"
            | "minecraft:water"
            | "minecraft:lava"
            | "minecraft:short_grass"
            | "minecraft:tall_grass"
            | "minecraft:fern"
            | "minecraft:large_fern"
            | "minecraft:seagrass"
            | "minecraft:tall_seagrass"
            | "minecraft:vine"
            | "minecraft:dead_bush"
            | "minecraft:snow"
            | "minecraft:light"
    )
}

fn recipe_for(host: &dyn Host, stack: &ItemStack) -> Option<recipes::Processing> {
    // `findRecipe`: crushing first, then milling.
    recipes::for_stack(host, CRUSHING, stack).or_else(|| recipes::for_stack(host, MILLING, stack))
}

fn slots_of(entity: &ModBlockEntity) -> Vec<Option<ItemStack>> {
    let mut slots = entity.items.clone();
    slots.resize(SLOTS, None);
    slots
}

fn read_clock(nbt: &Compound) -> Clock {
    match util::get(nbt, INVENTORY) {
        Some(Tag::Compound(inventory)) => Clock {
            remaining: util::float(inventory, PROCESSING_TIME).unwrap_or(0.0),
            applied: util::int(inventory, APPLIED_RECIPE) != 0,
        },
        _ => Clock {
            remaining: 0.0,
            applied: false,
        },
    }
}

/// `ProcessingInventory.serializeNBT` without the slots (which are the entity's `Items`).
fn clock_tag(clock: Clock) -> Tag {
    let mut tag = Compound::new();
    tag.insert(PROCESSING_TIME.into(), Tag::Float(clock.remaining.to_bits()));
    tag.insert(RECIPE_TIME.into(), Tag::Float(0.0f32.to_bits()));
    tag.insert(APPLIED_RECIPE.into(), Tag::Byte(i8::from(clock.applied)));
    Tag::Compound(tag)
}

fn stack_has(slots: &[Option<ItemStack>]) -> bool {
    slots.iter().any(Option::is_some)
}

// --- the wheel ----------------------------------------------------------------------------

/// `CrushingWheelBlockEntity.fixControllers`: for each side across the axis, the controller
/// that should stand between this wheel and the one two blocks away (`updateControllers`).
fn fix_controllers(host: &mut dyn Host, pos: Pos, axis: Axis, rpm: f32) {
    let dimension = host.dimension();
    for side in Dir::ALL.into_iter().filter(|side| side.axis() != axis) {
        let controller = offset(pos, side, 1);
        let other_pos = offset(pos, side, 2);
        let exists = host.block(controller) == CONTROLLER;
        let other = (host.block(other_pos) == WHEEL)
            .then(|| axis_of(host, other_pos).map(|axis| (axis, wheel_speed(host, other_pos))))
            .flatten();
        let Some(wanted) = rules::controller_wanted(axis, side, rpm, other) else {
            if exists {
                // `world.setBlockAndUpdate(controllerPos, AIR)`; `onRemove` drops its contents.
                host.destroy_block(controller, None, false);
            }
            continue;
        };
        if !exists {
            if !replaceable(&host.block(controller)) {
                continue;
            }
            let Some(state) = host.default_state(CONTROLLER) else {
                continue;
            };
            host.set_block(dimension, controller, state, util::UPDATE_ALL);
        }
        let valid = if wanted.valid { "true" } else { "false" };
        if host.block_property(controller, "valid").as_deref() != Some(valid) {
            host.set_block_property(controller, "valid", valid);
        }
        if host.block_property(controller, "facing").as_deref() != Some(wanted.facing.name()) {
            host.set_block_property(controller, "facing", wanted.facing.name());
        }
    }
}

fn wheel_tick(host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
    let pos = event.pos;
    let Some(axis) = axis_of(host, pos) else {
        return;
    };
    let node = Node {
        impact: rules::IMPACT,
        ..Node::passive(Kind::Shaft(axis))
    };
    let Some((rpm, changed)) = kinetic::tick(host, pos, event.entity, node) else {
        return;
    };
    // `onSpeedChanged` and `lazyTick`, which runs every 20 ticks.
    let lazy = (util::game_time(host) + i64::from(pos[0] + pos[1] * 3 + pos[2] * 7))
        .rem_euclid(rules::LAZY_TICK)
        == 0;
    if changed || lazy {
        fix_controllers(host, pos, axis, rpm);
    }
    if changed {
        host.block_entity_sync(pos);
    }
}

/// `KineticBlockEntityRenderer` with the model `crushing_wheel/block`, turned as the blockstate
/// turns it (`blockstates/crushing_wheel.json`: x: x 90 y 90, z: x 90 y 180).
fn wheel_draw(host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
    let Some(axis) = host
        .block_state_property(event.state, "axis")
        .and_then(Axis::from_name)
    else {
        return;
    };
    let rpm = kinetic::speed(event.entity);
    // `KineticBlockEntityVisual.rotationOffset`: 22.5 degrees where the two other coordinates add up even.
    let sum: i32 = (0..3).filter(|i| *i != axis.index()).map(|i| event.pos[i]).sum();
    let phase = if sum.rem_euclid(2) == 0 { 22.5 } else { 0.0 };
    let draw = &mut *event.draw;
    draw.spin(axis.unit(), CENTRE, rpm * 0.3, phase);
    draw.push();
    // A blockstate's x and y turn clockwise seen from the axis' positive end: the opposite
    // sign of `rotate_about`'s, and y is applied after x, so it is called first.
    let (x, y) = match axis {
        Axis::Y => (0.0, 0.0),
        Axis::X => (90.0, 90.0),
        Axis::Z => (90.0, 180.0),
    };
    if y != 0.0 {
        draw.rotate_about(Axis::Y.unit(), -y, CENTRE);
    }
    if x != 0.0 {
        draw.rotate_about(Axis::X.unit(), -x, CENTRE);
    }
    host.draw_model(draw, WHEEL_MODEL, 0xFF_FFFF);
    draw.pop();
    draw.still();
    draw.keep();
}

// --- the controller -----------------------------------------------------------------------

/// Both wheels the controller stands between are still there.
fn supported(host: &dyn Host, pos: Pos) -> bool {
    Axis::ALL.into_iter().any(|along| {
        let mut ends = Dir::ALL.into_iter().filter(|side| side.axis() == along);
        let (Some(a), Some(b)) = (ends.next(), ends.next()) else {
            return false;
        };
        let wheel = |side: Dir| {
            let at = offset(pos, side, 1);
            (host.block(at) == WHEEL)
                .then(|| axis_of(host, at))
                .flatten()
                .filter(|axis| *axis != along)
        };
        matches!((wheel(a), wheel(b)), (Some(x), Some(y)) if x == y)
    })
}

/// `CrushingWheelControllerBlock.updateSpeed`: the first wheel beside it, in direction order,
/// that isn't turning about the line between them.
fn crushing_speed(host: &dyn Host, pos: Pos) -> f32 {
    for side in Dir::ALL {
        let at = offset(pos, side, 1);
        if host.block(at) != WHEEL || axis_of(host, at) == Some(side.axis()) {
            continue;
        }
        return rules::crushing_speed(wheel_speed(host, at));
    }
    0.0
}

fn centre(pos: Pos) -> [f64; 3] {
    pos.map(|v| f64::from(v) + 0.5)
}

/// `CrushingWheelControllerBlock.checkEntityForProcessing`'s player rules.
fn crushable_player(host: &mut dyn Host, id: u32, dimension: &'static str) -> bool {
    let Some(player) = host
        .server_players()
        .into_iter()
        .find(|player| u32::try_from(player.entity_id).is_ok_and(|e| e == id))
    else {
        return true;
    };
    if player.game_mode == util::CREATIVE || player.game_mode == util::SPECTATOR {
        return false;
    }
    let peaceful = host
        .level(dimension)
        .and_then(|level| host.level_difficulty(level))
        == Some("peaceful");
    !peaceful
}

/// The first item entity in the controller that it isn't leaving alone.
fn item_inside(host: &mut dyn Host, pos: Pos, now: i64) -> Option<(u32, ItemStack)> {
    let dimension = host.dimension();
    let [min, max] = rules::cell(pos, 0.0);
    let ignored: Vec<u32> = with_runtime(dimension, pos, |state| {
        state.bypass.retain(|(_, until)| *until > now);
        state.bypass.iter().map(|(id, _)| *id).collect()
    });
    host.item_entities(dimension, min, max)
        .into_iter()
        .find(|item| !ignored.contains(&item.id))
        .map(|item| (item.id, item.stack))
}

/// A mob (or player) in the controller that may be crushed.
fn mob_inside(host: &mut dyn Host, pos: Pos) -> Option<u32> {
    let dimension = host.dimension();
    let cell = rules::cell(pos, 0.0);
    let wide = rules::cell(pos, 1.0);
    for id in host.entities_in(dimension, wide[0], wide[1]) {
        let Some(entity) = host.entity(id) else {
            continue;
        };
        if host.living(entity).is_none() {
            continue;
        }
        let Some(bounds) = host.entity_bounding_box(entity) else {
            continue;
        };
        if !rules::boxes_overlap(bounds, cell) {
            continue;
        }
        if host.entity_kind(entity).as_deref() == Some("minecraft:player")
            && !crushable_player(host, id, dimension)
        {
            continue;
        }
        return Some(id);
    }
    None
}

/// `applyRecipe`: the input becomes one roll of the results per item, in slots 1 on.
fn apply_recipe(host: &mut dyn Host, slots: &mut [Option<ItemStack>]) {
    let input = slots[0].take();
    slots.iter_mut().for_each(|slot| *slot = None);
    let Some(input) = input else { return };
    let Some(recipe) = recipe_for(host, &input) else {
        return; // No recipe: the inventory is cleared and the item is gone, as in Create.
    };
    let dimension = host.dimension();
    let mut list: Vec<ItemStack> = Vec::new();
    for _ in 0..input.count {
        for made in recipes::roll(host, dimension, &recipe.results) {
            util::merge(host, &mut list, made);
        }
    }
    for (index, stack) in list.into_iter().enumerate() {
        if index + 1 < SLOTS {
            slots[index + 1] = Some(stack);
        }
    }
}

/// Throws every output out of the controller (`Eject Items`), each left alone by it afterwards.
fn eject(host: &mut dyn Host, pos: Pos, facing: Dir, slots: &mut [Option<ItemStack>], now: i64) {
    let dimension = host.dimension();
    let at = rules::out_pos(centre(pos), facing);
    let motion = rules::out_speed(facing);
    for stack in slots.iter_mut().filter_map(Option::take) {
        if let Some(id) = util::spawn_item(host, at, &stack, motion, 0) {
            with_runtime(dimension, pos, |state| state.bypass.push((id, now + BYPASS_TICKS)));
        }
    }
}

fn float_of(nbt: &Compound, key: &str) -> Option<f32> {
    util::float(nbt, key)
}

/// One tick of crushing a mob (`CrushingWheelControllerBlockEntity.tick`, the entity branch).
/// Answers whether it goes on.
fn crush_entity(host: &mut dyn Host, pos: Pos, facing: Dir, speed: f32, id: u32) -> bool {
    let dimension = host.dimension();
    let Some(entity) = host.entity(id) else {
        return false;
    };
    let Some(data) = host.entity_nbt(entity) else {
        return false;
    };
    // `!processingEntity.isAlive()`.
    if float_of(&data, "Health").is_some_and(|health| health <= 0.0) {
        return false;
    }
    let held = host
        .entity_bounding_box(entity)
        .is_some_and(|bounds| rules::boxes_overlap(bounds, rules::cell(pos, rules::HOLD_RANGE)));
    if !held {
        return false;
    }
    let centre = centre(pos);
    let at = host.entity_position(entity).unwrap_or(centre);
    let sneaking = host.entity_is_shift_key_down(entity) == Some(true);
    host.entity_set_velocity(entity, rules::pull(facing, speed, at, centre, !sneaking));

    let exit = rules::entity_out_pos(centre, facing);
    let damage = rules::CRUSHING_DAMAGE;
    let health = float_of(&data, "Health").unwrap_or(f32::MAX);
    let hurt_time = match util::get(&data, "HurtTime") {
        Some(Tag::Short(t)) => i32::from(*t),
        _ => 0,
    };
    // Moved out before it can die, so its drops land at the exit.
    if health - damage <= 0.0 && hurt_time <= 0 {
        host.entity_teleport_to(entity, exit, None);
    }
    host.hurt_entity(dimension, id, damage, CRUSH, None);
    true
}

fn controller_tick(host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
    let pos = event.pos;
    let dimension = host.dimension();
    let now = util::game_time(host);
    if now.rem_euclid(SUPPORT_CHECK) == (i64::from(pos[0] ^ pos[2])).rem_euclid(SUPPORT_CHECK)
        && !supported(host, pos)
    {
        // `CrushingWheelBlock.onRemove` takes the controller with the wheel.
        host.destroy_block(pos, None, false);
        return;
    }
    let valid = host.block_property(pos, "valid").as_deref() == Some("true");
    let facing = util::facing(host, pos, "facing").unwrap_or(Dir::Down);

    // `updateSpeed`.
    let speed = if valid { crushing_speed(host, pos) } else { 0.0 };
    let mut sync = false;
    if util::float(&event.entity.nbt, SPEED) != Some(speed) {
        util::set_float(&mut event.entity.nbt, SPEED, speed);
        sync = true;
    }

    let mut slots = slots_of(event.entity);
    let before = read_clock(&event.entity.nbt);
    let mut clock = before;
    let mut processing = with_runtime(dimension, pos, |state| state.entity);

    // An item put in by a hopper (`itemInserted`): the clock starts.
    if slots[0].is_some() && !clock.applied && clock.remaining <= 0.0 {
        clock = Clock::inserted(
            recipe_for(host, slots[0].as_ref().expect("input")).map(|recipe| recipe.processing_time),
        );
    }

    if speed != 0.0 {
        if processing.is_none() && !stack_has(&slots) {
            // `checkEntityForProcessing`: it takes the first thing in it, only while it is valid.
            if valid && now.rem_euclid(ITEM_SCAN) == 0 {
                if let Some((id, stack)) = item_inside(host, pos, now) {
                    host.set_item_entity(dimension, id, ItemStack { count: 0, ..stack.clone() });
                    clock = Clock::inserted(recipe_for(host, &stack).map(|r| r.processing_time));
                    slots[0] = Some(stack);
                }
            }
            if valid && slots[0].is_none() && now.rem_euclid(MOB_SCAN) == 0 {
                processing = mob_inside(host, pos);
                with_runtime(dimension, pos, |state| state.entity = processing);
            }
        }
        if let Some(id) = processing {
            if !crush_entity(host, pos, facing, speed, id) {
                with_runtime(dimension, pos, |state| state.entity = None);
            }
        } else if let Some(count) = slots[0].as_ref().map(|stack| stack.count) {
            match clock.tick(speed, count) {
                Phase::Wait => {}
                Phase::Apply => {
                    apply_recipe(host, &mut slots);
                    clock.recipe_applied();
                    sync = true;
                }
                Phase::Output => {
                    eject(host, pos, facing, &mut slots, now);
                    clock = Clock {
                        remaining: 0.0,
                        applied: false,
                    };
                    sync = true;
                }
            }
        } else if stack_has(&slots) {
            // The results of a recipe wait in slots 1 on, with the clock applied: output them.
            if clock.tick(speed, 1) == Phase::Output {
                eject(host, pos, facing, &mut slots, now);
                clock = Clock {
                    remaining: 0.0,
                    applied: false,
                };
                sync = true;
            }
        }
    }
    if slots != event.entity.items {
        event.entity.items = slots;
    }
    if clock != before {
        event
            .entity
            .nbt
            .insert(INVENTORY.into(), clock_tag(clock));
    }
    if sync {
        host.block_entity_sync(pos);
    }
}

// --- the hooks ----------------------------------------------------------------------------

/// `ProcessingInventory`: slot 0 takes an item when the inventory is empty and nothing is
/// being crushed, and nothing comes out to a hopper (`extractItem` gives none).
fn controller_container() -> ModContainer {
    ModContainer {
        slots: SLOTS,
        faces: [(); 6].map(|()| vec![0]),
        inputs: vec![0],
        outputs: Vec::new(),
        menu: None,
    }
}

impl Hooks for Crushing {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.push(WHEEL_MODEL.to_owned());
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        match event.block {
            WHEEL => Some((WHEEL.to_owned(), kinetic::new_entity())),
            CONTROLLER => {
                let mut nbt = Compound::new();
                util::set_float(&mut nbt, SPEED, 0.0);
                nbt.insert(INVENTORY.into(), clock_tag(Clock {
                    remaining: 0.0,
                    applied: false,
                }));
                Some((CONTROLLER.to_owned(), nbt))
            }
            _ => None,
        }
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == WHEEL || event.block == CONTROLLER {
            event.ticks = true;
        }
    }

    fn block_entity_container(
        &self,
        _: &dyn Host,
        event: &mut BlockEntityContainerEvent<'_>,
    ) -> Option<ModContainer> {
        (event.block == CONTROLLER).then(controller_container)
    }

    /// `isItemValid`: `slot == 0 && isEmpty() && processingEntity == null`.
    fn container_accepts(&self, host: &dyn Host, event: &mut ContainerAcceptsEvent<'_>) -> Flow {
        if event.entity.id != CONTROLLER {
            return Flow::Continue;
        }
        let busy = with_runtime(host.dimension(), event.pos, |state| state.entity.is_some());
        if event.slot != 0 || stack_has(&event.entity.items) || busy {
            return Flow::Cancel;
        }
        Flow::Continue
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        match &*event.entity.id {
            WHEEL => wheel_tick(host, event),
            CONTROLLER => controller_tick(host, event),
            _ => {}
        }
    }

    fn block_entity_update_tag(
        &self,
        event: &mut BlockEntityUpdateTagEvent<'_>,
    ) -> Option<Compound> {
        match &*event.entity.id {
            WHEEL => Some(kinetic::update_tag(event.entity)),
            CONTROLLER => {
                let mut tag = Compound::new();
                util::set_float(
                    &mut tag,
                    SPEED,
                    util::float(&event.entity.nbt, SPEED).unwrap_or(0.0),
                );
                Some(tag)
            }
            _ => None,
        }
    }

    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block == WHEEL {
            wheel_draw(host, event);
        }
    }

    /// `RotatedPillarKineticBlock.getStateForPlacement` without the shaft preference, which
    /// `place_block` applies: the axis the player looks along.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != WHEEL {
            return None;
        }
        let axis = looking(context.yaw, context.pitch).axis();
        level
            .state_with_property(event.state, "axis", axis.name())
            .map(Some)
    }

    /// `RotatedPillarKineticBlock.getPreferredAxis`: a wheel set down beside shaft ends takes their axis.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if &*event.stack.item != WHEEL || host.acting_player().is_none_or(|p| p.sneaking) {
            return;
        }
        let pos = event.pos;
        let preferred = preferred_shaft(|side| {
            let at = offset(pos, side, 1);
            let id = host.block(at);
            let axis = host.block_property(at, "axis");
            let facing = host.block_property(at, "facing");
            if id == WHEEL {
                return axis.as_deref().and_then(Axis::from_name).map(Kind::Shaft);
            }
            crate::Block::of(&id)?.kind(|key| match key {
                "axis" => axis.as_deref(),
                "facing" => facing.as_deref(),
                _ => None,
            })
        });
        if let Some(side) = preferred {
            host.set_block_property(pos, "axis", side.axis().name());
        }
    }

    /// The Wrench: sneak-click picks a wheel up, a click turns its axis.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != WHEEL {
            return Flow::Continue;
        }
        let pos = event.pos;
        util::wrench(host, pos, |host| {
            if let Some(axis) = axis_of(host, pos) {
                let next = Axis::ALL[(axis.index() + 1) % 3];
                host.set_block_property(pos, "axis", next.name());
            }
        })
    }
}

