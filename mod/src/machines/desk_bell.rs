//! The Desk Bell (`DeskBellBlock`, `DeskBellBlockEntity`, `DeskBellRenderer`): click it to ring
//! it. It dings, gives a 20 tick redstone pulse and its plunger and bell move. The rules are
//! `create_kinetics::rules::desk_bell`; the hooks that give a signal are in the same style as
//! `analog_lever.rs`. Its header lists the known differences from Create; one more here: the
//! ding is played for everyone, the player who clicked included, by the server (Create plays it
//! locally for them and for the others from the server), and the sound's 64 block range is set by
//! Create's `sounds.json` entry.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{LazyLock, Mutex};

use create_kinetics::rules::desk_bell as rules;
use create_kinetics::rules::redstone_link::horizontal_angle;
use create_kinetics::{Dir, Pos};
use crate::compat::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, BlockSignalEvent, BlockStateId, Compound, Flow, Hooks, Host,
    IsSignalSourceEvent, LevelHost, RightClickBlockEvent, ServerTickEvent, ServerTickPhase,
    StandaloneModelsEvent, StateForPlacementEvent, Tag, UseItemOnEvent,
};

use super::diodes_signal as signal;
use crate::{int, util};

pub const ID: &str = "create:desk_bell";

const TIMER: &str = "Timer";
const SINCE: &str = "Since";

/// `AllPartialModels.DESK_BELL_PLUNGER` and `DESK_BELL_BELL`.
const PLUNGER: &str = "create:block/desk_bell/plunger";
const BELL: &str = "create:block/desk_bell/bell";
/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;
const FACINGS: &[&str] = &["down", "up", "north", "south", "west", "east"];
const MAX_AGE: i64 = 3;

/// A server tick counter, for pruning.
static NOW: AtomicI64 = AtomicI64::new(0);

/// Whether each ticking bell is powered and when it last ticked, by level and position.
static SIGNALS: LazyLock<Mutex<HashMap<(u8, Pos), (bool, i64)>>> = LazyLock::new(Default::default);

struct DeskBell;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(DeskBell)
}

fn publish(level: u8, pos: Pos, powered: bool) -> bool {
    let now = NOW.load(Ordering::Relaxed);
    match SIGNALS.lock() {
        Ok(mut table) => table.insert((level, pos), (powered, now)).is_none(),
        Err(_) => false,
    }
}

impl Hooks for DeskBell {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.push(PLUNGER.into());
        event.models.push(BELL.into());
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == ID).then(|| {
            let mut nbt = Compound::new();
            nbt.insert(TIMER.into(), Tag::Int(0));
            nbt.insert(SINCE.into(), Tag::Long(-1000));
            (ID.to_owned(), nbt)
        })
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    /// `DeskBellBlockEntity.tick`: after 20 ticks the bell lets go.
    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        let pos = event.pos;
        let level = signal::host_level(host);
        let timer = int(&event.entity.nbt, TIMER);
        if publish(level, pos, timer > 0) && timer > 0 {
            let dimension = host.dimension();
            host.update_signal_neighbors(dimension, pos);
        }
        if timer > 0 {
            let (next, release) = rules::tick(timer);
            event.entity.nbt.insert(TIMER.into(), Tag::Int(next));
            if release {
                publish(level, pos, false);
                host.set_block_property(pos, "powered", "false");
                let dimension = host.dimension();
                host.update_signal_neighbors(dimension, pos);
            }
        }
    }

    fn block_entity_update_tag(
        &self,
        event: &mut BlockEntityUpdateTagEvent<'_>,
    ) -> Option<Compound> {
        if event.entity.id != ID {
            return None;
        }
        let mut tag = Compound::new();
        tag.insert(SINCE.into(), Tag::Long(util::long(&event.entity.nbt, SINCE)));
        Some(tag)
    }

    /// `DeskBellRenderer`: while ringing the plunger and the bell are drawn apart from the block
    /// model (which is then the base alone, `block_powered`).
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let since = util::long(&event.entity.nbt, SINCE);
        let age = (event.draw.game_time - since) as f32 + event.draw.partial;
        let p = rules::progress(age);
        let powered = host.block_state_property(event.state, "powered") == Some("true");
        if rules::is_resting(p, powered) {
            event.draw.keep();
            return;
        }
        let facing = host
            .block_state_property(event.state, "facing")
            .and_then(Dir::from_name)
            .unwrap_or(Dir::Up);
        let yaw = horizontal_angle(facing.normal()) as f32;
        let turn = rules::vertical_turn(facing);
        let pose = rules::pose(p, rules::animation_offset(since));
        let draw = &mut *event.draw;
        let up = [0.0, 1.0, 0.0];
        let east = [1.0, 0.0, 0.0];
        let south = [0.0, 0.0, 1.0];
        let origin = [0.0; 3];

        draw.push();
        draw.translate([0.5; 3]);
        draw.rotate_about(up, yaw, origin);
        draw.rotate_about(east, turn, origin);
        draw.translate([-0.5; 3]);
        draw.translate([0.0, pose.plunger, 0.0]);
        host.draw_model(draw, PLUNGER, 0xFF_FFFF);
        draw.pop();

        draw.push();
        draw.translate([0.5; 3]);
        draw.rotate_about(up, yaw, origin);
        draw.rotate_about(east, turn, origin);
        draw.translate([0.0, -1.0 / 16.0, 0.0]);
        draw.rotate_about(east, pose.tilt_x, origin);
        draw.rotate_about(south, pose.tilt_z, origin);
        draw.translate([0.0, 1.0 / 16.0, 0.0]);
        draw.scale([0.995; 3]);
        draw.translate([-0.5; 3]);
        host.draw_model(draw, BELL, 0xFF_FFFF);
        draw.pop();
    }

    /// Prunes the bells that stopped ticking.
    fn server_tick(&self, _: &mut dyn Host, event: &mut ServerTickEvent<'_>) {
        if let ServerTickPhase::ServerTickStart { world_running: true } = event.phase {
            let now = NOW.fetch_add(1, Ordering::Relaxed) + 1;
            if let Ok(mut table) = SIGNALS.lock() {
                table.retain(|_, (_, seen)| now - *seen < MAX_AGE);
            }
        }
    }

    /// `DeskBellBlock.getStateForPlacement`: facing out of the face clicked.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != ID {
            return None;
        }
        level
            .state_with_property(event.state, "facing", util::dir(context.face).name())
            .map(Some)
    }

    /// `DeskBellBlock.getSignal` and `getDirectSignal`.
    fn block_signal(&self, level: &dyn LevelHost, event: &mut BlockSignalEvent) {
        let key = (
            signal::level_index(event.dimension),
            [event.pos.x, event.pos.y, event.pos.z],
        );
        let powered = SIGNALS
            .lock()
            .ok()
            .and_then(|table| table.get(&key).map(|&(powered, _)| powered));
        if let Some(powered) = powered
            && let Some(facing) = signal::choice(level, event.state, "facing", FACINGS)
                .and_then(Dir::from_name)
        {
            // `event.face` is the side that touches the reader: Java's side is its opposite.
            let towards_support = util::dir(event.face) == facing.opposite();
            event.signal = rules::signal(powered, event.direct, towards_support);
        }
        signal::mask(event);
    }

    /// `DeskBellBlock.isSignalSource`: a block with a bell's `facing` and `powered` (vanilla's
    /// observer and lightning rod have them and are sources already).
    fn is_signal_source(&self, level: &dyn LevelHost, event: &mut IsSignalSourceEvent) {
        if signal::choice(level, event.state, "facing", FACINGS).is_some()
            && signal::boolean(level, event.state, "powered").is_some()
        {
            event.source = true;
        }
    }

    /// `DeskBellBlock.useWithoutItem`: any click that is not a sneaking one with something in the
    /// hands rings it.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR || (player.sneaking && !signal::hands_empty(host)) {
            return Flow::Continue;
        }
        if !util::first_click(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        util::create_sound_at(host, "desk_bell", pos, 1.0, 1.0);
        let now = util::game_time(host);
        host.change_mod_block_entity(pos, &mut |entity| {
            entity.nbt.insert(TIMER.into(), Tag::Int(rules::POWERED_TICKS));
            entity.nbt.insert(SINCE.into(), Tag::Long(now));
        });
        host.block_entity_sync(pos);
        let level = signal::host_level(host);
        publish(level, pos, true);
        host.set_block_property(pos, "powered", "true");
        let dimension = host.dimension();
        host.update_signal_neighbors(dimension, pos);
        Flow::Cancel
    }

    /// `WrenchableDirectionalBlock`: the Wrench turns it, sneaking picks it up.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != ID {
            return Flow::Continue;
        }
        let pos = event.pos;
        let Some(face) = Dir::from_normal(event.face) else {
            return Flow::Continue;
        };
        util::wrench(host, pos, |host| {
            if let Some(facing) = util::facing(host, pos, "facing") {
                let next = rules::rotated(facing, face);
                if next != facing {
                    host.set_block_property(pos, "facing", next.name());
                }
            }
        })
    }
}
