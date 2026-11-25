//! Putting a derailed or stopped train back on the track with the Wrench (`TrainRelocator`, `TrainRelocationPacket`). The decisions
//! are `create_kinetics::rules::trains::relocator`; this finds the train, the clicked track's place on the graph and tells the
//! player.
//!
//! A Wrench click on a carriage that no block of it takes starts a session (`ContraptionHandlerClient.handleSpecialInteractions`,
//! `TrainRelocator.carriageWrenched`: `structure_click.rs` calls [`carriage_wrenched`]); `/train relocate` with a Wrench in hand starts
//! it on the nearest train. The next click with the Wrench on a piece of track, or on a segment of a curve, puts the train there (the
//! clicked end follows the way you look, as `getNearestTrackAxis` does); a sneak-click, or putting the Wrench away, ends it.
//!
//! The client tells the server where the cursor is (`create:relocation_hover`, from `train_overlay.rs`), and the server answers with the
//! places the wheels would take and whether the train fits (`create:relocation_preview`): the client draws the line, the server says
//! "Can relocate to here, Click to Confirm" or "Cannot relocate Train to here" on the action bar, as `relocateClient(simulate)` does.
//!
//! Known differences from Create, for the README:
//! - The session is the server's, not the client's: it tells the client when it begins and ends (`create:relocation`). The preview
//!   is the server's walk of the client's hover, not the client's own `relocate(.., simulate)`.
//! - A click on something that isn't a block (the air) doesn't end the session; `/train relocate` again does.
//! - The click's origin is where the carriage was clicked (its hit point), not `car.toGlobalVector(center of the local block)`.
//! - The "derailed train" hint of the goggles' tooltip (`addToTooltip`) isn't shown, and the carriages' non-damage ticks
//!   (`nonDamageTicks`) don't exist: nothing is hurt by being moved.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};

use create_kinetics::Pos;
use create_kinetics::rules::trains::graph::Uuid;
use create_kinetics::rules::trains::graph_helper::{TrackQuery, bezier_graph_location_at, graph_location_at};
use create_kinetics::rules::trains::network::RailNetworks;
use create_kinetics::rules::trains::railways::Railways;
use create_kinetics::rules::trains::relocator as rules;
use create_kinetics::rules::trains::targeting::BezierTrackPointLocation;
use create_kinetics::rules::trains::track_block::nearest_track_axis;
use create_kinetics::rules::trains::track_overlay::{self as overlay, CurveClick, CurveUse, Hover, Preview};
use create_kinetics::rules::trains::travelling_point::TrackGraphLocation;
use create_kinetics::rules::trains::vec::Vec3;
use crate::compat::TextExt;
use crate::compat::{
    Flow, Hooks, Host, LevelTickPostEvent, NativeServerPlayer, NetFromClientEvent, PerformCommandEvent, PlayerTickEvent, RightClickBlockEvent,
    Text,
};

use super::track as world;
use super::train_net as net;
use super::trains;
use crate::util;

/// `ChatFormatting.RED` and `GREEN`.
const RED: u32 = 0xFF5555;
const GREEN: u32 = 0x55FF55;
/// Ticks between two reminders of the session on the action bar.
const REMIND_EVERY: i64 = 10;
/// A hover answered this recently keeps the reminder off the action bar (the line it left is what the player reads).
const HOVER_QUIET: i64 = 15;
/// Nearest train a `/train relocate` takes, in blocks from one of its carriages.
const RANGE: f64 = 24.0;

struct Relocate;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Relocate)
}

/// `relocatingTrain` and `relocatingOrigin`: what a player is relocating, and where they started.
#[derive(Clone, Debug)]
struct Session {
    dimension: String,
    train: Uuid,
    name: String,
    origin: Pos,
}

static SESSIONS: LazyLock<Mutex<HashMap<u128, Session>>> = LazyLock::new(Mutex::default);

/// What the clients said, to answer in the sender's own turn.
#[derive(Clone, Copy, Debug)]
enum Said {
    Hover(Hover),
    Curve(CurveClick),
}

static SAID: LazyLock<Mutex<Vec<(u128, Said)>>> = LazyLock::new(Mutex::default);
static SAID_COUNT: AtomicUsize = AtomicUsize::new(0);
/// When each player's hover was last answered (game time).
static ANSWERED: LazyLock<Mutex<HashMap<u128, i64>>> = LazyLock::new(Mutex::default);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn say(host: &mut dyn Host, uuid: u128, text: Text) {
    host.action_bar(uuid, text);
}

fn red(host: &mut dyn Host, uuid: u128, line: &str) {
    say(host, uuid, Text::colored(line.to_owned(), RED));
}

/// The client is told a session begins (`1`) or ends (`0`), so it starts and stops asking for previews.
fn tell_client(host: &mut dyn Host, uuid: u128, on: bool) {
    let _ = host.net_send_to_player(uuid, overlay::CHANNEL_SESSION, &[u8::from(on)]);
}

/// A session is over, for whatever reason.
fn end(host: &mut dyn Host, uuid: u128) {
    lock(&SESSIONS).remove(&uuid);
    lock(&ANSWERED).remove(&uuid);
    tell_client(host, uuid, false);
}

fn eye_of(player: &NativeServerPlayer) -> [f64; 3] {
    [player.position[0], player.position[1] + if player.sneaking { 1.27 } else { 1.62 }, player.position[2]]
}

fn holds_wrench(host: &dyn Host, uuid: u128) -> bool {
    host.held_stack(uuid).is_some_and(|stack| &*stack.item == util::WRENCH)
}

fn player_of(host: &mut dyn Host, uuid: u128) -> Option<NativeServerPlayer> {
    host.server_players().into_iter().find(|p| p.uuid == uuid)
}

/// The train with a carriage nearest to `at` within `range`: its id, name and speed.
fn nearest_train(dimension: &str, at: [f64; 3], range: f64) -> Option<(Uuid, String, f64)> {
    let did = world::dimension_id(dimension);
    world::with_railways(dimension, |rails| {
        rails
            .trains
            .values()
            .filter_map(|train| {
                let distance = train
                    .carriages
                    .iter()
                    .filter_map(|c| c.dimensions.get(&did)?.position_anchor)
                    .map(|p| (0..3).map(|i| (p[i] - at[i]).powi(2)).sum::<f64>().sqrt())
                    .min_by(f64::total_cmp)?;
                (distance <= range).then(|| (distance, train.id, train.name.clone(), train.speed))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, id, name, speed)| (id, name, speed))
    })
    .flatten()
}

fn speed_of(dimension: &str, train: Uuid) -> Option<f64> {
    world::with_railways(dimension, |rails| rails.trains.get(&train).map(|t| t.speed)).flatten()
}

fn name_of(dimension: &str, train: Uuid) -> Option<String> {
    world::with_railways(dimension, |rails| rails.trains.get(&train).map(|t| t.name.clone())).flatten()
}

/// Starts a session for `train` (`carriageWrenched`): `origin` is where the player clicked.
fn begin(host: &mut dyn Host, uuid: u128, train: Uuid, name: String, origin: Pos) {
    say(host, uuid, Text::literal(rules::message_start(&name)));
    let dimension = host.dimension().to_owned();
    lock(&SESSIONS).insert(uuid, Session { dimension, train, name, origin });
    tell_client(host, uuid, true);
}

/// `TrainRelocator.carriageWrenched`: a Wrench click on a carriage that nothing on it took. `at` is the clicked point.
pub fn carriage_wrenched(host: &mut dyn Host, uuid: u128, at: [f64; 3], train: Uuid) -> bool {
    let dimension = host.dimension();
    let Some(name) = name_of(dimension, train) else { return false };
    if !holds_wrench(&*host, uuid) {
        return false;
    }
    begin(host, uuid, train, name, [at[0].floor() as i32, at[1].floor() as i32, at[2].floor() as i32]);
    true
}

/// `/train relocate`: starts a session for the nearest train, or ends the one running.
fn start(host: &mut dyn Host, uuid: u128) {
    if lock(&SESSIONS).contains_key(&uuid) {
        end(host, uuid);
        red(host, uuid, rules::MSG_ABORT);
        return;
    }
    let dimension = host.dimension();
    let Some(player) = player_of(host, uuid) else { return };
    if !holds_wrench(&*host, uuid) {
        red(host, uuid, "Hold a Wrench");
        return;
    }
    let Some((train, name, speed)) = nearest_train(dimension, player.position, RANGE) else {
        red(host, uuid, "No train within 24 blocks");
        return;
    };
    if rules::is_moving(speed) {
        red(host, uuid, rules::MSG_MOVING);
        return;
    }
    let origin = [player.position[0].floor() as i32, player.position[1].floor() as i32, player.position[2].floor() as i32];
    begin(host, uuid, train, name, origin);
}

/// A spot on the track: a straight block, or a segment of a curve (with the side the player looks from).
#[derive(Clone, Copy, Debug)]
enum Spot {
    Block(Pos),
    Curve { block: Pos, curve: Pos, segment: i32, front: bool },
}

impl Spot {
    fn block(self) -> Pos {
        match self {
            Spot::Block(pos) | Spot::Curve { block: pos, .. } => pos,
        }
    }
}

/// `TrackGraphHelper.getGraphLocationAt` / `getBezierGraphLocationAt` for a spot, with the track's up (`getUpNormal`).
fn locate(host: &dyn Host, nets: &RailNetworks, spot: Spot, look: Vec3) -> Option<(TrackGraphLocation, Vec3)> {
    let query = net::query(host);
    let pos = spot.block();
    let shape = world::shape_at(host, pos)?;
    let location = match spot {
        Spot::Block(pos) => {
            let axes = query.axes(pos);
            // `getNearestTrackAxis`: the axis whose line is nearest to the look, and the way along it.
            let (axis, front) = nearest_track_axis(&axes, look);
            graph_location_at(&query, nets, pos, if front { 1 } else { -1 }, axis)?
        }
        Spot::Curve { block, curve, segment, front } => {
            bezier_graph_location_at(&query, nets, block, front, BezierTrackPointLocation { curve_target: curve, segment })?
        }
    };
    Some((location, shape.normal()))
}

/// `TrainRelocator.relocate(..)` for a spot, the way the player looks: whether the train was put there.
fn relocate_at(host: &dyn Host, player: &NativeServerPlayer, session: &Session, spot: Spot) -> bool {
    let dimension = host.dimension();
    let look = world::look_of(player.yaw, player.pitch);
    let train_id = session.train;
    let did = world::dimension_id(dimension);
    let done = world::with_railways(dimension, |rails| {
        let Railways { nets, trains, .. } = rails;
        let (location, up) = locate(host, nets, spot, look)?;
        let mut train = trains.remove(&train_id)?;
        let max_length = f64::from(train.config.max_assembly_length);
        let ok = rules::relocate(&mut train, nets, trains, &location, up, did, max_length, false);
        trains.insert(train_id, train);
        Some(ok)
    })
    .flatten()
    .unwrap_or(false);
    if done {
        trains::with_dim(dimension, |d| d.dirty());
    }
    done
}

/// `TrainRelocator.relocateClient(simulate)`: what the train would do at a spot the client hovers.
fn preview_at(host: &dyn Host, session: &Session, spot: Spot, look: Vec3) -> Option<Preview> {
    let dimension = host.dimension();
    let did = world::dimension_id(dimension);
    let train_id = session.train;
    world::with_railways(dimension, |rails| {
        let Railways { nets, trains, .. } = rails;
        let (location, up) = locate(host, nets, spot, look)?;
        let train = trains.remove(&train_id)?;
        let max_length = f64::from(train.config.max_assembly_length);
        let upside_down = train.carriages.first().is_some_and(|c| c.leading_bogey().is_upside_down());
        let (valid, points) = rules::preview(&train, nets, trains, &location, up, did, max_length);
        trains.insert(train_id, train);
        Some(Preview { valid, upside_down, points })
    })
    .flatten()
}

/// `TrainRelocator.onClicked` and `TrainRelocationPacket.handle`, for a click with the Wrench on a track block or a curve.
fn clicked(host: &mut dyn Host, player: &NativeServerPlayer, session: Session, spot: Spot) {
    let uuid = player.uuid;
    let eye = eye_of(player);
    let reach = rules::can_interact_with_block(eye, session.origin, 24.0);
    if rules::click(player.sneaking, reach) == rules::Click::Abort {
        end(host, uuid);
        red(host, uuid, rules::MSG_ABORT);
        return;
    }
    // The packet's check of the clicked track (`maxTrackPlacementLength * 2`).
    if !rules::can_interact_with_block(eye, spot.block(), rules::CLICK_BUFFER) {
        red(host, uuid, rules::MSG_TOO_FAR);
        return;
    }
    if relocate_at(&*host, player, &session, spot) {
        end(host, uuid);
        say(host, uuid, Text::colored(rules::MSG_SUCCESS.to_owned(), GREEN));
    } else {
        red(host, uuid, rules::MSG_INVALID);
    }
}

/// `TrainRelocator.clientTick` for each running session.
fn tick(host: &mut dyn Host) {
    let dimension = host.dimension();
    let now = util::game_time(&*host);
    let sessions: Vec<(u128, Session)> = lock(&SESSIONS).iter().filter(|(_, s)| s.dimension == dimension).map(|(k, s)| (*k, s.clone())).collect();
    for (uuid, session) in sessions {
        let Some(player) = player_of(host, uuid) else { continue };
        let moving = speed_of(dimension, session.train).is_none_or(rules::is_moving);
        let reach = rules::can_interact_with_block(eye_of(&player), session.origin, 24.0);
        match rules::check_session(moving, holds_wrench(&*host, uuid), reach) {
            rules::Check::Moving => {
                end(host, uuid);
                red(host, uuid, rules::MSG_MOVING);
            }
            rules::Check::Abort => {
                end(host, uuid);
                red(host, uuid, rules::MSG_ABORT);
            }
            rules::Check::TooFar => red(host, uuid, rules::MSG_TOO_FAR),
            rules::Check::Keep => {
                let quiet = lock(&ANSWERED).get(&uuid).is_some_and(|at| now - *at < HOVER_QUIET);
                if now % REMIND_EVERY == 0 && !quiet {
                    say(host, uuid, Text::literal(rules::message_start(&session.name)));
                }
            }
        }
    }
}

/// Answers what a player's client said: a hover gets a preview, a click on a curve relocates.
fn answer(host: &mut dyn Host, player: &NativeServerPlayer, said: Said) {
    let uuid = player.uuid;
    let Some(session) = lock(&SESSIONS).get(&uuid).cloned() else { return };
    if session.dimension != host.dimension() || player.game_mode == util::SPECTATOR || !holds_wrench(&*host, uuid) {
        return;
    }
    match said {
        Said::Hover(hover) => {
            let spot = match hover.bezier {
                Some((curve, segment)) => Spot::Curve { block: hover.pos, curve, segment, front: hover.front },
                None => Spot::Block(hover.pos),
            };
            if !rules::can_interact_with_block(eye_of(player), hover.pos, rules::CLICK_BUFFER) {
                return;
            }
            let Some(preview) = preview_at(&*host, &session, spot, hover.look.map(f64::from)) else { return };
            let _ = host.net_send_to_player(uuid, overlay::CHANNEL_PREVIEW, &preview.encode());
            lock(&ANSWERED).insert(uuid, util::game_time(&*host));
            if preview.valid {
                say(host, uuid, Text::colored(rules::MSG_VALID.to_owned(), GREEN));
            } else {
                red(host, uuid, rules::MSG_INVALID);
            }
        }
        Said::Curve(click) => {
            if !util::first_click(&*host, uuid, click.block) {
                return;
            }
            clicked(host, player, session, Spot::Curve { block: click.block, curve: click.curve, segment: click.segment, front: click.front });
        }
    }
}

impl Hooks for Relocate {
    fn level_tick_post(&self, host: &mut dyn Host, _: &mut LevelTickPostEvent<'_>) {
        if host.window() || lock(&SESSIONS).is_empty() {
            return;
        }
        tick(host);
    }

    /// The Wrench on a piece of track while a train waits to be put there. It runs before the Wrench's own handling.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        if lock(&SESSIONS).is_empty() {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else { return Flow::Continue };
        let Some(session) = lock(&SESSIONS).get(&player.uuid).cloned() else { return Flow::Continue };
        if session.dimension != host.dimension() || player.game_mode == util::SPECTATOR || !holds_wrench(&*host, player.uuid) {
            return Flow::Continue;
        }
        if world::shape_at(&*host, event.pos).is_none() {
            return Flow::Continue;
        }
        if !util::first_click(&*host, player.uuid, event.pos) {
            return Flow::Cancel;
        }
        clicked(host, &player, session, Spot::Block(event.pos));
        Flow::Cancel
    }

    /// The hovers and curve clicks of the clients (`create:relocation_hover`, `create:curve_click`), kept for the sender's turn.
    fn net_from_client(&self, _: &mut dyn Host, event: &mut NetFromClientEvent<'_>) {
        let said = match event.channel {
            overlay::CHANNEL_HOVER => Hover::decode(event.data).map(Said::Hover),
            overlay::CHANNEL_CURVE => CurveClick::decode(event.data).filter(|c| c.purpose == CurveUse::Relocate).map(Said::Curve),
            _ => None,
        };
        if let Some(said) = said {
            let mut queue = lock(&SAID);
            // One hover is enough: the newest replaces the one before.
            if matches!(said, Said::Hover(_)) {
                queue.retain(|(who, s)| !(*who == event.player && matches!(s, Said::Hover(_))));
            }
            if queue.len() < 64 {
                queue.push((event.player, said));
            }
            SAID_COUNT.store(queue.len(), Ordering::Relaxed);
        }
    }

    fn player_tick(&self, host: &mut dyn Host, _: &mut PlayerTickEvent) {
        if SAID_COUNT.load(Ordering::Relaxed) == 0 {
            return;
        }
        let Some(player) = host.acting_player() else { return };
        let mine: Vec<Said> = {
            let mut queue = lock(&SAID);
            let (mine, rest): (Vec<_>, Vec<_>) = queue.drain(..).partition(|(who, _)| *who == player.uuid);
            *queue = rest;
            SAID_COUNT.store(queue.len(), Ordering::Relaxed);
            mine.into_iter().map(|(_, said)| said).collect()
        };
        for said in mine {
            answer(host, &player, said);
        }
    }

    /// `/train relocate`, ahead of `/train`'s own.
    fn perform_command(&self, host: &mut dyn Host, event: &mut PerformCommandEvent<'_>) -> Flow {
        if event.command.trim() != "train relocate" {
            return Flow::Continue;
        }
        start(host, event.player.uuid);
        Flow::Cancel
    }
}
