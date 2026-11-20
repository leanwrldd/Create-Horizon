//! The Stock Ticker (`create:stock_ticker`) (`StockTickerBlock`, `StockTickerBlockEntity`, `StockCheckingBlockEntity`,
//! `StockTickerInteractionHandler`, `PackageOrderRequestPacket`, `AllShapes.STOCK_TICKER`): the block that sees a logistics network's
//! stock and orders from it. The rules are `rules::stock_ticker` (the order list, the packet) and `rules::logistics_network` (the
//! network); the shared run time is `logistics.rs`; this is the block and the commands that stand in for its screen.
//!
//! How it works here:
//! - Right-click a Stock Ticker (an empty hand, or any item but a linked one) to open it: the chat shows how many Stock Links
//!   hold the network up and how to order. A ticker takes the frequency of the player's last tuning, or starts a network of its own.
//! - `/stock list [page]` shows what the network holds, the most first. `/stock add <item> [count|stack|all]` and
//!   `/stock remove ...` build the order (`StockKeeperRequestScreen.mouseClicked`, at most nine kinds, never more than is held),
//!   `/stock address <text>` sets where the boxes are addressed, `/stock send` broadcasts the order (`PackageOrderRequestPacket`) and
//!   `/stock program` remembers it for the next Redstone Requester you place (`RedstoneRequesterBlock.programRequester`).
//!
//! Known differences from Create:
//! - Create's request screen (the item grid, categories, the recipe viewer, the lock button, the hat) is replaced by commands, and no
//!   shopkeeper has to be seated beside the ticker (`isKeeperPresent`). The categories and the lock are not offered; a network
//!   owner is kept from the Stock Link's placer (`mayInteract`) and unlocked networks are open to everyone.
//! - A shopkeeper doesn't have to be seated for `/stock` (`isKeeperPresent`), but a Table Cloth shop needs one (a mob on a Seat beside
//!   the ticker, or a Blaze Burner), as in Create. Right-click the ticker holding a Shopping List to hand it over: the payment goes into
//!   the ticker's 27 payment slots (`receivedPayments`, reachable by hoppers and the like) and the goods are ordered to the list's address
//!   (`StockTickerInteractionHandler.interactWithShop`); Create has you hand the list to the shopkeeper itself. A click with anything else
//!   first collects the payments (`StockTickerBlock.useItemOn`), and only an empty ticker opens.
//! - The order and the address live in memory per player, not in the block.

use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex, MutexGuard};

use create_kinetics::Pos;
use create_kinetics::rules::filter::display_name;
use create_kinetics::rules::logistics_network::Freq;
use create_kinetics::rules::package::Context;
use create_kinetics::rules::packager::RequestType;
use create_kinetics::rules::redstone_requester::AutoRequestData;
use create_kinetics::rules::stock_commands::{self as commands, Stock};
use create_kinetics::rules::stock_ticker::{self as rules, OrderList, OrderRequest};
use create_kinetics::rules::package::Big;
use crate::compat::{
    BlockEntityContainerEvent, ModContainer, NativeInventoryKind, BlockEntityEvent, BlockEntityTickEvent, BlockEntityTicksEvent, Compound, Flow, Hooks, Host, ItemStack, NativeCommand,
    PerformCommandEvent, PlaceBlockEvent, RegisterCommandsEvent, RightClickBlockEvent, StateForPlacementEvent, Tag, Text,
    UseItemOnEvent,
};

use super::logistics as network;
use super::package as items;
use super::table_cloth;
use crate::util;

const PREVIOUS_ADDRESS: &str = "PreviousAddress";

struct Tickers;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Tickers)
}

// --- what a player has open ----------------------------------------------------------------------------

/// The Stock Ticker a player has open and the order they are building at it (`StockKeeperRequestScreen`'s `itemsToOrder` and its
/// address box).
struct Session {
    ticker: Pos,
    order: OrderList<ItemStack>,
    address: String,
}

static SESSIONS: LazyLock<Mutex<BTreeMap<u128, Session>>> = LazyLock::new(Mutex::default);

fn sessions() -> MutexGuard<'static, BTreeMap<u128, Session>> {
    SESSIONS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// An order programmed for the next Redstone Requester a player places.
pub struct Programmed {
    pub data: AutoRequestData<ItemStack>,
}

static PROGRAMMED: LazyLock<Mutex<BTreeMap<u128, Programmed>>> = LazyLock::new(Mutex::default);

/// The order a player programmed, used up by the requester they place.
pub fn take_programmed(uuid: u128) -> Option<Programmed> {
    PROGRAMMED.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&uuid)
}

fn tell(host: &mut dyn Host, uuid: u128, line: impl Into<String>) {
    host.tell(uuid, Text::literal(line.into()));
}

/// `receivedPayments`: `new SmartInventory(27, this, 64, false)`.
const PAYMENT_SLOTS: usize = rules::PAYMENT_SLOTS;
/// `AllBlockEntityTypes.HEATER`: the blocks that count as a shopkeeper beside the ticker.
const HEATER: &str = "create:blaze_burner";

/// `StockTickerBlockEntity.isKeeperPresent`: a mob on a Seat beside the ticker (in its row or the one below) or a Blaze Burner beside it.
pub fn keeper_present(host: &dyn Host, ticker: Pos) -> bool {
    let dimension = host.dimension();
    rules::keeper_present(
        ticker,
        &|seat| !super::seat::riders_at(host, dimension, seat).is_empty(),
        &|at| host.block(at) == HEATER,
    )
}

/// `StockTickerBlock.useItemOn`: what the payment slots hold goes to the player (placed back into the inventory). Whether there was any.
fn collect_payments(host: &mut dyn Host, uuid: u128, pos: Pos) -> bool {
    let Some(entity) = host.mod_block_entity(pos) else {
        return false;
    };
    let paid: Vec<ItemStack> = entity.items.iter().flatten().cloned().collect();
    if paid.is_empty() {
        return false;
    }
    host.change_mod_block_entity(pos, &mut |entity| entity.items = vec![None; PAYMENT_SLOTS]);
    for stack in paid {
        if let Some(rest) = host.give_stack(uuid, stack) {
            let at = host.acting_player().map_or([0.0; 3], |p| p.position);
            util::spawn_item(host, at, &rest, [0.0; 3], 0);
        }
    }
    host.play_sound("minecraft:entity.item.pickup", util::centre(pos), 2, 0.2, 1.4);
    true
}

fn deny(host: &mut dyn Host, uuid: u128, line: &str) {
    if let Some(player) = host.acting_player() {
        host.play_sound("create:deny", player.position, 7, 1.0, 1.0);
    }
    host.action_bar(uuid, Text::literal(line.to_owned()));
}

/// `StockTickerInteractionHandler.interactWithShop` with the Shopping List in the player's hand.
fn sell(host: &mut dyn Host, uuid: u128, pos: Pos, held: &ItemStack) {
    let Some(list) = table_cloth::list_of(held) else {
        return;
    };
    let Some(freq) = ticker_freq(host, pos) else {
        return;
    };
    let (goods, payment) = table_cloth::bake(host, &list);
    // "Must be up-to-date".
    let recent = network::summary(host, freq, true);
    let Some(entity) = host.mod_block_entity(pos) else {
        return;
    };
    let mut received: Vec<Option<Big<ItemStack>>> =
        entity.items.iter().map(|slot| slot.as_ref().map(|stack| (items::key(stack), stack.count))).collect();
    received.resize(PAYMENT_SLOTS, None);
    let stacks = host.player_stacks(uuid).unwrap_or_default();
    let player: Vec<Option<Big<ItemStack>>> =
        stacks.iter().take(rules::PLAYER_SLOTS).map(|slot| slot.as_ref().map(|stack| (items::key(stack), stack.count))).collect();
    let outcome = {
        let view: &dyn Host = &*host;
        let max = |key: &ItemStack| view.max_stack_size(&key.item).unwrap_or(64);
        rules::sell(freq, list.shop_network, (&goods, &payment), &recent, &received, &player, &max)
    };
    match outcome {
        rules::Sale::WrongNetwork => deny(host, uuid, "This list is for another shop"),
        rules::Sale::StockTooLow => deny(host, uuid, "Purchase cancelled: Stock levels lower than expected"),
        rules::Sale::CashRegisterFull => deny(host, uuid, "Purchase cancelled: Not enough space in Cash Register"),
        rules::Sale::TooBroke => deny(host, uuid, "You cannot afford this purchase"),
        rules::Sale::Sold { removed, received, order } => {
            for (slot, count) in removed {
                let Some(Some(stack)) = stacks.get(slot) else {
                    continue;
                };
                let left = stack.count.saturating_sub(count);
                let new = (left > 0).then(|| ItemStack { count: left, ..stack.clone() });
                let _ = host.set_inventory_slot_typed(uuid, NativeInventoryKind::Player, slot, Some(stack), new);
            }
            let slots: Vec<Option<ItemStack>> =
                received.into_iter().map(|slot| slot.map(|(key, count)| ItemStack { count, ..key })).collect();
            host.change_mod_block_entity(pos, &mut |entity| entity.items = slots.clone());
            let address = table_cloth::address_of(held);
            // Its result isn't looked at, as in Create (`broadcastPackageRequest(RequestType.PLAYER, order, null, address)`).
            network::broadcast(host, freq, RequestType::Player, &order, &address);
            host.change_mod_block_entity(pos, &mut |entity| {
                util::put(&mut entity.nbt, PREVIOUS_ADDRESS, Tag::String(address.as_str().into()));
            });
            items::shrink_held(uuid, table_cloth::SHOPPING_LIST);
            if !order.is_empty() {
                host.play_sound("create:stock_ticker_trade", util::centre(pos), 2, 1.0, 1.0);
            }
        }
    }
}

fn key_of(item: &str) -> ItemStack {
    ItemStack { item: item.into(), count: 1, damage: 0, extra: None }
}

fn name_of(key: &ItemStack) -> String {
    display_name(&key.item)
}

// --- opening ------------------------------------------------------------------------------------------

/// The frequency of the ticker at `pos`.
fn ticker_freq(host: &mut dyn Host, pos: Pos) -> Option<Freq> {
    host.mod_block_entity(pos).and_then(|entity| network::freq_of(&entity.nbt))
}

/// `StockTickerInteractionHandler.interactWithLogisticsManagerAt` for a player without a Shopping List: the menu opens unless the
/// network is locked to someone else.
fn open(host: &mut dyn Host, uuid: u128, pos: Pos) {
    let Some(freq) = ticker_freq(host, pos) else {
        tell(host, uuid, "The Stock Ticker is not on a network yet");
        return;
    };
    if !network::may_interact(host, freq, uuid) {
        return;
    }
    let previous = host.mod_block_entity(pos).map(|entity| match util::get(&entity.nbt, PREVIOUS_ADDRESS) {
        Some(Tag::String(text)) => text.to_string_lossy(),
        _ => String::new(),
    });
    let mut open = sessions();
    let session = open.entry(uuid).or_insert_with(|| Session { ticker: pos, order: OrderList::default(), address: String::new() });
    if session.ticker != pos {
        *session = Session { ticker: pos, order: OrderList::default(), address: String::new() };
    }
    if session.address.is_empty() {
        session.address = previous.unwrap_or_default();
    }
    drop(open);
    let links = network::linked_count(host, freq);
    tell(host, uuid, format!("Stock Ticker: {links} Stock Link(s) on this network. /stock list, add, remove, address, send, program"));
}

fn show_order(host: &mut dyn Host, uuid: u128, session: &Session) {
    if session.order.items.is_empty() {
        tell(host, uuid, "Order: nothing yet");
    } else {
        let line: Vec<String> = session.order.items.iter().map(|(key, count)| format!("{} x{count}", name_of(key))).collect();
        tell(host, uuid, format!("Order: {}", line.join(", ")));
    }
    let address = if session.address.is_empty() { "none" } else { session.address.as_str() };
    tell(host, uuid, format!("Address: {address}"));
}

// --- the commands -----------------------------------------------------------------------------------------

/// `/stock ...`: what the player asked of the ticker they have open.
fn run(host: &mut dyn Host, uuid: u128, line: &str) {
    let command = match commands::parse_stock(line) {
        Ok(command) => command,
        Err(message) => {
            tell(host, uuid, message);
            return;
        }
    };
    if let Stock::Open(pos) = command {
        if host.block(pos) == network::STOCK_TICKER {
            open(host, uuid, pos);
        } else {
            tell(host, uuid, "There is no Stock Ticker there");
        }
        return;
    }
    let Some((ticker, address)) = sessions().get(&uuid).map(|s| (s.ticker, s.address.clone())) else {
        tell(host, uuid, "Right-click a Stock Ticker first (or /stock open <x> <y> <z>)");
        return;
    };
    if host.block(ticker) != network::STOCK_TICKER {
        sessions().remove(&uuid);
        tell(host, uuid, "That Stock Ticker is gone");
        return;
    }
    let Some(freq) = ticker_freq(host, ticker) else {
        return;
    };
    if !network::may_interact(host, freq, uuid) {
        return;
    }
    let _ = address;
    let remove = matches!(command, Stock::Remove { .. });
    let programming = matches!(command, Stock::Program);
    match command {
        Stock::Show | Stock::Open(_) => {
            let stock = network::summary(host, freq, false);
            let mut open = sessions();
            if let Some(session) = open.get_mut(&uuid) {
                // `revalidateOrders` against the latest snapshot.
                session.order.revalidate(Some(&stock));
                let (order, address) = (session.order.items.clone(), session.address.clone());
                drop(open);
                let snapshot = Session { ticker, order: OrderList { items: order }, address };
                tell(host, uuid, format!("{} kinds of item in stock, {} Stock Link(s)", stock.stacks.len(), stock.contributing_links));
                show_order(host, uuid, &snapshot);
            }
        }
        Stock::List(page) => {
            let stock = network::summary(host, freq, false);
            let stacks = stock.stacks_by_count();
            if stacks.is_empty() {
                tell(host, uuid, "The network holds nothing (no Stock Link on a Packager with items behind it?)");
                return;
            }
            let pages = stacks.len().div_ceil(10);
            let page = page.min(pages);
            tell(host, uuid, format!("Stock, page {page} of {pages}:"));
            for (key, count) in stacks.iter().skip((page - 1) * 10).take(10) {
                tell(host, uuid, format!("  {} x{count}  ({})", name_of(key), key.item));
            }
        }
        Stock::Add { item, count } | Stock::Remove { item, count } => {
            let key = key_of(&item);
            let stock = network::summary(host, freq, false);
            let held = stock.count_of(&key);
            if !remove && held == 0 {
                tell(host, uuid, format!("{} is not in stock", name_of(&key)));
                return;
            }
            let max = host.max_stack_size(&key.item).unwrap_or(64);
            let transfer = count.resolve(max, held).min(i32::MAX as u32) as i32;
            let mut open = sessions();
            let Some(session) = open.get_mut(&uuid) else {
                return;
            };
            let change = session.order.click(&key, held, false, remove, transfer);
            let (order, address) = (session.order.items.clone(), session.address.clone());
            drop(open);
            if matches!(change, rules::Change::None) {
                tell(host, uuid, "Nothing changed (the order holds nine kinds at most, and no more than the network has)");
            }
            show_order(host, uuid, &Session { ticker, order: OrderList { items: order }, address });
        }
        Stock::Clear => {
            if let Some(session) = sessions().get_mut(&uuid) {
                session.order.items.clear();
            }
            tell(host, uuid, "Order cleared");
        }
        Stock::Address(text) => {
            if let Some(session) = sessions().get_mut(&uuid) {
                session.address = text.clone();
            }
            tell(host, uuid, if text.is_empty() { "Address cleared".to_owned() } else { format!("Address: {text}") });
        }
        Stock::Send | Stock::Program => {
            let stock = network::summary(host, freq, true);
            let (order, address) = {
                let mut open = sessions();
                let Some(session) = open.get_mut(&uuid) else {
                    return;
                };
                session.order.revalidate(Some(&stock));
                (session.order.to_order(), session.address.clone())
            };
            if order.is_empty() {
                tell(host, uuid, "The order is empty");
                return;
            }
            match rules::apply_order_packet(order, address.clone(), programming) {
                OrderRequest::Program { order, address } => program(host, uuid, ticker, freq, order, address),
                OrderRequest::Broadcast { order, address } => send(host, uuid, ticker, freq, &order, &address),
            }
        }
    }
}

/// `StockTickerBlockEntity.broadcastPackageRequest` with `RequestType.PLAYER`.
fn send(host: &mut dyn Host, uuid: u128, ticker: Pos, freq: Freq, order: &Context<ItemStack>, address: &str) {
    let accepted = network::broadcast(host, freq, RequestType::Player, order, address);
    // `previouslyUsedAddress` is kept whether or not it went.
    host.change_mod_block_entity(ticker, &mut |entity| {
        util::put(&mut entity.nbt, PREVIOUS_ADDRESS, Tag::String(address.into()));
    });
    if accepted {
        host.play_sound("create:stock_ticker_request", util::centre(ticker), 2, 1.0, 1.0);
        if let Some(session) = sessions().get_mut(&uuid) {
            session.order.items.clear();
        }
        let total: u32 = order.stacks.iter().map(|(_, count)| *count).sum();
        tell(host, uuid, format!("Order sent: {total} item(s)"));
    } else {
        tell(host, uuid, "The order was not sent (nothing in stock for it, or a Packager has too many boxes waiting)");
    }
}

/// `PackageOrderRequestPacket` with `encodeRequester`: the order is kept for the next requester.
fn program(host: &mut dyn Host, uuid: u128, ticker: Pos, freq: Freq, order: Context<ItemStack>, address: String) {
    let data = AutoRequestData::programmed(order, address, ticker, host.dimension());
    PROGRAMMED.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(uuid, Programmed { data });
    network::tune(uuid, freq);
    host.play_sound("create:stock_ticker_request", util::centre(ticker), 2, 1.0, 1.0);
    tell(host, uuid, "Order programmed: the next Redstone Requester you place sends it (and joins this network)");
}

// --- the hooks -----------------------------------------------------------------------------------------------

impl Hooks for Tickers {
    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        (event.block == network::STOCK_TICKER).then(|| (event.block.to_owned(), Compound::new()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == network::STOCK_TICKER {
            event.ticks = true;
        }
    }

    /// `receivedPayments`, reachable from every side (`registerCapabilities`).
    fn block_entity_container(&self, _: &dyn Host, event: &mut BlockEntityContainerEvent<'_>) -> Option<ModContainer> {
        (event.block == network::STOCK_TICKER).then(|| ModContainer {
            slots: PAYMENT_SLOTS,
            faces: [(); 6].map(|()| (0..PAYMENT_SLOTS).collect()),
            inputs: (0..PAYMENT_SLOTS).collect(),
            outputs: (0..PAYMENT_SLOTS).collect(),
            menu: None,
        })
    }

    /// `StockTickerBlock.getStateForPlacement`: it faces the player (the other way when sneaking).
    fn state_for_placement(
        &self,
        level: &dyn crate::compat::LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<crate::compat::BlockStateId>> {
        let context = event.context;
        if &*context.item != network::STOCK_TICKER {
            return None;
        }
        let facing = create_kinetics::rules::deployer::horizontal_direction(context.yaw).opposite();
        let facing = if context.sneaking { facing.opposite() } else { facing };
        Some(level.state_with_property(event.state, "facing", facing.name()))
    }

    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if &*event.stack.item == network::STOCK_TICKER {
            network::note_placed(host, event.pos);
        }
    }

    /// The ticker is a reader of the network (`LogisticallyLinkedBehaviour(be, false)`): it keeps itself alive on its frequency.
    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != network::STOCK_TICKER {
            return;
        }
        let now = util::game_time(&*host);
        let changes = network::tick_link(host, event.pos, &event.entity.nbt, false, 0, now);
        for (key, value) in changes {
            if util::differs(&event.entity.nbt, key, &value) {
                util::put(&mut event.entity.nbt, key, value);
            }
        }
    }

    fn destroy_block(&self, host: &mut dyn Host, event: &mut crate::compat::DestroyBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) == network::STOCK_TICKER
            && let Some(entity) = host.mod_block_entity(pos)
        {
            network::link_removed(host, pos, &entity.nbt, false);
            // `StockTickerBlockEntity.destroy`: the payments are dropped.
            for stack in entity.items.iter().flatten() {
                util::spawn_item(host, util::centre(pos), stack, [0.0; 3], 10);
            }
        }
        Flow::Continue
    }

    /// An empty hand opens the ticker.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != network::STOCK_TICKER {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        if host.held_stack(player.uuid).is_some() {
            return Flow::Continue;
        }
        if !util::first_click(&*host, player.uuid, pos) {
            return Flow::Cancel;
        }
        if collect_payments(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        open(host, player.uuid, pos);
        Flow::Cancel
    }

    /// `StockTickerBlock.useItemOn`: the linked items and the Wrench are the item's; anything else opens it.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        if host.block(pos) != network::STOCK_TICKER {
            return Flow::Continue;
        }
        if &*event.stack.item == util::WRENCH {
            return util::wrench(host, pos, |_| {});
        }
        if network::is_linked_item(&event.stack.item) {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.sneaking || player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        if !util::first_click(&*host, player.uuid, pos) {
            return Flow::Cancel;
        }
        if &*event.stack.item == table_cloth::SHOPPING_LIST {
            let held = event.stack.clone();
            sell(host, player.uuid, pos, &held);
            return Flow::Cancel;
        }
        if collect_payments(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        open(host, player.uuid, pos);
        Flow::Cancel
    }

    fn register_commands(&self, _: &mut dyn Host, event: &mut RegisterCommandsEvent<'_>) {
        event.commands.push(NativeCommand::new("stock"));
        event.commands.push(NativeCommand::new("stock <text:text>"));
    }

    fn perform_command(&self, host: &mut dyn Host, event: &mut PerformCommandEvent<'_>) -> Flow {
        let line = event.command.trim();
        let Some(rest) = (if line == "stock" { Some("") } else { line.strip_prefix("stock ") }) else {
            return Flow::Continue;
        };
        run(host, event.player.uuid, rest);
        Flow::Cancel
    }
}
