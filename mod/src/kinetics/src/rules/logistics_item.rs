//! What a logistics item carries (`LogisticallyLinkedBlockItem`, `FactoryPanelBlockItem`, `RedstoneRequesterBlockItem`,
//! `TableClothBlockItem`, `RedstoneRequesterBlock.programRequester`): a Stock Link, Stock Ticker, Redstone Requester or Factory Gauge item
//! is "tuned" when it holds a network frequency (Create keeps it in `DataComponents.BLOCK_ENTITY_DATA` under `Freq`); a Redstone
//! Requester or a Table Cloth also carries the order a Stock Ticker programmed it with (`AllDataComponents.AUTO_REQUEST_DATA`).
//!
//! No game types: these are the decisions about which item takes which change and when a click is refused. The glue writes the
//! components (`machines/logistics.rs`).
//!
//! Known differences from Create, for the README:
//! - The frequency is its own registered component (`create:logistics_frequency`, four ints), not the block entity data of the
//!   item: a mod can't write `minecraft:block_entity_data`. `isTuned` is "has it".
//! - `FactoryPanelBlockItem.fixCtrlCopiedStack` (a pick-block copy of a gauge with panel data) isn't needed: a copy carries no data.

/// `AllBlocks.STOCK_LINK`, `STOCK_TICKER`, `REDSTONE_REQUESTER`, `FACTORY_GAUGE`: the items that are `LogisticallyLinkedBlockItem`s.
pub const LINKED_ITEMS: [&str; 4] =
    ["create:stock_link", "create:stock_ticker", "create:redstone_requester", "create:factory_gauge"];

/// The registered component with a network frequency.
pub const FREQ_COMPONENT: &str = "create:logistics_frequency";
/// `AllDataComponents.AUTO_REQUEST_DATA`.
pub const REQUEST_COMPONENT: &str = "create:auto_request_data";

pub fn is_linked_item(item: &str) -> bool {
    LINKED_ITEMS.contains(&item)
}

pub fn is_requester(item: &str) -> bool {
    item == "create:redstone_requester"
}

/// `AllItemTags.TABLE_CLOTHS`: the sixteen colours and the three covers.
pub fn is_table_cloth(item: &str) -> bool {
    item.strip_prefix("create:").is_some_and(|name| name.ends_with("_table_cloth"))
}

/// A change to the stack in a player's hand, queued by a click and written by the next inventory tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Write {
    /// `LogisticallyLinkedBlockItem.assignFrequency`.
    Tune,
    /// `LogisticallyLinkedBlockItem.use` on a tuned item: the frequency is cleared.
    Clear,
    /// `RedstoneRequesterBlock.programRequester`: the order goes onto the item, and a requester also gets the ticker's frequency.
    Program,
}

/// Whether `write` can be made to `item` (`tuned`: it holds a frequency now).
pub fn write_applies(write: Write, item: &str, tuned: bool) -> bool {
    match write {
        Write::Tune => is_linked_item(item),
        Write::Clear => is_linked_item(item) && tuned,
        Write::Program => is_requester(item) || is_table_cloth(item),
    }
}

/// Whether a `Program` write also tunes the item (`isRequester`: the cloth takes the order only).
pub fn program_tunes(item: &str) -> bool {
    is_requester(item)
}

/// `LogisticallyLinkedBlockItem.useOn`: a linked item used (not sneaking) on a block that is on a network takes its frequency; a
/// spectator does nothing.
pub fn tunes_on_click(item: &str, sneaking: bool, spectator: bool, block_is_linked: bool) -> bool {
    is_linked_item(item) && block_is_linked && !sneaking && !spectator
}

/// `FactoryPanelBlockItem.place` and `FactoryPanelBlock.useItemOn`: an untuned gauge can't be placed (or added to a block).
pub fn gauge_refused(item: &str, tuned: bool) -> bool {
    item == "create:factory_gauge" && !tuned
}

/// `LogisticallyLinkedBlockItem.use`: a tuned linked item used in the air clears itself.
pub fn clears_on_use(item: &str, tuned: bool) -> bool {
    is_linked_item(item) && tuned
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_items_are_linked() {
        for item in LINKED_ITEMS {
            assert!(is_linked_item(item));
        }
        assert!(!is_linked_item("create:packager"));
        assert!(!is_linked_item("create:red_table_cloth"));
    }

    #[test]
    fn cloths_and_requesters_take_an_order_and_only_requesters_a_frequency() {
        assert!(write_applies(Write::Program, "create:red_table_cloth", false));
        assert!(write_applies(Write::Program, "create:andesite_table_cloth", false));
        assert!(write_applies(Write::Program, "create:redstone_requester", false));
        assert!(!write_applies(Write::Program, "create:stock_link", false));
        assert!(program_tunes("create:redstone_requester"));
        assert!(!program_tunes("create:red_table_cloth"));
    }

    #[test]
    fn only_a_tuned_item_clears() {
        assert!(!write_applies(Write::Clear, "create:stock_link", false));
        assert!(write_applies(Write::Clear, "create:stock_link", true));
        assert!(clears_on_use("create:factory_gauge", true));
        assert!(!clears_on_use("create:factory_gauge", false));
        assert!(!clears_on_use("create:red_table_cloth", true));
    }

    #[test]
    fn a_click_tunes_unless_sneaking() {
        assert!(tunes_on_click("create:stock_ticker", false, false, true));
        assert!(!tunes_on_click("create:stock_ticker", true, false, true));
        assert!(!tunes_on_click("create:stock_ticker", false, true, true));
        assert!(!tunes_on_click("create:stock_ticker", false, false, false));
        assert!(!tunes_on_click("minecraft:stick", false, false, true));
    }

    #[test]
    fn an_untuned_gauge_is_refused() {
        assert!(gauge_refused("create:factory_gauge", false));
        assert!(!gauge_refused("create:factory_gauge", true));
        assert!(!gauge_refused("create:stock_link", false));
    }
}
