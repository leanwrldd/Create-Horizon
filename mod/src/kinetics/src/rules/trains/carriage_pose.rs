//! Where a carriage is and how it is turned, as `DimensionalCarriageEntity.alignEntity`
//! (content/trains/entity/Carriage.java) puts the carriage's entity, and how
//! `CarriageContraptionEntityRenderer` / `OrientedContraptionEntity.applyLocalTransforms` turn what it draws
//! (CarriageContraptionEntityRenderer.java, OrientedContraptionEntity.java), and `CarriageBogey.updateAngles`
//! (CarriageBogey.java) for the bogeys' own turn and their wheels.
//!
//! There is no entity here: the glue keeps a [`CarriagePose`] per carriage and draws the structure under the
//! operations [`carriage_ops`] and each bogey under [`bogey_ops`], in the order Java pushes them on the `PoseStack`
//! (the first is the outermost). The numbers are Java's; the game does the drawing.

use super::vec::{Vec3, deg};

/// A turn or a move of the drawing pose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    /// A turn about the Y axis, in degrees (`rotateYDegrees`).
    RotateY(f32),
    RotateX(f32),
    RotateZ(f32),
    Translate(Vec3),
}

/// What `alignEntity` sets: the entity's position and its yaw and pitch (`entity.yaw`, `entity.pitch`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CarriagePose {
    /// `setPos(positionAnchor)`.
    pub position: Vec3,
    /// `entity.yaw`.
    pub yaw: f32,
    /// `entity.pitch`.
    pub pitch: f32,
}

/// `alignEntity`'s angles from the two rotation anchors (`rotationAnchors.getFirst()` is the leading one).
pub fn align(position_anchor: Vec3, rotation_anchors: [Vec3; 2]) -> CarriagePose {
    let [lead, trail] = rotation_anchors;
    let (dx, dy, dz) = (lead[0] - trail[0], lead[1] - trail[1], lead[2] - trail[2]);
    CarriagePose {
        position: position_anchor,
        // `(float) (Mth.atan2(diffZ, diffX) * 180 / Math.PI) + 180`.
        yaw: (dz.atan2(dx) * 180.0 / std::f64::consts::PI) as f32 + 180.0,
        // `(float) (Math.atan2(diffY, Math.sqrt(diffX * diffX + diffZ * diffZ)) * 180 / Math.PI) * -1`.
        pitch: (dy.atan2((dx * dx + dz * dz).sqrt()) * 180.0 / std::f64::consts::PI) as f32 * -1.0,
    }
}

/// `Direction.toYRot()` of the horizontal direction with this `(x, z)` step: south 0, west 90, north 180, east 270.
pub fn direction_y_rot(step: [i32; 2]) -> f32 {
    match step {
        [0, 1] => 0.0,
        [-1, 0] => 90.0,
        [0, -1] => 180.0,
        _ => 270.0,
    }
}

/// `getInitialYaw()`: the yaw of the initial orientation, `assemblyDirection.getClockWise()` (north to east, east
/// to south, south to west, west to north), given the assembly direction's `(x, z)` step.
pub fn initial_yaw(assembly_direction: [i32; 2]) -> f32 {
    let clockwise = [-assembly_direction[1], assembly_direction[0]];
    direction_y_rot(clockwise)
}

/// `OrientedContraptionEntity.getViewYRot(1)`.
pub fn view_y_rot(yaw: f32) -> f32 {
    -yaw
}

/// `applyLocalTransforms`: the structure is drawn from the entity's position under these. The block at the local
/// origin has its corner half a block west and north of the position (`translate(-.5, 0, -.5)`), and the turns
/// are about the middle of that block (`center()` ... `uncenter()`).
pub fn carriage_ops(yaw: f32, pitch: f32, initial_yaw: f32) -> [Op; 4] {
    [Op::Translate([-0.5, 0.0, -0.5]), Op::RotateY(view_y_rot(yaw)), Op::RotateZ(pitch), Op::RotateY(initial_yaw)]
}

/// The point the turns of [`carriage_ops`] go about, in the local block's own coordinates (`center()`).
pub const CENTRE: Vec3 = [0.5, 0.5, 0.5];

/// What `CarriageBogey.updateAngles` works out of the wheels' two positions: the yaw (already negated, as
/// `yaw.setValue(-yRot)`) and the pitch of the bogey's own turn.
pub fn bogey_angles(leading: Vec3, trailing: Vec3) -> (f32, f32) {
    let (dx, dy, dz) = (leading[0] - trailing[0], leading[1] - trailing[1], leading[2] - trailing[2]);
    let y_rot = deg(dz.atan2(dx)) + 90.0;
    let x_rot = deg(dy.atan2((dx * dx + dz * dz).sqrt()));
    (-y_rot, x_rot)
}

/// The bogey's angles when the carriage is derailed (`-90 + entity.yaw - derailAngle`) or in another dimension
/// (`-90 + entity.yaw`), as `(yaw, pitch)` of `updateAngles` (`yaw.setValue(-yRot)`).
pub fn bogey_angles_derailed(carriage_yaw: f32, derail_angle: Option<i32>) -> (f32, f32) {
    let y_rot = -90.0 + carriage_yaw - derail_angle.map_or(0.0, |a| a as f32);
    (-y_rot, 0.0)
}

/// How far the wheels turn for a distance moved (`360 * distanceMoved / (Math.PI * 2 * radius)`).
pub fn wheel_step(distance_moved: f64, wheel_radius: f64) -> f64 {
    360.0 * distance_moved / (std::f64::consts::PI * 2.0 * wheel_radius)
}

/// `newWheelAngle = (wheelAngle.getValue() - angleDiff) % 360`.
pub fn next_wheel_angle(angle: f64, distance_moved: f64, wheel_radius: f64) -> f64 {
    (angle - wheel_step(distance_moved, wheel_radius)) % 360.0
}

/// The signed distance a carriage moved this tick: `diff.length() * signum(-relativeDiff.x)` where
/// `relativeDiff = VecHelper.rotate(diff, yaw, Axis.Y)`; the second value is `movingBackwards`.
pub fn distance_moved(diff: Vec3, yaw: f32) -> (f64, bool) {
    let rad = f64::from(yaw).to_radians();
    // `VecHelper.rotate(vec, deg, Y)`: x' = x cos + z sin.
    let x = diff[0] * rad.cos() + diff[2] * rad.sin();
    let signum = if -x > 0.0 { 1.0 } else if -x < 0.0 { -1.0 } else { 0.0 };
    let length = (diff[0] * diff[0] + diff[1] * diff[1] + diff[2] * diff[2]).sqrt();
    (length * signum, signum < 0.0)
}

/// `Mth.wrapDegrees`: an angle difference in -180..180.
pub fn wrap_degrees(angle: f32) -> f32 {
    let a = angle.rem_euclid(360.0);
    if a >= 180.0 { a - 360.0 } else { a }
}

/// A carriage's pose and how it is changing, as the server sends it: the client draws the carriage where it will
/// be a moment later (`now`, in ticks and the fraction of the frame) from the last news (Create's entity has its
/// position and rotation synced every tick and the renderer lerps between the last two; this stands in for that,
/// news arriving less often).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub position: Vec3,
    /// Blocks per tick.
    pub velocity: Vec3,
    pub yaw: f32,
    /// Degrees per tick.
    pub yaw_rate: f32,
    pub pitch: f32,
    pub pitch_rate: f32,
    /// The game time the pose is of.
    pub time: i64,
}

impl Motion {
    /// The pose now and the one a tick before it.
    pub fn new(now: &CarriagePose, before: &CarriagePose, time: i64) -> Motion {
        Motion {
            position: now.position,
            velocity: [
                now.position[0] - before.position[0],
                now.position[1] - before.position[1],
                now.position[2] - before.position[2],
            ],
            yaw: now.yaw,
            yaw_rate: wrap_degrees(now.yaw - before.yaw),
            pitch: now.pitch,
            pitch_rate: wrap_degrees(now.pitch - before.pitch),
            time,
        }
    }

    /// The pose at `now` ticks of game time (a fraction counts), at most `LOOKAHEAD` ticks past the news.
    pub fn at(&self, now: f64) -> CarriagePose {
        let dt = (now - self.time as f64).clamp(0.0, LOOKAHEAD);
        CarriagePose {
            position: [
                self.position[0] + self.velocity[0] * dt,
                self.position[1] + self.velocity[1] * dt,
                self.position[2] + self.velocity[2] * dt,
            ],
            yaw: self.yaw + self.yaw_rate * dt as f32,
            pitch: self.pitch + self.pitch_rate * dt as f32,
        }
    }

    pub fn is_moving(&self) -> bool {
        self.velocity.iter().any(|v| *v != 0.0) || self.yaw_rate != 0.0 || self.pitch_rate != 0.0
    }
}

/// How many ticks past its news a pose is extrapolated (a client that hears nothing for longer holds still).
pub const LOOKAHEAD: f64 = 6.0;

/// How the wheels turn per tick while the carriage moves by `diff` this tick (`updateAngles`): the change of
/// `wheelAngle` in degrees.
pub fn wheel_rate(diff: Vec3, yaw: f32, wheel_radius: f64) -> f64 {
    let (moved, _) = distance_moved(diff, yaw);
    -wheel_step(moved, wheel_radius)
}

/// `CarriageContraptionEntityRenderer.translateBogey`: where a bogey is drawn, from the entity's own position.
/// `bogey_yaw` and `bogey_pitch` are the bogey's own angles ([`bogey_angles`]).
#[allow(clippy::too_many_arguments)]
pub fn bogey_ops(
    self_upside_down: bool,
    leading_upside_down: bool,
    is_leading: bool,
    bogey_spacing: i32,
    view_y_rot: f32,
    view_x_rot: f32,
    bogey_yaw: f32,
    bogey_pitch: f32,
) -> Vec<Op> {
    let mut ops = vec![
        Op::RotateY(view_y_rot + 90.0),
        Op::RotateX(-view_x_rot),
        Op::RotateY(180.0),
        Op::Translate([0.0, 0.0, if is_leading { 0.0 } else { -(bogey_spacing as f64) }]),
        Op::RotateY(-180.0),
        Op::RotateX(view_x_rot),
        Op::RotateY(-view_y_rot - 90.0),
        Op::RotateY(bogey_yaw),
        Op::RotateX(bogey_pitch),
        Op::Translate([0.0, 0.5, 0.0]),
    ];
    if self_upside_down {
        ops.push(Op::RotateZ(180.0));
    }
    if self_upside_down != leading_upside_down {
        ops.push(Op::Translate([0.0, 2.0, 0.0]));
    }
    ops
}

/// What `StandardBogeyRenderer` draws, a bogey of the small or large size, as steps the glue maps to the game's
/// drawing: the shaft blocks, the frame and wheels partial models, with their poses. The wheel angle is the
/// bogey's `wheelAngle`.
#[derive(Clone, Debug, PartialEq)]
pub enum BogeyPart {
    /// A `create:shaft` block drawn from its corner: `axis` is `"x"` or `"z"`.
    Shaft { axis: char, at: Vec3, turn_about: char, angle: f32 },
    /// A partial model with its pose, in the order Java's calls stack them: `translate(at)`, `rotateX(angle_x)`,
    /// `translate(then)`, `rotateX(unturn)`, then the `scale`.
    Model { model: &'static str, at: Vec3, angle_x: f32, then: Vec3, unturn: f32, scale: f32, scroll: Option<f32> },
}

/// `AllPartialModels` names of the bogey's pieces (`block("track/bogey/...")`).
pub const FRAME: &str = "create:block/track/bogey/bogey_frame";
pub const SMALL_WHEELS: &str = "create:block/track/bogey/bogey_wheel";
pub const PIN: &str = "create:block/track/bogey/bogey_drive_wheel_pin";
pub const PISTON: &str = "create:block/track/bogey/bogey_drive_piston";
pub const DRIVE: &str = "create:block/track/bogey/bogey_drive";
pub const LARGE_WHEELS: &str = "create:block/track/bogey/bogey_drive_wheel";
pub const BELT: &str = "create:block/track/bogey/bogey_drive_belt";

/// `StandardBogeyRenderer.render` and its `Small` / `Large` subclasses.
pub fn standard_bogey(large: bool, wheel_angle: f32) -> Vec<BogeyPart> {
    fn model(parts: &mut Vec<BogeyPart>, model: &'static str, at: Vec3, angle_x: f32, scale: f32, then: Vec3, scroll: Option<f32>) {
        parts.push(BogeyPart::Model { model, at, angle_x, then, unturn: 0.0, scale, scroll });
    }
    let mut parts = Vec::new();
    let angle = wheel_angle;
    // The two shafts (`translate(-.5, .25, i * -1)`, turned about Z at its centre).
    for i in [0.0, -1.0] {
        parts.push(BogeyPart::Shaft { axis: 'z', at: [-0.5, 0.25, i], turn_about: 'z', angle });
    }
    let shrink = 1.0 - 1.0 / 512.0;
    if !large {
        model(&mut parts, FRAME, [0.0; 3], 0.0, shrink, [0.0; 3], None);
        for side in [1.0, -1.0] {
            model(&mut parts, SMALL_WHEELS, [0.0, 12.0 / 16.0, side], angle, 1.0, [0.0; 3], None);
        }
        return parts;
    }
    // Large: the secondary shafts, then the drive (`Large.render`).
    for i in [0.0, 1.0] {
        parts.push(BogeyPart::Shaft { axis: 'x', at: [-0.5, 0.25, 0.5 + i * -2.0], turn_about: 'x', angle });
    }
    let radius_in_uv = 5.0_f32 / 16.0;
    let scroll = radius_in_uv * (std::f32::consts::PI / 180.0) * angle;
    let scroll = scroll - scroll.floor();
    model(&mut parts, DRIVE, [0.0; 3], 0.0, shrink, [0.0; 3], None);
    model(&mut parts, BELT, [0.0; 3], 0.0, shrink, [0.0; 3], Some(scroll));
    let piston_z = 0.25 * f64::from(angle.to_radians()).sin();
    model(&mut parts, PISTON, [0.0, 0.0, piston_z], 0.0, 1.0, [0.0; 3], None);
    model(&mut parts, LARGE_WHEELS, [0.0, 1.0, 0.0], angle, 1.0, [0.0; 3], None);
    // The pin: turned with the wheel, moved a quarter up along the turned axis, turned back.
    model(&mut parts, PIN, [0.0, 1.0, 0.0], angle, 1.0, [0.0, 0.25, 0.0], None);
    if let Some(BogeyPart::Model { unturn, .. }) = parts.last_mut() {
        *unturn = -angle;
    }
    parts
}

// --- the pose news a moving carriage sends its viewers ----------------------------------------------------------------

/// A bogey's part of the pose news.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BogeyNews {
    pub large: bool,
    pub upside_down: bool,
    pub leading: bool,
    pub yaw: f32,
    pub pitch: f32,
    pub wheel: f64,
    pub wheel_rate: f64,
}

/// What a moving carriage tells the clients near it, without its structure (Create syncs the entity's position and
/// rotation each tick, `CarriageContraptionEntity`'s data; here the block entity that draws it is told by a message, so
/// the structure goes only with the block entity's own, rarer updates): the anchor block it is drawn by, the motion,
/// whether it is derailed, and the bogeys.
#[derive(Clone, Debug, PartialEq)]
pub struct PoseNews {
    pub anchor: [i32; 3],
    pub motion: Motion,
    pub derailed: bool,
    pub bogeys: Vec<BogeyNews>,
}

/// The size of one bogey in [`PoseNews::encode`]: flags, yaw, pitch, wheel and its rate.
const BOGEY_BYTES: usize = 1 + 4 + 4 + 8 + 8;
/// The size of [`PoseNews::encode`] before the bogeys.
const HEAD_BYTES: usize = 12 + 24 + 24 + 16 + 8 + 1 + 1;

impl PoseNews {
    /// Little-endian bytes: the anchor, the position and velocity, yaw, its rate, pitch, its rate, the time, the
    /// derailed flag, the bogey count, then each bogey.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEAD_BYTES + self.bogeys.len() * BOGEY_BYTES);
        for v in self.anchor {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for v in self.motion.position.iter().chain(&self.motion.velocity) {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for v in [self.motion.yaw, self.motion.yaw_rate, self.motion.pitch, self.motion.pitch_rate] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&self.motion.time.to_le_bytes());
        out.push(u8::from(self.derailed));
        out.push(self.bogeys.len().min(255) as u8);
        for b in self.bogeys.iter().take(255) {
            out.push(u8::from(b.large) | u8::from(b.upside_down) << 1 | u8::from(b.leading) << 2);
            out.extend_from_slice(&b.yaw.to_le_bytes());
            out.extend_from_slice(&b.pitch.to_le_bytes());
            out.extend_from_slice(&b.wheel.to_le_bytes());
            out.extend_from_slice(&b.wheel_rate.to_le_bytes());
        }
        out
    }

    /// The news [`PoseNews::encode`] wrote, or none when the bytes are short, long or not finite.
    pub fn decode(data: &[u8]) -> Option<Self> {
        let mut at = 0usize;
        let mut take = |n: usize| -> Option<&[u8]> {
            let part = data.get(at..at + n)?;
            at += n;
            Some(part)
        };
        let i32_at = |b: &[u8]| i32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        let f32_at = |b: &[u8]| f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        let f64_at = |b: &[u8]| f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
        let mut anchor = [0; 3];
        for v in &mut anchor {
            *v = i32_at(take(4)?);
        }
        let mut vectors = [0.0; 6];
        for v in &mut vectors {
            *v = f64_at(take(8)?);
        }
        let mut angles = [0.0f32; 4];
        for v in &mut angles {
            *v = f32_at(take(4)?);
        }
        let time_bytes = take(8)?;
        let time = i64::from_le_bytes(time_bytes.try_into().ok()?);
        let derailed = take(1)?[0] != 0;
        let count = usize::from(take(1)?[0]);
        let mut bogeys = Vec::with_capacity(count);
        for _ in 0..count {
            let flags = take(1)?[0];
            let yaw = f32_at(take(4)?);
            let pitch = f32_at(take(4)?);
            let wheel = f64_at(take(8)?);
            let wheel_rate = f64_at(take(8)?);
            bogeys.push(BogeyNews {
                large: flags & 1 != 0,
                upside_down: flags & 2 != 0,
                leading: flags & 4 != 0,
                yaw,
                pitch,
                wheel,
                wheel_rate,
            });
        }
        if at != data.len() {
            return None;
        }
        let finite = vectors.iter().all(|v| v.is_finite())
            && angles.iter().all(|v| v.is_finite())
            && bogeys.iter().all(|b| b.yaw.is_finite() && b.pitch.is_finite() && b.wheel.is_finite() && b.wheel_rate.is_finite());
        if !finite {
            return None;
        }
        let motion = Motion {
            position: [vectors[0], vectors[1], vectors[2]],
            velocity: [vectors[3], vectors[4], vectors[5]],
            yaw: angles[0],
            yaw_rate: angles[1],
            pitch: angles[2],
            pitch_rate: angles[3],
            time,
        };
        Some(PoseNews { anchor, motion, derailed, bogeys })
    }
}

#[cfg(test)]
mod news_tests {
    use super::*;

    fn news() -> PoseNews {
        PoseNews {
            anchor: [-12, 64, 300],
            motion: Motion {
                position: [1.5, -2.25, 1e6],
                velocity: [0.25, 0.0, -0.5],
                yaw: 91.5,
                yaw_rate: -1.25,
                pitch: 3.0,
                pitch_rate: 0.5,
                time: 123_456_789,
            },
            derailed: true,
            bogeys: vec![
                BogeyNews { large: true, upside_down: false, leading: true, yaw: 10.0, pitch: -2.0, wheel: 359.5, wheel_rate: 12.0 },
                BogeyNews { large: false, upside_down: true, leading: false, yaw: 0.0, pitch: 0.0, wheel: 0.0, wheel_rate: 0.0 },
            ],
        }
    }

    #[test]
    fn news_survives_the_round_trip() {
        let n = news();
        let bytes = n.encode();
        assert_eq!(bytes.len(), HEAD_BYTES + 2 * BOGEY_BYTES);
        assert_eq!(PoseNews::decode(&bytes), Some(n));
    }

    #[test]
    fn short_long_or_broken_news_is_refused() {
        let bytes = news().encode();
        assert_eq!(PoseNews::decode(&bytes[..bytes.len() - 1]), None);
        let mut long = bytes.clone();
        long.push(0);
        assert_eq!(PoseNews::decode(&long), None);
        let mut broken = news();
        broken.motion.yaw = f32::NAN;
        assert_eq!(PoseNews::decode(&broken.encode()), None);
        assert_eq!(PoseNews::decode(&[]), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn a_carriage_facing_west_to_east_has_no_turn() {
        // The leading bogey is the one further west (a train assembled at a station facing the station).
        let pose = align([5.0, 70.0, 5.0], [[4.0, 70.0, 5.0], [8.0, 70.0, 5.0]]);
        assert!(near(pose.yaw.rem_euclid(360.0), 0.0) || near(pose.yaw.rem_euclid(360.0), 360.0), "{}", pose.yaw);
        assert!(near(pose.pitch, 0.0));
    }

    #[test]
    fn the_motion_runs_on_from_its_news_and_stops_after_a_while() {
        let before = CarriagePose { position: [0.0, 0.0, 0.0], yaw: 350.0, pitch: 0.0 };
        let now = CarriagePose { position: [1.0, 0.0, 0.0], yaw: 10.0, pitch: 0.0 };
        let m = Motion::new(&now, &before, 100);
        assert!(m.is_moving());
        assert!((m.yaw_rate - 20.0).abs() < 1e-4);
        let later = m.at(102.5);
        assert!((later.position[0] - 3.5).abs() < 1e-9);
        assert!((later.yaw - 60.0).abs() < 1e-3);
        // Never before the news, never beyond the look-ahead.
        assert_eq!(m.at(90.0).position, now.position);
        assert!((m.at(200.0).position[0] - (1.0 + LOOKAHEAD)).abs() < 1e-9);
        let still = Motion::new(&now, &now, 5);
        assert!(!still.is_moving());
    }

    #[test]
    fn the_wheels_turn_against_the_way_the_carriage_goes() {
        assert!(wheel_rate([-0.5, 0.0, 0.0], 0.0, 0.5) < 0.0);
        assert!(wheel_rate([0.5, 0.0, 0.0], 0.0, 0.5) > 0.0);
        assert_eq!(wheel_rate([0.0; 3], 0.0, 0.5), 0.0);
    }

    #[test]
    fn a_carriage_the_other_way_turns_half_round() {
        let pose = align([5.0, 70.0, 5.0], [[8.0, 70.0, 5.0], [4.0, 70.0, 5.0]]);
        assert!(near(pose.yaw, 180.0));
    }

    #[test]
    fn a_climbing_carriage_pitches() {
        let pose = align([0.0; 3], [[4.0, 1.0, 0.0], [0.0, 0.0, 0.0]]);
        assert!(pose.pitch < 0.0);
        assert!(near(pose.pitch, -(1.0_f32.atan2(4.0).to_degrees())));
    }

    #[test]
    fn the_initial_orientation_is_the_assembly_direction_turned_clockwise() {
        // East: clockwise is south, `toYRot` 0. North: east, 270. West: north, 180. South: west, 90.
        assert_eq!(initial_yaw([1, 0]), 0.0);
        assert_eq!(initial_yaw([0, -1]), 270.0);
        assert_eq!(initial_yaw([-1, 0]), 180.0);
        assert_eq!(initial_yaw([0, 1]), 90.0);
    }

    #[test]
    fn the_carriage_is_turned_about_its_centre_after_moving_half_a_block() {
        let ops = carriage_ops(180.0, 5.0, 90.0);
        assert_eq!(ops[0], Op::Translate([-0.5, 0.0, -0.5]));
        assert_eq!(ops[1], Op::RotateY(-180.0));
        assert_eq!(ops[2], Op::RotateZ(5.0));
        assert_eq!(ops[3], Op::RotateY(90.0));
    }

    #[test]
    fn a_trailing_bogey_sits_its_spacing_behind() {
        let ops = bogey_ops(false, false, false, 3, 0.0, 0.0, 0.0, 0.0);
        assert!(ops.contains(&Op::Translate([0.0, 0.0, -3.0])));
        assert_eq!(ops.last(), Some(&Op::Translate([0.0, 0.5, 0.0])));
        let up = bogey_ops(true, false, true, 3, 0.0, 0.0, 0.0, 0.0);
        assert!(up.contains(&Op::RotateZ(180.0)) && up.contains(&Op::Translate([0.0, 2.0, 0.0])));
    }

    #[test]
    fn the_wheels_turn_with_the_distance() {
        // A radius of one half: a full turn per pi blocks.
        assert!((wheel_step(std::f64::consts::PI, 0.5) - 360.0).abs() < 1e-9);
        assert!((next_wheel_angle(10.0, 0.0, 0.5) - 10.0).abs() < 1e-12);
        assert!(next_wheel_angle(10.0, 1.0, 0.5) < 10.0);
    }

    #[test]
    fn moving_along_the_carriage_is_forwards() {
        // Yaw 0: `x' = x`, so moving to -x is a positive distance.
        let (d, back) = distance_moved([-1.0, 0.0, 0.0], 0.0);
        assert!((d - 1.0).abs() < 1e-9 && !back);
        let (d, back) = distance_moved([1.0, 0.0, 0.0], 0.0);
        assert!((d + 1.0).abs() < 1e-9 && back);
    }

    #[test]
    fn the_small_bogey_has_a_frame_and_two_wheels_and_the_large_a_drive() {
        let small = standard_bogey(false, 30.0);
        assert_eq!(small.iter().filter(|p| matches!(p, BogeyPart::Shaft { .. })).count(), 2);
        assert_eq!(small.iter().filter(|p| matches!(p, BogeyPart::Model { model, .. } if *model == SMALL_WHEELS)).count(), 2);
        let large = standard_bogey(true, 30.0);
        assert!(large.iter().any(|p| matches!(p, BogeyPart::Model { model, scroll: Some(_), .. } if *model == BELT)));
        assert_eq!(large.iter().filter(|p| matches!(p, BogeyPart::Shaft { axis: 'x', .. })).count(), 2);
    }
}
