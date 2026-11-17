//! The Linked Controller (`LinkedControllerItem`, `LinkedControllerClientHandler`,
//! `LinkedControllerServerHandler`, `LinkedControllerBindPacket`, `LinkedControllerInputPacket`):
//! six keys, each bound to a pair of Redstone Link frequencies, that transmit while held. Right-click it
//! to take over the movement keys, right-click a Redstone Link with it and press one of them to bind that
//! link's frequencies to the key. The modes, the messages and the entries are
//! `create_kinetics::rules::linked_controller`; this is the glue.
//!
//! Known differences from Create, for the README:
//! - The Lectern Controller (`lectern_controller.rs`) uses this module's client and server halves: its messages carry
//!   the lectern's position, and the keys then belong to the controller on that lectern, not the one in the hand.
//! - **No manual frequency screen** (`LinkedControllerMenu`, a sneaking right click): frequencies are
//!   set only by binding. Nothing happens on a sneaking right click.
//! - The keys are the default bindings (W, S, A, D, Space, Shift): Horizon gives a mod no way to read
//!   the player's own, so a rebound movement key doesn't work. Bind mode shows its hint as a line on
//!   the HUD, not as Create's tooltip box; no controller click sound; the item model doesn't show
//!   its buttons moving (`LinkedControllerItemRenderer`).
//! - The player's position goes to the link network as the block they stand in at each key
//!   message, as `player.blockPosition()` does. Entries are kept by the controller's own frequency
//!   pair, so two keys bound to the same pair share one entry.
//! - A bind (the Redstone Link's two frequency items copied to the key) is written to the held
//!   controller on the next inventory tick: Horizon can't change a held stack from a message.
//! - The player's own mouse button decides: a right click while the controller is in the main hand
//!   and no screen is open, so it also toggles when a block's own use takes the click.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use create_kinetics::rules::linked_controller::{self as rules, Client, Effect, Message, Mode, Server};
use create_kinetics::rules::redstone_link::Key;
use crate::compat::{
    Compound, ComponentTypesEvent, EquipmentSlot, Flow, Hooks, Host, HudEvent, InventoryTickEvent,
    ItemStack, KeyEvent, MovementInputEvent, NativeScreenKind, NetFromClientEvent, PlayerTickEvent,
    Tag, Text, UseItemOnEvent,
};

use super::diodes_signal as signal;
use super::lectern_controller;
use super::redstone_link as link;
use crate::util;

const LINK: &str = "create:redstone_link";
/// The lectern blocks a click with the controller in hand is not a toggle on.
const LECTERN: &str = "minecraft:lectern";
const LECTERN_CONTROLLER: &str = "create:lectern_controller";
/// How far a player reaches for a link (the block interaction range).
const REACH: f32 = 4.5;

struct Controller;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Controller)
}

// ---- the item's frequencies (`LinkedControllerItem.getFrequencyItems`) --------------------------

fn is_controller(stack: &ItemStack) -> bool {
    &*stack.item == rules::CONTROLLER
}

fn items_of(stack: &ItemStack) -> Vec<Option<ItemStack>> {
    match stack.data().and_then(|d| d.other(rules::COMPONENT)) {
        Some(Tag::Compound(tag)) => items_of_tag(tag),
        _ => vec![None; rules::FREQUENCY_SLOTS],
    }
}

/// The frequency items in the layout the component saves (`Items`, each with a `Slot`), as a Lectern Controller
/// keeps them under `ControllerData`.
pub(super) fn items_of_tag(tag: &Compound) -> Vec<Option<ItemStack>> {
    let mut slots: Vec<Option<ItemStack>> = vec![None; rules::FREQUENCY_SLOTS];
    if let Some(Tag::List(items)) = util::get(tag, "Items") {
        for item in items {
            if let Some(Tag::Byte(slot)) = item.get("Slot")
                && let Some(entry) = slots.get_mut(*slot as usize)
            {
                *entry = util::stack_of(item);
            }
        }
    }
    slots
}

fn tag_of_items(slots: &[Option<ItemStack>]) -> Compound {
    let mut items = Vec::new();
    for (slot, stack) in slots.iter().enumerate() {
        if let Some(stack) = stack
            && let Tag::Compound(mut tag) = util::stack_tag(stack)
        {
            tag.insert("Slot".into(), Tag::Byte(slot as i8));
            items.push(Tag::Compound(tag));
        }
    }
    let mut tag = Compound::new();
    tag.insert("Items".into(), Tag::List(items));
    tag
}

/// `LinkedControllerItem.toFrequency`.
fn key_of(items: &[Option<ItemStack>], key: usize) -> Key {
    link::controller_key(
        items[rules::frequency_slot(key, true)].as_ref(),
        items[rules::frequency_slot(key, false)].as_ref(),
    )
}

// ---- state kept in memory ----------------------------------------------------------------------

static CLIENT: Mutex<Client> = Mutex::new(Client {
    mode: Mode::Idle,
    down: std::collections::BTreeSet::new(),
    sent: std::collections::BTreeSet::new(),
    cooldown: 0,
});

/// Messages from clients, by player, waiting for that player's tick.
static INBOX: Mutex<BTreeMap<u128, Vec<Message>>> = Mutex::new(BTreeMap::new());
/// Each player's open entries and the level index they are in.
static SERVERS: Mutex<BTreeMap<u128, (Server<Key>, u8)>> = Mutex::new(BTreeMap::new());
/// A bind waiting for the next inventory tick: the key and the link's two frequency items.
static BINDS: Mutex<BTreeMap<u128, (usize, [Option<ItemStack>; 2])>> = Mutex::new(BTreeMap::new());
static BIND_COUNT: AtomicUsize = AtomicUsize::new(0);

fn lock<T>(m: &'static Mutex<T>) -> std::sync::MutexGuard<'static, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// `LinkedControllerServerHandler.receivePressed` for a player's keys, by the controller's frequency items.
fn receive(uuid: u128, level: u8, pos: [i32; 3], items: &[Option<ItemStack>], keys: &[usize], pressed: bool) -> Vec<Effect<Key>> {
    let keyed: Vec<(usize, Key)> = keys.iter().map(|k| (*k, key_of(items, *k))).collect();
    let mut servers = lock(&SERVERS);
    let entry = servers.entry(uuid).or_insert_with(|| (Server::default(), level));
    entry.1 = level;
    entry.0.receive(pos, &keyed, pressed)
}

fn send(host: &mut dyn Host, messages: Vec<Message>) {
    for message in messages {
        let _ = host.net_send_to_server(rules::CHANNEL, &rules::encode(&message));
    }
}

/// The server says this player now uses the lectern at `pos` (`LinkedControllerClientHandler.activateInLectern`).
pub(super) fn lectern_on(pos: [i32; 3]) {
    lock(&CLIENT).activate_in_lectern(pos);
}

/// The server says this player no longer uses their lectern (`deactivateInLectern`).
pub(super) fn lectern_off(host: &mut dyn Host) {
    let messages = lock(&CLIENT).deactivate_in_lectern();
    send(host, messages);
}

fn holds_controller(host: &dyn Host) -> bool {
    [EquipmentSlot::MainHand, EquipmentSlot::OffHand]
        .into_iter()
        .any(|slot| host.equipment(slot).is_some_and(|stack| is_controller(&stack)))
}

fn playing(host: &dyn Host) -> bool {
    host.client_state().is_some_and(|client| client.screen == NativeScreenKind::Playing)
}

fn apply(host: &mut dyn Host, level: u8, effects: Vec<Effect<Key>>) {
    let _ = host;
    for effect in effects {
        match effect {
            Effect::Add { key, pos } => link::controller_add(level, &key, pos),
            Effect::Move { key, from, to } => {
                link::controller_remove(level, &key, from);
                link::controller_add(level, &key, to);
            }
            Effect::Remove { key, pos } => link::controller_remove(level, &key, pos),
        }
    }
}

impl Hooks for Controller {
    fn component_types(&self, event: &mut ComponentTypesEvent<'_>) {
        if !event.types.iter().any(|t| t == rules::COMPONENT) {
            event.types.push(rules::COMPONENT.to_owned());
        }
    }

    // ---- the client --------------------------------------------------------------------------------

    /// The six keys' state, and `LinkedControllerItem.use`/`onItemUseFirst`'s toggle on a right click.
    fn key(&self, host: &mut dyn Host, event: &mut KeyEvent<'_>) {
        if !host.window() {
            return;
        }
        if let Some(slot) = rules::key_slot(event.key) {
            lock(&CLIENT).key(slot, event.pressed);
            return;
        }
        if event.key != "key.mouse.right" || !event.pressed || !playing(&*host) || host.sneaking() {
            return;
        }
        if !host.equipment(EquipmentSlot::MainHand).is_some_and(|stack| is_controller(&stack)) {
            return;
        }
        // On a Redstone Link it is bind mode; on a lectern the controller goes there (or is used there), not toggled;
        // anywhere else it toggles the controller.
        let hit = host.raycast(REACH);
        if hit.as_ref().is_some_and(|hit| hit.block == LECTERN || hit.block == LECTERN_CONTROLLER) {
            return;
        }
        let at_link = hit
            .filter(|hit| hit.block == LINK)
            .map(|hit| [hit.block_x, hit.block_y, hit.block_z]);
        let messages = match at_link {
            Some(pos) => lock(&CLIENT).toggle_bind(pos),
            None => lock(&CLIENT).toggle(),
        };
        send(host, messages);
    }

    fn client_tick_post(&self, host: &mut dyn Host, _: &mut crate::compat::ClientTickPostEvent) {
        if !host.window() || lock(&CLIENT).mode == Mode::Idle {
            return;
        }
        let holding = holds_controller(&*host);
        let blocked = !playing(&*host);
        let messages = lock(&CLIENT).tick(holding, blocked);
        send(host, messages);
    }

    /// `kb.setDown(false)`: the movement keys belong to the controller while it is on.
    fn movement_input(&self, host: &dyn Host, event: &mut MovementInputEvent) {
        if host.window() && lock(&CLIENT).captures_keys() {
            event.forward = 0.0;
            event.strafe = 0.0;
            event.jump = false;
            event.sneak = false;
        }
    }

    /// Bind mode's hint (`renderOverlay`).
    fn hud(&self, host: &mut dyn Host, event: &mut HudEvent<'_>) {
        if !matches!(lock(&CLIENT).mode, Mode::Bind(_)) {
            return;
        }
        let Some(client) = host.client_state() else {
            return;
        };
        if client.screen != NativeScreenKind::Playing || client.gui_hidden {
            return;
        }
        let line = format!("Bind mode: press {} to bind this link's frequency", rules::KEY_LABELS.join(", "));
        let width = host.text_width(&line);
        let (w, h) = (client.gui_size[0] as f32, client.gui_size[1] as f32);
        host.text(event.batch, &line, [(w - width) / 2.0, h - 60.0], 0xFF_AA00, true);
    }

    // ---- the server --------------------------------------------------------------------------------

    /// `LinkedControllerInputPacket` and `LinkedControllerBindPacket` arrive.
    fn net_from_client(&self, _: &mut dyn Host, event: &mut NetFromClientEvent<'_>) {
        if event.channel != rules::CHANNEL {
            return;
        }
        if let Some(message) = rules::decode(event.data) {
            let mut inbox = lock(&INBOX);
            let queue = inbox.entry(event.player).or_default();
            if queue.len() < 64 {
                queue.push(message);
            }
        }
    }

    /// The messages are handled in their player's own tick (it knows where they stand), and their
    /// entries age and report to the link network.
    fn player_tick(&self, host: &mut dyn Host, _: &mut PlayerTickEvent) {
        let Some(player) = host.acting_player() else {
            return;
        };
        let uuid = player.uuid;
        let level = signal::host_level(&*host);
        let messages = lock(&INBOX).remove(&uuid).unwrap_or_default();
        let has_server = lock(&SERVERS).contains_key(&uuid);
        if messages.is_empty() && !has_server {
            return;
        }
        let pos = [
            player.position[0].floor() as i32,
            player.position[1].floor() as i32,
            player.position[2].floor() as i32,
        ];
        let held = [host.held_stack(uuid), host.player_equipment(uuid, EquipmentSlot::OffHand)]
            .into_iter()
            .flatten()
            .find(is_controller);
        let mut effects = Vec::new();
        for message in messages {
            match message {
                Message::Input { keys, pressed } => {
                    let Some(held) = &held else {
                        continue;
                    };
                    if player.game_mode == util::SPECTATOR && pressed {
                        continue;
                    }
                    effects.extend(receive(uuid, level, pos, &items_of(held), &keys, pressed));
                }
                // `LinkedControllerInputPacket.handleLectern`: the controller on the lectern, if this player uses it.
                Message::LecternInput { lectern, keys, pressed } => {
                    if player.game_mode == util::SPECTATOR && pressed {
                        continue;
                    }
                    if host.block(lectern) != LECTERN_CONTROLLER {
                        continue;
                    }
                    let Some(entity) = host.mod_block_entity(lectern) else {
                        continue;
                    };
                    if !lectern_controller::is_used_by(&entity.nbt, uuid) {
                        continue;
                    }
                    let Some(items) = lectern_controller::controller_items(&entity.nbt) else {
                        continue;
                    };
                    effects.extend(receive(uuid, level, pos, &items, &keys, pressed));
                }
                Message::StopLectern { lectern } => lectern_controller::try_stop(host, uuid, lectern),
                Message::Bind { key, pos: at } => {
                    if player.game_mode == util::SPECTATOR || held.is_none() {
                        continue;
                    }
                    if host.block(at) != LINK {
                        continue;
                    }
                    let Some(entity) = host.mod_block_entity(at) else {
                        continue;
                    };
                    let stacks = link::link_stacks(&entity.nbt);
                    lock(&BINDS).insert(uuid, (key, stacks));
                    BIND_COUNT.store(lock(&BINDS).len(), Ordering::Relaxed);
                    host.action_bar(uuid, Text::literal(format!("Frequency bound to {}", rules::KEY_LABELS[key])));
                }
            }
        }
        {
            let mut servers = lock(&SERVERS);
            if let Some((server, _)) = servers.get_mut(&uuid) {
                effects.extend(server.tick());
                for (key, at) in server.alive() {
                    link::controller_touch(level, key, at);
                }
                if server.is_empty() {
                    servers.remove(&uuid);
                }
            }
        }
        apply(host, level, effects);
    }

    /// A bind is written to the held controller.
    fn inventory_tick(&self, host: &mut dyn Host, event: &mut InventoryTickEvent) {
        if !event.selected || BIND_COUNT.load(Ordering::Relaxed) == 0 {
            return;
        }
        let Some(player) = host.acting_player() else {
            return;
        };
        let Some((key, stacks)) = lock(&BINDS).get(&player.uuid).cloned() else {
            return;
        };
        let Some(mut stack) = host.carried_stack(event.slot).map(|s| s.clone()) else {
            return;
        };
        if !is_controller(&stack) {
            return;
        }
        let mut items = items_of(&stack);
        items[rules::frequency_slot(key, true)] = stacks[0].clone();
        items[rules::frequency_slot(key, false)] = stacks[1].clone();
        let _ = host.set_component(&mut stack, rules::COMPONENT, Tag::Compound(tag_of_items(&items)));
        if let Some(slot) = host.carried_stack(event.slot) {
            *slot = stack;
        }
        let mut binds = lock(&BINDS);
        binds.remove(&player.uuid);
        BIND_COUNT.store(binds.len(), Ordering::Relaxed);
    }

    /// `onItemUseFirst`: a click on a Redstone Link with the controller is bind mode, not a
    /// frequency item (this module runs before the link's own).
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if !is_controller(event.stack) || host.block(event.pos) != LINK {
            return Flow::Continue;
        }
        Flow::Cancel
    }
}
