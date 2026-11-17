//! The Analog Lever in the game (`AnalogLeverBlock`, `AnalogLeverBlockEntity`,
//! `AnalogLeverRenderer`): click to raise its 0 to 15 output, sneak-click to lower it. The rules
//! are `create_kinetics::rules::analog_lever`; the signal helpers are in `diodes_signal.rs`.
//!
//! Known differences from Create, for the README:
//! - A hook that gives a signal (`block_signal`) can read only the level's blocks, not a block
//!   entity, so the output is also kept in a table (`SIGNALS`) that the block entity fills; a lever
//!   in a chunk that has just loaded gives 0 until its block entity has ticked once, and
//!   tells its neighbours then.
//! - The handle's easing is worked out from the time of the change (saved as `From` and `Since`).
//! - Goggles show "Analog Strength" (`goggles.rs`).
//! - A click with an item in the hand is taken by the lever too (Create's `useWithoutItem` is
//!   reached for any held item); sneaking with an item in either hand places it, as in Create.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{LazyLock, Mutex};

use create_kinetics::rules::analog_lever::{
    self, CHANGE_DELAY, SETTLED_AFTER, change_state, click_pitch, eased, handle_angle,
    indicator_colour, tick_timer,
};
use create_kinetics::rules::redstone_link::horizontal_angle;
use create_kinetics::{Dir, Pos, looking};
use horizon_client::mod_api::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, BlockSignalEvent, BlockStateId, Compound, CubeFace, Flow, Hooks,
    Host, IsSignalSourceEvent, LevelHost, ModBlockEntity, RightClickBlockEvent, ServerTickEvent,
    ServerTickPhase, StandaloneModelsEvent, StateForPlacementEvent, Tag,
};

use super::diodes_signal as signal;
use crate::{int, util};

pub const ID: &str = "create:analog_lever";

const STATE: &str = "State";
const CHANGE_TIMER: &str = "ChangeTimer";
const FROM: &str = "From";
const SINCE: &str = "Since";

/// `AllPartialModels.ANALOG_LEVER_HANDLE`.
const HANDLE: &str = "create:block/analog_lever/handle";
/// `AllPartialModels.ANALOG_LEVER_INDICATOR`'s texture (the indicator is drawn as a tinted box, as
/// the model's faces carry no tint index).
const INDICATOR_SPRITE: &str = "create:block/analog_lever";
const CENTRE: [f32; 3] = [0.5; 3];
const UP: [f32; 3] = [0.0, 1.0, 0.0];
const EAST: [f32; 3] = [1.0, 0.0, 0.0];

/// A server tick counter, for pruning.
static NOW: AtomicI64 = AtomicI64::new(0);

/// What each ticking lever gives: its output and when it last ticked, by level and position.
static SIGNALS: LazyLock<Mutex<HashMap<(u8, Pos), (i32, i64)>>> = LazyLock::new(Default::default);

/// Ticks a lever may miss before it counts as gone.
const MAX_AGE: i64 = 3;

struct AnalogLever;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(AnalogLever)
}

/// Publishes a lever's output and the tick it was published in. True when the table had no entry
/// for it before (the lever is new, or has just loaded).
fn publish(level: u8, pos: Pos, state: i32) -> bool {
    let now = NOW.load(Ordering::Relaxed);
    match SIGNALS.lock() {
        Ok(mut table) => table.insert((level, pos), (state, now)).is_none(),
        Err(_) => false,
    }
}

fn tick(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity) {
    let level = signal::host_level(host);
    let dimension = host.dimension();
    let state = int(&entity.nbt, STATE);
    // A lever that has just loaded tells its neighbours what it gives.
    if publish(level, pos, state) && state != 0 {
        host.update_signal_neighbors(dimension, pos);
    }
    // `AnalogLeverBlockEntity.tick`.
    let timer = int(&entity.nbt, CHANGE_TIMER);
    if timer > 0 {
        let (next, fire) = tick_timer(timer);
        entity.nbt.insert(CHANGE_TIMER.into(), Tag::Int(next));
        if fire {
            // `AnalogLeverBlock.updateNeighbors`: the lever's and the block it hangs on.
            host.update_signal_neighbors(dimension, pos);
        }
    }
}

/// The lever's face, facing and what its block entity says, for drawing.
fn handle_value(entity: &ModBlockEntity, now: f32) -> (f32, f32) {
    let nbt = &entity.nbt;
    let target = int(nbt, STATE) as f32;
    let from = util::float(nbt, FROM).unwrap_or(0.0);
    let age = now - util::long(nbt, SINCE) as f32;
    (eased(from, target, age), age)
}

impl Hooks for AnalogLever {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.push(HANDLE.into());
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == ID).then(|| {
            let mut nbt = Compound::new();
            nbt.insert(STATE.into(), Tag::Int(0));
            nbt.insert(CHANGE_TIMER.into(), Tag::Int(0));
            util::set_float(&mut nbt, FROM, 0.0);
            nbt.insert(SINCE.into(), Tag::Long(0));
            (ID.to_owned(), nbt)
        })
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id == ID {
            tick(host, event.pos, event.entity);
        }
    }

    /// Prunes the levers that stopped ticking (their chunk unloaded, or the block went).
    fn server_tick(&self, _: &mut dyn Host, event: &mut ServerTickEvent<'_>) {
        if let ServerTickPhase::ServerTickStart { world_running: true } = event.phase {
            let now = NOW.fetch_add(1, Ordering::Relaxed) + 1;
            if let Ok(mut table) = SIGNALS.lock() {
                table.retain(|_, (_, seen)| now - *seen < MAX_AGE);
            }
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        if event.entity.id != ID {
            return None;
        }
        let nbt = &event.entity.nbt;
        let mut tag = Compound::new();
        tag.insert(STATE.into(), Tag::Int(int(nbt, STATE)));
        util::set_float(&mut tag, FROM, util::float(nbt, FROM).unwrap_or(0.0));
        tag.insert(SINCE.into(), Tag::Long(util::long(nbt, SINCE)));
        Some(tag)
    }

    /// `AnalogLeverRenderer`: the handle turns a quarter turn as the output rises, and the
    /// indicator on the base goes from dark red to red.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let face = host.block_state_property(event.state, "face").unwrap_or("floor");
        let facing = host
            .block_state_property(event.state, "facing")
            .and_then(Dir::from_name)
            .unwrap_or(Dir::North);
        let (value, age) = handle_value(
            event.entity,
            event.draw.game_time as f32 + event.draw.partial,
        );
        // `AnalogLeverRenderer.transform`: x by the face, y by the facing, about the centre.
        let x_turn = match face {
            "floor" => 0.0,
            "wall" => 90.0,
            _ => 180.0,
        };
        let y_turn = horizontal_angle(facing.normal()) as f32;
        let draw = &mut *event.draw;

        draw.push();
        draw.rotate_about(UP, y_turn, CENTRE);
        draw.rotate_about(EAST, x_turn, CENTRE);
        draw.rotate_about(EAST, handle_angle(value), [0.5, 1.0 / 16.0, 0.5]);
        host.draw_model(draw, HANDLE, 0xFF_FFFF);
        draw.pop();

        draw.push();
        draw.rotate_about(UP, y_turn, CENTRE);
        draw.rotate_about(EAST, x_turn, CENTRE);
        let strip = |uv: [f32; 4]| Some(CubeFace { sprite: INDICATOR_SPRITE, uv: Some(uv) });
        host.draw_cube(
            draw,
            [4.99, 1.0, 3.99],
            [11.01, 2.0, 12.01],
            [
                None,
                None,
                strip([3.0, 9.0, 9.0, 10.0]),
                strip([3.0, 9.0, 9.0, 10.0]),
                strip([3.0, 9.0, 11.02, 10.0]),
                strip([3.0, 9.0, 11.02, 10.0]),
            ],
            indicator_colour(value),
        );
        draw.pop();
        // The handle eases for a while after a change; once it has settled the frame may be kept.
        if age > SETTLED_AFTER {
            draw.keep();
        }
    }

    /// `FaceAttachedHorizontalDirectionalBlock.getStateForPlacement`: on a floor or a ceiling
    /// facing the way the player does; on a wall, facing out of it.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != ID {
            return None;
        }
        let face = util::dir(context.face);
        let (attach, facing) = match face {
            Dir::Up => ("floor", looking(context.yaw, 0.0)),
            Dir::Down => ("ceiling", looking(context.yaw, 0.0)),
            wall => ("wall", wall),
        };
        let state = level.state_with_property(event.state, "face", attach)?;
        level
            .state_with_property(state, "facing", facing.name())
            .map(Some)
    }

    fn block_signal(&self, level: &dyn LevelHost, event: &mut BlockSignalEvent) {
        if let Some((face, facing)) = lever_of(level, event.state) {
            let dimension = signal::level_index(event.dimension);
            let pos = [event.pos.x, event.pos.y, event.pos.z];
            let state = SIGNALS
                .lock()
                .ok()
                .and_then(|table| table.get(&(dimension, pos)).map(|&(s, _)| s))
                .unwrap_or(0);
            // `AnalogLeverBlock.getSignal` is the output on every side; `getDirectSignal` only
            // into the block it is attached to (`getConnectedDirection() == side`).
            event.signal = if !event.direct {
                state
            } else {
                let connected = analog_lever::connected_direction(face, facing.name());
                let towards_support = Dir::from_name(connected).map(Dir::opposite);
                if Some(util::dir(event.face)) == towards_support { state } else { 0 }
            };
        }
        signal::mask(event);
    }

    /// `AnalogLeverBlock.isSignalSource`.
    fn is_signal_source(&self, level: &dyn LevelHost, event: &mut IsSignalSourceEvent) {
        if lever_of(level, event.state).is_some() {
            event.source = true;
        }
    }

    /// `AnalogLeverBlock.useWithoutItem`: a click raises the output, a click while sneaking lowers
    /// it. A player who sneaks while holding something uses that instead.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.sneaking && !signal::hands_empty(host) {
            return Flow::Continue;
        }
        // The off hand's turn of a click that already stepped it.
        if util::click_repeated(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        let back = player.sneaking;
        let now = util::game_time(host);
        let mut new_state = 0;
        let changed = host.change_mod_block_entity(pos, &mut |entity| {
            let nbt = &entity.nbt;
            let state = int(nbt, STATE);
            let (next, changed) = change_state(state, back);
            new_state = next;
            if changed {
                // The handle stands where it had got to on its way to the old output.
                let from = util::float(nbt, FROM).unwrap_or(0.0);
                let since = util::long(nbt, SINCE);
                let shown = eased(from, state as f32, (now - since) as f32);
                entity.nbt.insert(STATE.into(), Tag::Int(next));
                entity.nbt.insert(CHANGE_TIMER.into(), Tag::Int(CHANGE_DELAY));
                util::set_float(&mut entity.nbt, FROM, shown);
                entity.nbt.insert(SINCE.into(), Tag::Long(now));
            }
        });
        if !changed {
            return Flow::Continue;
        }
        util::click_taken(host, player.uuid, pos);
        let level = signal::host_level(host);
        publish(level, pos, new_state);
        host.block_entity_sync(pos);
        signal::lever_click(host, pos, 0.2, click_pitch(new_state));
        Flow::Cancel
    }
}

/// States already looked at: a state never changes.
static LEVER_STATES: LazyLock<Mutex<HashMap<u32, Option<(&'static str, Dir)>>>> =
    LazyLock::new(Default::default);

/// The analog lever's `face` and `facing`, if the state is one: a block with `face` and `facing`
/// and no `powered` (a vanilla lever or button has it).
fn lever_of(level: &dyn LevelHost, state: BlockStateId) -> Option<(&'static str, Dir)> {
    if let Ok(cache) = LEVER_STATES.lock()
        && let Some(known) = cache.get(&state.0)
    {
        return *known;
    }
    let found = lever_uncached(level, state);
    if let Ok(mut cache) = LEVER_STATES.lock() {
        cache.insert(state.0, found);
    }
    found
}

fn lever_uncached(level: &dyn LevelHost, state: BlockStateId) -> Option<(&'static str, Dir)> {
    let face = signal::choice(level, state, "face", &["floor", "wall", "ceiling"])?;
    if signal::takes(level, state, "powered", "true") {
        return None;
    }
    let facing = signal::horizontal_facing(level, state)?;
    Some((face, facing))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machines::diodes_signal::fake::Registry;

    const FACES: &[&str] = &["floor", "wall", "ceiling"];
    const FACING: &[&str] = &["north", "south", "west", "east"];

    #[test]
    fn only_the_analog_lever_is_taken_for_one() {
        let mut level = Registry::default();
        // The analog lever, a vanilla lever or button (with `powered`) and a block with a `face`
        // but no `facing`.
        level.block(&[("face", FACES), ("facing", FACING)]);
        level.block(&[("face", FACES), ("facing", FACING), ("powered", &["false", "true"])]);
        level.block(&[("face", FACES)]);

        let ours = level.find(&[("face", "ceiling"), ("facing", "east")]);
        assert_eq!(lever_of(&level, ours), Some(("ceiling", Dir::East)));
        let vanilla = level.find(&[("face", "wall"), ("facing", "north"), ("powered", "true")]);
        assert_eq!(lever_of(&level, vanilla), None);
        let facing_less = level.find(&[("face", "floor")]);
        assert_eq!(lever_of(&level, facing_less), None);
    }

    #[test]
    fn a_table_entry_is_new_once_and_the_output_is_kept() {
        let pos = [3, 4, 5];
        assert!(publish(7, pos, 4));
        assert!(!publish(7, pos, 5));
        assert_eq!(SIGNALS.lock().unwrap().get(&(7, pos)).map(|e| e.0), Some(5));
    }
}
