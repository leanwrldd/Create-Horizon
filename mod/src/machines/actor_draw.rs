//! What the actors on a structure draw besides their block's model: a Drill's turning head, a
//! Saw's blade, a Harvester's blade (`DrillRenderer.renderInContraption`,
//! `SawRenderer.renderInContraption`, `HarvesterRenderer.renderInContraption`, `RollerRenderer.renderInContraption`).
//! A controller says
//! how its structure moves ([`set_motion`]) just before it draws the blocks, and
//! `contraption::draw_blocks` asks [`draw_actor`] for each block.
//!
//! Create turns each at the actor's `MovementContext.getAnimationSpeed()`: its motion in blocks a
//! tick (the length in thousandths, rounded to hundreds, negative), `angle = renderTime / 20 *
//! speed` degrees, 700 while the structure is stalled. The client has no contexts, so the motion
//! is worked out from the controller's own speed: the structure's blocks a tick for a piston or a
//! gantry, the angle it turns a tick times the block's distance from the axis for a bearing.
//! A head sleeps when the structure moves with its back first in Create; here it turns anyway.

use std::cell::Cell;

use create_kinetics::rules::actor;
use create_kinetics::rules::contraption::State;
use create_kinetics::{Axis, Dir, Pos};
use crate::compat::{BlockEntityDraw, Hooks, Host};

struct Shared;

impl Hooks for Shared {}

/// A module of shared code: no hooks of its own.
pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

/// How the structure being drawn moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Motion {
    Still,
    /// Held still by an actor (`contraption.stalled` on the client): 700.
    Stalled,
    /// Slides, this many blocks a tick.
    Slide(f32),
    /// Turns about the axis through its anchor, this many radians a tick.
    Turn { axis: Axis, radians: f32 },
}

thread_local! {
    static MOTION: Cell<Motion> = const { Cell::new(Motion::Still) };
}

/// The motion of the structure about to be drawn. Taken by the next `draw_blocks`.
pub fn set_motion(motion: Motion) {
    MOTION.with(|m| m.set(motion));
}

/// Takes the motion `draw_blocks` draws with, leaving none for whatever draws next.
pub(super) fn take_motion() -> Motion {
    MOTION.with(|m| m.replace(Motion::Still))
}

/// `MovementContext.getAnimationSpeed` of the block at `local` of a structure that moves so.
fn animation_speed(motion: Motion, local: Pos) -> f32 {
    match motion {
        Motion::Still => 0.0,
        Motion::Stalled => 700.0,
        Motion::Slide(blocks) => actor::animation_speed([f64::from(blocks), 0.0, 0.0], false),
        Motion::Turn { axis, radians } => {
            let l = local.map(|c| c as f32);
            let across: f32 = match axis {
                Axis::X => l[1].hypot(l[2]),
                Axis::Y => l[0].hypot(l[2]),
                Axis::Z => l[0].hypot(l[1]),
            };
            actor::animation_speed([f64::from(radians * across), 0.0, 0.0], false)
        }
    }
}

const CENTRE: [f32; 3] = [0.5; 3];
const BLADE: &str = "create:block/mechanical_harvester/blade";
/// `HarvesterRenderer.PIVOT` in pixels.
const HARVESTER_PIVOT: [f32; 3] = [0.0, 6.0 / 16.0, 9.0 / 16.0];

/// Draws the moving parts of the block `state` at `local`, under the pose in force.
pub(super) fn draw_actor(host: &dyn Host, draw: &mut BlockEntityDraw, state: &State, local: Pos, motion: Motion) {
    let Some(kind) = actor::kind_of(&state.name) else { return };
    let Some(facing) = state.dir("facing") else { return };
    let speed = animation_speed(motion, local);
    // Degrees a tick: `angle = renderTime / 20 * speed`.
    let rate = speed / 20.0;
    match kind {
        actor::Kind::Drill => {
            draw.push();
            if rate != 0.0 {
                draw.spin(facing.axis().unit(), CENTRE, rate * 1.0, 0.0);
            }
            super::drill::draw_facing(host, draw, super::drill::HEAD, facing);
            draw.still();
            draw.pop();
        }
        actor::Kind::Saw => {
            if !create_kinetics::rules::saw::is_horizontal(facing) {
                return;
            }
            // `SawRenderer.renderBlade`: the animated model follows the sign of the speed.
            let at = if speed > 0.0 { 0 } else if speed < 0.0 { 2 } else { 1 };
            draw.push();
            crate::draw::face_south(draw, facing);
            host.draw_model(draw, super::saw::BLADES[at], 0xFF_FFFF);
            draw.pop();
        }
        actor::Kind::Harvester => {
            if matches!(motion, Motion::Stalled) {
                draw_blade(host, draw, facing, 0.0);
            } else {
                draw_blade(host, draw, facing, rate);
            }
        }
        actor::Kind::Roller => {
            // `RollerRenderer.renderInContraption`: the wheel (still while the structure is stalled) and the
            // frame. Create turns the wheel the other way when the structure moves backwards; the client has
            // no motion's direction, so it turns one way. Its `translate(0, -.25, 0)` is in world space there,
            // here along the structure's own down.
            let rate = if matches!(motion, Motion::Stalled) { 0.0 } else { rate };
            super::roller::draw_wheel(host, draw, facing, rate);
            super::roller::draw_frame(host, draw, facing);
        }
        _ => {}
    }
}

/// `HarvesterRenderer.transform`: the blade turned to its facing, then about the pivot.
pub(super) fn draw_blade(host: &dyn Host, draw: &mut BlockEntityDraw, facing: Dir, rate: f32) {
    draw.push();
    crate::draw::face_south(draw, facing);
    if rate != 0.0 {
        // About `Direction.WEST`.
        draw.spin([-1.0, 0.0, 0.0], HARVESTER_PIVOT, rate, 0.0);
    }
    host.draw_model(draw, BLADE, 0xFF_FFFF);
    draw.still();
    draw.pop();
}
