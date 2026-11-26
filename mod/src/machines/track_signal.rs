//! The Train Signal (`SignalBlock`, `SignalBlockEntity`, `SignalVisual`): a block that targets a spot on a track edge
//! through a `TrackTargetingBehaviour` and shows the state of the signal block ahead of it. The rules are
//! `create_kinetics::rules::trains::{edge_blocks, signal}`; the shared part (finding and making the edge point, the tick
//! of the points) is `train_net.rs`.
//!
//! The block entity makes its point in the graph, reports its redstone power to it (a powered signal is forced red),
//! follows the point's type (entry or cross), and takes the state the signal groups' occupancy give it: red, yellow,
//! green, or invalid with no point. The block entity's `State` is what the indicator light shows and what a comparator
//! reads (15 while red).
//!
//! Known differences from Create, for the README:
//! - The computer behaviour isn't there (`SignalStateChangeEvent`, `hasAttachedComputer`).
//! - The overlay on the track (`TRACK_SIGNAL_OVERLAY`, `TrackTargetingBehaviour.render`) is drawn by `train_overlay.rs` from this
//!   entity's data (`TargetTrack`, `TargetDirection`, `Bezier`, `Overlay`).

//! - `SignalBlock.neighborChanged` is polled from the block entity's tick through `signals.rs`; the four-tick release of
//!   the power (`scheduleTick(pos, this, 4)`) is a countdown in `PowerOff`.
//! - A signal placed against redstone starts unpowered until its first tick reads it (`getStateForPlacement`'s
//!   `hasNeighborSignal`).
//! - The red light is full bright in Create (`LightTexture.FULL_BLOCK`); here it is the indicator model's own lighting.

use create_kinetics::rules::trains::edge_blocks::{self as rules, SignalLights};
use create_kinetics::rules::trains::graph::EdgePointType;
use create_kinetics::rules::trains::signal::{OverlayState, SignalState, SignalType};
use create_kinetics::rules::trains::targeting::BlockEntityInfo;
use create_kinetics::Pos;
use crate::compat::{
    AnalogOutputEvent, BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent, Compound, Flow,
    HasAnalogOutputEvent, Hooks, Host, LevelHost, StandaloneModelsEvent, Tag, UseItemOnEvent,
};

use super::diodes_signal as signal;
use super::train_net as net;
use crate::signals;
use crate::{int, util};

pub const SIGNAL: &str = "create:track_signal";
/// `AllPartialModels.SIGNAL_ON` and `SIGNAL_OFF`.
pub const INDICATOR_ON: &str = "create:block/track_signal/indicator_on";
pub const INDICATOR_OFF: &str = "create:block/track_signal/indicator_off";

const STATE: &str = "State";
const OVERLAY: &str = "Overlay";
const POWER: &str = "Power";
/// Not Create's: the ticks a green light has kept red off, which Create keeps in memory too.
const SWITCH: &str = "SwitchToRed";
const POWER_OFF: &str = "PowerOff";

struct Signals;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Signals)
}

fn text(nbt: &Compound, key: &str) -> String {
    match util::get(nbt, key) {
        Some(Tag::String(s)) => s.to_string_lossy(),
        _ => String::new(),
    }
}

fn type_of(host: &dyn Host, pos: Pos) -> Option<SignalType> {
    host.block_property(pos, "type").as_deref().and_then(SignalType::from_property)
}

/// `SignalBlockEntity.tick`, with `SignalBlock.neighborChanged` before it.
fn tick(host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
    let pos = event.pos;
    let level = signal::host_level(&*host);
    let mut nbt = event.entity.nbt.clone();

    // The block's own redstone (`neighborChanged` and its scheduled tick).
    let neighbor = signals::read(host, &mut nbt, pos) > 0;
    let powered = host.block_property(pos, "powered").as_deref() == Some("true");
    let (now_powered, pending) = rules::signal_power_step(powered, neighbor, int(&nbt, POWER_OFF));
    if now_powered != powered {
        host.set_block_property(pos, "powered", if now_powered { "true" } else { "false" });
    }
    if pending != int(&nbt, POWER_OFF) {
        nbt.insert(POWER_OFF.into(), Tag::Int(pending));
    }

    // The edge point.
    let mut target = net::load(&nbt, EdgePointType::Signal, pos);
    let mut lights = SignalLights::from_saved(
        SignalState::from_nbt_name(&text(&nbt, STATE)),
        OverlayState::from_nbt_name(&text(&nbt, OVERLAY)),
        util::boolean(&nbt, POWER),
    );
    lights.switch_to_red_after_train_entered = int(&nbt, SWITCH);
    if !util::has(&nbt, STATE) {
        lights.state = SignalState::Invalid;
        lights.overlay = OverlayState::Skip;
    }
    let info = BlockEntityInfo {
        pos,
        dimension: super::track::dimension_id(host.dimension()),
        signal_type: type_of(&*host, pos),
        reported_power: lights.last_reported_power,
        assembling: false,
        filter: None,
    };
    let ensured = net::ensure_point(&*host, &mut target, &info);

    let boundary = net::with_point(&*host, EdgePointType::Signal, target.id, |p| p.signal().cloned()).flatten();
    let out = lights.tick(boundary.as_ref(), pos, Some(now_powered), type_of(&*host, pos));
    if let Some(power) = out.report_power {
        net::with_point(&*host, EdgePointType::Signal, target.id, |p| {
            if let Some(s) = p.signal_mut() {
                s.update_block_entity_power(pos, power);
            }
        });
    }
    if let Some(wanted) = out.set_type {
        host.set_block_property(pos, "type", wanted.property_value());
    }
    net::set_output(level, pos, lights.analog_output());

    net::store(&target, &mut nbt, pos);
    nbt.insert(STATE.into(), Tag::String(lights.state.nbt_name().into()));
    nbt.insert(OVERLAY.into(), Tag::String(lights.overlay.nbt_name().into()));
    nbt.insert(POWER.into(), Tag::Byte(i8::from(lights.last_reported_power)));
    nbt.insert(SWITCH.into(), Tag::Int(lights.switch_to_red_after_train_entered));
    let _ = ensured;
    if nbt != event.entity.nbt {
        event.entity.nbt = nbt;
    }
    if out.changed {
        host.block_entity_sync(pos);
    }
}

/// `SignalBlock.onWrenched`: the signal's type switches, and the player is told which.
fn wrench(host: &mut dyn Host, pos: Pos) {
    let Some(uuid) = host.acting_player().map(|p| p.uuid) else { return };
    let Some(entity) = host.mod_block_entity(pos) else { return };
    let target = net::load(&entity.nbt, EdgePointType::Signal, pos);
    let boundary = net::with_point(&*host, EdgePointType::Signal, target.id, |point| {
        if let Some(s) = point.signal_mut() {
            s.cycle_signal_type(pos);
        }
        point.signal().cloned()
    })
    .flatten();
    let key = rules::signal_wrench_message(boundary.as_ref(), pos);
    net::say(host, uuid, &key, &[]);
}

/// Whether a block state is a signal's (`type`: entry or cross).
fn is_signal(level: &dyn LevelHost, state: crate::compat::BlockStateId) -> bool {
    signal::choice(level, state, "type", &["entry_signal", "cross_signal"]).is_some()
}

impl Hooks for Signals {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.push(INDICATOR_ON.to_owned());
        event.models.push(INDICATOR_OFF.to_owned());
    }

    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        (event.block == SIGNAL).then(|| (SIGNAL.to_owned(), Compound::new()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == SIGNAL {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id == SIGNAL {
            // The placing item's selection is copied into the new block entity's components: Create's holds none.
            if super::track::leaked_components(&event.entity.nbt) {
                super::track::strip_components(&mut event.entity.nbt);
                host.block_entity_sync(event.pos);
            }
            tick(host, event);
        }
    }

    /// `SignalVisual`: the indicator is lit while the light is red (an invalid signal blinks).
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != SIGNAL {
            return;
        }
        let state = SignalState::from_nbt_name(&text(&event.entity.nbt, STATE));
        let state = if util::has(&event.entity.nbt, STATE) { state } else { SignalState::Invalid };
        let time = util::game_time(host) as f32;
        let model = if state.is_red_light(time) { INDICATOR_ON } else { INDICATOR_OFF };
        host.draw_model(&mut *event.draw, model, 0xFF_FFFF);
        // Only an invalid signal changes by itself (it blinks).
        if state != SignalState::Invalid {
            event.draw.keep();
        }
    }

    /// `SignalBlock.hasAnalogOutputSignal`.
    fn has_analog_output(&self, level: &dyn LevelHost, event: &mut HasAnalogOutputEvent) {
        if is_signal(level, event.state) {
            event.output = true;
        }
    }

    /// `SignalBlock.getAnalogOutputSignal`: 15 while the signal is red.
    fn analog_output(&self, level: &dyn LevelHost, event: &mut AnalogOutputEvent) {
        if is_signal(level, event.state) {
            let pos = [event.pos.x, event.pos.y, event.pos.z];
            event.output = net::output(signal::level_index(event.dimension), pos);
        }
    }

    /// The Wrench switches the signal's type (`IWrenchable.onWrenched`), sneaking picks it up.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        if &*event.stack.item != util::WRENCH || host.block(pos) != SIGNAL {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else { return Flow::Continue };
        if !util::first_click(&*host, player.uuid, pos) {
            return Flow::Cancel;
        }
        util::wrench(host, pos, |host| wrench(host, pos))
    }
}
