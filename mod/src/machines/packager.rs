//! The Packager (`create:packager`) and the Re-Packager (`create:repackager`) (`PackagerBlock`,
//! `PackagerBlockEntity`, `PackagerItemHandler`, `PackagerRenderer`, `RepackagerBlock`,
//! `RepackagerBlockEntity`, `PackagerGenerator`): a block that boxes up what is in the inventory
//! behind it, and unboxes the packages it is given into it. The state machine (the tray, the
//! queue, the cooldown, the sign address) is `create_kinetics::rules::packager`; what goes into a
//! box and what fits an inventory is `rules::package`; the Re-Packager's joining of orders is
//! `rules::repackager` and `machines/repackager.rs`. This is the glue.
//!
//! How it works here:
//! - The block faces away from its inventory (it turns its back on the first chest, barrel or
//!   vault it is placed against; `facing` is where the hatch is not). A redstone signal (a
//!   rising edge, or held: every 10 ticks) packs up to nine stacks of the inventory into a
//!   package, addressed by the text of a sign touching the packager (any side; the front, else
//!   the back; the last in down, up, north, south, west, east order wins) and slides it out of
//!   the tray.
//! - The box is the block's one item slot, so hoppers, funnels, chutes and Mechanical Arms take
//!   it from any side once the tray is back, and put packages in (the same slot, while nothing
//!   is held or waiting): a package that arrives is unpacked into the inventory behind, if it
//!   fits, else it stays in the slot to be taken out again. A belt hands packages in too.
//! - Right-click with a package in hand unpacks it into the inventory; with an empty hand it
//!   takes the held box; the Wrench turns the block (sneaking picks it up).
//! - Boxes that can't leave yet wait in a queue (`QueuedExitingPackages`).
//!
//! Known differences from Create:
//! - No computer peripheral. A Stock Link sets the block's `linked` state (the link's tick, `stock_link.rs`), and a linked
//!   packager ignores redstone (`redstoneModeActive`) and serves the network's orders (`attempt_requests`).
//! - Create's comparator reads 15 while a box is held or in motion; here it reads what a vanilla
//!   comparator reads from a one-slot container.
//! - The inventory is read from saved data and can't be asked first: see `packager_inv.rs`.
//! - A package slides out of the hatch visually, but the hatch faces the inventory side as in
//!   Create (the tray and the hatch are drawn on the side opposite `facing`).

use std::sync::OnceLock;

use create_kinetics::rules::package::{self as package_rules, Slot, empty_contents};
use create_kinetics::rules::packager::{self as rules, Cue, Edge, State};
use create_kinetics::{Dir, Pos};
use crate::compat::{
    BlockEntityContainerEvent, BlockEntityEvent, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent,
    BlockEntityTickEvent, BlockEntityTicksEvent, BlockEntityUpdateTagEvent, BlockStateId, Compound,
    ContainerAcceptsEvent, DestroyBlockEvent, DisplayContext, EquipmentSlot, Flow, Hooks, Host, ItemStack,
    LevelHost, ModBlockEntity, ModContainer, PlaceBlockEvent, RightClickBlockEvent, StandaloneModelsEvent,
    StateForPlacementEvent, Tag, UseItemOnEvent, serde_json,
};

use super::chute_inv as inv;
use super::package as items;
use super::packager_inv as target_inv;
use crate::util;

pub const PACKAGER: &str = "create:packager";
pub const REPACKAGER: &str = "create:repackager";

const HATCH_CLOSED: &str = "create:block/packager/hatch_closed";
const HATCH_OPEN: &str = "create:block/packager/hatch_open";
const TRAY: &str = "create:block/packager/tray";
const TRAY_DEFRAG: &str = "create:block/repackager/tray";

/// The keys `PackagerBlockEntity.write` saves, under Create's names.
const ACTIVE: &str = "Active";
const INWARD: &str = "AnimationInward";
const SIGN: &str = "SignAddress";
const HELD: &str = "HeldBox";
const INSERTED: &str = "InsertedBox";
const QUEUED: &str = "QueuedExitingPackages";
/// This port's own: the game time the animation started at (-1 when it isn't running), and the
/// game time a button signal's cooldown runs until.
const START: &str = "AnimStart";
const COOLDOWN: &str = "CooldownUntil";

/// Called when a package comes to rest in a packager's tray, with the packager's position: the
/// Frogport on top of it (`wakeTheFrogs`) pulls from its inventory then. The Frogport module
/// fills it in when it starts.
pub(crate) static WAKE_FROGS: OnceLock<fn(&mut dyn Host, Pos)> = OnceLock::new();

struct Packagers;

pub fn hooks() -> Box<dyn Hooks> {
    // A belt that ends at a packager hands its packages in.
    crate::belt::register_input(crate::belt::BeltInput {
        claims: |block| block == PACKAGER || block == REPACKAGER,
        can_insert: |host, pos, _| accepts_now(host, pos),
        insert: |host, pos, _, item, simulate| {
            let mut rest = item.stack.clone();
            if !items::is_package(&rest) || !accepts_now(host, pos) {
                return rest;
            }
            if simulate {
                rest.count -= 1;
                return rest;
            }
            if receive_at(host, pos, &rest) {
                rest.count -= 1;
            }
            rest
        },
    });
    Box::new(Packagers)
}

fn is_ours(block: &str) -> bool {
    block == PACKAGER || block == REPACKAGER
}

// --- the saved form ------------------------------------------------------------------------------

fn stack_or_none(tag: Option<&Tag>) -> Option<ItemStack> {
    tag.and_then(util::stack_of)
}

fn empty() -> Tag {
    Tag::Compound(Compound::new())
}

/// `PackagerBlockEntity.read`.
pub fn read(nbt: &Compound) -> State<ItemStack> {
    let queued = match util::get(nbt, QUEUED) {
        Some(Tag::List(list)) => list
            .iter()
            .filter_map(|entry| {
                let stack = util::stack_of(entry.get("item_stack")?)?;
                let count = match entry.get("count") {
                    Some(Tag::Int(count)) => (*count).max(0) as u32,
                    _ => 1,
                };
                Some((stack, count))
            })
            .collect(),
        _ => Vec::new(),
    };
    let start = util::long(nbt, START);
    State {
        held: stack_or_none(util::get(nbt, HELD)),
        unwrapped: stack_or_none(util::get(nbt, INSERTED)),
        queued,
        start: (util::has(nbt, START) && start >= 0).then_some(start),
        inward: !util::has(nbt, INWARD) || util::boolean(nbt, INWARD),
        cooldown_until: util::long(nbt, COOLDOWN),
        powered: util::boolean(nbt, ACTIVE),
    }
}

/// `PackagerBlockEntity.write`, as the entries that differ from `nbt`.
fn diff(nbt: &Compound, state: &State<ItemStack>, sign: Option<&str>) -> Vec<(&'static str, Tag)> {
    let stack = |s: &Option<ItemStack>| s.as_ref().map_or_else(empty, util::stack_tag);
    let queued = Tag::List(
        state
            .queued
            .iter()
            .map(|(boxed, count)| {
                let mut entry = Compound::new();
                entry.insert("item_stack".into(), util::stack_tag(boxed));
                entry.insert("count".into(), Tag::Int(*count as i32));
                Tag::Compound(entry)
            })
            .collect(),
    );
    let mut wanted = vec![
        (ACTIVE, Tag::Byte(i8::from(state.powered))),
        (INWARD, Tag::Byte(i8::from(state.inward))),
        (HELD, stack(&state.held)),
        (INSERTED, stack(&state.unwrapped)),
        (START, Tag::Long(state.start.unwrap_or(-1))),
        (COOLDOWN, Tag::Long(state.cooldown_until)),
        (QUEUED, queued),
    ];
    if let Some(sign) = sign {
        wanted.push((SIGN, Tag::String(sign.into())));
    }
    wanted.retain(|(key, value)| util::differs(nbt, key, value));
    wanted
}

/// Writes `state` into the block entity (only what differs) and keeps its slot as the tray
/// leaves it: the held box while the tray is at rest, nothing while it moves. Whether anything
/// changed.
fn store(entity: &mut ModBlockEntity, state: &State<ItemStack>, sign: Option<&str>) -> bool {
    let mut changed = false;
    let entries = diff(&entity.nbt, state, sign);
    if !entries.is_empty() {
        changed = true;
        for (key, value) in entries {
            util::put(&mut entity.nbt, key, value);
        }
    }
    let want = if state.start.is_none() { state.held.clone() } else { None };
    if entity.items.first() != Some(&want) {
        changed = true;
        entity.items = vec![want];
    }
    changed
}

fn sign_of(nbt: &Compound) -> String {
    match util::get(nbt, SIGN) {
        Some(Tag::String(text)) => text.to_string_lossy(),
        _ => String::new(),
    }
}

// --- the world ----------------------------------------------------------------------------------

/// The text of one line of a sign (`Component.getString`).
fn line_text(tag: &Tag) -> String {
    match tag {
        Tag::String(text) => {
            let text = text.to_string_lossy();
            if text.starts_with('"') || text.starts_with('{') || text.starts_with('[') {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                    return json_text(&value);
                }
            }
            text
        }
        Tag::Compound(_) => tag.get("text").map(line_text).unwrap_or_default(),
        _ => String::new(),
    }
}

fn json_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Object(map) => {
            let mut text = map.get("text").map(json_text).unwrap_or_default();
            if let Some(serde_json::Value::Array(extra)) = map.get("extra") {
                for part in extra {
                    text.push_str(&json_text(part));
                }
            }
            text
        }
        serde_json::Value::Array(parts) => parts.iter().map(json_text).collect(),
        _ => String::new(),
    }
}

/// The lines of one side of a sign's saved data.
fn side_lines(data: &Tag, side: &str) -> Vec<String> {
    match data.get(side).and_then(|text| text.get("messages")) {
        Some(Tag::List(lines)) => lines.iter().map(line_text).collect(),
        _ => Vec::new(),
    }
}

/// `PackagerBlockEntity.updateSignAddress`: the address on the signs touching `pos`.
pub fn sign_address(host: &mut dyn Host, pos: Pos) -> String {
    let dimension = host.dimension();
    let mut signs: Vec<Option<String>> = Vec::new();
    for dir in [Dir::Down, Dir::Up, Dir::North, Dir::South, Dir::West, Dir::East] {
        let at = util::add(pos, dir.normal());
        let sign = match host.block_entity_tag(dimension, at) {
            Some(data) if data.get("front_text").is_some() => {
                rules::sign_text(&side_lines(&data, "front_text"), &side_lines(&data, "back_text"))
            }
            _ => None,
        };
        signs.push(sign);
    }
    rules::sign_address(&signs)
}

/// Where the packager looks for its inventory and the side of that block that touches it.
fn target_of(host: &dyn Host, pos: Pos) -> Option<(Pos, Dir)> {
    let facing = util::facing(host, pos, "facing")?;
    Some((util::add(pos, facing.opposite().normal()), facing))
}

fn max_stack(host: &dyn Host, stack: &ItemStack) -> u32 {
    host.max_stack_size(&stack.item).unwrap_or(64)
}

fn is_bulky(stack: &ItemStack) -> bool {
    items::is_package(stack) || stack.item.ends_with("shulker_box")
}

/// `PackagerBlockEntity.attemptToSend(null)`: packs the inventory behind into a box and starts
/// it out (or queues it). Returns whether a box was made.
fn attempt_to_send(host: &mut dyn Host, pos: Pos, state: &mut State<ItemStack>, now: i64, sign: &str, repackager: bool) -> bool {
    if !state.can_send(now) || !state.queued.is_empty() && repackager {
        return false;
    }
    let Some((target, face)) = target_of(&*host, pos) else {
        return false;
    };
    if repackager {
        return super::repackager::attempt(host, pos, target, face, state, now, sign);
    }
    pack_from(host, target, face, state, now, sign, None)
}

/// `PackagerBlockEntity.attemptToSend(queuedRequests)`: the Stock Ticker's, Redstone Requester's and
/// Factory Gauge's orders, as the network rules planned them for this packager. The first request is
/// served (finished ones leave the list), and the box is addressed and carries its place in the order.
/// Returns whether a box was made; a request that finds nothing is dropped, as Create does.
pub(crate) fn attempt_requests(host: &mut dyn Host, pos: Pos, requests: &mut Vec<package_rules::Request<ItemStack>>) -> bool {
    let now = util::game_time(&*host);
    let (Some(entity), Some((target, face))) = (host.mod_block_entity(pos), target_of(&*host, pos)) else {
        requests.clear();
        return false;
    };
    let mut state = read(&entity.nbt);
    let sent = pack_from(host, target, face, &mut state, now, "", Some(requests));
    if sent {
        host.change_mod_block_entity(pos, &mut |e| {
            store(e, &state, None);
        });
        host.block_entity_sync(pos);
    }
    sent
}

/// The packing of `attempt_to_send` (the redstone way, `requests` none) and of `attempt_requests`.
fn pack_from(
    host: &mut dyn Host,
    target: Pos,
    face: Dir,
    state: &mut State<ItemStack>,
    now: i64,
    sign: &str,
    mut requests: Option<&mut Vec<package_rules::Request<ItemStack>>>,
) -> bool {
    let Some(held) = target_inv::read(host, target) else {
        if let Some(requests) = requests.as_deref_mut()
            && !requests.is_empty()
        {
            requests.remove(0);
        }
        return false;
    };
    let slots: Vec<Option<Slot<ItemStack>>> = held
        .iter()
        .map(|(_, stack)| {
            Some(Slot {
                key: items::key(stack),
                count: stack.count,
                max: max_stack(&*host, stack),
                bulky: is_bulky(stack),
                package: items::is_package(stack),
            })
        })
        .collect();
    let Some(packed) = package_rules::pack(&slots, requests.as_deref_mut()) else {
        return false;
    };
    // Take what was decided; the box holds what actually came out.
    let mut contents = empty_contents::<ItemStack>();
    let mut taken_package: Option<ItemStack> = None;
    for (index, count) in &packed.taken {
        let (slot, stack) = &held[*index];
        let got = target_inv::take(host, target, face, *slot, *count, stack);
        if got == 0 {
            continue;
        }
        let max = max_stack(&*host, stack);
        package_rules::insert_stacked(&mut contents, &items::key(stack), got, max);
        if packed.package_slot == Some(*index) {
            let mut boxed = stack.clone();
            boxed.count = 1;
            taken_package = Some(boxed);
        }
    }
    if contents.iter().all(Option::is_none) {
        return false;
    }
    let mut created = match taken_package {
        // A package taken alone goes out as it is.
        Some(boxed) => boxed,
        None => items::containing(&*host, &contents),
    };
    items::clear_address(&*host, &mut created);
    // `if (fixedAddress != null) addAddress; if (requestQueue) setOrder; if (!requestQueue && !sign.isBlank()) addAddress(sign)`.
    if let Some(address) = &packed.address {
        items::set_address(&*host, &mut created, address);
    }
    if let Some(order) = &packed.order {
        items::set_order(&*host, &mut created, order);
    }
    if requests.is_none() && !rules_blank(sign) {
        items::set_address(&*host, &mut created, sign);
    }
    state.send(created, now);
    true
}

fn rules_blank(text: &str) -> bool {
    package_rules::is_blank(text)
}

/// `PackagerBlockEntity.unwrapBox(box, false)`: unpacks a box into the inventory behind (the
/// Re-Packager puts it in whole). The state gets the box going in, or keeps it held when it
/// couldn't go. Whether it went.
fn unwrap(host: &mut dyn Host, pos: Pos, state: &mut State<ItemStack>, boxed: &ItemStack, now: i64, repackager: bool) -> bool {
    let Some((target, face)) = target_of(&*host, pos) else {
        return false;
    };
    let ok = if repackager {
        target_inv::insert_whole(host, target, face, boxed)
    } else {
        let contents = items::contents_of(boxed);
        // `items.isEmpty()`: an empty package is accepted without a word.
        contents.is_empty() || target_inv::unpack(host, target, face, &contents, pos) == target_inv::Unpack::Done
    };
    if ok {
        let mut one = boxed.clone();
        one.count = 1;
        state.begin_unwrap(one, now);
    }
    ok
}

/// Whether the packager at `pos` takes a box now (`PackagerItemHandler.insertItem`'s guards).
fn accepts_now(host: &dyn Host, pos: Pos) -> bool {
    let now = util::game_time(host);
    host.mod_block_entity(pos).is_some_and(|entity| read(&entity.nbt).accepts(now))
}

/// A box handed in from outside (a belt, a click): unpacks it, or leaves it held when it can't
/// be.
fn receive_at(host: &mut dyn Host, pos: Pos, boxed: &ItemStack) -> bool {
    let now = util::game_time(host);
    let Some(entity) = host.mod_block_entity(pos) else {
        return false;
    };
    let mut state = read(&entity.nbt);
    if !state.accepts(now) {
        return false;
    }
    let repackager = host.block(pos) == REPACKAGER;
    if !unwrap(host, pos, &mut state, boxed, now, repackager) {
        return false;
    }
    host.change_mod_block_entity(pos, &mut |e| {
        store(e, &state, None);
    });
    host.block_entity_sync(pos);
    true
}

// --- the hooks ----------------------------------------------------------------------------------

impl Hooks for Packagers {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        for model in [HATCH_CLOSED, HATCH_OPEN, TRAY, TRAY_DEFRAG] {
            if !event.models.iter().any(|m| m == model) {
                event.models.push(model.to_owned());
            }
        }
    }

    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        is_ours(event.block).then(|| (event.block.to_owned(), Compound::new()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if is_ours(event.block) {
            event.ticks = true;
        }
    }

    /// The one slot is the held box (`PackagerItemHandler`): reachable from every side.
    fn block_entity_container(&self, _: &dyn Host, event: &mut BlockEntityContainerEvent<'_>) -> Option<ModContainer> {
        is_ours(event.block).then(|| ModContainer {
            slots: 1,
            faces: [(); 6].map(|()| vec![0]),
            inputs: vec![0],
            outputs: vec![0],
            menu: None,
        })
    }

    /// `PackagerItemHandler.insertItem` and `isItemValid`: packages only, one at a time, while
    /// nothing is held, waiting or moving.
    fn container_accepts(&self, host: &dyn Host, event: &mut ContainerAcceptsEvent<'_>) -> Flow {
        if !is_ours(&event.entity.id) {
            return Flow::Continue;
        }
        let now = util::game_time(host);
        let state = read(&event.entity.nbt);
        if items::is_package(event.stack) && state.accepts(now) && event.entity.items.first().is_none_or(Option::is_none) {
            Flow::Continue
        } else {
            Flow::Cancel
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if !is_ours(&event.entity.id) {
            return;
        }
        let pos = event.pos;
        let repackager = event.entity.id == REPACKAGER;
        let now = util::game_time(host);
        let mut state = read(&event.entity.nbt);
        let mut sign: Option<String> = None;

        // A box put in or taken out of the slot by something else (a hopper, a funnel).
        if state.start.is_none() {
            let slot = event.entity.items.first().cloned().flatten();
            let held_now = state.held.clone();
            match (&slot, &held_now) {
                (Some(boxed), held) if held.as_ref() != Some(boxed) => {
                    if items::is_package(boxed) && state.queued.is_empty() {
                        let boxed = boxed.clone();
                        if !unwrap(host, pos, &mut state, &boxed, now, repackager) {
                            // Nowhere to put it: it stays in the tray, to be taken out again.
                            state.held = Some(boxed);
                        }
                    } else {
                        // Not a package (a stacked or foreign item): thrown back out.
                        let stray = boxed.clone();
                        util::spawn_item(host, util::centre(pos), &stray, [0.0; 3], 10);
                    }
                    event.entity.items = vec![None];
                }
                (None, Some(_)) => state.held = None,
                _ => {}
            }
        }

        let tick = state.tick(now);
        match tick.cue {
            Some(Cue::Packager) => {
                host.play_sound("create:packager", util::centre(pos), 2, 1.0, 1.0);
            }
            Some(Cue::Trapdoor) => {
                host.play_sound("minecraft:block.iron_trapdoor.close", util::centre(pos), 2, 0.25, 0.75);
            }
            None => {}
        }

        // Redstone: the rising edge of the neighbour signal sends, a held one sends every 10 ticks.
        // `PackagerBlock.neighborChanged`: the signal the neighbour event left (`signals`).
        let signal = crate::signals::read(host, &mut event.entity.nbt, pos) > 0;
        let block_powered = host.block_property(pos, "powered").as_deref() == Some("true");
        // `redstoneModeActive`: a packager with a Stock Link on it ignores redstone (it is `linked`).
        let linked = host.block_property(pos, "linked").as_deref() == Some("true");
        match rules::edge(block_powered, signal) {
            Edge::Rising => {
                host.set_block_property(pos, "powered", "true");
                if !linked {
                    let address = sign_address(host, pos);
                    attempt_to_send(host, pos, &mut state, now, &address, repackager);
                    sign = Some(address);
                    state.activate(now);
                }
            }
            Edge::Falling => {
                host.set_block_property(pos, "powered", "false");
            }
            Edge::None => {}
        }
        // `lazyTick`.
        let salt = (i64::from(pos[0]) * 31 + i64::from(pos[1]) * 17 + i64::from(pos[2]) * 7).rem_euclid(rules::LAZY_PERIOD);
        if (now + salt) % rules::LAZY_PERIOD == 0 && state.powered {
            state.powered = signal;
            if (state.powered && !linked) || repackager {
                let address = sign_address(host, pos);
                attempt_to_send(host, pos, &mut state, now, &address, repackager);
                sign = Some(address);
            }
        }
        // The Re-Packager works without a signal (`redstoneModeActive` is always true).
        if repackager && (now + salt) % rules::LAZY_PERIOD == 0 {
            let address = sign_address(host, pos);
            attempt_to_send(host, pos, &mut state, now, &address, true);
            sign = Some(address);
        }

        let sign_text = sign.as_deref().filter(|text| *text != sign_of(&event.entity.nbt));
        if store(event.entity, &state, sign_text) || tick.changed {
            host.block_entity_sync(pos);
        }
        // `getAvailableItems` -> `submitNewArrivals`: what came into the inventory is told to the promise queues waiting for it.
        if !repackager {
            super::logistics::check_arrivals(host, pos, now);
        }
        if tick.finished
            && let Some(wake) = WAKE_FROGS.get()
        {
            wake(host, util::add(pos, Dir::Up.normal()));
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        if !is_ours(&event.entity.id) {
            return None;
        }
        let nbt = &event.entity.nbt;
        let mut tag = Compound::new();
        for key in [HELD, INSERTED] {
            tag.insert(key.into(), util::get(nbt, key).cloned().unwrap_or_else(empty));
        }
        tag.insert(START.into(), Tag::Long(if util::has(nbt, START) { util::long(nbt, START) } else { -1 }));
        tag.insert(INWARD.into(), Tag::Byte(i8::from(!util::has(nbt, INWARD) || util::boolean(nbt, INWARD))));
        Some(tag)
    }

    fn block_entity_render_bounding_box(&self, _: &dyn Host, event: &mut BlockEntityRenderBoundingBoxEvent<'_>) {
        if is_ours(event.block) {
            *event.bounds = [-1.0, -1.0, -1.0, 2.0, 2.0, 2.0];
        }
    }

    /// `PackagerRenderer`: the hatch, the tray sliding out and the box on it.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if !is_ours(event.block) {
            return;
        }
        let state = read(&event.entity.nbt);
        let now = event.draw.game_time as i64;
        let partial = event.draw.partial;
        let Some(facing) = util::facing(host, event.pos, "facing") else {
            return;
        };
        let draw = &mut *event.draw;
        let towards = facing.opposite();
        let normal = towards.normal().map(|n| n as f32);
        let angle = match towards {
            Dir::South => 0.0,
            Dir::East => 90.0,
            Dir::West => -90.0,
            Dir::North => 180.0,
            _ => 0.0,
        };
        let offset = state.tray_offset(now, partial);
        const CENTRE: [f32; 3] = [0.5; 3];
        const UP: [f32; 3] = [0.0, 1.0, 0.0];

        // The hatch sits at the face towards the inventory, half a texel out.
        draw.push();
        draw.translate(normal.map(|n| n * 0.49999));
        draw.rotate_about(UP, angle, CENTRE);
        if matches!(towards, Dir::Up | Dir::Down) {
            draw.rotate_about([1.0, 0.0, 0.0], if towards == Dir::Up { -90.0 } else { 90.0 }, CENTRE);
        }
        host.draw_model(draw, if state.hatch_open(now) { HATCH_OPEN } else { HATCH_CLOSED }, 0xFF_FFFF);
        draw.pop();

        let block = host.block(event.pos);
        draw.push();
        draw.translate(normal.map(|n| n * offset));
        draw.rotate_about(UP, angle, CENTRE);
        host.draw_model(draw, if block == REPACKAGER { TRAY_DEFRAG } else { TRAY }, 0xFF_FFFF);
        draw.pop();

        if let Some(boxed) = state.rendered_box(now) {
            draw.push();
            draw.translate(normal.map(|n| n * offset));
            draw.translate([0.5, 0.5 + 2.0 / 16.0, 0.5]);
            draw.rotate_about(UP, angle, [0.0; 3]);
            draw.scale([1.49; 3]);
            host.draw_item(draw, boxed, DisplayContext::Fixed, None, 0);
            draw.pop();
        }
        draw.still();
    }

    /// `PackagerBlock.getStateForPlacement` (the part that needs no neighbours): the way the
    /// player looks. The inventory it turns its back on is found when it is placed.
    fn state_for_placement(&self, level: &dyn LevelHost, event: &mut StateForPlacementEvent<'_>) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != PACKAGER && &*context.item != REPACKAGER {
            return None;
        }
        let order = rules::nearest_looking(context.yaw, context.pitch);
        let facing = rules::preferred_facing(order, &|_| false, context.sneaking);
        level.state_with_property(event.state, "facing", facing.name()).map(Some)
    }

    /// `PackagerBlock.getStateForPlacement`'s inventory search, and `POWERED` from the neighbour
    /// signal.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if &*event.stack.item != PACKAGER && &*event.stack.item != REPACKAGER {
            return;
        }
        let pos = event.pos;
        let Some(player) = host.acting_player() else {
            return;
        };
        let order = rules::nearest_looking(player.yaw, player.pitch);
        let mut found = Vec::new();
        for face in order {
            let at = util::add(pos, face.normal());
            let block = host.block(at);
            found.push(!target_inv::is_packager(&block) && inv::is_inventory(host, at));
        }
        let index = |dir: Dir| order.iter().position(|d| *d == dir).unwrap_or(0);
        let facing = rules::preferred_facing(order, &|dir| found[index(dir)], player.sneaking);
        if util::facing(&*host, pos, "facing") != Some(facing) {
            host.set_block_property(pos, "facing", facing.name());
        }
        let dimension = host.dimension();
        if host.neighbor_signal(&dimension, pos) > 0 {
            host.set_block_property(pos, "powered", "true");
        }
    }

    /// A broken packager drops the box in its tray and the ones waiting (`destroy`); a box in
    /// the slot drops with the container.
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        let pos = event.pos;
        if !is_ours(&host.block(pos)) {
            return Flow::Continue;
        }
        let Some(entity) = host.mod_block_entity(pos) else {
            return Flow::Continue;
        };
        let state = read(&entity.nbt);
        let at = util::centre(pos);
        if state.start.is_some()
            && let Some(held) = &state.held
        {
            util::spawn_item(host, at, held, [0.0; 3], 10);
        }
        for (boxed, count) in &state.queued {
            for _ in 0..*count {
                util::spawn_item(host, at, boxed, [0.0; 3], 10);
            }
        }
        Flow::Continue
    }

    /// `PackagerBlock.useItemOn` with an empty hand: the held box goes to the player.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if !is_ours(&host.block(pos)) {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        // A click with something in either hand is the item's (`use_item_on`).
        if host.held_stack(player.uuid).is_some() || host.player_equipment(player.uuid, EquipmentSlot::OffHand).is_some() {
            return Flow::Continue;
        }
        if !util::first_click(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        take_held(host, pos, player.uuid);
        Flow::Cancel
    }

    /// `PackagerBlock.useItemOn`: a package is unpacked, anything else but the Wrench and the
    /// stock parts is refused.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        if !is_ours(&host.block(pos)) {
            return Flow::Continue;
        }
        let stack = event.stack;
        if &*stack.item == util::WRENCH {
            return util::wrench(host, pos, |host| {
                let clicked = Dir::from_normal(event.face).map(Dir::axis);
                if let (Some(facing), Some(clicked)) = (util::facing(&*host, pos, "facing"), clicked) {
                    let next = create_kinetics::rules::creative_crate::wrenched(facing, clicked);
                    if next != facing {
                        host.set_block_property(pos, "facing", next.name());
                    }
                }
            });
        }
        // The Stock Link, the Factory Gauge and the Frogport are placed against it.
        if matches!(&*stack.item, "create:stock_link" | "create:factory_gauge" | "create:package_frogport") {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.sneaking || player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        if !util::first_click(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        let now = util::game_time(&*host);
        let Some(entity) = host.mod_block_entity(pos) else {
            return Flow::Cancel;
        };
        let state = read(&entity.nbt);
        if state.held.is_none() {
            if state.animating(now) {
                return Flow::Cancel;
            }
            if items::is_package(stack) && receive_at(host, pos, stack) {
                items::shrink_held(player.uuid, &stack.item);
                host.play_sound("create:depot_plop", util::centre(pos), 2, 1.0, 1.0);
            }
            return Flow::Cancel;
        }
        if !state.animating(now) {
            take_held(host, pos, player.uuid);
        }
        Flow::Cancel
    }
}

/// The held box goes to the player (`placeItemBackInInventory`).
fn take_held(host: &mut dyn Host, pos: Pos, uuid: u128) {
    let now = util::game_time(&*host);
    let Some(entity) = host.mod_block_entity(pos) else {
        return;
    };
    let mut state = read(&entity.nbt);
    let Some(boxed) = state.extract(now) else {
        return;
    };
    host.change_mod_block_entity(pos, &mut |e| {
        store(e, &state, None);
    });
    host.block_entity_sync(pos);
    if let Some(rest) = host.give_stack(uuid, boxed) {
        util::spawn_item(host, util::centre(pos), &rest, [0.0; 3], 10);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(item: &str) -> ItemStack {
        ItemStack { item: item.into(), count: 1, damage: 0, extra: None }
    }

    #[test]
    fn a_state_survives_being_saved_and_read() {
        let mut state: State<ItemStack> = State::default();
        state.held = Some(stack("create:cardboard_package_12x12"));
        state.unwrapped = Some(stack("create:cardboard_package_10x8"));
        state.queued = vec![(stack("create:cardboard_package_12x10"), 3)];
        state.start = Some(42);
        state.inward = false;
        state.cooldown_until = 99;
        state.powered = true;
        let mut nbt = Compound::new();
        for (key, value) in diff(&nbt, &state, Some("Farm")) {
            util::put(&mut nbt, key, value);
        }
        assert_eq!(read(&nbt), state);
        assert_eq!(sign_of(&nbt), "Farm");
        // Nothing differs now.
        assert!(diff(&nbt, &state, Some("Farm")).is_empty());
    }

    #[test]
    fn an_empty_entity_reads_as_an_idle_tray() {
        let state = read(&Compound::new());
        assert_eq!(state, State::default());
    }

    #[test]
    fn sign_lines_are_read_from_strings_and_text_components() {
        assert_eq!(line_text(&Tag::String("Farm".into())), "Farm");
        assert_eq!(line_text(&Tag::String("\"Mill\"".into())), "Mill");
        assert_eq!(line_text(&Tag::String("{\"text\":\"Mine\"}".into())), "Mine");
        let mut compound = Compound::new();
        compound.insert("text".into(), Tag::String("Pit".into()));
        assert_eq!(line_text(&Tag::Compound(compound)), "Pit");
    }
}
