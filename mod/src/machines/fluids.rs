//! Create's fluids in the game: what the fluid blocks of this family are to each other
//! (`FluidPropagator.hasFluidCapability`, `FluidPipeBlock.canConnectTo`), how a tank's content is
//! asked for room and drawn, and the fluids' declarations (`tools/content/fluids.py`:
//! `create:honey`, `create:chocolate`, their buckets, names and tags). The tank, the pipes and
//! the pump are the other modules of the family: `fluid_tank`, `fluid_pipe`, `pump`.
//!
//! Known differences from Create:
//! - `create:tea` and `create:potion` have no `Fluid` of their own: Horizon declares fluids that
//!   flow in the world (`data/<id>/horizon/fluid/`) and Create's are virtual, so they exist only
//!   as ids tanks hold (as Horizon's tank API takes any id outside `minecraft`). Potions
//!   (`PotionFluid`) are not carried at all.
//! - Honey and chocolate flow in the world as Horizon's water does, with Create's numbers
//!   (`AllFluids`), and are drawn from one sprite for still and flowing (Create's art is only
//!   imported from the jar for the still one). Their lava interactions (`registerFluidInteractions`)
//!   are not declared.

use create_kinetics::rules::pump::{Cell, Faces};
use create_kinetics::{Axis, Dir};
use crate::compat::{
    BlockEntityDraw, Compound, CubeFace, Direction, Hooks, Host, NativeFluidRenderOptions,
    NativeFluidStack,
};

pub type Pos = [i32; 3];

pub const FLUID_TANK: &str = "create:fluid_tank";
pub const CREATIVE_FLUID_TANK: &str = "create:creative_fluid_tank";
pub const FLUID_PIPE: &str = "create:fluid_pipe";
pub const GLASS_FLUID_PIPE: &str = "create:glass_fluid_pipe";
pub const MECHANICAL_PUMP: &str = "create:mechanical_pump";
pub const FLUID_VALVE: &str = "create:fluid_valve";
pub const SMART_FLUID_PIPE: &str = "create:smart_fluid_pipe";
pub const ENCASED_FLUID_PIPE: &str = "create:encased_fluid_pipe";
pub const SPOUT: &str = "create:spout";
pub const ITEM_DRAIN: &str = "create:item_drain";
/// A Smart Fluid Pipe's saved filter: the id of the fluid its filter item holds, empty for an
/// item that holds none; absent without a filter.
pub const FILTER_FLUID: &str = "FilterFluid";

/// The key of the filter item (`FilteringBehaviour.write`), as `chute_inv::FILTER`.
fn inv_filter() -> &'static str {
    super::chute_inv::FILTER
}

struct Fluids;

/// The module has no behaviour of its own to hook in; the family's blocks do.
pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Fluids)
}

impl Hooks for Fluids {}

/// A room that never runs out, for a block that voids what it is given (`CreativeSmartFluidTank`).
pub const UNLIMITED: i32 = i32::MAX / 4;

pub fn direction(dir: Dir) -> Direction {
    crate::util::direction(dir)
}

/// `FluidTransportBehaviour.canHaveFlowToward` of the block at `pos`, or a fluid handler, or
/// nothing: what the pipes and the pump make of their neighbours.
#[allow(dead_code)] // the pump reads `classify_for`; other modules may ask without a fluid
pub fn classify(host: &dyn Host, pos: Pos) -> Cell {
    classify_for(host, pos, None)
}

/// Whether the block at `pos` is a Smart Fluid Pipe with a filter set.
pub fn is_filtered(host: &dyn Host, pos: Pos) -> bool {
    host.block(pos) == SMART_FLUID_PIPE
        && host
            .mod_block_entity(pos)
            .is_some_and(|e| crate::util::get(&e.nbt, FILTER_FLUID).is_some())
}

/// `IAxisPipe.getAxisOf`: the axis a Glass Fluid Pipe, a Fluid Valve or a Smart Fluid Pipe at `pos`
/// carries fluid along; `None` for anything else.
pub fn pipe_axis(host: &dyn Host, pos: Pos) -> Option<Axis> {
    let property = |key: &str| host.block_property(pos, key);
    match host.block(pos).as_str() {
        GLASS_FLUID_PIPE => property("axis").as_deref().and_then(Axis::from_name),
        FLUID_VALVE => {
            let facing = property("facing").as_deref().and_then(Dir::from_name)?;
            let along_first = property("axis_along_first").as_deref() == Some("true");
            Some(create_kinetics::rules::fluid_pipe::valve_pipe_axis(facing, along_first))
        }
        SMART_FLUID_PIPE => {
            let face = property("face")?;
            let facing = property("facing").as_deref().and_then(Dir::from_name)?;
            Some(create_kinetics::rules::fluid_pipe::smart_pipe_axis(&face, facing))
        }
        _ => None,
    }
}

/// `FluidTransportBehaviour.canHaveFlowToward` of a Fluid Valve, a Smart Fluid Pipe or an Encased
/// Fluid Pipe at `pos`, whether or not it lets fluid through now: the faces its pipe is open on.
pub fn transport_faces(host: &dyn Host, pos: Pos) -> Option<Faces> {
    match host.block(pos).as_str() {
        FLUID_VALVE | SMART_FLUID_PIPE => pipe_axis(host, pos).map(Faces::axis),
        ENCASED_FLUID_PIPE => {
            let open: Vec<Dir> = Dir::ALL
                .into_iter()
                .filter(|d| host.block_property(pos, d.name()).as_deref() == Some("true"))
                .collect();
            Some(Faces::of(&open))
        }
        _ => None,
    }
}

/// [`classify`] for `fluid` passing through: a Smart Fluid Pipe whose filter doesn't take it is
/// nothing (`SmartPipeBehaviour.canPullFluidFrom`); a closed Fluid Valve is nothing for any fluid
/// (`ValvePipeBehaviour.canPullFluidFrom`).
pub fn classify_for(host: &dyn Host, pos: Pos, fluid: Option<&str>) -> Cell {
    let name = host.block(pos);
    match name.as_str() {
        // `FluidValveBlockEntity.ValvePipeBehaviour`: shut until the pointer has turned `enabled`.
        FLUID_VALVE => {
            if host.block_property(pos, "enabled").as_deref() != Some("true") {
                return Cell::Nothing;
            }
            transport_faces(host, pos).map_or(Cell::Nothing, Cell::Pipe)
        }
        // `SmartFluidPipeBlockEntity.SmartPipeBehaviour`: only the fluid its filter takes.
        SMART_FLUID_PIPE => {
            if let Some(fluid) = fluid
                && let Some(entity) = host.mod_block_entity(pos)
                && let Some(filter) = crate::util::get(&entity.nbt, FILTER_FLUID)
            {
                // A Filter item in the filter slot asks its own rules (`filter.rs`).
                let filter_item = crate::util::get(&entity.nbt, inv_filter())
                    .and_then(crate::util::stack_of)
                    .filter(|item| super::filter::is_filter_item(&item.item));
                let passes = match filter_item {
                    Some(item) => super::filter::matches_fluid(&item, fluid),
                    None => matches!(filter, crate::compat::Tag::String(held)
                        if held.to_string_lossy() == fluid),
                };
                if !passes {
                    return Cell::Nothing;
                }
            }
            transport_faces(host, pos).map_or(Cell::Nothing, Cell::Pipe)
        }
        // `FluidPipeBlockEntity.PipeFluidTransferBehaviour` for the encased pipe: the same six
        // states as a pipe.
        ENCASED_FLUID_PIPE => transport_faces(host, pos).map_or(Cell::Nothing, Cell::Pipe),
        // `registerCapabilities`: the Spout's tank is closed below, the drain's above.
        SPOUT | ITEM_DRAIN => Cell::Handler,
        FLUID_PIPE => {
            let open: Vec<Dir> = Dir::ALL
                .into_iter()
                .filter(|d| host.block_property(pos, d.name()).as_deref() == Some("true"))
                .collect();
            Cell::Pipe(Faces::of(&open))
        }
        GLASS_FLUID_PIPE => match host.block_property(pos, "axis").as_deref().and_then(Axis::from_name) {
            Some(axis) => Cell::Pipe(Faces::axis(axis)),
            None => Cell::Nothing,
        },
        MECHANICAL_PUMP => match host.block_property(pos, "facing").as_deref().and_then(Dir::from_name) {
            Some(facing) => Cell::Pump(facing),
            None => Cell::Nothing,
        },
        "minecraft:air" => Cell::Nothing,
        _ => {
            // `hasFluidCapability`: any block holding tanks (ours, other mods', a cauldron).
            if host.fluid_tanks(host.dimension(), pos, Direction::Up).is_some() {
                Cell::Handler
            } else {
                Cell::Nothing
            }
        }
    }
}

/// How much of `fluid` the block at `pos` has room for through `face`: its fillable tanks
/// holding it or none. A creative tank has no end.
pub fn room(host: &mut dyn Host, pos: Pos, face: Direction, fluid: &str) -> i32 {
    if host.block(pos) == CREATIVE_FLUID_TANK {
        return UNLIMITED;
    }
    let dimension = host.dimension();
    host.fluid_tanks_mut(dimension, pos, face)
        .map(|tanks| {
            tanks
                .iter()
                .filter(|t| t.can_fill && t.fluid.as_deref().is_none_or(|f| f == fluid))
                .map(|t| (t.capacity - t.amount).max(0))
                .sum()
        })
        .unwrap_or(0)
}

/// The fluid the block at `pos` gives through `face` and how much: its first drainable tank.
pub fn contents(host: &mut dyn Host, pos: Pos, face: Direction) -> Option<(String, i32)> {
    let dimension = host.dimension();
    let tanks = host.fluid_tanks_mut(dimension, pos, face)?;
    let first = tanks.iter().find(|t| t.can_drain && t.fluid.is_some() && t.amount > 0)?;
    let fluid = first.fluid.clone()?;
    let total = tanks
        .iter()
        .filter(|t| t.can_drain && t.fluid.as_deref() == Some(fluid.as_str()))
        .map(|t| t.amount)
        .sum();
    Some((fluid, total))
}

/// Draws `fluid` as a box `bounds` (`[x0, y0, z0, x1, y1, z1]` in blocks) under the draw's pose:
/// Horizon's own water and lava, and any other fluid as a white box tinted with its colour.
pub fn draw_fluid(
    host: &dyn Host,
    draw: &mut BlockEntityDraw,
    fluid: &str,
    amount: i32,
    bounds: [f32; 6],
    render_bottom: bool,
) {
    // A potion fluid (`rules/potion_fluid.rs`) is water in the potion's colour, as Create's `PotionFluidType` tints it.
    let potion = create_kinetics::rules::potion_fluid::parse(fluid);
    let colour = potion.as_ref().map(|p| crate::compat::brewing::potion_color(p.potion.as_deref()));
    let stack = NativeFluidStack {
        fluid: if potion.is_some() { "minecraft:water" } else { fluid }.to_owned(),
        amount,
        components: Compound::new(),
    };
    let options = NativeFluidRenderOptions {
        argb: colour.map(|rgb| 0xFF00_0000 | rgb),
        render_bottom,
        uv_shrink_ratio: 4.0 / 1024.0,
    };
    if host.draw_fluid_box(draw, &stack, bounds, options) {
        return;
    }
    let face = Some(CubeFace {
        sprite: "block/white_concrete",
        uv: None,
    });
    let [x0, y0, z0, x1, y1, z1] = bounds.map(|v| v * 16.0);
    host.draw_cube(
        draw,
        [x0, y0, z0],
        [x1, y1, z1],
        [face; 6],
        colour.unwrap_or_else(|| create_kinetics::rules::fluids::tint(fluid)),
    );
}
