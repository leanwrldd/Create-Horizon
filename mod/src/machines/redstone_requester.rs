//! The Redstone Requester (`create:redstone_requester`) (`RedstoneRequesterBlock`, `RedstoneRequesterBlockEntity`,
//! `RedstoneRequesterBlockItem`, `AutoRequestData`, `RedstoneRequesterConfigurationPacket`): a block that, when a redstone
//! signal reaches it, orders what it was programmed with from a logistics network. The rules are `rules::redstone_requester`; the
//! network is `logistics.rs`; this is the block and the commands that stand in for its screen.
//!
//! How it works here:
//! - Program an order at a Stock Ticker (`/stock add ...` then `/stock program`), then place a Redstone Requester: it takes that order,
//!   its address, and the ticker's network (`AutoRequestData.readFromItem`: the ticker must be within 128 blocks, in the same
//!   dimension, or the requester says so and is left empty). A requester placed without one starts a network of its own and holds no
//!   order.
//! - A rising signal sends the order once (`onRedstonePowerChanged`); unless partial requests are allowed it sends nothing when the
//!   network holds less than any one stack. The comparator reads 15 after a request that was sent.
//! - Right-click it (an empty hand) to open it, then `/requester set <item> <count>`, `remove <item>`, `clear`, `address <text>` and
//!   `partial <true|false>` (`RedstoneRequesterScreen`: nine ghost slots, the address box, the partial switch).
//!
//! Known differences from Create:
//! - The configuration screen and the particles are not ported; the sounds are.
//! - The order and address are written from a player's memory, not carried in the item (see `logistics.rs`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

use create_kinetics::Pos;
use create_kinetics::rules::filter::display_name;
use create_kinetics::rules::packager::RequestType;
use create_kinetics::rules::redstone_requester::{AutoRequestData, Message, Requester, Trigger};
use create_kinetics::rules::stock_commands::{self as commands, Requester as Command};
use crate::compat::{
    AnalogOutputEvent, BlockEntityEvent, BlockEntityTickEvent, BlockEntityTicksEvent, BlockStateId, Compound, DestroyBlockEvent, Flow,
    HasAnalogOutputEvent, Hooks, Host, ItemStack, LevelHost, NativeCommand, PerformCommandEvent, PlaceBlockEvent,
    RegisterCommandsEvent, RightClickBlockEvent, StateForPlacementEvent, Tag, Text, UseItemOnEvent,
};

use super::diodes_signal as signal;
use super::logistics as network;
use super::package as items;
use crate::util;

const ID: &str = network::REDSTONE_REQUESTER;
/// `RedstoneRequesterBlockEntity.write`.
const POWERED: &str = "Powered";
const SUCCESS: &str = "Success";
const ALLOW_PARTIAL: &str = "AllowPartial";
const ADDRESS: &str = "EncodedAddress";
const REQUEST: &str = "EncodedRequest";

struct Requesters;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Requesters)
}

/// The order a requester was placed with, until its block entity ticks and takes it.
static INITIAL: LazyLock<Mutex<BTreeMap<(String, Pos), AutoRequestData<ItemStack>>>> = LazyLock::new(Mutex::default);
/// The requester each player has open, for `/requester`.
static OPEN: LazyLock<Mutex<BTreeMap<u128, Pos>>> = LazyLock::new(Mutex::default);
/// The requesters whose last request went out (`lastRequestSucceeded`), for the comparator.
static SUCCEEDED: LazyLock<Mutex<BTreeSet<(u8, Pos)>>> = LazyLock::new(Mutex::default);

fn lock<T>(mutex: &'static Mutex<T>) -> MutexGuard<'static, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn tell(host: &mut dyn Host, uuid: u128, line: impl Into<String>) {
    host.tell(uuid, Text::literal(line.into()));
}

// --- the saved form -----------------------------------------------------------------------------------

/// `RedstoneRequesterBlockEntity.read`.
fn read(nbt: &Compound) -> Requester<ItemStack> {
    Requester {
        allow_partial_requests: util::boolean(nbt, ALLOW_PARTIAL),
        encoded_request: util::get(nbt, REQUEST).and_then(items::context_of).unwrap_or_default(),
        encoded_target_address: match util::get(nbt, ADDRESS) {
            Some(Tag::String(text)) => text.to_string_lossy(),
            _ => String::new(),
        },
        last_request_succeeded: util::boolean(nbt, SUCCESS),
        redstone_powered: util::boolean(nbt, POWERED),
    }
}

/// `RedstoneRequesterBlockEntity.write`, the entries that differ from `nbt`.
fn diff(nbt: &Compound, state: &Requester<ItemStack>) -> Vec<(&'static str, Tag)> {
    let mut wanted = vec![
        (POWERED, Tag::Byte(i8::from(state.redstone_powered))),
        (SUCCESS, Tag::Byte(i8::from(state.last_request_succeeded))),
        (ALLOW_PARTIAL, Tag::Byte(i8::from(state.allow_partial_requests))),
        (ADDRESS, Tag::String(state.encoded_target_address.as_str().into())),
        (REQUEST, items::context_tag(&state.encoded_request)),
    ];
    wanted.retain(|(key, value)| util::differs(nbt, key, value));
    wanted
}

fn store(host: &mut dyn Host, pos: Pos, state: &Requester<ItemStack>) {
    host.change_mod_block_entity(pos, &mut |entity| {
        for (key, value) in diff(&entity.nbt, state) {
            util::put(&mut entity.nbt, key, value);
        }
    });
}

fn is_requester_state(level: &dyn LevelHost, state: BlockStateId) -> bool {
    signal::takes(level, state, "axis", "x")
        && signal::takes(level, state, "axis", "z")
        && !signal::takes(level, state, "axis", "y")
        && signal::takes(level, state, "powered", "true")
}

fn describe(state: &Requester<ItemStack>) -> String {
    let order: Vec<String> =
        state.encoded_request.stacks.iter().map(|(key, count)| format!("{} x{count}", display_name(&key.item))).collect();
    format!(
        "Redstone Requester: {} | address: {} | partial requests: {}",
        if order.is_empty() { "no order".to_owned() } else { order.join(", ") },
        if state.encoded_target_address.is_empty() { "none" } else { state.encoded_target_address.as_str() },
        state.allow_partial_requests
    )
}

// --- the commands -----------------------------------------------------------------------------------------

/// One `/requester` change (`rules::stock_commands::configure`).
fn configure(state: &mut Requester<ItemStack>, command: Command) -> Result<(), String> {
    commands::configure(
        state,
        command,
        &|id| ItemStack { item: id.into(), count: 1, damage: 0, extra: None },
        &|key| key.item.to_string(),
    )
}

fn run(host: &mut dyn Host, uuid: u128, line: &str) {
    let command = match commands::parse_requester(line) {
        Ok(command) => command,
        Err(message) => {
            tell(host, uuid, message);
            return;
        }
    };
    let Some(pos) = lock(&OPEN).get(&uuid).copied() else {
        tell(host, uuid, "Right-click a Redstone Requester first");
        return;
    };
    if host.block(pos) != ID {
        lock(&OPEN).remove(&uuid);
        tell(host, uuid, "That Redstone Requester is gone");
        return;
    }
    let Some(entity) = host.mod_block_entity(pos) else {
        return;
    };
    if let Some(freq) = network::freq_of(&entity.nbt)
        && !network::may_interact(host, freq, uuid)
    {
        return;
    }
    let mut state = read(&entity.nbt);
    match configure(&mut state, command) {
        Ok(()) => {
            store(host, pos, &state);
            host.block_entity_sync(pos);
            tell(host, uuid, describe(&state));
        }
        Err(message) => tell(host, uuid, message),
    }
}

// --- the hooks ---------------------------------------------------------------------------------------------

impl Hooks for Requesters {
    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        (event.block == ID).then(|| (event.block.to_owned(), Compound::new()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    /// `RedstoneRequesterBlock.getStateForPlacement`: the axis the player looks along.
    fn state_for_placement(&self, level: &dyn LevelHost, event: &mut StateForPlacementEvent<'_>) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != ID {
            return None;
        }
        let axis = create_kinetics::rules::deployer::horizontal_direction(context.yaw).axis();
        Some(level.state_with_property(event.state, "axis", axis.name()))
    }

    /// `RedstoneRequesterBlock.setPlacedBy`: the programmed order, if the player had one, and `POWERED` from the neighbour signal.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if &*event.stack.item != ID {
            return;
        }
        let pos = event.pos;
        let dimension = host.dimension();
        if host.neighbor_signal(dimension, pos) > 0 {
            host.set_block_property(pos, "powered", "true");
        }
        // `AutoRequestData.readFromItem`: the order the item carries.
        let programmed = host.acting_player().and_then(|player| network::stack_request(event.stack).map(|data| (player.uuid, data)));
        network::note_placed(host, pos, event.stack);
        if let Some((uuid, carried)) = programmed {
            let (data, message) = carried.read_from_item(dimension, pos);
            // `AutoRequestData.readFromItem`: a ticker out of reach leaves the requester empty.
            if data.is_valid {
                lock(&INITIAL).insert((dimension.to_owned(), pos), data);
            }
            tell(
                host,
                uuid,
                match message {
                    Message::KeeperConnected => "Redstone Requester connected to its Stock Ticker",
                    Message::KeeperTooFarAway => "The Stock Ticker is too far away (128 blocks, same dimension)",
                },
            );
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        let pos = event.pos;
        let dimension = host.dimension();
        let now = util::game_time(&*host);
        let mut state = read(&event.entity.nbt);
        // The order it was placed with.
        let initial = lock(&INITIAL).remove(&(dimension.to_owned(), pos));
        let mut changed = false;
        if let Some(data) = initial {
            state = Requester::from_item(&data);
            changed = true;
        }
        let changes = network::tick_link(host, pos, &event.entity.nbt, false, 0, now);
        for (key, value) in changes {
            if util::differs(&event.entity.nbt, key, &value) {
                util::put(&mut event.entity.nbt, key, value);
            }
        }
        let freq = network::freq_of(&event.entity.nbt);
        // `RedstoneRequesterBlock.neighborChanged`: the signal the neighbour event left (`signals`).
        let signal_now = crate::signals::read(host, &mut event.entity.nbt, pos) > 0;
        if let Some(freq) = freq {
            let change = state.on_redstone_power_changed(signal_now, || network::summary(host, freq, true));
            if change.changed {
                changed = true;
                // `pLevel.setBlockAndUpdate(pPos, pState.setValue(POWERED, hasNeighborSignal))`.
                host.set_block_property(pos, "powered", if signal_now { "true" } else { "false" });
            }
            match change.trigger {
                Trigger::Nothing => {}
                Trigger::Denied => {
                    host.play_sound("create:deny", util::centre(pos), 2, 0.5, 1.0);
                }
                Trigger::Send { order, address, success_effect } => {
                    network::broadcast(host, freq, RequestType::Redstone, &order, &address);
                    if success_effect {
                        host.play_sound("create:confirm", util::centre(pos), 2, 0.5, 1.5);
                        host.play_sound("create:stock_link", util::centre(pos), 2, 1.0, 1.0);
                    } else {
                        host.play_sound("create:deny", util::centre(pos), 2, 0.5, 1.0);
                    }
                }
            }
        }
        if changed || !diff(&event.entity.nbt, &state).is_empty() {
            for (key, value) in diff(&event.entity.nbt, &state) {
                util::put(&mut event.entity.nbt, key, value);
            }
            host.block_entity_sync(pos);
        }
        // `getAnalogOutputSignal`: published for the comparator.
        let level = signal::host_level(&*host);
        let mut table = lock(&SUCCEEDED);
        if state.last_request_succeeded {
            table.insert((level, pos));
        } else {
            table.remove(&(level, pos));
        }
    }

    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) == ID
            && let Some(entity) = host.mod_block_entity(pos)
        {
            network::link_removed(host, pos, &entity.nbt, false);
        }
        let level = signal::host_level(&*host);
        lock(&SUCCEEDED).remove(&(level, pos));
        Flow::Continue
    }

    fn has_analog_output(&self, level: &dyn LevelHost, event: &mut HasAnalogOutputEvent) {
        if is_requester_state(level, event.state) {
            event.output = true;
        }
    }

    /// `RedstoneRequesterBlock.getAnalogOutputSignal`: 15 when the last request went out.
    fn analog_output(&self, level: &dyn LevelHost, event: &mut AnalogOutputEvent) {
        if !is_requester_state(level, event.state) {
            return;
        }
        let at = [event.pos.x, event.pos.y, event.pos.z];
        if lock(&SUCCEEDED).contains(&(signal::level_index(event.dimension), at)) {
            event.output = 15;
        }
    }

    /// `RedstoneRequesterBlockEntity.use`: an empty hand opens it.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.sneaking || player.game_mode == util::SPECTATOR || host.held_stack(player.uuid).is_some() {
            return Flow::Continue;
        }
        if !util::first_click(&*host, player.uuid, pos) {
            return Flow::Cancel;
        }
        self.open(host, player.uuid, pos);
        Flow::Cancel
    }

    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        if &*event.stack.item == util::WRENCH {
            return util::wrench(host, pos, |_| {});
        }
        Flow::Continue
    }

    fn register_commands(&self, _: &mut dyn Host, event: &mut RegisterCommandsEvent<'_>) {
        event.commands.push(NativeCommand::new("requester"));
        event.commands.push(NativeCommand::new("requester <text:text>"));
    }

    fn perform_command(&self, host: &mut dyn Host, event: &mut PerformCommandEvent<'_>) -> Flow {
        let line = event.command.trim();
        let Some(rest) = (if line == "requester" { Some("") } else { line.strip_prefix("requester ") }) else {
            return Flow::Continue;
        };
        run(host, event.player.uuid, rest);
        Flow::Cancel
    }
}

impl Requesters {
    fn open(&self, host: &mut dyn Host, uuid: u128, pos: Pos) {
        let Some(entity) = host.mod_block_entity(pos) else {
            return;
        };
        if let Some(freq) = network::freq_of(&entity.nbt)
            && !network::may_interact(host, freq, uuid)
        {
            return;
        }
        lock(&OPEN).insert(uuid, pos);
        tell(host, uuid, describe(&read(&entity.nbt)));
        tell(host, uuid, "/requester set <item> <count>, remove <item>, clear, address <text>, partial <true|false>");
    }
}
