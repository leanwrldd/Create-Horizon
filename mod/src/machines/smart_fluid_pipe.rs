//! The Smart Fluid Pipe (`create:smart_fluid_pipe`; `SmartFluidPipeBlock`,
//! `SmartFluidPipeBlockEntity`): a straight pipe with a fluid filter, that lets only the fluid its
//! filter item holds pass. The pump's network reads the filter (`fluids::classify_for`); the
//! placement and the filter test are `create_kinetics::rules::smart_fluid_pipe`.
//!
//! Known differences from Create: see `rules/smart_fluid_pipe.rs`. Also:
//! - The filter is Create's filter slot (a value box, `machines/value_box.rs`; the item is drawn on the pipe,
//!   not a fluid). Its fluid is what `GenericItemEmptying` finds in it (buckets, the honey bottle, Builder's Tea,
//!   a bottle of water, an emptying recipe); an item that holds none lets nothing through.
//! - A pump re-reads the filter every second (`pump.rs`), where Create's pipe tells the network as
//!   soon as the filter changes (`onFilterChanged`: `FluidPropagator.propagateChangedPipe`).
//! - No waterlogging, no bracket, and the shape drops the nozzle's top box that rises past the
//!   block (`AllShapes.SMART_FLUID_PIPE_*`: boxes reaching 17/16).
//! - The Wrench turns the pipe about the top or bottom face only
//!   (`IWrenchable.getRotatedBlockState`: it has a horizontal `facing`).
//! - The Encased Fluid Pipe is in `smart_fluid_pipe_encased.rs`.

use create_kinetics::rules::smart_fluid_pipe::{self as rules, Face};
use create_kinetics::{Axis, Dir};
use crate::compat::{
    BlockEntityEvent, BlockStateId, Compound, Flow, Hooks, Host, ItemStack, LevelHost,
    PlaceBlockEvent, StateForPlacementEvent, Tag, UseItemOnEvent,
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

/// Sets the filter of the pipe at `pos` (`FilteringBehaviour.setFilter`, from the filter slot's value box,
/// `value_box.rs`): `after` is the item, `None` clears it. Also saves the fluid it stands for.
pub(super) fn set_filter(host: &mut dyn Host, pos: [i32; 3], after: Option<ItemStack>) {
    let Some(entity) = host.mod_block_entity(pos).filter(|e| e.id == SMART_FLUID_PIPE) else {
        return;
    };
    let before = inv::filter_of(&entity.nbt).item;
    // `GenericItemEmptying` of the filter: the fluid it stands for, none for an item that holds none.
    // A Filter item stands for the fluids its items hold (`filter.rs`): the pipe asks it per fluid.
    let fluid: Option<Option<String>> = after.as_ref().map(|stack| {
        if super::filter::is_filter_item(&stack.item) {
            Some(String::new())
        } else {
            spout_fill::empty_item(&*host, stack).map(|emptied| emptied.fluid)
        }
    });
    if after != before {
        host.change_mod_block_entity(pos, &mut |entity| {
            let amount = inv::filter_of(&entity.nbt).amount;
            inv::write_filter(&mut entity.nbt, &Filter { item: after.clone(), amount });
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
    /// pipe's `facing` clockwise).
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
        Flow::Continue
    }
}
