//! The Andesite and Brass Funnels, and the funnels on a belt or a depot (`FunnelBlock`,
//! `BeltFunnelBlock`, `AbstractFunnelBlock`, `FunnelBlockEntity`, `BeltFunnelInteractionHandler`,
//! `FunnelRenderer`): a funnel takes items out of the inventory behind it (`extracting`) and puts
//! them out in front, or takes in the items that fall into it; on a belt or a depot it puts items
//! onto it or takes them off it. The rules are `create_kinetics::rules::funnel` and
//! `rules::chute`; this is the glue.
//!
//! Known differences from Create:
//! - A funnel's state follows its neighbours (the redstone signal every other tick; the belt or
//!   depot below, turning a funnel into a belt funnel and back, every 4 ticks and when it is
//!   placed) rather than on each neighbour update. A funnel placed against another funnel is not
//!   refused (`AbstractFunnelBlock.canSurvive`: the placement hook can't read neighbours), and a
//!   funnel turning into a belt funnel keeps its filter by copying it over.
//! - A funnel looks at an inventory every other tick for something to take (Create waits for the
//!   inventory to change). An item entity in a funnel's block is caught by a look every 2 ticks,
//!   not as it touches it (`entityInside`). Items waiting in front of an extracting funnel are
//!   looked for in its block and the one below it.
//! - Belt funnels work on the items of a belt once a tick from the funnel's side, nudging an item
//!   back to the funnel's entry when it has passed it, rather than inside the belt's own tick
//!   (`BeltFunnelInteractionHandler.checkForFunnels`). A blocked item can jitter a little.
//! - Putting items into a container is done for real when asked (see `chute_inv`), so Create's
//!   "exactly N" belt funnels put in what fits instead of nothing when it can't all go in.
//! - An inventory is an `Items` list (see `chute_inv`); a chute behind a funnel is not one.
//! - Brass filter: Create's filter slot (a value box, `machines/value_box.rs`): right-click it with an item,
//!   an empty hand clears it, hold for the amount where the funnel extracts. Andesite funnels move one item
//!   at a time and take anything.
//! - The flaps are drawn (swinging out and settling as `FlapStuffs` moves them) but make no
//!   sound (`AllSoundEvents.FUNNEL_FLAP`); no advancements, no Smart Observer, no contraption
//!   movement behaviour (`FunnelMovementBehaviour`), no waterlogging.
//! - Funnels on an upright saw (`SawBlockEntity` allows belt funnels) are not belt funnels.
//! - Collision boxes are cut to the block's own cell.

use create_kinetics::belt::Item;
use create_kinetics::rules::chute::{self as chute, Mode};
use create_kinetics::rules::funnel::{self as rules, Activity, Kind, Reaction, Shape};
use create_kinetics::{Dir, Pos};
use horizon_client::mod_api::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, BlockStateId, Compound, Flow, Hooks, Host, ItemStack,
    LevelHost, ModBlockEntity, PlaceBlockEvent, StandaloneModelsEvent,
    StateForPlacementEvent, Tag, UseItemOnEvent,
};

use super::chute_inv::{self as inv, Filter};
use crate::belt::{self, BeltInput};
use crate::{float, int, util};

pub const ANDESITE: &str = "create:andesite_funnel";
pub const BRASS: &str = "create:brass_funnel";
pub const ANDESITE_BELT: &str = "create:andesite_belt_funnel";
pub const BRASS_BELT: &str = "create:brass_belt_funnel";
const MECHANICAL_ARM: &str = "create:mechanical_arm";
const FLAP: &str = "create:block/funnel/flap";
const BELT_FLAP: &str = "create:block/belt_funnel/flap";

/// `TransferCooldown`, as `FunnelBlockEntity.write` keeps it.
const COOLDOWN: &str = "TransferCooldown";
/// Which way the flap last moved (1 out, -1 in) and when, for clients.
const FLAP_KEY: &str = "Flap";
const FLAP_TICK: &str = "FlapTick";
/// Ticks between tries at an inventory.
const ATTEMPT_RATE: i64 = 2;
/// Ticks between a funnel's looks at the blocks around it.
const STATE_RATE: i64 = 4;
/// How long clients keep drawing a flap settling (`flap_value` is nearly 0 by then).
const FLAP_TICKS: f32 = 120.0;

struct Funnels;

pub fn hooks() -> Box<dyn Hooks> {
    // A belt ends at a funnel that opens upwards.
    belt::register_input(BeltInput {
        claims: |block| is_funnel(block),
        can_insert: |host, pos, _side| {
            view(host, pos).is_some_and(|v| {
                rules::supports_direct_belt_input(!v.belt, v.extracting, v.facing)
            })
        },
        insert: |host, pos, _side, item, simulate| direct_belt_input(host, pos, &item.stack, simulate),
    });
    Box::new(Funnels)
}

pub fn is_funnel(block: &str) -> bool {
    matches!(block, ANDESITE | BRASS | ANDESITE_BELT | BRASS_BELT)
}

fn is_brass(block: &str) -> bool {
    matches!(block, BRASS | BRASS_BELT)
}

fn is_belt_funnel(block: &str) -> bool {
    matches!(block, ANDESITE_BELT | BRASS_BELT)
}

/// `getEquivalentBeltFunnel`'s block.
fn belt_variant(block: &str) -> &'static str {
    if is_brass(block) { BRASS_BELT } else { ANDESITE_BELT }
}

/// What a belt funnel falls back to (`BeltFunnelBlock.parent`).
fn plain_variant(block: &str) -> &'static str {
    if is_brass(block) { BRASS } else { ANDESITE }
}

fn add(pos: Pos, dir: Dir) -> Pos {
    util::add(pos, dir.normal())
}

fn salt(pos: Pos) -> i64 {
    (i64::from(pos[0]) * 31 + i64::from(pos[1]) * 17 + i64::from(pos[2]) * 7).rem_euclid(1000)
}

fn bool_name(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

/// A funnel's block state.
#[derive(Clone, Debug)]
struct View {
    block: String,
    brass: bool,
    belt: bool,
    facing: Dir,
    powered: bool,
    /// A plain funnel's `extracting`.
    extracting: bool,
    /// A belt funnel's `shape`.
    shape: Shape,
}

fn view(host: &dyn Host, pos: Pos) -> Option<View> {
    let block = host.block(pos);
    if !is_funnel(&block) {
        return None;
    }
    let property = |key: &str| host.block_property(pos, key);
    Some(View {
        brass: is_brass(&block),
        belt: is_belt_funnel(&block),
        facing: util::facing(host, pos, "facing")?,
        powered: property("powered").as_deref() == Some("true"),
        extracting: property("extracting").as_deref() == Some("true"),
        shape: property("shape")
            .as_deref()
            .and_then(Shape::from_name)
            .unwrap_or(Shape::Retracted),
        block,
    })
}

/// The facing of the belt block at `pos`, none for another block (`AllBlocks.BELT.has`).
fn belt_facing(host: &dyn Host, pos: Pos) -> Option<Dir> {
    if host.block(pos) != belt::ID || host.block_property(pos, "slope").as_deref() != Some("horizontal") {
        return None;
    }
    util::facing(host, pos, "facing")
}

/// `DirectBeltInputBehaviour.canSupportBeltFunnels` / `BeltFunnelBlock.isOnValidBelt` for the
/// block at `pos`: a belt that carries items, or a depot.
fn supports_belt_funnels(host: &dyn Host, pos: Pos) -> bool {
    belt_facing(host, pos).is_some() || host.block(pos) == crate::depot::ID
}

/// `FunnelBlockEntity.determineCurrentMode`.
fn activity(host: &dyn Host, pos: Pos, v: &View) -> Activity {
    let kind = if v.belt {
        // Only a retracted or extended funnel reads the belt's movement.
        let moves = if matches!(v.shape, Shape::Retracted | Shape::Extended) {
            let below = add(pos, Dir::Down);
            belt_facing(host, below).and_then(|facing| {
                let rpm = host
                    .mod_block_entity(below)
                    .and_then(|e| float(&e.nbt, crate::SPEED))?;
                Some(create_kinetics::belt::movement_facing(rpm, facing) == v.facing)
            })
        } else {
            None
        };
        Kind::Belt { shape: v.shape, belt_moves_towards_facing: moves }
    } else {
        Kind::Plain { extracting: v.extracting }
    };
    rules::determine_activity(kind, v.powered)
}

fn kind_of(v: &View) -> Kind {
    if v.belt {
        Kind::Belt { shape: v.shape, belt_moves_towards_facing: None }
    } else {
        Kind::Plain { extracting: v.extracting }
    }
}

/// The inventory a funnel works on: the block behind it.
fn behind(pos: Pos, v: &View) -> Pos {
    add(pos, v.facing.opposite())
}

fn filter_of(v: &View, nbt: &Compound) -> Filter {
    if v.brass {
        inv::filter_of(nbt)
    } else {
        Filter { item: None, amount: chute::Amount::default() }
    }
}

/// `FunnelBlockEntity.getAmountToExtract` and `getModeToExtract`.
fn extraction(host: &dyn Host, pos: Pos, v: &View, filter: &Filter) -> (u32, Mode) {
    let belt_below = belt_facing(host, add(pos, Dir::Down)).is_some();
    let supports = rules::supports_amount(kind_of(v), belt_below);
    // `filtering.isActive()`: only the brass funnels filter.
    (
        rules::amount_to_extract(supports, v.brass, filter.amount),
        rules::mode_to_extract(supports, v.brass, filter.amount),
    )
}

// --- the tick -----------------------------------------------------------------------------------

fn tick(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity) {
    let Some(v) = view(host, pos) else {
        return;
    };
    let now = util::game_time(host);
    let phase = now + salt(pos);

    if phase % 2 == 0 && sync_powered(host, pos, &v) {
        return;
    }
    if phase % STATE_RATE == 0 && refresh_state(host, pos, entity, &v) {
        return;
    }
    let mode = activity(host, pos, &v);
    let cooldown = int(&entity.nbt, COOLDOWN);
    // Redstone resets the extraction cooldown.
    if mode == Activity::Paused && cooldown != 0 {
        entity.nbt.insert(COOLDOWN.into(), Tag::Int(0));
    }
    // A powered belt funnel still holds back what is on its belt.
    if v.belt && v.powered && v.shape != Shape::Pushing {
        pull_from_belt(host, pos, entity, &v, now);
        return;
    }
    if mode == Activity::TakingFromBelt {
        pull_from_belt(host, pos, entity, &v, now);
        return;
    }
    if cooldown > 0 {
        entity.nbt.insert(COOLDOWN.into(), Tag::Int(cooldown - 1));
        return;
    }
    let attempt = phase % ATTEMPT_RATE == 0;
    match mode {
        Activity::PushingToBelt if attempt => push_to_belt(host, pos, entity, &v, now),
        Activity::Extract if attempt => extract(host, pos, entity, &v, now),
        Activity::Collect if phase % rules::COLLECT_SCAN == 0 => collect(host, pos, entity, &v, now),
        _ => {}
    }
}

/// `AbstractFunnelBlock.tick`: the state's `powered` follows redstone.
fn sync_powered(host: &mut dyn Host, pos: Pos, v: &View) -> bool {
    let dimension = host.dimension();
    let powered = host.neighbor_signal(dimension, pos) > 0;
    if powered != v.powered {
        host.set_block_property(pos, "powered", bool_name(powered));
        return true;
    }
    false
}

/// `FunnelBlock.updateShape` and `BeltFunnelBlock.updateShape`: a funnel over a belt or a depot
/// becomes a belt funnel, a belt funnel without one a plain funnel, and a belt funnel's shape
/// follows the belt under it. True when the block was swapped.
fn refresh_state(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity, v: &View) -> bool {
    let below = add(pos, Dir::Down);
    let supported = supports_belt_funnels(host, below);
    if !v.belt {
        if v.facing.axis() == create_kinetics::Axis::Y || !supported {
            return false;
        }
        let shape = rules::shape_for_position(belt_facing(host, below), v.facing, v.extracting);
        swap(host, pos, entity, belt_variant(&v.block), &[
            ("facing", v.facing.name()),
            ("powered", bool_name(v.powered)),
            ("shape", shape.name()),
        ]);
        return true;
    }
    if !supported {
        swap(host, pos, entity, plain_variant(&v.block), &[
            ("facing", v.facing.name()),
            ("powered", bool_name(v.powered)),
            ("extracting", bool_name(rules::falls_back_extracting(v.shape))),
        ]);
        return true;
    }
    let wanted = rules::shape_for_position(belt_facing(host, below), v.facing, v.shape == Shape::Pushing);
    let next = rules::updated_shape(v.shape, wanted);
    if next != v.shape {
        host.set_block_property(pos, "shape", next.name());
    }
    false
}

/// Replaces the funnel at `pos` with another block (a belt funnel and its funnel), keeping what
/// the block entity holds that isn't about its type: the filter and the cooldown.
fn swap(host: &mut dyn Host, pos: Pos, entity: &ModBlockEntity, block: &str, props: &[(&str, &str)]) {
    let carry: Vec<(&str, Tag)> = [inv::FILTER, inv::FILTER_AMOUNT, inv::UP_TO, COOLDOWN]
        .into_iter()
        .filter_map(|key| util::get(&entity.nbt, key).map(|tag| (key, tag.clone())))
        .collect();
    let dimension = host.dimension();
    let Some(state) = host.default_state(block) else {
        return;
    };
    if !host.set_block(dimension, pos, state, util::UPDATE_ALL) {
        return;
    }
    for (key, value) in props {
        host.set_block_property(pos, key, value);
    }
    host.change_mod_block_entity(pos, &mut |e| {
        for (key, tag) in &carry {
            e.nbt.insert((*key).into(), tag.clone());
        }
    });
    host.block_entity_sync(pos);
}

fn start_cooldown(entity: &mut ModBlockEntity) {
    entity
        .nbt
        .insert(COOLDOWN.into(), Tag::Int(rules::EXTRACTION_TIMER));
}

/// `FunnelBlockEntity.flap`: the flaps swing, in (`inward`) or out.
fn flap(entity: &mut ModBlockEntity, inward: bool, now: i64) {
    entity.nbt.insert(FLAP_KEY.into(), Tag::Byte(if inward { -1 } else { 1 }));
    entity.nbt.insert(FLAP_TICK.into(), Tag::Long(now));
}

fn has_flap(v: &View) -> bool {
    v.facing.axis() != create_kinetics::Axis::Y
}

// --- putting items out --------------------------------------------------------------------------

/// `FunnelBlockEntity.activateExtractor`: takes a stack out of the inventory behind and puts it
/// out in front.
fn extract(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity, v: &View, now: i64) {
    let source = behind(pos, v);
    let filter = filter_of(v, &entity.nbt);
    let (amount, mode) = extraction(host, pos, v, &filter);
    // Something still lying in front blocks it.
    let (low, high) = rules::overflow_area(v.facing);
    let [x, y, z] = pos.map(f64::from);
    let dimension = host.dimension();
    if !host
        .item_entities(
            dimension,
            [x + low[0], y + low[1], z + low[2]],
            [x + high[0], y + high[1], z + high[2]],
        )
        .is_empty()
    {
        return;
    }
    let Some(plan) = inv::plan(host, source, mode, amount, &mut |host, stack| filter.test(host, stack)) else {
        return;
    };
    let Some(stack) = inv::take(host, source, v.facing, &plan) else {
        return;
    };
    flap(entity, false, now);
    let (offset, motion) = rules::output_of(v.facing);
    let c = util::centre(pos);
    util::spawn_item(
        host,
        [c[0] + offset[0], c[1] + offset[1], c[2] + offset[2]],
        &stack,
        motion,
        chute::PICKUP_DELAY,
    );
    start_cooldown(entity);
    host.block_entity_sync(pos);
}

/// `FunnelBlockEntity.activateExtractingBeltFunnel`: takes a stack out of the inventory behind
/// and puts it on the belt or the depot below.
fn push_to_belt(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity, v: &View, now: i64) {
    let below = add(pos, Dir::Down);
    let side = v.facing;
    if belt::can_insert_from_side(host, below, side) != Some(true) {
        return;
    }
    let source = behind(pos, v);
    let filter = filter_of(v, &entity.nbt);
    let (amount, mode) = extraction(host, pos, v, &filter);
    let Some(plan) = inv::plan(host, source, mode, amount, &mut |host, stack| {
        // The belt or depot must take it all.
        let probe = Item::dropped(stack.clone(), 0, 180, true);
        filter.test(host, stack)
            && belt::handle_insertion(host, below, side, &probe, false, true)
                .is_some_and(|rest| rest.count == 0)
    }) else {
        return;
    };
    let Some(stack) = inv::take(host, source, v.facing, &plan) else {
        return;
    };
    flap(entity, false, now);
    let angle = (util::random() % 360) as i32;
    let item = Item::dropped(stack.clone(), 0, angle, true);
    let rest = belt::handle_insertion(host, below, side, &item, false, false);
    if let Some(rest) = rest
        && rest.count > 0
    {
        // The belt filled up between asking and giving: what is left goes back.
        host.insert_item(source, util::direction(v.facing), rest);
    }
    start_cooldown(entity);
    host.block_entity_sync(pos);
}

/// `FunnelBlock.entityInside`: an item entity in the funnel's block, on its open side, goes in.
fn collect(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity, v: &View, now: i64) {
    if v.belt || !rules::can_insert_into(v.powered, v.extracting) {
        return;
    }
    let [x, y, z] = pos.map(f64::from);
    let dimension = host.dimension();
    let centre = util::centre(pos);
    for found in host.item_entities(dimension, [x, y, z], [x + 1.0, y + 1.0, z + 1.0]) {
        let at = [
            found.position[0] - centre[0],
            found.position[1] - centre[1],
            found.position[2] - centre[2],
        ];
        if !rules::on_open_side(v.facing, at, true) {
            continue;
        }
        let rest = try_insert(host, pos, entity, v, &found.stack, now);
        if rest.count == found.stack.count {
            continue;
        }
        host.set_item_entity(dimension, found.id, rest);
        host.block_entity_sync(pos);
        return;
    }
}

/// `AbstractFunnelBlock.tryInsert`: a stack into the inventory behind the funnel if its filter
/// lets it by. What is left (count 0 for all of it).
fn try_insert(
    host: &mut dyn Host,
    pos: Pos,
    entity: &mut ModBlockEntity,
    v: &View,
    stack: &ItemStack,
    now: i64,
) -> ItemStack {
    if !filter_of(v, &entity.nbt).test(host, stack) {
        return stack.clone();
    }
    let source = behind(pos, v);
    let Some(rest) = inv::insert(host, source, v.facing, stack, false) else {
        return stack.clone();
    };
    if rest.count != stack.count && has_flap(v) {
        flap(entity, true, now);
    }
    rest
}

// --- taking items off a belt or a depot ------------------------------------------------------

/// `BeltFunnelInteractionHandler.checkForFunnels` for this funnel, and the depot's
/// `handleBeltFunnelOutput`: what is on the belt below is taken in as it reaches the funnel.
fn pull_from_belt(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity, v: &View, now: i64) {
    let below = add(pos, Dir::Down);
    if host.block(below) == crate::depot::ID {
        if v.shape == Shape::Pulling && !v.powered {
            take_from_depot(host, pos, entity, v, below, now);
        }
        return;
    }
    if belt_facing(host, below).is_none() {
        return;
    }
    let filter = filter_of(v, &entity.nbt);
    let (amount, mode) = extraction(host, pos, v, &filter);
    let source = behind(pos, v);
    let mut flaps = false;
    let powered = v.powered;
    let (facing, shape) = (v.facing, v.shape);
    belt::edit_items(host, below, &mut |host, lane, items| {
        let reaction = rules::reaction(facing, lane.movement, shape);
        if reaction == Reaction::Ignore {
            return false;
        }
        let mut changed = false;
        for item in items.iter_mut() {
            let next = item.pos + lane.speed;
            let Some(entry) = rules::belt_entry(
                item.pos,
                next,
                lane.index,
                lane.length,
                lane.positive,
                shape == Shape::Extended,
            ) else {
                continue;
            };
            let blocking = reaction == Reaction::Blocking;
            if blocking && item.pos != entry {
                item.pos = entry;
                changed = true;
            }
            if powered || !filter.test(host, &item.stack) {
                continue;
            }
            let offer = rules::belt_offer(amount, mode, item.stack.count);
            let rules::Offer::Take { count, .. } = offer else {
                continue;
            };
            let mut put = item.stack.clone();
            put.count = count;
            let Some(rest) = inv::insert(host, source, facing, &put, false) else {
                continue;
            };
            if rest.count == put.count {
                continue;
            }
            // What went in, and what was never offered, stay on the belt.
            let not_offered = item.stack.count - count;
            item.stack.count = rest.count + not_offered;
            flaps = true;
            changed = true;
        }
        changed
    });
    if flaps {
        flap(entity, true, now);
        host.block_entity_sync(pos);
    }
}

/// `DepotBehaviour.handleBeltFunnelOutput`: a belt funnel pulling on a depot takes what is in
/// its output buffer, then what it holds.
fn take_from_depot(
    host: &mut dyn Host,
    pos: Pos,
    entity: &mut ModBlockEntity,
    v: &View,
    depot: Pos,
    now: i64,
) {
    let Some(inside) = host.mod_block_entity(depot) else {
        return;
    };
    let order: Vec<usize> = (1..inside.items.len()).chain(0..1.min(inside.items.len())).collect();
    for slot in order {
        let Some(stack) = inside.items[slot].clone() else {
            continue;
        };
        let rest = try_insert(host, pos, entity, v, &stack, now);
        if rest.count == stack.count {
            continue;
        }
        host.change_mod_block_entity(depot, &mut |e| {
            if slot < e.items.len() {
                e.items[slot] = (rest.count > 0).then(|| rest.clone());
            }
        });
        host.block_entity_sync(depot);
        host.block_entity_sync(pos);
        return;
    }
}

// --- the DirectBeltInputBehaviour ---------------------------------------------------------------

/// `FunnelBlockEntity.handleDirectBeltInput`: a stack a belt ends at the funnel with goes into
/// the inventory behind it. What is left.
fn direct_belt_input(host: &mut dyn Host, pos: Pos, stack: &ItemStack, simulate: bool) -> ItemStack {
    let (Some(v), Some(entity)) = (view(host, pos), host.mod_block_entity(pos)) else {
        return stack.clone();
    };
    if !filter_of(&v, &entity.nbt).test(host, stack) || v.powered {
        return stack.clone();
    }
    if simulate {
        return stack.clone();
    }
    inv::insert(host, behind(pos, &v), v.facing, stack, false).unwrap_or_else(|| stack.clone())
}

/// `DirectBeltInputBehaviour.tryExportingToBeltFunnel` for a block that has a funnel on top (the Item Drain's output):
/// the funnel at `funnel` takes `stack` into the inventory behind it if it is a belt funnel in its pulling shape facing
/// `side`. `None` when the block there is no such funnel (the stack goes on as it would), the stack itself when the funnel
/// is powered, else what is left. The funnel flaps when it took something.
pub(crate) fn export_from_below(host: &mut dyn Host, funnel: Pos, stack: &ItemStack, side: Dir) -> Option<ItemStack> {
    let v = view(host, funnel)?;
    if !create_kinetics::rules::item_drain::belt_funnel_applies(v.belt, v.shape == Shape::Pulling, v.facing, side) {
        return None;
    }
    let mut entity = host.mod_block_entity(funnel)?;
    if v.powered {
        return Some(stack.clone());
    }
    let now = util::game_time(host);
    let rest = try_insert(host, funnel, &mut entity, &v, stack, now);
    if rest.count != stack.count {
        let (key, tick) = (util::get(&entity.nbt, FLAP_KEY).cloned(), util::get(&entity.nbt, FLAP_TICK).cloned());
        host.change_mod_block_entity(funnel, &mut |e| {
            if let (Some(key), Some(tick)) = (&key, &tick) {
                e.nbt.insert(FLAP_KEY.into(), key.clone());
                e.nbt.insert(FLAP_TICK.into(), tick.clone());
            }
        });
        host.block_entity_sync(funnel);
    }
    Some(rest)
}

// --- drawing ------------------------------------------------------------------------------------

/// `AngleHelper.horizontalAngle`: south 0, east 90, north 180, west 270.
fn horizontal_angle(facing: Dir) -> f32 {
    match facing {
        Dir::East => 90.0,
        Dir::North => 180.0,
        Dir::West => 270.0,
        _ => 0.0,
    }
}

/// `FunnelRenderer`: the four flaps, swinging as the funnel last moved something.
fn draw(host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
    let property = |key: &str| host.block_state_property(event.state, key);
    let Some(facing) = property("facing").and_then(Dir::from_name) else {
        event.draw.keep();
        return;
    };
    if facing.axis() == create_kinetics::Axis::Y {
        event.draw.keep();
        return;
    }
    let belt = is_belt_funnel(event.block);
    let kind = if belt {
        Kind::Belt {
            shape: property("shape").and_then(Shape::from_name).unwrap_or(Shape::Retracted),
            belt_moves_towards_facing: None,
        }
    } else {
        Kind::Plain { extracting: false }
    };
    let nbt = &event.entity.nbt;
    let since = (event.draw.game_time - util::long(nbt, FLAP_TICK)) as f32 + event.draw.partial;
    let swinging = util::has(nbt, FLAP_KEY) && since < FLAP_TICKS;
    let flapness = if swinging {
        rules::flap_value(int(nbt, FLAP_KEY) < 0, since)
    } else {
        0.0
    };
    let model = if belt { BELT_FLAP } else { FLAP };
    let draw = &mut *event.draw;
    draw.push();
    draw.rotate_about([0.0, 1.0, 0.0], horizontal_angle(facing.opposite()), [0.5; 3]);
    draw.translate([rules::FLAP_X_OFFSET, 0.0, -rules::flap_offset(kind)]);
    for segment in 0..rules::FLAP_COUNT {
        draw.push();
        draw.rotate_about([1.0, 0.0, 0.0], rules::flap_angle(flapness, segment), rules::FLAP_PIVOT);
        host.draw_model(draw, model, 0xFF_FFFF);
        draw.pop();
        draw.translate([rules::FLAP_SEGMENT_STEP, 0.0, 0.0]);
    }
    draw.pop();
    if !swinging {
        draw.keep();
    }
}

// --- the hooks ----------------------------------------------------------------------------------

impl Hooks for Funnels {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.push(FLAP.to_owned());
        event.models.push(BELT_FLAP.to_owned());
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        is_funnel(event.block).then(|| (event.block.to_owned(), Compound::new()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if is_funnel(event.block) {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if is_funnel(&event.entity.id) {
            tick(host, event.pos, event.entity);
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        if !is_funnel(&event.entity.id) {
            return None;
        }
        let nbt = &event.entity.nbt;
        let mut tag = Compound::new();
        tag.insert(FLAP_KEY.into(), Tag::Byte(int(nbt, FLAP_KEY) as i8));
        tag.insert(FLAP_TICK.into(), Tag::Long(util::long(nbt, FLAP_TICK)));
        Some(tag)
    }

    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if is_funnel(event.block) {
            draw(host, event);
        }
    }

    /// `FunnelBlock.getStateForPlacement`: a funnel opens towards the player, extracting unless
    /// they sneak (`AbstractFunnelBlock` powers it from redstone once it is placed).
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if !matches!(&*context.item, ANDESITE | BRASS) {
            return None;
        }
        let order = rules::nearest_looking(context.yaw, context.pitch);
        let facing = rules::placement_facing(order, |_| true)?;
        let state = level.state_with_property(event.state, "facing", facing.name())?;
        level
            .state_with_property(state, "extracting", bool_name(!context.sneaking))
            .map(Some)
    }

    /// A funnel placed over a belt or a depot is a belt funnel at once
    /// (`FunnelItem.getPlacementState`), and takes its redstone.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if !is_funnel(&event.stack.item) {
            return;
        }
        let pos = event.pos;
        let Some(v) = view(host, pos) else {
            return;
        };
        if sync_powered(host, pos, &v) {
            return;
        }
        if let Some(mut entity) = host.mod_block_entity(pos) {
            refresh_state(host, pos, &mut entity, &v);
        }
    }

    /// The Wrench
    /// (`FunnelBlock.onWrenched`, `BeltFunnelBlock.onWrenched`), and an item put into the funnel's
    /// inventory by hand (`FunnelBlock.useItemOn`).
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        let Some(v) = view(host, pos) else {
            return Flow::Continue;
        };
        let item = &*event.stack.item;
        if item == util::WRENCH {
            return util::wrench(host, pos, |host| {
                if v.belt {
                    let next = rules::wrenched(v.shape, false);
                    if next != v.shape {
                        host.set_block_property(pos, "shape", next.name());
                    }
                } else {
                    host.set_block_property(pos, "extracting", bool_name(!v.extracting));
                }
            });
        }
        // Hand insertion: through the funnel's open face, if it takes things in.
        let open = Dir::from_normal(event.face) == Some(v.facing);
        if v.belt || !open || item == MECHANICAL_ARM || !rules::can_insert_into(v.powered, v.extracting) {
            return Flow::Continue;
        }
        let Some(mut entity) = host.mod_block_entity(pos) else {
            return Flow::Cancel;
        };
        let now = util::game_time(host);
        let rest = try_insert(host, pos, &mut entity, &v, event.stack, now);
        let moved = event.stack.count.saturating_sub(rest.count);
        if moved > 0 {
            if host.acting_player().is_some_and(|p| p.game_mode == util::SURVIVAL) {
                host.take_items(&event.stack.item, moved);
            }
            let flapped = util::get(&entity.nbt, FLAP_TICK).cloned();
            host.change_mod_block_entity(pos, &mut |e| {
                if let Some(tag) = &flapped {
                    e.nbt.insert(FLAP_TICK.into(), tag.clone());
                    e.nbt.insert(FLAP_KEY.into(), Tag::Byte(-1));
                }
            });
            host.block_entity_sync(pos);
        }
        Flow::Cancel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_funnel_blocks_are_told_apart() {
        assert!(is_funnel(ANDESITE) && is_funnel(BRASS_BELT));
        assert!(!is_funnel("create:chute"));
        assert!(is_brass(BRASS) && is_brass(BRASS_BELT) && !is_brass(ANDESITE_BELT));
        assert_eq!(belt_variant(BRASS), BRASS_BELT);
        assert_eq!(belt_variant(ANDESITE), ANDESITE_BELT);
        assert_eq!(plain_variant(BRASS_BELT), BRASS);
        assert_eq!(plain_variant(ANDESITE_BELT), ANDESITE);
    }

    #[test]
    fn the_flaps_face_the_way_the_funnel_does() {
        assert_eq!(horizontal_angle(Dir::South), 0.0);
        assert_eq!(horizontal_angle(Dir::West), 270.0);
        // The flaps are turned by the angle of the funnel's back.
        assert_eq!(horizontal_angle(Dir::North.opposite()), 0.0);
    }
}
