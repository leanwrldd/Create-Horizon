//! Draw Create's baked models with the kinetic speed supplied by the block entity.

use create_kinetics::{Axis, Dir, Kind, Pos, face_sign};
use horizon_client::mod_api::{BlockEntityDraw, Host};

const CENTRE: [f32; 3] = [0.5; 3];
const SHAFT_HALF: &str = "create:block/shaft_half";
const CRANK_HANDLE: &str = "create:block/hand_crank/handle";
const WHEEL: &str = "create:block/water_wheel/wheel";
pub const MODELS: [&str; 3] = [SHAFT_HALF, CRANK_HANDLE, WHEEL];

/// The half-shaft and crank handle models point south at rest.
pub(crate) fn face_south(draw: &mut BlockEntityDraw, face: Dir) {
    let (axis, angle) = match face {
        Dir::South => (Axis::Y, 0.0),
        Dir::East => (Axis::Y, 90.0),
        Dir::West => (Axis::Y, -90.0),
        Dir::North => (Axis::Y, 180.0),
        Dir::Up => (Axis::X, -90.0),
        Dir::Down => (Axis::X, 90.0),
    };
    draw.rotate_about(axis.unit(), angle, CENTRE);
}

pub(crate) fn draw_shaft_half(host: &dyn Host, draw: &mut BlockEntityDraw, face: Dir, turn: f32) {
    shaft_half(host, draw, face, turn, None);
}

fn shaft_half(
    host: &dyn Host,
    draw: &mut BlockEntityDraw,
    face: Dir,
    turn: f32,
    angle: Option<f32>,
) {
    spin(draw, face.axis().unit(), turn, 0.0, angle);
    draw.push();
    face_south(draw, face);
    host.draw_model(draw, SHAFT_HALF, 0xFF_FFFF);
    draw.pop();
}

/// Ponder supplies an angle from its own playback clock. Use a stationary
/// spin with that phase so that world time cannot advance it.
pub(crate) fn spin(
    draw: &mut BlockEntityDraw,
    axis: [f32; 3],
    turn: f32,
    phase: f32,
    angle: Option<f32>,
) {
    let (turn, phase) = angle.map_or((turn, phase), |angle| (0.0, phase + angle));
    draw.spin(axis, CENTRE, turn, phase);
}

/// Create staggers adjacent gears so their teeth interleave.
fn tooth_phase(pos: Pos, axis: Axis, large: bool) -> f32 {
    let in_plane: i32 = (0..3).filter(|i| *i != axis.index()).map(|i| pos[i]).sum();
    match (in_plane.rem_euclid(2), large) {
        (0, _) => 22.5,
        (_, true) => 11.25,
        _ => 0.0,
    }
}

pub fn block(
    host: &dyn Host,
    draw: &mut BlockEntityDraw,
    kind: Kind,
    pos: Pos,
    rpm: f32,
    angle: Option<f32>,
) {
    let turn = rpm * 0.3; // Degrees per tick.
    match kind {
        Kind::Shaft(axis) => {
            spin(draw, axis.unit(), turn, 0.0, angle);
            host.draw_block(draw, "create:shaft", &[("axis", axis.name())]);
        }
        Kind::Cogwheel(axis) => {
            spin(
                draw,
                axis.unit(),
                turn,
                tooth_phase(pos, axis, false),
                angle,
            );
            host.draw_block(draw, "create:cogwheel", &[("axis", axis.name())]);
        }
        Kind::LargeCogwheel(axis) => {
            spin(draw, axis.unit(), turn, tooth_phase(pos, axis, true), angle);
            host.draw_block(draw, "create:large_cogwheel", &[("axis", axis.name())]);
        }
        Kind::Gearbox(axis) => {
            host.draw_block(draw, "create:gearbox", &[("axis", axis.name())]);
            for face in Dir::ALL.into_iter().filter(|face| face.axis() != axis) {
                let sign = face_sign(kind, face);
                shaft_half(
                    host,
                    draw,
                    face,
                    turn * sign,
                    angle.map(|angle| angle * sign),
                );
            }
        }
        Kind::Motor(facing) => {
            host.draw_block(draw, "create:creative_motor", &[("facing", facing.name())]);
            shaft_half(host, draw, facing, turn, angle);
        }
        Kind::Crank(facing) => {
            spin(draw, facing.axis().unit(), turn, 0.0, angle);
            host.draw_block(draw, "create:hand_crank", &[("facing", facing.name())]);
            draw.push();
            face_south(draw, facing.opposite());
            host.draw_model(draw, CRANK_HANDLE, 0xFF_FFFF);
            draw.pop();
        }
        Kind::WaterWheel(axis) => {
            spin(draw, axis.unit(), turn, 0.0, angle);
            draw.push();
            match axis {
                Axis::X => draw.rotate_about(Axis::Z.unit(), -90.0, CENTRE),
                Axis::Z => draw.rotate_about(Axis::X.unit(), 90.0, CENTRE),
                Axis::Y => {}
            }
            host.draw_model(draw, WHEEL, 0xFF_FFFF);
            draw.pop();
        }
        // Drawn with its items by `belt::draw`.
        Kind::Belt(..) => {}
        // A machine draws itself (`machines/`).
        Kind::Shafts(_) => {}
    }
    draw.still();
    draw.keep();
}
