//! The Sequenced Gearshift in the game (`SequencedGearshiftBlock`, `SequencedGearshiftBlockEntity`,
//! `SplitShaftRenderer`): a casing with a shaft out of each end that, when redstone starts it, runs
//! a list of up to five instructions, and passes the rotation on at the speed each one asks for.
//!
//! The instruction machine is `create_kinetics::rules::sequenced_gearshift` and how the output end's speed
//! is carried through the network is `Kind::sequenced`; this reads the world, asks the rules, and writes
//! what they say. The rules' header lists the known differences; the ones that live here:
//!
//! - **The screen is replaced by clicks.** An empty hand moves a cursor over the fields of the
//!   program (a sneaking one back) and shows the program on the action bar with the field in
//!   brackets. The Wrench steps the field under the cursor: forward on a face pointing the positive
//!   way (up, south, east), back on a negative one (down, north, west), through the values in
//!   `Op::steps`; sneaking with it picks the block up. Editing stops a running sequence
//!   (`ConfigureSequencedGearshiftPacket`). The Wrench therefore doesn't turn this block as Create's does
//!   (`onWrenched` toggles `vertical`): place it again, or sneak-place it, to change its axis.
//! - Redstone is read every tick, but the block acts only when the signal changes, as it does
//!   after a neighbour update in Create.
//! - Placement along the axis the player looks, or a shaft beside it, is done as the other axis
//!   blocks' (`encased.rs`); Create's `TransformableBlock` isn't.
//! - Create's `ComputerCraft` peripheral is not here.

use std::collections::BTreeMap;

use create_kinetics::generators::preferred_shaft;
use create_kinetics::rules::gearshift::{clutch_input, ends};
use create_kinetics::rules::sequenced_gearshift::{
    self as rules, Cursor, Instruction, Modifier, Op, Sequence, TRIMMED_OVERSHOOT,
};
use create_kinetics::{Axis, Dir, Kind, Node, Pos, face_sign, looking};
use crate::compat::{
    AnalogOutputEvent, BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent,
    BlockEntityTicksEvent, BlockEntityUpdateTagEvent, BlockStateId, Compound, Flow,
    HasAnalogOutputEvent, Hooks, Host, LevelHost, PlaceBlockEvent, RightClickBlockEvent,
    StateForPlacementEvent, Tag, Text, UseItemOnEvent,
};

use super::encased;
use crate::{kinetic, util};

pub const ID: &str = "create:sequenced_gearshift";

const INDEX: &str = "InstructionIndex";
const DURATION: &str = "InstructionDuration";
const PROGRESS: &str = "InstructionProgress";
const TIMER: &str = "Timer";
const PREV_POWERED: &str = "PrevPowered";
const INSTRUCTIONS: &str = "Instructions";
/// The stand-in's cursor (`Cursor::to_int`): where the edits go.
const CURSOR: &str = "Cursor";
/// Sent to clients for the drawing: the output end's multiple of the input's, and the input end.
const MODIFIER: &str = "Modifier";
const INPUT: &str = "InputFace";

/// Ticks without a turning input before a running sequence ends (the network needs a few after a load).
const STOPPED_TICKS: u32 = 5;

/// How long each running sequencer has had no input speed, by position: only those that have none.
static STOPPED: std::sync::Mutex<BTreeMap<(&'static str, Pos), u32>> = std::sync::Mutex::new(BTreeMap::new());

/// Counts another tick of no speed at `pos` (`stopped`), or forgets the count; the count so far.
fn stopped_for(host: &dyn Host, pos: Pos, stopped: bool) -> u32 {
    let mut map = STOPPED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let key = (host.dimension(), pos);
    if stopped {
        let count = map.entry(key).or_insert(0);
        *count = (*count + 1).min(STOPPED_TICKS + 1);
        *count
    } else {
        map.remove(&key);
        0
    }
}

struct Sequencer;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Sequencer)
}

// --- saved state --------------------------------------------------------------------------------

fn text(nbt: &Tag, key: &str) -> Option<String> {
    match nbt.get(key) {
        Some(Tag::String(value)) => Some(value.to_string_lossy()),
        _ => None,
    }
}

fn read_instruction(tag: &Tag) -> Option<Instruction> {
    let op = Op::from_name(&text(tag, "Type")?)?;
    let modifier = Modifier::from_name(&text(tag, "Modifier")?).unwrap_or(Modifier::Forward);
    let value = match tag.get("Value") {
        Some(Tag::Int(value)) => *value,
        _ => 0,
    };
    Some(Instruction { op, modifier, value })
}

fn write_instruction(instruction: &Instruction) -> Tag {
    let mut tag = Compound::new();
    tag.insert("Type".into(), Tag::String(instruction.op.name().into()));
    tag.insert("Modifier".into(), Tag::String(instruction.modifier.name().into()));
    tag.insert("Value".into(), Tag::Int(instruction.value));
    Tag::Compound(tag)
}

/// `read`: the saved sequence; a block with none saved is idle with `createDefault`'s program.
fn read_sequence(nbt: &Compound) -> Sequence {
    let instructions = match util::get(nbt, INSTRUCTIONS) {
        Some(Tag::List(list)) => list.iter().filter_map(read_instruction).collect(),
        _ => Vec::new(),
    };
    let int_or = |key: &str| if util::has(nbt, key) { util::int(nbt, key) } else { -1 };
    Sequence {
        instructions: if instructions.is_empty() { rules::default_program() } else { instructions },
        current: int_or(INDEX),
        duration: int_or(DURATION),
        progress: util::float(nbt, PROGRESS).unwrap_or(0.0),
        timer: util::int(nbt, TIMER),
        powered_previously: util::boolean(nbt, PREV_POWERED),
        overshoot: TRIMMED_OVERSHOOT,
    }
}

/// Writes only what differs, as every write to a block entity counts as a change.
fn write_sequence(nbt: &mut Compound, now: &Sequence, before: &Sequence) {
    if now.current != before.current || !util::has(nbt, INDEX) {
        nbt.insert(INDEX.into(), Tag::Int(now.current));
    }
    if now.duration != before.duration || !util::has(nbt, DURATION) {
        nbt.insert(DURATION.into(), Tag::Int(now.duration));
    }
    if now.progress != before.progress {
        util::set_float(nbt, PROGRESS, now.progress);
    }
    if now.timer != before.timer {
        nbt.insert(TIMER.into(), Tag::Int(now.timer));
    }
    if now.powered_previously != before.powered_previously {
        nbt.insert(PREV_POWERED.into(), Tag::Byte(i8::from(now.powered_previously)));
    }
    if now.instructions != before.instructions {
        let list = now.instructions.iter().map(write_instruction).collect();
        nbt.insert(INSTRUCTIONS.into(), Tag::List(list));
    }
}

fn state_value(host: &dyn Host, pos: Pos) -> u8 {
    host.block_property(pos, "state")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

fn set_state(host: &mut dyn Host, pos: Pos, value: Option<u8>) {
    if let Some(value) = value
        && state_value(host, pos) != value
    {
        host.set_block_property(pos, "state", &value.to_string());
    }
}

/// `getRotationAxis`: the state's `axis`, or up when `vertical`.
fn axis_at(host: &dyn Host, pos: Pos) -> Option<Axis> {
    let horizontal = host.block_property(pos, "axis").as_deref().and_then(Axis::from_name)?;
    let vertical = host.block_property(pos, "vertical").as_deref() == Some("true");
    Some(rules::rotation_axis(horizontal, vertical))
}

/// The speed of the shaft end the block at `side` of `pos` points at it, if it has one.
fn neighbour_speed(host: &dyn Host, pos: Pos, side: Dir) -> Option<f32> {
    let at = util::add(pos, side.normal());
    let dimension = host.dimension();
    let networks = kinetic::networks();
    let kind = networks.kind_at(dimension, at)?;
    let end = side.opposite();
    if !kind.has_shaft(end) {
        return None;
    }
    networks.speed_at(dimension, at).map(|speed| speed * face_sign(kind, end))
}

fn input_of(nbt: &Compound) -> Option<Dir> {
    match util::get(nbt, INPUT) {
        Some(Tag::String(name)) => Dir::from_name(&name.to_string_lossy()),
        _ => None,
    }
}

// --- the hooks ------------------------------------------------------------------------------------

impl Hooks for Sequencer {
    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        // An idle sequence with `createDefault`'s program: `read_sequence` fills in what is missing.
        (event.block == ID).then(|| (ID.to_owned(), kinetic::new_entity()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        let pos = event.pos;
        let Some(axis) = axis_at(host, pos) else { return };
        let before = read_sequence(&event.entity.nbt);
        let mut seq = before.clone();
        let previous = kinetic::speed(event.entity);

        // `getRotationSpeedModifier`: the output end turns by what the instruction running says.
        let modifier = seq.modifier();
        let node = Node::passive(Kind::sequenced(axis, modifier));
        let Some((speed, mut changed)) = kinetic::tick(host, pos, event.entity, node) else {
            return;
        };
        let theory = kinetic::stress_at(host, pos).1;
        // `SequencedGearshiftBlock.neighborChanged`: the signal the neighbour event left (`signals`).
        let signal = crate::signals::read(host, &mut event.entity.nbt, pos) > 0;

        let mut state = None;
        // `onSpeedChanged`. A network that has just been loaded reports no speed for a few ticks:
        // a stop ends the sequence only once it has lasted.
        if speed == 0.0 && !seq.is_idle() {
            if stopped_for(host, pos, true) == STOPPED_TICKS {
                state = seq.speed_changed(1.0, 0.0, theory, signal).or(state);
            }
        } else {
            stopped_for(host, pos, false);
            if previous != speed {
                state = seq.speed_changed(previous, speed, theory, signal).or(state);
            }
        }
        // `onRedstoneUpdate`, when the signal changed (Create's `neighborChanged` schedules it).
        if signal != seq.powered_previously {
            let running = state_value(host, pos) != 0;
            state = seq.redstone(signal, running, speed, theory).or(state);
        }
        // `tick`.
        state = seq.tick(speed, theory, signal).or(state);
        set_state(host, pos, state);

        if seq != before {
            write_sequence(&mut event.entity.nbt, &seq, &before);
        }
        // What clients draw: which end is the input, and what the other turns at.
        let new_modifier = seq.modifier();
        let old_input = input_of(&event.entity.nbt);
        let input = clutch_input(axis, speed, old_input, |side| neighbour_speed(host, pos, side));
        if input != old_input {
            event.entity.nbt.insert(INPUT.into(), Tag::String(input.map_or("", Dir::name).into()));
            changed = true;
        }
        if util::int(&event.entity.nbt, MODIFIER) != new_modifier {
            event.entity.nbt.insert(MODIFIER.into(), Tag::Int(new_modifier));
            changed = true;
        }
        if changed {
            host.block_entity_sync(pos);
        }
    }

    fn block_entity_update_tag(
        &self,
        event: &mut BlockEntityUpdateTagEvent<'_>,
    ) -> Option<Compound> {
        if event.entity.id != ID {
            return None;
        }
        let mut tag = kinetic::update_tag(event.entity);
        tag.insert(MODIFIER.into(), Tag::Int(util::int(&event.entity.nbt, MODIFIER)));
        let name = input_of(&event.entity.nbt).map_or("", Dir::name);
        tag.insert(INPUT.into(), Tag::String(name.into()));
        Some(tag)
    }

    /// `SplitShaftRenderer`: each end's half shaft at the speed times its modifier.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let Some(axis) = axis_at_state(host, event.state) else { return };
        let rpm = kinetic::speed(event.entity);
        let modifier = util::int(&event.entity.nbt, MODIFIER) as f32;
        let input = input_of(&event.entity.nbt);
        for face in ends(axis) {
            let speed = if input.is_none_or(|input| input == face) { rpm } else { rpm * modifier };
            encased::draw_half(host, &mut *event.draw, face, speed, encased::shaft_phase(axis, event.pos));
        }
        event.draw.keep();
    }

    /// `SequencedGearshiftBlock.getStateForPlacement`: along the axis the player looks.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        let item: &str = &context.item;
        if item != ID {
            return None;
        }
        let looked = looking(context.yaw, context.pitch);
        let (axis, vertical) = rules::placed_state(looked.axis(), looking(context.yaw, 0.0));
        let state = level.state_with_property(event.state, "axis", axis.name())?;
        level
            .state_with_property(state, "vertical", if vertical { "true" } else { "false" })
            .map(Some)
    }

    /// Lines the block up with a shaft end beside it (`RotatedPillarKineticBlock.getPreferredAxis`),
    /// unless the player sneaks.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if &*event.stack.item != ID {
            return;
        }
        let Some(player) = host.acting_player() else { return };
        if player.sneaking {
            return;
        }
        let pos = event.pos;
        let Some(preferred) = preferred_shaft(|side| encased::neighbour_kind(host, pos, side)).map(Dir::axis)
        else {
            return;
        };
        let (axis, vertical) = rules::placed_state(preferred, looking(player.yaw, 0.0));
        if host.block_property(pos, "axis").as_deref() != Some(axis.name()) {
            host.set_block_property(pos, "axis", axis.name());
        }
        let vertical = if vertical { "true" } else { "false" };
        if host.block_property(pos, "vertical").as_deref() != Some(vertical) {
            host.set_block_property(pos, "vertical", vertical);
        }
    }

    /// An empty hand moves the cursor over the program's fields.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else { return Flow::Continue };
        if player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        if host.held_stack(player.uuid).is_some_and(|stack| stack.count > 0) {
            return Flow::Continue;
        }
        if !util::first_click(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        let mut shown = None;
        host.change_mod_block_entity(pos, &mut |entity| {
            let seq = read_sequence(&entity.nbt);
            let cursor = Cursor::from_int(util::int(&entity.nbt, CURSOR));
            let cursor = rules::move_cursor(&seq.instructions, cursor, !player.sneaking);
            if util::int(&entity.nbt, CURSOR) != cursor.to_int() {
                entity.nbt.insert(CURSOR.into(), Tag::Int(cursor.to_int()));
            }
            shown = Some(rules::describe(&seq.instructions, cursor));
        });
        if let Some(line) = shown {
            host.action_bar(player.uuid, Text::literal(line));
        }
        Flow::Cancel
    }

    /// The Wrench steps the field under the cursor, and picks the block up while sneaking.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        if &*event.stack.item != util::WRENCH || host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(face) = Dir::from_normal(event.face) else { return Flow::Continue };
        let Some(player) = host.acting_player() else { return Flow::Continue };
        if !util::first_click(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        util::wrench(host, pos, |host| {
            let signal = host.neighbor_signal(host.dimension(), pos) > 0;
            let theory = kinetic::stress_at(host, pos).1;
            let mut result = None;
            host.change_mod_block_entity(pos, &mut |entity| {
                let mut seq = read_sequence(&entity.nbt);
                let before = seq.clone();
                let cursor = Cursor::from_int(util::int(&entity.nbt, CURSOR));
                // `ConfigureSequencedGearshiftPacket`: stop the sequence, then replace the program.
                let state = seq.run(-1, theory, signal);
                rules::step_field(&mut seq.instructions, cursor, face.sign() > 0);
                write_sequence(&mut entity.nbt, &seq, &before);
                result = Some((state, rules::describe(&seq.instructions, cursor)));
            });
            if let Some((state, line)) = result {
                set_state(host, pos, state);
                host.block_entity_sync(pos);
                host.action_bar(player.uuid, Text::literal(line));
            }
        })
    }

    /// `hasAnalogOutputSignal`: the comparator reads the instruction running, 0 to 5.
    fn has_analog_output(&self, level: &dyn LevelHost, event: &mut HasAnalogOutputEvent) {
        if state_of(level, event.state).is_some() {
            event.output = true;
        }
    }

    fn analog_output(&self, level: &dyn LevelHost, event: &mut AnalogOutputEvent) {
        if let Some(state) = state_of(level, event.state) {
            event.output = rules::comparator(state);
        }
    }
}

/// The `state` property of a Sequenced Gearshift's block state, read from the state alone;
/// `None` for any other block.
fn state_of(level: &dyn LevelHost, state: BlockStateId) -> Option<u8> {
    level.state_with_property(state, "vertical", "true")?;
    (0..=5u8).find(|n| level.state_with_property(state, "state", &n.to_string()) == Some(state))
}

fn axis_at_state(host: &dyn Host, state: BlockStateId) -> Option<Axis> {
    let horizontal = host.block_state_property(state, "axis").and_then(Axis::from_name)?;
    let vertical = host.block_state_property(state, "vertical") == Some("true");
    Some(rules::rotation_axis(horizontal, vertical))
}
