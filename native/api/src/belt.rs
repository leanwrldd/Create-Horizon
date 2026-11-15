//! The Mechanical Belt in the game: placing it with the Belt Connector, turning it with its
//! network, carrying dropped stacks, and drawing it. The rules are `create_kinetics::belt`.
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
    self, Ending, Inventory, Item, Part, chain_speed, clockwise, connect, foresee,
    movement_facing, movement_speed, part_at, rotation_axis, segment_position,
};
use create_kinetics::{Axis, Dir, Kind, Networks, Node, Pos};
use horizon_client::item_data::{decode_components, encode_components};
use horizon_client::mod_api::{
    BlockEntityDraw, Compound, DisplayContext, EquipmentSlot, Flow, Host, ItemStack,
    ModBlockEntity, Tag,
};

use crate::{CREATIVE, SPECTATOR, float, int, set_float};

pub const ID: &str = "create:belt";
pub const CONNECTOR: &str = "create:belt_connector";
const SHAFT: &str = "create:shaft";
const SPEED: &str = crate::SPEED;
const INDEX: &str = "Index";
const LENGTH: &str = "Length";
const INVENTORY: &str = "Inventory";
const ITEMS: &str = "Items";
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
pub const MODELS: [&str; 7] = [
    "create:block/belt/start",
    "create:block/belt/middle",
    "create:block/belt/end",
    "create:block/belt/start_bottom",
    "create:block/belt/middle_bottom",
    "create:block/belt/end_bottom",
    PULLEY,
];

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
        .and_then(|level| level.game_time(host))
        .unwrap_or(0)
}

/// The belt block at `pos`: its facing and part, or none for another block or another slope.
fn segment(host: &dyn Host, pos: Pos) -> Option<(Dir, Part)> {
    if host.block(pos) != ID || host.block_property(pos, "slope").as_deref() != Some("horizontal")
    {
        return None;
    }
    Some((
        host.block_property(pos, "facing").as_deref().and_then(Dir::from_name)?,
        host.block_property(pos, "part").as_deref().and_then(Part::from_name)?,
    ))
}

/// The neighbour `forward` (or back) of a block is the next block of the same belt.
fn linked(host: &dyn Host, pos: Pos, facing: Dir, part: Part, forward: bool) -> bool {
    let Some(next) = belt::next_segment(pos, facing, part, forward) else {
        return true;
    };
    matches!(segment(host, next), Some((f, p)) if f == facing
        && if forward { p != Part::Start } else { p != Part::End })
}

/// `BeltBlock.initBelt`: the block's index from the start and the belt's length.
fn locate(host: &dyn Host, pos: Pos, facing: Dir, part: Part) -> Option<(i32, i32)> {
    let walk = |forward: bool| {
        let (mut at, mut part, mut steps) = (pos, part, 0);
        while let Some(next) = belt::next_segment(at, facing, part, forward) {
            let (_, next_part) = segment(host, next)?;
            (at, part, steps) = (next, next_part, steps + 1);
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

/// Whether a block can be built over (`BlockState.canBeReplaced`), for the ones a belt meets.
fn replaceable(host: &dyn Host, block: &str) -> bool {
    matches!(block, "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air")
        || host
            .block_name_in_tag(block, "minecraft:replaceable")
            .unwrap_or(false)
}

/// `Block.isFaceFull` of the collision boxes of the block at `pos` on its side `side`.
fn face_full(host: &dyn Host, pos: Pos, side: Dir) -> bool {
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

/// `BeltInventory.resolveEnding`, without the funnels and belt inputs this slice lacks.
fn resolve_ending(host: &dyn Host, controller: Pos, facing: Dir, length: i32, positive: bool) -> Ending {
    let next = segment_position(controller, facing, if positive { length } else { -1 });
    let moving = if positive { facing } else { facing.opposite() };
    if face_full(host, next, moving.opposite()) {
        Ending::Blocked
    } else {
        Ending::Eject
    }
}

/// `BeltInventory.eject`: the stack flies off the belt's end as an item entity.
fn eject(host: &mut dyn Host, controller: Pos, facing: Dir, rpm: f32, item: &Item<ItemStack>) {
    let n = facing.normal().map(f64::from);
    let along = f64::from(item.pos) - 0.5;
    let c = centre(controller);
    let mut out = [c[0] + n[0] * along, c[1], c[2] + n[2] * along];
    let speed = f64::from(movement_speed(rpm).abs().max(1.0 / 8.0));
    let moving = movement_facing(rpm, facing).normal().map(f64::from);
    let motion = [moving[0] * speed, 1.0 / 8.0, moving[2] * speed];
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
fn eject_all(host: &mut dyn Host, controller: Pos, facing: Dir, rpm: f32, inventory: &Inventory<ItemStack>) {
    for item in &inventory.items {
        eject(host, controller, facing, rpm, item);
    }
}

pub fn new_entity() -> (String, Compound) {
    let mut nbt = Compound::new();
    set_float(&mut nbt, SPEED, 0.0);
    nbt.insert(INDEX.into(), Tag::Int(0));
    nbt.insert(LENGTH.into(), Tag::Int(0));
    (ID.to_owned(), nbt)
}

/// A belt block's server tick (`BeltBlockEntity.tick`).
pub fn tick(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity, networks: &Mutex<Networks>) {
    let Some((facing, part)) = segment(host, pos) else {
        return;
    };
    // `BeltBlock.onRemove` takes the whole belt down with one block; here each block goes
    // once its neighbour has, a block a tick. A pulley leaves its shaft behind.
    if !linked(host, pos, facing, part, true) || !linked(host, pos, facing, part, false) {
        if int(&entity.nbt, INDEX) == 0 && int(&entity.nbt, LENGTH) > 0 {
            let rpm = float(&entity.nbt, SPEED).unwrap_or(0.0);
            eject_all(host, pos, facing, rpm, &read_inventory(&entity.nbt));
        }
        let dimension = host.dimension();
        if part.has_pulley() {
            if let Some(shaft) = host.default_state(SHAFT) {
                host.set_block(dimension, pos, shaft, UPDATE_ALL);
                host.set_block_property(pos, "axis", rotation_axis(facing).name());
            }
        } else if let Some(air) = host.default_state("minecraft:air") {
            host.set_block(dimension, pos, air, UPDATE_ALL);
        }
        return;
    }
    let mut sync = false;
    if int(&entity.nbt, LENGTH) == 0 {
        let Some((index, length)) = locate(host, pos, facing, part) else {
            return;
        };
        entity.nbt.insert(INDEX.into(), Tag::Int(index));
        entity.nbt.insert(LENGTH.into(), Tag::Int(length));
        sync = true;
    }
    let (index, length) = (int(&entity.nbt, INDEX), int(&entity.nbt, LENGTH));

    let node = Node::passive(Kind::Belt(facing, part));
    let report = networks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .report(host.dimension(), pos, node);
    let (rpm, breaks) = report;
    if breaks {
        if index == 0 {
            let old = float(&entity.nbt, SPEED).unwrap_or(0.0);
            eject_all(host, pos, facing, old, &read_inventory(&entity.nbt));
        }
        host.destroy_block(pos, None, false);
        return;
    }
    if float(&entity.nbt, SPEED) != Some(rpm) {
        set_float(&mut entity.nbt, SPEED, rpm);
        sync = true;
    }
    if index == 0 {
        sync |= carry(host, pos, entity, facing, length, rpm);
    }
    if sync {
        entity.nbt.insert(TICK.into(), Tag::Long(game_time(host)));
        host.block_entity_sync(pos);
    }
}

/// The controller's share of the tick: stacks dropped on the belt get on, then everything moves
/// (`BeltBlock.entityInside`, `ItemHandlerBeltSegment.insertItem`, `BeltInventory.tick`).
/// True when clients need the items again.
fn carry(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity, facing: Dir, length: i32, rpm: f32) -> bool {
    let had = get(&entity.nbt, INVENTORY).is_some();
    let mut inventory = read_inventory(&entity.nbt);
    let mut changed = false;
    let mut sync = false;

    // Item entities in any block of the belt, not rising.
    let dimension = host.dimension();
    let last = segment_position(pos, facing, length - 1);
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
        let n = facing.normal();
        let offset = (0..3).map(|i| (block[i] - pos[i]) * n[i]).sum::<i32>();
        if !(0..length).contains(&offset) || segment_position(pos, facing, offset) != block {
            continue;
        }
        if !inventory.can_insert_at(offset) {
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
        let ending = resolve_ending(host, pos, facing, length, positive);
        if get(&entity.nbt, ENDING).is_none() || int(&entity.nbt, ENDING) != i32::from(ending.id()) {
            entity.nbt.insert(ENDING.into(), Tag::Byte(ending.id()));
            sync = true;
        }
        let moved = inventory.tick(speed, length, || ending);
        for item in &moved.ejected {
            eject(host, pos, facing, rpm, item);
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
    tag
}

/// The controller draws past its own block: its items ride the whole belt
/// (`BeltBlockEntity.createRenderBoundingBox` inflates by the length).
pub fn bounds(facing: Dir, entity: &ModBlockEntity, bounds: &mut [f32; 6]) {
    if int(&entity.nbt, INDEX) != 0 {
        return;
    }
    let length = int(&entity.nbt, LENGTH).max(1);
    let far = facing.normal().map(|v| (v * (length - 1)) as f32);
    for i in 0..3 {
        bounds[i] = bounds[i].min(far[i] - 1.0);
        bounds[i + 3] = bounds[i + 3].max(far[i] + 2.0);
    }
}

/// `AngleHelper.horizontalAngle`: south 0, east 90, north 180, west 270.
fn horizontal_angle(facing: Dir) -> f32 {
    match facing {
        Dir::East => 90.0,
        Dir::North => 180.0,
        Dir::West => 270.0,
        _ => 0.0,
    }
}

/// `BeltRenderer.renderSafe` for a horizontal belt without casing, then its items.
pub fn draw(host: &dyn Host, draw: &mut BlockEntityDraw, facing: Dir, part: Part, entity: &ModBlockEntity) {
    let rpm = float(&entity.nbt, SPEED).unwrap_or(0.0);
    let name = match part {
        Part::Start => "start",
        Part::End => "end",
        Part::Middle | Part::Pulley => "middle",
    };
    // Create scrolls the belt's texture as it moves; this draws it still.
    for bottom in ["_bottom", ""] {
        draw.push();
        draw.rotate_about(UP, horizontal_angle(facing), CENTRE);
        host.draw_model(draw, &format!("create:block/belt/{name}{bottom}"), 0xFF_FFFF);
        draw.pop();
    }
    if part.has_pulley() {
        let axis = rotation_axis(facing);
        draw.spin(axis.unit(), CENTRE, rpm * 0.3, 0.0);
        draw.push();
        if axis == Axis::X {
            draw.rotate_about(UP, 90.0, CENTRE);
        }
        draw.rotate_about([1.0, 0.0, 0.0], 90.0, CENTRE);
        host.draw_model(draw, PULLEY, 0xFF_FFFF);
        draw.pop();
        draw.still();
    }
    let drew_items = int(&entity.nbt, INDEX) == 0 && draw_items(host, draw, facing, rpm, entity);
    // Moving items read the time; a belt without them is drawn once.
    if !drew_items {
        draw.keep();
    }
}

/// `BeltRenderer.renderItems`: the controller draws every item where the server will have
/// moved it by now.
fn draw_items(host: &dyn Host, draw: &mut BlockEntityDraw, facing: Dir, rpm: f32, entity: &ModBlockEntity) -> bool {
    let inventory = read_inventory(&entity.nbt);
    if inventory.items.is_empty() {
        return false;
    }
    let length = int(&entity.nbt, LENGTH).max(1);
    let margin = Ending::from_id(int(&entity.nbt, ENDING) as i8).margin();
    let since = (draw.game_time - long(&entity.nbt, TICK).unwrap_or(draw.game_time)) as f32
        + draw.partial;
    let positions: Vec<f32> = inventory.items.iter().map(|i| i.pos).collect();
    let drawn = foresee(&positions, chain_speed(rpm, facing), since, length, margin);
    let n = facing.normal().map(|v| v as f32);
    let along_x = clockwise(facing).axis() == Axis::X;
    for (item, pos) in inventory.items.iter().zip(drawn) {
        draw.push();
        draw.translate([0.5 - n[0] * 0.5, 15.0 / 16.0, 0.5 - n[2] * 0.5]);
        draw.translate(nudge(item.angle));
        draw.translate([n[0] * pos, 0.0, n[2] * pos]);
        let side = if along_x { item.side } else { -item.side };
        draw.translate(if along_x { [side, 0.0, 0.0] } else { [0.0, 0.0, side] });
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
    let Some((facing, _)) = segment(host, pos) else {
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
    let controller = segment_position(pos, facing, -index);
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
    let Some((facing, part)) = segment(host, pos) else {
        return;
    };
    let Some((index, _)) = locate(host, pos, facing, part) else {
        return;
    };
    let controller = segment_position(pos, facing, -index);
    let Some(entity) = host.mod_block_entity(controller) else {
        return;
    };
    let inventory = read_inventory(&entity.nbt);
    if inventory.items.is_empty() {
        return;
    }
    let rpm = float(&entity.nbt, SPEED).unwrap_or(0.0);
    eject_all(host, controller, facing, rpm, &inventory);
    host.change_mod_block_entity(controller, &mut |entity| {
        entity
            .nbt
            .insert(INVENTORY.into(), inventory_tag(&Inventory::default()));
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
    let Some((facing, chain)) = can_connect(host, first, pos) else {
        return Flow::Cancel;
    };
    create_belt(host, facing, &chain);
    if player.game_mode != CREATIVE {
        host.take_items(CONNECTOR, 1);
    }
    first_shafts().remove(&player.uuid);
    Flow::Cancel
}

/// `BeltConnectorItem.canConnect`: the geometry, the speeds, and the blocks between.
fn can_connect(host: &dyn Host, first: Pos, second: Pos) -> Option<(Dir, Vec<Pos>)> {
    let axis = |pos| host.block_property(pos, "axis").as_deref().and_then(Axis::from_name);
    let shaft_axis = axis(first)?;
    let (facing, chain) = connect(first, second, shaft_axis, axis(second)?).ok()?;
    let speed = |pos| {
        host.mod_block_entity(pos)
            .and_then(|e| float(&e.nbt, SPEED))
            .unwrap_or(0.0)
    };
    let (a, b) = (speed(first), speed(second));
    if a != 0.0 && b != 0.0 && a.signum() != b.signum() {
        return None;
    }
    let between = &chain[1..chain.len() - 1];
    let clear = between.iter().all(|&at| {
        let block = host.block(at);
        (block == SHAFT && axis(at) == Some(shaft_axis)) || replaceable(host, &block)
    });
    clear.then_some((facing, chain))
}

/// `BeltConnectorItem.createBelts` for a horizontal belt.
fn create_belt(host: &mut dyn Host, facing: Dir, chain: &[Pos]) {
    let Some(state) = host.default_state(ID) else {
        return;
    };
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
        host.set_block_property(at, "facing", facing.name());
        host.set_block_property(at, "part", parts[&at].name());
    }
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
    fn angles_follow_create() {
        // The start model's closed end is north at rest: it goes to the back of the belt.
        assert_eq!(horizontal_angle(Dir::South), 0.0);
        assert_eq!(horizontal_angle(Dir::East), 90.0);
        assert_eq!(dir_id(Dir::Up), 1);
        assert_eq!(dir_id(Dir::East), 5);
    }
}
