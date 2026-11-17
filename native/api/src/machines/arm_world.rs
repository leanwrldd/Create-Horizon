//! What a Mechanical Arm's points do in the world (`AllArmInteractionPointTypes`: each type's
//! `canCreatePoint`, `insert`, `extract` and `getSlotCount`), by Horizon's container calls and the
//! other modules' hand-offs. The decisions are `create_kinetics::rules::arm` and `arm_points`; this
//! reads the world, asks them and applies the answer.
//!
//! Known differences from Create, for `arm.rs`'s list:
//! - A container can't be asked "would this fit" without putting it in, so the slots of the
//!   basin, millstone, deployer and crushing wheel controller (read from the block entity) and of
//!   a funnel's chest (read from its saved `Items` and sized by the block) are worked out here
//!   with the handlers' rules (`insert_into_slots`) and the real insertion is left to the game.
//! - A belt that is stopped can't be taken from (the belt hands its items out while it moves).
//! - The Mechanical Saw and the Crushing Wheels take items but give none back; a funnel's flap
//!   doesn't swing for the arm.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use create_kinetics::rules::arm::{Angles, Arm, Env};
use create_kinetics::rules::arm_items as items;
use create_kinetics::rules::arm_points::{self as points, Block, Fed, Point, PointType};
use create_kinetics::rules::{basin, millstone};
use create_kinetics::{Dir, Pos};
use horizon_client::mod_api::{Direction, Hooks, Host, ItemStack, NativeUseTarget, Tag};

use super::burner;
use super::chute_inv as inv;
use super::crafter_world;
use super::saw;
use crate::{belt, depot, recipes, util};

/// A module of shared code: no hooks of its own.
struct Shared;

impl Hooks for Shared {}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

const DEPOT: &str = "create:depot";
const MILLING: &str = "create:milling";
/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;
/// `DepotItemHandler.getSlots`.
const DEPOT_SLOTS: usize = 9;

/// Where the arm keeps the belt block whose item it is reaching for (`keepAlive`), and when: the
/// belt asks through `belt::register_hold`.
static KEEP: LazyLock<Mutex<HashMap<Pos, i64>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn keep() -> std::sync::MutexGuard<'static, HashMap<Pos, i64>> {
    KEEP.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The belt's question: is an arm reaching for the item at this belt block?
pub(super) fn holds(host: &dyn Host, at: Pos) -> bool {
    let now = util::game_time(host);
    keep().get(&at).is_some_and(|since| now - since <= 2)
}

/// Forgets belts nobody has reached for lately.
pub(super) fn prune(now: i64) {
    keep().retain(|_, since| now - *since < 100);
}

/// `ArmInteractionPointType.getPrimaryType` of the block at `pos`.
pub(super) fn classify_at(host: &dyn Host, pos: Pos) -> Option<PointType> {
    let name = host.block(pos);
    let above = host.block([pos[0], pos[1] + 1, pos[2]]);
    let property = |key: &str| host.block_property(pos, key);
    let campfire = host.block_name_in_tag(&name, "minecraft:campfires").unwrap_or(false);
    let moving = name == "create:mechanical_saw"
        && host.mod_block_entity(pos).is_some_and(|e| crate::kinetic::speed(&e) != 0.0);
    points::classify(&Block { name: &name, property: &property, above: &above, campfire, moving })
}

fn stack_of_count(stack: &ItemStack, count: u32) -> ItemStack {
    let mut out = stack.clone();
    out.count = count;
    out
}

/// What is left of `stack` when `taken` of it went in.
fn minus(stack: ItemStack, taken: u32) -> ItemStack {
    let left = stack.count.saturating_sub(taken);
    stack_of_count(&stack, left)
}

/// A stack with its count made 1, the key two stacks are compared by.
fn key_of(stack: &ItemStack) -> ItemStack {
    stack_of_count(stack, 1)
}

/// The slots of a container as `insert_into_slots` reads them.
fn slot_keys(slots: &[Option<ItemStack>]) -> Vec<Option<(ItemStack, u32)>> {
    slots.iter().map(|s| s.as_ref().map(|s| (key_of(s), s.count))).collect()
}

fn max_stack(host: &dyn Host, stack: &ItemStack) -> u32 {
    host.max_stack_size(&stack.item).unwrap_or(64)
}

fn digit(host: &dyn Host, pos: Pos, key: &str) -> Option<u8> {
    host.block_property(pos, key)?.parse().ok()
}

/// The world as one arm sees it: its two lists of points and what each does.
pub(super) struct ArmWorld<'a> {
    pub host: &'a mut dyn Host,
    pub arm: Pos,
    pub ceiling: bool,
    pub inputs: Vec<Point>,
    pub outputs: Vec<Point>,
    kinds: HashMap<Pos, Option<PointType>>,
}

impl<'a> ArmWorld<'a> {
    pub fn new(host: &'a mut dyn Host, arm: Pos, ceiling: bool, all: &[Point]) -> Self {
        let (inputs, outputs) = points::split(all);
        ArmWorld {
            host,
            arm,
            ceiling,
            inputs: inputs.into_iter().map(|i| all[i]).collect(),
            outputs: outputs.into_iter().map(|i| all[i]).collect(),
            kinds: HashMap::new(),
        }
    }

    /// `ArmInteractionPoint.isValid`.
    pub fn valid(&mut self, point: &Point) -> bool {
        let host = &*self.host;
        let kind = *self.kinds.entry(point.pos).or_insert_with(|| classify_at(host, point.pos));
        kind == Some(point.ty)
    }

    /// `getTargetAngles` of a point, reading the facing the Deployer and the funnels turn it by.
    pub fn angles(&self, point: &Point) -> Angles {
        let facing = matches!(point.ty, PointType::Deployer | PointType::Funnel | PointType::Crafter)
            .then(|| util::facing(&*self.host, point.pos, "facing"))
            .flatten();
        points::point_angles(point.ty, point.pos, facing, self.arm, self.ceiling)
    }

    fn input(&self, index: usize) -> Point {
        self.inputs[index]
    }
}

impl Env<ItemStack> for ArmWorld<'_> {
    fn count(&self, stack: &ItemStack) -> u32 {
        stack.count
    }

    fn same_item(&self, a: &ItemStack, b: &ItemStack) -> bool {
        a.item == b.item
    }

    fn matches(&self, a: &ItemStack, b: &ItemStack) -> bool {
        a == b
    }

    fn input_count(&self) -> usize {
        self.inputs.len()
    }

    fn output_count(&self) -> usize {
        self.outputs.len()
    }

    fn input_valid(&mut self, index: usize) -> bool {
        let point = self.input(index);
        self.valid(&point)
    }

    fn output_valid(&mut self, index: usize) -> bool {
        let point = self.outputs[index];
        self.valid(&point)
    }

    fn input_slots(&mut self, index: usize) -> usize {
        let point = self.input(index);
        slots(&*self.host, &point).end
    }

    fn extract(&mut self, index: usize, slot: usize, amount: u32, simulate: bool) -> Option<ItemStack> {
        let point = self.input(index);
        extract(self.host, &point, slot, amount, simulate).filter(|s| s.count > 0)
    }

    fn insert(&mut self, index: usize, stack: ItemStack, simulate: bool) -> ItemStack {
        let point = self.outputs[index];
        insert(self.host, &point, stack, simulate)
    }

    /// `BeltPoint.keepAlive`: the first item of the belt block stops where it is.
    fn keep_alive(&mut self, index: usize) {
        let point = self.input(index);
        if point.ty != PointType::Belt {
            return;
        }
        let now = util::game_time(&*self.host);
        keep().insert(point.pos, now);
        belt::edit_items(self.host, point.pos, &mut |_, lane, items| {
            let middle = lane.index as f32 + 0.5;
            match items.iter_mut().find(|i| (i.pos - middle).abs() < 0.51) {
                Some(item) if !item.locked => {
                    item.locked = true;
                    true
                }
                _ => false,
            }
        });
    }

    fn has_music(&mut self) -> bool {
        let all: Vec<Point> = self.inputs.iter().chain(&self.outputs).copied().collect();
        all.iter().any(|p| {
            p.ty == PointType::Jukebox
                && self.host.block_property(p.pos, "has_record").as_deref() == Some("true")
        })
    }
}

// --- what a point holds -----------------------------------------------------------------------------

/// The slots of the point's block an arm can take from, and with it `getSlotCount`: the container's
/// own slot numbers, so a range that starts above 0 counts the slots it can't take from.
fn slots(host: &dyn Host, point: &Point) -> std::ops::Range<usize> {
    if let Some(n) = point.ty.fixed_slots() {
        return 0..n;
    }
    match point.ty {
        // `CombinedInvWrapper(inputInventory, outputInventory)`: all of it can be taken.
        PointType::Basin => 0..basin::SLOTS,
        // `MillstoneInventoryHandler.extractItem`: not from the input.
        PointType::Millstone => millstone::INPUT_SLOTS..millstone::INPUT_SLOTS + millstone::OUTPUT_SLOTS,
        PointType::Depot => {
            if host.block(point.pos) == DEPOT {
                0..DEPOT_SLOTS
            } else {
                0..1
            }
        }
        // `1 + overflowItems.size()`: the held stack and what did not fit.
        PointType::Deployer => {
            let used = host
                .mod_block_entity(point.pos)
                .map_or(0, |e| e.items.iter().rposition(Option::is_some).map_or(0, |i| i + 1));
            0..used.max(1)
        }
        // `getSlotCount`: one slot to each crafter of the grid.
        PointType::Crafter => 0..crafter_world::handler_slots(host, point.pos),
        PointType::Chute | PointType::Belt => 0..1,
        _ => 0..0,
    }
}

fn extract(
    host: &mut dyn Host,
    point: &Point,
    slot: usize,
    amount: u32,
    simulate: bool,
) -> Option<ItemStack> {
    let pos = point.pos;
    match point.ty {
        PointType::Belt => belt_extract(host, pos, amount, simulate),
        // `CrafterPoint.extract`: the arm may take what a crafter holds (Create allows extraction
        // for the arm only).
        PointType::Crafter => crafter_world::handler_extract(host, pos, slot, simulate),
        PointType::Composter => {
            let level = digit(host, pos, "level")?;
            let item = points::composter_output(level)?;
            if slot != 0 {
                return None;
            }
            if !simulate {
                // `ComposterBlock.extractProduce`: the level drops to 0.
                host.set_block_property(pos, "level", "0");
                host.play_sound("minecraft:block.composter.empty", util::centre(pos), BLOCKS, 1.0, 1.0);
            }
            Some(ItemStack::new(item, 1))
        }
        PointType::Jukebox => {
            if host.block_property(pos, "has_record").as_deref() != Some("true") {
                return None;
            }
            let dimension = host.dimension();
            if simulate {
                let Some(Tag::Compound(tag)) = host.block_entity_tag(dimension, pos) else {
                    return None;
                };
                return util::get(&tag, "RecordItem").and_then(util::stack_of);
            }
            host.extract_item(dimension, pos, Direction::Up, 0, 1)
        }
        _ => {
            if !slots(&*host, point).contains(&slot) || slot < container_start(point.ty) {
                return None;
            }
            let have = host.mod_block_entity(pos)?.items.get(slot)?.clone()?;
            let n = amount.min(have.count);
            if n == 0 {
                return None;
            }
            if simulate {
                return Some(stack_of_count(&have, n));
            }
            let dimension = host.dimension();
            host.extract_item(dimension, pos, Direction::Up, slot, n)
        }
    }
}

/// The first slot of the container an arm can take from.
fn container_start(ty: PointType) -> usize {
    if ty == PointType::Millstone { millstone::INPUT_SLOTS } else { 0 }
}

/// `ItemHandlerBeltSegment.extractItem(0, amount, simulate)`: from the first stack on the belt
/// block (`getStackAtOffset`: past the block's start, up to its end).
fn belt_extract(host: &mut dyn Host, pos: Pos, amount: u32, simulate: bool) -> Option<ItemStack> {
    let mut out = None;
    belt::edit_items(host, pos, &mut |_, lane, items| {
        let low = lane.index as f32;
        let Some(item) = items.iter_mut().find(|i| i.pos > low && i.pos <= low + 1.0) else {
            return false;
        };
        let n = amount.min(item.stack.count);
        out = Some(stack_of_count(&item.stack, n));
        if simulate || n == 0 {
            return false;
        }
        item.stack.count -= n;
        true
    });
    out
}

// --- putting items in -----------------------------------------------------------------------------------

/// Gives `stack` to the container at `pos` as a hopper would, answering what is left; with
/// `simulate`, what `room` items of it would leave.
fn put(host: &mut dyn Host, pos: Pos, stack: ItemStack, simulate: bool, room: u32) -> ItemStack {
    if simulate {
        return minus(stack, room);
    }
    match host.insert_item(pos, Direction::Up, stack.clone()) {
        Some(rest) => rest,
        None => stack_of_count(&stack, 0),
    }
}

fn insert(host: &mut dyn Host, point: &Point, stack: ItemStack, simulate: bool) -> ItemStack {
    let pos = point.pos;
    if stack.count == 0 {
        return stack;
    }
    match point.ty {
        PointType::Basin => {
            let mut held = host.mod_block_entity(pos).map(|e| e.items.clone()).unwrap_or_default();
            held.resize(basin::SLOTS, None);
            let inputs = &held[..basin::INPUT_SLOTS];
            let empty: Vec<bool> = inputs.iter().map(Option::is_none).collect();
            let same = |slot: usize| {
                inputs.iter().enumerate().any(|(i, s)| {
                    i != slot && s.as_ref().is_some_and(|s| util::same_item(s, &stack))
                })
            };
            let accepts = |slot: usize| basin::may_insert_input(slot, &empty, same(slot));
            let left = points::insert_into_slots(
                &slot_keys(inputs),
                &key_of(&stack),
                stack.count,
                &|_| basin::SLOT_LIMIT,
                &accepts,
            );
            put(host, pos, stack.clone(), simulate, stack.count - left)
        }
        PointType::Millstone => {
            // `MillstoneInventoryHandler.isItemValid`: only what a recipe takes.
            if recipes::for_stack(&*host, MILLING, &stack).is_none() {
                return stack;
            }
            let mut held = host.mod_block_entity(pos).map(|e| e.items.clone()).unwrap_or_default();
            held.resize(millstone::INPUT_SLOTS, None);
            let limit = max_stack(&*host, &stack).min(millstone::SLOT_LIMIT);
            let left = points::insert_into_slots(
                &slot_keys(&held[..millstone::INPUT_SLOTS]),
                &key_of(&stack),
                stack.count,
                &|_| limit,
                &|_| true,
            );
            put(host, pos, stack.clone(), simulate, stack.count - left)
        }
        PointType::Deployer => {
            let Some(entity) = host.mod_block_entity(pos) else {
                return stack;
            };
            // `DeployerItemHandler.isItemValid`: the filter's, for the held slot.
            if !inv::filter_of(&entity.nbt).test(host, &stack) {
                return stack;
            }
            let held = vec![entity.items.first().cloned().flatten()];
            let limit = max_stack(&*host, &stack);
            let left = points::insert_into_slots(
                &slot_keys(&held),
                &key_of(&stack),
                stack.count,
                &|_| limit,
                &|_| true,
            );
            put(host, pos, stack.clone(), simulate, stack.count - left)
        }
        PointType::CrushingWheels => {
            // `isItemValid`: slot 0, when empty.
            let held = host.mod_block_entity(pos).map(|e| e.items.first().cloned().flatten());
            if held != Some(None) {
                return stack;
            }
            let limit = max_stack(&*host, &stack);
            put(host, pos, stack.clone(), simulate, limit.min(stack.count))
        }
        PointType::Depot => {
            if host.block(pos) == DEPOT {
                depot::handler_insert(host, pos, &stack, simulate)
            } else {
                hand_over(host, pos, stack, simulate, true)
            }
        }
        PointType::Belt => belt::handler_insert(host, pos, &stack, simulate).unwrap_or(stack),
        // The grid's combined inventory: one item to each crafter in turn.
        PointType::Crafter => crafter_world::handler_insert(host, pos, &stack, simulate),
        PointType::Chute => hand_over(host, pos, stack, simulate, false),
        PointType::Saw => saw::insert_from_side(host, pos, &stack, simulate),
        PointType::BlazeBurner => blaze_burner(host, pos, stack, simulate),
        PointType::Funnel => funnel(host, pos, stack, simulate),
        PointType::Campfire => campfire(host, pos, stack, simulate),
        PointType::Composter => {
            let level = digit(host, pos, "level").unwrap_or(0);
            if !points::composter_takes(level, items::compostable(&stack.item)) {
                return stack;
            }
            put(host, pos, stack.clone(), simulate, 1)
        }
        PointType::Jukebox => {
            let has_record = host.block_property(pos, "has_record").as_deref() == Some("true");
            if !points::jukebox_takes(items::jukebox_playable(&stack.item), has_record) {
                return stack;
            }
            let one = stack_of_count(&stack, 1);
            if simulate {
                return minus(stack, 1);
            }
            match host.insert_item(pos, Direction::Up, one) {
                Some(rest) if rest.count > 0 => stack,
                _ => minus(stack, 1),
            }
        }
        PointType::RespawnAnchor => {
            let charge = digit(host, pos, "charges");
            let Some(next) = points::respawn_anchor_charge(&stack.item, charge) else {
                return stack;
            };
            if !simulate {
                host.set_block_property(pos, "charges", &next.to_string());
                host.play_sound("minecraft:block.respawn_anchor.charge", util::centre(pos), BLOCKS, 1.0, 1.0);
            }
            minus(stack, 1)
        }
    }
}

/// An item handed to a block that takes belt input from above (the Weighted Ejector, a chute):
/// `DepotItemHandler` and `ChuteItemHandler` through the belt's table of inputs. `needs_room` asks
/// `canInsertFromSide` first (the ejector takes nothing while it isn't charged).
fn hand_over(host: &mut dyn Host, pos: Pos, stack: ItemStack, simulate: bool, needs_room: bool) -> ItemStack {
    if needs_room && belt::can_insert_from_side(&*host, pos, Dir::Up) != Some(true) {
        return stack;
    }
    let angle = (util::random() % 360) as i32;
    let item = create_kinetics::belt::Item::dropped(stack.clone(), 0, angle, true);
    belt::handle_insertion(host, pos, Dir::Up, &item, false, simulate).unwrap_or(stack)
}

/// `BlazeBurnerPoint.insert`: what `BlazeBurnerBlock.tryInsert` left of the stack, and a container
/// (an empty bucket) it handed back, which comes back as the stack once the fuel is used up and
/// is dropped at the burner otherwise.
fn blaze_burner(host: &mut dyn Host, pos: Pos, stack: ItemStack, simulate: bool) -> ItemStack {
    let (taken, creative, container) = burner::feed_from_arm(host, pos, &stack, simulate);
    match points::blaze_burner_fed(stack.count, taken, creative, container.is_some()) {
        Fed::Unchanged => stack,
        Fed::Took { left: 0, .. } => {
            container.map_or_else(|| stack_of_count(&stack, 0), |id| ItemStack::new(id, 1))
        }
        Fed::Took { left, .. } => {
            if !simulate && let Some(id) = container {
                util::spawn_item(host, util::centre(pos), &ItemStack::new(id, 1), [0.0; 3], 10);
            }
            stack_of_count(&stack, left)
        }
    }
}

/// `FunnelPoint.insert`: the stack goes into the inventory behind the funnel unless the funnel is
/// powered or its filter (a Brass Funnel's) refuses it.
fn funnel(host: &mut dyn Host, pos: Pos, stack: ItemStack, simulate: bool) -> ItemStack {
    let Some(entity) = host.mod_block_entity(pos) else {
        return stack;
    };
    let Some(facing) = util::facing(&*host, pos, "facing") else {
        return stack;
    };
    let powered = host.block_property(pos, "powered").as_deref() == Some("true");
    let brass = host.block(pos).contains("brass");
    let passes = !brass || inv::filter_of(&entity.nbt).test(host, &stack);
    if !points::funnel_takes(powered, passes) {
        return stack;
    }
    let behind = util::add(pos, facing.opposite().normal());
    if !simulate {
        return inv::insert(host, behind, facing, &stack, false).unwrap_or(stack);
    }
    // Belts, depots and chutes answer a simulation; a container is worked out from its slots.
    if let Some(rest) = inv::insert(host, behind, facing, &stack, true)
        && rest.count < stack.count
    {
        return rest;
    }
    let room = container_room(host, behind, &stack);
    minus(stack, room)
}

/// How much of `stack` the container at `pos` has room for, from its saved items and its size.
fn container_room(host: &mut dyn Host, pos: Pos, stack: &ItemStack) -> u32 {
    if !inv::is_inventory(host, pos) {
        return 0;
    }
    let name = host.block(pos);
    let size = match host.mod_block_entity(pos) {
        Some(entity) => entity.items.len(),
        None => points::vanilla_container_size(&name).map_or(0, |n| n * inv::parts(&*host, pos).len()),
    };
    let mut slots: Vec<Option<(ItemStack, u32)>> = vec![None; size];
    for (slot, have) in inv::items_of(host, pos).unwrap_or_default() {
        if slot < slots.len() {
            slots[slot] = Some((key_of(&have), have.count));
        }
    }
    let limit = max_stack(&*host, stack);
    let left = points::insert_into_slots(&slots, &key_of(stack), stack.count, &|_| limit, &|_| true);
    stack.count - left
}

/// `CampfirePoint.insert`: a food a campfire recipe cooks goes onto a free spot, one at a time.
fn campfire(host: &mut dyn Host, pos: Pos, stack: ItemStack, simulate: bool) -> ItemStack {
    let dimension = host.dimension();
    let busy = match host.block_entity_tag(dimension, pos) {
        Some(Tag::Compound(tag)) => match util::get(&tag, "Items") {
            Some(Tag::List(list)) => list.len(),
            _ => 0,
        },
        _ => return stack,
    };
    if points::campfire_takes(items::cooking_time(&stack.item), 4usize.saturating_sub(busy)).is_none() {
        return stack;
    }
    if simulate {
        return minus(stack, 1);
    }
    // `placeFood`: the campfire's own use puts one item of the hand on it.
    let [x, y, z] = pos;
    let target = NativeUseTarget::Block {
        pos,
        face: Direction::Up,
        point: [f64::from(x) + 0.5, f64::from(y) + 0.5, f64::from(z) + 0.5],
    };
    let eye = [f64::from(x) + 0.5, f64::from(y) + 2.0, f64::from(z) + 0.5];
    match host.use_as_player(stack.clone(), eye, 0.0, 90.0, target, false) {
        Some(used) if used.consumed => used.held.unwrap_or_else(|| stack_of_count(&stack, 0)),
        _ => stack,
    }
}

/// The arm's `Arm` for tests and reads that want one.
#[allow(dead_code)]
pub(super) fn new_arm() -> Arm<ItemStack> {
    Arm::new()
}
