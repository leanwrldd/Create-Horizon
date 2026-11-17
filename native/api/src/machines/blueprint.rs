//! The Crafting Blueprint (`BlueprintItem`, `BlueprintEntity`, `BlueprintRenderer`): a plate on a
//! wall, floor or ceiling that holds a crafting recipe as ghost items and crafts it from what the
//! player carries. The rules are `create_kinetics::rules::blueprint`; its header lists the known
//! differences from Create (a block stands in for the hanging entity, there is no screen, the
//! recipes are the baked crafting tables).
//!
//! Using it (stand-in for Create's menu; the lines show on the action bar):
//! - a click crafts the recipe once from the carried items, a sneaking click with an empty hand
//!   until the ingredients run out (at most 64 items);
//! - a sneaking click with an item sets the recipe that makes it, and the same item again moves
//!   on to the next recipe for it;
//! - the Wrench clears the recipe, sneaking with it picks the plate up.

use create_kinetics::rules::blueprint::{self as rules, Carried, Ghost};
use create_kinetics::rules::crafter_grid::Grid;
use create_kinetics::rules::crafter_recipes::crafting_remainder;
use create_kinetics::rules::placard as plate;
use create_kinetics::rules::redstone_link::horizontal_angle;
use create_kinetics::{Dir, Pos, looking};
use horizon_client::mod_api::serde_json::Value;
use horizon_client::mod_api::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, BlockStateId, Compound, DisplayContext, Flow,
    Hooks, Host, ItemStack, LevelHost, RightClickBlockEvent, StateForPlacementEvent, Tag, Text,
};

use super::crafter_recipes;
use super::filter;
use crate::{int, util};

pub const ID: &str = "create:crafting_blueprint";

const SLOTS: &str = "Slots";
const VARIANT: &str = "Variant";
const FOR: &str = "For";
/// The baked crafting tables (`crafter_recipes.rs`), to drop when the project reads vanilla's
/// recipes through the new mod API's `recipe_for`.
const BAKED_SHAPED: &str = "create:crafter_baked_shaped";
const BAKED_SHAPELESS: &str = "create:crafter_baked_shapeless";
/// `SoundSource.PLAYERS`.
const PLAYERS: usize = 7;
const SUPPORT_EVERY: i64 = 20;

struct Blueprint;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Blueprint)
}

fn tell(host: &mut dyn Host, line: String) {
    if let Some(player) = host.acting_player() {
        host.action_bar(player.uuid, Text::literal(line));
    }
}

/// The ghost items of a blueprint: ten slots, the grid and the result.
fn slots_of(nbt: &Compound) -> [Option<ItemStack>; 10] {
    let mut slots: [Option<ItemStack>; 10] = Default::default();
    if let Some(Tag::List(entries)) = nbt.get(&SLOTS.into()) {
        for entry in entries {
            let slot = match entry.get("Slot") {
                Some(Tag::Int(slot)) => *slot as usize,
                _ => continue,
            };
            if slot < 10 {
                slots[slot] = entry.get("Item").and_then(util::stack_of);
            }
        }
    }
    slots
}

fn set_slots(nbt: &mut Compound, slots: &[Option<ItemStack>; 10]) {
    let entries = slots
        .iter()
        .enumerate()
        .filter_map(|(slot, stack)| {
            let mut entry = Compound::new();
            entry.insert("Slot".into(), Tag::Int(slot as i32));
            entry.insert("Item".into(), util::stack_tag(stack.as_ref()?));
            Some(Tag::Compound(entry))
        })
        .collect();
    nbt.insert(SLOTS.into(), Tag::List(entries));
}

// --- the recipes ----------------------------------------------------------------------------------

/// The entries an ingredient of a baked recipe accepts: an item, `#tag`, or a list of them.
fn accepted(ingredient: &Value) -> Vec<String> {
    match ingredient {
        Value::String(entry) => vec![entry.clone()],
        Value::Array(list) => list.iter().filter_map(|e| e.as_str().map(str::to_owned)).collect(),
        Value::Object(o) => o
            .get("item")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| o.get("tag").and_then(Value::as_str).map(|tag| format!("#{tag}")))
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

/// A ghost as the stack a slot holds (`BlueprintItem.convertIItemListToFilter`).
fn ghost_stack(host: &dyn Host, ghost: &Ghost) -> Option<ItemStack> {
    let plain = |id: &str| ItemStack { item: id.into(), count: 1, damage: 0, extra: None };
    Some(match ghost {
        Ghost::Empty => return None,
        Ghost::Item(id) => plain(id),
        Ghost::Tag(tag) => filter::attribute_filter_for_tag(host, tag),
        Ghost::List(entries) => {
            let items: Vec<ItemStack> = entries.iter().filter_map(|e| ghost_stack(host, e)).collect();
            filter::list_filter(host, &items)
        }
    })
}

/// Every baked crafting recipe whose result is `item`, as the ten ghost slots it fills.
fn recipes_making(host: &dyn Host, item: &str) -> Vec<[Option<Ghost>; 10]> {
    let mut found = Vec::new();
    for kind in [BAKED_SHAPED, BAKED_SHAPELESS] {
        for recipe in host.recipes_of_type(kind) {
            let json = &recipe.json;
            if json.get("result").and_then(|r| r.get("id")).and_then(Value::as_str) != Some(item) {
                continue;
            }
            let ghosts = if kind == BAKED_SHAPED {
                let (Some(pattern), Some(key)) = (
                    json.get("pattern").and_then(Value::as_array),
                    json.get("key").and_then(Value::as_object),
                ) else {
                    continue;
                };
                let rows: Vec<Vec<Option<Ghost>>> = pattern
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|row| {
                        row.chars()
                            .map(|c| {
                                key.get(&c.to_string())
                                    .map(|ingredient| rules::ghost_for(&accepted(ingredient)))
                            })
                            .collect()
                    })
                    .collect();
                rules::shaped_slots(&rows)
            } else {
                let Some(list) = json.get("ingredients").and_then(Value::as_array) else {
                    continue;
                };
                let ingredients: Vec<Ghost> =
                    list.iter().map(|i| rules::ghost_for(&accepted(i))).collect();
                rules::shapeless_slots(&ingredients)
            };
            let mut slots: [Option<Ghost>; 10] = Default::default();
            slots[..9].clone_from_slice(&ghosts);
            found.push(slots);
        }
    }
    found
}

/// A click with `item` while sneaking: sets the recipe that makes it.
fn assign(host: &mut dyn Host, pos: Pos, item: &ItemStack) {
    let variants = recipes_making(&*host, &item.item);
    if variants.is_empty() {
        tell(host, format!("No crafting recipe makes {}", filter_name(&item.item)));
        return;
    }
    let entity = host.mod_block_entity(pos);
    let (same, before) = entity
        .as_ref()
        .map(|e| {
            let wanted = matches!(e.nbt.get(&FOR.into()), Some(Tag::String(s)) if s.to_string_lossy() == *item.item);
            (wanted, int(&e.nbt, VARIANT))
        })
        .unwrap_or((false, 0));
    let variant = if same { rules::next_variant(before, variants.len()) } else { 0 };
    let ghosts = &variants[variant as usize % variants.len()];
    let mut slots: [Option<ItemStack>; 10] = Default::default();
    for (slot, ghost) in ghosts.iter().enumerate() {
        slots[slot] = ghost.as_ref().and_then(|g| ghost_stack(&*host, g));
    }
    let mut result = ItemStack { item: item.item.clone(), count: 1, damage: 0, extra: None };
    if let Some(count) = result_count(&*host, &item.item, variant as usize) {
        result.count = count;
    }
    slots[rules::RESULT_SLOT] = Some(result);
    let name = filter_name(&item.item);
    host.change_mod_block_entity(pos, &mut |entity| {
        set_slots(&mut entity.nbt, &slots);
        entity.nbt.insert(VARIANT.into(), Tag::Int(variant));
        entity.nbt.insert(FOR.into(), Tag::String((&*item.item).into()));
    });
    host.block_entity_sync(pos);
    tell(host, format!("Recipe for {name} ({}/{})", variant + 1, variants.len()));
}

/// How many items the `variant`th recipe for `item` makes.
fn result_count(host: &dyn Host, item: &str, variant: usize) -> Option<u32> {
    let mut all = Vec::new();
    for kind in [BAKED_SHAPED, BAKED_SHAPELESS] {
        for recipe in host.recipes_of_type(kind) {
            let result = recipe.json.get("result")?;
            if result.get("id").and_then(Value::as_str) == Some(item) {
                all.push(result.get("count").and_then(Value::as_u64).unwrap_or(1) as u32);
            }
        }
    }
    all.get(variant).copied()
}

fn filter_name(item: &str) -> String {
    create_kinetics::rules::filter::display_name(item)
}

// --- crafting -------------------------------------------------------------------------------------

/// `BlueprintEntity.interactAt`: crafts the recipe from what the acting player carries.
fn craft(host: &mut dyn Host, pos: Pos, repeat: bool) {
    let Some(player) = host.acting_player() else {
        return;
    };
    let Some(entity) = host.mod_block_entity(pos) else {
        return;
    };
    let slots = slots_of(&entity.nbt);
    if slots[rules::RESULT_SLOT].is_none() {
        tell(host, "Sneak-click with an item to set the recipe that makes it".to_owned());
        return;
    }
    let mut requests: [Option<ItemStack>; 9] = Default::default();
    requests.clone_from_slice(&slots[..9]);
    // Only plain stacks can be taken from the inventory here.
    let carried: Vec<Carried<ItemStack>> = host
        .carried_items()
        .into_iter()
        .map(|(id, count)| Carried {
            stack: ItemStack { item: id.as_str().into(), count: 1, damage: 0, extra: None },
            count,
        })
        .collect();
    let session = {
        let world = &*host;
        rules::craft(
            &requests,
            &carried,
            repeat,
            &mut |request, stack| filter::matches(world, request, stack),
            &mut |grid| {
                let mut cells = Grid::default();
                for (slot, stack) in grid.iter().enumerate() {
                    cells.cells.insert(((slot % 3) as i32, 2 - (slot / 3) as i32), stack.clone());
                }
                crafter_recipes::apply(world, &cells).map(|made| made.count)
            },
        )
    };
    if session.grids.is_empty() {
        let line = match session.missing {
            Some(slot) => match &requests[slot] {
                Some(request) => format!("Missing: {}", filter_name(&request.item)),
                None => "Missing an ingredient".to_owned(),
            },
            None => "That recipe makes nothing".to_owned(),
        };
        tell(host, line);
        return;
    }
    // Take what the crafts used, by item.
    let mut by_item: Vec<(String, u32)> = Vec::new();
    for (index, n) in &session.taken {
        let id = carried[*index].stack.item.to_string();
        match by_item.iter_mut().find(|(have, _)| *have == id) {
            Some((_, total)) => *total += n,
            None => by_item.push((id, *n)),
        }
    }
    for (id, total) in &by_item {
        if host.take_items(id, *total) != *total {
            // The inventory changed under us: nothing more is made.
            tell(host, "Your inventory changed: try again".to_owned());
            return;
        }
    }
    let world = &*host;
    let mut outputs: Vec<ItemStack> = Vec::new();
    for grid in &session.grids {
        let mut cells = Grid::default();
        for (slot, stack) in grid.iter().enumerate() {
            cells.cells.insert(((slot % 3) as i32, 2 - (slot / 3) as i32), stack.clone());
        }
        if let Some(made) = crafter_recipes::apply(world, &cells) {
            outputs.push(made);
        }
        for stack in grid.iter().flatten() {
            if let Some(remainder) = crafting_remainder(&stack.item) {
                outputs.push(ItemStack { item: remainder.into(), count: 1, damage: 0, extra: None });
            }
        }
    }
    for output in outputs {
        if let Some(rest) = host.give_stack(player.uuid, output) {
            util::spawn_item(host, player.position, &rest, [0.0; 3], 0);
        }
    }
    // `SoundEvents.ITEM_PICKUP`, `.2f`, pitch 1 to 2.
    host.play_sound("minecraft:entity.item.pickup", player.position, PLAYERS, 0.2, 1.5);
}

impl Hooks for Blueprint {
    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == ID).then(|| (ID.to_owned(), Compound::new()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    /// `HangingEntity.survives`: the plate falls off (drops) when what it hangs on goes.
    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        let pos = event.pos;
        if util::game_time(host) % SUPPORT_EVERY != (i64::from(pos[0] ^ pos[2]) & 7) {
            return;
        }
        let (Some(face), Some(facing)) = (
            host.block_property(pos, "face").as_deref().and_then(plate::Attach::from_name),
            util::facing(host, pos, "facing"),
        ) else {
            return;
        };
        let support = util::add(pos, plate::connected_direction(face, facing).opposite().normal());
        if host.chunk_is_loaded(host.dimension(), [support[0] >> 4, support[2] >> 4])
            && host.block_collision(support).is_empty()
        {
            host.destroy_block(pos, None, false);
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        if event.entity.id != ID {
            return None;
        }
        let mut tag = Compound::new();
        set_slots(&mut tag, &slots_of(&event.entity.nbt));
        Some(tag)
    }

    /// `BlueprintRenderer`: the recipe's result is shown on the plate.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let Some(result) = slots_of(&event.entity.nbt)[rules::RESULT_SLOT].clone() else {
            event.draw.keep();
            return;
        };
        let face = host
            .block_state_property(event.state, "face")
            .and_then(plate::Attach::from_name)
            .unwrap_or(plate::Attach::Wall);
        let facing = host
            .block_state_property(event.state, "facing")
            .and_then(Dir::from_name)
            .unwrap_or(Dir::North);
        let block_item = host.default_state(&result.item).is_some();
        let mut pose = plate::item_pose(face, horizontal_angle(facing.normal()) as f32, block_item);
        pose.forward = 0.43;
        let draw = &mut *event.draw;
        draw.push();
        draw.translate([0.5; 3]);
        draw.rotate_about([0.0, 1.0, 0.0], pose.y_degrees, [0.0; 3]);
        draw.rotate_about([1.0, 0.0, 0.0], pose.x_degrees, [0.0; 3]);
        draw.translate([0.0, 0.0, pose.forward]);
        draw.scale([pose.scale; 3]);
        host.draw_item(draw, &result, DisplayContext::Fixed, None, 0);
        draw.pop();
        draw.keep();
    }

    /// `BlueprintItem.useOn`'s placement: on the face clicked, facing out of it.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != ID {
            return None;
        }
        let (face, facing) = plate::placement(util::dir(context.face), looking(context.yaw, 0.0));
        let state = level.state_with_property(event.state, "face", face.name())?;
        level.state_with_property(state, "facing", facing.name()).map(Some)
    }

    /// `BlueprintEntity.interactAt`, and the stand-in for its menu.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        if !util::first_click(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        let held = host.held_stack(player.uuid);
        match held {
            // The Wrench clears the recipe; sneaking with it picks the plate up.
            Some(stack) if &*stack.item == util::WRENCH => util::wrench(host, pos, |host| {
                host.change_mod_block_entity(pos, &mut |entity| {
                    set_slots(&mut entity.nbt, &Default::default());
                    util::remove(&mut entity.nbt, FOR);
                });
                host.block_entity_sync(pos);
                tell(host, "Recipe cleared".to_owned());
            }),
            Some(stack) if player.sneaking => {
                assign(host, pos, &stack);
                Flow::Cancel
            }
            _ => {
                craft(host, pos, player.sneaking);
                Flow::Cancel
            }
        }
    }
}
