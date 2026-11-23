//! The Redstone Link in the game (`RedstoneLinkBlock`, `RedstoneLinkBlockEntity`, `LinkBehaviour`,
//! `LinkHandler`, `LinkRenderer`, `RedstoneLinkNetworkHandler`): a transmitter that sends the
//! signal it reads, or a receiver that gives the strongest signal sent to links on the same two
//! items within 256 blocks. The rules are `create_kinetics::rules::redstone_link`; the signal helpers
//! are in `diodes_signal.rs`.
//!
//! Known differences from Create, for the README:
//! - There is no frequency slot box: clicking the front of a link with an item sets the frequency
//!   slot that its upper half (a link on a wall) or its south half (a link lying flat) stands for,
//!   which is the first, and the other half sets the second (the way `LinkHandler` picks a slot for a
//!   fake player: the nearer slot's centre). The click is told where it landed by following the
//!   player's eye ray to the front of the link. An empty hand clears the slot, as in Create. The
//!   action bar names the item. Clicks on the other faces of a link (to put a block beside it)
//!   are left alone.
//! - A transmitter reads `getBestNeighborSignal` (the strongest of its six neighbours); Create also
//!   asks every side again as if from above (`getSignal(pos, UP)`), which lets it hear redstone
//!   wire that doesn't point at it.
//! - Links are tracked while their block entity ticks (`Links::touch`) and pruned a few ticks after
//!   it stops (an unloaded chunk or a broken block), where Create asks `isAlive` on each update.
//! - A receiver's output is kept in a table (`STATE.out`) for the `block_signal` hook, which
//!   can't read block entities; a receiver in a freshly loaded chunk gives 0 until it has ticked once.
//! - A link falls off when the block it is on is air, water, a plant or another replaceable thing
//!   (`RedstoneLinkBlock.canSurvive`: `!neighbour.canBeReplaced()`), checked each tick.
//! - Factory Gauge panels (`FactoryPanelSupportBehaviour`, `panel_support.rs`) are: a transmitter sends
//!   15 while a linked panel is satisfied with a target, a receiver pauses the panel it is linked to
//!   while it receives a signal. The Linked Controller part of Create's link is left out.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{LazyLock, Mutex};

use create_kinetics::rules::redstone_link::{
    Frequency, Key, Links, first_slot_hit, front_hit, horizontal_angle, slot_centre, view_vector,
};
use create_kinetics::{Dir, Pos};
use crate::compat::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, BlockSignalEvent, BlockStateId, Compound, DisplayContext, Flow,
    Hooks, Host, IsSignalSourceEvent, ItemStack, LevelHost, ModBlockEntity, RightClickBlockEvent,
    ServerTickEvent, ServerTickPhase, StateForPlacementEvent, Tag, UseItemOnEvent,
};

use super::diodes_signal as signal;
use super::panel_support;
use crate::{int, util};

pub const ID: &str = "create:redstone_link";

const TRANSMITTER: &str = "Transmitter";
const RECEIVE: &str = "Receive";
const RECEIVED_CHANGED: &str = "ReceivedChanged";
const TRANSMIT: &str = "Transmit";
const FREQUENCY_FIRST: &str = "FrequencyFirst";
const FREQUENCY_LAST: &str = "FrequencyLast";
const LAST_KNOWN_POSITION: &str = "LastKnownPosition";

/// `Level.UPDATE_CLIENTS`.
const UPDATE_CLIENTS: i32 = 2;
/// Ticks a link may miss before it counts as gone.
const MAX_AGE: i64 = 3;
const SPECTATOR: i32 = 3;

/// A server tick counter, for pruning.
static NOW: AtomicI64 = AtomicI64::new(0);

#[derive(Default)]
struct State {
    links: Links,
    /// What each receiver gives (`RedstoneLinkBlockEntity.getReceivedSignal`), for `block_signal`.
    out: HashMap<(u8, Pos), i32>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

struct RedstoneLink;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(RedstoneLink)
}

// --- frequencies --------------------------------------------------------------------------------

/// `Frequency.of`: the item, and the colour of a dyed one.
fn frequency_of(stack: &ItemStack) -> Frequency {
    let colour = match util::stack_tag(stack) {
        Tag::Compound(tag) => match util::get(&tag, "components") {
            Some(Tag::Compound(components)) => match util::get(components, "minecraft:dyed_color") {
                Some(Tag::Int(rgb)) => *rgb,
                Some(Tag::Compound(dyed)) => {
                    if util::has(dyed, "rgb") { int(dyed, "rgb") } else { -1 }
                }
                _ => -1,
            },
            _ => -1,
        },
        _ => -1,
    };
    Frequency::of(&stack.item, colour)
}

/// The stack a frequency slot holds (`LinkBehaviour.read`: `ItemStack.parseOptional`).
fn slot_stack(nbt: &Compound, key: &str) -> Option<ItemStack> {
    util::get(nbt, key).and_then(util::stack_of)
}

fn slot_frequency(nbt: &Compound, key: &str) -> Frequency {
    slot_stack(nbt, key)
        .map(|stack| frequency_of(&stack))
        .unwrap_or_else(Frequency::empty)
}

fn key_of(nbt: &Compound) -> Key {
    Key::new(
        slot_frequency(nbt, FREQUENCY_FIRST),
        slot_frequency(nbt, FREQUENCY_LAST),
    )
}

/// `ItemStack.saveOptional`: the stack, or an empty compound.
fn slot_tag(stack: Option<&ItemStack>) -> Tag {
    match stack {
        Some(stack) => util::stack_tag(stack),
        None => Tag::Compound(Compound::new()),
    }
}

/// `BlockPos.asLong`.
fn pack(pos: Pos) -> i64 {
    (i64::from(pos[0]) & 0x3FF_FFFF) << 38 | (i64::from(pos[2]) & 0x3FF_FFFF) << 12 | (i64::from(pos[1]) & 0xFFF)
}

// --- the block as a level hook sees it ----------------------------------------------------------

/// A link's state: whether it receives, whether it is powered, and where it faces.
#[derive(Clone, Copy, Debug)]
struct Decoded {
    receiver: bool,
    powered: bool,
    facing: Dir,
}

static DECODED: LazyLock<Mutex<HashMap<u32, Option<Decoded>>>> = LazyLock::new(Default::default);

fn decode(level: &dyn LevelHost, state: BlockStateId) -> Option<Decoded> {
    if let Ok(cache) = DECODED.lock()
        && let Some(known) = cache.get(&state.0)
    {
        return *known;
    }
    let decoded = (|| {
        let receiver = signal::boolean(level, state, "receiver")?;
        let powered = signal::boolean(level, state, "powered")?;
        let facing = signal::choice(
            level,
            state,
            "facing",
            &["north", "south", "west", "east", "up", "down"],
        )
        .and_then(Dir::from_name)?;
        Some(Decoded { receiver, powered, facing })
    })();
    if let Ok(mut cache) = DECODED.lock() {
        cache.insert(state.0, decoded);
    }
    decoded
}

// --- the block entity ---------------------------------------------------------------------------

fn new_entity() -> Compound {
    let mut nbt = Compound::new();
    nbt.insert(TRANSMITTER.into(), Tag::Byte(0));
    nbt.insert(RECEIVE.into(), Tag::Int(0));
    nbt.insert(RECEIVED_CHANGED.into(), Tag::Byte(0));
    nbt.insert(TRANSMIT.into(), Tag::Int(0));
    nbt.insert(FREQUENCY_FIRST.into(), slot_tag(None));
    nbt.insert(FREQUENCY_LAST.into(), slot_tag(None));
    nbt
}

fn flag(host: &dyn Host, pos: Pos, key: &str) -> bool {
    host.block_property(pos, key).as_deref() == Some("true")
}

fn tick(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity) {
    let dimension = host.dimension();
    let level = signal::host_level(host);
    let now = NOW.load(Ordering::Relaxed);
    let Some(facing) = util::facing(host, pos, "facing") else {
        return;
    };
    // `RedstoneLinkBlock.canSurvive`.
    let attached = util::add(pos, facing.opposite().normal());
    if host.chunk_is_loaded(dimension, [pos[0] >> 4, pos[2] >> 4])
        && signal::replaceable(&host.block(attached))
    {
        host.destroy_block(pos, None, false);
        return;
    }
    let receiver = flag(host, pos, "receiver");
    let powered = flag(host, pos, "powered");
    let transmitter = !receiver;
    let key = key_of(&entity.nbt);
    let saved_transmit = int(&entity.nbt, TRANSMIT);
    let saved_receive = int(&entity.nbt, RECEIVE);

    // The entity's own fields, brought in line (`RedstoneLinkBlockEntity.tick`).
    let mut stored = Vec::new();
    if int(&entity.nbt, TRANSMITTER) != i32::from(transmitter) {
        stored.push((TRANSMITTER, Tag::Byte(i8::from(transmitter))));
    }
    if !util::has(&entity.nbt, LAST_KNOWN_POSITION) {
        stored.push((LAST_KNOWN_POSITION, Tag::Long(pack(pos))));
    }

    // `LinkBehaviour.initialize`/`unload`, `setFrequency`, and the swap of behaviours when the mode
    // changes: a link that is registered as something else is taken out and put in again.
    {
        let Ok(mut state) = STATE.lock() else {
            return;
        };
        let registered = state.links.registered(level, pos).cloned();
        let up_to_date = registered
            .as_ref()
            .is_some_and(|r| r.key == key && r.listening == receiver);
        if !up_to_date {
            if let Some(old) = registered {
                state.links.remove(level, &old.key, pos);
            }
            let transmit = if transmitter { saved_transmit } else { 0 };
            state.links.add(level, &key, pos, receiver, transmit, now);
        }
        state.links.touch(level, &key, pos, now);
    }

    if transmitter {
        // `RedstoneLinkBlock.updateTransmittedSignal`: the signal beside it is what it transmits,
        // and its `powered` state follows.
        // `RedstoneLinkBlock.neighborChanged` calls it; the signal is the one the neighbour event left (`signals`).
        let neighbour = crate::signals::read(host, &mut entity.nbt, pos);
        // `FactoryPanelSupportBehaviour.shouldBePoweredTristate`: a satisfied linked panel sends 15; while one is in an unloaded chunk the
        // update is suppressed and the link keeps what it sent.
        let from_panels = panel_support::powered_by_panels(&*host, &mut entity.nbt);
        if let Some(power) = create_kinetics::rules::factory_gauge::transmitted_power(neighbour, from_panels) {
            if (power > 0) != powered {
                host.set_block_property_with(pos, "powered", if power > 0 { "true" } else { "false" }, UPDATE_CLIENTS);
            }
            if power != saved_transmit {
                stored.push((TRANSMIT, Tag::Int(power)));
                if let Ok(mut state) = STATE.lock() {
                    state.links.set_transmit(level, &key, pos, power);
                }
            }
        }
        if let Ok(mut state) = STATE.lock() {
            state.out.remove(&(level, pos));
        }
    } else {
        let received = STATE
            .lock()
            .ok()
            .and_then(|state| state.links.received(level, &key, pos))
            .unwrap_or(saved_receive);
        if let Ok(mut state) = STATE.lock() {
            state.out.insert((level, pos), received);
        }
        let mut changed = int(&entity.nbt, RECEIVED_CHANGED) != 0;
        if received != saved_receive {
            stored.push((RECEIVE, Tag::Int(received)));
            changed = true;
        }
        if (received > 0) != powered {
            host.set_block_property(pos, "powered", if received > 0 { "true" } else { "false" });
            changed = true;
        }
        if changed {
            // `updateSelfAndAttached`: the link and the block it is on are told.
            host.update_signal_neighbors(dimension, pos);
            if int(&entity.nbt, RECEIVED_CHANGED) != 0 {
                stored.push((RECEIVED_CHANGED, Tag::Byte(0)));
            }
        }
    }
    for (key, value) in stored {
        entity.nbt.insert(key.into(), value);
    }
}

// --- the hooks ----------------------------------------------------------------------------------

impl Hooks for RedstoneLink {
    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == ID).then(|| (ID.to_owned(), new_entity()))
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

    /// Prunes the links that stopped ticking.
    fn server_tick(&self, _: &mut dyn Host, event: &mut ServerTickEvent<'_>) {
        if let ServerTickPhase::ServerTickStart { world_running: true } = event.phase {
            let now = NOW.fetch_add(1, Ordering::Relaxed) + 1;
            if let Ok(mut state) = STATE.lock() {
                for gone in state.links.prune(now, MAX_AGE) {
                    state.out.remove(&gone);
                }
            }
        }
    }

    /// `LinkBehaviour.write` to the client: the two frequencies, for the items on the block.
    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        if event.entity.id != ID {
            return None;
        }
        let mut tag = Compound::new();
        for key in [FREQUENCY_FIRST, FREQUENCY_LAST] {
            tag.insert(key.into(), slot_tag(slot_stack(&event.entity.nbt, key).as_ref()));
        }
        Some(tag)
    }

    /// `LinkRenderer.renderOnBlockEntity`: each frequency item on its slot
    /// (`ValueBoxRenderer.renderItemIntoValueBox`: flat items at half size and set back a little).
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let Some(facing) = host
            .block_state_property(event.state, "facing")
            .and_then(Dir::from_name)
        else {
            return;
        };
        let draw = &mut *event.draw;
        for (first, key) in [(true, FREQUENCY_FIRST), (false, FREQUENCY_LAST)] {
            let Some(stack) = slot_stack(&event.entity.nbt, key) else {
                continue;
            };
            let at = slot_centre(facing.normal(), first);
            // `RedstoneLinkFrequencySlot.rotate`.
            let (y_turn, x_turn) = if facing.normal()[1] == 0 {
                (horizontal_angle(facing.normal()) as f32 + 180.0, 0.0)
            } else {
                (0.0, if facing == Dir::Up { 90.0 } else { 270.0 })
            };
            let block_item = host.default_state(&stack.item).is_some();
            let set_back = if block_item { 0.0 } else { -0.15 };
            draw.push();
            draw.translate([at[0] as f32, at[1] as f32, at[2] as f32]);
            draw.rotate_about([0.0, 1.0, 0.0], y_turn, [0.0; 3]);
            draw.rotate_about([1.0, 0.0, 0.0], x_turn, [0.0; 3]);
            draw.scale([0.4975; 3]);
            // `renderItemIntoValueBox`: `(!blockItem ? .5f : 1f) + 1 / 64f`.
            let item_scale = if block_item { 1.0 } else { 0.5 } + 1.0 / 64.0;
            draw.scale([item_scale; 3]);
            draw.translate([0.0, 0.0, set_back]);
            host.draw_item(draw, &stack, DisplayContext::Fixed, None, 0);
            draw.pop();
        }
        draw.keep();
    }

    /// `RedstoneLinkBlock.getStateForPlacement`: facing out of the face clicked.
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

    fn block_signal(&self, level: &dyn LevelHost, event: &mut BlockSignalEvent) {
        if let Some(link) = decode(level, event.state) {
            let level_index = signal::level_index(event.dimension);
            let pos = [event.pos.x, event.pos.y, event.pos.z];
            event.signal = if link.receiver {
                let received = STATE
                    .lock()
                    .ok()
                    .and_then(|state| state.out.get(&(level_index, pos)).copied())
                    .unwrap_or(0);
                // `RedstoneLinkBlock.getSignal` is the received signal on every side;
                // `getDirectSignal` only into the block the link is on.
                if !event.direct || event.face == util::direction(link.facing.opposite()) {
                    received
                } else {
                    0
                }
            } else {
                0
            };
        }
        signal::mask(event);
    }

    /// `RedstoneLinkBlock.isSignalSource`: a powered receiver.
    fn is_signal_source(&self, level: &dyn LevelHost, event: &mut IsSignalSourceEvent) {
        if let Some(link) = decode(level, event.state)
            && link.receiver
            && link.powered
        {
            event.source = true;
        }
    }

    /// `RedstoneLinkBlock.useWithoutItem` and `LinkHandler.onBlockActivated`: a sneak-click with
    /// empty hands switches between transmitting and receiving; a click on the front of the link
    /// sets the frequency slot it lands on to the item in hand (or clears it).
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.sneaking {
            if !signal::hands_empty(host) {
                return Flow::Continue;
            }
            // Once a click: the off hand's turn would switch it back.
            if util::first_click(host, player.uuid, pos) {
                toggle_mode(host, pos);
            }
            return Flow::Cancel;
        }
        // `LinkHandler`: not for spectators or while holding the Wrench (which toggles).
        if player.game_mode == SPECTATOR || signal::holds_wrench(host) {
            return Flow::Continue;
        }
        let Some(facing) = util::facing(host, pos, "facing") else {
            return Flow::Continue;
        };
        if Dir::from_normal(event.face) != Some(facing) {
            return Flow::Continue;
        }
        let eye = [
            player.position[0],
            player.position[1] + f64::from(player.eye_height),
            player.position[2],
        ];
        let Some(local) = front_hit(
            facing.normal(),
            pos,
            eye,
            view_vector(player.yaw, player.pitch),
        ) else {
            return Flow::Continue;
        };
        let first = first_slot_hit(facing.normal(), local);
        let held = host.held_stack(player.uuid).map(|mut stack| {
            stack.count = 1;
            stack
        });
        if util::first_click(host, player.uuid, pos) {
            set_frequency(host, pos, first, held);
        }
        Flow::Cancel
    }

    /// The Wrench: `RedstoneLinkBlock.onWrenched` toggles the mode. Sneaking picks the block up.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != ID {
            return Flow::Continue;
        }
        let pos = event.pos;
        util::wrench(host, pos, |host| toggle_mode(host, pos))
    }
}

/// `RedstoneLinkBlock.toggleMode`: the other mode, powered as the signal beside it says.
fn toggle_mode(host: &mut dyn Host, pos: Pos) {
    let receiver = flag(host, pos, "receiver");
    host.set_block_property(pos, "receiver", if receiver { "false" } else { "true" });
    let powered = host.neighbor_signal(host.dimension(), pos) > 0;
    host.set_block_property(pos, "powered", if powered { "true" } else { "false" });
}

/// `LinkBehaviour.setFrequency`: puts `stack` (one of it) in a slot, or empties it; the link joins
/// the network of its new pair of items on its next tick.
fn set_frequency(host: &mut dyn Host, pos: Pos, first: bool, stack: Option<ItemStack>) {
    let key = if first { FREQUENCY_FIRST } else { FREQUENCY_LAST };
    let mut changed = false;
    let wanted = slot_tag(stack.as_ref());
    host.change_mod_block_entity(pos, &mut |entity| {
        let have = slot_stack(&entity.nbt, key);
        let same = match (&have, &stack) {
            (Some(a), Some(b)) => util::same_item(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            entity.nbt.insert(key.into(), wanted.clone());
            changed = true;
        }
    });
    if changed {
        host.block_entity_sync(pos);
    }
    signal::sound(host, "minecraft:entity.item_frame.add_item", pos, 0.25, 0.1);
    let which = if first { "Frequency #1" } else { "Frequency #2" };
    let line = match &stack {
        Some(stack) => format!("{which}: {}", stack.item),
        None => format!("{which}: cleared"),
    };
    signal::tell(host, &line);
}

// --- the Linked Controller (`machines/linked_controller.rs`) ------------------------------------

/// `LinkedControllerItem.toFrequency`: the network key of two frequency items.
pub(crate) fn controller_key(first: Option<&ItemStack>, second: Option<&ItemStack>) -> Key {
    Key::new(
        first.map_or_else(Frequency::empty, frequency_of),
        second.map_or_else(Frequency::empty, frequency_of),
    )
}

/// `LinkedControllerServerHandler.ManualFrequencyEntry` joining the network: a transmitter of 15
/// where the player is.
pub(crate) fn controller_add(level: u8, key: &Key, pos: Pos) {
    let now = NOW.load(Ordering::Relaxed);
    if let Ok(mut state) = STATE.lock() {
        state.links.add(level, key, pos, false, 15, now);
    }
}

/// The entry is still alive this tick (a link not heard from for a few ticks is pruned).
pub(crate) fn controller_touch(level: u8, key: &Key, pos: Pos) {
    let now = NOW.load(Ordering::Relaxed);
    if let Ok(mut state) = STATE.lock() {
        state.links.touch(level, key, pos, now);
    }
}

pub(crate) fn controller_remove(level: u8, key: &Key, pos: Pos) {
    if let Ok(mut state) = STATE.lock() {
        state.links.remove(level, key, pos);
    }
}

/// `LinkedControllerBindPacket`: the two frequency items a link is set to.
pub(crate) fn link_stacks(nbt: &Compound) -> [Option<ItemStack>; 2] {
    [slot_stack(nbt, FREQUENCY_FIRST), slot_stack(nbt, FREQUENCY_LAST)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machines::diodes_signal::fake::Registry;

    const BOOL: &[&str] = &["false", "true"];

    #[test]
    fn a_state_is_read_as_a_link() {
        let mut level = Registry::default();
        level.block(&[
            ("facing", &["north", "south", "east", "west", "up", "down"]),
            ("powered", BOOL),
            ("receiver", BOOL),
        ]);
        level.block(&[
            ("facing", &["north", "south", "west", "east"]),
            ("powered", BOOL),
            ("powering", BOOL),
        ]);
        let link = level.find(&[("facing", "down"), ("powered", "true"), ("receiver", "true")]);
        let d = decode(&level, link).expect("a link");
        assert_eq!((d.receiver, d.powered, d.facing), (true, true, Dir::Down));
        let diode = level.find(&[("facing", "north"), ("powered", "true"), ("powering", "true")]);
        assert!(decode(&level, diode).is_none());
    }

    #[test]
    fn frequencies_are_the_item_and_its_dye() {
        let plain = ItemStack { item: "minecraft:redstone".into(), count: 3, damage: 0, extra: None };
        assert_eq!(frequency_of(&plain), Frequency::of("minecraft:redstone", -1));
        assert!(slot_frequency(&Compound::new(), FREQUENCY_FIRST).is_empty());
        let mut nbt = Compound::new();
        nbt.insert(FREQUENCY_FIRST.into(), slot_tag(Some(&plain)));
        assert_eq!(slot_frequency(&nbt, FREQUENCY_FIRST), Frequency::of("minecraft:redstone", -1));
        assert_eq!(
            key_of(&nbt),
            Key::new(Frequency::of("minecraft:redstone", -1), Frequency::empty())
        );
        assert_eq!(pack([1, 2, 3]), (1i64 << 38) | (3i64 << 12) | 2);
        assert_eq!(pack([-1, 0, -1]), (0x3FF_FFFFi64 << 38) | (0x3FF_FFFFi64 << 12));
    }
}
