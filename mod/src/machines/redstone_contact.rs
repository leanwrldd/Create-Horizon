//! The Redstone Contact (`RedstoneContactBlock`, `RedstoneContactItem`, `ContactMovementBehaviour`):
//! two facing each other power each other, 15 on every side but the one they face through, and a
//! contact on a structure powers the one it passes (`actor_more.rs` does the moving half through
//! [`hold`] and [`release`]). The cabin of an Elevator Pulley carries one (`elevator.rs`), and the
//! Wrench makes one that faces a column of floors into an Elevator Contact. The rules are
//! `create_kinetics::rules::redstone_contact`.
//!
//! Known differences from Create:
//! - A contact looks at what is in front of it each tick (its block entity polls) rather than on
//!   `updateShape` and a scheduled tick; a contact a moving one passes is held powered until the
//!   moving one leaves it (Create sets the block state and schedules the reset one tick after the
//!   structure moves on).
//! - The item doesn't turn into an Elevator Contact when placed facing a column
//!   (`RedstoneContactItem.getPlacementState`): the Wrench does it, as `onWrenched` does.
//! - A placed contact reads another block with `facing` and `powered` (an observer) as a contact
//!   until its first tick looks at the block itself.
//! - The Elevator's contact lights while it powers in Create; a Redstone Contact's `powered` model
//!   is Create's own (`block_powered`).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{LazyLock, Mutex};

use create_kinetics::rules::contraption::State;
use create_kinetics::rules::elevator;
use create_kinetics::rules::redstone_contact::{self as rules, Front};
use create_kinetics::{Dir, Pos, looking};
use crate::compat::{
    BlockEntityEvent, BlockEntityTickEvent, BlockEntityTicksEvent, BlockSignalEvent, BlockStateId, Compound,
    Flow, Hooks, Host, IsSignalSourceEvent, LevelHost, ServerTickEvent, ServerTickPhase, StateForPlacementEvent,
    UseItemOnEvent,
};

use super::diodes_signal as signal;
use crate::util;

pub const ID: &str = rules::ID;
const MAX_AGE: i64 = 3;
const FACINGS: [&str; 6] = ["down", "up", "north", "south", "west", "east"];

/// A server tick counter, for pruning.
static NOW: AtomicI64 = AtomicI64::new(0);

/// Whether each ticking contact is powered, which way it faces and when it last ticked.
static SIGNALS: LazyLock<Mutex<HashMap<(u8, Pos), (bool, Dir, i64)>>> = LazyLock::new(Default::default);

/// The contacts a moving contact holds powered.
static HELD: LazyLock<Mutex<HashSet<(u8, Pos)>>> = LazyLock::new(Default::default);

struct Contacts;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Contacts)
}

/// A moving contact powers the contact at `pos` and keeps it so (`setBlockAndUpdate(.., POWERED)`).
pub(super) fn hold(level: u8, pos: Pos) {
    if let Ok(mut held) = HELD.lock() {
        held.insert((level, pos));
    }
}

/// The moving contact lets go (`scheduleTick(last, .., 1)`): the contact's own look decides again.
pub(super) fn release(level: u8, pos: Pos) {
    if let Ok(mut held) = HELD.lock() {
        held.remove(&(level, pos));
    }
}

fn is_held(level: u8, pos: Pos) -> bool {
    HELD.lock().is_ok_and(|held| held.contains(&(level, pos)))
}

fn publish(level: u8, pos: Pos, powered: bool, facing: Dir) {
    let now = NOW.load(Ordering::Relaxed);
    if let Ok(mut table) = SIGNALS.lock() {
        table.insert((level, pos), (powered, facing, now));
    }
}

/// What is in front of the contact at `pos` facing `facing`.
fn front_of(host: &dyn Host, pos: Pos, facing: Dir) -> Front {
    let at = util::add(pos, facing.normal());
    let block = host.block(at);
    let is_contact = block == rules::ID || block == rules::ELEVATOR_ID;
    Front { is_contact, facing: if is_contact { util::facing(host, at, "facing") } else { None } }
}

impl Hooks for Contacts {
    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        (event.block == ID).then(|| (ID.to_owned(), Compound::new()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    /// `RedstoneContactBlock.tick` and `updateShape`: `powered` follows the contact in front.
    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        let pos = event.pos;
        let Some(facing) = util::facing(&*host, pos, "facing") else { return };
        let level = signal::host_level(&*host);
        let powered = host.block_property(pos, "powered").as_deref() == Some("true");
        let valid = rules::has_valid_contact(front_of(&*host, pos, facing), facing) || is_held(level, pos);
        let known = SIGNALS.lock().ok().is_some_and(|table| table.contains_key(&(level, pos)));
        publish(level, pos, valid, facing);
        if valid != powered {
            host.set_block_property(pos, "powered", if valid { "true" } else { "false" });
        }
        // A contact that has just loaded, or changed, tells its neighbours what it gives.
        if valid != powered || (!known && valid) {
            let dimension = host.dimension();
            host.update_signal_neighbors(dimension, pos);
        }
    }

    /// Prunes the contacts that stopped ticking (their chunk unloaded, or the block went).
    fn server_tick(&self, _: &mut dyn Host, event: &mut ServerTickEvent<'_>) {
        if let ServerTickPhase::ServerTickStart { world_running: true } = event.phase {
            let now = NOW.fetch_add(1, Ordering::Relaxed) + 1;
            if let Ok(mut table) = SIGNALS.lock() {
                table.retain(|_, (_, _, seen)| now - *seen < MAX_AGE);
            }
        }
    }

    /// `RedstoneContactBlock.getStateForPlacement`.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != ID {
            return None;
        }
        let here = [context.pos.x, context.pos.y, context.pos.z];
        let valid = |direction: Dir| {
            let n = direction.normal();
            let front = level.block_state([here[0] + n[0], here[1] + n[1], here[2] + n[2]]);
            // A contact has `facing` and `powered`; an Elevator Contact `calling` too. The front one
            // must face back at the new contact.
            let is_contact = signal::boolean(level, front, "powered").is_some()
                && signal::choice(level, front, "facing", &FACINGS).is_some();
            let faces_back = level.state_with_property(front, "facing", direction.opposite().name()) == Some(front);
            is_contact && faces_back
        };
        let (facing, powered) = rules::placement(
            looking(context.yaw, context.pitch),
            util::dir(context.face),
            context.sneaking,
            &valid,
        );
        let state = level.state_with_property(event.state, "facing", facing.name())?;
        level
            .state_with_property(state, "powered", if powered { "true" } else { "false" })
            .map(Some)
    }

    /// `RedstoneContactBlock.getSignal`: 15 on the sides it doesn't face through, never as a direct
    /// (strong) signal.
    fn block_signal(&self, _level: &dyn LevelHost, event: &mut BlockSignalEvent) {
        let key = (signal::level_index(event.dimension), [event.pos.x, event.pos.y, event.pos.z]);
        let known = SIGNALS.lock().ok().and_then(|table| table.get(&key).map(|&(powered, facing, _)| (powered, facing)));
        if let Some((powered, facing)) = known {
            // `event.face` is the side that touches the reader: Java's side is its opposite.
            let side = util::dir(event.face).opposite();
            event.signal = if event.direct { 0 } else { rules::signal(powered, facing, side) };
        }
        signal::mask(event);
    }

    /// `RedstoneContactBlock.isSignalSource` (`POWERED`): a block with a contact's states that is
    /// powered (an observer's states are the same and it is a source already: only ever set, never
    /// cleared).
    fn is_signal_source(&self, level: &dyn LevelHost, event: &mut IsSignalSourceEvent) {
        if let Some(powered) = signal::boolean(level, event.state, "powered")
            && signal::choice(level, event.state, "facing", &FACINGS).is_some()
            && rules::is_signal_source(powered)
        {
            event.source = true;
        }
    }

    /// The Wrench turns a contact about the face clicked; one that then faces a column of floors becomes
    /// an Elevator Contact (`RedstoneContactBlock.onWrenched`).
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != ID {
            return Flow::Continue;
        }
        let (pos, clicked) = (event.pos, Dir::from_normal(event.face));
        util::wrench(host, pos, |host| {
            let (Some(facing), Some(clicked)) = (util::facing(host, pos, "facing"), clicked) else {
                return;
            };
            let next = create_kinetics::rules::sticker::wrenched(facing, clicked);
            if next != facing {
                host.set_block_property(pos, "facing", next.name());
            }
            if next.axis() == create_kinetics::Axis::Y {
                return;
            }
            if !super::elevator::column_exists(host.dimension(), elevator::column_of(pos, next)) {
                return;
            }
            let powered = host.block_property(pos, "powered").as_deref() == Some("true");
            let state = State::new(rules::ELEVATOR_ID)
                .with("facing", next.name())
                .with("powered", if powered { "true" } else { "false" })
                .with("calling", "false")
                .with("powering", "false");
            super::contraption::place_state(host, pos, &state);
        })
    }
}
