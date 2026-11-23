//! Clicking blocks on a moving structure (`ContraptionHandlerClient.rightClickingOnContraptionsGetsHandledLocally`,
//! `AbstractContraptionEntity.handlePlayerInteraction`, `ContraptionInteractionPacket`): a right click whose
//! ray meets a block of a structure before any real block opens that block's behaviour: levers switch,
//! wooden doors, trapdoors and fence gates open, seats seat, Deployers swap the held item (or turn their mode
//! for the Wrench) and Contraption Controls switch actors. The rules are
//! `create_kinetics::rules::contraption_click`; this reads the structure and writes what they say.
//!
//! # Where the click is heard
//!
//! A structure is not a block, so the game aims the click at what is behind it, or at the air. Three hooks
//! hear that, all handled on the server (the client lets the click through, as every other machine here does):
//!
//! - `right_click_block` (both sides): the click aimed at a real block. The structure wins when its block is
//!   nearer than the point clicked (Create clips the ray to `mc.hitResult`).
//! - `right_click_item` (both sides): a held item, aimed at the air (a click on a real block is heard by the hook
//!   above, and the item hook it falls through to is ignored in the same tick).
//! - `right_click_empty` (**client only**, empty hand, nothing aimed at): the client sends one message
//!   (`create:structure_click`), the server rays from where its player stands, looks and reaches.
//!
//! A click that opens something is cancelled with `Success` (Create cancels the use and does not swing).
//!
//! # Known differences from Create, for the README
//!
//! - The ray starts at the server's idea of the eye (position, eye height, yaw and pitch) and reaches 4.5 blocks
//!   (5 in creative), not the player's `block_interaction_range` attribute.
//! - The client does not cancel the click itself (Create's client does, before the packet): the game's own
//!   prediction (a block placed at the face behind) is corrected by the server's answer.
//! - Only the main hand acts (the Deployer's swap, the Wrench).
//! - `Contraption.disabledActors` is kept in memory with the actors (not saved with the structure), so a
//!   reload starts from the Controls' saved `Disabled` again. The Controls' button does not sink on a structure,
//!   and `ContraptionDisableActorPacket` (the client's copy of the list) is not sent: the client draws what the
//!   server's actors write.
//! - The Controls' elevator branch (`ElevatorTargetFloorPacket`), the Train Door, the Conductor's burner, the
//!   Depot's `MountedDepotInteractionBehaviour`, train controls and `handlePlayerStorageInteraction` (a chest on
//!   a structure) are not here.
//! - A Deployer's fake player is not made for the swap: the hand is this port's own (`actor_tools`).

use create_kinetics::Pos;
use create_kinetics::rules::collider::StructurePose;
use create_kinetics::rules::contraption_click::{
    self as rules, ClickBlock, DeployerClick, Hit, Interaction, Route, Toggle,
};
use create_kinetics::rules::nixie_tube::look_vector;

use crate::compat::{Adapter, Compound, Flow, Hooks, Host, ItemStack, NetFromClientEvent, Tag};
use crate::machines::contraption::{self as engine, CONTRAPTION, Moving};
use crate::solid::registry::{self, Entry, Side};
use crate::solid::shapes;
use crate::util;

pub const CHANNEL: &str = "create:structure_click";
const WRENCH: &str = "create:wrench";
/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;
/// `Player.blockInteractionRange`, survival and creative.
const REACH: f64 = 4.5;
const CREATIVE_REACH: f64 = 5.0;
const CREATIVE: i32 = 1;

struct Shared;

impl Hooks for Shared {
    /// The client's click in the air with an empty hand.
    fn net_from_client(&self, host: &mut dyn Host, event: &mut NetFromClientEvent<'_>) {
        if event.channel != CHANNEL {
            return;
        }
        // `ServerHost` is behind the adapter: the message is answered where the hook can reach it.
        if let Some(pending) = PENDING.lock().ok().as_mut() {
            if pending.len() < 64 {
                pending.push(event.player);
            }
        }
        let _ = host;
    }
}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

/// Players whose empty-handed click arrived, waiting for the level tick (the hook is given the old host,
/// and the new API's `ServerHost` is what the structure's shapes need).
static PENDING: std::sync::Mutex<Vec<u128>> = std::sync::Mutex::new(Vec::new());

/// The level tick: answers the messages that arrived.
pub fn level_tick(host: &mut dyn mod_api::ServerHost) {
    let waiting: Vec<u128> = match PENDING.lock() {
        Ok(mut pending) => std::mem::take(&mut *pending),
        Err(_) => return,
    };
    for who in waiting {
        handle(host, who, None);
    }
}

// --- the hooks ---------------------------------------------------------------------------------------------

/// `right_click_block` on either side. Cancels (Success) a click a structure's block took.
pub fn right_click_block(host: &mut mod_api::HostSide<'_>, event: &mut mod_api::RightClickBlockEvent<'_>) -> mod_api::Flow {
    let mod_api::HostSide::Server(host) = host else { return mod_api::Flow::Continue };
    let click = event.click;
    if click.hand != mod_api::Hand::Main {
        // The off hand's turn of a click the main hand took.
        return if repeated(&mut **host, click.player) { cancel(event.cancellation_result) } else { mod_api::Flow::Continue };
    }
    if repeated(&mut **host, click.player) {
        return cancel(event.cancellation_result);
    }
    note_block_click(&mut **host, click.player);
    let eye_to_hit = eye_of(&mut **host, click.player).map(|(eye, _)| distance(eye, click.hit.point));
    if handle(&mut **host, click.player, eye_to_hit) {
        taken(&mut **host, click.player);
        return cancel(event.cancellation_result);
    }
    mod_api::Flow::Continue
}

/// `right_click_item` on either side: a held item, aimed at the air.
pub fn right_click_item(host: &mut mod_api::HostSide<'_>, event: &mut mod_api::RightClickItemEvent<'_>) -> mod_api::Flow {
    let mod_api::HostSide::Server(host) = host else { return mod_api::Flow::Continue };
    let click = event.click;
    if repeated(&mut **host, click.player) {
        return cancel(event.cancellation_result);
    }
    // The click met a real block first (`right_click_block` has looked at it already).
    if click.hand == mod_api::Hand::Main && !at_a_block(&mut **host, click.player) && handle(&mut **host, click.player, None) {
        taken(&mut **host, click.player);
        return cancel(event.cancellation_result);
    }
    mod_api::Flow::Continue
}

/// `right_click_empty` (client): tells the server.
pub fn right_click_empty(host: &mut dyn mod_api::ClientHost, event: &mut mod_api::RightClickEmptyEvent) {
    if event.hand != mod_api::Hand::Main || !registry::client_any() {
        return;
    }
    let mut adapter = Adapter::ClientMut(host);
    let _ = Host::net_send_to_server(&mut adapter, CHANNEL, &[1]);
}

fn cancel(result: &mut mod_api::InteractionResult) -> mod_api::Flow {
    *result = mod_api::InteractionResult::Success;
    mod_api::Flow::Cancel
}

// --- the click's memory (`click.rs`) -----------------------------------------------------------------------

static LAST_BLOCK_CLICK: std::sync::Mutex<Option<std::collections::HashMap<u128, i64>>> = std::sync::Mutex::new(None);

fn now(host: &mut dyn mod_api::ServerHost) -> i64 {
    let dimension = host.dimension();
    host.level(dimension).and_then(|level| level.game_time(&*host)).unwrap_or(0)
}

/// The same click, heard again (the other hand, the item hook after the block hook).
fn repeated(host: &mut dyn mod_api::ServerHost, who: u128) -> bool {
    let adapter = Adapter::Server(host);
    util::click_repeated_anywhere(&adapter, who)
}

fn taken(host: &mut dyn mod_api::ServerHost, who: u128) {
    let adapter = Adapter::Server(host);
    util::click_taken(&adapter, who, [0; 3]);
}

fn note_block_click(host: &mut dyn mod_api::ServerHost, who: u128) {
    let t = now(host);
    LAST_BLOCK_CLICK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_or_insert_with(Default::default)
        .insert(who, t);
}

/// Whether `right_click_block` was heard for this player this tick (the click was aimed at a block).
fn at_a_block(host: &mut dyn mod_api::ServerHost, who: u128) -> bool {
    let t = now(host);
    LAST_BLOCK_CLICK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .and_then(|map| map.get(&who))
        .is_some_and(|&seen| (0..=1).contains(&(t - seen)))
}

// --- finding the block ---------------------------------------------------------------------------------------

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>().sqrt()
}

/// The eye and the look of a player, as the server has them.
fn eye_of(host: &mut dyn mod_api::ServerHost, who: u128) -> Option<([f64; 3], [f64; 3])> {
    let adapter = Adapter::Server(host);
    let players = Host::server_players(&adapter);
    let player = players.iter().find(|p| p.uuid == who)?;
    let eye = [player.position[0], player.position[1] + f64::from(player.eye_height), player.position[2]];
    Some((eye, look_vector(player.yaw, player.pitch)))
}

struct Found {
    controller: Pos,
    entry: Entry,
    pose: StructurePose,
    hit: Hit,
    world: [f64; 3],
    distance: f64,
}

/// `rayTraceContraption` for every structure of the level: the nearest hit within `limit` (the distance to
/// the real block aimed at, if any) and the reach.
fn find(host: &mut dyn mod_api::ServerHost, who: u128, limit: Option<f64>, game_mode: i32) -> Option<(Found, Moving)> {
    let (eye, look) = eye_of(host, who)?;
    let reach = if game_mode == CREATIVE { CREATIVE_REACH } else { REACH };
    let reach = limit.map_or(reach, |l| l.min(reach));
    let dimension = host.dimension();
    let level = host.level(dimension)?;
    let now = level.game_time(&*host)?;
    let target = [0, 1, 2].map(|i| eye[i] + look[i] * reach);
    let mut best: Option<(Found, Moving)> = None;
    for (controller, entry) in registry::active(Side::Server, dimension, now) {
        let pose = entry.pose(now);
        let bounds = entry.solid.world_bounds(&pose);
        if (0..3).any(|i| eye[i].max(target[i]) + 1.0 < bounds.min[i] || eye[i].min(target[i]) - 1.0 > bounds.max[i]) {
            continue;
        }
        let from = pose.to_local_vector(eye, 1.0, false);
        let to = pose.to_local_vector(target, 1.0, false);
        let Some(entity) = Host::mod_block_entity(&Adapter::Server(&mut *host), controller) else { continue };
        let Some(live) = engine::live(dimension, controller, &entity.nbt) else { continue };
        let shapes: Vec<_> =
            live.moving.blocks.iter().map(|b| shapes::outline_of(&*host, level, controller, &b.state)).collect();
        let blocks = live.moving.blocks.iter().zip(&shapes).map(|(b, boxes)| ClickBlock { local: b.local, boxes });
        let Some(hit) = rules::ray_trace(blocks, from, to) else { continue };
        let world = pose.to_global_vector(hit.point, 1.0, false);
        let d = distance(eye, world);
        if best.as_ref().is_none_or(|(b, _)| d < b.distance) {
            best = Some((Found { controller, entry, pose, hit, world, distance: d }, live.moving.clone()));
        }
    }
    best
}

// --- the click -----------------------------------------------------------------------------------------------------

/// `handlePlayerInteraction` for the structure block the player's ray meets within `limit`. True when the
/// click was taken.
fn handle(host: &mut dyn mod_api::ServerHost, who: u128, limit: Option<f64>) -> bool {
    if !registry_has_server_structures(host) {
        return false;
    }
    let (game_mode, held, uuid_ok) = {
        let adapter = Adapter::Server(&mut *host);
        let players = Host::server_players(&adapter);
        let Some(player) = players.iter().find(|p| p.uuid == who) else { return false };
        (player.game_mode, Host::held_stack(&adapter, who), true)
    };
    if !uuid_ok || game_mode == util::SPECTATOR {
        return false;
    }
    let Some((found, moving)) = find(host, who, limit, game_mode) else { return false };
    let Some(block) = moving.blocks.iter().find(|b| b.local == found.hit.local) else { return false };
    let wrench = held.as_ref().is_some_and(|stack| &*stack.item == WRENCH);
    let dimension = host.dimension().to_owned();
    match rules::route(&block.state.name, wrench) {
        Route::Storage => false,
        Route::Sit => sit(host, &dimension, &found, who),
        Route::Behaviour(kind) => behave(host, &dimension, &found, moving, kind, who, held, wrench),
    }
}

fn registry_has_server_structures(host: &mut dyn mod_api::ServerHost) -> bool {
    let dimension = host.dimension();
    let Some(now) = host.level(dimension).and_then(|level| level.game_time(&*host)) else { return false };
    !registry::active(Side::Server, dimension, now).is_empty()
}

fn sit(host: &mut dyn mod_api::ServerHost, dimension: &str, found: &Found, who: u128) -> bool {
    let local = found.hit.local;
    let anchor = found.entry.anchor;
    let cell = [anchor[0] + local[0], anchor[1] + local[1], anchor[2] + local[2]];
    // `SeatEntity.setPos`: the cell's centre, a little below the middle of the seat, as the carried marker.
    let centre = found.pose.to_global_vector([f64::from(local[0]) + 0.5, f64::from(local[1]) + 0.5, f64::from(local[2]) + 0.5], 1.0, false);
    let lift = create_kinetics::rules::seat::carried_lift();
    let at = [centre[0] + lift[0], centre[1] + lift[1], centre[2] + lift[2]];
    let adapter = Adapter::Server(host);
    super::seat::sit_on_structure(&adapter, dimension, found.controller, local, cell, who, at)
}

#[allow(clippy::too_many_arguments)]
fn behave(
    host: &mut dyn mod_api::ServerHost,
    dimension: &str,
    found: &Found,
    moving: Moving,
    kind: Interaction,
    who: u128,
    held: Option<ItemStack>,
    wrench: bool,
) -> bool {
    let local = found.hit.local;
    let Some(block) = moving.blocks.iter().find(|b| b.local == local) else { return false };
    match kind {
        Interaction::Seat => false,
        Interaction::Lever | Interaction::Door | Interaction::Trapdoor | Interaction::FenceGate => {
            let other = [local[0], local[1] + if block.state.get("half") == Some("upper") { -1 } else { 1 }, local[2]];
            let other = moving.blocks.iter().find(|b| b.local == other).map(|b| &b.state);
            let rand = (util::random() % 1000) as f32 / 1000.0;
            let Some(toggle) = rules::simple_toggle(kind, local, &block.state, other, rand) else { return false };
            apply_toggle(host, dimension, found, moving, &toggle);
            true
        }
        Interaction::Deployer => deployer(host, dimension, found, moving, who, held, wrench),
        Interaction::Controls => controls(host, dimension, found, moving, who),
    }
}

/// `setContraptionBlockData` for each change, `invalidateColliders`, and the sound.
fn apply_toggle(host: &mut dyn mod_api::ServerHost, dimension: &str, found: &Found, mut moving: Moving, toggle: &Toggle) {
    for (at, state) in &toggle.changes {
        if let Some(block) = moving.blocks.iter_mut().find(|b| b.local == *at) {
            block.state = state.clone();
        }
    }
    write(host, dimension, found.controller, moving);
    let mut adapter = Adapter::Server(host);
    let sound = &toggle.sound;
    Host::play_sound(&mut adapter, sound.event, found.world, BLOCKS, sound.volume, sound.pitch);
}

/// The structure's changed blocks into the controller's saved structure and the cached copy
/// (`engine::set_live`, as actors do): the controller's next tick rebuilds the colliders from it
/// (the new `Live` is a new token) and the update tag sends the new states to the clients.
fn write(host: &mut dyn mod_api::ServerHost, dimension: &str, controller: Pos, moving: Moving) {
    let saved = moving.to_nbt(false);
    let mut adapter = Adapter::Server(&mut *host);
    Host::change_mod_block_entity(&mut adapter, controller, &mut |entity| {
        entity.nbt.insert(CONTRAPTION.into(), Tag::Compound(saved.clone()));
    });
    Host::block_entity_sync(&mut adapter, controller);
    engine::set_live(dimension, controller, moving);
}

// --- Contraption Controls -----------------------------------------------------------------------------------

/// `ContraptionControlsMovingInteraction.handlePlayerInteraction`.
fn controls(host: &mut dyn mod_api::ServerHost, dimension: &str, found: &Found, moving: Moving, who: u128) -> bool {
    let local = found.hit.local;
    let filter_stack = moving
        .blocks
        .iter()
        .find(|b| b.local == local)
        .and_then(|b| b.data.as_ref())
        .and_then(|data| util::get(data, "Filter"))
        .and_then(util::stack_of);
    let filter = filter_stack.as_ref().map_or_else(String::new, |s| s.item.to_string());
    let Some(disable) = super::actor::toggle_controls(dimension, found.controller, &filter) else { return false };
    let mut adapter = Adapter::Server(host);
    let line = super::contraption_controls::status(filter_stack.as_ref(), !disable);
    Host::action_bar(&mut adapter, who, line);
    // `AllSoundEvents.CONTROLLER_CLICK`, with the stand-in the Controls on the ground use.
    let pitch = create_kinetics::rules::contraption_controls::click_pitch(disable);
    Host::play_sound(&mut adapter, "minecraft:entity.item_frame.add_item", found.world, BLOCKS, 0.35, pitch);
    true
}

// --- the Deployer -------------------------------------------------------------------------------------------------

/// `DeployerMovingInteraction.handlePlayerInteraction`.
fn deployer(
    host: &mut dyn mod_api::ServerHost,
    dimension: &str,
    found: &Found,
    mut moving: Moving,
    who: u128,
    held: Option<ItemStack>,
    wrench: bool,
) -> bool {
    use super::actor::with_actor;
    use super::actor_tools as tools;
    let (controller, local) = (found.controller, found.hit.local);
    let mode = with_actor(dimension, controller, local, |a| tools::deployer_mode(a)).unwrap_or("USE");
    match rules::deployer_click(wrench, mode) {
        DeployerClick::Mode(new) => {
            if with_actor(dimension, controller, local, |a| tools::set_deployer_mode(a, new)).is_none() {
                return false;
            }
            if let Some(block) = moving.blocks.iter_mut().find(|b| b.local == local) {
                block.data.get_or_insert_with(Compound::new).insert("Mode".into(), Tag::String(new.into()));
            }
            write(host, dimension, controller, moving);
            true
        }
        DeployerClick::Swap => {
            // The player's main hand goes empty before the Deployer's item is given (`empty_main_hand`).
            let selected = {
                let adapter = Adapter::Server(&mut *host);
                Host::server_players(&adapter).iter().find(|p| p.uuid == who).map(|p| (p.selected_slot, p.position))
            };
            let Some((slot, position)) = selected else { return false };
            let Some(swapped) = with_actor(dimension, controller, local, |a| tools::swap_held(a, held.clone())) else {
                return false;
            };
            let (items, old) = swapped;
            if held.is_none() && old.is_none() {
                return true;
            }
            let mut adapter = Adapter::Server(&mut *host);
            if let Some(stack) = &held {
                if !take_main_hand(&mut adapter, who, usize::from(slot), stack) {
                    // The hand could not be emptied: put the Deployer's back.
                    let _ = with_actor(dimension, controller, local, |a| tools::swap_held(a, old.clone()));
                    return false;
                }
            }
            if let Some(stack) = old {
                if let Some(rest) = Host::give_stack(&mut adapter, who, stack) {
                    util::spawn_item(&mut adapter, position, &rest, [0.0; 3], 0);
                }
            }
            if let Some(block) = moving.blocks.iter_mut().find(|b| b.local == local) {
                tools::write_hand(block.data.get_or_insert_with(Compound::new), &items);
            }
            drop(adapter);
            write(host, dimension, controller, moving);
            true
        }
    }
}

/// Empties the player's main hand of `stack` (`depot::empty_main_hand`: inside a player's hook only
/// `take_items` works, and only for a plain stack).
fn take_main_hand(host: &mut dyn Host, who: u128, slot: usize, stack: &ItemStack) -> bool {
    use crate::compat::NativeInventoryKind;
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
    host.set_inventory_slot_typed(who, NativeInventoryKind::Player, slot, Some(stack), None).is_ok()
}
