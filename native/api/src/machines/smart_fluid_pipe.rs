//! The Smart Fluid Pipe (`create:smart_fluid_pipe`; `SmartFluidPipeBlock`,
//! `SmartFluidPipeBlockEntity`): a straight pipe with a fluid filter, that lets only the fluid its
//! filter item holds pass. The pump's network reads the filter (`fluids::classify_for`); the
//! placement and the filter test are `create_kinetics::rules::smart_fluid_pipe`.
//!
//! Known differences from Create: see `rules/smart_fluid_pipe.rs`. Also:
//! - The filter is set as the Brass Funnel's and the Smart Chute's is (`chute_inv`), not in Create's
//!   filter slot and value box: sneak-right-click with an item sets the filter to it, with the same
//!   item again clears it, an empty hand clears it. The item shows no fluid slot on the pipe. Its
//!   fluid is what `GenericItemEmptying` finds in it (buckets, the honey bottle, Builder's Tea,
//!   a bottle of water, an emptying recipe); an item that holds none lets nothing through.
//! - A pump re-reads the filter every second (`pump.rs`), where Create's pipe tells the network as
//!   soon as the filter changes (`onFilterChanged`: `FluidPropagator.propagateChangedPipe`).
//! - No waterlogging, no bracket, and the shape drops the nozzle's top box that rises past the
//!   block (`AllShapes.SMART_FLUID_PIPE_*`: boxes reaching 17/16).
//! - The Wrench turns the pipe about the top or bottom face only
//!   (`IWrenchable.getRotatedBlockState`: it has a horizontal `facing`).
//! - The Encased Fluid Pipe is in `smart_fluid_pipe_encased.rs`.

use create_kinetics::rules::chute::Amount;
use create_kinetics::rules::smart_fluid_pipe::{self as rules, Face};
use create_kinetics::{Axis, Dir};
use horizon_client::mod_api::{
    BlockEntityEvent, BlockStateId, Compound, EquipmentSlot, Flow, Hooks, Host, ItemStack, LevelHost,
    PlaceBlockEvent, RightClickBlockEvent, StateForPlacementEvent, Tag, UseItemOnEvent,
};

use super::chute_inv::{self as inv, Filter};
use super::fluids::{FILTER_FLUID, SMART_FLUID_PIPE};
use super::{fluid_pipe, spout_fill};
use crate::util;

struct SmartPipes;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(SmartPipes)
}

/// `Direction.getClockWise()`: a quarter turn clockwise seen from above.
fn clockwise(facing: Dir) -> Dir {
    match facing {
        Dir::North => Dir::East,
        Dir::East => Dir::South,
        Dir::South => Dir::West,
        Dir::West => Dir::North,
        other => other,
    }
}

/// What a filter click does to the filter of the pipe at `pos`: `hand` sets it, the same item again
/// or an empty hand clears it. Answers the line to tell the player.
fn click(host: &mut dyn Host, pos: [i32; 3], hand: Option<&ItemStack>) -> Option<String> {
    let entity = host.mod_block_entity(pos).filter(|e| e.id == SMART_FLUID_PIPE)?;
    let before = inv::filter_of(&entity.nbt).item;
    let after = match hand {
        Some(stack) => {
            let mut one = stack.clone();
            one.count = 1;
            // The same item again clears it.
            (!before.as_ref().is_some_and(|have| util::same_item(have, &one))).then_some(one)
        }
        None => None,
    };
    // `GenericItemEmptying` of the filter: the fluid it stands for, none for an item that holds none.
    let fluid: Option<Option<String>> = after
        .as_ref()
        .map(|stack| spout_fill::empty_item(&*host, stack).map(|emptied| emptied.fluid));
    let line = match (&after, &fluid) {
        (None, _) => "Fluid filter cleared".to_owned(),
        (Some(stack), Some(Some(fluid))) => format!("Fluid filter: {} ({fluid})", stack.item),
        (Some(stack), _) => format!("Fluid filter: {} (holds no fluid: nothing passes)", stack.item),
    };
    if after != before {
        host.change_mod_block_entity(pos, &mut |entity| {
            inv::write_filter(&mut entity.nbt, &Filter { item: after.clone(), amount: Amount::default() });
            match &fluid {
                Some(fluid) => {
                    let tag = Tag::String(fluid.as_deref().unwrap_or("").into());
                    entity.nbt.insert(FILTER_FLUID.into(), tag);
                }
                None => {
                    util::remove(&mut entity.nbt, FILTER_FLUID);
                }
            }
        });
        host.block_entity_sync(pos);
    }
    Some(line)
}

impl Hooks for SmartPipes {
    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == SMART_FLUID_PIPE).then(|| (SMART_FLUID_PIPE.to_owned(), Compound::new()))
    }

    /// `FaceAttachedHorizontalDirectionalBlock.getStateForPlacement` (the first direction the
    /// player looks along, `canSurvive` being true); `place_block` then lines the pipe up with its
    /// neighbours.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != SMART_FLUID_PIPE {
            return None;
        }
        let order = rules::nearest_looking(context.yaw, context.pitch);
        let (face, facing) = rules::placement(&order, rules::horizontal_direction(context.yaw));
        let state = level.state_with_property(event.state, "face", face.name())?;
        level.state_with_property(state, "facing", facing.name()).map(Some)
    }

    /// `SmartFluidPipeBlock.getStateForPlacement`'s search for a neighbouring pipe, tank or pump.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if &*event.stack.item != SMART_FLUID_PIPE {
            return;
        }
        let pos = event.pos;
        let Some(player) = host.acting_player() else {
            return;
        };
        let face = host.block_property(pos, "face").as_deref().and_then(Face::from_name);
        let facing = util::facing(&*host, pos, "facing");
        let (Some(face), Some(facing)) = (face, facing) else {
            return;
        };
        let order = rules::nearest_looking(player.yaw, player.pitch);
        let preferred: Option<Axis> = {
            let host: &dyn Host = &*host;
            rules::preferred_axis(|side| fluid_pipe::connects(host, pos, side))
        };
        let (next_face, next_facing) = rules::adjust(face, facing, preferred, &order);
        if next_face != face {
            host.set_block_property(pos, "face", next_face.name());
        }
        if next_facing != facing {
            host.set_block_property(pos, "facing", next_facing.name());
        }
    }

    /// The Wrench (`IWrenchable.getRotatedBlockState`: a click on the top or bottom turns the
    /// pipe's `facing` clockwise) and, sneaking with any other item, the filter.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        if host.block(pos) != SMART_FLUID_PIPE {
            return Flow::Continue;
        }
        if &*event.stack.item == util::WRENCH {
            let clicked = Dir::from_normal(event.face);
            return util::wrench(host, pos, |host| {
                if clicked.is_some_and(|c| c.axis() == Axis::Y)
                    && let Some(facing) = util::facing(&*host, pos, "facing")
                {
                    host.set_block_property(pos, "facing", clockwise(facing).name());
                }
            });
        }
        if !host.acting_player().is_some_and(|p| p.sneaking) {
            return Flow::Continue;
        }
        if let Some(line) = click(host, pos, Some(event.stack)) {
            inv::tell(host, &line);
        }
        Flow::Cancel
    }

    /// An empty hand sneaking clears the filter.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != SMART_FLUID_PIPE {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if !player.sneaking || player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        if host.player_equipment(player.uuid, EquipmentSlot::MainHand).is_some() {
            return Flow::Continue;
        }
        if let Some(line) = click(host, pos, None) {
            inv::tell(host, &line);
        }
        Flow::Cancel
    }
}
