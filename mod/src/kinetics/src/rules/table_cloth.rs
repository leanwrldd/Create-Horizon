//! Table Cloths and the Shopping List (`TableClothBlock`, `TableClothBlockEntity`,
//! `TableClothFilteringBehaviour`, `ShoppingListItem`): a cloth programmed from a Stock Ticker
//! (`AutoRequestData`, `rules::redstone_requester`) is a shop: its price tag is the item and
//! amount a buyer pays per order, and each click adds one order to the buyer's Shopping List.
//! The list is handed to the shopkeeper at the Stock Ticker, which takes the payment and sends
//! the goods (`rules::stock_ticker::sell`). A cloth without a request holds up to four items
//! for show.
//!
//! No game types: `K` is an item with its components. What the cloth reads (the ticker's
//! summary, whether a shopkeeper sits at it, the other cloths a list names, the buyer's hotbar)
//! is plain input; what it decides (the new list, which hotbar lists to take, the refusal) is
//! plain output.
//!
//! Known differences from Create, for the README:
//! - A list's purchases key a cloth by `BlockPos` with the Y value "pixel perfect (x16)":
//!   [`Purchase::cloth`] is whatever position the caller keys its cloths by.
//! - The cloth's placement helper (placing the next cloth beside it, `PlacementHelper`),
//!   its occlusion and shape, and the render lists are the glue's.
//! - The client's `getStockLevelForTrade` (a snapshot the ticker last sent, refreshed when 15
//!   ticks old) is the caller's: [`stock_level_for_trade`] takes the summary it was given.

use super::package::{Big, Summary};
use super::redstone_requester::AutoRequestData;
use crate::{Dir, Pos};

/// `TableClothBlockEntity`: the most items a cloth holds for show (`manuallyAddedItems.size() >= 4`).
pub const MANUAL_ITEMS: usize = 4;
/// `TableClothFilteringBehaviour.createBoard`: the price's largest setting.
pub const MAX_PRICE: i32 = 100;
/// `TableClothBlockEntity.use`'s hotbar scan for earlier lists: `for (int i = 0; i < 9; i++)`.
pub const HOTBAR: usize = 9;

/// `ShoppingListItem.ShoppingList`'s `IntAttached<BlockPos>`: how many orders from a cloth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Purchase {
    pub cloth: Pos,
    pub count: i32,
}

/// `ShoppingListItem.ShoppingList`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShoppingList {
    pub purchases: Vec<Purchase>,
    /// `shopOwner`.
    pub shop_owner: u128,
    /// `shopNetwork`: the Stock Ticker's frequency.
    pub shop_network: u128,
}

/// A cloth as a list's baking reads it (`TableClothBlockEntity` at a purchase's position).
#[derive(Clone, Debug, PartialEq)]
pub struct ClothView<K> {
    /// `getPaymentItem()`: none when no price is set.
    pub payment_item: Option<K>,
    /// `getPaymentAmount()`.
    pub payment_amount: i32,
    /// `requestData.encodedRequest().stacks()`.
    pub goods: Vec<Big<K>>,
}

impl ShoppingList {
    /// `getPurchases(clothPos)`.
    pub fn purchases_from(&self, cloth: Pos) -> i32 {
        self.purchases.iter().find(|p| p.cloth == cloth).map_or(0, |p| p.count)
    }

    /// `Mutable.addPurchases`.
    pub fn add_purchases(&mut self, cloth: Pos, amount: i32) {
        match self.purchases.iter_mut().find(|p| p.cloth == cloth) {
            Some(p) => p.count += amount,
            None => self.purchases.push(Purchase { cloth, count: amount }),
        }
    }

    /// `bakeEntries(level, clothPosToIgnore)`: `(goods, payment)` of the whole list, each
    /// purchase's cloth read by `cloth_at` (a cloth that is gone is skipped). A cloth without a
    /// price adds no payment.
    pub fn bake_entries<K: PartialEq + Clone>(
        &self,
        ignore: Option<Pos>,
        cloth_at: &dyn Fn(Pos) -> Option<ClothView<K>>,
    ) -> (Summary<K>, Summary<K>) {
        let mut input: Summary<K> = Summary::default();
        let mut output: Summary<K> = Summary::default();
        for purchase in &self.purchases {
            if ignore == Some(purchase.cloth) {
                continue;
            }
            let Some(cloth) = cloth_at(purchase.cloth) else {
                continue;
            };
            if let Some(item) = &cloth.payment_item {
                input.add(item, i64::from(cloth.payment_amount) * i64::from(purchase.count));
            }
            for (key, count) in &cloth.goods {
                output.add(key, i64::from(*count) * i64::from(purchase.count));
            }
        }
        (output, input)
    }
}

/// `TableClothBlockEntity.getStockLevelForTrade`: how many orders of the cloth's goods the
/// network can fill: the smallest quotient over the goods of what the summary holds, less what
/// the buyer's other purchases (`other_purchases`, baked without this cloth) already take, over
/// the count per order. `i32::MAX` when the cloth sells nothing; none summary is 0.
pub fn stock_level_for_trade<K: PartialEq + Clone>(
    goods: &[Big<K>],
    summary: Option<&Summary<K>>,
    other_purchases: Option<&Summary<K>>,
) -> i64 {
    let Some(summary) = summary else {
        return 0;
    };
    let mut smallest = i64::from(i32::MAX);
    for (key, count) in goods {
        if *count == 0 {
            continue;
        }
        let taken = other_purchases.map_or(0, |o| o.count_of(key));
        let quotient = (i64::from(summary.count_of(key)) - i64::from(taken)) / i64::from(*count);
        smallest = smallest.min(quotient);
    }
    smallest
}

// --- using a cloth --------------------------------------------------------------------------------------

/// What a click on a Table Cloth block with an item does before the block entity is asked
/// (`TableClothBlock.useItemOn`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockUse {
    /// The default block interaction (the item's own use).
    PassToDefault,
    /// A held cloth is placed beside it (the placement helper).
    PlaceNext,
    /// The cloth gets its block entity (`HAS_BE`) and then handles the click.
    Use { gives_block_entity: bool },
}

/// `TableClothBlock.useItemOn`, on the server.
pub fn block_use(
    hit_face_down: bool,
    may_build: bool,
    held_is_cloth: bool,
    sneaking: bool,
    held_empty: bool,
    has_block_entity: bool,
) -> BlockUse {
    if hit_face_down || !may_build {
        return BlockUse::PassToDefault;
    }
    if held_is_cloth {
        return if sneaking { BlockUse::PassToDefault } else { BlockUse::PlaceNext };
    }
    if (sneaking || held_empty) && !has_block_entity {
        return BlockUse::PassToDefault;
    }
    BlockUse::Use { gives_block_entity: !has_block_entity }
}

/// `TableClothFilteringBehaviour.mayInteract`: the owner (or anyone, when there is none) sets the
/// price.
pub fn may_set_price(owner: Option<u128>, player: u128) -> bool {
    owner.is_none_or(|o| o == player)
}

/// `TableClothFilteringBehaviour`'s predicate: a price is any item but a filter or a Shopping List.
pub fn may_be_price(is_filter_item: bool, is_shopping_list: bool) -> bool {
    !is_filter_item && !is_shopping_list
}

/// A Table Cloth's state (`TableClothBlockEntity`).
#[derive(Clone, Debug, PartialEq)]
pub struct Cloth<K> {
    /// `requestData`.
    pub request_data: AutoRequestData<K>,
    /// `manuallyAddedItems`.
    pub manually_added_items: Vec<K>,
    /// `owner`.
    pub owner: Option<u128>,
    /// `facing`.
    pub facing: Dir,
    /// The price tag's item (`priceTag.getFilter()`) and count (`priceTag.count`, starting at 1).
    pub price_item: Option<K>,
    pub price_count: i32,
}

impl<K> Default for Cloth<K> {
    fn default() -> Self {
        Self {
            request_data: AutoRequestData::default(),
            manually_added_items: Vec::new(),
            owner: None,
            facing: Dir::South,
            price_item: None,
            price_count: 1,
        }
    }
}

/// What a click on a cloth that isn't a shop did (`TableClothBlockEntity.use`).
#[derive(Clone, Debug, PartialEq)]
pub enum DisplayUse<K> {
    /// Nothing (an empty hand on an empty cloth, or four items already).
    Nothing,
    /// The last item is handed to the player; with it gone, the cloth loses its block entity
    /// (unless a computer is attached).
    Took { item: K, remove_block_entity: bool },
    /// One of the held item was put on the cloth, which now faces the player's way round.
    Put,
}

impl<K: PartialEq + Clone> Cloth<K> {
    /// `isShop`.
    pub fn is_shop(&self) -> bool {
        !self.request_data.encoded_request.is_empty()
    }

    /// `getPaymentAmount`: 1 without a price item.
    pub fn payment_amount(&self) -> i32 {
        if self.price_item.is_none() { 1 } else { self.price_count }
    }

    /// `getPaymentItem`, `getPaymentAmount` and the goods, as a list's baking reads them.
    pub fn view(&self) -> ClothView<K> {
        ClothView {
            payment_item: self.price_item.clone(),
            payment_amount: self.payment_amount(),
            goods: self.request_data.encoded_request.stacks.clone(),
        }
    }

    /// `TableClothBlockEntity.setPlacedBy` through `TableClothBlock.setPlacedBy`: a cloth placed
    /// from a programmed item is a shop owned by the placer, facing back at them.
    pub fn placed(data: AutoRequestData<K>, player: u128, player_facing: Dir) -> Self {
        Self { request_data: data, owner: Some(player), facing: player_facing.opposite(), ..Self::default() }
    }

    /// `TableClothFilteringBehaviour.setFilter`: the price item changes, the count stays.
    pub fn set_price_item(&mut self, item: Option<K>) {
        self.price_item = item;
    }

    /// `TableClothFilteringBehaviour.setValueSettings`: at least 1. Returns whether it changed.
    pub fn set_price_count(&mut self, value: i32) -> bool {
        let count = value.max(1);
        if count == self.price_count {
            return false;
        }
        self.price_count = count;
        true
    }

    /// `TableClothBlockEntity.use` on a cloth that isn't a shop: an empty hand takes the last
    /// item, a held item is added (up to four) and the cloth turns to face away from the
    /// player's direction (`player.getDirection().getOpposite()`).
    pub fn use_display(&mut self, held: Option<&K>, player_facing: Dir, computer_attached: bool) -> DisplayUse<K> {
        let Some(held) = held else {
            let Some(item) = self.manually_added_items.pop() else {
                return DisplayUse::Nothing;
            };
            let remove = self.manually_added_items.is_empty() && !computer_attached;
            return DisplayUse::Took { item, remove_block_entity: remove };
        };
        if self.manually_added_items.len() >= MANUAL_ITEMS {
            return DisplayUse::Nothing;
        }
        self.manually_added_items.push(held.clone());
        self.facing = player_facing.opposite();
        DisplayUse::Put
    }
}

/// Why a click on a shop was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// "stock_keeper.shopping_list_empty_hand": holding something that isn't a list.
    HoldingOtherItem,
    /// "stock_keeper.no_price_set".
    NoPriceSet,
    /// "stock_keeper.keeper_missing": no shopkeeper at the ticker.
    KeeperMissing,
    /// "stock_keeper.out_of_stock": the earlier list, if any, is handed back.
    OutOfStock,
}

/// What a click on a shop cloth did to the buyer's list (`TableClothBlockEntity.useShop`).
#[derive(Clone, Debug, PartialEq)]
pub enum ShopUse {
    Refused {
        why: Refusal,
        /// The earlier list, handed back (to the hand when it is empty, else the inventory), only
        /// when out of stock.
        give_back: Option<ShoppingList>,
    },
    /// The buyer gets this list (to the hand when it is empty, else the inventory).
    Listed {
        list: ShoppingList,
        address: String,
        /// "stock_keeper.limited_stock": the cloth's purchases were cut to the stock.
        limited_stock: bool,
        /// "stock_keeper.use_list_to_add_purchases" and the page-turn sound: a new list.
        started_new_list: bool,
    },
}

/// What `useShop` reads.
pub struct ShopClick<'a, K> {
    pub cloth_pos: Pos,
    pub cloth: &'a Cloth<K>,
    /// The item in the main hand: none when empty, `Some(None)` for another item,
    /// `Some(Some(list))` for a Shopping List.
    pub held: Option<Option<&'a ShoppingList>>,
    /// The Shopping Lists in the hotbar, by slot, which the click takes out of the inventory
    /// (`setItem(i, EMPTY)`), the last one being the "previous" list.
    pub hotbar_lists: &'a [(usize, ShoppingList)],
    /// The ticker at `requestData.targetOffset + pos`: its frequency when it is a Stock Ticker
    /// with a shopkeeper present.
    pub keeper_network: Option<u128>,
    /// The ticker's summary (`getRecentSummary`), none without a Stock Ticker.
    pub summary: Option<&'a Summary<K>>,
    /// The other cloths a list names.
    pub cloth_at: &'a dyn Fn(Pos) -> Option<ClothView<K>>,
}

/// `TableClothBlockEntity.useShop`. The hotbar lists are taken out of the inventory whatever
/// the result (the caller empties `cleared_slots`); an earlier list that doesn't belong to this
/// shop is lost, as in Create.
pub fn use_shop<K: PartialEq + Clone>(click: &ShopClick<'_, K>) -> (Vec<usize>, ShopUse) {
    let mut previous: Option<&ShoppingList> = None;
    let mut add_onto_list = false;
    let cleared: Vec<usize> = click.hotbar_lists.iter().map(|(slot, _)| *slot).collect();
    for (_, list) in click.hotbar_lists {
        previous = Some(list);
        add_onto_list = true;
    }
    if let Some(Some(list)) = click.held {
        previous = Some(list);
        add_onto_list = true;
    }

    if click.held.is_some() && !add_onto_list {
        return (cleared, ShopUse::Refused { why: Refusal::HoldingOtherItem, give_back: None });
    }
    if click.cloth.price_item.is_none() {
        return (cleared, ShopUse::Refused { why: Refusal::NoPriceSet, give_back: None });
    }

    let other_purchases = previous.map(|l| l.bake_entries(Some(click.cloth_pos), click.cloth_at).0);
    let stock_level = stock_level_for_trade(
        &click.cloth.request_data.encoded_request.stacks,
        click.summary,
        other_purchases.as_ref(),
    );

    let Some(ticker) = click.keeper_network else {
        return (cleared, ShopUse::Refused { why: Refusal::KeeperMissing, give_back: None });
    };
    let owner = click.cloth.owner.unwrap_or_default();

    if stock_level == 0 {
        return (cleared, ShopUse::Refused { why: Refusal::OutOfStock, give_back: previous.cloned() });
    }

    let mut list = ShoppingList { purchases: Vec::new(), shop_owner: owner, shop_network: ticker };
    if add_onto_list {
        let previous = previous.expect("a previous list");
        if click.cloth.owner == Some(previous.shop_owner) && ticker == previous.shop_network {
            list = previous.clone();
        } else {
            add_onto_list = false;
        }
    }

    let mut limited_stock = false;
    let mut started_new_list = false;
    if i64::from(list.purchases_from(click.cloth_pos)) >= stock_level {
        for purchase in &mut list.purchases {
            if purchase.cloth == click.cloth_pos {
                purchase.count = i64::from(purchase.count).min(stock_level) as i32;
            }
        }
        limited_stock = true;
    } else {
        list.add_purchases(click.cloth_pos, 1);
        started_new_list = !add_onto_list;
    }

    let address = click.cloth.request_data.encoded_target_address.clone();
    (cleared, ShopUse::Listed { list, address, limited_stock, started_new_list })
}

/// `ShoppingListItem.use` and `useOn`: sneaking with the list in the main hand throws it away.
pub fn discards_list(main_hand: bool, sneaking: bool) -> bool {
    main_hand && sneaking
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::package::Context;
    use crate::rules::stock_ticker::{Sale, sell};

    const SHOP_OWNER: u128 = 1;
    const TICKER: u128 = 100;

    fn summary(stacks: &[(&'static str, u32)]) -> Summary<&'static str> {
        let mut s = Summary::default();
        for (k, c) in stacks {
            s.add(k, i64::from(*c));
        }
        s
    }

    /// A cloth selling 3 cogwheels for 5 gold nuggets.
    fn cog_shop() -> Cloth<&'static str> {
        let data = AutoRequestData {
            encoded_request: Context { stacks: vec![("cog", 3)], crafts: Vec::new() },
            encoded_target_address: "Shipping".into(),
            ..AutoRequestData::default()
        };
        let mut cloth = Cloth::placed(data, SHOP_OWNER, Dir::North);
        cloth.set_price_item(Some("nugget"));
        cloth.set_price_count(5);
        cloth
    }

    fn click<'a>(
        pos: Pos,
        cloth: &'a Cloth<&'static str>,
        held: Option<Option<&'a ShoppingList>>,
        hotbar: &'a [(usize, ShoppingList)],
        stock: &'a Summary<&'static str>,
        cloth_at: &'a dyn Fn(Pos) -> Option<ClothView<&'static str>>,
    ) -> ShopClick<'a, &'static str> {
        ShopClick {
            cloth_pos: pos,
            cloth,
            held,
            hotbar_lists: hotbar,
            keeper_network: Some(TICKER),
            summary: Some(stock),
            cloth_at,
        }
    }

    #[test]
    fn a_shop_cloth_is_placed_facing_the_buyer_and_prices_are_set_by_its_owner() {
        let cloth = cog_shop();
        assert!(cloth.is_shop());
        assert_eq!(cloth.facing, Dir::South);
        assert_eq!(cloth.owner, Some(SHOP_OWNER));
        assert_eq!(cloth.payment_amount(), 5);
        assert!(may_set_price(cloth.owner, SHOP_OWNER));
        assert!(!may_set_price(cloth.owner, 2));
        assert!(may_set_price(None, 2));
        assert!(!may_be_price(true, false) && !may_be_price(false, true) && may_be_price(false, false));
        // The price counts from 1, and changing the item keeps the count.
        let mut c = cog_shop();
        assert!(!c.set_price_count(5));
        assert!(c.set_price_count(0));
        assert_eq!(c.price_count, 1);
        c.set_price_count(9);
        c.set_price_item(Some("gold"));
        assert_eq!(c.price_count, 9);
        // Without a price item the amount is 1.
        c.set_price_item(None);
        assert_eq!(c.payment_amount(), 1);
        // The block's use.
        assert_eq!(block_use(true, true, false, false, false, true), BlockUse::PassToDefault);
        assert_eq!(block_use(false, false, false, false, false, true), BlockUse::PassToDefault);
        assert_eq!(block_use(false, true, true, false, false, true), BlockUse::PlaceNext);
        assert_eq!(block_use(false, true, true, true, false, true), BlockUse::PassToDefault);
        assert_eq!(block_use(false, true, false, false, true, false), BlockUse::PassToDefault);
        assert_eq!(block_use(false, true, false, false, false, false), BlockUse::Use { gives_block_entity: true });
        assert_eq!(block_use(false, true, false, true, false, true), BlockUse::Use { gives_block_entity: false });
    }

    #[test]
    fn a_click_adds_an_order_to_the_buyers_list_until_the_stock_runs_out() {
        let cloth = cog_shop();
        let pos = [10, 64, 10];
        let stock = summary(&[("cog", 7)]);
        let none = |_: Pos| None;
        // First click, empty hand: a new list with one order.
        let (cleared, first) = use_shop(&click(pos, &cloth, None, &[], &stock, &none));
        assert!(cleared.is_empty());
        let ShopUse::Listed { list, address, limited_stock, started_new_list } = first else { panic!("{first:?}") };
        assert_eq!(list.purchases, vec![Purchase { cloth: pos, count: 1 }]);
        assert_eq!((list.shop_owner, list.shop_network), (SHOP_OWNER, TICKER));
        assert_eq!(address, "Shipping");
        assert!(!limited_stock && started_new_list);
        // Second click, list in hand: 2 orders (6 of the 7 cogs).
        let (_, second) = use_shop(&click(pos, &cloth, Some(Some(&list)), &[], &stock, &none));
        let ShopUse::Listed { list: list2, started_new_list, .. } = second else { panic!() };
        assert_eq!(list2.purchases_from(pos), 2);
        assert!(!started_new_list);
        // Third click: 7 cogs hold only 2 orders of 3, so the list is held at 2: limited stock.
        let (_, third) = use_shop(&click(pos, &cloth, Some(Some(&list2)), &[], &stock, &none));
        let ShopUse::Listed { list: list3, limited_stock, .. } = third else { panic!() };
        assert_eq!(list3.purchases_from(pos), 2);
        assert!(limited_stock);
    }

    #[test]
    fn a_list_in_the_hotbar_is_taken_up_and_another_shops_list_is_lost() {
        let cloth = cog_shop();
        let pos = [10, 64, 10];
        let stock = summary(&[("cog", 30)]);
        let none = |_: Pos| None;
        let mine = ShoppingList { purchases: vec![Purchase { cloth: pos, count: 2 }], shop_owner: SHOP_OWNER, shop_network: TICKER };
        // A list in hotbar slot 4: it is taken out and added onto (a hand holding nothing).
        let hotbar = [(4usize, mine.clone())];
        let (cleared, used) = use_shop(&click(pos, &cloth, None, &hotbar, &stock, &none));
        assert_eq!(cleared, vec![4]);
        let ShopUse::Listed { list, started_new_list, .. } = used else { panic!() };
        assert_eq!(list.purchases_from(pos), 3);
        assert!(!started_new_list);
        // A list from another shop is taken out and not carried over: a fresh list of one.
        let theirs = ShoppingList { purchases: vec![Purchase { cloth: [0, 0, 0], count: 5 }], shop_owner: 9, shop_network: 99 };
        let hotbar = [(1usize, theirs)];
        let (cleared, used) = use_shop(&click(pos, &cloth, None, &hotbar, &stock, &none));
        assert_eq!(cleared, vec![1]);
        let ShopUse::Listed { list, started_new_list, .. } = used else { panic!() };
        assert_eq!(list.purchases, vec![Purchase { cloth: pos, count: 1 }]);
        assert!(started_new_list);
    }

    #[test]
    fn other_purchases_on_the_list_use_up_the_stock_first() {
        // A second cloth sells 4 cogs; the list already holds 1 order of it: 4 cogs are spoken for.
        let cloth = cog_shop();
        let pos = [10, 64, 10];
        let other_pos = [11, 64, 10];
        let mut other = cog_shop();
        other.request_data.encoded_request.stacks = vec![("cog", 4)];
        let other_view = other.view();
        let cloth_at = move |p: Pos| (p == other_pos).then(|| other_view.clone());
        let list = ShoppingList { purchases: vec![Purchase { cloth: other_pos, count: 1 }], shop_owner: SHOP_OWNER, shop_network: TICKER };
        // 10 cogs, 4 taken: (10 - 4) / 3 = 2 orders here.
        let stock = summary(&[("cog", 10)]);
        let (_, a) = use_shop(&click(pos, &cloth, Some(Some(&list)), &[], &stock, &cloth_at));
        let ShopUse::Listed { list: a_list, .. } = a else { panic!() };
        assert_eq!(a_list.purchases_from(pos), 1);
        let (_, b) = use_shop(&click(pos, &cloth, Some(Some(&a_list)), &[], &stock, &cloth_at));
        let ShopUse::Listed { list: b_list, .. } = b else { panic!() };
        let (_, c) = use_shop(&click(pos, &cloth, Some(Some(&b_list)), &[], &stock, &cloth_at));
        let ShopUse::Listed { list: c_list, limited_stock, .. } = c else { panic!() };
        assert_eq!(c_list.purchases_from(pos), 2);
        assert!(limited_stock);
        // `stock_level_for_trade` directly: 7 cogs, 3 per order, 4 spoken for: 1 order.
        let taken = summary(&[("cog", 4)]);
        assert_eq!(stock_level_for_trade(&[("cog", 3)], Some(&summary(&[("cog", 7)])), Some(&taken)), 1);
        assert_eq!(stock_level_for_trade(&[("cog", 3)], None, None), 0);
        // Selling two goods takes the smaller quotient.
        assert_eq!(stock_level_for_trade(&[("cog", 3), ("shaft", 1)], Some(&summary(&[("cog", 30), ("shaft", 2)])), None), 2);
    }

    #[test]
    fn a_click_is_refused_without_a_price_a_keeper_or_stock() {
        let pos = [10, 64, 10];
        let none = |_: Pos| None;
        let stock = summary(&[("cog", 30)]);
        // Holding another item.
        let cloth = cog_shop();
        let (_, r) = use_shop(&click(pos, &cloth, Some(None), &[], &stock, &none));
        assert_eq!(r, ShopUse::Refused { why: Refusal::HoldingOtherItem, give_back: None });
        // No price.
        let mut unpriced = cog_shop();
        unpriced.set_price_item(None);
        let (_, r) = use_shop(&click(pos, &unpriced, None, &[], &stock, &none));
        assert_eq!(r, ShopUse::Refused { why: Refusal::NoPriceSet, give_back: None });
        // No shopkeeper.
        let mut no_keeper = click(pos, &cloth, None, &[], &stock, &none);
        no_keeper.keeper_network = None;
        assert_eq!(use_shop(&no_keeper).1, ShopUse::Refused { why: Refusal::KeeperMissing, give_back: None });
        // Out of stock: the list in hand is given back.
        let empty = summary(&[("cog", 2)]);
        let list = ShoppingList { purchases: vec![], shop_owner: SHOP_OWNER, shop_network: TICKER };
        let (_, r) = use_shop(&click(pos, &cloth, Some(Some(&list)), &[], &empty, &none));
        assert_eq!(r, ShopUse::Refused { why: Refusal::OutOfStock, give_back: Some(list) });
        // A ticker without a summary is out of stock too.
        let mut no_summary = click(pos, &cloth, None, &[], &stock, &none);
        no_summary.summary = None;
        assert_eq!(use_shop(&no_summary).1, ShopUse::Refused { why: Refusal::OutOfStock, give_back: None });
    }

    #[test]
    fn a_sale_at_the_cloth_takes_the_price_and_gives_the_goods() {
        // The buyer clicks twice (2 orders of 3 cogs for 5 nuggets), then hands the list to the
        // shopkeeper with 12 nuggets in the inventory.
        let cloth = cog_shop();
        let pos = [10, 64, 10];
        let stock = summary(&[("cog", 20)]);
        let view = cloth.view();
        let cloth_at = move |p: Pos| (p == pos).then(|| view.clone());
        let mut list = ShoppingList { purchases: vec![], shop_owner: SHOP_OWNER, shop_network: TICKER };
        list.add_purchases(pos, 1);
        list.add_purchases(pos, 1);
        let (goods, payment) = list.bake_entries(None, &cloth_at);
        assert_eq!(goods.stacks, vec![("cog", 6)]);
        assert_eq!(payment.stacks, vec![("nugget", 10)]);
        let mut inventory: Vec<Option<Big<&'static str>>> = (0..36).map(|_| None).collect();
        inventory[3] = Some(("nugget", 12));
        let register: Vec<Option<Big<&'static str>>> = (0..27).map(|_| None).collect();
        let sale = sell(TICKER, list.shop_network, (&goods, &payment), &stock, &register, &inventory, &|_| 64);
        match sale {
            Sale::Sold { removed, received, order } => {
                assert_eq!(removed, vec![(3, 10)], "the price: 10 nuggets");
                assert_eq!(received[0], Some(("nugget", 10)));
                assert_eq!(order.stacks, vec![("cog", 6)], "the goods");
            }
            other => panic!("{other:?}"),
        }
        // The list names a cloth that is gone: skipped.
        let gone = |_: Pos| -> Option<ClothView<&'static str>> { None };
        let (goods, payment) = list.bake_entries(None, &gone);
        assert!(goods.is_empty() && payment.is_empty());
        // And a cloth ignored is left out of the baking.
        let (goods, _) = list.bake_entries(Some(pos), &cloth_at);
        assert!(goods.is_empty());
        assert!(discards_list(true, true) && !discards_list(false, true) && !discards_list(true, false));
    }

    #[test]
    fn a_plain_cloth_holds_four_items() {
        let mut cloth: Cloth<&'static str> = Cloth::default();
        assert!(!cloth.is_shop());
        assert_eq!(cloth.use_display(None, Dir::East, false), DisplayUse::Nothing);
        for item in ["a", "b", "c", "d"] {
            assert_eq!(cloth.use_display(Some(&item), Dir::East, false), DisplayUse::Put);
        }
        assert_eq!(cloth.facing, Dir::West);
        assert_eq!(cloth.use_display(Some(&"e"), Dir::East, false), DisplayUse::Nothing);
        assert_eq!(cloth.use_display(None, Dir::East, false), DisplayUse::Took { item: "d", remove_block_entity: false });
        cloth.manually_added_items = vec!["z"];
        assert_eq!(cloth.use_display(None, Dir::East, true), DisplayUse::Took { item: "z", remove_block_entity: false });
        cloth.manually_added_items = vec!["z"];
        assert_eq!(cloth.use_display(None, Dir::East, false), DisplayUse::Took { item: "z", remove_block_entity: true });
    }
}
