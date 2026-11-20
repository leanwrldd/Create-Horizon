//! The Rose Quartz Lamp in the game (`RoseQuartzLampBlock`): it lights on a redstone signal and
//! keeps giving power afterwards, puts out the lamps joined to it when it lights, gives a comparator
//! a signal that falls with the distance to the lit one, and is toggled by the Wrench. The rules are
//! `create_kinetics::rules::rose_quartz_lamp`; the signal helpers are in `diodes_signal.rs`.
//!
//! Known differences from Create, for the README:
//! - Create reacts to `neighborChanged` and schedules a tick; each lamp has a block entity here that
//!   reads `hasNeighborSignal` once a tick, and runs the tick (`ACTIVATE`) on the next.
//! - The lamps of the cluster a lighting lamp puts out are told to their neighbours at once, not a
//!   tick later, but after the lit lamp's state is set (a comparator beside the row reads it).
//! - `getStateForPlacement` sets `POWERED` from the signal beside the lamp: done when it is placed,
//!   not in the placement state.
//! - A lamp gives its signal as a weak one only (`getDirectSignal` is not overridden in Create).

use create_kinetics::rules::rose_quartz_lamp::{
    Lamp, Reaction, Step, cluster, distance_to_powered, lighting, lit, react, scheduled_tick,
    signal as lamp_signal,
};
use create_kinetics::Pos;
use crate::compat::{
    BlockEntityEvent, BlockEntityTickEvent, BlockEntityTicksEvent, BlockSignalEvent, BlockStateId,
    Compound, Flow, Hooks, Host, IsSignalSourceEvent, LevelHost, PlaceBlockEvent, UseItemOnEvent,
};

use super::diodes_signal as signal;
use crate::util;

pub const ID: &str = "create:rose_quartz_lamp";

/// `Level.UPDATE_CLIENTS`.
const UPDATE_CLIENTS: i32 = 2;

struct RoseQuartzLamp;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(RoseQuartzLamp)
}

fn flag(host: &dyn Host, pos: Pos, key: &str) -> bool {
    host.block_property(pos, key).as_deref() == Some("true")
}

fn set(host: &mut dyn Host, pos: Pos, key: &str, value: bool) {
    host.set_block_property_with(pos, key, if value { "true" } else { "false" }, UPDATE_CLIENTS);
}

fn read(host: &dyn Host, pos: Pos) -> Lamp {
    Lamp {
        powered: flag(host, pos, "powered"),
        powering: flag(host, pos, "powering"),
        activate: flag(host, pos, "activate"),
    }
}

/// `RoseQuartzLampBlock.forEachInCluster`.
fn lamps_around(host: &dyn Host, origin: Pos) -> Vec<Pos> {
    cluster(origin, |pos| host.block(pos) == ID)
}

fn tick(host: &mut dyn Host, pos: Pos, nbt: &mut Compound) {
    let dimension = host.dimension();
    let mut lamp = read(host, pos);
    // `RoseQuartzLampBlock.tick`, the tick `scheduleActivation` asked for.
    if lamp.activate
        && let Some(next) = scheduled_tick(lamp)
    {
        set(host, pos, "activate", next.activate);
        set(host, pos, "powering", next.powering);
        lamp = next;
        host.update_signal_neighbors(dimension, pos);
    }
    // `RoseQuartzLampBlock.neighborChanged`: the signal the neighbour event left (`signals`).
    let signal_now = crate::signals::read(host, nbt, pos) > 0;
    match react(lamp, signal_now) {
        Reaction::None => {}
        Reaction::Unpower => set(host, pos, "powered", false),
        Reaction::Light => {
            // `lighting`: every state is set before any lamp is told, so a comparator beside
            // the row reads the lit lamp (Create tells the cluster from their scheduled ticks).
            let next = lit();
            set(host, pos, "powered", next.powered);
            set(host, pos, "activate", next.activate);
            for step in lighting(pos, &lamps_around(host, pos)) {
                match step {
                    Step::Powering(at, on) => set(host, at, "powering", on),
                    Step::Notify(at) => {
                        host.update_signal_neighbors(dimension, at);
                    }
                }
            }
        }
    }
}

// --- the block as a level hook sees it ----------------------------------------------------------

/// Whether the state is a Rose Quartz Lamp's, and if so whether it is powering.
fn lamp_state(level: &dyn LevelHost, state: BlockStateId) -> Option<bool> {
    use std::collections::HashMap;
    use std::sync::{LazyLock, Mutex};
    static KNOWN: LazyLock<Mutex<HashMap<u32, Option<bool>>>> = LazyLock::new(Default::default);
    if let Ok(cache) = KNOWN.lock()
        && let Some(known) = cache.get(&state.0)
    {
        return *known;
    }
    let found = (|| {
        signal::boolean(level, state, "activate")?;
        signal::boolean(level, state, "powered")?;
        signal::boolean(level, state, "powering")
    })();
    if let Ok(mut cache) = KNOWN.lock() {
        cache.insert(state.0, found);
    }
    found
}

impl Hooks for RoseQuartzLamp {
    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == ID).then(|| (ID.to_owned(), Compound::new()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id == ID {
            tick(host, event.pos, &mut event.entity.nbt);
        }
    }

    /// `RoseQuartzLampBlock.getStateForPlacement`: `POWERED` is whether a signal reaches it.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if &*event.stack.item == ID && host.neighbor_signal(host.dimension(), event.pos) > 0 {
            set(host, event.pos, "powered", true);
        }
    }

    /// `RoseQuartzLampBlock.getSignal`: 15 while powering, to everything but another lamp; a
    /// comparator reads the distance to the lit lamp along the line it looks down.
    fn block_signal(&self, level: &dyn LevelHost, event: &mut BlockSignalEvent) {
        if let Some(powering) = lamp_state(level, event.state) {
            event.signal = if event.direct {
                0
            } else {
                let pos = [event.pos.x, event.pos.y, event.pos.z];
                let normal = event.face.normal();
                let reader = [pos[0] + normal[0], pos[1] + normal[1], pos[2] + normal[2]];
                let reader_state = level.block_state(reader);
                let reader_is_lamp = lamp_state(level, reader_state).is_some();
                let reader_is_comparator = signal::takes(level, reader_state, "mode", "compare");
                // `column` is the direction from the reader to the lamp, as Create passes it.
                let column = event.face.opposite().normal();
                let distance = if reader_is_comparator {
                    distance_to_powered(|step| {
                        let at = [
                            pos[0] + column[0] * step,
                            pos[1] + column[1] * step,
                            pos[2] + column[2] * step,
                        ];
                        lamp_state(level, level.block_state(at))
                    })
                } else {
                    0
                };
                lamp_signal(powering, reader_is_lamp, reader_is_comparator, distance)
            };
        }
        signal::mask(event);
    }

    /// `RoseQuartzLampBlock.isSignalSource`.
    fn is_signal_source(&self, level: &dyn LevelHost, event: &mut IsSignalSourceEvent) {
        if lamp_state(level, event.state).is_some() {
            event.source = true;
        }
    }

    /// The Wrench (`RoseQuartzLampBlock.getRotatedBlockState`, `onWrenched`): flips `POWERING` and
    /// tells the lamps around. Sneaking picks the block up.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != ID {
            return Flow::Continue;
        }
        let pos = event.pos;
        util::wrench(host, pos, |host| {
            let dimension = host.dimension();
            let powering = flag(host, pos, "powering");
            host.set_block_property(pos, "powering", if powering { "false" } else { "true" });
            for other in lamps_around(host, pos) {
                host.update_signal_neighbors(dimension, other);
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machines::diodes_signal::fake::Registry;

    const BOOL: &[&str] = &["false", "true"];

    #[test]
    fn a_state_is_read_as_a_lamp() {
        let mut level = Registry::default();
        level.block(&[("activate", BOOL), ("powered", BOOL), ("powering", BOOL)]);
        level.block(&[
            ("facing", &["north", "south", "west", "east"]),
            ("powered", BOOL),
            ("powering", BOOL),
        ]);
        level.block(&[("mode", &["compare", "subtract"]), ("powered", BOOL)]);
        let lit = level.find(&[("activate", "false"), ("powered", "true"), ("powering", "true")]);
        let dim = level.find(&[("activate", "true"), ("powered", "true"), ("powering", "false")]);
        assert_eq!(lamp_state(&level, lit), Some(true));
        assert_eq!(lamp_state(&level, dim), Some(false));
        let diode = level.find(&[("facing", "north"), ("powered", "true"), ("powering", "true")]);
        assert_eq!(lamp_state(&level, diode), None);
        // A comparator is the block with a `mode` that takes `compare`.
        let comparator = level.find(&[("mode", "compare"), ("powered", "false")]);
        assert!(signal::takes(&level, comparator, "mode", "compare"));
        assert!(!signal::takes(&level, lit, "mode", "compare"));
    }
}
