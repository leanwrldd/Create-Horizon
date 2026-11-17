//! Drawing the Mechanical Arm (`ArmRenderer`, `ArmVisual`): the cog turning in the base, and the
//! arm's partial models (base, lower body, upper body, claw, two claw grips) stacked through the
//! transforms `ArmRenderer` uses, with the held item in the claw. The angles are
//! `create_kinetics::rules::arm::pose` (and `rave`) of the saved movement at the frame's time.

use create_kinetics::rules::arm::{self as rules, Phase};
use crate::compat::{BlockEntityDraw, BlockEntitySubmitEvent, DisplayContext, Host, Hooks};

use super::arm::{GOGGLES, HELD, MODEL_COG, MODELS, read_saved};
use crate::{kinetic, util};

const X: [f32; 3] = [1.0, 0.0, 0.0];
const Y: [f32; 3] = [0.0, 1.0, 0.0];
const Z: [f32; 3] = [0.0, 0.0, 1.0];
const ORIGIN: [f32; 3] = [0.0; 3];
const WHITE: u32 = 0xFF_FFFF;

/// A module of shared code: no hooks of its own.
struct Shared;

impl Hooks for Shared {}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

/// `AllPartialModels.ARM_*`, in the order of [`MODELS`].
const BASE: &str = MODELS[0];
const LOWER_BODY: &str = MODELS[1];
const UPPER_BODY: &str = MODELS[2];
const CLAW: &str = MODELS[3];
const CLAW_GOGGLES: &str = MODELS[4];
const UPPER_GRIP: &str = MODELS[5];
const LOWER_GRIP: &str = MODELS[6];

pub(super) fn draw(host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
    let entity = event.entity;
    let nbt = &entity.nbt;
    let speed = kinetic::speed(entity);
    let ceiling = host.block_state_property(event.state, "ceiling") == Some("true");
    let goggles = util::boolean(nbt, GOGGLES);
    let held = util::get(nbt, HELD).and_then(util::stack_of);
    let draw = &mut *event.draw;
    let saved = read_saved(nbt, draw.game_time);

    // The cog under the arm turns with the shaft (`KineticBlockEntityRenderer.getRotatedModel`).
    draw.spin(Y, [0.5; 3], speed * 0.3, 0.0);
    host.draw_model(draw, MODEL_COG, WHITE);
    draw.still();

    // `ArmRenderer.renderSafe`: where the arm is, and in what colour.
    let dancing = saved.arm.phase == Phase::Dancing && speed != 0.0;
    let progress = rules::progress_at(saved.start, saved.since, speed, draw.game_time, draw.partial);
    let (angles, colour) = if dancing {
        let time = (draw.game_time % 3600) as f32 + draw.partial;
        let salt = (event.pos[0] * 31 + event.pos[1] * 17 + event.pos[2] * 7) % 64;
        rules::rave(time, salt, draw.game_time)
    } else {
        (rules::render_angles(rules::pose(progress, &saved.motion)), WHITE)
    };
    let [base, lower, upper, head] = angles;

    draw.push();
    // `msr.center()`; a ceiling arm hangs upside down.
    draw.translate([0.5; 3]);
    if ceiling {
        draw.rotate_about(X, 180.0, ORIGIN);
    }
    // `transformBase`.
    draw.translate([0.0, 4.0 / 16.0, 0.0]);
    draw.rotate_about(Y, base, ORIGIN);
    host.draw_model(draw, BASE, WHITE);
    // `transformLowerArm`.
    draw.translate([0.0, 2.0 / 16.0, 0.0]);
    draw.rotate_about(X, lower + 135.0, ORIGIN);
    host.draw_model(draw, LOWER_BODY, colour);
    // `transformUpperArm`.
    draw.translate([0.0, 0.0, -14.0 / 16.0]);
    draw.rotate_about(X, upper - 90.0, ORIGIN);
    host.draw_model(draw, UPPER_BODY, colour);
    // `transformHead`; the claw with Goggles on a ceiling arm is turned over.
    draw.translate([0.0, 0.0, -15.0 / 16.0]);
    draw.rotate_about(X, head - 45.0, ORIGIN);
    let flipped = ceiling && goggles;
    if flipped {
        draw.rotate_about(Z, 180.0, ORIGIN);
    }
    host.draw_model(draw, if goggles { CLAW_GOGGLES } else { CLAW }, WHITE);
    if flipped {
        draw.rotate_about(Z, 180.0, ORIGIN);
    }

    let is_block_item = held.as_ref().is_some_and(|s| host.default_state(&s.item).is_some());
    for flip in [1, -1] {
        draw.push();
        draw.translate(rules::claw_offset(held.is_some(), is_block_item, flip));
        host.draw_model(draw, if flip > 0 { LOWER_GRIP } else { UPPER_GRIP }, WHITE);
        draw.pop();
    }
    if let Some(stack) = &held {
        draw_held(host, draw, stack, is_block_item);
    }
    draw.pop();

    // Kept while nothing moves: at rest, or stopped. (A dancing arm reads the time.)
    if !dancing && (progress >= 1.0 || rules::rate(speed) == 0.0) {
        draw.keep();
    }
}

/// The item in the claw: turned flat, below the claw, scaled (`ArmRenderer`'s item transform).
fn draw_held(host: &dyn Host, draw: &mut BlockEntityDraw, stack: &crate::compat::ItemStack, is_block_item: bool) {
    let (scale, down) = rules::held_item_fit(is_block_item);
    draw.push();
    draw.rotate_about(X, 90.0, ORIGIN);
    draw.translate([0.0, -down, 0.0]);
    draw.scale([scale; 3]);
    host.draw_item(draw, stack, DisplayContext::Fixed, None, 0);
    draw.pop();
}
