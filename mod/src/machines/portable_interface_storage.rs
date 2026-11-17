//! What a Portable Storage Interface and a Portable Fluid Interface reach in a structure: the
//! inventories and tanks among its blocks (`MountedStorageManager`, `ChestMountedStorage`,
//! `SimpleMountedStorage`, `ItemVaultMountedStorage`, `FluidTankMountedStorage`). A structure lives
//! in its controller's saved data, in Create's layout, so its storage is read from the blocks'
//! saved `Items` and `TankContent`, worked on by `create_kinetics::rules::portable_interface`
//! and written back there; the controller's cached structure is replaced so the set-down puts
//! the new contents where they belong.
//!
//! This file has no hooks of its own.

use std::collections::{BTreeMap, HashMap};

use create_kinetics::Pos;
use create_kinetics::rules::fluid_tank::CAPACITY;
use create_kinetics::rules::portable_interface::{self as rules, Inventory, Slot, Stack, StorageTank, Tanks};
use crate::compat::{Compound, Hooks, Host, ItemStack, Tag};

use super::contraption::{self as engine, CONTRAPTION, Moving, RUNNING};
use crate::util;

struct Shared;

impl Hooks for Shared {}

/// A module of shared code: no hooks of its own.
pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

/// The structure of the controller at `controller`, if it is running one.
pub(crate) fn load(host: &dyn Host, controller: Pos) -> Option<Moving> {
    let entity = host.mod_block_entity(controller)?;
    if !util::boolean(&entity.nbt, RUNNING) {
        return None;
    }
    let live = engine::live(host.dimension(), controller, &entity.nbt)?;
    Some(live.moving.clone())
}

/// Puts a structure back in its controller: its saved layout, and the cached copy the set-down
/// places from.
pub(crate) fn save(host: &mut dyn Host, controller: Pos, moving: Moving) {
    let tag = Tag::Compound(moving.to_nbt(false));
    host.change_mod_block_entity(controller, &mut |entity| {
        entity.nbt.insert(CONTRAPTION.into(), tag.clone());
    });
    engine::set_live(host.dimension(), controller, moving);
}

// --- items ------------------------------------------------------------------------------------------------

/// The structure's inventories as one, with what is needed to write them back.
pub(crate) struct Items {
    pub inventory: Inventory,
    templates: HashMap<String, ItemStack>,
}

fn key_of(stack: &ItemStack) -> String {
    format!("{}|{}|{:?}", stack.item, stack.damage, stack.extra)
}

fn slot_index(entry: &Tag) -> Option<usize> {
    match entry.get("Slot") {
        Some(Tag::Byte(slot)) => Some(usize::from(*slot as u8)),
        Some(Tag::Int(slot)) => usize::try_from(*slot).ok(),
        _ => None,
    }
}

/// The combined inventory of a structure: each storage block's slots, the blocks in order of
/// their position in the structure.
pub(crate) fn items_of(host: &dyn Host, moving: &Moving) -> Items {
    let mut templates: HashMap<String, ItemStack> = HashMap::new();
    let mut blocks: BTreeMap<Pos, (usize, Vec<Option<Stack>>)> = BTreeMap::new();
    for block in &moving.blocks {
        let Some(count) = rules::slots_of_block(&block.state.name) else {
            continue;
        };
        let mut slots: Vec<Option<Stack>> = vec![None; count];
        if let Some(data) = &block.data
            && let Some(Tag::List(list)) = util::get(data, "Items")
        {
            for entry in list {
                let (Some(index), Some(stack)) = (slot_index(entry), util::stack_of(entry)) else {
                    continue;
                };
                if index >= count {
                    continue;
                }
                let key = key_of(&stack);
                let max = host.max_stack_size(&stack.item).unwrap_or(64) as i32;
                slots[index] = Some(Stack { key: key.clone(), count: stack.count as i32, max });
                templates.entry(key).or_insert(stack);
            }
        }
        blocks.insert(block.local, (count, slots));
    }
    let mut inventory = Inventory::default();
    for (local, (_, slots)) in blocks {
        for (index, stack) in slots.into_iter().enumerate() {
            inventory.slots.push(Slot { block: local, index, stack });
        }
    }
    Items { inventory, templates }
}

impl Items {
    /// A stack of the inventory as the game's item stack.
    pub fn to_stack(&self, stack: &Stack) -> Option<ItemStack> {
        let mut out = self.templates.get(&stack.key)?.clone();
        out.count = u32::try_from(stack.count).ok()?;
        Some(out)
    }

    /// The rules' view of a game stack, remembering what it is.
    pub fn from_stack(&mut self, host: &dyn Host, stack: &ItemStack) -> Stack {
        let key = key_of(stack);
        self.templates.entry(key.clone()).or_insert_with(|| stack.clone());
        Stack { key, count: stack.count as i32, max: host.max_stack_size(&stack.item).unwrap_or(64) as i32 }
    }
}

/// Writes the inventory's slots into the blocks' saved `Items`.
pub(crate) fn apply_items(moving: &mut Moving, items: &Items) {
    let mut by_block: BTreeMap<Pos, Vec<Tag>> = BTreeMap::new();
    for slot in &items.inventory.slots {
        let entries = by_block.entry(slot.block).or_default();
        let Some(stack) = &slot.stack else { continue };
        let Some(game) = items.to_stack(stack) else { continue };
        let Tag::Compound(mut tag) = util::stack_tag(&game) else { continue };
        tag.insert("Slot".into(), Tag::Byte(slot.index as i8));
        entries.push(Tag::Compound(tag));
    }
    for block in &mut moving.blocks {
        let Some(entries) = by_block.remove(&block.local) else {
            continue;
        };
        let data = block.data.get_or_insert_with(Compound::new);
        data.insert("Items".into(), Tag::List(entries));
    }
}

// --- fluids -----------------------------------------------------------------------------------------------

/// A Fluid Tank's controller block has no `Controller` of its own.
fn is_tank_controller(block: &super::contraption::MovedBlock) -> bool {
    block.state.name == "create:fluid_tank"
        && block.data.as_ref().is_none_or(|data| !util::has(data, "Controller"))
}

/// The structure's tanks (each a Fluid Tank's controller): the fluid and its amount, the
/// capacity of the whole multi-block.
pub(crate) fn tanks_of(moving: &Moving) -> Tanks {
    let mut tanks: BTreeMap<Pos, StorageTank> = BTreeMap::new();
    for block in &moving.blocks {
        if !is_tank_controller(block) {
            continue;
        }
        let empty = Compound::new();
        let data = block.data.as_ref().unwrap_or(&empty);
        let width = util::int(data, "Size").max(1);
        let height = util::int(data, "Height").max(1);
        let (fluid, amount) = match util::get(data, "TankContent") {
            Some(content @ Tag::Compound(_)) => {
                let id = match content.get("id") {
                    Some(Tag::String(id)) => id.to_string_lossy(),
                    _ => String::new(),
                };
                let amount = match content.get("amount") {
                    Some(Tag::Int(amount)) => *amount,
                    _ => 0,
                };
                if id.is_empty() || id == "minecraft:empty" || amount <= 0 {
                    (None, 0)
                } else {
                    (Some(id), amount)
                }
            }
            _ => (None, 0),
        };
        tanks.insert(
            block.local,
            StorageTank { block: block.local, capacity: width * width * height * CAPACITY, fluid, amount },
        );
    }
    Tanks { tanks: tanks.into_values().collect() }
}

/// Writes the tanks' contents into the blocks' saved `TankContent`.
pub(crate) fn apply_tanks(moving: &mut Moving, tanks: &Tanks) {
    for tank in &tanks.tanks {
        let Some(block) = moving.blocks.iter_mut().find(|b| b.local == tank.block) else {
            continue;
        };
        let mut content = Compound::new();
        content.insert("id".into(), Tag::String(tank.fluid.as_deref().unwrap_or("minecraft:empty").into()));
        content.insert("amount".into(), Tag::Int(if tank.fluid.is_some() { tank.amount } else { 0 }));
        let data = block.data.get_or_insert_with(Compound::new);
        data.insert("TankContent".into(), Tag::Compound(content));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::contraption::MovedBlock;
    use create_kinetics::rules::contraption::State;

    fn block(local: Pos, name: &str, data: Option<Compound>) -> MovedBlock {
        MovedBlock { local, state: State::parse(name), data }
    }

    fn tank_data(size: i32, height: i32, fluid: &str, amount: i32) -> Compound {
        let mut content = Compound::new();
        content.insert("id".into(), Tag::String(fluid.into()));
        content.insert("amount".into(), Tag::Int(amount));
        let mut data = Compound::new();
        data.insert("Size".into(), Tag::Int(size));
        data.insert("Height".into(), Tag::Int(height));
        data.insert("TankContent".into(), Tag::Compound(content));
        data
    }

    #[test]
    fn a_tank_controller_is_a_storage_tank_the_size_of_the_whole_multi_block() {
        let mut part = tank_data(2, 2, "minecraft:empty", 0);
        part.insert("Controller".into(), Tag::IntArray(vec![0, 0, 0]));
        let moving = Moving {
            anchor: [0, 0, 0],
            blocks: vec![
                block([0, 0, 0], "create:fluid_tank", Some(tank_data(2, 2, "minecraft:water", 3000))),
                block([1, 0, 0], "create:fluid_tank", Some(part)),
                block([2, 0, 0], "minecraft:stone", None),
            ],
            ..Moving::default()
        };
        let tanks = tanks_of(&moving);
        assert_eq!(tanks.tanks.len(), 1);
        assert_eq!(tanks.tanks[0].capacity, 2 * 2 * 2 * CAPACITY);
        assert_eq!(tanks.tanks[0].fluid.as_deref(), Some("minecraft:water"));
        assert_eq!(tanks.tanks[0].amount, 3000);
    }

    #[test]
    fn fluid_written_to_the_tanks_comes_back_out_of_the_structure() {
        let mut moving = Moving {
            anchor: [0, 0, 0],
            blocks: vec![block([0, 0, 0], "create:fluid_tank", Some(tank_data(1, 1, "minecraft:empty", 0)))],
            ..Moving::default()
        };
        let mut tanks = tanks_of(&moving);
        assert_eq!(tanks.fill("create:honey", 1500, false), 1500);
        apply_tanks(&mut moving, &tanks);
        let back = tanks_of(&moving);
        assert_eq!(back.tanks[0].fluid.as_deref(), Some("create:honey"));
        assert_eq!(back.tanks[0].amount, 1500);
        // It survives the structure's own saved layout.
        let reread = Moving::from_nbt(&moving.to_nbt(false)).expect("reads");
        assert_eq!(tanks_of(&reread).tanks[0].amount, 1500);
    }

    fn chest_with(items: &[(u8, &str, i32)]) -> Compound {
        let list = items
            .iter()
            .map(|(slot, id, count)| {
                let mut entry = Compound::new();
                entry.insert("Slot".into(), Tag::Byte(*slot as i8));
                entry.insert("id".into(), Tag::String((*id).into()));
                entry.insert("count".into(), Tag::Int(*count));
                Tag::Compound(entry)
            })
            .collect();
        let mut data = Compound::new();
        data.insert("Items".into(), Tag::List(list));
        data
    }

    #[test]
    fn the_stacks_in_a_chest_are_slots_and_changed_slots_are_written_back_as_items() {
        // `util::stack_of` reads these; the maths has no game types, so this is the glue's own check
        // of the slot layout, not of `max_stack_size` (which needs a host).
        let chest = chest_with(&[(3, "minecraft:stone", 5), (0, "minecraft:dirt", 1)]);
        let moving = Moving {
            anchor: [0, 0, 0],
            blocks: vec![block([2, 0, 0], "minecraft:chest[facing=north,type=single]", Some(chest))],
            ..Moving::default()
        };
        let slots: Vec<(usize, i32)> = moving.blocks[0]
            .data
            .as_ref()
            .and_then(|d| util::get(d, "Items"))
            .map(|items| match items {
                Tag::List(list) => list.iter().filter_map(|e| Some((slot_index(e)?, 1))).collect(),
                _ => Vec::new(),
            })
            .unwrap_or_default();
        assert_eq!(slots, vec![(3, 1), (0, 1)]);
        assert_eq!(rules::slots_of_block(&moving.blocks[0].state.name), Some(27));
    }
}
