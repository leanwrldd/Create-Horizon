//! The Pulse Repeater, Pulse Extender, Pulse Timer, Powered Latch and Powered Toggle Latch in the
//! game (`BrassDiodeBlock`, `BrassDiodeBlockEntity`, `PulseRepeaterBlockEntity`,
//! `PulseExtenderBlockEntity`, `PulseTimerBlockEntity`, `PoweredLatchBlock`, `ToggleLatchBlock`,
//! `AbstractDiodeBlock` and the vanilla `DiodeBlock` they build on). The rules are
//! `create_kinetics::rules::diodes`; the signal helpers are in `diodes_signal.rs`.
//!
//! Known differences from Create, for the README:
//! - Create schedules block ticks. Each of these blocks has a block entity here (the latches have
//!   none in Create) that polls its input every tick and counts the delay (2 ticks for the brass
//!   diodes, 1 for the latches) itself; the saved `Pending` is that count. A change is seen up to a
//!   tick later than a neighbour update would announce it, and `TickPriority` is not used.
//! - There is no scroll-value box: a Wrench clicked on a side of a Pulse Repeater, Extender or Timer
//!   steps its delay through 2, 3, 4, 5, 10, 20 ticks, 2, 3, 5, 10 seconds, 30 seconds, 1, 5, 10, 30
//!   and 60 minutes (the range of Create's box is 2 ticks to an hour: `diodes::STEPS`), coming round
//!   to 2 after an hour. The action bar shows the interval. A Wrench on the top still turns the
//!   block (`IWrenchable.getRotatedBlockState`); sneaking with it picks the block up.
//! - A redstone wire beside a diode connects to every side of it (no `canConnectRedstone`
//!   hook): the signal a diode reads is still only the one from its back, and a Powered Latch's
//!   sides, as in Create.
//! - A diode is torn off when the block under it is air, water, a plant or another thing that can
//!   be replaced (`DiodeBlock.canSurvive` asks whether the block can carry a rigid block: a slab's
//!   or a stair's top is not told apart).
//! - The indicator's colour (`BrassDiodeRenderer`, `ColoredOverlayBlockEntityRenderer`) is drawn
//!   as a tinted box with the model's own texture, and updated at 1/32 steps of progress.
//! - Right-clicking with the hand full still toggles (`BrassDiodeBlock.useItemOn` does too).

use create_kinetics::rules::analog_lever::mix_colours;
use create_kinetics::rules::diodes::{
    self, BRASS_DELAY, Brass, LATCH_DELAY, Latch, Toggle, brass_output, brass_step, default_value,
    diode_tick, latch_click, latch_output, latch_sides, latch_tick, latch_wants_tick, toggle_tick,
    wants_tick,
};
use create_kinetics::{Axis, Dir, Pos, looking};
use horizon_client::mod_api::{
    AnalogOutputEvent, BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent,
    BlockEntityTicksEvent, BlockEntityUpdateTagEvent, BlockSignalEvent, BlockStateId, Compound,
    CubeFace, Flow, HasAnalogOutputEvent, Hooks, Host, IsSignalSourceEvent, LevelHost,
    ModBlockEntity, PlaceBlockEvent, RightClickBlockEvent, StateForPlacementEvent, Tag,
    UseItemOnEvent,
};

use super::diodes_signal as signal;
use crate::{int, util};

pub const PULSE_REPEATER: &str = "create:pulse_repeater";
pub const PULSE_EXTENDER: &str = "create:pulse_extender";
pub const PULSE_TIMER: &str = "create:pulse_timer";
pub const POWERED_LATCH: &str = "create:powered_latch";
pub const POWERED_TOGGLE_LATCH: &str = "create:powered_toggle_latch";

/// `ScrollValueBehaviour.write`: the interval in ticks.
const SCROLL_VALUE: &str = "ScrollValue";
/// `BrassDiodeBlockEntity.write`: the counter.
const STATE: &str = "State";
/// The ticks left until the diode's block tick runs (`scheduleTick`), saved so a reload keeps it.
const PENDING: &str = "Pending";

/// The sprite of `block/diodes/indicator`'s `flexpeater_off`.
const INDICATOR_SPRITE: &str = "create:block/diodes/pulse_repeater/idle";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Brass(Brass),
    Toggle,
    Latch,
}

fn kind_of(id: &str) -> Option<Kind> {
    Some(match id {
        PULSE_REPEATER => Kind::Brass(Brass::Repeater),
        PULSE_EXTENDER => Kind::Brass(Brass::Extender),
        PULSE_TIMER => Kind::Brass(Brass::Timer),
        POWERED_TOGGLE_LATCH => Kind::Toggle,
        POWERED_LATCH => Kind::Latch,
        _ => return None,
    })
}

struct Diodes;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Diodes)
}

// --- the blocks as a level hook sees them -------------------------------------------------------

/// What a diode's state says, read in a hook that has only a level.
#[derive(Clone, Copy, Debug)]
struct Decoded {
    kind: DecodedKind,
    powering: bool,
    inverted: bool,
    facing: Dir,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DecodedKind {
    Brass,
    Toggle,
    Latch,
}

impl Decoded {
    /// `getOutputSignal`.
    fn output(self) -> i32 {
        match self.kind {
            DecodedKind::Brass => brass_output(self.powering, self.inverted),
            _ => latch_output(self.powering),
        }
    }
}

/// States already decoded: a state never changes, so each is looked at once.
static DECODED: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<u32, Option<Decoded>>>> =
    std::sync::LazyLock::new(Default::default);

/// Tells a diode's state from any other by its properties: `powering` with a horizontal `facing`,
/// and `inverted` (the brass diodes), `powered_side` (the Powered Latch) or neither (the Powered
/// Toggle Latch).
fn decode(level: &dyn LevelHost, state: BlockStateId) -> Option<Decoded> {
    if let Ok(cache) = DECODED.lock()
        && let Some(known) = cache.get(&state.0)
    {
        return *known;
    }
    let decoded = decode_uncached(level, state);
    if let Ok(mut cache) = DECODED.lock() {
        cache.insert(state.0, decoded);
    }
    decoded
}

fn decode_uncached(level: &dyn LevelHost, state: BlockStateId) -> Option<Decoded> {
    let powering = signal::boolean(level, state, "powering")?;
    let facing = signal::horizontal_facing(level, state)?;
    signal::boolean(level, state, "powered")?;
    let (kind, inverted) = if let Some(inverted) = signal::boolean(level, state, "inverted") {
        (DecodedKind::Brass, inverted)
    } else if signal::boolean(level, state, "powered_side").is_some() {
        (DecodedKind::Latch, false)
    } else {
        (DecodedKind::Toggle, false)
    };
    Some(Decoded { kind, powering, inverted, facing })
}

// --- the block entity ---------------------------------------------------------------------------

fn new_entity(kind: Kind) -> Compound {
    let mut nbt = Compound::new();
    if let Kind::Brass(brass) = kind {
        // `BrassDiodeBlockEntity.addBehaviours`: `maxState.setValue(defaultValue())`.
        nbt.insert(SCROLL_VALUE.into(), Tag::Int(default_value(brass)));
        nbt.insert(STATE.into(), Tag::Int(0));
    }
    nbt
}

fn flag(host: &dyn Host, pos: Pos, key: &str) -> bool {
    host.block_property(pos, key).as_deref() == Some("true")
}

fn set(host: &mut dyn Host, pos: Pos, key: &str, value: bool) {
    host.set_block_property(pos, key, if value { "true" } else { "false" });
}

fn write(host: &mut dyn Host, pos: Pos, key: &'static str, value: i32) {
    host.change_mod_block_entity(pos, &mut |entity| {
        if util::differs(&entity.nbt, key, &Tag::Int(value)) {
            entity.nbt.insert(key.into(), Tag::Int(value));
        }
    });
}

/// The interval a Pulse Repeater, Extender or Timer is set to.
fn interval(nbt: &Compound, brass: Brass) -> i32 {
    if util::has(nbt, SCROLL_VALUE) {
        int(nbt, SCROLL_VALUE).clamp(diodes::MIN_VALUE, diodes::MAX_VALUE)
    } else {
        default_value(brass)
    }
}

fn tick(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity, kind: Kind) {
    let dimension = host.dimension();
    // `DiodeBlock.canSurvive`: it stands on something.
    if host.chunk_is_loaded(dimension, [pos[0] >> 4, pos[2] >> 4])
        && signal::replaceable(&host.block(util::add(pos, [0, -1, 0])))
    {
        host.destroy_block(pos, None, false);
        return;
    }
    let Some(facing) = util::facing(host, pos, "facing") else {
        return;
    };
    let mut powered = flag(host, pos, "powered");
    let mut powering = flag(host, pos, "powering");
    let inverted = flag(host, pos, "inverted");
    let output_before = match kind {
        Kind::Brass(_) => brass_output(powering, inverted),
        _ => latch_output(powering),
    };
    let delay = match kind {
        Kind::Brass(_) => BRASS_DELAY,
        _ => LATCH_DELAY,
    };

    // `DiodeBlock.getInputSignal`: the back (`facing`); the Powered Latch's sides are
    // `PoweredLatchBlock.isPoweredOnSides`.
    let should = signal::read_sides(host, pos, &[facing]) > 0;
    let side = kind == Kind::Latch && signal::read_sides(host, pos, &latch_sides(facing)) > 0;
    let mut powered_side = flag(host, pos, "powered_side");

    // `checkTickOnNeighbor` and `tick`.
    let mut pending = int(&entity.nbt, PENDING);
    if pending > 0 {
        pending -= 1;
        if pending == 0 {
            match kind {
                Kind::Brass(_) => {
                    let (next, again) = diode_tick(powered, should);
                    if next != powered {
                        set(host, pos, "powered", next);
                        powered = next;
                    }
                    if again {
                        pending = delay;
                    }
                }
                Kind::Toggle => {
                    let (next, again) = toggle_tick(Toggle { powered, powering }, should);
                    if next.powered != powered {
                        set(host, pos, "powered", next.powered);
                    }
                    if next.powering != powering {
                        set(host, pos, "powering", next.powering);
                    }
                    powered = next.powered;
                    powering = next.powering;
                    if again {
                        pending = delay;
                    }
                }
                Kind::Latch => {
                    let now = Latch { powered, powering, powered_side };
                    let next = latch_tick(now, should, side);
                    if next.powered != powered {
                        set(host, pos, "powered", next.powered);
                    }
                    if next.powered_side != powered_side {
                        set(host, pos, "powered_side", next.powered_side);
                    }
                    if next.powering != powering {
                        set(host, pos, "powering", next.powering);
                    }
                    (powered, powering, powered_side) = (next.powered, next.powering, next.powered_side);
                }
            }
        }
    } else {
        let wanted = match kind {
            Kind::Latch => latch_wants_tick(Latch { powered, powering, powered_side }, should, side, false),
            _ => wants_tick(powered, should, false),
        };
        if wanted {
            pending = delay;
        }
    }
    if pending != int(&entity.nbt, PENDING) {
        entity.nbt.insert(PENDING.into(), Tag::Int(pending));
    }

    // `BrassDiodeBlockEntity.tick`: the counter, from the block's `powered` and `powering`.
    let mut sync = false;
    if let Kind::Brass(brass) = kind {
        let max = interval(&entity.nbt, brass);
        let state = int(&entity.nbt, STATE);
        let step = brass_step(brass, state, max, powered, powering);
        if step.state != state {
            entity.nbt.insert(STATE.into(), Tag::Int(step.state));
            sync = quantised(state, max) != quantised(step.state, max);
        }
        if step.powering != powering {
            set(host, pos, "powering", step.powering);
            powering = step.powering;
        }
    }
    let _ = (powered, powered_side);
    let output_after = match kind {
        Kind::Brass(_) => brass_output(powering, inverted),
        _ => latch_output(powering),
    };
    if output_after != output_before {
        host.update_signal_neighbors(dimension, pos);
    }
    if sync {
        host.block_entity_sync(pos);
    }
}

/// The indicator is redrawn at 1/32 steps of progress.
fn quantised(state: i32, max: i32) -> i32 {
    (diodes::progress(state, max) * 32.0) as i32
}

// --- the hooks ----------------------------------------------------------------------------------

impl Hooks for Diodes {
    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        let kind = kind_of(event.block)?;
        Some((event.block.to_owned(), new_entity(kind)))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if kind_of(event.block).is_some() {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        let Some(kind) = kind_of(&event.entity.id) else {
            return;
        };
        tick(host, event.pos, event.entity, kind);
    }

    /// What the client draws the indicator with: the counter and the interval.
    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        let Some(Kind::Brass(_)) = kind_of(&event.entity.id) else {
            return None;
        };
        let mut tag = Compound::new();
        for key in [STATE, SCROLL_VALUE] {
            tag.insert(key.into(), Tag::Int(int(&event.entity.nbt, key)));
        }
        Some(tag)
    }

    /// `BrassDiodeRenderer`: the indicator lamp on top, from dark red to red as the counter runs
    /// from 0 to the interval (`FLEXPEATER_INDICATOR`, `Color.mixColors(0x2C0300, 0xCD0000, progress)`).
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        let Some(Kind::Brass(brass)) = kind_of(event.block) else {
            return;
        };
        let max = interval(&event.entity.nbt, brass);
        let progress = diodes::progress(int(&event.entity.nbt, STATE), max);
        let colour = mix_colours(0x2C0300, 0xCD0000, progress);
        let face = |uv: [f32; 4]| Some(CubeFace { sprite: INDICATOR_SPRITE, uv: Some(uv) });
        host.draw_cube(
            &mut *event.draw,
            [3.96875, 1.96875, 3.96875],
            [12.03125, 3.0, 12.03125],
            [
                None,
                face([4.0, 4.0, 12.0, 12.0]),
                face([4.0, 4.0, 12.0, 5.0]),
                face([4.0, 11.0, 12.0, 12.0]),
                face([4.0, 4.0, 5.0, 12.0]),
                face([11.0, 4.0, 12.0, 12.0]),
            ],
            colour,
        );
    }

    /// `DiodeBlock.getStateForPlacement`: the output faces where the player looks.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        kind_of(&context.item)?;
        let facing = looking(context.yaw, 0.0).opposite();
        level
            .state_with_property(event.state, "facing", facing.name())
            .map(Some)
    }

    /// `DiodeBlock.onPlace`: the block in front is told.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if kind_of(&event.stack.item).is_some() {
            host.update_signal_neighbors(host.dimension(), event.pos);
        }
    }

    fn block_signal(&self, level: &dyn LevelHost, event: &mut BlockSignalEvent) {
        if let Some(diode) = decode(level, event.state) {
            // `DiodeBlock.getSignal` and `getDirectSignal`: out of the side opposite `facing`.
            let out = util::direction(diodes::output_side(diode.facing));
            event.signal = if event.face == out { diode.output() } else { 0 };
        }
        signal::mask(event);
    }

    /// `AbstractDiodeBlock.isSignalSource`.
    fn is_signal_source(&self, level: &dyn LevelHost, event: &mut IsSignalSourceEvent) {
        if decode(level, event.state).is_some() {
            event.source = true;
        }
    }

    /// `DiodeBlock.hasAnalogOutputSignal`: comparators read it.
    fn has_analog_output(&self, level: &dyn LevelHost, event: &mut HasAnalogOutputEvent) {
        if decode(level, event.state).is_some() {
            event.output = true;
        }
    }

    /// `DiodeBlock.getAnalogOutputSignal`.
    fn analog_output(&self, level: &dyn LevelHost, event: &mut AnalogOutputEvent) {
        if let Some(diode) = decode(level, event.state) {
            event.output = diode.output();
        }
    }

    /// `BrassDiodeBlock.useItemOn`, `ToggleLatchBlock.useItemOn` and `PoweredLatchBlock.activated`:
    /// a click flips the brass diodes' `inverted` and the latches' `powering`, unless the player
    /// is sneaking, holds a Wrench (it has its own job) or may not build.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        let Some(kind) = kind_of(&host.block(pos)) else {
            return Flow::Continue;
        };
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if !signal::may_build(player.game_mode) || player.sneaking || signal::holds_wrench(host) {
            return Flow::Continue;
        }
        let dimension = host.dimension();
        match kind {
            Kind::Brass(_) => {
                let inverted = flag(host, pos, "inverted");
                set(host, pos, "inverted", !inverted);
                signal::lever_click(host, pos, 0.3, diodes::click_pitch(inverted));
            }
            Kind::Toggle => {
                let powering = flag(host, pos, "powering");
                set(host, pos, "powering", !powering);
                signal::lever_click(host, pos, 0.3, diodes::click_pitch(powering));
            }
            Kind::Latch => {
                let state = Latch {
                    powered: flag(host, pos, "powered"),
                    powering: flag(host, pos, "powering"),
                    powered_side: flag(host, pos, "powered_side"),
                };
                let Some(next) = latch_click(state) else {
                    return Flow::Continue;
                };
                set(host, pos, "powering", next.powering);
                signal::lever_click(host, pos, 0.3, diodes::click_pitch(state.powering));
            }
        }
        host.update_signal_neighbors(dimension, pos);
        Flow::Cancel
    }

    /// The Wrench: on the top it turns the diode (`IWrenchable.getRotatedBlockState`); on a side of
    /// a brass diode it steps the interval (the stand-in for the scroll value box). Sneaking picks
    /// the block up.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH {
            return Flow::Continue;
        }
        let pos = event.pos;
        let Some(kind) = kind_of(&host.block(pos)) else {
            return Flow::Continue;
        };
        let face = Dir::from_normal(event.face);
        util::wrench(host, pos, |host| {
            if face.is_some_and(|f| f.axis() == Axis::Y) {
                if let Some(facing) = util::facing(host, pos, "facing") {
                    host.set_block_property(pos, "facing", diodes::clockwise(facing).name());
                }
                return;
            }
            let Kind::Brass(brass) = kind else {
                return;
            };
            let Some(entity) = host.mod_block_entity(pos) else {
                return;
            };
            let next = diodes::next_step(interval(&entity.nbt, brass));
            write(host, pos, SCROLL_VALUE, next);
            // `BrassDiodeBlockEntity.onMaxDelayChanged`.
            let state = diodes::clamp_state(int(&entity.nbt, STATE), next);
            write(host, pos, STATE, state);
            host.block_entity_sync(pos);
            signal::tell(host, &format!("Redstone Interval: {}", diodes::format(next)));
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machines::diodes_signal::fake::Registry;

    const BOOL: &[&str] = &["false", "true"];
    const FACING: &[&str] = &["north", "south", "west", "east"];

    #[test]
    fn a_state_is_read_as_the_diode_it_is() {
        let mut level = Registry::default();
        // The three kinds of diode, and the blocks that must not be taken for one: a Rose Quartz
        // Lamp (no `facing`), a Redstone Link (no `powering`) and a repeater (`delay`).
        level.block(&[("facing", FACING), ("powered", BOOL), ("powering", BOOL), ("inverted", BOOL)]);
        level.block(&[("facing", FACING), ("powered", BOOL), ("powering", BOOL), ("powered_side", BOOL)]);
        level.block(&[("facing", FACING), ("powered", BOOL), ("powering", BOOL)]);
        level.block(&[("activate", BOOL), ("powered", BOOL), ("powering", BOOL)]);
        level.block(&[("facing", FACING), ("powered", BOOL), ("receiver", BOOL)]);
        level.block(&[("facing", FACING), ("powered", BOOL), ("delay", &["1", "2"])]);

        let brass = level.find(&[
            ("facing", "south"),
            ("powered", "false"),
            ("powering", "true"),
            ("inverted", "true"),
        ]);
        let d = decode(&level, brass).expect("a brass diode");
        assert_eq!((d.kind, d.facing, d.powering, d.inverted), (DecodedKind::Brass, Dir::South, true, true));
        // Powering and inverted cancel.
        assert_eq!(d.output(), 0);
        let plain = level.find(&[
            ("facing", "west"),
            ("powered", "true"),
            ("powering", "false"),
            ("inverted", "true"),
        ]);
        assert_eq!(decode(&level, plain).unwrap().output(), 15);

        let latch = level.find(&[
            ("facing", "east"),
            ("powered", "true"),
            ("powering", "true"),
            ("powered_side", "false"),
        ]);
        let d = decode(&level, latch).expect("a powered latch");
        assert_eq!((d.kind, d.facing, d.output()), (DecodedKind::Latch, Dir::East, 15));

        let toggle = level.find(&[("facing", "north"), ("powered", "false"), ("powering", "false")]);
        let d = decode(&level, toggle).expect("a toggle latch");
        assert_eq!((d.kind, d.facing, d.output()), (DecodedKind::Toggle, Dir::North, 0));

        let lamp = level.find(&[("activate", "false"), ("powered", "true"), ("powering", "true")]);
        assert!(decode(&level, lamp).is_none());
        let link = level.find(&[("facing", "north"), ("powered", "true"), ("receiver", "true")]);
        assert!(decode(&level, link).is_none());
        let repeater = level.find(&[("facing", "north"), ("powered", "true"), ("delay", "2")]);
        assert!(decode(&level, repeater).is_none());
    }

    #[test]
    fn the_kinds_are_told_apart_by_id() {
        assert_eq!(kind_of(PULSE_TIMER), Some(Kind::Brass(Brass::Timer)));
        assert_eq!(kind_of(POWERED_LATCH), Some(Kind::Latch));
        assert_eq!(kind_of(POWERED_TOGGLE_LATCH), Some(Kind::Toggle));
        assert_eq!(kind_of("create:shaft"), None);
        assert_eq!(quantised(0, 20), 0);
        assert_eq!(quantised(10, 20), 16);
        assert_eq!(quantised(20, 20), 32);
    }
}
