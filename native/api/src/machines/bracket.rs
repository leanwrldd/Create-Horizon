//! The Wooden and Metal Brackets (`BracketBlock`, `BracketBlockItem`,
//! `BracketedBlockEntityBehaviour`, `BracketedKineticBlockEntityRenderer`,
//! `IWrenchableWithBracket`): an item that clings to a shaft, a cogwheel or a fluid pipe from the side
//! clicked and is drawn on it. The rules are `create_kinetics::rules::bracket`; this is the glue.
//!
//! How it is stored: as in Create, the bracket is not a block in the world. The shaft, the cogwheel
//! (small or large) or the fluid pipe keeps it in its block entity under `Bracket`, a block state
//! as `NbtUtils.writeBlockState` writes it (`Name` and `Properties`: `type`, `facing`,
//! `axis_along_first`). This module adds the key to those block entities from outside
//! (`change_mod_block_entity`), asks for it in their update tag (the shaft's through
//! `util::copy_bracket`, the pipe's here) and draws the bracket's block model on them
//! (`block_entity_submit`, after the block's own drawing).
//!
//! Known differences from Create, for the README:
//! - A Glass Fluid Pipe takes no bracket (its block entity, made in `fluid_pipe_attachments.rs`, is
//!   not asked for one), and a bracketed pipe still
//!   changes its sides with its neighbours (Create freezes them while the bracket is on,
//!   `FluidPipeBlock.updateBlockState`, and a neighbouring pipe joins it only along its straight
//!   axis). Nothing else brackets: the Encased and Powered Shafts don't in Create either.
//! - The bracket's item is returned to a survival player when a wrench takes it off, and dropped
//!   where the block was when a player or an explosion breaks the block; a block removed another
//!   way (a piston, a replaced block, a contraption) loses the bracket.
//! - Brackets are not carried by contraptions (`transformBracket`).
//! - The bracket item places nothing anywhere else: a click that doesn't fit takes no item, as
//!   `BracketBlockItem.useOn` does, but the client may briefly show the bracket block placed (the
//!   known placement-prediction gap).
//! - The sound is `scaffolding` or `netherite_block`'s place sound, played at 0.75 volume.

use create_kinetics::rules::bracket::{self as rules, Bracket, Type};
use create_kinetics::{Axis, Dir, looking};
use horizon_client::mod_api::{
    BlockEntitySubmitEvent, BlockEntityUpdateTagEvent, Compound, EquipmentSlot, Flow, Hooks, Host,
    DestroyBlockEvent, ItemStack, LootDropsEvent, RightClickBlockEvent, Tag, UseItemOnEvent,
};

use crate::util;

const WOODEN: &str = "create:wooden_bracket";
const METAL: &str = "create:metal_bracket";
const KEY: &str = "Bracket";
const SHAFT: &str = "create:shaft";
const COGWHEEL: &str = "create:cogwheel";
const LARGE_COGWHEEL: &str = "create:large_cogwheel";
const FLUID_PIPE: &str = "create:fluid_pipe";
/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;

struct Brackets;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Brackets)
}

type Pos = [i32; 3];

fn is_bracket(item: &str) -> bool {
    item == WOODEN || item == METAL
}

/// `BracketedBlockEntityBehaviour.canHaveBracket` for the block named: a shaft or cogwheel
/// (`AbstractSimpleShaftBlock`) or a fluid pipe that isn't encased.
fn can_have_bracket(block: &str) -> bool {
    matches!(block, SHAFT | COGWHEEL | LARGE_COGWHEEL | FLUID_PIPE)
}

/// `BracketBlock.getSuitableBracket(blockState, direction)` for the block at `pos`.
fn suitable(host: &dyn Host, pos: Pos, direction: Dir) -> Option<Bracket> {
    let block = host.block(pos);
    let (axis, kind) = match block.as_str() {
        SHAFT => (host.block_property(pos, "axis").as_deref().and_then(Axis::from_name), Type::Shaft),
        COGWHEEL | LARGE_COGWHEEL => {
            (host.block_property(pos, "axis").as_deref().and_then(Axis::from_name), Type::Cog)
        }
        FLUID_PIPE => {
            let sides = Dir::ALL.map(|d| host.block_property(pos, d.name()).as_deref() == Some("true"));
            (rules::straight_pipe_axis(&sides), Type::Pipe)
        }
        _ => return None,
    };
    rules::suitable(axis, direction, kind)
}

/// A bracket as `NbtUtils.writeBlockState` saves it.
fn to_tag(item: &str, bracket: &Bracket) -> Tag {
    let mut properties = Compound::new();
    properties.insert("type".into(), Tag::String(bracket.kind.name().into()));
    properties.insert("facing".into(), Tag::String(bracket.facing.name().into()));
    properties.insert(
        "axis_along_first".into(),
        Tag::String(if bracket.axis_along_first { "true" } else { "false" }.into()),
    );
    let mut tag = Compound::new();
    tag.insert("Name".into(), Tag::String(item.into()));
    tag.insert("Properties".into(), Tag::Compound(properties));
    Tag::Compound(tag)
}

/// The block name and its properties saved under `Bracket`, if a bracket is there.
fn from_tag(nbt: &Compound) -> Option<(String, Vec<(String, String)>)> {
    let Some(Tag::Compound(tag)) = util::get(nbt, KEY) else {
        return None;
    };
    let Some(Tag::String(name)) = util::get(tag, "Name") else {
        return None;
    };
    let name = name.to_string_lossy().to_string();
    if !is_bracket(&name) {
        return None;
    }
    let mut properties = Vec::new();
    if let Some(Tag::Compound(saved)) = util::get(tag, "Properties") {
        for (key, value) in saved.iter() {
            if let Tag::String(value) = value {
                properties.push((key.to_string_lossy().to_string(), value.to_string_lossy().to_string()));
            }
        }
    }
    Some((name, properties))
}

/// Whether a block entity's saved data holds a bracket (`BracketedBlockEntityBehaviour.isBracketPresent`).
pub(crate) fn in_nbt(nbt: &Compound) -> bool {
    from_tag(nbt).is_some()
}

/// The bracket on the block at `pos`: its item's name.
fn present(host: &dyn Host, pos: Pos) -> Option<String> {
    if !can_have_bracket(&host.block(pos)) {
        return None;
    }
    from_tag(&host.mod_block_entity(pos)?.nbt).map(|(name, _)| name)
}

/// The same bracket as the one at `pos` (`bracket == newBracket`).
fn same(host: &dyn Host, pos: Pos, item: &str, bracket: &Bracket) -> bool {
    let Some(entity) = host.mod_block_entity(pos) else {
        return false;
    };
    util::get(&entity.nbt, KEY) == Some(&to_tag(item, bracket))
}

/// `BracketedBlockEntityBehaviour.applyBracket` / `removeBracket`: writes or clears the bracket and
/// tells the clients.
fn set(host: &mut dyn Host, pos: Pos, bracket: Option<(&str, &Bracket)>) {
    host.change_mod_block_entity(pos, &mut |entity| match bracket {
        Some((item, bracket)) => {
            entity.nbt.insert(KEY.into(), to_tag(item, bracket));
        }
        None => {
            util::remove(&mut entity.nbt, KEY);
        }
    });
    host.block_entity_sync(pos);
    // `applyBracket` / `removeBracket`: `updateNeighbourShapes`, so the pipes beside a pipe join or
    // leave it by the new rule (`FluidPipeBlock.canConnectTo`).
    if host.block(pos) == FLUID_PIPE {
        super::fluid_pipe::refresh_neighbours(host, pos);
    }
}

/// `removeBracket`: takes the bracket off the block at `pos` and answers its item.
fn remove(host: &mut dyn Host, pos: Pos) -> Option<String> {
    let item = present(&*host, pos)?;
    set(host, pos, None);
    Some(item)
}

fn place_sound(item: &str) -> &'static str {
    // `SoundType.SCAFFOLDING` and `SoundType.NETHERITE_BLOCK` place sounds.
    if item == WOODEN {
        "minecraft:block.scaffolding.place"
    } else {
        "minecraft:block.netherite_block.place"
    }
}

/// The player holds the Wrench in either hand.
fn wrench_in_hand(host: &mut dyn Host, uuid: u128) -> bool {
    [EquipmentSlot::MainHand, EquipmentSlot::OffHand]
        .into_iter()
        .any(|slot| host.player_equipment(uuid, slot).is_some_and(|stack| &*stack.item == util::WRENCH))
}

impl Hooks for Brackets {
    /// The pipe's update tag: its own module sends none, so this sends the bracket. (The shaft's and the
    /// cogwheels' tags are made in `lib.rs`.)
    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        if event.entity.id != FLUID_PIPE {
            return None;
        }
        let mut tag = Compound::new();
        util::copy_bracket(&mut tag, &event.entity.nbt);
        // This answer shadows the pipe's own: the arms and rims go with it.
        super::fluid_pipe_attachments::copy_rims(&mut tag, &event.entity.nbt);
        Some(tag)
    }

    /// `BracketedKineticBlockEntityRenderer` and `PipeAttachmentModel`: the bracket's model on the
    /// block, after the block's own drawing.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if !can_have_bracket(event.block) {
            return;
        }
        let Some((name, properties)) = from_tag(&event.entity.nbt) else {
            return;
        };
        let properties: Vec<(&str, &str)> =
            properties.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        event.draw.still();
        host.draw_block(&mut *event.draw, &name, &properties);
        // Whatever the block's own drawing kept stays kept: it changes with the entity's data.
    }

    /// `IWrenchableWithBracket.tryRemoveBracket`: a click with the Wrench takes the bracket off before
    /// anything else (before the shaft is turned or the pipe made glass); a sneaking click lets the
    /// block be picked up and the bracket drop with it.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if !can_have_bracket(&host.block(pos)) || present(&*host, pos).is_none() {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR || !wrench_in_hand(host, player.uuid) {
            return Flow::Continue;
        }
        let Some(item) = remove(host, pos) else {
            return Flow::Continue;
        };
        if player.sneaking {
            // The block is about to be picked up: the bracket drops where it was.
            util::spawn_item(host, util::centre(pos), &ItemStack::new(&item, 1), [0.0; 3], 10);
            return Flow::Continue;
        }
        if player.game_mode == util::CREATIVE {
            return Flow::Cancel;
        }
        // `player.getInventory().placeItemBackInInventory`.
        if let Some(rest) = host.give_stack(player.uuid, ItemStack::new(&item, 1)) {
            util::spawn_item(host, util::centre(pos), &rest, [0.0; 3], 10);
        }
        Flow::Cancel
    }

    /// `BracketBlockItem.useOn`.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let item = event.stack.item.to_string();
        if !is_bracket(&item) {
            return Flow::Continue;
        }
        let pos = event.pos;
        // Nothing but a shaft, a cogwheel or a pipe takes one, and the item never places a block.
        if !can_have_bracket(&host.block(pos)) {
            return Flow::Cancel;
        }
        let Some(clicked) = Dir::from_normal(event.face) else {
            return Flow::Cancel;
        };
        let Some(player) = host.acting_player() else {
            return Flow::Cancel;
        };
        let mut fit = suitable(&*host, pos, clicked);
        if fit.is_none() {
            // `Direction.orderedByNearest(player)[0].getOpposite()`.
            fit = suitable(&*host, pos, rules::towards_player(looking(player.yaw, player.pitch)));
        }
        let Some(new) = fit else {
            return Flow::Cancel;
        };
        if same(&*host, pos, &item, &new) {
            return Flow::Cancel;
        }
        let old = present(&*host, pos);
        host.play_sound(place_sound(&item), util::centre(pos), BLOCKS, 0.75, 1.0);
        set(host, pos, Some((&item, &new)));
        if player.game_mode != util::CREATIVE {
            host.take_items(&item, 1);
            if let Some(old) = old
                && let Some(rest) = host.give_stack(player.uuid, ItemStack::new(&old, 1))
            {
                util::spawn_item(host, util::centre(pos), &rest, [0.0; 3], 10);
            }
        }
        Flow::Cancel
    }

    /// `AbstractSimpleShaftBlock.onRemove`, `FluidPipeBlock.onRemove`: the bracket pops out where the
    /// block was when a player breaks it.
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        let pos = event.pos;
        if !can_have_bracket(&host.block(pos)) {
            return Flow::Continue;
        }
        if let Some(item) = remove(host, pos) {
            util::spawn_item(host, util::centre(pos), &ItemStack::new(&item, 1), [0.0; 3], 10);
        }
        Flow::Continue
    }

    /// The same for an explosion: it breaks the block through its loot table, which runs the drops.
    fn loot_drops(&self, host: &mut dyn Host, event: &mut LootDropsEvent<'_>) {
        let context = event.context;
        let Some((pos, _)) = context.block else {
            return;
        };
        if context.player.is_some() || context.explosion.is_none() {
            return;
        }
        if let Some(item) = present(&*host, pos) {
            event.drops.push(ItemStack::new(&item, 1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bracket_is_saved_as_a_block_state_and_read_back() {
        let bracket = rules::suitable(Some(Axis::Y), Dir::North, Type::Cog).unwrap();
        let mut nbt = Compound::new();
        nbt.insert(KEY.into(), to_tag(WOODEN, &bracket));
        let (name, properties) = from_tag(&nbt).unwrap();
        assert_eq!(name, WOODEN);
        assert!(properties.contains(&("type".to_owned(), "cog".to_owned())));
        assert!(properties.contains(&("facing".to_owned(), "north".to_owned())));
        assert!(properties.contains(&("axis_along_first".to_owned(), "false".to_owned())));
        // Nothing saved, or something that isn't a bracket, reads as none.
        assert_eq!(from_tag(&Compound::new()), None);
        let mut other = Compound::new();
        other.insert(KEY.into(), to_tag("create:shaft", &bracket));
        assert_eq!(from_tag(&other), None);
    }

    #[test]
    fn only_shafts_cogwheels_and_pipes_take_one() {
        for block in [SHAFT, COGWHEEL, LARGE_COGWHEEL, FLUID_PIPE] {
            assert!(can_have_bracket(block));
        }
        assert!(!can_have_bracket("create:gearbox"));
        assert!(!can_have_bracket("create:encased_fluid_pipe"));
        assert!(!can_have_bracket("create:glass_fluid_pipe"));
    }

    #[test]
    fn the_tags_sent_to_clients_always_carry_the_key() {
        let mut tag = Compound::new();
        util::copy_bracket(&mut tag, &Compound::new());
        assert!(matches!(util::get(&tag, KEY), Some(Tag::Compound(c)) if c.is_empty()));
        let bracket = rules::suitable(Some(Axis::X), Dir::Up, Type::Shaft).unwrap();
        let mut nbt = Compound::new();
        nbt.insert(KEY.into(), to_tag(METAL, &bracket));
        let mut tag = Compound::new();
        util::copy_bracket(&mut tag, &nbt);
        assert_eq!(util::get(&tag, KEY), util::get(&nbt, KEY));
    }
}
