//! The logistics network at run time (`LogisticallyLinkedBehaviour`, `GlobalLogisticsManager`, `LogisticsManager`,
//! `PackagerLinkBlockEntity.getPackager`): what the Stock Link, the Stock Ticker and the Redstone Requester share. Every block
//! that sits on a network has a frequency (`Freq`, a UUID), a signal strength (`Power`) and the flag `Added`; the table of
//! networks, the links present on each frequency and the cached summaries are `rules::logistics_network`, kept here in one
//! process-wide table (as Create keeps `Create.LOGISTICS` and `LogisticallyLinkedBehaviour.LINKS`); how an order is split across
//! the packagers is `rules::logistics_network::broadcast_package_request`. This is the glue: it reads the blocks, builds the
//! rules' views (a link, the packager behind it and what its inventory holds) and hands each packager its requests
//! (`packager::attempt_requests`).
//!
//! Known differences from Create, for the README:
//! - A frequency is carried to a new block by the player, not by the item: using a Stock Link, Stock Ticker, Redstone Requester
//!   or Factory Gauge item on a linked block remembers its network for the next such block that player places (Create writes
//!   the frequency into the stack's block entity data, which a mod can't write here). Without it, a placed block starts a
//!   new network. The remembered frequency is kept until it is used or the game quits.
//! - The network table is not saved (`LogisticsNetworkSavedData`): it is rebuilt from the loaded links, which add themselves
//!   (`Added` is saved but `link_added` runs again after a restart, so an owner is kept only for links placed in the same run).
//! - Only links in the dimension the order is made in are asked (Create's `GlobalPos` keeps all).
//! - A Creative Crate behind a packager is read as the stacks it holds, not as bottomless.
//! - No Wi-Fi particles; the link's sound plays at the packager.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{LazyLock, Mutex, MutexGuard};

use create_kinetics::rules::logistics_network::{
    self as net, Freq, GlobalPos, Link, LinkId, LinkView, Logistics, PackagerId, PackagerView, PlayerId, Random,
};
use create_kinetics::rules::package::{Context, Summary};
use create_kinetics::rules::packager::RequestType;
use create_kinetics::rules::stock_link::{self as link_rules, AttachFace};
use create_kinetics::{Dir, Pos};
use crate::compat::{Compound, Flow, Hooks, Host, ItemStack, Tag, Text, UseItemOnEvent};

use super::chute_inv as inv;
use super::package as items;
use super::packager as packager_glue;
use super::packager_inv as target_inv;
use crate::{int, util};

pub const STOCK_LINK: &str = "create:stock_link";
pub const STOCK_TICKER: &str = "create:stock_ticker";
pub const REDSTONE_REQUESTER: &str = "create:redstone_requester";
pub const FACTORY_GAUGE: &str = "create:factory_gauge";

/// The keys `LogisticallyLinkedBehaviour.write` and `PackagerLinkBlockEntity.write` save.
pub const FREQ: &str = "Freq";
pub const POWER: &str = "Power";
pub const ADDED: &str = "Added";
pub const PLACED_BY: &str = "PlacedBy";

/// A block of the network: a Stock Link (global), or a block that only reads it.
pub fn is_linked_block(block: &str) -> bool {
    matches!(block, STOCK_LINK | STOCK_TICKER | REDSTONE_REQUESTER | FACTORY_GAUGE)
}

/// What `LogisticallyLinkedBlockItem` is: the items that carry a frequency.
pub fn is_linked_item(item: &str) -> bool {
    is_linked_block(item)
}

struct Shared;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

// --- the table ----------------------------------------------------------------------------------------

#[derive(Default)]
struct Runtime {
    logistics: Logistics<ItemStack>,
    /// `LINK_ID_GENERATOR`, by block.
    ids: BTreeMap<(String, Pos), LinkId>,
    positions: BTreeMap<LinkId, (String, Pos)>,
    next_id: LinkId,
    /// Links `initialize` has run for (`loadedGlobally` and its like, which aren't saved).
    seen: BTreeSet<LinkId>,
    /// `LogisticallyLinkedBlockItem.assignFrequency`, kept by player until they place a block.
    tuned: BTreeMap<u128, Freq>,
    /// A block just placed, until its block entity ticks: the frequency and owner it starts with.
    placed: BTreeMap<(String, Pos), (Option<Freq>, u128)>,
}

impl Runtime {
    fn id_for(&mut self, dimension: &str, pos: Pos) -> LinkId {
        if let Some(id) = self.ids.get(&(dimension.to_owned(), pos)) {
            return *id;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.ids.insert((dimension.to_owned(), pos), id);
        self.positions.insert(id, (dimension.to_owned(), pos));
        id
    }
}

static RUNTIME: LazyLock<Mutex<Runtime>> = LazyLock::new(Mutex::default);

/// The table, never held across a call that changes the game.
fn runtime() -> MutexGuard<'static, Runtime> {
    RUNTIME.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

// --- the saved form -----------------------------------------------------------------------------------

/// `tag.getUUID("Freq")`.
pub fn freq_of(nbt: &Compound) -> Option<Freq> {
    match util::get(nbt, FREQ) {
        Some(Tag::IntArray(ints)) if ints.len() == 4 => Some(link_rules::freq_from_ints([ints[0], ints[1], ints[2], ints[3]])),
        _ => None,
    }
}

pub fn freq_tag(freq: Freq) -> Tag {
    Tag::IntArray(link_rules::freq_to_ints(freq).to_vec())
}

fn placed_by(nbt: &Compound) -> Option<PlayerId> {
    match util::get(nbt, PLACED_BY) {
        Some(Tag::IntArray(ints)) if ints.len() == 4 => Some(link_rules::freq_from_ints([ints[0], ints[1], ints[2], ints[3]])),
        _ => None,
    }
}

// --- the link's own tick ------------------------------------------------------------------------------

/// A block was placed by the acting player: its network is the one that player last tuned to (if any), and the player owns
/// it. Called from `place_block` of each linked block; the block entity's first tick uses it ([`tick_link`]).
pub fn note_placed(host: &mut dyn Host, pos: Pos) {
    let Some(player) = host.acting_player() else {
        return;
    };
    let dimension = host.dimension().to_owned();
    let mut rt = runtime();
    let freq = rt.tuned.remove(&player.uuid);
    rt.placed.insert((dimension, pos), (freq, player.uuid));
}

/// `LogisticallyLinkedBehaviour`'s tick for the block entity at `pos` whose saved data is `nbt`: the frequency it starts with, the
/// registration with the network (`initialize`), the redstone priority (`redstonePowerChanged`), the keep-alive every ten ticks
/// (`lazyTick`). Returns the entries of the saved data that changed.
pub fn tick_link(host: &mut dyn Host, pos: Pos, nbt: &Compound, global: bool, power: u8, now: i64) -> Vec<(&'static str, Tag)> {
    let dimension = host.dimension().to_owned();
    let mut changes: Vec<(&'static str, Tag)> = Vec::new();
    let mut guard = runtime();
    let rt = &mut *guard;
    let mut owner = placed_by(nbt);
    let freq = match freq_of(nbt) {
        Some(freq) => freq,
        None => {
            let (tuned, by) = rt.placed.remove(&(dimension.clone(), pos)).unwrap_or((None, 0));
            let freq = tuned.unwrap_or_else(|| (u128::from(util::random()) << 64) | u128::from(util::random()));
            changes.push((FREQ, freq_tag(freq)));
            if by != 0 && owner.is_none() {
                owner = Some(by);
                changes.push((PLACED_BY, freq_tag(by)));
            }
            freq
        }
    };
    let id = rt.id_for(&dimension, pos);
    let mut link = Link::new(id, freq, global);
    link.redstone_power = int(nbt, POWER).clamp(0, 15) as u8;
    link.added_globally = util::boolean(nbt, ADDED);
    link.placed_by = owner;
    link.loaded_globally = rt.seen.contains(&id) && global;
    let global_pos = GlobalPos { dimension: dimension.clone(), pos };
    if rt.seen.insert(id) {
        link.initialize(&mut rt.logistics, &global_pos, now);
        link.keep_alive(&mut rt.logistics, now);
    }
    let logistics = &mut rt.logistics;
    if link.redstone_power_changed(logistics, power, now) {
        changes.push((POWER, Tag::Int(i32::from(power))));
    }
    if (now + link_rules::lazy_salt(pos, net::LAZY_PERIOD)) % net::LAZY_PERIOD == 0 {
        link.lazy_tick(logistics, now);
    }
    if link.added_globally != util::boolean(nbt, ADDED) {
        changes.push((ADDED, Tag::Byte(i8::from(link.added_globally))));
    }
    changes
}

/// `LogisticallyLinkedBehaviour.destroy` and `unload`: the block is gone.
pub fn link_removed(host: &mut dyn Host, pos: Pos, nbt: &Compound, global: bool) {
    let dimension = host.dimension().to_owned();
    let now = util::game_time(&*host);
    let mut guard = runtime();
    let rt = &mut *guard;
    let Some(id) = rt.ids.remove(&(dimension.clone(), pos)) else {
        return;
    };
    rt.positions.remove(&id);
    rt.seen.remove(&id);
    let Some(freq) = freq_of(nbt) else {
        return;
    };
    let mut link = Link::new(id, freq, global);
    link.added_globally = util::boolean(nbt, ADDED);
    link.destroy(&mut rt.logistics, &GlobalPos { dimension, pos });
    rt.logistics.links.remove(freq, id, now);
}

/// A network's owner may be asked before using it (`mayInteract`): false tells the player (`logistically_linked.protected`).
pub fn may_interact(host: &mut dyn Host, freq: Freq, player: u128) -> bool {
    let allowed = runtime().logistics.networks.may_interact(freq, player);
    if !allowed {
        host.action_bar(player, Text::literal("This network is protected by its owner".to_owned()));
    }
    allowed
}

/// What the network's owner has done to the network (`StockKeeperLockPacket`): read by the ticker's command.
pub fn is_locked(freq: Freq) -> bool {
    runtime().logistics.networks.is_locked(freq)
}

pub fn set_locked(freq: Freq, player: PlayerId, lock: bool) {
    runtime().logistics.networks.set_locked(freq, player, lock);
}

pub fn may_administrate(freq: Freq, player: PlayerId) -> bool {
    runtime().logistics.networks.may_administrate(freq, player)
}

pub fn is_lockable(freq: Freq) -> bool {
    runtime().logistics.networks.is_lockable(freq)
}

/// `LogisticallyLinkedBlockItem.assignFrequency`: the next linked block `player` places joins `freq`.
pub fn tune(player: u128, freq: Freq) {
    runtime().tuned.insert(player, freq);
}

// --- the views of the rules -------------------------------------------------------------------------------

/// The attach face and facing of the Stock Link at `pos`, and so the way it is connected.
pub fn link_connection(host: &dyn Host, pos: Pos) -> Option<Dir> {
    let face = host.block_property(pos, "face").as_deref().and_then(AttachFace::from_name)?;
    let facing = util::facing(host, pos, "facing")?;
    Some(link_rules::connected_direction(face, facing))
}

/// The identity of the inventory at `target` (`InventoryIdentifier`): every part of a vault or a double chest has the same.
fn inventory_id(host: &dyn Host, target: Pos) -> u64 {
    let parts = inv::parts(host, target);
    let least = parts.iter().map(|(pos, _)| *pos).min().unwrap_or(target);
    link_rules::pos_id(least)
}

/// `PackagerBlockEntity.getAvailableItems` for the packager at `packager`, as the rules' view.
fn packager_view(host: &mut dyn Host, packager: Pos, ignored: Option<u64>) -> Option<PackagerView<ItemStack>> {
    let facing = util::facing(&*host, packager, "facing")?;
    let target = util::add(packager, facing.opposite().normal());
    let queued = host.mod_block_entity(packager).map_or(0, |entity| packager_glue::read(&entity.nbt).queued.len());
    let held = target_inv::read(host, target);
    let inventory = held.is_some().then(|| inventory_id(&*host, target));
    let stacks: Vec<(ItemStack, u32)> =
        held.unwrap_or_default().iter().map(|(_, stack)| (items::key(stack), stack.count)).collect();
    Some(PackagerView {
        id: link_rules::pos_id(packager),
        inventory,
        targets_ignored: inventory.is_some() && inventory == ignored,
        queued,
        available: net::available_items(&stacks, None),
    })
}

/// The views of the links present: `getAllPresent` with `isValidLink` and `PackagerLinkBlockEntity.getPackager`. Also where each
/// packager is.
fn build_views(host: &mut dyn Host, entries: &[(LinkId, String, Pos)], ignored: Option<u64>) -> (Vec<LinkView<ItemStack>>, Vec<(PackagerId, Pos)>) {
    let mut views = Vec::new();
    let mut packagers = Vec::new();
    for (id, dimension, pos) in entries {
        if dimension != host.dimension() {
            continue;
        }
        let block = host.block(*pos);
        if !is_linked_block(&block) {
            continue;
        }
        let power = host
            .mod_block_entity(*pos)
            .map_or(0, |entity| int(&entity.nbt, POWER).clamp(0, 15) as u8);
        let mut packager = None;
        if block == STOCK_LINK
            && power != 15
            && let Some(connected) = link_connection(&*host, *pos)
        {
            let at = net::packager_pos(*pos, connected);
            if host.block(at) == packager_glue::PACKAGER {
                packager = packager_view(host, at, ignored);
                if packager.is_some() {
                    packagers.push((link_rules::pos_id(at), at));
                }
            }
        }
        views.push(LinkView { id: *id, power, packager });
    }
    (views, packagers)
}

fn entries_of(rt: &mut Runtime, freq: Freq, now: i64) -> Vec<(LinkId, String, Pos)> {
    rt.logistics
        .links
        .all_present(freq, now)
        .into_iter()
        .filter_map(|id| rt.positions.get(&id).map(|(dimension, pos)| (id, dimension.clone(), *pos)))
        .collect()
}

/// `LogisticsManager.getSummaryOfNetwork`: what the network holds (cached 20 ticks, or 1 for the accurate one).
pub fn summary(host: &mut dyn Host, freq: Freq, accurate: bool) -> Summary<ItemStack> {
    let now = util::game_time(&*host);
    let mut guard = runtime();
    let Runtime { logistics, positions, .. } = &mut *guard;
    logistics.summary_of_network(freq, accurate, now, |ids| {
        let entries: Vec<(LinkId, String, Pos)> =
            ids.iter().filter_map(|id| positions.get(id).map(|(dimension, pos)| (*id, dimension.clone(), *pos))).collect();
        build_views(host, &entries, None).0
    })
}

struct Rng;

impl Random for Rng {
    fn next_int(&mut self) -> i32 {
        util::random() as i32
    }

    fn next_below(&mut self, bound: usize) -> usize {
        (util::random() % bound.max(1) as u64) as usize
    }
}

/// `LogisticsManager.broadcastPackageRequest`: the order is split over the packagers that hold the goods, and each is told to
/// pack it (`performPackageRequests`). Whether it was accepted (an empty order, or a packager with too many boxes queued, is not).
pub fn broadcast(host: &mut dyn Host, freq: Freq, kind: RequestType, order: &Context<ItemStack>, address: &str) -> bool {
    let now = util::game_time(&*host);
    let entries = entries_of(&mut runtime(), freq, now);
    let (views, packagers) = build_views(host, &entries, None);
    let result = net::broadcast_package_request(&views, kind, order, address, &mut Rng);
    if !result.accepted {
        return false;
    }
    for dispatch in result.dispatches {
        let Some((_, at)) = packagers.iter().find(|(id, _)| *id == dispatch.packager) else {
            continue;
        };
        let at = *at;
        let mut requests = dispatch.requests;
        // `flashLink`.
        host.play_sound("create:stock_link", util::centre(at), 2, 0.75, 1.25);
        net::run_attempts(&mut requests, |list| {
            packager_glue::attempt_requests(host, at, list);
        });
    }
    true
}

/// How many links hold the network up now (`InventorySummary.contributingLinks`), for a status line.
pub fn linked_count(host: &mut dyn Host, freq: Freq) -> u32 {
    summary(host, freq, false).contributing_links
}

// --- tuning ---------------------------------------------------------------------------------------------------

impl Hooks for Shared {
    /// `LogisticallyLinkedBlockItem.useOn`: one of the linked items used on a linked block takes its network along.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if !is_linked_item(&event.stack.item) {
            return Flow::Continue;
        }
        let pos = event.pos;
        if !is_linked_block(&host.block(pos)) {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.sneaking || player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        let Some(freq) = host.mod_block_entity(pos).and_then(|entity| freq_of(&entity.nbt)) else {
            return Flow::Continue;
        };
        if !util::first_click(&*host, player.uuid, pos) {
            return Flow::Cancel;
        }
        if may_interact(host, freq, player.uuid) {
            runtime().tuned.insert(player.uuid, freq);
            host.action_bar(player.uuid, Text::literal("Tuned to this network: the next linked block you place joins it".to_owned()));
        }
        Flow::Cancel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frequency_is_saved_as_four_ints() {
        let mut nbt = Compound::new();
        assert_eq!(freq_of(&nbt), None);
        nbt.insert(FREQ.into(), freq_tag(0xdead_beef_u128 << 40));
        assert_eq!(freq_of(&nbt), Some(0xdead_beef_u128 << 40));
    }

    #[test]
    fn the_four_linked_blocks_are_known() {
        for block in [STOCK_LINK, STOCK_TICKER, REDSTONE_REQUESTER, FACTORY_GAUGE] {
            assert!(is_linked_block(block));
        }
        assert!(!is_linked_block("create:packager"));
    }
}
