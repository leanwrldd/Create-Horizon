//! The Copycat Step and Copycat Panel (`CopycatBlock`, `CopycatBlockEntity`, `CopycatStepBlock`,
//! `CopycatPanelBlock`, `CopycatModel`). Every rule is in `create_kinetics::rules::copycat`; this
//! keeps the material in the block entity, draws what the rules plan, and applies the clicks.
//!
//! How it is stored: as in Create, the block entity holds `Material` (a block state as
//! `NbtUtils.writeBlockState` writes it: `Name` and `Properties`) and `Item` (the stack used up);
//! none means the Copycat Base. The material is sent to clients in the update tag.
//!
//! Known differences from Create, for the README:
//! - The material is drawn whole, scaled into the shape, not cropped from its model, and a bars
//!   material shows the Copycat Bars model with its own texture (see the rules' header).
//! - Which blocks are taken is a baked list, and light, sound, friction, hardness, explosion
//!   resistance, the pick-block item and the connected textures of the material
//!   (`getSoundType`, `getLightEmission`, `getDestroyProgress`, `getCloneItemStack`, `getAppearance`...)
//!   are the Copycat's own: a lit lamp as material gives no light.
//! - The sound of a material being set is a stone block's place sound, not the material's.
//! - Neighbouring Copycats don't share a material and don't hide each other's faces
//!   (`CopycatBlockEntity.setMaterial`, `hidesNeighborFace`).
//! - The step's and panel's placement assists (`PoleHelper`, `PlacementHelper`) and
//!   waterlogging are left out. The item is returned when a player breaks the Copycat (not in
//!   Creative) or an explosion does; a block removed another way keeps it.

use create_kinetics::rules::copycat::{self as rules, Click, Material, PANEL, STEP};
use create_kinetics::rules::girder::hit_from_centre;
use create_kinetics::{Dir, Pos, looking};
use crate::compat::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityUpdateTagEvent, BlockStateId, Compound,
    DestroyBlockEvent, EquipmentSlot, Flow, Hooks, Host, ItemStack, LevelHost, LootDropsEvent,
    PlaceBlockEvent, StateForPlacementEvent, Tag, UseItemOnEvent,
};

use crate::util;

const MATERIAL: &str = "Material";
const ITEM: &str = "Item";
/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;

struct Copycats;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Copycats)
}

fn is_mine(block: &str) -> bool {
    block == STEP || block == PANEL
}

// --- the material in NBT -----------------------------------------------------------------------

fn material_tag(material: &Material) -> Tag {
    let mut properties = Compound::new();
    for (key, value) in &material.props {
        properties.insert(key.as_str().into(), Tag::String(value.as_str().into()));
    }
    let mut tag = Compound::new();
    tag.insert("Name".into(), Tag::String(material.block.as_str().into()));
    tag.insert("Properties".into(), Tag::Compound(properties));
    Tag::Compound(tag)
}

fn material_of(nbt: &Compound) -> Material {
    let Some(Tag::Compound(tag)) = util::get(nbt, MATERIAL) else {
        return Material::base();
    };
    let Some(Tag::String(name)) = util::get(tag, "Name") else {
        return Material::base();
    };
    let mut material = Material { block: name.to_string_lossy().to_string(), props: Vec::new() };
    if let Some(Tag::Compound(saved)) = util::get(tag, "Properties") {
        for (key, value) in saved.iter() {
            if let Tag::String(value) = value {
                material = material.with(&key.to_string_lossy(), &value.to_string_lossy());
            }
        }
    }
    material
}

fn read(host: &dyn Host, pos: Pos) -> (Material, Option<ItemStack>) {
    let Some(entity) = host.mod_block_entity(pos) else {
        return (Material::base(), None);
    };
    let item = util::get(&entity.nbt, ITEM).and_then(util::stack_of);
    (material_of(&entity.nbt), item)
}

fn write(host: &mut dyn Host, pos: Pos, material: &Material, item: Option<&ItemStack>) {
    host.change_mod_block_entity(pos, &mut |entity| {
        if material.is_custom() {
            entity.nbt.insert(MATERIAL.into(), material_tag(material));
        } else {
            util::remove(&mut entity.nbt, MATERIAL);
        }
        match item {
            Some(item) => {
                entity.nbt.insert(ITEM.into(), util::stack_tag(item));
            }
            None => {
                if material.is_custom() && util::has(&entity.nbt, ITEM) {
                    return;
                }
                util::remove(&mut entity.nbt, ITEM);
            }
        }
    });
    host.block_entity_sync(pos);
}

// --- the hooks ---------------------------------------------------------------------------------

impl Hooks for Copycats {
    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        is_mine(event.block).then(|| (event.block.to_owned(), Compound::new()))
    }

    /// The material, an empty compound for none so a client that had one drops it.
    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        if !is_mine(&event.entity.id) {
            return None;
        }
        let material = material_of(&event.entity.nbt);
        let mut tag = Compound::new();
        tag.insert(
            MATERIAL.into(),
            if material.is_custom() { material_tag(&material) } else { Tag::Compound(Compound::new()) },
        );
        Some(tag)
    }

    /// `CopycatModel`: the material in the Copycat's shape.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if !is_mine(event.block) {
            return;
        }
        let state = event.state;
        let Some(facing) = host.block_state_property(state, "facing").and_then(Dir::from_name) else {
            return;
        };
        let (panel, shape) = if event.block == PANEL {
            (Some(facing), rules::panel_box(facing))
        } else {
            (None, rules::step_box(facing, host.block_state_property(state, "half") == Some("top")))
        };
        let material = material_of(&event.entity.nbt);
        for part in rules::draw_plan(panel, shape, &material) {
            let props: Vec<(&str, &str)> = part.props.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            event.draw.push();
            event.draw.translate(part.translate);
            event.draw.scale(part.scale);
            host.draw_block(&mut *event.draw, &part.block, &props);
            event.draw.pop();
        }
        // It depends on its state and material only.
        event.draw.keep();
    }

    /// `CopycatStepBlock.getStateForPlacement`, `CopycatPanelBlock.getStateForPlacement`.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if context.item == STEP {
            let (top, facing) = rules::step_placement(
                rules::horizontal_of_yaw(context.yaw),
                util::dir(context.face),
                context.hit.y - f64::from(context.pos.y),
            );
            let state = level.state_with_property(event.state, "half", if top { "top" } else { "bottom" })?;
            return level.state_with_property(state, "facing", facing.name()).map(Some);
        }
        if context.item == PANEL {
            let facing = rules::panel_placement(looking(context.yaw, context.pitch));
            return level.state_with_property(event.state, "facing", facing.name()).map(Some);
        }
        None
    }

    /// `CopycatBlock.setPlacedBy`: an item in the off hand becomes the material.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        let item = &*event.stack.item;
        if item != STEP && item != PANEL {
            return;
        }
        let Some(player) = host.acting_player() else {
            return;
        };
        let Some(offhand) = host.player_equipment(player.uuid, EquipmentSlot::OffHand) else {
            return;
        };
        let face = looking(player.yaw, player.pitch);
        let Some(material) = rules::accepted(&offhand.item, Some(face), item == PANEL) else {
            return;
        };
        if read(&*host, event.pos).0.is_custom() {
            return;
        }
        let used = ItemStack::new(&offhand.item, 1);
        write(host, event.pos, &material, Some(&used));
        if player.game_mode != util::CREATIVE {
            host.take_items(&offhand.item, 1);
        }
    }

    /// `CopycatBlock.useItemOn` and `onWrenched`.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        let block = host.block(pos);
        if !is_mine(&block) {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        let item = event.stack.item.to_string();
        if item == util::WRENCH {
            // Sneaking picks the block up, which gives the item back; otherwise the material is reset.
            if player.sneaking {
                return util::wrench(host, pos, |_| {});
            }
            let (material, used) = read(&*host, pos);
            if !material.is_custom() {
                return Flow::Cancel;
            }
            if player.game_mode != util::CREATIVE
                && let Some(used) = used
                && let Some(rest) = host.give_stack(player.uuid, used)
            {
                util::spawn_item(host, util::centre(pos), &rest, [0.0; 3], 10);
            }
            host.change_mod_block_entity(pos, &mut |entity| {
                util::remove(&mut entity.nbt, MATERIAL);
                util::remove(&mut entity.nbt, ITEM);
            });
            host.block_entity_sync(pos);
            return Flow::Cancel;
        }
        if player.sneaking {
            return Flow::Continue;
        }
        let Some(face) = Dir::from_normal(event.face) else {
            return Flow::Continue;
        };
        let panel = block == PANEL;
        let eye = [player.position[0], player.position[1] + f64::from(player.eye_height), player.position[2]];
        let hit_y = hit_from_centre(eye, player.yaw, player.pitch, pos, face)[1] + 0.5;
        let applied = rules::accepted(&item, Some(face), panel).map(|m| {
            let facing = util::facing(&*host, pos, "facing").unwrap_or(Dir::Up);
            rules::prepare_material(m, facing, hit_y, rules::horizontal_of_yaw(player.yaw))
        });
        let (current, _) = read(&*host, pos);
        match rules::click(&current, applied) {
            Click::Pass => Flow::Continue,
            Click::Cycle(turned) => {
                write(host, pos, &turned, None);
                host.play_sound("minecraft:entity.item_frame.add_item", util::centre(pos), BLOCKS, 0.75, 0.95);
                Flow::Cancel
            }
            Click::Set(material) => {
                write(host, pos, &material, Some(&ItemStack::new(&item, 1)));
                host.play_sound("minecraft:block.stone.place", util::centre(pos), BLOCKS, 1.0, 0.75);
                if player.game_mode != util::CREATIVE {
                    host.take_items(&item, 1);
                }
                Flow::Cancel
            }
        }
    }

    /// `CopycatBlock.onRemove`: the item used up pops out where the block was (`playerWillDestroy`
    /// forgets it in Creative).
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        let pos = event.pos;
        if !is_mine(&host.block(pos)) {
            return Flow::Continue;
        }
        let creative = host.acting_player().is_some_and(|p| p.game_mode == util::CREATIVE);
        if !creative && let (material, Some(used)) = read(&*host, pos) && material.is_custom() {
            util::spawn_item(host, util::centre(pos), &used, [0.0; 3], 10);
        }
        Flow::Continue
    }

    /// The same for an explosion, which breaks the block through its loot table.
    fn loot_drops(&self, host: &mut dyn Host, event: &mut LootDropsEvent<'_>) {
        let context = event.context;
        let Some((pos, _)) = context.block else {
            return;
        };
        if context.player.is_some() || context.explosion.is_none() || !is_mine(&host.block(pos)) {
            return;
        }
        if let (material, Some(used)) = read(&*host, pos)
            && material.is_custom()
        {
            event.drops.push(used);
        }
    }
}
