//! Mail between trains and Postboxes (`GlobalStation.runMailTransfer`, `GlobalPackagePort`, `StationBlockEntity.attachPackagePort`,
//! `PackagePortTarget.TrainStationFrogportTarget`, `PackagePortTargetSelectionHandler`): a Postbox linked to a Track Station lets a
//! train standing there collect the packages in the box that aren't for it and leave the ones that are. The decisions are
//! `create_kinetics::rules::trains::mail`; this reads the carriages' storage (`train_cargo`'s: chests, barrels, shulker boxes,
//! vaults, Depot, Toolbox, Creative Crate) and the Postboxes, and writes back what moved.
//!
//! - **Linking**: right-click a Track Station with a Postbox in hand (`PackagePortTargetSelectionHandler.onUse`) to pick it; the next
//!   Postbox placed within five blocks of it is linked (`PackagePortItem` and `PackagePortPlacementPacket`): its `Target` is
//!   `{type: "create:train_station", relative_pos}` as Create saves it. Only a Postbox links to a station (`canSupport`); a Frogport
//!   links to a chain (`frogport.rs`).
//! - **Registering**: a linked Postbox registers with its station every ten ticks (`PackagePortBlockEntity.lazyTick`): the station
//!   gets a `connectedPorts` record with the box's address (the address a Name Tag sets, taken again at once:
//!   `filterChanged`). The station's records, and the offline buffers that hold what a train left in a Postbox while its chunk
//!   wasn't loaded, are saved in the station's own data as `Ports` (Create saves them with the graph).
//! - **Transfer**: `TrainEvent::MailTransfer` (a train arriving, and every 20 ticks it stands at the station) runs
//!   [`transfer`]. Packages come out of a Postbox into the carriages' storage and the carried ones go into the first Postbox
//!   whose address matches. A loaded box is its block's slots; one that isn't is the offline buffer, which the box takes
//!   when it loads (`restoreOfflineBuffer`).
//! - **Waiting packages** ([`waiting`]): what `Fetch Packages` reads (`port_packages`), refreshed every 20 ticks and after a transfer.
//!
//! Differences from Create, for the README:
//! - No target selection outline or ghost: the pick is a message on the action bar, and a Wrench doesn't show the link.
//! - `box.spawnParticles()` is a burst of `happy_villager` particles (bone meal's), sent as a command.
//! - The ComputerCraft events (`package_sent`, `package_received`) aren't there.
//! - A Postbox's address is saved in the record when it registers or is renamed (a Name Tag), as `attachPackagePort` and
//!   `filterChanged` do; the schedule reads that.
//!
//! Type-checked and unit-tested only (in the rules): not played.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

use create_kinetics::rules::portable_interface::Stack;
use create_kinetics::rules::trains::graph::{EdgePointType, PointData, Uuid};
use create_kinetics::rules::trains::mail::{self as rules, PORT_SLOTS, Port};
use create_kinetics::rules::trains::station::PackagePort;
use create_kinetics::rules::trains::train_cargo as cargo;
use create_kinetics::Pos;

use crate::compat::{Compound, Flow, Hooks, Host, ItemStack, LevelTickPostEvent, ModBlockEntity, PlaceBlockEvent, Tag, Text, UseItemOnEvent};

use super::contraption::Moving;
use super::package as items;
use super::portable_interface_storage::{Items, apply_items_with, items_of_with, key_of};
use super::postbox::is_postbox;
use super::track_station::STATION;
use super::train_net as net;
use crate::util;

struct Mail;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Mail)
}

const TARGET: &str = "Target";
const TYPE: &str = "type";
const RELATIVE_POS: &str = "relative_pos";
/// `AllPackagePortTargetTypes.TRAIN_STATION`'s id.
const TRAIN_STATION: &str = "create:train_station";
/// The station's saved ports (`GlobalStation.write`: `Ports`).
const PORTS: &str = "Ports";
const ADDRESS: &str = "Address";
const OFFLINE_BUFFER: &str = "OfflineBuffer";
const PRIMED: &str = "Primed";
const POS: &str = "Pos";
/// `PackagePortBlockEntity.lazyTick`: ten ticks.
const LAZY: i64 = 10;
/// How often the stations are looked at.
const SWEEP: i64 = 20;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

// --- memory -------------------------------------------------------------------------------------------------------------------

type PortKey = (String, Pos);

/// `GlobalPackagePort.offlineBuffer`: each registered port's last known slots (a loaded Postbox keeps it in step: `saveOfflineBuffer`).
static BUFFERS: LazyLock<Mutex<HashMap<PortKey, Vec<Option<ItemStack>>>>> = LazyLock::new(Mutex::default);
/// The addresses of the packages in each port, for `Fetch Packages` (`port_packages`): by dimension index and position.
static WAITING: LazyLock<Mutex<HashMap<(u32, Pos), Vec<String>>>> = LazyLock::new(Mutex::default);
/// The stations whose saved `Ports` were read into the graph (dimension, station id).
static LOADED: LazyLock<Mutex<HashSet<(String, Uuid)>>> = LazyLock::new(Mutex::default);
/// The station each player picked with a Postbox in hand.
static SELECTED: LazyLock<Mutex<HashMap<u128, (String, Pos)>>> = LazyLock::new(Mutex::default);

/// The addresses of the packages waiting in the port at `port` (`FetchPackagesInstruction.start` walks them slot by slot).
pub fn waiting(port: Pos, dimension: u32) -> Vec<String> {
    lock(&WAITING).get(&(dimension, port)).cloned().unwrap_or_default()
}

// --- the Postbox's target --------------------------------------------------------------------------------------------------------

fn target_of(nbt: &Compound) -> Option<Pos> {
    let Some(Tag::Compound(target)) = util::get(nbt, TARGET) else { return None };
    match util::get(target, TYPE) {
        Some(Tag::String(t)) if t.to_string_lossy() == TRAIN_STATION => {}
        _ => return None,
    }
    match util::get(target, RELATIVE_POS) {
        Some(Tag::IntArray(v)) if v.len() == 3 => Some([v[0], v[1], v[2]]),
        _ => None,
    }
}

fn target_tag(relative: Pos) -> Tag {
    let mut c = Compound::new();
    c.insert(TYPE.into(), Tag::String(TRAIN_STATION.into()));
    c.insert(RELATIVE_POS.into(), Tag::IntArray(relative.to_vec()));
    Tag::Compound(c)
}

fn address_of_box(nbt: &Compound) -> String {
    match util::get(nbt, "AddressFilter") {
        Some(Tag::String(text)) => text.to_string_lossy(),
        _ => String::new(),
    }
}

fn add(a: Pos, b: Pos) -> Pos {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

// --- the stations -------------------------------------------------------------------------------------------------------------------

/// A station block's edge point id, from its saved data.
fn station_id(host: &dyn Host, pos: Pos) -> Option<Uuid> {
    let entity = host.mod_block_entity(pos)?;
    util::has(&entity.nbt, "Id").then(|| net::load(&entity.nbt, EdgePointType::Station, pos).id)
}

/// Runs `f` on the station block at `pos`'s `GlobalStation` (`connectedPorts`).
fn with_station<R>(host: &dyn Host, pos: Pos, f: impl FnOnce(&mut create_kinetics::rules::trains::station::GlobalStation) -> R) -> Option<R> {
    let id = station_id(host, pos)?;
    net::with_point(host, EdgePointType::Station, id, |p| p.station_mut().map(f)).flatten()
}

/// `StationBlockEntity.write`'s ports: the station's records, with the buffer of each primed one.
fn save_ports(host: &mut dyn Host, station: Pos, ports: &BTreeMap<Pos, PackagePort>) {
    let dimension = host.dimension().to_owned();
    let list: Vec<Tag> = ports
        .iter()
        .map(|(pos, port)| {
            let mut c = Compound::new();
            c.insert(ADDRESS.into(), Tag::String(port.address.as_str().into()));
            c.insert(PRIMED.into(), Tag::Byte(i8::from(port.primed)));
            c.insert(POS.into(), Tag::IntArray(pos.to_vec()));
            if port.primed {
                let buffer = lock(&BUFFERS).get(&(dimension.clone(), *pos)).cloned().unwrap_or_default();
                let slots: Vec<Tag> = buffer
                    .iter()
                    .enumerate()
                    .filter_map(|(i, s)| s.as_ref().map(|s| (i, s)))
                    .filter_map(|(i, s)| match util::stack_tag(s) {
                        Tag::Compound(mut t) => {
                            t.insert("Slot".into(), Tag::Byte(i as i8));
                            Some(Tag::Compound(t))
                        }
                        _ => None,
                    })
                    .collect();
                c.insert(OFFLINE_BUFFER.into(), Tag::List(slots));
            }
            Tag::Compound(c)
        })
        .collect();
    let list = Tag::List(list);
    host.change_mod_block_entity(station, &mut |e| {
        if util::differs(&e.nbt, PORTS, &list) {
            e.nbt.insert(PORTS.into(), list.clone());
        }
    });
}

/// Reads a station's saved `Ports` into the graph's record (`GlobalStation.read`), once.
fn load_ports(host: &mut dyn Host, station: Pos, id: Uuid) {
    let dimension = host.dimension().to_owned();
    if !lock(&LOADED).insert((dimension.clone(), id)) {
        return;
    }
    let Some(entity) = host.mod_block_entity(station) else {
        lock(&LOADED).remove(&(dimension, id));
        return;
    };
    let Some(Tag::List(saved)) = util::get(&entity.nbt, PORTS).cloned() else { return };
    let mut records: Vec<(Pos, PackagePort)> = Vec::new();
    for entry in &saved {
        let Some(Tag::IntArray(p)) = entry.get(POS) else { continue };
        if p.len() != 3 {
            continue;
        }
        let pos = [p[0], p[1], p[2]];
        let address = match entry.get(ADDRESS) {
            Some(Tag::String(a)) => a.to_string_lossy(),
            _ => String::new(),
        };
        let primed = matches!(entry.get(PRIMED), Some(Tag::Byte(b)) if *b != 0);
        if primed && let Some(Tag::List(list)) = entry.get(OFFLINE_BUFFER) {
            let mut buffer: Vec<Option<ItemStack>> = vec![None; PORT_SLOTS];
            for slot in list {
                let index = match slot.get("Slot") {
                    Some(Tag::Byte(b)) => usize::from(*b as u8),
                    _ => continue,
                };
                if let (Some(stack), true) = (util::stack_of(slot), index < PORT_SLOTS) {
                    buffer[index] = Some(stack);
                }
            }
            lock(&BUFFERS).insert((dimension.clone(), pos), buffer);
        }
        records.push((pos, PackagePort { address, primed }));
    }
    // What a Postbox already registered this session is newer than what was saved.
    net::with_point(&*host, EdgePointType::Station, id, |p| {
        if let Some(s) = p.station_mut() {
            for (pos, record) in records {
                s.connected_ports.entry(pos).or_insert(record);
            }
        }
    });
}

// --- a Postbox's side ----------------------------------------------------------------------------------------------------------------

fn slots_of(entity: &ModBlockEntity) -> Vec<Option<ItemStack>> {
    let mut slots = entity.items.clone();
    slots.resize(PORT_SLOTS, None);
    slots
}

fn addresses(slots: &[Option<ItemStack>]) -> Vec<String> {
    slots.iter().flatten().filter(|s| items::is_package(s)).map(items::address).collect()
}

/// `PackagePortBlockEntity.lazyTick` for a Postbox (the box's own tick calls it): registers with the station of its `Target`
/// (`attachPackagePort`: a new record takes the address, a primed one gives its buffer back), and keeps the buffer in step with
/// the slots (`saveOfflineBuffer`). `entity` is the Postbox's own data, which is out of the world while it ticks.
pub fn tick_postbox(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity, now: i64) {
    let Some(relative) = target_of(&entity.nbt) else { return };
    if (now + i64::from(pos[0]) + i64::from(pos[2])).rem_euclid(LAZY) != 0 {
        return;
    }
    let station = add(pos, relative);
    if host.block(station) != STATION {
        return;
    }
    let dimension = host.dimension().to_owned();
    let address = address_of_box(&entity.nbt);
    let key = (dimension.clone(), pos);
    // `restoreOfflineBuffer` first: what a train left while this box wasn't loaded.
    let record = with_station(&*host, station, |s| s.connected_ports.get(&pos).cloned()).flatten();
    let mut changed_record = false;
    match record {
        None => {
            with_station(&*host, station, |s| {
                s.connected_ports.insert(pos, PackagePort { address: address.clone(), primed: false });
            });
            changed_record = true;
        }
        Some(record) if record.primed => {
            let buffer = lock(&BUFFERS).get(&key).cloned().unwrap_or_default();
            entity.items = buffer.iter().cloned().chain(std::iter::repeat(None)).take(PORT_SLOTS).collect();
            with_station(&*host, station, |s| {
                if let Some(p) = s.connected_ports.get_mut(&pos) {
                    p.primed = false;
                }
            });
            host.block_entity_sync(pos);
            changed_record = true;
        }
        Some(_) => {}
    }
    // `saveOfflineBuffer`: the buffer follows the slots.
    let slots = slots_of(entity);
    let mut buffers = lock(&BUFFERS);
    if buffers.get(&key) != Some(&slots) {
        buffers.insert(key, slots.clone());
    }
    drop(buffers);
    lock(&WAITING).insert((super::track::dimension_id(&dimension), pos), addresses(&slots));
    if changed_record
        && let Some(ports) = with_station(&*host, station, |s| s.connected_ports.clone())
    {
        save_ports(host, station, &ports);
    }
}

/// `PackagePortBlockEntity.filterChanged`: the address changed, so the record is made again with the new one.
pub fn filter_changed(host: &mut dyn Host, pos: Pos) {
    let Some(entity) = host.mod_block_entity(pos) else { return };
    let Some(relative) = target_of(&entity.nbt) else { return };
    let station = add(pos, relative);
    let address = address_of_box(&entity.nbt);
    let done = with_station(&*host, station, |s| {
        s.connected_ports.insert(pos, PackagePort { address: address.clone(), primed: false });
        s.connected_ports.clone()
    });
    if let Some(ports) = done {
        save_ports(host, station, &ports);
    }
}

// --- the transfer ------------------------------------------------------------------------------------------------------------------

/// `GlobalStation.runMailTransfer` for the train whose carriages' structures are `carriages` (in carriage order), at the station
/// with edge point `station`. Returns the carriages whose storage changed (their structures were edited).
pub fn transfer(host: &mut dyn Host, station: Uuid, carriages: &mut [&mut Moving]) -> Vec<usize> {
    let dimension = host.dimension().to_owned();
    let located = net::with_point(&*host, EdgePointType::Station, station, |p| {
        p.station().and_then(|s| s.block_entity_pos.map(|pos| (pos, s.connected_ports.clone())))
    })
    .flatten();
    let Some((station_pos, records)) = located else { return Vec::new() };
    if records.is_empty() {
        return Vec::new();
    }
    // The carriages' storage, as one inventory each.
    let mut inventories: Vec<Items> = carriages
        .iter()
        .map(|moving| items_of_with(&*host, moving, &cargo::cargo_slots, &cargo::items_path))
        .collect();
    // The ports: a loaded Postbox is its block's slots, any other is the offline buffer.
    let mut ports: BTreeMap<Pos, Port> = BTreeMap::new();
    let mut loaded_slots: HashMap<Pos, ModBlockEntity> = HashMap::new();
    let mut lookup: HashMap<String, ItemStack> = HashMap::new();
    for (pos, record) in &records {
        let is_loaded = host.chunk_is_loaded(&dimension, [pos[0] >> 4, pos[2] >> 4]) && is_postbox(&host.block(*pos));
        let entity = if is_loaded { host.mod_block_entity(*pos) } else { None };
        let (slots, loaded) = match entity {
            Some(e) => {
                let slots = slots_of(&e);
                loaded_slots.insert(*pos, e);
                (slots, true)
            }
            None => (lock(&BUFFERS).get(&(dimension.clone(), *pos)).cloned().unwrap_or_else(|| vec![None; PORT_SLOTS]), false),
        };
        let converted: Vec<Option<Stack>> = slots
            .iter()
            .map(|slot| {
                slot.as_ref().map(|stack| {
                    lookup.insert(key_of(stack), stack.clone());
                    // Every carriage's inventory must know the stack to give it back.
                    let mut made = None;
                    for items in &mut inventories {
                        made = Some(items.from_stack(&*host, stack));
                    }
                    made.unwrap_or_else(|| Stack { key: key_of(stack), count: stack.count as i32, max: host.max_stack_size(&stack.item).unwrap_or(64) as i32 })
                })
            })
            .collect();
        ports.insert(*pos, Port::new(&record.address, converted, loaded, record.primed));
    }
    // What the carriages hold, to tell a package from anything else.
    for items in &inventories {
        for slot in &items.inventory.slots {
            if let Some(stack) = &slot.stack
                && let Some(real) = items.to_stack(stack)
            {
                lookup.entry(stack.key.clone()).or_insert(real);
            }
        }
    }
    let mut invs: Vec<_> = inventories.iter().map(|i| i.inventory.clone()).collect();
    let is_package = |s: &Stack| lookup.get(&s.key).filter(|real| items::is_package(real)).map(|real| items::address(real));
    let bottomless = |carriage: usize, local: Pos| {
        carriages.get(carriage).is_some_and(|m| m.blocks.iter().any(|b| b.local == local && cargo::is_bottomless(&b.state.name)))
    };
    let outcome = rules::run_mail_transfer(&mut invs, &mut ports, &is_package, &bottomless);
    // The carriages' structures.
    for (i, moving) in carriages.iter_mut().enumerate() {
        if !outcome.carriages_changed.contains(&i) {
            continue;
        }
        inventories[i].inventory = invs[i].clone();
        apply_items_with(moving, &inventories[i], &cargo::items_path);
    }
    // The ports.
    let mut records_changed = false;
    let mut new_records = records.clone();
    for (pos, port) in &ports {
        let slots: Vec<Option<ItemStack>> = port
            .slots
            .iter()
            .map(|slot| {
                slot.as_ref().and_then(|stack| {
                    let mut real = lookup.get(&stack.key)?.clone();
                    real.count = u32::try_from(stack.count).ok()?;
                    Some(real)
                })
            })
            .collect();
        if let Some(record) = new_records.get_mut(pos)
            && record.primed != port.primed
        {
            record.primed = port.primed;
            records_changed = true;
        }
        if outcome.ports_changed.contains(pos) {
            lock(&BUFFERS).insert((dimension.clone(), *pos), slots.clone());
            lock(&WAITING).insert((super::track::dimension_id(&dimension), *pos), addresses(&slots));
            if port.loaded {
                let slots_now = slots.clone();
                host.change_mod_block_entity(*pos, &mut |e| e.items = slots_now.clone());
                host.block_entity_sync(*pos);
            } else {
                records_changed = true;
            }
        }
    }
    if records_changed {
        net::with_point(&*host, EdgePointType::Station, station, |p| {
            if let Some(s) = p.station_mut() {
                s.connected_ports = new_records.clone();
            }
        });
        save_ports(host, station_pos, &new_records);
    }
    // `box.spawnParticles()` (`BoneMealItem.addGrowthParticles`).
    for pos in &outcome.particles {
        let _ = host.run_command(&format!(
            "execute in {dimension} run particle minecraft:happy_villager {:.2} {:.2} {:.2} 0.4 0.4 0.4 0 12",
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5
        ));
    }
    outcome.carriages_changed
}

// --- the hooks --------------------------------------------------------------------------------------------------------------------------

impl Hooks for Mail {
    /// `PackagePortTargetSelectionHandler.onUse`: a Postbox in hand on a Track Station picks the station.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if !is_postbox(&event.stack.item) || host.block(event.pos) != STATION {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else { return Flow::Continue };
        if player.sneaking || player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        if !util::first_click(&*host, player.uuid, event.pos) {
            return Flow::Cancel;
        }
        // `sbe.edgePoint == null`: a station that isn't on track yet can't be a target.
        if station_id(&*host, event.pos).is_none() {
            return Flow::Continue;
        }
        let dimension = host.dimension().to_owned();
        lock(&SELECTED).insert(player.uuid, (dimension, event.pos));
        host.action_bar(player.uuid, Text::literal("Station selected: place the Postbox within 5 blocks of it".to_owned()));
        Flow::Cancel
    }

    /// `PackagePortItem.updateCustomBlockEntityTag`, `flushSettings` and `PackagePortPlacementPacket.handle`: a Postbox placed after
    /// a station was picked is linked to it when it is in range.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if !is_postbox(&event.stack.item) || !is_postbox(&host.block(event.pos)) {
            return;
        }
        let Some(player) = host.acting_player() else { return };
        let Some((dimension, station)) = lock(&SELECTED).remove(&player.uuid) else {
            // `activePackageTarget == null`: `gui.package_port.not_targeting_anything`.
            host.action_bar(player.uuid, Text::literal("No target selected".to_owned()));
            return;
        };
        if dimension != host.dimension() || host.block(station) != STATION {
            return;
        }
        let centre = [f64::from(station[0]) + 0.5, f64::from(station[1]) + 0.5, f64::from(station[2]) + 0.5];
        if let Some(reason) = rules::validate_target(centre, event.pos) {
            host.action_bar(player.uuid, Text::literal(rules::error_text(reason).to_owned()));
            return;
        }
        if !rules::server_accepts(centre, event.pos) {
            return;
        }
        let relative = [station[0] - event.pos[0], station[1] - event.pos[1], station[2] - event.pos[2]];
        let pos = event.pos;
        host.change_mod_block_entity(pos, &mut |e| {
            e.nbt.insert(TARGET.into(), target_tag(relative));
        });
        host.block_entity_sync(pos);
        host.action_bar(player.uuid, Text::literal("Postbox linked to the station".to_owned()));
    }

    /// The stations' saved ports are read once, the records of ports whose block is gone are dropped, and what `Fetch Packages`
    /// reads is refreshed (`GlobalStation.read`, `removePackagePort`).
    fn level_tick_post(&self, host: &mut dyn Host, _: &mut LevelTickPostEvent<'_>) {
        if util::game_time(&*host) % SWEEP != 0 {
            return;
        }
        let dimension = host.dimension().to_owned();
        let stations: Vec<(Uuid, Pos)> = net::with_nets(&*host, |nets| {
            nets.graphs
                .values()
                .flat_map(|g| g.edge_points.values(EdgePointType::Station))
                .filter_map(|p| match &p.data {
                    PointData::Station(s) => s.block_entity_pos.map(|pos| (p.id, pos)),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
        let did = super::track::dimension_id(&dimension);
        for (id, pos) in stations {
            if !host.chunk_is_loaded(&dimension, [pos[0] >> 4, pos[2] >> 4]) {
                continue;
            }
            load_ports(host, pos, id);
            let records = net::with_point(&*host, EdgePointType::Station, id, |p| p.station().map(|s| s.connected_ports.clone())).flatten();
            let Some(records) = records else { continue };
            let mut kept = records.clone();
            for port in records.keys() {
                let here = host.chunk_is_loaded(&dimension, [port[0] >> 4, port[2] >> 4]);
                // `StationBlockEntity.removePackagePort`: the Postbox is gone.
                if here && !is_postbox(&host.block(*port)) {
                    kept.remove(port);
                    lock(&BUFFERS).remove(&(dimension.clone(), *port));
                    lock(&WAITING).remove(&(did, *port));
                    continue;
                }
                if !here {
                    let buffer = lock(&BUFFERS).get(&(dimension.clone(), *port)).cloned().unwrap_or_default();
                    lock(&WAITING).insert((did, *port), addresses(&buffer));
                }
            }
            if kept.len() != records.len() {
                net::with_point(&*host, EdgePointType::Station, id, |p| {
                    if let Some(s) = p.station_mut() {
                        s.connected_ports = kept.clone();
                    }
                });
                save_ports(host, pos, &kept);
            }
        }
    }
}
