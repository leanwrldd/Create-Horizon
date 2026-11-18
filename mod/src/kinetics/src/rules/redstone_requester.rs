//! The Redstone Requester (`RedstoneRequesterBlockEntity`, `RedstoneRequesterBlock`,
//! `AutoRequestData`, `RedstoneRequesterConfigurationPacket`, `RedstoneRequesterMenu`): a rising
//! redstone signal sends the order it was programmed with to the network; unless it allows
//! partial requests, it sends nothing when the network holds less than any one of the stacks.
//! `AutoRequestData` is also what a Table Cloth keeps (`rules::table_cloth`).
//!
//! No game types: `K` is an item with its components. The block entity reads the level's
//! redstone signal and the network's accurate summary, which are plain input to
//! [`Requester::on_redstone_power_changed`]; it decides to send the order, which is plain
//! output ([`Trigger`]). Broadcasting it is `rules::logistics_network::broadcast_package_request`
//! with `RequestType::Redstone`.
//!
//! Known differences from Create, for the README:
//! - The menu's ghost slots only keep items (`SorterProofSlot`); an empty `ItemStack` in a
//!   request (`!stack.isEmpty()` in the configuration packet) can't be told here, and isn't a
//!   thing a request holds.
//! - `Vec3i.closerThan` (the range check of [`AutoRequestData::read_from_item`]) is vanilla's:
//!   the squared distance is less than the square of the range.

use super::package::{Big, Context, Summary};
use crate::Pos;

/// `AutoRequestData.readFromItem`: the largest distance (exclusive) a programmed
/// requester or table cloth may be from its Stock Ticker.
pub const RANGE: i64 = 128;
/// The menu's ghost slots (`new ItemStackHandler(9)`).
pub const GHOST_SLOTS: usize = 9;

/// `AutoRequestData`: an order, the address it goes to, and where the Stock Ticker it was
/// programmed at is (`target_offset`, relative to the block it is on, or absolute while in an
/// item).
#[derive(Clone, Debug, PartialEq)]
pub struct AutoRequestData<K> {
    pub encoded_request: Context<K>,
    pub encoded_target_address: String,
    pub target_offset: Pos,
    pub target_dim: String,
    pub is_valid: bool,
}

impl<K> Default for AutoRequestData<K> {
    /// `new AutoRequestData()`: nothing, at `BlockPos.ZERO`, in the dimension "null".
    fn default() -> Self {
        Self {
            encoded_request: Context::default(),
            encoded_target_address: String::new(),
            target_offset: [0, 0, 0],
            target_dim: "null".to_owned(),
            is_valid: false,
        }
    }
}

/// What `readFromItem` tells the player (`redstone_requester.keeper_connected` or
/// `keeper_too_far_away`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {
    KeeperConnected,
    KeeperTooFarAway,
}

impl<K: Clone> AutoRequestData<K> {
    /// `RedstoneRequesterBlock.programRequester`: the data a requester or table cloth in a
    /// player's hand gets from a Stock Ticker's request screen: the ticker's own position
    /// (absolute, from `BlockPos.ZERO`), not yet valid.
    pub fn programmed(order: Context<K>, address: String, ticker: Pos, dimension: &str) -> Self {
        Self {
            encoded_request: order,
            encoded_target_address: address,
            target_offset: ticker,
            target_dim: dimension.to_owned(),
            is_valid: false,
        }
    }

    /// `writeToItem(position, stack)`: the offset becomes `position + offset` (`BlockPos.offset`).
    pub fn write_to_item(&self, position: Pos) -> Self {
        let mut data = self.clone();
        data.target_offset = [
            position[0] + self.target_offset[0],
            position[1] + self.target_offset[1],
            position[2] + self.target_offset[2],
        ];
        data
    }

    /// `readFromItem(level, player, position, stack)`: the offset is made relative to where the
    /// block is placed, and the data is valid when the ticker is within [`RANGE`] blocks and in
    /// the same dimension. The message goes to the player (when there is one).
    pub fn read_from_item(&self, dimension: &str, position: Pos) -> (Self, Message) {
        let mut data = self.clone();
        data.target_offset = [
            self.target_offset[0] - position[0],
            self.target_offset[1] - position[1],
            self.target_offset[2] - position[2],
        ];
        let dist_sqr: i64 = data.target_offset.iter().map(|c| i64::from(*c) * i64::from(*c)).sum();
        data.is_valid = dist_sqr < RANGE * RANGE && self.target_dim == dimension;
        let message = if data.is_valid { Message::KeeperConnected } else { Message::KeeperTooFarAway };
        (data, message)
    }

    /// `targetOffset.offset(worldPosition)`: where the ticker is, from the block the data is on.
    pub fn ticker_pos(&self, position: Pos) -> Pos {
        [
            self.target_offset[0] + position[0],
            self.target_offset[1] + position[1],
            self.target_offset[2] + position[2],
        ]
    }
}

/// What a rising signal did (`RedstoneRequesterBlockEntity.triggerRequest`).
#[derive(Clone, Debug, PartialEq)]
pub enum Trigger<K> {
    /// No signal change, no order, or nothing to do: no effect, `lastRequestSucceeded` unchanged.
    Nothing,
    /// The network holds less than a stack of the order and partial requests aren't allowed:
    /// the deny effect (`RedstoneRequesterEffectPacket(pos, false)`), nothing sent.
    Denied,
    /// The order is broadcast (`RequestType.REDSTONE`) to the address; the effect is a success
    /// when at least one stack of the order was in stock whole.
    Send { order: Context<K>, address: String, success_effect: bool },
}

/// The state of a `RedstoneRequesterBlockEntity`.
#[derive(Clone, Debug, PartialEq)]
pub struct Requester<K> {
    /// `allowPartialRequests`.
    pub allow_partial_requests: bool,
    /// `encodedRequest`.
    pub encoded_request: Context<K>,
    /// `encodedTargetAdress`.
    pub encoded_target_address: String,
    /// `lastRequestSucceeded`.
    pub last_request_succeeded: bool,
    /// `redstonePowered`.
    pub redstone_powered: bool,
}

impl<K> Default for Requester<K> {
    fn default() -> Self {
        Self {
            allow_partial_requests: false,
            encoded_request: Context::default(),
            encoded_target_address: String::new(),
            last_request_succeeded: false,
            redstone_powered: false,
        }
    }
}

/// What a change of the neighbour signal did.
#[derive(Clone, Debug, PartialEq)]
pub struct PowerChange<K> {
    /// The signal differed from the saved one: the block entity is notified, and the block's
    /// `POWERED` property is set to the signal.
    pub changed: bool,
    pub trigger: Trigger<K>,
}

impl<K: PartialEq + Clone> Requester<K> {
    /// `RedstoneRequesterBlock.setPlacedBy`: the programmed data of the item that was placed.
    pub fn from_item(data: &AutoRequestData<K>) -> Self {
        Self {
            encoded_request: data.encoded_request.clone(),
            encoded_target_address: data.encoded_target_address.clone(),
            ..Self::default()
        }
    }

    /// `onRedstonePowerChanged`: `has_neighbor_signal` is `level.hasNeighborSignal(pos)`, and
    /// `accurate_summary` the network's `getAccurateSummary()`, asked for only when the signal
    /// rises. A rising signal sends the order once; a held signal does nothing more.
    pub fn on_redstone_power_changed(
        &mut self,
        has_neighbor_signal: bool,
        accurate_summary: impl FnOnce() -> Summary<K>,
    ) -> PowerChange<K> {
        if self.redstone_powered == has_neighbor_signal {
            return PowerChange { changed: false, trigger: Trigger::Nothing };
        }
        self.last_request_succeeded = false;
        let trigger = if has_neighbor_signal { self.trigger_request(&accurate_summary()) } else { Trigger::Nothing };
        self.redstone_powered = has_neighbor_signal;
        PowerChange { changed: true, trigger }
    }

    /// `triggerRequest`: an empty order does nothing; every stack of the order (equal items
    /// merged) the network holds whole counts as a success, and one it doesn't denies the
    /// request unless partial requests are allowed. The order sent is the saved one as it is.
    pub fn trigger_request(&mut self, accurate: &Summary<K>) -> Trigger<K> {
        if self.encoded_request.is_empty() {
            return Trigger::Nothing;
        }
        let mut any_succeeded = false;
        let mut summary_of_order: Summary<K> = Summary::default();
        for (key, count) in &self.encoded_request.stacks {
            summary_of_order.add(key, i64::from(*count));
        }
        for (key, count) in &summary_of_order.stacks {
            if accurate.count_of(key) >= *count {
                any_succeeded = true;
                continue;
            }
            if !self.allow_partial_requests {
                return Trigger::Denied;
            }
        }
        self.last_request_succeeded = true;
        Trigger::Send {
            order: self.encoded_request.clone(),
            address: self.encoded_target_address.clone(),
            success_effect: any_succeeded,
        }
    }

    /// `RedstoneRequesterConfigurationPacket.applySettings`: the address, the amount of each
    /// stack (by position in the request), and whether partial requests are allowed. A request
    /// whose recipes no longer use up what is ordered loses its recipes.
    pub fn apply_configuration(&mut self, address: &str, allow_partial: bool, amounts: &[i32]) {
        self.encoded_target_address = address.to_owned();
        for (stack, amount) in self.encoded_request.stacks.iter_mut().zip(amounts) {
            stack.1 = (*amount).max(0) as u32;
        }
        if !self.encoded_request.ordered_stacks_match_ordered_recipes() {
            self.encoded_request = Context { stacks: self.encoded_request.stacks.clone(), crafts: Vec::new() };
        }
        self.allow_partial_requests = allow_partial;
    }

    /// `RedstoneRequesterMenu.createGhostInventory`: the nine ghost slots, one of each item.
    pub fn ghost_slots(&self) -> [Option<K>; GHOST_SLOTS] {
        std::array::from_fn(|i| self.encoded_request.stacks.get(i).map(|(key, _)| key.clone()))
    }

    /// `RedstoneRequesterMenu.saveData`: the ghost slots become the request's stacks, each
    /// keeping the count the stack at the same index had (1 for a new one), and the recipes are
    /// kept only while they use up exactly what is ordered.
    pub fn save_ghost_slots(&mut self, ghost: &[Option<K>]) {
        let mut list: Vec<Big<K>> = Vec::new();
        for (i, slot) in ghost.iter().enumerate().take(GHOST_SLOTS) {
            let Some(key) = slot else {
                continue;
            };
            let count = self.encoded_request.stacks.get(i).map_or(1, |(_, count)| *count);
            list.push((key.clone(), count));
        }
        let new_request = Context { stacks: list, crafts: self.encoded_request.crafts.clone() };
        self.encoded_request = if new_request.ordered_stacks_match_ordered_recipes() {
            new_request
        } else {
            Context { stacks: new_request.stacks, crafts: Vec::new() }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::package::Craft;

    fn summary(stacks: &[(&'static str, u32)]) -> Summary<&'static str> {
        let mut s = Summary::default();
        for (k, c) in stacks {
            s.add(k, i64::from(*c));
        }
        s
    }

    fn requester(stacks: &[(&'static str, u32)]) -> Requester<&'static str> {
        Requester {
            encoded_request: Context { stacks: stacks.to_vec(), crafts: Vec::new() },
            encoded_target_address: "Base".to_owned(),
            ..Requester::default()
        }
    }

    #[test]
    fn a_requester_fires_once_per_rising_edge() {
        let mut r = requester(&[("iron", 10), ("gold", 4)]);
        let stock = summary(&[("iron", 30), ("gold", 4)]);
        // The signal rises: the order is sent as it is.
        let change = r.on_redstone_power_changed(true, || stock.clone());
        assert!(change.changed);
        assert_eq!(
            change.trigger,
            Trigger::Send { order: r.encoded_request.clone(), address: "Base".into(), success_effect: true }
        );
        assert!(r.last_request_succeeded && r.redstone_powered);
        // Held: nothing happens, however often the neighbours update, and the summary isn't read.
        let held = r.on_redstone_power_changed(true, || panic!("the summary is read only on a rising signal"));
        assert!(!held.changed);
        assert_eq!(held.trigger, Trigger::Nothing);
        assert!(r.last_request_succeeded);
        // The signal drops: the last result is forgotten, nothing is sent.
        let fall = r.on_redstone_power_changed(false, || panic!("not read"));
        assert!(fall.changed);
        assert_eq!(fall.trigger, Trigger::Nothing);
        assert!(!r.last_request_succeeded && !r.redstone_powered);
        // And rises again: sent again.
        let again = r.on_redstone_power_changed(true, || stock.clone());
        assert!(matches!(again.trigger, Trigger::Send { .. }));
    }

    #[test]
    fn a_short_network_denies_the_order_unless_partial_requests_are_allowed() {
        // The network has the iron but only 3 of the 4 gold.
        let stock = summary(&[("iron", 30), ("gold", 3)]);
        let mut r = requester(&[("iron", 10), ("gold", 4)]);
        let change = r.on_redstone_power_changed(true, || stock.clone());
        assert_eq!(change.trigger, Trigger::Denied);
        assert!(!r.last_request_succeeded);
        assert!(r.redstone_powered, "the signal was still taken");
        // Partial: sent anyway, as a success because one stack was whole.
        let mut r = requester(&[("iron", 10), ("gold", 4)]);
        r.allow_partial_requests = true;
        let change = r.on_redstone_power_changed(true, || stock.clone());
        assert!(matches!(change.trigger, Trigger::Send { success_effect: true, .. }));
        assert!(r.last_request_succeeded);
        // Partial with nothing whole is still sent, as a failure effect.
        let mut r = requester(&[("gold", 4)]);
        r.allow_partial_requests = true;
        let change = r.on_redstone_power_changed(true, || stock.clone());
        assert!(matches!(change.trigger, Trigger::Send { success_effect: false, .. }));
        // Equal items of an order are added up before checking: 2 + 2 gold is 4.
        let mut r = requester(&[("gold", 2), ("gold", 2)]);
        let change = r.on_redstone_power_changed(true, || stock.clone());
        assert_eq!(change.trigger, Trigger::Denied);
        // An empty order does nothing (and doesn't touch lastRequestSucceeded).
        let mut r = requester(&[]);
        let change = r.on_redstone_power_changed(true, || stock.clone());
        assert_eq!(change.trigger, Trigger::Nothing);
        assert!(change.changed);
    }

    #[test]
    fn configuring_sets_amounts_and_drops_recipes_that_no_longer_match() {
        let mut r = requester(&[("planks", 4), ("stick", 2)]);
        r.encoded_request.crafts = vec![Craft { pattern: vec![("planks", 2), ("stick", 1)], count: 2 }];
        assert!(r.encoded_request.ordered_stacks_match_ordered_recipes());
        // The same amounts keep the recipes.
        r.apply_configuration("Shop", true, &[4, 2]);
        assert_eq!(r.encoded_request.crafts.len(), 1);
        assert_eq!(r.encoded_target_address, "Shop");
        assert!(r.allow_partial_requests);
        // A different amount no longer matches: the order stays, without recipes.
        r.apply_configuration("Shop", false, &[8]);
        assert_eq!(r.encoded_request.stacks, vec![("planks", 8), ("stick", 2)]);
        assert!(r.encoded_request.crafts.is_empty());
    }

    #[test]
    fn the_ghost_slots_keep_their_counts_by_index() {
        let mut r = requester(&[("iron", 12), ("gold", 5)]);
        assert_eq!(r.ghost_slots()[..3], [Some("iron"), Some("gold"), None]);
        // A new item in the third slot starts at one; the second slot emptied shifts the
        // list, so the third item takes the count of the stack at its slot index (index 2: none -> 1).
        r.save_ghost_slots(&[Some("iron"), None, Some("coal")]);
        assert_eq!(r.encoded_request.stacks, vec![("iron", 12), ("coal", 1)]);
        r.save_ghost_slots(&[Some("iron"), Some("coal")]);
        assert_eq!(r.encoded_request.stacks, vec![("iron", 12), ("coal", 1)]);
    }

    #[test]
    fn a_programmed_requester_knows_its_ticker_while_in_range() {
        let order = Context { stacks: vec![("iron", 4)], crafts: Vec::new() };
        let item = AutoRequestData::programmed(order, "Base".into(), [100, 64, 100], "minecraft:overworld");
        assert!(!item.is_valid);
        // Placed 3 blocks away: offset (97, 64 - y, ...) is in range.
        let (placed, message) = item.read_from_item("minecraft:overworld", [10, 64, 10]);
        assert_eq!(placed.target_offset, [90, 0, 90]);
        assert!(placed.is_valid);
        assert_eq!(message, Message::KeeperConnected);
        assert_eq!(placed.ticker_pos([10, 64, 10]), [100, 64, 100]);
        // 128 blocks away is out (`closerThan` is strict), 127 is in.
        let (far, message) = item.read_from_item("minecraft:overworld", [-28, 64, 100]);
        assert!(!far.is_valid);
        assert_eq!(message, Message::KeeperTooFarAway);
        let (near, _) = item.read_from_item("minecraft:overworld", [-27, 64, 100]);
        assert!(near.is_valid);
        // Another dimension is out.
        let (nether, _) = item.read_from_item("minecraft:the_nether", [100, 64, 100]);
        assert!(!nether.is_valid);
        // Broken out of a Table Cloth, the offset is made absolute again.
        assert_eq!(placed.write_to_item([10, 64, 10]).target_offset, [100, 64, 100]);
        // A requester placed from it takes the order and the address.
        assert_eq!(Requester::from_item(&placed).encoded_target_address, "Base");
        assert_eq!(AutoRequestData::<&str>::default().target_dim, "null");
    }
}
