//! The Mechanical Belt in the game: placing it with the Belt Connector, turning it with its
//! network, carrying dropped stacks, and drawing it. The rules are `create_kinetics::belt`, and
//! for a slope `rules::belt_slope`, `belt_look` and `belt_shapes` (Belts III).
//!
//! Ported from Create 6.0.10 (MIT): `BeltConnectorItem`, `BeltBlock`, `BeltBlockEntity`,
//! `BeltInventory` and `BeltRenderer`. Every block of a belt has a block entity; the first
//! (the controller, `Index` 0) keeps the items and moves them. Clients get the items with the
//! game time they were sent at and move them on themselves, so a moving belt is not re-sent
//! every tick, only when an item comes, goes or stops differently.

use std::collections::{BTreeMap, HashMap};
use std::hash::{BuildHasher, Hasher};
use std::sync::Mutex;

use create_kinetics::belt::{
    self, Chain, Connection, Crossing, Ending, Insertion, Inventory, Item, Part, Processing, Slope,
    Surroundings, chain_speed, clockwise, connect, entry, foresee, movement_facing, movement_speed,
    part_at, rotation_axis_of, segment_position,
};
use create_kinetics::rules::{belt_carry, belt_look, belt_slope};
use create_kinetics::{Axis, Dir, Kind, Networks, Node, Pos};
use crate::compat::item_data::{decode_components, encode_components};
use crate::compat::{
    BlockEntityDraw, Compound, DisplayContext, EquipmentSlot, Flow, Host, ItemStack,
    ModBlockEntity, Tag,
};

use crate::{CREATIVE, SPECTATOR, depot, float, int, press, set_float};

pub const ID: &str = "create:belt";
pub const CONNECTOR: &str = "create:belt_connector";
const SHAFT: &str = "create:shaft";
const SPEED: &str = crate::SPEED;
const INDEX: &str = "Index";
const LENGTH: &str = "Length";
const INVENTORY: &str = "Inventory";
const ITEMS: &str = "Items";
/// Stacks other blocks handed to this belt since it last ticked (`BeltInventory.toInsert`); the
/// controller puts them on at the start of its next tick.
const INCOMING: &str = "Incoming";
/// `BeltBlockEntity.casing`, as the ordinal of `CasingType`: 0 none, 1 andesite, 2 brass.
pub(crate) const CASING: &str = "Casing";
/// `BeltBlockEntity.color`, the `DyeColor`'s name (`NBTHelper.writeEnum`); absent when undyed.
pub(crate) const DYE: &str = "Dye";
const POSITIVE: &str = "PositiveOrder";
/// The end the items last met (`Ending` as a byte), so clients stop them where the server does.
const ENDING: &str = "Ending";
/// The game time the saved positions are from.
const TICK: &str = "Tick";
/// `Block.UPDATE_ALL`.
const UPDATE_ALL: i32 = 3;
/// `SoundSource` ordinals.
const BLOCKS: usize = 4;
const PLAYERS: usize = 7;
/// `ItemEntity.setDefaultPickUpDelay`.
const PICKUP_DELAY: i16 = 10;

const CENTRE: [f32; 3] = [0.5; 3];
const UP: [f32; 3] = [0.0, 1.0, 0.0];
const ORIGIN: [f32; 3] = [0.0; 3];
const PULLEY: &str = "create:block/belt_pulley";
/// Names every sprite a belt is drawn shifted onto (the scrolling and dyed belt sprites and the
/// andesite casing's), which no model of Create's draws, so the game loads them: it is never
/// drawn itself. `tools/content/belts_ii.py` writes it.
pub(crate) const SPRITES: &str = "create:block/belt_scroll_sprites";
/// The models the belt's own block entity draws: every part of its body on every slope
/// (`belt_look::body_models`) and the pulley. The casing's are `belt_look::casing_models`.
pub fn models() -> Vec<String> {
    let mut models = vec![SPRITES.to_owned(), PULLEY.to_owned()];
    models.extend(belt_look::body_models());
    models
}

/// The first shaft each player clicked with a Belt Connector, standing in for Create's
/// `BELT_FIRST_SHAFT` item component.
static FIRST_SHAFT: Mutex<BTreeMap<u128, (String, Pos)>> = Mutex::new(BTreeMap::new());

fn first_shafts() -> std::sync::MutexGuard<'static, BTreeMap<u128, (String, Pos)>> {
    FIRST_SHAFT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn get<'m, K: Ord + for<'a> From<&'a str>>(nbt: &'m BTreeMap<K, Tag>, key: &str) -> Option<&'m Tag> {
    nbt.get(&K::from(key))
}

fn long<K: Ord + for<'a> From<&'a str>>(nbt: &BTreeMap<K, Tag>, key: &str) -> Option<i64> {
    match get(nbt, key)? {
        Tag::Long(value) => Some(*value),
        _ => None,
    }
}

fn random() -> u64 {
    std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish()
}

fn centre(pos: Pos) -> [f64; 3] {
    pos.map(|v| f64::from(v) + 0.5)
}

/// `ItemStack.CODEC`: id, count, components.
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
    Some(ItemStack {
        item: id.as_str().into(),
        count,
        damage,
        extra,
    })
}

/// `Direction.get3DDataValue`.
fn dir_id(dir: Dir) -> i32 {
    Dir::ALL.iter().position(|d| *d == dir).unwrap_or(1) as i32
}

/// `TransportedItemStack.serializeNBT`, less the processing fields this slice has no use for.
fn item_tag(item: &Item<ItemStack>) -> Tag {
    let mut tag = Compound::new();
    tag.insert("Item".into(), stack_tag(&item.stack));
    set_float(&mut tag, "Pos", item.pos);
    set_float(&mut tag, "Offset", item.side);
    tag.insert("InSegment".into(), Tag::Int(item.inserted_at));
    tag.insert("Angle".into(), Tag::Int(item.angle));
    tag.insert("InDirection".into(), Tag::Int(dir_id(item.inserted_from)));
    if item.locked {
        tag.insert("Locked".into(), Tag::Byte(1));
    }
    if item.at_tunnel {
        tag.insert("AtTunnel".into(), Tag::Byte(1));
    }
    Tag::Compound(tag)
}

fn item_of(tag: &Tag) -> Option<Item<ItemStack>> {
    let Tag::Compound(nbt) = tag else {
        return None;
    };
    Some(Item {
        stack: stack_of(get(nbt, "Item")?)?,
        pos: float(nbt, "Pos").unwrap_or(0.0),
        side: float(nbt, "Offset").unwrap_or(0.0),
        angle: int(nbt, "Angle"),
        inserted_at: int(nbt, "InSegment"),
        inserted_from: Dir::ALL
            .get(int(nbt, "InDirection") as usize)
            .copied()
            .unwrap_or(Dir::Up),
        locked: int(nbt, "Locked") != 0,
        at_tunnel: int(nbt, "AtTunnel") != 0,
    })
}

/// `BeltInventory.read`.
fn read_inventory<K: Ord + for<'a> From<&'a str>>(nbt: &BTreeMap<K, Tag>) -> Inventory<ItemStack> {
    let Some(Tag::Compound(inventory)) = get(nbt, INVENTORY) else {
        return Inventory::default();
    };
    let items = match get(inventory, ITEMS) {
        Some(Tag::List(items)) => items.iter().filter_map(item_of).collect(),
        _ => Vec::new(),
    };
    Inventory {
        items,
        positive: int(inventory, POSITIVE) != 0,
    }
}

/// The stacks waiting to be put on the belt.
fn read_incoming<K: Ord + for<'a> From<&'a str>>(nbt: &BTreeMap<K, Tag>) -> Vec<Item<ItemStack>> {
    match get(nbt, INCOMING) {
        Some(Tag::List(items)) => items.iter().filter_map(item_of).collect(),
        _ => Vec::new(),
    }
}

/// The items on the belt and the ones waiting to go on it (`getTransportedItems` plus
/// `toInsert`), for what looks at all of them.
fn read_all<K: Ord + for<'a> From<&'a str>>(nbt: &BTreeMap<K, Tag>) -> Inventory<ItemStack> {
    let mut inventory = read_inventory(nbt);
    for item in read_incoming(nbt) {
        inventory.insert(item);
    }
    inventory
}

/// `BeltInventory.write`.
fn inventory_tag(inventory: &Inventory<ItemStack>) -> Tag {
    let mut tag = Compound::new();
    tag.insert(
        ITEMS.into(),
        Tag::List(inventory.items.iter().map(item_tag).collect()),
    );
    tag.insert(POSITIVE.into(), Tag::Byte(i8::from(inventory.positive)));
    Tag::Compound(tag)
}

fn game_time(host: &dyn Host) -> i64 {
    host.level(host.dimension())
        .and_then(|level| level.game_time(host.new_host()))
        .unwrap_or(0)
}

/// One block of a belt as its state says (`BeltBlock`): which way it faces, on what slope, and
/// which part of the chain it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Seg {
    pub facing: Dir,
    pub slope: Slope,
    pub part: Part,
}

/// The belt block at `pos` on any slope, or none for another block.
pub(crate) fn seg(host: &dyn Host, pos: Pos) -> Option<Seg> {
    if host.block(pos) != ID {
        return None;
    }
    Some(Seg {
        facing: host.block_property(pos, "facing").as_deref().and_then(Dir::from_name)?,
        slope: host.block_property(pos, "slope").as_deref().and_then(Slope::from_name)?,
        part: host.block_property(pos, "part").as_deref().and_then(Part::from_name)?,
    })
}

/// The horizontal belt block at `pos`: its facing and part, or none for another block or another
/// slope. What only a flat belt has (a tunnel, a funnel, a processor above) asks this.
fn segment(host: &dyn Host, pos: Pos) -> Option<(Dir, Part)> {
    let s = seg(host, pos)?;
    (s.slope == Slope::Horizontal).then_some((s.facing, s.part))
}

/// The neighbour `forward` (or back) of a block is the next block of the same belt.
fn linked(host: &dyn Host, pos: Pos, s: Seg, forward: bool) -> bool {
    let Some(next) = belt_slope::next_segment(pos, s.facing, s.slope, s.part, forward) else {
        return true;
    };
    matches!(seg(host, next), Some(n) if n.facing == s.facing && n.slope == s.slope
        && if forward { n.part != Part::Start } else { n.part != Part::End })
}

/// `BeltBlock.initBelt`: the block's index from the start and the belt's length.
fn locate(host: &dyn Host, pos: Pos, s: Seg) -> Option<(i32, i32)> {
    let walk = |forward: bool| {
        let (mut at, mut part, mut steps) = (pos, s.part, 0);
        while let Some(next) = belt_slope::next_segment(at, s.facing, s.slope, part, forward) {
            let n = seg(host, next).filter(|n| n.facing == s.facing && n.slope == s.slope)?;
            (at, part, steps) = (next, n.part, steps + 1);
            if steps > 1000 {
                return None;
            }
        }
        Some(steps)
    };
    let index = walk(false)?;
    let after = walk(true)?;
    Some((index, index + after + 1))
}

/// The belt the block at `pos` is part of, as its block entity and its controller's say (length
/// 0 where the belt isn't set up yet).
pub(crate) fn chain_of(host: &dyn Host, pos: Pos) -> Option<Chain> {
    let s = seg(host, pos)?;
    let index = int(&host.mod_block_entity(pos)?.nbt, INDEX);
    let controller = Chain::controller_of(pos, s.facing, s.slope, index);
    let length = int(&host.mod_block_entity(controller)?.nbt, LENGTH);
    Some(Chain { controller, facing: s.facing, slope: s.slope, length })
}

/// Whether a block can be built over (`BlockState.canBeReplaced`), for the ones a belt meets.
fn replaceable(host: &dyn Host, block: &str) -> bool {
    matches!(block, "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air")
        || host
            .block_name_in_tag(block, "minecraft:replaceable")
            .unwrap_or(false)
}

/// `Block.isFaceFull` of the collision boxes of the block at `pos` on its side `side`.
pub(crate) fn face_full(host: &dyn Host, pos: Pos, side: Dir) -> bool {
    const CELLS: usize = 16;
    let axis = side.axis().index();
    let (u, v) = match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    let mut covered = [[false; CELLS]; CELLS];
    for b in host.block_collision(pos) {
        let touches = if side.sign() > 0 {
            b[axis + 3] >= 1.0 - 1e-6
        } else {
            b[axis] <= 1e-6
        };
        if !touches {
            continue;
        }
        let cells = |lo: f64, hi: f64| {
            let lo = (lo.max(0.0) * CELLS as f64 + 1e-6).floor() as usize;
            let hi = (hi.min(1.0) * CELLS as f64 - 1e-6).ceil() as usize;
            lo..hi.min(CELLS)
        };
        for i in cells(b[u], b[u + 3]) {
            for j in cells(b[v], b[v + 3]) {
                covered[i][j] = true;
            }
        }
    }
    covered.iter().flatten().all(|c| *c)
}

/// A block of another module that takes the items a belt hands it
/// (`DirectBeltInputBehaviour`): the chutes and the funnels. A module adds its blocks with
/// [`register_input`] when it starts, so this file doesn't name it.
#[derive(Clone, Copy)]
pub(crate) struct BeltInput {
    /// Whether the block named is one of its blocks.
    pub claims: fn(&str) -> bool,
    /// `canInsertFromSide`: whether the block at the position takes items moving towards the side.
    pub can_insert: fn(&dyn Host, Pos, Dir) -> bool,
    /// `handleInsertion`: gives the item, moving towards the side, to the block at the position
    /// (`simulate`: only asks). What is left of the stack, count 0 where it all went.
    pub insert: fn(&mut dyn Host, Pos, Dir, &Item<ItemStack>, bool) -> ItemStack,
}

static EXTRA_INPUTS: Mutex<Vec<BeltInput>> = Mutex::new(Vec::new());

#[allow(dead_code)] // the chute and funnel modules call it; a build without them doesn't
pub(crate) fn register_input(input: BeltInput) {
    EXTRA_INPUTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(input);
}

/// The registered input that claims `block`.
fn extra_input(block: &str) -> Option<BeltInput> {
    EXTRA_INPUTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .find(|input| (input.claims)(block))
        .copied()
}

/// What the Andesite and Brass Tunnels do over a belt (`BeltTunnelInteractionHandler`): the module
/// of the tunnels adds it with [`register_tunnels`] when it starts, so this file doesn't name it.
#[derive(Clone, Copy)]
pub(crate) struct TunnelHandler {
    /// `flapTunnelsAndCheckIfStuck`: an item moving towards `movement` crosses from the belt block
    /// at `from` into the one at `to`; the tunnels sit one block above each.
    pub cross: fn(&mut dyn Host, &mut Item<ItemStack>, Pos, Pos, Dir) -> Crossing,
    /// `flapTunnel` at the end of the belt: an item left it past the belt block at `last`, moving
    /// towards `movement`.
    pub left: fn(&mut dyn Host, Pos, Dir),
    /// `BeltBlock.entityInside`: a tunnel over the belt block at the position, whose items an item
    /// entity dropped there doesn't join.
    pub covers: fn(&dyn Host, Pos) -> bool,
}

static TUNNELS: Mutex<Option<TunnelHandler>> = Mutex::new(None);

#[allow(dead_code)] // the tunnel module calls it; a build without it doesn't
pub(crate) fn register_tunnels(handler: TunnelHandler) {
    *TUNNELS.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(handler);
}

fn tunnels() -> Option<TunnelHandler> {
    *TUNNELS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The horizontal belt block at `pos` as a tunnel over it sees it (`BeltHelper.getSegmentBE`):
/// the way it faces, its speed in RPM and the way its top moves (`getMovementFacing`).
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // the tunnel module's
pub(crate) struct Below {
    pub facing: Dir,
    pub rpm: f32,
    pub movement: Dir,
}

#[allow(dead_code)] // the tunnel module's
pub(crate) fn below(host: &dyn Host, pos: Pos) -> Option<Below> {
    let (facing, _) = segment(host, pos)?;
    let rpm = host
        .mod_block_entity(pos)
        .and_then(|e| float(&e.nbt, SPEED))
        .unwrap_or(0.0);
    Some(Below { facing, rpm, movement: movement_facing(rpm, facing) })
}

/// Something that keeps the stopped item of a belt block where it is (`TransportedItemStack.
/// lockedExternally`): the Mechanical Arm, while it moves to take the item. Asked of a locked item
/// at the belt block's position; a module adds its own with [`register_hold`] when it starts.
pub(crate) type Hold = fn(&dyn Host, Pos) -> bool;

static HOLDS: Mutex<Vec<Hold>> = Mutex::new(Vec::new());

#[allow(dead_code)] // the arm module calls it; a build without it doesn't
pub(crate) fn register_hold(hold: Hold) {
    let mut all = HOLDS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // `hooks()` may run more than once.
    if !all.iter().any(|h| *h as usize == hold as usize) {
        all.push(hold);
    }
}

fn held_externally(host: &dyn Host, at: Pos) -> bool {
    let all: Vec<Hold> = HOLDS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    all.iter().any(|hold| hold(host, at))
}

/// A block two above a belt or a depot that holds, changes or takes the items that come to the
/// middle of the block under it (`BeltProcessingBehaviour`), other than the press: the Spout.
/// A module adds its block with [`register_processor`] when it starts, so this file doesn't
/// name it.
#[derive(Clone, Copy)]
pub(crate) struct BeltProcessor {
    /// Whether the block named is its block.
    pub claims: fn(&str) -> bool,
    /// `handleReceivedItem`: an item came to the middle of the block two under the processor at
    /// the position.
    pub received: fn(&mut dyn Host, Pos, &ItemStack) -> Processing,
    /// `handleHeldItem`: whether it goes on holding the item it caught.
    pub held: fn(&mut dyn Host, Pos, &ItemStack) -> Processing,
}

static PROCESSORS: Mutex<Vec<BeltProcessor>> = Mutex::new(Vec::new());

#[allow(dead_code)] // the spout module calls it; a build without it doesn't
pub(crate) fn register_processor(processor: BeltProcessor) {
    let mut all = PROCESSORS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // `hooks()` may run more than once.
    if !all.iter().any(|p| p.claims as usize == processor.claims as usize) {
        all.push(processor);
    }
}

/// The registered processor that claims `block`.
pub(crate) fn extra_processor(block: &str) -> Option<BeltProcessor> {
    PROCESSORS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .find(|processor| (processor.claims)(block))
        .copied()
}

/// Blocks that take items from a belt (`DirectBeltInputBehaviour`): a horizontal belt, a depot,
/// a basin, an upright saw, and what other modules register (chutes, funnels).
fn takes_belt_input(host: &dyn Host, pos: Pos) -> bool {
    match host.block(pos).as_str() {
        ID => seg(host, pos).is_some(),
        depot::ID | "create:basin" => true,
        // An upright saw takes items onto its blade (`SawBlockEntity`'s `DirectBeltInputBehaviour`).
        SAW => crate::util::facing(host, pos, "facing") == Some(Dir::Up),
        other => extra_input(other).is_some(),
    }
}

const SAW: &str = "create:mechanical_saw";

/// `BeltInventory.resolveEnding`, without the funnels.
fn resolve_ending(host: &dyn Host, chain: Chain, positive: bool) -> Ending {
    let next = chain.at(if positive { chain.length } else { -1 });
    if takes_belt_input(host, next) {
        return Ending::Insert;
    }
    let moving = if positive { chain.facing } else { chain.facing.opposite() };
    if face_full(host, next, moving.opposite()) {
        Ending::Blocked
    } else {
        Ending::Eject
    }
}

/// `BeltInventory.eject`: the stack flies off the belt where it lies, along the belt: up a slope
/// going up it, down going down (`getBeltChainDirection`).
fn eject(host: &mut dyn Host, chain: Chain, rpm: f32, item: &Item<ItemStack>) {
    let mut out = belt_slope::vector_for_offset(chain, item.pos);
    let speed = f64::from(movement_speed(rpm).abs().max(1.0 / 8.0));
    let along = belt_slope::movement_direction(rpm, chain.facing, chain.slope, Part::Start, true, true)
        .map(f64::from);
    let motion = [along[0] * speed, along[1] * speed + 1.0 / 8.0, along[2] * speed];
    let norm = motion.iter().map(|m| m * m).sum::<f64>().sqrt();
    for i in 0..3 {
        out[i] += motion[i] / norm * 0.001;
    }
    out[1] += 6.0 / 16.0;
    spawn_item(host, out, &item.stack, motion);
}

fn spawn_item(host: &mut dyn Host, at: [f64; 3], stack: &ItemStack, motion: [f64; 3]) {
    let mut nbt = Compound::new();
    nbt.insert("Item".into(), stack_tag(stack));
    nbt.insert(
        "Motion".into(),
        Tag::List(motion.iter().map(|m| Tag::Double(m.to_bits())).collect()),
    );
    nbt.insert("PickupDelay".into(), Tag::Short(PICKUP_DELAY));
    let dimension = host.dimension();
    if let Some(id) = host.summon(dimension, "minecraft:item", at, nbt) {
        // Components the summon's item compound can't carry are set whole.
        host.set_item_entity(dimension, id, stack.clone());
    }
}

/// Every item the belt carries goes off where it lies (`BeltInventory.ejectAll`).
fn eject_all(host: &mut dyn Host, chain: Chain, rpm: f32, inventory: &Inventory<ItemStack>) {
    for item in &inventory.items {
        eject(host, chain, rpm, item);
    }
}

pub fn new_entity() -> (String, Compound) {
    let mut nbt = Compound::new();
    set_float(&mut nbt, SPEED, 0.0);
    nbt.insert(INDEX.into(), Tag::Int(0));
    nbt.insert(LENGTH.into(), Tag::Int(0));
    (ID.to_owned(), nbt)
}

/// The game tick a moving belt that carries things last ran, for what only matters while one does
/// (the mobs' carry listens to every mob only then).
static LAST_MOVING: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static NOW: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// The server's tick counter, advanced once a tick (`machines/belt_carry.rs`).
pub(crate) fn next_tick() {
    NOW.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Whether a belt that carries things has turned in the last few seconds.
pub(crate) fn recently_moving() -> bool {
    let now = NOW.load(std::sync::atomic::Ordering::Relaxed);
    now.saturating_sub(LAST_MOVING.load(std::sync::atomic::Ordering::Relaxed)) < 100
}

/// A belt block's server tick (`BeltBlockEntity.tick`).
pub fn tick(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity, networks: &Mutex<Networks>) {
    let Some(s) = seg(host, pos) else {
        return;
    };
    // `BeltBlock.onRemove` takes the whole belt down with one block; here each block goes
    // once its neighbour has, a block a tick. A pulley leaves its shaft behind.
    if !linked(host, pos, s, true) || !linked(host, pos, s, false) {
        if int(&entity.nbt, INDEX) == 0 && int(&entity.nbt, LENGTH) > 0 {
            let rpm = float(&entity.nbt, SPEED).unwrap_or(0.0);
            let chain = Chain {
                controller: pos,
                facing: s.facing,
                slope: s.slope,
                length: int(&entity.nbt, LENGTH),
            };
            eject_all(host, chain, rpm, &read_all(&entity.nbt));
        }
        let dimension = host.dimension();
        if s.part.has_pulley() {
            if let Some(shaft) = host.default_state(SHAFT) {
                host.set_block(dimension, pos, shaft, UPDATE_ALL);
                host.set_block_property(pos, "axis", rotation_axis_of(s.facing, s.slope).name());
            }
        } else if let Some(air) = host.default_state("minecraft:air") {
            host.set_block(dimension, pos, air, UPDATE_ALL);
        }
        return;
    }
    let mut sync = false;
    if int(&entity.nbt, LENGTH) == 0 {
        let Some((index, length)) = locate(host, pos, s) else {
            return;
        };
        entity.nbt.insert(INDEX.into(), Tag::Int(index));
        entity.nbt.insert(LENGTH.into(), Tag::Int(length));
        sync = true;
    }
    let (index, length) = (int(&entity.nbt, INDEX), int(&entity.nbt, LENGTH));

    let node = Node::passive(Kind::Belt(s.facing, s.part, s.slope));
    let report = networks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .report(host.dimension(), pos, node);
    let (rpm, breaks) = report;
    if breaks {
        if index == 0 {
            let old = float(&entity.nbt, SPEED).unwrap_or(0.0);
            let chain = Chain { controller: pos, facing: s.facing, slope: s.slope, length };
            eject_all(host, chain, old, &read_all(&entity.nbt));
        }
        host.destroy_block(pos, None, false);
        return;
    }
    if float(&entity.nbt, SPEED) != Some(rpm) {
        set_float(&mut entity.nbt, SPEED, rpm);
        sync = true;
    }
    if rpm != 0.0 && s.slope.transports() {
        LAST_MOVING.store(NOW.load(std::sync::atomic::Ordering::Relaxed), std::sync::atomic::Ordering::Relaxed);
    }
    if index == 0 {
        sync |= carry(host, pos, entity, s, length, rpm);
    }
    if sync {
        entity.nbt.insert(TICK.into(), Tag::Long(game_time(host)));
        host.block_entity_sync(pos);
    }
}

/// What the belt asks of the world while its items move (`BeltInventory.tick`): the processors
/// above (the press) and the block at the end.
struct Around<'a> {
    host: &'a mut dyn Host,
    chain: Chain,
    rpm: f32,
    positive: bool,
    ending: Ending,
}

impl Around<'_> {
    /// The block of the belt at `segment` (`BeltHelper.getPositionForOffset`).
    fn at(&self, segment: i32) -> Pos {
        self.chain.at(segment)
    }

    /// `flapTunnel(this, lastOffset, movementFacing, false)`: an item left the belt's last block.
    fn flap_last_block(&mut self) {
        // Tunnels sit on flat belts only.
        if self.chain.slope != Slope::Horizontal {
            return;
        }
        if let Some(tunnels) = tunnels() {
            let last = self.at(if self.positive { self.chain.length - 1 } else { 0 });
            (tunnels.left)(self.host, last, movement_facing(self.rpm, self.chain.facing));
        }
    }
}

impl Surroundings<ItemStack> for Around<'_> {
    fn ending(&mut self) -> Ending {
        self.ending
    }

    /// Processors, funnels and tunnels work on flat belts only (`BeltInventory.tick`'s `horizontal`).
    fn horizontal(&self) -> bool {
        self.chain.slope == Slope::Horizontal
    }

    /// `getBeltProcessingAtSegment(segment)`: the block two above.
    fn held(&mut self, segment: i32, item: &Item<ItemStack>) -> Option<Processing> {
        let at = self.at(segment);
        if held_externally(&*self.host, at) {
            return Some(Processing::Hold);
        }
        let above = [at[0], at[1] + 2, at[2]];
        if let Some(processor) = extra_processor(&self.host.block(above)) {
            return Some((processor.held)(self.host, above, &item.stack));
        }
        press::holding(self.host, above)
    }

    fn received(&mut self, segment: i32, item: &Item<ItemStack>) -> Option<Processing> {
        let at = self.at(segment);
        let above = [at[0], at[1] + 2, at[2]];
        let name = self.host.block(above);
        if let Some(processor) = extra_processor(&name) {
            if blocked(self.host, at) {
                return None;
            }
            return Some((processor.received)(self.host, above, &item.stack));
        }
        if name != press::ID || blocked(self.host, at) {
            return None;
        }
        press::offered(self.host, above, &item.stack)
    }

    /// `BeltTunnelInteractionHandler.flapTunnelsAndCheckIfStuck`.
    fn crossing(&mut self, from: i32, to: i32, item: &mut Item<ItemStack>) -> Crossing {
        if self.chain.slope != Slope::Horizontal {
            return Crossing::Pass;
        }
        let Some(tunnels) = tunnels() else {
            return Crossing::Pass;
        };
        let movement = movement_facing(self.rpm, self.chain.facing);
        let (from, to) = (self.at(from), self.at(to));
        (tunnels.cross)(self.host, item, from, to, movement)
    }

    /// `Ending.INSERT`: the item is at the end; the block past it may take it.
    fn insert(&mut self, item: &mut Item<ItemStack>) -> Insertion {
        let next = self.at(if self.positive { self.chain.length } else { -1 });
        let side = movement_facing(self.rpm, self.chain.facing);
        if can_insert_from_side(self.host, next, side) != Some(true) {
            return Insertion::Nothing;
        }
        let before = item.stack.count;
        let result = match handle_insertion(self.host, next, side, item, true, false) {
            Some(rest) if rest.count == 0 => Insertion::All,
            Some(rest) if rest.count != before => {
                item.stack = rest;
                Insertion::Part
            }
            _ => Insertion::Nothing,
        };
        if result != Insertion::Nothing {
            self.flap_last_block();
        }
        result
    }
}

/// `BeltProcessingBehaviour.isBlocked`: something solid over the block `space` (the block a
/// processor works on) leaves no room for it.
pub(crate) fn blocked(host: &dyn Host, space: Pos) -> bool {
    !host.block_collision([space[0], space[1] + 1, space[2]]).is_empty()
}

/// `DirectBeltInputBehaviour.canInsertFromSide`: whether the block at `pos` takes items that
/// arrive moving towards `side`; `None` where the block takes none from belts.
pub(crate) fn can_insert_from_side(host: &dyn Host, pos: Pos, side: Dir) -> Option<bool> {
    match host.block(pos).as_str() {
        ID => {
            let s = seg(host, pos)?;
            // `BeltBlockEntity.canInsertFrom`: a sideways or a vertical belt takes nothing.
            let rpm = host
                .mod_block_entity(pos)
                .and_then(|e| float(&e.nbt, SPEED))
                .unwrap_or(0.0);
            Some(s.slope.transports() && rpm != 0.0 && movement_facing(rpm, s.facing) != side.opposite())
        }
        depot::ID | "create:basin" => Some(true),
        SAW => Some(crate::util::facing(host, pos, "facing") == Some(Dir::Up)),
        other => extra_input(other).map(|input| (input.can_insert)(host, pos, side)),
    }
}

/// `DirectBeltInputBehaviour.handleInsertion`: hands `item`, moving towards `side`, to the block
/// at `pos`. `from_belt` says a belt was carrying it (`prevBeltPosition != 0`). Answers what was
/// left (count 0: all of it went in), or `None` where the block takes nothing from belts. A
/// basin can't be asked first, so `simulate` leaves it out (the stack comes back whole).
pub(crate) fn handle_insertion(
    host: &mut dyn Host,
    pos: Pos,
    side: Dir,
    item: &Item<ItemStack>,
    from_belt: bool,
    simulate: bool,
) -> Option<ItemStack> {
    match host.block(pos).as_str() {
        ID => belt_insert(host, pos, side, item, from_belt, simulate),
        depot::ID => Some(depot::insert_from_side(host, pos, side, item, simulate)),
        "create:basin" => {
            if simulate {
                return Some(item.stack.clone());
            }
            let face = crate::util::direction(side.opposite());
            Some(host.insert_item(pos, face, item.stack.clone()).unwrap_or_else(|| {
                let mut none = item.stack.clone();
                none.count = 0;
                none
            }))
        }
        SAW => Some(crate::machines::saw::insert_from_side(host, pos, &item.stack, simulate)),
        other => extra_input(other).map(|input| (input.insert)(host, pos, side, item, simulate)),
    }
}

/// An `ItemHandler` insertion into the block at `pos` as a basin's spout or a hopper makes it
/// (`ItemHandlerBeltSegment.insertItem`, `DepotItemHandler.insertItem`): answers what was left
/// (count 0: all of it), or `None` where `pos` is neither a belt nor a depot. This is the path
/// a basin's spout should use for a belt or a depot.
#[allow(dead_code)]
pub(crate) fn handler_insert(
    host: &mut dyn Host,
    pos: Pos,
    stack: &ItemStack,
    simulate: bool,
) -> Option<ItemStack> {
    match host.block(pos).as_str() {
        ID => {
            let s = seg(host, pos)?;
            // No item handler on a belt that only turns (`BeltBlockEntity.registerCapabilities`).
            if !s.slope.transports() {
                return None;
            }
            let whole = stack.clone();
            let index = host.mod_block_entity(pos).map(|e| int(&e.nbt, INDEX))?;
            let controller = Chain::controller_of(pos, s.facing, s.slope, index);
            let ctl = host.mod_block_entity(controller)?;
            if int(&ctl.nbt, LENGTH) == 0 {
                return Some(whole);
            }
            let inventory = read_all(&ctl.nbt);
            if !inventory.can_insert_at(index) {
                return Some(whole);
            }
            // `ItemHelper.limitCountToMaxStackSize`.
            let limit = host.max_stack_size(&whole.item).unwrap_or(64);
            let mut taken = whole.clone();
            taken.count = whole.count.min(limit);
            let mut rest = whole;
            rest.count -= taken.count;
            if !simulate {
                let item = Item::dropped(taken, index, (random() % 360) as i32, inventory.positive);
                push_incoming(host, controller, vec![item]);
            }
            Some(rest)
        }
        depot::ID => Some(depot::handler_insert(host, pos, stack, simulate)),
        _ => None,
    }
}

fn push_incoming(host: &mut dyn Host, controller: Pos, items: Vec<Item<ItemStack>>) {
    host.change_mod_block_entity(controller, &mut |entity| {
        let mut list = match get(&entity.nbt, INCOMING) {
            Some(Tag::List(list)) => list.clone(),
            _ => Vec::new(),
        };
        list.extend(items.iter().map(item_tag));
        entity.nbt.insert(INCOMING.into(), Tag::List(list));
    });
}

/// `BeltBlockEntity.tryInsertingFromSide` for the belt block at `pos`.
fn belt_insert(
    host: &mut dyn Host,
    pos: Pos,
    side: Dir,
    item: &Item<ItemStack>,
    from_belt: bool,
    simulate: bool,
) -> Option<ItemStack> {
    let s = seg(host, pos)?;
    let facing = s.facing;
    let whole = item.stack.clone();
    // `tryInsertingFromSide`: `canTransportObjects`.
    if !s.slope.transports() {
        return Some(whole);
    }
    let Some(entity) = host.mod_block_entity(pos) else {
        return Some(whole);
    };
    let rpm = float(&entity.nbt, SPEED).unwrap_or(0.0);
    let index = int(&entity.nbt, INDEX);
    // `isOccupied`: a belt that is stopped, not set up, or moving towards the side the item
    // comes from takes nothing.
    if int(&entity.nbt, LENGTH) == 0 || rpm == 0.0 {
        return Some(whole);
    }
    let movement = movement_facing(rpm, facing);
    if movement == side.opposite() {
        return Some(whole);
    }
    let controller = Chain::controller_of(pos, s.facing, s.slope, index);
    let Some(ctl) = host.mod_block_entity(controller) else {
        return Some(whole);
    };
    if !read_all(&ctl.nbt).can_insert_at_from_side(index, side, movement) {
        return Some(whole);
    }
    let mut none = whole.clone();
    none.count = 0;
    if simulate {
        return Some(none);
    }
    let [dx, dy, dz] = movement.opposite().normal();
    let behind = [pos[0] + dx, pos[1] + dy, pos[2] + dz];
    let placed = entry(
        index,
        chain_speed(rpm, facing),
        movement,
        side,
        item.side,
        from_belt,
        host.block(behind) == ID,
    );
    let new = Item {
        stack: whole,
        pos: placed.pos,
        side: placed.side,
        angle: item.angle,
        inserted_at: index,
        inserted_from: side,
        locked: false,
        at_tunnel: false,
    };
    push_incoming(host, controller, vec![new]);
    Some(none)
}

/// `TransportedItemStackHandlerBehaviour.handleCenteredProcessingOnAllItems(.5, ...)` with a
/// removing result, as a chute pulling from a belt does it: the first stack on the belt block at
/// `pos` (within half a block of its middle) that `accept` takes is taken off the belt.
#[allow(dead_code)] // the chute module's
pub(crate) fn take_centered(
    host: &mut dyn Host,
    pos: Pos,
    accept: &mut dyn FnMut(&ItemStack) -> bool,
) -> Option<ItemStack> {
    let s = seg(host, pos)?;
    let index = host.mod_block_entity(pos).map(|e| int(&e.nbt, INDEX))?;
    let controller = Chain::controller_of(pos, s.facing, s.slope, index);
    let ctl = host.mod_block_entity(controller)?;
    let mut inventory = read_inventory(&ctl.nbt);
    let middle = index as f32 + 0.5;
    let at = inventory
        .items
        .iter()
        .position(|i| (i.pos - middle).abs() < 0.5 && accept(&i.stack))?;
    let item = inventory.items.remove(at);
    host.change_mod_block_entity(controller, &mut |entity| {
        entity.nbt.insert(INVENTORY.into(), inventory_tag(&inventory));
    });
    host.block_entity_sync(controller);
    Some(item.stack)
}

/// Where the items of the belt block at `pos` are, for a block that works on them as they pass
/// (a belt funnel: `BeltFunnelInteractionHandler`).
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // the funnel module's
pub(crate) struct Lane {
    /// The belt block's index along the belt (`BeltBlockEntity.index`) and the belt's length.
    pub index: i32,
    pub length: i32,
    /// `beltMovementPositive`: the items move towards the belt's end.
    pub positive: bool,
    /// Blocks an item moves each tick (the belt's `chain_speed`), negative towards its start.
    pub speed: f32,
    /// The way the belt's top moves (`getMovementFacing`).
    pub movement: Dir,
}

/// Lets `edit` change the items of the belt that the block at `pos` is part of, with the stacks
/// left empty dropped afterwards: it is given the world, where the belt block is, and the items
/// (the one furthest along first). When it answers true the controller keeps the result and
/// clients are sent it. True when the belt was changed.
#[allow(dead_code)] // the funnel module's
pub(crate) fn edit_items(
    host: &mut dyn Host,
    pos: Pos,
    edit: &mut dyn FnMut(&mut dyn Host, &Lane, &mut Vec<Item<ItemStack>>) -> bool,
) -> bool {
    let Some(s) = seg(host, pos) else {
        return false;
    };
    let facing = s.facing;
    let Some(entity) = host.mod_block_entity(pos) else {
        return false;
    };
    let index = int(&entity.nbt, INDEX);
    let controller = Chain::controller_of(pos, s.facing, s.slope, index);
    let Some(ctl) = host.mod_block_entity(controller) else {
        return false;
    };
    let length = int(&ctl.nbt, LENGTH);
    let rpm = float(&ctl.nbt, SPEED).unwrap_or(0.0);
    if length == 0 || rpm == 0.0 {
        return false;
    }
    let mut inventory = read_inventory(&ctl.nbt);
    if inventory.items.is_empty() {
        return false;
    }
    let speed = chain_speed(rpm, facing);
    let lane = Lane {
        index,
        length,
        positive: speed > 0.0,
        speed,
        movement: movement_facing(rpm, facing),
    };
    if !edit(host, &lane, &mut inventory.items) {
        return false;
    }
    inventory.items.retain(|item| item.stack.count > 0);
    host.change_mod_block_entity(controller, &mut |entity| {
        entity.nbt.insert(INVENTORY.into(), inventory_tag(&inventory));
    });
    host.block_entity_sync(controller);
    true
}

/// The stack a processor above holds in the middle of the block of the belt at `pos`
/// (`TransportedItemStackHandlerBehaviour.applyToEachWithin(index + .5, .51, ...)` for a locked
/// item): `None` when none is held there.
#[allow(dead_code)] // the spout module
pub(crate) fn locked_stack(host: &dyn Host, pos: Pos) -> Option<ItemStack> {
    let (facing, _) = segment(host, pos)?;
    let index = host.mod_block_entity(pos).map(|e| int(&e.nbt, INDEX))?;
    let controller = segment_position(pos, facing, -index);
    let ctl = host.mod_block_entity(controller)?;
    let middle = index as f32 + 0.5;
    read_inventory(&ctl.nbt)
        .items
        .iter()
        .find(|i| i.locked && (i.pos - middle).abs() < 0.51)
        .map(|i| i.stack.clone())
}

/// What the press does at the bottom of its cycle to the item it holds on the block of the belt
/// at `pos` (`BeltPressingCallbacks.whenItemHeld`): `results` gives what the press makes of a
/// stack, or `None` where it takes none. The held item goes, what it makes comes in its place and
/// stays held until the press lets go; what a press that works one item at a time left over
/// waits to be taken in its turn. True when an item was pressed.
pub(crate) fn process_held(
    host: &mut dyn Host,
    pos: Pos,
    results: &mut dyn FnMut(&mut dyn Host, &ItemStack) -> Option<Vec<ItemStack>>,
) -> bool {
    let Some((facing, _)) = segment(host, pos) else {
        return false;
    };
    let Some(index) = host.mod_block_entity(pos).map(|e| int(&e.nbt, INDEX)) else {
        return false;
    };
    let controller = segment_position(pos, facing, -index);
    let Some(ctl) = host.mod_block_entity(controller) else {
        return false;
    };
    let mut inventory = read_inventory(&ctl.nbt);
    let middle = index as f32 + 0.5;
    // `applyToEachWithin(index + .5, .51, ...)`.
    let Some(at) = inventory
        .items
        .iter()
        .position(|i| i.locked && (i.pos - middle).abs() < 0.51)
    else {
        return false;
    };
    let item = inventory.items.remove(at);
    let (take, left) = create_kinetics::press::belt_split(item.stack.count);
    let mut probe = item.stack.clone();
    probe.count = take;
    let Some(made) = results(host, &probe) else {
        // `PASS`: the press takes nothing of it after all, so it goes on.
        let mut free = item;
        free.locked = false;
        inventory.items.insert(at, free);
        host.change_mod_block_entity(controller, &mut |entity| {
            entity.nbt.insert(INVENTORY.into(), inventory_tag(&inventory));
        });
        return false;
    };
    let mut incoming = Vec::new();
    for stack in made {
        let mut out = item.clone();
        out.stack = stack;
        out.locked = true;
        out.angle = (random() % 360) as i32;
        incoming.push(out);
    }
    if left > 0 {
        // `held.beltPosition = (int) position + .5f - (positive ? 1 / 512f : -1 / 512f)`.
        let mut rest = item.clone();
        rest.stack.count = left;
        rest.locked = false;
        rest.pos = middle - if inventory.positive { 1.0 / 512.0 } else { -1.0 / 512.0 };
        incoming.push(rest);
    }
    host.change_mod_block_entity(controller, &mut |entity| {
        entity.nbt.insert(INVENTORY.into(), inventory_tag(&inventory));
        let mut list = match get(&entity.nbt, INCOMING) {
            Some(Tag::List(list)) => list.clone(),
            _ => Vec::new(),
        };
        list.extend(incoming.iter().map(item_tag));
        entity.nbt.insert(INCOMING.into(), Tag::List(list));
    });
    true
}

/// What a Deployer does at the end of its stroke to the item held in the middle of the block of
/// the belt at `pos` (`BeltDeployerCallbacks.activate`): `results` gives what applying its recipe
/// makes of ONE of the held stack (`copyWithCount(1)`), or `None` where there is no recipe. What
/// it makes goes on the belt unlocked, where the item was; the rest of the stack stays held where
/// it was, a hair behind the middle (`convertToAndLeaveHeld`). True when a recipe was applied.
#[allow(dead_code)] // the deployer module
pub(crate) fn apply_held(
    host: &mut dyn Host,
    pos: Pos,
    results: &mut dyn FnMut(&mut dyn Host, &ItemStack) -> Option<Vec<ItemStack>>,
) -> bool {
    let Some((facing, _)) = segment(host, pos) else {
        return false;
    };
    let Some(index) = host.mod_block_entity(pos).map(|e| int(&e.nbt, INDEX)) else {
        return false;
    };
    let controller = segment_position(pos, facing, -index);
    let Some(ctl) = host.mod_block_entity(controller) else {
        return false;
    };
    let mut inventory = read_inventory(&ctl.nbt);
    let middle = index as f32 + 0.5;
    // `applyToEachWithin(index + .5, .51, ...)`.
    let Some(at) = inventory
        .items
        .iter()
        .position(|i| i.locked && (i.pos - middle).abs() < 0.51)
    else {
        return false;
    };
    let mut probe = inventory.items[at].stack.clone();
    probe.count = 1;
    let Some(made) = results(host, &probe) else {
        return false;
    };
    let item = inventory.items.remove(at);
    let mut incoming = Vec::new();
    for stack in made {
        let mut out = item.clone();
        out.stack = stack;
        out.locked = false;
        out.angle = (random() % 360) as i32;
        incoming.push(out);
    }
    let mut left = item.clone();
    left.stack.count = left.stack.count.saturating_sub(1);
    if left.stack.count > 0 {
        // With something made, the rest waits where `held.beltPosition` puts it; with nothing
        // made it simply stays (`convertTo(left)`).
        if !incoming.is_empty() {
            left.pos = middle - if inventory.positive { 1.0 / 512.0 } else { -1.0 / 512.0 };
        }
        incoming.push(left);
    }
    host.change_mod_block_entity(controller, &mut |entity| {
        entity.nbt.insert(INVENTORY.into(), inventory_tag(&inventory));
        let mut list = match get(&entity.nbt, INCOMING) {
            Some(Tag::List(list)) => list.clone(),
            _ => Vec::new(),
        };
        list.extend(incoming.iter().map(item_tag));
        entity.nbt.insert(INCOMING.into(), Tag::List(list));
    });
    true
}

/// The controller's share of the tick: stacks other blocks handed over get on, stacks dropped on
/// the belt get on, then everything moves (`BeltBlock.entityInside`,
/// `ItemHandlerBeltSegment.insertItem`, `BeltInventory.tick`). True when clients need the items
/// again.
fn carry(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity, s: Seg, length: i32, rpm: f32) -> bool {
    let facing = s.facing;
    let chain = Chain { controller: pos, facing, slope: s.slope, length };
    // `BeltBlock.initBelt`: a belt that only turns lets go of anything it holds.
    if !chain.transports() {
        let held = read_all(&entity.nbt);
        let had = get(&entity.nbt, INVENTORY).is_some() && !held.items.is_empty();
        if !held.items.is_empty() {
            eject_all(host, chain, rpm, &held);
        }
        if had {
            entity.nbt.insert(INVENTORY.into(), inventory_tag(&Inventory::default()));
            crate::util::remove(&mut entity.nbt, INCOMING);
        }
        return had;
    }
    let had = get(&entity.nbt, INVENTORY).is_some();
    let mut inventory = read_inventory(&entity.nbt);
    let mut changed = false;
    let mut sync = false;

    // `toInsert`, at the start of `BeltInventory.tick`.
    let incoming = read_incoming(&entity.nbt);
    if !incoming.is_empty() {
        for item in incoming {
            inventory.insert(item);
        }
        crate::util::remove(&mut entity.nbt, INCOMING);
        changed = true;
        sync = true;
    }

    // Item entities in any block of the belt, not rising.
    let dimension = host.dimension();
    let last = chain.at(length - 1);
    let min = [0, 1, 2].map(|i| f64::from(pos[i].min(last[i])));
    let max = [0, 1, 2].map(|i| f64::from(pos[i].max(last[i]) + 1));
    for found in host.item_entities(dimension, min, max) {
        let rising = host
            .entity(found.id)
            .and_then(|e| host.entity_velocity(e))
            .is_some_and(|v| v[1] > 0.0);
        if rising {
            continue;
        }
        let block = found.position.map(|v| v.floor() as i32);
        let Some(offset) = (0..length).find(|&offset| chain.at(offset) == block) else {
            continue;
        };
        if !inventory.can_insert_at(offset) {
            continue;
        }
        if tunnels().is_some_and(|t| (t.covers)(&*host, block)) {
            continue;
        }
        // `ItemHelper.limitCountToMaxStackSize`: one stack's worth gets on.
        let limit = host.max_stack_size(&found.stack.item).unwrap_or(64);
        let taking = found.stack.count.min(limit);
        let mut stack = found.stack.clone();
        stack.count = taking;
        let mut rest = found.stack.clone();
        rest.count -= taking;
        let angle = (random() % 360) as i32;
        inventory.insert(Item::dropped(stack, offset, angle, inventory.positive));
        host.set_item_entity(dimension, found.id, rest);
        changed = true;
        sync = true;
    }

    let speed = chain_speed(rpm, facing);
    if !inventory.items.is_empty() {
        let positive = if speed != 0.0 {
            speed > 0.0
        } else {
            inventory.positive
        };
        let ending = resolve_ending(host, chain, positive);
        if get(&entity.nbt, ENDING).is_none() || int(&entity.nbt, ENDING) != i32::from(ending.id()) {
            entity.nbt.insert(ENDING.into(), Tag::Byte(ending.id()));
            sync = true;
        }
        let mut around = Around {
            host: &mut *host,
            chain,
            rpm,
            positive,
            ending,
        };
        let moved = inventory.tick_with(speed, length, &mut around);
        for item in &moved.ejected {
            eject(host, chain, rpm, item);
            if s.slope == Slope::Horizontal
                && let Some(tunnels) = tunnels()
            {
                let last = chain.at(if positive { length - 1 } else { 0 });
                (tunnels.left)(host, last, movement_facing(rpm, facing));
            }
        }
        changed |= moved.moved || moved.reordered;
        sync |= moved.reordered;
    }
    if changed || (!had && sync) {
        entity.nbt.insert(INVENTORY.into(), inventory_tag(&inventory));
        // The positions are as of now: clients move them on from here.
        entity.nbt.insert(TICK.into(), Tag::Long(game_time(host)));
    }
    sync
}

pub fn update_tag(entity: &ModBlockEntity) -> Compound {
    let mut tag = Compound::new();
    set_float(&mut tag, SPEED, float(&entity.nbt, SPEED).unwrap_or(0.0));
    for key in [INDEX, LENGTH] {
        tag.insert(key.into(), Tag::Int(int(&entity.nbt, key)));
    }
    if int(&entity.nbt, INDEX) == 0 {
        let inventory = get(&entity.nbt, INVENTORY)
            .cloned()
            .unwrap_or_else(|| inventory_tag(&Inventory::default()));
        tag.insert(INVENTORY.into(), inventory);
        tag.insert(ENDING.into(), Tag::Byte(int(&entity.nbt, ENDING) as i8));
        tag.insert(TICK.into(), Tag::Long(long(&entity.nbt, TICK).unwrap_or(0)));
    }
    // Always sent: a client that had a casing or a dye must hear it is gone.
    tag.insert(CASING.into(), Tag::Int(int(&entity.nbt, CASING)));
    let dye = match get(&entity.nbt, DYE) {
        Some(Tag::String(name)) => name.to_string_lossy(),
        _ => String::new(),
    };
    tag.insert(DYE.into(), Tag::String(dye.as_str().into()));
    tag
}

/// `BeltBlockEntity.casing` of the belt block at `pos`: 0 none, 1 andesite, 2 brass.
pub(crate) fn casing(host: &dyn Host, pos: Pos) -> i32 {
    host.mod_block_entity(pos)
        .map_or(0, |entity| int(&entity.nbt, CASING))
}

/// `BeltBlockEntity.setCasingType` for the belt block at `pos`: the state's `casing` follows.
pub(crate) fn set_casing(host: &mut dyn Host, pos: Pos, kind: i32) {
    if casing(host, pos) == kind {
        return;
    }
    host.change_mod_block_entity(pos, &mut |entity| {
        entity.nbt.insert(CASING.into(), Tag::Int(kind));
    });
    host.set_block_property(pos, "casing", if kind != 0 { "true" } else { "false" });
    host.block_entity_sync(pos);
}

/// `BeltBlockEntity.applyColor`: dyes the whole belt `color` (a `DyeColor` name), or washes it
/// with `None`. False, changing nothing, when it already is that colour.
pub(crate) fn apply_color(host: &mut dyn Host, pos: Pos, color: Option<&str>) -> bool {
    let Some(chain) = chain_of(host, pos) else {
        return false;
    };
    let controller = chain.controller;
    let current = match host.mod_block_entity(controller).and_then(|e| get(&e.nbt, DYE).cloned()) {
        Some(Tag::String(name)) => Some(name.to_string_lossy()),
        _ => None,
    };
    if current.as_deref() == color {
        return false;
    }
    for offset in 0..chain.length {
        let at = chain.at(offset);
        host.change_mod_block_entity(at, &mut |entity| match color {
            Some(name) => {
                entity.nbt.insert(DYE.into(), Tag::String(name.into()));
            }
            None => {
                crate::util::remove(&mut entity.nbt, DYE);
            }
        });
        host.block_entity_sync(at);
    }
    true
}

/// The controller draws past its own block: its items ride the whole belt
/// (`BeltBlockEntity.createRenderBoundingBox` inflates by the length).
pub fn bounds(facing: Dir, slope: Slope, entity: &ModBlockEntity, bounds: &mut [f32; 6]) {
    if int(&entity.nbt, INDEX) != 0 {
        return;
    }
    let length = int(&entity.nbt, LENGTH).max(1);
    let each = belt_slope::step(facing, slope);
    let far = each.map(|v| (v * (length - 1)) as f32);
    for i in 0..3 {
        bounds[i] = bounds[i].min(far[i] - 1.0);
        bounds[i + 3] = bounds[i + 3].max(far[i] + 2.0);
    }
}

/// The belt's colour as saved (`Dye`, `DyeColor.name()`); none when undyed.
fn dye_of(entity: &ModBlockEntity) -> Option<String> {
    match get(&entity.nbt, DYE) {
        // The update tag sends an empty name for no colour.
        Some(Tag::String(name)) => Some(name.to_string_lossy()).filter(|name| !name.is_empty()),
        _ => None,
    }
}

/// `BeltRenderer.renderSafe`, then its items. Which sprite a half scrolls over and how far
/// (`AllSpriteShifts`), and the casing's sprite, are `belt::belt_shift` and `belt::casing_shift`;
/// which models and turns a slope takes are `belt_look`.
pub fn draw(
    host: &dyn Host,
    draw: &mut BlockEntityDraw,
    facing: Dir,
    slope: Slope,
    part: Part,
    entity: &ModBlockEntity,
) {
    let rpm = float(&entity.nbt, SPEED).unwrap_or(0.0);
    // `AnimationTickHolder.getRenderTime`.
    let time = draw.game_time as f64 + f64::from(draw.partial);
    let dye = dye_of(entity);
    let body = belt_look::body(facing, slope, part);
    // `center().rotateY().rotateZ().rotateX().uncenter()` for the belt's own models.
    for bottom in [true, false] {
        let shift = belt::belt_shift(rpm, facing, slope, bottom, dye.as_deref(), time);
        draw.push();
        draw.rotate_about(UP, body.y, CENTRE);
        if body.z != 0.0 {
            draw.rotate_about([0.0, 0.0, 1.0], body.z, CENTRE);
        }
        if body.x != 0.0 {
            draw.rotate_about([1.0, 0.0, 0.0], body.x, CENTRE);
        }
        let model = belt_look::partial(body.diagonal, body.start, body.end, bottom);
        crate::draw::draw_shifted(host, draw, &model, shift.as_ref());
        draw.pop();
        // Diagonal belts do not have a separate bottom model.
        if body.diagonal {
            break;
        }
    }
    // A cased belt block has its casing round it (`belt_casing/*`, the block's own model in
    // Create), in the brass sprite or, shifted, the andesite one.
    let casing = belt::Casing::from_ordinal(int(&entity.nbt, CASING));
    if casing != belt::Casing::None {
        // The blockstate turns x then y, clockwise.
        let pose = belt_look::casing(facing, slope, part);
        draw.push();
        draw.rotate_about(UP, -(pose.y as f32), CENTRE);
        if pose.x != 0 {
            draw.rotate_about([1.0, 0.0, 0.0], -(pose.x as f32), CENTRE);
        }
        crate::draw::draw_shifted(host, draw, &pose.model, belt::casing_shift(casing).as_ref());
        draw.pop();
    }
    if part.has_pulley() {
        let pulley = belt_look::pulley(facing, slope);
        crate::draw::spin(draw, pulley.axis.unit(), rpm * 0.3, 0.0, float(&entity.nbt, crate::ponder::ANGLE));
        draw.push();
        if pulley.y != 0.0 {
            draw.rotate_about(UP, pulley.y, CENTRE);
        }
        draw.rotate_about([1.0, 0.0, 0.0], pulley.x, CENTRE);
        host.draw_model(draw, PULLEY, 0xFF_FFFF);
        draw.pop();
        draw.still();
    }
    let drew_items = int(&entity.nbt, INDEX) == 0 && draw_items(host, draw, facing, slope, rpm, entity);
    // Moving items and a scrolling texture read the time; a still belt without items is drawn
    // once (a new speed arrives as an update tag, which drops what was kept).
    if !drew_items && !belt::scrolls(rpm) {
        draw.keep();
    }
}

/// `BeltRenderer.renderItems`: the controller draws every item where the server will have
/// moved it by now, up the slope and tilted on it.
fn draw_items(
    host: &dyn Host,
    draw: &mut BlockEntityDraw,
    facing: Dir,
    slope: Slope,
    rpm: f32,
    entity: &ModBlockEntity,
) -> bool {
    let inventory = read_inventory(&entity.nbt);
    if inventory.items.is_empty() {
        return false;
    }
    let length = int(&entity.nbt, LENGTH).max(1);
    let margin = Ending::from_id(int(&entity.nbt, ENDING) as i8).margin();
    let since = (draw.game_time - long(&entity.nbt, TICK).unwrap_or(draw.game_time)) as f32
        + draw.partial;
    let positions: Vec<f32> = inventory.items.iter().map(|i| i.pos).collect();
    let locked: Vec<bool> = inventory.items.iter().map(|i| i.locked || i.at_tunnel).collect();
    let drawn = foresee(&positions, &locked, chain_speed(rpm, facing), since, length, margin);
    let n = facing.normal().map(|v| v as f32);
    let along_x = clockwise(facing).axis() == Axis::X;
    for (item, pos) in inventory.items.iter().zip(drawn) {
        let pose = belt_slope::item_pose(facing, slope, length, pos);
        draw.push();
        draw.translate([0.5 - n[0] * 0.5, 15.0 / 16.0, 0.5 - n[2] * 0.5]);
        draw.translate(nudge(item.angle));
        draw.translate([n[0] * pos, pose.vertical, n[2] * pos]);
        let side = if along_x { item.side } else { -item.side };
        draw.translate(if along_x { [side, 0.0, 0.0] } else { [0.0, 0.0, side] });
        if pose.on_slope {
            draw.rotate_about(pose.tilt_axis.unit(), pose.tilt, ORIGIN);
            draw.translate([0.0, pose.lift, 0.0]);
        }
        draw_stack(host, draw, &item.stack, item.angle);
        draw.pop();
    }
    true
}

/// `BeltRenderer.renderItem`'s copies: a block in a small heap, anything else flat in a fan.
fn draw_stack(host: &dyn Host, draw: &mut BlockEntityDraw, stack: &ItemStack, angle: i32) {
    // `Mth.log2`, rounded down, halved. Create draws the copies only within 16 blocks.
    let copies = stack.count.max(1).ilog2() / 2;
    // Stands in for `BakedModel.isGui3d`: an item that is also a block.
    let block = host.default_state(&stack.item).is_some();
    let mut r = JavaRandom::new(i64::from(angle));
    for i in 0..=copies {
        let i = i as f32;
        draw.push();
        draw.rotate_about(UP, angle as f32, ORIGIN);
        if !block {
            draw.translate([0.0, -0.09375, 0.0]);
            draw.rotate_about([1.0, 0.0, 0.0], 90.0, ORIGIN);
        } else {
            let x = r.next_float() * 0.0625 * i;
            let z = r.next_float() * 0.0625 * i;
            draw.translate([x, 0.0, z]);
        }
        draw.scale([0.5; 3]);
        host.draw_item(draw, stack, DisplayContext::Fixed, None, 0);
        draw.pop();
        if !block {
            draw.rotate_about(UP, 10.0, ORIGIN);
        }
        draw.translate([0.0, if block { 1.0 / 64.0 } else { 1.0 / 16.0 }, 0.0]);
    }
}

/// `java.util.Random`, seeded as `BeltRenderer` seeds it with the item's angle.
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

/// Flywheel's `Translate.nudge`: a tiny offset so overlapping items don't flicker.
fn nudge(seed: i32) -> [f32; 3] {
    let mut bits = i64::from(seed).wrapping_mul(31).wrapping_mul(493_286_711);
    bits = bits
        .wrapping_mul(bits)
        .wrapping_mul(4_392_167_121)
        .wrapping_add(bits.wrapping_mul(98_761));
    let part = |shift: u32| (((bits >> shift & 7) as f32 + 0.5) / 8.0 - 0.5) * 0.004;
    [part(16), part(20), part(24)]
}

/// An empty main hand takes what lies on the clicked block of the belt (`BeltBlock.useItemOn`).
pub fn right_click(host: &mut dyn Host, pos: Pos) -> Flow {
    let Some(s) = seg(host, pos) else {
        return Flow::Continue;
    };
    let Some(player) = host.acting_player() else {
        return Flow::Continue;
    };
    if player.game_mode == SPECTATOR
        || player.sneaking
        || host
            .player_equipment(player.uuid, EquipmentSlot::MainHand)
            .is_some()
    {
        return Flow::Continue;
    }
    let Some(index) = host.mod_block_entity(pos).map(|e| int(&e.nbt, INDEX)) else {
        return Flow::Continue;
    };
    let controller = Chain::controller_of(pos, s.facing, s.slope, index);
    let mut taken = Vec::new();
    host.change_mod_block_entity(controller, &mut |entity| {
        let mut inventory = read_inventory(&entity.nbt);
        taken = inventory.take_within(index as f32 + 0.5, 0.55);
        if !taken.is_empty() {
            entity.nbt.insert(INVENTORY.into(), inventory_tag(&inventory));
        }
    });
    if taken.is_empty() {
        return Flow::Continue;
    }
    host.block_entity_sync(controller);
    for item in taken {
        // `Inventory.placeItemBackInInventory`: what does not fit drops at the player.
        if let Some(rest) = host.give_stack(player.uuid, item.stack) {
            spawn_item(host, player.position, &rest, [0.0; 3]);
        }
    }
    let pitch = 1.0 + (random() % 1_000_000) as f32 / 1_000_000.0;
    host.play_sound("minecraft:entity.item.pickup", centre(pos), PLAYERS, 0.2, pitch);
    Flow::Cancel
}

/// Breaking a belt by hand drops what it carried (`BeltBlock.spawnAfterBreak`). The rest of
/// the belt follows in the blocks' own ticks.
pub fn destroyed(host: &mut dyn Host, pos: Pos) {
    let Some(s) = seg(host, pos) else {
        return;
    };
    let Some((index, length)) = locate(host, pos, s) else {
        return;
    };
    let controller = Chain::controller_of(pos, s.facing, s.slope, index);
    let chain = Chain { controller, facing: s.facing, slope: s.slope, length };
    let Some(entity) = host.mod_block_entity(controller) else {
        return;
    };
    let inventory = read_all(&entity.nbt);
    if inventory.items.is_empty() {
        return;
    }
    let rpm = float(&entity.nbt, SPEED).unwrap_or(0.0);
    eject_all(host, chain, rpm, &inventory);
    host.change_mod_block_entity(controller, &mut |entity| {
        entity
            .nbt
            .insert(INVENTORY.into(), inventory_tag(&Inventory::default()));
        crate::util::remove(&mut entity.nbt, INCOMING);
    });
}

/// The Belt Connector on a block (`BeltConnectorItem.useOn`): the first shaft is remembered,
/// the second gets the belt. Sneaking forgets the first.
pub fn use_connector(host: &mut dyn Host, pos: Pos) -> Flow {
    let Some(player) = host.acting_player() else {
        return Flow::Continue;
    };
    let mut firsts = first_shafts();
    if player.sneaking {
        firsts.remove(&player.uuid);
        return Flow::Cancel;
    }
    let dimension = host.dimension().to_owned();
    let valid = host.block(pos) == SHAFT;
    // Forget a first shaft that is gone or out of reach.
    if let Some((at, first)) = firsts.get(&player.uuid).cloned() {
        let far = (0..3).map(|i| (first[i] - pos[i]).pow(2)).sum::<i32>()
            >= (belt::MAX_LENGTH * 2).pow(2);
        if at != dimension || host.block(first) != SHAFT || far {
            firsts.remove(&player.uuid);
        }
    }
    if !valid {
        return Flow::Cancel;
    }
    let Some((_, first)) = firsts.get(&player.uuid).cloned() else {
        firsts.insert(player.uuid, (dimension, pos));
        return Flow::Cancel;
    };
    // The game runs hooks again inside `set_block`: never hold the lock across it.
    drop(firsts);
    let Some(connection) = can_connect(host, first, pos) else {
        return Flow::Cancel;
    };
    create_belt(host, &connection);
    if player.game_mode != CREATIVE {
        host.take_items(CONNECTOR, 1);
    }
    first_shafts().remove(&player.uuid);
    Flow::Cancel
}

/// `BeltConnectorItem.canConnect`: the geometry (a flat, diagonal, vertical or sideways belt,
/// `belt_slope::connect`), the speeds, and the blocks between.
fn can_connect(host: &dyn Host, first: Pos, second: Pos) -> Option<Connection> {
    let axis = |pos| host.block_property(pos, "axis").as_deref().and_then(Axis::from_name);
    let shaft_axis = axis(first)?;
    let connection = connect(first, second, shaft_axis, axis(second)?).ok()?;
    let speed = |pos| {
        host.mod_block_entity(pos)
            .and_then(|e| float(&e.nbt, SPEED))
            .unwrap_or(0.0)
    };
    let (a, b) = (speed(first), speed(second));
    if a != 0.0 && b != 0.0 && a.signum() != b.signum() {
        return None;
    }
    let chain = &connection.chain;
    let between = &chain[1..chain.len() - 1];
    let clear = between.iter().all(|&at| {
        let block = host.block(at);
        (block == SHAFT && axis(at) == Some(shaft_axis)) || replaceable(host, &block)
    });
    clear.then_some(connection)
}

/// `BeltConnectorItem.createBelts`.
fn create_belt(host: &mut dyn Host, connection: &Connection) {
    let Some(state) = host.default_state(ID) else {
        return;
    };
    let chain = &connection.chain;
    let mid = {
        let (a, b) = (centre(chain[0]), centre(chain[chain.len() - 1]));
        [0, 1, 2].map(|i| (a[i] + b[i]) / 2.0)
    };
    host.play_sound("minecraft:block.wool.place", mid, BLOCKS, 0.5, 1.0);
    let dimension = host.dimension();
    let parts: HashMap<Pos, Part> = chain
        .iter()
        .enumerate()
        .map(|(i, &at)| (at, part_at(i, chain.len(), host.block(at) == SHAFT)))
        .collect();
    for &at in chain {
        host.set_block(dimension, at, state, UPDATE_ALL);
        host.set_block_property(at, "facing", connection.facing.name());
        host.set_block_property(at, "slope", connection.slope.name());
        host.set_block_property(at, "part", parts[&at].name());
    }
}

/// The belt block at `pos` as something that stands on it sees it (`BeltMovementHandler`): where it
/// is, how it faces and slopes, and how fast it turns. None for another block, or a belt that only
/// turns. The belt's speed is its block entity's.
pub(crate) fn surface(host: &dyn Host, pos: Pos) -> Option<belt_carry::Belt> {
    let s = seg(host, pos).filter(|s| s.slope.transports())?;
    let rpm = float(&host.mod_block_entity(pos)?.nbt, SPEED)?;
    Some(belt_carry::Belt { pos, facing: s.facing, slope: s.slope, part: s.part, rpm })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_round_trip_through_nbt() {
        let mut inventory = Inventory {
            items: Vec::new(),
            positive: true,
        };
        inventory.insert(Item::dropped(ItemStack::new("minecraft:cobblestone", 64), 2, 37, true));
        let mut nbt = Compound::new();
        nbt.insert(INVENTORY.into(), inventory_tag(&inventory));
        assert_eq!(read_inventory(&nbt), inventory);
    }

    #[test]
    fn held_and_waiting_items_round_trip_through_nbt() {
        let mut held = Item::dropped(ItemStack::new("minecraft:iron_ingot", 3), 1, 90, true);
        held.locked = true;
        held.inserted_from = Dir::East;
        let tag = item_tag(&held);
        assert_eq!(item_of(&tag), Some(held.clone()));
        let mut nbt = Compound::new();
        nbt.insert(INCOMING.into(), Tag::List(vec![tag]));
        assert_eq!(read_incoming(&nbt), vec![held.clone()]);
        // What waits to go on counts with what is on, for what looks at both.
        assert_eq!(read_all(&nbt).items, vec![held]);
        // A free item saves no `Locked` key.
        let free = Item::dropped(ItemStack::new("minecraft:iron_ingot", 1), 0, 10, true);
        let Tag::Compound(saved) = item_tag(&free) else { panic!() };
        assert!(!saved.contains_key(&"Locked".into()));
        assert!(!saved.contains_key(&"AtTunnel".into()));
    }

    #[test]
    fn an_item_waiting_at_a_tunnel_is_saved_so_clients_stop_it() {
        let mut waiting = Item::dropped(ItemStack::new("minecraft:iron_ingot", 3), 1, 90, true);
        waiting.at_tunnel = true;
        waiting.pos = 1.99;
        let tag = item_tag(&waiting);
        assert_eq!(item_of(&tag), Some(waiting));
    }

    #[test]
    fn directions_are_saved_by_their_data_value() {
        assert_eq!(dir_id(Dir::Up), 1);
        assert_eq!(dir_id(Dir::East), 5);
    }
}
