//! The logistics network: Stock Links on a frequency (`LogisticallyLinkedBehaviour`,
//! `PackagerLinkBlockEntity`, `PackagerLinkBlock.getPower`), the table of networks
//! (`GlobalLogisticsManager`, `LogisticsNetwork`), the summary of what a network holds
//! (`LogisticsManager.getSummaryOfNetwork`, `createSummaryOfNetwork`, `getStockOf`), and how an
//! order is split across the packagers that hold the goods (`LogisticsManager.
//! findPackagersForRequest`, `broadcastPackageRequest`, `performPackageRequests`). Request
//! promises are `rules::request_promises`; what a packager does with a request is
//! `rules::package::pack`.
//!
//! No game types. What the rules read of the game is plain input the caller fills in:
//! - a [`LinkView`] per link present on the frequency, holding the packager behind it as a
//!   [`PackagerView`] (its id, the inventory it targets, how many boxes wait to leave it, and
//!   what it holds: `PackagerBlockEntity.getAvailableItems`, see [`available_items`]);
//! - the game tick (`TickBasedCache.currentTick`) as `now`;
//! - the random numbers (`Random`) the order id and the choice between links on one inventory
//!   take.
//!
//! What the rules decide is plain output: the requests each packager should be given
//! ([`Dispatch`]; the glue calls `PackagerBlockEntity.attemptToSend` on them as
//! [`run_attempts`] does, then flashes the link and re-reads the packager's items), the summary,
//! the arrivals a packager saw (`submitNewArrivals`, see [`new_arrivals`]).
//!
//! Known differences from Create, for the README:
//! - Create's `HashMap`/`HashMultimap`/`ConcurrentHashMap` orders aren't defined. Links keep
//!   the order of their ids (their creation), links on one inventory are grouped in the order
//!   their first member appears, and each packager's requests are in the order they were made.
//! - `PackagerBlockEntity`'s inventory version tracker (`invVersionTracker`, which only saves
//!   it from re-reading a handler that didn't change) is the caller's cache.
//! - The `WeakReference`s of the link caches are the caller's: a link whose block entity is
//!   gone, removed or in an unloaded chunk (`isValidLink`) is left out of the [`LinkView`]s.
//! - `LogisticsNetworkSavedData` (saving the networks) is the caller's: the data is public.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::package::{Big, Context, Request, Summary};
use super::packager::{RequestType, too_busy_for};
use super::request_promises::Queue;
use crate::{Dir, Pos};

/// A network's frequency (`LogisticallyLinkedBehaviour.freqId`, a UUID).
pub type Freq = u128;
/// `LogisticallyLinkedBehaviour.linkId` (`LINK_ID_GENERATOR`).
pub type LinkId = u32;
/// A player's UUID.
pub type PlayerId = u128;
/// Names a packager to the glue.
pub type PackagerId = u64;
/// `InventoryIdentifier`: two packagers with the same one target the same inventory.
pub type InventoryId = u64;

/// `LogisticsManager.SUMMARIES`: `new TickBasedCache<>(20, false)`.
pub const SUMMARY_TIMEOUT: i64 = 20;
/// `LogisticsManager.ACCURATE_SUMMARIES`: `new TickBasedCache<>(1, false)`.
pub const ACCURATE_SUMMARY_TIMEOUT: i64 = 1;
/// The inner cache of `LogisticallyLinkedBehaviour.LINKS`: `new TickBasedCache<>(400, false)`.
pub const LINK_TIMEOUT: i64 = 400;
/// `LogisticallyLinkedBehaviour.LINKS`: `new TickBasedCache<>(20, true)`.
pub const LINK_GROUP_TIMEOUT: i64 = 20;
/// `PackagerLinkBlockEntity` and `StockCheckingBlockEntity`: `setLazyTickRate(10)`.
pub const LAZY_PERIOD: i64 = 10;
/// `performPackageRequests`: `for (int i = 0; i < 100; i++)`.
pub const PERFORM_ATTEMPTS: usize = 100;
/// A link powered this much isn't on the network (`redstonePower == 15`).
pub const SILENCING_POWER: u8 = 15;

// --- the cache --------------------------------------------------------------------------------

/// `TickBasedCache`: entries that time out `timeout` ticks after they were written (or read,
/// with `reset_on_access`). `now` is `TickBasedCache.ticks()`.
#[derive(Clone, Debug)]
pub struct TickCache<K, V> {
    entries: BTreeMap<K, (V, i64)>,
    timeout: i64,
    reset_on_access: bool,
}

impl<K: Ord + Clone, V> TickCache<K, V> {
    pub fn new(timeout: i64, reset_on_access: bool) -> Self {
        Self { entries: BTreeMap::new(), timeout, reset_on_access }
    }

    /// `getIfPresent`: a timed-out entry is dropped and gone; a live one's timer is reset when
    /// the cache does that.
    pub fn get_if_present(&mut self, key: &K, now: i64) -> Option<&mut V> {
        let stamp = self.entries.get(key)?.1;
        if stamp < now - self.timeout {
            self.entries.remove(key);
            return None;
        }
        let entry = self.entries.get_mut(key)?;
        if self.reset_on_access {
            entry.1 = now;
        }
        Some(&mut entry.0)
    }

    /// `get(key, loader)`.
    pub fn get_or_insert_with(&mut self, key: &K, now: i64, load: impl FnOnce() -> V) -> &mut V {
        if self.get_if_present(key, now).is_none() {
            self.entries.insert(key.clone(), (load(), now));
        }
        &mut self.entries.get_mut(key).expect("just inserted").0
    }

    /// `put`.
    pub fn put(&mut self, key: K, value: V, now: i64) {
        self.entries.insert(key, (value, now));
    }

    /// `invalidate`.
    pub fn invalidate(&mut self, key: &K) {
        self.entries.remove(key);
    }

    /// `cleanUp`: timed-out entries go; with `reset_on_access`, every entry's timer is reset
    /// (as Create's does, to every entry).
    pub fn clean_up(&mut self, now: i64) {
        let timeout = self.timeout;
        let reset = self.reset_on_access;
        let mut outdated = Vec::new();
        for (key, (_, stamp)) in &mut self.entries {
            if *stamp < now - timeout {
                outdated.push(key.clone());
            }
            if reset {
                *stamp = now;
            }
        }
        for key in outdated {
            self.entries.remove(&key);
        }
    }

    /// `asMap().keySet()`, after `cleanUp`.
    pub fn keys(&mut self, now: i64) -> Vec<K> {
        self.clean_up(now);
        self.entries.keys().cloned().collect()
    }
}

// --- the table of networks ----------------------------------------------------------------------

/// `GlobalPos`: a block in a dimension.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GlobalPos {
    pub dimension: String,
    pub pos: Pos,
}

/// `LogisticsNetwork`.
#[derive(Clone, Debug)]
pub struct Network<K> {
    pub id: Freq,
    /// `panelPromises`: what Factory Gauges asked this network for.
    pub promises: Queue<K>,
    pub total_links: BTreeSet<GlobalPos>,
    pub loaded_links: BTreeSet<GlobalPos>,
    pub owner: Option<PlayerId>,
    pub locked: bool,
}

impl<K> Network<K> {
    pub fn new(id: Freq) -> Self {
        Self {
            id,
            promises: Queue::default(),
            total_links: BTreeSet::new(),
            loaded_links: BTreeSet::new(),
            owner: None,
            locked: false,
        }
    }
}

/// `GlobalLogisticsManager`: `Create.LOGISTICS`.
#[derive(Clone, Debug)]
pub struct Networks<K> {
    pub networks: HashMap<Freq, Network<K>>,
    /// `markDirty`: the data should be saved. The caller reads and clears it.
    pub dirty: bool,
}

impl<K> Default for Networks<K> {
    fn default() -> Self {
        Self { networks: HashMap::new(), dirty: false }
    }
}

impl<K: PartialEq + Clone> Networks<K> {
    /// `mayInteract`: an unknown, unowned or unlocked network, or its owner.
    pub fn may_interact(&self, network: Freq, player: PlayerId) -> bool {
        match self.networks.get(&network) {
            None => true,
            Some(n) => n.owner.is_none() || !n.locked || n.owner == Some(player),
        }
    }

    /// `mayAdministrate`: an unknown or unowned network, or its owner.
    pub fn may_administrate(&self, network: Freq, player: PlayerId) -> bool {
        match self.networks.get(&network) {
            None => true,
            Some(n) => n.owner.is_none() || n.owner == Some(player),
        }
    }

    /// `isLockable`: the network exists.
    pub fn is_lockable(&self, network: Freq) -> bool {
        self.networks.contains_key(&network)
    }

    /// `isLocked`.
    pub fn is_locked(&self, network: Freq) -> bool {
        self.networks.get(&network).is_some_and(|n| n.locked)
    }

    /// `StockKeeperLockPacket.applySettings`: the owner (or anyone, when unowned) locks or
    /// unlocks the network.
    pub fn set_locked(&mut self, network: Freq, player: PlayerId, lock: bool) {
        if !self.may_administrate(network, player) {
            return;
        }
        if let Some(n) = self.networks.get_mut(&network) {
            n.locked = lock;
            self.dirty = true;
        }
    }

    /// `linkAdded`: the link joins the network (made if new); the first owner stays.
    pub fn link_added(&mut self, network: Freq, pos: GlobalPos, owned_by: Option<PlayerId>) {
        let entry = self.networks.entry(network).or_insert_with(|| Network::new(network));
        entry.total_links.insert(pos);
        if owned_by.is_some() && entry.owner.is_none() {
            entry.owner = owned_by;
        }
        self.dirty = true;
    }

    /// `linkLoaded`.
    pub fn link_loaded(&mut self, network: Freq, pos: GlobalPos) {
        self.networks.entry(network).or_insert_with(|| Network::new(network)).loaded_links.insert(pos);
    }

    /// `linkRemoved`: the network goes with its last link.
    pub fn link_removed(&mut self, network: Freq, pos: &GlobalPos) {
        let Some(n) = self.networks.get_mut(&network) else {
            return;
        };
        n.total_links.remove(pos);
        n.loaded_links.remove(pos);
        if n.total_links.is_empty() {
            self.networks.remove(&network);
        }
        self.dirty = true;
    }

    /// `linkInvalidated`: the link's chunk unloaded.
    pub fn link_invalidated(&mut self, network: Freq, pos: &GlobalPos) {
        if let Some(n) = self.networks.get_mut(&network) {
            n.loaded_links.remove(pos);
        }
    }

    /// `getUnloadedLinkCount`.
    pub fn unloaded_link_count(&self, network: Freq) -> usize {
        self.networks.get(&network).map_or(0, |n| n.total_links.len().saturating_sub(n.loaded_links.len()))
    }

    /// `getQueuedPromises`.
    pub fn queued_promises(&mut self, network: Freq) -> Option<&mut Queue<K>> {
        self.networks.get_mut(&network).map(|n| &mut n.promises)
    }

    /// `hasQueuedPromises`.
    pub fn has_queued_promises(&self, network: Freq) -> bool {
        self.networks.get(&network).is_some_and(|n| !n.promises.is_empty())
    }

    /// `GlobalLogisticsManager.tick`, run for the overworld only: every network's promises age.
    pub fn tick(&mut self) {
        for n in self.networks.values_mut() {
            n.promises.tick();
        }
    }

    /// `PackagerBlockEntity.submitNewArrivals`'s last part: the arrivals a packager saw are
    /// told to the promise queue of every network a Stock Link on it belongs to (each once,
    /// and only a network with promises).
    pub fn submit_arrivals(&mut self, links_on_packager: &[Freq], arrivals: &[Big<K>]) {
        let mut done: Vec<Freq> = Vec::new();
        for &freq in links_on_packager {
            if done.contains(&freq) || !self.has_queued_promises(freq) {
                continue;
            }
            done.push(freq);
            if let Some(queue) = self.queued_promises(freq) {
                for (key, count) in arrivals {
                    queue.item_entered_system(key, *count);
                }
            }
        }
    }
}

/// `PackagerBlockEntity.getAvailableItems`: the inventory's stacks added up; a bottomless
/// handler (a Creative Crate) holds [`INF`](super::package::INF) of what it makes.
pub fn available_items<K: PartialEq + Clone>(stacks: &[Big<K>], bottomless: Option<&K>) -> Summary<K> {
    let mut summary = Summary::default();
    if let Some(key) = bottomless {
        summary.add(key, i64::from(super::package::INF));
        return summary;
    }
    for (key, count) in stacks {
        summary.add(key, i64::from(*count));
    }
    summary
}

/// `PackagerBlockEntity.submitNewArrivals`: what grew between two readings of a packager's
/// items, in the order of the later reading. `None` before it was ever read, or an empty
/// reading, means none.
pub fn new_arrivals<K: PartialEq + Clone>(before: Option<&Summary<K>>, after: &Summary<K>) -> Vec<Big<K>> {
    let Some(before) = before else {
        return Vec::new();
    };
    if after.is_empty() {
        return Vec::new();
    }
    after
        .stacks
        .iter()
        .filter_map(|(key, count)| {
            let had = before.count_of(key);
            (*count > had).then(|| (key.clone(), *count - had))
        })
        .collect()
}

// --- the links ------------------------------------------------------------------------------------

/// The links present on each frequency (`LogisticallyLinkedBehaviour.LINKS`, or `CLIENT_LINKS`:
/// a world has one of each). A link is present for [`LINK_TIMEOUT`] ticks after it last kept
/// itself alive, and a frequency's group for [`LINK_GROUP_TIMEOUT`] ticks after it was last read.
#[derive(Clone, Debug)]
pub struct LinkRegistry {
    groups: TickCache<Freq, TickCache<LinkId, ()>>,
}

impl Default for LinkRegistry {
    fn default() -> Self {
        Self { groups: TickCache::new(LINK_GROUP_TIMEOUT, true) }
    }
}

impl LinkRegistry {
    /// `keepAlive`: a link powered by 15 isn't kept.
    pub fn keep_alive(&mut self, freq: Freq, link: LinkId, redstone_power: u8, now: i64) {
        if redstone_power == SILENCING_POWER {
            return;
        }
        let group = self.groups.get_or_insert_with(&freq, now, || TickCache::new(LINK_TIMEOUT, false));
        group.put(link, (), now);
    }

    /// `LogisticallyLinkedBehaviour.remove`.
    pub fn remove(&mut self, freq: Freq, link: LinkId, now: i64) {
        if let Some(group) = self.groups.get_if_present(&freq, now) {
            group.invalidate(&link);
        }
    }

    /// `getAllPresent` before the validity filter and the sort: the ids on the frequency, in
    /// the order of their creation. The caller keeps the valid ones (`isValidLink`) as
    /// [`LinkView`]s.
    pub fn all_present(&mut self, freq: Freq, now: i64) -> Vec<LinkId> {
        match self.groups.get_if_present(&freq, now) {
            Some(group) => group.keys(now),
            None => Vec::new(),
        }
    }
}

/// The state of one `LogisticallyLinkedBehaviour`.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub id: LinkId,
    pub freq: Freq,
    /// `redstonePower`: the priority a Stock Link sorts by, and 15 takes it off the network.
    pub redstone_power: u8,
    /// `global`: a Stock Link is, a Stock Ticker or Redstone Requester isn't.
    pub global: bool,
    pub added_globally: bool,
    pub loaded_globally: bool,
    /// `PackagerLinkBlockEntity.placedBy`.
    pub placed_by: Option<PlayerId>,
}

impl Link {
    /// `new LogisticallyLinkedBehaviour(be, global)`, with the id and the frequency the
    /// constructor would draw.
    pub fn new(id: LinkId, freq: Freq, global: bool) -> Self {
        Self { id, freq, redstone_power: 0, global, added_globally: false, loaded_globally: false, placed_by: None }
    }

    /// `initialize` (on the server): the link is loaded and added to the network's table.
    pub fn initialize<K: PartialEq + Clone>(&mut self, logistics: &mut Logistics<K>, pos: &GlobalPos, now: i64) {
        if !self.loaded_globally && self.global {
            self.loaded_globally = true;
            logistics.networks.link_loaded(self.freq, pos.clone());
            // "Call keepAlive regardless of redstone power."
            self.keep_alive(logistics, now);
        }
        if !self.added_globally && self.global {
            self.added_globally = true;
            logistics.networks.link_added(self.freq, pos.clone(), self.placed_by);
        }
        // A link that joins a network changes what it holds: a summary read just before (an `open` at a ticker that no link was on
        // yet) must not keep saying "0 links" for its 20 ticks. Create lets it expire; here the first registration drops it.
        logistics.invalidate_summaries(self.freq);
    }

    /// The link's saved frequency is no longer the one it was registered on (a `/data modify` or a copied `Freq`; Create has no way to
    /// change it but placing a new block): it leaves `old`'s network and joins its own, as `destroy` followed by `initialize` would.
    pub fn retune<K: PartialEq + Clone>(&mut self, logistics: &mut Logistics<K>, old: Freq, pos: &GlobalPos, now: i64) {
        if old == self.freq {
            return;
        }
        logistics.links.remove(old, self.id, now);
        if self.global {
            if self.loaded_globally {
                logistics.networks.link_invalidated(old, pos);
                logistics.networks.link_loaded(self.freq, pos.clone());
            }
            if self.added_globally {
                logistics.networks.link_removed(old, pos);
                logistics.networks.link_added(self.freq, pos.clone(), self.placed_by);
            }
        }
        self.keep_alive(logistics, now);
        logistics.invalidate_summaries(old);
        logistics.invalidate_summaries(self.freq);
    }

    /// `keepAlive`.
    pub fn keep_alive<K: PartialEq + Clone>(&self, logistics: &mut Logistics<K>, now: i64) {
        logistics.links.keep_alive(self.freq, self.id, self.redstone_power, now);
    }

    /// `lazyTick`, every [`LAZY_PERIOD`] ticks.
    pub fn lazy_tick<K: PartialEq + Clone>(&self, logistics: &mut Logistics<K>, now: i64) {
        self.keep_alive(logistics, now);
    }

    /// `unload`.
    pub fn unload<K: PartialEq + Clone>(&self, logistics: &mut Logistics<K>, pos: &GlobalPos, now: i64) {
        if self.loaded_globally && self.global {
            logistics.networks.link_invalidated(self.freq, pos);
        }
        logistics.links.remove(self.freq, self.id, now);
    }

    /// `destroy`.
    pub fn destroy<K: PartialEq + Clone>(&self, logistics: &mut Logistics<K>, pos: &GlobalPos) {
        if self.added_globally && self.global {
            logistics.networks.link_removed(self.freq, pos);
        }
    }

    /// `redstonePowerChanged`: returns whether it changed (the block entity is marked dirty).
    pub fn redstone_power_changed<K: PartialEq + Clone>(
        &mut self,
        logistics: &mut Logistics<K>,
        power: u8,
        now: i64,
    ) -> bool {
        if power == self.redstone_power {
            return false;
        }
        self.redstone_power = power;
        if power == SILENCING_POWER {
            logistics.links.remove(self.freq, self.id, now);
        } else {
            self.keep_alive(logistics, now);
        }
        true
    }

    /// `mayInteract`.
    pub fn may_interact<K: PartialEq + Clone>(&self, logistics: &Logistics<K>, player: PlayerId) -> bool {
        logistics.networks.may_interact(self.freq, player)
    }

    /// `mayAdministrate`.
    pub fn may_administrate<K: PartialEq + Clone>(&self, logistics: &Logistics<K>, player: PlayerId) -> bool {
        logistics.networks.may_administrate(self.freq, player)
    }
}

/// `PackagerLinkBlock.getPower`: the strongest signal reaching the link from any side but the
/// one it is attached to. `signals[i]` is `level.getSignal(pos.relative(d), d)` for
/// `Dir::ALL[i]`.
pub fn link_power(connected: Dir, signals: &[u8; 6]) -> u8 {
    let mut power = 0;
    for (d, signal) in Dir::ALL.iter().zip(signals) {
        if d.opposite() != connected {
            power = power.max(*signal);
        }
    }
    power
}

/// `PackagerLinkBlockEntity.getPackager`: the packager is on the side the link is attached to.
pub fn packager_pos(link: Pos, connected: Dir) -> Pos {
    let n = connected.opposite().normal();
    [link[0] + n[0], link[1] + n[1], link[2] + n[2]]
}

/// The packager behind a Stock Link, as the network rules read it
/// (`PackagerLinkBlockEntity.getPackager`: none when the link has power 15, no packager is
/// there, or it is a Re-Packager).
#[derive(Clone, Debug, PartialEq)]
pub struct PackagerView<K> {
    pub id: PackagerId,
    /// `InventoryIdentifier` of the inventory it targets, when it has one and it is identified.
    pub inventory: Option<InventoryId>,
    /// `isTargetingSameInventory(ignoredHandler)` for the call being made: it targets the
    /// inventory the order must not be taken from.
    pub targets_ignored: bool,
    /// `queuedExitingPackages.size()`.
    pub queued: usize,
    /// `getAvailableItems()`.
    pub available: Summary<K>,
}

/// A link present on a frequency (a Stock Link, or the Stock Ticker or Redstone Requester that
/// keeps itself alive there, whose `packager` is none).
#[derive(Clone, Debug, PartialEq)]
pub struct LinkView<K> {
    pub id: LinkId,
    /// `redstonePower`.
    pub power: u8,
    pub packager: Option<PackagerView<K>>,
}

// --- the summary ----------------------------------------------------------------------------------

/// `LogisticsManager.createSummaryOfNetwork`: every link's packager's items added up; links on
/// an inventory another link already showed are skipped; each link with a packager counts in
/// `contributing_links`. `links` are the links in the order of [`LinkRegistry::all_present`].
pub fn create_summary_of_network<K: PartialEq + Clone>(links: &[LinkView<K>]) -> Summary<K> {
    let mut total = Summary::default();
    let mut processed: Vec<InventoryId> = Vec::new();
    for link in links {
        if let Some(inventory) = link.packager.as_ref().and_then(|p| p.inventory) {
            if processed.contains(&inventory) {
                continue;
            }
            processed.push(inventory);
        }
        // `fetchSummaryFromPackager(null)`: no packager is `InventorySummary.EMPTY`, which isn't
        // counted; a packager with nothing in reach is a summary, and counts.
        let Some(packager) = &link.packager else {
            continue;
        };
        total.contributing_links += 1;
        total.add_summary(&packager.available);
    }
    total
}

/// `LogisticsManager.getStockOf`: what the links' packagers hold of `key`, not counting those
/// that target the ignored inventory (and not skipping links that share one inventory).
pub fn stock_of<K: PartialEq + Clone>(links: &[LinkView<K>], key: &K) -> u32 {
    let mut sum: u32 = 0;
    for link in links {
        let Some(packager) = &link.packager else {
            continue;
        };
        if packager.targets_ignored {
            continue;
        }
        sum = sum.saturating_add(packager.available.count_of(key));
    }
    sum
}

/// The world's logistics state: `Create.LOGISTICS`, `LogisticallyLinkedBehaviour.LINKS`,
/// `LogisticsManager.SUMMARIES` and `ACCURATE_SUMMARIES`.
#[derive(Clone, Debug)]
pub struct Logistics<K> {
    pub networks: Networks<K>,
    pub links: LinkRegistry,
    summaries: TickCache<Freq, Summary<K>>,
    accurate: TickCache<Freq, Summary<K>>,
}

impl<K: PartialEq + Clone> Default for Logistics<K> {
    fn default() -> Self {
        Self {
            networks: Networks::default(),
            links: LinkRegistry::default(),
            summaries: TickCache::new(SUMMARY_TIMEOUT, false),
            accurate: TickCache::new(ACCURATE_SUMMARY_TIMEOUT, false),
        }
    }
}

impl<K: PartialEq + Clone> Logistics<K> {
    /// `LogisticsManager.getSummaryOfNetwork`: the cached summary (20 ticks, or 1 for the
    /// accurate one), else one made from the links, which `views` builds from the ids present.
    pub fn summary_of_network(
        &mut self,
        freq: Freq,
        accurate: bool,
        now: i64,
        views: impl FnOnce(&[LinkId]) -> Vec<LinkView<K>>,
    ) -> Summary<K> {
        let cache = if accurate { &mut self.accurate } else { &mut self.summaries };
        if let Some(summary) = cache.get_if_present(&freq, now) {
            return summary.clone();
        }
        let ids = self.links.all_present(freq, now);
        let summary = create_summary_of_network(&views(&ids));
        let cache = if accurate { &mut self.accurate } else { &mut self.summaries };
        cache.put(freq, summary.clone(), now);
        summary
    }

    /// Both caches of a network are dropped (a link joined or left it).
    pub fn invalidate_summaries(&mut self, freq: Freq) {
        self.summaries.invalidate(&freq);
        self.accurate.invalidate(&freq);
    }

    /// `SUMMARIES.invalidate(network)`: a gauge whose links all loaded wants a fresh summary.
    pub fn invalidate_summary(&mut self, freq: Freq) {
        self.summaries.invalidate(&freq);
    }

    /// `LogisticallyLinkedBehaviour.deductFromAccurateSummary`: a packager made a box from its
    /// inventory, so the accurate summary of this tick holds less.
    pub fn deduct_from_accurate_summary(&mut self, freq: Freq, contents: &[Big<K>], now: i64) {
        let Some(summary) = self.accurate.get_if_present(&freq, now) else {
            return;
        };
        for (key, count) in contents {
            let deduct = summary.count_of(key).min(*count);
            summary.add(key, -i64::from(deduct));
        }
    }
}

// --- splitting an order ---------------------------------------------------------------------------

/// The numbers `LogisticsManager` draws from `java.util.Random`.
pub trait Random {
    /// `nextInt()`.
    fn next_int(&mut self) -> i32;
    /// `nextInt(bound)`.
    fn next_below(&mut self, bound: usize) -> usize;
}

/// One request made of one packager (the `Multimap<PackagerBlockEntity, PackagingRequest>`'s
/// entries, in the order they were made).
#[derive(Clone, Debug, PartialEq)]
pub struct Planned<K> {
    pub packager: PackagerId,
    pub request: Request<K>,
}

/// `LogisticsManager.findPackagersForRequest`: asks the links, cheapest priority first (the
/// lower the redstone power, the earlier), for each stack of the order in turn, until what was
/// ordered is found or the links run out; the requests are the order split across packagers.
///
/// Links on one inventory are one choice: a random one of them is used. The first request
/// carries the order itself as its context. Each box of a link knows its place in the order
/// (`link_index`), and the last box of the last link is final (`final_link`), but only when the
/// whole order was found. The caller gives the order id and the numbers it needs by `rng`
/// (the choices between links first, then the order id).
pub fn find_packagers_for_request<K: PartialEq + Clone>(
    links: &[LinkView<K>],
    order: &Context<K>,
    address: &str,
    rng: &mut dyn Random,
) -> Vec<Planned<K>> {
    let stacks: Vec<&Big<K>> = order.stacks.iter().filter(|(_, count)| *count > 0).collect();

    // `getAllPresent(freqId, true)`: sorted by priority (a stable sort).
    let mut sorted: Vec<&LinkView<K>> = links.iter().collect();
    sorted.sort_by_key(|link| link.power);

    // Group links by inventory, and pick one of each group at random.
    let mut by_inventory: Vec<(InventoryId, Vec<&LinkView<K>>)> = Vec::new();
    let mut available: Vec<&LinkView<K>> = Vec::new();
    for link in sorted {
        match link.packager.as_ref().and_then(|p| p.inventory) {
            Some(inventory) => match by_inventory.iter_mut().find(|(id, _)| *id == inventory) {
                Some((_, group)) => group.push(link),
                None => by_inventory.push((inventory, vec![link])),
            },
            None => available.push(link),
        }
    }
    for (_, group) in &by_inventory {
        available.push(group[rng.next_below(group.len())]);
    }

    // `finalLinkTracker` and every `isFinalLink`: flags shared by the requests that hold them.
    let mut flags: Vec<bool> = vec![false];
    let mut tracker = 0usize;
    let mut used_links: Vec<LinkId> = Vec::new();
    let mut context: Option<Context<K>> = Some(order.clone());
    let order_id = rng.next_int();

    let mut planned: Vec<(PackagerId, Request<K>, usize)> = Vec::new();
    for (i, (item, count)) in stacks.iter().enumerate() {
        let mut remaining = i64::from(*count);
        let final_entry = i == stacks.len() - 1;

        for link in &available {
            let used_index = used_links.iter().position(|id| *id == link.id);
            let link_index = used_index.unwrap_or(used_links.len());
            let flag = if link_index + 1 == used_links.len() {
                tracker
            } else {
                flags.push(false);
                flags.len() - 1
            };

            // `PackagerLinkBlockEntity.processRequest`.
            let Some(packager) = &link.packager else {
                continue;
            };
            if packager.targets_ignored {
                continue;
            }
            let in_stock = packager.available.count_of(item);
            if in_stock == 0 {
                continue;
            }
            let to_withdraw = (remaining.min(i64::from(in_stock))) as u32;
            planned.push((
                packager.id,
                Request {
                    item: item.clone(),
                    count: to_withdraw,
                    address: address.to_owned(),
                    link_index: link_index as i32,
                    final_link: false,
                    package_counter: 0,
                    order_id,
                    context: context.clone(),
                },
                flag,
            ));

            if to_withdraw > 0 && used_index.is_none() {
                context = None;
                used_links.push(link.id);
                tracker = flag;
            }

            remaining -= i64::from(to_withdraw);
            if remaining > 0 {
                continue;
            }
            if final_entry {
                flags[tracker] = true;
            }
            break;
        }
    }

    planned
        .into_iter()
        .map(|(packager, mut request, flag)| {
            request.final_link = flags[flag];
            Planned { packager, request }
        })
        .collect()
}

/// What one packager is asked to make (`performPackageRequests`'s per-packager lists).
#[derive(Clone, Debug, PartialEq)]
pub struct Dispatch<K> {
    pub packager: PackagerId,
    pub requests: Vec<Request<K>>,
}

/// Groups a plan by packager, in the order each first appears (`requests.asMap()`).
pub fn dispatches<K: Clone>(planned: &[Planned<K>]) -> Vec<Dispatch<K>> {
    let mut out: Vec<Dispatch<K>> = Vec::new();
    for entry in planned {
        match out.iter_mut().find(|d| d.packager == entry.packager) {
            Some(d) => d.requests.push(entry.request.clone()),
            None => out.push(Dispatch { packager: entry.packager, requests: vec![entry.request.clone()] }),
        }
    }
    out
}

/// What `LogisticsManager.broadcastPackageRequest` decided.
#[derive(Clone, Debug, PartialEq)]
pub struct Broadcast<K> {
    /// Its return value: an empty order, or a packager with too many boxes queued, is false.
    pub accepted: bool,
    /// Each packager and its requests; the caller flashes its link, runs `attemptToSend` on
    /// the list up to [`PERFORM_ATTEMPTS`] times ([`run_attempts`]), then triggers its stock
    /// check and notifies its update. Empty when not accepted.
    pub dispatches: Vec<Dispatch<K>>,
}

/// `LogisticsManager.broadcastPackageRequest`.
pub fn broadcast_package_request<K: PartialEq + Clone>(
    links: &[LinkView<K>],
    kind: RequestType,
    order: &Context<K>,
    address: &str,
    rng: &mut dyn Random,
) -> Broadcast<K> {
    let rejected = Broadcast { accepted: false, dispatches: Vec::new() };
    if order.is_empty() {
        return rejected;
    }
    let planned = find_packagers_for_request(links, order, address, rng);
    // "Check if packagers have accumulated too many packages already".
    for entry in &planned {
        let queued = links
            .iter()
            .filter_map(|l| l.packager.as_ref())
            .find(|p| p.id == entry.packager)
            .map_or(0, |p| p.queued);
        if too_busy_for(queued, kind) {
            return rejected;
        }
    }
    Broadcast { accepted: true, dispatches: dispatches(&planned) }
}

/// Whether any packager of a plan has too many boxes queued for `kind` (the gauge checks this
/// across several networks' plans before sending any).
pub fn any_too_busy<K>(planned: &[Planned<K>], packagers: &[(PackagerId, usize)], kind: RequestType) -> bool {
    planned.iter().any(|entry| {
        packagers.iter().find(|(id, _)| *id == entry.packager).is_some_and(|(_, queued)| too_busy_for(*queued, kind))
    })
}

/// `performPackageRequests`'s loop for one packager: `attempt` is
/// `PackagerBlockEntity.attemptToSend(queuedRequests)`, which takes the finished and dropped
/// requests off the list; it runs until the list is empty, at most [`PERFORM_ATTEMPTS`] times.
pub fn run_attempts<K>(requests: &mut Vec<Request<K>>, mut attempt: impl FnMut(&mut Vec<Request<K>>)) {
    for _ in 0..PERFORM_ATTEMPTS {
        if requests.is_empty() {
            break;
        }
        attempt(requests);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::package::{self, Slot};

    /// A fixed sequence for the random numbers: `below` picks the first of any group.
    struct Fixed(i32);
    impl Random for Fixed {
        fn next_int(&mut self) -> i32 {
            self.0
        }
        fn next_below(&mut self, _bound: usize) -> usize {
            0
        }
    }

    fn summary(stacks: &[(&'static str, u32)]) -> Summary<&'static str> {
        let mut s = Summary::default();
        for (k, c) in stacks {
            s.add(k, i64::from(*c));
        }
        s
    }

    fn packager(id: PackagerId, inventory: Option<InventoryId>, stacks: &[(&'static str, u32)]) -> PackagerView<&'static str> {
        PackagerView { id, inventory, targets_ignored: false, queued: 0, available: summary(stacks) }
    }

    fn link(id: LinkId, power: u8, packager: Option<PackagerView<&'static str>>) -> LinkView<&'static str> {
        LinkView { id, power, packager }
    }

    fn order(stacks: &[(&'static str, u32)]) -> Context<&'static str> {
        Context { stacks: stacks.to_vec(), crafts: Vec::new() }
    }

    fn pos(freq: Freq, x: i32) -> (Freq, GlobalPos) {
        (freq, GlobalPos { dimension: "minecraft:overworld".to_owned(), pos: [x, 64, 0] })
    }

    fn here(x: i32) -> GlobalPos {
        GlobalPos { dimension: "minecraft:overworld".to_owned(), pos: [x, 64, 0] }
    }

    /// The summary of `freq` with link 1 on a packager holding iron and any other link (a ticker) on none.
    fn read(logistics: &mut Logistics<&'static str>, freq: Freq, now: i64) -> Summary<&'static str> {
        logistics.summary_of_network(freq, false, now, |ids| {
            ids.iter().map(|id| link(*id, 0, (*id == 1).then(|| packager(10, Some(100), &[("iron", 12)])))).collect()
        })
    }

    #[test]
    fn a_link_and_a_ticker_that_share_a_frequency_see_each_other_at_once() {
        // The registration path of a placed (or setblock-placed) Stock Link and Stock Ticker: the first tick of each.
        let mut logistics: Logistics<&'static str> = Logistics::default();
        let (mut link1, mut ticker) = (Link::new(1, 7, true), Link::new(2, 7, false));
        link1.initialize(&mut logistics, &here(0), 100);
        link1.keep_alive(&mut logistics, 100);
        ticker.initialize(&mut logistics, &here(3), 100);
        ticker.keep_alive(&mut logistics, 100);
        let summary = read(&mut logistics, 7, 101);
        assert_eq!(summary.contributing_links, 1);
        assert_eq!(summary.count_of(&"iron"), 12);
    }

    #[test]
    fn a_summary_read_before_the_link_registered_is_not_kept() {
        let mut logistics: Logistics<&'static str> = Logistics::default();
        assert_eq!(read(&mut logistics, 7, 100).contributing_links, 0);
        let mut link1 = Link::new(1, 7, true);
        link1.initialize(&mut logistics, &here(0), 101);
        assert_eq!(read(&mut logistics, 7, 102).contributing_links, 1);
    }

    #[test]
    fn a_link_whose_frequency_was_edited_moves_to_the_new_network() {
        let mut logistics: Logistics<&'static str> = Logistics::default();
        let mut link1 = Link::new(1, 7, true);
        link1.initialize(&mut logistics, &here(0), 100);
        assert_eq!(read(&mut logistics, 7, 101).contributing_links, 1);
        // The tester copies the ticker's frequency (9) onto the link.
        link1.freq = 9;
        link1.retune(&mut logistics, 7, &here(0), 102);
        assert_eq!(read(&mut logistics, 9, 102).contributing_links, 1);
        assert_eq!(read(&mut logistics, 7, 102).contributing_links, 0);
        assert!(logistics.networks.networks.contains_key(&9));
        assert!(!logistics.networks.networks.contains_key(&7));
    }

    #[test]
    fn a_network_summary_adds_two_links() {
        // Two links on separate chests: 40 + 24 iron, 8 gold. The ticker beside them keeps
        // itself alive on the frequency, has no packager, and doesn't contribute.
        let links = vec![
            link(1, 0, Some(packager(10, Some(100), &[("iron", 40), ("gold", 8)]))),
            link(2, 0, Some(packager(11, Some(101), &[("iron", 24)]))),
            link(3, 0, None),
        ];
        let summary = create_summary_of_network(&links);
        assert_eq!(summary.count_of(&"iron"), 64);
        assert_eq!(summary.count_of(&"gold"), 8);
        assert_eq!(summary.contributing_links, 2);
        assert_eq!(summary.total_count, 72);
        // A link with a packager that targets no inventory still counts as contributing.
        let links = vec![link(1, 0, Some(packager(10, None, &[])))];
        assert_eq!(create_summary_of_network(&links).contributing_links, 1);
    }

    #[test]
    fn links_on_one_inventory_show_it_once() {
        // Two links on the same vault (inventory 7): its 100 iron counts once in the summary,
        // but `getStockOf` (which doesn't skip them) counts it twice.
        let links = vec![
            link(1, 0, Some(packager(10, Some(7), &[("iron", 100)]))),
            link(2, 0, Some(packager(11, Some(7), &[("iron", 100)]))),
        ];
        let summary = create_summary_of_network(&links);
        assert_eq!(summary.count_of(&"iron"), 100);
        assert_eq!(summary.contributing_links, 1);
        assert_eq!(stock_of(&links, &"iron"), 200);
        // An ignored inventory is left out of the stock.
        let mut links = links;
        links[1].packager.as_mut().unwrap().targets_ignored = true;
        assert_eq!(stock_of(&links, &"iron"), 100);
    }

    #[test]
    fn the_summary_is_cached_for_twenty_ticks_and_the_accurate_one_for_one() {
        let mut logistics: Logistics<&'static str> = Logistics::default();
        let freq = 42;
        let (_, gpos) = pos(freq, 0);
        let mut stock_link = Link::new(1, freq, true);
        stock_link.initialize(&mut logistics, &gpos, 100);
        let mut stock = 5u32;
        let read = |logistics: &mut Logistics<&'static str>, accurate: bool, now: i64, stock: u32| {
            logistics
                .summary_of_network(freq, accurate, now, |ids| {
                    assert_eq!(ids, &[1]);
                    vec![link(1, 0, Some(packager(10, Some(100), &[("iron", stock)])))]
                })
                .count_of(&"iron")
        };
        assert_eq!(read(&mut logistics, false, 100, stock), 5);
        stock = 9;
        // The link keeps itself alive every 10 ticks, which keeps its frequency's group in the cache.
        for t in [110, 120, 130] {
            stock_link.lazy_tick(&mut logistics, t);
        }
        // 20 ticks on, the cached one is still served (`timestamp < now - 20` is false) ...
        assert_eq!(read(&mut logistics, false, 120, stock), 5);
        // ... and a tick later it is made again.
        assert_eq!(read(&mut logistics, false, 121, stock), 9);
        // The accurate one lasts one tick, and is deducted from as packages are made.
        assert_eq!(read(&mut logistics, true, 121, stock), 9);
        logistics.deduct_from_accurate_summary(freq, &[("iron", 4), ("gold", 3)], 121);
        assert_eq!(read(&mut logistics, true, 121, 99), 5);
        assert_eq!(read(&mut logistics, true, 122, 99), 5);
        assert_eq!(read(&mut logistics, true, 123, 99), 99);
    }

    #[test]
    fn a_link_lives_four_hundred_ticks_and_power_fifteen_silences_it() {
        let mut logistics: Logistics<&'static str> = Logistics::default();
        let freq = 1;
        let (_, gpos) = pos(freq, 0);
        let mut a = Link::new(1, freq, true);
        let b = Link::new(2, freq, false);
        a.initialize(&mut logistics, &gpos, 0);
        b.keep_alive(&mut logistics, 0);
        assert_eq!(logistics.links.all_present(freq, 1), vec![1, 2]);
        // `a` keeps itself alive every 10 ticks; `b` stops.
        for t in (10..=400).step_by(10) {
            a.lazy_tick(&mut logistics, t);
        }
        assert_eq!(logistics.links.all_present(freq, 401), vec![1], "b timed out after 400 ticks");
        // Power 15 takes `a` off at once; changing it back brings it back.
        assert!(a.redstone_power_changed(&mut logistics, 15, 402));
        assert!(!a.redstone_power_changed(&mut logistics, 15, 402));
        assert!(logistics.links.all_present(freq, 402).is_empty());
        a.lazy_tick(&mut logistics, 410);
        assert!(logistics.links.all_present(freq, 410).is_empty(), "a powered link doesn't keep itself alive");
        assert!(a.redstone_power_changed(&mut logistics, 3, 411));
        assert_eq!(logistics.links.all_present(freq, 411), vec![1]);
    }

    #[test]
    fn networks_are_made_by_links_and_go_with_the_last() {
        let mut logistics: Logistics<&'static str> = Logistics::default();
        let freq = 5;
        let owner = 77;
        let (_, p1) = pos(freq, 1);
        let (_, p2) = pos(freq, 2);
        let mut a = Link::new(1, freq, true);
        a.placed_by = Some(owner);
        let mut b = Link::new(2, freq, true);
        b.placed_by = Some(88);
        a.initialize(&mut logistics, &p1, 0);
        b.initialize(&mut logistics, &p2, 0);
        assert!(logistics.networks.is_lockable(freq));
        // The first owner stays.
        assert_eq!(logistics.networks.networks[&freq].owner, Some(owner));
        assert_eq!(logistics.networks.unloaded_link_count(freq), 0);
        b.unload(&mut logistics, &p2, 1);
        assert_eq!(logistics.networks.unloaded_link_count(freq), 1);
        // Locked: only the owner may interact; anyone may before.
        assert!(logistics.networks.may_interact(freq, 88));
        logistics.networks.set_locked(freq, 88, true);
        assert!(!logistics.networks.is_locked(freq), "a stranger can't lock it");
        logistics.networks.set_locked(freq, owner, true);
        assert!(logistics.networks.is_locked(freq));
        assert!(!logistics.networks.may_interact(freq, 88));
        assert!(logistics.networks.may_interact(freq, owner));
        assert!(!logistics.networks.may_administrate(freq, 88));
        // An unknown network is open.
        assert!(logistics.networks.may_interact(999, 88));
        a.destroy(&mut logistics, &p1);
        b.destroy(&mut logistics, &p2);
        assert!(!logistics.networks.is_lockable(freq));
    }

    #[test]
    fn link_power_ignores_the_side_it_is_attached_to() {
        // Attached to a packager to its north, `connected` = South (the direction it faces away
        // from it): the signal from the north is skipped.
        let mut signals = [0u8; 6];
        signals[Dir::North as usize] = 9;
        assert_eq!(link_power(Dir::South, &signals), 0);
        signals[Dir::East as usize] = 4;
        assert_eq!(link_power(Dir::South, &signals), 4);
        assert_eq!(packager_pos([5, 5, 5], Dir::South), [5, 5, 4]);
    }

    #[test]
    fn an_order_is_split_across_two_packagers_by_what_each_holds() {
        // Order: 100 iron and 5 gold. Chest A holds 60 iron, chest B 70 iron and 5 gold. Links in
        // priority order (power 0 before 1): A first. 60 from A, the remaining 40 from B, then
        // the gold from B (A has none).
        let links = vec![
            link(1, 0, Some(packager(10, Some(100), &[("iron", 60)]))),
            link(2, 1, Some(packager(11, Some(101), &[("iron", 70), ("gold", 5)]))),
        ];
        let order = order(&[("iron", 100), ("gold", 5)]);
        let planned = find_packagers_for_request(&links, &order, "Base", &mut Fixed(1234));
        let summary: Vec<_> = planned.iter().map(|p| (p.packager, p.request.item, p.request.count, p.request.link_index)).collect();
        assert_eq!(summary, vec![(10, "iron", 60, 0), (11, "iron", 40, 1), (11, "gold", 5, 1)]);
        assert!(planned.iter().all(|p| p.request.order_id == 1234 && p.request.address == "Base"));
        // Only the first request carries the order's context, the last link's boxes are final.
        assert_eq!(planned[0].request.context.as_ref(), Some(&order));
        assert!(planned[1..].iter().all(|p| p.request.context.is_none()));
        assert_eq!(
            planned.iter().map(|p| p.request.final_link).collect::<Vec<_>>(),
            vec![false, true, true]
        );

        // Grouped by packager: A gets one request, B two.
        let sent = dispatches(&planned);
        assert_eq!(sent.len(), 2);
        assert_eq!((sent[0].packager, sent[0].requests.len()), (10, 1));
        assert_eq!((sent[1].packager, sent[1].requests.len()), (11, 2));

        // Run them against the chests' contents as the packagers would (`pack` with the requests
        // as `attemptToSend` does): A packs 60 iron, B packs 40 iron and the gold in one box.
        let chest = |items: &[(&'static str, u32)]| -> Vec<Option<Slot<&'static str>>> {
            items.iter().map(|&(key, count)| Some(Slot { key, count, max: 64, bulky: false, package: false })).collect()
        };
        let mut a_requests = sent[0].requests.clone();
        let a_box = package::pack(&chest(&[("iron", 60)]), Some(&mut a_requests)).unwrap();
        assert_eq!(a_box.taken, vec![(0, 60)]);
        assert!(a_requests.is_empty());
        let mut b_requests = sent[1].requests.clone();
        let mut attempts = 0;
        let mut boxes = Vec::new();
        run_attempts(&mut b_requests, |queue| {
            attempts += 1;
            boxes.push(package::pack(&chest(&[("iron", 70), ("gold", 5)]), Some(queue)).unwrap());
        });
        assert_eq!(attempts, 1, "one box holds both requests of the order");
        assert_eq!(boxes[0].taken, vec![(0, 40), (1, 5)]);
        assert!(boxes[0].order.as_ref().unwrap().is_final_link);
    }

    #[test]
    fn a_short_order_has_no_final_link_and_ignored_inventories_are_skipped() {
        let links = vec![link(1, 0, Some(packager(10, Some(100), &[("iron", 3)])))];
        let planned = find_packagers_for_request(&links, &order(&[("iron", 10)]), "", &mut Fixed(1));
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].request.count, 3);
        assert!(!planned[0].request.final_link, "the order isn't found whole: nothing is final");
        // Packagers that target the ignored inventory are asked for nothing.
        let mut ignored = links.clone();
        ignored[0].packager.as_mut().unwrap().targets_ignored = true;
        assert!(find_packagers_for_request(&ignored, &order(&[("iron", 10)]), "", &mut Fixed(1)).is_empty());
        // A count of zero is not asked for.
        assert!(find_packagers_for_request(&links, &order(&[("iron", 0)]), "", &mut Fixed(1)).is_empty());
    }

    #[test]
    fn one_link_per_inventory_is_chosen_at_random() {
        // Links 1 and 2 sit on the same chest; link 3 on its own with a lower priority number.
        // Unidentified inventories come first, then one link of each group.
        struct Pick(usize);
        impl Random for Pick {
            fn next_int(&mut self) -> i32 {
                0
            }
            fn next_below(&mut self, bound: usize) -> usize {
                self.0.min(bound - 1)
            }
        }
        let links = vec![
            link(1, 0, Some(packager(10, Some(7), &[("iron", 5)]))),
            link(2, 0, Some(packager(11, Some(7), &[("iron", 5)]))),
            link(3, 5, Some(packager(12, None, &[("iron", 5)]))),
        ];
        let planned = find_packagers_for_request(&links, &order(&[("iron", 8)]), "", &mut Pick(1));
        // Unidentified link 3 first (5), then the chosen member of the group (the second: 11).
        let packagers: Vec<_> = planned.iter().map(|p| (p.packager, p.request.count)).collect();
        assert_eq!(packagers, vec![(12, 5), (11, 3)]);
    }

    #[test]
    fn a_broadcast_is_refused_for_an_empty_order_or_a_busy_packager() {
        let mut busy = packager(10, Some(100), &[("iron", 50)]);
        let links = vec![link(1, 0, Some(busy.clone()))];
        let ok = broadcast_package_request(&links, RequestType::Player, &order(&[("iron", 5)]), "A", &mut Fixed(1));
        assert!(ok.accepted);
        assert_eq!(ok.dispatches.len(), 1);
        assert!(!broadcast_package_request(&links, RequestType::Player, &order(&[]), "A", &mut Fixed(1)).accepted);
        // Create's limits: 50 boxes for a player, 20 for redstone, 10 for a restocker.
        busy.queued = 20;
        let links = vec![link(1, 0, Some(busy))];
        assert!(broadcast_package_request(&links, RequestType::Player, &order(&[("iron", 5)]), "A", &mut Fixed(1)).accepted);
        let redstone = broadcast_package_request(&links, RequestType::Redstone, &order(&[("iron", 5)]), "A", &mut Fixed(1));
        assert!(!redstone.accepted);
        assert!(redstone.dispatches.is_empty());
        assert!(any_too_busy(
            &find_packagers_for_request(&links, &order(&[("iron", 5)]), "A", &mut Fixed(1)),
            &[(10, 20)],
            RequestType::Restock
        ));
    }

    #[test]
    fn arrivals_are_what_grew_between_two_readings() {
        let before = summary(&[("iron", 10), ("gold", 4)]);
        let after = summary(&[("iron", 18), ("gold", 2), ("coal", 3)]);
        assert_eq!(new_arrivals(Some(&before), &after), vec![("iron", 8), ("coal", 3)]);
        assert!(new_arrivals(None, &after).is_empty());
        assert!(new_arrivals(Some(&before), &Summary::default()).is_empty());
        // They are told to the promise queue of each network with promises, once.
        let mut networks: Networks<&'static str> = Networks::default();
        networks.link_added(1, pos(1, 0).1, None);
        networks.link_added(2, pos(2, 0).1, None);
        networks.queued_promises(1).unwrap().add(crate::rules::request_promises::Promise::new(("iron", 20)));
        networks.submit_arrivals(&[1, 1, 2], &new_arrivals(Some(&before), &after));
        let left = networks.queued_promises(1).unwrap().flatten(false);
        assert_eq!(left[0].stack, ("iron", 12), "told once, not twice");
    }

    #[test]
    fn available_items_adds_the_slots_and_a_bottomless_handler_is_infinite() {
        let stacks = [("iron", 64), ("iron", 12), ("gold", 3)];
        let items = available_items(&stacks, None);
        assert_eq!((items.count_of(&"iron"), items.count_of(&"gold")), (76, 3));
        let crate_items = available_items(&[], Some(&"cobble"));
        assert_eq!(crate_items.count_of(&"cobble"), package::INF);
        // An infinite stack doesn't grow when more is added to it.
        let mut s = crate_items;
        s.add(&"cobble", 5);
        assert_eq!(s.count_of(&"cobble"), package::INF);
    }

    #[test]
    fn a_tick_cache_resets_on_access_only_when_told_to() {
        let mut keeps: TickCache<u32, &str> = TickCache::new(10, true);
        let mut forgets: TickCache<u32, &str> = TickCache::new(10, false);
        keeps.put(1, "a", 0);
        forgets.put(1, "a", 0);
        assert!(keeps.get_if_present(&1, 10).is_some());
        assert!(forgets.get_if_present(&1, 10).is_some());
        // At 20: the first was read at 10 and lives until 20; the second was written at 0.
        assert!(keeps.get_if_present(&1, 20).is_some());
        assert!(forgets.get_if_present(&1, 20).is_none());
    }
}
