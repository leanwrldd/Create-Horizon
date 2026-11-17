//! The Postboxes (`create:<colour>_postbox`, sixteen colours) (`PostboxBlock`,
//! `PostboxBlockEntity`, `PostboxRenderer`, `PackagePortBlockEntity`): a barrel-sized box that
//! holds up to 18 packages and has an address. In Create it is the end of a train line (a
//! Train Station links it, and trains carry packages to it); trains come last in this port, so
//! only the block, its inventory and its address are here.
//!
//! How it works here: the inventory is the block's container, so hoppers, funnels and chutes put
//! packages in (only those that don't match its own address, as the port's automation wrapper
//! lets) and take them out. A package right-clicked on it goes in; an empty hand shows its address
//! and packages; a sneaking empty hand takes them out; a renamed Name Tag sets the address. The
//! flag goes up while it holds packages.
//!
//! Known differences from Create: no train station link, no screen (the address is a Name Tag),
//! the lid is never drawn open (`open` stays false: Create sets it while the screen is open), the
//! flag rises at once and doesn't wiggle, no waterlogging, no clipboard address copying.

use create_kinetics::rules::frogport as rules;
use create_kinetics::rules::package as package_rules;
use horizon_client::mod_api::{
    BlockEntityContainerEvent, BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, Compound, ContainerAcceptsEvent, EquipmentSlot, Flow, Hooks, Host,
    ModBlockEntity, ModContainer, RightClickBlockEvent, StandaloneModelsEvent, Tag, Text, UseItemOnEvent,
};

use super::package as items;
use crate::util;

const ADDRESS: &str = "AddressFilter";
const FLAG: &str = "create:block/package_postbox/flag";

/// The dye colours, in Create's `DyeColor` order.
pub const COLOURS: [&str; 16] = [
    "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple",
    "blue", "brown", "green", "red", "black",
];

struct Postboxes;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Postboxes)
}

pub fn is_postbox(block: &str) -> bool {
    block.strip_prefix("create:").and_then(|b| b.strip_suffix("_postbox")).is_some_and(|c| COLOURS.contains(&c))
}

fn address(nbt: &Compound) -> String {
    match util::get(nbt, ADDRESS) {
        Some(Tag::String(text)) => text.to_string_lossy(),
        _ => String::new(),
    }
}

fn matches_own(nbt: &Compound, stack: &horizon_client::mod_api::ItemStack) -> Option<bool> {
    Some(package_rules::match_address(&items::address(stack), &address(nbt)))
}

fn filled(entity: &ModBlockEntity) -> usize {
    entity.items.iter().flatten().count()
}

impl Hooks for Postboxes {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        if !event.models.iter().any(|m| m == FLAG) {
            event.models.push(FLAG.to_owned());
        }
    }

    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        is_postbox(event.block).then(|| (event.block.to_owned(), Compound::new()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, _: &mut BlockEntityTicksEvent<'_>) {}

    fn block_entity_container(&self, _: &dyn Host, event: &mut BlockEntityContainerEvent<'_>) -> Option<ModContainer> {
        is_postbox(event.block).then(|| {
            let all: Vec<usize> = (0..rules::SLOTS).collect();
            ModContainer { slots: rules::SLOTS, faces: [(); 6].map(|()| all.clone()), inputs: all.clone(), outputs: all, menu: None }
        })
    }

    /// `PackagePortAutomationInventoryWrapper.insertItem`.
    fn container_accepts(&self, _: &dyn Host, event: &mut ContainerAcceptsEvent<'_>) -> Flow {
        if !is_postbox(&event.entity.id) {
            return Flow::Continue;
        }
        let stack = event.stack;
        if rules::automation_can_insert(items::is_package(stack), matches_own(&event.entity.nbt, stack)) {
            Flow::Continue
        } else {
            Flow::Cancel
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        if !is_postbox(&event.entity.id) {
            return None;
        }
        let mut tag = Compound::new();
        tag.insert(ADDRESS.into(), Tag::String(address(&event.entity.nbt).as_str().into()));
        // The client has no item slots: the flag needs only whether there are packages.
        tag.insert("Filled".into(), Tag::Byte(i8::from(filled(event.entity) > 0)));
        Some(tag)
    }

    /// `PostboxRenderer`: the flag, up while something is inside.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if !is_postbox(event.block) {
            return;
        }
        let up = util::boolean(&event.entity.nbt, "Filled");
        let Some(facing) = util::facing(host, event.pos, "facing") else {
            return;
        };
        // `180 - facing.toYRot()`.
        let angle = match facing {
            create_kinetics::Dir::South => 180.0,
            create_kinetics::Dir::West => 90.0,
            create_kinetics::Dir::North => 0.0,
            create_kinetics::Dir::East => -90.0,
            _ => 0.0,
        };
        let draw = &mut *event.draw;
        draw.push();
        draw.rotate_about([0.0, 1.0, 0.0], angle, [0.5; 3]);
        if up {
            draw.rotate_about([1.0, 0.0, 0.0], -90.0, [0.5, 10.0 / 16.0, 2.0 / 16.0]);
        }
        host.draw_model(draw, FLAG, 0xFF_FFFF);
        draw.pop();
        draw.still();
    }

    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if !is_postbox(&host.block(pos)) {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR
            || host.held_stack(player.uuid).is_some()
            || host.player_equipment(player.uuid, EquipmentSlot::OffHand).is_some()
        {
            return Flow::Continue;
        }
        if !util::first_click(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        let Some(entity) = host.mod_block_entity(pos) else {
            return Flow::Cancel;
        };
        if player.sneaking {
            let mut taken = 0;
            let mut stacks: Vec<_> = entity.items.clone();
            for slot in stacks.iter_mut() {
                if let Some(boxed) = slot.take() {
                    taken += 1;
                    if let Some(rest) = host.give_stack(player.uuid, boxed) {
                        util::spawn_item(host, player.position, &rest, [0.0; 3], 0);
                    }
                }
            }
            if taken > 0 {
                host.change_mod_block_entity(pos, &mut |e| e.items = stacks.clone());
                host.block_entity_sync(pos);
            }
            host.action_bar(player.uuid, Text::literal(format!("Took {taken} packages")));
            return Flow::Cancel;
        }
        let name = address(&entity.nbt);
        let line = format!(
            "Postbox: {}, {} of {} packages",
            if package_rules::is_blank(&name) { "no address".to_owned() } else { format!("\u{2192} {name}") },
            filled(&entity),
            rules::SLOTS
        );
        host.action_bar(player.uuid, Text::literal(line));
        Flow::Cancel
    }

    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        if !is_postbox(&host.block(pos)) {
            return Flow::Continue;
        }
        let stack = event.stack;
        let is_tag = &*stack.item == "minecraft:name_tag";
        if !is_tag && !items::is_package(stack) {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.sneaking || player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        if !util::first_click(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        if is_tag {
            let name = stack.data().and_then(|d| d.custom_name_plain()).unwrap_or_default();
            host.change_mod_block_entity(pos, &mut |e| {
                util::put(&mut e.nbt, ADDRESS, Tag::String(name.as_str().into()));
            });
            host.block_entity_sync(pos);
            let line = if name.is_empty() { "Address cleared".to_owned() } else { format!("Address: {name}") };
            host.action_bar(player.uuid, Text::literal(line));
            return Flow::Cancel;
        }
        let Some(entity) = host.mod_block_entity(pos) else {
            return Flow::Cancel;
        };
        let mut slots = entity.items.clone();
        slots.resize(rules::SLOTS, None);
        if let Some(slot) = slots.iter_mut().find(|s| s.is_none()) {
            let mut one = stack.clone();
            one.count = 1;
            *slot = Some(one);
            host.change_mod_block_entity(pos, &mut |e| e.items = slots.clone());
            host.block_entity_sync(pos);
            items::shrink_held(player.uuid, &stack.item);
        }
        Flow::Cancel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_sixteen_colours_are_postboxes() {
        assert!(is_postbox("create:red_postbox"));
        assert!(is_postbox("create:light_blue_postbox"));
        assert!(!is_postbox("create:plaid_postbox"));
        assert!(!is_postbox("create:package_postbox"));
        assert!(!is_postbox("minecraft:red_postbox"));
    }
}
