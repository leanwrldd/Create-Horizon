//! `TrainCargoManager` (content/contraptions/minecart/TrainCargoManager.java) over `MountedStorageManager`
//! (content/contraptions/MountedStorageManager.java): what a carriage's chests, barrels, shulker boxes and vaults and its
//! fluid tanks hold, as the [`CargoState`] the train's schedule conditions and fuel burning read.
//!
//! A carriage keeps its storage in its blocks' saved data (`Items`, `TankContent`), as every structure here does, so
//! this works on a snapshot of it: the glue lists each mounted storage's slots, [`contents`] sorts them the way
//! `MountedStorageManager.initialize` does (`items` = every non-internal storage, `fuelItems` = those not on the
//! fuel blacklist), and [`refresh`] puts the picture into the carriage's `CargoState`.
//!
//! What Java does and how this differs:
//! - `CargoInvWrapper` and `CargoTankWrapper` call `changeDetected()` (the version goes up and the idle ticker resets)
//!   on every insert, extract or set that really moves something. Nothing here can see those calls: the glue reads the
//!   contents every few ticks, and [`refresh`] counts one change when they differ from the last picture (many moves
//!   inside one interval are one change; `getVersion` is only ever compared for equality, `ItemThresholdCondition`'s
//!   and the other conditions' `cargoVersion` checks).
//! - `MountedItemStorageWrapper` walks its storages in a `HashMap`'s order; here the storages come in the order of their
//!   position in the carriage, and a slot's place in `fuel` is the place of the same slot in the walk.
//! - The storages Create mounts that have no copy here (the Depot, the Toolbox, the Creative Crate, the fallback for
//!   other mods' inventories) are not in the list the glue builds.
//! - `Train.burnFuel` also puts the container item of a fuel (a lava bucket's bucket) back with
//!   `ItemHandlerHelper.insertItemStacked(fuelItems, ..)`: [`consume_fuel`].

use super::carriage::{CargoItem, CargoState, FuelSlot};
use crate::Pos;
use crate::rules::portable_interface::{Inventory, Stack};

/// `create:mounted_item_storage_type/internal` (`AllMountedItemStorageTypeTags.INTERNAL`): `create:dispenser`, the
/// storage of a dispenser or dropper. Not exposed: `isExposed` is false for it.
pub fn is_internal(block: &str) -> bool {
    matches!(block, "minecraft:dispenser" | "minecraft:dropper")
}

/// `MountedStorageManager.canUseForFuel`: exposed, and not on `create:mounted_item_storage_type/fuel_blacklist`
/// (`create:vault`, the Item Vault's storage).
pub fn can_use_for_fuel(block: &str) -> bool {
    !is_internal(block) && block != "create:item_vault"
}

/// What the glue knows of one stack in a slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotItem {
    pub id: String,
    pub count: i32,
    pub max_stack: i32,
    /// `stack.getBurnTime(null)` of one item (0 for something that doesn't burn).
    pub burn_time: i32,
    /// A package's address (`PackageItem.getAddress`).
    pub package_address: Option<String>,
}

/// One mounted item storage: the block that holds it and each slot of it, empty ones included
/// (`getSlots()` counts them, and `burnFuel` walks them).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Storage {
    pub block: String,
    pub local: Pos,
    pub slots: Vec<Option<SlotItem>>,
}

/// One mounted fluid tank (`getFluidInTank`): `None` when it holds nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tank {
    pub fluid: Option<String>,
    pub amount: i32,
}

/// What a carriage holds, sorted as `MountedStorageManager.initialize` does.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Contents {
    /// `getAllItems()`: the slots of every exposed storage that hold something.
    pub items: Vec<CargoItem>,
    /// `getFluids()`: the tanks that hold something.
    pub fluids: Vec<(String, i32)>,
    /// `getFuelItems()`: every slot of every fuel storage, empty ones too (a fuel slot of count 0).
    pub fuel: Vec<FuelSlot>,
}

/// `initialize`'s split of the storages into `items` and `fuelItems`, and the tanks.
pub fn contents(storages: &[Storage], tanks: &[Tank]) -> Contents {
    let mut out = Contents::default();
    for storage in storages.iter().filter(|s| !is_internal(&s.block)) {
        let fuel = can_use_for_fuel(&storage.block);
        for slot in &storage.slots {
            match slot {
                Some(item) => {
                    out.items.push(CargoItem {
                        id: item.id.clone(),
                        count: item.count,
                        max_stack: item.max_stack,
                        package_address: item.package_address.clone(),
                    });
                    if fuel {
                        out.fuel.push(FuelSlot { count: item.count, burn_time: item.burn_time });
                    }
                }
                None => {
                    if fuel {
                        out.fuel.push(FuelSlot { count: 0, burn_time: 0 });
                    }
                }
            }
        }
    }
    out.fluids = tanks
        .iter()
        .filter_map(|t| t.fluid.as_ref().filter(|_| t.amount > 0).map(|f| (f.clone(), t.amount)))
        .collect();
    out
}

/// Where each entry of [`Contents::fuel`] lives: the storage's position and the slot in it.
pub fn fuel_addresses(storages: &[Storage]) -> Vec<(Pos, usize)> {
    storages
        .iter()
        .filter(|s| can_use_for_fuel(&s.block))
        .flat_map(|s| (0..s.slots.len()).map(move |i| (s.local, i)))
        .collect()
}

/// Puts a fresh picture into the carriage's cargo. A different set of items or fluids is a change
/// (`changeDetected`: the version goes up and the idle ticker restarts) unless `silent` says it was counted already (the
/// train's own `burnFuel` calls `change_detected` itself). The fuel slots always follow. Returns whether the picture
/// changed.
pub fn refresh(state: &mut CargoState, now: Contents, silent: bool) -> bool {
    let changed = state.items != now.items || state.fluids != now.fluids;
    if changed && !silent {
        state.change_detected();
    }
    state.items = now.items;
    state.fluids = now.fluids;
    state.fuel = now.fuel;
    changed
}

/// What `Train.burnFuel` does to the storage when it burns the item of fuel slot `slot` of the carriage: one item
/// leaves it, and the item's container (`getCraftingRemainingItem`: the bucket of a lava bucket) is put back into
/// the fuel storages (`insertItemStacked`); what doesn't fit is lost, as in Java. `fuel_block` says whether the
/// storage at a position is a fuel storage. False when the slot is empty or there is no such slot.
pub fn consume_fuel(inventory: &mut Inventory, fuel_block: &dyn Fn(Pos) -> bool, slot: usize, remainder: Option<&Stack>) -> bool {
    let positions: Vec<usize> = inventory.slots.iter().enumerate().filter(|(_, s)| fuel_block(s.block)).map(|(i, _)| i).collect();
    let Some(&at) = positions.get(slot) else { return false };
    if inventory.extract(at, 1, false).is_none() {
        return false;
    }
    if let Some(container) = remainder {
        // `insertItemStacked(fuelItems, ..)`: into the fuel slots only.
        let mut fuel = Inventory { slots: positions.iter().map(|&i| inventory.slots[i].clone()).collect() };
        fuel.insert_stacked(container, false);
        for (slot, &i) in fuel.slots.into_iter().zip(&positions) {
            inventory.slots[i] = slot;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::portable_interface::Slot;

    fn coal(count: i32) -> Option<SlotItem> {
        Some(SlotItem { id: "minecraft:coal".into(), count, max_stack: 64, burn_time: 1600, package_address: None })
    }

    fn stone(count: i32) -> Option<SlotItem> {
        Some(SlotItem { id: "minecraft:stone".into(), count, max_stack: 64, burn_time: 0, package_address: None })
    }

    fn chest(local: Pos, slots: Vec<Option<SlotItem>>) -> Storage {
        Storage { block: "minecraft:chest".into(), local, slots }
    }

    #[test]
    fn items_are_the_filled_slots_and_fuel_is_every_slot() {
        let storages = [chest([0, 1, 0], vec![coal(3), None, stone(10)])];
        let c = contents(&storages, &[]);
        assert_eq!(c.items.len(), 2);
        assert_eq!(c.items[1].id, "minecraft:stone");
        assert_eq!(c.fuel, vec![FuelSlot { count: 3, burn_time: 1600 }, FuelSlot { count: 0, burn_time: 0 }, FuelSlot { count: 10, burn_time: 0 }]);
    }

    #[test]
    fn a_vault_holds_cargo_but_no_fuel() {
        let storages = [
            Storage { block: "create:item_vault".into(), local: [0, 1, 0], slots: vec![coal(64), None] },
            chest([1, 1, 0], vec![coal(1)]),
        ];
        let c = contents(&storages, &[]);
        assert_eq!(c.items.len(), 2);
        // Only the chest's slot is a fuel slot.
        assert_eq!(c.fuel, vec![FuelSlot { count: 1, burn_time: 1600 }]);
        assert_eq!(fuel_addresses(&storages), vec![([1, 1, 0], 0)]);
    }

    #[test]
    fn an_internal_storage_is_neither() {
        let storages = [Storage { block: "minecraft:dispenser".into(), local: [0, 0, 0], slots: vec![coal(5)] }];
        let c = contents(&storages, &[]);
        assert!(c.items.is_empty() && c.fuel.is_empty());
    }

    #[test]
    fn only_tanks_with_something_count() {
        let tanks = [
            Tank { fluid: Some("minecraft:water".into()), amount: 4000 },
            Tank { fluid: None, amount: 0 },
            Tank { fluid: Some("minecraft:lava".into()), amount: 0 },
        ];
        assert_eq!(contents(&[], &tanks).fluids, vec![("minecraft:water".to_owned(), 4000)]);
    }

    #[test]
    fn a_change_of_contents_is_one_change() {
        let mut state = CargoState::default();
        state.ticks_since_last_exchange = 50;
        let first = contents(&[chest([0, 1, 0], vec![stone(1)])], &[]);
        assert!(refresh(&mut state, first.clone(), false));
        assert_eq!((state.version, state.ticks_since_last_exchange), (1, 0));
        // The same picture again changes nothing.
        state.tick_idle_cargo_tracker();
        assert!(!refresh(&mut state, first, false));
        assert_eq!((state.version, state.ticks_since_last_exchange), (1, 1));
        let second = contents(&[chest([0, 1, 0], vec![stone(2)])], &[]);
        assert!(refresh(&mut state, second, false));
        assert_eq!(state.version, 2);
    }

    #[test]
    fn a_counted_change_does_not_count_twice() {
        let mut state = CargoState::default();
        refresh(&mut state, contents(&[chest([0, 1, 0], vec![coal(2)])], &[]), false);
        let v = state.version;
        // `burnFuel` already called change_detected; the new picture is the same change.
        state.change_detected();
        assert!(refresh(&mut state, contents(&[chest([0, 1, 0], vec![coal(1)])], &[]), true));
        assert_eq!(state.version, v + 1);
    }

    fn slot(block: Pos, index: usize, key: &str, count: i32) -> Slot {
        Slot { block, index, stack: Some(Stack { key: key.into(), count, max: if key.contains("bucket") { 1 } else { 64 } }) }
    }

    fn empty(block: Pos, index: usize) -> Slot {
        Slot { block, index, stack: None }
    }

    #[test]
    fn burning_takes_one_item() {
        let mut inv = Inventory { slots: vec![empty([0, 0, 0], 0), slot([0, 0, 0], 1, "coal", 3)] };
        assert!(consume_fuel(&mut inv, &|_| true, 1, None));
        assert_eq!(inv.slots[1].stack.as_ref().map(|s| s.count), Some(2));
        // The last one empties the slot.
        let mut inv = Inventory { slots: vec![slot([0, 0, 0], 0, "coal", 1)] };
        assert!(consume_fuel(&mut inv, &|_| true, 0, None));
        assert!(inv.slots[0].stack.is_none());
        // Nothing to take from an empty slot or a slot that isn't there.
        assert!(!consume_fuel(&mut inv, &|_| true, 0, None));
        assert!(!consume_fuel(&mut inv, &|_| true, 5, None));
    }

    #[test]
    fn a_lava_bucket_leaves_its_bucket_in_the_fuel_storage_not_the_vault() {
        let vault = [9, 9, 9];
        let chest_pos = [0, 0, 0];
        let mut inv = Inventory { slots: vec![empty(vault, 0), slot(chest_pos, 0, "lava_bucket", 1), empty(chest_pos, 1)] };
        let bucket = Stack { key: "bucket".into(), count: 1, max: 1 };
        assert!(consume_fuel(&mut inv, &|p| p == chest_pos, 0, Some(&bucket)));
        // The lava bucket's own slot is the first empty slot of the fuel storage now.
        assert_eq!(inv.slots[1].stack.as_ref().map(|s| s.key.as_str()), Some("bucket"));
        assert!(inv.slots[0].stack.is_none());
    }

    #[test]
    fn fuel_slots_skip_the_non_fuel_storages() {
        // Slot 0 of the fuel walk is the chest's first slot, whatever sits before it in the inventory.
        let vault = [9, 9, 9];
        let chest_pos = [0, 0, 0];
        let mut inv = Inventory { slots: vec![slot(vault, 0, "coal", 5), slot(chest_pos, 0, "coal", 2)] };
        assert!(consume_fuel(&mut inv, &|p| p == chest_pos, 0, None));
        assert_eq!(inv.slots[0].stack.as_ref().map(|s| s.count), Some(5));
        assert_eq!(inv.slots[1].stack.as_ref().map(|s| s.count), Some(1));
    }
}
