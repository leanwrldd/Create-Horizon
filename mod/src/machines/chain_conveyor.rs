//! The Chain Conveyor (`create:chain_conveyor`) (`ChainConveyorBlock`, `ChainConveyorBlockEntity`,
//! `ChainConveyorRenderer`, `ChainConveyorConnectionHandler`, `ChainConveyorConnectionPacket`):
//! a kinetic wheel on a shaft along Y. Chains join two conveyors, which then turn together, and
//! carry packages between their wheels. A Frogport (`frogport.rs`) puts packages on a conveyor
//! and takes the ones for its address off. The rules (geometry, a package's journey, routing,
//! joining) are `create_kinetics::rules::chain_conveyor`; the kinetics core gets the chains as
//! links (`Kind::chain_conveyor`, `solve_with_links`).
//!
//! How it works here:
//! - Click a conveyor with iron chains (Create's `Items.CHAIN`, `minecraft:iron_chain` in this
//!   game version), then a second one: if the rules allow (32 blocks at most, 2.5 at least, not
//!   too steep, 4 chains at most for each, not joined yet) they are joined, costing one chain for
//!   each 2.5 blocks (free in Creative). Sneaking clears the selection. To take a chain off,
//!   click one conveyor and then the other with the Wrench (the chains come back); sneaking with
//!   the Wrench picks the conveyor up, and the chains it held drop.
//! - The packages on a conveyor are kept in memory while it ticks and saved when something
//!   changes hands (a box put on, taken off, a conveyor reached) and every 20 ticks; a reload
//!   sets a box back to where it was last saved. A client draws each box at the position its
//!   saved position and the speed say it has reached.
//!
//! Known differences from Create:
//! - Create's connection handler and package clicks are client-side (the highlights, the outline
//!   of the chain you aim at, picking a package off a chain by clicking it, riding the chains with
//!   a Wrench): none of that is here. A package on a chain can't be taken by hand.
//! - The chains don't scroll with the speed, and the package hangs from its rigging as an item
//!   (the box model), without Create's swinging physics (`ChainConveyorPackagePhysicsData`).
//! - The shape is a full block (Create's interaction shape reaches past the block).
//! - A conveyor can't be placed within one block (sideways) of another: when it is, it goes
//!   straight back to the player (a placement hook can't read neighbours).
//! - Chains drop where the conveyor stood, not spread along the chain.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Mutex, MutexGuard, PoisonError};

use create_kinetics::rules::chain_conveyor::{
    self as rules, Candidate, Conveyor, Env, Package, Port, Stats, Target,
};
use create_kinetics::{Kind, Node, Pos};
use crate::compat::{
    BlockEntityEvent, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent, BlockEntityTickEvent,
    BlockEntityTicksEvent, BlockEntityUpdateTagEvent, Compound, DestroyBlockEvent, DisplayContext, Flow, Hooks,
    Host, ItemStack, PlaceBlockEvent, StandaloneModelsEvent, Tag, Text, UseItemOnEvent,
};

use crate::{kinetic, util};

pub const ID: &str = "create:chain_conveyor";
/// Create's `Items.CHAIN`, `iron_chain` in this game version.
pub const CHAIN: &str = "minecraft:iron_chain";

const SHAFT: &str = "create:block/chain_conveyor/shaft";
const WHEEL: &str = "create:block/chain_conveyor/wheel";
const GUARD: &str = "create:block/chain_conveyor/guard";

const CONNECTIONS: &str = "Connections";
const TRAVELLING: &str = "TravellingPackages";
const LOOPING: &str = "LoopingPackages";
/// This port's own: the game time the saved positions are those of.
const SINCE: &str = "Since";

/// The block atlas's key for the chain texture: vanilla's sprites are keyed without the namespace
/// (`block/iron_chain`); `minecraft:block/iron_chain` is unknown to `draw_quad`, which then draws
/// nothing (the in-game report: no chains between the wheels).
const SPRITE: &str = "block/iron_chain";

struct Conveyors;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Conveyors)
}

// --- what runs while the conveyor ticks --------------------------------------------------------

/// A conveyor's packages and ports as the server holds them.
struct Runtime {
    conveyor: Conveyor<ItemStack>,
    /// The game time of the last tick, for pruning.
    last_tick: i64,
    /// When each port (by its offset) last registered.
    ports_seen: BTreeMap<Pos, i64>,
    /// Something changed that the saved form doesn't have yet.
    dirty: bool,
    /// The game time the saved positions are those of.
    saved_at: i64,
}

type Key = (String, Pos);

static STATE: Mutex<Option<HashMap<Key, Runtime>>> = Mutex::new(None);

fn state() -> MutexGuard<'static, Option<HashMap<Key, Runtime>>> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

fn with_map<R>(f: impl FnOnce(&mut HashMap<Key, Runtime>) -> R) -> R {
    let mut guard = state();
    f(guard.get_or_insert_with(HashMap::new))
}

// --- saved form ---------------------------------------------------------------------------------

fn pos_tag(pos: Pos) -> Tag {
    Tag::IntArray(pos.to_vec())
}

fn pos_of(tag: &Tag) -> Option<Pos> {
    match tag {
        Tag::IntArray(v) if v.len() == 3 => Some([v[0], v[1], v[2]]),
        _ => None,
    }
}

fn float_of(tag: Option<&Tag>) -> f32 {
    match tag {
        Some(Tag::Float(bits)) => f32::from_bits(*bits),
        _ => 0.0,
    }
}

fn package_tag(boxed: &Package<ItemStack>) -> Tag {
    let mut tag = Compound::new();
    tag.insert("Position".into(), Tag::Float(boxed.position.to_bits()));
    tag.insert("Item".into(), util::stack_tag(&boxed.item));
    Tag::Compound(tag)
}

fn package_of(tag: &Tag) -> Option<Package<ItemStack>> {
    let item = util::stack_of(tag.get("Item")?)?;
    Some(Package {
        position: float_of(tag.get("Position")),
        address: super::package::address(&item),
        item,
        just_flipped: false,
    })
}

/// `ChainConveyorBlockEntity.read`.
pub fn read(nbt: &Compound) -> Conveyor<ItemStack> {
    let mut conveyor = Conveyor::default();
    if let Some(Tag::List(list)) = util::get(nbt, CONNECTIONS) {
        conveyor.connections = list.iter().filter_map(pos_of).collect();
    }
    if let Some(Tag::List(list)) = util::get(nbt, TRAVELLING) {
        for entry in list {
            let (Some(target), Some(Tag::List(boxes))) = (entry.get("Target").and_then(pos_of), entry.get("Packages")) else {
                continue;
            };
            conveyor.travelling.insert(target, boxes.iter().filter_map(package_of).collect());
        }
    }
    if let Some(Tag::List(list)) = util::get(nbt, LOOPING) {
        conveyor.looping = list.iter().filter_map(package_of).collect();
    }
    conveyor
}

/// The connections of the conveyor in a block entity's data.
pub fn connections_of(nbt: &Compound) -> BTreeSet<Pos> {
    match util::get(nbt, CONNECTIONS) {
        Some(Tag::List(list)) => list.iter().filter_map(pos_of).collect(),
        _ => BTreeSet::new(),
    }
}

/// `ChainConveyorBlockEntity.write`, as the entries that differ from `nbt`.
fn diff(nbt: &Compound, conveyor: &Conveyor<ItemStack>, now: i64) -> Vec<(&'static str, Tag)> {
    let connections = Tag::List(conveyor.connections.iter().map(|c| pos_tag(*c)).collect());
    let travelling = Tag::List(
        conveyor
            .travelling
            .iter()
            .filter(|(_, boxes)| !boxes.is_empty())
            .map(|(target, boxes)| {
                let mut entry = Compound::new();
                entry.insert("Target".into(), pos_tag(*target));
                entry.insert("Packages".into(), Tag::List(boxes.iter().map(package_tag).collect()));
                Tag::Compound(entry)
            })
            .collect(),
    );
    let looping = Tag::List(conveyor.looping.iter().map(package_tag).collect());
    let mut wanted = vec![(CONNECTIONS, connections), (TRAVELLING, travelling), (LOOPING, looping)];
    wanted.retain(|(key, value)| util::differs(nbt, key, value));
    if !wanted.is_empty() {
        wanted.push((SINCE, Tag::Long(now)));
    }
    wanted
}

fn store(entity: &mut crate::compat::ModBlockEntity, conveyor: &Conveyor<ItemStack>, now: i64) -> bool {
    let entries = diff(&entity.nbt, conveyor, now);
    if entries.is_empty() {
        return false;
    }
    for (key, value) in entries {
        util::put(&mut entity.nbt, key, value);
    }
    true
}

// --- the world -----------------------------------------------------------------------------------

fn offset_pos(pos: Pos, offset: Pos) -> Pos {
    util::add(pos, offset)
}

/// The speed of a conveyor in RPM (`getSpeed`), from its saved data.
fn speed_at(host: &dyn Host, pos: Pos) -> f32 {
    host.mod_block_entity(pos).map_or(0.0, |entity| kinetic::speed(&entity))
}

/// Brings the conveyor at `pos` into memory when it isn't, from its saved data.
fn ensure(host: &dyn Host, dimension: &str, pos: Pos) -> bool {
    let key = (dimension.to_owned(), pos);
    if with_map(|map| map.contains_key(&key)) {
        return true;
    }
    let Some(entity) = host.mod_block_entity(pos) else {
        return false;
    };
    let conveyor = read(&entity.nbt);
    with_map(|map| {
        map.entry(key).or_insert(Runtime {
            conveyor,
            last_tick: util::game_time(host),
            ports_seen: BTreeMap::new(),
            dirty: false,
            saved_at: util::game_time(host),
        });
    });
    true
}

/// Runs `f` on the conveyor at `pos`, if there is one.
fn with_conveyor<R>(host: &dyn Host, pos: Pos, f: impl FnOnce(&mut Runtime) -> R) -> Option<R> {
    let dimension = host.dimension();
    if host.block(pos) != ID || !ensure(host, dimension, pos) {
        return None;
    }
    with_map(|map| map.get_mut(&(dimension.to_owned(), pos)).map(f))
}

struct World<'a> {
    host: &'a mut dyn Host,
    pos: Pos,
}

impl Env<ItemStack> for World<'_> {
    fn export(&mut self, port: Pos, item: &ItemStack) -> bool {
        super::frogport::catch(self.host, offset_pos(self.pos, port), item)
    }

    fn anticipate(&mut self, _port: Pos) {}

    fn conveyor_at(&self, connection: Pos) -> bool {
        self.host.block(offset_pos(self.pos, connection)) == ID
    }

    fn accepts_from_other(&self, connection: Pos) -> bool {
        let other = offset_pos(self.pos, connection);
        let dimension = self.host.dimension().to_owned();
        with_map(|map| map.get(&(dimension, other)).is_none_or(|rt| rt.conveyor.can_accept_from_other()))
    }
}

// --- what the frogports ask ----------------------------------------------------------------------

/// `PackagePortTarget.ChainConveyorFrogportTarget.register`: a Frogport at `port_offset` (the
/// port's offset from the conveyor) tells the conveyor its address and where it works.
pub fn register_port(host: &dyn Host, conveyor: Pos, port_offset: Pos, target: &Target, filter: &str) {
    let now = util::game_time(host);
    with_conveyor(host, conveyor, |rt| {
        rt.conveyor.routing.receive_port_info(filter, target.connection.unwrap_or([0, 0, 0]));
        let port = Port { position: target.chain_pos, connection: target.connection, filter: filter.to_owned() };
        let map = if target.connection.is_none() { &mut rt.conveyor.loop_ports } else { &mut rt.conveyor.travel_ports };
        if map.get(&port_offset) != Some(&port) {
            map.insert(port_offset, port);
        }
        rt.ports_seen.insert(port_offset, now);
    });
}

/// `deregister`.
pub fn deregister_port(host: &dyn Host, conveyor: Pos, port_offset: Pos, filter: &str) {
    with_conveyor(host, conveyor, |rt| {
        rt.conveyor.loop_ports.remove(&port_offset);
        rt.conveyor.travel_ports.remove(&port_offset);
        rt.ports_seen.remove(&port_offset);
        rt.conveyor.routing.entries_by_distance.retain(|e| !(e.end_of_route && e.port == filter));
        rt.conveyor.routing.changed = true;
    });
}

/// `ChainConveyorFrogportTarget.export`: puts a box on the conveyor at `target`; with `simulate`
/// only asks whether it would be taken.
pub fn export_from_port(host: &mut dyn Host, conveyor: Pos, target: &Target, item: &ItemStack, simulate: bool) -> bool {
    let rpm = speed_at(&*host, conveyor);
    let boxed = Package {
        position: target.chain_pos,
        address: super::package::address(item),
        item: item.clone(),
        just_flipped: false,
    };
    let taken = with_conveyor(&*host, conveyor, |rt| {
        let others_accept = |rt: &Runtime, connection: Option<Pos>| match connection {
            None => rt.conveyor.can_accept_more(),
            Some(c) => rt.conveyor.connections.contains(&c),
        };
        if let Some(connection) = target.connection {
            if !rt.conveyor.connections.contains(&connection) {
                return None;
            }
        }
        if simulate {
            return Some(rpm != 0.0 && others_accept(rt, target.connection));
        }
        let added = match target.connection {
            None => {
                rt.conveyor.add_looping(boxed);
                true
            }
            Some(connection) => rt.conveyor.add_travelling(boxed, connection),
        };
        rt.dirty |= added;
        Some(added)
    })
    .flatten()
    .unwrap_or(false);
    // `canAcceptPackagesFor(connection)`: the conveyor the chain leads to must have room too.
    if simulate && taken {
        if let Some(connection) = target.connection {
            let other = offset_pos(conveyor, connection);
            return with_conveyor(&*host, other, |rt| rt.conveyor.can_accept_more()).unwrap_or(false);
        }
    }
    taken
}

/// The conveyors near `pos`, as the rules need them, for a Frogport looking for a place to
/// attach (`nearest_target`): offsets are from `pos`.
pub fn candidates_near(host: &dyn Host, pos: Pos) -> Vec<Candidate> {
    let mut out = Vec::new();
    for dx in -6..=6 {
        for dy in -1..=6 {
            for dz in -6..=6 {
                let at = [pos[0] + dx, pos[1] + dy, pos[2] + dz];
                if host.block(at) != ID {
                    continue;
                }
                let Some(entity) = host.mod_block_entity(at) else {
                    continue;
                };
                out.push(Candidate {
                    offset: [dx, dy, dz],
                    connections: connections_of(&entity.nbt).into_iter().collect(),
                    reversed: kinetic::speed(&entity) < 0.0,
                });
            }
        }
    }
    out
}

/// Where a Frogport's target is in the world, for its head and tongue: the point on the
/// conveyor `target` names (`getExactTargetLocation`).
pub fn exact_location(host: &dyn Host, port: Pos, target: &Target) -> Option<[f64; 3]> {
    let conveyor = offset_pos(port, target.relative_pos);
    let entity = host.mod_block_entity(conveyor)?;
    let origin = [f64::from(conveyor[0]), f64::from(conveyor[1]), f64::from(conveyor[2])];
    let relative = match target.connection {
        None => rules::loop_position(target.chain_pos),
        Some(connection) => {
            let stats = rules::connection_stats(connection, kinetic::speed(&entity) < 0.0);
            rules::travel_position(&stats, target.chain_pos)
        }
    };
    Some([origin[0] + relative[0], origin[1] + relative[1], origin[2] + relative[2]])
}

// --- the hooks ---------------------------------------------------------------------------------

impl Hooks for Conveyors {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        for model in [SHAFT, WHEEL, GUARD] {
            if !event.models.iter().any(|m| m == model) {
                event.models.push(model.to_owned());
            }
        }
    }

    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        (event.block == ID).then(|| (ID.to_owned(), kinetic::new_entity()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        let pos = event.pos;
        let dimension = host.dimension().to_owned();
        let now = util::game_time(host);
        let key = (dimension.clone(), pos);

        // Into memory, and out of the table for the length of the tick: the world is asked from
        // inside it.
        let mut rt = with_map(|map| map.remove(&key)).unwrap_or_else(|| Runtime {
            conveyor: read(&event.entity.nbt),
            last_tick: now,
            ports_seen: BTreeMap::new(),
            dirty: false,
            saved_at: now,
        });
        rt.last_tick = now;

        // The shaft: a conveyor's chains join its network.
        let links: Vec<Pos> = rt.conveyor.connections.iter().map(|c| offset_pos(pos, *c)).collect();
        let node = Node { impact: 1.0, ..Node::passive(Kind::chain_conveyor()) };
        let Some((rpm, changed)) = kinetic::tick_linked(host, pos, event.entity, node, Some(&links)) else {
            return; // The network jammed and the block broke.
        };

        // `removeInvalidConnections`: a chain whose other end is gone (or doesn't list this one).
        let mut dropped: Vec<Package<ItemStack>> = Vec::new();
        if (now + i64::from(pos[0] + pos[2]).rem_euclid(20)) % 20 == 0 {
            let connections: Vec<Pos> = rt.conveyor.connections.iter().copied().collect();
            for connection in connections {
                let target = offset_pos(pos, connection);
                if !host.chunk_is_loaded(&dimension, [target[0] >> 4, target[2] >> 4]) {
                    continue;
                }
                let valid = host.block(target) == ID
                    && host.mod_block_entity(target).is_some_and(|e| {
                        connections_of(&e.nbt).contains(&[-connection[0], -connection[1], -connection[2]])
                    });
                if !valid {
                    dropped.extend(rt.conveyor.remove_connection(connection).unwrap_or_default());
                    rt.dirty = true;
                }
            }
            // Ports that stopped registering are forgotten.
            let stale: Vec<Pos> = rt.ports_seen.iter().filter(|(_, seen)| now - **seen > 100).map(|(p, _)| *p).collect();
            for port in stale {
                rt.ports_seen.remove(&port);
                rt.conveyor.loop_ports.remove(&port);
                rt.conveyor.travel_ports.remove(&port);
            }
        }

        // Routing: this table ages, and tells its neighbours what it knows.
        rt.conveyor.routing.tick();
        if rt.conveyor.routing.should_advertise() {
            let connections: Vec<Pos> = rt.conveyor.connections.iter().copied().collect();
            for connection in connections {
                let other = offset_pos(pos, connection);
                if host.block(other) != ID || !ensure(&*host, &dimension, other) {
                    continue;
                }
                let table = rt.conveyor.routing.clone();
                with_map(|map| {
                    if let Some(neighbour) = map.get_mut(&(dimension.clone(), other)) {
                        table.advertise_to(connection, &mut neighbour.conveyor.routing);
                    }
                });
            }
            rt.conveyor.routing.changed = false;
            rt.conveyor.routing.last_update = 0;
        }

        // The packages.
        let tick = {
            let mut world = World { host: &mut *host, pos };
            rt.conveyor.tick(rpm, &mut world)
        };
        for (connection, boxed) in tick.handoffs {
            let other = offset_pos(pos, connection);
            with_conveyor(&*host, other, |neighbour| {
                neighbour.conveyor.add_looping(boxed);
                neighbour.dirty = true;
            });
        }
        for (connection, boxed) in tick.flips {
            let other = offset_pos(pos, connection);
            with_conveyor(&*host, other, |neighbour| {
                neighbour.conveyor.add_travelling(boxed, [-connection[0], -connection[1], -connection[2]]);
                neighbour.dirty = true;
            });
        }
        for boxed in dropped {
            util::spawn_item(host, util::centre(pos), &boxed.item, [0.0; 3], 10);
        }

        // Saved when something changed, and now and then while boxes move.
        let moving = !rt.conveyor.looping.is_empty() || rt.conveyor.travelling.values().any(|b| !b.is_empty());
        let due = moving && now - rt.saved_at >= 20;
        let mut saved = false;
        if tick.changed || rt.dirty || due {
            saved = store(event.entity, &rt.conveyor, now);
            rt.saved_at = now;
            if tick.changed || rt.dirty {
                host.block_entity_sync(pos);
            }
            rt.dirty = false;
        }
        if changed && !saved {
            host.block_entity_sync(pos);
        }
        with_map(|map| {
            map.insert(key, rt);
            // Conveyors that stopped ticking (broken, or their chunk unloaded) are forgotten.
            if now % 200 == 0 {
                map.retain(|_, runtime| now - runtime.last_tick < 400);
            }
        });
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        if event.entity.id != ID {
            return None;
        }
        let nbt = &event.entity.nbt;
        let mut tag = kinetic::update_tag(event.entity);
        for key in [CONNECTIONS, TRAVELLING, LOOPING] {
            tag.insert(key.into(), util::get(nbt, key).cloned().unwrap_or_else(|| Tag::List(Vec::new())));
        }
        tag.insert(SINCE.into(), Tag::Long(util::long(nbt, SINCE)));
        Some(tag)
    }

    /// `createRenderBoundingBox`: the block grown by 3, and out to the conveyors it is chained to.
    fn block_entity_render_bounding_box(&self, _: &dyn Host, event: &mut BlockEntityRenderBoundingBoxEvent<'_>) {
        if event.block != ID {
            return;
        }
        let mut bounds = [-3.0f32, -3.0, -3.0, 4.0, 4.0, 4.0];
        for connection in connections_of(&event.entity.nbt) {
            for axis in 0..3 {
                let at = connection[axis] as f32;
                bounds[axis] = bounds[axis].min(at - 3.0);
                bounds[axis + 3] = bounds[axis + 3].max(at + 4.0);
            }
        }
        *event.bounds = bounds;
    }

    /// `ChainConveyorRenderer`: the shaft and wheel, the guards and chains and the boxes.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let nbt = &event.entity.nbt;
        let rpm = kinetic::speed(event.entity);
        let conveyor = read(nbt);
        let since = util::long(nbt, SINCE);
        let now = event.draw.game_time as f64 + f64::from(event.draw.partial);
        let draw = &mut *event.draw;
        const CENTRE: [f32; 3] = [0.5; 3];
        const UP: [f32; 3] = [0.0, 1.0, 0.0];

        // The shaft turns with the rotation, the wheel at the pace the packages ride it.
        draw.spin(UP, CENTRE, rpm * 0.3, 0.0);
        host.draw_model(draw, SHAFT, 0xFF_FFFF);
        draw.still();
        let speed = rpm / 360.0;
        let degrees_per_tick = speed / (std::f32::consts::PI * rules::RADIUS) * 360.0;
        draw.spin(UP, CENTRE, degrees_per_tick, 0.0);
        host.draw_model(draw, WHEEL, 0xFF_FFFF);
        draw.still();

        let reversed = rpm < 0.0;
        for connection in &conveyor.connections {
            let stats = rules::connection_stats(*connection, reversed);
            let direction = [stats.end[0] - stats.start[0], stats.end[1] - stats.start[1], stats.end[2] - stats.start[2]];
            let yaw = direction[0].atan2(direction[2]).to_degrees() as f32;
            draw.push();
            draw.rotate_about(UP, yaw, CENTRE);
            host.draw_model(draw, GUARD, 0xFF_FFFF);
            draw.pop();
            draw_chain(host, draw, &stats);
        }

        // The packages, at where their saved position and the speed have taken them.
        let elapsed = (now - since as f64).max(0.0) as f32;
        let distance_per_tick = speed.abs();
        for boxed in &conveyor.looping {
            let degrees = rules::wrap_angle(boxed.position + degrees_per_tick * elapsed);
            let at = rules::loop_position(degrees);
            let yaw = rules::wrap_angle(degrees) + if reversed { 180.0 } else { 0.0 };
            draw_box(host, draw, &boxed.item, at, yaw);
        }
        for (connection, boxes) in &conveyor.travelling {
            let stats = rules::connection_stats(*connection, reversed);
            let direction = [stats.end[0] - stats.start[0], stats.end[2] - stats.start[2]];
            let yaw = direction[0].atan2(direction[1]).to_degrees() as f32 - 90.0;
            for boxed in boxes {
                let along = (boxed.position + distance_per_tick * elapsed).min(stats.chain_length);
                draw_box(host, draw, &boxed.item, rules::travel_position(&stats, along), yaw);
            }
        }
    }

    /// `ChainConveyorBlock.getStateForPlacement`: not within one block of another conveyor.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if &*event.stack.item != ID {
            return;
        }
        let pos = event.pos;
        let mut near = Vec::new();
        for x in -1..=1 {
            for z in -1..=1 {
                if (x, z) != (0, 0) && host.block([pos[0] + x, pos[1], pos[2] + z]) == ID {
                    near.push([x, 0, z]);
                }
            }
        }
        if rules::can_place_beside(&near) {
            return;
        }
        let dimension = host.dimension();
        if let Some(air) = host.default_state("minecraft:air") {
            host.set_block(dimension, pos, air, util::UPDATE_ALL);
        }
        if let Some(player) = host.acting_player() {
            if player.game_mode != util::CREATIVE {
                if let Some(rest) = host.give_stack(player.uuid, event.stack.clone()) {
                    util::spawn_item(host, util::centre(pos), &rest, [0.0; 3], 0);
                }
            }
            host.action_bar(player.uuid, Text::literal("Too close to another chain conveyor".to_owned()));
        }
    }

    /// A broken conveyor takes its chains off its neighbours and drops its packages and chains.
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let dimension = host.dimension().to_owned();
        let creative = host.acting_player().is_some_and(|p| p.game_mode == util::CREATIVE);
        let Some(entity) = host.mod_block_entity(pos) else {
            return Flow::Continue;
        };
        let mut conveyor = with_map(|map| map.remove(&(dimension.clone(), pos))).map_or_else(|| read(&entity.nbt), |rt| rt.conveyor);
        let at = util::centre(pos);
        for boxed in conveyor.looping.drain(..) {
            util::spawn_item(host, at, &boxed.item, [0.0; 3], 10);
        }
        for boxed in conveyor.travelling.values_mut().flat_map(|b| b.drain(..)) {
            util::spawn_item(host, at, &boxed.item, [0.0; 3], 10);
        }
        for connection in &conveyor.connections {
            let other = offset_pos(pos, *connection);
            // `chainDestroyed` (the cost comes back as chains) and `removeConnectionTo` on the other end.
            let cost = rules::chain_cost(*connection);
            if !creative {
                let chains = ItemStack { item: CHAIN.into(), count: cost as u32, damage: 0, extra: None };
                util::spawn_item(host, at, &chains, [0.0; 3], 0);
            }
            remove_connection_at(host, other, [-connection[0], -connection[1], -connection[2]]);
        }
        Flow::Continue
    }

    /// Chains join two conveyors; the Wrench takes a chain off.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let item = &*event.stack.item;
        let is_wrench = item == util::WRENCH;
        if !is_wrench && item != CHAIN {
            // `useItemOn`: a Frogport is placed against it; anything else uses the block normally.
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        if is_wrench && player.sneaking {
            // The conveyor is picked up (`onSneakWrenched`).
            return util::wrench(host, pos, |_| {});
        }
        if !util::first_click(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        let dimension = host.dimension().to_owned();
        if player.sneaking {
            selections().remove(&player.uuid);
            host.action_bar(player.uuid, Text::literal("Selection cleared".to_owned()));
            return Flow::Cancel;
        }
        let first = selections().get(&player.uuid).filter(|(dim, _, _)| *dim == dimension).map(|(_, p, _)| *p);
        let Some(first) = first.filter(|first| host.block(*first) == ID) else {
            selections().insert(player.uuid, (dimension, pos, is_wrench));
            host.action_bar(player.uuid, Text::literal(if is_wrench { "Select the conveyor the chain goes to" } else { "Select the second conveyor" }.to_owned()));
            return Flow::Cancel;
        };
        selections().remove(&player.uuid);
        if first == pos {
            return Flow::Cancel;
        }
        let line = if is_wrench { disconnect(host, &player, first, pos) } else { connect(host, &player, first, pos) };
        host.action_bar(player.uuid, Text::literal(line));
        Flow::Cancel
    }
}

/// The conveyor each player selected first: the dimension, where, and whether with the Wrench.
fn selections() -> MutexGuard<'static, BTreeMap<u128, (String, Pos, bool)>> {
    static SELECTIONS: Mutex<BTreeMap<u128, (String, Pos, bool)>> = Mutex::new(BTreeMap::new());
    SELECTIONS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `validateAndConnect` and `ChainConveyorConnectionPacket.applySettings`: joins `first` and
/// `second`. The line to show the player.
fn connect(host: &mut dyn Host, player: &crate::compat::NativeServerPlayer, first: Pos, second: Pos) -> String {
    let Some(entity) = host.mod_block_entity(second) else {
        return "Invalid conveyors".to_owned();
    };
    let Some(first_entity) = host.mod_block_entity(first) else {
        return "Invalid conveyors".to_owned();
    };
    let connections = connections_of(&entity.nbt);
    if let Err(reject) = rules::validate_connection(first, second, &connections) {
        return reject.text().to_owned();
    }
    if connections_of(&first_entity.nbt).len() >= rules::MAX_CONNECTIONS {
        return rules::Reject::CannotAddMoreConnections.text().to_owned();
    }
    let offset = [second[0] - first[0], second[1] - first[1], second[2] - first[2]];
    let cost = rules::chain_cost(offset) as u32;
    if player.game_mode != util::CREATIVE {
        let taken = host.take_items(CHAIN, cost);
        if taken < cost {
            if taken > 0 {
                host.give_items(CHAIN, taken);
            }
            return "Not enough chains".to_owned();
        }
    }
    add_connection(host, first, offset);
    add_connection(host, second, [-offset[0], -offset[1], -offset[2]]);
    host.play_sound("minecraft:block.chain.place", util::centre(second), 4, 1.0, 0.8);
    format!("Connected ({cost} chains)")
}

/// The Wrench on a second conveyor: takes the chain between two off and returns its chains.
fn disconnect(host: &mut dyn Host, player: &crate::compat::NativeServerPlayer, first: Pos, second: Pos) -> String {
    let offset = [second[0] - first[0], second[1] - first[1], second[2] - first[2]];
    let joined = host.mod_block_entity(first).is_some_and(|e| connections_of(&e.nbt).contains(&offset));
    if !joined {
        return "Those conveyors aren't joined".to_owned();
    }
    if player.game_mode != util::CREATIVE {
        let chains = ItemStack { item: CHAIN.into(), count: rules::chain_cost(offset) as u32, damage: 0, extra: None };
        if let Some(rest) = host.give_stack(player.uuid, chains) {
            util::spawn_item(host, player.position, &rest, [0.0; 3], 0);
        }
    }
    remove_connection_at(host, first, offset);
    remove_connection_at(host, second, [-offset[0], -offset[1], -offset[2]]);
    host.play_sound("minecraft:block.chain.break", util::centre(second), 4, 1.0, 1.0);
    "Chain removed".to_owned()
}

/// `addConnectionTo`.
fn add_connection(host: &mut dyn Host, pos: Pos, offset: Pos) {
    with_conveyor(&*host, pos, |rt| {
        rt.conveyor.connections.insert(offset);
        rt.dirty = true;
    });
    let now = util::game_time(&*host);
    host.change_mod_block_entity(pos, &mut |entity| {
        let mut conveyor = read(&entity.nbt);
        conveyor.connections.insert(offset);
        store(entity, &conveyor, now);
    });
    host.block_entity_sync(pos);
}

/// `removeConnectionTo`: the chain goes, and the boxes on it drop.
fn remove_connection_at(host: &mut dyn Host, pos: Pos, offset: Pos) {
    if host.block(pos) != ID {
        return;
    }
    let dropped = with_conveyor(&*host, pos, |rt| {
        rt.dirty = true;
        rt.conveyor.remove_connection(offset)
    })
    .flatten()
    .unwrap_or_default();
    for boxed in dropped {
        util::spawn_item(host, util::centre(pos), &boxed.item, [0.0; 3], 10);
    }
    let now = util::game_time(&*host);
    host.change_mod_block_entity(pos, &mut |entity| {
        let mut conveyor = read(&entity.nbt);
        conveyor.connections.remove(&offset);
        conveyor.travelling.remove(&offset);
        store(entity, &conveyor, now);
    });
    host.block_entity_sync(pos);
}

// --- drawing -----------------------------------------------------------------------------------------

/// A chain: two crossed strips of the chain texture along `stats`, in steps of a block.
fn draw_chain(host: &dyn Host, draw: &mut crate::compat::BlockEntityDraw, stats: &Stats) {
    let d = [stats.end[0] - stats.start[0], stats.end[1] - stats.start[1], stats.end[2] - stats.start[2]];
    let norm = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if norm < 1e-6 {
        return;
    }
    let u = [d[0] / norm, d[1] / norm, d[2] / norm];
    // Two directions across the chain: sideways (horizontal) and the third axis.
    let side = {
        let s = [u[2], 0.0, -u[0]];
        let n = (s[0] * s[0] + s[2] * s[2]).sqrt().max(1e-6);
        [s[0] / n, 0.0, s[2] / n]
    };
    let up = [
        u[1] * side[2] - u[2] * side[1],
        u[2] * side[0] - u[0] * side[2],
        u[0] * side[1] - u[1] * side[0],
    ];
    let radius = 1.5 / 16.0;
    let steps = norm.ceil() as usize;
    for step in 0..steps {
        let from = step as f64;
        let to = (step as f64 + 1.0).min(norm);
        let v1 = ((to - from) * 16.0) as f32;
        let at = |t: f64, across: [f64; 3], offset: f64| {
            [
                (stats.start[0] + u[0] * t + across[0] * offset) as f32,
                (stats.start[1] + u[1] * t + across[1] * offset) as f32,
                (stats.start[2] + u[2] * t + across[2] * offset) as f32,
            ]
        };
        for (across, u0) in [(side, 0.0f32), (up, 3.0f32)] {
            let uv = [u0, 0.0, u0 + 3.0, v1];
            let quad = [at(to, across, -radius), at(from, across, -radius), at(from, across, radius), at(to, across, radius)];
            host.draw_quad(draw, SPRITE, quad, uv, 0xFF_FFFF);
            // And the back of it.
            let back = [quad[3], quad[2], quad[1], quad[0]];
            host.draw_quad(draw, SPRITE, back, uv, 0xFF_FFFF);
        }
    }
}

/// A package hanging at `at` (block-local, where the chain holds it), turned `yaw` degrees.
fn draw_box(host: &dyn Host, draw: &mut crate::compat::BlockEntityDraw, item: &ItemStack, at: [f64; 3], yaw: f32) {
    let hook = super::package::style_hook(&item.item);
    draw.push();
    draw.translate([at[0] as f32, at[1] as f32 - hook + 7.0 / 16.0 - 0.1, at[2] as f32]);
    draw.rotate_about([0.0, 1.0, 0.0], yaw, [0.0; 3]);
    draw.scale([1.49; 3]);
    host.draw_item(draw, item, DisplayContext::Fixed, None, 0);
    draw.pop();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(item: &str) -> ItemStack {
        ItemStack { item: item.into(), count: 1, damage: 0, extra: None }
    }

    #[test]
    fn a_conveyor_survives_being_saved_and_read() {
        let mut conveyor: Conveyor<ItemStack> = Conveyor::default();
        conveyor.connections.insert([10, 0, 0]);
        conveyor.connections.insert([0, 2, -8]);
        let boxed = |position: f32| Package {
            position,
            address: String::new(),
            item: stack("create:cardboard_package_12x12"),
            just_flipped: false,
        };
        conveyor.looping.push(boxed(33.5));
        conveyor.travelling.insert([10, 0, 0], vec![boxed(2.25), boxed(4.0)]);
        let mut nbt = Compound::new();
        for (key, value) in diff(&nbt, &conveyor, 99) {
            util::put(&mut nbt, key, value);
        }
        let back = read(&nbt);
        assert_eq!(back.connections, conveyor.connections);
        assert_eq!(back.looping, conveyor.looping);
        assert_eq!(back.travelling, conveyor.travelling);
        assert_eq!(util::long(&nbt, SINCE), 99);
        // Nothing differs now.
        assert!(diff(&nbt, &back, 100).is_empty());
    }

    #[test]
    fn the_chain_sprite_is_keyed_as_the_atlas_keys_vanilla_sprites() {
        assert!(!SPRITE.contains(':'), "{SPRITE}");
        assert!(SPRITE.starts_with("block/"));
    }

    #[test]
    fn an_empty_entity_reads_as_a_conveyor_with_nothing() {
        let conveyor = read(&Compound::new());
        assert!(conveyor.connections.is_empty() && conveyor.looping.is_empty() && conveyor.travelling.is_empty());
    }
}
