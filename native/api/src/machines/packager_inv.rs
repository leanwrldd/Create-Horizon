//! What the Packager, the Repackager and the Frogport share: reading the inventory they work on,
//! taking stacks out of it, and unpacking a package's items into it (`InvManipulationBehaviour`,
//! `PackagerBlockEntity.attemptToSend`'s extraction, `DefaultUnpackingHandler.unpack`). The
//! decisions are `create_kinetics::rules::package`; the reading and the moving go through
//! `chute_inv.rs`, as the chutes and funnels do.
//!
//! Known differences from Create, for the README:
//! - Horizon can't ask a container whether it would take a stack, so the simulated unpack
//!   (`DefaultUnpackingHandler`'s first pass) runs on the container's saved items and a table of
//!   how many slots each kind of container has (chests, barrels, shulker boxes, hoppers, droppers,
//!   the Item Vault). A container that isn't in the table takes the items for real, and whatever
//!   doesn't fit is dropped as items at the Packager rather than refusing the box.
//! - A belt or a depot as the inventory behind a Packager can be unpacked into (as a hopper
//!   would) but isn't read for packing.

use create_kinetics::rules::package::{self as rules, Target};
use create_kinetics::{Dir, Pos};
use horizon_client::mod_api::{Hooks, Host, ItemStack};

use super::chute_inv as inv;
use super::package as package_glue;
use crate::util;

/// A module of shared code: no hooks of its own.
struct Shared;

impl Hooks for Shared {}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

/// `ItemStackHandler`'s slot limit.
const SLOT_LIMIT: u32 = 64;

/// The blocks that are Create's own packaging machines: they are never the inventory behind a
/// Packager (`targetInv instanceof PackagerItemHandler`).
pub fn is_packager(block: &str) -> bool {
    block == super::packager::PACKAGER || block == super::packager::REPACKAGER
}

/// The slots of the inventory at `pos` that hold something, in slot order. `None` where the block
/// isn't an inventory that can be read (`chute_inv::items_of`), or is a packager.
pub fn read(host: &mut dyn Host, pos: Pos) -> Option<Vec<(usize, ItemStack)>> {
    if is_packager(&host.block(pos)) {
        return None;
    }
    let mut items = inv::items_of(host, pos)?;
    items.sort_by_key(|(slot, _)| *slot);
    Some(items)
}

/// How many slots the container at `pos` has, where that is known.
pub fn capacity(host: &dyn Host, pos: Pos) -> Option<usize> {
    let block = host.block(pos);
    let name = block.strip_prefix("minecraft:").unwrap_or(&block);
    match name {
        "chest" | "trapped_chest" => Some(27 * inv::parts(host, pos).len()),
        "barrel" => Some(27),
        "hopper" => Some(5),
        "dropper" | "dispenser" => Some(9),
        _ if name.ends_with("shulker_box") => Some(27),
        "create:item_vault" => Some(inv::VAULT_SLOTS * inv::parts(host, pos).len()),
        _ => None,
    }
}

/// `ItemHelper.extract`'s taking, for the slot of one stack: takes `count` of `stack` out of
/// `slot` of the container at `pos`, through its side `face`. Returns what came out.
pub fn take(host: &mut dyn Host, pos: Pos, face: Dir, slot: usize, count: u32, stack: &ItemStack) -> u32 {
    let plan = rules_plan(stack, slot, count);
    inv::take(host, pos, face, &plan).map_or(0, |taken| taken.count)
}

fn rules_plan(stack: &ItemStack, slot: usize, count: u32) -> create_kinetics::rules::chute::Plan<ItemStack> {
    create_kinetics::rules::chute::Plan { key: package_glue::key(stack), count, takes: vec![(slot, count)] }
}

/// What unpacking into the inventory at `pos` came to.
#[derive(Debug, PartialEq, Eq)]
pub enum Unpack {
    /// Everything went in (what didn't, when the container couldn't be asked first, was dropped).
    Done,
    /// There was no room, or no inventory: nothing moved.
    Refused,
}

/// Whether `items` would fit the container at `pos` (`UnpackingHandler.unpack(.., simulate)`);
/// `None` where it can't be said.
fn fits(host: &mut dyn Host, pos: Pos, items: &[ItemStack]) -> Option<bool> {
    let name = host.block(pos);
    // The Creative Crate takes everything (its second slot voids it).
    if name == "create:creative_crate" {
        return Some(true);
    }
    // Belts and depots can be asked.
    if name == crate::belt::ID || name == crate::depot::ID {
        let mut fits = true;
        for stack in items {
            fits &= crate::belt::handler_insert(host, pos, stack, true).is_some_and(|rest| rest.count == 0);
        }
        return Some(fits);
    }
    let slots = capacity(host, pos)?;
    let held = inv::items_of(host, pos).unwrap_or_default();
    let mut targets: Vec<Target<ItemStack>> = (0..slots).map(|_| Target { stack: None, limit: SLOT_LIMIT }).collect();
    for (slot, stack) in held {
        if let Some(target) = targets.get_mut(slot) {
            target.stack = Some((package_glue::key(&stack), stack.count));
        }
    }
    let sizes: Vec<(ItemStack, u32)> = items
        .iter()
        .map(|stack| (package_glue::key(stack), host.max_stack_size(&stack.item).unwrap_or(64)))
        .collect();
    let max = |key: &ItemStack| sizes.iter().find(|(k, _)| k == key).map_or(64, |(_, max)| *max);
    let big: Vec<(ItemStack, u32)> = items.iter().map(|stack| (package_glue::key(stack), stack.count)).collect();
    Some(rules::unpack_fits(&targets, &big, &max))
}

/// `PackagerBlockEntity.unwrapBox` into the inventory at `target` (reached through its side
/// `face`): puts the items in if they all fit. `at` is where what doesn't fit drops.
pub fn unpack(host: &mut dyn Host, target: Pos, face: Dir, items: &[ItemStack], at: Pos) -> Unpack {
    if is_packager(&host.block(target)) || !inv::is_inventory(host, target) {
        return Unpack::Refused;
    }
    if fits(host, target, items) == Some(false) {
        return Unpack::Refused;
    }
    for stack in items {
        let rest = inv::insert(host, target, face, stack, false);
        if let Some(rest) = rest
            && rest.count > 0
        {
            util::spawn_item(host, util::centre(at), &rest, [0.0; 3], 10);
        }
    }
    Unpack::Done
}

/// A package into the inventory at `target` whole (`RepackagerBlockEntity.unwrapBox`): it goes
/// in if some slot takes it.
pub fn insert_whole(host: &mut dyn Host, target: Pos, face: Dir, stack: &ItemStack) -> bool {
    if is_packager(&host.block(target)) || !inv::is_inventory(host, target) {
        return false;
    }
    // A container with a known number of slots says first whether there is a free one or room
    // beside the same package (a package stacks to one).
    if let Some(slots) = capacity(host, target) {
        let held = inv::items_of(host, target).unwrap_or_default();
        if held.len() >= slots {
            return false;
        }
    }
    inv::insert(host, target, face, stack, false).is_some_and(|rest| rest.count == 0)
}
