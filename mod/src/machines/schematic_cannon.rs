//! The Schematicannon (`SchematicannonBlock`, `SchematicannonBlockEntity`, `SchematicannonRenderer`,
//! `SchematicannonInventory`, `SchematicannonMenu`, `ConfigureSchematicannonPacket`): prints a
//! deployed Schematic block by block, taking the items from the containers beside it and gunpowder
//! as fuel, and draws the cannon turning to its target and the block flying. The state machine, the
//! order, the replace modes and the aim are `create_kinetics::rules::schematic_cannon` and
//! `schematic_printer`; this is the glue.
//!
//! Known differences from Create, for the README (the stand-ins for its screen):
//! - **Slots.** Right-clicking the cannon opens a hopper-shaped menu: slot 1 the Schematic, slot 2
//!   where the Empty Schematic comes out, slot 3 and 4 the Material List's book and its output
//!   (unused: no list is printed), slot 5 the gunpowder. The menu does not check what goes in a slot:
//!   a slot holding the wrong item is ignored (a Schematic slot that holds something else reads as
//!   an invalid schematic). Hoppers and funnels can't reach the slots.
//! - **Status and settings.** `/schematicannon status` shows what the screen's status and fuel show
//!   (for the cannon you right-clicked last), `checklist` what the Schematic needs against what the
//!   containers beside hold, `play`, `pause` and `stop` are its buttons (the Wrench on the cannon
//!   toggles play and pause, sneaking with it picks the cannon up), `replace <dont|solid|any|empty>`,
//!   `skip <on|off>` and `protect <on|off>` its options.
//! - No Material List printing, no entities, no belts (`schematic_cannon.rs` and `schematic_printer.rs`
//!   in the rules). Settings are not copied onto the dropped item.
//! - The cannon's inventories are read as the chutes read them (`chute_inv.rs`): chests, barrels,
//!   hoppers, shulker boxes and Create's containers with items in them; a container that is empty
//!   for the moment is found only once it holds something, and the beside-look is every 100 ticks.
//! - Blocks land by `Host::run_command("setblock")` from the cannon's tick, with the block's data
//!   handed over by `set_block_tag`; Create's `LaunchedItem.ForBlockState.place` also plays no sound.

use std::collections::HashMap;
use std::sync::Mutex;

use create_kinetics::rules::chute::Mode;
use create_kinetics::rules::contraption::State;
use create_kinetics::rules::schematic_cannon::{
    self as rules, Blueprint, Cannon, Effect, Flying, Io, LoadError, RunState, SKIPS_PER_TICK, cannon_angles,
    flight_pose, recoil,
};
use create_kinetics::rules::schematic_printer::{Existing, Layout, Printer};
use create_kinetics::rules::schematic_structure::Pos;
use create_kinetics::Dir;
use crate::compat::{
    BlockEntityContainerEvent, BlockEntityDraw, BlockEntityEvent, BlockEntityRenderBoundingBoxEvent,
    BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent, BlockEntityUpdateTagEvent, Compound, Flow,
    Hooks, Host, ItemStack, ModBlockEntity, ModContainer, NativeCommand, PerformCommandEvent, PlaceBlockEvent,
    RegisterCommandsEvent, RightClickBlockEvent, StandaloneModelsEvent, Tag, Text, UseItemOnEvent,
};

use super::chute_inv as inv;
use super::schematic::{self as items, EMPTY};
use super::schematic_files as files;
use super::schematic_world as world;
use crate::util;

pub const ID: &str = "create:schematicannon";
const CONNECTOR: &str = "create:block/schematicannon/connector";
const PIPE: &str = "create:block/schematicannon/pipe";
const CREATIVE_CRATE: &str = "create:creative_crate";
const GUNPOWDER: &str = "minecraft:gunpowder";

/// `SchematicannonInventory`: the schematic, the empty schematic out, the book in and the checklist
/// out (unused here), the gunpowder.
const SLOTS: usize = 5;
const BLUEPRINT: usize = 0;
const OUTPUT: usize = 1;
const FUEL: usize = 4;

struct Cannons;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Cannons)
}

fn lock<T>(m: &'static Mutex<T>) -> std::sync::MutexGuard<'static, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

// ---- the saved form ----------------------------------------------------------------------------------

fn int(nbt: &Compound, key: &str) -> i32 {
    match util::get(nbt, key) {
        Some(Tag::Int(v)) => *v,
        _ => 0,
    }
}

fn text(nbt: &Compound, key: &str) -> String {
    match util::get(nbt, key) {
        Some(Tag::String(s)) => s.to_string_lossy(),
        _ => String::new(),
    }
}

fn pos_of(nbt: &Compound, key: &str) -> Option<Pos> {
    match util::get(nbt, key) {
        Some(Tag::IntArray(v)) if v.len() == 3 => Some([v[0], v[1], v[2]]),
        _ => None,
    }
}

fn pos_tag(pos: Option<Pos>) -> Tag {
    Tag::IntArray(pos.map_or_else(Vec::new, |p| p.to_vec()))
}

fn flying_tag(f: &Flying) -> Tag {
    let mut tag = Compound::new();
    tag.insert("Launched".into(), Tag::Long(f.launched));
    tag.insert("TotalTicks".into(), Tag::Int(f.total_ticks));
    tag.insert("Target".into(), Tag::IntArray(f.target.to_vec()));
    tag.insert("Icon".into(), Tag::String(f.icon.as_str().into()));
    tag.insert("BlockState".into(), Tag::String(f.state.text().as_str().into()));
    if let Some(data) = &f.data {
        tag.insert("Data".into(), files::tag_of(data));
    }
    Tag::Compound(tag)
}

fn flying_of(tag: &Tag) -> Option<Flying> {
    let Some(Tag::IntArray(target)) = tag.get("Target") else {
        return None;
    };
    let Some(Tag::String(state)) = tag.get("BlockState") else {
        return None;
    };
    Some(Flying {
        launched: match tag.get("Launched") {
            Some(Tag::Long(v)) => *v,
            _ => 0,
        },
        total_ticks: match tag.get("TotalTicks") {
            Some(Tag::Int(v)) => *v,
            _ => 10,
        },
        target: [*target.first()?, *target.get(1)?, *target.get(2)?],
        icon: match tag.get("Icon") {
            Some(Tag::String(s)) => s.to_string_lossy(),
            _ => String::new(),
        },
        state: State::parse(&state.to_string_lossy()),
        data: tag.get("Data").and_then(files::nbt_of),
    })
}

/// `SchematicannonBlockEntity.read`.
fn read_cannon(nbt: &Compound, pos: Pos) -> Cannon {
    let mut cannon = Cannon::new(pos);
    let status = text(nbt, "Status");
    if !status.is_empty() {
        cannon.status = status;
    }
    cannon.state = RunState::from_name(&text(nbt, "State"));
    cannon.remaining_fuel = int(nbt, "RemainingFuel");
    cannon.blocks_placed = int(nbt, "AmountPlaced");
    cannon.blocks_to_place = int(nbt, "AmountToPlace");
    cannon.printer_cooldown = int(nbt, "Cooldown");
    cannon.position_not_loaded = util::boolean(nbt, "PositionNotLoaded");
    cannon.missing_item = match util::get(nbt, "MissingItem") {
        Some(Tag::String(s)) if !s.to_string_lossy().is_empty() => Some(s.to_string_lossy()),
        _ => None,
    };
    if let Some(Tag::Compound(options)) = util::get(nbt, "Options") {
        cannon.replace_mode = match util::get(options, "replace_mode") {
            Some(Tag::Int(v)) => (*v).clamp(0, 3),
            _ => 2,
        };
        cannon.skip_missing = util::boolean(options, "skip_missing");
        cannon.replace_block_entities = util::boolean(options, "replace_block_entities");
    }
    if let Some(Tag::List(list)) = util::get(nbt, "FlyingBlocks") {
        cannon.flying = list.iter().filter_map(flying_of).collect();
    }
    cannon
}

/// `SchematicannonBlockEntity.write`, as the values to put in the compound.
fn write_cannon(cannon: &Cannon, target: Option<Pos>, previous: Option<Pos>) -> Vec<(&'static str, Tag)> {
    let mut options = Compound::new();
    options.insert("replace_mode".into(), Tag::Int(cannon.replace_mode));
    options.insert("skip_missing".into(), Tag::Byte(i8::from(cannon.skip_missing)));
    options.insert("replace_block_entities".into(), Tag::Byte(i8::from(cannon.replace_block_entities)));
    vec![
        ("Status", Tag::String(cannon.status.as_str().into())),
        ("State", Tag::String(cannon.state.name().into())),
        ("RemainingFuel", Tag::Int(cannon.remaining_fuel)),
        ("AmountPlaced", Tag::Int(cannon.blocks_placed)),
        ("AmountToPlace", Tag::Int(cannon.blocks_to_place)),
        ("Cooldown", Tag::Int(cannon.printer_cooldown)),
        ("PositionNotLoaded", Tag::Byte(i8::from(cannon.position_not_loaded))),
        ("MissingItem", Tag::String(cannon.missing_item.as_deref().unwrap_or("").into())),
        ("Options", Tag::Compound(options)),
        ("FlyingBlocks", Tag::List(cannon.flying.iter().map(flying_tag).collect())),
        ("Target", pos_tag(target)),
        ("PrevTarget", pos_tag(previous)),
    ]
}

fn new_entity() -> Compound {
    let mut nbt = Compound::new();
    for (key, value) in write_cannon(&Cannon::new([0; 3]), None, None) {
        nbt.insert(key.into(), value);
    }
    nbt.insert("DefaultYaw".into(), Tag::Float(0.0f32.to_bits()));
    nbt
}

fn container() -> ModContainer {
    ModContainer {
        slots: SLOTS,
        faces: [(); 6].map(|()| Vec::new()),
        inputs: Vec::new(),
        outputs: Vec::new(),
        menu: Some("minecraft:hopper".to_owned()),
    }
}

// ---- what is beside the cannon --------------------------------------------------------------------------

/// What the last look beside a cannon found (`findInventories`): the containers, with the face of
/// them that touches the cannon, and whether a Creative Crate stands there.
#[derive(Clone, Default)]
struct Around {
    checked: i64,
    inventories: Vec<(Pos, Dir)>,
    creative: bool,
}

static AROUND: Mutex<Option<HashMap<Pos, Around>>> = Mutex::new(None);
/// The printer of each cannon (`SchematicPrinter`), loaded from its Schematic.
static PRINTERS: Mutex<Option<HashMap<Pos, Printer>>> = Mutex::new(None);
/// The cannon each player right-clicked last, for the commands.
static TARGETS: Mutex<Option<HashMap<u128, Pos>>> = Mutex::new(None);

/// `findInventories`.
fn look_around(host: &mut dyn Host, pos: Pos) -> Around {
    let mut around = Around { checked: util::game_time(&*host), ..Around::default() };
    let dimension = host.dimension();
    for face in Dir::ALL {
        let other = util::add(pos, face.normal());
        if !host.chunk_is_loaded(dimension, [other[0] >> 4, other[2] >> 4]) {
            continue;
        }
        let block = host.block(other);
        if block == CREATIVE_CRATE {
            around.creative = true;
        }
        if inv::is_inventory(host, other) {
            around.inventories.push((other, face.opposite()));
        }
    }
    around
}

// ---- the world as the rules see it ----------------------------------------------------------------------

/// What the cannon's printing reads and does: its slots, the containers beside it and the world.
struct Live<'a> {
    host: &'a mut dyn Host,
    items: &'a mut Vec<Option<ItemStack>>,
    around: &'a Around,
}

impl Live<'_> {
    fn slot_data(&self, slot: usize) -> Option<items::Data> {
        self.items.get(slot)?.as_ref().and_then(items::data_of)
    }
}

impl Io for Live<'_> {
    fn blueprint(&self) -> Blueprint {
        match self.items.get(BLUEPRINT).and_then(Option::as_ref) {
            None => Blueprint::Empty,
            Some(stack) => match items::data_of(stack) {
                None => Blueprint::Invalid,
                Some(data) if !data.deployed => Blueprint::NotDeployed,
                Some(_) => Blueprint::Deployed,
            },
        }
    }

    fn output_empty(&self) -> bool {
        self.items.get(OUTPUT).is_none_or(Option::is_none)
    }

    fn load(&mut self) -> Result<Printer, LoadError> {
        let data = self.slot_data(BLUEPRINT).ok_or(LoadError::Errored)?;
        let template = files::load(&*self.host, &data.file).map_err(|_| LoadError::Expired)?;
        let layout = Layout::new(&template, data.rotation, data.mirror);
        Ok(Printer::new(data.anchor, layout))
    }

    fn is_loaded(&self, pos: Pos) -> bool {
        world::loaded(&*self.host, pos)
    }

    fn world(&mut self, pos: Pos) -> Existing {
        world::existing(&mut *self.host, pos)
    }

    fn item_exists(&self, item: &str) -> bool {
        self.host.max_stack_size(item).is_some()
    }

    /// `grabItemsFromAttachedInventories`.
    fn take(&mut self, item: &str, count: u32, simulate: bool) -> bool {
        if self.around.creative {
            return true;
        }
        let mut remaining = count;
        for (inventory, face) in &self.around.inventories {
            if remaining == 0 {
                break;
            }
            let wanted = item.to_owned();
            let Some(plan) = inv::plan(&mut *self.host, *inventory, Mode::UpTo, remaining, &mut |_, stack| {
                *stack.item == *wanted
            }) else {
                continue;
            };
            if simulate {
                remaining = remaining.saturating_sub(plan.count);
            } else if let Some(taken) = inv::take(&mut *self.host, *inventory, *face, &plan) {
                remaining = remaining.saturating_sub(taken.count);
            }
        }
        remaining == 0
    }

    /// `refillFuelIfPossible`'s gunpowder: slot 5 first, then the containers beside.
    fn take_gunpowder(&mut self) -> bool {
        if let Some(Some(stack)) = self.items.get_mut(FUEL)
            && &*stack.item == GUNPOWDER
        {
            stack.count -= 1;
            if stack.count == 0 {
                self.items[FUEL] = None;
            }
            return true;
        }
        for (inventory, face) in &self.around.inventories {
            let Some(plan) = inv::plan(&mut *self.host, *inventory, Mode::UpTo, 1, &mut |_, stack| {
                &*stack.item == GUNPOWDER
            }) else {
                continue;
            };
            if inv::take(&mut *self.host, *inventory, *face, &plan).is_some() {
                return true;
            }
        }
        false
    }
}

// ---- the hooks -----------------------------------------------------------------------------------------

/// Writes the values that differ from the entity's and says whether any did (a write to a block
/// entity bumps its revision, so only a changed value is written).
fn store(entity: &mut ModBlockEntity, values: Vec<(&'static str, Tag)>) -> bool {
    let changed: Vec<(&'static str, Tag)> = values
        .into_iter()
        .filter(|(key, value)| util::differs(&entity.nbt, key, value))
        .collect();
    if changed.is_empty() {
        return false;
    }
    for (key, value) in changed {
        entity.nbt.insert(key.into(), value);
    }
    true
}

fn spawn_empty_schematic(host: &mut dyn Host, pos: Pos) {
    let stack = ItemStack { item: EMPTY.into(), count: 1, damage: 0, extra: None };
    util::spawn_item(host, util::centre([pos[0], pos[1] + 1, pos[2]]), &stack, [0.0; 3], 10);
}

impl Hooks for Cannons {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.push(CONNECTOR.to_owned());
        event.models.push(PIPE.to_owned());
    }

    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        (event.block == ID).then(|| (ID.to_owned(), new_entity()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    fn block_entity_container(&self, _: &dyn Host, event: &mut BlockEntityContainerEvent<'_>) -> Option<ModContainer> {
        (event.block == ID).then(container)
    }

    /// `AllShapes` aside: the cannon's drawing may reach a long way (`AABB.INFINITE`).
    fn block_entity_render_bounding_box(&self, _: &dyn Host, event: &mut BlockEntityRenderBoundingBoxEvent<'_>) {
        if event.block != ID {
            return;
        }
        let mut reach = 4;
        if let Some(Tag::List(list)) = util::get(&event.entity.nbt, "FlyingBlocks") {
            for tag in list {
                if let Some(f) = flying_of(tag) {
                    for i in 0..3 {
                        reach = reach.max((f.target[i] - event.pos[i]).abs() + 2);
                    }
                }
            }
        }
        for i in 0..3 {
            event.bounds[i] = event.bounds[i].min(-(reach as f32));
            event.bounds[i + 3] = event.bounds[i + 3].max(reach as f32 + 1.0);
        }
    }

    /// `SchematicannonBlock.setPlacedBy`: the cannon rests turned the way its placer faced.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if &*event.stack.item != ID {
            return;
        }
        let Some(player) = host.acting_player() else {
            return;
        };
        let turned = player.yaw + if player.sneaking { 180.0 } else { 0.0 };
        let steps = ((turned * 16.0 / 360.0 + 0.5).floor() as i32).wrapping_neg() & 15;
        let yaw = steps as f32 * 360.0 / 16.0;
        host.change_mod_block_entity(event.pos, &mut |entity| {
            entity.nbt.insert("DefaultYaw".into(), Tag::Float(yaw.to_bits()));
        });
    }

    /// `SchematicannonBlockEntity.tick`.
    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        let pos = event.pos;
        let now = util::game_time(&*host);
        let mut cannon = read_cannon(&event.entity.nbt, pos);
        let mut slots = event.entity.items.clone();
        slots.resize(SLOTS, None);
        let slots_before = slots.clone();

        // `findInventories`, every `NEIGHBOUR_CHECKING` ticks while not stopped.
        let mut around = lock(&AROUND).get_or_insert_with(HashMap::new).get(&pos).cloned().unwrap_or_default();
        if around.checked == 0
            || (cannon.state != RunState::Stopped && now - around.checked >= i64::from(rules::NEIGHBOUR_CHECKING))
        {
            around = look_around(host, pos);
            lock(&AROUND).get_or_insert_with(HashMap::new).insert(pos, around.clone());
        }
        cannon.has_creative_crate = around.creative;

        let mut printer = lock(&PRINTERS).get_or_insert_with(HashMap::new).remove(&pos);
        let mut effects: Vec<Effect> = Vec::new();
        let previous = printer.as_ref().map(Printer::target);

        // The blocks that have landed.
        for landed in cannon.landed(now) {
            world::place_schematic_block(host, landed.target, &landed.state, landed.data.as_ref());
        }
        {
            let mut live = Live { host: &mut *host, items: &mut slots, around: &around };
            effects.extend(cannon.tick_loader(&mut printer, &mut live));
            cannon.refill_fuel_if_possible(&mut live, &mut effects);
            let mut skips = SKIPS_PER_TICK;
            loop {
                effects.extend(cannon.tick_printer(&mut printer, &mut live, now));
                skips -= 1;
                if !cannon.block_skipped || skips <= 0 {
                    break;
                }
            }
        }

        // What the effects ask of the world and the slots.
        for effect in &effects {
            match effect {
                Effect::Fired => {
                    util::create_sound_at(host, "schematicannon_launch_block", pos, 1.0, 1.0);
                }
                Effect::FinishSound => {
                    util::create_sound_at(host, "schematicannon_finish", pos, 1.0, 1.0);
                }
                Effect::ClearBlueprint { give_empty } => {
                    slots[BLUEPRINT] = None;
                    if *give_empty {
                        if slots[OUTPUT].is_none() {
                            slots[OUTPUT] = Some(ItemStack { item: EMPTY.into(), count: 1, damage: 0, extra: None });
                        } else {
                            spawn_empty_schematic(host, pos);
                        }
                    }
                }
                Effect::Update | Effect::ResetPrinter => {}
            }
        }

        let target = printer.as_ref().map(Printer::target);
        if let Some(printer) = printer {
            lock(&PRINTERS).get_or_insert_with(HashMap::new).insert(pos, printer);
        }
        let mut changed = store(event.entity, write_cannon(&cannon, target, previous));
        if slots != slots_before {
            event.entity.items = slots;
            changed = true;
        }
        if changed {
            host.block_entity_sync(pos);
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        if event.entity.id != ID {
            return None;
        }
        let mut tag = Compound::new();
        for key in ["Status", "State", "RemainingFuel", "AmountPlaced", "AmountToPlace", "FlyingBlocks", "Target", "PrevTarget", "DefaultYaw", "MissingItem"] {
            if let Some(value) = util::get(&event.entity.nbt, key) {
                tag.insert(key.into(), value.clone());
            }
        }
        Some(tag)
    }

    /// `SchematicannonRenderer`: the connector and the pipe turned to the target, and the blocks in
    /// flight.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let nbt = &event.entity.nbt;
        let draw: &mut BlockEntityDraw = &mut *event.draw;
        let pos = event.pos;
        let now = draw.game_time;
        let partial = f64::from(draw.partial);
        let default_yaw = f64::from(match util::get(nbt, "DefaultYaw") {
            Some(Tag::Float(bits)) => f32::from_bits(*bits),
            _ => 0.0,
        });
        let flying: Vec<Flying> = match util::get(nbt, "FlyingBlocks") {
            Some(Tag::List(list)) => list.iter().filter_map(flying_of).collect(),
            _ => Vec::new(),
        };
        let (yaw, pitch) = cannon_angles(pos, pos_of(nbt, "Target"), pos_of(nbt, "PrevTarget"), partial, default_yaw);
        let recoil = recoil(&flying, now, partial);

        draw.push();
        draw.rotate_about([0.0, 1.0, 0.0], (yaw + 90.0) as f32, [0.5, 0.0, 0.5]);
        host.draw_model(draw, CONNECTOR, 0x00FF_FFFF);
        draw.pop();
        draw.push();
        draw.rotate_about([0.0, 1.0, 0.0], (yaw + 90.0) as f32, [0.5, 15.0 / 16.0, 0.5]);
        draw.rotate_about([0.0, 0.0, 1.0], pitch as f32, [0.5, 15.0 / 16.0, 0.5]);
        draw.translate([0.0, -(recoil as f32) / 100.0, 0.0]);
        host.draw_model(draw, PIPE, 0x00FF_FFFF);
        draw.pop();

        for f in &flying {
            let Some((at, turn)) = flight_pose(pos, f, now, partial) else {
                continue;
            };
            // Block-local: the pose starts at the cannon's corner, `flight_pose` is relative to it.
            draw.push();
            draw.translate([at[0] as f32, at[1] as f32, at[2] as f32]);
            draw.translate([0.125, 0.125, 0.125]);
            draw.rotate_about([0.0, 1.0, 0.0], turn as f32, [0.0; 3]);
            draw.rotate_about([1.0, 0.0, 0.0], turn as f32, [0.0; 3]);
            draw.translate([-0.125, -0.125, -0.125]);
            draw.scale([0.3, 0.3, 0.3]);
            let props: Vec<(&str, &str)> = f.state.props.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            if !host.draw_block(draw, &f.state.name, &props) {
                host.draw_block(draw, &f.state.name, &[]);
            }
            draw.pop();
        }
    }

    /// Remembers the cannon for the commands; the menu opens as it always does.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        if host.block(event.pos) != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        lock(&TARGETS).get_or_insert_with(HashMap::new).insert(player.uuid, event.pos);
        Flow::Continue
    }

    /// The Wrench: plays and pauses the cannon (the screen's Play and Pause buttons), and picks it up
    /// sneaking.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != ID {
            return Flow::Continue;
        }
        let pos = event.pos;
        let Some(player) = host.acting_player() else {
            return Flow::Cancel;
        };
        if player.sneaking {
            return util::wrench(host, pos, |_| {});
        }
        if !util::first_click(&*host, player.uuid, pos) {
            return Flow::Cancel;
        }
        lock(&TARGETS).get_or_insert_with(HashMap::new).insert(player.uuid, pos);
        let Some(entity) = host.mod_block_entity(pos) else {
            return Flow::Cancel;
        };
        let mut cannon = read_cannon(&entity.nbt, pos);
        let line = if cannon.state == RunState::Running {
            cannon.pause();
            "Paused".to_owned()
        } else {
            cannon.play();
            "Running".to_owned()
        };
        apply(host, pos, &cannon);
        host.action_bar(player.uuid, Text::literal(line));
        Flow::Cancel
    }

    fn register_commands(&self, _: &mut dyn Host, event: &mut RegisterCommandsEvent<'_>) {
        event.commands.extend(USAGES.iter().map(|usage| NativeCommand::new(usage)));
    }

    fn perform_command(&self, host: &mut dyn Host, event: &mut PerformCommandEvent<'_>) -> Flow {
        let line = event.command.trim();
        let Some(rest) = line.strip_prefix("schematicannon") else {
            return Flow::Continue;
        };
        if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
            return Flow::Continue;
        }
        command(host, event.player.uuid, rest.trim());
        Flow::Cancel
    }
}

/// Writes a cannon's settings and state into the block entity at `pos`.
fn apply(host: &mut dyn Host, pos: Pos, cannon: &Cannon) {
    let values = write_cannon(cannon, None, None);
    host.change_mod_block_entity(pos, &mut |entity| {
        // The target and the flying blocks belong to the tick; the rest is the cannon's.
        for (key, value) in &values {
            if matches!(*key, "Target" | "PrevTarget" | "FlyingBlocks") {
                continue;
            }
            if util::differs(&entity.nbt, key, value) {
                entity.nbt.insert((*key).into(), value.clone());
            }
        }
    });
    host.block_entity_sync(pos);
}

// ---- the commands -----------------------------------------------------------------------------------------

const USAGES: [&str; 9] = [
    "schematicannon help",
    "schematicannon status",
    "schematicannon checklist",
    "schematicannon play",
    "schematicannon pause",
    "schematicannon stop",
    "schematicannon replace <mode:word>",
    "schematicannon skip <enabled:word>",
    "schematicannon protect <enabled:word>",
];

fn say(host: &mut dyn Host, uuid: u128, line: impl Into<String>) {
    host.tell(uuid, Text::literal(line.into()));
}

fn on_off(word: Option<&&str>) -> Option<bool> {
    match word?.to_lowercase().as_str() {
        "on" | "true" | "yes" | "enable" | "enabled" => Some(true),
        "off" | "false" | "no" | "disable" | "disabled" => Some(false),
        _ => None,
    }
}

fn replace_mode(word: &str) -> Option<i32> {
    match word.to_lowercase().as_str() {
        "dont" | "dont_replace" | "none" | "0" => Some(0),
        "solid" | "replace_solid" | "1" => Some(1),
        "any" | "replace_any" | "2" => Some(2),
        "empty" | "replace_empty" | "clear" | "3" => Some(3),
        _ => None,
    }
}

fn command(host: &mut dyn Host, uuid: u128, rest: &str) {
    let words: Vec<&str> = rest.split_whitespace().collect();
    let sub = words.first().copied().unwrap_or("help");
    if sub == "help" {
        for line in [
            "/schematicannon status | checklist     what the cannon is doing | what the schematic needs",
            "/schematicannon play | pause | stop    the screen's buttons (the Wrench plays and pauses)",
            "/schematicannon replace dont|solid|any|empty    what it may overwrite (default any)",
            "/schematicannon skip on|off      skip a block whose item is missing instead of waiting",
            "/schematicannon protect on|off   keep chests and other block entities (default on)",
            "They act on the cannon you right-clicked last.",
        ] {
            say(host, uuid, line);
        }
        return;
    }
    let target = lock(&TARGETS).get_or_insert_with(HashMap::new).get(&uuid).copied();
    let Some(pos) = target.filter(|pos| host.block(*pos) == ID) else {
        say(host, uuid, "Right-click a Schematicannon first");
        return;
    };
    let Some(entity) = host.mod_block_entity(pos) else {
        say(host, uuid, "That Schematicannon isn't loaded");
        return;
    };
    let mut cannon = read_cannon(&entity.nbt, pos);
    match sub {
        "status" => {
            let around = look_around(host, pos);
            cannon.has_creative_crate = around.creative;
            for line in cannon.summary() {
                say(host, uuid, line);
            }
        }
        "checklist" => checklist(host, uuid, pos, &entity, cannon),
        "play" => {
            cannon.play();
            apply(host, pos, &cannon);
            say(host, uuid, "Running");
        }
        "pause" => {
            cannon.pause();
            apply(host, pos, &cannon);
            say(host, uuid, "Paused");
        }
        "stop" => {
            cannon.stop();
            apply(host, pos, &cannon);
            say(host, uuid, "Stopped");
        }
        "replace" => match words.get(1).and_then(|w| replace_mode(w)) {
            Some(mode) => {
                cannon.replace_mode = mode;
                apply(host, pos, &cannon);
                say(host, uuid, format!("Replace mode: {}", rules::REPLACE_MODES[mode as usize]));
            }
            None => say(host, uuid, "Usage: /schematicannon replace dont|solid|any|empty"),
        },
        "skip" => match on_off(words.get(1)) {
            Some(on) => {
                cannon.skip_missing = on;
                apply(host, pos, &cannon);
                say(host, uuid, if on { "Skipping missing blocks" } else { "Waiting for missing blocks" });
            }
            None => say(host, uuid, "Usage: /schematicannon skip on|off"),
        },
        "protect" => match on_off(words.get(1)) {
            Some(on) => {
                cannon.replace_block_entities = !on;
                apply(host, pos, &cannon);
                say(host, uuid, if on { "Block entities are kept" } else { "Block entities may be replaced" });
            }
            None => say(host, uuid, "Usage: /schematicannon protect on|off"),
        },
        other => say(host, uuid, format!("Unknown: schematicannon {other}. /schematicannon help lists what there is.")),
    }
}

/// `updateChecklist`: what the Schematic needs, and what the containers beside the cannon hold.
fn checklist(host: &mut dyn Host, uuid: u128, pos: Pos, entity: &ModBlockEntity, mut cannon: Cannon) {
    let mut slots = entity.items.clone();
    slots.resize(SLOTS, None);
    let around = look_around(host, pos);
    cannon.has_creative_crate = around.creative;
    let mut live = Live { host: &mut *host, items: &mut slots, around: &around };
    if live.blueprint() != Blueprint::Deployed {
        say(live.host, uuid, "Put a positioned Schematic in the cannon's first slot");
        return;
    }
    let Ok(printer) = live.load() else {
        say(live.host, uuid, "The Schematic's file could not be read");
        return;
    };
    cannon.update_checklist(Some(&printer), &mut live);
    let mut list = cannon.checklist.clone();
    // What the containers hold of each needed item.
    for (inventory, _) in &around.inventories {
        if let Some(stacks) = inv::items_of(&mut *live.host, *inventory) {
            for (_, stack) in stacks {
                if list.required.contains_key(&*stack.item) {
                    list.collect(&stack.item, stack.count);
                }
            }
        }
    }
    let lines = list.lines();
    say(live.host, uuid, format!("{} blocks to place; {} kinds of items needed", cannon.blocks_to_place, lines.len()));
    for line in lines.iter().take(40) {
        say(live.host, uuid, line.clone());
    }
    if lines.len() > 40 {
        say(live.host, uuid, format!("... and {} more", lines.len() - 40));
    }
    if list.not_loaded {
        say(live.host, uuid, "Some of the target area is not loaded");
    }
}
