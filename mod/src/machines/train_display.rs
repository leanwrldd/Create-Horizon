//! What the trains tell the Display Link (`GlobalTrainDisplayData`, `StationBlockEntity.tick`'s call of
//! `DisplayLinkBlock.notifyGatherers`, and what `StationSummaryDisplaySource`, `TrainStatusDisplaySource` and
//! `ObservedTrainNameSource` read from the world). The decisions are `create_kinetics::rules::trains::display`; the sources
//! themselves are registered in `rules::display_link` and read through `display_link_source.rs`.
//!
//! Every 100 ticks (`GlobalRailwayManager.tick`: `gameTime % 100 == 0`) each dimension's table of predictions is made again from
//! its trains. Every tick a station with a train standing at it whose schedule asked for a refresh (`displayLinkUpdateRequested`)
//! has the Display Links attached to it written again and clears the request.
//!
//! Differences from Create, for the README:
//! - The table is kept for each dimension (Create has one for the whole manager); a summary lists the trains of its own dimension.
//! - A "Players Seated" status counts the players on the seats of the train's carriages (`trains::count_passengers`). The
//!   departure board's predictions (`submitPredictions`) don't read passengers, as in Create.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use create_kinetics::Pos;
use create_kinetics::rules::trains::display::{self as rules, GlobalTrainDisplayData};
use create_kinetics::rules::trains::graph::{EdgePointType, Uuid};
use crate::compat::{Hooks, Host, LevelTickPostEvent};

use super::display_link;
use super::track as world;
use super::train_net as net;
use crate::util;

/// The station's saved name (`track_station.rs`).
const STATION_NAME: &str = "StationName";

struct Display;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Display)
}

/// Each dimension's `GlobalTrainDisplayData`.
static DATA: LazyLock<Mutex<HashMap<String, GlobalTrainDisplayData>>> = LazyLock::new(Mutex::default);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The predictions of the dimension as they were at the last refresh.
pub fn departures(dimension: &str) -> GlobalTrainDisplayData {
    lock(&DATA).get(dimension).cloned().unwrap_or_default()
}

/// `getStation()` of the station block entity at `pos`: its edge point's id.
fn station_id(host: &dyn Host, pos: Pos) -> Option<Uuid> {
    let entity = host.mod_block_entity(pos)?;
    Some(net::load(&entity.nbt, EdgePointType::Station, pos).id)
}

/// The name of the station block entity at `pos` (`GlobalStation.name`), which `StationSummaryDisplaySource.populateData` takes
/// as the filter.
pub fn station_name(host: &dyn Host, pos: Pos) -> Option<String> {
    let entity = host.mod_block_entity(pos)?;
    match util::get(&entity.nbt, STATION_NAME) {
        Some(crate::compat::Tag::String(name)) => Some(name.to_string_lossy()),
        _ => None,
    }
}

/// `TrainStatusDisplaySource.provideLine`: what the train at the station block `pos` waits for; `None` is `EMPTY_LINE`.
pub fn train_status(host: &dyn Host, pos: Pos) -> Option<String> {
    let station = station_id(host, pos)?;
    let day_time = host.day_time();
    let dimension = host.dimension();
    let present = world::with_railways(dimension, |rails| {
        let graph = rails.nets.graphs.values().find(|g| g.point(EdgePointType::Station, station).is_some())?;
        graph.point(EdgePointType::Station, station)?.station()?.present_train(station, &rails.trains)
    })
    .flatten()?;
    // `train.countPlayerPassengers()`: the players on the seats of its carriages ("Players Seated").
    let passengers = super::trains::count_passengers(host, dimension, present);
    world::with_railways(dimension, |rails| rules::train_status_line(rails.trains.get(&present), day_time, passengers)).flatten()
}

/// `ObservedTrainNameSource.provideLine`: the name of the train the observer at `pos` sees; `None` is `EMPTY_LINE`.
pub fn observed_train_name(host: &dyn Host, pos: Pos) -> Option<String> {
    let entity = host.mod_block_entity(pos)?;
    let id = net::load(&entity.nbt, EdgePointType::Observer, pos).id;
    let current = net::with_point(host, EdgePointType::Observer, id, |point| point.observer().and_then(|o| o.current_train())).flatten()?;
    world::with_railways(host.dimension(), |rails| rules::observed_train_name(rails.trains.get(&current))).flatten()
}

/// The blocks of the stations whose train asked for its Display Links to be written (`displayLinkUpdateRequested`), the request
/// cleared.
fn requested_stations(dimension: &str) -> Vec<Pos> {
    world::with_railways(dimension, |rails| {
        let stations: Vec<(Uuid, Option<Pos>)> = rails
            .nets
            .graphs
            .values()
            .flat_map(|g| g.edge_points.values(EdgePointType::Station).into_iter().map(|p| (p.id, p.station().and_then(|s| s.block_entity_pos))))
            .collect();
        let mut asked = Vec::new();
        for (id, pos) in stations {
            let state = rails.station_train_state(id, &|_| None);
            if !state.display_link_update_requested {
                continue;
            }
            if let Some(train) = state.imminent_train.and_then(|t| rails.trains.get_mut(&t)) {
                train.runtime.display_link_update_requested = false;
            }
            asked.extend(pos);
        }
        asked
    })
    .unwrap_or_default()
}

fn tick(host: &mut dyn Host) {
    let dimension = host.dimension();
    let now = util::game_time(&*host);
    if now % rules::REFRESH_EVERY == 0 {
        let mut data = lock(&DATA).remove(dimension).unwrap_or_default();
        if world::with_railways(dimension, |rails| data.refresh(rails)).is_none() {
            data.clear();
        }
        lock(&DATA).insert(dimension.to_owned(), data);
    }
    for pos in requested_stations(dimension) {
        display_link::notify_gatherers(host, pos);
    }
}

impl Hooks for Display {
    fn level_tick_post(&self, host: &mut dyn Host, _: &mut LevelTickPostEvent<'_>) {
        if host.window() {
            return;
        }
        tick(host);
    }
}
