//! The Factory Gauge (`FactoryPanelBehaviour`, `FactoryPanelBlockEntity`, `FactoryPanelConnection`,
//! `FactoryPanelPosition`, `FactoryPanelSupportBehaviour`, `FactoryPanelConnectionHandler`,
//! `FactoryPanelConfigurationPacket`): up to four panels on one block, each with a target
//! amount of one item. A panel counts what its network holds and has been promised; when it is
//! short, it asks the network every `factoryGaugeTimer` ticks for the ingredients (the panels
//! pointing at it, with the amounts on their connections), to be packaged to its address
//! (a Repackager and crafters make the item from them), or, on a Packager, asks for the item
//! itself ("restocker", `tryRestock`). The ask is kept as a request promise until the item
//! turns up.
//!
//! No game types: `K` is an item with its components. What the panel reads of the game is plain
//! input: [`Reads`] (what the network holds, how many links are unloaded), [`RequestEnv`] (the
//! other panels, the summaries and the links of the networks). What it decides is plain output:
//! [`StorageTick`], [`RequestOutcome`] (effects, the requests to dispatch to packagers, the
//! promise it kept).
//!
//! Known differences from Create, for the README:
//! - Connection paths (`FactoryPanelConnection.getPath`, `calculatePathDiff`, the arrow bend
//!   mode search that avoids other gauges), the bulb and the value box text are drawing, not
//!   rules: the arrow bend mode is kept as a number ([`Connection::arrow_bend_mode`]) and
//!   cycled ([`cycle_arrow_bend`]).
//! - `FactoryPanelBehaviour.moveTo` (relocating a panel) reads and changes the world around it
//!   and isn't ported; the slot positions (`FactoryPanelSlotPositioning`) are drawing.
//! - A panel's connections are kept in the order they were made (Create's `HashMap` order isn't
//!   defined), and so are the networks a request is split over.
//! - `getLevelInStorage` and `getUnloadedLinks` of a virtual (Ponder) block entity are the
//!   caller's.

use super::logistics_network::{
    Dispatch, Freq, LinkView, Networks, PackagerId, Random, dispatches, find_packagers_for_request, stock_of,
    any_too_busy,
};
use super::package::{Context, Craft, Summary};
use super::packager::RequestType;
use super::request_promises::{Promise, Queue};
use crate::Pos;

/// `CLogistics.factoryGaugeTimer`: `i(100, 5, ...)`, the ticks a gauge waits between requests.
pub const DEFAULT_REQUEST_INTERVAL: i32 = 100;
/// The least the config allows.
pub const MIN_REQUEST_INTERVAL: i32 = 5;
/// `FactoryPanelBehaviour.setLazyTickRate(40)`.
pub const LAZY_PERIOD: i64 = 40;
/// `addConnection`, `checkForIssues`: a panel takes at most nine inputs.
pub const MAX_INPUTS: usize = 9;
/// `tryRestock`: a restock order is at most this many stacks.
pub const RESTOCK_STACKS: i64 = 9;
/// `FactoryPanelConnectionHandler.checkForIssues`: connected panels are closer than this
/// (`closerThan(BlockPos.ZERO, 16)`).
pub const CONNECTION_RANGE: i64 = 16;
/// `FactoryPanelBehaviour.createBoard`: the target amount's largest setting.
pub const MAX_TARGET: i32 = 100;
/// `FactoryPanelBehaviour.getPromiseExpiryTimeInTicks` for interval 0.
pub const DEFAULT_PROMISE_EXPIRY: i32 = 20 * 30;

/// `FactoryPanelBlock.PanelSlot` with its offsets on the block's face.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Slot {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Slot {
    pub const ALL: [Slot; 4] = [Slot::TopLeft, Slot::TopRight, Slot::BottomLeft, Slot::BottomRight];

    /// `xOffset`.
    pub const fn x_offset(self) -> i32 {
        match self {
            Slot::TopLeft | Slot::BottomLeft => 1,
            Slot::TopRight | Slot::BottomRight => 0,
        }
    }

    /// `yOffset`.
    pub const fn y_offset(self) -> i32 {
        match self {
            Slot::TopLeft | Slot::TopRight => 1,
            Slot::BottomLeft | Slot::BottomRight => 0,
        }
    }
}

/// `FactoryPanelPosition`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PanelPosition {
    pub pos: Pos,
    pub slot: Slot,
}

/// `FactoryPanelConnection`: an input to a panel, from another panel, with the amount of its item
/// the target needs for one craft.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Connection {
    pub from: PanelPosition,
    pub amount: i32,
    /// `arrowBendMode`: -1 lets the drawing choose.
    pub arrow_bend_mode: i32,
}

impl Connection {
    /// `new FactoryPanelConnection(from, amount)`.
    pub fn new(from: PanelPosition, amount: i32) -> Self {
        Self { from, amount, arrow_bend_mode: -1 }
    }
}

/// `FactoryPanelBehaviour.onShortInteract` with a wrench: the next arrow bend mode
/// (`(mode + 1) % 4`), shared by every connection of the panel.
pub fn cycle_arrow_bend(first_mode: i32) -> i32 {
    (first_mode + 1) % 4
}

/// What a panel's effect packet shows (`FactoryPanelEffectPacket(from, to, success)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Effect {
    pub from: PanelPosition,
    pub to: PanelPosition,
    pub success: bool,
}

/// The state of one `FactoryPanelBehaviour`.
#[derive(Clone, Debug, PartialEq)]
pub struct Gauge<K> {
    pub slot: Slot,
    /// `active`: the slot holds a panel.
    pub active: bool,
    /// `getFilter()`: the item, none when empty.
    pub filter: Option<K>,
    /// `count`, the target.
    pub count: i32,
    /// `upTo`: the target is in items, else in stacks.
    pub up_to: bool,
    /// `targetedBy`: the inputs, in the order they were made.
    pub targeted_by: Vec<Connection>,
    /// `targetedByLinks`: redstone links and display links attached.
    pub targeted_by_links: Vec<Connection>,
    /// `targeting`: the panels this one is an input to.
    pub targeting: Vec<PanelPosition>,
    /// `activeCraftingArrangement`: the nine slots of a crafting grid (an empty slot is a key the
    /// caller treats as empty).
    pub active_crafting_arrangement: Vec<K>,
    pub satisfied: bool,
    pub promised_satisfied: bool,
    pub waiting_for_network: bool,
    pub recipe_address: String,
    /// `recipeOutput`: how many of the item one craft makes.
    pub recipe_output: u32,
    /// `promiseClearingInterval`: -1 never, 0 thirty seconds, else minutes.
    pub promise_clearing_interval: i32,
    pub force_clear_promises: bool,
    /// `network`.
    pub network: Freq,
    pub redstone_powered: bool,
    /// `restockerPromises`.
    pub restocker_promises: Queue<K>,
    pub last_reported_unloaded_links: u32,
    pub last_reported_level_in_storage: u32,
    pub last_reported_promises: u32,
    /// `timer`.
    pub timer: i32,
}

impl<K> Gauge<K> {
    /// `new FactoryPanelBehaviour(be, slot)`.
    pub fn new(slot: Slot, network: Freq) -> Self {
        Self {
            slot,
            active: false,
            filter: None,
            count: 0,
            up_to: true,
            targeted_by: Vec::new(),
            targeted_by_links: Vec::new(),
            targeting: Vec::new(),
            active_crafting_arrangement: Vec::new(),
            satisfied: false,
            promised_satisfied: false,
            waiting_for_network: false,
            recipe_address: String::new(),
            recipe_output: 1,
            promise_clearing_interval: -1,
            force_clear_promises: false,
            network,
            redstone_powered: false,
            restocker_promises: Queue::default(),
            last_reported_unloaded_links: 0,
            last_reported_level_in_storage: 0,
            last_reported_promises: 0,
            timer: 0,
        }
    }
}

/// What a panel reads each tick of the game around it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reads {
    /// `panelBE().restocker`: the gauge sits on a Packager.
    pub restocker: bool,
    /// `getUnloadedLinks()`: the network's unloaded links, or for a restocker 1 without a
    /// packager behind it.
    pub unloaded_links: u32,
    /// `getLevelInStorage()`: what the network's summary holds of the item (the restocked
    /// packager's, for a restocker).
    pub level_in_storage: u32,
    /// `getFilter().getMaxStackSize()`.
    pub max_stack_size: u32,
}

/// What `tickStorageMonitor` decided.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StorageTick {
    /// `blockEntity.sendData()`.
    pub changed: bool,
    /// The panel became satisfied (with a demand): the two confirm sounds.
    pub confirm: bool,
    /// `satisfied` changed: linked redstone links and display links are told.
    pub notify_outputs: bool,
}

impl<K: PartialEq + Clone> Gauge<K> {
    /// `getAmount() * (upTo ? 1 : maxStackSize)`.
    pub fn demand(&self, max_stack_size: u32) -> i64 {
        i64::from(self.count) * if self.up_to { 1 } else { i64::from(max_stack_size) }
    }

    /// `getPromiseExpiryTimeInTicks`.
    pub fn promise_expiry_time(&self) -> i32 {
        match self.promise_clearing_interval {
            -1 => -1,
            0 => DEFAULT_PROMISE_EXPIRY,
            minutes => minutes * 20 * 60,
        }
    }

    /// `resetTimerSlightly`.
    pub fn reset_timer_slightly(&mut self, interval: i32) {
        self.timer = interval / 2;
    }

    /// `getPromised`: what is promised of the item, from the network's queue (`network_queue`,
    /// none when the network isn't known) or a restocker's own. A forced clear drops the item's
    /// promises first (and shortens the timer); expired ones are dropped as they are counted.
    pub fn promised(&mut self, restocker: bool, network_queue: Option<&mut Queue<K>>, interval: i32) -> u32 {
        let Some(item) = self.filter.clone() else {
            return 0;
        };
        let expiry = self.promise_expiry_time();
        if restocker {
            if self.force_clear_promises {
                self.restocker_promises.force_clear(&item);
                self.reset_timer_slightly(interval);
            }
            self.force_clear_promises = false;
            return self.restocker_promises.total_promised_and_remove_expired(&item, expiry);
        }
        let Some(promises) = network_queue else {
            return 0;
        };
        if self.force_clear_promises {
            promises.force_clear(&item);
            self.reset_timer_slightly(interval);
        }
        self.force_clear_promises = false;
        promises.total_promised_and_remove_expired(&item, expiry)
    }

    /// Whether `tickStorageMonitor` is about to invalidate the network's cached summary
    /// (`LogisticsManager.SUMMARIES.invalidate`): the last of its links just loaded. Asked
    /// before the summary is read for [`Reads::level_in_storage`].
    pub fn wants_fresh_summary(&self, restocker: bool, unloaded_links: u32) -> bool {
        !restocker && unloaded_links == 0 && self.last_reported_unloaded_links != 0
    }

    /// `tickStorageMonitor`: compares what the network holds and has been promised with the
    /// demand, and keeps what it saw for the clients.
    pub fn tick_storage_monitor(&mut self, reads: &Reads, promised: u32) -> StorageTick {
        let in_storage = reads.level_in_storage;
        let demand = self.demand(reads.max_stack_size);
        let should_satisfy = self.filter.is_none() || i64::from(in_storage) >= demand;
        let should_promise_satisfy = self.filter.is_none() || i64::from(in_storage) + i64::from(promised) >= demand;
        let should_wait = reads.unloaded_links > 0;

        if self.last_reported_level_in_storage == in_storage
            && self.last_reported_promises == promised
            && self.last_reported_unloaded_links == reads.unloaded_links
            && self.satisfied == should_satisfy
            && self.promised_satisfied == should_promise_satisfy
            && self.waiting_for_network == should_wait
        {
            return StorageTick::default();
        }

        let confirm = !self.satisfied && should_satisfy && demand > 0;
        let notify_outputs = self.satisfied != should_satisfy;
        self.last_reported_level_in_storage = in_storage;
        self.satisfied = should_satisfy;
        self.last_reported_promises = promised;
        self.promised_satisfied = should_promise_satisfy;
        self.last_reported_unloaded_links = reads.unloaded_links;
        self.waiting_for_network = should_wait;
        StorageTick { changed: true, confirm, notify_outputs }
    }

    /// `isMissingAddress`.
    pub fn is_missing_address(&self, restocker: bool) -> bool {
        (!self.targeted_by.is_empty() || restocker) && self.count != 0 && self.recipe_address.trim().is_empty()
    }
}

// --- requests -----------------------------------------------------------------------------------

/// An input panel as `FactoryPanelBehaviour.at(level, connection)` finds it.
#[derive(Clone, Debug, PartialEq)]
pub struct Source<K> {
    pub filter: Option<K>,
    pub network: Freq,
}

/// What a gauge reads of the world to make a request.
pub struct RequestEnv<'a, K> {
    /// `FactoryPanelBehaviour.at(level, connection)`: none when the panel is missing, inactive or
    /// unloaded (which cancels the request).
    pub source: &'a dyn Fn(&PanelPosition) -> Option<Source<K>>,
    /// `LogisticsManager.getSummaryOfNetwork(network, true)`.
    pub accurate_summary: &'a dyn Fn(Freq) -> Summary<K>,
    /// The links present on a network (`getAllPresent`), as views. For a restocker, the views
    /// have `targets_ignored` set for the packager the gauge sits on.
    pub links: &'a dyn Fn(Freq) -> Vec<LinkView<K>>,
    pub rng: &'a mut dyn Random,
    pub networks: &'a mut Networks<K>,
    /// `getConfigRequestIntervalInTicks`.
    pub interval: i32,
    /// A restocker's packager: whether it has an inventory (`targetInventory.hasInventory()`);
    /// none when the gauge isn't a restocker or the packager is missing.
    pub restocked_packager_has_inventory: Option<bool>,
}

/// What a tick of requests did.
#[derive(Clone, Debug, PartialEq)]
pub struct RequestOutcome<K> {
    /// `sendEffect`s, in order.
    pub effects: Vec<Effect>,
    /// What to make: each packager's requests (`performPackageRequests`: flash the link, run
    /// `attemptToSend` up to 100 times, trigger the stock check, notify).
    pub dispatches: Vec<Dispatch<K>>,
    /// The promise this gauge kept for its own item (on the network's queue, or its own for a
    /// restocker).
    pub promised: Option<Promise<K>>,
    /// `AllAdvancements.FACTORY_GAUGE` for the block entity's player.
    pub advancement: bool,
}

impl<K> Default for RequestOutcome<K> {
    fn default() -> Self {
        Self { effects: Vec::new(), dispatches: Vec::new(), promised: None, advancement: false }
    }
}

impl<K: PartialEq + Clone> Gauge<K> {
    pub fn position(&self, pos: Pos) -> PanelPosition {
        PanelPosition { pos, slot: self.slot }
    }

    /// `tickRequests`. `pos` is the block entity's position.
    pub fn tick_requests(&mut self, pos: Pos, reads: &Reads, env: &mut RequestEnv<'_, K>) -> RequestOutcome<K> {
        let mut out = RequestOutcome::default();
        if self.targeted_by.is_empty() && !reads.restocker {
            return out;
        }
        if reads.restocker {
            self.restocker_promises.tick();
        }
        if self.satisfied || self.promised_satisfied || self.waiting_for_network || self.redstone_powered {
            return out;
        }
        if self.timer > 0 {
            self.timer = self.timer.min(env.interval);
            self.timer -= 1;
            return out;
        }

        // `resetTimer`.
        self.timer = env.interval;

        if self.recipe_address.trim().is_empty() {
            return out;
        }

        if reads.restocker {
            self.try_restock(pos, reads, env, &mut out);
            return out;
        }

        let here = self.position(pos);
        let mut failed = false;

        // Consolidate the inputs by network and item.
        struct Consolidated<K> {
            network: Freq,
            item: Option<K>,
            from: Vec<PanelPosition>,
            total_amount: i64,
        }
        let mut consolidated: Vec<Consolidated<K>> = Vec::new();
        for connection in &self.targeted_by {
            let Some(source) = (env.source)(&connection.from) else {
                return out;
            };
            match consolidated.iter_mut().find(|c| c.network == source.network && c.item == source.filter) {
                Some(c) => {
                    c.from.push(connection.from);
                    c.total_amount += i64::from(connection.amount);
                }
                None => consolidated.push(Consolidated {
                    network: source.network,
                    item: source.filter,
                    from: vec![connection.from],
                    total_amount: i64::from(connection.amount),
                }),
            }
        }

        // What to ask of each network.
        let mut to_request: Vec<(Freq, Vec<(K, u32)>)> = Vec::new();
        let mut summaries: Vec<(Freq, Summary<K>)> = Vec::new();
        for entry in &consolidated {
            if !summaries.iter().any(|(n, _)| *n == entry.network) {
                summaries.push((entry.network, (env.accurate_summary)(entry.network)));
            }
            let summary = &summaries.iter().find(|(n, _)| *n == entry.network).expect("summary read").1;
            let enough = match &entry.item {
                Some(item) => entry.total_amount != 0 && i64::from(summary.count_of(item)) >= entry.total_amount,
                None => false,
            };
            if !enough {
                for from in &entry.from {
                    out.effects.push(Effect { from: *from, to: here, success: false });
                }
                failed = true;
                continue;
            }
            let item = entry.item.clone().expect("checked above");
            let stack = (item, entry.total_amount as u32);
            match to_request.iter_mut().find(|(n, _)| *n == entry.network) {
                Some((_, stacks)) => stacks.push(stack),
                None => to_request.push((entry.network, vec![stack])),
            }
            for from in &entry.from {
                out.effects.push(Effect { from: *from, to: here, success: true });
            }
        }

        if failed {
            return out;
        }

        // "Panel may enforce item arrangement."
        let crafts: Vec<Craft<K>> = if self.active_crafting_arrangement.is_empty() {
            Vec::new()
        } else {
            vec![Craft { pattern: self.active_crafting_arrangement.iter().map(|k| (k.clone(), 1)).collect(), count: 1 }]
        };

        // Collect request distributions.
        let mut plans = Vec::new();
        let mut busy_check: Vec<(PackagerId, usize)> = Vec::new();
        for (network, stacks) in &to_request {
            let order = Context { stacks: stacks.clone(), crafts: crafts.clone() };
            let links = (env.links)(*network);
            for link in &links {
                if let Some(p) = &link.packager {
                    busy_check.push((p.id, p.queued));
                }
            }
            plans.push(find_packagers_for_request(&links, &order, &self.recipe_address, env.rng));
        }

        // "Check if any packager is busy - cancel all".
        if plans.iter().any(|plan| any_too_busy(plan, &busy_check, RequestType::Restock)) {
            return out;
        }

        // Send it.
        for plan in &plans {
            out.dispatches.extend(dispatches(plan));
        }

        // Keep the output promise.
        if let (Some(item), Some(queue)) = (self.filter.clone(), env.networks.queued_promises(self.network)) {
            let promise = Promise::new((item, self.recipe_output));
            queue.add(promise.clone());
            out.promised = Some(promise);
        }
        out.advancement = true;
        out
    }

    /// `tryRestock`.
    fn try_restock(&mut self, pos: Pos, reads: &Reads, env: &mut RequestEnv<'_, K>, out: &mut RequestOutcome<K>) {
        let Some(item) = self.filter.clone() else {
            return;
        };
        if env.restocked_packager_has_inventory != Some(true) {
            return;
        }
        let here = self.position(pos);
        let links = (env.links)(self.network);
        let available_on_network = stock_of(&links, &item);
        if available_on_network == 0 {
            out.effects.push(Effect { from: here, to: here, success: false });
            return;
        }

        let in_storage = reads.level_in_storage;
        let promised = self.promised(true, None, env.interval);
        let max_stack_size = i64::from(reads.max_stack_size);
        let demand = self.demand(reads.max_stack_size);
        let amount_to_order = (demand - i64::from(promised) - i64::from(in_storage)).clamp(0, max_stack_size * RESTOCK_STACKS);

        let ordered = (item, amount_to_order.min(i64::from(available_on_network)) as u32);
        let order = Context { stacks: vec![ordered.clone()], crafts: Vec::new() };

        out.effects.push(Effect { from: here, to: here, success: true });

        // `broadcastPackageRequest(network, RESTOCK, order, ignored, address)`.
        let plan = find_packagers_for_request(&links, &order, &self.recipe_address, env.rng);
        let busy: Vec<(PackagerId, usize)> =
            links.iter().filter_map(|l| l.packager.as_ref()).map(|p| (p.id, p.queued)).collect();
        if any_too_busy(&plan, &busy, RequestType::Restock) {
            return;
        }
        out.dispatches.extend(dispatches(&plan));

        let promise = Promise::new(ordered);
        self.restocker_promises.add(promise.clone());
        out.promised = Some(promise);
    }

    // --- redstone ---

    /// `checkForRedstoneInput`, every [`LAZY_PERIOD`] ticks: powered when any attached link
    /// says so. A link that is unloaded or gone stops the check (nothing changes). Returns
    /// whether the power changed (the block entity is notified, and the timer set to 1).
    pub fn check_for_redstone_input(&mut self, links: &[LinkRead]) -> bool {
        if !self.active {
            return false;
        }
        let mut should_power = false;
        for read in links {
            match read {
                LinkRead::Unloaded | LinkRead::Missing => return false,
                LinkRead::Link { powers_panel, .. } => should_power |= *powers_panel,
            }
        }
        if should_power == self.redstone_powered {
            return false;
        }
        self.redstone_powered = should_power;
        self.timer = 1;
        true
    }

    /// `notifyRedstoneOutputs`: the links whose `notifyLink` runs, which stop at the first one
    /// that is unloaded, missing or an output (`return`, not `continue`, as in Create).
    pub fn links_to_notify(&self, links: &[(Pos, LinkRead)]) -> Vec<Pos> {
        let mut notify = Vec::new();
        for (pos, read) in links {
            match read {
                LinkRead::Unloaded | LinkRead::Missing => return notify,
                LinkRead::Link { is_output, .. } => {
                    if *is_output {
                        return notify;
                    }
                    notify.push(*pos);
                }
            }
        }
        notify
    }

    // --- configuration ---

    /// `setValueSettings`: the target (`value`, never below 0) and whether it is in items
    /// (`row == 0`) or stacks. Returns whether it changed; then the timer is shortened and the
    /// outputs are told.
    pub fn set_value_settings(&mut self, row: i32, value: i32, interval: i32) -> bool {
        let up_to = row == 0;
        let count = value.max(0);
        if self.up_to == up_to && self.count == count {
            return false;
        }
        self.count = count;
        self.up_to = up_to;
        self.reset_timer_slightly(interval);
        true
    }

    /// `setFilter`: a filter item can't be the target. Returns whether it was set.
    pub fn set_filter(&mut self, filter: Option<K>, is_filter_item: bool) -> bool {
        if is_filter_item {
            return false;
        }
        self.filter = filter;
        true
    }

    /// `enable`.
    pub fn enable(&mut self) {
        self.active = true;
    }

    /// `disable`: the panel is emptied (its connections are removed by the caller with
    /// [`Gauge::disconnect_all`]'s result).
    pub fn disable(&mut self) {
        self.active = false;
        self.targeted_by.clear();
        self.targeting.clear();
        self.count = 0;
        self.satisfied = false;
        self.promised_satisfied = false;
        self.recipe_address.clear();
        self.recipe_output = 1;
        self.filter = None;
    }

    /// `addConnection(fromPos)`: what the world says is at `from` (a link, a panel, or neither).
    /// A link is attached; a panel becomes an input unless this gauge is a restocker or already
    /// has nine inputs, and the source's `targeting` gets this panel (returned: the caller
    /// adds it).
    pub fn add_connection(&mut self, here: PanelPosition, from: PanelPosition, at: ConnectionSource, restocker: bool) -> Added {
        match at {
            ConnectionSource::Link => {
                self.targeted_by_links.retain(|c| c.from.pos != from.pos);
                self.targeted_by_links.push(Connection::new(from, 1));
                Added::Link
            }
            ConnectionSource::Panel => {
                if restocker || self.targeted_by.len() >= MAX_INPUTS {
                    return Added::No;
                }
                self.targeted_by.retain(|c| c.from != from);
                self.targeted_by.push(Connection::new(from, 1));
                Added::Panel { source_targets: here }
            }
            ConnectionSource::None => Added::No,
        }
    }

    /// `FactoryPanelConfigurationPacket.applySettings` for this panel.
    pub fn apply_configuration(&mut self, here: PanelPosition, packet: &Configuration<K>) -> ConfigurationEffect {
        self.recipe_address = if packet.reset { String::new() } else { packet.address.clone() };
        self.recipe_output = if packet.reset { 1 } else { packet.output_amount };
        self.promise_clearing_interval = if packet.reset { -1 } else { packet.promise_clearing_interval };
        self.active_crafting_arrangement = if packet.reset { Vec::new() } else { packet.crafting_arrangement.clone() };

        if packet.reset {
            self.force_clear_promises = true;
            let disconnected = self.disconnect_all(here);
            self.filter = None;
            self.count = 0;
            return ConfigurationEffect { disconnected: Some(disconnected), links_disconnected: Vec::new(), source_forgets: None };
        }

        if packet.redstone_reset {
            let links = self.disconnect_all_links();
            return ConfigurationEffect { disconnected: None, links_disconnected: links, source_forgets: None };
        }

        for (key, amount) in &packet.input_amounts {
            if let Some(connection) = self.targeted_by.iter_mut().find(|c| c.from == *key) {
                connection.amount = *amount;
            }
        }

        let mut source_forgets = None;
        if let Some(remove) = packet.remove_connection {
            self.targeted_by.retain(|c| c.from != remove);
            source_forgets = Some(remove);
        }

        if packet.clear_promises {
            self.force_clear_promises = true;
        }
        ConfigurationEffect { disconnected: None, links_disconnected: Vec::new(), source_forgets }
    }

    /// `disconnectAllLinks`: the links' positions (each is told to `disconnect`).
    pub fn disconnect_all_links(&mut self) -> Vec<Pos> {
        let links: Vec<Pos> = self.targeted_by_links.iter().map(|c| c.from.pos).collect();
        self.targeted_by_links.clear();
        links
    }

    /// `disconnectAll`: what the caller must undo on the other side.
    pub fn disconnect_all(&mut self, here: PanelPosition) -> Disconnected {
        let links = self.disconnect_all_links();
        let sources: Vec<PanelPosition> = self.targeted_by.iter().map(|c| c.from).collect();
        let targets: Vec<PanelPosition> = self.targeting.clone();
        self.targeted_by.clear();
        self.targeting.clear();
        Disconnected { here, links, sources_stop_targeting: sources, targets_stop_being_targeted_by: targets }
    }
}

/// What a connected link says (`FactoryPanelSupportBehaviour`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkRead {
    /// Its chunk isn't loaded.
    Unloaded,
    /// The block entity or behaviour is gone.
    Missing,
    Link {
        /// `isOutput()`.
        is_output: bool,
        /// `shouldPanelBePowered()`: it is an output (a Redstone Link listening) and receives a
        /// signal.
        powers_panel: bool,
    },
}

/// What `addConnection` finds at the position clicked from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionSource {
    /// A `FactoryPanelSupportBehaviour`.
    Link,
    /// An active panel.
    Panel,
    None,
}

/// What `addConnection` did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Added {
    No,
    /// The link's `connect(this)` is called.
    Link,
    /// The source panel's `targeting` gets this panel.
    Panel { source_targets: PanelPosition },
}

/// `FactoryPanelConfigurationPacket`.
#[derive(Clone, Debug, PartialEq)]
pub struct Configuration<K> {
    pub address: String,
    pub input_amounts: Vec<(PanelPosition, i32)>,
    pub crafting_arrangement: Vec<K>,
    pub output_amount: u32,
    pub promise_clearing_interval: i32,
    pub remove_connection: Option<PanelPosition>,
    pub clear_promises: bool,
    pub reset: bool,
    pub redstone_reset: bool,
}

/// What `applySettings` asks the caller to do on other blocks.
#[derive(Clone, Debug, PartialEq)]
pub struct ConfigurationEffect {
    /// A reset disconnects everything.
    pub disconnected: Option<Disconnected>,
    /// A redstone reset disconnects these links.
    pub links_disconnected: Vec<Pos>,
    /// The removed input's panel forgets it targets this one.
    pub source_forgets: Option<PanelPosition>,
}

/// What `disconnectAll` leaves for the caller.
#[derive(Clone, Debug, PartialEq)]
pub struct Disconnected {
    pub here: PanelPosition,
    /// Links to `disconnect(this)`.
    pub links: Vec<Pos>,
    /// Input panels remove `here` from their `targeting`.
    pub sources_stop_targeting: Vec<PanelPosition>,
    /// Target panels remove `here` from their `targetedBy`.
    pub targets_stop_being_targeted_by: Vec<PanelPosition>,
}

// --- connecting panels -------------------------------------------------------------------------------

/// The refusals of `FactoryPanelConnectionHandler.checkForIssues`, as their lang keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Issue {
    ConnectionAborted,
    AlreadyConnected,
    CannotAddMoreInputs,
    SameOrientation,
    SameSurface,
    TooFarApart,
    InputInRestockMode,
    NoItem,
}

impl Issue {
    pub fn lang_key(self) -> &'static str {
        match self {
            Issue::ConnectionAborted => "factory_panel.connection_aborted",
            Issue::AlreadyConnected => "factory_panel.already_connected",
            Issue::CannotAddMoreInputs => "factory_panel.cannot_add_more_inputs",
            Issue::SameOrientation => "factory_panel.same_orientation",
            Issue::SameSurface => "factory_panel.same_surface",
            Issue::TooFarApart => "factory_panel.too_far_apart",
            Issue::InputInRestockMode => "factory_panel.input_in_restock_mode",
            Issue::NoItem => "factory_panel.no_item",
        }
    }

    /// The English text of `lang_key` (`create.<key>` in Create's `en_us.json`).
    pub fn message(self) -> &'static str {
        match self {
            Issue::ConnectionAborted => "Input connection aborted",
            Issue::AlreadyConnected => "Gauges are already connected",
            Issue::CannotAddMoreInputs => "Cannot add more inputs to this gauge",
            Issue::SameOrientation => "Blocks must have the same orientation",
            Issue::SameSurface => "Blocks must be on the same surface",
            Issue::TooFarApart => "Blocks are too far away from each other",
            Issue::InputInRestockMode => "Input gauge cannot be in restock mode",
            Issue::NoItem => "Input gauge must have an item first",
        }
    }
}

/// `FactoryPanelBlock.getStateForPlacement`: `FaceAttachedHorizontalDirectionalBlock.getStateForPlacement` (`look` is the first of
/// `getNearestLookingDirections`, `horizontal` is `getHorizontalDirection`: looking up attaches to the ceiling, down to the floor, any
/// other way to a wall facing back at the player), then a gauge on the floor faces the opposite way.
pub fn placement(look: crate::Dir, horizontal: crate::Dir) -> (super::stock_link::AttachFace, crate::Dir) {
    use super::stock_link::AttachFace;
    match look {
        crate::Dir::Up => (AttachFace::Ceiling, horizontal),
        crate::Dir::Down => (AttachFace::Floor, horizontal.opposite()),
        wall => (AttachFace::Wall, wall.opposite()),
    }
}

/// The panel a gauge item put on a block goes into when the click's spot can't be read (Create picks the slot nearest to the click,
/// `FactoryPanelBlock.getTargetedSlot`): the first free one, bottom row first.
pub fn next_free_slot(active: [bool; 4]) -> Option<Slot> {
    [Slot::BottomLeft, Slot::BottomRight, Slot::TopLeft, Slot::TopRight]
        .into_iter()
        .find(|slot| !active[Slot::ALL.iter().position(|s| s == slot).unwrap_or(0)])
}

/// `checkForIssues(from, to)` for two panels: connecting `from` as an input to `to`.
#[derive(Clone, Copy, Debug)]
pub struct PanelPair {
    /// `from == null`.
    pub from_missing: bool,
    /// `from.targetedBy.containsKey(to.getPanelPosition())`.
    pub from_targeted_by_to: bool,
    /// `from.targetedBy.size()`.
    pub from_inputs: usize,
    /// The block states are equal but for waterlogging and power.
    pub same_orientation: bool,
    /// `to.getPos() - from.getPos()`.
    pub diff: Pos,
    /// The component of `diff` along the axis of the face the gauges are on.
    pub diff_along_face_axis: i32,
    pub to_restocker: bool,
    pub to_filter_empty: bool,
    pub from_filter_empty: bool,
}

fn closer_than_zero(diff: Pos, range: i64) -> bool {
    diff.iter().map(|c| i64::from(*c) * i64::from(*c)).sum::<i64>() < range * range
}

/// `FactoryPanelConnectionHandler.checkForIssues(from, to)`: the first refusal, in Create's order.
pub fn check_panels(pair: &PanelPair) -> Option<Issue> {
    if pair.from_missing {
        return Some(Issue::ConnectionAborted);
    }
    if pair.from_targeted_by_to {
        return Some(Issue::AlreadyConnected);
    }
    if pair.from_inputs >= MAX_INPUTS {
        return Some(Issue::CannotAddMoreInputs);
    }
    if !pair.same_orientation {
        return Some(Issue::SameOrientation);
    }
    if pair.diff_along_face_axis != 0 {
        return Some(Issue::SameSurface);
    }
    if !closer_than_zero(pair.diff, CONNECTION_RANGE) {
        return Some(Issue::TooFarApart);
    }
    if pair.to_restocker {
        return Some(Issue::InputInRestockMode);
    }
    if pair.to_filter_empty || pair.from_filter_empty {
        return Some(Issue::NoItem);
    }
    None
}

/// `checkForIssues(from, FactoryPanelSupportBehaviour to)`: a redstone or display link.
pub fn check_link(from_missing: bool, same_facing: bool, diff: Pos, diff_along_face_axis: i32) -> Option<Issue> {
    if from_missing {
        return Some(Issue::ConnectionAborted);
    }
    if !same_facing {
        return Some(Issue::SameOrientation);
    }
    if diff_along_face_axis != 0 {
        return Some(Issue::SameSurface);
    }
    if !closer_than_zero(diff, CONNECTION_RANGE) {
        return Some(Issue::TooFarApart);
    }
    None
}

// --- the support behaviour --------------------------------------------------------------------------------

/// What a linked panel says to its link (`FactoryPanelSupportBehaviour.shouldBePoweredTristate`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelRead {
    Unloaded,
    /// No active panel there: it is forgotten.
    Missing,
    Present { satisfied: bool, count: i32 },
}

/// `FactoryPanelSupportBehaviour`'s list of linked panels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Support {
    pub linked_panels: Vec<PanelPosition>,
    /// `changed`.
    pub changed: bool,
}

impl Support {
    /// `connect`.
    pub fn connect(&mut self, panel: PanelPosition) {
        if self.linked_panels.contains(&panel) {
            return;
        }
        self.linked_panels.push(panel);
        self.changed = true;
    }

    /// `disconnect`.
    pub fn disconnect(&mut self, panel: PanelPosition) {
        self.linked_panels.retain(|p| *p != panel);
        self.changed = true;
    }

    /// `shouldBePoweredTristate`: true when any linked panel is satisfied with a target, false
    /// when none is, none (`null`: the link keeps its signal) while a panel is unloaded; panels
    /// that are gone are dropped from the list on the way.
    pub fn should_be_powered(&mut self, read: &dyn Fn(&PanelPosition) -> PanelRead) -> Option<bool> {
        let mut i = 0;
        while i < self.linked_panels.len() {
            match read(&self.linked_panels[i]) {
                PanelRead::Unloaded => return None,
                PanelRead::Missing => {
                    self.linked_panels.remove(i);
                    self.changed = true;
                }
                PanelRead::Present { satisfied, count } => {
                    if satisfied && count != 0 {
                        return Some(true);
                    }
                    i += 1;
                }
            }
        }
        Some(false)
    }
}

// --- what a link says and shows ----------------------------------------------------------------------------

/// `RedstoneLinkBlock.updateTransmittedSignal`: the signal a transmitting Redstone Link sends given what its neighbours give
/// (`getPower`) and what its linked panels say (`shouldBePoweredTristate`: 15 for a satisfied panel). `None` is Create's early return
/// ("Suppress update if an input panel exists but is not loaded"): the link keeps what it sent.
pub fn transmitted_power(neighbour: i32, from_panels: Option<bool>) -> Option<i32> {
    let panels = match from_panels {
        None => return None,
        Some(true) => 15,
        Some(false) => 0,
    };
    Some(neighbour.max(panels))
}

/// `FactoryGaugeDisplaySource.createEntry`'s mark before the item's name: `demand != 0` gives a tick when satisfied, an up arrow while
/// something is promised, else a small square; no demand gives a blank (`" "`).
pub fn status_symbol(demand: i64, satisfied: bool, promised: u32) -> char {
    if demand == 0 {
        ' '
    } else if satisfied {
        '\u{2714}'
    } else if promised != 0 {
        '\u{2191}'
    } else {
        '\u{25aa}'
    }
}

/// `FactoryGaugeDisplaySource.createEntry`: the row for one linked panel, as the name the list shows and the value (`getLevelInStorage`).
/// Create colours the mark with `getIngredientStatusColor`; the text here is plain. `item_name` is the filter's hover name.
pub fn display_entry<K: PartialEq + Clone>(panel: &Gauge<K>, max_stack_size: u32, item_name: &str) -> (String, i64) {
    let demand = panel.demand(max_stack_size);
    let mark = status_symbol(demand, panel.satisfied, panel.last_reported_promises);
    (format!("{mark} {item_name}"), i64::from(panel.last_reported_level_in_storage))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::logistics_network::PackagerView;

    #[test]
    fn a_link_sends_the_larger_of_its_neighbours_and_its_panels() {
        assert_eq!(transmitted_power(0, Some(false)), Some(0));
        assert_eq!(transmitted_power(7, Some(false)), Some(7));
        assert_eq!(transmitted_power(7, Some(true)), Some(15));
        assert_eq!(transmitted_power(0, Some(true)), Some(15));
        // An unloaded panel leaves the link as it was.
        assert_eq!(transmitted_power(9, None), None);
    }

    #[test]
    fn a_panels_row_has_a_mark_and_its_level() {
        assert_eq!(status_symbol(0, true, 3), ' ');
        assert_eq!(status_symbol(64, true, 0), '\u{2714}');
        assert_eq!(status_symbol(64, false, 5), '\u{2191}');
        assert_eq!(status_symbol(64, false, 0), '\u{25aa}');
        let mut panel: Gauge<&str> = Gauge::new(Slot::BottomLeft, 1);
        panel.count = 2;
        panel.up_to = false;
        panel.last_reported_level_in_storage = 130;
        // Two stacks of 16 is a demand of 32, so a panel with nothing promised and not satisfied is a small square.
        assert_eq!(display_entry(&panel, 16, "Ender Pearl"), ("\u{25aa} Ender Pearl".to_owned(), 130));
        panel.satisfied = true;
        assert_eq!(display_entry(&panel, 16, "Ender Pearl").0, "\u{2714} Ender Pearl");
        panel.count = 0;
        assert_eq!(display_entry(&panel, 16, "Ender Pearl").0, "  Ender Pearl");
    }

    #[test]
    fn a_panel_connects_to_a_link_once_and_the_link_remembers_it() {
        let here = PanelPosition { pos: [0, 64, 0], slot: Slot::BottomLeft };
        let link = PanelPosition { pos: [3, 64, 0], slot: Slot::TopLeft };
        let mut panel: Gauge<&str> = Gauge::new(Slot::BottomLeft, 1);
        panel.enable();
        assert_eq!(panel.add_connection(here, link, ConnectionSource::Link, false), Added::Link);
        let again = PanelPosition { pos: [3, 64, 0], slot: Slot::BottomRight };
        assert_eq!(panel.add_connection(here, again, ConnectionSource::Link, false), Added::Link);
        assert_eq!(panel.targeted_by_links.len(), 1);
        assert_eq!(panel.targeted_by_links[0].from, again);
        // A powered Redstone Link receiving a signal pauses the panel, until it stops.
        let reads = [LinkRead::Link { is_output: true, powers_panel: true }];
        assert!(panel.check_for_redstone_input(&reads));
        assert!(panel.redstone_powered);
        assert!(!panel.check_for_redstone_input(&reads));
        assert!(panel.check_for_redstone_input(&[LinkRead::Link { is_output: true, powers_panel: false }]));
        // A transmitting link or a Display Link (not an output) never pauses it, and is told when the panel changes.
        let display = [(link.pos, LinkRead::Link { is_output: false, powers_panel: false })];
        assert_eq!(panel.links_to_notify(&display), vec![link.pos]);
        assert_eq!(panel.disconnect_all_links(), vec![link.pos]);
    }

    #[test]
    fn a_gauge_faces_back_at_the_player_and_a_floor_gauge_the_other_way() {
        use crate::Dir;
        use crate::rules::stock_link::AttachFace;
        assert_eq!(placement(Dir::North, Dir::South), (AttachFace::Wall, Dir::South));
        assert_eq!(placement(Dir::Up, Dir::East), (AttachFace::Ceiling, Dir::East));
        assert_eq!(placement(Dir::Down, Dir::East), (AttachFace::Floor, Dir::West));
    }

    #[test]
    fn a_new_panel_takes_the_first_free_slot_bottom_row_first() {
        // `Slot::ALL` order: top left, top right, bottom left, bottom right.
        assert_eq!(next_free_slot([false; 4]), Some(Slot::BottomLeft));
        assert_eq!(next_free_slot([false, false, true, false]), Some(Slot::BottomRight));
        assert_eq!(next_free_slot([false, false, true, true]), Some(Slot::TopLeft));
        assert_eq!(next_free_slot([true; 4]), None);
    }

    #[test]
    fn every_refusal_has_a_message() {
        for issue in [
            Issue::ConnectionAborted,
            Issue::AlreadyConnected,
            Issue::CannotAddMoreInputs,
            Issue::SameOrientation,
            Issue::SameSurface,
            Issue::TooFarApart,
            Issue::InputInRestockMode,
            Issue::NoItem,
        ] {
            assert!(!issue.message().is_empty());
            assert!(issue.lang_key().starts_with("factory_panel."));
        }
    }

    struct Fixed;
    impl Random for Fixed {
        fn next_int(&mut self) -> i32 {
            7
        }
        fn next_below(&mut self, _: usize) -> usize {
            0
        }
    }

    const NET: Freq = 1;

    fn summary(stacks: &[(&'static str, u32)]) -> Summary<&'static str> {
        let mut s = Summary::default();
        for (k, c) in stacks {
            s.add(k, i64::from(*c));
        }
        s
    }

    fn panel(x: i32, slot: Slot) -> PanelPosition {
        PanelPosition { pos: [x, 64, 0], slot }
    }

    fn reads(in_storage: u32) -> Reads {
        Reads { restocker: false, unloaded_links: 0, level_in_storage: in_storage, max_stack_size: 64 }
    }

    fn gauge(filter: &'static str, count: i32) -> Gauge<&'static str> {
        let mut g = Gauge::new(Slot::TopLeft, NET);
        g.active = true;
        g.filter = Some(filter);
        g.count = count;
        g
    }

    #[test]
    fn a_gauge_reaches_its_target_and_stops() {
        let mut g = gauge("gear", 10);
        // 4 of 10: not satisfied, nothing promised. The first reading is a change.
        let tick = g.tick_storage_monitor(&reads(4), 0);
        assert_eq!(tick, StorageTick { changed: true, confirm: false, notify_outputs: false });
        assert!(!g.satisfied && !g.promised_satisfied);
        // The same reading again changes nothing.
        assert_eq!(g.tick_storage_monitor(&reads(4), 0), StorageTick::default());
        // Reaching 10: satisfied, the confirm sounds play and the outputs are told.
        let tick = g.tick_storage_monitor(&reads(10), 0);
        assert_eq!(tick, StorageTick { changed: true, confirm: true, notify_outputs: true });
        assert!(g.satisfied && g.promised_satisfied);
        // A satisfied gauge asks for nothing, however long it waits.
        g.targeted_by.push(Connection::new(panel(1, Slot::TopLeft), 2));
        g.recipe_address = "Gears".into();
        let mut networks: Networks<&'static str> = Networks::default();
        let mut rng = Fixed;
        let mut env = RequestEnv {
            source: &|_| panic!("a satisfied gauge doesn't look at its inputs"),
            accurate_summary: &|_| panic!("not asked"),
            links: &|_| panic!("not asked"),
            rng: &mut rng,
            networks: &mut networks,
            interval: 100,
            restocked_packager_has_inventory: None,
        };
        for _ in 0..500 {
            assert_eq!(g.tick_requests([0, 64, 0], &reads(10), &mut env), RequestOutcome::default());
        }
        // In stacks mode the target is multiplied by the stack size: 1 stack needs 64.
        let mut stacks = gauge("gear", 1);
        stacks.up_to = false;
        stacks.tick_storage_monitor(&reads(63), 0);
        assert!(!stacks.satisfied);
        stacks.tick_storage_monitor(&reads(64), 0);
        assert!(stacks.satisfied);
        // Without an item the panel is always satisfied.
        let mut empty = Gauge::<&'static str>::new(Slot::TopLeft, NET);
        empty.tick_storage_monitor(&reads(0), 0);
        assert!(empty.satisfied && empty.promised_satisfied);
        // A target of 0 is satisfied by 0 and doesn't confirm.
        let mut zero = gauge("gear", 0);
        assert!(!zero.tick_storage_monitor(&reads(0), 0).confirm);
    }

    #[test]
    fn a_gauge_with_promised_items_waits() {
        let mut g = gauge("gear", 10);
        let mut networks: Networks<&'static str> = Networks::default();
        networks.link_added(NET, crate::rules::logistics_network::GlobalPos { dimension: "o".into(), pos: [0, 0, 0] }, None);
        networks.queued_promises(NET).unwrap().add(Promise::new(("gear", 6)));
        let promised = g.promised(false, networks.queued_promises(NET), 100);
        assert_eq!(promised, 6);
        // 4 in storage and 6 promised: promised-satisfied, not satisfied.
        let tick = g.tick_storage_monitor(&reads(4), promised);
        assert!(tick.changed && !tick.confirm);
        assert!(!g.satisfied && g.promised_satisfied);
        // It asks for nothing while promised-satisfied.
        g.targeted_by.push(Connection::new(panel(1, Slot::TopLeft), 2));
        g.recipe_address = "Gears".into();
        let mut rng = Fixed;
        let mut env = RequestEnv {
            source: &|_| panic!("not asked while waiting"),
            accurate_summary: &|_| panic!("not asked"),
            links: &|_| panic!("not asked"),
            rng: &mut rng,
            networks: &mut networks,
            interval: 100,
            restocked_packager_has_inventory: None,
        };
        assert_eq!(g.tick_requests([0, 64, 0], &reads(4), &mut env), RequestOutcome::default());
        // 30 seconds on (interval 0 = 600 ticks) the promise expires, and the gauge is short again.
        g.promise_clearing_interval = 0;
        for _ in 0..600 {
            env.networks.tick();
        }
        let promised = g.promised(false, env.networks.queued_promises(NET), 100);
        assert_eq!(promised, 0);
        g.tick_storage_monitor(&reads(4), promised);
        assert!(!g.promised_satisfied);
        // Unloaded links make it wait, and it asks the network for a fresh summary once they load.
        let loading = Reads { unloaded_links: 2, ..reads(0) };
        let tick = g.tick_storage_monitor(&loading, 0);
        assert!(tick.changed && g.waiting_for_network);
        assert!(g.wants_fresh_summary(false, 0));
        g.tick_storage_monitor(&reads(4), 0);
        assert!(!g.wants_fresh_summary(false, 0));
        // Forced clear drops the item's promises and shortens the timer.
        let mut cleared = gauge("gear", 10);
        let q = env.networks.queued_promises(NET).unwrap();
        q.add(Promise::new(("gear", 5)));
        cleared.force_clear_promises = true;
        assert_eq!(cleared.promised(false, Some(q), 100), 0);
        assert_eq!(cleared.timer, 50);
        assert!(!cleared.force_clear_promises);
    }

    fn links_with(stock: &[(&'static str, u32)], id: PackagerId, queued: usize) -> Vec<LinkView<&'static str>> {
        vec![LinkView {
            id: 1,
            power: 0,
            packager: Some(PackagerView { id, inventory: Some(id), targets_ignored: false, queued, available: summary(stock) }),
        }]
    }

    #[test]
    fn a_gauge_asks_for_a_recipes_ingredients_and_keeps_a_promise() {
        // A gauge for gears (target 10) fed by two panels: 2 iron plates and 1 cogwheel per craft,
        // making 1 gear per craft. Inputs on the same network.
        let mut g = gauge("gear", 10);
        g.recipe_address = "Press".into();
        g.recipe_output = 1;
        let plate = panel(1, Slot::TopLeft);
        let cog = panel(2, Slot::TopLeft);
        g.targeted_by = vec![Connection::new(plate, 2), Connection::new(cog, 1)];
        g.active_crafting_arrangement = vec!["plate", "plate", "cog"];
        g.tick_storage_monitor(&reads(0), 0);

        let mut networks: Networks<&'static str> = Networks::default();
        networks.link_added(NET, crate::rules::logistics_network::GlobalPos { dimension: "o".into(), pos: [0, 0, 0] }, None);
        let mut rng = Fixed;
        let source = |p: &PanelPosition| {
            Some(Source { filter: Some(if *p == plate { "plate" } else { "cog" }), network: NET })
        };
        let stock = summary(&[("plate", 40), ("cog", 5)]);
        let accurate = |_: Freq| stock.clone();
        let links = |_: Freq| links_with(&[("plate", 40), ("cog", 5)], 9, 0);
        let mut env = RequestEnv {
            source: &source,
            accurate_summary: &accurate,
            links: &links,
            rng: &mut rng,
            networks: &mut networks,
            interval: 100,
            restocked_packager_has_inventory: None,
        };

        // The first tick: the timer is 0, so it asks at once.
        let out = g.tick_requests([0, 64, 0], &reads(0), &mut env);
        let here = g.position([0, 64, 0]);
        assert_eq!(
            out.effects,
            vec![Effect { from: plate, to: here, success: true }, Effect { from: cog, to: here, success: true }]
        );
        assert_eq!(out.dispatches.len(), 1);
        let requests = &out.dispatches[0].requests;
        assert_eq!(requests.iter().map(|r| (r.item, r.count)).collect::<Vec<_>>(), vec![("plate", 2), ("cog", 1)]);
        assert!(requests.iter().all(|r| r.address == "Press"));
        // The first request carries the order and the gauge's crafting arrangement as one recipe.
        let context = requests[0].context.as_ref().unwrap();
        assert_eq!(context.stacks, vec![("plate", 2), ("cog", 1)]);
        assert_eq!(context.crafts.len(), 1);
        assert_eq!(context.crafts[0].pattern, vec![("plate", 1), ("plate", 1), ("cog", 1)]);
        assert!(requests[1].final_link);
        // A promise for the gear is kept on the network, and the timer restarts.
        assert_eq!(out.promised, Some(Promise::new(("gear", 1))));
        assert_eq!(env.networks.queued_promises(NET).unwrap().flatten(false), vec![Promise::new(("gear", 1))]);
        assert!(out.advancement);
        assert_eq!(g.timer, 100);
        // It waits an interval before asking again: 100 ticks of counting down (101st asks).
        let mut asked = None;
        for tick in 1..=101 {
            if !g.tick_requests([0, 64, 0], &reads(0), &mut env).dispatches.is_empty() {
                asked = Some(tick);
                break;
            }
        }
        assert_eq!(asked, Some(101), "100 ticks of waiting, then it asks on the next");
    }

    #[test]
    fn a_gauge_short_of_an_ingredient_asks_for_nothing() {
        let mut g = gauge("gear", 10);
        g.recipe_address = "Press".into();
        let plate = panel(1, Slot::TopLeft);
        g.targeted_by = vec![Connection::new(plate, 2)];
        let mut networks: Networks<&'static str> = Networks::default();
        let mut rng = Fixed;
        let stock = summary(&[("plate", 1)]);
        let source = |_: &PanelPosition| Some(Source { filter: Some("plate"), network: NET });
        let accurate = |_: Freq| stock.clone();
        let links = |_: Freq| links_with(&[("plate", 1)], 9, 0);
        let mut env = RequestEnv {
            source: &source,
            accurate_summary: &accurate,
            links: &links,
            rng: &mut rng,
            networks: &mut networks,
            interval: 100,
            restocked_packager_has_inventory: None,
        };
        let out = g.tick_requests([0, 64, 0], &reads(0), &mut env);
        assert_eq!(out.effects, vec![Effect { from: plate, to: g.position([0, 64, 0]), success: false }]);
        assert!(out.dispatches.is_empty() && out.promised.is_none());
        assert_eq!(g.timer, 100, "the timer was reset anyway");
        // No address: it does nothing (but resets the timer).
        g.timer = 0;
        g.recipe_address = "  ".into();
        assert_eq!(g.tick_requests([0, 64, 0], &reads(0), &mut env), RequestOutcome::default());
        // A missing input panel cancels the request.
        g.timer = 0;
        g.recipe_address = "Press".into();
        let gone = |_: &PanelPosition| None;
        env.source = &gone;
        assert_eq!(g.tick_requests([0, 64, 0], &reads(0), &mut env), RequestOutcome::default());
        // A busy packager (10 boxes queued) cancels it all.
        g.timer = 0;
        let rich = summary(&[("plate", 40)]);
        let accurate = |_: Freq| rich.clone();
        let busy = |_: Freq| links_with(&[("plate", 40)], 9, 10);
        env.source = &source;
        env.accurate_summary = &accurate;
        env.links = &busy;
        let out = g.tick_requests([0, 64, 0], &reads(0), &mut env);
        assert!(out.dispatches.is_empty() && out.promised.is_none());
        assert_eq!(out.effects.len(), 1, "the effect was already sent");
    }

    #[test]
    fn a_restocker_orders_up_to_nine_stacks_and_promises_them() {
        // On a packager: target 2 stacks (128) of cobble, 20 in storage, 30 promised: asks for 78.
        let mut g = gauge("cobble", 2);
        g.up_to = false;
        g.recipe_address = "Self".into();
        g.restocker_promises.add(Promise::new(("cobble", 30)));
        let mut networks: Networks<&'static str> = Networks::default();
        let mut rng = Fixed;
        let accurate = |_: Freq| Summary::default();
        let source = |_: &PanelPosition| None;
        let links = |_: Freq| links_with(&[("cobble", 1000)], 9, 0);
        let mut env = RequestEnv {
            source: &source,
            accurate_summary: &accurate,
            links: &links,
            rng: &mut rng,
            networks: &mut networks,
            interval: 100,
            restocked_packager_has_inventory: Some(true),
        };
        let restocker = Reads { restocker: true, unloaded_links: 0, level_in_storage: 20, max_stack_size: 64 };
        let out = g.tick_requests([0, 64, 0], &restocker, &mut env);
        let here = g.position([0, 64, 0]);
        assert_eq!(out.effects, vec![Effect { from: here, to: here, success: true }]);
        assert_eq!(out.dispatches[0].requests[0].count, 78);
        assert_eq!(out.promised, Some(Promise::new(("cobble", 78))));
        assert_eq!(g.restocker_promises.flatten(false).len(), 2);
        // Never more than nine stacks (576), and never more than the network has.
        let mut big = gauge("cobble", 100);
        big.up_to = false;
        big.recipe_address = "Self".into();
        let out = big.tick_requests([0, 64, 0], &Reads { level_in_storage: 0, ..restocker }, &mut env);
        assert_eq!(out.dispatches[0].requests[0].count, 576);
        let few = |_: Freq| links_with(&[("cobble", 10)], 9, 0);
        env.links = &few;
        let mut small = gauge("cobble", 100);
        small.up_to = false;
        small.recipe_address = "Self".into();
        let out = small.tick_requests([0, 64, 0], &Reads { level_in_storage: 0, ..restocker }, &mut env);
        assert_eq!(out.dispatches[0].requests[0].count, 10);
        // Nothing on the network: a failure effect, no promise.
        let none = |_: Freq| links_with(&[], 9, 0);
        env.links = &none;
        let mut dry = gauge("cobble", 100);
        dry.recipe_address = "Self".into();
        let out = dry.tick_requests([0, 64, 0], &restocker, &mut env);
        assert!(!out.effects[0].success);
        assert!(out.promised.is_none());
        // A restocker with no packager inventory asks for nothing, and a restocker ignores inputs.
        env.restocked_packager_has_inventory = Some(false);
        let mut idle = gauge("cobble", 100);
        idle.recipe_address = "Self".into();
        assert_eq!(idle.tick_requests([0, 64, 0], &restocker, &mut env), RequestOutcome::default());
    }

    #[test]
    fn redstone_links_pause_a_gauge_and_are_told_when_it_is_satisfied() {
        let mut g = gauge("gear", 10);
        g.targeted_by_links = vec![Connection::new(panel(5, Slot::BottomLeft), 1)];
        let on = LinkRead::Link { is_output: true, powers_panel: true };
        let off = LinkRead::Link { is_output: true, powers_panel: false };
        assert!(g.check_for_redstone_input(&[on]));
        assert!(g.redstone_powered);
        assert_eq!(g.timer, 1);
        assert!(!g.check_for_redstone_input(&[on]), "no change");
        // An unloaded link stops the check.
        assert!(!g.check_for_redstone_input(&[LinkRead::Unloaded]));
        assert!(g.redstone_powered);
        assert!(g.check_for_redstone_input(&[off]));
        assert!(!g.redstone_powered);
        // A paused gauge doesn't ask.
        g.redstone_powered = true;
        g.recipe_address = "x".into();
        g.targeted_by.push(Connection::new(panel(1, Slot::TopLeft), 1));
        let mut networks: Networks<&'static str> = Networks::default();
        let mut rng = Fixed;
        let mut env = RequestEnv {
            source: &|_| panic!("paused"),
            accurate_summary: &|_| panic!("paused"),
            links: &|_| panic!("paused"),
            rng: &mut rng,
            networks: &mut networks,
            interval: 100,
            restocked_packager_has_inventory: None,
        };
        assert_eq!(g.tick_requests([0, 64, 0], &reads(0), &mut env), RequestOutcome::default());
        // Links are told of a change of satisfaction, up to the first output or missing one.
        let input = LinkRead::Link { is_output: false, powers_panel: false };
        let p = |x| [x, 64, 0];
        assert_eq!(g.links_to_notify(&[(p(1), input), (p(2), input), (p(3), on), (p(4), input)]), vec![p(1), p(2)]);
        assert_eq!(g.links_to_notify(&[(p(1), LinkRead::Missing), (p(2), input)]), Vec::<Pos>::new());
        // The link's view of its panels.
        let mut support = Support::default();
        support.connect(panel(0, Slot::TopLeft));
        support.connect(panel(0, Slot::TopLeft));
        support.connect(panel(0, Slot::TopRight));
        assert_eq!(support.linked_panels.len(), 2);
        let read = |p: &PanelPosition| match p.slot {
            Slot::TopLeft => PanelRead::Present { satisfied: true, count: 0 },
            _ => PanelRead::Present { satisfied: true, count: 5 },
        };
        assert_eq!(support.should_be_powered(&read), Some(true));
        let gone = |p: &PanelPosition| if p.slot == Slot::TopLeft { PanelRead::Missing } else { PanelRead::Present { satisfied: false, count: 5 } };
        assert_eq!(support.should_be_powered(&gone), Some(false));
        assert_eq!(support.linked_panels, vec![panel(0, Slot::TopRight)]);
        assert_eq!(support.should_be_powered(&|_| PanelRead::Unloaded), None);
    }

    #[test]
    fn connecting_panels_follows_create_s_checks_in_order() {
        let ok = PanelPair {
            from_missing: false,
            from_targeted_by_to: false,
            from_inputs: 0,
            same_orientation: true,
            diff: [3, 0, 2],
            diff_along_face_axis: 0,
            to_restocker: false,
            to_filter_empty: false,
            from_filter_empty: false,
        };
        assert_eq!(check_panels(&ok), None);
        assert_eq!(check_panels(&PanelPair { from_missing: true, ..ok }), Some(Issue::ConnectionAborted));
        assert_eq!(check_panels(&PanelPair { from_targeted_by_to: true, from_inputs: 9, ..ok }), Some(Issue::AlreadyConnected));
        assert_eq!(check_panels(&PanelPair { from_inputs: 9, ..ok }), Some(Issue::CannotAddMoreInputs));
        assert_eq!(check_panels(&PanelPair { same_orientation: false, ..ok }), Some(Issue::SameOrientation));
        assert_eq!(check_panels(&PanelPair { diff_along_face_axis: 1, ..ok }), Some(Issue::SameSurface));
        // `closerThan(ZERO, 16)`: 15 blocks away is in, 16 is out.
        assert_eq!(check_panels(&PanelPair { diff: [15, 0, 0], ..ok }), None);
        assert_eq!(check_panels(&PanelPair { diff: [16, 0, 0], ..ok }), Some(Issue::TooFarApart));
        assert_eq!(check_panels(&PanelPair { to_restocker: true, to_filter_empty: true, ..ok }), Some(Issue::InputInRestockMode));
        assert_eq!(check_panels(&PanelPair { from_filter_empty: true, ..ok }), Some(Issue::NoItem));
        assert_eq!(Issue::TooFarApart.lang_key(), "factory_panel.too_far_apart");
        assert_eq!(check_link(false, true, [1, 0, 0], 0), None);
        assert_eq!(check_link(false, false, [1, 0, 0], 0), Some(Issue::SameOrientation));
        assert_eq!(check_link(false, true, [0, 16, 0], 0), Some(Issue::TooFarApart));
    }

    #[test]
    fn inputs_are_limited_and_configuration_changes_them() {
        let here = panel(0, Slot::TopLeft);
        let mut g = gauge("gear", 10);
        for x in 1..=9 {
            assert_eq!(g.add_connection(here, panel(x, Slot::TopLeft), ConnectionSource::Panel, false), Added::Panel { source_targets: here });
        }
        assert_eq!(g.add_connection(here, panel(10, Slot::TopLeft), ConnectionSource::Panel, false), Added::No, "nine inputs at most");
        assert_eq!(g.add_connection(here, panel(1, Slot::BottomLeft), ConnectionSource::Link, false), Added::Link);
        assert_eq!(g.targeted_by.len(), 9);
        assert_eq!(g.targeted_by_links.len(), 1);
        let mut restocker = gauge("gear", 10);
        assert_eq!(restocker.add_connection(here, panel(1, Slot::TopLeft), ConnectionSource::Panel, true), Added::No);
        // Configuration sets the address, output, arrangement and amounts, and removes an input.
        let packet = Configuration {
            address: "Press".into(),
            input_amounts: vec![(panel(1, Slot::TopLeft), 4)],
            crafting_arrangement: vec!["a", "b"],
            output_amount: 3,
            promise_clearing_interval: 2,
            remove_connection: Some(panel(2, Slot::TopLeft)),
            clear_promises: true,
            reset: false,
            redstone_reset: false,
        };
        let effect = g.apply_configuration(here, &packet);
        assert_eq!(effect.source_forgets, Some(panel(2, Slot::TopLeft)));
        assert_eq!(g.targeted_by.len(), 8);
        assert_eq!(g.targeted_by[0].amount, 4);
        assert_eq!((g.recipe_address.as_str(), g.recipe_output, g.promise_clearing_interval), ("Press", 3, 2));
        assert!(g.force_clear_promises);
        assert_eq!(g.promise_expiry_time(), 2 * 20 * 60);
        // A reset disconnects everything and empties the panel.
        let effect = g.apply_configuration(here, &Configuration { reset: true, ..packet.clone() });
        let gone = effect.disconnected.unwrap();
        assert_eq!(gone.sources_stop_targeting.len(), 8);
        assert_eq!(gone.links, vec![[1, 64, 0]]);
        assert!(g.targeted_by.is_empty() && g.filter.is_none() && g.count == 0);
        assert_eq!((g.recipe_address.as_str(), g.recipe_output, g.promise_clearing_interval), ("", 1, -1));
        // Target settings: never below zero, the row picks items or stacks.
        assert!(g.set_value_settings(1, -5, 100));
        assert_eq!((g.count, g.up_to), (0, false));
        assert!(!g.set_value_settings(1, 0, 100));
        assert!(g.set_value_settings(0, 64, 100));
        assert_eq!(g.timer, 50);
        // A filter item can't be the target.
        assert!(!g.set_filter(Some("filter"), true));
        assert!(g.set_filter(Some("gear"), false));
        assert_eq!(cycle_arrow_bend(3), 0);
        assert_eq!(cycle_arrow_bend(-1), 0);
    }

    #[test]
    fn a_missing_address_is_flagged_while_a_target_is_set() {
        let mut g = gauge("gear", 5);
        assert!(!g.is_missing_address(false), "no inputs, not a restocker");
        assert!(g.is_missing_address(true));
        g.recipe_address = "Home".into();
        assert!(!g.is_missing_address(true));
        g.count = 0;
        g.recipe_address.clear();
        assert!(!g.is_missing_address(true), "no target set");
        assert_eq!(Slot::TopLeft.x_offset() + Slot::TopLeft.y_offset(), 2);
        assert_eq!(Slot::BottomRight.x_offset() + Slot::BottomRight.y_offset(), 0);
    }
}
