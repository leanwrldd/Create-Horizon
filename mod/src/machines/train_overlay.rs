//! What the trains' client draws over the track and what it tells the server about it: the marker an edge point shows on its
//! track (`TrackTargetingClient.render`: while its item is held, `TrackTargetingBehaviour.render`: once placed), the station's flag and
//! assembly spots (`StationRenderer`), the segment of a curve the cursor is on (`TrackBlockOutline.pickCurves`, `drawCurveSelection`)
//! with the click that selects it, and the relocation preview (`TrainRelocator.relocateClient`). The numbers are
//! `create_kinetics::rules::trains::track_overlay`; this reads the client's world and draws.
//!
//! - **Held item**: `client_tick` (from `lib.rs`'s `client_tick_post`) works out what `TrackTargetingClient.clientTick` does: the
//!   track under the cursor, or the selected one, or a curve segment, checked with `withGraphLocation`; `submit_custom_geometry`
//!   draws the overlay model as an instance at `1 + 1/16` scale.
//! - **Placed**: `block_entity_submit` of the Train Station, Signal and Observer draws the overlay at the track the block targets, in the
//!   block's own pose. The station also raises its flag and, while assembling, draws the blue and green spots along the track.
//! - **Curves**: the track blocks that hold curves are remembered as they are drawn (`TRACKS_WITH_TURNS`); the cursor's ray is tried against
//!   each curve's segments. A click on one, with an edge point item in hand or a Wrench while relocating, is cancelled on the client and
//!   sent as `create:curve_click`, as Create's `CurvedTrackSelectionPacket` and relocation packet.
//! - **Relocation preview**: the client says which track the cursor is on (`create:relocation_hover`); the server walks the train's
//!   wheels there and answers with the places (`create:relocation_preview`), drawn as a line (green, orange for the last stretch when
//!   it can't be).
//!
//! Known differences from Create, for the README:
//! - The graphs aren't sent to clients: a held item's overlay is checked against the graphs only where the client can read them (a
//!   single-player world, or the host of a LAN game); elsewhere it is drawn on any track. The relocation preview doesn't need them.
//! - The overlay as an instance is lit by the track block's light with a floor of 4 (an unreadable light would hide it).
//! - The station's assembly spots are drawn as four coloured boxes with the model's texture (a model can't be tinted all over); the
//!   flag's `LerpedFloat` runs on the frame's clock, not the client tick.
//! - A tilted track's overlay shift (`TrackBlockEntityTilt`) isn't there; the tilted track's model isn't tilted (a chunk model can't be).
//! - **Straight track's selection box** (`TrackBlockOutline.drawCustomBlockSelection`, `rules::trains::track_outline`) is drawn as world lines
//!   over the block under the cursor, white or red while holding track as Create colours it; Horizon can't cancel the vanilla outline, so
//!   both show.
//! - **Breaking a curve** (`CurvedTrackInteraction.clientTick`, `rules::trains::curve_interaction`): holding attack on the picked curve builds
//!   the progress; the crack stage goes to the server (`create:curve_break`, which sets it as `destroyBlockProgress`), and a full
//!   progress sends `create:curve_destroy`; a sneaking Wrench click sends the same with the pick-up flag (`track_curve.rs` answers both).
//!   Left out: the hit sound, the particle and the swing (no client call for them), creative speed (the client can't read the game mode:
//!   it uses the survival rate, the server still doesn't drop items for creative), and the cancelling of the vanilla attack on the block
//!   behind the curve (`onClickInput`; a click with a track item in hand on a curve is cancelled without Create's message).

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

use create_kinetics::Pos;
use create_kinetics::rules::trains::bezier::BezierConnection;
use create_kinetics::rules::trains::graph::EdgePointType;
use create_kinetics::rules::trains::graph_helper::with_graph_location;
use create_kinetics::rules::trains::targeting::{BezierTrackPointLocation, TargetingState};
use create_kinetics::rules::deployer;
use create_kinetics::rules::trains::curve_interaction::{self as breaking, BreakProgress, CurveBreakStage, CurveDestroy};
use create_kinetics::rules::trains::track_block::{TRACK, nearest_track_axis};
use create_kinetics::rules::trains::track_outline;
use create_kinetics::rules::trains::track_overlay::{
    self as rules, Chain, CurveClick, CurvePick, CurveUse, Flag, Hover, Op, OverlaySpot, OverlayType, Preview,
};
use create_kinetics::rules::trains::vec::{Vec3, add, block_containing, scale};
use crate::compat::{
    Adapter, BlockEntityDraw, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent, CubeFace, EquipmentSlot, Hooks, Host,
    ItemStack, NetFromServerEvent, StandaloneModelsEvent, WorldPrimitivesEvent,
};

use super::track as world;
use super::track_station;
use super::train_net as net;
use crate::util;

const STATION: &str = "create:track_station";
const SIGNAL: &str = "create:track_signal";
const OBSERVER: &str = "create:track_observer";
/// The station's key for a train standing at it (`StationBlockEntity.trainPresent`), written by `track_station.rs` for the flag.
pub const TRAIN_PRESENT: &str = "TrainPresent";
/// The signal's overlay state (`SignalBlockEntity.overlay`).
const OVERLAY: &str = "Overlay";
/// The assembly overlay's texture, imported with the model of the same name.
const ASSEMBLY_SPRITE: &str = "create:block/track_assembly_overlay";
/// The model that names it, so the texture is imported.
const ASSEMBLY_MODEL: &str = "create:block/track_overlay/assembling";

/// `Player.blockInteractionRange` (the extendo grip's is not read here).
const REACH: f64 = 4.5;
/// A held overlay is worked out again at least this often (the graph changes under it).
const RECHECK: i64 = 20;
/// The hover is sent again at least this often while relocating, to keep the preview current.
const HOVER_EVERY: i64 = 10;
/// A preview older than this many ticks is dropped.
const PREVIEW_KEEPS: i64 = 40;
/// A station's assembly is scanned again this often (`lazyTick` is 20).
const ASSEMBLY_EVERY: i64 = 20;

struct Draws;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Draws)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// --- what the client keeps --------------------------------------------------------------------------------------------------------

/// The overlay the held item shows: the track block and the pose of the model in its frame.
#[derive(Clone, Debug)]
struct Shown {
    block: Pos,
    ty: OverlayType,
    rows: [f32; 12],
    light: (u8, u8),
}

/// What decided `shown` (so it is worked out again only when this changes).
#[derive(Clone, Debug, PartialEq)]
struct Key {
    item: String,
    block: Pos,
    front: bool,
    bezier: Option<(Pos, i32)>,
}

/// A station's assembly as the client scanned it.
#[derive(Clone, Debug)]
struct Area {
    direction: [i32; 3],
    length: i32,
    bogeys: Vec<i32>,
}

#[derive(Default)]
struct Client {
    /// `TRACKS_WITH_TURNS`: the curves each track block entity that was drawn holds (primary ones), by dimension and position.
    tracks: HashMap<(&'static str, Pos), Vec<BezierConnection>>,
    pick: Option<CurvePick>,
    /// `CurvedTrackInteraction`'s `breakProgress` and the last crack stage sent.
    breaking: BreakProgress,
    sent_stage: Option<(Pos, i32)>,
    /// The straight track block under the cursor: its outline lines and colour (`drawCustomBlockSelection`).
    outline: Option<(Vec<[Vec3; 2]>, u32)>,
    /// Whether the main hand holds track (the curve's outline turns red).
    holding_track: bool,
    shown: Option<Shown>,
    key: Option<Key>,
    checked_at: i64,
    swept_at: i64,
    /// The server says this player is relocating a train.
    relocating: bool,
    /// The game time a curve click was last sent.
    curve_sent_at: Option<i64>,
    sent: Option<(Hover, i64)>,

    preview: Option<(Preview, i64)>,
    flags: HashMap<(&'static str, Pos), Flag>,
    areas: HashMap<(&'static str, Pos), (i64, Option<Area>)>,
}

static CLIENT: LazyLock<Mutex<Client>> = LazyLock::new(Mutex::default);

// --- reading the client -----------------------------------------------------------------------------------------------------------

struct View {
    eye: Vec3,
    look: Vec3,
}

fn view_of(host: &dyn Host) -> Option<View> {
    let state = host.client_state()?;
    Some(View { eye: state.eye_position?, look: world::look_of(state.yaw, state.pitch) })
}

fn held(host: &dyn Host) -> Option<ItemStack> {
    host.equipment(EquipmentSlot::MainHand)
}

fn distance_sqr(a: Vec3, b: Vec3) -> f64 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum()
}

/// The graphs, where this process has them (the integrated server's): `None` elsewhere, or while the server holds them.
fn with_nets<R>(dimension: &str, f: impl FnOnce(&create_kinetics::rules::trains::network::RailNetworks) -> R) -> Option<R> {
    world::try_with_networks(dimension, |nets| f(nets))
}

/// `TrackTargetingBlockItem.withGraphLocation`'s verdict for a spot, `true` where it can't be asked.
fn spot_is_valid(host: &dyn Host, key: &Key, ty: EdgePointType) -> bool {
    let query = net::query(host);
    let bezier = key.bezier.map(|(curve_target, segment)| BezierTrackPointLocation { curve_target, segment });
    with_nets(host.dimension(), |nets| with_graph_location(&query, nets, key.block, key.front, bezier, ty).0.feedback().is_none())
        .unwrap_or(true)
}

fn edge_type(item: &str) -> Option<EdgePointType> {
    match item {
        "create:track_station" => Some(EdgePointType::Station),
        "create:track_signal" => Some(EdgePointType::Signal),
        "create:track_observer" => Some(EdgePointType::Observer),
        _ => None,
    }
}

fn light_at(host: &dyn Host, pos: Pos) -> (u8, u8) {
    let (block, sky) = host.light(pos);
    (block.max(4), sky.max(4))
}

/// The pose of the overlay at a spot, in the track block's frame (`prepareTrackOverlay`).
fn spot_chain(host: &dyn Host, block: Pos, positive: bool, bezier: Option<(Pos, i32)>, size: f64) -> Option<Chain> {
    match bezier {
        Some((curve, segment)) => {
            let bc = world::connections_at(host, block).into_iter().find(|c| c.key() == curve)?;
            rules::overlay_chain(OverlaySpot::Curve { curve: &bc, segment }, block, positive, size)
        }
        None => {
            let shape = world::shape_at(host, block)?;
            let axis = *shape.axes().first()?;
            rules::overlay_chain(OverlaySpot::Straight { axis, up: shape.normal() }, block, positive, size)
        }
    }
}

// --- the client tick ---------------------------------------------------------------------------------------------------------------

/// `TrackTargetingClient.clientTick`, `TrackBlockOutline.pickCurves` and `TrainRelocator.clientTick`'s hover, once a client tick.
pub(crate) fn client_tick(host: &mut dyn mod_api::ClientHost) {
    let mut adapter = Adapter::ClientMut(host);
    let Some(view) = view_of(&adapter) else { return };
    let now = util::game_time(&adapter);
    let dimension = adapter.dimension();

    let attack = adapter.client_host().is_some_and(|h| h.is_key_down("key.mouse.left"));
    let stack = held(&adapter);
    let item = stack.as_ref().map(|s| s.item.to_string()).unwrap_or_default();
    let ty = edge_type(&item);
    let wrench = item == util::WRENCH;
    let relocating = lock(&CLIENT).relocating;
    sweep(&adapter, dimension, now, &view);

    // The aimed block and its distance (`mc.hitResult`).
    let hit = adapter.raycast(REACH as f32);
    let aimed_sqr = hit.as_ref().map_or(f64::MAX, |h| distance_sqr(view.eye, [h.x, h.y, h.z]));

    // `TrackBlockOutline.pickCurves`: every tick (the break, the outline and the clicks all read it).
    let pick = {
        let range = aimed_sqr.min(REACH) + 1.0;
        let target = add(view.eye, scale(view.look, range));
        let client = lock(&CLIENT);
        let curves: Vec<(Pos, &BezierConnection)> = client
            .tracks
            .iter()
            .filter(|((d, _), _)| *d == dimension)
            .flat_map(|((_, pos), list)| list.iter().map(move |bc| (*pos, bc)))
            .collect();
        rules::pick_curves(&curves, view.eye, target, aimed_sqr)
    };
    lock(&CLIENT).pick = pick;
    let holding_track = item == TRACK;
    straight_outline(&adapter, hit.as_ref().map(|h| ([h.block_x, h.block_y, h.block_z], h.block.as_str())), pick.is_some(), holding_track);
    break_tick(&mut adapter, attack, pick, &item);

    let hover = hovered(&adapter, &view, hit.as_ref().map(|h| ([h.block_x, h.block_y, h.block_z], h.block.as_str())), stack.as_ref(), pick);
    held_overlay(&adapter, ty, &item, hover.clone(), now);

    if relocating && wrench {
        send_hover(&mut adapter, &view, hover, now);
    } else {
        let mut client = lock(&CLIENT);
        client.sent = None;
        client.preview = None;
    }
    let mut client = lock(&CLIENT);
    if client.preview.as_ref().is_some_and(|(_, at)| now - *at > PREVIEW_KEEPS) {
        client.preview = None;
    }
}

/// `TrackBlockOutline.drawCustomBlockSelection`: the lines of the straight track block under the cursor (not while a curve is picked,
/// which takes the cursor's hit away in Create).
fn straight_outline(host: &dyn Host, hit: Option<(Pos, &str)>, curve_picked: bool, holding_track: bool) {
    let outline = hit.filter(|(_, block)| *block == TRACK && !curve_picked).and_then(|(pos, _)| {
        let shape = world::shape_at(host, pos)?;
        let tilted = world::is_tilted(host, pos);
        let colour = track_outline::colour(holding_track, track_outline::can_connect_from(shape, tilted));
        Some((track_outline::selection_lines(shape, pos), colour))
    });
    let mut client = lock(&CLIENT);
    client.outline = outline;
    client.holding_track = holding_track;
}

/// `CurvedTrackInteraction.clientTick`: the break progress of the picked curve; what it asks of the world is sent to the server.
fn break_tick(host: &mut Adapter<'_>, attack: bool, pick: Option<CurvePick>, item: &str) {
    let destroy_progress = if attack && pick.is_some() {
        let in_tag = |tag: &str| Host::block_name_in_tag(&*host, TRACK, tag).unwrap_or(false);
        let held = (!item.is_empty()).then_some(item);
        let speed = deployer::mining_speed(held, TRACK, in_tag);
        // `SharedProperties.stone` with `strength(0.8)`.
        deployer::destroy_progress(speed, 0.8, deployer::has_correct_tool(held, TRACK, in_tag))
    } else {
        0.0
    };
    let input = breaking::Input {
        may_build: true,
        attack_down: attack,
        picked: pick.map(|p| p.block),
        block_is_air: pick.is_some_and(|p| host.block(p.block) != TRACK),
        creative: false,
        destroy_progress,
    };
    let effects = lock(&CLIENT).breaking.tick(&input);
    for effect in effects {
        match effect {
            breaking::Effect::Crack(block, stage) => {
                let changed = {
                    let mut client = lock(&CLIENT);
                    let new = Some((block, stage));
                    let changed = client.sent_stage != new && !(stage < 0 && client.sent_stage.is_none());
                    client.sent_stage = if stage < 0 { None } else { new };
                    changed
                };
                if changed {
                    let _ = Host::net_send_to_server(host, breaking::CHANNEL_STAGE, &CurveBreakStage { block, stage }.encode());
                }
            }
            breaking::Effect::Destroy => {
                if let Some(pick) = pick {
                    send_destroy(host, &pick, false);
                }
            }
            // Horizon has no client call for the sound, the particle or the swing.
            breaking::Effect::HitSound | breaking::Effect::Particle | breaking::Effect::Swing => {}
        }
    }
}

/// `new CurvedTrackDestroyPacket(pos, curveTarget, BlockPos.containing(vec), wrench)`.
fn send_destroy(host: &mut Adapter<'_>, pick: &CurvePick, wrench: bool) {
    let message = CurveDestroy { block: pick.block, curve: pick.curve, sound_source: block_containing(pick.anchor), wrench };
    let _ = Host::net_send_to_server(host, breaking::CHANNEL_DESTROY, &message.encode());
}

/// What the cursor is on, as `TrackTargetingClient` and `relocateClient` read it: the block, the side, and a curve segment.
#[derive(Clone, Debug, PartialEq)]
struct Hovered {
    block: Pos,
    front: bool,
    bezier: Option<(Pos, i32)>,
}

fn hovered(host: &dyn Host, view: &View, hit: Option<(Pos, &str)>, stack: Option<&ItemStack>, pick: Option<CurvePick>) -> Option<Hovered> {
    // An item that has picked its spot already shows that.
    if let Some(selected) = stack.and_then(super::track_target::selection_of_stack) {
        return Some(Hovered { block: selected.pos, front: selected.front, bezier: selected.bezier.map(|b| (b.curve_target, b.segment)) });
    }
    if let Some(pick) = pick {
        return Some(Hovered { block: pick.block, front: rules::curve_front(view.look, pick.direction), bezier: Some((pick.curve, pick.segment)) });
    }
    let (pos, block) = hit?;
    if block != TRACK {
        return None;
    }
    let axes = world::shape_at(host, pos)?.axes();
    let (_, front) = nearest_track_axis(&axes, view.look);
    Some(Hovered { block: pos, front, bezier: None })
}

/// `TrackTargetingClient.clientTick`'s second half: check the hovered spot and keep the overlay to draw.
fn held_overlay(host: &dyn Host, ty: Option<EdgePointType>, item: &str, hover: Option<Hovered>, now: i64) {
    let (Some(ty), Some(hover)) = (ty, hover) else {
        let mut client = lock(&CLIENT);
        client.shown = None;
        client.key = None;
        return;
    };
    let key = Key { item: item.to_owned(), block: hover.block, front: hover.front, bezier: hover.bezier };
    {
        let client = lock(&CLIENT);
        if client.key.as_ref() == Some(&key) && now - client.checked_at < RECHECK {
            return;
        }
    }
    let shown = if spot_is_valid(host, &key, ty) {
        OverlayType::of_item(item)
            .and_then(|overlay| {
                let chain = spot_chain(host, hover.block, hover.front, hover.bezier, rules::HELD_SCALE)?;
                Some(Shown { block: hover.block, ty: overlay, rows: chain.affine().rows(), light: light_at(host, hover.block) })
            })
    } else {
        None
    };
    let mut client = lock(&CLIENT);
    client.shown = shown;
    client.key = Some(key);
    client.checked_at = now;
}

/// Forgets track blocks that are gone or left behind (every 100 ticks).
fn sweep(host: &dyn Host, dimension: &'static str, now: i64, view: &View) {
    let mut client = lock(&CLIENT);
    if now - client.swept_at < 100 {
        return;
    }
    client.swept_at = now;
    client.tracks.retain(|(d, pos), _| {
        *d == dimension && distance_sqr(view.eye, [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])]) < 256.0 * 256.0 && host.block(*pos) == TRACK
    });
}

/// While relocating: tell the server which track the cursor is on (when it changes, and now and then).
fn send_hover(host: &mut Adapter<'_>, view: &View, hover: Option<Hovered>, now: i64) {
    let Some(hover) = hover else {
        lock(&CLIENT).sent = None;
        return;
    };
    let message = Hover {
        pos: hover.block,
        look: view.look.map(|v| v as f32),
        front: hover.front,
        bezier: hover.bezier,
    };
    {
        let client = lock(&CLIENT);
        if let Some((last, at)) = &client.sent
            && *last == message
            && now - *at < HOVER_EVERY
        {
            return;
        }
    }
    if Host::net_send_to_server(host, rules::CHANNEL_HOVER, &message.encode()).is_ok() {
        lock(&CLIENT).sent = Some((message, now));
    }
}

// --- clicks on a curve (the new API's hooks call these) ----------------------------------------------------------------------------------

/// What a click on the picked curve is for with this item in the main hand, as `CurvedTrackInteraction.onClickInput` and
/// `TrainRelocator.onClicked` decide it.
fn curve_use(item: &str, relocating: bool, sneaking: bool) -> Option<Take> {
    if item == TRACK {
        // `onClickInput`: track in hand on a curve says "turn_start" and does nothing.
        Some(Take::Refuse)
    } else if edge_type(item).is_some() {
        Some(Take::Send(CurveUse::Select))
    } else if relocating && item == util::WRENCH {
        Some(Take::Send(CurveUse::Relocate))
    } else if item == util::WRENCH && sneaking {
        Some(Take::PickUp)
    } else {
        None
    }
}

/// What a taken click does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Take {
    Send(CurveUse),
    /// The Wrench picks the curve up (`CurvedTrackDestroyPacket` with `wrench`).
    PickUp,
    Refuse,
}

/// A right click, or one aimed at nothing, while a curve segment is under the cursor: it is the curve's. Cancelled on the client and sent
/// to the server (once for the click, whichever hooks and hands hear it). True when taken.
fn curve_clicked(host: &mut dyn mod_api::ClientHost, main_hand: bool) -> bool {
    let mut adapter = Adapter::ClientMut(host);
    let Some(view) = view_of(&adapter) else { return false };
    let now = util::game_time(&adapter);
    let (pick, relocating, sent_at) = {
        let client = lock(&CLIENT);
        (client.pick, client.relocating, client.curve_sent_at)
    };
    let Some(pick) = pick else { return false };
    let item = held(&adapter).map(|s| s.item.to_string()).unwrap_or_default();
    let Some(purpose) = curve_use(&item, relocating, Host::sneaking(&adapter)) else { return false };
    // The block hook, the item hook and the off hand's turn of one click: the first sends, the rest only take it.
    if sent_at == Some(now) || !main_hand {
        return sent_at == Some(now);
    }
    match purpose {
        Take::Send(purpose) => {
            let click = CurveClick { purpose, block: pick.block, curve: pick.curve, segment: pick.segment, front: rules::curve_front(view.look, pick.direction) };
            let _ = Host::net_send_to_server(&mut adapter, rules::CHANNEL_CURVE, &click.encode());
        }
        Take::PickUp => {
            send_destroy(&mut adapter, &pick, true);
            lock(&CLIENT).breaking = BreakProgress::default();
        }
        Take::Refuse => {}
    }
    lock(&CLIENT).curve_sent_at = Some(now);
    true
}

fn take_curve_click(host: &mut mod_api::HostSide<'_>, hand: mod_api::Hand, result: &mut mod_api::InteractionResult) -> mod_api::Flow {
    let mod_api::HostSide::Client(host) = host else { return mod_api::Flow::Continue };
    if curve_clicked(&mut **host, hand == mod_api::Hand::Main) {
        *result = mod_api::InteractionResult::Consume;
        mod_api::Flow::Cancel
    } else {
        mod_api::Flow::Continue
    }
}

/// `right_click_block` on the client: the curve under the cursor takes the click (Create clears `hitResult` so the block behind doesn't).
pub(crate) fn right_click_block(host: &mut mod_api::HostSide<'_>, event: &mut mod_api::RightClickBlockEvent<'_>) -> mod_api::Flow {
    take_curve_click(host, event.click.hand, event.cancellation_result)
}

/// `right_click_item` on the client: the same, for a click that aims at the air.
pub(crate) fn right_click_item(host: &mut mod_api::HostSide<'_>, event: &mut mod_api::RightClickItemEvent<'_>) -> mod_api::Flow {
    take_curve_click(host, event.click.hand, event.cancellation_result)
}

// --- drawing ----------------------------------------------------------------------------------------------------------------------------

/// `submit_custom_geometry`: the held item's overlay.
pub(crate) fn submit_custom_geometry(_: &dyn mod_api::ClientHost, event: &mut mod_api::SubmitCustomGeometryEvent<'_>) {
    let Some(shown) = lock(&CLIENT).shown.clone() else { return };
    let Some(model) = crate::compat::adapter::model(shown.ty.model()) else { return };
    let origin = shown.block.map(f64::from);
    let light = mod_api::LightCoords::new(shown.light.0, shown.light.1).unwrap_or(mod_api::LightCoords::FULL_BRIGHT);
    let instance = mod_api::ModelInstance::new(origin, shown.rows, 0xFFFF_FFFF, light, mod_api::Overlay::NONE);
    let _ = event.draw.draw_model_instances(model, &[instance]);
}

/// The lines: the picked curve's segment, and the relocation preview.
fn lines(event: &mut WorldPrimitivesEvent<'_>) {
    let client = lock(&CLIENT);
    if let Some(pick) = client.pick {
        let colour = if client.holding_track { rules::OUTLINE_TRACK_HELD } else { rules::OUTLINE_PLAIN };
        for [a, b] in rules::curve_outline(&pick) {
            event.primitives.line(a, b, colour, 2.0);
        }
    }
    if let Some((lines, colour)) = &client.outline {
        for [a, b] in lines {
            event.primitives.line(*a, *b, *colour, 2.0);
        }
    }
    if let Some((preview, _)) = &client.preview {
        for line in rules::preview_lines(&preview.points, preview.valid, preview.upside_down) {
            event.primitives.line(line.from, line.to, 0xFF00_0000 | line.colour, line.width * 16.0);
        }
    }
}

fn replay(draw: &mut BlockEntityDraw, chain: &Chain) {
    for op in &chain.0 {
        match *op {
            Op::Translate(v) => draw.translate(v.map(|x| x as f32)),
            Op::Scale(v) => draw.scale(v.map(|x| x as f32)),
            Op::Rotate(axis, degrees) => draw.rotate_about(axis, degrees, [0.0; 3]),
        }
    }
}

fn offset_of(from: Pos, to: Pos) -> Vec3 {
    [f64::from(to[0] - from[0]), f64::from(to[1] - from[1]), f64::from(to[2] - from[2])]
}

/// `TrackTargetingBehaviour.render` for a placed edge point, in the pose of its block entity.
fn draw_marker(host: &dyn Host, draw: &mut BlockEntityDraw, pos: Pos, target: &TargetingState, ty: OverlayType) {
    let track = target.global_position(pos);
    if host.block(track) != TRACK {
        return;
    }
    let bezier = target.target_bezier.map(|b| (b.curve_target, b.segment));
    let Some(chain) = spot_chain(host, track, target.target_positive, bezier, 1.0) else { return };
    draw.push();
    draw.translate(offset_of(pos, track).map(|v| v as f32));
    replay(draw, &chain);
    host.draw_model(draw, ty.model(), 0xFF_FFFF);
    draw.pop();
}

fn text(nbt: &crate::compat::Compound, key: &str) -> String {
    match util::get(nbt, key) {
        Some(crate::compat::Tag::String(s)) => s.to_string_lossy(),
        _ => String::new(),
    }
}

fn draw_station(host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
    let pos = event.pos;
    let nbt = &event.entity.nbt;
    let target = net::load(nbt, EdgePointType::Station, pos);
    let track = target.global_position(pos);
    let Some(shape) = world::shape_at(host, track) else { return };
    let dimension = host.dimension();
    let assembling = host.block_property(pos, "assembling").as_deref() == Some("true");
    let train_present = util::boolean(nbt, TRAIN_PRESENT);
    let time = event.draw.game_time as f64 + f64::from(event.draw.partial);

    // The flag: `StationBlockEntity.tick`'s client half, on the frame's clock.
    let flag = {
        let mut client = lock(&CLIENT);
        let entry = client.flags.entry((dimension, pos)).or_insert_with(Flag::lowered);
        *entry = entry.follow(rules::flag_wanted(train_present, assembling), time);
        *entry
    };
    let value = flag.value_at(time);
    let settled = flag.settled_at(time);
    let step = if target.target_positive { 1.0 } else { -1.0 };
    let axis = shape.axes().last().map(|a| scale(*a, step)).unwrap_or([0.0, 0.0, 1.0]);
    let (y_rot, flipped) = rules::flag_angle(axis, [track[0] - pos[0], track[1] - pos[1], track[2] - pos[2]]);
    let model = rules::flag_model(assembling, train_present, value);
    let progress = rules::flag_progress(value, flag.target, settled);
    let flag_chain = rules::flag_chain(y_rot, flipped, progress);
    let draw = &mut *event.draw;
    draw.push();
    replay(draw, &flag_chain);
    host.draw_model(draw, model.model(), 0xFF_FFFF);
    draw.pop();

    if !assembling || train_present {
        draw_marker(host, draw, pos, &target, OverlayType::Station);
    } else if let Some(area) = area_of(host, pos, &target, time as i64) {
        draw_assembly(host, draw, offset_of(pos, track), &area);
    }
    // The flag moves until it settles (and wiggles while it does); then the picture only changes with the data.
    if settled && !assembling {
        draw.keep();
    }
}

/// The assembly area as the client scans it (`refreshAssemblyInfo` in `tick` on the client), every second.
fn area_of(host: &dyn Host, pos: Pos, target: &TargetingState, now: i64) -> Option<Area> {
    let dimension = host.dimension();
    {
        let client = lock(&CLIENT);
        if let Some((at, area)) = client.areas.get(&(dimension, pos))
            && now - *at < ASSEMBLY_EVERY
            && now >= *at
        {
            return area.clone();
        }
    }
    let area = track_station::refresh(host, target, pos)
        .map(|a| Area { direction: a.direction, length: a.scan.length, bogeys: a.scan.bogey_locations.clone() });
    lock(&CLIENT).areas.insert((dimension, pos), (now, area.clone()));
    area
}

/// `StationRenderer`'s loop: a coloured spot for each place a bogey may go and where one is.
fn draw_assembly(host: &dyn Host, draw: &mut BlockEntityDraw, to_track: Vec3, area: &Area) {
    let spots = rules::assembly_spots(area.length, &area.bogeys, |i| create_kinetics::rules::trains::station::is_valid_bogey_offset(i, &area.bogeys));
    for spot in spots {
        draw.push();
        draw.translate(to_track.map(|v| v as f32));
        replay(draw, &rules::assembly_chain(area.direction, spot.step));
        for b in &rules::ASSEMBLY_BOXES {
            let face = |uv: [f32; 4]| Some(CubeFace::new(ASSEMBLY_SPRITE).with_uv(uv));
            let faces = [face(b.down), face(b.up), face(b.north), face(b.south), None, None];
            host.draw_cube(draw, b.from.map(|v| v as f32), b.to.map(|v| v as f32), faces, spot.colour);
        }
        draw.pop();
    }
}

impl Hooks for Draws {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        for ty in OverlayType::ALL {
            event.models.push(ty.model().to_owned());
        }
        for flag in [rules::FlagModel::On, rules::FlagModel::Off, rules::FlagModel::Assemble] {
            event.models.push(flag.model().to_owned());
        }
        // Named so that its texture is imported: the spots are drawn as boxes.
        event.models.push(ASSEMBLY_MODEL.to_owned());
        event.models.push(create_kinetics::rules::trains::train_hat::MODEL.to_owned());
    }

    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        match event.block {
            TRACK => {
                // `TRACKS_WITH_TURNS`: remember the curves this block holds, to pick among them.
                let curves: Vec<BezierConnection> = world::connections_in(&event.entity.nbt, event.pos).into_iter().filter(|c| c.is_primary()).collect();
                let mut client = lock(&CLIENT);
                if curves.is_empty() {
                    client.tracks.remove(&(host.dimension(), event.pos));
                } else {
                    client.tracks.insert((host.dimension(), event.pos), curves);
                }
            }
            STATION => draw_station(host, event),
            SIGNAL => {
                let target = net::load(&event.entity.nbt, EdgePointType::Signal, event.pos);
                let ty = match text(&event.entity.nbt, OVERLAY).as_str() {
                    "SKIP" => return,
                    "DUAL" => OverlayType::DualSignal,
                    _ => OverlayType::Signal,
                };
                draw_marker(host, &mut *event.draw, event.pos, &target, ty);
            }
            OBSERVER => {
                let target = net::load(&event.entity.nbt, EdgePointType::Observer, event.pos);
                draw_marker(host, &mut *event.draw, event.pos, &target, OverlayType::Observer);
            }
            _ => {}
        }
    }

    /// The marker is drawn away from the block: its box reaches the target track (and the curve a point is on).
    fn block_entity_render_bounding_box(&self, host: &dyn Host, event: &mut BlockEntityRenderBoundingBoxEvent<'_>) {
        let ty = match event.block {
            STATION => EdgePointType::Station,
            SIGNAL => EdgePointType::Signal,
            OBSERVER => EdgePointType::Observer,
            _ => return,
        };
        let target = net::load(&event.entity.nbt, ty, event.pos);
        let track = target.global_position(event.pos);
        let at = offset_of(event.pos, track);
        let mut lo = [at[0] - 1.0, at[1] - 1.0, at[2] - 1.0];
        let mut hi = [at[0] + 2.0, at[1] + 2.0, at[2] + 2.0];
        if let Some(b) = target.target_bezier
            && let Some(bc) = world::connections_at(host, track).into_iter().find(|c| c.key() == b.curve_target)
        {
            let [a, z] = bc.bounds();
            let origin = event.pos.map(f64::from);
            for i in 0..3 {
                lo[i] = lo[i].min(a[i] - origin[i]);
                hi[i] = hi[i].max(z[i] - origin[i]);
            }
        }
        // A station may assemble along a long track.
        if event.block == STATION {
            for i in 0..3 {
                lo[i] -= 24.0;
                hi[i] += 24.0;
            }
        }
        let b = &mut *event.bounds;
        for i in 0..3 {
            b[i] = b[i].min(lo[i] as f32);
            b[i + 3] = b[i + 3].max(hi[i] as f32);
        }
    }

    /// The lines of the curve under the cursor and of the relocation preview.
    fn world_primitives(&self, _: &dyn Host, event: &mut WorldPrimitivesEvent<'_>) {
        lines(event);
    }

    /// The server's session news and the preview of a hovered spot.
    fn net_from_server(&self, host: &mut dyn Host, event: &mut NetFromServerEvent<'_>) {
        match event.channel {
            rules::CHANNEL_SESSION => {
                let mut client = lock(&CLIENT);
                client.relocating = matches!(event.data, [1, ..]);
                if !client.relocating {
                    client.preview = None;
                    client.sent = None;
                }
            }
            rules::CHANNEL_PREVIEW => {
                if let Some(preview) = Preview::decode(event.data) {
                    let now = util::game_time(&*host);
                    lock(&CLIENT).preview = Some((preview, now));
                }
            }
            _ => {}
        }
    }
}
