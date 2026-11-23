//! The edge point items (`TrackTargetingBlockItem`): the Train Station, Signal and Observer are each placed with two clicks. A
//! click on a piece of track picks the spot (the track position and the way the look points along it) and checks it
//! (`withGraphLocation`: not an intersection, nothing within three quarters of a block, a graph there); the click on the block
//! that puts the item down takes the selection along (`TargetTrack` as the offset to the track, `TargetDirection`) and the block
//! entity makes its point from it. Sneaking clears the selection.
//!
//! The rules are `create_kinetics::rules::trains::{graph_helper, edge_blocks, track_block}`; the block entities that read the
//! data are `track_station.rs`, `track_signal.rs` and `track_observer.rs`.
//!
//! Known differences from Create, for the README:
//! - The selection is one component, `create:track_targeting_item_selection` (the position and the direction), where Create has
//!   three (`TRACK_TARGETING_ITEM_SELECTED_POS`, `_DIRECTION`, `_BEZIER`); it is written to the held stack at the next inventory
//!   tick (a click can't change a stack), and remembered by player until then. It shows as the enchantment glint.
//! - **A point on a curve can't be selected** (`useOnCurve` is a client ray at the curve's segments, `CurvedTrackSelectionPacket`):
//!   only the straight track targets. A curve's end block that is a straight piece too does.
//! - The selection is kept when the block can't be placed (Create clears it before it tries).
//! - The signal advancement (`AllAdvancements.SIGNAL`) isn't awarded.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};

use create_kinetics::rules::trains::edge_blocks as rules;
use create_kinetics::rules::trains::graph::EdgePointType;
use create_kinetics::rules::trains::graph_helper::{OverlapResult, TrackQuery, with_graph_location};
use create_kinetics::rules::trains::track_block::nearest_track_axis;
use create_kinetics::Pos;
use crate::compat::{
    ComponentTypesEvent, Flow, Hooks, Host, InventoryTickEvent, ItemStack, NativeServerPlayer, PlaceBlockEvent, Tag, UseItemOnEvent,
};

use super::track as world;
use super::train_net as net;
use crate::util;

/// The selection's component.
pub const COMPONENT: &str = "create:track_targeting_item_selection";
/// `AllConfigs.server().trains.maxTrackPlacementLength`.
const MAX_LENGTH: i32 = 32;
/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;
/// The glint that shows a selection.
const GLINT: &str = "minecraft:enchantment_glint_override";

/// Where a player has picked: the item it was picked with, the track and the direction (`front`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Selection {
    pos: Pos,
    front: bool,
}

/// What each player's selection should be on the held stack (`Some(None)` clears it), and the item that holds it. Written by the
/// next inventory tick and read first by every click.
static PENDING: LazyLock<Mutex<HashMap<u128, (String, Option<Selection>)>>> = LazyLock::new(Mutex::default);
static PENDING_COUNT: AtomicUsize = AtomicUsize::new(0);

struct Targets;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Targets)
}

fn set_pending(uuid: u128, item: &str, value: Option<Selection>) {
    let mut pending = net::lock(&PENDING);
    pending.insert(uuid, (item.to_owned(), value));
    PENDING_COUNT.store(pending.len(), Ordering::Relaxed);
}

fn type_of(item: &str) -> Option<EdgePointType> {
    match item {
        "create:track_station" => Some(EdgePointType::Station),
        "create:track_signal" => Some(EdgePointType::Signal),
        "create:track_observer" => Some(EdgePointType::Observer),
        _ => None,
    }
}

fn tag_of(selection: &Selection) -> Tag {
    let mut c = crate::compat::Compound::new();
    c.insert("pos".into(), Tag::IntArray(selection.pos.to_vec()));
    c.insert("front".into(), Tag::Byte(i8::from(selection.front)));
    Tag::Compound(c)
}

fn from_tag(tag: &Tag) -> Option<Selection> {
    let pos = match tag.get("pos") {
        Some(Tag::IntArray(v)) if v.len() == 3 => [v[0], v[1], v[2]],
        _ => return None,
    };
    let front = matches!(tag.get("front"), Some(Tag::Byte(b)) if *b != 0);
    Some(Selection { pos, front })
}

/// The selection a player has on the item in hand.
fn selection(uuid: u128, stack: &ItemStack) -> Option<Selection> {
    if let Some((item, pending)) = net::lock(&PENDING).get(&uuid)
        && **item == *stack.item
    {
        return *pending;
    }
    stack.data()?.other(COMPONENT).and_then(from_tag)
}

fn click_sound(host: &mut dyn Host, pos: Pos, volume: f32, pitch: f32) {
    host.play_sound("create:controller_click", util::centre(pos), BLOCKS, volume, pitch);
}

fn deny(host: &mut dyn Host, pos: Pos, volume: f32) {
    host.play_sound("create:deny", util::centre(pos), BLOCKS, volume, 1.0);
}

/// `useOn` on a track block: the first click.
fn pick(host: &mut dyn Host, player: &NativeServerPlayer, item: &str, ty: EdgePointType, pos: Pos) -> Flow {
    if !util::first_click(&*host, player.uuid, pos) {
        return Flow::Cancel;
    }
    let query = net::query(&*host);
    let axes = query.axes(pos);
    let look = world::look_of(player.yaw, player.pitch);
    let (_, front) = nearest_track_axis(&axes, look);
    let result = net::with_nets(&*host, |nets| with_graph_location(&query, nets, pos, front, None, ty).0).unwrap_or(OverlapResult::NoTrack);
    if let Some(key) = result.feedback() {
        net::say(host, player.uuid, key, &[]);
        deny(host, pos, 0.5);
        return Flow::Cancel;
    }
    set_pending(player.uuid, item, Some(Selection { pos, front }));
    net::say(host, player.uuid, "create.track_target.set", &[]);
    click_sound(host, pos, 1.0, 1.0);
    Flow::Cancel
}

impl Hooks for Targets {
    fn component_types(&self, event: &mut ComponentTypesEvent<'_>) {
        if !event.types.iter().any(|t| t == COMPONENT) {
            event.types.push(COMPONENT.to_owned());
        }
    }

    /// `TrackTargetingBlockItem.useOn`.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let item = &*event.stack.item;
        let Some(ty) = type_of(item) else { return Flow::Continue };
        let Some(player) = host.acting_player() else { return Flow::Continue };
        if player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        let pos = event.pos;
        let selected = selection(player.uuid, event.stack);
        if player.sneaking && selected.is_some() {
            if util::first_click(&*host, player.uuid, pos) {
                set_pending(player.uuid, item, None);
                net::say(host, player.uuid, "create.track_target.clear", &[]);
                click_sound(host, pos, 1.0, 0.5);
            }
            return Flow::Cancel;
        }
        if world::shape_at(&*host, pos).is_some() {
            return pick(host, &player, item, ty, pos);
        }
        let Some(selected) = selected else {
            if util::first_click(&*host, player.uuid, pos) {
                net::say(host, player.uuid, "create.track_target.missing", &[]);
            }
            return Flow::Cancel;
        };
        // `placedPos`: the clicked block itself when it can be replaced, otherwise the one beside it.
        let placed = if world::can_replace(&*host, pos) { pos } else { util::add(pos, event.face) };
        if !rules::closer_than(selected.pos, placed, rules::targeting_reach(false, MAX_LENGTH)) {
            if util::first_click(&*host, player.uuid, pos) {
                net::say(host, player.uuid, "create.track_target.too_far", &[]);
            }
            return Flow::Cancel;
        }
        // The game places the block; `place_block` carries the selection over.
        Flow::Continue
    }

    /// `TrackTargetingBlockItem.useOn`'s second half: the new block's data, and the selection spent.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        let item = &*event.stack.item;
        if type_of(item).is_none() || host.block(event.pos) != item {
            return;
        }
        let Some(player) = host.acting_player() else { return };
        let Some(selected) = selection(player.uuid, event.stack) else { return };
        let data = net::item_data(selected.pos, selected.front, None, event.pos);
        host.change_mod_block_entity(event.pos, &mut |e| {
            for (key, value) in &data {
                e.nbt.insert(key.clone(), value.clone());
            }
        });
        host.block_entity_sync(event.pos);
        set_pending(player.uuid, item, None);
        net::say(host, player.uuid, "create.track_target.success", &[]);
    }

    /// Writes the selection to the held stack: the component and the glint, or neither.
    fn inventory_tick(&self, host: &mut dyn Host, event: &mut InventoryTickEvent) {
        if !event.selected || PENDING_COUNT.load(Ordering::Relaxed) == 0 {
            return;
        }
        let Some(player) = host.acting_player() else { return };
        let Some((item, value)) = net::lock(&PENDING).get(&player.uuid).cloned() else { return };
        let Some(mut stack) = host.carried_stack(event.slot).map(|s| s.clone()) else { return };
        if *stack.item != *item {
            return;
        }
        match value {
            Some(selection) => {
                let _ = host.set_component(&mut stack, COMPONENT, tag_of(&selection));
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
        let mut pending = net::lock(&PENDING);
        pending.remove(&player.uuid);
        PENDING_COUNT.store(pending.len(), Ordering::Relaxed);
    }
}
