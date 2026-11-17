//! The Re-Packager's side of `packager.rs` (`RepackagerBlockEntity.attemptToSend`,
//! `attemptToRepackage`, `unwrapBox`): it takes whole packages out of the inventory behind it,
//! passing the ones that are not a piece of an order straight out, and joining the pieces of an
//! order (once every piece is in the inventory) into fresh packages. Put in whole, it takes a
//! package into the inventory as the Packager would an item. The joining is
//! `create_kinetics::rules::repackager`.
//!
//! Known differences from Create: see `rules/repackager.rs` and `packager.rs`. Create's
//! Repackager also reports to computers (`RepackageEvent`); there are none here.

use create_kinetics::rules::package::OrderData;
use create_kinetics::rules::packager::State;
use create_kinetics::rules::repackager::{Fragment, Helper, OutBox};
use create_kinetics::{Dir, Pos};
use horizon_client::mod_api::{Hooks, Host, ItemStack};

use super::package as items;
use super::packager_inv as target_inv;
use crate::util;

/// A module of shared code: the block is `packager.rs`'s.
struct Shared;

impl Hooks for Shared {}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

/// The fragment a package is, when it carries order data.
fn fragment_of(stack: &ItemStack) -> Option<Fragment<ItemStack>> {
    let order = items::order_of(stack)?;
    Some(Fragment { order, address: items::address(stack), contents: items::contents_big(stack) })
}

/// The package a [`OutBox`] becomes.
fn make_box(host: &dyn Host, out: &OutBox<ItemStack>) -> ItemStack {
    let mut boxed = items::containing(host, &out.contents);
    items::set_address(host, &mut boxed, &out.address);
    if let Some(order) = &out.order {
        let order: &OrderData<ItemStack> = order;
        items::set_order(host, &mut boxed, order);
    }
    boxed
}

/// `RepackagerBlockEntity.attemptToSend`: one go over the inventory behind the Re-Packager.
/// Returns whether something started out or was queued.
pub fn attempt(
    host: &mut dyn Host,
    pos: Pos,
    target: Pos,
    face: Dir,
    state: &mut State<ItemStack>,
    now: i64,
    sign: &str,
) -> bool {
    let Some(inventory) = target_inv::read(host, target) else {
        return false;
    };
    let mut helper: Helper<ItemStack> = Helper::default();
    let mut completed: Option<i32> = None;

    for (slot, stack) in &inventory {
        if !items::is_package(stack) {
            continue;
        }
        let Some(fragment) = fragment_of(stack) else {
            // Not a piece of an order: it goes straight out.
            if target_inv::take(host, target, face, *slot, 1, stack) != 1 {
                continue;
            }
            let mut boxed = stack.clone();
            boxed.count = 1;
            state.begin_exit(boxed, now);
            // `updateSignAddress` then `addAddress(heldBox, signBasedAddress)`.
            if !create_kinetics::rules::package::is_blank(sign) {
                if let Some(held) = state.held.as_mut() {
                    items::set_address(&*host, held, sign);
                }
            }
            let _ = pos;
            return true;
        };
        completed = helper.add_fragment(fragment);
        if completed.is_some() {
            break;
        }
    }
    let Some(order_id) = completed else {
        return false;
    };

    let sizes = |key: &ItemStack| host.max_stack_size(&key.item).unwrap_or(64);
    let mut random = || util::random() as i32;
    let boxes = helper.repack(order_id, &sizes, &mut random);

    // The order's pieces leave the inventory, one from each slot that holds one.
    for (slot, stack) in &inventory {
        if items::is_package(stack) && items::order_id(stack) == order_id {
            target_inv::take(host, target, face, *slot, 1, stack);
        }
    }
    if boxes.is_empty() {
        return false;
    }
    for out in &boxes {
        let boxed = make_box(&*host, out);
        state.queued.push((boxed, out.count));
    }
    true
}
