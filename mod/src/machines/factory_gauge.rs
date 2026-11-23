//! The Factory Gauge (`create:factory_gauge`) (`FactoryPanelBlock`, `FactoryPanelBlockEntity`, `FactoryPanelBehaviour`,
//! `FactoryPanelConnectionHandler`, `FactoryPanelConfigurationPacket`, `AllShapes.FACTORY_PANEL_FALLBACK`): up to four panels on one
//! face of a block, each watching how much of one item a logistics network holds and asking for more. The rules are
//! `rules::factory_gauge` (the storage monitor, the requests, the restocker, the connections) and `rules::request_promises`; the
//! network is `logistics.rs`; this is the block, the saved form and the commands that stand in for the gauge's screen.
//!
//! How it works here:
//! - A Stock Link, Stock Ticker, Redstone Requester or Factory Gauge item carries a network frequency (`logistics.rs`): click a linked
//!   block (or a panel) with it to tune it, use it in the air to clear it. A gauge item must be tuned to be placed (`FactoryPanelBlockItem`);
//!   the first panel joins the item's network. A tuned gauge item used on the block adds a panel at the slot the click is nearest to
//!   (`FactoryPanelBlock.getTargetedSlot`, from the player's eye ray against the panel plane): up to four.
//! - Right-click a panel with an empty hand to open it, or with an item (not a gauge or a linked one) to set the item it watches. The
//!   panel is the one the click is nearest to. `/gauge` then shows it, and `/gauge slot`, `item`, `amount <0-100> [items|stacks]`,
//!   `address <text>`, `output <n>`, `promises <never|30s|minutes>`, `clearpromises`, `connect <x> <y> <z> <slot>`, `input ...`,
//!   `disconnect ...`, `move <x> <y> <z> <slot>`, `bend` and `reset` change it (the screen and the value box). A panel with inputs is a
//!   crafting recipe: when it is short of its item it asks the network for the inputs, addressed to `address`. A panel on a Packager is a
//!   restocker: it orders its item for that Packager's inventory.
//! - The arrows between connected panels are drawn from Create's own models (`FactoryPanelRenderer.renderPath`), along the path
//!   `FactoryPanelConnection.getPath` finds; the Wrench bends them.
//! - Sneak-use the Wrench to take the clicked panel off (the block goes with the last one); breaking the block takes the clicked panel
//!   first while there are two or more, as in Create.
//!
//! Known differences from Create:
//! - Redstone Links and Display Links (`FactoryPanelSupportBehaviour`, `panel_support.rs`): `/gauge connect <x> <y> <z>` attaches the
//!   link there (its own slot is the one nearest the panel, as `FactoryPanelConnectionHandler` picks), with an arrow drawn to it;
//!   `/gauge unlink` lets every link go (the "redstone reset" button). A Redstone Link that receives a signal pauses the panel
//!   (`checkForRedstoneInput`, every 40 ticks); a transmitting one sends 15 while a linked panel is satisfied with a target; a Display
//!   Link with panels reads them as the "gauge status" source, one row each.
//! - The screen, the value box and the bulb are not ported; so `activeCraftingArrangement` (Mechanical Crafting) can't be set. The arrows'
//!   glow animation and UV shift aren't drawn, and their status colour is a tint. Connections are made with `/gauge connect`, not by
//!   clicking a second panel or a link.
//! - `moveTo` only moves a panel to a free slot of an existing gauge block in the same state (it doesn't place a new block).
//! - An untuned gauge item is refused on any block that isn't a linked block, so it doesn't open chests and the like either.
//! - The panels' effect packets (the sparks along the arrows) aren't sent.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

use create_kinetics::rules::factory_gauge::{
    self as rules, Added, Configuration, Connection, ConnectionSource, Disconnected, Gauge, LinkRead, PanelPair, PanelPosition, PanelRead,
    Reads, RequestEnv, Slot, Source,
};
use create_kinetics::rules::factory_gauge_geometry as geometry;
use create_kinetics::rules::filter::display_name;
use create_kinetics::rules::logistics_item as item_rules;
use create_kinetics::rules::logistics_network::{Freq, Network, Networks};
use create_kinetics::rules::stock_link::AttachFace;
use create_kinetics::rules::value_box::{Vec3, view_vector};
use create_kinetics::rules::package::Big;
use create_kinetics::rules::request_promises::{Promise, Queue};
use create_kinetics::rules::shop_commands::{self as commands, Gauge as Command, ItemArg, slot_name};
use create_kinetics::{Dir, Pos};
use crate::compat::{
    BlockEntityEvent, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, BlockStateId, Compound, DestroyBlockEvent, Flow, Hooks, Host, ItemStack, LevelHost, NativeCommand,
    NativeServerPlayer, PerformCommandEvent, PlaceBlockEvent, RegisterCommandsEvent, RightClickBlockEvent, StandaloneModelsEvent,
    StateForPlacementEvent, Tag, Text, UseItemOnEvent,
};

use super::logistics as network;
use super::panel_support as support;
use super::display_link;
use super::package as items;
use super::packager::PACKAGER;
use super::packager_inv as target_inv;
use crate::{int, util};

pub const ID: &str = network::FACTORY_GAUGE;
/// `FactoryPanelBlockEntity.write`.
const RESTOCKER: &str = "Restocker";
/// `CLogistics.factoryGaugeTimer`.
const INTERVAL: i32 = rules::DEFAULT_REQUEST_INTERVAL;
/// A filter, which a panel can't watch (`FilterItem`).
const FILTERS: [&str; 3] = ["create:filter", "create:attribute_filter", "create:package_filter"];

struct Gauges;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Gauges)
}

fn lock<T>(mutex: &'static Mutex<T>) -> MutexGuard<'static, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The panel each player has open, for `/gauge`.
static OPEN: LazyLock<Mutex<BTreeMap<u128, PanelPosition>>> = LazyLock::new(Mutex::default);
/// A gauge just placed, until its block entity ticks: the network its first panel starts with and the slot the click meant.
static PLACED: LazyLock<Mutex<BTreeMap<(String, Pos), (Freq, Option<Slot>)>>> = LazyLock::new(Mutex::default);
/// `AllPartialModels.FACTORY_PANEL_ARROWS`, `_LINES`, `_DOTTED`, by the direction they are named after.
const CONNECTION_MODELS: [&str; 3] = ["arrow", "line", "dotted"];
/// The paths found for the arrows, kept while the bend mode is the one they were found with (`FactoryPanelConnection.path`).
static PATHS: LazyLock<Mutex<BTreeMap<(Pos, Slot, PanelPosition, i32), Vec<Dir>>>> = LazyLock::new(Mutex::default);

fn connection_model(kind: &str, dir: Dir) -> String {
    format!("create:block/factory_gauge/connections/{kind}_{}", dir.name())
}

fn face_and_facing(host: &dyn Host, pos: Pos) -> Option<(AttachFace, Dir)> {
    let face = host.block_property(pos, "face").as_deref().and_then(AttachFace::from_name)?;
    Some((face, util::facing(host, pos, "facing")?))
}

/// `getTargetedSlot` for where the player's eye ray meets the gauge's plane (the click spot the events don't carry).
fn hit_slot(host: &dyn Host, player: &NativeServerPlayer, pos: Pos) -> Option<Slot> {
    let (face, facing) = face_and_facing(host, pos)?;
    let side = connected_direction(host, pos)?;
    let eye = [player.position[0], player.position[1] + f64::from(player.eye_height), player.position[2]];
    let look = view_vector(f64::from(player.yaw), f64::from(player.pitch));
    let at = create_kinetics::rules::display_link::hit_on_face(eye, look, pos, side.normal());
    let local: Vec3 = [at[0] - f64::from(pos[0]), at[1] - f64::from(pos[1]), at[2] - f64::from(pos[2])];
    Some(geometry::targeted_slot(face, facing, local))
}

fn tell(host: &mut dyn Host, uuid: u128, line: impl Into<String>) {
    host.tell(uuid, Text::literal(line.into()));
}

fn status(host: &mut dyn Host, uuid: u128, line: impl Into<String>) {
    host.action_bar(uuid, Text::literal(line.into()));
}

fn gauge_item() -> ItemStack {
    ItemStack { item: ID.into(), count: 1, damage: 0, extra: None }
}

fn key_of(id: &str) -> ItemStack {
    ItemStack { item: id.into(), count: 1, damage: 0, extra: None }
}

fn random_freq() -> Freq {
    (u128::from(util::random()) << 64) | u128::from(util::random())
}

// --- the saved form ---------------------------------------------------------------------------------------

pub(super) fn pos_tag(at: &PanelPosition) -> Tag {
    let mut tag = Compound::new();
    tag.insert("pos".into(), Tag::IntArray(at.pos.to_vec()));
    tag.insert("slot".into(), Tag::String(slot_name(at.slot).into()));
    Tag::Compound(tag)
}

pub(super) fn pos_of(tag: &Tag) -> Option<PanelPosition> {
    let Some(Tag::IntArray(ints)) = tag.get("pos") else {
        return None;
    };
    let Some(Tag::String(name)) = tag.get("slot") else {
        return None;
    };
    Some(PanelPosition { pos: [*ints.first()?, *ints.get(1)?, *ints.get(2)?], slot: commands::parse_slot(&name.to_string_lossy()).ok()? })
}

/// `FactoryPanelConnection.CODEC`.
fn connection_tag(connection: &Connection) -> Tag {
    let Tag::Compound(mut tag) = pos_tag(&connection.from) else {
        unreachable!("a position is a compound");
    };
    tag.insert("amount".into(), Tag::Int(connection.amount));
    tag.insert("arrow_bend_mode".into(), Tag::Int(connection.arrow_bend_mode));
    Tag::Compound(tag)
}

fn connection_of(tag: &Tag) -> Option<Connection> {
    let from = pos_of(tag)?;
    let number = |key: &str, default: i32| match tag.get(key) {
        Some(Tag::Int(value)) => *value,
        _ => default,
    };
    Some(Connection { from, amount: number("amount", 1), arrow_bend_mode: number("arrow_bend_mode", -1) })
}

fn list_of<T>(nbt: &Compound, key: &str, read: impl Fn(&Tag) -> Option<T>) -> Vec<T> {
    match util::get(nbt, key) {
        Some(Tag::List(list)) => list.iter().filter_map(read).collect(),
        _ => Vec::new(),
    }
}

fn sized(key: &ItemStack, count: u32) -> ItemStack {
    ItemStack { count, ..key.clone() }
}

/// `FactoryPanelBehaviour.write`, for an active panel.
fn write_panel(panel: &Gauge<ItemStack>) -> Compound {
    let mut tag = Compound::new();
    if let Some(filter) = &panel.filter {
        tag.insert("Filter".into(), util::stack_tag(filter));
    }
    tag.insert("UpTo".into(), Tag::Byte(i8::from(panel.up_to)));
    tag.insert("FilterAmount".into(), Tag::Int(panel.count));
    tag.insert("Timer".into(), Tag::Int(panel.timer));
    tag.insert("LastLevel".into(), Tag::Int(panel.last_reported_level_in_storage as i32));
    tag.insert("LastPromised".into(), Tag::Int(panel.last_reported_promises as i32));
    tag.insert("LastUnloadedLinks".into(), Tag::Int(panel.last_reported_unloaded_links as i32));
    tag.insert("Satisfied".into(), Tag::Byte(i8::from(panel.satisfied)));
    tag.insert("PromisedSatisfied".into(), Tag::Byte(i8::from(panel.promised_satisfied)));
    tag.insert("Waiting".into(), Tag::Byte(i8::from(panel.waiting_for_network)));
    tag.insert("RedstonePowered".into(), Tag::Byte(i8::from(panel.redstone_powered)));
    tag.insert("Targeting".into(), Tag::List(panel.targeting.iter().map(pos_tag).collect()));
    tag.insert("TargetedBy".into(), Tag::List(panel.targeted_by.iter().map(connection_tag).collect()));
    tag.insert("TargetedByLinks".into(), Tag::List(panel.targeted_by_links.iter().map(connection_tag).collect()));
    tag.insert("RecipeAddress".into(), Tag::String(panel.recipe_address.as_str().into()));
    tag.insert("RecipeOutput".into(), Tag::Int(panel.recipe_output as i32));
    tag.insert("PromiseClearingInterval".into(), Tag::Int(panel.promise_clearing_interval));
    tag.insert("Freq".into(), network::freq_tag(panel.network));
    tag.insert("Craft".into(), util::stacks_tag(&panel.active_crafting_arrangement));
    let promises: Vec<Tag> = panel
        .restocker_promises
        .flatten(false)
        .into_iter()
        .map(|promise| {
            let mut entry = Compound::new();
            entry.insert("ticks".into(), Tag::Int(promise.ticks_existed));
            entry.insert("stack".into(), util::stack_tag(&sized(&promise.stack.0, promise.stack.1)));
            Tag::Compound(entry)
        })
        .collect();
    tag.insert("Promises".into(), Tag::List(promises));
    tag
}

/// `FactoryPanelBehaviour.read` of the panel in `slot` (inactive when the entity holds nothing for it).
fn read_panel(nbt: &Compound, slot: Slot) -> Gauge<ItemStack> {
    let Some(Tag::Compound(tag)) = util::get(nbt, slot_name(slot)) else {
        return Gauge::new(slot, 0);
    };
    let mut panel = Gauge::new(slot, network::freq_of(tag).unwrap_or(0));
    panel.active = true;
    panel.filter = util::get(tag, "Filter").and_then(util::stack_of).map(|stack| items::key(&stack));
    panel.up_to = !util::has(tag, "UpTo") || util::boolean(tag, "UpTo");
    panel.count = int(tag, "FilterAmount");
    panel.timer = int(tag, "Timer");
    panel.last_reported_level_in_storage = int(tag, "LastLevel").max(0) as u32;
    panel.last_reported_promises = int(tag, "LastPromised").max(0) as u32;
    panel.last_reported_unloaded_links = int(tag, "LastUnloadedLinks").max(0) as u32;
    panel.satisfied = util::boolean(tag, "Satisfied");
    panel.promised_satisfied = util::boolean(tag, "PromisedSatisfied");
    panel.waiting_for_network = util::boolean(tag, "Waiting");
    panel.redstone_powered = util::boolean(tag, "RedstonePowered");
    panel.targeting = list_of(tag, "Targeting", pos_of);
    panel.targeted_by = list_of(tag, "TargetedBy", connection_of);
    panel.targeted_by_links = list_of(tag, "TargetedByLinks", connection_of);
    panel.recipe_address = match util::get(tag, "RecipeAddress") {
        Some(Tag::String(text)) => text.to_string_lossy(),
        _ => String::new(),
    };
    panel.recipe_output = int(tag, "RecipeOutput").max(1) as u32;
    panel.promise_clearing_interval = if util::has(tag, "PromiseClearingInterval") { int(tag, "PromiseClearingInterval") } else { -1 };
    panel.active_crafting_arrangement = util::stacks_of(tag, "Craft").iter().map(items::key).collect();
    let mut queue: Queue<ItemStack> = Queue::default();
    for entry in list_of(tag, "Promises", |entry| {
        let stack = entry.get("stack").and_then(util::stack_of)?;
        let ticks = match entry.get("ticks") {
            Some(Tag::Int(ticks)) => *ticks,
            _ => 0,
        };
        Some(Promise { ticks_existed: ticks, stack: (items::key(&stack), stack.count) })
    }) {
        queue.add(entry);
    }
    queue.take_changed();
    panel.restocker_promises = queue;
    panel
}

fn read_panels(nbt: &Compound) -> Vec<Gauge<ItemStack>> {
    Slot::ALL.iter().map(|slot| read_panel(nbt, *slot)).collect()
}

fn index(slot: Slot) -> usize {
    Slot::ALL.iter().position(|s| *s == slot).unwrap_or(0)
}

/// Writes the panels and the restocker flag; whether anything differed.
fn store(nbt: &mut Compound, panels: &[Gauge<ItemStack>], restocker: bool) -> bool {
    let mut changed = false;
    for panel in panels {
        let key = slot_name(panel.slot);
        if panel.active {
            let value = Tag::Compound(write_panel(panel));
            if util::differs(nbt, key, &value) {
                util::put(nbt, key, value);
                changed = true;
            }
        } else if util::has(nbt, key) {
            util::remove(nbt, key);
            changed = true;
        }
    }
    let flag = Tag::Byte(i8::from(restocker));
    if util::differs(nbt, RESTOCKER, &flag) {
        util::put(nbt, RESTOCKER, flag);
        changed = true;
    }
    changed
}

fn active_count(panels: &[Gauge<ItemStack>]) -> usize {
    panels.iter().filter(|p| p.active).count()
}

// --- the world around a gauge ---------------------------------------------------------------------------------

/// `FactoryPanelBlock.connectedDirection`.
pub fn connected_direction(host: &dyn Host, pos: Pos) -> Option<Dir> {
    let face = host.block_property(pos, "face").as_deref().and_then(create_kinetics::rules::stock_link::AttachFace::from_name)?;
    let facing = util::facing(host, pos, "facing")?;
    Some(create_kinetics::rules::stock_link::connected_direction(face, facing))
}

/// `FactoryPanelBlockEntity.getRestockedPackager`: the Packager the gauge is on (a Re-Packager doesn't count).
fn restocked_packager(host: &dyn Host, pos: Pos) -> Option<Pos> {
    let behind = util::add(pos, connected_direction(host, pos)?.opposite().normal());
    (host.block(behind) == PACKAGER).then_some(behind)
}

/// What the Packager at `packager` holds of `key`.
fn held_of(host: &mut dyn Host, packager: Pos, key: &ItemStack) -> u32 {
    let Some(facing) = util::facing(&*host, packager, "facing") else {
        return 0;
    };
    let target = util::add(packager, facing.opposite().normal());
    target_inv::read(host, target)
        .unwrap_or_default()
        .iter()
        .filter(|(_, stack)| items::key(stack) == *key)
        .map(|(_, stack)| stack.count)
        .sum()
}

/// A panel as `FactoryPanelBehaviour.at` finds it: none unless it is active.
fn panel_at(host: &dyn Host, at: &PanelPosition) -> Option<(Gauge<ItemStack>, bool)> {
    if host.block(at.pos) != ID {
        return None;
    }
    let entity = host.mod_block_entity(at.pos)?;
    let panel = read_panel(&entity.nbt, at.slot);
    panel.active.then(|| (panel, util::boolean(&entity.nbt, RESTOCKER)))
}

/// What a link's `FactoryPanelSupportBehaviour` sees of a panel (`shouldBePoweredTristate`): not loaded, gone, or its state.
pub(super) fn panel_read(host: &dyn Host, at: &PanelPosition) -> PanelRead {
    if !host.chunk_is_loaded(host.dimension(), [at.pos[0] >> 4, at.pos[2] >> 4]) {
        return PanelRead::Unloaded;
    }
    match panel_at(host, at) {
        Some((panel, _)) => PanelRead::Present { satisfied: panel.satisfied, count: panel.count },
        None => PanelRead::Missing,
    }
}

/// `FactoryGaugeDisplaySource.provideEntries`: a row for each panel a Display Link is attached to (the ones that are gone are left out).
pub fn status_entries(host: &dyn Host, panels: &[PanelPosition]) -> Vec<(String, i64)> {
    panels
        .iter()
        .filter_map(|at| {
            let (panel, _) = panel_at(host, at)?;
            let (name, max_stack) = match &panel.filter {
                Some(key) => (display_name(&key.item), host.max_stack_size(&key.item).unwrap_or(64)),
                None => ("Air".to_owned(), 64),
            };
            Some(rules::display_entry(&panel, max_stack, &name))
        })
        .collect()
}

/// Changes the panel at `at` and saves it; whether there was one.
fn update_panel(host: &mut dyn Host, at: &PanelPosition, change: impl FnOnce(&mut Gauge<ItemStack>)) -> bool {
    if host.block(at.pos) != ID {
        return false;
    }
    let mut change = Some(change);
    let mut found = false;
    host.change_mod_block_entity(at.pos, &mut |entity| {
        let mut panels = read_panels(&entity.nbt);
        let panel = &mut panels[index(at.slot)];
        if !panel.active {
            return;
        }
        if let Some(change) = change.take() {
            change(panel);
        }
        let restocker = util::boolean(&entity.nbt, RESTOCKER);
        store(&mut entity.nbt, &panels, restocker);
        found = true;
    });
    if found {
        host.block_entity_sync(at.pos);
    }
    found
}

/// What `disconnectAll` leaves for the other panels.
fn apply_disconnected(host: &mut dyn Host, gone: &Disconnected) {
    for link in &gone.links {
        unlink(host, *link, gone.here);
    }
    for source in &gone.sources_stop_targeting {
        update_panel(host, source, |panel| panel.targeting.retain(|p| *p != gone.here));
    }
    for target in &gone.targets_stop_being_targeted_by {
        update_panel(host, target, |panel| panel.targeted_by.retain(|c| c.from != gone.here));
    }
}

/// `FactoryPanelSupportBehaviour.disconnect(panel)` of the link at `link`; a Display Link writes its target again, for it no longer
/// reads that panel.
fn unlink(host: &mut dyn Host, link: Pos, panel: PanelPosition) {
    if support::disconnect(host, link, panel) && host.block(link) == display_link::ID {
        display_link::refresh(host, link);
    }
}

/// `FactoryPanelBehaviour.addConnection` to a link (`FactoryPanelConnectionHandler` after a click on a Redstone Link or Display Link):
/// the panel at `at` keeps the link, and the link keeps the panel.
fn connect_link(host: &mut dyn Host, at: &PanelPosition, restocker: bool, link: Pos) -> Result<String, String> {
    let block = host.block(link);
    if !support::is_support(&block) {
        return Err("There is no Redstone Link or Display Link there".to_owned());
    }
    let (Some((face, facing)), Some(connected)) = (face_and_facing(&*host, at.pos), connected_direction(&*host, at.pos)) else {
        return Err(rules::Issue::ConnectionAborted.message().to_owned());
    };
    // `checkForIssues(from, FactoryPanelSupportBehaviour to)`: the link's own `FACING` must be the gauge's connected direction.
    let same_facing = match util::facing(&*host, link, "facing") {
        Some(f) => f == connected,
        None => true,
    };
    let diff = [link[0] - at.pos[0], link[1] - at.pos[1], link[2] - at.pos[2]];
    let along = match connected.axis().name() {
        "x" => diff[0],
        "y" => diff[1],
        _ => diff[2],
    };
    if let Some(issue) = rules::check_link(false, same_facing, diff, along) {
        return Err(issue.message().to_owned());
    }
    let from = PanelPosition { pos: link, slot: geometry::best_link_slot(face, facing, link, *at) };
    let mut added = Added::No;
    update_panel(host, at, |panel| {
        added = panel.add_connection(*at, from, ConnectionSource::Link, restocker);
    });
    if added != Added::Link {
        return Err("Nothing was connected".to_owned());
    }
    support::connect(host, link, *at);
    if block == display_link::ID {
        display_link::refresh(host, link);
    }
    Ok(format!("Connected to {}", create_kinetics::rules::arm_points::block_title(&block)))
}

/// `FactoryPanelBehaviour.checkForRedstoneInput` for the panels with links: every attached link is read, and a link that is gone is
/// forgotten (Create's `FactoryPanelSupportBehaviour.destroy` does that when the link is broken).
fn check_redstone_input(host: &mut dyn Host, panels: &mut [Gauge<ItemStack>]) -> bool {
    let mut dirty = false;
    for panel in panels.iter_mut().filter(|p| p.active && !p.targeted_by_links.is_empty()) {
        let mut reads = Vec::new();
        let mut gone = Vec::new();
        for connection in &panel.targeted_by_links {
            let read = support::read_link(host, connection.from.pos);
            if read == LinkRead::Missing {
                gone.push(connection.from.pos);
            }
            reads.push(read);
        }
        if !gone.is_empty() {
            panel.targeted_by_links.retain(|c| !gone.contains(&c.from.pos));
            dirty = true;
        }
        let present: Vec<LinkRead> = reads.into_iter().filter(|r| *r != LinkRead::Missing).collect();
        dirty |= panel.check_for_redstone_input(&present);
    }
    dirty
}

// --- the tick ---------------------------------------------------------------------------------------------------

/// `FactoryPanelBehaviour.tickStorageMonitor` and `tickRequests` for the panel at index `i`; whether it changed what is saved or shown.
fn tick_panel(host: &mut dyn Host, pos: Pos, panels: &mut [Gauge<ItemStack>], i: usize, restocker: bool) -> bool {
    let mut dirty = false;
    let freq = panels[i].network;
    let packager = if restocker { restocked_packager(&*host, pos) } else { None };
    let unloaded = if restocker { u32::from(packager.is_none()) } else { network::unloaded_links(freq) };
    if panels[i].wants_fresh_summary(restocker, unloaded) {
        network::invalidate_summary(freq);
    }
    let filter = panels[i].filter.clone();
    let max_stack = filter.as_ref().map_or(64, |key| host.max_stack_size(&key.item).unwrap_or(64));
    // `getLevelInStorage`.
    let level = match &filter {
        None => 0,
        Some(key) if restocker => packager.map_or(0, |at| held_of(host, at, key)),
        Some(key) => network::summary(host, freq, false).count_of(key),
    };
    // `getPromised`.
    let promised = if restocker {
        panels[i].promised(true, None, INTERVAL)
    } else {
        let panel = &mut panels[i];
        network::with_queue(freq, |queue| panel.promised(false, queue, INTERVAL))
    };
    let reads = Reads { restocker, unloaded_links: unloaded, level_in_storage: level, max_stack_size: max_stack };
    let tick = panels[i].tick_storage_monitor(&reads, promised);
    if tick.confirm {
        host.play_sound("create:confirm", util::centre(pos), 2, 0.075, 1.0);
        host.play_sound("create:confirm_2", util::centre(pos), 2, 0.125, 0.575);
    }
    dirty |= tick.changed;

    if panels[i].targeted_by.is_empty() && !restocker {
        return dirty;
    }
    let ignored = packager.and_then(|at| network::packager_inventory_id(host, at));
    let has_inventory = restocker.then(|| packager.is_some() && ignored.is_some());
    // The network's promise queue is worked on as a copy, and put back once the gauge has made its request.
    let mut scratch: Networks<ItemStack> = Networks::default();
    let queue = network::with_queue(freq, |queue| queue.cloned());
    if let Some(queue) = queue {
        let mut entry: Network<ItemStack> = Network::new(freq);
        entry.promises = queue;
        scratch.networks.insert(freq, entry);
    }
    let here = panels[i].position(pos);
    let outcome = {
        let cell = RefCell::new(&mut *host);
        let packagers = RefCell::new(Vec::new());
        let source = |at: &PanelPosition| -> Option<Source<ItemStack>> {
            if at.pos == pos {
                return None;
            }
            panel_at(&mut **cell.borrow_mut(), at).map(|(panel, _)| Source { filter: panel.filter, network: panel.network })
        };
        let summary = |freq: Freq| network::summary(&mut **cell.borrow_mut(), freq, true);
        let links = |freq: Freq| {
            let (views, found) = network::links_with_packagers(&mut **cell.borrow_mut(), freq, ignored);
            packagers.borrow_mut().extend(found);
            views
        };
        let mut rng = network::Rng;
        let mut env = RequestEnv {
            source: &source,
            accurate_summary: &summary,
            links: &links,
            rng: &mut rng,
            networks: &mut scratch,
            interval: INTERVAL,
            restocked_packager_has_inventory: has_inventory,
        };
        // A sibling panel of this block is read from the world, which still holds what was last saved.
        let outcome = panels[i].tick_requests(pos, &reads, &mut env);
        (outcome, packagers.into_inner())
    };
    let (outcome, packagers) = outcome;
    if outcome.promised.is_some() && !restocker {
        let kept = scratch.networks.remove(&freq).map(|n| n.promises);
        if let Some(kept) = kept {
            network::with_queue(freq, |queue| {
                if let Some(queue) = queue {
                    *queue = kept;
                }
            });
        }
    }
    let _ = here;
    if !outcome.dispatches.is_empty() {
        network::perform(host, outcome.dispatches, &packagers);
        dirty = true;
    }
    dirty
}

// --- commands ---------------------------------------------------------------------------------------------------

fn describe(panel: &Gauge<ItemStack>, restocker: bool) -> String {
    let item = panel.filter.as_ref().map_or("no item".to_owned(), |key| display_name(&key.item));
    let amount = format!("{} {}", panel.count, if panel.up_to { "items" } else { "stacks" });
    let state = if panel.filter.is_none() {
        "New factory task"
    } else if panel.waiting_for_network {
        "Some links are not loaded"
    } else if panel.redstone_powered {
        "Redstone paused"
    } else if panel.satisfied {
        "Satisfied"
    } else if panel.promised_satisfied {
        "Waiting for promised items"
    } else if panel.is_missing_address(restocker) {
        "Inactive: missing a target address"
    } else if !panel.targeted_by.is_empty() && panel.count == 0 {
        "Inactive: no target amount set"
    } else {
        "In progress"
    };
    format!(
        "Factory Gauge [{}]{}: {item}, target {amount}, in storage {}, promised {} | {state} | address: {} | output {} | {} input(s)",
        slot_name(panel.slot),
        if restocker { " (restocker)" } else { "" },
        panel.last_reported_level_in_storage,
        panel.last_reported_promises,
        if panel.recipe_address.is_empty() { "none" } else { panel.recipe_address.as_str() },
        panel.recipe_output,
        panel.targeted_by.len()
    )
}

/// The panel a player's commands are for: the one they opened, if it is still there.
fn open_panel(host: &mut dyn Host, uuid: u128) -> Option<(PanelPosition, Gauge<ItemStack>, bool)> {
    let at = lock(&OPEN).get(&uuid).copied()?;
    let Some((panel, restocker)) = panel_at(host, &at) else {
        lock(&OPEN).remove(&uuid);
        return None;
    };
    Some((at, panel, restocker))
}

/// `FactoryPanelConfigurationPacket` carrying a panel's own values (what the screen would send unchanged).
fn unchanged(panel: &Gauge<ItemStack>) -> Configuration<ItemStack> {
    Configuration {
        address: panel.recipe_address.clone(),
        input_amounts: panel.targeted_by.iter().map(|c| (c.from, c.amount)).collect(),
        crafting_arrangement: panel.active_crafting_arrangement.clone(),
        output_amount: panel.recipe_output,
        promise_clearing_interval: panel.promise_clearing_interval,
        remove_connection: None,
        clear_promises: false,
        reset: false,
        redstone_reset: false,
    }
}

fn is_filter(item: &str) -> bool {
    FILTERS.contains(&item)
}

/// `FactoryPanelConnectionHandler.checkForIssues(from, to)` for connecting `source` as an input of `at`.
fn connect_issue(host: &mut dyn Host, at: &PanelPosition, target: &Gauge<ItemStack>, source: &PanelPosition) -> Option<rules::Issue> {
    let found = panel_at(host, source);
    let Some((from, from_restocker)) = found else {
        return Some(rules::Issue::ConnectionAborted);
    };
    let same_state = |key: &str| host.block_property(at.pos, key) == host.block_property(source.pos, key);
    let same_orientation = same_state("face") && same_state("facing");
    let diff = [source.pos[0] - at.pos[0], source.pos[1] - at.pos[1], source.pos[2] - at.pos[2]];
    let along = match connected_direction(&*host, at.pos).map(|d| d.axis()) {
        Some(axis) => match axis.name() {
            "x" => diff[0],
            "y" => diff[1],
            _ => diff[2],
        },
        None => 0,
    };
    rules::check_panels(&PanelPair {
        from_missing: false,
        from_targeted_by_to: target.targeted_by.iter().any(|c| c.from == *source),
        from_inputs: target.targeted_by.len(),
        same_orientation,
        diff,
        diff_along_face_axis: along,
        to_restocker: from_restocker,
        to_filter_empty: from.filter.is_none(),
        from_filter_empty: target.filter.is_none(),
    })
}

/// `FactoryPanelBehaviour.moveTo`: the panel at `at` goes to the free slot `to` of a Factory Gauge in the same state (this port doesn't
/// place a new gauge block for the target), and its connections follow it.
fn move_panel(host: &mut dyn Host, at: &PanelPosition, panel: &Gauge<ItemStack>, to: PanelPosition) -> Result<String, &'static str> {
    let target_is_gauge = host.block(to.pos) == ID;
    let target_active = panel_at(host, &to).is_some();
    let same_state = ["face", "facing"].iter().all(|key| host.block_property(at.pos, key) == host.block_property(to.pos, key));
    let diff = [to.pos[0] - at.pos[0], to.pos[1] - at.pos[1], to.pos[2] - at.pos[2]];
    let along = match connected_direction(&*host, at.pos).map(|d| d.axis()) {
        Some(axis) => match axis.name() {
            "x" => diff[0],
            "y" => diff[1],
            _ => diff[2],
        },
        None => 0,
    };
    let packager_behind = restocked_packager(&*host, to.pos).is_some();
    let connected: Vec<Pos> =
        panel
            .targeting
            .iter()
            .map(|p| p.pos)
            .chain(panel.targeted_by.iter().map(|c| c.from.pos))
            .chain(panel.targeted_by_links.iter().map(|c| c.from.pos))
            .collect();
    if let Some(why) = geometry::check_move(target_active, target_is_gauge, same_state, &connected, to.pos, along, packager_behind) {
        return Err(match why {
            geometry::MoveRefusal::Occupied => "That panel slot is taken",
            geometry::MoveRefusal::NotAGauge => "Move it to a slot of another Factory Gauge",
            geometry::MoveRefusal::OtherState => "The gauges must face the same way",
            geometry::MoveRefusal::TooFar => "A connection would be too long",
            geometry::MoveRefusal::OffSurface => "The panels must be on the same surface",
            geometry::MoveRefusal::OnPackager => "A restocker can't go there",
        });
    }
    let mut moved = panel.clone();
    moved.slot = to.slot;
    let keep = moved.clone();
    let from = *at;
    if from.pos == to.pos {
        host.change_mod_block_entity(from.pos, &mut |entity| {
            let mut panels = read_panels(&entity.nbt);
            panels[index(from.slot)].disable();
            panels[index(to.slot)] = keep.clone();
            let restocker = util::boolean(&entity.nbt, RESTOCKER);
            store(&mut entity.nbt, &panels, restocker);
        });
    } else {
        host.change_mod_block_entity(to.pos, &mut |entity| {
            let mut panels = read_panels(&entity.nbt);
            panels[index(to.slot)] = keep.clone();
            let restocker = util::boolean(&entity.nbt, RESTOCKER);
            store(&mut entity.nbt, &panels, restocker);
        });
        host.change_mod_block_entity(from.pos, &mut |entity| {
            let mut panels = read_panels(&entity.nbt);
            panels[index(from.slot)].disable();
            let restocker = util::boolean(&entity.nbt, RESTOCKER);
            store(&mut entity.nbt, &panels, restocker);
        });
    }
    // "Rewire connections".
    for target in &panel.targeting {
        update_panel(host, target, |other| {
            for connection in other.targeted_by.iter_mut().filter(|c| c.from == from) {
                connection.from = to;
            }
        });
    }
    for connection in &panel.targeted_by {
        update_panel(host, &connection.from, |other| {
            for target in other.targeting.iter_mut().filter(|p| **p == from) {
                *target = to;
            }
        });
    }
    // "Reconnect links": each link forgets the old position and learns the new one.
    for connection in &panel.targeted_by_links {
        support::disconnect(host, connection.from.pos, from);
        support::connect(host, connection.from.pos, to);
    }
    host.block_entity_sync(from.pos);
    host.block_entity_sync(to.pos);
    host.play_sound("minecraft:block.copper.place", util::centre(to.pos), 2, 1.0, 0.8);
    Ok(format!("Panel moved to {} {} {} [{}]", to.pos[0], to.pos[1], to.pos[2], slot_name(to.slot)))
}

fn run(host: &mut dyn Host, uuid: u128, line: &str) {
    let command = match commands::parse_gauge(line) {
        Ok(command) => command,
        Err(message) => {
            tell(host, uuid, message);
            return;
        }
    };
    let Some((at, panel, restocker)) = open_panel(host, uuid) else {
        tell(host, uuid, "Right-click a Factory Gauge first");
        return;
    };
    if let Command::Slot(slot) = command {
        let next = PanelPosition { pos: at.pos, slot };
        match panel_at(host, &next) {
            Some((panel, restocker)) => {
                lock(&OPEN).insert(uuid, next);
                tell(host, uuid, describe(&panel, restocker));
            }
            None => tell(host, uuid, "There is no panel there"),
        }
        return;
    }
    if !network::may_interact(host, panel.network, uuid) {
        return;
    }
    let mut message: Option<String> = None;
    let mut ok = true;
    match command {
        Command::Show | Command::Slot(_) => {}
        Command::Item(arg) => {
            let key = match arg {
                ItemArg::Clear => None,
                ItemArg::Id(id) => Some(key_of(&id)),
                ItemArg::Held => match host.held_stack(uuid) {
                    Some(stack) => Some(items::key(&stack)),
                    None => {
                        tell(host, uuid, "Hold the item to monitor, or name it: /gauge item <item>");
                        return;
                    }
                },
            };
            let filter_item = key.as_ref().is_some_and(|k| is_filter(&k.item));
            ok = update_panel(host, &at, |panel| {
                panel.set_filter(key, filter_item);
            }) && !filter_item;
            if filter_item {
                message = Some("A filter can't be the item".to_owned());
            }
        }
        Command::Amount { value, stacks } => {
            update_panel(host, &at, |panel| {
                panel.set_value_settings(i32::from(stacks), value, INTERVAL);
            });
        }
        Command::Address(text) => {
            update_panel(host, &at, |panel| {
                let mut packet = unchanged(panel);
                packet.address = text;
                panel.apply_configuration(at, &packet);
            });
        }
        Command::Output(amount) => {
            update_panel(host, &at, |panel| {
                let mut packet = unchanged(panel);
                packet.output_amount = amount;
                panel.apply_configuration(at, &packet);
            });
        }
        Command::Promises(interval) => {
            update_panel(host, &at, |panel| {
                let mut packet = unchanged(panel);
                packet.promise_clearing_interval = interval;
                panel.apply_configuration(at, &packet);
            });
        }
        Command::ClearPromises => {
            update_panel(host, &at, |panel| {
                let mut packet = unchanged(panel);
                packet.clear_promises = true;
                panel.apply_configuration(at, &packet);
            });
        }
        Command::Connect { pos, slot } => {
            let source = PanelPosition { pos, slot };
            if let Some(issue) = connect_issue(host, &at, &panel, &source) {
                host.play_sound("create:deny", util::centre(at.pos), 2, 1.0, 1.0);
                status(host, uuid, issue.message());
                return;
            }
            let mut added = Added::No;
            update_panel(host, &at, |panel| {
                added = panel.add_connection(at, source, ConnectionSource::Panel, restocker);
            });
            if let Added::Panel { source_targets } = added {
                update_panel(host, &source, |from| {
                    if !from.targeting.contains(&source_targets) {
                        from.targeting.push(source_targets);
                    }
                });
                let name = |p: &Gauge<ItemStack>| p.filter.as_ref().map_or("?".to_owned(), |k| display_name(&k.item));
                let from = panel_at(host, &source).map(|(p, _)| name(&p)).unwrap_or_default();
                message = Some(format!("Now using {from} to create {}", name(&panel)));
            } else {
                message = Some("Nothing was connected".to_owned());
            }
        }
        Command::ConnectLink { pos } => match connect_link(host, &at, restocker, pos) {
            Ok(text) => message = Some(text),
            Err(text) => {
                host.play_sound("create:deny", util::centre(at.pos), 2, 1.0, 1.0);
                status(host, uuid, text);
                return;
            }
        },
        Command::Unlink => {
            let mut links = Vec::new();
            update_panel(host, &at, |panel| links = panel.disconnect_all_links());
            for link in &links {
                unlink(host, *link, at);
            }
            message = Some(if links.is_empty() { "Nothing is linked to this panel".to_owned() } else { "Links removed".to_owned() });
        }
        Command::Input { pos, slot, amount } => {
            let source = PanelPosition { pos, slot };
            ok = panel.targeted_by.iter().any(|c| c.from == source);
            update_panel(host, &at, |panel| {
                let mut packet = unchanged(panel);
                packet.input_amounts = vec![(source, amount)];
                panel.apply_configuration(at, &packet);
            });
            if !ok {
                message = Some("That panel is not an input".to_owned());
            }
        }
        Command::Disconnect { pos, slot } => {
            let source = PanelPosition { pos, slot };
            ok = panel.targeted_by.iter().any(|c| c.from == source);
            let mut effect = None;
            update_panel(host, &at, |panel| {
                let mut packet = unchanged(panel);
                packet.remove_connection = Some(source);
                effect = Some(panel.apply_configuration(at, &packet));
            });
            if let Some(forgets) = effect.and_then(|e| e.source_forgets) {
                update_panel(host, &forgets, |from| from.targeting.retain(|p| *p != at));
            }
            if !ok {
                message = Some("That panel is not an input".to_owned());
            }
        }
        Command::Move { pos, slot } => {
            let to = PanelPosition { pos, slot };
            match move_panel(host, &at, &panel, to) {
                Ok(text) => {
                    lock(&OPEN).insert(uuid, to);
                    message = Some(text);
                }
                Err(text) => {
                    host.play_sound("create:deny", util::centre(at.pos), 2, 1.0, 1.0);
                    status(host, uuid, text);
                    return;
                }
            }
        }
        Command::Bend => {
            // `onShortInteract` with a wrench: one mode for every connection leaving this panel.
            let mut mode = -1;
            for target in &panel.targeting {
                if let Some((other, _)) = panel_at(host, target)
                    && let Some(connection) = other.targeted_by.iter().find(|c| c.from == at)
                {
                    mode = rules::cycle_arrow_bend(connection.arrow_bend_mode);
                    break;
                }
            }
            // The links attached to this panel take the same mode (`targetedByLinks`).
            let links_mode = panel.targeted_by_links.first().map(|c| rules::cycle_arrow_bend(c.arrow_bend_mode));
            if mode == -1
                && let Some(first) = links_mode
            {
                mode = first;
            }
            if mode != -1 && !panel.targeted_by_links.is_empty() {
                update_panel(host, &at, |own| {
                    for connection in &mut own.targeted_by_links {
                        connection.arrow_bend_mode = mode;
                    }
                });
            }
            if mode == -1 {
                message = Some("Nothing is connected to this panel's output".to_owned());
            } else {
                for target in &panel.targeting {
                    update_panel(host, target, |other| {
                        for connection in other.targeted_by.iter_mut().filter(|c| c.from == at) {
                            connection.arrow_bend_mode = mode;
                        }
                    });
                }
                message = Some(format!("Arrow path mode {mode}"));
            }
        }
        Command::Reset => {
            let mut effect = None;
            update_panel(host, &at, |panel| {
                let mut packet = unchanged(panel);
                packet.reset = true;
                effect = Some(panel.apply_configuration(at, &packet));
            });
            if let Some(gone) = effect.and_then(|e| e.disconnected) {
                apply_disconnected(host, &gone);
            }
        }
    }
    match message {
        Some(text) if !ok => tell(host, uuid, text),
        Some(text) => tell(host, uuid, text),
        None => {
            if let Some((panel, restocker)) = panel_at(host, &at) {
                tell(host, uuid, describe(&panel, restocker));
            }
        }
    }
}

/// Which panel a click on the block means (`getTargetedSlot`): the panel the click landed nearest to, if it is there; else the one
/// the player has open here, else the first active one.
fn clicked_panel(host: &mut dyn Host, uuid: u128, pos: Pos) -> Option<PanelPosition> {
    if let Some(player) = host.acting_player().filter(|p| p.uuid == uuid)
        && let Some(slot) = hit_slot(&*host, &player, pos)
    {
        let at = PanelPosition { pos, slot };
        if panel_at(host, &at).is_some() {
            return Some(at);
        }
    }
    if let Some(at) = lock(&OPEN).get(&uuid).copied()
        && at.pos == pos
        && panel_at(host, &at).is_some()
    {
        return Some(at);
    }
    let entity = host.mod_block_entity(pos)?;
    let panels = read_panels(&entity.nbt);
    panels.iter().find(|p| p.active).map(|p| PanelPosition { pos, slot: p.slot })
}

fn open(host: &mut dyn Host, uuid: u128, pos: Pos) {
    let Some(at) = clicked_panel(host, uuid, pos) else {
        return;
    };
    let Some((panel, restocker)) = panel_at(host, &at) else {
        return;
    };
    if !network::may_interact(host, panel.network, uuid) {
        return;
    }
    lock(&OPEN).insert(uuid, at);
    tell(host, uuid, describe(&panel, restocker));
    tell(host, uuid, "/gauge slot, item, amount, address, output, promises, clearpromises, connect, unlink, input, disconnect, bend, reset");
}

/// `FactoryPanelBlock.useItemOn`: a gauge item used on the block adds a panel at the slot the click is nearest to, on the network the
/// item carries (it must be tuned).
fn add_panel(host: &mut dyn Host, uuid: u128, pos: Pos, creative: bool, stack: &ItemStack) {
    let Some(freq) = network::stack_freq(stack) else {
        host.play_sound("create:deny", util::centre(pos), 2, 1.0, 1.0);
        status(host, uuid, "Tune to a stock link before placing");
        return;
    };
    let Some(entity) = host.mod_block_entity(pos) else {
        return;
    };
    let panels = read_panels(&entity.nbt);
    let active = [panels[0].active, panels[1].active, panels[2].active, panels[3].active];
    // `canBeReplaced`/`addPanel`: the clicked slot has to be free; where the click can't be read, the first free one.
    let clicked = host.acting_player().and_then(|player| hit_slot(&*host, &player, pos));
    let Some(slot) = clicked.filter(|slot| !active[index(*slot)]).or_else(|| rules::next_free_slot(active)) else {
        return;
    };
    let at = PanelPosition { pos, slot };
    host.change_mod_block_entity(pos, &mut |entity| {
        let mut panels = read_panels(&entity.nbt);
        panels[index(slot)].enable();
        panels[index(slot)].network = freq;
        let restocker = util::boolean(&entity.nbt, RESTOCKER);
        store(&mut entity.nbt, &panels, restocker);
    });
    host.block_entity_sync(pos);
    host.play_sound("minecraft:block.copper.place", util::centre(pos), 2, 1.0, 0.8);
    lock(&OPEN).insert(uuid, at);
    status(host, uuid, "Connected to the network");
    if !creative {
        host.take_items(ID, 1);
    }
}

/// `FactoryPanelBlockEntity.removePanel`: the open panel comes off. Whether the block still has panels.
fn remove_panel(host: &mut dyn Host, uuid: u128, pos: Pos, creative: bool) -> bool {
    let Some(at) = clicked_panel(host, uuid, pos) else {
        return false;
    };
    let mut gone = None;
    let mut left = 0;
    let mut found = false;
    host.change_mod_block_entity(pos, &mut |entity| {
        let mut panels = read_panels(&entity.nbt);
        let panel = &mut panels[index(at.slot)];
        if !panel.active {
            return;
        }
        gone = Some(panel.disconnect_all(at));
        panel.disable();
        left = active_count(&panels);
        let restocker = util::boolean(&entity.nbt, RESTOCKER);
        store(&mut entity.nbt, &panels, restocker);
        found = true;
    });
    if !found {
        return false;
    }
    if let Some(gone) = gone {
        apply_disconnected(host, &gone);
    }
    lock(&OPEN).remove(&uuid);
    host.block_entity_sync(pos);
    if !creative {
        host.give_stack(uuid, gauge_item());
    }
    host.play_sound("minecraft:block.copper.break", util::centre(pos), 2, 1.0, 0.8);
    if left == 0 {
        host.destroy_block(pos, None, false);
    }
    left > 0
}

// --- arrivals -------------------------------------------------------------------------------------------------

/// `PackagerBlockEntity.submitNewArrivals`' gauge part: the arrivals are told to every active panel's own promises of a restocker
/// on the Packager.
pub fn restock_arrivals(host: &mut dyn Host, pos: Pos, arrivals: &[Big<ItemStack>]) {
    let Some(entity) = host.mod_block_entity(pos) else {
        return;
    };
    if !util::boolean(&entity.nbt, RESTOCKER) {
        return;
    }
    let mut panels = read_panels(&entity.nbt);
    let mut changed = false;
    for panel in panels.iter_mut().filter(|p| p.active) {
        for (key, count) in arrivals {
            panel.restocker_promises.item_entered_system(key, *count);
        }
        changed |= panel.restocker_promises.take_changed();
    }
    if changed {
        host.change_mod_block_entity(pos, &mut |entity| {
            store(&mut entity.nbt, &panels, true);
        });
    }
}

// --- the hooks -----------------------------------------------------------------------------------------------------

/// Whether the point is inside a block's collision boxes (`level.noCollision(AABB(v, v).inflate(1/128))`).
fn point_blocked(host: &dyn Host, point: Vec3) -> bool {
    let cell = [point[0].floor() as i32, point[1].floor() as i32, point[2].floor() as i32];
    let local = [point[0] - f64::from(cell[0]), point[1] - f64::from(cell[1]), point[2] - f64::from(cell[2])];
    let pad = 1.0 / 128.0;
    host.block_collision(cell)
        .iter()
        .any(|b| (0..3).all(|i| local[i] + pad > b[i] && local[i] - pad < b[i + 3]))
}

/// `FactoryPanelRenderer.renderPath`: the arrows into every active panel.
fn draw_arrows(host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
    let Some((face, facing)) = host
        .block_state_property(event.state, "face")
        .and_then(AttachFace::from_name)
        .zip(host.block_state_property(event.state, "facing").and_then(Dir::from_name))
    else {
        return;
    };
    let panels = read_panels(&event.entity.nbt);
    let restocker = util::boolean(&event.entity.nbt, RESTOCKER);
    let (x_rot, y_rot) = (geometry::x_rot(face), geometry::y_rot(face, facing));
    let pos = event.pos;
    let draw = &mut *event.draw;
    const UP: [f32; 3] = [0.0, 1.0, 0.0];
    const EAST: [f32; 3] = [1.0, 0.0, 0.0];
    const CENTRE: [f32; 3] = [0.5; 3];
    let mut any = false;
    for panel in panels.iter().filter(|p| p.active) {
        let to = panel.position(pos);
        let colour = geometry::arrow_color(
            panel.count,
            panel.is_missing_address(restocker),
            panel.redstone_powered,
            panel.waiting_for_network,
            panel.satisfied,
            panel.promised_satisfied,
        );
        let height = geometry::arrow_height(panel.satisfied, panel.promised_satisfied);
        // `targetedBy` then `targetedByLinks`: the arrows to attached Redstone Links and Display Links run the same way.
        for connection in panel.targeted_by.iter().chain(&panel.targeted_by_links) {
            let key = (pos, panel.slot, connection.from, connection.arrow_bend_mode);
            let cached = lock(&PATHS).get(&key).cloned();
            let path = match cached {
                Some(path) => path,
                None => {
                    let diff = geometry::path_diff(face, facing, connection.from, to);
                    let centre = geometry::center_of_slot(face, facing, panel.slot);
                    let start = [centre[0] + f64::from(pos[0]), centre[1] + f64::from(pos[1]), centre[2] + f64::from(pos[2])];
                    let path = geometry::connection_path(diff, connection.arrow_bend_mode, start, face, facing, &|p| point_blocked(host, p));
                    lock(&PATHS).insert(key, path.clone());
                    path
                }
            };
            for i in 0..path.len() {
                let (offset, model) = geometry::segment(panel.slot, &path, i, height);
                draw.push();
                draw.rotate_about(UP, y_rot as f32, CENTRE);
                draw.rotate_about(EAST, (x_rot + 90.0) as f32, CENTRE);
                draw.rotate_about(UP, 180.0, CENTRE);
                draw.translate([offset[0] as f32, offset[1] as f32, offset[2] as f32]);
                host.draw_model(draw, &connection_model(if i == 0 { CONNECTION_MODELS[0] } else { CONNECTION_MODELS[1] }, model), colour);
                draw.pop();
                any = true;
            }
        }
    }
    if any {
        draw.still();
    }
}

impl Hooks for Gauges {
    /// `AllPartialModels.FACTORY_PANEL_ARROWS` and `_LINES`.
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        for kind in &CONNECTION_MODELS[..2] {
            for dir in [Dir::North, Dir::East, Dir::South, Dir::West] {
                event.models.push(connection_model(kind, dir));
            }
        }
    }

    /// The panels' state reaches the client, which draws the arrows from it.
    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        (event.entity.id == ID).then(|| event.entity.nbt.clone())
    }

    /// A connection is at most 16 blocks long (`CONNECTION_RANGE`).
    fn block_entity_render_bounding_box(&self, _: &dyn Host, event: &mut BlockEntityRenderBoundingBoxEvent<'_>) {
        if event.block == ID {
            *event.bounds = [-16.0, -16.0, -16.0, 17.0, 17.0, 17.0];
        }
    }

    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block == ID {
            draw_arrows(host, event);
        }
    }

    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        (event.block == ID).then(|| (event.block.to_owned(), Compound::new()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    /// `FactoryPanelBlock.getStateForPlacement` (the part that needs no neighbours): the face it is on and the way it faces.
    fn state_for_placement(&self, level: &dyn LevelHost, event: &mut StateForPlacementEvent<'_>) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != ID {
            return None;
        }
        let look = create_kinetics::rules::packager::nearest_looking(context.yaw, context.pitch)[0];
        let (face, facing) = rules::placement(look, create_kinetics::rules::deployer::horizontal_direction(context.yaw));
        let state = level.state_with_property(event.state, "face", face.name())?;
        Some(level.state_with_property(state, "facing", facing.name()))
    }

    /// The first panel, on the network the player last tuned (`setPlacedBy`: `addPanel(initialSlot, networkFromStack)`).
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if &*event.stack.item != ID {
            return;
        }
        let dimension = host.dimension().to_owned();
        let freq = network::stack_freq(event.stack).unwrap_or_else(random_freq);
        let slot = host.acting_player().and_then(|player| hit_slot(&*host, &player, event.pos));
        lock(&PLACED).insert((dimension, event.pos), (freq, slot));
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        let pos = event.pos;
        let now = util::game_time(&*host);
        let dimension = host.dimension().to_owned();
        let mut panels = read_panels(&event.entity.nbt);
        let mut restocker = util::boolean(&event.entity.nbt, RESTOCKER);
        let mut dirty = false;
        if active_count(&panels) == 0
            && let Some((freq, clicked)) = lock(&PLACED).remove(&(dimension, pos))
        {
            let slot = clicked.or_else(|| rules::next_free_slot([false; 4])).unwrap_or(Slot::BottomLeft);
            panels[index(slot)].enable();
            panels[index(slot)].network = freq;
            dirty = true;
        }
        // `FactoryPanelBlockEntity.lazyTick`: an empty block goes, and the Packager behind decides the restocking.
        if (now + create_kinetics::rules::stock_link::lazy_salt(pos, 40)) % 40 == 0 {
            if active_count(&panels) == 0 {
                host.destroy_block(pos, None, false);
                return;
            }
            let should = restocked_packager(&*host, pos).is_some();
            if should != restocker {
                restocker = should;
                dirty = true;
            }
            // `lazyTick`: `checkForRedstoneInput`.
            dirty |= check_redstone_input(host, &mut panels);
        }
        for i in 0..panels.len() {
            if panels[i].active {
                dirty |= tick_panel(host, pos, &mut panels, i, restocker);
            }
        }
        let changed = store(&mut event.entity.nbt, &panels, restocker);
        if dirty && changed {
            host.block_entity_sync(pos);
        }
    }

    /// Breaking the block takes the open panel first while there are two or more (`tryDestroySubPanelFirst`); the last one takes the
    /// block, its connections, and any spare gauges (`destroy`).
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(entity) = host.mod_block_entity(pos) else {
            return Flow::Continue;
        };
        let panels = read_panels(&entity.nbt);
        let count = active_count(&panels);
        if count >= 2
            && let Some(player) = host.acting_player()
        {
            remove_panel(host, player.uuid, pos, player.game_mode == util::CREATIVE);
            return Flow::Cancel;
        }
        for panel in panels.iter().filter(|p| p.active) {
            let at = panel.position(pos);
            let mut copy = panel.clone();
            let gone = copy.disconnect_all(at);
            apply_disconnected(host, &gone);
        }
        if count > 1 {
            let mut spare = gauge_item();
            spare.count = (count - 1) as u32;
            util::spawn_item(host, util::centre(pos), &spare, [0.0; 3], 10);
        }
        Flow::Continue
    }

    /// An empty hand opens the panel.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.sneaking || player.game_mode == util::SPECTATOR || host.held_stack(player.uuid).is_some() {
            return Flow::Continue;
        }
        if !util::first_click(&*host, player.uuid, pos) {
            return Flow::Cancel;
        }
        open(host, player.uuid, pos);
        Flow::Cancel
    }

    /// `FactoryPanelBlock.useItemOn` and `FactoryPanelBehaviour.onShortInteract`: a gauge adds a panel, a linked item is tuned to the
    /// panel's network, the Wrench takes the panel off or bends the arrows, another item sets the monitored item of an empty panel.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        // `FactoryPanelBlockItem.place`: an untuned gauge can't be placed anywhere; on a linked block the click tunes it instead.
        if &*event.stack.item == ID
            && item_rules::gauge_refused(&event.stack.item, network::is_tuned(event.stack))
            && !network::is_linked_block(&host.block(pos))
        {
            if let Some(player) = host.acting_player()
                && player.game_mode != util::SPECTATOR
            {
                host.play_sound("create:deny", util::centre(pos), 2, 1.0, 1.0);
                status(host, player.uuid, "Tune to a stock link before placing");
                return Flow::Cancel;
            }
            return Flow::Continue;
        }
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        let item = event.stack.item.to_string();
        if item == util::WRENCH {
            if player.sneaking {
                if util::first_click(&*host, player.uuid, pos) {
                    remove_panel(host, player.uuid, pos, player.game_mode == util::CREATIVE);
                }
                return Flow::Cancel;
            }
            if let Some(at) = clicked_panel(host, player.uuid, pos) {
                lock(&OPEN).insert(player.uuid, at);
                run(host, player.uuid, "bend");
            }
            return Flow::Cancel;
        }
        if player.sneaking {
            return Flow::Continue;
        }
        if !util::first_click(&*host, player.uuid, pos) {
            return Flow::Cancel;
        }
        if item == ID {
            add_panel(host, player.uuid, pos, player.game_mode == util::CREATIVE, event.stack);
            return Flow::Cancel;
        }
        let Some(at) = clicked_panel(host, player.uuid, pos) else {
            return Flow::Cancel;
        };
        let Some((panel, _)) = panel_at(host, &at) else {
            return Flow::Cancel;
        };
        if !network::may_interact(host, panel.network, player.uuid) {
            return Flow::Cancel;
        }
        if network::is_linked_item(&item) {
            // "Bind logistics items to this panels' frequency".
            network::tune(player.uuid, panel.network);
            status(host, player.uuid, "Tuned to this network");
            return Flow::Cancel;
        }
        if panel.filter.is_none() && !is_filter(&item) {
            lock(&OPEN).insert(player.uuid, at);
            update_panel(host, &at, |panel| {
                panel.set_filter(Some(items::key(event.stack)), false);
            });
        }
        open(host, player.uuid, pos);
        Flow::Cancel
    }

    fn register_commands(&self, _: &mut dyn Host, event: &mut RegisterCommandsEvent<'_>) {
        event.commands.push(NativeCommand::new("gauge"));
        event.commands.push(NativeCommand::new("gauge <text:text>"));
    }

    fn perform_command(&self, host: &mut dyn Host, event: &mut PerformCommandEvent<'_>) -> Flow {
        let line = event.command.trim();
        let Some(rest) = (if line == "gauge" { Some("") } else { line.strip_prefix("gauge ") }) else {
            return Flow::Continue;
        };
        run(host, event.player.uuid, rest);
        Flow::Cancel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(item: &str) -> ItemStack {
        key_of(item)
    }

    #[test]
    fn a_panel_survives_being_saved_and_read() {
        let mut panel: Gauge<ItemStack> = Gauge::new(Slot::TopRight, 0xabcd_u128 << 70);
        panel.enable();
        panel.filter = Some(stack("minecraft:iron_ingot"));
        panel.count = 12;
        panel.up_to = false;
        panel.recipe_address = "Press".into();
        panel.recipe_output = 2;
        panel.promise_clearing_interval = 5;
        panel.timer = 37;
        panel.satisfied = true;
        let from = PanelPosition { pos: [1, 64, -3], slot: Slot::BottomLeft };
        panel.targeted_by.push(Connection { from, amount: 3, arrow_bend_mode: 2 });
        panel.targeting.push(PanelPosition { pos: [2, 64, -3], slot: Slot::TopLeft });
        panel.restocker_promises.add(Promise { ticks_existed: 9, stack: (stack("minecraft:iron_ingot"), 5) });
        let mut nbt = Compound::new();
        assert!(store(&mut nbt, &[panel.clone()], true));
        let back = read_panel(&nbt, Slot::TopRight);
        assert!(back.active);
        assert_eq!(back.filter, panel.filter);
        assert_eq!((back.count, back.up_to, back.timer), (12, false, 37));
        assert_eq!(back.network, panel.network);
        assert_eq!(back.recipe_address, "Press");
        assert_eq!((back.recipe_output, back.promise_clearing_interval), (2, 5));
        assert_eq!(back.targeted_by, panel.targeted_by);
        assert_eq!(back.targeting, panel.targeting);
        assert_eq!(back.restocker_promises.flatten(false), panel.restocker_promises.flatten(false));
        assert!(util::boolean(&nbt, RESTOCKER));
        // A panel that is not active is not saved, and reads as inactive.
        assert!(!read_panel(&nbt, Slot::TopLeft).active);
    }

    #[test]
    fn an_emptied_panel_leaves_the_saved_form() {
        let mut panel: Gauge<ItemStack> = Gauge::new(Slot::BottomLeft, 1);
        panel.enable();
        let mut nbt = Compound::new();
        store(&mut nbt, &[panel.clone()], false);
        assert!(util::has(&nbt, "bottom_left"));
        panel.disable();
        store(&mut nbt, &[panel], false);
        assert!(!util::has(&nbt, "bottom_left"));
    }
}
