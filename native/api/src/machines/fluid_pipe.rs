//! The Fluid Pipe and the Glass Fluid Pipe (`FluidPipeBlock`, `FluidPipeBlockEntity`,
//! `AxisPipeBlock`, `GlassFluidPipeBlock`): six `down` .. `east` states saying which sides are
//! open, that follow the pipe's neighbours. The sides' rules are
//! `create_kinetics::rules::fluid_pipe`; this is the glue. A pipe holds no fluid: the Mechanical
//! Pump (`pump.rs`) works out what its pipes reach.
//!
//! Known differences from Create: see `rules/fluid_pipe.rs`. Also, a pipe block entity only
//! refreshes its sides every ten ticks (and at once when a pipe is placed), where Create reacts
//! to each neighbour's update; the encased pipe (`EncasedPipeBlock`), brackets and waterlogging
//! are not done.

use create_kinetics::rules::fluid_pipe::{self as rules, Sides};
use create_kinetics::{Dir, looking};
use horizon_client::mod_api::{
    BlockEntityEvent, BlockEntityTickEvent, BlockEntityTicksEvent, BlockStateId, Compound, Flow,
    Hooks, Host, LevelHost, PlaceBlockEvent, StateForPlacementEvent, UseItemOnEvent,
};

use super::fluids::{FLUID_PIPE, GLASS_FLUID_PIPE, MECHANICAL_PUMP, direction};
use crate::util;

type Pos = [i32; 3];

/// How often a pipe looks at its neighbours.
const REFRESH: i64 = 10;

struct Pipes;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Pipes)
}

fn add(pos: Pos, dir: Dir) -> Pos {
    util::add(pos, dir.normal())
}

fn sides_of(host: &dyn Host, pos: Pos) -> Sides {
    Dir::ALL.map(|d| host.block_property(pos, d.name()).as_deref() == Some("true"))
}

/// `FluidPipeBlock.canConnectTo`: whether the pipe at `pos` joins what is beside it on `d`.
fn connects(host: &dyn Host, pos: Pos, d: Dir) -> bool {
    let beside = add(pos, d);
    match host.block(beside).as_str() {
        // `isPipe(neighbour)`.
        FLUID_PIPE => true,
        // An `AxisPipeBlock` has its ends on its axis only (`canHaveFlowToward`).
        GLASS_FLUID_PIPE => host.block_property(beside, "axis").as_deref() == Some(d.axis().name()),
        // `PumpFluidTransferBehaviour.canHaveFlowToward`: the pump's axis.
        MECHANICAL_PUMP => {
            host.block_property(beside, "facing").as_deref().and_then(Dir::from_name).map(Dir::axis)
                == Some(d.axis())
        }
        "minecraft:air" => false,
        // `hasFluidCapability`: any block holding tanks.
        _ => host.fluid_tanks(host.dimension(), beside, direction(d.opposite())).is_some(),
    }
}

/// `FluidPipeBlock.updateBlockState` for the pipe at `pos`, written to its block state.
fn refresh(host: &mut dyn Host, pos: Pos) {
    if host.block(pos) != FLUID_PIPE {
        return;
    }
    let prev = sides_of(&*host, pos);
    let preferred = Dir::ALL.into_iter().find(|d| prev[*d as usize]).unwrap_or(Dir::North);
    let next = {
        let host: &dyn Host = &*host;
        rules::update_sides(&prev, preferred, None, |d| connects(host, pos, d))
    };
    for d in Dir::ALL {
        if next[d as usize] != prev[d as usize] {
            host.set_block_property(pos, d.name(), if next[d as usize] { "true" } else { "false" });
        }
    }
}

/// The pipes beside `pos`, and `pos` itself when it is one.
fn refresh_around(host: &mut dyn Host, pos: Pos) {
    refresh(host, pos);
    for d in Dir::ALL {
        refresh(host, add(pos, d));
    }
}

impl Hooks for Pipes {
    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == FLUID_PIPE).then(|| (FLUID_PIPE.to_owned(), Compound::new()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == FLUID_PIPE {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != FLUID_PIPE {
            return;
        }
        let pos = event.pos;
        let salt = i64::from(pos[0]) + 3 * i64::from(pos[1]) + 7 * i64::from(pos[2]);
        if (util::game_time(host) + salt).rem_euclid(REFRESH) == 0 {
            refresh(host, pos);
        }
    }

    /// `FluidPipeBlock.getStateForPlacement`: a pipe with nothing beside it runs along the way
    /// the player looks (`getNearestLookingDirection`); the neighbours' own say the rest once
    /// it is placed (`place_block`).
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        if &*event.context.item != FLUID_PIPE {
            return None;
        }
        let look = looking(event.context.yaw, event.context.pitch);
        let state = level.state_with_property(event.state, look.name(), "true")?;
        let state = level.state_with_property(state, look.opposite().name(), "true")?;
        Some(Some(state))
    }

    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if &*event.stack.item == FLUID_PIPE {
            refresh_around(host, event.pos);
        }
    }

    /// The Wrench: sneak-click picks the pipe up; a click turns a pipe into a glass one along an
    /// axis (`FluidPipeBlock.onWrenched`) and a glass one back into a pipe
    /// (`GlassFluidPipeBlock.onWrenched`).
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH {
            return Flow::Continue;
        }
        let pos = event.pos;
        let block = host.block(pos);
        if block != FLUID_PIPE && block != GLASS_FLUID_PIPE {
            return Flow::Continue;
        }
        let Some(clicked) = Dir::from_normal(event.face) else {
            return Flow::Cancel;
        };
        util::wrench(host, pos, |host| {
            let dimension = host.dimension();
            if block == FLUID_PIPE {
                let sides = sides_of(&*host, pos);
                let Some(axis) = rules::glass_axis(&sides, clicked) else {
                    return;
                };
                if let Some(state) = host.default_state(GLASS_FLUID_PIPE) {
                    host.set_block(dimension, pos, state, util::UPDATE_ALL);
                    host.set_block_property(pos, "axis", axis.name());
                }
            } else {
                let Some(axis) =
                    host.block_property(pos, "axis").as_deref().and_then(create_kinetics::Axis::from_name)
                else {
                    return;
                };
                // `toRegularPipe`: both ends on the axis.
                if let Some(state) = host.default_state(FLUID_PIPE) {
                    host.set_block(dimension, pos, state, util::UPDATE_ALL);
                    for d in Dir::ALL {
                        if d.axis() == axis {
                            host.set_block_property(pos, d.name(), "true");
                        }
                    }
                }
            }
            refresh_around(host, pos);
        })
    }
}
