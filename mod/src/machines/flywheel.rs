//! The Flywheel in the game (`FlywheelBlock`, `FlywheelBlockEntity`, `FlywheelRenderer`): a brass
//! wheel on a shaft axis that turns with the network and, being heavy, takes a while to speed up
//! and slow down. It has no stress impact (`CStress.setNoImpact`).
//!
//! The numbers are `create_kinetics::rules::flywheel`; its header lists the known differences.
//! The server keeps the wheel's visual state where it stood when the speed last changed
//! (`Wheel`: speed, angle, game time, target) and tells clients; every viewer works the turn out from
//! the game time, so nothing is written between changes. A steady wheel is drawn with `spin` and
//! kept, as any shaft; one still speeding up or slowing down is redrawn every frame.
//!
//! Known differences from Create, for the README (and the rules' header):
//! - The wheel is checked once a tick on the server rather than chasing on each client.
//! - The Wrench turns a Flywheel about the face clicked as it does any pillar; placement lines it up
//!   with a shaft beside it as the other axis blocks do (`encased.rs`).

use create_kinetics::rules::flywheel::Wheel;
use create_kinetics::{Axis, Dir, Node, Kind};
use crate::compat::{
    BlockEntityEvent, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent, BlockEntityTickEvent,
    BlockEntityTicksEvent, BlockEntityUpdateTagEvent, BlockStateId, Compound, Flow, Hooks, Host,
    LevelHost, PlaceBlockEvent, StandaloneModelsEvent, StateForPlacementEvent, Tag, UseItemOnEvent,
};

use super::encased;
use crate::{kinetic, util};

pub const ID: &str = "create:flywheel";
/// `AllPartialModels`-style: the wheel without its shaft, Create's `.obj`.
const WHEEL_MODEL: &str = "create:block/flywheel/block";

const WHEEL_SPEED: &str = "WheelSpeed";
const WHEEL_ANGLE: &str = "WheelAngle";
const WHEEL_SINCE: &str = "WheelSince";
const WHEEL_TARGET: &str = "WheelTarget";

const CENTRE: [f32; 3] = [0.5; 3];

struct Flywheel;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Flywheel)
}

fn read(nbt: &Compound) -> Wheel {
    Wheel {
        speed: util::float(nbt, WHEEL_SPEED).unwrap_or(0.0),
        angle: util::float(nbt, WHEEL_ANGLE).unwrap_or(0.0),
        since: util::long(nbt, WHEEL_SINCE),
        target: util::float(nbt, WHEEL_TARGET).unwrap_or(0.0),
    }
}

fn write(nbt: &mut Compound, wheel: Wheel) {
    util::set_float(nbt, WHEEL_SPEED, wheel.speed);
    util::set_float(nbt, WHEEL_ANGLE, wheel.angle);
    nbt.insert(WHEEL_SINCE.into(), Tag::Long(wheel.since));
    util::set_float(nbt, WHEEL_TARGET, wheel.target);
}

impl Hooks for Flywheel {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.push(WHEEL_MODEL.to_owned());
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == ID).then(|| (ID.to_owned(), kinetic::new_entity()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    /// `FlywheelBlockEntity.createRenderBoundingBox`: inflated by 2 blocks.
    fn block_entity_render_bounding_box(
        &self,
        _: &dyn Host,
        event: &mut BlockEntityRenderBoundingBoxEvent<'_>,
    ) {
        if event.block == ID {
            *event.bounds = [-2.0, -2.0, -2.0, 3.0, 3.0, 3.0];
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        let pos = event.pos;
        let Some(axis) = host
            .block_property(pos, "axis")
            .as_deref()
            .and_then(Axis::from_name)
        else {
            return;
        };
        // `CStress.setNoImpact`: a shaft with a wheel on it.
        let Some((speed, mut changed)) =
            kinetic::tick(host, pos, event.entity, Node::passive(Kind::Shaft(axis)))
        else {
            return;
        };
        // The wheel chases the speed (`visualSpeed.updateChaseTarget(getSpeed())`): when the target
        // changes, where it stood is saved, and it chases from there.
        let wheel = read(&event.entity.nbt);
        if wheel.target != speed {
            write(&mut event.entity.nbt, wheel.retarget(util::game_time(host), speed));
            changed = true;
        }
        if changed {
            host.block_entity_sync(pos);
        }
    }

    fn block_entity_update_tag(
        &self,
        event: &mut BlockEntityUpdateTagEvent<'_>,
    ) -> Option<Compound> {
        if event.entity.id != ID {
            return None;
        }
        let mut tag = kinetic::update_tag(event.entity);
        write(&mut tag, read(&event.entity.nbt));
        Some(tag)
    }

    /// `FlywheelRenderer`: the shaft at the network's speed, and the wheel at its own.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let Some(axis) = host
            .block_state_property(event.state, "axis")
            .and_then(Axis::from_name)
        else {
            return;
        };
        let rpm = kinetic::speed(event.entity);
        encased::draw_shaft(host, &mut *event.draw, axis, event.pos, rpm);

        let wheel = read(&event.entity.nbt);
        let now = event.draw.game_time;
        let draw = &mut *event.draw;
        let settled = wheel.is_settled(now);
        if settled {
            // At speed: an ordinary steady turn.
            let (degrees, phase) = wheel.steady();
            draw.spin(axis.unit(), CENTRE, degrees, phase);
        } else {
            // Speeding up or down: the angle is worked out for this frame.
            let (_, angle) = wheel.frame(now, draw.partial);
            draw.rotate_about(axis.unit(), angle, CENTRE);
        }
        draw.push();
        // The model is drawn for the y axis; the blockstate turns it for the others
        // (`blockstates/flywheel.json`: x is x 90 y 90, z is x 90 y 180).
        let (x, y) = match axis {
            Axis::Y => (0.0, 0.0),
            Axis::X => (90.0, 90.0),
            Axis::Z => (90.0, 180.0),
        };
        if y != 0.0 {
            draw.rotate_about(Axis::Y.unit(), -y, CENTRE);
        }
        if x != 0.0 {
            draw.rotate_about(Axis::X.unit(), -x, CENTRE);
        }
        host.draw_model(draw, WHEEL_MODEL, 0xFF_FFFF);
        draw.pop();
        if settled {
            draw.still();
            draw.keep();
        }
    }

    /// `RotatedPillarKineticBlock.getStateForPlacement`: along the axis the player looks.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        encased::placement_state(level, event, &[ID])
    }

    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        encased::align_axis(host, event, &[ID]);
    }

    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != ID {
            return Flow::Continue;
        }
        let Some(face) = Dir::from_normal(event.face) else {
            return Flow::Continue;
        };
        if host
            .acting_player()
            .is_some_and(|player| !util::first_click(host, player.uuid, event.pos))
        {
            return Flow::Cancel;
        }
        // `IWrenchable`: sneaking picks it up, otherwise a quarter turn about the face's axis.
        util::wrench(host, event.pos, |host| {
            encased::rotate_block(host, event.pos, face);
        })
    }
}
