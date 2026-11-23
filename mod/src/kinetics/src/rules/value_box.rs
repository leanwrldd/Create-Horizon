//! Create's value boxes: where the small in-world box of a block sits, which face of a block it is
//! on, and whether a ray from the player's eye lands on it (`ValueBoxTransform`,
//! `CenteredSideValueBoxTransform`, and the slot class each block has: `SmartChuteFilterSlotPositioning`,
//! `FunnelFilterSlotPositioning`, `BrassTunnelFilterSlot`, `BrassTunnelModeSlot`, `SawFilterSlot`,
//! `DeployerFilterSlot`, `CreativeMotorBlockEntity.MotorValueBox`,
//! `SpeedControllerBlockEntity.ControllerValueBoxTransform`, `ArmBlockEntity.SelectionModeValueBox`,
//! `FilteredDetectorFilterSlot`, `SmartFluidPipeBlockEntity.SmartPipeFilterSlot`).
//!
//! Everything here is plain numbers: a slot answers from the block's state properties (`Props`)
//! and the glue (`native/api/src/machines/value_box*.rs`) only reads the world and draws. The
//! boxes a block has are in `value_box_blocks`, the value board in `value_box_board`.
//!
//! Known differences from Create, for the README:
//! - `Dual` transforms (`ValueBoxTransform.Dual`, two slots on one face) aren't ported: no block of this
//!   port has one yet (the Redstone Link's frequency slots keep their own rule in `redstone_link.rs`).

use crate::{Axis, Dir};

/// A point or vector, in blocks.
pub type Vec3 = [f64; 3];

/// `ValueBoxTransform.getScale`: the box is half a block wide, and a hit within half of that of its centre
/// counts (`testHit`: `localHit.distanceTo(offset) < scale / 2`).
pub const SCALE: f64 = 0.5;

/// `VecHelper.voxelSpace`: texels (16 to a block) to blocks.
pub fn voxel_space(x: f64, y: f64, z: f64) -> Vec3 {
    [x / 16.0, y / 16.0, z / 16.0]
}

/// `VecHelper.rotate`: `vec` turned `degrees` about the origin along `axis`, right-handed as a `PoseStack`
/// turns (+y about y takes +z towards +x).
pub fn rotate(vec: Vec3, degrees: f64, axis: Axis) -> Vec3 {
    if degrees == 0.0 {
        return vec;
    }
    let (sin, cos) = degrees.to_radians().sin_cos();
    let [x, y, z] = vec;
    match axis {
        Axis::X => [x, y * cos - z * sin, z * cos + y * sin],
        Axis::Y => [x * cos + z * sin, y, z * cos - x * sin],
        Axis::Z => [x * cos - y * sin, y * cos + x * sin, z],
    }
}

/// `VecHelper.rotateCentered`: the same turn about the block's centre.
pub fn rotate_centered(vec: Vec3, degrees: f64, axis: Axis) -> Vec3 {
    let c = [0.5; 3];
    let turned = rotate([vec[0] - c[0], vec[1] - c[1], vec[2] - c[2]], degrees, axis);
    [turned[0] + c[0], turned[1] + c[1], turned[2] + c[2]]
}

/// `AngleHelper.horizontalAngle`: south 0, east 90, north 180, west 270, 0 for up and down.
pub fn horizontal_angle(facing: Dir) -> f64 {
    match facing {
        Dir::South | Dir::Up | Dir::Down => 0.0,
        Dir::East => 90.0,
        Dir::North => 180.0,
        Dir::West => 270.0,
    }
}

/// `AngleHelper.verticalAngle`: up -90, down 90, 0 for the sides.
pub fn vertical_angle(facing: Dir) -> f64 {
    match facing {
        Dir::Up => -90.0,
        Dir::Down => 90.0,
        _ => 0.0,
    }
}

fn is_horizontal(dir: Dir) -> bool {
    dir.axis() != Axis::Y
}

fn normal(dir: Dir) -> Vec3 {
    let n = dir.normal();
    [f64::from(n[0]), f64::from(n[1]), f64::from(n[2])]
}

// --- the block's state, as a slot reads it ---------------------------------------------------------

/// The state properties of the block a slot sits on (`facing`, `axis`, `shape`...), as text, and the facts the
/// glue works out from the world and the block entity (keys starting `fact:`, `"true"` or `"false"`). A
/// closure keeps the rules free of the game's types.
pub struct Props<'a> {
    pub block: &'a str,
    get: &'a dyn Fn(&str) -> Option<String>,
}

impl<'a> Props<'a> {
    pub fn new(block: &'a str, get: &'a dyn Fn(&str) -> Option<String>) -> Self {
        Self { block, get }
    }

    pub fn text(&self, key: &str) -> Option<String> {
        (self.get)(key)
    }

    pub fn is(&self, key: &str, value: &str) -> bool {
        self.text(key).as_deref() == Some(value)
    }

    pub fn dir(&self, key: &str) -> Option<Dir> {
        self.text(key).as_deref().and_then(Dir::from_name)
    }

    pub fn axis(&self, key: &str) -> Option<Axis> {
        self.text(key).as_deref().and_then(Axis::from_name)
    }

    pub fn flag(&self, key: &str) -> bool {
        self.is(key, "true")
    }
}

/// `DirectionalAxisKineticBlock.getRotationAxis`: the shaft's axis of a block that faces one way and has
/// `axis_along_first_coordinate` (Create's saw, drill, deployer, harvester).
pub fn directional_rotation_axis(facing: Dir, along_first: bool) -> Axis {
    match (facing.axis(), along_first) {
        (Axis::X, true) => Axis::Y,
        (Axis::X, false) => Axis::Z,
        (Axis::Y, true) => Axis::X,
        (Axis::Y, false) => Axis::Z,
        (Axis::Z, true) => Axis::X,
        (Axis::Z, false) => Axis::Y,
    }
}

/// `DirectedDirectionalBlock.getTargetDirection`: the way a smart observer or a stockpile switch looks.
pub fn target_direction(target: &str, facing: Dir) -> Dir {
    match target {
        "ceiling" => Dir::Up,
        "floor" => Dir::Down,
        _ => facing,
    }
}

// --- the slots -------------------------------------------------------------------------------------

/// Which sides a `CenteredSideValueBoxTransform` is active on (its `BiPredicate<BlockState, Direction>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sides {
    /// Every side (the default).
    Any,
    /// Only the top (`BrassTunnelModeSlot`).
    Top,
    /// The four horizontal ones (`ArmBlockEntity.SelectionModeValueBox`).
    Horizontal,
}

/// One value box position: the Create class that places it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// `SmartChuteFilterSlotPositioning`.
    SmartChute,
    /// `FunnelFilterSlotPositioning` (funnels and belt funnels).
    Funnel,
    /// `BrassTunnelFilterSlot`: one on each open side.
    BrassTunnelFilter,
    /// `BrassTunnelModeSlot`: centred on top, tinted red.
    BrassTunnelMode,
    /// `SawFilterSlot`: on top of an upright saw only.
    Saw,
    /// `DeployerFilterSlot`.
    Deployer,
    /// `CreativeMotorBlockEntity.MotorValueBox`.
    Motor,
    /// `SpeedControllerBlockEntity.ControllerValueBoxTransform`.
    SpeedController,
    /// `ArmBlockEntity.SelectionModeValueBox`.
    Arm,
    /// `FilteredDetectorFilterSlot(hasSlotAtBottom)`: the stockpile switch has one at the bottom, the smart
    /// observer doesn't.
    FilteredDetector { bottom: bool },
    /// `SmartFluidPipeBlockEntity.SmartPipeFilterSlot`.
    SmartPipe,
    /// `RollerBlockEntity.RollerValueBox(hOffset)`: on top of the block, 3 pixels to one side of its middle for the
    /// filter (`hOffset` 3) and 3 to the other for the mode (-3), turned with the block's facing.
    Roller { h_offset: i32 },
    /// `TrackObserverBlockEntity.createFilter`'s transform: the middle of the top face (`(0.5, 15.5 / 16, 0.5)`), turned
    /// 90 degrees about x. Not sided.
    TrackObserver,
    /// `CenteredSideValueBoxTransform(allowed)`, with the plain default of (8, 8, 15.5).
    Centered(Sides),
}

/// `Sided.getSouthLocation` and `Sided.getLocalOffset`: the slot's place on the south face, turned to `side`.
fn sided_offset(south: Vec3, side: Dir) -> Vec3 {
    let turned = rotate_centered(south, horizontal_angle(side), Axis::Y);
    rotate_centered(turned, vertical_angle(side), Axis::X)
}

/// `Sided.rotate`: the turn that puts the box's face on `side`.
fn sided_rotation(side: Dir) -> Vec<(Axis, f64)> {
    let y = horizontal_angle(side) + 180.0;
    let x = match side {
        Dir::Up => 90.0,
        Dir::Down => 270.0,
        _ => 0.0,
    };
    vec![(Axis::Y, y), (Axis::X, x)]
}

impl Slot {
    /// Whether the box depends on the face of the block that was looked at (`ValueBoxTransform.Sided`).
    pub fn sided(self) -> bool {
        !matches!(self, Slot::Saw | Slot::SmartPipe | Slot::Roller { .. } | Slot::TrackObserver)
    }

    /// `ValueBoxTransform.getScale`.
    pub fn scale(self) -> f64 {
        match self {
            // `SmartPipeFilterSlot.getScale`: `super.getScale() * 1.02f`.
            Slot::SmartPipe => SCALE * 1.02,
            _ => SCALE,
        }
    }

    /// What `testHit` divides the scale by: 2, and 3 for the Roller's (`RollerValueBox.testHit`:
    /// `distanceTo(offset) < scale / 3`), whose two boxes sit close together.
    fn hit_divisor(self) -> f64 {
        match self {
            Slot::Roller { .. } => 3.0,
            _ => 2.0,
        }
    }

    /// `ValueBoxTransform.getOverrideColor`: the tint of the box's icon or text, if it has one
    /// (`BrassTunnelModeSlot`'s dark red).
    pub fn override_color(self) -> Option<u32> {
        match self {
            Slot::BrassTunnelMode => Some(0x59_2424),
            _ => None,
        }
    }

    /// `Sided.isSideActive`: whether the box is on `side` of this block.
    pub fn side_active(self, p: &Props, side: Dir) -> bool {
        match self {
            Slot::SmartChute => is_horizontal(side),
            Slot::Funnel => {
                let Some(facing) = p.dir("facing") else {
                    return false;
                };
                if !is_horizontal(facing) {
                    return is_horizontal(side);
                }
                if is_belt_funnel(p) && p.is("shape", "extended") {
                    return side == Dir::Up;
                }
                side == facing
            }
            // `BrassTunnelBlockEntity.isValidFaceForFilter`: the sides the tunnel has an opening on.
            Slot::BrassTunnelFilter => p.flag(&format!("fact:side_{}", side.name())),
            Slot::BrassTunnelMode => side == Dir::Up,
            Slot::Saw | Slot::SmartPipe | Slot::Roller { .. } | Slot::TrackObserver => true,
            Slot::Deployer => {
                let Some(facing) = p.dir("facing") else {
                    return false;
                };
                if side.axis() == facing.axis() {
                    return false;
                }
                let rotation = directional_rotation_axis(facing, p.flag("axis_along_first"));
                rotation != side.axis()
            }
            Slot::Motor => {
                let Some(facing) = p.dir("facing") else {
                    return false;
                };
                if facing.axis() != Axis::Y && side == Dir::Down {
                    return false;
                }
                side.axis() != facing.axis()
            }
            Slot::SpeedController => {
                if side.axis() == Axis::Y {
                    return false;
                }
                // `SpeedControllerBlock.HORIZONTAL_AXIS` is the block state's `axis`.
                p.axis("axis") != Some(side.axis())
            }
            Slot::Arm => is_horizontal(side),
            Slot::FilteredDetector { bottom } => {
                let Some(facing) = p.dir("facing") else {
                    return false;
                };
                let target = target_direction(p.text("target").as_deref().unwrap_or("wall"), facing);
                let mut side = side;
                if side == target {
                    return false;
                }
                if target.opposite() == side {
                    return true;
                }
                if target.axis() != Axis::Y {
                    return side == Dir::Up || (side == Dir::Down && bottom);
                }
                if target == Dir::Up {
                    side = side.opposite();
                }
                if !bottom {
                    return side == facing;
                }
                // `facing.getClockWise().getAxis()`: the axis across the facing, in the horizontal plane.
                side.axis() == facing.clockwise_about(Axis::Y).axis()
            }
            Slot::Centered(sides) => match sides {
                Sides::Any => true,
                Sides::Top => side == Dir::Up,
                Sides::Horizontal => is_horizontal(side),
            },
        }
    }

    /// `ValueBoxTransform.getLocalOffset`: the centre of the box, in the block (0 to 1), `None` for a block
    /// whose state has no box. `side` is the face looked at, for the slots that depend on it.
    pub fn offset(self, p: &Props, side: Dir) -> Option<Vec3> {
        match self {
            Slot::SmartChute => Some(rotate_centered(voxel_space(8.0, 11.0, 15.5), horizontal_angle(side), Axis::Y)),
            Slot::Funnel => funnel_offset(p, side),
            Slot::BrassTunnelFilter => Some(sided_offset(voxel_space(8.0, 13.0, 15.5), side)),
            Slot::BrassTunnelMode | Slot::Centered(_) => Some(sided_offset(voxel_space(8.0, 8.0, 15.5), side)),
            Slot::Saw => {
                if p.dir("facing") != Some(Dir::Up) {
                    return None;
                }
                let offset = if p.flag("flipped") { -3.0 } else { 3.0 };
                let x = voxel_space(8.0, 12.5, 8.0 + offset);
                let z = voxel_space(8.0 + offset, 12.5, 8.0);
                Some(if p.flag("axis_along_first") { z } else { x })
            }
            Slot::Deployer => {
                let facing = p.dir("facing")?;
                let mut at = voxel_space(8.0, 8.0, 15.5);
                at = rotate_centered(at, horizontal_angle(side), Axis::Y);
                at = rotate_centered(at, vertical_angle(side), Axis::X);
                let n = normal(facing);
                Some([at[0] - n[0] * 2.0 / 16.0, at[1] - n[1] * 2.0 / 16.0, at[2] - n[2] * 2.0 / 16.0])
            }
            Slot::Motor => {
                let facing = p.dir("facing")?;
                let at = sided_offset(voxel_space(8.0, 8.0, 12.5), side);
                let n = normal(facing);
                Some([at[0] - n[0] / 16.0, at[1] - n[1] / 16.0, at[2] - n[2] / 16.0])
            }
            Slot::SpeedController => Some(sided_offset(voxel_space(8.0, 11.0, 15.5), side)),
            Slot::Arm => {
                let y = if p.flag("ceiling") { 16.0 - 3.0 } else { 3.0 };
                Some(rotate_centered(voxel_space(8.0, y, 15.5), horizontal_angle(side), Axis::Y))
            }
            Slot::FilteredDetector { .. } => Some(sided_offset(voxel_space(8.0, 8.0, 15.5), side)),
            Slot::TrackObserver => Some(voxel_space(8.0, 15.5, 8.0)),
            Slot::Roller { h_offset } => {
                let angle = horizontal_angle(p.dir("facing")?) + 180.0;
                Some(rotate_centered(voxel_space(8.0 + f64::from(h_offset), 15.5, 11.0), angle, Axis::Y))
            }
            Slot::SmartPipe => {
                let face = p.text("face").unwrap_or_else(|| "floor".into());
                let (y, z) = match face.as_str() {
                    "ceiling" => (0.55, 4.6),
                    "wall" => (11.4, 0.55),
                    _ => (15.45, 4.625),
                };
                Some(rotate_centered(voxel_space(8.0, y, z), smart_pipe_angle_y(p), Axis::Y))
            }
        }
    }

    /// `ValueBoxTransform.rotate`: the turns, in the order the pose stack applies them, that face the box
    /// outwards.
    pub fn rotations(self, p: &Props, side: Dir) -> Vec<(Axis, f64)> {
        match self {
            Slot::Funnel => funnel_rotation(p, side),
            Slot::Saw => {
                let y = if p.flag("axis_along_first") { 90.0 } else { 0.0 } + if p.flag("flipped") { 0.0 } else { 180.0 };
                vec![(Axis::Y, y), (Axis::X, 90.0)]
            }
            Slot::Deployer => {
                let mut turns = Vec::new();
                if side.axis() == Axis::Y {
                    let facing = p.dir("facing").unwrap_or(Dir::North);
                    turns.push((Axis::Y, 180.0 + horizontal_angle(facing)));
                }
                let y = horizontal_angle(side) + 180.0;
                let x = match side {
                    Dir::Up => 90.0,
                    Dir::Down => 270.0,
                    _ => 0.0,
                };
                turns.push((Axis::Y, y));
                turns.push((Axis::X, x));
                turns
            }
            Slot::Motor | Slot::FilteredDetector { .. } => {
                // Both classes: `super.rotate`, then `rotateZDegrees(-horizontalAngle(facing) + 180)` for a
                // horizontal facing, on the top face only.
                let mut turns = sided_rotation(side);
                if let Some(facing) = p.dir("facing")
                    && facing.axis() != Axis::Y
                    && side == Dir::Up
                {
                    turns.push((Axis::Z, -horizontal_angle(facing) + 180.0));
                }
                turns
            }
            Slot::TrackObserver => vec![(Axis::X, 90.0)],
            Slot::Roller { .. } => {
                // `RollerValueBox.rotate`: `rotateYDegrees(horizontalAngle(facing) + 180).rotateXDegrees(90)`.
                let angle = horizontal_angle(p.dir("facing").unwrap_or(Dir::North)) + 180.0;
                vec![(Axis::Y, angle), (Axis::X, 90.0)]
            }
            Slot::SmartPipe => {
                let x = if p.is("face", "ceiling") { -45.0 } else { 45.0 };
                vec![(Axis::Y, smart_pipe_angle_y(p)), (Axis::X, x)]
            }
            _ => sided_rotation(side),
        }
    }

    /// The whole pose: where the box is, how it is turned and how big it is
    /// (`ValueBoxTransform.transform`).
    pub fn pose(self, p: &Props, side: Dir) -> Option<Pose> {
        Some(Pose { offset: self.offset(p, side)?, rotations: self.rotations(p, side), scale: self.scale() })
    }

    /// `ValueBoxTransform.testHit`: whether `local_hit` (where the player's eye ray met the block, from the
    /// block's lower corner) is on the box, with `side` the face it met. A `Sided` slot answers only when
    /// it is active on that face (`ValueSettingsInputHandler` checks `isSideActive` first).
    pub fn test_hit(self, p: &Props, side: Dir, local_hit: Vec3) -> bool {
        if self.sided() && !self.side_active(p, side) {
            return false;
        }
        let Some(offset) = self.offset(p, side) else {
            return false;
        };
        distance(local_hit, offset) < self.scale() / self.hit_divisor()
    }

    /// `ValueBoxTransform.shouldRender`: the box is there for this face.
    pub fn exists(self, p: &Props, side: Dir) -> bool {
        (!self.sided() || self.side_active(p, side)) && self.offset(p, side).is_some()
    }
}

fn distance(a: Vec3, b: Vec3) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn is_belt_funnel(p: &Props) -> bool {
    p.block.ends_with("belt_funnel")
}

/// `SmartPipeFilterSlot.angleY`.
fn smart_pipe_angle_y(p: &Props) -> f64 {
    let facing = p.dir("facing").unwrap_or(Dir::North);
    let mut angle = horizontal_angle(facing);
    if p.is("face", "wall") {
        angle += 180.0;
    }
    angle
}

/// `FunnelFilterSlotPositioning.getLocalOffset`.
fn funnel_offset(p: &Props, side: Dir) -> Option<Vec3> {
    let facing = p.dir("facing")?;
    let h_angle = horizontal_angle(side);
    let state_angle = horizontal_angle(facing);
    if is_belt_funnel(p) {
        let shape = p.text("shape").unwrap_or_else(|| "retracted".into());
        return Some(match shape.as_str() {
            "extended" => rotate_centered(voxel_space(8.0, 15.5, 13.0), state_angle, Axis::Y),
            "pulling" | "pushing" => rotate_centered(voxel_space(8.0, 12.0, 8.675), h_angle, Axis::Y),
            _ => rotate_centered(voxel_space(8.0, 13.0, 7.5), h_angle, Axis::Y),
        });
    }
    if !is_horizontal(facing) {
        let y = if facing == Dir::Down { 14.0 } else { 2.0 };
        return Some(rotate_centered(voxel_space(8.0, y, 15.5), h_angle, Axis::Y));
    }
    Some(rotate_centered(voxel_space(8.0, 12.2, 8.55), h_angle, Axis::Y))
}

/// `FunnelFilterSlotPositioning.rotate`.
fn funnel_rotation(p: &Props, side: Dir) -> Vec<(Axis, f64)> {
    let Some(facing) = p.dir("facing") else {
        return sided_rotation(side);
    };
    if !is_horizontal(facing) {
        return sided_rotation(side);
    }
    let belt = is_belt_funnel(p);
    let shape = p.text("shape").unwrap_or_else(|| "retracted".into());
    if belt && shape != "extended" {
        let mut turns = sided_rotation(side);
        if shape == "pulling" || shape == "pushing" {
            turns.push((Axis::X, -22.5));
        }
        return turns;
    }
    if !belt {
        let mut turns = sided_rotation(side);
        turns.push((Axis::X, -22.5));
        return turns;
    }
    // An extended belt funnel: facing is horizontal here, so only the facing's turn and a quarter about x.
    vec![(Axis::Y, horizontal_angle(facing)), (Axis::X, 90.0)]
}

// --- the pose ---------------------------------------------------------------------------------------

/// A box's place in the block: translate, the turns in order, then scale (`ValueBoxTransform.transform`).
#[derive(Clone, Debug, PartialEq)]
pub struct Pose {
    pub offset: Vec3,
    pub rotations: Vec<(Axis, f64)>,
    pub scale: f64,
}

impl Pose {
    /// A point of the box's own frame (the square is -0.5 to 0.5 across and 0 deep, its face towards -z)
    /// in the block's: scaled, then turned last turn first, then moved to `offset`.
    pub fn apply(&self, local: Vec3) -> Vec3 {
        let mut v = [local[0] * self.scale, local[1] * self.scale, local[2] * self.scale];
        for (axis, degrees) in self.rotations.iter().rev() {
            v = rotate(v, *degrees, *axis);
        }
        [v[0] + self.offset[0], v[1] + self.offset[1], v[2] + self.offset[2]]
    }

    /// The four corners of the box's outline, in order round it, nudged `lift` of the box's own units
    /// outwards.
    pub fn corners(&self, half: f64, lift: f64) -> [Vec3; 4] {
        let z = -lift;
        [
            self.apply([-half, -half, z]),
            self.apply([half, -half, z]),
            self.apply([half, half, z]),
            self.apply([-half, half, z]),
        ]
    }

    /// Where a point `lift` box units in front of the centre is: the place for text and an item.
    pub fn front(&self, lift: f64) -> Vec3 {
        self.apply([0.0, 0.0, -lift])
    }
}

// --- the ray ----------------------------------------------------------------------------------------

/// The first of `boxes` (block-local `[min x, y, z, max x, y, z]`) a ray from `origin` (block-local) along
/// `dir` enters, within `reach` blocks: the point it enters at and the face of the block's shape it met.
/// This is what the server needs of `BlockHitResult` (`getLocation`, `getDirection`), which
/// `RightClickBlockEvent` doesn't carry.
pub fn ray_boxes(boxes: &[[f64; 6]], origin: Vec3, dir: Vec3, reach: f64) -> Option<(Vec3, Dir)> {
    let mut best: Option<(f64, Dir)> = None;
    for b in boxes {
        let mut t_near = f64::NEG_INFINITY;
        let mut t_far = f64::INFINITY;
        let mut face = Dir::Up;
        let mut ok = true;
        for axis in 0..3 {
            let (lo, hi) = (b[axis], b[axis + 3]);
            if dir[axis].abs() < 1e-12 {
                if origin[axis] < lo || origin[axis] > hi {
                    ok = false;
                    break;
                }
                continue;
            }
            let (mut t0, mut t1) = ((lo - origin[axis]) / dir[axis], (hi - origin[axis]) / dir[axis]);
            // Moving the positive way enters through the low face, which looks the negative way.
            let mut entered = low_face(axis);
            if t0 > t1 {
                std::mem::swap(&mut t0, &mut t1);
                entered = entered.opposite();
            }
            if t0 > t_near {
                t_near = t0;
                face = entered;
            }
            t_far = t_far.min(t1);
            if t_near > t_far {
                ok = false;
                break;
            }
        }
        if !ok || t_far < 0.0 || t_near > reach {
            continue;
        }
        // An origin inside a box has no entry face; Create's clip would find none either.
        if t_near < 0.0 {
            continue;
        }
        if best.is_none_or(|(t, _)| t_near < t) {
            best = Some((t_near, face));
        }
    }
    best.map(|(t, face)| ([origin[0] + dir[0] * t, origin[1] + dir[1] * t, origin[2] + dir[2] * t], face))
}

fn low_face(axis: usize) -> Dir {
    [Dir::West, Dir::Down, Dir::North][axis]
}

/// The face of the block a point on its surface is nearest to: when the exact shape is unknown (a block
/// with no collision boxes), the face of the cube the point is on.
pub fn nearest_face(local: Vec3) -> Dir {
    let mut best = (f64::INFINITY, Dir::Up);
    for (axis, low, high) in [(0, Dir::West, Dir::East), (1, Dir::Down, Dir::Up), (2, Dir::North, Dir::South)] {
        for (face, plane) in [(low, 0.0), (high, 1.0)] {
            let d = (local[axis] - plane).abs();
            if d < best.0 {
                best = (d, face);
            }
        }
    }
    best.1
}

/// `Entity.calculateViewVector`: the direction of the eye for `yaw` and `pitch` in degrees.
pub fn view_vector(yaw: f64, pitch: f64) -> Vec3 {
    let (yaw, pitch) = (yaw.to_radians(), pitch.to_radians());
    [-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props<'a>(block: &'a str, get: &'a dyn Fn(&str) -> Option<String>) -> Props<'a> {
        Props::new(block, get)
    }

    fn near(a: Vec3, b: Vec3) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-9)
    }

    #[test]
    fn angles_are_the_angle_helpers() {
        assert_eq!(horizontal_angle(Dir::South), 0.0);
        assert_eq!(horizontal_angle(Dir::East), 90.0);
        assert_eq!(horizontal_angle(Dir::North), 180.0);
        assert_eq!(horizontal_angle(Dir::West), 270.0);
        assert_eq!(vertical_angle(Dir::Up), -90.0);
        assert_eq!(vertical_angle(Dir::Down), 90.0);
    }

    #[test]
    fn a_smart_chute_box_sits_on_each_horizontal_side() {
        let get = |_: &str| None;
        let p = props("create:smart_chute", &get);
        assert!(near(Slot::SmartChute.offset(&p, Dir::South).unwrap(), [0.5, 11.0 / 16.0, 15.5 / 16.0]));
        // `rotateCentered` east: the south face's box moves to the east face.
        assert!(near(Slot::SmartChute.offset(&p, Dir::East).unwrap(), [15.5 / 16.0, 11.0 / 16.0, 0.5]));
        assert!(near(Slot::SmartChute.offset(&p, Dir::North).unwrap(), [0.5, 11.0 / 16.0, 0.5 / 16.0]));
        assert!(Slot::SmartChute.side_active(&p, Dir::West));
        assert!(!Slot::SmartChute.side_active(&p, Dir::Up));
        assert!(!Slot::SmartChute.side_active(&p, Dir::Down));
    }

    #[test]
    fn a_hit_counts_within_a_quarter_block_of_the_centre() {
        let get = |_: &str| None;
        let p = props("create:smart_chute", &get);
        let centre = [0.5, 11.0 / 16.0, 15.5 / 16.0];
        assert!(Slot::SmartChute.test_hit(&p, Dir::South, centre));
        assert!(Slot::SmartChute.test_hit(&p, Dir::South, [centre[0] + 0.2, centre[1], centre[2]]));
        assert!(!Slot::SmartChute.test_hit(&p, Dir::South, [centre[0] + 0.26, centre[1], centre[2]]));
        // On the top there is no box, however near.
        assert!(!Slot::SmartChute.test_hit(&p, Dir::Up, [0.5, 1.0, 0.5]));
    }

    #[test]
    fn the_pose_faces_the_box_out_of_the_block() {
        let get = |_: &str| None;
        let p = props("create:smart_chute", &get);
        for (side, outward) in [
            (Dir::South, [0.0, 0.0, 1.0]),
            (Dir::North, [0.0, 0.0, -1.0]),
            (Dir::East, [1.0, 0.0, 0.0]),
            (Dir::West, [-1.0, 0.0, 0.0]),
        ] {
            let pose = Slot::SmartChute.pose(&p, side).unwrap();
            // The box's front is towards -z; lifting it moves the point outwards.
            let a = pose.front(0.0);
            let b = pose.front(1.0);
            let moved = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let length = pose.scale;
            assert!(near(moved, [outward[0] * length, outward[1] * length, outward[2] * length]), "{side:?}: {moved:?}");
        }
        // The corners are a square half a block across, centred on the offset.
        let pose = Slot::SmartChute.pose(&p, Dir::South).unwrap();
        let [a, b, c, d] = pose.corners(0.5, 0.0);
        assert!(((b[0] - a[0]).abs() - 0.5).abs() < 1e-9 || ((b[2] - a[2]).abs() - 0.5).abs() < 1e-9);
        let centre = [(a[0] + c[0]) / 2.0, (a[1] + c[1]) / 2.0, (a[2] + c[2]) / 2.0];
        assert!(near(centre, pose.offset));
        assert!(near([(b[0] + d[0]) / 2.0, (b[1] + d[1]) / 2.0, (b[2] + d[2]) / 2.0], pose.offset));
    }

    #[test]
    fn the_motor_box_is_off_its_shaft_and_hidden_underneath() {
        let get = |key: &str| (key == "facing").then(|| "north".to_owned());
        let p = props("create:creative_motor", &get);
        // Facing north (a horizontal shaft): no box under it, none on the shaft's own faces.
        assert!(!Slot::Motor.side_active(&p, Dir::Down));
        assert!(!Slot::Motor.side_active(&p, Dir::North));
        assert!(!Slot::Motor.side_active(&p, Dir::South));
        assert!(Slot::Motor.side_active(&p, Dir::Up));
        assert!(Slot::Motor.side_active(&p, Dir::East));
        // On the east face: (8, 8, 12.5) turned east, then 1/16 back from the facing (towards +z).
        let at = Slot::Motor.offset(&p, Dir::East).unwrap();
        assert!(near(at, [12.5 / 16.0, 0.5, 0.5 + 1.0 / 16.0]), "{at:?}");
        // On top of a horizontal motor the box is turned about z as well.
        let turns = Slot::Motor.rotations(&p, Dir::Up);
        assert_eq!(turns.last(), Some(&(Axis::Z, -180.0 + 180.0)));
        let up = |key: &str| (key == "facing").then(|| "up".to_owned());
        let p = props("create:creative_motor", &up);
        assert!(Slot::Motor.side_active(&p, Dir::Down) == false);
        assert!(Slot::Motor.side_active(&p, Dir::East));
    }

    #[test]
    fn the_speed_controller_box_is_on_the_sides_across_its_shaft() {
        let get = |key: &str| (key == "axis").then(|| "x".to_owned());
        let p = props("create:rotation_speed_controller", &get);
        assert!(Slot::SpeedController.side_active(&p, Dir::North));
        assert!(Slot::SpeedController.side_active(&p, Dir::South));
        assert!(!Slot::SpeedController.side_active(&p, Dir::East));
        assert!(!Slot::SpeedController.side_active(&p, Dir::Up));
        assert!(near(Slot::SpeedController.offset(&p, Dir::South).unwrap(), [0.5, 11.0 / 16.0, 15.5 / 16.0]));
    }

    #[test]
    fn the_roller_has_two_boxes_on_top_turned_with_its_facing() {
        let facing = |name: &'static str| move |key: &str| (key == "facing").then(|| name.to_owned());
        let filter = Slot::Roller { h_offset: 3 };
        let mode = Slot::Roller { h_offset: -3 };
        // North: `horizontalAngle + 180` is a full turn, so the boxes sit as written: x 11 and 5 px, z 11 px.
        let get = facing("north");
        let p = props("create:mechanical_roller", &get);
        assert!(near(filter.offset(&p, Dir::Up).unwrap(), [11.0 / 16.0, 15.5 / 16.0, 11.0 / 16.0]));
        assert!(near(mode.offset(&p, Dir::Up).unwrap(), [5.0 / 16.0, 15.5 / 16.0, 11.0 / 16.0]));
        assert_eq!(filter.rotations(&p, Dir::Up), vec![(Axis::Y, 360.0), (Axis::X, 90.0)]);
        // South: half a turn about the block's centre.
        let get = facing("south");
        let p = props("create:mechanical_roller", &get);
        assert!(near(filter.offset(&p, Dir::Up).unwrap(), [5.0 / 16.0, 15.5 / 16.0, 5.0 / 16.0]));
        // Not sided: it answers whatever face the ray met, and counts a hit within a sixth of a block
        // (`scale / 3`), not a quarter.
        assert!(!filter.sided());
        assert!(filter.exists(&p, Dir::North));
        assert!(filter.test_hit(&p, Dir::Up, [5.0 / 16.0 + 0.15, 15.5 / 16.0, 5.0 / 16.0]));
        assert!(!filter.test_hit(&p, Dir::Up, [5.0 / 16.0 + 0.2, 15.5 / 16.0, 5.0 / 16.0]));
        assert!(!mode.test_hit(&p, Dir::Up, [5.0 / 16.0, 15.5 / 16.0, 5.0 / 16.0]));
        assert!(mode.test_hit(&p, Dir::Up, [11.0 / 16.0, 15.5 / 16.0, 5.0 / 16.0]));
    }

    #[test]
    fn an_upright_saw_has_its_box_on_top_beside_the_blade() {
        let state = |facing: &'static str, flipped: bool, first: bool| {
            move |key: &str| match key {
                "facing" => Some(facing.to_owned()),
                "flipped" => Some(flipped.to_string()),
                "axis_along_first" => Some(first.to_string()),
                _ => None,
            }
        };
        let get = state("up", false, false);
        let p = props("create:mechanical_saw", &get);
        assert!(near(Slot::Saw.offset(&p, Dir::Up).unwrap(), [0.5, 12.5 / 16.0, 11.0 / 16.0]));
        let get = state("up", true, true);
        let p = props("create:mechanical_saw", &get);
        assert!(near(Slot::Saw.offset(&p, Dir::Up).unwrap(), [5.0 / 16.0, 12.5 / 16.0, 0.5]));
        let get = state("north", false, false);
        let p = props("create:mechanical_saw", &get);
        assert!(Slot::Saw.offset(&p, Dir::Up).is_none());
        assert!(!Slot::Saw.test_hit(&p, Dir::Up, [0.5, 0.78, 0.7]));
    }

    #[test]
    fn a_funnel_box_is_on_its_open_face_or_the_sides() {
        let state = |facing: &'static str, shape: &'static str| {
            move |key: &str| match key {
                "facing" => Some(facing.to_owned()),
                "shape" => Some(shape.to_owned()),
                _ => None,
            }
        };
        let get = state("north", "retracted");
        let p = props("create:brass_funnel", &get);
        assert!(Slot::Funnel.side_active(&p, Dir::North));
        assert!(!Slot::Funnel.side_active(&p, Dir::South));
        let get = state("down", "retracted");
        let p = props("create:brass_funnel", &get);
        assert!(Slot::Funnel.side_active(&p, Dir::East));
        assert!(!Slot::Funnel.side_active(&p, Dir::Up));
        assert!(near(Slot::Funnel.offset(&p, Dir::South).unwrap(), [0.5, 14.0 / 16.0, 15.5 / 16.0]));
        let get = state("north", "extended");
        let p = props("create:brass_belt_funnel", &get);
        assert!(Slot::Funnel.side_active(&p, Dir::Up));
        assert!(!Slot::Funnel.side_active(&p, Dir::North));
        // North turned about the centre by 180: (8, 15.5, 13) becomes (8, 15.5, 3).
        assert!(near(Slot::Funnel.offset(&p, Dir::Up).unwrap(), [0.5, 15.5 / 16.0, 3.0 / 16.0]));
        let get = state("north", "pushing");
        let p = props("create:brass_belt_funnel", &get);
        let turns = Slot::Funnel.rotations(&p, Dir::North);
        assert_eq!(turns.last(), Some(&(Axis::X, -22.5)));
    }

    #[test]
    fn the_deployer_box_slides_back_from_the_hand() {
        let get = |key: &str| match key {
            "facing" => Some("south".to_owned()),
            "axis_along_first" => Some("false".to_owned()),
            _ => None,
        };
        let p = props("create:deployer", &get);
        // Facing south with the shaft along y (not along first): its box is on the sides facing east, west and up/down? The
        // shaft axis (`directional_rotation_axis(Z, false)` = Y) rules out the up and down faces.
        assert_eq!(directional_rotation_axis(Dir::South, false), Axis::Y);
        assert!(Slot::Deployer.side_active(&p, Dir::East));
        assert!(!Slot::Deployer.side_active(&p, Dir::Up));
        assert!(!Slot::Deployer.side_active(&p, Dir::South));
        let at = Slot::Deployer.offset(&p, Dir::East).unwrap();
        // (8, 8, 15.5) turned east is (15.5, 8, 8); the facing's normal (0, 0, 1) scaled by 2/16 comes off z.
        assert!(near(at, [15.5 / 16.0, 0.5, 0.5 - 2.0 / 16.0]), "{at:?}");
    }

    #[test]
    fn the_arm_box_hangs_low_on_a_floor_arm_and_high_on_a_ceiling_one() {
        let floor = |_: &str| None;
        let p = props("create:mechanical_arm", &floor);
        assert!(near(Slot::Arm.offset(&p, Dir::South).unwrap(), [0.5, 3.0 / 16.0, 15.5 / 16.0]));
        let ceiling = |key: &str| (key == "ceiling").then(|| "true".to_owned());
        let p = props("create:mechanical_arm", &ceiling);
        assert!(near(Slot::Arm.offset(&p, Dir::South).unwrap(), [0.5, 13.0 / 16.0, 15.5 / 16.0]));
        assert!(!Slot::Arm.side_active(&p, Dir::Up));
    }

    #[test]
    fn the_detector_slot_follows_the_target() {
        let state = |facing: &'static str, target: &'static str| {
            move |key: &str| match key {
                "facing" => Some(facing.to_owned()),
                "target" => Some(target.to_owned()),
                _ => None,
            }
        };
        // A wall observer looking north: no box on its target face, one on the back, top (and bottom for the switch).
        let get = state("north", "wall");
        let p = props("create:content_observer", &get);
        let observer = Slot::FilteredDetector { bottom: false };
        let switch = Slot::FilteredDetector { bottom: true };
        assert!(!observer.side_active(&p, Dir::North));
        assert!(observer.side_active(&p, Dir::South));
        assert!(observer.side_active(&p, Dir::Up));
        assert!(!observer.side_active(&p, Dir::Down));
        assert!(switch.side_active(&p, Dir::Down));
        assert!(!observer.side_active(&p, Dir::East));
        // Looking down at a facing east: the box is on the side the observer faces, and the switch also across.
        let get = state("east", "floor");
        let p = props("create:content_observer", &get);
        assert!(!observer.side_active(&p, Dir::Down));
        assert!(observer.side_active(&p, Dir::Up));
        assert!(observer.side_active(&p, Dir::East));
        assert!(!observer.side_active(&p, Dir::West));
        assert!(switch.side_active(&p, Dir::North) && switch.side_active(&p, Dir::South));
        assert!(!switch.side_active(&p, Dir::East));
    }

    #[test]
    fn the_smart_pipe_box_follows_its_face() {
        let state = |face: &'static str| {
            move |key: &str| match key {
                "face" => Some(face.to_owned()),
                "facing" => Some("south".to_owned()),
                _ => None,
            }
        };
        let get = state("floor");
        let p = props("create:smart_fluid_pipe", &get);
        assert!(near(Slot::SmartPipe.offset(&p, Dir::Up).unwrap(), [0.5, 15.45 / 16.0, 4.625 / 16.0]));
        assert_eq!(Slot::SmartPipe.rotations(&p, Dir::Up), vec![(Axis::Y, 0.0), (Axis::X, 45.0)]);
        let get = state("wall");
        let p = props("create:smart_fluid_pipe", &get);
        assert_eq!(Slot::SmartPipe.rotations(&p, Dir::Up)[0], (Axis::Y, 180.0));
        let get = state("ceiling");
        let p = props("create:smart_fluid_pipe", &get);
        assert_eq!(Slot::SmartPipe.rotations(&p, Dir::Up)[1], (Axis::X, -45.0));
        assert!((Slot::SmartPipe.scale() - 0.51).abs() < 1e-12);
    }

    #[test]
    fn a_ray_finds_the_face_it_enters_by() {
        let full = [[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]];
        // From the south, looking north, at the middle of the south face.
        let (hit, face) = ray_boxes(&full, [0.5, 0.5, 3.0], [0.0, 0.0, -1.0], 6.0).unwrap();
        assert_eq!(face, Dir::South);
        assert!(near(hit, [0.5, 0.5, 1.0]));
        // From above and to the west, steeply down: the top.
        let (hit, face) = ray_boxes(&full, [0.4, 3.0, 0.4], [0.0, -1.0, 0.0], 6.0).unwrap();
        assert_eq!((face, hit), (Dir::Up, [0.4, 1.0, 0.4]));
        // From the west.
        let (_, face) = ray_boxes(&full, [-2.0, 0.5, 0.5], [1.0, 0.0, 0.0], 6.0).unwrap();
        assert_eq!(face, Dir::West);
        // A miss, and out of reach.
        assert!(ray_boxes(&full, [3.0, 0.5, 0.5], [0.0, 0.0, -1.0], 6.0).is_none());
        assert!(ray_boxes(&full, [0.5, 0.5, 9.0], [0.0, 0.0, -1.0], 6.0).is_none());
        // A thin plate in front: the nearer box wins.
        let two = [[0.0, 0.0, 0.0, 1.0, 1.0, 0.2], [0.0, 0.0, 0.8, 1.0, 1.0, 1.0]];
        let (hit, face) = ray_boxes(&two, [0.5, 0.5, 3.0], [0.0, 0.0, -1.0], 6.0).unwrap();
        assert_eq!(face, Dir::South);
        assert!(near(hit, [0.5, 0.5, 1.0]));
    }

    #[test]
    fn the_nearest_face_of_a_point_on_the_surface() {
        assert_eq!(nearest_face([0.5, 0.5, 1.0]), Dir::South);
        assert_eq!(nearest_face([0.5, 1.0, 0.5]), Dir::Up);
        assert_eq!(nearest_face([0.0, 0.3, 0.6]), Dir::West);
    }

    #[test]
    fn the_view_vector_is_the_games() {
        assert!(near(view_vector(0.0, 0.0), [0.0, 0.0, 1.0]));
        let east = view_vector(270.0, 0.0);
        assert!(near([east[0].round(), east[1], east[2].round()], [1.0, 0.0, 0.0]));
        let down = view_vector(0.0, 90.0);
        assert!(down[1] < -0.999);
    }
}
