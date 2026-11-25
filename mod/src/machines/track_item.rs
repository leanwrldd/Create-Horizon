//! The Train Track item (`TrackBlockItem`, `TrackPlacement`): the click that picks a curve's start and the click that
//! puts the curve down. With a track in hand, clicking a piece of track stores where the curve starts on the stack
//! (`TRACK_CONNECTING_FROM`, here the component `create:track_connecting_from`); clicking a second piece, or a block to
//! put one on, asks the merged rules (`rules::trains::placement::try_connect`) whether the curve is allowed and what it
//! sets down (`rules::trains::track_block::plan_tracks`), then places the straight runs, the two curve ends with their
//! `Connections`, takes the tracks it costs and queues the graph update (`track.rs`). A refusal is the key
//! `PlacementInfo.message` carries, shown in the action bar. Sneaking, or a click in the air while sneaking, clears the
//! selection.
//!
//! Known differences from Create, for the README:
//! - The selection is written to the held stack at the next inventory tick (a click handler can't change a stack);
//!   until then it is remembered by player. The selection shows as the enchantment glint (`isFoil`).
//! - No preview while holding a selection (`TrackPlacement.clientTick`: the outlined rails, the green and red
//!   hints, the "Can Connect" line): the result is told only when the second click is made.
//! - No sprint-maximised turns (`PlaceExtendedCurvePacket`), no girders on a curve (the off-hand girder) and no
//!   pavement (`paveTracks`, the off-hand block): a curve is always the standard one, bare.
//! - The max length is Create's default config, 32.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};

use create_kinetics::rules::trains::placement::{ConnectingFrom, PlacementInput, try_connect};
use create_kinetics::rules::trains::track_block::{self as rules, COMPONENT, MATERIAL, TRACK, WorldAsk};
use create_kinetics::rules::trains::track_shape::TrackShape;
use create_kinetics::rules::trains::vec::Vec3;
use create_kinetics::Pos;
use crate::compat::TextExt;
use crate::compat::{
    ComponentTypesEvent, Flow, Hooks, Host, InventoryTickEvent, ItemStack, NativeServerPlayer, Tag, Text, UseItemEvent,
    UseItemOnEvent,
};

use super::track as world;
use crate::util;

struct TrackItem;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(TrackItem)
}

/// `AllConfigs.server().trains.maxTrackPlacementLength`.
const MAX_LENGTH: i32 = 32;
/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;
/// The glint that shows a selection (`TrackBlockItem.isFoil`).
const GLINT: &str = "minecraft:enchantment_glint_override";

/// What each player's selection should be on the held stack: `Some(None)` clears it. Written by the next inventory tick
/// and read first by every click (the stack lags a tick behind).
static PENDING: LazyLock<Mutex<HashMap<u128, Option<ConnectingFrom>>>> = LazyLock::new(Mutex::default);
static PENDING_COUNT: AtomicUsize = AtomicUsize::new(0);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn set_pending(uuid: u128, value: Option<ConnectingFrom>) {
    let mut pending = lock(&PENDING);
    pending.insert(uuid, value);
    PENDING_COUNT.store(pending.len(), Ordering::Relaxed);
}

// --- the component (`ConnectingFrom.CODEC`: pos, axis, normal, end) -------------------------------------------------

fn tag_of(from: &ConnectingFrom) -> Tag {
    let mut c = crate::compat::Compound::new();
    c.insert("pos".into(), Tag::IntArray(from.pos.to_vec()));
    c.insert("axis".into(), world::vec_tag(from.axis));
    c.insert("normal".into(), world::vec_tag(from.normal));
    c.insert("end".into(), world::vec_tag(from.end));
    Tag::Compound(c)
}

fn from_tag(tag: &Tag) -> Option<ConnectingFrom> {
    let pos = match tag.get("pos") {
        Some(Tag::IntArray(v)) if v.len() == 3 => [v[0], v[1], v[2]],
        _ => return None,
    };
    Some(ConnectingFrom {
        pos,
        axis: world::vec_of(tag.get("axis")?)?,
        normal: world::vec_of(tag.get("normal")?)?,
        end: world::vec_of(tag.get("end")?)?,
    })
}

/// The selection a player has on the track in hand (`stack.get(TRACK_CONNECTING_FROM)`).
fn selection(uuid: u128, stack: &ItemStack) -> Option<ConnectingFrom> {
    if let Some(pending) = lock(&PENDING).get(&uuid) {
        return *pending;
    }
    stack.data()?.other(COMPONENT).and_then(from_tag)
}

// --- messages -------------------------------------------------------------------------------------------------------

fn say(host: &mut dyn Host, uuid: u128, key: &str) {
    let line = Text::translate_with_fallback(&rules::message_key(key), rules::message_text(key), &[]);
    host.action_bar(uuid, line);
}

fn deny(host: &mut dyn Host, player: &NativeServerPlayer) {
    host.play_sound("create:deny", player.position, BLOCKS, 1.0, 1.0);
}

// --- clicks ---------------------------------------------------------------------------------------------------------

/// `TrackBlockItem.clearSelection`.
fn clear(host: &mut dyn Host, player: &NativeServerPlayer) {
    set_pending(player.uuid, None);
    say(host, player.uuid, "track.selection_cleared");
    host.play_sound("minecraft:entity.item_frame.remove_item", player.position, BLOCKS, 0.75, 1.0);
}

/// The first click: a track piece picked as the curve's start (`TrackBlockItem.useOn` without a selection). `None`
/// leaves the click to the game (an ordinary placement).
fn start(host: &mut dyn Host, player: &NativeServerPlayer, pos: Pos) -> Option<Flow> {
    let shape = world::shape_at(&*host, pos)?;
    if !util::first_click(&*host, player.uuid, pos) {
        return Some(Flow::Cancel);
    }
    if shape.is_junction() {
        say(host, player.uuid, "track.junction_start");
        return Some(Flow::Cancel);
    }
    // `TrackBlockItem.useOn`: a tilted block can't start a curve either.
    if world::is_tilted(&*host, pos) {
        say(host, player.uuid, "track.turn_start");
        return Some(Flow::Cancel);
    }
    let look = world::look_of(player.yaw, player.pitch);
    set_pending(player.uuid, Some(rules::select(pos, shape, look)));
    host.play_sound("minecraft:entity.item_frame.add_item", util::centre(pos), BLOCKS, 0.75, 1.0);
    Some(Flow::Cancel)
}

fn count_tracks(host: &dyn Host, uuid: u128) -> i32 {
    host.player_stacks(uuid)
        .unwrap_or_default()
        .iter()
        .flatten()
        .filter(|stack| &*stack.item == TRACK)
        .map(|stack| stack.count as i32)
        .sum()
}

/// The second click (`TrackBlockItem.useOn` with a selection): the curve, or the message why not.
fn connect(host: &mut dyn Host, player: &NativeServerPlayer, from: ConnectingFrom, clicked: Pos, face: [i32; 3]) -> Flow {
    if !util::first_click(&*host, player.uuid, clicked) {
        return Flow::Cancel;
    }
    let look: Vec3 = world::look_of(player.yaw, player.pitch);
    // The clicked end: a track piece, or the block a piece would be put in.
    let placing = world::shape_at(&*host, clicked).is_none();
    let (pos2, shape2) = if placing {
        let pos = if world::can_replace(&*host, clicked) { clicked } else { util::add(clicked, face) };
        if !world::can_replace(&*host, pos) {
            return Flow::Cancel;
        }
        (pos, world::placement_shape(&*host, pos, look, player.yaw))
    } else {
        (clicked, world::shape_at(&*host, clicked).unwrap_or(TrackShape::Zo))
    };
    let (axis2, normal2, end2) = rules::target_end(pos2, shape2, look);
    let shape1 = world::shape_at(&*host, from.pos);
    let start1 = |axis: Vec3| rules::curve_start(from.pos, axis);
    let start2 = |axis: Vec3| rules::curve_start(pos2, axis);
    let input = PlacementInput {
        from,
        original_has_be: shape1.is_some(),
        pos2,
        axis2,
        normal2,
        end2,
        target_tilted: world::is_tilted(&*host, pos2),
        curve_start1: &start1,
        curve_start2: &start2,
        max_length: MAX_LENGTH,
        maximise_turn: false,
        girder: false,
        material: MATERIAL.to_owned(),
        client: false,
    };
    let info = try_connect(&input);
    if !info.valid {
        if let Some(error) = info.error {
            say(host, player.uuid, error.key());
        }
        deny(host, player);
        return Flow::Cancel;
    }
    let Some(shape1) = shape1 else { return Flow::Cancel };
    let curve = info.curve.clone();
    let connected = |host: &dyn Host| {
        curve.as_ref().is_some_and(|bc| {
            let (t1, t2) = (info.target_pos1, info.target_pos2);
            world::shape_at(host, t1).is_some()
                && world::shape_at(host, t2).is_some()
                && host.block_property(t1, "turn").as_deref() == Some("true")
                && host.block_property(t2, "turn").as_deref() == Some("true")
                && world::connections_at(host, t1).iter().any(|c| c.key() == t2 && bc.key() == t2)
        })
    };
    let plan = {
        let h: &dyn Host = &*host;
        let can_replace = |p: Pos| world::can_replace(h, p);
        let track_at = |p: Pos| world::shape_at(h, p);
        let ask = WorldAsk { can_replace: &can_replace, track_at: &track_at, already_connected: connected(h) };
        rules::plan_tracks(&info, shape1, shape2, &ask)
    };
    let creative = player.game_mode == util::CREATIVE;
    if !creative {
        if count_tracks(&*host, player.uuid) < plan.required_tracks {
            say(host, player.uuid, "track.not_enough_tracks");
            deny(host, player);
            return Flow::Cancel;
        }
        if plan.required_tracks > 0 {
            host.take_items(TRACK, plan.required_tracks as u32);
        }
    }

    // `placeTracks`: the blocks, then the curve in both ends' block entities.
    let dimension = host.dimension().to_owned();
    if let Some(track) = host.default_state(TRACK) {
        for block in &plan.blocks {
            let existing = world::shape_at(&*host, block.pos);
            if existing.is_none() {
                host.set_block(&dimension, block.pos, track, util::UPDATE_ALL);
            }
            if existing != Some(block.shape) && (existing.is_some() || block.shape != TrackShape::Zo) {
                host.set_block_property(block.pos, "shape", block.shape.serialized_name());
            }
        }
    }
    if let Some(bc) = &info.curve {
        world::add_connection(host, info.target_pos1, bc);
        world::add_connection(host, info.target_pos2, &bc.secondary());
        // `TrackPlacement.placeTracks`: both ends try to smooth.
        world::apply_tilt(host, info.target_pos1);
        world::apply_tilt(host, info.target_pos2);
    }
    for block in &plan.blocks {
        world::queue_added(&dimension, block.pos);
    }
    set_pending(player.uuid, None);
    host.play_sound("minecraft:block.metal.place", util::centre(pos2), BLOCKS, 1.0, 1.2);
    Flow::Cancel
}

impl Hooks for TrackItem {
    fn component_types(&self, event: &mut ComponentTypesEvent<'_>) {
        if !event.types.iter().any(|t| t == COMPONENT) {
            event.types.push(COMPONENT.to_owned());
        }
    }

    /// `TrackBlockItem.useOn`.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != TRACK {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else { return Flow::Continue };
        if player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        match selection(player.uuid, event.stack) {
            None => start(host, &player, event.pos).unwrap_or(Flow::Continue),
            Some(_) if player.sneaking => {
                if util::first_click(&*host, player.uuid, event.pos) {
                    clear(host, &player);
                }
                Flow::Cancel
            }
            Some(from) => connect(host, &player, from, event.pos, event.face),
        }
    }

    /// `TrackBlockItem.use`: sneaking with a selection clears it, in the air too.
    fn use_item(&self, host: &mut dyn Host, event: &mut UseItemEvent<'_>) -> Flow {
        if &*event.stack.item != TRACK {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else { return Flow::Continue };
        if player.sneaking && selection(player.uuid, event.stack).is_some() {
            clear(host, &player);
            return Flow::Cancel;
        }
        Flow::Continue
    }

    /// Writes the selection to the held stack: the component and the glint, or neither.
    fn inventory_tick(&self, host: &mut dyn Host, event: &mut InventoryTickEvent) {
        if !event.selected || PENDING_COUNT.load(Ordering::Relaxed) == 0 {
            return;
        }
        let Some(player) = host.acting_player() else { return };
        let Some(value) = lock(&PENDING).get(&player.uuid).copied() else { return };
        let Some(mut stack) = host.carried_stack(event.slot).map(|s| s.clone()) else { return };
        if &*stack.item != TRACK {
            return;
        }
        match value {
            Some(from) => {
                let _ = host.set_component(&mut stack, COMPONENT, tag_of(&from));
                let _ = host.set_component(&mut stack, GLINT, Tag::Byte(1));
            }
            None => {
                let _ = host.remove_component(&mut stack, COMPONENT);
                let _ = host.remove_component(&mut stack, GLINT);
            }
        }
        if let Some(slot) = host.carried_stack(event.slot) {
            *slot = stack;
        }
        let mut pending = lock(&PENDING);
        pending.remove(&player.uuid);
        PENDING_COUNT.store(pending.len(), Ordering::Relaxed);
    }
}
