//! The Deployer (`create:deployer`; `DeployerBlock`, `DeployerBlockEntity`, `DeployerRenderer`,
//! `BeltDeployerCallbacks`): a casing with a pole and a hand that reaches two blocks out in front
//! of it each stroke and clicks there as a player would. The stroke, the belt callbacks and the
//! hand are `create_kinetics::rules::deployer`; the click is `deployer_use.rs`; the recipes it
//! applies to an item on a belt or depot are `deployer_apply.rs`; this is the block entity, its
//! hooks and its drawing.
//!
//! What it does: a held item, put in by right-clicking the hand with an item (an empty hand takes
//! it back) or by a hopper or funnel; in `USE` mode (the default) the hand uses the item on the
//! entity or block two blocks in front (places blocks, fills and empties buckets, flips levers,
//! applies `create:item_application` recipes); in `PUNCH` mode (the Wrench on the hand toggles it)
//! it attacks the entity there or mines the block. Facing down over a belt or a depot it applies
//! `create:deploying` and `create:item_application` recipes to the item held under it, one at a
//! time (`belt::register_processor`, as the Spout and the press). Two `PUNCH` deployers facing
//! each other bump fists. Redstone power stops it between strokes.
//!
//! Known differences from Create:
//! - Deployers on contraptions (`DeployerMovementBehaviour`, `DeployerMovingInteraction`) are a
//!   later slice, like the drill's and the saw's.
//! - No filter slot: the stand-in of the Brass Funnel (`chute_inv`): right-click the Deployer
//!   while sneaking, anywhere but the hand, with an item to make that item the filter, or with an
//!   empty hand to clear it. The filter is the one `FilteringBehaviour.test` uses (same item):
//!   only a matching item goes in by a hopper or a funnel, and a held item that doesn't match is
//!   moved to the overflow. The rule that a matching held item can't be extracted
//!   (`DeployerItemHandler.extractItem`) is not applied (Horizon has no hook for extraction).
//! - The overflow (`overflowItems`) is the container's slots 1 to 11 as Horizon saves them
//!   (`Items`); the held item is slot 0 (and is saved as `HeldItem` too, as Create does, for
//!   clients). `Inventory` (the fake player's own inventory) and `Owner` are not saved: there is
//!   no fake player, and nothing reads the owner.
//! - No particles (`player.spawnedItemEffects`, `SandPaperItem.spawnParticles`), no goggle
//!   tooltip or overflow hint (`hint.full_deployer`), no advancements (`DEPLOYER`, `FIST_BUMP`,
//!   the casings), no comparator and no waterlogging.
//! - A placed Deployer isn't lined up with a shaft beside it (`prefersConnectionTo`), isn't set
//!   facing down over a belt or a depot (`AssemblyOperatorBlockItem`), and right-clicking a
//!   Deployer with a Deployer doesn't place another beside it (`PlacementHelper`): Horizon's
//!   placement hook can't read neighbours.
//! - The hit of a click on the hand is found from the player's eye ray (the hook doesn't give it),
//!   against the shape's boxes; redstone is read by polling every tick (`neighborChanged`).
//! - A swap with the player (right-click on the hand) puts the Deployer's stack in the player's
//!   first free slot, not necessarily the hand they held: `give_stack` is what works inside the
//!   click, and it is the only way to give a stack with components. A stack with components can't
//!   be taken out of the hand there either, so the swap is refused for one.
//! - `rules/deployer.rs` and `deployer_use.rs` list the differences of the click, the mining and
//!   the attack; `deployer_apply.rs` the recipes'.

use create_kinetics::belt::Processing;
use create_kinetics::rules::deployer::{self as rules, BeltView, Mode, Pose, State, Stroke};
use create_kinetics::rules::drill;
use create_kinetics::{Dir, Kind, Node, Pos, looking};
use horizon_client::mod_api::{
    BlockEntityContainerEvent, BlockEntityDraw, BlockEntityEvent, BlockEntityRenderBoundingBoxEvent,
    BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent, BlockEntityUpdateTagEvent,
    BlockStateId, Compound, ContainerAcceptsEvent, DisplayContext, Flow, Hooks,
    Host, ItemStack, LevelHost, ModBlockEntity, ModContainer, NativeInventoryKind,
    NativeServerPlayer, RightClickBlockEvent, StandaloneModelsEvent, StateForPlacementEvent, Tag,
    UseItemOnEvent,
};

use super::chute_inv as inv;
use super::deployer_apply as apply;
use super::deployer_use::{self as work, Crack, Items, SLOTS};
use crate::belt::{self, BeltProcessor};
use crate::{depot, kinetic, util};

const ID: &str = "create:deployer";
/// `AllPartialModels.DEPLOYER_POLE`.
const POLE: &str = "create:block/deployer/pole";
const CENTRE: [f32; 3] = [0.5; 3];
const ORIGIN: [f32; 3] = [0.0; 3];
const UP: [f32; 3] = [0.0, 1.0, 0.0];
const EAST: [f32; 3] = [1.0, 0.0, 0.0];
const SOUTH: [f32; 3] = [0.0, 0.0, 1.0];
/// `SoundSource` ordinals.
const BLOCKS: usize = 4;

// Saved as `DeployerBlockEntity.write` names them.
const MODE: &str = "Mode";
const STATE: &str = "State";
const TIMER: &str = "Timer";
const POWERED: &str = "Powered";
const HELD_ITEM: &str = "HeldItem";
// What the packet to clients holds (`clientPacket`).
const FIST_BUMP: &str = "Fistbump";
const REACH: &str = "Reach";
/// The game time `Timer` was true at, for clients to run the stroke on from.
const SINCE: &str = "Since";
/// `blockBreakingProgress`, which Create doesn't save.
const CRACK_AT: &str = "CrackAt";
const CRACK_PROGRESS: &str = "CrackProgress";

/// How often the held item is compared with what clients were told (a hopper may change it).
const HELD_SCAN: i64 = 4;

struct Deployers;

pub fn hooks() -> Box<dyn Hooks> {
    belt::register_processor(BeltProcessor { claims: |block| block == ID, received, held });
    Box::new(Deployers)
}

// --- the block entity's saved state ---------------------------------------------------------------

/// What the stroke and the mode are, as saved.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Saved {
    stroke: Stroke,
    mode: Mode,
    /// `redstoneLocked`.
    locked: bool,
    crack: Option<Crack>,
}

fn text(nbt: &Compound, key: &str) -> Option<String> {
    match util::get(nbt, key) {
        Some(Tag::String(text)) => Some(text.to_string_lossy()),
        _ => None,
    }
}

fn read(nbt: &Compound) -> Saved {
    let crack = match (util::get(nbt, CRACK_AT), util::float(nbt, CRACK_PROGRESS)) {
        (Some(Tag::IntArray(at)), Some(progress)) if at.len() == 3 => {
            Some(Crack { pos: [at[0], at[1], at[2]], progress })
        }
        _ => None,
    };
    Saved {
        stroke: Stroke {
            state: text(nbt, STATE).and_then(|n| State::from_name(&n)).unwrap_or(State::Waiting),
            timer: util::int(nbt, TIMER),
            reach: util::float(nbt, REACH).unwrap_or(0.0),
            fist_bump: util::boolean(nbt, FIST_BUMP),
        },
        mode: text(nbt, MODE).and_then(|n| Mode::from_name(&n)).unwrap_or(Mode::Use),
        locked: util::boolean(nbt, POWERED),
        crack,
    }
}

/// Writes what differs between `old` and `new`: touching a block entity's data counts as a change.
fn write(entity: &mut ModBlockEntity, old: &Saved, new: &Saved, now: i64) {
    if new.mode != old.mode {
        entity.nbt.insert(MODE.into(), Tag::String(new.mode.name().into()));
    }
    let (o, n) = (&old.stroke, &new.stroke);
    if n.state != o.state {
        entity.nbt.insert(STATE.into(), Tag::String(n.state.name().into()));
    }
    if n.timer != o.timer {
        entity.nbt.insert(TIMER.into(), Tag::Int(n.timer));
    }
    if n.reach != o.reach {
        util::set_float(&mut entity.nbt, REACH, n.reach);
    }
    if n.fist_bump != o.fist_bump {
        entity.nbt.insert(FIST_BUMP.into(), Tag::Byte(i8::from(n.fist_bump)));
    }
    if n.timer != o.timer || n.state != o.state || n.reach != o.reach || n.fist_bump != o.fist_bump {
        entity.nbt.insert(SINCE.into(), Tag::Long(now));
    }
    if new.locked != old.locked {
        entity.nbt.insert(POWERED.into(), Tag::Byte(i8::from(new.locked)));
    }
    if new.crack != old.crack {
        match new.crack {
            Some(crack) => {
                entity.nbt.insert(CRACK_AT.into(), Tag::IntArray(crack.pos.to_vec()));
                util::set_float(&mut entity.nbt, CRACK_PROGRESS, crack.progress);
            }
            None => {
                util::remove(&mut entity.nbt, CRACK_AT);
                util::remove(&mut entity.nbt, CRACK_PROGRESS);
            }
        }
    }
}

fn new_entity() -> Compound {
    let mut nbt = kinetic::new_entity();
    nbt.insert(MODE.into(), Tag::String(Mode::Use.name().into()));
    nbt.insert(STATE.into(), Tag::String(State::Waiting.name().into()));
    nbt.insert(TIMER.into(), Tag::Int(0));
    nbt.insert(POWERED.into(), Tag::Byte(0));
    nbt.insert(HELD_ITEM.into(), Tag::Compound(Compound::new()));
    nbt
}

/// `player.getMainHandItem().saveOptional`: the stack, or an empty compound.
fn held_tag(held: Option<&ItemStack>) -> Tag {
    held.map_or_else(|| Tag::Compound(Compound::new()), util::stack_tag)
}

fn held_of(entity: &ModBlockEntity) -> Option<ItemStack> {
    entity.items.first().cloned().flatten()
}

// --- the block's state ----------------------------------------------------------------------------

/// `(facing, axis_along_first)` of the block at `pos`.
fn state_of(host: &dyn Host, pos: Pos) -> Option<(Dir, bool)> {
    let facing = util::facing(host, pos, "facing")?;
    let along_first = host.block_property(pos, "axis_along_first").as_deref() == Some("true");
    Some((facing, along_first))
}

/// A shaft through both ends of its rotation axis, `DirectionalAxisKineticBlock.hasShaftTowards`.
fn node(facing: Dir, along_first: bool) -> Node {
    Node {
        impact: rules::IMPACT,
        ..Node::passive(Kind::shafts(&rules::shaft_faces(facing, along_first)))
    }
}

fn bool_name(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn add(a: Pos, b: Pos, times: i32) -> Pos {
    [a[0] + b[0] * times, a[1] + b[1] * times, a[2] + b[2] * times]
}

/// Whether the block at `pos` holds items moving on it
/// (`TransportedItemStackHandlerBehaviour`): a belt or a depot.
fn transports(host: &dyn Host, pos: Pos) -> bool {
    let block = host.block(pos);
    block == belt::ID || block == depot::ID
}

/// `start`: how far the hand reaches, by the ray through the cell two blocks in front.
fn reach_at(host: &dyn Host, pos: Pos, facing: Dir) -> f32 {
    let clicked = add(pos, facing.normal(), 2);
    rules::reach_of(rules::front_hit(facing, &host.block_collision(clicked)).distance)
}

// --- the stroke -----------------------------------------------------------------------------------

/// `DeployerHandler.shouldActivate` for what the Deployer holds and what is in front.
fn should_activate(host: &dyn Host, pos: Pos, facing: Dir, held: Option<&ItemStack>) -> bool {
    let target = host.block(add(pos, facing.normal(), 2));
    let item = held.map(|stack| &*stack.item);
    // A `BlockItem` here is an item with a block of its own id.
    let block = item.filter(|id| host.default_state(id).is_some());
    rules::should_activate(
        item,
        block,
        item.and_then(rules::bucket_fluid),
        &target,
        facing,
        transports(host, add(pos, facing.normal(), 2)),
    )
}

/// The partner of a fist bump: the first Deployer 2 to 4 blocks in front (`startFistBump`,
/// `triggerFistBump`), with its distance. `None` where an unloaded block is met first.
fn partner(host: &dyn Host, pos: Pos, facing: Dir) -> Option<(i32, Pos)> {
    let dimension = host.dimension();
    for i in 2..5 {
        let other = add(pos, facing.normal(), i);
        if !host.chunk_is_loaded(dimension, [other[0] >> 4, other[2] >> 4]) {
            return None;
        }
        if host.block(other) == ID && host.mod_block_entity(other).is_some() {
            return Some((i, other));
        }
    }
    None
}

/// `startFistBump`: a Deployer in `PUNCH` mode with another one facing it in range that is
/// turning and also punching: both set out at once, to meet half way.
fn start_fist_bump(host: &mut dyn Host, pos: Pos, facing: Dir, saved: &mut Saved, now: i64) -> bool {
    let Some((i, other)) = partner(&*host, pos, facing) else {
        return false;
    };
    let Some(entity) = host.mod_block_entity(other) else {
        return false;
    };
    if util::facing(&*host, other, "facing") != Some(facing.opposite())
        || read(&entity.nbt).mode != Mode::Punch
        || kinetic::speed(&entity) == 0.0
    {
        return false;
    }
    let reach = (i - 2) as f32 * 0.5;
    saved.stroke.start_fist_bump(reach);
    host.change_mod_block_entity(other, &mut |entity| {
        let old = read(&entity.nbt);
        let mut new = old;
        new.stroke.start_fist_bump(reach);
        write(entity, &old, &new, now);
    });
    host.block_entity_sync(other);
    true
}

/// `triggerFistBump`: the partner, still out and run down, goes back at once.
fn trigger_fist_bump(host: &mut dyn Host, pos: Pos, facing: Dir, saved: &mut Saved, now: i64) {
    let Some((_, other)) = partner(&*host, pos, facing) else {
        return;
    };
    let Some(entity) = host.mod_block_entity(other) else {
        return;
    };
    let theirs = read(&entity.nbt).stroke;
    if !theirs.fist_bump || theirs.state != State::Expanding || theirs.timer > 0 {
        return;
    }
    saved.stroke.fist_bump = false;
    host.change_mod_block_entity(other, &mut |entity| {
        let old = read(&entity.nbt);
        let mut new = old;
        new.stroke.bumped();
        write(entity, &old, &new, now);
    });
    host.block_entity_sync(other);
    // `BlockPos.containing(centre + centre of the other, scaled by a half)`.
    let middle = [0, 1, 2].map(|i| (f64::from(pos[i]) + f64::from(other[i]) + 1.0) / 2.0);
    let at = middle.map(|v| v.floor() as i32);
    host.play_sound("minecraft:entity.player.attack.nodamage", util::centre(at), BLOCKS, 0.75, 0.75);
}

/// The waiting half of `tick`. True when clients need to hear of a change.
fn waiting(
    host: &mut dyn Host,
    pos: Pos,
    facing: Dir,
    entity: &mut ModBlockEntity,
    saved: &mut Saved,
    rpm: f32,
    now: i64,
) -> bool {
    // `!overflowItems.isEmpty()`.
    if entity.items.iter().skip(1).any(Option::is_some) {
        saved.stroke.back_off(rpm);
        return false;
    }
    // `item != stack || !filtering.test(item)`: a held item that fails the filter is moved out.
    let held = held_of(entity);
    if let Some(stack) = &held
        && !inv::filter_of(&entity.nbt).test(stack)
    {
        if entity.items.len() < SLOTS {
            entity.items.resize(SLOTS, None);
        }
        entity.items[1] = entity.items[0].take();
        saved.stroke.back_off(rpm);
        return true;
    }
    if saved.mode == Mode::Use && !should_activate(&*host, pos, facing, held.as_ref()) {
        saved.stroke.back_off(rpm);
        return false;
    }
    // Check for advancement conditions.
    if saved.mode == Mode::Punch
        && !saved.stroke.fist_bump
        && start_fist_bump(host, pos, facing, saved, now)
    {
        return true;
    }
    if saved.locked {
        return false;
    }
    saved.stroke.start(reach_at(&*host, pos, facing));
    true
}

/// `activate`: the world's work of the stroke, at the top of the hand's reach.
fn activate(
    host: &mut dyn Host,
    pos: Pos,
    facing: Dir,
    entity: &mut ModBlockEntity,
    saved: &mut Saved,
) {
    let clicked = add(pos, facing.normal(), 2);
    if entity.items.len() < SLOTS {
        entity.items.resize(SLOTS, None);
    }
    if facing == Dir::Down && transports(&*host, clicked) {
        // Belt processing is `BeltDeployerCallbacks.activate`.
        apply_on_belt(host, pos, clicked, &mut entity.items);
        return;
    }
    work::activate(host, pos, facing, saved.mode, &mut entity.items, &mut saved.crack);
}

/// `BeltDeployerCallbacks.activate`: applies the recipe the held item has for the item held under
/// the Deployer, uses up what the recipe takes of the held item, and makes the sounds.
fn apply_on_belt(host: &mut dyn Host, pos: Pos, clicked: Pos, items: &mut Items) {
    let Some(held) = items.first().cloned().flatten() else {
        return;
    };
    let mut keep = false;
    let mut recipe = |host: &mut dyn Host, probe: &ItemStack| -> Option<Vec<ItemStack>> {
        let done = apply::apply(host, probe, &held)?;
        keep = done.keep_held;
        Some(done.results)
    };
    let applied = if host.block(clicked) == belt::ID {
        belt::apply_held(host, clicked, &mut recipe)
    } else {
        depot::apply_held(host, clicked, &mut recipe)
    };
    if !applied {
        return;
    }
    work::use_up(host, pos, items, keep);
    let at = util::centre(pos);
    if items.first().is_none_or(Option::is_none) {
        let (sound, volume, pitch) = rules::BREAK_SOUND;
        host.play_sound(sound, at, BLOCKS, volume, pitch);
    }
    let (sound, volume, pitch) = rules::PICKUP_SOUND;
    host.play_sound(sound, at, BLOCKS, volume, pitch);
}

// --- the belt's callbacks (`BeltDeployerCallbacks`) ------------------------------------------------

fn belt_view(host: &dyn Host, pos: Pos) -> Option<(BeltView, Option<ItemStack>)> {
    let entity = host.mod_block_entity(pos).filter(|e| e.id == ID)?;
    let saved = read(&entity.nbt);
    let held = held_of(&entity);
    Some((
        BeltView {
            rpm: kinetic::speed(&entity),
            mode: saved.mode,
            facing: util::facing(host, pos, "facing")?,
            state: saved.stroke.state,
            redstone_locked: saved.locked,
            holding_nothing: held.is_none(),
        },
        held,
    ))
}

/// `DeployerBlockEntity.start()`, from a belt that has an item for the hand.
fn start_stroke(host: &mut dyn Host, pos: Pos, facing: Dir) {
    let reach = reach_at(&*host, pos, facing);
    let now = util::game_time(&*host);
    host.change_mod_block_entity(pos, &mut |entity| {
        let old = read(&entity.nbt);
        let mut new = old;
        new.stroke.start(reach);
        write(entity, &old, &new, now);
    });
    host.block_entity_sync(pos);
}

/// `onItemReceived`.
fn received(host: &mut dyn Host, pos: Pos, stack: &ItemStack) -> Processing {
    let Some((view, held)) = belt_view(&*host, pos) else {
        return Processing::Pass;
    };
    let (answer, start) = rules::on_item_received(&view, || {
        held.as_ref().is_some_and(|held| apply::has_recipe(&*host, stack, held))
    });
    if start {
        start_stroke(host, pos, view.facing);
    }
    answer
}

/// `whenItemHeld`, except the stroke's end (the Deployer's own tick applies the recipe then).
fn held(host: &mut dyn Host, pos: Pos, stack: &ItemStack) -> Processing {
    let Some((view, held)) = belt_view(&*host, pos) else {
        return Processing::Pass;
    };
    let (answer, start) = rules::while_item_held(&view, || {
        held.as_ref().is_some_and(|held| apply::has_recipe(&*host, stack, held))
    });
    if start {
        start_stroke(host, pos, view.facing);
    }
    answer
}

// --- clicks ---------------------------------------------------------------------------------------

/// How far from the casing's back the acting player's click meets the Deployer at `pos`
/// (`location.length()` of `DeployerBlock.useItemOn`), found from their eye ray.
fn click_along(host: &dyn Host, pos: Pos, facing: Dir) -> Option<f64> {
    let player = host.acting_player()?;
    let eye = [
        player.position[0],
        player.position[1] + f64::from(player.eye_height),
        player.position[2],
    ];
    let direction = rules::view_vector(player.yaw, player.pitch);
    let local = [0, 1, 2].map(|i| eye[i] - f64::from(pos[i]));
    let t = rules::ray_boxes(local, direction, &rules::interaction_boxes(facing))?;
    let hit = [0, 1, 2].map(|i| eye[i] + direction[i] * t);
    Some(rules::along_from_back(facing, hit, pos))
}

/// `player.setItemInHand(hand, ItemStack.EMPTY)`; false when the hand could not be emptied, and
/// then nothing may be swapped. As `depot::empty_main_hand`: inside a player's hook only
/// `take_items` works, and only for a plain stack.
fn empty_main_hand(host: &mut dyn Host, player: &NativeServerPlayer, stack: &ItemStack) -> bool {
    if stack.extra.is_none() {
        let taken = host.take_items(&stack.item, stack.count);
        if taken == stack.count {
            return true;
        }
        if taken > 0 {
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

/// `DeployerBlock.useItemOn`: the stack in the player's hand and the one in the Deployer's change
/// places.
fn swap(host: &mut dyn Host, pos: Pos, player: &NativeServerPlayer, held: Option<ItemStack>) {
    let Some(entity) = host.mod_block_entity(pos) else {
        return;
    };
    let mine = held_of(&entity);
    if mine.is_none() && held.is_none() {
        return;
    }
    if let Some(stack) = &held
        && !empty_main_hand(host, player, stack)
    {
        return;
    }
    let given = held.clone();
    host.change_mod_block_entity(pos, &mut |entity| {
        if entity.items.len() < SLOTS {
            entity.items.resize(SLOTS, None);
        }
        entity.items[0] = given.clone();
        entity.nbt.insert(HELD_ITEM.into(), held_tag(given.as_ref()));
    });
    host.block_entity_sync(pos);
    if let Some(stack) = mine
        && let Some(rest) = host.give_stack(player.uuid, stack)
    {
        // `Inventory.placeItemBackInInventory`: what does not fit drops at the player.
        util::spawn_item(host, player.position, &rest, [0.0; 3], 0);
    }
}

// --- drawing --------------------------------------------------------------------------------------

/// `AngleHelper.horizontalAngle`.
fn horizontal_angle(facing: Dir) -> f32 {
    match facing {
        Dir::South | Dir::Up | Dir::Down => 0.0,
        Dir::West => -90.0,
        Dir::North => 180.0,
        Dir::East => 90.0,
    }
}

/// `DeployerRenderer.transform`: a part made facing south, turned to `facing`; the pole also by
/// the axis (`axisDirectionMatters`).
fn draw_part(
    host: &dyn Host,
    draw: &mut BlockEntityDraw,
    model: &str,
    facing: Dir,
    along_first: bool,
    axis_matters: bool,
    offset: [f32; 3],
) {
    draw.push();
    draw.translate(offset);
    crate::draw::face_south(draw, facing);
    if axis_matters && (along_first ^ (facing.axis() == create_kinetics::Axis::Z)) {
        draw.rotate_about(SOUTH, 90.0, CENTRE);
    }
    host.draw_model(draw, model, 0xFF_FFFF);
    draw.pop();
}

/// `DeployerRenderer.renderItem`. True when the item turns with the time (a floating item).
fn draw_held(
    host: &dyn Host,
    draw: &mut BlockEntityDraw,
    stack: &ItemStack,
    facing: Dir,
    rpm: f32,
    punching: bool,
    offset: [f32; 3],
) -> bool {
    let block_item = host.default_state(&stack.item).is_some();
    let display = facing == Dir::Up && rpm == 0.0 && !punching;
    draw.push();
    draw.translate([0.5 + offset[0], 0.5 + offset[1], 0.5 + offset[2]]);
    draw.rotate_about(UP, horizontal_angle(facing) + 180.0, ORIGIN);
    if !display {
        let x_rot = match facing {
            Dir::Up => 90.0,
            Dir::Down => 270.0,
            _ => 0.0,
        };
        draw.rotate_about(EAST, x_rot, ORIGIN);
        draw.translate([0.0, 0.0, -11.0 / 16.0]);
    }
    if punching {
        draw.translate([0.0, 1.0 / 8.0, -1.0 / 16.0]);
    }
    let context;
    if display {
        let scale = if block_item { 1.25 } else { 1.0 };
        draw.translate([0.0, if block_item { 9.0 / 16.0 } else { 11.0 / 16.0 }, 0.0]);
        draw.scale([scale; 3]);
        // `AnimationTickHolder.getRenderTime`, in degrees.
        let time = (draw.game_time as f64 + f64::from(draw.partial)).rem_euclid(360.0) as f32;
        draw.rotate_about(UP, time, ORIGIN);
        context = DisplayContext::Ground;
    } else {
        let scale = if punching {
            0.75
        } else if block_item {
            0.75 - 1.0 / 64.0
        } else {
            0.5
        };
        draw.scale([scale; 3]);
        context = if punching { DisplayContext::ThirdPersonRightHand } else { DisplayContext::Fixed };
    }
    host.draw_item(draw, stack, context, None, 0);
    draw.pop();
    display
}

// --- the hooks ------------------------------------------------------------------------------------

impl Hooks for Deployers {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.push(POLE.to_owned());
        for pose in [Pose::Pointing, Pose::Punching, Pose::Holding] {
            event.models.push(format!("create:{}", pose.model()));
        }
    }

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

    /// `DeployerItemHandler`: the held item is slot 0, the overflow the others; every side, put
    /// into slot 0 only and taken out of any.
    fn block_entity_container(
        &self,
        _: &dyn Host,
        event: &mut BlockEntityContainerEvent<'_>,
    ) -> Option<ModContainer> {
        (event.block == ID).then(|| {
            let all: Vec<usize> = (0..SLOTS).collect();
            ModContainer {
                slots: SLOTS,
                faces: [(); 6].map(|()| all.clone()),
                inputs: vec![0],
                outputs: all,
                menu: None,
            }
        })
    }

    /// `DeployerItemHandler.isItemValid`: the filter's, for the held slot only.
    fn container_accepts(&self, _: &dyn Host, event: &mut ContainerAcceptsEvent<'_>) -> Flow {
        if event.entity.id != ID {
            return Flow::Continue;
        }
        if event.slot != 0 || !inv::filter_of(&event.entity.nbt).test(event.stack) {
            return Flow::Cancel;
        }
        Flow::Continue
    }

    /// `DeployerBlockEntity.tick`.
    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        let pos = event.pos;
        let Some((facing, along_first)) = state_of(host, pos) else {
            return;
        };
        let Some((rpm, changed)) = kinetic::tick(host, pos, event.entity, node(facing, along_first))
        else {
            return; // The network jammed and the block broke.
        };
        let now = util::game_time(host);
        let mut sync = changed;
        let old = read(&event.entity.nbt);
        let mut saved = old;

        // `redstoneUpdate`.
        let dimension = host.dimension();
        let powered = host.neighbor_signal(dimension, pos) > 0;
        if powered != saved.locked {
            saved.locked = powered;
            sync = true;
        }

        // What clients were told the hand holds (a hopper or a funnel may have changed it).
        if now.rem_euclid(HELD_SCAN) == 0 {
            let wanted = held_tag(held_of(event.entity).as_ref());
            if util::differs(&event.entity.nbt, HELD_ITEM, &wanted) {
                event.entity.nbt.insert(HELD_ITEM.into(), wanted);
                sync = true;
            }
        }

        if rpm != 0.0 {
            sync |= stroke(host, pos, facing, event.entity, &mut saved, rpm, now);
        }
        if saved != old {
            write(event.entity, &old, &saved, now);
        }
        if sync {
            host.block_entity_sync(pos);
        }
    }

    fn block_entity_update_tag(
        &self,
        event: &mut BlockEntityUpdateTagEvent<'_>,
    ) -> Option<Compound> {
        if event.entity.id != ID {
            return None;
        }
        let entity = event.entity;
        let saved = read(&entity.nbt);
        let mut tag = kinetic::update_tag(entity);
        tag.insert(MODE.into(), Tag::String(saved.mode.name().into()));
        tag.insert(STATE.into(), Tag::String(saved.stroke.state.name().into()));
        tag.insert(TIMER.into(), Tag::Int(saved.stroke.timer));
        tag.insert(SINCE.into(), Tag::Long(util::long(&entity.nbt, SINCE)));
        util::set_float(&mut tag, REACH, saved.stroke.reach);
        tag.insert(FIST_BUMP.into(), Tag::Byte(i8::from(saved.stroke.fist_bump)));
        // Always sent, an empty compound for nothing, so a client that held something drops it.
        tag.insert(HELD_ITEM.into(), held_tag(held_of(entity).as_ref()));
        Some(tag)
    }

    /// `DeployerBlockEntity.createRenderBoundingBox`: `inflate(3)`.
    fn block_entity_render_bounding_box(
        &self,
        _: &dyn Host,
        event: &mut BlockEntityRenderBoundingBoxEvent<'_>,
    ) {
        if event.block == ID {
            *event.bounds = [-3.0, -3.0, -3.0, 4.0, 4.0, 4.0];
        }
    }

    /// `DeployerRenderer`: the shaft, the pole and the hand moving out and back, and the held
    /// item in the hand. The casing is in the chunk mesh.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let Some(facing) = host
            .block_state_property(event.state, "facing")
            .and_then(Dir::from_name)
        else {
            return;
        };
        let along_first = host.block_state_property(event.state, "axis_along_first") == Some("true");
        let entity = event.entity;
        let rpm = kinetic::speed(entity);
        let saved = read(&entity.nbt);
        let held = util::get(&entity.nbt, HELD_ITEM).and_then(util::stack_of);
        let draw = &mut *event.draw;

        // `getHandOffset`: the timer as a client has it by now.
        let since = util::long(&entity.nbt, SINCE);
        let ticks = (draw.game_time - since) as f32 + draw.partial;
        let timer = saved.stroke.timer as f32 - rules::timer_speed(rpm) as f32 * ticks;
        let pose = Pose::of(saved.mode, held.is_some());
        let distance = rules::hand_offset(saved.stroke.state, timer, saved.stroke.fist_bump, saved.stroke.reach, pose);
        let offset = facing.normal().map(|n| n as f32 * distance);

        kinetic::draw_shaft(host, draw, rules::rotation_axis(facing, along_first), rpm);
        draw_part(host, draw, POLE, facing, along_first, true, offset);
        draw_part(host, draw, &format!("create:{}", pose.model()), facing, along_first, false, offset);
        let turning = held.as_ref().is_some_and(|stack| {
            draw_held(host, draw, stack, facing, rpm, saved.mode == Mode::Punch, offset)
        });
        // At rest it reads nothing of the time (except a floating item).
        if saved.stroke.state == State::Waiting && !turning {
            draw.keep();
        }
    }

    /// `DirectionalAxisKineticBlock.getStateForPlacement` without a shaft to prefer.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != ID {
            return None;
        }
        let look = looking(context.yaw, context.pitch);
        let horizontal = rules::horizontal_direction(context.yaw);
        let (facing, along_first) = rules::placement(look, horizontal, context.sneaking);
        let state = level.state_with_property(event.state, "facing", facing.name())?;
        level
            .state_with_property(state, "axis_along_first", bool_name(along_first))
            .map(Some)
    }

    /// The Wrench: on the hand it toggles the mode (`DeployerBlock.onWrenched`), elsewhere it
    /// turns the Deployer round the face clicked (`IWrenchable.getRotatedBlockState`).
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != ID {
            return Flow::Continue;
        }
        let pos = event.pos;
        let clicked = Dir::from_normal(event.face);
        util::wrench(host, pos, |host| {
            let Some((facing, along_first)) = state_of(host, pos) else {
                return;
            };
            if click_along(host, pos, facing).is_some_and(rules::is_hand) {
                // `changeMode`.
                let now = util::game_time(host);
                host.change_mod_block_entity(pos, &mut |entity| {
                    let old = read(&entity.nbt);
                    let mut new = old;
                    new.mode = old.mode.toggled();
                    write(entity, &old, &new, now);
                });
                host.block_entity_sync(pos);
                return;
            }
            let Some(clicked) = clicked else {
                return;
            };
            let (next, next_along) = rules::wrench(facing, along_first, clicked);
            if next != facing {
                host.set_block_property(pos, "facing", next.name());
            }
            if next_along != along_first {
                host.set_block_property(pos, "axis_along_first", bool_name(next_along));
            }
        })
    }

    /// `DeployerBlock.useItemOn`: a click on the hand swaps what the player holds with what the
    /// Deployer holds. Sneaking elsewhere with an item sets the filter, with an empty hand clears
    /// it (the stand-in for the filter slot).
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.fake || player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        let held = host.held_stack(player.uuid);
        // `AllItems.WRENCH.isIn(heldByPlayer)`: the Wrench has its own say.
        if held.as_ref().is_some_and(|stack| &*stack.item == util::WRENCH) {
            return Flow::Continue;
        }
        let Some((facing, _)) = state_of(host, pos) else {
            return Flow::Continue;
        };
        let on_hand = click_along(host, pos, facing).map_or_else(
            // No ray: the face the click named.
            || Dir::from_normal(event.face) == Some(facing),
            rules::is_hand,
        );
        if !on_hand {
            if player.sneaking {
                if let Some(line) = inv::click(host, pos, held.as_ref()) {
                    inv::tell(host, &line);
                }
                return Flow::Cancel;
            }
            return Flow::Continue;
        }
        // A player sneaking with something in hand uses the item, not the block.
        if player.sneaking && held.is_some() {
            return Flow::Continue;
        }
        swap(host, pos, &player, held);
        Flow::Cancel
    }
}

/// The part of `tick` after the speed check (`rpm != 0`): the crack, the countdown and the state
/// machine. True when clients need to hear of a change.
fn stroke(
    host: &mut dyn Host,
    pos: Pos,
    facing: Dir,
    entity: &mut ModBlockEntity,
    saved: &mut Saved,
    rpm: f32,
    now: i64,
) -> bool {
    // A crack on a block that is gone is taken away.
    if let Some(crack) = saved.crack
        && host.block(crack.pos) == "minecraft:air"
    {
        host.set_break_progress(drill::breaker_id(pos), crack.pos, -1);
        saved.crack = None;
    }
    if saved.stroke.count_down(rpm) {
        return false;
    }
    match saved.stroke.state {
        State::Waiting => waiting(host, pos, facing, entity, saved, rpm, now),
        State::Expanding => {
            if saved.stroke.fist_bump {
                trigger_fist_bump(host, pos, facing, saved, now);
            }
            activate(host, pos, facing, entity, saved);
            saved.stroke.expanded();
            true
        }
        State::Retracting => {
            saved.stroke.retracted();
            true
        }
        State::Dumping => false,
    }
}
