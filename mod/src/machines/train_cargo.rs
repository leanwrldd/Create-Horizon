//! The cargo of a train's carriages (`TrainCargoManager`, `MountedStorageManager`): the chests, barrels, shulker boxes and
//! Item Vaults among a carriage's blocks and its Fluid Tanks, as the counts the schedule conditions read and the fuel
//! `Train.burnFuel` takes. The decisions are `create_kinetics::rules::trains::train_cargo`; the storage is read from the
//! carriage's structure (`portable_interface_storage`, which reads a structure's blocks' saved `Items` and `TankContent`).
//!
//! What this does:
//! - [`sync`]: every 20 ticks (and after a burn) the carriage's `CargoState` is made to match what its blocks hold. A change
//!   of the items or fluids is one `changeDetected` (the version `ItemThresholdCondition` and the others compare, and the idle
//!   ticker `IdleCargoCondition` reads).
//! - [`burn`]: `TrainEvent::FuelConsumed`: one item leaves the fuel slot the train burned (and the container of a lava
//!   bucket is put back), in the structure and so in what the carriage sets down.
//!
//! Differences from Create, for the README: see `rules::trains::train_cargo`'s header. Nothing puts items into or takes them
//! out of a carriage's chests while it runs here (a click on a chest on a structure opens a view of it,
//! `structure_click.rs`; the Portable Storage Interface isn't wired to trains), so the picture only changes by burning
//! fuel or by clicks on its storage. The Depot, the Toolbox and the Creative Crate on a carriage are not cargo.
//!
//! Type-checked and unit-tested only: not played.

use create_kinetics::Pos;
use create_kinetics::rules::portable_interface::Stack;
use create_kinetics::rules::trains::carriage::CargoState;
use create_kinetics::rules::trains::train_cargo::{self as rules, SlotItem, Storage, Tank};

use crate::compat::{Hooks, Host, ItemStack};

use super::contraption::Moving;
use super::portable_interface_storage::{apply_items, items_of, tanks_of};

struct Cargo;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Cargo)
}

impl Hooks for Cargo {}

/// Ticks between two looks at a carriage's storage.
pub const EVERY: i64 = 20;

fn block_name(moving: &Moving, local: Pos) -> String {
    moving.blocks.iter().find(|b| b.local == local).map(|b| b.state.name.clone()).unwrap_or_default()
}

/// A carriage's mounted item storages, slot by slot, in the order of their position.
fn storages(host: &dyn Host, moving: &Moving) -> Vec<Storage> {
    let items = items_of(host, moving);
    let mut out: Vec<Storage> = Vec::new();
    for slot in &items.inventory.slots {
        if out.last().is_none_or(|s| s.local != slot.block) {
            out.push(Storage { block: block_name(moving, slot.block), local: slot.block, slots: Vec::new() });
        }
        let item = slot.stack.as_ref().and_then(|stack| items.to_stack(stack)).map(|stack: ItemStack| SlotItem {
            max_stack: slot.stack.as_ref().map_or(64, |s| s.max),
            burn_time: super::burner::furnace_burn_time(host, &stack).unwrap_or(0).max(0),
            package_address: super::package::address_of(&stack),
            count: stack.count as i32,
            id: stack.item.to_string(),
        });
        if let Some(storage) = out.last_mut() {
            storage.slots.push(item);
        }
    }
    out
}

/// The carriage's contents as the rules sort them.
pub fn contents(host: &dyn Host, moving: &Moving) -> rules::Contents {
    let tanks: Vec<Tank> =
        tanks_of(moving).tanks.into_iter().map(|t| Tank { fluid: t.fluid, amount: t.amount }).collect();
    rules::contents(&storages(host, moving), &tanks)
}

/// Makes `storage` match the carriage's blocks, if `EVERY` ticks passed since it last did (or `force`). `silent` says a
/// change in them was already counted (the train's own `burnFuel`). Returns whether the picture was refreshed.
pub fn sync(host: &dyn Host, moving: &Moving, storage: &mut CargoState, last: &mut i64, now: i64, force: bool, silent: bool) -> bool {
    if !force && now - *last < EVERY {
        return false;
    }
    *last = now;
    rules::refresh(storage, contents(host, moving), silent);
    true
}

/// `Train.burnFuel`'s extraction from fuel slot `slot` of the carriage (and `insertItemStacked` of what the fuel leaves).
/// False when the slot held nothing.
pub fn burn(host: &dyn Host, moving: &mut Moving, slot: usize) -> bool {
    let mut items = items_of(host, moving);
    // What the slot holds, to know what it leaves behind.
    let fuel_pos: Vec<usize> = items
        .inventory
        .slots
        .iter()
        .enumerate()
        .filter(|(_, s)| rules::can_use_for_fuel(&block_name(moving, s.block)))
        .map(|(i, _)| i)
        .collect();
    let Some(&at) = fuel_pos.get(slot) else { return false };
    let Some(have) = items.inventory.slots[at].stack.clone() else { return false };
    let leaves: Option<Stack> = items
        .to_stack(&have)
        .and_then(|stack| super::burner::fuel_remainder(&stack.item))
        .map(|id| items.from_stack(host, &ItemStack::new(id, 1)));
    let fuel_block = |local: Pos| rules::can_use_for_fuel(&block_name(moving, local));
    if !rules::consume_fuel(&mut items.inventory, &fuel_block, slot, leaves.as_ref()) {
        return false;
    }
    apply_items(moving, &items);
    true
}
