//! Table Cloths and the Shopping List (`TableClothBlock`, `TableClothBlockEntity`, `TableClothFilteringBehaviour`, `TableClothRenderer`,
//! `ShoppingListItem`, `AllShapes.TABLE_CLOTH`): a cloth lays up to four items out for show, or, once it has been programmed at a Stock
//! Ticker, is a shop: each click adds one order to the buyer's Shopping List, which the shopkeeper at the ticker turns into a sale
//! (`stock_ticker.rs`). The rules are `rules::table_cloth` (the list, the shop click, the display) and `rules::shop_commands`; this is
//! the blocks (the sixteen colours and the Andesite, Brass and Copper Table Covers), their saved form, the Shopping List item and its
//! hooks.
//!
//! How it works here:
//! - Right-click a cloth with an item (the main hand) to lay it out, up to four; with an empty hand to take the last one back.
//! - Program a shop at a Stock Ticker (`/stock add ...` then `/stock program`), then place a cloth: it becomes your shop, facing you,
//!   its goods the order, its address the order's. Sneak-click it with the price item to set the price tag (the owner only; any item
//!   but a filter or a Shopping List), then `/cloth price <1-100>` for the amount per order.
//! - Right-click a shop with an empty hand (or a Shopping List) to add one order to a list. The shopkeeper at the ticker has to be there
//!   (a mob on a Seat beside the ticker, or a Blaze Burner), as in Create. Right-click the ticker with the list to pay and order.
//! - Sneak-click with a Shopping List in hand to throw it away.
//!
//! Known differences from Create:
//! - The price tag is set by a sneaking click and a command, not by clicking the tag's spot with the value box. The block has no
//!   `entity` state (every cloth keeps a block entity), no placement helper (`PlacementHelper`: place cloths beside each other by hand),
//!   no occlusion of its side (`sideOccluded`, the price tag's top or side model), and no price tag model drawn: the shop shows its goods.
//! - A new Shopping List goes into the first free slot of the inventory, not the hand; no tooltip lists the goods and the cost.
//! - A shop's order and address come from a player's memory (`/stock program`), not the placed item (see `logistics.rs`). The data
//!   `writeToItem` would put in the dropped cloth isn't kept: a broken shop drops a plain cloth.
//! - A list keys a cloth by its block position (Create's Y is "pixel perfect").

use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};
use std::sync::atomic::{AtomicUsize, Ordering};

use create_kinetics::rules::filter::display_name;
use create_kinetics::rules::package::Summary;
use create_kinetics::rules::redstone_requester::AutoRequestData;
use create_kinetics::rules::shop_commands::{self as commands, Cloth as Command};
use create_kinetics::rules::table_cloth::{self as rules, BlockUse, Cloth, ClothView, DisplayUse, Refusal, ShopClick, ShopUse, ShoppingList};
use create_kinetics::{Dir, Pos};
use crate::compat::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent, BlockEntityUpdateTagEvent,
    ComponentTypesEvent, Compound, DisplayContext, Flow, Hooks, Host, ItemStack, NativeCommand, NativeInventoryKind, PerformCommandEvent,
    PlaceBlockEvent, RegisterCommandsEvent, RightClickBlockEvent, Tag, Text, UseItemEvent, UseItemOnEvent,
};

use super::logistics as network;
use super::package as items;
use super::stock_ticker;
use crate::{int, util};

pub const SHOPPING_LIST: &str = "create:shopping_list";
const LIST: &str = "create:shopping_list";
const LIST_ADDRESS: &str = "create:shopping_list_address";

/// `TableClothBlockEntity.write`, and the price tag's own two entries.
const ITEMS: &str = "Items";
const FACING: &str = "Facing";
const REQUEST: &str = "RequestData";
const OWNER: &str = "OwnerUUID";
const PRICE_ITEM: &str = "PriceItem";
const PRICE_COUNT: &str = "PriceCount";

struct Cloths;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Cloths)
}

fn lock<T>(mutex: &'static Mutex<T>) -> MutexGuard<'static, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The shop a cloth was placed with, until its block entity ticks and takes it: its data, the owner and the way it faces.
static INITIAL: LazyLock<Mutex<BTreeMap<(String, Pos), (AutoRequestData<ItemStack>, u128, Dir)>>> = LazyLock::new(Mutex::default);
static INITIAL_COUNT: AtomicUsize = AtomicUsize::new(0);
/// The cloth each player last sneak-clicked, for `/cloth`.
static SELECTED: LazyLock<Mutex<BTreeMap<u128, Pos>>> = LazyLock::new(Mutex::default);

fn tell(host: &mut dyn Host, uuid: u128, line: impl Into<String>) {
    host.action_bar(uuid, Text::literal(line.into()));
}

/// Whether a block or item id is one of the nineteen cloths.
pub fn is_cloth(id: &str) -> bool {
    id.strip_prefix("create:").is_some_and(|name| name.ends_with("_table_cloth"))
}

// --- the Shopping List item ------------------------------------------------------------------------------------

fn freq_tag(freq: u128) -> Tag {
    network::freq_tag(freq)
}

fn freq_from(tag: Option<&Tag>) -> u128 {
    match tag {
        Some(Tag::IntArray(ints)) if ints.len() == 4 => create_kinetics::rules::stock_link::freq_from_ints([ints[0], ints[1], ints[2], ints[3]]),
        _ => 0,
    }
}

/// `ShoppingListItem.getList`.
pub fn list_of(stack: &ItemStack) -> Option<ShoppingList> {
    if &*stack.item != SHOPPING_LIST {
        return None;
    }
    let tag = stack.data()?.other(LIST)?;
    let purchases = match tag.get("purchases") {
        Some(Tag::List(list)) => list
            .iter()
            .filter_map(|entry| {
                let Some(Tag::IntArray(at)) = entry.get("pos") else {
                    return None;
                };
                let count = match entry.get("count") {
                    Some(Tag::Int(count)) => *count,
                    _ => return None,
                };
                Some(rules::Purchase { cloth: [*at.first()?, *at.get(1)?, *at.get(2)?], count })
            })
            .collect(),
        _ => Vec::new(),
    };
    Some(ShoppingList { purchases, shop_owner: freq_from(tag.get("shop_owner")), shop_network: freq_from(tag.get("shop_network")) })
}

/// `ShoppingListItem.getAddress`.
pub fn address_of(stack: &ItemStack) -> String {
    match stack.data().and_then(|data| data.other(LIST_ADDRESS)) {
        Some(Tag::String(text)) => text.to_string_lossy(),
        _ => String::new(),
    }
}

/// `ShoppingListItem.saveList` on a new Shopping List.
pub fn list_item(host: &dyn Host, list: &ShoppingList, address: &str) -> ItemStack {
    let mut stack = ItemStack { item: SHOPPING_LIST.into(), count: 1, damage: 0, extra: None };
    let mut tag = Compound::new();
    let purchases = list
        .purchases
        .iter()
        .map(|purchase| {
            let mut entry = Compound::new();
            entry.insert("pos".into(), Tag::IntArray(purchase.cloth.to_vec()));
            entry.insert("count".into(), Tag::Int(purchase.count));
            Tag::Compound(entry)
        })
        .collect();
    tag.insert("purchases".into(), Tag::List(purchases));
    tag.insert("shop_owner".into(), freq_tag(list.shop_owner));
    tag.insert("shop_network".into(), freq_tag(list.shop_network));
    let _ = host.set_component(&mut stack, LIST, Tag::Compound(tag));
    let _ = host.set_component(&mut stack, LIST_ADDRESS, Tag::String(address.into()));
    stack
}

/// What a list's baking reads of the cloth at `pos` (`TableClothBlockEntity`): none when there is none.
pub fn cloth_view(host: &mut dyn Host, pos: Pos) -> Option<ClothView<ItemStack>> {
    if !is_cloth(&host.block(pos)) {
        return None;
    }
    host.mod_block_entity(pos).map(|entity| read(&entity.nbt).view())
}

/// The views of every cloth the lists name, for the rules' lookups.
fn views_of(host: &mut dyn Host, lists: &[&ShoppingList]) -> BTreeMap<Pos, ClothView<ItemStack>> {
    let mut map = BTreeMap::new();
    for list in lists {
        for purchase in &list.purchases {
            if let std::collections::btree_map::Entry::Vacant(slot) = map.entry(purchase.cloth)
                && let Some(view) = cloth_view(host, purchase.cloth)
            {
                slot.insert(view);
            }
        }
    }
    map
}

/// `ShoppingList.bakeEntries(level, null)`: the goods and the payment.
pub fn bake(host: &mut dyn Host, list: &ShoppingList) -> (Summary<ItemStack>, Summary<ItemStack>) {
    let views = views_of(host, &[list]);
    list.bake_entries(None, &|pos| views.get(&pos).cloned())
}

// --- the saved form -----------------------------------------------------------------------------------------------

fn facing_value(dir: Dir) -> i32 {
    match dir {
        Dir::South => 0,
        Dir::West => 1,
        Dir::North => 2,
        _ => 3,
    }
}

fn facing_of(value: i32) -> Dir {
    [Dir::South, Dir::West, Dir::North, Dir::East][value.rem_euclid(4) as usize]
}

fn data_tag(data: &AutoRequestData<ItemStack>) -> Tag {
    let mut tag = Compound::new();
    tag.insert("request".into(), items::context_tag(&data.encoded_request));
    tag.insert("address".into(), Tag::String(data.encoded_target_address.as_str().into()));
    tag.insert("offset".into(), Tag::IntArray(data.target_offset.to_vec()));
    tag.insert("dim".into(), Tag::String(data.target_dim.as_str().into()));
    tag.insert("valid".into(), Tag::Byte(i8::from(data.is_valid)));
    Tag::Compound(tag)
}

fn data_of(tag: &Tag) -> AutoRequestData<ItemStack> {
    let mut data = AutoRequestData::default();
    if let Some(request) = tag.get("request").and_then(items::context_of) {
        data.encoded_request = request;
    }
    if let Some(Tag::String(text)) = tag.get("address") {
        data.encoded_target_address = text.to_string_lossy();
    }
    if let Some(Tag::IntArray(at)) = tag.get("offset")
        && at.len() == 3
    {
        data.target_offset = [at[0], at[1], at[2]];
    }
    if let Some(Tag::String(text)) = tag.get("dim") {
        data.target_dim = text.to_string_lossy();
    }
    data.is_valid = matches!(tag.get("valid"), Some(Tag::Byte(1)));
    data
}

/// `TableClothBlockEntity.read`.
fn read(nbt: &Compound) -> Cloth<ItemStack> {
    let mut cloth = Cloth::default();
    cloth.manually_added_items = util::stacks_of(nbt, ITEMS).iter().map(items::key).collect();
    if util::has(nbt, FACING) {
        cloth.facing = facing_of(int(nbt, FACING));
    }
    if let Some(tag) = util::get(nbt, REQUEST) {
        cloth.request_data = data_of(tag);
    }
    cloth.owner = match util::get(nbt, OWNER) {
        Some(tag @ Tag::IntArray(_)) => Some(freq_from(Some(tag))),
        _ => None,
    };
    cloth.price_item = util::get(nbt, PRICE_ITEM).and_then(util::stack_of).map(|stack| items::key(&stack));
    if util::has(nbt, PRICE_COUNT) {
        cloth.price_count = int(nbt, PRICE_COUNT).max(1);
    }
    cloth
}

/// `TableClothBlockEntity.write`: whether anything differed.
fn store(nbt: &mut Compound, cloth: &Cloth<ItemStack>) -> bool {
    let mut wanted: Vec<(&str, Option<Tag>)> = vec![
        (ITEMS, Some(util::stacks_tag(&cloth.manually_added_items))),
        (FACING, Some(Tag::Int(facing_value(cloth.facing)))),
        (REQUEST, Some(data_tag(&cloth.request_data))),
        (OWNER, cloth.owner.map(freq_tag)),
        (PRICE_ITEM, cloth.price_item.as_ref().map(util::stack_tag)),
        (PRICE_COUNT, Some(Tag::Int(cloth.price_count))),
    ];
    let mut changed = false;
    for (key, value) in wanted.drain(..) {
        match value {
            Some(value) => {
                if util::differs(nbt, key, &value) {
                    util::put(nbt, key, value);
                    changed = true;
                }
            }
            None => {
                if util::has(nbt, key) {
                    util::remove(nbt, key);
                    changed = true;
                }
            }
        }
    }
    changed
}

fn save(host: &mut dyn Host, pos: Pos, cloth: &Cloth<ItemStack>) {
    host.change_mod_block_entity(pos, &mut |entity| {
        store(&mut entity.nbt, cloth);
    });
    host.block_entity_sync(pos);
}

// --- using a cloth -------------------------------------------------------------------------------------------------------

fn refusal_text(why: Refusal) -> &'static str {
    match why {
        Refusal::HoldingOtherItem => "Empty hand required to start a shopping list",
        Refusal::NoPriceSet => "Shop owner must set a price first",
        Refusal::KeeperMissing => "Stock keeper missing",
        Refusal::OutOfStock => "Out of Stock",
    }
}

/// `TableClothBlockEntity.useShop`.
fn use_shop(host: &mut dyn Host, uuid: u128, pos: Pos, cloth: &Cloth<ItemStack>, held: Option<ItemStack>) {
    let stacks = host.player_stacks(uuid).unwrap_or_default();
    let hotbar: Vec<(usize, ItemStack, ShoppingList)> = stacks
        .iter()
        .take(rules::HOTBAR)
        .enumerate()
        .filter_map(|(slot, stack)| {
            let stack = stack.as_ref()?;
            list_of(stack).map(|list| (slot, stack.clone(), list))
        })
        .collect();
    let hotbar_lists: Vec<(usize, ShoppingList)> = hotbar.iter().map(|(slot, _, list)| (*slot, list.clone())).collect();
    let held_list = held.as_ref().map(list_of);
    let ticker = cloth.request_data.ticker_pos(pos);
    let ticker_freq = if host.block(ticker) == network::STOCK_TICKER {
        host.mod_block_entity(ticker).and_then(|entity| network::freq_of(&entity.nbt))
    } else {
        None
    };
    let keeper_network = ticker_freq.filter(|_| stock_ticker::keeper_present(host, ticker));
    let summary = ticker_freq.map(|freq| network::summary(host, freq, false));
    let mut all: Vec<&ShoppingList> = hotbar_lists.iter().map(|(_, list)| list).collect();
    if let Some(Some(list)) = &held_list {
        all.push(list);
    }
    let views = views_of(host, &all);
    let (cleared, outcome) = rules::use_shop(&ShopClick {
        cloth_pos: pos,
        cloth,
        held: held_list.as_ref().map(Option::as_ref),
        hotbar_lists: &hotbar_lists,
        keeper_network,
        summary: summary.as_ref(),
        cloth_at: &|at| views.get(&at).cloned(),
    });
    // The lists in the hotbar are taken out of the inventory whatever came of the click.
    for slot in cleared {
        if let Some((_, stack, _)) = hotbar.iter().find(|(s, _, _)| *s == slot) {
            let _ = host.set_inventory_slot_typed(uuid, NativeInventoryKind::Player, slot, Some(stack), None);
        }
    }
    let centre = util::centre(pos);
    match outcome {
        ShopUse::Refused { why, give_back } => {
            tell(host, uuid, refusal_text(why));
            host.play_sound("create:deny", centre, 2, 0.5, 1.0);
            if let Some(list) = give_back {
                let address = hotbar.iter().find(|(_, _, l)| *l == list).map(|(_, stack, _)| address_of(stack)).unwrap_or_default();
                let back = list_item(&*host, &list, &address);
                give(host, uuid, back);
            }
        }
        ShopUse::Listed { list, address, limited_stock, started_new_list } => {
            if limited_stock {
                tell(host, uuid, "Stock limit reached");
            } else {
                host.play_sound("create:confirm_2", centre, 2, 0.5, 1.0);
                if started_new_list {
                    tell(host, uuid, "Use this list to add more to your purchase");
                    host.play_sound("minecraft:item.book.page_turn", centre, 2, 1.0, 1.5);
                }
            }
            let item = list_item(&*host, &list, &address);
            give(host, uuid, item);
        }
    }
}

fn give(host: &mut dyn Host, uuid: u128, stack: ItemStack) {
    if let Some(rest) = host.give_stack(uuid, stack) {
        let at = host.acting_player().map_or([0.0; 3], |p| p.position);
        util::spawn_item(host, at, &rest, [0.0; 3], 0);
    }
}

/// `TableClothBlockEntity.use` for a cloth that isn't a shop.
fn use_display(host: &mut dyn Host, uuid: u128, pos: Pos, cloth: &mut Cloth<ItemStack>, held: Option<&ItemStack>, facing: Dir) {
    let key = held.map(items::key);
    match cloth.use_display(key.as_ref(), facing, false) {
        DisplayUse::Nothing => {}
        DisplayUse::Took { item, .. } => {
            give(host, uuid, item);
            host.play_sound("minecraft:entity.item_frame.remove_item", util::centre(pos), 2, 0.5, 1.0);
            save(host, pos, cloth);
        }
        DisplayUse::Put => {
            if let Some(stack) = held {
                host.take_items(&stack.item, 1);
            }
            host.play_sound("minecraft:entity.item_frame.add_item", util::centre(pos), 2, 0.5, 1.0);
            save(host, pos, cloth);
        }
    }
}

/// `TableClothBlock.useItemOn` for the main hand: `held` is what the player holds.
fn click(host: &mut dyn Host, pos: Pos, face: [i32; 3], held: Option<ItemStack>) -> Flow {
    let Some(player) = host.acting_player() else {
        return Flow::Continue;
    };
    let may_build = matches!(player.game_mode, util::SURVIVAL | util::CREATIVE);
    let Some(entity) = host.mod_block_entity(pos) else {
        return Flow::Continue;
    };
    let mut cloth = read(&entity.nbt);
    let has_entity = cloth.is_shop() || !cloth.manually_added_items.is_empty();
    // The price tag: a sneaking click with the price item, by the owner.
    if player.sneaking && cloth.is_shop() && may_build && face != [0, -1, 0] {
        if !util::first_click(&*host, player.uuid, pos) {
            return Flow::Cancel;
        }
        if !rules::may_set_price(cloth.owner, player.uuid) {
            tell(host, player.uuid, "This shop belongs to someone else");
            return Flow::Cancel;
        }
        lock(&SELECTED).insert(player.uuid, pos);
        if let Some(stack) = &held {
            let filter = ["create:filter", "create:attribute_filter", "create:package_filter"].contains(&&*stack.item);
            if rules::may_be_price(filter, &*stack.item == SHOPPING_LIST) {
                cloth.set_price_item(Some(items::key(stack)));
                save(host, pos, &cloth);
            }
        }
        tell(host, player.uuid, describe(&cloth));
        return Flow::Cancel;
    }
    let held_is_cloth = held.as_ref().map(|stack| is_cloth(&stack.item));
    match rules::block_use(face == [0, -1, 0], may_build, held_is_cloth.unwrap_or(false), player.sneaking, held.is_none(), has_entity) {
        BlockUse::PassToDefault | BlockUse::PlaceNext => Flow::Continue,
        BlockUse::Use { .. } => {
            if !util::first_click(&*host, player.uuid, pos) {
                return Flow::Cancel;
            }
            if cloth.is_shop() {
                use_shop(host, player.uuid, pos, &cloth, held);
            } else {
                let facing = create_kinetics::rules::deployer::horizontal_direction(player.yaw);
                use_display(host, player.uuid, pos, &mut cloth, held.as_ref(), facing);
            }
            Flow::Cancel
        }
    }
}

fn describe(cloth: &Cloth<ItemStack>) -> String {
    let goods: Vec<String> =
        cloth.request_data.encoded_request.stacks.iter().map(|(key, count)| format!("{} x{count}", display_name(&key.item))).collect();
    let price = match &cloth.price_item {
        Some(key) => format!("{} x{}", display_name(&key.item), cloth.price_count),
        None => "no price set".to_owned(),
    };
    format!("Table Cloth shop: {} | price per order: {price}", if goods.is_empty() { "nothing".to_owned() } else { goods.join(", ") })
}

fn run(host: &mut dyn Host, uuid: u128, line: &str) {
    let command = match commands::parse_cloth(line) {
        Ok(command) => command,
        Err(message) => {
            host.tell(uuid, Text::literal(message));
            return;
        }
    };
    let Some(pos) = lock(&SELECTED).get(&uuid).copied() else {
        host.tell(uuid, Text::literal("Sneak-click a Table Cloth shop first".to_owned()));
        return;
    };
    let Some(entity) = host.mod_block_entity(pos).filter(|_| is_cloth(&host.block(pos))) else {
        lock(&SELECTED).remove(&uuid);
        host.tell(uuid, Text::literal("That cloth is gone".to_owned()));
        return;
    };
    let mut cloth = read(&entity.nbt);
    if !rules::may_set_price(cloth.owner, uuid) && !matches!(command, Command::Show | Command::Owner) {
        host.tell(uuid, Text::literal("This shop belongs to someone else".to_owned()));
        return;
    }
    match command {
        Command::Show => {}
        Command::Owner => {
            let who = if cloth.owner == Some(uuid) { "you" } else if cloth.owner.is_some() { "another player" } else { "nobody (anyone may set the price)" };
            host.tell(uuid, Text::literal(format!("Owner: {who}")));
            return;
        }
        Command::Price(count) => {
            cloth.set_price_count(count);
        }
        Command::NoPrice => cloth.set_price_item(None),
    }
    save(host, pos, &cloth);
    host.tell(uuid, Text::literal(describe(&cloth)));
}

// --- the hooks ---------------------------------------------------------------------------------------------------------

impl Hooks for Cloths {
    /// `AllDataComponents.SHOPPING_LIST` and `SHOPPING_LIST_ADDRESS`.
    fn component_types(&self, event: &mut ComponentTypesEvent<'_>) {
        for id in [LIST, LIST_ADDRESS] {
            if !event.types.iter().any(|t| t == id) {
                event.types.push(id.to_owned());
            }
        }
    }

    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        is_cloth(event.block).then(|| (event.block.to_owned(), Compound::new()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if is_cloth(event.block) {
            event.ticks = true;
        }
    }

    /// `TableClothBlock.setPlacedBy`: a cloth placed after `/stock program` is that shop, owned by the placer, facing them.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if !is_cloth(&event.stack.item) {
            return;
        }
        let Some(player) = host.acting_player() else {
            return;
        };
        let Some(programmed) = stock_ticker::take_programmed(player.uuid) else {
            return;
        };
        let dimension = host.dimension().to_owned();
        let (data, message) = programmed.data.read_from_item(&dimension, event.pos);
        let facing = create_kinetics::rules::deployer::horizontal_direction(player.yaw);
        {
            let mut initial = lock(&INITIAL);
            initial.insert((dimension, event.pos), (data, player.uuid, facing));
            INITIAL_COUNT.store(initial.len(), Ordering::Relaxed);
        }
        host.tell(
            player.uuid,
            Text::literal(
                match message {
                    create_kinetics::rules::redstone_requester::Message::KeeperConnected => "Item shop configured",
                    create_kinetics::rules::redstone_requester::Message::KeeperTooFarAway => {
                        "Item shop configured, but its Stock Ticker is too far away (128 blocks, same dimension)"
                    }
                }
                .to_owned(),
            ),
        );
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if !is_cloth(&event.entity.id) || INITIAL_COUNT.load(Ordering::Relaxed) == 0 {
            return;
        }
        let dimension = host.dimension().to_owned();
        let taken = {
            let mut initial = lock(&INITIAL);
            let taken = initial.remove(&(dimension, event.pos));
            INITIAL_COUNT.store(initial.len(), Ordering::Relaxed);
            taken
        };
        let Some((data, owner, facing)) = taken else {
            return;
        };
        let mut cloth = read(&event.entity.nbt);
        cloth.request_data = data;
        cloth.owner = Some(owner);
        cloth.facing = facing.opposite();
        store(&mut event.entity.nbt, &cloth);
        host.block_entity_sync(event.pos);
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        is_cloth(&event.entity.id).then(|| event.entity.nbt.clone())
    }

    fn block_entity_render_bounding_box(&self, _: &dyn Host, event: &mut crate::compat::BlockEntityRenderBoundingBoxEvent<'_>) {
        if is_cloth(event.block) {
            *event.bounds = [-1.0, -1.0, -1.0, 2.0, 2.0, 2.0];
        }
    }

    /// `TableClothRenderer`: the goods of a shop (the first four), else the items laid out, round the middle of the cloth.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if !is_cloth(event.block) {
            return;
        }
        let cloth = read(&event.entity.nbt);
        let shown: Vec<ItemStack> = if cloth.is_shop() {
            cloth.request_data.encoded_request.stacks.iter().take(rules::MANUAL_ITEMS).map(|(key, _)| key.clone()).collect()
        } else {
            cloth.manually_added_items.clone()
        };
        if shown.is_empty() {
            return;
        }
        let draw = &mut *event.draw;
        const UP: [f32; 3] = [0.0, 1.0, 0.0];
        const CENTRE: [f32; 3] = [0.5; 3];
        let yaw = 180.0 - facing_yaw(cloth.facing);
        let count = shown.len();
        for (i, stack) in shown.iter().enumerate() {
            draw.push();
            draw.rotate_about(UP, yaw, CENTRE);
            draw.translate([0.5, 3.0 / 16.0, 0.5]);
            if count > 1 {
                let turn = i as f32 * (360.0 / count as f32) + 45.0;
                draw.rotate_about(UP, turn, [0.0; 3]);
                draw.translate([0.0, if i % 2 == 0 { -0.005 } else { 0.0 }, 5.0 / 16.0]);
                draw.rotate_about(UP, -turn, [0.0; 3]);
            }
            host.draw_item(draw, stack, DisplayContext::Ground, None, i as i32);
            draw.pop();
        }
        draw.still();
    }

    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if !is_cloth(&host.block(pos)) {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR || host.held_stack(player.uuid).is_some() {
            return Flow::Continue;
        }
        click(host, pos, event.face, None)
    }

    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        if !is_cloth(&host.block(pos)) {
            return Flow::Continue;
        }
        if &*event.stack.item == util::WRENCH {
            return util::wrench(host, pos, |_| {});
        }
        click(host, pos, event.face, Some(event.stack.clone()))
    }

    /// `ShoppingListItem.use`: sneaking throws the list away.
    fn use_item(&self, host: &mut dyn Host, event: &mut UseItemEvent<'_>) -> Flow {
        if &*event.stack.item != SHOPPING_LIST {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if !rules::discards_list(true, player.sneaking) {
            return Flow::Continue;
        }
        if util::click_repeated_anywhere(&*host, player.uuid) {
            return Flow::Cancel;
        }
        let at = [player.position[0] as i32, player.position[1] as i32, player.position[2] as i32];
        util::click_taken(&*host, player.uuid, at);
        items::shrink_held(player.uuid, SHOPPING_LIST);
        tell(host, player.uuid, "Shopping list discarded");
        host.play_sound("minecraft:item.book.page_turn", player.position, 7, 1.0, 1.0);
        Flow::Cancel
    }

    fn register_commands(&self, _: &mut dyn Host, event: &mut RegisterCommandsEvent<'_>) {
        event.commands.push(NativeCommand::new("cloth"));
        event.commands.push(NativeCommand::new("cloth <text:text>"));
    }

    fn perform_command(&self, host: &mut dyn Host, event: &mut PerformCommandEvent<'_>) -> Flow {
        let line = event.command.trim();
        let Some(rest) = (if line == "cloth" { Some("") } else { line.strip_prefix("cloth ") }) else {
            return Flow::Continue;
        };
        run(host, event.player.uuid, rest);
        Flow::Cancel
    }
}

/// `Direction.toYRot`.
fn facing_yaw(dir: Dir) -> f32 {
    match dir {
        Dir::South => 0.0,
        Dir::West => 90.0,
        Dir::North => 180.0,
        _ => 270.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_nineteen_cloths_are_cloths() {
        assert!(is_cloth("create:red_table_cloth"));
        assert!(is_cloth("create:andesite_table_cloth"));
        assert!(!is_cloth("create:shopping_list"));
        assert!(!is_cloth("minecraft:red_table_cloth"));
    }

    #[test]
    fn a_facing_is_saved_as_the_two_d_data_value() {
        for dir in [Dir::South, Dir::West, Dir::North, Dir::East] {
            assert_eq!(facing_of(facing_value(dir)), dir);
        }
        assert_eq!(facing_value(Dir::North), 2);
        assert_eq!(facing_of(-1), Dir::East);
    }

    #[test]
    fn a_cloth_survives_being_saved_and_read() {
        let mut cloth: Cloth<ItemStack> = Cloth::default();
        let key = |id: &str| ItemStack { item: id.into(), count: 1, damage: 0, extra: None };
        cloth.manually_added_items = vec![key("minecraft:apple"), key("minecraft:bread")];
        cloth.facing = Dir::West;
        cloth.owner = Some(0x1234_5678_u128 << 40);
        cloth.price_item = Some(key("minecraft:gold_nugget"));
        cloth.price_count = 5;
        cloth.request_data.encoded_request.stacks = vec![(key("create:cogwheel"), 3)];
        cloth.request_data.encoded_target_address = "Shipping".into();
        cloth.request_data.target_offset = [1, 0, -4];
        cloth.request_data.target_dim = "minecraft:overworld".into();
        cloth.request_data.is_valid = true;
        let mut nbt = Compound::new();
        assert!(store(&mut nbt, &cloth));
        assert!(!store(&mut nbt, &cloth));
        assert_eq!(read(&nbt), cloth);
        // An owner that is not set is not saved.
        cloth.owner = None;
        store(&mut nbt, &cloth);
        assert!(!util::has(&nbt, OWNER));
        assert_eq!(read(&nbt).owner, None);
    }
}
