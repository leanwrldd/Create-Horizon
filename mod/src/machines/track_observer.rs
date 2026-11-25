//! The Train Observer (`TrackObserverBlock`, `TrackObserverBlockEntity`): a block that targets a spot on a track edge and
//! gives redstone power while a train passes it. The rules are `create_kinetics::rules::trains::{edge_blocks, observer}`;
//! the shared part (making the edge point, the tick of the points, which counts `activated` down) is `train_net.rs`.
//!
//! The block entity makes its point, hands it the cargo filter (a value box on top, `value_box.rs`: the rows in
//! `rules::value_box_blocks`) and powers the block (`powered`) while the point is activated, 15 on every side.
//!
//! Known differences from Create, for the README:
//! - The computer behaviour isn't there (`TrainPassEvent`). The display source (`OBSERVED_TRAIN_NAME`) is in
//!   `rules::display_link`, read through `train_display.rs`, and a change of power writes the links attached
//!   (`DisplayLinkBlock.notifyGatherers`).
//! - The filter is matched as an item id (a Filter item as its own id): the matching of trains' cargo is part 3's
//!   (`TrainWorld::item_matches`).
//! - The overlay on the track (`TRACK_OBSERVER_OVERLAY`) is drawn by `train_overlay.rs` (see `track_signal.rs`).


use create_kinetics::rules::trains::edge_blocks as rules;
use create_kinetics::rules::trains::graph::EdgePointType;
use create_kinetics::rules::trains::targeting::BlockEntityInfo;
use crate::compat::{
    BlockEntityEvent, BlockEntityTickEvent, BlockEntityTicksEvent, BlockSignalEvent, Compound, Hooks, Host, IsSignalSourceEvent,
    LevelHost,
};

use super::chute_inv;
use super::diodes_signal as signal;
use super::display_link;
use super::train_net as net;

pub const OBSERVER: &str = "create:track_observer";

struct Observers;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Observers)
}

/// `TrackObserverBlockEntity.tick`.
fn tick(host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
    let pos = event.pos;
    let level = signal::host_level(&*host);
    let mut nbt = event.entity.nbt.clone();
    let mut target = net::load(&nbt, EdgePointType::Observer, pos);
    let filter = chute_inv::filter_of(&nbt).item.map(|stack| stack.item.to_string());

    let info = BlockEntityInfo {
        pos,
        dimension: super::track::dimension_id(host.dimension()),
        signal_type: None,
        reported_power: false,
        assembling: false,
        filter: filter.clone(),
    };
    net::ensure_point(&*host, &mut target, &info);

    // `onFilterChanged`: the point follows the block entity's filter.
    let observer = net::with_point(&*host, EdgePointType::Observer, target.id, |point| {
        let Some(observer) = point.observer_mut() else { return None };
        if observer.filter != filter {
            observer.set_filter(filter.clone());
        }
        Some(observer.clone())
    })
    .flatten();

    let powered = host.block_property(pos, "powered").as_deref() == Some("true");
    if let Some(now) = rules::observer_power_step(powered, observer.as_ref()) {
        host.set_block_property(pos, "powered", if now { "true" } else { "false" });
        let dimension = host.dimension();
        host.update_signal_neighbors(dimension, pos);
        display_link::notify_gatherers(host, pos);
    }
    let powered = host.block_property(pos, "powered").as_deref() == Some("true");
    net::set_source(level, pos, rules::observer_signal(powered));

    net::store(&target, &mut nbt, pos);
    if nbt != event.entity.nbt {
        event.entity.nbt = nbt;
    }
}

impl Hooks for Observers {
    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        (event.block == OBSERVER).then(|| (OBSERVER.to_owned(), Compound::new()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == OBSERVER {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id == OBSERVER {
            tick(host, event);
        }
    }

    /// `TrackObserverBlock.getSignal`: 15 on every side while powered (`getDirectSignal` is the default, 0). The block is
    /// told apart by its position, which the block entity's tick has put in the sources.
    fn block_signal(&self, _: &dyn LevelHost, event: &mut BlockSignalEvent) {
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        let level = signal::level_index(event.dimension);
        if !event.direct {
            event.signal = event.signal.max(net::source(level, pos));
        }
    }

    /// `TrackObserverBlock.isSignalSource`.
    fn is_signal_source(&self, level: &dyn LevelHost, event: &mut IsSignalSourceEvent) {
        // A state with only `powered` (a pressure plate has it too, and is a source already).
        if signal::boolean(level, event.state, "powered") == Some(true) && !signal::takes(level, event.state, "facing", "north") {
            event.source = true;
        }
    }
}
