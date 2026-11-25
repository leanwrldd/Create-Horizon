//! Clicking blocks on a moving structure (`ContraptionHandlerClient.rightClickingOnContraptionsGetsHandledLocally`,
//! `AbstractContraptionEntity.handlePlayerInteraction`, `ContraptionInteractionPacket`): a right click whose
//! ray meets a block of a structure before any real block opens that block's behaviour: levers switch,
//! wooden doors, trapdoors and fence gates open, seats seat, Deployers swap the held item (or turn their mode
//! for the Wrench), Contraption Controls switch actors (or, on an elevator's cabin, call the chosen floor).
//! The rules are `create_kinetics::rules::contraption_click`; this reads the structure and writes what they say.
//!
//! # Where the click is heard
//!
//! A structure is not a block, so the game aims the click at what is behind it, or at the air. Hooks hear that:
//!
//! - `right_click_block` (both sides): the click aimed at a real block. The structure wins when its block is
//!   nearer than the point clicked (Create clips the ray to `mc.hitResult`). **The client cancels it itself**
//!   (Create's `InteractionKeyMappingTriggered` does): it rays against the outlines the controller's update
//!   tag carries (`Outline` in each palette entry, beside the collision `Shape`), so the game's own prediction
//!   (a block placed at the face behind) never shows. The use packet still reaches the server, which takes it.
//! - `right_click_item` (both sides): a held item, aimed at the air (a click on a real block is heard by the hook
//!   above, and the item hook it falls through to is ignored in the same tick). The client cancels it the same
//!   way, and, as a cancelled item use sends nothing, sends the click as a message.
//! - `right_click_empty` (**client only**, empty hand, nothing aimed at): the client sends one message
//!   (`create:structure_click`), the server rays from where its player stands, looks and reaches.
//! - `mouse_scrolling` (client): the wheel over an elevator's Controls chooses the floor
//!   (`ElevatorControlsHandler.onScroll`); the client sends the steps, the server keeps the choice.
//!
//! A click a structure's block takes is cancelled with `Consume` (Create cancels the use and does not swing).
//! Create's client cancels every click that meets a structure's block, whether the block does anything or not
//! (a stone block, a chest): so does this.
//!
//! # Known differences from Create, for the README
//!
//! - The server's ray starts at its idea of the eye (position, eye height, yaw and pitch) and reaches the
//!   player's `block_interaction_range` attribute (4.5 survival, 5 creative when it can't be read), as the
//!   client's does.
//! - Only the main hand acts (the Deployer's swap, the Wrench): Create's client acts on the first hand whose key
//!   event meets the structure, which is the main hand, and cancelling it ends the click, so the off hand never
//!   gets a turn. Here the off hand's turn after a taken click is cancelled likewise.
//! - `Contraption.disabledActors` is saved with the structure (`DisabledActors` in the controller's
//!   `Contraption`, written when the Controls are clicked and read when the actors start again), as strings
//!   (Create saves item stacks). The Controls' button does not sink on a structure,
//!   and `ContraptionDisableActorPacket` (the client's copy of the list) is not sent: the client draws what the
//!   server's actors write.
//! - The elevator's Controls choose the floor on the server (Create keeps `ElevatorFloorSelection` in the
//!   client's actor context, and draws the floor's name on the Controls): the wheel steps a choice kept for each
//!   player, the action bar names it, and the click calls it (`ElevatorTargetFloorPacket.handle`'s checks). The
//!   Controls' screen and indicator are not drawn.
//! - `handlePlayerStorageInteraction`: a chest, barrel or shulker box on a structure opens a chest view of its own
//!   slots (`open_storage`; a double chest only the clicked half, an Item Vault nothing). The Depot's
//!   `MountedDepotInteractionBehaviour` is not here: a click on such a block is taken (cancelled) and does nothing. The
//!   Train Doors (`DoorMovingInteraction` on a `SlidingDoorBlock`), the Conductor's Blaze Burner
//!   (`ConductorBlockInteractionBehavior`) and the train controls are (`rules::trains::{train_doors, schedule_hand}`).
//! - A Deployer's fake player is not made for the swap: the hand is this port's own (`actor_tools`).

use std::collections::BTreeMap;

use create_kinetics::Pos;
use create_kinetics::rules::collider::StructurePose;
use create_kinetics::rules::contraption_click::{
    self as rules, ClickBlock, DeployerClick, FloorClick, Hit, Interaction, Route, Toggle,
};
use create_kinetics::rules::nixie_tube::look_vector;

use crate::compat::TextExt;
use crate::compat::{Adapter, Compound, Hooks, Host, ItemStack, MenuInputClosedEvent, NetFromClientEvent, Tag, Text};
use crate::machines::contraption::{self as engine, CONTRAPTION, Moving};
use crate::solid::registry::{self, Entry, Side};
use crate::solid::shapes;
use crate::util;

pub const CHANNEL: &str = "create:structure_click";
const WRENCH: &str = "create:wrench";
const CONTROLS: &str = "create:contraption_controls";
const ELEVATOR_PULLEY: &str = "create:elevator_pulley";
/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;
/// `Player.blockInteractionRange` when the attribute can't be read: survival and creative.
const REACH: f64 = 4.5;
const CREATIVE_REACH: f64 = 5.0;
const CREATIVE: i32 = 1;
const REACH_ATTRIBUTE: &str = "minecraft:block_interaction_range";

/// `player.blockInteractionRange()`: the attribute's value (the Extendo Grip raises it), else the default for the mode.
fn reach_of(host: &dyn mod_api::ServerHost, who: u128, game_mode: i32) -> f64 {
    host.player(who)
        .and_then(|player| host.living(player.entity()))
        .and_then(|living| living.attribute_value(host, REACH_ATTRIBUTE))
        .filter(|reach| reach.is_finite() && *reach > 0.0)
        .unwrap_or(if game_mode == CREATIVE { CREATIVE_REACH } else { REACH })
}

struct Shared;

/// What a message of the client says (the first byte of its data).
#[derive(Clone, Copy, Debug)]
enum Message {
    /// A click in the air with an empty hand (`right_click_empty`), or with an item a structure took.
    Click,
    /// The wheel turned over an elevator's Controls by this many steps.
    Scroll(i8),
}

impl Hooks for Shared {
    /// The client's click in the air with an empty hand, or its wheel over an elevator's Controls.
    fn net_from_client(&self, host: &mut dyn Host, event: &mut NetFromClientEvent<'_>) {
        if event.channel != CHANNEL {
            return;
        }
        let message = match event.data {
            [3, steps, ..] => Message::Scroll(*steps as i8),
            _ => Message::Click,
        };
        // `ServerHost` is behind the adapter: the message is answered where the hook can reach it.
        if let Some(pending) = PENDING.lock().ok().as_mut() {
            if pending.len() < 64 {
                pending.push((event.player, message));
            }
        }
        let _ = host;
    }

    /// A structure chest's view closes: the slots go back into the carried block's `Items`. What arrived in the block
    /// while the view was open (an actor's drops) goes to the player with whatever the view keeps; if the structure or
    /// the block is gone, everything left in the slots goes to the player (the compat layer hands the slots back).
    fn menu_input_closed(&self, host: &mut dyn Host, event: &mut MenuInputClosedEvent<'_>) {
        let Some(open) = lock_views().remove(&event.window) else { return };
        let Some(entity) = host.mod_block_entity(open.controller) else { return };
        let Some(live) = engine::live(&open.dimension, open.controller, &entity.nbt) else { return };
        let mut moving = live.moving.clone();
        let Some(block) = moving.blocks.iter_mut().find(|b| b.local == open.local && b.state.name == open.name) else {
            return;
        };
        let arrived = stored_items(block.data.as_ref(), open.slots).into_iter().flatten();
        let mut entries = Vec::new();
        for (index, slot) in event.slots.iter_mut().enumerate().take(open.slots) {
            if let Some(stack) = slot.take()
                && let Tag::Compound(mut tag) = util::stack_tag(&stack)
            {
                tag.insert("Slot".into(), Tag::Byte(index as i8));
                entries.push(Tag::Compound(tag));
            }
        }
        block.data.get_or_insert_with(Compound::new).insert("Items".into(), Tag::List(entries));
        let mut back = arrived;
        for slot in event.slots.iter_mut() {
            if slot.is_none() {
                *slot = back.next();
            }
        }
        for rest in back {
            util::spawn_item(host, util::centre(open.controller), &rest, [0.0; 3], 10);
        }
        write_host(host, &open.dimension, open.controller, moving);
        host.play_sound("minecraft:block.chest.close", util::centre(open.controller), BLOCKS, 0.5, 1.0);
    }
}

/// A chest view opened on a storage block of a structure, by window id.
struct OpenStorage {
    dimension: String,
    controller: Pos,
    local: Pos,
    name: String,
    slots: usize,
}

static STORAGE_VIEWS: std::sync::Mutex<BTreeMap<i32, OpenStorage>> = std::sync::Mutex::new(BTreeMap::new());

fn lock_views() -> std::sync::MutexGuard<'static, BTreeMap<i32, OpenStorage>> {
    STORAGE_VIEWS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A carried storage block's saved `Items`, slot by slot (`Slot` as a byte or an int).
fn stored_items(data: Option<&Compound>, slots: usize) -> Vec<Option<ItemStack>> {
    let mut items = vec![None; slots];
    if let Some(Tag::List(list)) = data.and_then(|data| util::get(data, "Items")) {
        for entry in list {
            let index = match entry.get("Slot") {
                Some(Tag::Byte(slot)) => usize::from(*slot as u8),
                Some(Tag::Int(slot)) => usize::try_from(*slot).unwrap_or(usize::MAX),
                _ => continue,
            };
            if index < slots {
                items[index] = util::stack_of(entry);
            }
        }
    }
    items
}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

/// Players whose message arrived, waiting for the level tick (the hook is given the old host,
/// and the new API's `ServerHost` is what the structure's shapes need).
static PENDING: std::sync::Mutex<Vec<(u128, Message)>> = std::sync::Mutex::new(Vec::new());

/// The level tick: answers the messages that arrived.
pub fn level_tick(host: &mut dyn mod_api::ServerHost) {
    let waiting: Vec<(u128, Message)> = match PENDING.lock() {
        Ok(mut pending) => std::mem::take(&mut *pending),
        Err(_) => return,
    };
    for (who, message) in waiting {
        match message {
            Message::Click => {
                // The same click may have come twice (the item hook and the client's message).
                if !repeated(host, who) && handle(host, who, None) {
                    taken(host, who);
                }
            }
            Message::Scroll(steps) => scroll(host, who, steps),
        }
    }
}

// --- the hooks ---------------------------------------------------------------------------------------------

/// `right_click_block` on either side. Cancels a click a structure's block took.
pub fn right_click_block(host: &mut mod_api::HostSide<'_>, event: &mut mod_api::RightClickBlockEvent<'_>) -> mod_api::Flow {
    let click = event.click;
    let host = match host {
        mod_api::HostSide::Server(host) => host,
        mod_api::HostSide::Client(host) => {
            // `ContraptionHandlerClient`: the structure's block nearer than the one aimed at takes the click.
            let host: &dyn mod_api::ClientHost = &**host;
            let Some(view) = client_view(host) else { return mod_api::Flow::Continue };
            let aimed = Some(distance(view.eye, click.hit.point));
            return if client_hit(&view, aimed).is_some() { cancel(event.cancellation_result) } else { mod_api::Flow::Continue };
        }
    };
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
    let click = event.click;
    let host = match host {
        mod_api::HostSide::Server(host) => host,
        mod_api::HostSide::Client(host) => {
            let Some(view) = client_view(&**host) else { return mod_api::Flow::Continue };
            // A click on a real block is `right_click_block`'s: it hears this one first when the block took it.
            let aimed = aimed_distance(&**host, view.eye);
            if client_hit(&view, aimed).is_none() {
                return mod_api::Flow::Continue;
            }
            if click.hand == mod_api::Hand::Main {
                // A cancelled item use sends nothing: the click goes as a message.
                let mut adapter = Adapter::ClientMut(&mut **host);
                let _ = Host::net_send_to_server(&mut adapter, CHANNEL, &[2]);
            }
            return cancel(event.cancellation_result);
        }
    };
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

/// `ElevatorControlsHandler.onScroll` (client): the wheel over an elevator's Controls is the floor's, not the hotbar's.
pub fn mouse_scrolling(host: &mut dyn mod_api::ClientHost, event: &mut mod_api::MouseScrollingEvent) -> mod_api::Flow {
    if !registry::client_any() || !host.playing() {
        return mod_api::Flow::Continue;
    }
    let delta = event.scroll[1];
    if delta == 0.0 {
        return mod_api::Flow::Continue;
    }
    let Some(view) = client_view(&*host) else { return mod_api::Flow::Continue };
    let aimed = aimed_distance(&*host, view.eye);
    let Some(hit) = client_hit(&view, aimed) else { return mod_api::Flow::Continue };
    if hit.name != CONTROLS || !hit.elevator {
        return mod_api::Flow::Continue;
    }
    // Rounded away from zero (a touchpad's 0.9 is a step).
    let steps = if delta > 0.0 { delta.ceil() } else { delta.floor() }.clamp(-100.0, 100.0) as i8;
    let mut adapter = Adapter::ClientMut(host);
    let _ = Host::net_send_to_server(&mut adapter, CHANNEL, &[3, steps as u8]);
    mod_api::Flow::Cancel
}

/// Create cancels the use and does not swing: `Consume`.
fn cancel(result: &mut mod_api::InteractionResult) -> mod_api::Flow {
    *result = mod_api::InteractionResult::Consume;
    mod_api::Flow::Cancel
}

// --- the client's ray -----------------------------------------------------------------------------------------

/// The local player's eye, look and reach (`ContraptionHandlerClient.getRayInputs`).
struct View {
    eye: [f64; 3],
    look: [f64; 3],
    reach: f64,
}

fn client_view(host: &dyn mod_api::ClientHost) -> Option<View> {
    if !registry::client_any() {
        return None;
    }
    let camera = host.camera()?;
    let eye = [camera.position[0], camera.position[1] + f64::from(camera.eye_height), camera.position[2]];
    let reach = host
        .camera_entity()
        .and_then(|entity| host.living(entity))
        .and_then(|living| living.attribute_value(host, REACH_ATTRIBUTE))
        .filter(|reach| reach.is_finite() && *reach > 0.0)
        .unwrap_or(REACH);
    Some(View { eye, look: look_vector(camera.yaw, camera.pitch), reach })
}

/// How far the crosshair's block or entity is from the eye (`mc.hitResult`).
fn aimed_distance(host: &dyn mod_api::ClientHost, eye: [f64; 3]) -> Option<f64> {
    match host.crosshair_target()? {
        mod_api::CrosshairTarget::Block { hit, .. } => Some(distance(eye, hit.point)),
        mod_api::CrosshairTarget::Entity { point, .. } => Some(distance(eye, point)),
        _ => None,
    }
}

/// The structure block the client's ray meets before `aimed`, if any.
fn client_hit(view: &View, aimed: Option<f64>) -> Option<crate::solid::client::ClientHit> {
    let reach = rules::ray_reach(view.reach, aimed);
    let hit = crate::solid::client::click_ray(view.eye, view.look, reach)?;
    rules::structure_takes_click(hit.distance, aimed).then_some(hit)
}

// --- the elevator's floor choice ---------------------------------------------------------------------------------

/// The floor each player chose on each elevator's Controls (`ElevatorFloorSelection.currentIndex`): by
/// dimension, controller and player.
static CHOSEN: std::sync::Mutex<Option<std::collections::HashMap<(String, Pos, u128), usize>>> = std::sync::Mutex::new(None);

fn chosen(dimension: &str, controller: Pos, who: u128) -> usize {
    CHOSEN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .and_then(|map| map.get(&(dimension.to_owned(), controller, who)).copied())
        .unwrap_or(0)
}

fn choose(dimension: &str, controller: Pos, who: u128, index: usize) {
    let mut guard = CHOSEN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let map = guard.get_or_insert_with(Default::default);
    if map.len() > 256 {
        map.clear();
    }
    map.insert((dimension.to_owned(), controller, who), index);
}

/// The line the action bar shows for a floor (Create draws it on the Controls: `currentShortName`,
/// `currentLongName`, and the unreachable text in place of the long name).
fn floor_line(short: &str, long: &str, unreachable: bool) -> Text {
    if unreachable {
        let reason = Text::translate_with_fallback("create.contraption.controls.floor_unreachable", "Unreachable", &[]);
        return Text::translate_with_fallback("create.contraption.controls.floor_line", "%s: %s", &[Text::literal(short.to_owned()), reason]);
    }
    Text::literal(if long.is_empty() { short.to_owned() } else { format!("{short}: {long}") })
}

/// `ElevatorControlsHandler.onScroll` on the server: the choice moves, and the action bar says where it is.
fn scroll(host: &mut dyn mod_api::ServerHost, who: u128, steps: i8) {
    if steps == 0 {
        return;
    }
    let Some(game_mode) = player_mode(host, who) else { return };
    if game_mode == util::SPECTATOR {
        return;
    }
    let Some((found, _)) = find(host, who, None, game_mode) else { return };
    let Some(nbt) = found.elevator.as_ref() else { return };
    if found.name != CONTROLS {
        return;
    }
    let dimension = host.dimension().to_owned();
    let mut adapter = Adapter::Server(&mut *host);
    let Some(floors) = super::elevator::floors_of(&adapter, &dimension, nbt) else { return };
    if floors.heights.is_empty() {
        return;
    }
    let before = chosen(&dimension, found.controller, who);
    let index = rules::scroll_floor(before, f64::from(steps), floors.heights.len());
    choose(&dimension, found.controller, who, index);
    let unreachable = floors.heights[index] < floors.min || floors.heights[index] > floors.max;
    let (short, long) = &floors.names[index];
    Host::action_bar(&mut adapter, who, floor_line(short, long, unreachable));
    if index != before {
        // `AllSoundEvents.SCROLL_VALUE`: a rising tick as the choice goes up the list.
        let pitch = 1.0 + 0.5 * index as f32 / floors.heights.len() as f32;
        Host::play_sound(&mut adapter, "minecraft:block.note_block.hat", found.world, BLOCKS, 1.0, pitch);
    }
}

fn player_mode(host: &mut dyn mod_api::ServerHost, who: u128) -> Option<i32> {
    let adapter = Adapter::Server(host);
    Host::server_players(&adapter).iter().find(|p| p.uuid == who).map(|p| p.game_mode)
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
    /// The name of the block hit.
    name: String,
    /// The controller's saved data when it is an elevator pulley (the cabin's floors are read from it).
    elevator: Option<Compound>,
}

/// `rayTraceContraption` for every structure of the level: the nearest hit within `limit` (the distance to
/// the real block aimed at, if any) and the reach.
fn find(host: &mut dyn mod_api::ServerHost, who: u128, limit: Option<f64>, game_mode: i32) -> Option<(Found, Moving)> {
    let (eye, look) = eye_of(host, who)?;
    let reach = reach_of(&*host, who, game_mode);
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
            let name = live.moving.blocks.iter().find(|b| b.local == hit.local).map(|b| b.state.name.clone()).unwrap_or_default();
            let elevator = (entity.id == ELEVATOR_PULLEY).then(|| entity.nbt.clone());
            best = Some((Found { controller, entry, pose, hit, world, distance: d, name, elevator }, live.moving.clone()));
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
    // Create's client cancels the click whatever `handlePlayerInteraction` answers: a block that does nothing
    // (stone, a chest) still takes it.
    let acted = match rules::route(&block.state.name, wrench) {
        Route::Storage => open_storage(host, &dimension, &found, moving, who),
        // A seat on a carriage is carried with it by the train's tick (`trains::carry_riders`).
        Route::Sit => sit(host, &dimension, &found, who),
        Route::Behaviour(kind) => behave(host, &dimension, &found, moving, kind, who, held, wrench),
    };
    // `handleSpecialInteractions`: a Wrench click that nothing on a carriage took starts relocating its train.
    if !acted && wrench {
        let mut adapter = Adapter::Server(&mut *host);
        if let Some((train, _)) = super::trains::carriage_of_anchor(&adapter, found.controller) {
            super::train_relocate::carriage_wrenched(&mut adapter, who, found.world, train);
        }
    }
    true
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
    let mut adapter = Adapter::Server(host);
    // A carriage's seats are kept by its train and carriage, not by the anchor block that moves with it.
    let controller = super::trains::seat_key_of_anchor(&adapter, found.controller).unwrap_or(found.controller);
    // `handlePlayerInteraction`: a player who rides something sits nowhere; `SeatBlock.getLeashed(..).or(player)`: a mob on their
    // lead sits instead. The player's entity id comes from the compat layer's player view (`server_players`).
    let player = Host::server_players(&adapter).into_iter().find(|p| p.uuid == who);
    if let Some(player) = player
        && let Ok(entity) = u32::try_from(player.entity_id)
        && Host::vehicle_of(&adapter, entity).is_none()
        && let Some((mob, mob_uuid)) = super::seat::leashed_by(&mut adapter, who, player.entity_id)
    {
        return super::seat::sit_led_on_structure(&adapter, dimension, controller, local, cell, mob, mob_uuid, at);
    }
    super::seat::sit_on_structure(&adapter, dimension, controller, local, cell, who, at)
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
        Interaction::SlidingDoor => sliding_door(host, dimension, found, moving),
        // `ConductorBlockInteractionBehavior.handlePlayerInteraction` (a wrench click is the Wrench's).
        Interaction::Conductor => {
            !wrench && super::train_riders::burner_click(&mut Adapter::Server(host), found.controller, local, who)
        }
        Interaction::Deployer => deployer(host, dimension, found, moving, who, held, wrench),
        Interaction::Controls => controls(host, dimension, found, moving, who),
        // `ControlsInteractionBehaviour.handlePlayerInteraction` (a wrench click is the Wrench's).
        Interaction::TrainControls => !wrench && super::trains::click_controls(&mut Adapter::Server(host), dimension, found.controller, who),
    }
}

/// `setContraptionBlockData` for each change, `invalidateColliders`, and the sound.
fn apply_toggle(host: &mut dyn mod_api::ServerHost, dimension: &str, found: &Found, moving: Moving, toggle: &Toggle) {
    let sound = Some((toggle.sound.event, toggle.sound.volume, toggle.sound.pitch));
    apply_changes(host, dimension, found, moving, &toggle.changes, sound);
}

fn apply_changes(
    host: &mut dyn mod_api::ServerHost,
    dimension: &str,
    found: &Found,
    mut moving: Moving,
    changes: &[(Pos, create_kinetics::rules::contraption::State)],
    sound: Option<(&str, f32, f32)>,
) {
    for (at, state) in changes {
        if let Some(block) = moving.blocks.iter_mut().find(|b| b.local == *at) {
            block.state = state.clone();
        }
    }
    write(host, dimension, found.controller, moving);
    if let Some((event, volume, pitch)) = sound {
        let mut adapter = Adapter::Server(host);
        Host::play_sound(&mut adapter, event, found.world, BLOCKS, volume, pitch);
    }
}

/// `DoorMovingInteraction.handle` on a Train Door (a `SlidingDoorBlock`): both halves, the other leaf of a double door, and
/// the open sound (volume .3, as `playSound(player, sound, pitch)` of `SimpleBlockMovingInteraction`).
fn sliding_door(host: &mut dyn mod_api::ServerHost, dimension: &str, found: &Found, moving: Moving) -> bool {
    use create_kinetics::rules::trains::train_doors as doors;
    let local = found.hit.local;
    let state_at = |at: Pos| moving.blocks.iter().find(|b| b.local == at).map(|b| b.state.clone());
    let Some(state) = state_at(local) else { return false };
    let other_at = [local[0], local[1] + if state.get("half") == Some("upper") { -1 } else { 1 }, local[2]];
    let double = doors::double_door_pos(local, &state).and_then(|at| state_at(at).map(|s| (at, s)));
    let pitch = (util::random() % 1000) as f32 / 1000.0 * 0.1 + 0.9;
    let Some(click) = doors::click(&state, local, state_at(other_at).as_ref(), double.as_ref().map(|(_, s)| s), pitch) else {
        return false;
    };
    let mut changes = click.changes.clone();
    // The other leaf of a double door is clicked too, with no player (no sound): `handlePlayerInteraction(null, ..)`.
    if let (Some(at), Some((_, leaf))) = (click.double, double.as_ref()) {
        let leaf_other_at = [at[0], at[1] + if leaf.get("half") == Some("upper") { -1 } else { 1 }, at[2]];
        if let Some(second) = doors::click(leaf, at, state_at(leaf_other_at).as_ref(), None, 1.0) {
            changes.extend(second.changes);
        }
    }
    let sound = click.sound.map(|(event, pitch)| (event, 0.3, pitch));
    apply_changes(host, dimension, found, moving.clone(), &changes, sound);
    true
}

/// The structure's changed blocks into the controller's saved structure and the cached copy
/// (`engine::set_live`, as actors do): the controller's next tick rebuilds the colliders from it
/// (the new `Live` is a new token) and the update tag sends the new states to the clients.
fn write(host: &mut dyn mod_api::ServerHost, dimension: &str, controller: Pos, moving: Moving) {
    write_host(&mut Adapter::Server(host), dimension, controller, moving);
}

fn write_host(host: &mut dyn Host, dimension: &str, controller: Pos, moving: Moving) {
    let saved = moving.to_nbt(false);
    host.change_mod_block_entity(controller, &mut |entity| {
        entity.nbt.insert(CONTRAPTION.into(), Tag::Compound(saved.clone()));
    });
    host.block_entity_sync(controller);
    engine::set_live(dimension, controller, moving);
}

/// `MountedStorageManager.handlePlayerStorageInteraction`: a chest, barrel or shulker box on a structure opens its own
/// slots (a double chest opens the half that was clicked; an Item Vault opens nothing here). The block is emptied while
/// the view is open, so a structure that is set down meanwhile can't give the same items out twice; `menu_input_closed`
/// writes the view back.
fn open_storage(host: &mut dyn mod_api::ServerHost, dimension: &str, found: &Found, mut moving: Moving, who: u128) -> bool {
    let local = found.hit.local;
    let Some(block) = moving.blocks.iter_mut().find(|b| b.local == local) else { return false };
    let name = block.state.name.clone();
    let Some(slots) = create_kinetics::rules::portable_interface::item_slots(&name).filter(|n| *n <= 54) else {
        return false;
    };
    let items = stored_items(block.data.as_ref(), slots);
    let rows = slots.div_ceil(9).clamp(1, 6) as u8;
    let title = Text::literal(name.rsplit(':').next().unwrap_or(&name).replace('_', " "));
    let window = Host::open_menu_with_input(&mut Adapter::Server(&mut *host), who, rows, title, items, 0..slots);
    let Some(window) = window else { return false };
    block.data.get_or_insert_with(Compound::new).insert("Items".into(), Tag::List(Vec::new()));
    write(host, dimension, found.controller, moving);
    lock_views().insert(window, OpenStorage { dimension: dimension.to_owned(), controller: found.controller, local, name, slots });
    Host::play_sound(&mut Adapter::Server(host), "minecraft:block.chest.open", found.world, BLOCKS, 0.5, 1.0);
    true
}

// --- Contraption Controls -----------------------------------------------------------------------------------

/// `ContraptionControlsMovingInteraction.handlePlayerInteraction`.
fn controls(host: &mut dyn mod_api::ServerHost, dimension: &str, found: &Found, mut moving: Moving, who: u128) -> bool {
    if let Some(nbt) = &found.elevator {
        return elevator_controls(host, dimension, found, nbt, who);
    }
    let local = found.hit.local;
    let filter_stack = moving
        .blocks
        .iter()
        .find(|b| b.local == local)
        .and_then(|b| b.data.as_ref())
        .and_then(|data| util::get(data, "Filter"))
        .and_then(util::stack_of);
    let filter = filter_stack.as_ref().map_or_else(String::new, |s| s.item.to_string());
    let Some((disable, list)) = super::actor::toggle_controls(dimension, found.controller, &filter) else { return false };
    // `Contraption.disabledActors` is saved with the structure.
    moving.disabled = Some(list);
    write(host, dimension, found.controller, moving);
    let mut adapter = Adapter::Server(host);
    let line = super::contraption_controls::status(filter_stack.as_ref(), !disable);
    Host::action_bar(&mut adapter, who, line);
    // `AllSoundEvents.CONTROLLER_CLICK`, with the stand-in the Controls on the ground use.
    let pitch = create_kinetics::rules::contraption_controls::click_pitch(disable);
    Host::play_sound(&mut adapter, "minecraft:entity.item_frame.add_item", found.world, BLOCKS, 0.35, pitch);
    true
}

/// `ContraptionControlsMovingInteraction.elevatorInteraction`: the sounds (the server's branch), and the floor the
/// player chose (the client's branch sends `ElevatorTargetFloorPacket`, whose handler is `call_floor`).
fn elevator_controls(host: &mut dyn mod_api::ServerHost, dimension: &str, found: &Found, nbt: &Compound, who: u128) -> bool {
    let mut adapter = Adapter::Server(host);
    // `CONTROLLER_CLICK` at pitch 1.5 and `CONTRAPTION_ASSEMBLE` (0.75, 0.8), with the stand-ins used elsewhere.
    Host::play_sound(&mut adapter, "minecraft:entity.item_frame.add_item", found.world, BLOCKS, 1.0, 1.5);
    Host::play_sound(&mut adapter, "minecraft:block.piston.contract", found.world, BLOCKS, 0.75, 0.8);
    let Some(floors) = super::elevator::floors_of(&adapter, dimension, nbt) else { return true };
    let index = chosen(dimension, found.controller, who);
    match rules::floor_click(&floors.heights, index, floors.target, floors.min, floors.max) {
        FloorClick::Call(y) => super::elevator::call_floor(&mut adapter, dimension, floors.coords, y),
        FloorClick::Unreachable => {
            let (short, long) = &floors.names[index.min(floors.names.len() - 1)];
            Host::action_bar(&mut adapter, who, floor_line(short, long, true));
        }
        FloorClick::AlreadyTarget | FloorClick::NoFloors => {}
    }
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
