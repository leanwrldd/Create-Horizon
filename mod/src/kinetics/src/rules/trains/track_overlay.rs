//! What the trains' client draws on and over the track, as numbers: `TrackBlock.prepareTrackOverlay` (the marker an edge point
//! shows on its track, held or placed), `TrackBlock.prepareAssemblyOverlay` and `StationRenderer` (the assembly area's coloured
//! spots and the station's flag), `TrackBlockOutline.pickCurves` (which segment of a curve the cursor is on) and the line the
//! relocation preview draws (`TrainRelocator.relocateClient`).
//!
//! Sources: `content/trains/track/{TrackBlock, TrackTargetingBehaviour, TrackTargetingClient, TrackBlockOutline}.java`,
//! `content/trains/station/StationRenderer.java`, `StationBlockEntity.java` (the flag's `LerpedFloat`),
//! `content/trains/entity/TrainRelocator.java`. The glue turns an [`Affine`] into a model instance or a draw pose.
//!
//! Left out: the tilt branch of `prepareTrackOverlay` (`TrackBlockEntityTilt` isn't ported, `isTilted` is always false).

use super::bezier::BezierConnection;
use super::track_render::model_angles;
use super::vec::{Vec3, add, normalize, sub};
use crate::Pos;

// --- a pose stack, as one matrix ---------------------------------------------------------------------------------------

/// What a `PoseStack` holds after a chain of `translate`, `scale` and turns: `p -> m * p + t`. Every operation acts on the frame the
/// earlier ones made, as `PoseStack`'s do (`m' = m * op`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub m: [[f64; 3]; 3],
    pub t: Vec3,
}

impl Affine {
    pub const IDENTITY: Affine = Affine { m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], t: [0.0; 3] };

    fn times(&self, v: Vec3) -> Vec3 {
        [0, 1, 2].map(|r| self.m[r][0] * v[0] + self.m[r][1] * v[1] + self.m[r][2] * v[2])
    }

    fn then(self, op: [[f64; 3]; 3]) -> Affine {
        let mut m = [[0.0; 3]; 3];
        for r in 0..3 {
            for c in 0..3 {
                m[r][c] = (0..3).map(|k| self.m[r][k] * op[k][c]).sum();
            }
        }
        Affine { m, t: self.t }
    }

    /// `translate(x, y, z)`.
    pub fn translate(self, by: Vec3) -> Affine {
        let moved = self.times(by);
        Affine { m: self.m, t: add(self.t, moved) }
    }

    /// `scale(s)`.
    pub fn scale(self, s: f64) -> Affine {
        self.then([[s, 0.0, 0.0], [0.0, s, 0.0], [0.0, 0.0, s]])
    }

    /// `scale(x, y, z)`.
    pub fn scale_xyz(self, s: Vec3) -> Affine {
        self.then([[s[0], 0.0, 0.0], [0.0, s[1], 0.0], [0.0, 0.0, s[2]]])
    }

    /// `rotateX(rad)`: `Axis.XP.rotation`, counter-clockwise looking down the axis.
    pub fn rotate_x(self, rad: f64) -> Affine {
        let (s, c) = rad.sin_cos();
        self.then([[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]])
    }

    /// `rotateY(rad)`.
    pub fn rotate_y(self, rad: f64) -> Affine {
        let (s, c) = rad.sin_cos();
        self.then([[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]])
    }

    /// `rotateZ(rad)`.
    pub fn rotate_z(self, rad: f64) -> Affine {
        let (s, c) = rad.sin_cos();
        self.then([[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]])
    }

    /// `center()`: `translate(.5, .5, .5)`.
    pub fn center(self) -> Affine {
        self.translate([0.5; 3])
    }

    /// `uncenter()`: `translate(-.5, -.5, -.5)`.
    pub fn uncenter(self) -> Affine {
        self.translate([-0.5; 3])
    }

    /// `rotateCentered(rad, UP)`.
    pub fn rotate_centered_y(self, rad: f64) -> Affine {
        self.center().rotate_y(rad).uncenter()
    }

    pub fn apply(&self, p: Vec3) -> Vec3 {
        add(self.times(p), self.t)
    }

    /// The row-major 3 by 4 of a model instance's transform.
    pub fn rows(&self) -> [f32; 12] {
        let mut out = [0.0_f32; 12];
        for r in 0..3 {
            for c in 0..3 {
                out[r * 4 + c] = self.m[r][c] as f32;
            }
            out[r * 4 + 3] = self.t[r] as f32;
        }
        out
    }
}

/// One draw operation, in the order the draw context's calls take them (each acts in the frame the earlier ones made).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Translate(Vec3),
    Scale(Vec3),
    /// A turn about an axis (`[1,0,0]`, `[0,1,0]` or `[0,0,1]`) by degrees.
    Rotate([f32; 3], f32),
}

/// A pose as the chain of operations that makes it: what a block entity's draw replays (`translate`, `scale`, `rotate_about`) and
/// what [`Chain::affine`] folds into a model instance's matrix.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chain(pub Vec<Op>);

impl Chain {
    pub fn new() -> Chain {
        Chain(Vec::new())
    }

    fn push(mut self, op: Op) -> Chain {
        self.0.push(op);
        self
    }

    pub fn translate(self, by: Vec3) -> Chain {
        self.push(Op::Translate(by))
    }

    pub fn scale(self, s: f64) -> Chain {
        self.push(Op::Scale([s; 3]))
    }

    pub fn scale_xyz(self, s: Vec3) -> Chain {
        self.push(Op::Scale(s))
    }

    pub fn rotate_x(self, rad: f64) -> Chain {
        self.push(Op::Rotate([1.0, 0.0, 0.0], rad.to_degrees() as f32))
    }

    pub fn rotate_y(self, rad: f64) -> Chain {
        self.push(Op::Rotate([0.0, 1.0, 0.0], rad.to_degrees() as f32))
    }

    pub fn rotate_z(self, rad: f64) -> Chain {
        self.push(Op::Rotate([0.0, 0.0, 1.0], rad.to_degrees() as f32))
    }

    pub fn center(self) -> Chain {
        self.translate([0.5; 3])
    }

    pub fn uncenter(self) -> Chain {
        self.translate([-0.5; 3])
    }

    pub fn rotate_centered_y(self, rad: f64) -> Chain {
        self.center().rotate_y(rad).uncenter()
    }

    /// The whole chain as one matrix.
    pub fn affine(&self) -> Affine {
        compose(&self.0)
    }
}

/// The operations as one transform.
pub fn compose(ops: &[Op]) -> Affine {
    let mut a = Affine::IDENTITY;
    for op in ops {
        a = match *op {
            Op::Translate(v) => a.translate(v),
            Op::Scale(v) => a.scale_xyz(v),
            Op::Rotate(axis, degrees) => {
                let rad = f64::from(degrees).to_radians();
                if axis[0] != 0.0 {
                    a.rotate_x(rad)
                } else if axis[1] != 0.0 {
                    a.rotate_y(rad)
                } else {
                    a.rotate_z(rad)
                }
            }
        };
    }
    a
}

// --- the marker on the track ---------------------------------------------------------------------------------------------

/// `RenderedTrackOverlayType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayType {
    Station,
    Signal,
    DualSignal,
    Observer,
}

impl OverlayType {
    /// `AllPartialModels.TRACK_STATION_OVERLAY` and the others.
    pub const fn model(self) -> &'static str {
        match self {
            OverlayType::Station => "create:block/track_overlay/station",
            OverlayType::Signal => "create:block/track_overlay/signal",
            OverlayType::DualSignal => "create:block/track_overlay/signal_dual",
            OverlayType::Observer => "create:block/track_overlay/observer",
        }
    }

    pub const ALL: [OverlayType; 4] = [OverlayType::Station, OverlayType::Signal, OverlayType::DualSignal, OverlayType::Observer];

    /// `TrackTargetingClient.render`'s choice by the held item's type.
    pub fn of_item(item: &str) -> Option<OverlayType> {
        match item {
            "create:track_station" => Some(OverlayType::Station),
            "create:track_signal" => Some(OverlayType::Signal),
            "create:track_observer" => Some(OverlayType::Observer),
            _ => None,
        }
    }
}

/// Where the overlay stands on the track: a straight block, or a segment of one of its curves.
#[derive(Clone, Copy, Debug)]
pub enum OverlaySpot<'a> {
    /// The block's first axis (`getAxes().get(0)`) and its up (`getUpNormal`).
    Straight { axis: Vec3, up: Vec3 },
    /// `BezierTrackPointLocation`: the curve as the block entity at the block holds it, and the segment.
    Curve { curve: &'a BezierConnection, segment: i32 },
}

/// `TrackBlock.prepareTrackOverlay` and the placing `TrackTargetingBehaviour.render` does after it (`translate(.5, 0, .5).scale(scale)
/// .translate(-.5, 0, -.5)`): the pose of the overlay model in the frame of the track block at `track`, `positive` being
/// `AxisDirection.POSITIVE`. `None` for a curve the block doesn't hold (`prepareTrackOverlay` returns null).
pub fn overlay_chain(spot: OverlaySpot<'_>, track: Pos, positive: bool, scale: f64) -> Option<Chain> {
    let step = if positive { 1.0 } else { -1.0 };
    let mut affine = Chain::new();
    let (angles, vertical_axis) = match spot {
        OverlaySpot::Curve { curve, segment } => {
            let length = (curve.length() * 2.0).floor();
            let seg = f64::from(segment + 1);
            let t = seg / length;
            let (tpre, tpost) = ((seg - 1.0) / length, (seg + 1.0) / length);
            let offset = curve.position(t);
            let normal = curve.normal(t);
            let diff = normalize(sub(curve.position(tpost), curve.position(tpre)));
            // `translate(offset.subtract(Vec3.atBottomCenterOf(pos)))` and `translate(0, -4 / 16f, 0)`.
            let bottom_centre = [f64::from(track[0]) + 0.5, f64::from(track[1]), f64::from(track[2]) + 0.5];
            affine = affine.translate(sub(offset, bottom_centre)).translate([0.0, -4.0 / 16.0, 0.0]);
            (model_angles(normal, diff), None)
        }
        OverlaySpot::Straight { axis, up } => {
            let diff = normalize([axis[0] * step, axis[1] * step, axis[2] * step]);
            (model_angles(up, diff), Some(axis))
        }
    };
    // `affine.center().rotateY(angles.y).rotateX(angles.x).uncenter()`.
    affine = affine.center().rotate_y(f64::from(angles[1] as f32)).rotate_x(f64::from(angles[0] as f32)).uncenter();
    match vertical_axis {
        Some(axis) => {
            // `translate(0, axis.y != 0 ? 7 / 16f : 0, axis.y != 0 ? direction.getStep() * 2.5f / 16f : 0)`.
            if axis[1] != 0.0 {
                affine = affine.translate([0.0, 7.0 / 16.0, step * 2.5 / 16.0]);
            }
        }
        None => {
            affine = affine.translate([0.0, 4.0 / 16.0, 0.0]);
            if !positive {
                affine = affine.rotate_centered_y(std::f64::consts::PI);
            }
        }
    }
    Some(affine.translate([0.5, 0.0, 0.5]).scale(scale).translate([-0.5, 0.0, -0.5]))
}

/// [`overlay_chain`] as a matrix, for a model instance.
pub fn overlay_pose(spot: OverlaySpot<'_>, track: Pos, positive: bool, scale: f64) -> Option<Affine> {
    overlay_chain(spot, track, positive, scale).map(|c| c.affine())
}

/// `TrackTargetingClient.render`'s scale for the held item's overlay: `1 + 1 / 16f`.
pub const HELD_SCALE: f64 = 1.0 + 1.0 / 16.0;

// --- the assembly area --------------------------------------------------------------------------------------------------

/// `colorWhenValid` and `colorWhenCarriage` of `StationRenderer`.
pub const COLOUR_VALID: u32 = 0x96B5FF;
pub const COLOUR_CARRIAGE: u32 = 0xCAFF96;

/// `AngleHelper.horizontalAngle`: south 0, east 90, north 180, west 270 (0 for up and down).
pub fn horizontal_angle(facing: [i32; 3]) -> f64 {
    match facing {
        [1, 0, 0] => 90.0,
        [0, 0, -1] => 180.0,
        [-1, 0, 0] => 270.0,
        _ => 0.0,
    }
}

/// One coloured spot of the assembly area: its step along the track (0 is the block after the station's) and its colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AssemblySpot {
    pub step: i32,
    pub colour: u32,
}

/// `StationRenderer`'s loop over `assemblyLength`: a spot where a bogey could go (`isValidBogeyOffset`, blue) or where one is
/// (green), nothing elsewhere.
pub fn assembly_spots(length: i32, bogey_locations: &[i32], valid_offset: impl Fn(i32) -> bool) -> Vec<AssemblySpot> {
    let mut out = Vec::new();
    for i in 0..length.max(0) {
        let mut colour = if valid_offset(i) { Some(COLOUR_VALID) } else { None };
        if bogey_locations.contains(&i) {
            colour = Some(COLOUR_CARRIAGE);
        }
        if let Some(colour) = colour {
            out.push(AssemblySpot { step: i, colour });
        }
    }
    out
}

/// `prepareAssemblyOverlay` then the loop's `translate(0, 0, 1)` per block, in the frame of the station's target track block: the
/// pose of the spot at `step`.
pub fn assembly_chain(direction: [i32; 3], step: i32) -> Chain {
    Chain::new().rotate_centered_y(horizontal_angle(direction).to_radians()).translate([0.0, 0.0, f64::from(step + 1)])
}

/// The four boxes of `block/track_overlay/assembling` (`from`, `to` in texels, the faces' `[u0, v0, u1, v1]` for north, south,
/// up and down; a face turned 180 degrees has its corners swapped). The glue draws them as cubes to give them the colour.
pub struct OverlayBox {
    pub from: Vec3,
    pub to: Vec3,
    pub north: [f32; 4],
    pub south: [f32; 4],
    pub up: [f32; 4],
    pub down: [f32; 4],
}

/// The model file's elements, in its order.
pub const ASSEMBLY_BOXES: [OverlayBox; 4] = [
    OverlayBox {
        from: [-2.95, -2.05625, 1.9],
        to: [8.0, 2.24375, 6.1],
        north: [0.0, 5.0, 11.0, 9.0],
        south: [11.0, 5.0, 0.0, 9.0],
        up: [11.0, 0.0, 0.0, 4.0],
        down: [11.0, 10.0, 0.0, 14.0],
    },
    OverlayBox {
        from: [-2.95, -2.05625, 9.9],
        to: [8.0, 2.24375, 14.1],
        north: [0.0, 5.0, 11.0, 9.0],
        south: [11.0, 5.0, 0.0, 9.0],
        up: [11.0, 0.0, 0.0, 4.0],
        down: [11.0, 10.0, 0.0, 14.0],
    },
    OverlayBox {
        from: [8.0, -2.05625, 1.9],
        to: [18.95, 2.24375, 6.1],
        north: [11.0, 5.0, 0.0, 9.0],
        south: [0.0, 5.0, 11.0, 9.0],
        up: [0.0, 4.0, 11.0, 0.0],
        down: [0.0, 14.0, 11.0, 10.0],
    },
    OverlayBox {
        from: [8.0, -2.05625, 9.9],
        to: [18.95, 2.24375, 14.1],
        north: [11.0, 5.0, 0.0, 9.0],
        south: [0.0, 5.0, 11.0, 9.0],
        up: [0.0, 4.0, 11.0, 0.0],
        down: [0.0, 14.0, 11.0, 10.0],
    },
];

// --- the station's flag --------------------------------------------------------------------------------------------------------

/// `StationBlockEntity.flag`: a `LerpedFloat.linear()` that `chase(target, 0.1f, LINEAR)` moves a tenth a tick towards 0 or 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flag {
    pub from: f32,
    pub target: f32,
    /// The game time (ticks, fractional) it started moving from `from`.
    pub since: f64,
}

/// `chase(.., 0.1f, ..)`.
pub const FLAG_SPEED: f32 = 0.1;

impl Flag {
    pub const fn lowered() -> Flag {
        Flag { from: 0.0, target: 0.0, since: 0.0 }
    }

    /// `getValue()` at game time `time`.
    pub fn value_at(&self, time: f64) -> f32 {
        let step = (FLAG_SPEED as f64 * (time - self.since).max(0.0)) as f32;
        let gap = self.target - self.from;
        self.from + gap.signum() * gap.abs().min(step)
    }

    /// `settled()`: it has arrived.
    pub fn settled_at(&self, time: f64) -> bool {
        (self.value_at(time) - self.target).abs() < 1e-6
    }

    /// `StationBlockEntity.tick`'s client half: `if (currentTarget == 0 || flag.settled())` and the wanted target differs, chase it.
    pub fn follow(self, wanted: bool, time: f64) -> Flag {
        let target = if wanted { 1.0 } else { 0.0 };
        if (self.target == 0.0 || self.settled_at(time)) && target != self.target {
            return Flag { from: self.value_at(time), target, since: time };
        }
        self
    }
}

/// `StationBlockEntity.trainPresent || isAssembling()`.
pub fn flag_wanted(train_present: bool, assembling: bool) -> bool {
    train_present || assembling
}

/// Which flag model `StationRenderer.renderSafe` picks: the assembling flag while it assembles with no train in the way, else on
/// (above `.75`) or off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlagModel {
    On,
    Off,
    Assemble,
}

impl FlagModel {
    pub const fn model(self) -> &'static str {
        match self {
            FlagModel::On => "create:block/track_station/flag_on",
            FlagModel::Off => "create:block/track_station/flag_off",
            FlagModel::Assemble => "create:block/track_station/flag_assemble",
        }
    }
}

/// `!isAssembling || (station == null || station.getPresentTrain() != null) && !be.isVirtual()` picks the on/off flag; otherwise the
/// assembling one.
pub fn flag_model(assembling: bool, train_present: bool, value: f32) -> FlagModel {
    if !assembling || train_present {
        if value > 0.75 { FlagModel::On } else { FlagModel::Off }
    } else {
        FlagModel::Assemble
    }
}

/// `transformFlag`'s `progress`: the lift squared, with the wiggle once it is up while it still settles.
pub fn flag_progress(value: f32, chase_target: f32, settled: bool) -> f32 {
    let mut progress = (value * 5.0).min(1.0).powi(2);
    if chase_target > 0.0 && !settled && progress == 1.0 {
        let wiggle = (value - 0.2) / 0.8;
        progress += ((wiggle * (2.0 * std::f32::consts::PI) * 4.0).sin() / 8.0) / (8.0 * wiggle).max(1.0);
    }
    progress
}

/// The way the flag faces: `resolveFlagAngle`'s `flagYRot` and `flagFlipped`. `axis` is the target track's first axis times the
/// direction's step; `to_track` the track block's offset from the station block.
pub fn flag_angle(axis: Vec3, to_track: [i32; 3]) -> (i32, bool) {
    // `Direction.getNearest(axis.x, 0, axis.z)`: the first of down, up, north, south, west, east with the greatest dot (strictly
    // greater wins). Its `toYRot()` (south 0, west 90, north 180, east 270) and its `getClockWise()` normal.
    let candidates: [([f64; 2], i32, [i32; 2]); 4] =
        [([0.0, -1.0], 180, [1, 0]), ([0.0, 1.0], 0, [-1, 0]), ([-1.0, 0.0], 90, [0, -1]), ([1.0, 0.0], 270, [0, 1])];
    let mut best = candidates[0];
    let mut best_dot = f64::NEG_INFINITY;
    for c in candidates {
        let dot = axis[0] * c.0[0] + axis[2] * c.0[1];
        if dot > best_dot {
            best_dot = dot;
            best = c;
        }
    }
    let flag_y_rot = -best.1 - 90;
    let (dx, dz) = (to_track[0], to_track[2]);
    if dx == 0 && dz == 0 {
        return (flag_y_rot, false);
    }
    (flag_y_rot, dx * best.2[0] + dz * best.2[1] > 0)
}

/// `transformFlag` then `renderFlag`'s extra turn, as the pose in the station block's frame.
pub fn flag_chain(y_rot: i32, flipped: bool, progress: f32) -> Chain {
    let nudge = 1.0 / 512.0;
    let angle = f64::from((if flipped { 1.0 } else { -1.0 }) * (progress * 90.0 + 270.0));
    Chain::new()
        .center()
        .rotate_y(f64::from(y_rot).to_radians())
        .translate([nudge, 9.5 / 16.0, if flipped { 14.0 / 16.0 - nudge } else { 2.0 / 16.0 + nudge }])
        .uncenter()
        .rotate_x(angle.to_radians())
        .translate([0.5 / 16.0, 0.0, 0.0])
        .rotate_y(if flipped { 0.0 } else { std::f64::consts::PI })
        .translate([-0.5 / 16.0, 0.0, 0.0])
}

// --- picking a point on a curve ----------------------------------------------------------------------------------------------------

/// `AABB.clip(from, to)`: where the segment first enters the box, `None` when it misses or starts inside.
pub fn clip_box(lo: Vec3, hi: Vec3, from: Vec3, to: Vec3) -> Option<Vec3> {
    let d = sub(to, from);
    let (mut near, mut far) = (0.0_f64, 1.0_f64);
    for i in 0..3 {
        if d[i].abs() < 1e-7 {
            if from[i] < lo[i] || from[i] > hi[i] {
                return None;
            }
            continue;
        }
        let (a, b) = ((lo[i] - from[i]) / d[i], (hi[i] - from[i]) / d[i]);
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        near = near.max(a);
        far = far.min(b);
        if near > far {
            return None;
        }
    }
    // Inside to begin with: vanilla finds no face to enter.
    if near <= 0.0 {
        return None;
    }
    Some(add(from, [d[0] * near, d[1] * near, d[2] * near]))
}

/// `TrackBlockOutline.pickCurves`'s `segmentBounds`: `AllShapes.TRACK_ORTHO` (south) moved by `(-.5, -ysize / 2, -.5)`.
pub const SEGMENT_BOUNDS: [Vec3; 2] = [[-1.375, -0.125, -0.5], [1.375, 0.125, 0.5]];

fn turn_x(v: Vec3, degrees: f64) -> Vec3 {
    let (s, c) = degrees.to_radians().sin_cos();
    [v[0], v[1] * c - v[2] * s, v[2] * c + v[1] * s]
}

fn turn_y(v: Vec3, degrees: f64) -> Vec3 {
    let (s, c) = degrees.to_radians().sin_cos();
    [v[0] * c + v[2] * s, v[1], v[2] * c - v[0] * s]
}

fn distance_sqr(a: Vec3, b: Vec3) -> f64 {
    let d = sub(a, b);
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
}

/// `BezierPointSelection`: the segment the cursor is on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurvePick {
    /// The block entity that holds the curve.
    pub block: Pos,
    /// `BezierTrackPointLocation.curveTarget`: the curve's other end, its key.
    pub curve: Pos,
    pub segment: i32,
    /// `vec`: the middle of the segment.
    pub anchor: Vec3,
    /// `angles` (pitch, yaw, roll in radians).
    pub angles: Vec3,
    /// `direction`: the segment's way along the curve.
    pub direction: Vec3,
}

/// The nearest of `pick_curve`s over the curves: `max_range_sqr` is `maxRange` (the squared distance to the aimed block, or large),
/// `origin` and `target` the eye and the end of the trace.
pub fn pick_curves(curves: &[(Pos, &BezierConnection)], origin: Vec3, target: Vec3, mut max_range_sqr: f64) -> Option<CurvePick> {
    let mut result = None;
    for (block, bc) in curves {
        if !bc.is_primary() {
            continue;
        }
        let [lo, hi] = bc.bounds();
        let inside = (0..3).all(|i| origin[i] >= lo[i] && origin[i] <= hi[i]);
        if !inside && clip_box(lo, hi, origin, target).is_none() {
            continue;
        }
        let steps = bc.step_lut().len();
        let mut best: Option<(CurvePick, f64)> = None;
        let mut new_max = max_range_sqr;
        for i in 0..steps.saturating_sub(2) {
            let t = f64::from(bc.segment_t(i));
            let t1 = f64::from(bc.segment_t(i + 1));
            let t2 = f64::from(bc.segment_t(i + 2));
            let (v1, v2) = (bc.position(t), bc.position(t2));
            let diff = sub(v2, v1);
            let angles = model_angles(bc.normal(t1), diff);
            let anchor = add(v1, [diff[0] * 0.5, diff[1] * 0.5, diff[2] * 0.5]);
            let mut local_origin = sub(origin, anchor);
            let mut local_dir = sub(target, origin);
            for v in [&mut local_origin, &mut local_dir] {
                *v = turn_x(*v, -angles[0].to_degrees());
                *v = turn_y(*v, -angles[1].to_degrees());
            }
            let Some(clip) = clip_box(SEGMENT_BOUNDS[0], SEGMENT_BOUNDS[1], local_origin, add(local_origin, local_dir)) else { continue };
            let from_middle = distance_sqr(clip, [0.0, f64::from(0.25_f32), 0.0]);
            if best.is_some_and(|(_, d)| d < from_middle) {
                continue;
            }
            let distance = distance_sqr(clip, local_origin);
            if distance > max_range_sqr {
                continue;
            }
            new_max = distance;
            best = Some((
                CurvePick { block: *block, curve: bc.key(), segment: i as i32, anchor, angles, direction: normalize(diff) },
                from_middle,
            ));
        }
        if let Some((pick, _)) = best {
            result = Some(pick);
            max_range_sqr = new_max;
        }
    }
    result
}

/// Whether the look is against the segment's way (`lookAngle.dot(selection.direction()) < 0`), the `front` of a curve click.
pub fn curve_front(look: Vec3, direction: Vec3) -> bool {
    look[0] * direction[0] + look[1] * direction[1] + look[2] * direction[2] < 0.0
}

/// The edges of `AllShapes.TRACK_ORTHO` (south), the box `[-14, 0, 0]` to `[30, 4, 16]` sixteenths, as `drawCurveSelection` draws it
/// under `translate(anchor + (0, .125, 0)).rotateY(yaw).rotateX(pitch).translate(-.5, -.125, -.5)`: twelve lines in world space.
pub fn curve_outline(pick: &CurvePick) -> Vec<[Vec3; 2]> {
    let lo = [-14.0 / 16.0, 0.0, 0.0];
    let hi = [30.0 / 16.0, 4.0 / 16.0, 1.0];
    let pose = Affine::IDENTITY
        .translate(add(pick.anchor, [0.0, 0.125, 0.0]))
        .rotate_y(pick.angles[1])
        .rotate_x(pick.angles[0])
        .translate([-0.5, -0.125, -0.5]);
    let corner = |i: usize| pose.apply([if i & 1 == 0 { lo[0] } else { hi[0] }, if i & 2 == 0 { lo[1] } else { hi[1] }, if i & 4 == 0 { lo[2] } else { hi[2] }]);
    const EDGES: [[usize; 2]; 12] = [[0, 1], [2, 3], [4, 5], [6, 7], [0, 2], [1, 3], [4, 6], [5, 7], [0, 4], [1, 5], [2, 6], [3, 7]];
    EDGES.iter().map(|[a, b]| [corner(*a), corner(*b)]).collect()
}

/// `renderShape`'s colour: black at `.4` alpha for a plain hover, the reddish `(1, .25, .125)` while holding track (`valid` false).
pub const OUTLINE_PLAIN: u32 = 0x6600_0000;
pub const OUTLINE_TRACK_HELD: u32 = 0x66FF_4020;

// --- the relocation preview ------------------------------------------------------------------------------------------------------

/// `TrainRelocator.relocateClient`'s lines: between each two recorded wheel positions, at `-.925` blocks, green (`0x95CD41`), the last
/// one orange (`0xEA5C2B`) when the relocation isn't valid; alternate pieces thinner (`1 / 6` against `1 / 4` of a block).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreviewLine {
    pub from: Vec3,
    pub to: Vec3,
    pub colour: u32,
    /// In blocks, as `Outliner.lineWidth`.
    pub width: f32,
}

pub const PREVIEW_GOOD: u32 = 0x95CD41;
pub const PREVIEW_BAD: u32 = 0xEA5C2B;

pub fn preview_lines(points: &[Vec3], valid: bool, upside_down: bool) -> Vec<PreviewLine> {
    let offset = if upside_down { [0.0, -0.5, 0.0] } else { [0.0; 3] };
    let n = points.len();
    (0..n.saturating_sub(1))
        .map(|i| PreviewLine {
            from: add(add(points[i], offset), [0.0, f64::from(-0.925_f32), 0.0]),
            to: add(add(points[i + 1], offset), [0.0, f64::from(-0.925_f32), 0.0]),
            colour: if valid || i != n - 2 { PREVIEW_GOOD } else { PREVIEW_BAD },
            width: if i % 2 == 1 { 1.0 / 6.0 } else { 1.0 / 4.0 },
        })
        .collect()
}

// --- what the client and the server say to each other -------------------------------------------------------------------------

/// Server to client: `[1]` a relocation session begins for the player, `[0]` it ends. (Create's session is the client's.)
pub const CHANNEL_SESSION: &str = "create:relocation";
/// Client to server: the track the cursor is on while relocating ([`Hover`]).
pub const CHANNEL_HOVER: &str = "create:relocation_hover";
/// Server to client: what the train would do there ([`Preview`]).
pub const CHANNEL_PREVIEW: &str = "create:relocation_preview";
/// Client to server: a click on a curve's segment ([`CurveClick`]; Create's `CurvedTrackSelectionPacket` and the relocation packet).
pub const CHANNEL_CURVE: &str = "create:curve_click";

fn put_pos(out: &mut Vec<u8>, p: Pos) {
    for v in p {
        out.extend_from_slice(&v.to_be_bytes());
    }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let (head, rest) = self.0.split_at_checked(N)?;
        self.0 = rest;
        head.try_into().ok()
    }
    fn u8(&mut self) -> Option<u8> {
        self.take::<1>().map(|b| b[0])
    }
    fn i32(&mut self) -> Option<i32> {
        self.take::<4>().map(i32::from_be_bytes)
    }
    fn f32(&mut self) -> Option<f32> {
        self.take::<4>().map(f32::from_be_bytes)
    }
    fn f64(&mut self) -> Option<f64> {
        self.take::<8>().map(f64::from_be_bytes)
    }
    fn u16(&mut self) -> Option<u16> {
        self.take::<2>().map(u16::from_be_bytes)
    }
    fn pos(&mut self) -> Option<Pos> {
        Some([self.i32()?, self.i32()?, self.i32()?])
    }
}

/// `relocateClient`'s input: the track block the cursor is on, how the player looks (`getNearestTrackAxis`), and for a curve its
/// key and segment with `front` (`lookAngle.dot(direction) < 0`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hover {
    pub pos: Pos,
    pub look: [f32; 3],
    pub front: bool,
    pub bezier: Option<(Pos, i32)>,
}

impl Hover {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![u8::from(self.front) | (u8::from(self.bezier.is_some()) << 1)];
        put_pos(&mut out, self.pos);
        for v in self.look {
            out.extend_from_slice(&v.to_be_bytes());
        }
        if let Some((curve, segment)) = self.bezier {
            put_pos(&mut out, curve);
            out.extend_from_slice(&segment.to_be_bytes());
        }
        out
    }

    pub fn decode(data: &[u8]) -> Option<Hover> {
        let mut r = Reader(data);
        let flags = r.u8()?;
        let pos = r.pos()?;
        let look = [r.f32()?, r.f32()?, r.f32()?];
        if look.iter().any(|v| !v.is_finite()) {
            return None;
        }
        let bezier = if flags & 2 != 0 { Some((r.pos()?, r.i32()?)) } else { None };
        Some(Hover { pos, look, front: flags & 1 != 0, bezier })
    }
}

/// The server's answer to a [`Hover`]: whether the train fits, the wheels' places, and whether the train hangs (its first bogey is
/// upside down).
#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    pub valid: bool,
    pub upside_down: bool,
    pub points: Vec<Vec3>,
}

/// The most places a preview carries (a train of 32 carriages has 128 wheels).
pub const MAX_PREVIEW_POINTS: usize = 256;

impl Preview {
    pub fn encode(&self) -> Vec<u8> {
        let points = &self.points[..self.points.len().min(MAX_PREVIEW_POINTS)];
        let mut out = vec![u8::from(self.valid) | (u8::from(self.upside_down) << 1)];
        out.extend_from_slice(&(points.len() as u16).to_be_bytes());
        for p in points {
            for v in p {
                out.extend_from_slice(&v.to_be_bytes());
            }
        }
        out
    }

    pub fn decode(data: &[u8]) -> Option<Preview> {
        let mut r = Reader(data);
        let flags = r.u8()?;
        let n = usize::from(r.u16()?);
        if n > MAX_PREVIEW_POINTS {
            return None;
        }
        let mut points = Vec::with_capacity(n);
        for _ in 0..n {
            let p = [r.f64()?, r.f64()?, r.f64()?];
            if p.iter().any(|v| !v.is_finite()) {
                return None;
            }
            points.push(p);
        }
        Some(Preview { valid: flags & 1 != 0, upside_down: flags & 2 != 0, points })
    }
}

/// What a click on a curve is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurveUse {
    /// The held Station, Signal or Observer picks the spot (`CurvedTrackSelectionPacket`).
    Select,
    /// The Wrench puts the relocating train there (`TrainRelocationPacket` with a `BezierTrackPointLocation`).
    Relocate,
}

/// A click on a segment of a curve (`BezierPointSelection`): the block entity that holds it, the curve's key, the segment and the
/// side the player looks from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CurveClick {
    pub purpose: CurveUse,
    pub block: Pos,
    pub curve: Pos,
    pub segment: i32,
    pub front: bool,
}

impl CurveClick {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![u8::from(self.front) | (u8::from(self.purpose == CurveUse::Relocate) << 1)];
        put_pos(&mut out, self.block);
        put_pos(&mut out, self.curve);
        out.extend_from_slice(&self.segment.to_be_bytes());
        out
    }

    pub fn decode(data: &[u8]) -> Option<CurveClick> {
        let mut r = Reader(data);
        let flags = r.u8()?;
        Some(CurveClick {
            purpose: if flags & 2 != 0 { CurveUse::Relocate } else { CurveUse::Select },
            block: r.pos()?,
            curve: r.pos()?,
            segment: r.i32()?,
            front: flags & 1 != 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-5)
    }

    #[test]
    fn a_turn_about_the_centre_keeps_the_centre() {
        let a = Affine::IDENTITY.rotate_centered_y(std::f64::consts::PI);
        assert!(close(a.apply([0.5, 0.3, 0.5]), [0.5, 0.3, 0.5]));
        assert!(close(a.apply([0.0, 0.0, 0.0]), [1.0, 0.0, 1.0]));
    }

    #[test]
    fn operations_act_in_the_frame_the_earlier_ones_made() {
        // Turn a quarter about y, then move 1 along the new z: the point ends up along +x.
        let a = Affine::IDENTITY.rotate_y(std::f64::consts::FRAC_PI_2).translate([0.0, 0.0, 1.0]);
        assert!(close(a.apply([0.0; 3]), [1.0, 0.0, 0.0]));
        let rows = a.rows();
        assert!((rows[3] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_flat_track_overlay_sits_on_the_track_facing_the_way_it_points() {
        let axis = [0.0, 0.0, 1.0];
        let up = [0.0, 1.0, 0.0];
        let pose = overlay_pose(OverlaySpot::Straight { axis, up }, [0, 64, 0], true, 1.0).unwrap();
        // Along z with up on y there is nothing to turn, and a flat track adds no shift.
        assert!(close(pose.apply([0.0, 0.0, 0.0]), [0.0, 0.0, 0.0]));
        // Pointing the other way turns the model a half turn about its own axis (yaw pi), about the block's centre.
        let back = overlay_pose(OverlaySpot::Straight { axis, up }, [0, 64, 0], false, 1.0).unwrap();
        let p = back.apply([0.25, 0.0, 0.0]);
        assert!((p[0] - 0.75).abs() < 1e-6 && (p[2] - 1.0).abs() < 1e-6, "{p:?}");
    }

    #[test]
    fn the_held_overlay_is_a_sixteenth_larger_about_the_middle() {
        let axis = [0.0, 0.0, 1.0];
        let pose = overlay_pose(OverlaySpot::Straight { axis, up: [0.0, 1.0, 0.0] }, [0, 64, 0], true, HELD_SCALE).unwrap();
        // The middle of the bottom stays; the edge moves out by half a sixteenth.
        assert!(close(pose.apply([0.5, 0.0, 0.5]), [0.5, 0.0, 0.5]));
        assert!((pose.apply([1.0, 0.0, 0.5])[0] - (0.5 + 0.5 * HELD_SCALE)).abs() < 1e-5);
    }

    #[test]
    fn a_slope_lifts_the_overlay_by_its_axis() {
        let axis = normalize([0.0, 1.0, 1.0]);
        let flat = overlay_pose(OverlaySpot::Straight { axis: [0.0, 0.0, 1.0], up: [0.0, 1.0, 0.0] }, [0, 0, 0], true, 1.0).unwrap();
        let sloped = overlay_pose(OverlaySpot::Straight { axis, up: [0.0, 1.0, 0.0] }, [0, 0, 0], true, 1.0).unwrap();
        assert_ne!(flat, sloped);
    }

    fn quarter_curve() -> BezierConnection {
        BezierConnection::new(
            [[0, 64, 0], [8, 64, 8]],
            [[0.5, 64.0, 1.0], [8.5, 64.0, 7.0]],
            [[0.0, 0.0, 1.0], [0.0, 0.0, -1.0]],
            [[0.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
            true,
            false,
            "create:andesite",
        )
    }

    #[test]
    fn a_curve_overlay_follows_the_curve() {
        let bc = quarter_curve();
        let pose = overlay_pose(OverlaySpot::Curve { curve: &bc, segment: 3 }, [0, 64, 0], true, 1.0).unwrap();
        let other = overlay_pose(OverlaySpot::Curve { curve: &bc, segment: 6 }, [0, 64, 0], true, 1.0).unwrap();
        assert_ne!(pose.t, other.t);
        // The model's origin is a few tenths of a block off the block's, along the curve, and below the track's surface plus 4 / 16.
        assert!(pose.t.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn assembly_spots_are_blue_where_a_bogey_may_go_and_green_where_one_is() {
        let spots = assembly_spots(8, &[0, 4], |i| crate::rules::trains::station::is_valid_bogey_offset(i, &[0, 4]));
        // Offset 0 holds a bogey; 1 and 2 are too close to it; 3 is within 2 of the one at 4; 7 is the first free.
        assert_eq!(spots[0], AssemblySpot { step: 0, colour: COLOUR_CARRIAGE });
        assert!(spots.iter().any(|s| s.step == 4 && s.colour == COLOUR_CARRIAGE));
        assert!(spots.iter().any(|s| s.step == 7 && s.colour == COLOUR_VALID));
        assert!(!spots.iter().any(|s| s.step == 2));
    }

    #[test]
    fn the_assembly_runs_one_block_on_for_each_step() {
        let south = assembly_chain([0, 0, 1], 0).affine();
        assert!(close(south.apply([0.0; 3]), [0.0, 0.0, 1.0]));
        let east = assembly_chain([1, 0, 0], 2).affine();
        // East turns a quarter: local +z becomes +x, so step 2 is three blocks east.
        assert!((east.apply([0.5, 0.0, 0.5])[0] - 3.5).abs() < 1e-6, "{:?}", east.apply([0.5, 0.0, 0.5]));
    }

    #[test]
    fn the_flag_rises_a_tenth_a_tick_and_waits_to_settle_before_turning_back() {
        let flag = Flag::lowered().follow(true, 100.0);
        assert_eq!(flag.target, 1.0);
        assert!((flag.value_at(105.0) - 0.5).abs() < 1e-6);
        assert!(flag.settled_at(110.0));
        assert_eq!(flag.value_at(500.0), 1.0);
        // Asked to lower midway: not until it has settled.
        assert_eq!(flag.follow(false, 104.0), flag);
        let lowered = flag.follow(false, 111.0);
        assert_eq!(lowered.target, 0.0);
        assert!((lowered.value_at(111.0) - 1.0).abs() < 1e-6);
        assert!((lowered.value_at(116.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_flag_model_depends_on_assembling_and_the_train() {
        assert_eq!(flag_model(false, false, 0.0), FlagModel::Off);
        assert_eq!(flag_model(false, true, 1.0), FlagModel::On);
        assert_eq!(flag_model(true, false, 0.5), FlagModel::Assemble);
        assert_eq!(flag_model(true, true, 0.5), FlagModel::Off);
        assert!(flag_wanted(false, true) && flag_wanted(true, false) && !flag_wanted(false, false));
    }

    #[test]
    fn the_flag_lifts_squared_and_wiggles_while_it_settles() {
        assert_eq!(flag_progress(0.1, 1.0, false), 0.25);
        assert_eq!(flag_progress(0.2, 1.0, false), 1.0);
        assert_eq!(flag_progress(0.6, 1.0, true), 1.0);
        assert!((flag_progress(0.45, 1.0, false) - 1.05).abs() < 1e-3);
    }

    #[test]
    fn the_flag_faces_along_the_track_and_flips_to_the_other_side() {
        // A track along +z (south): `toYRot` 0, so the flag turns -90. The track on the west side of the station block.
        let (rot, flipped) = flag_angle([0.0, 0.0, 1.0], [-1, 0, 0]);
        assert_eq!(rot, -90);
        // `getClockWise` of south is west: the track to the west is on the clockwise side.
        assert!(flipped);
        let (_, flipped) = flag_angle([0.0, 0.0, 1.0], [1, 0, 0]);
        assert!(!flipped);
        // East: `toYRot` 270.
        assert_eq!(flag_angle([1.0, 0.0, 0.0], [0, 0, 0]), (-360, false));
        assert_eq!(flag_angle([-1.0, 0.0, 0.0], [0, 0, 1]).0, -180);
    }

    #[test]
    fn a_ray_enters_a_box_and_a_start_inside_finds_no_face() {
        let hit = clip_box([0.0; 3], [1.0; 3], [-1.0, 0.5, 0.5], [2.0, 0.5, 0.5]).unwrap();
        assert!(close(hit, [0.0, 0.5, 0.5]));
        assert!(clip_box([0.0; 3], [1.0; 3], [0.5; 3], [2.0, 0.5, 0.5]).is_none());
        assert!(clip_box([0.0; 3], [1.0; 3], [-1.0, 2.0, 0.5], [2.0, 2.0, 0.5]).is_none());
    }

    #[test]
    fn the_cursor_picks_a_segment_of_the_curve_it_points_at() {
        let bc = quarter_curve();
        let curves = vec![([0, 64, 0], &bc)];
        // Aim at the middle of the curve from above.
        let mid = bc.position(0.5);
        let origin = add(mid, [0.0, 3.0, 0.0]);
        let target = add(mid, [0.0, -2.0, 0.0]);
        let pick = pick_curves(&curves, origin, target, f64::MAX).expect("a segment");
        assert_eq!(pick.block, [0, 64, 0]);
        assert_eq!(pick.curve, bc.key());
        assert!(distance_sqr(pick.anchor, mid) < 1.5, "{:?} vs {mid:?}", pick.anchor);
        // Something nearer than the curve hides it.
        assert!(pick_curves(&curves, origin, target, 0.5).is_none());
        // Pointing away finds nothing.
        assert!(pick_curves(&curves, origin, add(origin, [0.0, 5.0, 0.0]), f64::MAX).is_none());
        // The outline is a box of twelve edges about the anchor.
        assert_eq!(curve_outline(&pick).len(), 12);
    }

    #[test]
    fn the_secondary_end_of_a_curve_is_not_picked() {
        let bc = quarter_curve().secondary();
        let mid = bc.position(0.5);
        let curves = vec![([8, 64, 8], &bc)];
        assert!(pick_curves(&curves, add(mid, [0.0, 3.0, 0.0]), add(mid, [0.0, -2.0, 0.0]), f64::MAX).is_none());
    }

    #[test]
    fn the_front_of_a_curve_click_is_against_the_look() {
        assert!(curve_front([0.0, 0.0, 1.0], [0.0, 0.0, -1.0]));
        assert!(!curve_front([0.0, 0.0, 1.0], [0.0, 0.0, 1.0]));
    }

    #[test]
    fn the_preview_is_green_with_the_last_piece_orange_when_it_fails() {
        let points = [[0.0, 64.0, 0.0], [0.0, 64.0, 2.0], [0.0, 64.0, 4.0]];
        let ok = preview_lines(&points, true, false);
        assert_eq!(ok.len(), 2);
        assert!(ok.iter().all(|l| l.colour == PREVIEW_GOOD));
        let bad = preview_lines(&points, false, false);
        assert_eq!((bad[0].colour, bad[1].colour), (PREVIEW_GOOD, PREVIEW_BAD));
        assert_eq!((bad[0].width, bad[1].width), (0.25, 1.0 / 6.0));
        assert!((bad[0].from[1] - (64.0 - 0.925)).abs() < 1e-6);
        // Upside down trains hang half a block lower.
        assert!((preview_lines(&points, true, true)[0].from[1] - (64.0 - 0.925 - 0.5)).abs() < 1e-6);
        assert!(preview_lines(&points[..1], true, false).is_empty());
    }

    #[test]
    fn the_messages_survive_the_wire() {
        let hover = Hover { pos: [3, -64, 12], look: [0.25, -0.5, 0.75], front: true, bezier: None };
        assert_eq!(Hover::decode(&hover.encode()), Some(hover));
        let on_curve = Hover { bezier: Some(([-9, 70, 4], 17)), front: false, ..hover };
        assert_eq!(Hover::decode(&on_curve.encode()), Some(on_curve));
        assert_eq!(Hover::decode(&on_curve.encode()[..10]), None);
        let nan = Hover { look: [f32::NAN, 0.0, 0.0], ..hover };
        assert_eq!(Hover::decode(&nan.encode()), None);

        let preview = Preview { valid: false, upside_down: true, points: vec![[1.5, 64.0, 2.5], [1.5, 64.0, 4.5]] };
        assert_eq!(Preview::decode(&preview.encode()), Some(preview.clone()));
        assert_eq!(Preview::decode(&[0, 255, 255]), None);
        let long = Preview { valid: true, upside_down: false, points: vec![[0.0; 3]; MAX_PREVIEW_POINTS + 10] };
        assert_eq!(Preview::decode(&long.encode()).unwrap().points.len(), MAX_PREVIEW_POINTS);

        let click = CurveClick { purpose: CurveUse::Relocate, block: [1, 2, 3], curve: [40, 2, -9], segment: 11, front: true };
        assert_eq!(CurveClick::decode(&click.encode()), Some(click));
        let select = CurveClick { purpose: CurveUse::Select, front: false, ..click };
        assert_eq!(CurveClick::decode(&select.encode()), Some(select));
        assert_eq!(CurveClick::decode(&[]), None);
    }
}
