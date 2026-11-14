//! The Mechanical Press in the game: its block entity's state, pressing items that lie under
//! it, and drawing its head. The cycle's rules are `create_kinetics::press`; this is the glue.
//!
//! Recipes are Create's own `create:pressing` files (`data/create/recipe/pressing/*.json`),
//! read through `Host::recipes_of_type`, so a data pack can add more.

use create_kinetics::press::{
    self, ENTITY_SCAN, IMPACT, InWorld, PICKUP_DELAY, Press, Step, TOSS, WORLD_HEAD_TRAVEL,
};
use create_kinetics::{Axis, Dir, Kind, Node, Pos};
use horizon_client::mod_api::serde_json::{Value, json};
use horizon_client::mod_api::{
    BlockEntityDraw, Compound, Host, ItemStack, ModBlockEntity, RandomSource, Tag,
};

use std::collections::BTreeMap;

use crate::{float, int};

pub const RECIPE_TYPE: &str = "create:pressing";
const HEAD: &str = "create:block/mechanical_press/head";
pub const MODELS: [&str; 1] = [HEAD];
const CENTRE: [f32; 3] = [0.5; 3];

// Saved as Create's `PressingBehaviour.write` names them.
const RUNNING: &str = "Running";
const TICKS: &str = "Ticks";
const FINISHED: &str = "Finished";
/// Not saved in Create (`entityScanCooldown`); kept here so the block entity holds all its state.
const SCAN: &str = "ScanCooldown";
/// The game time at which `Ticks` was true, for clients to run the head on from.
const SINCE: &str = "Since";

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

/// One server tick of the press at `pos` turning at `rpm`. `speed_changed` says the network
/// gave it a new speed this tick. Answers whether clients must be told.
pub fn tick(
    host: &mut dyn Host,
    pos: Pos,
    entity: &mut ModBlockEntity,
    rpm: f32,
    speed_changed: bool,
) -> bool {
    let old = read(&entity.nbt);
    let mut state = old;
    let mut sync = false;
    match state.tick(rpm) {
        Step::Wait => {}
        Step::Scan => {
            if something_to_press(host, pos) {
                state.start();
                sync = true;
            }
        }
        Step::Apply => {
            if press_items(host, pos) {
                state.pressed();
            }
            sync = true;
        }
        Step::Finished => sync = true,
    }
    write(entity, old, state);
    // Clients run the head from `Ticks` at `Since`: renew the pair whenever the progress moved,
    // so the update tag is right whenever it is made.
    if state.running && (state.ticks != old.ticks || sync || speed_changed) {
        let now = game_time(host);
        if long(&entity.nbt, SINCE) != now {
            entity.nbt.insert(SINCE.into(), Tag::Long(now));
        }
    }
    sync || (state.running && speed_changed)
}

fn game_time(host: &dyn Host) -> i64 {
    host.level(host.dimension())
        .and_then(|level| level.game_time(host))
        .unwrap_or(0)
}

/// What clients need: the speed is added by the caller.
pub fn update_tag(nbt: &Compound, tag: &mut Compound) {
    let state = read(nbt);
    tag.insert(RUNNING.into(), Tag::Byte(i8::from(state.running)));
    tag.insert(TICKS.into(), Tag::Int(state.ticks));
    tag.insert(SINCE.into(), Tag::Long(long(nbt, SINCE)));
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
        if let Some(results) = recipe_for(host, &item.stack) {
            press_one(
                host,
                dimension,
                item.id,
                item.position,
                item.stack,
                &results,
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
    results: &[Output],
) {
    match press::in_world(stack.count) {
        InWorld::Convert => {
            // The whole stack, one roll per item (`RecipeApplier.applyRecipeOn`).
            let mut made: Vec<ItemStack> = Vec::new();
            for _ in 0..stack.count {
                for piece in roll(host, dimension, results) {
                    merge(host, &mut made, piece);
                }
            }
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
            for made in roll(host, dimension, results) {
                let motion = toss(host, dimension);
                spawn(host, dimension, at, made, motion, PICKUP_DELAY);
            }
            stack.count -= 1;
            host.set_item_entity(dimension, id, stack);
        }
    }
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
fn recipe_for(host: &dyn Host, stack: &ItemStack) -> Option<Vec<Output>> {
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
        (!results.is_empty()).then_some(results)
    })
}

// --- drawing (`MechanicalPressRenderer`) ----------------------------------------------------

/// The shaft turning through the press, and the head at its height in the cycle.
pub fn draw(host: &dyn Host, draw: &mut BlockEntityDraw, facing: Dir, nbt: &Compound) {
    let rpm = float(nbt, crate::SPEED).unwrap_or(0.0);
    let axis = facing.axis();
    draw.spin(axis.unit(), CENTRE, rpm * 0.3, 0.0);
    host.draw_block(draw, "create:shaft", &[("axis", axis.name())]);
    draw.still();

    let state = read(nbt);
    let offset = if state.running {
        let now = draw.game_time as f64 + f64::from(draw.partial);
        let ticks = press::client_ticks(state.ticks, long(nbt, SINCE), now, rpm);
        press::head_offset(true, ticks) * WORLD_HEAD_TRAVEL
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
    if !state.running {
        draw.keep();
    }
}
