//! The Item Drain (`create:item_drain`; `ItemDrainBlock`, `ItemDrainBlockEntity`,
//! `ItemDrainItemHandler`, `ItemDrainRenderer`, `GenericItemEmptying`): an item put in it by a
//! belt, a funnel, a chute or dropped on top slides to the middle, is emptied there into the
//! drain's tank, and slides on out of the far side; a bucket or bottle right-clicked on it is
//! emptied at once. Pipes draw from the tank (never into it). The timing is
//! `create_kinetics::rules::item_drain`; what an item gives is `spout_fill`.
//!
//! Known differences from Create: see `rules/item_drain.rs` and `spout_fill.rs`. Also:
//! - Hoppers and other inventories can't insert into or take from a drain
//!   (`ItemDrainItemHandler` is not a container here); belts, funnels, chutes and items falling
//!   on it can.
//! - A belt funnel beside the drain doesn't take its output
//!   (`DirectBeltInputBehaviour.tryExportingToBeltFunnel`).
//! - The fluid in the tank rises at once where Create's `LerpedFloat` chases it, and there are no
//!   comparator signal (the hook gets no block entity), goggle tooltip or advancements.
//! - The block drops what it holds when a player breaks it; an explosion or a piston doesn't.


use create_kinetics::belt::Item;
use create_kinetics::rules::fluids as fluid_rules;
use create_kinetics::rules::item_drain::{self as rules, Processing, Slide};
use create_kinetics::{Axis, Dir};
use horizon_client::mod_api::{
    BlockEntityEvent, BlockEntityFluidsEvent, BlockEntitySubmitEvent, BlockEntityTickEvent,
    BlockEntityTicksEvent, BlockEntityUpdateTagEvent, Compound, DestroyBlockEvent, DisplayContext,
    EquipmentSlot, Flow, Hooks, Host, ItemStack, ModBlockEntity, ModFluidTanks, RightClickBlockEvent,
    Tag, UseItemEvent,
};

use super::fluids;
use super::spout_fill as fill;
use crate::belt::{self, BeltInput};
use crate::{float, set_float, util};

type Pos = [i32; 3];

pub const ID: &str = "create:item_drain";
/// `ItemDrainBlockEntity.write`.
const PROCESSING: &str = "ProcessingTicks";
const HELD_ITEM: &str = "HeldItem";
/// What the stream above the held item is made of: the fluid it gives (clients have no recipes).
const HELD_FLUID: &str = "HeldFluid";
/// `TransportedItemStack.serializeNBT`.
const ITEM: &str = "Item";
const POS: &str = "Pos";
const PREV_POS: &str = "PrevPos";
const OFFSET: &str = "Offset";
const PREV_OFFSET: &str = "PrevOffset";
const ANGLE: &str = "Angle";
const IN_DIRECTION: &str = "InDirection";
const UP: [f32; 3] = [0.0, 1.0, 0.0];
const ORIGIN: [f32; 3] = [0.0; 3];

struct Drains;

pub fn hooks() -> Box<dyn Hooks> {
    belt::register_input(BeltInput {
        claims: |block| block == ID,
        // `DirectBeltInputBehaviour.canInsertFromSide`: any side.
        can_insert: |_, _, _| true,
        insert: |host, pos, side, item, simulate| try_inserting(host, pos, side, item, simulate),
    });
    Box::new(Drains)
}

// --- what the drain holds -------------------------------------------------------------------------

/// `TransportedItemStack` of the held item.
#[derive(Clone, Debug, PartialEq)]
struct Held {
    stack: ItemStack,
    /// `beltPosition`: 0 at the edge it came in by, 1 at the far one.
    pos: f32,
    prev_pos: f32,
    /// `sideOffset`.
    offset: f32,
    prev_offset: f32,
    angle: i32,
    /// `insertedFrom`: the way it moves.
    from: Dir,
}

impl Held {
    fn read(nbt: &Compound) -> Option<Self> {
        let Some(Tag::Compound(held)) = util::get(nbt, HELD_ITEM) else {
            return None;
        };
        let stack = util::stack_of(util::get(held, ITEM)?)?;
        let int = |key: &str| match util::get(held, key) {
            Some(Tag::Int(n)) => *n,
            _ => 0,
        };
        Some(Held {
            stack,
            pos: float(held, POS).unwrap_or(0.0),
            prev_pos: float(held, PREV_POS).unwrap_or(0.0),
            offset: float(held, OFFSET).unwrap_or(0.0),
            prev_offset: float(held, PREV_OFFSET).unwrap_or(0.0),
            angle: int(ANGLE),
            from: Dir::ALL[int(IN_DIRECTION).rem_euclid(6) as usize],
        })
    }

    fn write(&self) -> Tag {
        let mut tag = Compound::new();
        tag.insert(ITEM.into(), util::stack_tag(&self.stack));
        set_float(&mut tag, POS, self.pos);
        set_float(&mut tag, PREV_POS, self.prev_pos);
        set_float(&mut tag, OFFSET, self.offset);
        set_float(&mut tag, PREV_OFFSET, self.prev_offset);
        tag.insert(ANGLE.into(), Tag::Int(self.angle));
        let from = Dir::ALL.iter().position(|d| *d == self.from).unwrap_or(0);
        tag.insert(IN_DIRECTION.into(), Tag::Int(from as i32));
        Tag::Compound(tag)
    }

    /// `new TransportedItemStack(stack)` put in from `from` (`tryInsertingFromSide`).
    fn entering(stack: ItemStack, from: Dir, offset: f32, angle: i32) -> Self {
        let pos = if from.axis() == Axis::Y { 0.5 } else { 0.0 };
        Held { stack, pos, prev_pos: pos, offset, prev_offset: offset, angle, from }
    }

    /// The belt's item type, to hand it on.
    fn as_item(&self) -> Item<ItemStack> {
        Item {
            stack: self.stack.clone(),
            pos: self.pos,
            side: self.offset,
            angle: self.angle,
            inserted_at: 0,
            inserted_from: self.from,
            locked: false,
            at_tunnel: false,
        }
    }
}

fn processing_of(nbt: &Compound) -> i32 {
    match util::get(nbt, PROCESSING) {
        Some(Tag::Int(n)) => *n,
        _ => 0,
    }
}

/// Writes (or clears) the held item and the fluid its stream shows, only where they differ.
fn write_held(host: &dyn Host, nbt: &mut Compound, held: Option<&Held>) {
    match held {
        Some(held) => {
            let tag = held.write();
            if util::differs(nbt, HELD_ITEM, &tag) {
                nbt.insert(HELD_ITEM.into(), tag);
            }
            // `ItemDrainRenderer`: the stream is what the item gives, else the tank's fluid.
            let fluid = fill::empty_item(host, &held.stack).map(|e| e.fluid);
            match fluid {
                Some(fluid) => {
                    let tag = Tag::String(fluid.as_str().into());
                    if util::differs(nbt, HELD_FLUID, &tag) {
                        nbt.insert(HELD_FLUID.into(), tag);
                    }
                }
                None => {
                    util::remove(nbt, HELD_FLUID);
                }
            }
        }
        None => {
            util::remove(nbt, HELD_ITEM);
            util::remove(nbt, HELD_FLUID);
        }
    }
}

fn new_entity() -> Compound {
    let mut nbt = Compound::new();
    nbt.insert(fill::FLUIDS.into(), fill::tank_tag(None, 0));
    nbt.insert(PROCESSING.into(), Tag::Int(0));
    nbt
}

/// `ItemDrainBlockEntity.registerCapabilities`: the tank to every side but up, drained from only
/// (`forbidInsertion`; the drain fills it itself).
fn tanks() -> ModFluidTanks {
    let mut faces: [Vec<usize>; 6] = [(); 6].map(|()| vec![0]);
    // `Direction.UP` is the second face.
    faces[1] = Vec::new();
    ModFluidTanks { tanks: vec![rules::CAPACITY], faces, inputs: Vec::new(), outputs: vec![0] }
}

// --- taking items ---------------------------------------------------------------------------------

/// `ItemDrainBlockEntity.tryInsertingFromSide` (the belt's `handleInsertion`): what is left of
/// `item`, moving towards `side`. A drain already holding something takes nothing.
fn try_inserting(
    host: &mut dyn Host,
    pos: Pos,
    side: Dir,
    item: &Item<ItemStack>,
    simulate: bool,
) -> ItemStack {
    let whole = item.stack.clone();
    let Some(entity) = host.mod_block_entity(pos).filter(|e| e.id == ID) else {
        return whole;
    };
    if Held::read(&entity.nbt).is_some() {
        return whole;
    }
    let (taken, returned) = rules::accept(whole.count, fill::can_be_emptied(&*host, &whole));
    let mut rest = whole.clone();
    rest.count = returned;
    if simulate {
        return rest;
    }
    let mut inserted = whole;
    inserted.count = taken;
    let held = Held::entering(inserted, side, item.side, item.angle);
    put_held(host, pos, &held);
    rest
}

fn put_held(host: &mut dyn Host, pos: Pos, held: &Held) {
    let mut nbt = Compound::new();
    write_held(&*host, &mut nbt, Some(held));
    host.change_mod_block_entity(pos, &mut |entity| {
        for key in [HELD_ITEM, HELD_FLUID] {
            match util::get(&nbt, key) {
                Some(value) => {
                    entity.nbt.insert(key.into(), value.clone());
                }
                None => {
                    util::remove(&mut entity.nbt, key);
                }
            }
        }
    });
    host.block_entity_sync(pos);
}

// --- the tick -------------------------------------------------------------------------------------

/// Whether the tank has room for all `amount` of `fluid` (`fill(SIMULATE) == amount`).
fn fits(nbt: &Compound, fluid: &str, amount: i32) -> bool {
    let (held, now) = fill::tank_of(nbt);
    held.as_deref().is_none_or(|f| f == fluid) && now + amount <= rules::CAPACITY
}

/// `ItemDrainBlockEntity.tick` (server).
fn tick(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity) {
    let Some(mut held) = Held::read(&entity.nbt) else {
        // `heldItem == null`: `processingTicks = 0`.
        if processing_of(&entity.nbt) != 0 {
            entity.nbt.insert(PROCESSING.into(), Tag::Int(0));
            host.block_entity_sync(pos);
        }
        landed(host, pos);
        return;
    };
    let before = held.clone();
    let processing = processing_of(&entity.nbt);
    let mut next = processing;
    let mut sync = false;
    let mut gone = false;

    if processing > 0 {
        held.prev_pos = 0.5;
        next -= 1;
        match continue_processing(&*host, entity, &mut held, next) {
            Step::Go => {}
            Step::Stop => {
                next = 0;
                sync = true;
            }
            Step::Restart => {
                next = rules::FILLING_TIME;
                sync = true;
            }
            Step::Emptied(left) => {
                match left {
                    Some(stack) => held.stack = stack,
                    None => gone = true,
                }
                sync = true;
            }
        }
    } else {
        held.prev_pos = held.pos;
        held.prev_offset = held.offset;
        let can_at_middle = held.pos < 0.5 && fill::can_be_emptied(&*host, &held.stack);
        match rules::slide(held.pos, can_at_middle) {
            Slide::Moving { pos: moved } => held.pos = moved,
            Slide::Start => {
                held.pos = 0.5;
                next = rules::FILLING_TIME;
                sync = true;
            }
            Slide::AtEnd => {
                held.pos = 1.0;
                match leave(host, pos, &held) {
                    Leave::Wait => {}
                    Leave::Gone => {
                        gone = true;
                        sync = true;
                    }
                    Leave::Left(rest) => {
                        held.stack = rest;
                        sync = true;
                    }
                }
            }
        }
    }

    if next != processing {
        entity.nbt.insert(PROCESSING.into(), Tag::Int(next));
    }
    if gone {
        write_held(&*host, &mut entity.nbt, None);
    } else if held != before {
        write_held(&*host, &mut entity.nbt, Some(&held));
    }
    // Clients don't tick the drain: each step of the slide and of the wait is sent.
    if sync || held != before || next != processing {
        host.block_entity_sync(pos);
    }
}

/// What `continueProcessing` did.
enum Step {
    /// Keep counting.
    Go,
    /// The item can't be emptied any more.
    Stop,
    /// The tank has no room: wait the whole time again.
    Restart,
    /// The item was emptied; what is left of it (`None`: nothing).
    Emptied(Option<ItemStack>),
}

fn continue_processing(host: &dyn Host, entity: &mut ModBlockEntity, held: &mut Held, ticks: i32) -> Step {
    if ticks < 5 {
        return Step::Go;
    }
    let emptied = fill::empty_item(host, &held.stack);
    let can = emptied.is_some() && fill::can_be_emptied(host, &held.stack);
    let fits = emptied.as_ref().is_some_and(|e| fits(&entity.nbt, &e.fluid, e.amount));
    match rules::continue_processing(ticks, can, fits) {
        Processing::Wait => Step::Go,
        Processing::Stop => Step::Stop,
        Processing::Restart => Step::Restart,
        Processing::Empty => {
            let Some(emptied) = emptied else {
                return Step::Stop;
            };
            let (fluid, now) = fill::tank_of(&entity.nbt);
            let into = fluid.unwrap_or_else(|| emptied.fluid.clone());
            fill::set_tank(&mut entity.nbt, Some(&into), now + emptied.amount);
            Step::Emptied(emptied.item)
        }
    }
}

enum Leave {
    /// Nothing moved.
    Wait,
    /// All of it went (or was thrown).
    Gone,
    /// Some of it went; this is what is left.
    Left(ItemStack),
}

/// The item at the far end tries to leave (`ItemDrainBlockEntity.tick` past `beltPosition > 1`).
fn leave(host: &mut dyn Host, pos: Pos, held: &Held) -> Leave {
    let side = held.from;
    let next = util::add(pos, side.normal());
    let Some(can) = belt::can_insert_from_side(&*host, next, side) else {
        // No `DirectBeltInputBehaviour` there: thrown out unless something solid is in the way.
        if belt::face_full(&*host, next, side.opposite()) {
            return Leave::Wait;
        }
        let n = side.normal().map(f64::from);
        let centre = util::centre(pos);
        let speed = f64::from(rules::MOVEMENT);
        let at = [centre[0] + n[0] * 0.75, centre[1] + 6.0 / 16.0, centre[2] + n[2] * 0.75];
        let motion = [n[0] * speed, 1.0 / 8.0, n[2] * speed];
        util::spawn_item(host, at, &held.stack, motion, 10);
        return Leave::Gone;
    };
    if !can {
        return Leave::Wait;
    }
    let before = held.stack.count;
    match belt::handle_insertion(host, next, side, &held.as_item(), false, false) {
        Some(rest) if rest.count == 0 => Leave::Gone,
        Some(rest) if rest.count != before => Leave::Left(rest),
        _ => Leave::Wait,
    }
}

/// `ItemDrainBlock.updateEntityAfterFallOn`: an item lying on top goes in, moving the way it was
/// travelling (north when it fell straight). Polled, as the depot does.
fn landed(host: &mut dyn Host, pos: Pos) {
    if util::game_time(&*host).rem_euclid(2) != 0 {
        return;
    }
    let [x, y, z] = pos.map(f64::from);
    let dimension = host.dimension().to_owned();
    for found in host.item_entities(&dimension, [x, y, z], [x + 1.0, y + 1.0, z + 1.0]) {
        let [px, py, pz] = found.position;
        if !(x..x + 1.0).contains(&px) || !(z..z + 1.0).contains(&pz) || !(y..y + 1.0).contains(&py) {
            continue;
        }
        let Some(entity) = host.entity(found.id) else {
            continue;
        };
        if !host.entity_on_ground(entity).unwrap_or(false) {
            continue;
        }
        // `Direction.getNearest` of the horizontal motion: its largest component, north for none.
        let velocity = host.entity_velocity(entity).unwrap_or([0.0; 3]);
        let side = if velocity[0].abs() > velocity[2].abs() && velocity[0].abs() > 1e-6 {
            if velocity[0] > 0.0 { Dir::East } else { Dir::West }
        } else if velocity[2].abs() > 1e-6 {
            if velocity[2] > 0.0 { Dir::South } else { Dir::North }
        } else {
            Dir::North
        };
        let angle = (util::random() % 360) as i32;
        let item = Item::dropped(found.stack.clone(), 0, angle, true);
        let rest = try_inserting(host, pos, side, &item, false);
        if rest.count != found.stack.count {
            host.set_item_entity(&dimension, found.id, rest);
        }
        return;
    }
}

// --- clicks ---------------------------------------------------------------------------------------

// As the tank's: the off hand's turn of a click a drain took, and the item use after it, are
// dropped while the click is fresh (`util::click_repeated`, `util::click_repeated_anywhere`).

/// `FluidHelper.tryEmptyItemIntoBE` with the tank open to filling: a plain container in hand
/// empties into the drain's tank, if all of it fits. True when it did.
fn empty_into(host: &mut dyn Host, pos: Pos, stack: &ItemStack, creative: bool) -> bool {
    if stack.extra.is_some() {
        return false;
    }
    let Some(emptied) = fill::empty_item(&*host, stack) else {
        return false;
    };
    // Only what `take_items` and `give_items` handle: plain stacks.
    if emptied.item.as_ref().is_some_and(|i| i.extra.is_some()) {
        return false;
    }
    let Some(entity) = host.mod_block_entity(pos).filter(|e| e.id == ID) else {
        return false;
    };
    if !fits(&entity.nbt, &emptied.fluid, emptied.amount) {
        return false;
    }
    if !creative {
        if host.take_items(&stack.item, 1) != 1 {
            return false;
        }
        if let Some(left) = &emptied.item {
            host.give_items(&left.item, 1);
        }
    }
    host.change_mod_block_entity(pos, &mut |entity| {
        let (fluid, now) = fill::tank_of(&entity.nbt);
        let into = fluid.unwrap_or_else(|| emptied.fluid.clone());
        fill::set_tank(&mut entity.nbt, Some(&into), now + emptied.amount);
    });
    host.play_sound(fluid_rules::empty_sound(&emptied.fluid), util::centre(pos), 4, 0.5, 1.0);
    host.block_entity_sync(pos);
    true
}

impl Hooks for Drains {
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

    fn block_entity_fluids(
        &self,
        _: &dyn Host,
        event: &mut BlockEntityFluidsEvent<'_>,
    ) -> Option<ModFluidTanks> {
        (event.block == ID).then(tanks)
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id == ID {
            tick(host, event.pos, event.entity);
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        if event.entity.id != ID {
            return None;
        }
        let nbt = &event.entity.nbt;
        let mut tag = Compound::new();
        if let Some(fluids) = util::get(nbt, fill::FLUIDS) {
            tag.insert(fill::FLUIDS.into(), fluids.clone());
        }
        tag.insert(PROCESSING.into(), Tag::Int(processing_of(nbt)));
        // Always sent: an empty compound tells a client the item is gone.
        let held = util::get(nbt, HELD_ITEM).cloned().unwrap_or_else(|| Tag::Compound(Compound::new()));
        tag.insert(HELD_ITEM.into(), held);
        tag.insert(
            HELD_FLUID.into(),
            util::get(nbt, HELD_FLUID).cloned().unwrap_or_else(|| Tag::String("".into())),
        );
        Some(tag)
    }

    /// `ItemDrainRenderer`: the fluid in the drain, the stream over the item being emptied, and
    /// the item.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let nbt = &event.entity.nbt;
        let (fluid, amount) = fill::tank_of(nbt);
        let held = Held::read(nbt);
        let processing = processing_of(nbt);
        let partial = event.draw.partial;
        let draw = &mut *event.draw;
        if let Some(fluid) = fluid.as_deref() {
            // `yMin = 5/16`, `min = 2/16`, a box `12/16` wide that rises `7/16` with the level.
            let level = (amount as f32 / rules::CAPACITY as f32).clamp(0.0, 1.0);
            let (y_min, min) = (5.0 / 16.0, 2.0 / 16.0);
            let max = min + 12.0 / 16.0;
            fluids::draw_fluid(host, draw, fluid, amount, [min, y_min, min, max, y_min + 7.0 / 16.0 * level, max], false);
        }
        if let Some(held) = &held {
            let hint = match util::get(nbt, HELD_FLUID) {
                Some(Tag::String(id)) if !id.to_string_lossy().is_empty() => Some(id.to_string_lossy()),
                _ => None,
            };
            let shown = hint.or(fluid.clone());
            if let (Some(shown), Some(radius)) = (shown.as_deref(), rules::stream(processing, partial)) {
                // `new AABB(0.5, 1.0, 0.5, 0.5, 0.25, 0.5).inflate(radius / 32f)`, corners sorted.
                let r = radius / 32.0;
                let (x0, x1) = ((0.5 + r).min(0.5 - r), (0.5 + r).max(0.5 - r));
                let (y0, y1) = ((0.25 - r).min(1.0 + r), (0.25 - r).max(1.0 + r));
                if x1 > x0 {
                    fluids::draw_fluid(host, draw, shown, 1000, [x0, y0, x0, x1, y1, x1], true);
                }
            }
            draw_held(host, draw, held, partial);
        }
        // Mid-wait and mid-slide the picture reads the partial tick.
        if held.is_none() {
            draw.keep();
        }
    }

    /// `ItemDrainBlock.onRemove`: what the drain holds drops.
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        if let Some(held) = host.mod_block_entity(pos).and_then(|e| Held::read(&e.nbt)) {
            let at = util::centre(pos);
            util::spawn_item(host, at, &held.stack, [0.0; 3], 10);
            host.change_mod_block_entity(pos, &mut |entity| {
                util::remove(&mut entity.nbt, HELD_ITEM);
            });
        }
        Flow::Continue
    }

    /// `ItemDrainBlock.useItemOn`: a container in hand is emptied into the drain at once; with
    /// anything else (or nothing) the item the drain holds comes back to the player.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        let uuid = player.uuid;
        let stack = host.player_equipment(uuid, EquipmentSlot::MainHand);
        // Sneaking with something in hand skips the block's own use.
        if player.sneaking && stack.is_some() {
            return Flow::Continue;
        }
        if util::click_repeated(&*host, uuid, pos) {
            return Flow::Cancel;
        }
        if let Some(stack) = &stack {
            let item: &str = &stack.item;
            let container = fluid_rules::is_container(item) || fill::can_be_emptied(&*host, stack);
            // `BlockItem` without a fluid handler: the click places it.
            if !container && host.default_state(item).is_some() {
                return Flow::Continue;
            }
            if empty_into(host, pos, stack, player.game_mode == util::CREATIVE)
                || fill::can_be_emptied(&*host, stack)
            {
                util::click_taken(&*host, uuid, pos);
                return Flow::Cancel;
            }
        }
        // The item the drain holds comes back.
        if let Some(held) = host.mod_block_entity(pos).and_then(|e| Held::read(&e.nbt)) {
            host.give_stack(uuid, held.stack);
            host.change_mod_block_entity(pos, &mut |entity| {
                util::remove(&mut entity.nbt, HELD_ITEM);
                util::remove(&mut entity.nbt, HELD_FLUID);
            });
            host.block_entity_sync(pos);
        }
        util::click_taken(&*host, uuid, pos);
        Flow::Cancel
    }

    /// The item use the client sends after a click a drain took (the bucket's own use).
    fn use_item(&self, host: &mut dyn Host, event: &mut UseItemEvent<'_>) -> Flow {
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if !util::click_repeated_anywhere(&*host, player.uuid) {
            return Flow::Continue;
        }
        let item: &str = &event.stack.item;
        if fluid_rules::is_container(item) || fluid_rules::emptied(item).is_some() {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

// --- drawing --------------------------------------------------------------------------------------

/// `ItemDrainRenderer.renderItem` for a horizontal slide: the held item at its place along the
/// slide, flat in a fan (a block in a small heap), one copy more for each fourfold of the count.
fn draw_held(host: &dyn Host, draw: &mut horizon_client::mod_api::BlockEntityDraw, held: &Held, partial: f32) {
    if held.from.axis() == Axis::Y {
        return;
    }
    let lerp = |from: f32, to: f32| from + (to - from) * partial;
    let along = lerp(held.prev_pos, held.pos);
    let mut side = lerp(held.prev_offset, held.offset);
    draw.push();
    draw.translate([0.5, 15.0 / 16.0, 0.5]);
    let back = held.from.opposite().normal().map(|v| v as f32 * (0.5 - along));
    draw.translate(back);
    // `insertedFrom.getClockWise().getAxis() == X`.
    let clockwise_x = matches!(held.from, Dir::North | Dir::South);
    if !clockwise_x {
        side = -side;
    }
    draw.translate(if clockwise_x { [side, 0.0, 0.0] } else { [0.0, 0.0, side] });
    let copies = held.stack.count.max(1).ilog2() / 2;
    let block = host.default_state(&held.stack.item).is_some();
    // The slide turns the item over as it goes (`verticalAngle = positive * offset * 360`).
    let vertical = held.from.sign() as f32 * along * 360.0;
    if held.from.axis() != Axis::X {
        draw.rotate_about([1.0, 0.0, 0.0], vertical, ORIGIN);
    }
    if held.from.axis() != Axis::Z {
        draw.rotate_about([0.0, 0.0, 1.0], -vertical, ORIGIN);
    }
    draw.rotate_about(UP, held.angle as f32, ORIGIN);
    for i in 0..=copies {
        draw.push();
        if block {
            // `r.nextFloat() * .0625f * i`, from `new Random(0)`: a fixed small scatter.
            let scatter = 0.0625 * i as f32 * 0.7;
            draw.translate([scatter, 0.0, scatter]);
        }
        draw.scale([0.5; 3]);
        if !block {
            draw.translate([0.0, -3.0 / 16.0, 0.0]);
            draw.rotate_about([1.0, 0.0, 0.0], 90.0, ORIGIN);
        }
        host.draw_item(draw, &held.stack, DisplayContext::Fixed, None, 0);
        draw.pop();
        if block {
            draw.translate([0.0, 1.0 / 64.0, 0.0]);
        } else {
            draw.rotate_about(UP, 10.0, ORIGIN);
            draw.translate([0.0, 1.0 / 16.0, 0.0]);
        }
    }
    draw.pop();
}
