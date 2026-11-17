//! The Schematic Table (`SchematicTableBlock`, `SchematicTableBlockEntity`, `SchematicTableMenu`):
//! writes a file of the schematics folder onto an Empty Schematic. Its screen lists the folder and
//! has an upload button; the two slots are the input and the output.
//!
//! Known differences from Create, for the README:
//! - **Stand-in for the screen.** The table opens a hopper-shaped menu: slot 1 is the input (an Empty
//!   Schematic, a Schematic And Quill or a Schematic), slot 2 the output; slots 3 to 5 are unused,
//!   leave them empty. `/schematic list` lists the files and `/schematic write <number or name>`
//!   writes one onto the input and puts the written Schematic in the output (Create's upload
//!   button), for the table you right-clicked last. The menu does not check what goes in a slot.
//! - Nothing is uploaded: the Schematic names a file of the one `schematics/` folder (see
//!   `schematic_files.rs`), so the write is instant and there is no progress bar.
//! - The table's slots are not reachable by hoppers or funnels.
//! - `/schematic give <file or number>` hands over a written Schematic at once: Horizon's `/give`
//!   can't carry a mod's data component (`Unknown item component`), so the command is the way to get
//!   one without the table.

use create_kinetics::looking;
use create_kinetics::rules::schematic_table::pick;
use crate::compat::{
    BlockEntityContainerEvent, BlockEntityEvent, BlockStateId, Compound, Flow, Hooks, Host,
    LevelHost, ModContainer, RightClickBlockEvent, StateForPlacementEvent, UseItemOnEvent,
};
use std::collections::BTreeMap;
use std::sync::Mutex;

use super::schematic::{self as items, EMPTY, QUILL, SCHEMATIC, TABLE};
use super::schematic_files as files;
use crate::util;

/// Slots of the hopper menu: the first is the input, the second the output.
const SLOTS: usize = 5;
const INPUT: usize = 0;
const OUTPUT: usize = 1;

struct Tables;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Tables)
}

/// The table each player right-clicked last, for the command.
static TARGETS: Mutex<BTreeMap<u128, (String, [i32; 3])>> = Mutex::new(BTreeMap::new());

fn container() -> ModContainer {
    ModContainer {
        slots: SLOTS,
        faces: [(); 6].map(|()| Vec::new()),
        inputs: Vec::new(),
        outputs: Vec::new(),
        menu: Some("minecraft:hopper".to_owned()),
    }
}

/// `/schematic write`: the upload button of Create's table screen.
pub fn write(host: &mut dyn Host, uuid: u128, name: &str) {
    let target = TARGETS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&uuid).cloned();
    let Some((dimension, pos)) = target.filter(|(d, pos)| d == host.dimension() && host.block(*pos) == TABLE) else {
        host.tell(uuid, crate::compat::Text::literal("Right-click a Schematic Table first".to_owned()));
        return;
    };
    let reply = |host: &mut dyn Host, line: String| {
        host.tell(uuid, crate::compat::Text::literal(line));
    };
    let Some(player) = host.acting_player() else {
        return;
    };
    let near = (0..3).map(|i| (player.position[i] - f64::from(pos[i])).powi(2)).sum::<f64>() < 12.0 * 12.0;
    if !near {
        reply(host, "That Schematic Table is too far away".to_owned());
        return;
    }
    let Some(entity) = host.mod_block_entity(pos) else {
        return;
    };
    let mut slots = entity.items.clone();
    slots.resize(SLOTS, None);
    let accepts = slots[INPUT].as_ref().is_some_and(|s| matches!(&*s.item, EMPTY | QUILL | SCHEMATIC));
    if !accepts {
        reply(host, "Put an Empty Schematic in the first slot of the table".to_owned());
        return;
    }
    if slots[OUTPUT].is_some() {
        reply(host, "Take the Schematic out of the second slot first".to_owned());
        return;
    }
    let names = files::list(&*host);
    let file = match pick(&names, name) {
        Ok(file) => file,
        Err(error) => {
            reply(host, error);
            return;
        }
    };
    match items::new_schematic(&*host, &file, &player.name) {
        Ok(stack) => {
            host.change_mod_block_entity(pos, &mut |e| {
                e.items.resize(SLOTS, None);
                e.items[INPUT] = None;
                e.items[OUTPUT] = Some(stack.clone());
            });
            host.block_entity_sync(pos);
            host.play_sound("minecraft:entity.item.pickup", util::centre(pos), 4, 0.5, 1.0);
            reply(host, format!("Upload Finished! {file}"));
        }
        Err(error) => reply(host, format!("Schematic failed to Load: {error}")),
    }
    let _ = dimension;
}

/// `/schematic give`: a written Schematic of a file of the folder into the player's inventory.
pub fn give(host: &mut dyn Host, uuid: u128, name: &str) {
    let reply = |host: &mut dyn Host, line: String| {
        host.tell(uuid, crate::compat::Text::literal(line));
    };
    let Some(player) = host.acting_player() else {
        return;
    };
    let names = files::list(&*host);
    let file = match pick(&names, name) {
        Ok(file) => file,
        Err(error) => {
            reply(host, error);
            return;
        }
    };
    match items::new_schematic(&*host, &file, &player.name) {
        Ok(stack) => {
            if host.give_stack(uuid, stack).is_some() {
                reply(host, "Your inventory is full".to_owned());
            } else {
                reply(host, format!("Gave a Schematic of {file}"));
            }
        }
        Err(error) => reply(host, format!("Schematic failed to Load: {error}")),
    }
}

impl Hooks for Tables {
    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        (event.block == TABLE).then(|| (TABLE.to_owned(), Compound::new()))
    }

    fn block_entity_container(&self, _: &dyn Host, event: &mut BlockEntityContainerEvent<'_>) -> Option<ModContainer> {
        (event.block == TABLE).then(container)
    }

    /// `SchematicTableBlock.getStateForPlacement`: facing the player.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != TABLE {
            return None;
        }
        let facing = looking(context.yaw, 0.0).opposite();
        level.state_with_property(event.state, "facing", facing.name()).map(Some)
    }

    /// Remembers the table for the command; the menu opens as it always does.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        if host.block(event.pos) != TABLE {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR || player.sneaking {
            return Flow::Continue;
        }
        let dimension = host.dimension().to_owned();
        TARGETS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(player.uuid, (dimension, event.pos));
        Flow::Continue
    }

    /// The Wrench picks the table up, sneaking.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != TABLE {
            return Flow::Continue;
        }
        let sneaking = host.acting_player().is_some_and(|p| p.sneaking);
        if !sneaking {
            return Flow::Continue;
        }
        util::wrench(host, event.pos, |_| {})
    }
}
