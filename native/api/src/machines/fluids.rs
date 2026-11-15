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
use horizon_client::mod_api::{
    BlockEntityDraw, Compound, CubeFace, Direction, Hooks, Host, NativeFluidRenderOptions,
    NativeFluidStack,
};

pub type Pos = [i32; 3];

pub const FLUID_TANK: &str = "create:fluid_tank";
pub const CREATIVE_FLUID_TANK: &str = "create:creative_fluid_tank";
pub const FLUID_PIPE: &str = "create:fluid_pipe";
pub const GLASS_FLUID_PIPE: &str = "create:glass_fluid_pipe";
pub const MECHANICAL_PUMP: &str = "create:mechanical_pump";

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
pub fn classify(host: &dyn Host, pos: Pos) -> Cell {
    let name = host.block(pos);
    match name.as_str() {
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
pub fn room(host: &dyn Host, pos: Pos, face: Direction, fluid: &str) -> i32 {
    if host.block(pos) == CREATIVE_FLUID_TANK {
        return UNLIMITED;
    }
    host.fluid_tanks(host.dimension(), pos, face)
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
pub fn contents(host: &dyn Host, pos: Pos, face: Direction) -> Option<(String, i32)> {
    let tanks = host.fluid_tanks(host.dimension(), pos, face)?;
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
    let stack = NativeFluidStack {
        fluid: fluid.to_owned(),
        amount,
        components: Compound::new(),
    };
    let options = NativeFluidRenderOptions {
        argb: None,
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
        create_kinetics::rules::fluids::tint(fluid),
    );
}
