//! The Weighted Ejector (`EjectorBlock`, `EjectorBlockEntity`, `EjectorItem`, `EjectorRenderer`,
//! `EjectorTargetHandler`, `EntityLauncher`, `SharedDepotBlockMethods`, `DepotBehaviour`): a depot
//! with a lid that charges with rotation and, when something is put on it or steps on it, throws
//! it along an arc to a target block. The arc and the numbers are
//! `create_kinetics::rules::weighted_ejector`; this is the glue.
//!
//! Choosing the target. There is no target-selection screen, so it is chosen as Create's own item does
//! (`EjectorItem`, `EjectorTargetHandler.rightClickingBlocksSelectsThem`, `flushSettings`): sneak and
//! right-click a block with the Weighted Ejector item to select it ("Target selected" on the action
//! bar), then place the ejector. It takes the selected block as its target if that is in line
//! with it along a horizontal axis, no further than 32 blocks off and not in the same column,
//! turns to face it, and otherwise throws to the block in front of it (Create says so on the action
//! bar too). The Wrench doesn't change a target afterwards (Create's doesn't either).
//!
//! Known differences from Create, for the README:
//! - No arc preview (particles and outlines) while choosing, no left-click to deselect, and the
//!   selection is remembered per player until they place an ejector (or the game closes).
//! - The stack size setting (`ScrollValueBehaviour`, 0 to 64, "*" for any) has no value box: it is
//!   saved as `ScrollValue` (default 0, as Create's) and can be edited with `/data`.
//! - What stands on it is found by polling each tick rather than on landing
//!   (`updateEntityAfterFallOn`), and is eased to the middle first as Create does. Players and mobs
//!   are thrown by setting their position and velocity (a player is told by message, as the Encased Fan
//!   does); the Elytra is not deployed, no advancement, and items on the lid don't trigger it unless
//!   they are inserted.
//! - A held stack and stacks put on by belts are merged at once into the held stack
//!   (`enableMerging`, without the `incoming` list's slide), up to the stack size setting or the item's
//!   limit. No output buffer or processing (nothing presses on an ejector). A belt funnel above the
//!   ejector doesn't take the stack instead of the arc (`tryExportingToBeltFunnel`); an observer
//!   beside it isn't pulsed; a package isn't drawn spinning.
//! - Only the first launched item of each launch, once every 120 ticks, is traced for blocks in its
//!   way (`ejectorScanInterval`), through the block outline rather than its collider
//!   (`pick_block`), and an open extracting funnel doesn't count as a miss.
//! - Redstone is polled every other tick; no comparator output; no waterlogging, no friction change
//!   while launching, no fall damage change.
//! - Saved state: `HorizontalDistance`, `VerticalDistance`, `Powered`, `State` and the held stack
//!   (`Items` and `HeldItem`) as Create's; the lid is saved as `Lid0` and `StateStart` (its value
//!   and the game time at the start of the state) instead of a `LerpedFloat`, and a launched item as
//!   `Start` (the game time it left) beside `Second`, instead of a counter.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use create_kinetics::belt::Item;
use create_kinetics::rules::weighted_ejector::{self as rules, Launcher, State};
use create_kinetics::{Dir, Kind, Node, looking};
use crate::compat::item_data::{decode_components, encode_components};
use crate::compat::{
    BlockEntityContainerEvent, BlockEntityEvent, BlockEntityRenderBoundingBoxEvent,
    BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent, BlockEntityUpdateTagEvent,
    BlockStateId, Compound, ContainerAcceptsEvent, DestroyBlockEvent, DisplayContext, EquipmentSlot,
    Flow, Hooks, Host, ItemStack, LevelHost, ModBlockEntity, ModContainer, NativeInventoryKind,
    NativeServerPlayer, PlaceBlockEvent, RightClickBlockEvent,
    StandaloneModelsEvent, StateForPlacementEvent, Tag, Text, UseItemOnEvent,
};

use super::fan;
use crate::{belt, kinetic, util};

pub const ID: &str = "create:weighted_ejector";
const TOP_MODEL: &str = "create:block/weighted_ejector/top";
const HELD: &str = "HeldItem";
const ITEM: &str = "Item";
const POS: &str = "Pos";
const PREV_POS: &str = "PrevPos";
const OFFSET: &str = "Offset";
const PREV_OFFSET: &str = "PrevOffset";
const ANGLE: &str = "Angle";
const IN_DIRECTION: &str = "InDirection";
const HORIZONTAL: &str = "HorizontalDistance";
const VERTICAL: &str = "VerticalDistance";
const POWERED: &str = "Powered";
const STATE: &str = "State";
const LID: &str = "Lid0";
const STATE_START: &str = "StateStart";
const SCROLL: &str = "ScrollValue";
const LAUNCHED: &str = "LaunchedItems";
const START: &str = "Start";
const SECOND: &str = "Second";
const EARLY_TARGET: &str = "EarlyTarget";
const EARLY_POS: &str = "EarlyTargetPos";
const EARLY_TIME: &str = "EarlyTargetTime";
/// `SoundSource` ordinals.
const BLOCKS: usize = 4;
const PLAYERS: usize = 7;
const MECHANICAL_ARM: &str = "create:mechanical_arm";
const SPECTATOR: i32 = 3;
const BASIN: &str = "create:basin";
/// Ticks between two looks at the neighbouring redstone, and at what lies on top.
const POLL: i64 = 2;

type Pos = [i32; 3];

struct Ejector;

pub fn hooks() -> Box<dyn Hooks> {
    // A belt, a depot or a funnel that ends at the ejector hands its items over.
    belt::register_input(belt::BeltInput {
        claims: |block| block == ID,
        can_insert: |host, pos, _side| state_of(host, pos) == State::Charged,
        insert: |host, pos, side, item, simulate| {
            insert(host, pos, &item.stack, side, Some(item.angle), simulate)
        },
    });
    Box::new(Ejector)
}

// --- the state and the selection ----------------------------------------------------------------

/// Where a player's sneak-clicked target is, until they place an ejector (`EjectorTargetHandler`).
static SELECTED: LazyLock<Mutex<HashMap<u128, Pos>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn selected() -> std::sync::MutexGuard<'static, HashMap<u128, Pos>> {
    SELECTED.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What is not saved: when the last obstacle scan of a launch was, and which launched item (by its start tick) is
/// being traced (`scanCooldown`, `trackedItem`).
#[derive(Clone, Copy, Default)]
struct Scan {
    cooldown: i32,
    tracked: Option<i64>,
}

static SCANS: LazyLock<Mutex<HashMap<(String, Pos), Scan>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn scans() -> std::sync::MutexGuard<'static, HashMap<(String, Pos), Scan>> {
    SCANS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The saved fields of an ejector.
#[derive(Clone, Copy)]
struct Data {
    state: State,
    start: i64,
    lid0: f32,
    horizontal: i32,
    vertical: i32,
    powered: bool,
    setting: i32,
}

fn read(nbt: &Compound) -> Data {
    let state = match util::get(nbt, STATE) {
        Some(Tag::String(name)) => State::from_name(&name.to_string_lossy()).unwrap_or(State::Retracting),
        _ => State::Retracting,
    };
    Data {
        state,
        start: util::long(nbt, STATE_START),
        lid0: util::float(nbt, LID).unwrap_or(1.0),
        horizontal: util::int(nbt, HORIZONTAL).max(1),
        vertical: util::int(nbt, VERTICAL),
        powered: util::boolean(nbt, POWERED),
        setting: util::int(nbt, SCROLL).clamp(0, 64),
    }
}

fn launcher(data: &Data) -> Launcher {
    let mut launcher = Launcher::new(data.horizontal, data.vertical);
    launcher.clamp(rules::MAX_DISTANCE);
    launcher
}

fn state_of(host: &dyn Host, pos: Pos) -> State {
    host.mod_block_entity(pos).map_or(State::Retracting, |e| read(&e.nbt).state)
}

/// `EjectorBlock.getRotationAxis`: the shaft runs across the facing.
fn shaft_axis(facing: Dir) -> create_kinetics::Axis {
    clockwise(facing).axis()
}

fn clockwise(facing: Dir) -> Dir {
    match facing {
        Dir::North => Dir::East,
        Dir::East => Dir::South,
        Dir::South => Dir::West,
        Dir::West => Dir::North,
        other => other,
    }
}

fn facing_of(host: &dyn Host, pos: Pos) -> Dir {
    util::facing(host, pos, "facing").unwrap_or(Dir::North)
}

/// The lid at game time `now` (0 shut and charged, 1 open), from the state's start.
fn lid_at(data: &Data, now: i64, rpm: f32) -> f32 {
    rules::lid_at(data.state, data.lid0, data.start, now, rpm, data.horizontal, data.vertical)
}

fn set_state(nbt: &mut Compound, state: State, lid0: f32, now: i64) {
    nbt.insert(STATE.into(), Tag::String(state.name().into()));
    nbt.insert(LID.into(), Tag::Float(lid0.to_bits()));
    nbt.insert(STATE_START.into(), Tag::Long(now));
}

// --- the held stack ------------------------------------------------------------------------------

/// Where the held stack is on its slide to the middle (`TransportedItemStack`, as `depot.rs` keeps it).
#[derive(Clone, Copy, PartialEq, Debug)]
struct Transport {
    pos: f32,
    prev_pos: f32,
    offset: f32,
    prev_offset: f32,
    angle: i32,
    from: Dir,
}

impl Transport {
    fn new(angle: i32, from: Dir, pos: f32) -> Self {
        let offset = (angle - 180) as f32 / (360.0 * 3.0);
        Transport { pos, prev_pos: pos, offset, prev_offset: offset, angle, from }
    }

    /// `DepotBehaviour.tick`'s slide: a quarter of the way to the middle each tick.
    fn tick(&mut self) {
        self.prev_pos = self.pos;
        self.prev_offset = self.offset;
        let diff = 0.5 - self.pos;
        if diff > 1.0 / 512.0 {
            if diff > 1.0 / 32.0 {
                self.angle += 1;
            }
            self.pos += diff / 4.0;
        }
    }

    fn at_rest(&self) -> bool {
        self.pos == self.prev_pos && self.offset == self.prev_offset
    }

    fn read(nbt: &Compound) -> Option<Self> {
        let Some(Tag::Compound(held)) = util::get(nbt, HELD) else {
            return None;
        };
        if !util::has(held, POS) {
            return None;
        }
        Some(Transport {
            pos: util::float(held, POS).unwrap_or(0.0),
            prev_pos: util::float(held, PREV_POS).unwrap_or(0.0),
            offset: util::float(held, OFFSET).unwrap_or(0.0),
            prev_offset: util::float(held, PREV_OFFSET).unwrap_or(0.0),
            angle: util::int(held, ANGLE),
            from: Dir::ALL[util::int(held, IN_DIRECTION).rem_euclid(6) as usize],
        })
    }

    fn write(&self) -> Compound {
        let mut tag = Compound::new();
        util::set_float(&mut tag, POS, self.pos);
        util::set_float(&mut tag, PREV_POS, self.prev_pos);
        util::set_float(&mut tag, OFFSET, self.offset);
        util::set_float(&mut tag, PREV_OFFSET, self.prev_offset);
        tag.insert(ANGLE.into(), Tag::Int(self.angle));
        let from = Dir::ALL.iter().position(|d| *d == self.from).unwrap_or(1);
        tag.insert(IN_DIRECTION.into(), Tag::Int(from as i32));
        tag
    }
}

fn held_stack(entity: &ModBlockEntity) -> Option<&ItemStack> {
    entity.items.first().and_then(Option::as_ref)
}

fn stack_tag(stack: &ItemStack) -> Tag {
    let mut tag = Compound::new();
    tag.insert("id".into(), Tag::String((&*stack.item).into()));
    tag.insert("count".into(), Tag::Int(stack.count as i32));
    if let Some(components) = encode_components(stack) {
        tag.insert("components".into(), components);
    }
    Tag::Compound(tag)
}

fn stack_of(tag: &Tag) -> Option<ItemStack> {
    let Some(Tag::String(id)) = tag.get("id") else {
        return None;
    };
    let id = id.to_string_lossy();
    if id.is_empty() || id == "minecraft:air" {
        return None;
    }
    let count = match tag.get("count") {
        Some(Tag::Int(count)) if *count > 0 => *count as u32,
        _ => 1,
    };
    let (damage, extra) = decode_components(tag.get("components"));
    Some(ItemStack { item: id.as_str().into(), count, damage, extra })
}

/// How many more items the ejector takes (`DepotBehaviour.getRemainingSpace`).
fn space(host: &dyn Host, data: &Data, entity: &ModBlockEntity, stack: &ItemStack) -> i32 {
    let limit = host.max_stack_size(&stack.item).unwrap_or(64);
    let present = held_stack(entity).map_or(0, |s| s.count);
    rules::remaining_space(data.setting, limit, present)
}

/// `DirectBeltInputBehaviour` of the ejector (`DepotBehaviour.insert` with merging): the stack goes
/// on while the ejector is charged; what doesn't fit comes back. `side` is the way it came in
/// from (it slides in from there unless it fell from above), `angle` how it lies.
fn insert(
    host: &mut dyn Host,
    pos: Pos,
    stack: &ItemStack,
    side: Dir,
    angle: Option<i32>,
    simulate: bool,
) -> ItemStack {
    let mut rest = stack.clone();
    let Some(entity) = host.mod_block_entity(pos).filter(|e| e.id == ID) else {
        return rest;
    };
    let data = read(&entity.nbt);
    // `canAcceptItems = state == CHARGED`.
    if data.state != State::Charged {
        return rest;
    }
    let room = space(&*host, &data, &entity, stack);
    if room <= 0 {
        return rest;
    }
    if let Some(held) = held_stack(&entity)
        && !util::same_item(held, stack)
    {
        return rest;
    }
    let taken = stack.count.min(room as u32);
    rest.count -= taken;
    if simulate {
        return rest;
    }
    let angle = angle.unwrap_or_else(|| (util::random() % 360) as i32);
    // `tryInsertingFromSide`: from the side the item starts at the edge, from above in the middle.
    let at = if side.axis() == create_kinetics::Axis::Y { 0.5 } else { 0.0 };
    let mut put = stack.clone();
    put.count = taken;
    host.change_mod_block_entity(pos, &mut |entity| {
        if entity.items.is_empty() {
            entity.items.push(None);
        }
        match entity.items[0].as_mut() {
            Some(held) => held.count += taken,
            None => {
                entity.items[0] = Some(put.clone());
                let transport = Transport::new(angle, side, at);
                entity.nbt.insert(HELD.into(), Tag::Compound(transport.write()));
            }
        }
    });
    // AllSoundEvents.DEPOT_SLIDE or DEPOT_PLOP.
    if side.axis() == create_kinetics::Axis::Y {
        host.play_sound("minecraft:entity.item_frame.add_item", util::centre(pos), BLOCKS, 0.25, 1.25);
    } else {
        host.play_sound("minecraft:block.sand.break", util::centre(pos), BLOCKS, 0.125, 1.5);
    }
    host.block_entity_sync(pos);
    rest
}

// --- launching -----------------------------------------------------------------------------------

/// Entities `EjectorBlockEntity.activateDeferred` never throws: items (they go with the stack),
/// and what a piston would ignore (`PushReaction.IGNORE`).
fn ignored(kind: &str) -> bool {
    matches!(
        kind,
        "minecraft:item"
            | "minecraft:marker"
            | "minecraft:interaction"
            | "minecraft:armor_stand"
            | "minecraft:item_frame"
            | "minecraft:glow_item_frame"
            | "minecraft:painting"
            | "minecraft:leash_knot"
            | "minecraft:block_display"
            | "minecraft:item_display"
            | "minecraft:text_display"
    )
}

/// The entities whose feet are on the lid: `AABB(worldPosition).inflate(-1 / 16f, 0, -1 / 16f)`.
fn standing(host: &mut dyn Host, pos: Pos) -> Vec<u32> {
    let [x, y, z] = pos.map(f64::from);
    let dimension = host.dimension();
    let ids = host.entities_in(dimension, [x + 1.0 / 16.0, y, z + 1.0 / 16.0], [x + 15.0 / 16.0, y + 1.0, z + 15.0 / 16.0]);
    ids.into_iter()
        .filter(|id| {
            host.entity(*id)
                .and_then(|e| host.entity_kind(e))
                .is_some_and(|kind| !ignored(&kind))
        })
        .collect()
}

/// `EjectorBlockEntity.activateDeferred`, for an ejector that is charged: throws what stands on it and
/// what it holds, and opens the lid.
fn activate(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity, now: i64) {
    let data = read(&entity.nbt);
    let facing = facing_of(&*host, pos);
    let launcher = launcher(&data);
    let [x, y, z] = pos.map(f64::from);

    // Entities.
    for id in standing(host, pos) {
        let Some(body) = fan::body(host, id) else {
            continue;
        };
        host.entity_teleport_to(body.entity, [x + 0.5, y + 1.0, z + 0.5], None);
        fan::set_velocity(host, &body, launcher.motion(facing));
    }

    // Items (`launchItems`): the held stack leaves as one.
    let mut list = match util::get(&entity.nbt, LAUNCHED) {
        Some(Tag::List(list)) => list.clone(),
        _ => Vec::new(),
    };
    if let Some(stack) = entity.items.first_mut().and_then(Option::take) {
        let mut tag = Compound::new();
        tag.insert(START.into(), Tag::Long(now));
        tag.insert(SECOND.into(), stack_tag(&stack));
        list.push(Tag::Compound(tag));
        util::remove(&mut entity.nbt, HELD);
        let mut scans = scans();
        let scan = scans.entry((host.dimension().to_owned(), pos)).or_default();
        if scan.tracked.is_none() && scan.cooldown == 0 {
            scan.cooldown = rules::SCAN_INTERVAL;
            scan.tracked = Some(now);
        }
    }
    entity.nbt.insert(LAUNCHED.into(), Tag::List(list));
    util::remove(&mut entity.nbt, EARLY_TARGET);
    util::remove(&mut entity.nbt, EARLY_POS);
    entity.nbt.insert(EARLY_TIME.into(), Tag::Float(0.0_f32.to_bits()));

    set_state(&mut entity.nbt, State::Launching, 0.0, now);
    host.play_sound("minecraft:block.wooden_trapdoor.close", util::centre(pos), BLOCKS, 0.35, 1.0);
    host.play_sound("minecraft:block.chest.open", util::centre(pos), BLOCKS, 0.1, 1.4);
    host.block_entity_sync(pos);
}

/// `EjectorBlockEntity.getTargetOpenInv`'s position: the early target if the item hit something,
/// else the block the arc is aimed at.
fn target_pos(data: &Data, pos: Pos, facing: Dir, early: Option<Pos>) -> Pos {
    early.unwrap_or_else(|| rules::target_position(pos, facing, data.horizontal, data.vertical))
}

fn early_target(nbt: &Compound) -> Option<([f64; 3], Pos, f32)> {
    let Some(Tag::List(at)) = util::get(nbt, EARLY_TARGET) else {
        return None;
    };
    let Some(Tag::IntArray(block)) = util::get(nbt, EARLY_POS) else {
        return None;
    };
    if at.len() != 3 || block.len() != 3 {
        return None;
    }
    let double = |t: &Tag| match t {
        Tag::Double(bits) => f64::from_bits(*bits),
        _ => 0.0,
    };
    Some(([double(&at[0]), double(&at[1]), double(&at[2])], [block[0], block[1], block[2]], util::float(nbt, EARLY_TIME).unwrap_or(0.0)))
}

/// `EjectorBlockEntity.scanTrajectoryForObstacles`: whether the traced item meets a block between
/// `time` and the next tick; if so remembers where, to land there.
fn scan_trajectory(host: &mut dyn Host, entity: &mut ModBlockEntity, pos: Pos, facing: Dir, data: &Data, time: i32) -> bool {
    if time <= 2 {
        return false;
    }
    let launcher = launcher(data);
    let source = launcher.global_pos(f64::from(time), facing, pos);
    let target = launcher.global_pos(f64::from(time + 1), facing, pos);
    let dimension = host.dimension();
    if host.clear_between(dimension, source, target) {
        if let Some((_, _, early_time)) = early_target(&entity.nbt)
            && early_time < (time + 1) as f32
        {
            util::remove(&mut entity.nbt, EARLY_TARGET);
            util::remove(&mut entity.nbt, EARLY_POS);
            entity.nbt.insert(EARLY_TIME.into(), Tag::Float(0.0_f32.to_bits()));
        }
        return false;
    }
    let dir = [target[0] - source[0], target[1] - source[1], target[2] - source[2]];
    let length = dir.iter().map(|v| v * v).sum::<f64>().sqrt();
    let Some(hit) = host.pick_block(source, dir, length, false) else {
        return false;
    };
    let at = [hit.x, hit.y, hit.z];
    // The face the ray entered through: where the hit lies on the block's boundary, else against the throw.
    let block = [hit.block_x, hit.block_y, hit.block_z];
    let mut normal = [0.0_f64; 3];
    let mut best = f64::MAX;
    for axis in 0..3 {
        let low = (at[axis] - f64::from(block[axis])).abs();
        let high = (at[axis] - f64::from(block[axis] + 1)).abs();
        if low < best {
            best = low;
            normal = [0.0; 3];
            normal[axis] = -1.0;
        }
        if high < best {
            best = high;
            normal = [0.0; 3];
            normal[axis] = 1.0;
        }
    }
    let early = [at[0] + normal[0] * 0.25, at[1] + normal[1] * 0.25, at[2] + normal[2] * 0.25];
    let dist = |a: [f64; 3], b: [f64; 3]| (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>().sqrt();
    let early_time = f64::from(time) + dist(source, at) / dist(source, target).max(1e-9);
    entity.nbt.insert(
        EARLY_TARGET.into(),
        Tag::List(early.iter().map(|v| Tag::Double(v.to_bits())).collect()),
    );
    entity.nbt.insert(EARLY_POS.into(), Tag::IntArray(block.to_vec()));
    entity.nbt.insert(EARLY_TIME.into(), Tag::Float((early_time as f32).to_bits()));
    host.block_entity_sync(pos);
    true
}

/// `EjectorBlockEntity.placeItemAtTarget`: the item lands. A belt, depot or the like at the target
/// takes it; otherwise (or for what it didn't take) it drops as an item entity.
fn place_item_at_target(host: &mut dyn Host, entity: &ModBlockEntity, pos: Pos, data: &Data, facing: Dir, stack: ItemStack, max_time: f32) {
    let early = early_target(&entity.nbt);
    let target = target_pos(data, pos, facing, early.map(|e| e.1));
    let item = Item::dropped(stack.clone(), 0, (util::random() % 360) as i32, true);
    let rest = belt::handle_insertion(host, target, Dir::Up, &item, false, false).unwrap_or(stack);
    if rest.count == 0 {
        return;
    }
    let launcher = launcher(data);
    let at = early
        .map(|e| e.0)
        .unwrap_or_else(|| launcher.global_pos(f64::from(max_time), facing, pos));
    let motion = launcher.global_velocity(f64::from(max_time), facing).map(|v| v * 0.5);
    util::spawn_item(host, at, &rest, motion, 10);
}

/// The per-tick flight of what was thrown: items land after their flight or on a block in their way.
fn fly(host: &mut dyn Host, entity: &mut ModBlockEntity, pos: Pos, data: &Data, facing: Dir, now: i64) -> bool {
    let Some(Tag::List(list)) = util::get(&entity.nbt, LAUNCHED) else {
        return false;
    };
    if list.is_empty() {
        return false;
    }
    let list = list.clone();
    let launcher = launcher(data);
    let total = rules::total_time(&launcher);
    let key = (host.dimension().to_owned(), pos);
    let mut keep = Vec::with_capacity(list.len());
    let mut landed = Vec::new();
    for tag in list {
        let start = util::long(match &tag {
            Tag::Compound(c) => c,
            _ => continue,
        }, START);
        let Some(stack) = tag.get(SECOND).and_then(stack_of) else {
            continue;
        };
        let elapsed = (now - start).max(0) as i32;
        let tracked = scans().get(&key).is_some_and(|s| s.tracked == Some(start));
        let hit = tracked && scan_trajectory(host, entity, pos, facing, data, elapsed);
        let max_time = early_target(&entity.nbt).map_or(total, |e| e.2.min(total));
        if hit || rules::has_landed(elapsed, max_time) {
            if tracked && let Some(scan) = scans().get_mut(&key) {
                scan.tracked = None;
            }
            landed.push((stack, max_time));
        } else {
            keep.push(tag);
        }
    }
    let changed = !landed.is_empty();
    if changed {
        entity.nbt.insert(LAUNCHED.into(), Tag::List(keep));
    }
    for (stack, max_time) in landed {
        place_item_at_target(host, entity, pos, data, facing, stack, max_time);
    }
    changed
}

/// `EjectorBlockEntity.ejectIfTriggered`.
fn eject_if_triggered(host: &mut dyn Host, entity: &mut ModBlockEntity, pos: Pos, data: &Data, facing: Dir) -> bool {
    if data.powered {
        return false;
    }
    let present = held_stack(entity).map_or(0, |s| s.count);
    let settled = Transport::read(&entity.nbt).map(|t| t.pos);
    if !rules::ready(present, data.setting, settled) {
        return false;
    }
    let Some(held) = held_stack(entity).cloned() else {
        return false;
    };
    // Not while the target can't take the stack (a belt, a depot or a chute that is full): the basin
    // can't be asked first, so it is not waited for.
    let target = target_pos(data, pos, facing, None);
    if host.block(target) != BASIN {
        let probe = Item::dropped(held.clone(), 0, 0, true);
        if let Some(rest) = belt::handle_insertion(host, target, Dir::Up, &probe, false, true)
            && rest.count == held.count
        {
            return false;
        }
    }
    true
}

// --- per tick --------------------------------------------------------------------------------------

fn tick(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity) {
    let now = util::game_time(host);
    let facing = facing_of(&*host, pos);
    let prev_rpm = kinetic::speed(entity);
    let ends: Vec<Dir> = Dir::ALL.into_iter().filter(|d| d.axis() == shaft_axis(facing)).collect();
    let node = Node { impact: rules::IMPACT, ..Node::passive(Kind::shafts(&ends)) };
    let Some((rpm, changed)) = kinetic::tick(host, pos, entity, node) else {
        return; // The network jammed and the block broke.
    };
    let mut sync = changed;
    let mut data = read(&entity.nbt);

    // A change of speed changes how fast the lid winds up from here on.
    if changed && data.state == State::Retracting {
        let lid = lid_at(&data, now, prev_rpm);
        set_state(&mut entity.nbt, State::Retracting, lid, now);
        data = read(&entity.nbt);
    }

    // `updateSignal`.
    let salt = i64::from(pos[0]) + 3 * i64::from(pos[1]) + 7 * i64::from(pos[2]);
    if (now + salt).rem_euclid(POLL) == 0 {
        // `EjectorBlock.neighborChanged`: the signal the neighbour event left (`signals`).
        let powered = crate::signals::read(host, &mut entity.nbt, pos) > 0;
        if powered != data.powered {
            entity.nbt.insert(POWERED.into(), Tag::Byte(i8::from(powered)));
            data.powered = powered;
            sync = true;
        }
    }

    // What was thrown flies on.
    if fly(host, entity, pos, &data, facing, now) {
        sync = true;
    }

    match data.state {
        State::Launching => {
            if rules::lid_is_open(lid_at(&data, now, rpm)) {
                set_state(&mut entity.nbt, State::Retracting, 1.0, now);
                sync = true;
            }
        }
        State::Retracting => {
            let lid = lid_at(&data, now, rpm);
            if lid <= 0.0 {
                set_state(&mut entity.nbt, State::Charged, 0.0, now);
                sync = true;
            } else {
                let wind = rules::wind_up_speed(rpm, data.horizontal, data.vertical);
                if wind > 0.0 {
                    let rate = (1.0 / (wind * 5.0)) as i64 + 1;
                    if now.rem_euclid(rate) == 0 {
                        host.play_sound("minecraft:block.wooden_button.click_off", util::centre(pos), BLOCKS, 0.125, 1.5 - lid);
                    }
                }
            }
        }
        State::Charged => {
            if (now + salt).rem_euclid(POLL) == 0 {
                catch_stray_items(host, entity, pos, &data);
            }
            let launching = eject_if_triggered(host, entity, pos, &data, facing) || step_on(host, pos, &data);
            if launching {
                activate(host, pos, entity, now);
                sync = false;
            }
        }
    }

    // The held stack slides to the middle (`DepotBehaviour.tick`).
    let saved = Transport::read(&entity.nbt);
    match (held_stack(entity).is_some(), saved) {
        (true, Some(mut transport)) => {
            transport.tick();
            if Some(transport) != saved {
                entity.nbt.insert(HELD.into(), Tag::Compound(transport.write()));
                sync = true;
            }
        }
        // A stack that arrived without its state (a hopper's) sits in the middle.
        (true, None) => {
            let transport = Transport::new((util::random() % 360) as i32, Dir::Up, 0.5);
            entity.nbt.insert(HELD.into(), Tag::Compound(transport.write()));
            sync = true;
        }
        (false, Some(_)) => {
            util::remove(&mut entity.nbt, HELD);
            sync = true;
        }
        (false, None) => {}
    }

    if sync {
        host.block_entity_sync(pos);
    }
}

/// What stands on a charged ejector triggers it (`EjectorBlock.updateEntityAfterFallOn`): drawn to
/// the middle of the lid first, thrown once it is there. True once something is there to throw.
fn step_on(host: &mut dyn Host, pos: Pos, data: &Data) -> bool {
    if data.powered {
        return false;
    }
    let [x, y, z] = pos.map(f64::from);
    let centre = [x + 0.5, y + 0.5 + 7.0 / 16.0, z + 0.5];
    let mut ready = false;
    for id in standing(host, pos) {
        let Some(body) = fan::body(host, id) else {
            continue;
        };
        if !host.entity_on_ground(body.entity).unwrap_or(false) {
            continue;
        }
        let diff = (0..3).map(|i| (centre[i] - body.position[i]).powi(2)).sum::<f64>().sqrt();
        if diff > 4.0 / 16.0 {
            // `setPos(midpoint)`: eased halfway to the middle.
            let half = [
                (centre[0] + body.position[0]) / 2.0,
                (centre[1] + body.position[1]) / 2.0,
                (centre[2] + body.position[2]) / 2.0,
            ];
            host.entity_teleport_to(body.entity, half, None);
            continue;
        }
        ready = true;
    }
    ready
}

/// `SharedDepotBlockMethods.onLanded`: an item entity lying on a charged ejector is taken in.
fn catch_stray_items(host: &mut dyn Host, entity: &mut ModBlockEntity, pos: Pos, data: &Data) {
    let [x, y, z] = pos.map(f64::from);
    let dimension = host.dimension();
    for item in host.item_entities(dimension, [x, y, z], [x + 1.0, y + 1.0, z + 1.0]) {
        let [px, py, pz] = item.position;
        if !(x..x + 1.0).contains(&px) || !(z..z + 1.0).contains(&pz) || !(y..y + 1.0).contains(&py) {
            continue;
        }
        if !host.entity(item.id).and_then(|e| host.entity_on_ground(e)).unwrap_or(false) {
            continue;
        }
        let room = space(&*host, data, entity, &item.stack);
        if room <= 0 || held_stack(entity).is_some_and(|held| !util::same_item(held, &item.stack)) {
            continue;
        }
        let taken = item.stack.count.min(room as u32);
        let mut rest = item.stack.clone();
        rest.count -= taken;
        host.set_item_entity(dimension, item.id, rest);
        if entity.items.is_empty() {
            entity.items.push(None);
        }
        match entity.items[0].as_mut() {
            Some(held) => held.count += taken,
            None => {
                let mut put = item.stack.clone();
                put.count = taken;
                entity.items[0] = Some(put);
                let transport = Transport::new((util::random() % 360) as i32, Dir::Down, 0.5);
                entity.nbt.insert(HELD.into(), Tag::Compound(transport.write()));
            }
        }
        host.play_sound("minecraft:entity.item_frame.add_item", util::centre(pos), BLOCKS, 0.25, 1.25);
        host.block_entity_sync(pos);
        return;
    }
}

// --- drawing -------------------------------------------------------------------------------------

/// `Direction.getClockWise` on the horizontal angle: `AngleHelper.horizontalAngle`.
fn angle_of(facing: Dir) -> f32 {
    rules::horizontal_angle(facing)
}

const UP: [f32; 3] = [0.0, 1.0, 0.0];
const ORIGIN: [f32; 3] = [0.0; 3];
const CENTRE: [f32; 3] = [0.5; 3];
const LID_PIVOT: [f32; 3] = [0.0, 11.25 / 16.0, 0.75 / 16.0];

/// `java.util.Random`, for the scatter of a block item's copies in a stack (`DepotRenderer` seeds it
/// with 0).
struct JavaRandom(u64);

impl JavaRandom {
    const MULTIPLIER: u64 = 0x5_DEEC_E66D;
    const MASK: u64 = (1 << 48) - 1;

    fn new(seed: i64) -> Self {
        JavaRandom((seed as u64 ^ Self::MULTIPLIER) & Self::MASK)
    }

    fn next_float(&mut self) -> f32 {
        self.0 = (self.0.wrapping_mul(Self::MULTIPLIER).wrapping_add(0xB)) & Self::MASK;
        (self.0 >> 24) as u32 as f32 / (1 << 24) as f32
    }
}

/// `DepotRenderer.renderItem`: a block lies in a small heap, any other item flat in a fan, one copy
/// more for each fourfold of the count.
fn draw_stack(host: &dyn Host, draw: &mut crate::compat::BlockEntityDraw, stack: &ItemStack, angle: i32) {
    let mut r = JavaRandom::new(0);
    let copies = stack.count.max(1).ilog2() / 2;
    let block = host.default_state(&stack.item).is_some();
    draw.push();
    draw.rotate_about(UP, angle as f32, ORIGIN);
    for i in 0..=copies {
        let i = i as f32;
        draw.push();
        if block {
            let x = r.next_float() * 0.0625 * i;
            let z = r.next_float() * 0.0625 * i;
            draw.translate([x, 0.0, z]);
        }
        draw.scale([0.5; 3]);
        if !block {
            draw.translate([0.0, -3.0 / 16.0, 0.0]);
            draw.rotate_about([1.0, 0.0, 0.0], 90.0, ORIGIN);
        }
        host.draw_item(draw, stack, DisplayContext::Fixed, None, 0);
        draw.pop();
        if !block {
            draw.rotate_about(UP, 10.0, ORIGIN);
        }
        draw.translate([0.0, if block { 1.0 / 64.0 } else { 1.0 / 16.0 }, 0.0]);
    }
    draw.pop();
}

fn draw(host: &dyn Host, draw: &mut crate::compat::BlockEntityDraw, state: BlockStateId, pos: Pos, entity: &ModBlockEntity) {
    let facing = host.block_state_property(state, "facing").and_then(Dir::from_name).unwrap_or(Dir::North);
    let data = read(&entity.nbt);
    let rpm = kinetic::speed(entity);
    let now = draw.game_time;
    let partial = draw.partial;

    // `ShaftRenderer`: the whole shaft across the facing.
    kinetic::draw_shaft(host, draw, shaft_axis(facing), rpm);

    // The lid, wound up or thrown open (`applyLidAngle`).
    let t = |n: i64| lid_at(&data, n, rpm);
    let lid = t(now) + (t(now + 1) - t(now)) * partial;
    let angle = lid * 70.0;
    let turn = 180.0 + angle_of(facing);
    draw.push();
    draw.rotate_about(UP, turn, CENTRE);
    draw.rotate_about([1.0, 0.0, 0.0], -angle, LID_PIVOT);
    host.draw_model(draw, TOP_MODEL, 0xFF_FFFF);
    draw.pop();

    // What lies on the lid.
    let held = match (Transport::read(&entity.nbt), util::get(&entity.nbt, HELD).and_then(|h| h.get(ITEM)).and_then(stack_of)) {
        (Some(transport), Some(stack)) => Some((transport, stack)),
        _ => None,
    };
    let mut at_rest = true;
    if let Some((transport, stack)) = held {
        draw.push();
        draw.rotate_about(UP, turn, CENTRE);
        draw.rotate_about([1.0, 0.0, 0.0], -angle, LID_PIVOT);
        draw.rotate_about(UP, -180.0 - angle_of(facing), CENTRE);
        draw.translate([0.5, 15.0 / 16.0, 0.5]);
        draw.translate(nudge());
        let along = transport.prev_pos + (transport.pos - transport.prev_pos) * partial;
        let side = transport.prev_offset + (transport.offset - transport.prev_offset) * partial;
        if transport.from.axis() != create_kinetics::Axis::Y {
            let normal = transport.from.opposite().normal().map(|v| v as f32 * (0.5 - along));
            draw.translate(normal);
            let along_x = clockwise(transport.from).axis() == create_kinetics::Axis::X;
            draw.translate(if along_x { [side, 0.0, 0.0] } else { [0.0, 0.0, -side] });
        }
        draw_stack(host, draw, &stack, transport.angle);
        draw.pop();
        at_rest = transport.at_rest();
    }

    // What flies (`launchedItems`).
    let flying = match util::get(&entity.nbt, LAUNCHED) {
        Some(Tag::List(list)) => list.as_slice(),
        _ => &[],
    };
    if !flying.is_empty() {
        let launcher = launcher(&data);
        let max_time = early_target(&entity.nbt)
            .filter(|e| e.2 > 0.0)
            .map_or(launcher.total_flying_ticks() as f32, |e| e.2);
        for tag in flying {
            let Some(stack) = tag.get(SECOND).and_then(stack_of) else {
                continue;
            };
            let start = match tag {
                Tag::Compound(c) => util::long(c, START),
                _ => continue,
            };
            let time = (now - start) as f32 + partial;
            if time > max_time || time < 0.0 {
                continue;
            }
            let location = launcher.global_pos(f64::from(time), facing, pos);
            let rot = [0.0, 2.0 / 16.0, -1.0 / 16.0];
            draw.push();
            draw.translate([
                (location[0] - f64::from(pos[0])) as f32,
                (location[1] - f64::from(pos[1])) as f32,
                (location[2] - f64::from(pos[2])) as f32,
            ]);
            draw.translate(rot);
            draw.scale([0.5; 3]);
            draw.rotate_about(UP, angle_of(facing), ORIGIN);
            draw.rotate_about([1.0, 0.0, 0.0], time * 40.0, ORIGIN);
            draw.translate(rot.map(|v| -v));
            host.draw_item(draw, &stack, DisplayContext::Fixed, None, 0);
            draw.pop();
        }
    }
    draw.still();
    // Kept only while nothing moves: the lid shut and charged, and the held stack at rest.
    if data.state == State::Charged && at_rest && flying.is_empty() {
        draw.keep();
    }
}

/// Flywheel's `Translate.nudge(0)`, as `depot.rs` has it.
fn nudge() -> [f32; 3] {
    [-0.00175; 3]
}

// --- the hooks -----------------------------------------------------------------------------------

/// What `EjectorTargetHandler.flushSettings` does once the ejector is placed: the target it was
/// sent to, if the selection is valid.
fn flush_target(host: &mut dyn Host, pos: Pos, player: &NativeServerPlayer) {
    let selection = selected().remove(&player.uuid);
    let Some(target) = selection else {
        say(host, player, "Ejecting to the adjacent block (no target was selected)");
        return;
    };
    let Some(facing) = rules::valid_direction(pos, target, rules::MAX_DISTANCE) else {
        say(host, player, "Ejecting to the adjacent block (target was not valid)");
        return;
    };
    say(host, player, &format!("Ejecting to [{}, {}, {}]", target[0], target[1], target[2]));
    let (h, v) = rules::distances(pos, target);
    let (h, v) = rules::target_distances(h, v);
    host.change_mod_block_entity(pos, &mut |entity| {
        entity.nbt.insert(HORIZONTAL.into(), Tag::Int(h));
        entity.nbt.insert(VERTICAL.into(), Tag::Int(v));
    });
    host.set_block_property(pos, "facing", facing.name());
    host.block_entity_sync(pos);
}

fn say(host: &mut dyn Host, player: &NativeServerPlayer, line: &str) {
    host.action_bar(player.uuid, Text::literal(line.to_owned()));
}

impl Hooks for Ejector {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.push(TOP_MODEL.to_owned());
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == ID).then(|| {
            // `new EntityLauncher(1, 0)`, `state = RETRACTING`, `lidProgress.startWithValue(1)`.
            let mut nbt = kinetic::new_entity();
            nbt.insert(HORIZONTAL.into(), Tag::Int(1));
            nbt.insert(VERTICAL.into(), Tag::Int(0));
            nbt.insert(POWERED.into(), Tag::Byte(0));
            nbt.insert(SCROLL.into(), Tag::Int(0));
            set_state(&mut nbt, State::Retracting, 1.0, 0);
            (ID.to_owned(), nbt)
        })
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id == ID {
            tick(host, event.pos, event.entity);
        }
    }

    /// `EjectorBlockEntity.write` for clients (`clientPacket`).
    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        if event.entity.id != ID {
            return None;
        }
        let nbt = &event.entity.nbt;
        let mut tag = kinetic::update_tag(event.entity);
        for key in [HORIZONTAL, VERTICAL, POWERED, STATE, LID, STATE_START, EARLY_TIME] {
            if let Some(value) = util::get(nbt, key) {
                tag.insert(key.into(), value.clone());
            }
        }
        // Always sent, empty when there is none, so a client drops what it had.
        for key in [LAUNCHED, EARLY_TARGET] {
            tag.insert(key.into(), util::get(nbt, key).cloned().unwrap_or(Tag::List(Vec::new())));
        }
        tag.insert(EARLY_POS.into(), util::get(nbt, EARLY_POS).cloned().unwrap_or(Tag::IntArray(Vec::new())));
        // The held stack goes with its stack in it, since a client has no item slots.
        let held = match (held_stack(event.entity), Transport::read(nbt)) {
            (Some(stack), Some(transport)) => {
                let mut held = transport.write();
                held.insert(ITEM.into(), stack_tag(stack));
                held
            }
            _ => Compound::new(),
        };
        tag.insert(HELD.into(), Tag::Compound(held));
        Some(tag)
    }

    /// `EjectorBlockEntity.getRenderBoundingBox` is infinite: a launched item is drawn wherever it flies.
    fn block_entity_render_bounding_box(
        &self,
        _: &dyn Host,
        event: &mut BlockEntityRenderBoundingBoxEvent<'_>,
    ) {
        if event.block == ID {
            let far = rules::MAX_DISTANCE as f32 + 2.0;
            for i in 0..3 {
                event.bounds[i] = event.bounds[i].min(-far);
                event.bounds[i + 3] = event.bounds[i + 3].max(1.0 + far);
            }
        }
    }

    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        draw(host, &mut *event.draw, event.state, event.pos, event.entity);
    }

    /// The hopper side of `DepotItemHandler`: one slot, from every side.
    fn block_entity_container(&self, _: &dyn Host, event: &mut BlockEntityContainerEvent<'_>) -> Option<ModContainer> {
        (event.block == ID).then(|| ModContainer {
            slots: 1,
            faces: [(); 6].map(|()| vec![0]),
            inputs: vec![0],
            outputs: vec![0],
            menu: None,
        })
    }

    /// `canAcceptItems`: a hopper puts items in only while the ejector is charged.
    fn container_accepts(&self, _: &dyn Host, event: &mut ContainerAcceptsEvent<'_>) -> Flow {
        if event.entity.id != ID {
            return Flow::Continue;
        }
        if read(&event.entity.nbt).state != State::Charged {
            return Flow::Cancel;
        }
        Flow::Continue
    }

    /// `EjectorBlock.getStateForPlacement` (`HorizontalKineticBlock`): faces the player, to be turned
    /// towards the target by `place_block`.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        if &*event.context.item != ID {
            return None;
        }
        let facing = looking(event.context.yaw, 0.0).opposite();
        level.state_with_property(event.state, "facing", facing.name()).map(Some)
    }

    /// `EjectorItem.useOn`: sneaking with the item selects a target instead of placing.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item == util::WRENCH {
            if host.block(event.pos) != ID {
                return Flow::Continue;
            }
            // `EjectorBlock.useItemOn` lets the Wrench through to the block.
            return util::wrench(host, event.pos, |_| {});
        }
        if &*event.stack.item != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if !player.sneaking {
            return Flow::Continue;
        }
        selected().insert(player.uuid, event.pos);
        say(host, &player, "Target selected");
        Flow::Cancel
    }

    /// `EjectorPlacementPacket`: once placed, the ejector faces and aims at the selected block.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if &*event.stack.item != ID {
            return;
        }
        if let Some(player) = host.acting_player() {
            flush_target(host, event.pos, &player);
        }
    }

    /// `SharedDepotBlockMethods.onUse`: a click on the top takes what is on the ejector back
    /// and puts the stack in the hand down on it, while it is charged.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == SPECTATOR {
            return Flow::Continue;
        }
        let uuid = player.uuid;
        let main = host.player_equipment(uuid, EquipmentSlot::MainHand);
        // The Wrench is let through to the block; sneaking with anything skips the block's own use.
        if main.as_ref().is_some_and(|s| &*s.item == util::WRENCH) {
            return Flow::Continue;
        }
        if player.sneaking
            && (main.is_some() || host.player_equipment(uuid, EquipmentSlot::OffHand).is_some())
        {
            return Flow::Continue;
        }
        if event.face != [0, 1, 0] {
            return Flow::Continue;
        }
        // `!behaviour.canAcceptItems.get()`: a click while it winds up or fires does nothing.
        if state_of(&*host, pos) != State::Charged {
            return Flow::Cancel;
        }
        // Once a click: the off hand's turn would take back what the first put down.
        if !util::first_click(host, uuid, pos) {
            return Flow::Cancel;
        }
        let mut taken: Vec<ItemStack> = Vec::new();
        host.change_mod_block_entity(pos, &mut |entity| {
            for slot in entity.items.iter_mut() {
                taken.extend(slot.take());
            }
            util::remove(&mut entity.nbt, HELD);
        });
        let took = !taken.is_empty();
        for stack in taken {
            if let Some(rest) = host.give_stack(uuid, stack) {
                util::spawn_item(host, player.position, &rest, [0.0; 3], 0);
            }
        }
        if took {
            let pitch = 1.0 + (util::random() % 1_000_000) as f32 / 1_000_000.0;
            host.play_sound("minecraft:entity.item.pickup", util::centre(pos), PLAYERS, 0.2, pitch);
        }
        if main.is_some()
            && let Some(stack) = host.player_equipment(uuid, EquipmentSlot::MainHand)
            && &*stack.item != MECHANICAL_ARM
            && empty_main_hand(host, &player, &stack)
        {
            let transport = Transport::new((util::random() % 360) as i32, looking(player.yaw, 0.0), 0.25);
            let mut put = Some(stack);
            host.change_mod_block_entity(pos, &mut |entity| {
                if entity.items.is_empty() {
                    entity.items.push(None);
                }
                entity.items[0] = put.take();
                entity.nbt.insert(HELD.into(), Tag::Compound(transport.write()));
            });
            host.play_sound("minecraft:block.sand.break", util::centre(pos), BLOCKS, 0.125, 1.5);
        }
        host.block_entity_sync(pos);
        Flow::Cancel
    }

    /// `EjectorBlockEntity.destroy`: what is still in the air drops where it was.
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(entity) = host.mod_block_entity(pos) else {
            return Flow::Continue;
        };
        let data = read(&entity.nbt);
        let facing = facing_of(&*host, pos);
        let launcher = launcher(&data);
        let now = util::game_time(&*host);
        if let Some(Tag::List(list)) = util::get(&entity.nbt, LAUNCHED) {
            for tag in list {
                let (Some(stack), Tag::Compound(c)) = (tag.get(SECOND).and_then(stack_of), tag) else {
                    continue;
                };
                let time = f64::from((now - util::long(c, START)).max(0) as i32);
                let at = launcher.global_pos(time, facing, pos);
                let motion = launcher.global_velocity(time, facing).map(|v| v * 0.5);
                util::spawn_item(host, at, &stack, motion, 10);
            }
        }
        Flow::Continue
    }
}

/// `player.setItemInHand(hand, ItemStack.EMPTY)`; as `depot.rs` does it inside a player's own hook.
fn empty_main_hand(host: &mut dyn Host, player: &NativeServerPlayer, stack: &ItemStack) -> bool {
    let living = u32::try_from(player.entity_id)
        .ok()
        .filter(|id| *id != 0)
        .and_then(|id| host.entity(id))
        .and_then(|entity| host.living(entity));
    if let Some(living) = living
        && host.living_set_equipment(living, EquipmentSlot::MainHand, None)
    {
        return true;
    }
    if stack.extra.is_none() {
        let taken = host.take_items(&stack.item, stack.count);
        if taken == stack.count {
            return true;
        }
        if taken > 0 {
            host.give_items(&stack.item, taken);
            return false;
        }
    }
    host.set_inventory_slot_typed(
        player.uuid,
        NativeInventoryKind::Player,
        usize::from(player.selected_slot),
        Some(stack),
        None,
    )
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_saved_state_reads_back() {
        let mut nbt = Compound::new();
        nbt.insert(HORIZONTAL.into(), Tag::Int(7));
        nbt.insert(VERTICAL.into(), Tag::Int(-3));
        nbt.insert(POWERED.into(), Tag::Byte(1));
        nbt.insert(SCROLL.into(), Tag::Int(16));
        set_state(&mut nbt, State::Charged, 0.0, 77);
        let data = read(&nbt);
        assert_eq!((data.horizontal, data.vertical, data.powered, data.setting), (7, -3, true, 16));
        assert_eq!((data.state, data.start), (State::Charged, 77));
        // A bare entity is Create's new one: aimed one block ahead, retracting.
        let data = read(&Compound::new());
        assert_eq!((data.horizontal, data.vertical, data.state), (1, 0, State::Retracting));
    }

    #[test]
    fn the_shaft_runs_across_the_facing() {
        assert_eq!(shaft_axis(Dir::North), create_kinetics::Axis::X);
        assert_eq!(shaft_axis(Dir::East), create_kinetics::Axis::Z);
    }

    #[test]
    fn a_held_stack_slides_and_saves() {
        let mut t = Transport::new(200, Dir::North, 0.25);
        for _ in 0..40 {
            t.tick();
        }
        assert!(t.at_rest() && (t.pos - 0.5).abs() <= 1.0 / 512.0);
        let mut nbt = Compound::new();
        nbt.insert(HELD.into(), Tag::Compound(t.write()));
        assert_eq!(Transport::read(&nbt), Some(t));
        assert_eq!(Transport::read(&Compound::new()), None);
    }

    #[test]
    fn stacks_round_trip_through_nbt() {
        let stack = ItemStack::new("minecraft:iron_ingot", 12);
        let back = stack_of(&stack_tag(&stack)).unwrap();
        assert_eq!((&*back.item, back.count), ("minecraft:iron_ingot", 12));
    }
}
