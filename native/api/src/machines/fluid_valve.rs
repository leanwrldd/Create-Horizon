//! The Fluid Valve (`create:fluid_valve`; `FluidValveBlock`, `FluidValveBlockEntity`,
//! `FluidValveRenderer`): a pipe with a shaft across it. Turned one way by its shaft its pointer
//! swings and, once it has reached the end, the block's `enabled` state opens it; turned the other
//! way, it shuts. A shut valve lets no fluid through (the pump's network treats it as no pipe,
//! `fluids::classify_for`). The numbers are `create_kinetics::rules::fluid_valve`.
//!
//! Known differences from Create: see `rules/fluid_valve.rs`. Also:
//! - A pump re-reads the valve's state every second (`pump.rs`), where Create's valve tells the
//!   network the moment it changes (`FluidPropagator.propagateChangedPipe`).
//! - The placement doesn't line the valve up with a shaft or a pipe beside it
//!   (`DirectionalAxisKineticBlock.getStateForPlacement`, `prefersConnectionTo`: the placement
//!   hook can't read neighbours), no waterlogging, no bracket.
//! - The pointer's turn on the client is read from the last sent `Pointer`, sent each tick.
//! - Handles are in `fluid_valve_handle.rs`, the Encased Fluid Pipe in `smart_fluid_pipe_encased.rs`.

use create_kinetics::rules::fluid_pipe::{valve_pipe_axis, valve_rotation_axis};
use create_kinetics::rules::fluid_valve as rules;
use create_kinetics::{Axis, Dir, Kind, Node, looking};
use horizon_client::mod_api::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, BlockStateId, Compound, Flow, Hooks, Host, LevelHost,
    StandaloneModelsEvent, StateForPlacementEvent, UseItemOnEvent,
};

use super::fluids::FLUID_VALVE;
use crate::{float, kinetic, set_float, util};

/// `AllPartialModels.FLUID_VALVE_POINTER`.
const POINTER: &str = "create:block/fluid_valve/pointer";
/// `FluidValveBlockEntity.write`: `pointer.writeNBT()`, here the value alone.
const POINTER_KEY: &str = "Pointer";
const CENTRE: [f32; 3] = [0.5; 3];

struct Valves;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Valves)
}

fn bool_name(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

/// `(facing, axis_along_first)` of the valve at `pos`.
fn state_of(host: &dyn Host, pos: [i32; 3]) -> Option<(Dir, bool)> {
    let facing = util::facing(host, pos, "facing")?;
    let along_first = host.block_property(pos, "axis_along_first").as_deref() == Some("true");
    Some((facing, along_first))
}

/// `AngleHelper.horizontalAngle`: south 0, east 90, north 180, west 270; 0 for up and down.
fn horizontal_angle(facing: Dir) -> f32 {
    match facing {
        Dir::East => 90.0,
        Dir::North => 180.0,
        Dir::West => 270.0,
        _ => 0.0,
    }
}

impl Hooks for Valves {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.push(POINTER.to_owned());
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == FLUID_VALVE).then(|| {
            let mut nbt = kinetic::new_entity();
            set_float(&mut nbt, POINTER_KEY, 0.0);
            (FLUID_VALVE.to_owned(), nbt)
        })
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == FLUID_VALVE {
            event.ticks = true;
        }
    }

    /// `FluidValveBlockEntity.tick`: the pointer chases the speed's sign, and the block's `enabled`
    /// state follows it once it has reached an end.
    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != FLUID_VALVE {
            return;
        }
        let pos = event.pos;
        let Some((facing, along_first)) = state_of(&*host, pos) else {
            return;
        };
        let faces = rules::shaft_faces(facing, along_first);
        // `AllBlocks.FLUID_VALVE` has no stress impact (`BlockStressValues.getImpact` is 0).
        let node = Node::passive(Kind::shafts(&faces));
        let Some((rpm, mut changed)) = kinetic::tick(host, pos, event.entity, node) else {
            return;
        };
        let pointer = float(&event.entity.nbt, POINTER_KEY).unwrap_or(0.0);
        let next = rules::pointer_step(pointer, rpm);
        if next != pointer {
            set_float(&mut event.entity.nbt, POINTER_KEY, next);
            changed = true;
        }
        let enabled = host.block_property(pos, "enabled").as_deref() == Some("true");
        if let Some(open) = rules::switch_enabled(next, enabled) {
            host.set_block_property(pos, "enabled", bool_name(open));
        }
        if changed {
            host.block_entity_sync(pos);
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        if event.entity.id != FLUID_VALVE {
            return None;
        }
        let mut tag = kinetic::update_tag(event.entity);
        set_float(&mut tag, POINTER_KEY, float(&event.entity.nbt, POINTER_KEY).unwrap_or(0.0));
        Some(tag)
    }

    /// `FluidValveRenderer`: the shaft through the valve and the pointer, turned a quarter as the
    /// valve opens. The casing is in the chunk mesh.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != FLUID_VALVE {
            return;
        }
        let property = |key: &str| host.block_state_property(event.state, key);
        let Some(facing) = property("facing").and_then(Dir::from_name) else {
            return;
        };
        let along_first = property("axis_along_first") == Some("true");
        let shaft = valve_rotation_axis(facing, along_first);
        let pipe = valve_pipe_axis(facing, along_first);
        let rpm = kinetic::speed(event.entity);
        let draw = &mut *event.draw;
        kinetic::draw_shaft(host, draw, shaft, rpm);

        let pointer = float(&event.entity.nbt, POINTER_KEY).unwrap_or(0.0);
        let rotation = -90.0 * pointer;
        let x_rot = match facing {
            Dir::Up => 0.0,
            Dir::Down => 180.0,
            _ => 90.0,
        };
        // `pipeAxis.isHorizontal() && shaftAxis == X || pipeAxis.isVertical()`.
        let twist = (pipe != Axis::Y && shaft == Axis::X) || pipe == Axis::Y;
        let offset = if twist { 90.0 } else { 0.0 };
        draw.push();
        draw.rotate_about([0.0, 1.0, 0.0], horizontal_angle(facing), CENTRE);
        draw.rotate_about([1.0, 0.0, 0.0], x_rot, CENTRE);
        draw.rotate_about([0.0, 1.0, 0.0], offset + rotation, CENTRE);
        host.draw_model(draw, POINTER, 0xFF_FFFF);
        draw.pop();
        draw.keep();
    }

    /// `DirectionalAxisKineticBlock.getStateForPlacement`, without the shaft beside it Create
    /// lines the valve up with.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != FLUID_VALVE {
            return None;
        }
        let look = looking(context.yaw, context.pitch);
        let horizontal = rules::horizontal_direction(context.yaw);
        let (facing, along_first) = rules::placement(look, horizontal, context.sneaking);
        let state = level.state_with_property(event.state, "facing", facing.name())?;
        level.state_with_property(state, "axis_along_first", bool_name(along_first)).map(Some)
    }

    /// The Wrench turns the valve round the face clicked (`IWrenchable.getRotatedBlockState`).
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != FLUID_VALVE {
            return Flow::Continue;
        }
        let (pos, clicked) = (event.pos, Dir::from_normal(event.face));
        util::wrench(host, pos, |host| {
            let (Some((facing, along_first)), Some(clicked)) = (state_of(&*host, pos), clicked) else {
                return;
            };
            let (next_facing, next_along) = rules::wrench(facing, along_first, clicked);
            if next_facing != facing {
                host.set_block_property(pos, "facing", next_facing.name());
            }
            if next_along != along_first {
                host.set_block_property(pos, "axis_along_first", bool_name(next_along));
            }
        })
    }
}
