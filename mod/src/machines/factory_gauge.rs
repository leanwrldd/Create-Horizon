//! The Factory Gauge (`create:factory_gauge`) (`FactoryPanelBlock`, `FactoryPanelBlockEntity`, `FactoryPanelBehaviour`,
//! `FactoryPanelConnectionHandler`, `FactoryPanelConfigurationPacket`, `AllShapes.FACTORY_PANEL_FALLBACK`): up to four panels on one
//! face of a block, each watching how much of one item a logistics network holds and asking for more. The rules are
//! `rules::factory_gauge` (the storage monitor, the requests, the restocker, the connections) and `rules::request_promises`; the
//! network is `logistics.rs`; this is the block, the saved form and the commands that stand in for the gauge's screen.
//!
//! How it works here:
//! - Place a Factory Gauge on a block: its first panel starts a network of its own, or joins the one you last tuned to (use a Stock
//!   Link, Stock Ticker or Redstone Requester on a linked block to tune; use a linked item on a gauge to tune to its network). Use
//!   another gauge item on the block to add a panel (it must be tuned first, as in Create): up to four, bottom row first.
//! - Right-click a panel with an empty hand to open it, or with an item (not a gauge or a linked one) to set the item it watches.
//!   `/gauge` then shows it, and `/gauge slot`, `item`, `amount <0-100> [items|stacks]`, `address <text>`, `output <n>`,
//!   `promises <never|30s|minutes>`, `clearpromises`, `connect <x> <y> <z> <slot>`, `input ...`, `disconnect ...`, `bend` and `reset` change
//!   it (the screen and the value box). A panel with inputs is a crafting recipe: when it is short of its item it asks the network for the
//!   inputs, addressed to `address`. A panel on a Packager is a restocker: it orders its item for that Packager's inventory.
//! - Sneak-use the Wrench to take the open panel off (the block goes with the last one); breaking the block takes the open panel first
//!   while there are two or more, as in Create.
//!
//! Known differences from Create:
//! - Which of the four panels a click means is the player's open panel (`/gauge slot`), not where the click lands, and a new panel takes
//!   the first free slot. The screen, the value box, the arrows (`FactoryPanelConnection.getPath`), the bulb, the relocation of a panel
//!   (`moveTo`) and the connection to Redstone Links and Display Links (`FactoryPanelSupportBehaviour`) are not ported; so the block
//!   never powers neighbours and `activeCraftingArrangement` (Mechanical Crafting) can't be set.
//! - A frequency isn't carried by the item: a placed gauge takes the network the player last tuned (see `logistics.rs`).
//! - The panels' effect packets (the sparks along the arrows) aren't sent.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

use create_kinetics::rules::factory_gauge::{
    self as rules, Added, Configuration, Connection, ConnectionSource, Disconnected, Gauge, PanelPair, PanelPosition, Reads, RequestEnv,
    Slot, Source,
};
use create_kinetics::rules::filter::display_name;
use create_kinetics::rules::logistics_network::{Freq, Network, Networks};
use create_kinetics::rules::package::Big;
use create_kinetics::rules::request_promises::{Promise, Queue};
use create_kinetics::rules::shop_commands::{self as commands, Gauge as Command, ItemArg, slot_name};
use create_kinetics::{Dir, Pos};
use crate::compat::{
    BlockEntityEvent, BlockEntityTickEvent, BlockEntityTicksEvent, BlockStateId, Compound, DestroyBlockEvent, Flow, Hooks, Host,
    ItemStack, LevelHost, NativeCommand, PerformCommandEvent, PlaceBlockEvent, RegisterCommandsEvent, RightClickBlockEvent,
    StateForPlacementEvent, Tag, Text, UseItemOnEvent,
};

use super::logistics as network;
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
/// A gauge just placed, until its block entity ticks: the network its first panel starts with.
static PLACED: LazyLock<Mutex<BTreeMap<(String, Pos), Freq>>> = LazyLock::new(Mutex::default);

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

fn pos_tag(at: &PanelPosition) -> Tag {
    let mut tag = Compound::new();
    tag.insert("pos".into(), Tag::IntArray(at.pos.to_vec()));
    tag.insert("slot".into(), Tag::String(slot_name(at.slot).into()));
    Tag::Compound(tag)
}

fn pos_of(tag: &Tag) -> Option<PanelPosition> {
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
fn panel_at(host: &mut dyn Host, at: &PanelPosition) -> Option<(Gauge<ItemStack>, bool)> {
    if host.block(at.pos) != ID {
        return None;
    }
    let entity = host.mod_block_entity(at.pos)?;
    let panel = read_panel(&entity.nbt, at.slot);
    panel.active.then(|| (panel, util::boolean(&entity.nbt, RESTOCKER)))
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
    for source in &gone.sources_stop_targeting {
        update_panel(host, source, |panel| panel.targeting.retain(|p| *p != gone.here));
    }
    for target in &gone.targets_stop_being_targeted_by {
        update_panel(host, target, |panel| panel.targeted_by.retain(|c| c.from != gone.here));
    }
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
                message = Some(format!("Arrow path mode {mode} (the arrows are not drawn here)"));
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

/// Which panel a click on the block means: the one the player has open here, else the first active one.
fn clicked_panel(host: &mut dyn Host, uuid: u128, pos: Pos) -> Option<PanelPosition> {
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
    tell(host, uuid, "/gauge slot, item, amount, address, output, promises, clearpromises, connect, input, disconnect, bend, reset");
}

/// `FactoryPanelBlockEntity.addPanel` for a gauge item used on the block: the next free slot, on the player's tuned network.
fn add_panel(host: &mut dyn Host, uuid: u128, pos: Pos, creative: bool) {
    let Some(freq) = network::tuned_peek(uuid) else {
        host.play_sound("create:deny", util::centre(pos), 2, 1.0, 1.0);
        status(host, uuid, "Tune to a stock link before placing");
        return;
    };
    let Some(entity) = host.mod_block_entity(pos) else {
        return;
    };
    let panels = read_panels(&entity.nbt);
    let active = [panels[0].active, panels[1].active, panels[2].active, panels[3].active];
    let Some(slot) = rules::next_free_slot(active) else {
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

impl Hooks for Gauges {
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
        let freq = host.acting_player().and_then(|player| network::tuned_peek(player.uuid)).unwrap_or_else(random_freq);
        lock(&PLACED).insert((dimension, event.pos), freq);
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
            && let Some(freq) = lock(&PLACED).remove(&(dimension, pos))
        {
            let slot = rules::next_free_slot([false; 4]).unwrap_or(Slot::BottomLeft);
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
            add_panel(host, player.uuid, pos, player.game_mode == util::CREATIVE);
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
            status(host, player.uuid, "Tuned to this network: the next linked block you place joins it");
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
