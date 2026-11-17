//! What a Mechanical Crafter shows (`MechanicalCrafterRenderer`): the shaftless cog turning, the
//! lid while it works or is covered, an arrow, or a belt towards the crafter it passes items to,
//! the item in its slot and the grid's items moving along, drawn together, and crafted. The
//! numbers of the animation are `create_kinetics::rules::crafter`.
//!
//! The client has no tick of its own for a block entity, so the animation counts from the last
//! update (`Anchor`: the `CountDown` then, at game time `Since`) at the count-down speed, which
//! is what Create's own client tick does.

use create_kinetics::belt::SpriteShift;
use create_kinetics::rules::crafter::{self as rules, Pointing};
use create_kinetics::rules::crafter_grid::{Grid, Phase};
use create_kinetics::{Axis, Dir};
use crate::compat::{
    BlockEntityDraw, BlockEntitySubmitEvent, Compound, DisplayContext, Hooks, Host, ItemStack,
};

use super::crafter_world::{self as world, ANCHOR, BEFORE, COVERED, GROUPED, PHASE, SINCE};
use crate::{kinetic, util};

/// A module of shared code: no hooks of its own.
struct Shared;

impl Hooks for Shared {}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

/// `AllPartialModels.MECHANICAL_CRAFTER_LID`, `_ARROW`, `_BELT_FRAME` and `_BELT`.
pub const LID: &str = "create:block/mechanical_crafter/lid";
pub const ARROW: &str = "create:block/mechanical_crafter/arrow";
pub const BELT_FRAME: &str = "create:block/mechanical_crafter/belt";
pub const BELT: &str = "create:block/mechanical_crafter/belt_animated";
/// `AllPartialModels.SHAFTLESS_COGWHEEL`.
pub const COG: &str = "create:block/cogwheel_shaftless";
/// `AllSpriteShifts.CRAFTER_THINGIES`: the belt's sprite is a strip of four frames.
const THINGIES: &str = "create:block/crafter_thingies";
/// The slot's stack for a client (its block entity has no item slots).
pub const INVENTORY: &str = "Inventory";

const CENTRE: [f32; 3] = [0.5; 3];
const ORIGIN: [f32; 3] = [0.0; 3];
const UP: [f32; 3] = [0.0, 1.0, 0.0];
const EAST: [f32; 3] = [1.0, 0.0, 0.0];

/// A small cogwheel's turn at `pos` about its axis (`KineticBlockEntityVisual.rotationOffset`):
/// 22.5 degrees where the other two coordinates add up even, so neighbours mesh.
fn cog_phase(axis: Axis, pos: [i32; 3]) -> f32 {
    let sum: i32 = (0..3).filter(|i| *i != axis.index()).map(|i| pos[i]).sum();
    if sum.rem_euclid(2) == 0 { 22.5 } else { 0.0 }
}

/// `renderAndTransform`: a part drawn for a crafter facing west (from `x` below 0), turned to
/// its facing and pointing about the block's centre.
fn turn_part(draw: &mut BlockEntityDraw, facing: Dir, pointing: Pointing) {
    draw.rotate_about(UP, (rules::horizontal_angle(facing) + 90) as f32, CENTRE);
    draw.rotate_about(EAST, pointing.x_rotation() as f32, CENTRE);
}

fn stack_of(nbt: &Compound, key: &str) -> Option<ItemStack> {
    util::get(nbt, key).and_then(util::stack_of)
}

/// `MechanicalCrafterRenderer.renderSafe` and `renderFast`.
pub fn submit(host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
    let state = event.state;
    let (Some(facing), Some(pointing)) = (
        host.block_state_property(state, "facing").and_then(Dir::from_name),
        host.block_state_property(state, "pointing").and_then(Pointing::from_name),
    ) else {
        return;
    };
    let nbt = &event.entity.nbt;
    let pos = event.pos;
    let rpm = kinetic::speed(event.entity);
    let speed = rules::count_down_speed(rpm);
    let phase = match util::get(nbt, PHASE) {
        Some(crate::compat::Tag::String(s)) => Phase::from_name(&s.to_string_lossy()),
        _ => Phase::Idle,
    };
    let covered = util::boolean(nbt, COVERED);
    let draw = &mut *event.draw;
    let now = draw.game_time as f32 + draw.partial;
    let count_down = rules::estimated_count_down(
        util::int(nbt, ANCHOR),
        speed,
        now - util::long(nbt, SINCE) as f32,
    );

    draw_items(host, draw, nbt, facing, pointing, phase, count_down);

    // `renderFast`: the cog turning about the crafter's axis.
    let axis = facing.axis();
    draw.spin(axis.unit(), CENTRE, rpm * 0.3, cog_phase(axis, pos));
    draw.push();
    match axis {
        Axis::X => draw.rotate_about(Axis::Z.unit(), -90.0, CENTRE),
        Axis::Z => draw.rotate_about(Axis::X.unit(), 90.0, CENTRE),
        Axis::Y => {}
    }
    host.draw_model(draw, COG, 0xFF_FFFF);
    draw.pop();
    draw.still();

    if (covered || phase != Phase::Idle) && phase != Phase::Crafting && phase != Phase::Inserting {
        draw.push();
        turn_part(draw, facing, pointing);
        host.draw_model(draw, LID, 0xFF_FFFF);
        draw.pop();
    }

    let target = util::add(pos, rules::target_direction(facing, pointing).normal());
    let other = (host.block(target) == world::ID)
        .then(|| {
            Some((
                util::facing(host, target, "facing")?,
                host.block_property(target, "pointing").as_deref().and_then(Pointing::from_name)?,
            ))
        })
        .flatten();
    draw.push();
    turn_part(draw, facing, pointing);
    if rules::is_valid_target(facing, pointing, other) {
        let shift = (phase == Phase::Exporting).then(|| {
            let frame = rules::belt_frame(speed, draw_ticks(now)).rem_euclid(4);
            SpriteShift {
                source: THINGIES.to_owned(),
                target: THINGIES.to_owned(),
                scale: [1.0, 1.0],
                offset: [frame as f32 / 4.0, 0.0],
            }
        });
        crate::draw::draw_shifted(host, draw, BELT, shift.as_ref());
        host.draw_model(draw, BELT_FRAME, 0xFF_FFFF);
    } else {
        host.draw_model(draw, ARROW, 0xFF_FFFF);
    }
    draw.pop();
}

/// `AnimationTickHolder.getTicks()`, as the whole ticks of the game's clock.
fn draw_ticks(now: f32) -> i64 {
    now as i64
}

fn read_grid(nbt: &Compound, key: &str) -> Grid<ItemStack> {
    world::read_grid(nbt, key)
}

/// `renderItems`: the slot's item at rest, else the grid's items.
fn draw_items(
    host: &dyn Host,
    draw: &mut BlockEntityDraw,
    nbt: &Compound,
    facing: Dir,
    pointing: Pointing,
    phase: Phase,
    count_down: f32,
) {
    draw.push();
    let front = facing.normal();
    let mut at = [0, 1, 2].map(|i| front[i] as f32 * 0.58 + 0.5);
    if phase == Phase::Exporting {
        let target = rules::target_direction(facing, pointing).normal();
        let progress = rules::export_progress(count_down);
        for i in 0..3 {
            at[i] += target[i] as f32 * progress * 0.75;
        }
    }
    draw.translate(at);
    draw.scale([0.5; 3]);
    draw.rotate_about(UP, rules::horizontal_angle(facing) as f32, ORIGIN);

    if phase == Phase::Idle {
        if let Some(stack) = stack_of(nbt, INVENTORY) {
            draw.push();
            draw.translate([0.0, 0.0, -1.0 / 256.0]);
            draw.rotate_about(UP, 180.0, ORIGIN);
            host.draw_item(draw, &stack, DisplayContext::Fixed, None, 0);
            draw.pop();
        }
        draw.pop();
        return;
    }

    // The grid's items.
    let mut grid = read_grid(nbt, GROUPED);
    let mut distance = 0.5;
    draw.push();
    if phase == Phase::Crafting {
        grid = read_grid(nbt, BEFORE);
        let progress = rules::gather_progress(count_down);
        let (early, late) = rules::early_late(progress);
        draw.scale([1.0 - late; 3]);
        let st = grid.stats();
        let (cx, cy) = rules::centering(st.min_x, st.min_y, st.width, st.height, early);
        draw.translate([cx * 0.5, cy * 0.5, 0.0]);
        distance = rules::crafting_spacing(progress);
    }
    let only_first = phase == Phase::Inserting || (phase == Phase::Crafting && count_down < 1000.0);
    for ((x, y), stack) in &grid.cells {
        let Some(stack) = stack else { continue };
        if only_first && (*x, *y) != (0, 0) {
            continue;
        }
        draw.push();
        draw.translate([*x as f32 * distance, *y as f32 * distance, 0.0]);
        let layer = if phase == Phase::Exporting { rules::export_layer(pointing) } else { 0 };
        draw.rotate_about(UP, 180.0, ORIGIN);
        draw.translate([0.0, 0.0, (*x + *y * 3 + layer * 9) as f32 / 1024.0]);
        host.draw_item(draw, stack, DisplayContext::Fixed, None, 0);
        draw.pop();
    }
    draw.pop();

    if phase == Phase::Crafting {
        // The result swells and spins in the middle as the items are drawn into it.
        let progress = ((1000.0 - count_down) / 1000.0).clamp(0.0, 1.0);
        let (early, late) = rules::early_late(progress);
        draw.rotate_about([0.0, 0.0, 1.0], early * 2.0 * 360.0, ORIGIN);
        let up = early * 1.125;
        let down = 1.0 + (1.0 - late) * 0.125;
        draw.scale([up; 3]);
        draw.scale([down; 3]);
        if let Some(Some(stack)) = read_grid(nbt, GROUPED).cells.get(&(0, 0)) {
            draw.push();
            draw.rotate_about(UP, 180.0, ORIGIN);
            host.draw_item(draw, stack, DisplayContext::Fixed, None, 0);
            draw.pop();
        }
    }
    draw.pop();
}
