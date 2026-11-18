//! The Mechanical Press in the game: its block entity's state, pressing items that lie under it,
//! items carried by a belt or lying on a depot under it, and the contents of a basin under it
//! (compacting), and drawing its head. The cycle's rules are `create_kinetics::press`; this is
//! the glue.
//!
//! `MechanicalPressBlockEntity.getRecipe` asks the sequenced assembly's next pressing step first
//! (`machines/sequenced.rs`, so a Precision Mechanism's or Sturdy Sheet's press step works
//! wherever the press works), then the `create:pressing` files below.
//!
//! Recipes are Create's own `create:pressing` files (`data/create/recipe/pressing/*.json`),
//! read through `Host::recipes_of_type`, so a data pack can add more; the basin side runs the
//! `create:compacting` recipes through `machines/basin.rs`.
//!
//! The press works in three places, as `PressingBehaviour.Mode` (`Mode` is saved as its ordinal):
//! - `WORLD`: an item entity lying on the ground in the block under it.
//! - `BELT`: the belt or depot two blocks under it holds an item in the middle of its block
//!   (`BeltPressingCallbacks`); the press runs a cycle and presses it at the bottom. The belt and
//!   the depot call [`offered`] and [`holding`]; the press's own tick does the pressing
//!   ([`press_held`]), so the order the blocks tick in does not matter.
//! - `BASIN`: a basin two blocks under it whose contents a `create:compacting` recipe takes
//!   (`MechanicalPressBlockEntity` over `BasinOperatingBlockEntity`).
//!
//! Known differences from Create (for the README):
//! - Create is told when a basin's contents change (`contentsChanged`); the press looks at the
//!   basin every five ticks while idle, and again every 200, as the mixer does.
//! - `currentRecipe` is kept as `RecipeId`, so a cycle survives a reload; Create forgets it.
//! - Crafting recipes of two by two or three by three identical ingredients (Create's
//!   `canCompress`) are not read: no mod can read vanilla crafting recipes. Only compacting
//!   recipes run in a basin.
//! - No particles, no advancement, and a press can be placed directly above a basin
//!   (`MechanicalPressBlock.canSurvive` needs a block read a placement hook doesn't have).

use create_kinetics::belt::Processing;
use create_kinetics::press::{
    self, BASIN_POLL, ENTITY_SCAN, IMPACT, InWorld, Mode, Next, PICKUP_DELAY, Press, Step, TOSS,
};
use create_kinetics::rules::basin::BACKUP_CHECK_TICKS;
use create_kinetics::rules::sequenced::Machine;
use create_kinetics::{Axis, Dir, Kind, Node, Pos};
use crate::compat::serde_json::{Value, json};
use crate::compat::{
    BlockEntityDraw, Compound, Host, ItemStack, ModBlockEntity, RandomSource, Tag,
};

use std::collections::BTreeMap;

use crate::machines::basin::{self, Kind as RecipeKind};
use crate::machines::sequenced;
use crate::{belt, depot, float, int, util};

pub const ID: &str = "create:mechanical_press";
pub const RECIPE_TYPE: &str = "create:pressing";
const HEAD: &str = "create:block/mechanical_press/head";
pub const MODELS: [&str; 1] = [HEAD];
const CENTRE: [f32; 3] = [0.5; 3];
/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;

// Saved as Create's `PressingBehaviour.write` names them.
const RUNNING: &str = "Running";
const TICKS: &str = "Ticks";
const FINISHED: &str = "Finished";
const MODE: &str = "Mode";
/// Not saved in Create (`entityScanCooldown`); kept here so the block entity holds all its state.
const SCAN: &str = "ScanCooldown";
/// The game time at which `Ticks` was true, for clients to run the head on from.
const SINCE: &str = "Since";
/// `currentRecipe` of `BasinOperatingBlockEntity`, by id.
const RECIPE: &str = "RecipeId";
/// `basinChecker.needsUpdate`: look at the basin again.
const RECHECK: &str = "Recheck";
/// What the basin looked like when last looked at (`BasinBlockEntity.contentsChanged`).
const SIGNATURE: &str = "BasinSignature";

/// The press on its network: a shaft along its facing's axis that costs [`IMPACT`] su per RPM.
pub fn node(facing: Dir) -> Node {
    Node {
        impact: IMPACT,
        ..Node::passive(kind(facing))
    }
}

pub fn kind(facing: Dir) -> Kind {
    Kind::Shaft(facing.axis())
}

pub fn new_entity(nbt: &mut Compound) {
    nbt.insert(RUNNING.into(), Tag::Byte(0));
    nbt.insert(TICKS.into(), Tag::Int(0));
    nbt.insert(FINISHED.into(), Tag::Byte(0));
    nbt.insert(MODE.into(), Tag::Int(Mode::World.id()));
    nbt.insert(SCAN.into(), Tag::Int(ENTITY_SCAN));
    nbt.insert(SINCE.into(), Tag::Long(0));
}

fn long<K: Ord + for<'a> From<&'a str>>(nbt: &BTreeMap<K, Tag>, key: &str) -> i64 {
    match nbt.get(&K::from(key)) {
        Some(Tag::Long(value)) => *value,
        _ => 0,
    }
}

fn read<K: Ord + for<'a> From<&'a str>>(nbt: &BTreeMap<K, Tag>) -> Press {
    Press {
        running: int(nbt, RUNNING) != 0,
        mode: Mode::from_id(int(nbt, MODE)),
        ticks: int(nbt, TICKS),
        finished: int(nbt, FINISHED) != 0,
        scan_cooldown: if nbt.contains_key(&K::from(SCAN)) {
            int(nbt, SCAN)
        } else {
            ENTITY_SCAN
        },
    }
}

/// Writes what changed. Even reaching the block entity's fields mutably counts as a change
/// (its `DerefMut` bumps the revision), so nothing is touched when nothing changed.
fn write(entity: &mut ModBlockEntity, old: Press, new: Press) {
    if old == new {
        return;
    }
    let nbt = &mut entity.nbt;
    if old.running != new.running {
        nbt.insert(RUNNING.into(), Tag::Byte(i8::from(new.running)));
    }
    if old.mode != new.mode {
        nbt.insert(MODE.into(), Tag::Int(new.mode.id()));
    }
    if old.ticks != new.ticks {
        nbt.insert(TICKS.into(), Tag::Int(new.ticks));
    }
    if old.finished != new.finished {
        nbt.insert(FINISHED.into(), Tag::Byte(i8::from(new.finished)));
    }
    if old.scan_cooldown != new.scan_cooldown {
        nbt.insert(SCAN.into(), Tag::Int(new.scan_cooldown));
    }
}

fn recipe_id<K: Ord + for<'a> From<&'a str>>(nbt: &BTreeMap<K, Tag>) -> Option<String> {
    match nbt.get(&K::from(RECIPE)) {
        Some(Tag::String(id)) => Some(id.to_string_lossy()).filter(|id| !id.is_empty()),
        _ => None,
    }
}

/// One server tick of the press at `pos` turning at `rpm`. `speed_changed` says the network
/// gave it a new speed this tick. Answers whether clients must be told.
pub fn tick(
    host: &mut dyn Host,
    pos: Pos,
    entity: &mut ModBlockEntity,
    rpm: f32,
    speed_changed: bool,
) -> bool {
    let now = util::game_time(host);
    let basin_pos = [pos[0], pos[1] - 2, pos[2]];
    let old = read(&entity.nbt);
    let mut state = old;
    let mut sync = false;
    let mut recheck = util::boolean(&entity.nbt, RECHECK);

    // `BasinOperatingBlockEntity.tick`: the basin was emptied or removed, the head goes up and
    // nothing else happens this tick. A press working on a basin that is no longer there stops
    // the same way (nothing here is told of a block entity going).
    let flagged = util::boolean(&entity.nbt, basin::OPERATOR_FLAG);
    if flagged || (state.running && state.mode == Mode::Basin && !basin::is_basin(host, basin_pos))
    {
        state.basin_removed();
        if flagged {
            entity.nbt.insert(basin::OPERATOR_FLAG.into(), Tag::Byte(0));
        }
        write(entity, old, state);
        return true;
    }

    // `onSpeedChanged`: look at the basin again.
    if speed_changed {
        recheck = true;
    }
    // Create hears of a change from the basin; here the basin is asked now and then.
    if !state.running && rpm != 0.0 && basin::is_basin(host, basin_pos) {
        let backup = now.rem_euclid(BACKUP_CHECK_TICKS) == 0;
        if backup || now.rem_euclid(BASIN_POLL) == 0 {
            if let Some(signature) = basin::signature(host, basin_pos) {
                let kept = match util::get(&entity.nbt, SIGNATURE) {
                    Some(Tag::Long(v)) => Some(*v as u64),
                    _ => None,
                };
                if kept != Some(signature) {
                    entity.nbt.insert(SIGNATURE.into(), Tag::Long(signature as i64));
                }
                if kept != Some(signature) || backup {
                    recheck = true;
                }
            }
        }
    }
    // `updateBasin`: a basin whose contents a compacting recipe takes starts the head. The check
    // is spent whatever it finds (`DeferralBehaviour`: the callback always answers true).
    if recheck {
        recheck = false;
        if press::looks_at_basin(rpm, state.running, basin::can_continue(host, basin_pos))
            && let Some(found) = basin::matching(host, basin_pos, RecipeKind::Compacting)
                .into_iter()
                .next()
            && press::may_start_on_basin(state.running, state.ticks)
        {
            let id = Tag::String(found.id.as_str().into());
            if util::differs(&entity.nbt, RECIPE, &id) {
                entity.nbt.insert(RECIPE.into(), id);
            }
            state.start_in(Mode::Basin);
            sync = true;
        }
    }

    match state.tick(rpm) {
        Step::Wait => {}
        Step::Scan => {
            // `PressingBehaviour.tick`: with a belt, a depot or a basin two blocks down, the
            // items on the ground are not its business.
            if !handler_below(host, pos) && something_to_press(host, pos) {
                state.start();
                sync = true;
            }
        }
        Step::Apply => {
            sync = true;
            match state.mode {
                Mode::World => {
                    if press_items(host, pos) {
                        state.pressed();
                    }
                }
                Mode::Belt => press_held(host, pos),
                Mode::Basin => apply_basin(host, basin_pos, recipe_id(&entity.nbt)),
            }
            activation_sound(host, pos, rpm);
        }
        Step::Finished => {
            sync = true;
            // `onPressingCompleted`.
            let recipe = recipe_id(&entity.nbt).and_then(|id| basin::recipe_by_id(host, &id));
            let matches = state.mode == Mode::Basin
                && recipe
                    .as_ref()
                    .is_some_and(|recipe| basin::matches(host, basin_pos, recipe));
            let can_continue = matches && basin::can_continue(host, basin_pos);
            match press::after_cycle(state.mode, matches, can_continue) {
                Next::Again => state.start_in(Mode::Basin),
                Next::Recheck => recheck = true,
            }
        }
    }

    if recheck != util::boolean(&entity.nbt, RECHECK) {
        entity.nbt.insert(RECHECK.into(), Tag::Byte(i8::from(recheck)));
    }
    write(entity, old, state);
    // Clients run the head from `Ticks` at `Since`: renew the pair whenever the progress moved,
    // so the update tag is right whenever it is made.
    if state.running && (state.ticks != old.ticks || sync || speed_changed) && long(&entity.nbt, SINCE) != now {
        entity.nbt.insert(SINCE.into(), Tag::Long(now));
    }
    sync || (state.running && speed_changed)
}

/// What clients need: the speed is added by the caller.
pub fn update_tag(nbt: &Compound, tag: &mut Compound) {
    let state = read(nbt);
    tag.insert(RUNNING.into(), Tag::Byte(i8::from(state.running)));
    tag.insert(MODE.into(), Tag::Int(state.mode.id()));
    tag.insert(TICKS.into(), Tag::Int(state.ticks));
    tag.insert(SINCE.into(), Tag::Long(long(nbt, SINCE)));
}

// --- the press and the belts and depots under it (`BeltPressingCallbacks`) ---------------------

fn press_entity(host: &dyn Host, pos: Pos) -> Option<ModBlockEntity> {
    if host.block(pos) != ID {
        return None;
    }
    host.mod_block_entity(pos).filter(|entity| entity.id == ID)
}

/// `BeltPressingCallbacks.onItemReceived` for `stack`, an item that has come to the middle of
/// the block two under the press at `press`: starts a cycle if the press takes the item. `None`
/// where there is no press there (no processor works over that block).
pub(crate) fn offered(host: &mut dyn Host, press: Pos, stack: &ItemStack) -> Option<Processing> {
    let entity = press_entity(host, press)?;
    let rpm = float(&entity.nbt, crate::SPEED).unwrap_or(0.0);
    let running = read(&entity.nbt).running;
    // Looked up only when it can matter.
    let recipe = rpm != 0.0 && !running && recipe_for(host, stack).is_some();
    if press::starts_on_item(rpm, running, recipe) {
        let now = util::game_time(host);
        host.change_mod_block_entity(press, &mut |entity| {
            let old = read(&entity.nbt);
            let mut state = old;
            state.start_in(Mode::Belt);
            write(entity, old, state);
            entity.nbt.insert(SINCE.into(), Tag::Long(now));
        });
        host.block_entity_sync(press);
    }
    Some(press::on_item_received(rpm, running, recipe))
}

/// `BeltPressingCallbacks.whenItemHeld`, less the pressing itself: whether the press at `press`
/// goes on holding an item it caught. `None` where there is no press (the item is let go).
pub(crate) fn holding(host: &dyn Host, press: Pos) -> Option<Processing> {
    let entity = press_entity(host, press)?;
    let rpm = float(&entity.nbt, crate::SPEED).unwrap_or(0.0);
    Some(press::while_item_held(rpm, read(&entity.nbt).running))
}

/// The head is down over a belt or a depot: presses the item held in the middle of the block two
/// under the press (`BeltPressingCallbacks.whenItemHeld` at `runningTicks == CYCLE / 2`).
fn press_held(host: &mut dyn Host, pos: Pos) {
    let target = [pos[0], pos[1] - 2, pos[2]];
    let dimension = host.dimension();
    // `tryProcessOnBelt`: the results for `stack`, one roll per item.
    let mut results = |host: &mut dyn Host, stack: &ItemStack| -> Option<Vec<ItemStack>> {
        let recipe = recipe_for(host, stack)?;
        Some(made_from(host, dimension, stack.count, &recipe))
    };
    let name = host.block(target);
    if name == belt::ID {
        belt::process_held(host, target, &mut results);
    } else if name == depot::ID {
        depot::process_held(host, target, &mut results);
    }
}

/// `PressingBehaviour.tick`'s test for a handler under the press: the belt or depot two blocks
/// down (`TransportedItemStackHandlerBehaviour`) or a basin.
fn handler_below(host: &dyn Host, pos: Pos) -> bool {
    let below = [pos[0], pos[1] - 2, pos[2]];
    let name = host.block(below);
    name == belt::ID || name == depot::ID || basin::is_basin(host, below)
}

// --- the basin (`MechanicalPressBlockEntity.tryProcessInBasin`) --------------------------------

/// The head is down over a basin: applies the recipe the cycle started for. Nothing to apply
/// (a reload lost it, the basin changed) does nothing, as Create's `currentRecipe == null` does.
fn apply_basin(host: &mut dyn Host, basin_pos: Pos, id: Option<String>) {
    if let Some(recipe) = id.and_then(|id| basin::recipe_by_id(host, &id)) {
        basin::apply(host, basin_pos, &recipe);
    }
}

/// `AllSoundEvents.MECHANICAL_PRESS_ACTIVATION` (and `_ON_BELT` over wool, which a belt is).
fn activation_sound(host: &mut dyn Host, pos: Pos, rpm: f32) {
    let name = host.block([pos[0], pos[1] - 2, pos[2]]);
    let at = util::centre(pos);
    if name == belt::ID || name.ends_with("_wool") || name.ends_with("_carpet") {
        host.play_sound("minecraft:block.wool.hit", at, BLOCKS, 0.75, 1.0);
        host.play_sound("minecraft:entity.item.break", at, BLOCKS, 0.15, 0.75);
    } else {
        // `playOnServer(level, pos, .5f, .75f + |speed| / 1024)`.
        let pitch = 0.75 + rpm.abs() / 1024.0;
        host.play_sound("minecraft:block.anvil.land", at, BLOCKS, 0.125 * 0.5, pitch);
        host.play_sound("minecraft:entity.item.break", at, BLOCKS, 0.5 * 0.5, pitch);
    }
}

// --- pressing items in the world (`PressingBehaviour.tick` and `applyInWorld`) ----------------

/// A box in blocks: min and max corners.
type Box3 = ([f64; 3], [f64; 3]);

/// The block under the press, shrunk by `inset` on every side.
fn below(pos: Pos, inset: f64) -> Box3 {
    let [x, y, z] = pos.map(f64::from);
    (
        [x + inset, y - 1.0 + inset, z + inset],
        [x + 1.0 - inset, y - inset, z + 1.0 - inset],
    )
}

fn on_ground(host: &dyn Host, id: u32) -> bool {
    host.entity(id)
        .and_then(|entity| host.entity_on_ground(entity))
        .unwrap_or(false)
}

/// The scan: an item on the ground under the press that a recipe takes. Create looks a bit
/// inside the block (`new AABB(worldPosition.below()).deflate(.125f)`).
fn something_to_press(host: &mut dyn Host, pos: Pos) -> bool {
    let (min, max) = below(pos, 0.125);
    let dimension = host.dimension();
    host.item_entities(dimension, min, max)
        .into_iter()
        .any(|item| on_ground(host, item.id) && recipe_for(host, &item.stack).is_some())
}

/// The head is down: presses the first item on the ground under it that a recipe takes, or
/// every one with Create's `bulkPressing`. Answers whether any item lay there on the ground.
fn press_items(host: &mut dyn Host, pos: Pos) -> bool {
    let (min, max) = below(pos, 0.0);
    let dimension = host.dimension();
    let mut found = false;
    for item in host.item_entities(dimension, min, max) {
        if !on_ground(host, item.id) {
            continue;
        }
        found = true;
        if let Some(recipe) = recipe_for(host, &item.stack) {
            press_one(
                host,
                dimension,
                item.id,
                item.position,
                item.stack,
                &recipe,
            );
        }
        if !press::BULK {
            break;
        }
    }
    found
}

/// `MechanicalPressBlockEntity.tryProcessInWorld` with the recipe's results for one item.
fn press_one(
    host: &mut dyn Host,
    dimension: &'static str,
    id: u32,
    at: [f64; 3],
    mut stack: ItemStack,
    recipe: &Recipe,
) {
    match press::in_world(stack.count) {
        InWorld::Convert => {
            // The whole stack, one roll per item (`RecipeApplier.applyRecipeOn`).
            let made = made_from(host, dimension, stack.count, recipe);
            let motion = host
                .entity(id)
                .and_then(|entity| host.entity_velocity(entity))
                .unwrap_or([0.0; 3]);
            let mut made = made.into_iter();
            let first = made
                .next()
                .unwrap_or_else(|| ItemStack::new("minecraft:air", 0));
            host.set_item_entity(dimension, id, first);
            for extra in made {
                spawn(host, dimension, at, extra, motion, 0);
            }
        }
        InWorld::TakeOne => {
            for made in roll_once(host, dimension, recipe) {
                let motion = toss(host, dimension);
                spawn(host, dimension, at, made, motion, PICKUP_DELAY);
            }
            stack.count -= 1;
            host.set_item_entity(dimension, id, stack);
        }
    }
}

/// `RecipeApplier.applyRecipeOn(level, stack, recipe, ...)`: what `count` items of the recipe's
/// input turn into, one roll of the results each, equal stacks topped up as they come.
fn made_from(host: &mut dyn Host, dimension: &str, count: u32, recipe: &Recipe) -> Vec<ItemStack> {
    let mut made: Vec<ItemStack> = Vec::new();
    for _ in 0..count {
        for piece in roll_once(host, dimension, recipe) {
            merge(host, &mut made, piece);
        }
    }
    made
}

/// Adds `stack` to `made`, topping up equal stacks first (`RecipeApplier.applyRecipeOn`).
fn merge(host: &dyn Host, made: &mut Vec<ItemStack>, mut stack: ItemStack) {
    let max = host.max_stack_size(&stack.item).unwrap_or(64);
    for have in made.iter_mut() {
        if stack.count == 0 {
            return;
        }
        let mut probe = stack.clone();
        probe.count = have.count;
        if probe != *have {
            continue;
        }
        let moved = max.saturating_sub(have.count).min(stack.count);
        have.count += moved;
        stack.count -= moved;
    }
    if stack.count > 0 {
        made.push(stack);
    }
}

/// `VecHelper.offsetRandomly(Vec3.ZERO, level.random, .05f)`.
fn toss(host: &mut dyn Host, dimension: &str) -> [f64; 3] {
    let Some(random) = host.level_random(dimension) else {
        return [0.0; 3];
    };
    [(); 3].map(|()| f64::from(random.next_float() - 0.5) * 2.0 * TOSS)
}

fn spawn(
    host: &mut dyn Host,
    dimension: &str,
    at: [f64; 3],
    stack: ItemStack,
    motion: [f64; 3],
    pickup_delay: i32,
) {
    let mut item = Compound::new();
    item.insert("id".into(), Tag::String((&*stack.item).into()));
    item.insert("count".into(), Tag::Int(stack.count as i32));
    let mut nbt = Compound::new();
    nbt.insert("Item".into(), Tag::Compound(item));
    nbt.insert(
        "Motion".into(),
        Tag::List(motion.map(|v| Tag::Double(v.to_bits())).to_vec()),
    );
    nbt.insert("PickupDelay".into(), Tag::Short(pickup_delay as i16));
    if let Some(id) = host.summon(dimension, "minecraft:item", at, nbt) {
        // The summon's `Item` names only the id and count: this carries the components too.
        host.set_item_entity(dimension, id, stack);
    }
}

// --- recipes -----------------------------------------------------------------------------

/// One of a recipe's results (`ProcessingOutput`).
#[derive(Clone)]
struct Output {
    stack: ItemStack,
    chance: f32,
}

/// The recipe `getRecipe` found for an item: a sequenced assembly's step, or a pressing recipe's
/// results.
enum Recipe {
    Plain(Vec<Output>),
    Step(sequenced::Step),
}

/// One roll of the recipe's results for one item (`ProcessingRecipe.rollResults`); a sequenced
/// step's first result is the item's next form.
fn roll_once(host: &mut dyn Host, dimension: &str, recipe: &Recipe) -> Vec<ItemStack> {
    match recipe {
        Recipe::Plain(results) => roll(host, dimension, results),
        Recipe::Step(step) => step.roll(host),
    }
}

fn roll(host: &mut dyn Host, dimension: &str, results: &[Output]) -> Vec<ItemStack> {
    results
        .iter()
        .filter(|output| {
            output.chance >= 1.0
                || host
                    .level_random(dimension)
                    .is_some_and(|random| press::rolls(output.chance, random.next_float()))
        })
        .map(|output| output.stack.clone())
        .collect()
}

/// An ingredient as a recipe for Minecraft 1.21.1 writes it (`{"tag": "c:ingots/iron"}`,
/// `{"item": ...}`), turned into the form Horizon reads (`"#c:ingots/iron"`); lists element by
/// element. Anything else is passed on as it is.
fn modern_ingredient(json: &Value) -> Value {
    match json {
        Value::Object(o) if o.len() == 1 => match (o.get("tag"), o.get("item")) {
            (Some(Value::String(tag)), _) => json!(format!("#{tag}")),
            (_, Some(Value::String(item))) => json!(item),
            _ => json.clone(),
        },
        Value::Array(a) => Value::Array(a.iter().map(modern_ingredient).collect()),
        _ => json.clone(),
    }
}

/// The first `create:pressing` recipe that takes `stack`, as its results for one item
/// (`AllRecipeTypes.PRESSING.find`). Create's pressing recipes have one ingredient.
fn recipe_for(host: &dyn Host, stack: &ItemStack) -> Option<Recipe> {
    // `SequencedAssemblyRecipe.getRecipe(level, item, PRESSING, PressingRecipe.class)` first.
    if let Some(step) = sequenced::find(host, stack, Machine::Pressing, &|_| true) {
        return Some(Recipe::Step(step));
    }
    host.recipes_of_type(RECIPE_TYPE).iter().find_map(|recipe| {
        let json = &recipe.json;
        let ingredient =
            host.recipe_ingredient(&modern_ingredient(json.get("ingredients")?.get(0)?))?;
        if !ingredient.test(stack) {
            return None;
        }
        let results: Vec<Output> = json
            .get("results")?
            .as_array()?
            .iter()
            .filter_map(|result| {
                // Only what `recipe_result` reads: `chance` is Create's.
                let mut plain = result.as_object()?.clone();
                let chance = plain
                    .remove("chance")
                    .and_then(|c| c.as_f64())
                    .unwrap_or(1.0) as f32;
                let stack = host.recipe_result(&Value::Object(plain))?;
                Some(Output { stack, chance })
            })
            .collect();
        (!results.is_empty()).then_some(Recipe::Plain(results))
    })
}

// --- drawing (`MechanicalPressRenderer`) ----------------------------------------------------

/// The shaft turning through the press, and the head at its height in the cycle.
pub fn draw(host: &dyn Host, draw: &mut BlockEntityDraw, pos: Pos, facing: Dir, nbt: &Compound) {
    let rpm = float(nbt, crate::SPEED).unwrap_or(0.0);
    let axis = facing.axis();
    // `KineticBlockEntityRenderer.getRotationOffsetForPosition`, so the shaft lines up with its neighbours.
    let phase = crate::machines::encased::shaft_phase(axis, pos);
    crate::draw::spin(draw, axis.unit(), rpm * 0.3, phase, float(nbt, crate::ponder::ANGLE));
    host.draw_block(draw, "create:shaft", &[("axis", axis.name())]);
    draw.still();

    let state = read(nbt);
    let scene = float(nbt, crate::ponder::PRESS_PROGRESS);
    let offset = if let Some(progress) = scene {
        // A Ponder scene says how far into the stroke the head is, and in which mode.
        let mode = press::Mode::from_id(int(nbt, crate::ponder::PRESS_MODE));
        press::head_offset(true, progress * press::CYCLE as f32) * mode.head_travel()
    } else if state.running {
        let now = draw.game_time as f64 + f64::from(draw.partial);
        let ticks = press::client_ticks(state.ticks, long(nbt, SINCE), now, rpm);
        // `getRenderedHeadOffset(partialTicks) * mode.headOffset`.
        press::head_offset(true, ticks) * state.mode.head_travel()
    } else {
        0.0
    };
    draw.push();
    draw.translate([0.0, -offset, 0.0]);
    // Turned as the block's own model is (`blockstates/mechanical_press.json`: y 0, 90, 180,
    // 270 for north, east, south, west); a blockstate's y turns clockwise seen from above.
    let turn = match facing {
        Dir::East => 90.0,
        Dir::South => 180.0,
        Dir::West => 270.0,
        _ => 0.0,
    };
    draw.rotate_about(Axis::Y.unit(), -turn, CENTRE);
    host.draw_model(draw, HEAD, 0xFF_FFFF);
    draw.pop();
    // A head in motion is drawn each frame; one at rest is kept with the turning shaft.
    if !state.running && scene.is_none() {
        draw.keep();
    }
}
