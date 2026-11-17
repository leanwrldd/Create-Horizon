//! The Stock Ticker's order rules (`StockTickerBlockEntity`, `StockCheckingBlockEntity`,
//! `StockTickerInteractionHandler`, `PackageOrderRequestPacket`, `StockKeeperLockPacket`,
//! `StockKeeperCategory*Packet`, and from `StockKeeperRequestScreen` the rules that decide
//! what an order may hold and how its recipes are split): the order a player builds, its
//! recipes (`convertRecipeToPackageOrderContext`, `sendIt`), the hidden categories and the lock,
//! how a stock snapshot is sorted into categories, and a sale to a Shopping List
//! (`interactWithShop`).
//!
//! No game types: `K` is an item with its components (equal when `isSameItemSameComponents`),
//! `F` a category's filter item. What the rules read of the game is plain input (a summary, a
//! player's inventory, the stock level a client last received); what they decide is plain
//! output (the order to broadcast, the slots to take from, the new contents of the cash
//! register). The network side is `rules::logistics_network`.
//!
//! Known differences from Create, for the README:
//! - The request screen's recipe viewer (`requestCraftable`, `maxCraftable`, `updateCraftableAmounts`
//!   and the JEI search sync) isn't ported: it chooses ingredients on screen from a recipe-viewer
//!   click. What it hands to the server is ported: the order and the crafting entries
//!   ([`craft_entries`]).
//! - `forcedEntries` (the screen's momentary lowered stack sizes after sending) and the sounds
//!   are the glue's.
//! - `isKeeperPresent` and `getStockTickerPosition` read seats and the blocks around, which the
//!   caller answers ([`keeper_present`], [`ticker_position`]).

use std::collections::HashMap;

use super::logistics_network::{Broadcast, Freq, LinkView, Networks, PlayerId, Random, broadcast_package_request};
use super::package::{Big, Context, Craft, Summary};
use super::packager::RequestType;
use crate::{Dir, Pos};

/// `StockKeeperRequestScreen.cols`: an order holds at most nine different items.
pub const ORDER_ENTRIES: usize = 9;
/// `StockTickerBlockEntity`: `new SmartInventory(27, this, 64, false)`.
pub const PAYMENT_SLOTS: usize = 27;
/// `InventorySummary.divideAndSendTo`: a snapshot is sent in lists of this many stacks.
pub const SNAPSHOT_CHUNK: usize = 100;
/// `Inventory.items.size()`: the slots a sale takes payment from.
pub const PLAYER_SLOTS: usize = 36;

// --- recipes in an order ----------------------------------------------------------------------------

/// A crafting recipe's ingredient: `None` for an empty one (`Ingredient.isEmpty`), else what
/// `Ingredient.test` says of an item.
pub type Ingredient<'a, K> = Option<&'a dyn Fn(&K) -> bool>;

/// A crafting recipe's shape: a shaped recipe has its width and height (`ShapedRecipe`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Shapeless,
    Shaped { width: usize, height: usize },
}

/// `FactoryPanelScreen.convertRecipeToPackageOrderContext`: the nine slots of a crafting grid
/// for a recipe, each ingredient replaced by the first of `inputs` (with items left) that fits
/// it, one of it; empty slots, and ingredients nothing fits, are `empty`
/// (`new BigItemStack(ItemStack.EMPTY, 1)`). With `respect_amounts`, each use takes one off the
/// input's count (on a copy).
pub fn recipe_pattern<K: PartialEq + Clone>(
    shape: Shape,
    ingredients: &[Ingredient<'_, K>],
    inputs: &[Big<K>],
    respect_amounts: bool,
    empty: &K,
) -> Vec<Big<K>> {
    let empty_ingredient: Big<K> = (empty.clone(), 1);
    let mut pattern: Vec<Big<K>> = Vec::new();
    let mut inputs: Vec<Big<K>> = inputs.to_vec();

    let mut width = ingredients.len().min(3);
    let mut height = (ingredients.len() / 3 + 1).min(3);
    if let Shape::Shaped { width: w, height: h } = shape {
        width = w;
        height = h;
    }

    if height == 1 {
        for _ in 0..3 {
            pattern.push(empty_ingredient.clone());
        }
    }
    if width == 1 {
        pattern.push(empty_ingredient.clone());
    }

    for (i, ingredient) in ingredients.iter().enumerate() {
        let mut crafting_ingredient = empty_ingredient.clone();
        if let Some(test) = ingredient {
            for input in inputs.iter_mut() {
                if input.1 > 0 && test(&input.0) {
                    crafting_ingredient = (input.0.clone(), 1);
                    if respect_amounts {
                        input.1 -= 1;
                    }
                    break;
                }
            }
        }
        pattern.push(crafting_ingredient);

        if width < 3 && width > 0 && (i + 1) % width == 0 {
            for _ in 0..3 - width {
                if pattern.len() < 9 {
                    pattern.push(empty_ingredient.clone());
                }
            }
        }
    }

    while pattern.len() < 9 {
        pattern.push(empty_ingredient.clone());
    }
    pattern
}

/// A recipe ordered through the Stock Ticker (`CraftableBigItemStack` of a crafting recipe).
pub struct OrderedRecipe<'a, K> {
    pub shape: Shape,
    pub ingredients: Vec<Ingredient<'a, K>>,
    /// `recipe.getResultItem().getCount()`.
    pub output_count: u32,
    /// `cbis.count`: how many items of the result were ordered.
    pub count: u32,
}

/// `StockKeeperRequestScreen.sendIt`'s crafting entries: for each ordered recipe, the patterns
/// it is crafted with and how many times each, so that what each craft will use is what was
/// ordered. `ordered` is the order's stacks (after [`OrderList::revalidate`]); the caller sends
/// the entries along with the order only when every ordered stack is used by a recipe
/// (`canRequestCraftingPackage`, from `updateCraftableAmounts`).
pub fn craft_entries<K: PartialEq + Clone>(
    recipes: &[OrderedRecipe<'_, K>],
    ordered: &[Big<K>],
    empty: &K,
) -> Vec<Craft<K>> {
    let mut crafts = Vec::new();
    for recipe in recipes {
        let mut crafted_count: u32 = 0;
        let target_count = recipe.count / recipe.output_count.max(1);
        let mut mutable: Vec<Big<K>> = ordered.to_vec();

        while crafted_count < target_count {
            // "Carefully split the ordered recipes based on what exactly will be used to craft them".
            let pattern = recipe_pattern(recipe.shape, &recipe.ingredients, &mutable, true, empty);
            let max_crafts = target_count - crafted_count;
            let mut available_crafts: u32 = 0;

            let mut items_exhausted = false;
            'outer: while available_crafts < max_crafts && !items_exhausted {
                let previous_snapshot = mutable.clone();
                items_exhausted = true;
                'pattern: for (pattern_key, _) in &pattern {
                    if pattern_key == empty {
                        continue;
                    }
                    for ordered in mutable.iter_mut() {
                        if ordered.0 != *pattern_key || ordered.1 == 0 {
                            continue;
                        }
                        ordered.1 -= 1;
                        items_exhausted = false;
                        continue 'pattern;
                    }
                    mutable = previous_snapshot;
                    break 'outer;
                }
                available_crafts += 1;
            }

            if available_crafts == 0 {
                break;
            }
            crafts.push(Craft { pattern, count: available_crafts });
            crafted_count += available_crafts;
        }
    }
    crafts
}

impl<K: PartialEq + Clone> Context<K> {
    /// `PackageOrderWithCrafts.orderedStacksMatchOrderedRecipes`: the recipes use up exactly
    /// what was ordered (an order without recipes doesn't match).
    pub fn ordered_stacks_match_ordered_recipes(&self) -> bool {
        if self.crafts.is_empty() {
            return false;
        }
        let mut stacks: Summary<K> = Summary::default();
        let mut crafts: Summary<K> = Summary::default();
        for (key, count) in &self.stacks {
            stacks.add(key, i64::from(*count));
        }
        for craft in &self.crafts {
            for (key, count) in &craft.pattern {
                crafts.add(key, i64::from(*count) * i64::from(craft.count));
            }
        }
        if stacks.stacks.len() != crafts.stacks.len() {
            return false;
        }
        stacks.stacks.iter().all(|(key, count)| crafts.count_of(key) == *count)
    }
}

// --- the order a player builds --------------------------------------------------------------------

/// What a click or a scroll did to an order, for the sounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Nothing changed (a full order, or nothing to remove).
    None,
    Added,
    Removed,
    /// The count of an item already there changed.
    Counted,
}

/// `StockKeeperRequestScreen.itemsToOrder`: the stacks being ordered, each with its count (a
/// count can pass a stack's size).
#[derive(Clone, Debug, PartialEq)]
pub struct OrderList<K> {
    pub items: Vec<(K, i32)>,
}

impl<K> Default for OrderList<K> {
    fn default() -> Self {
        Self { items: Vec::new() }
    }
}

impl<K: PartialEq + Clone> OrderList<K> {
    /// `mouseClicked`'s transfer: a stack with shift, ten with control, else one.
    pub fn click_transfer(shift: bool, control: bool, max_stack_size: u32) -> i32 {
        if shift {
            max_stack_size as i32
        } else if control {
            10
        } else {
            1
        }
    }

    /// `mouseScrolled`'s transfer: `ceil(|scroll|)`, times ten with control.
    pub fn scroll_transfer(scroll_y: f64, control: bool) -> i32 {
        scroll_y.abs().ceil() as i32 * if control { 10 } else { 1 }
    }

    /// `getOrderForItem`.
    fn position(&self, key: &K) -> Option<usize> {
        self.items.iter().position(|(k, _)| k == key)
    }

    /// `mouseClicked` on an item (the stock list, or the order itself with `from_order`): the
    /// left button adds up to `transfer` of it, never past what the last stock snapshot holds
    /// (`stock`); the right button, or a click on the order, takes it off, and an item whose
    /// count falls to zero leaves the order. A tenth different item isn't added.
    pub fn click(&mut self, key: &K, stock: u32, right: bool, from_order: bool, transfer: i32) -> Change {
        self.change(key, stock, right || from_order, transfer)
    }

    /// `mouseScrolled` on an item: up adds, down takes off.
    pub fn scroll(&mut self, key: &K, stock: u32, scroll_y: f64, control: bool) -> Change {
        self.change(key, stock, scroll_y < 0.0, Self::scroll_transfer(scroll_y, control))
    }

    fn change(&mut self, key: &K, stock: u32, remove: bool, transfer: i32) -> Change {
        let mut added = false;
        let index = match self.position(key) {
            Some(index) => index,
            None => {
                if self.items.len() >= ORDER_ENTRIES || remove {
                    return Change::None;
                }
                self.items.push((key.clone(), 0));
                added = true;
                self.items.len() - 1
            }
        };
        let current = self.items[index].1;
        if remove {
            self.items[index].1 = current - transfer;
            if self.items[index].1 <= 0 {
                self.items.remove(index);
                return Change::Removed;
            }
            return Change::Counted;
        }
        self.items[index].1 = current + transfer.min(stock as i32 - current);
        if added {
            Change::Added
        } else if self.items[index].1 != current {
            Change::Counted
        } else {
            Change::None
        }
    }

    /// `revalidateOrders`: each count is cut to what the snapshot holds, and what holds none
    /// (or everything, without a snapshot or a source) is dropped.
    pub fn revalidate(&mut self, snapshot: Option<&Summary<K>>) {
        let Some(snapshot) = snapshot else {
            self.items.clear();
            return;
        };
        for entry in &mut self.items {
            entry.1 = entry.1.min(snapshot.count_of(&entry.0).min(i32::MAX as u32) as i32);
        }
        self.items.retain(|(_, count)| *count > 0);
    }

    /// `PackageOrderWithCrafts.simple(itemsToOrder)`.
    pub fn to_order(&self) -> Context<K> {
        Context { stacks: self.items.iter().map(|(k, c)| (k.clone(), (*c).max(0) as u32)).collect(), crafts: Vec::new() }
    }
}

// --- sending a snapshot -----------------------------------------------------------------------------

/// `InventorySummary.divideAndSendTo`: the stacks, the most first, in lists of 100; each is sent
/// with whether it is the last (`endOfTransmission`). An empty summary is one empty last list.
pub fn divide_for_sending<K: PartialEq + Clone>(summary: &Summary<K>) -> Vec<(bool, Vec<Big<K>>)> {
    let stacks = summary.stacks_by_count();
    if stacks.is_empty() {
        return vec![(true, Vec::new())];
    }
    let mut out = Vec::new();
    let mut remaining = stacks.len();
    let mut current: Vec<Big<K>> = Vec::new();
    for entry in stacks {
        current.push(entry);
        remaining -= 1;
        if remaining == 0 {
            break;
        }
        if current.len() < SNAPSHOT_CHUNK {
            continue;
        }
        out.push((false, std::mem::take(&mut current)));
    }
    out.push((true, current));
    out
}

/// `StockTickerBlockEntity.receiveStockPacket`: the snapshot sorted into the ticker's
/// categories, in their order; each category takes (and removes from what is left) the stacks
/// its filter passes (an empty filter takes none), and the last list is what no category took.
pub fn categorize<K: Clone, F>(
    snapshot: &[Big<K>],
    categories: &[Option<F>],
    mut passes: impl FnMut(&F, &K) -> bool,
) -> Vec<Vec<Big<K>>> {
    let mut left: Vec<Big<K>> = snapshot.to_vec();
    let mut lists = Vec::new();
    for filter in categories {
        let mut in_category = Vec::new();
        if let Some(filter) = filter {
            let mut kept = Vec::new();
            for entry in left {
                if passes(filter, &entry.0) {
                    in_category.push(entry);
                } else {
                    kept.push(entry);
                }
            }
            left = kept;
        }
        lists.push(in_category);
    }
    lists.push(left);
    lists
}

// --- the block entity -----------------------------------------------------------------------------

/// What `StockTickerInteractionHandler.interactWithLogisticsManagerAt` does for a player who
/// isn't holding a Shopping List.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// "stock_keeper.locked": the network is locked to someone else.
    Locked,
    /// Opens the request menu, sending the lock options (`showLockOption`, `isCurrentlyLocked`).
    Open { show_lock_option: bool, locked: bool },
}

/// The state of a `StockTickerBlockEntity`.
#[derive(Clone, Debug)]
pub struct Ticker<K, F> {
    /// `previouslyUsedAddress`.
    pub previously_used_address: String,
    /// `activeLinks`: how many links the last summary came from (sent to clients).
    pub active_links: u32,
    /// `categories`: filter items, an empty slot is `None`.
    pub categories: Vec<Option<F>>,
    /// `hiddenCategoriesByPlayer`.
    pub hidden_categories_by_player: HashMap<PlayerId, Vec<i32>>,
    /// `receivedPayments`: what a shop was paid.
    pub received_payments: Vec<Option<Big<K>>>,
}

impl<K, F> Default for Ticker<K, F> {
    fn default() -> Self {
        Self {
            previously_used_address: String::new(),
            active_links: 0,
            categories: Vec::new(),
            hidden_categories_by_player: HashMap::new(),
            received_payments: (0..PAYMENT_SLOTS).map(|_| None).collect(),
        }
    }
}

impl<K: PartialEq + Clone, F> Ticker<K, F> {
    /// `StockTickerBlockEntity.broadcastPackageRequest`: the order is broadcast, and the address
    /// is remembered (whether or not it was accepted).
    pub fn broadcast_package_request(
        &mut self,
        links: &[LinkView<K>],
        kind: RequestType,
        order: &Context<K>,
        address: &str,
        rng: &mut dyn Random,
    ) -> Broadcast<K> {
        let result = broadcast_package_request(links, kind, order, address, rng);
        self.previously_used_address = address.to_owned();
        result
    }

    /// `StockTickerBlockEntity.getRecentSummary`: the number of links the summary came from is
    /// kept and sent when it changed (and the block is still there).
    pub fn note_summary(&mut self, contributing_links: u32, removed: bool) -> bool {
        if self.active_links != contributing_links && !removed {
            self.active_links = contributing_links;
            return true;
        }
        false
    }

    /// `StockTickerInteractionHandler.interactWithLogisticsManagerAt`, for a player without a
    /// Shopping List in hand.
    pub fn access(&self, networks: &Networks<K>, network: Freq, player: PlayerId) -> Access {
        if !networks.may_interact(network, player) {
            return Access::Locked;
        }
        Access::Open {
            show_lock_option: networks.may_administrate(network, player) && networks.is_lockable(network),
            locked: networks.is_locked(network),
        }
    }

    /// `StockKeeperCategoryHidingPacket.applySettings`: no indices clears the player's hidden
    /// categories; else they are stored (and the block entity notified, which is returned).
    pub fn set_hidden_categories(&mut self, player: PlayerId, indices: Vec<i32>) -> bool {
        if indices.is_empty() {
            self.hidden_categories_by_player.remove(&player);
            false
        } else {
            self.hidden_categories_by_player.insert(player, indices);
            true
        }
    }

    /// The categories a player has hidden (`hiddenCategories`; -1 is the unsorted list).
    pub fn hidden_categories(&self, player: PlayerId) -> &[i32] {
        self.hidden_categories_by_player.get(&player).map_or(&[], Vec::as_slice)
    }

    /// `StockKeeperCategoryEditPacket.applySettings`.
    pub fn set_categories(&mut self, categories: Vec<Option<F>>) {
        self.categories = categories;
    }

    /// `StockTickerBlockEntity.read`: a category that isn't a filter item is dropped
    /// (`categories.removeIf(stack -> !stack.isEmpty() && !(item instanceof FilterItem))`).
    pub fn retain_filters(&mut self, is_filter: impl Fn(&F) -> bool) {
        self.categories.retain(|c| c.as_ref().is_none_or(&is_filter));
    }

    /// `StockTickerBlockEntity.addToTooltip`'s list: what the shop was paid, the most first.
    pub fn payments_by_count(&self) -> Vec<Big<K>> {
        let mut summary = Summary::default();
        for (key, count) in self.received_payments.iter().flatten() {
            summary.add(key, i64::from(*count));
        }
        summary.stacks_by_count()
    }
}

/// `StockTickerBlockEntity.isKeeperPresent`: a shopkeeper sits at a seat next to the ticker (in
/// its row or the one below), or on a heater beside it (in its own row only).
pub fn keeper_present(
    ticker: Pos,
    occupied_seat_at: &dyn Fn(Pos) -> bool,
    heater_at: &dyn Fn(Pos) -> bool,
) -> bool {
    for y_offset in [0, 1] {
        for side in [Dir::North, Dir::South, Dir::West, Dir::East] {
            let n = side.normal();
            let seat = [ticker[0] + n[0], ticker[1] - y_offset + n[1], ticker[2] + n[2]];
            if occupied_seat_at(seat) {
                return true;
            }
            if y_offset == 0 && heater_at(seat) {
                return true;
            }
        }
    }
    false
}

/// `StockTickerInteractionHandler.getStockTickerPosition`: the ticker a seated shopkeeper (at
/// `entity`'s block) serves: the one stock ticker beside it, in its row or the one above; none
/// when there are none or several. (The caller checks that the entity rides a seat, is a living
/// one, and isn't a package.)
pub fn ticker_position(entity: Pos, is_ticker: &dyn Fn(Pos) -> bool) -> Option<Pos> {
    let mut stations = 0;
    let mut target = None;
    for d in [Dir::South, Dir::West, Dir::North, Dir::East] {
        for y in [0, 1] {
            let n = d.normal();
            let pos = [entity[0] + n[0], entity[1] + y, entity[2] + n[2]];
            if !is_ticker(pos) {
                continue;
            }
            target = Some(pos);
            stations += 1;
        }
    }
    if stations == 1 { target } else { None }
}

// --- the order packet --------------------------------------------------------------------------------

/// What `PackageOrderRequestPacket.applySettings` does with an order a player sent.
#[derive(Clone, Debug, PartialEq)]
pub enum OrderRequest<K> {
    /// `encodeRequester`: the order is saved on the Redstone Requester or Table Cloth in the
    /// player's hand (`RedstoneRequesterBlock.programRequester`), with a sound when it isn't
    /// empty; nothing is sent.
    Program { order: Context<K>, address: String },
    /// The order is broadcast with `RequestType.PLAYER` (with the request sound, the advancement
    /// and the wifi effect when it isn't empty).
    Broadcast { order: Context<K>, address: String },
}

/// `PackageOrderRequestPacket.applySettings`.
pub fn apply_order_packet<K>(order: Context<K>, address: String, encode_requester: bool) -> OrderRequest<K> {
    if encode_requester {
        OrderRequest::Program { order, address }
    } else {
        OrderRequest::Broadcast { order, address }
    }
}

// --- a sale --------------------------------------------------------------------------------------------

/// `ItemHandlerHelper.insertItemStacked` into a list of slots of `slot_limit`: onto stacks of
/// the same item first, then into empty slots. Returns what is left.
pub fn insert_stacked<K: PartialEq + Clone>(
    slots: &mut [Option<Big<K>>],
    key: &K,
    count: u32,
    max_stack: u32,
    slot_limit: u32,
) -> u32 {
    let limit = max_stack.min(slot_limit).max(1);
    let mut left = count;
    if max_stack > 1 {
        for (held, have) in slots.iter_mut().flatten() {
            if left == 0 {
                return 0;
            }
            if held == key {
                let moved = limit.saturating_sub(*have).min(left);
                *have += moved;
                left -= moved;
            }
        }
    }
    for slot in slots.iter_mut() {
        if left == 0 {
            break;
        }
        if slot.is_none() {
            let moved = limit.min(left);
            *slot = Some((key.clone(), moved));
            left -= moved;
        }
    }
    left
}

/// What a Shopping List handed to the Stock Ticker's shopkeeper came to
/// (`StockTickerInteractionHandler.interactWithShop`).
#[derive(Clone, Debug, PartialEq)]
pub enum Sale<K> {
    /// "stock_keeper.wrong_network": the list is for another shop.
    WrongNetwork,
    /// "stock_keeper.stock_level_too_low".
    StockTooLow,
    /// "stock_keeper.cash_register_full".
    CashRegisterFull,
    /// "stock_keeper.too_broke": the player doesn't have the price.
    TooBroke,
    /// The sale went through: the list is used up (the player's hand is emptied).
    Sold {
        /// `(slot, count)` taken from the player's inventory, in slot order.
        removed: Vec<(usize, u32)>,
        /// The ticker's payment slots after the payments were put in.
        received: Vec<Option<Big<K>>>,
        /// The order to broadcast with `RequestType.PLAYER`, to the list's address
        /// (`ShoppingListItem.getAddress`). Its result is not checked, as in Create.
        order: Context<K>,
    },
}

/// `StockTickerInteractionHandler.interactWithShop`: `ticker_network` is the ticker's frequency
/// and `list_network` the list's (`shopNetwork`); `order` and `payment` are the list's baked
/// entries (`ShoppingList.bakeEntries`: the goods, and the price); `recent` is the ticker's
/// summary (`getRecentSummary`, after `getAccurateSummary` refreshed it); `received` the cash
/// register's slots; `player` the player's 36 main inventory slots.
pub fn sell<K: PartialEq + Clone>(
    ticker_network: Freq,
    list_network: Freq,
    order: &Summary<K>,
    payment: &Summary<K>,
    recent: &Summary<K>,
    received: &[Option<Big<K>>],
    player: &[Option<Big<K>>],
    max_stack: &dyn Fn(&K) -> u32,
) -> Sale<K> {
    if ticker_network != list_network {
        return Sale::WrongNetwork;
    }
    let goods = order.stacks_by_count();

    // Check stock levels.
    for (key, count) in &goods {
        if recent.count_of(key) < *count {
            return Sale::StockTooLow;
        }
    }

    // Check space in the stock ticker: whole slots, not counting stacks it could add to.
    let mut occupied_slots: i64 = 0;
    for (key, count) in payment.stacks_by_count() {
        occupied_slots += i64::from(count.div_ceil(max_stack(&key).max(1)));
    }
    occupied_slots -= received.iter().filter(|s| s.is_none()).count() as i64;
    if occupied_slots > 0 {
        return Sale::CashRegisterFull;
    }

    // Transfer the payment to the stock ticker (first simulated, then for real).
    let mut tally = payment.copy();
    let mut removed: Vec<(usize, u32)> = Vec::new();
    for (index, slot) in player.iter().take(PLAYER_SLOTS).enumerate() {
        let Some((key, have)) = slot else {
            continue;
        };
        let in_tally = tally.count_of(key);
        if in_tally == 0 {
            continue;
        }
        let to_remove = (*have).min(in_tally);
        tally.add(key, -i64::from(to_remove));
        removed.push((index, to_remove));
    }
    if tally.total_count != 0 {
        return Sale::TooBroke;
    }
    let mut received_after = received.to_vec();
    for (index, count) in &removed {
        let Some((key, _)) = &player[*index] else {
            continue;
        };
        insert_stacked(&mut received_after, key, *count, max_stack(key), 64);
    }

    Sale::Sold { removed, received: received_after, order: Context { stacks: goods, crafts: Vec::new() } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::logistics_network::{GlobalPos, PackagerView};

    fn summary(stacks: &[(&'static str, u32)]) -> Summary<&'static str> {
        let mut s = Summary::default();
        for (k, c) in stacks {
            s.add(k, i64::from(*c));
        }
        s
    }

    fn max(key: &&str) -> u32 {
        if *key == "egg" { 16 } else { 64 }
    }

    #[test]
    fn an_order_holds_nine_items_and_no_more_than_the_stock() {
        let mut order: OrderList<&str> = OrderList::default();
        // Left click: one at a time, up to the stock (3 iron).
        assert_eq!(order.click(&"iron", 3, false, false, 1), Change::Added);
        assert_eq!(order.click(&"iron", 3, false, false, 1), Change::Counted);
        // Shift takes a stack's worth but the stock caps it at 3.
        assert_eq!(order.click(&"iron", 3, false, false, OrderList::<&str>::click_transfer(true, false, 64)), Change::Counted);
        assert_eq!(order.items, vec![("iron", 3)]);
        assert_eq!(order.click(&"iron", 3, false, false, 1), Change::None);
        // Control takes ten.
        assert_eq!(OrderList::<&str>::click_transfer(false, true, 64), 10);
        assert_eq!(order.click(&"gold", 50, false, false, 10), Change::Added);
        assert_eq!(order.items[1], ("gold", 10));
        // Scrolling: ceil of the scroll, ten times that with control; down takes off.
        assert_eq!(order.scroll(&"gold", 50, 1.0, true), Change::Counted);
        assert_eq!(order.items[1], ("gold", 20));
        assert_eq!(order.scroll(&"gold", 50, -1.5, false), Change::Counted);
        assert_eq!(order.items[1], ("gold", 18));
        // The right button on the stock list, or a click on the order, takes off; to zero removes.
        assert_eq!(order.click(&"iron", 3, true, false, 1), Change::Counted);
        assert_eq!(order.click(&"iron", 3, false, true, 5), Change::Removed);
        assert_eq!(order.items, vec![("gold", 18)]);
        // Removing what isn't there does nothing.
        assert_eq!(order.click(&"coal", 9, true, false, 1), Change::None);
        // Nine different items fill it.
        for (i, name) in ["a", "b", "c", "d", "e", "f", "g", "h"].iter().enumerate() {
            assert_eq!(order.click(name, 5, false, false, 1), Change::Added, "{i}");
        }
        assert_eq!(order.items.len(), ORDER_ENTRIES);
        assert_eq!(order.click(&"i", 5, false, false, 1), Change::None);
        assert_eq!(order.items.len(), ORDER_ENTRIES);
    }

    #[test]
    fn a_stale_order_is_cut_to_the_stock_when_sent() {
        let mut order: OrderList<&str> = OrderList { items: vec![("iron", 30), ("gold", 5), ("coal", 2)] };
        let snapshot = summary(&[("iron", 12), ("coal", 9)]);
        order.revalidate(Some(&snapshot));
        assert_eq!(order.items, vec![("iron", 12), ("coal", 2)]);
        assert_eq!(order.to_order().stacks, vec![("iron", 12), ("coal", 2)]);
        order.revalidate(None);
        assert!(order.items.is_empty());
    }

    #[test]
    fn a_stock_snapshot_is_sent_in_lists_of_a_hundred() {
        let mut s: Summary<u32> = Summary::default();
        for i in 0..250u32 {
            s.add(&i, i64::from(i) + 1);
        }
        let parts = divide_for_sending(&s);
        assert_eq!(parts.iter().map(|(end, list)| (*end, list.len())).collect::<Vec<_>>(), vec![(false, 100), (false, 100), (true, 50)]);
        assert_eq!(parts[0].1[0], (249, 250), "the most first");
        // Exactly a hundred is one last list.
        let mut hundred: Summary<u32> = Summary::default();
        for i in 0..100u32 {
            hundred.add(&i, 1);
        }
        let parts = divide_for_sending(&hundred);
        assert_eq!(parts.iter().map(|(end, list)| (*end, list.len())).collect::<Vec<_>>(), vec![(true, 100)]);
        assert_eq!(divide_for_sending(&Summary::<u32>::default()), vec![(true, Vec::new())]);
    }

    #[test]
    fn categories_take_what_their_filter_passes_in_turn() {
        let snapshot = vec![("iron_ingot", 5), ("iron_nugget", 9), ("gold_ingot", 1), ("dirt", 40)];
        let categories: Vec<Option<&str>> = vec![Some("ingot"), None, Some("iron")];
        let lists = categorize(&snapshot, &categories, |f, k| k.contains(f));
        assert_eq!(lists.len(), 4);
        assert_eq!(lists[0], vec![("iron_ingot", 5), ("gold_ingot", 1)]);
        assert!(lists[1].is_empty(), "an empty filter takes nothing");
        assert_eq!(lists[2], vec![("iron_nugget", 9)], "what the first took is no longer there");
        assert_eq!(lists[3], vec![("dirt", 40)]);
    }

    #[test]
    fn the_lock_and_hidden_categories() {
        let mut networks: Networks<&'static str> = Networks::default();
        networks.link_added(1, GlobalPos { dimension: "overworld".into(), pos: [0, 0, 0] }, Some(10));
        let mut ticker: Ticker<&'static str, &'static str> = Ticker::default();
        // Open: the owner may lock it, a stranger may not.
        assert_eq!(ticker.access(&networks, 1, 10), Access::Open { show_lock_option: true, locked: false });
        assert_eq!(ticker.access(&networks, 1, 20), Access::Open { show_lock_option: false, locked: false });
        networks.set_locked(1, 10, true);
        assert_eq!(ticker.access(&networks, 1, 20), Access::Locked);
        assert_eq!(ticker.access(&networks, 1, 10), Access::Open { show_lock_option: true, locked: true });
        // Hidden categories: stored per player, empty clears.
        assert!(ticker.set_hidden_categories(10, vec![0, -1]));
        assert_eq!(ticker.hidden_categories(10), &[0, -1]);
        assert!(ticker.hidden_categories(20).is_empty());
        assert!(!ticker.set_hidden_categories(10, vec![]));
        assert!(ticker.hidden_categories(10).is_empty());
        // Reading drops what isn't a filter.
        ticker.set_categories(vec![Some("filter"), Some("dirt"), None]);
        ticker.retain_filters(|f| *f == "filter");
        assert_eq!(ticker.categories, vec![Some("filter"), None]);
        // The summary's link count is kept and reported when it changes.
        assert!(ticker.note_summary(3, false));
        assert!(!ticker.note_summary(3, false));
        assert!(!ticker.note_summary(4, true));
    }

    #[test]
    fn a_ticker_remembers_the_address_it_last_sent_to() {
        struct Fixed;
        impl Random for Fixed {
            fn next_int(&mut self) -> i32 {
                9
            }
            fn next_below(&mut self, _: usize) -> usize {
                0
            }
        }
        let mut ticker: Ticker<&'static str, ()> = Ticker::default();
        let links = vec![LinkView {
            id: 1,
            power: 0,
            packager: Some(PackagerView { id: 5, inventory: Some(1), targets_ignored: false, queued: 0, available: summary(&[("iron", 9)]) }),
        }];
        let order = Context { stacks: vec![("iron", 4)], crafts: Vec::new() };
        let sent = ticker.broadcast_package_request(&links, RequestType::Player, &order, "Workshop", &mut Fixed);
        assert!(sent.accepted);
        assert_eq!(ticker.previously_used_address, "Workshop");
        // Refused (an empty order) still remembers.
        let refused = ticker.broadcast_package_request(&links, RequestType::Player, &Context::default(), "Home", &mut Fixed);
        assert!(!refused.accepted);
        assert_eq!(ticker.previously_used_address, "Home");
    }

    #[test]
    fn a_keeper_is_found_at_a_seat_or_a_heater() {
        let ticker = [10, 64, 10];
        let seat = [11, 64, 10];
        assert!(keeper_present(ticker, &|p| p == seat, &|_| false));
        // A seat one block lower counts; a heater there doesn't.
        let low = [10, 63, 9];
        assert!(keeper_present(ticker, &|p| p == low, &|_| false));
        assert!(!keeper_present(ticker, &|_| false, &|p| p == low));
        assert!(keeper_present(ticker, &|_| false, &|p| p == seat));
        assert!(!keeper_present(ticker, &|p| p == [12, 64, 10], &|_| false));
        // The shopkeeper's ticker: exactly one beside them.
        let keeper = [5, 64, 5];
        assert_eq!(ticker_position(keeper, &|p| p == [5, 65, 6]), Some([5, 65, 6]));
        assert_eq!(ticker_position(keeper, &|_| false), None);
        assert_eq!(ticker_position(keeper, &|p| p == [5, 64, 6] || p == [4, 64, 5]), None);
    }

    #[test]
    fn a_recipe_is_laid_out_on_the_crafting_grid() {
        let planks = |k: &&str| *k == "planks";
        let stick = |k: &&str| *k == "stick";
        // The stick recipe: two planks above each other, 1 wide and 2 high.
        let ingredients: Vec<Ingredient<&str>> = vec![Some(&planks), Some(&planks)];
        let inputs = [("planks", 4), ("stick", 1)];
        let pattern = recipe_pattern(Shape::Shaped { width: 1, height: 2 }, &ingredients, &inputs, false, &"");
        let names: Vec<&str> = pattern.iter().map(|p| p.0).collect();
        // `width == 1` puts one empty first; each row of the 1-wide recipe is padded by two.
        assert_eq!(names, vec!["", "planks", "", "", "planks", "", "", "", ""]);
        // A shapeless two-ingredient recipe: 2 wide, 1 high: three empties up front, rows padded.
        let ingredients: Vec<Ingredient<&str>> = vec![Some(&planks), Some(&stick)];
        let pattern = recipe_pattern(Shape::Shapeless, &ingredients, &inputs, false, &"");
        let names: Vec<&str> = pattern.iter().map(|p| p.0).collect();
        assert_eq!(names, vec!["", "", "", "planks", "stick", "", "", "", ""]);
        assert_eq!(pattern.len(), 9);
    }

    #[test]
    fn recipes_in_an_order_are_split_by_what_each_craft_uses() {
        let planks = |k: &&str| *k == "planks";
        let stick = |k: &&str| *k == "stick";
        // Two crafts of a recipe of 2 planks + 1 stick (shapeless): ordered 4 planks and 2 sticks.
        let recipe = OrderedRecipe {
            shape: Shape::Shapeless,
            ingredients: vec![Some(&planks as &dyn Fn(&&str) -> bool), Some(&planks), Some(&stick)],
            output_count: 1,
            count: 2,
        };
        let ordered = [("planks", 4), ("stick", 2)];
        let crafts = craft_entries(&[recipe], &ordered, &"");
        // Computed from `sendIt`: the pattern is [planks, planks, stick] and six empties (the
        // pattern's own use of the ordered stacks is on a copy); two crafts of it fit what
        // was ordered, so one entry of count 2.
        assert_eq!(crafts.len(), 1);
        assert_eq!(crafts[0].count, 2);
        let names: Vec<&str> = crafts[0].pattern.iter().map(|p| p.0).collect();
        assert_eq!(&names[..4], &["planks", "planks", "stick", ""]);
        assert_eq!(names.len(), 9);

        // Only 3 planks were ordered for two crafts: the first craft takes the pattern as it is,
        // and the second can only be laid out from what is left (1 plank, 1 stick).
        let recipe = OrderedRecipe {
            shape: Shape::Shapeless,
            ingredients: vec![Some(&planks as &dyn Fn(&&str) -> bool), Some(&planks), Some(&stick)],
            output_count: 1,
            count: 2,
        };
        let crafts = craft_entries(&[recipe], &[("planks", 3), ("stick", 2)], &"");
        assert_eq!(crafts.iter().map(|c| c.count).collect::<Vec<_>>(), vec![1, 1]);
        assert_eq!(crafts[1].pattern.iter().take(3).map(|p| p.0).collect::<Vec<_>>(), vec!["planks", "", "stick"]);

        // Orders whose crafts use up the ordered stacks exactly match.
        let context = Context { stacks: vec![("planks", 4), ("stick", 2)], crafts: vec![Craft { pattern: vec![("planks", 2), ("stick", 1)], count: 2 }] };
        assert!(context.ordered_stacks_match_ordered_recipes());
        let short = Context { stacks: vec![("planks", 5), ("stick", 2)], crafts: context.crafts.clone() };
        assert!(!short.ordered_stacks_match_ordered_recipes());
        assert!(!Context { stacks: vec![("planks", 4)], crafts: Vec::new() }.ordered_stacks_match_ordered_recipes());
    }

    #[test]
    fn the_order_packet_programs_or_sends() {
        let order = Context { stacks: vec![("iron", 4)], crafts: Vec::new() };
        assert_eq!(
            apply_order_packet(order.clone(), "Home".into(), true),
            OrderRequest::Program { order: order.clone(), address: "Home".into() }
        );
        assert_eq!(
            apply_order_packet(order.clone(), "Home".into(), false),
            OrderRequest::Broadcast { order, address: "Home".into() }
        );
    }

    fn inventory(items: &[(usize, &'static str, u32)]) -> Vec<Option<Big<&'static str>>> {
        let mut slots: Vec<Option<Big<&'static str>>> = (0..PLAYER_SLOTS).map(|_| None).collect();
        for &(slot, key, count) in items {
            slots[slot] = Some((key, count));
        }
        slots
    }

    fn register() -> Vec<Option<Big<&'static str>>> {
        (0..PAYMENT_SLOTS).map(|_| None).collect()
    }

    #[test]
    fn a_sale_takes_the_price_from_the_player_and_sends_the_goods() {
        // Two purchases of 3 cogwheels for 5 gold nuggets each: 6 cogs for 10 nuggets.
        let order = summary(&[("cog", 6)]);
        let payment = summary(&[("nugget", 10)]);
        let stock = summary(&[("cog", 20)]);
        // The player has 7 + 9 nuggets in slots 2 and 5, and other things.
        let player = inventory(&[(0, "dirt", 30), (2, "nugget", 7), (5, "nugget", 9)]);
        let sale = sell(1, 1, &order, &payment, &stock, &register(), &player, &max);
        match sale {
            Sale::Sold { removed, received, order } => {
                // 7 from slot 2, then the 3 still owed from slot 5.
                assert_eq!(removed, vec![(2, 7), (5, 3)]);
                assert_eq!(received[0], Some(("nugget", 10)));
                assert!(received[1..].iter().all(Option::is_none));
                assert_eq!(order.stacks, vec![("cog", 6)]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_sale_is_refused_for_the_wrong_shop_too_little_stock_no_room_or_no_money() {
        let order = summary(&[("cog", 6)]);
        let payment = summary(&[("nugget", 10)]);
        let stock = summary(&[("cog", 20)]);
        let rich = inventory(&[(0, "nugget", 64)]);
        assert_eq!(sell(1, 2, &order, &payment, &stock, &register(), &rich, &max), Sale::WrongNetwork);
        assert_eq!(sell(1, 1, &order, &payment, &summary(&[("cog", 5)]), &register(), &rich, &max), Sale::StockTooLow);
        // The cash register: 27 full slots of other things leave no room for one more stack.
        let mut full = register();
        for slot in full.iter_mut() {
            *slot = Some(("junk", 64));
        }
        assert_eq!(sell(1, 1, &order, &payment, &stock, &full, &rich, &max), Sale::CashRegisterFull);
        // Not counting stacks it could add to: 26 full slots and one holding nuggets still
        // refuse a price of one more stack ... but an empty slot makes room.
        let mut nearly = full.clone();
        nearly[26] = None;
        assert!(matches!(sell(1, 1, &order, &payment, &stock, &nearly, &rich, &max), Sale::Sold { .. }));
        // 9 nuggets can't pay 10.
        assert_eq!(sell(1, 1, &order, &payment, &stock, &register(), &inventory(&[(0, "nugget", 9)]), &max), Sale::TooBroke);
        // A price of 2 stacks (eggs stack to 16: 20 eggs are 2 slots) in a register with one free slot.
        let eggs = summary(&[("egg", 20)]);
        let mut one_free = full.clone();
        one_free[0] = None;
        assert_eq!(
            sell(1, 1, &order, &eggs, &stock, &one_free, &inventory(&[(0, "egg", 20)]), &max),
            Sale::CashRegisterFull
        );
    }
}
