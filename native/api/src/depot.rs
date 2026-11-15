//! Create's Depot: one stack sits on top of it. Right-click the top with an item to put the
//! item down, and right-click it with anything to take the stack back
//! (`DepotBlock`, `SharedDepotBlockMethods.onUse`, `DepotBehaviour`, `DepotRenderer`).
//!
//! The stack lives in the block entity's one item slot, which Horizon saves (`Items`) and
//! drops when the block breaks. The rest of Create's `TransportedItemStack` (where the item
//! is on its slide to the centre, its angle, the side it came from) is saved under
//! `HeldItem` with Create's key names. Clients have no item slots, so the update tag sends
//! `HeldItem` with the stack in it under `Item`, as Create's own `HeldItem` holds it.

use std::collections::BTreeMap;
use std::hash::{BuildHasher, Hasher};

use create_kinetics::{Axis, Dir, looking};
use horizon_client::item_data::{decode_components, encode_components};
use horizon_client::mod_api::{
    BlockEntityDraw, Compound, DisplayContext, EquipmentSlot, Flow, Host, ItemStack,
    ModBlockEntity, ModContainer, NativeInventoryKind, NativeServerPlayer, Tag,
};

use crate::{float, int, set_float};

pub const ID: &str = "create:depot";
const HELD: &str = "HeldItem";
const ITEM: &str = "Item";
const POS: &str = "Pos";
const PREV_POS: &str = "PrevPos";
const OFFSET: &str = "Offset";
const PREV_OFFSET: &str = "PrevOffset";
const ANGLE: &str = "Angle";
const IN_DIRECTION: &str = "InDirection";
/// `SharedDepotBlockMethods.onUse` never puts a Mechanical Arm down: the arm's item uses it.
const MECHANICAL_ARM: &str = "create:mechanical_arm";
const SPECTATOR: i32 = 3;
/// `SoundSource` ordinals.
const BLOCKS: usize = 4;
const PLAYERS: usize = 7;

/// One slot, which no hopper reaches: Create's belts and funnels feed a depot, and neither
/// exists here yet. Breaking the block drops it, as `DepotBehaviour.destroy` does.
pub fn container(block: &str) -> Option<ModContainer> {
    (block == ID).then(|| ModContainer {
        slots: 1,
        ..ModContainer::default()
    })
}

/// The parts of Create's `TransportedItemStack` a depot uses, without the stack.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Transport {
    /// How far along its slide from the edge (0) to the centre (0.5) the item is.
    pos: f32,
    prev_pos: f32,
    /// How far it sits to one side of its slide.
    offset: f32,
    prev_offset: f32,
    /// Its turn about the vertical, in degrees.
    angle: i32,
    /// The side it came in from: `Up` drops it straight onto the centre.
    from: Dir,
}

impl Transport {
    /// `new TransportedItemStack(stack)`: the side offset follows from the angle.
    fn new(angle: i32, from: Dir, pos: f32) -> Self {
        let offset = (angle - 180) as f32 / (360.0 * 3.0); // getTargetSideOffset
        Transport {
            pos,
            prev_pos: pos,
            offset,
            prev_offset: offset,
            angle,
            from,
        }
    }

    /// `DepotBehaviour.tick`: the item eases a quarter of the way to the centre each tick,
    /// turning a degree a tick while it is still far off. (Create turns no item drawn
    /// upright, `BeltHelper.isItemUpright`; no item here is drawn upright yet.)
    fn tick(&mut self) {
        self.prev_pos = self.pos;
        self.prev_offset = self.offset;
        let diff = 0.5 - self.pos;
        if diff > 1.0 / 512.0 {
            if diff > 1.0 / 32.0 {
                self.angle += 1;
            }
            self.pos += diff / 4.0;
        }
    }

    fn at_rest(&self) -> bool {
        self.pos == self.prev_pos && self.offset == self.prev_offset
    }

    fn read<K: Ord + for<'a> From<&'a str>>(nbt: &BTreeMap<K, Tag>) -> Option<Self> {
        let Some(Tag::Compound(held)) = get(nbt, HELD) else {
            return None;
        };
        Some(Transport {
            pos: float(held, POS).unwrap_or(0.0),
            prev_pos: float(held, PREV_POS).unwrap_or(0.0),
            offset: float(held, OFFSET).unwrap_or(0.0),
            prev_offset: float(held, PREV_OFFSET).unwrap_or(0.0),
            angle: int(held, ANGLE),
            // `Direction.from3DDataValue`: down, up, north, south, west, east.
            from: Dir::ALL[int(held, IN_DIRECTION).rem_euclid(6) as usize],
        })
    }

    /// `TransportedItemStack.serializeNBT` without the item.
    fn write(&self) -> Compound {
        let mut tag = Compound::new();
        set_float(&mut tag, POS, self.pos);
        set_float(&mut tag, PREV_POS, self.prev_pos);
        set_float(&mut tag, OFFSET, self.offset);
        set_float(&mut tag, PREV_OFFSET, self.prev_offset);
        tag.insert(ANGLE.into(), Tag::Int(self.angle));
        let from = Dir::ALL.iter().position(|d| *d == self.from).unwrap_or(1);
        tag.insert(IN_DIRECTION.into(), Tag::Int(from as i32));
        tag
    }
}

fn remove<K: Ord + for<'a> From<&'a str>>(nbt: &mut BTreeMap<K, Tag>, key: &str) -> Option<Tag> {
    nbt.remove(&K::from(key))
}

fn has<K: Ord + for<'a> From<&'a str>>(nbt: &BTreeMap<K, Tag>, key: &str) -> bool {
    nbt.contains_key(&K::from(key))
}

fn get<'m, K: Ord + for<'a> From<&'a str>>(
    nbt: &'m BTreeMap<K, Tag>,
    key: &str,
) -> Option<&'m Tag> {
    nbt.get(&K::from(key))
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

/// A fresh random number, standing in for Create's static `Random` in `TransportedItemStack`.
fn random() -> u64 {
    std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish()
}

fn centre(pos: [i32; 3]) -> [f64; 3] {
    pos.map(|v| f64::from(v) + 0.5)
}

/// The server tick: slides the held item to the centre and keeps the saved state in step
/// with the slot. Writes and syncs only when something changed.
pub fn tick(host: &mut dyn Host, pos: [i32; 3], entity: &mut ModBlockEntity) {
    let holding = entity.items.first().is_some_and(Option::is_some);
    let saved = Transport::read(&entity.nbt);
    if !holding {
        if saved.is_some() {
            remove(&mut entity.nbt, HELD);
            host.block_entity_sync(pos);
        }
        return;
    }
    // A stack that arrived without its state (none can yet) sits in the centre, as
    // `DepotBehaviour.setCenteredHeldItem` puts one.
    let mut transport =
        saved.unwrap_or_else(|| Transport::new((random() % 360) as i32, Dir::Up, 0.5));
    transport.tick();
    if saved != Some(transport) {
        entity
            .nbt
            .insert(HELD.into(), Tag::Compound(transport.write()));
        host.block_entity_sync(pos);
    }
}

/// What clients get: `HeldItem` with its stack.
pub fn update_tag(entity: &ModBlockEntity) -> Compound {
    let mut tag = Compound::new();
    if let (Some(Some(stack)), Some(transport)) =
        (entity.items.first(), Transport::read(&entity.nbt))
    {
        let mut held = transport.write();
        held.insert(ITEM.into(), stack_tag(stack));
        tag.insert(HELD.into(), Tag::Compound(held));
    }
    tag
}

/// `SharedDepotBlockMethods.onUse`, for a right-click on any face of a depot at `pos`.
pub fn right_click(host: &mut dyn Host, pos: [i32; 3], face: [i32; 3]) -> Flow {
    if host.block(pos) != ID {
        return Flow::Continue;
    }
    let Some(player) = host.acting_player() else {
        return Flow::Continue;
    };
    if player.game_mode == SPECTATOR {
        return Flow::Continue;
    }
    let uuid = player.uuid;
    let main = host.player_equipment(uuid, EquipmentSlot::MainHand);
    // Sneaking with anything in either hand skips the block's own use
    // (`ServerPlayerGameMode.useItemOn`): the item is used instead, so a wrench picks it up.
    if player.sneaking
        && (main.is_some()
            || host
                .player_equipment(uuid, EquipmentSlot::OffHand)
                .is_some())
    {
        return Flow::Continue;
    }
    // PASS_TO_DEFAULT_BLOCK_INTERACTION on any face but the top.
    if face != [0, 1, 0] {
        return Flow::Continue;
    }

    // Whatever is on it goes back to the player first.
    let mut taken = None;
    host.change_mod_block_entity(pos, &mut |entity| {
        if entity.items.first().is_some_and(Option::is_some) {
            taken = entity.items[0].take();
        }
        if has(&entity.nbt, HELD) {
            remove(&mut entity.nbt, HELD);
        }
    });
    if let Some(stack) = taken {
        // `Inventory.placeItemBackInInventory`: what does not fit drops at the player.
        if let Some(rest) = host.give_stack(uuid, stack) {
            let dimension = host.dimension();
            let mut nbt = Compound::new();
            nbt.insert(ITEM.into(), stack_tag(&rest));
            if let Some(id) = host.summon(&dimension, "minecraft:item", player.position, nbt) {
                host.set_item_entity(&dimension, id, rest);
            }
        }
        let pitch = 1.0 + (random() % 1_000_000) as f32 / 1_000_000.0;
        host.play_sound(
            "minecraft:entity.item.pickup",
            centre(pos),
            PLAYERS,
            0.2,
            pitch,
        );
    }

    // Then what the hand holds goes down. Read again: what came back may have joined the
    // hand's stack, and Create puts down that same, grown, stack.
    if main.is_some()
        && let Some(stack) = host.player_equipment(uuid, EquipmentSlot::MainHand)
        && &*stack.item != MECHANICAL_ARM
        && empty_main_hand(host, &player, &stack)
    {
        // `player.getDirection()`, starting a quarter of the way in.
        let transport = Transport::new((random() % 360) as i32, looking(player.yaw, 0.0), 0.25);
        let mut stack = Some(stack);
        host.change_mod_block_entity(pos, &mut |entity| {
            if entity.items.is_empty() {
                entity.items.push(None);
            }
            entity.items[0] = stack.take();
            entity
                .nbt
                .insert(HELD.into(), Tag::Compound(transport.write()));
        });
        // AllSoundEvents.DEPOT_SLIDE: sand breaking at volume 0.125 and pitch 1.5.
        host.play_sound(
            "minecraft:block.sand.break",
            centre(pos),
            BLOCKS,
            0.125,
            1.5,
        );
    }
    host.block_entity_sync(pos);
    Flow::Cancel
}

/// `player.setItemInHand(hand, ItemStack.EMPTY)`; false when the hand could not be emptied,
/// and then nothing may go down. Inside the player's own hook (a right-click is one) the
/// `native_inventory` calls always refuse (`InsidePlayerTurn`), and in a single-player click
/// the acting player isn't found as an entity either, so a `LivingRef` can't be had there.
/// What works there is `take_items` on the acting player, for plain stacks only (no
/// components; a damaged tool may not count). The `LivingRef` path is kept for where the
/// lookup succeeds, and outside a player's turn the slot is set by number, only if it still
/// holds `stack`. A stack none of these can take stays in the hand.
fn empty_main_hand(host: &mut dyn Host, player: &NativeServerPlayer, stack: &ItemStack) -> bool {
    let living = u32::try_from(player.entity_id)
        .ok()
        .filter(|id| *id != 0)
        .and_then(|id| host.entity(id))
        .and_then(|entity| host.living(entity));
    if let Some(living) = living
        && host.living_set_equipment(living, EquipmentSlot::MainHand, None)
    {
        return true;
    }
    // In a single-player world's click the lookup above finds no entity (tested in game:
    // `entity(1)` gives None), but `take_items` edits the acting player's own inventory,
    // main hand first. It only takes plain stacks (no components), whole or not at all here.
    if stack.extra.is_none() {
        let taken = host.take_items(&stack.item, stack.count);
        if taken == stack.count {
            return true;
        }
        if taken > 0 {
            // Partly taken: give it back rather than lose it, and put nothing down.
            host.give_items(&stack.item, taken);
            return false;
        }
    }
    host.set_inventory_slot_typed(
        player.uuid,
        NativeInventoryKind::Player,
        usize::from(player.selected_slot),
        Some(stack),
        None,
    )
    .is_ok()
}

/// `java.util.Random`, for the scatter of a block item's copies (`DepotRenderer` seeds it with 0).
struct JavaRandom(u64);

impl JavaRandom {
    const MULTIPLIER: u64 = 0x5_DEEC_E66D;
    const MASK: u64 = (1 << 48) - 1;

    fn new(seed: i64) -> Self {
        JavaRandom((seed as u64 ^ Self::MULTIPLIER) & Self::MASK)
    }

    fn next(&mut self, bits: u32) -> u32 {
        self.0 = (self.0.wrapping_mul(Self::MULTIPLIER).wrapping_add(0xB)) & Self::MASK;
        (self.0 >> (48 - bits)) as u32
    }

    fn next_float(&mut self) -> f32 {
        self.next(24) as f32 / (1 << 24) as f32
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

/// `Direction.getClockWise` seen from above.
fn clockwise(dir: Dir) -> Dir {
    match dir {
        Dir::North => Dir::East,
        Dir::East => Dir::South,
        Dir::South => Dir::West,
        Dir::West => Dir::North,
        other => other,
    }
}

const UP: [f32; 3] = [0.0, 1.0, 0.0];
const ORIGIN: [f32; 3] = [0.0; 3];

/// `DepotRenderer.renderItemsOf` for the held item. The depot's body is in the chunk mesh.
pub fn draw(host: &dyn Host, draw: &mut BlockEntityDraw, entity: &ModBlockEntity) {
    let (Some(transport), Some(stack)) = (
        Transport::read(&entity.nbt),
        get(&entity.nbt, HELD)
            .and_then(|held| held.get(ITEM))
            .and_then(stack_of),
    ) else {
        draw.keep();
        return;
    };
    let partial = draw.partial;
    let lerp = |from: f32, to: f32| from + (to - from) * partial;

    draw.push();
    draw.translate([0.5, 15.0 / 16.0, 0.5]);
    draw.translate(nudge(0));
    let along = lerp(transport.prev_pos, transport.pos);
    let mut side = lerp(transport.prev_offset, transport.offset);
    let from = transport.from;
    if from.axis() != Axis::Y {
        let normal = from.opposite().normal().map(|v| v as f32 * (0.5 - along));
        draw.translate(normal);
        let along_x = clockwise(from).axis() == Axis::X;
        if !along_x {
            side = -side;
        }
        draw.translate(if along_x {
            [side, 0.0, 0.0]
        } else {
            [0.0, 0.0, side]
        });
    }
    draw_stack(host, draw, &stack, transport.angle, &mut JavaRandom::new(0));
    draw.pop();
    // Kept once it has stopped sliding: until then it reads the partial tick.
    if transport.at_rest() {
        draw.keep();
    }
}

/// `DepotRenderer.renderItem`: a block lies in a small heap, any other item flat in a fan,
/// one copy more for each fourfold of the count.
fn draw_stack(
    host: &dyn Host,
    draw: &mut BlockEntityDraw,
    stack: &ItemStack,
    angle: i32,
    r: &mut JavaRandom,
) {
    // `Mth.log2`, rounded down, halved.
    let copies = stack.count.max(1).ilog2() / 2;
    // Stands in for `BakedModel.isGui3d`: an item that is also a block.
    let block = host.default_state(&stack.item).is_some();
    draw.push();
    draw.rotate_about(UP, angle as f32, ORIGIN);
    for i in 0..=copies {
        let i = i as f32;
        draw.push();
        if block {
            let x = r.next_float() * 0.0625 * i;
            let z = r.next_float() * 0.0625 * i;
            draw.translate([x, 0.0, z]);
        }
        draw.scale([0.5; 3]);
        if !block {
            draw.translate([0.0, -3.0 / 16.0, 0.0]);
            draw.rotate_about([1.0, 0.0, 0.0], 90.0, ORIGIN);
        }
        host.draw_item(draw, stack, DisplayContext::Fixed, None, 0);
        draw.pop();
        if !block {
            draw.rotate_about(UP, 10.0, ORIGIN);
        }
        draw.translate([0.0, if block { 1.0 / 64.0 } else { 1.0 / 16.0 }, 0.0]);
    }
    draw.pop();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_random_matches_java() {
        // new Random(0).nextFloat(), twice.
        let mut r = JavaRandom::new(0);
        assert_eq!(r.next_float(), 0.730_967_76);
        assert_eq!(r.next_float(), 0.831_441);
    }

    #[test]
    fn nudge_zero_is_the_same_small_step_on_every_axis() {
        assert_eq!(nudge(0), [-0.00175; 3]);
    }

    #[test]
    fn slide_settles_in_the_centre() {
        let mut t = Transport::new(200, Dir::North, 0.25);
        for _ in 0..40 {
            t.tick();
        }
        assert!((t.pos - 0.5).abs() <= 1.0 / 512.0);
        assert!(t.at_rest());
        // A degree a tick while more than 1/32 away: 0.25 * 0.75^n > 1/32 for n < 8.
        assert_eq!(t.angle, 208);
    }

    #[test]
    fn transport_round_trips() {
        let t = Transport::new(90, Dir::West, 0.25);
        let mut nbt = Compound::new();
        nbt.insert(HELD.into(), Tag::Compound(t.write()));
        assert_eq!(Transport::read(&nbt), Some(t));
    }
}
