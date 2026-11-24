//! Putting a derailed or stopped train back on the track with the Wrench (`TrainRelocator`, `TrainRelocationPacket`). The decisions
//! are `create_kinetics::rules::trains::relocator`; this finds the train, the clicked track's place on the graph and tells the
//! player.
//!
//! `/train relocate` with a Wrench in hand starts it on the nearest train (Create: a Wrench click on one of the carriage's blocks,
//! `ContraptionHandlerClient.handleSpecialInteractions`). The next click with the Wrench on a piece of track puts the train there
//! (the clicked end follows the way you look, as `getNearestTrackAxis` does); a sneak-click, or putting the Wrench away, ends it.
//!
//! Known differences from Create, for the README:
//! - The session is the server's, not the client's: no preview line along the track and no "Can relocate to here" message, the
//!   click tries at once and says "Cannot relocate Train to here" when it can't.
//! - Only a straight track can be clicked (a curve's point is a client ray, `TrackBlockOutline`).
//! - A click on something that isn't a block (the air) doesn't end the session; `/train relocate` again does.
//! - The "derailed train" hint of the goggles' tooltip (`addToTooltip`) isn't shown, and the carriages' non-damage ticks
//!   (`nonDamageTicks`) don't exist: nothing is hurt by being moved.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use create_kinetics::Pos;
use create_kinetics::rules::trains::graph::Uuid;
use create_kinetics::rules::trains::graph_helper::{TrackQuery, graph_location_at};
use create_kinetics::rules::trains::railways::Railways;
use create_kinetics::rules::trains::relocator as rules;
use create_kinetics::rules::trains::track_block::nearest_track_axis;
use crate::compat::TextExt;
use crate::compat::{Flow, Hooks, Host, LevelTickPostEvent, NativeServerPlayer, PerformCommandEvent, RightClickBlockEvent, Text};

use super::track as world;
use super::train_net as net;
use super::trains;
use crate::util;

/// `ChatFormatting.RED` and `GREEN`.
const RED: u32 = 0xFF5555;
const GREEN: u32 = 0x55FF55;
/// Ticks between two reminders of the session on the action bar.
const REMIND_EVERY: i64 = 10;
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

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn say(host: &mut dyn Host, uuid: u128, text: Text) {
    host.action_bar(uuid, text);
}

fn red(host: &mut dyn Host, uuid: u128, line: &str) {
    say(host, uuid, Text::colored(line.to_owned(), RED));
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

/// `/train relocate`: starts a session for the nearest train, or ends the one running.
fn start(host: &mut dyn Host, uuid: u128) {
    if lock(&SESSIONS).remove(&uuid).is_some() {
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
    say(host, uuid, Text::literal(rules::message_start(&name)));
    lock(&SESSIONS).insert(uuid, Session { dimension: dimension.to_owned(), train, name, origin });
}

/// `TrainRelocator.relocate(..)` for the track block at `pos`, with the way the player looks: whether the train was put there.
fn relocate_at(host: &mut dyn Host, player: &NativeServerPlayer, session: &Session, pos: Pos) -> bool {
    let dimension = host.dimension();
    let Some(shape) = world::shape_at(&*host, pos) else { return false };
    let query = net::query(&*host);
    let axes = query.axes(pos);
    let look = world::look_of(player.yaw, player.pitch);
    // `getNearestTrackAxis`: the axis whose line is nearest to the look, and the way along it.
    let (axis, front) = nearest_track_axis(&axes, look);
    let up = shape.normal();
    let train_id = session.train;
    let done = world::with_railways(dimension, |rails| {
        let Railways { nets, trains, .. } = rails;
        let location = graph_location_at(&query, nets, pos, if front { 1 } else { -1 }, axis)?;
        let mut train = trains.remove(&train_id)?;
        let max_length = f64::from(train.config.max_assembly_length);
        let ok = rules::relocate(&mut train, nets, trains, &location, up, query.dimension(), max_length, false);
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

/// `TrainRelocator.onClicked` and `TrainRelocationPacket.handle`, for a click with the Wrench on a track block.
fn clicked(host: &mut dyn Host, player: &NativeServerPlayer, session: Session, pos: Pos) {
    let uuid = player.uuid;
    let eye = eye_of(player);
    let reach = rules::can_interact_with_block(eye, session.origin, 24.0);
    if rules::click(player.sneaking, reach) == rules::Click::Abort {
        lock(&SESSIONS).remove(&uuid);
        red(host, uuid, rules::MSG_ABORT);
        return;
    }
    // The packet's check of the clicked track (`maxTrackPlacementLength * 2`).
    if !rules::can_interact_with_block(eye, pos, rules::CLICK_BUFFER) {
        red(host, uuid, rules::MSG_TOO_FAR);
        return;
    }
    if relocate_at(host, player, &session, pos) {
        lock(&SESSIONS).remove(&uuid);
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
                lock(&SESSIONS).remove(&uuid);
                red(host, uuid, rules::MSG_MOVING);
            }
            rules::Check::Abort => {
                lock(&SESSIONS).remove(&uuid);
                red(host, uuid, rules::MSG_ABORT);
            }
            rules::Check::TooFar => red(host, uuid, rules::MSG_TOO_FAR),
            rules::Check::Keep => {
                if now % REMIND_EVERY == 0 {
                    say(host, uuid, Text::literal(rules::message_start(&session.name)));
                }
            }
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
        clicked(host, &player, session, event.pos);
        Flow::Cancel
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
