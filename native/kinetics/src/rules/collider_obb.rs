//! The oriented-box maths behind Create's collision between entities and moving structures
//! (`foundation/collision`): `Matrix3d`, `OrientedBB`, `CollisionList` and
//! `ContinuousOBBCollider.collideMany` with its `ContinuousSeparationManifold`, plus the little
//! bit of Minecraft maths they use (`Vec3`, `AABB`, `Mth.sin`). Plain `f64` vectors and 3x3
//! matrices, no game types. The entity side of the collision (`ContraptionCollider`) is in
//! `collider.rs`.
//!
//! Java behaviours that matter and are kept: `Math.signum(0.0)` is 0 (Rust's is 1), `Vec3.equals`
//! compares with `Double.compare` (so `-0.0` is not the zero vector), and `Mth.sin` is the
//! quantised sine table.

use std::sync::OnceLock;

use super::actor::Vec3;

// --- Minecraft maths --------------------------------------------------------------------------

const SIN_SCALE: f64 = 10430.378350470453;

fn sine_table() -> &'static [f32] {
    static TABLE: OnceLock<Box<[f32]>> = OnceLock::new();
    TABLE.get_or_init(|| {
        // The same formula Horizon uses for vanilla's table (`horizon-core/src/geometry/trig.rs`).
        (0..65536).map(|i| (f64::from(i) / SIN_SCALE).sin() as f32).collect::<Vec<_>>().into_boxed_slice()
    })
}

/// `Mth.sin`: vanilla's quantised sine table.
pub fn mth_sin(radians: f32) -> f32 {
    sine_table()[((f64::from(radians) * SIN_SCALE) as i64 & 65535) as usize]
}

/// `Mth.cos`.
pub fn mth_cos(radians: f32) -> f32 {
    sine_table()[((f64::from(radians) * SIN_SCALE + 16384.0) as i64 & 65535) as usize]
}

/// `Math.signum(double)`: 0 for 0, -0 for -0, NaN for NaN.
pub fn jsignum(v: f64) -> f64 {
    if v > 0.0 {
        1.0
    } else if v < 0.0 {
        -1.0
    } else {
        v
    }
}

/// `Math.max(double, double)`: NaN if either is NaN.
fn jmax(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.max(b) }
}

/// `Math.min(double, double)`.
fn jmin(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.min(b) }
}

pub fn vadd(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn vsub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// `Vec3.scale`.
pub fn vscale(a: Vec3, s: f64) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

/// `Vec3.multiply(x, y, z)`.
pub fn vmul(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}

pub fn vdot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// `Vec3.cross`.
pub fn vcross(a: Vec3, b: Vec3) -> Vec3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

pub fn vlength(a: Vec3) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

pub fn vlength_sqr(a: Vec3) -> f64 {
    a[0] * a[0] + a[1] * a[1] + a[2] * a[2]
}

/// `Vec3.horizontalDistanceSqr`.
pub fn vhorizontal_sqr(a: Vec3) -> f64 {
    a[0] * a[0] + a[2] * a[2]
}

/// `Vec3.normalize`: the zero vector under 1e-5.
pub fn vnormalize(a: Vec3) -> Vec3 {
    let d = vlength(a);
    if d < 1.0E-5 { [0.0; 3] } else { [a[0] / d, a[1] / d, a[2] / d] }
}

/// `v.equals(Vec3.ZERO)`: `Vec3.equals` compares with `Double.compare`, so only +0.0 matches.
pub fn is_zero_vec(v: Vec3) -> bool {
    v.iter().all(|c| c.to_bits() == 0.0f64.to_bits())
}

/// `BlockPos.containing(vec)`: the cell a point is in.
pub fn block_containing(v: Vec3) -> [i32; 3] {
    [v[0].floor() as i32, v[1].floor() as i32, v[2].floor() as i32]
}

/// `Mth.lerp(double, double, double)`.
fn lerp(t: f64, a: f64, b: f64) -> f64 {
    a + t * (b - a)
}

/// Minecraft's `AABB`: a box from `min` to `max`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub const fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    /// `new AABB(x1, y1, z1, x2, y2, z2)`: the corners in any order.
    pub fn from_corners(a: Vec3, b: Vec3) -> Self {
        Self {
            min: [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])],
            max: [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])],
        }
    }

    /// `AABB.getCenter`.
    pub fn center(&self) -> Vec3 {
        [0, 1, 2].map(|i| lerp(0.5, self.min[i], self.max[i]))
    }

    pub fn x_size(&self) -> f64 {
        self.max[0] - self.min[0]
    }

    pub fn y_size(&self) -> f64 {
        self.max[1] - self.min[1]
    }

    pub fn z_size(&self) -> f64 {
        self.max[2] - self.min[2]
    }

    /// `AABB.move(Vec3)`.
    pub fn moved(&self, by: Vec3) -> Self {
        Self { min: vadd(self.min, by), max: vadd(self.max, by) }
    }

    /// `AABB.inflate(double)`.
    pub fn inflate(&self, by: f64) -> Self {
        self.inflate_xyz(by, by, by)
    }

    /// `AABB.inflate(x, y, z)`.
    pub fn inflate_xyz(&self, x: f64, y: f64, z: f64) -> Self {
        Self {
            min: [self.min[0] - x, self.min[1] - y, self.min[2] - z],
            max: [self.max[0] + x, self.max[1] + y, self.max[2] + z],
        }
    }

    /// `AABB.expandTowards(Vec3)`: grown on the side the vector points to.
    pub fn expand_towards(&self, v: Vec3) -> Self {
        let mut r = *self;
        for i in 0..3 {
            if v[i] < 0.0 {
                r.min[i] += v[i];
            } else if v[i] > 0.0 {
                r.max[i] += v[i];
            }
        }
        r
    }

    /// `AABB.contract(x, y, z)`: shrunk on the side the vector points to.
    pub fn contract(&self, x: f64, y: f64, z: f64) -> Self {
        let v = [x, y, z];
        let mut r = *self;
        for i in 0..3 {
            if v[i] < 0.0 {
                r.min[i] -= v[i];
            } else if v[i] > 0.0 {
                r.max[i] -= v[i];
            }
        }
        r
    }

    /// `AABB.intersects(AABB)`: overlapping, not just touching.
    pub fn intersects(&self, o: &Aabb) -> bool {
        self.min[0] < o.max[0]
            && self.max[0] > o.min[0]
            && self.min[1] < o.max[1]
            && self.max[1] > o.min[1]
            && self.min[2] < o.max[2]
            && self.max[2] > o.min[2]
    }
}

// --- Matrix3d -----------------------------------------------------------------------------------

/// `Matrix3d`: `m[row][column]` (`m01` is `m[0][1]`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix3d {
    pub m: [[f64; 3]; 3],
}

impl Matrix3d {
    /// `new Matrix3d().asIdentity()`.
    pub const fn identity() -> Self {
        Self { m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] }
    }

    /// `Matrix3d.asXRotation`.
    pub fn x_rotation(radians: f32) -> Self {
        let mut r = Self::identity();
        if radians == 0.0 {
            return r;
        }
        let s = f64::from(mth_sin(radians));
        let c = f64::from(mth_cos(radians));
        r.m[2][2] = c;
        r.m[1][1] = c;
        r.m[2][1] = s;
        r.m[1][2] = -s;
        r
    }

    /// `Matrix3d.asYRotation`.
    pub fn y_rotation(radians: f32) -> Self {
        let mut r = Self::identity();
        if radians == 0.0 {
            return r;
        }
        let s = f64::from(mth_sin(radians));
        let c = f64::from(mth_cos(radians));
        r.m[0][0] = c;
        r.m[2][2] = c;
        r.m[0][2] = s;
        r.m[2][0] = -s;
        r
    }

    /// `Matrix3d.asZRotation`.
    pub fn z_rotation(radians: f32) -> Self {
        let mut r = Self::identity();
        if radians == 0.0 {
            return r;
        }
        let s = f64::from(mth_sin(radians));
        let c = f64::from(mth_cos(radians));
        r.m[0][0] = c;
        r.m[1][1] = c;
        r.m[0][1] = -s;
        r.m[1][0] = s;
        r
    }

    /// `Matrix3d.scale`: the diagonal times `d`.
    pub fn scale(&mut self, d: f64) -> &mut Self {
        self.m[0][0] *= d;
        self.m[1][1] *= d;
        self.m[2][2] *= d;
        self
    }

    /// `Matrix3d.multiply`: `self = self * other`.
    pub fn multiply(&mut self, other: &Matrix3d) -> &mut Self {
        let a = self.m;
        let b = other.m;
        let mut n = [[0.0; 3]; 3];
        for r in 0..3 {
            for c in 0..3 {
                n[r][c] = a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c];
            }
        }
        self.m = n;
        self
    }

    /// `Matrix3d.transform`.
    pub fn transform(&self, v: Vec3) -> Vec3 {
        let m = &self.m;
        [
            v[0] * m[0][0] + v[1] * m[0][1] + v[2] * m[0][2],
            v[0] * m[1][0] + v[1] * m[1][1] + v[2] * m[1][2],
            v[0] * m[2][0] + v[1] * m[2][1] + v[2] * m[2][2],
        ]
    }

    /// `Matrix3d.transformTransposed`.
    pub fn transform_transposed(&self, v: Vec3) -> Vec3 {
        let m = &self.m;
        [
            v[0] * m[0][0] + v[1] * m[1][0] + v[2] * m[2][0],
            v[0] * m[0][1] + v[1] * m[1][1] + v[2] * m[2][1],
            v[0] * m[0][2] + v[1] * m[1][2] + v[2] * m[2][2],
        ]
    }
}

// --- OrientedBB and CollisionList -------------------------------------------------------------

/// `OrientedBB`: a box given by its centre, half extents and rotation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrientedBB {
    pub center: Vec3,
    pub extents: Vec3,
    pub rotation: Matrix3d,
}

impl OrientedBB {
    /// `new OrientedBB(AABB)`: centre and half sizes, no rotation.
    pub fn from_aabb(bb: &Aabb) -> Self {
        Self {
            center: bb.center(),
            extents: [bb.x_size() / 2.0, bb.y_size() / 2.0, bb.z_size() / 2.0],
            rotation: Matrix3d::identity(),
        }
    }

    /// `OrientedBB.move`.
    pub fn move_by(&mut self, offset: Vec3) {
        self.center = vadd(self.center, offset);
    }

    /// `OrientedBB.getAsAABB`.
    pub fn as_aabb(&self) -> Aabb {
        Aabb::new(self.center, self.center).inflate_xyz(self.extents[0], self.extents[1], self.extents[2])
    }
}

/// `CollisionList`: boxes as centre and half extents, in the structure's local space.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CollisionList {
    pub center: Vec<Vec3>,
    pub extents: Vec<Vec3>,
}

impl CollisionList {
    pub fn new() -> Self {
        Self::default()
    }

    /// `size`.
    pub fn len(&self) -> usize {
        self.center.len()
    }

    pub fn is_empty(&self) -> bool {
        self.center.is_empty()
    }

    /// `size = 0` (keeps the allocation, as Create's list does).
    pub fn clear(&mut self) {
        self.center.clear();
        self.extents.clear();
    }

    /// `Populate.append`.
    pub fn append(&mut self, center: Vec3, extents: Vec3) {
        self.center.push(center);
        self.extents.push(extents);
    }

    /// `Populate.consume` with its offset: one box of a shape (`VoxelShape.forAllBoxes`), `min` to
    /// `max`, in the block at `offset`.
    pub fn append_box(&mut self, offset: [i32; 3], min: Vec3, max: Vec3) {
        let o = offset.map(f64::from);
        self.append(
            [0, 1, 2].map(|i| o[i] + 0.5 * (max[i] + min[i])),
            [0, 1, 2].map(|i| 0.5 * (max[i] - min[i])),
        );
    }

    /// `Populate.appendFrom`.
    pub fn append_from(&mut self, other: &CollisionList, index: usize) {
        self.append(other.center[index], other.extents[index]);
    }

    /// `Contraption.invalidateColliders`: the boxes of every block that has a collision shape.
    /// `blocks` yields each block's position and the boxes of its collision shape (`min`, `max`,
    /// relative to the block). Cache the result while the structure's blocks don't change.
    pub fn from_blocks<'a>(blocks: impl IntoIterator<Item = ([i32; 3], &'a [Aabb])>) -> Self {
        let mut list = Self::new();
        for (pos, boxes) in blocks {
            for b in boxes {
                list.append_box(pos, b.min, b.max);
            }
        }
        list
    }
}

// --- ContinuousOBBCollider ----------------------------------------------------------------------

/// `ContinuousOBBCollider.CollisionResponse`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CollisionResponse {
    pub surface_collision: bool,
    pub collision_response: Vec3,
    pub normal: Vec3,
    pub location: Vec3,
    pub temporal_response: f64,
}

const U_A0: Vec3 = [1.0, 0.0, 0.0];
const U_A1: Vec3 = [0.0, 1.0, 0.0];
const U_A2: Vec3 = [0.0, 0.0, 1.0];

/// `ContinuousSeparationManifold`.
struct Manifold {
    latest_entry: f64,
    earliest_exit: f64,
    is_discrete: bool,
    collision: Vec3,
    step_axis: Vec3,
    step_separation: f64,
    normal_axis: Option<Vec3>,
    normal_separation: f64,
    axis: Option<Vec3>,
    separation: f64,
}

const UNDEFINED: f64 = -1.0;

fn with_signed_epsilon(sep: f64) -> f64 {
    sep + (jsignum(sep) * 1E-4)
}

impl Manifold {
    fn new(step_axis: Vec3) -> Self {
        Self {
            latest_entry: UNDEFINED,
            earliest_exit: f64::MAX,
            is_discrete: true,
            collision: [0.0; 3],
            step_axis,
            step_separation: 0.0,
            normal_axis: None,
            normal_separation: 0.0,
            axis: None,
            separation: 0.0,
        }
    }

    /// `ContinuousSeparationManifold.reset` (the collision point is not reset, as in Create).
    fn reset(&mut self) {
        self.axis = None;
        self.normal_axis = None;
        self.separation = f64::MAX;
        self.step_separation = f64::MAX;
        self.normal_separation = f64::MAX;
        self.latest_entry = -1.0;
        self.earliest_exit = f64::MAX;
        self.is_discrete = true;
    }

    /// `ContinuousSeparationManifold.separate`: true if not colliding.
    fn separate(&mut self, axis: Vec3, tl: f64, ra: f64, rb: f64, projected_motion: f64, axis_of_obj_a: bool) -> bool {
        let distance = tl.abs();
        let diff = distance - (ra + rb);

        let discrete_collision = diff <= 0.0;
        if !discrete_collision && jsignum(projected_motion) == jsignum(tl) {
            return true;
        }

        let s_tl = jsignum(tl);
        let separation = s_tl * diff.abs();

        if !discrete_collision {
            self.is_discrete = false;

            // Missed on this axis, means we missed entirely.
            if separation.abs() > projected_motion.abs() {
                return true;
            }

            let entry_time = separation.abs() / projected_motion.abs();
            let exit_time = (diff + ra.abs() + rb.abs()) / projected_motion.abs();
            self.latest_entry = jmax(entry_time, self.latest_entry);
            self.earliest_exit = jmin(exit_time, self.earliest_exit);
        }

        if axis_of_obj_a && distance != 0.0 && -diff <= self.normal_separation.abs() {
            self.normal_axis = Some(axis);
            self.normal_separation = separation;
        }

        let dot = vdot(self.step_axis, axis);
        if dot != 0.0 && discrete_collision {
            let cross = vcross(axis, self.step_axis);
            let dot_separation = jsignum(dot) * tl - (ra + rb);
            let mut step_separation = -dot_separation;

            if !is_zero_vec(cross) {
                let sep_vec = vscale(axis, dot_separation);
                let axis_plane = vcross(axis, cross);
                let step_plane = vcross(self.step_axis, cross);
                let step_separation_vec =
                    vsub(sep_vec, vscale(axis_plane, vdot(sep_vec, step_plane) / vdot(axis_plane, step_plane)));
                step_separation = vlength(step_separation_vec);
                if self.step_separation.abs() > step_separation.abs() && step_separation != 0.0 {
                    self.step_separation = step_separation;
                }
            } else if self.step_separation.abs() > step_separation {
                self.step_separation = step_separation;
            }
        }

        if distance != 0.0 && -diff <= self.separation.abs() {
            self.axis = Some(axis);
            self.separation = separation;
            let scale = jsignum(tl) * (if axis_of_obj_a { -ra } else { -rb }) - jsignum(separation) * 0.125;
            self.collision = [axis[0] * scale, axis[1] * scale, axis[2] * scale];
        }

        false
    }

    /// `ContinuousSeparationManifold.getTimeOfImpact`.
    fn time_of_impact(&self) -> f64 {
        if self.latest_entry == UNDEFINED {
            return UNDEFINED;
        }
        if self.latest_entry > self.earliest_exit {
            return UNDEFINED;
        }
        self.latest_entry
    }
}

/// `ContinuousOBBCollider.collideMany`: collides the oriented box `obb` (an entity, in the
/// structure's local space, rotated by `obb.rotation`) moving by `motion` against every box in
/// `collidable`. `dense` is scratch space for the boxes near enough to matter (reuse one list
/// across calls). `entity_max_step` is the entity's step height; `do_horizontal_pass` is
/// `!rotation.hasVerticalRotation()`.
///
/// Returns how far to push the entity out (`collision_response`, in the entity's frame, as Create
/// leaves it), the contact `normal` and `location`, the fraction of `motion` it can travel before
/// touching (`temporal_response`, 1 for all of it) and whether it rests on a surface.
pub fn collide_many(
    collidable: &CollisionList,
    dense: &mut CollisionList,
    obb: &OrientedBB,
    motion: Vec3,
    entity_max_step: f32,
    do_horizontal_pass: bool,
) -> CollisionResponse {
    let obb_center = obb.center;
    let obb_extents = obb.extents;
    let rotation = &obb.rotation;
    let r = &rotation.m;

    let a00 = r[0][0].abs();
    let a01 = r[0][1].abs();
    let a02 = r[0][2].abs();
    let a10 = r[1][0].abs();
    let a11 = r[1][1].abs();
    let a12 = r[1][2].abs();
    let a20 = r[2][0].abs();
    let a21 = r[2][1].abs();
    let a22 = r[2][2].abs();

    // The extents of the AABB encompassing the OBB, in local space.
    let aabb_in_local_x = (a00 + a01 + a02) * obb_extents[0] + 0.5;
    let aabb_in_local_y = (a10 + a11 + a12) * obb_extents[1] + 0.5;
    let aabb_in_local_z = (a20 + a21 + a22) * obb_extents[2] + 0.5;

    // Filter down the list of potential colliders, accounting for the entity's motion.
    dense.clear();
    for i in 0..collidable.len() {
        let c = collidable.center[i];
        let e = collidable.extents[i];
        if ((obb_center[0] + motion[0]) - c[0]).abs() > e[0] + aabb_in_local_x {
            continue;
        }
        if ((obb_center[1] + motion[1]) - c[1]).abs() > e[1] + aabb_in_local_y {
            continue;
        }
        if ((obb_center[2] + motion[2]) - c[2]).abs() > e[2] + aabb_in_local_z {
            continue;
        }
        dense.append_from(collidable, i);
    }

    // No collisions
    if dense.is_empty() {
        return CollisionResponse {
            surface_collision: false,
            collision_response: [0.0; 3],
            normal: [0.0; 3],
            location: [0.0; 3],
            temporal_response: 1.0,
        };
    }

    let mut response = [0.0f64; 3];
    let mut location = [0.0f64; 3];
    let mut normal = [0.0f64; 3];
    let mut surface_collision = false;
    let mut temporal_response = 1.0f64;

    let u_b0 = vnormalize([r[0][0], r[1][0], r[2][0]]);
    let u_b1 = vnormalize([r[0][1], r[1][1], r[2][1]]);
    let u_b2 = vnormalize([r[0][2], r[1][2], r[2][2]]);

    // Motion in the entity's frame.
    let motion2 = rotation.transform_transposed(motion);

    // Re-use the same manifold object per collider check.
    let mut mf = Manifold::new(u_b1);

    // Apply separation maths (`Iterate.trueAndFalse`)
    for horizontal_pass in [true, false] {
        let vertical_pass = !horizontal_pass || !do_horizontal_pass;

        if horizontal_pass && !do_horizontal_pass {
            continue;
        }

        for bb_idx in 0..dense.len() {
            let c = dense.center[bb_idx];
            let delta_x = obb_center[0] + response[0] - c[0];
            let delta_y = obb_center[1] + response[1] - c[1];
            let delta_z = obb_center[2] + response[2] - c[2];

            mf.reset();

            let [extents_x, extents_y, extents_z] = dense.extents[bb_idx];

            // Separate along A's local axes (global XYZ)
            if mf.separate(U_A0, delta_x, extents_x, a00 * obb_extents[0] + a01 * obb_extents[1] + a02 * obb_extents[2], motion[0], true)
                || mf.separate(U_A1, delta_y, extents_y, a10 * obb_extents[0] + a11 * obb_extents[1] + a12 * obb_extents[2], motion[1], true)
                || mf.separate(U_A2, delta_z, extents_z, a20 * obb_extents[0] + a21 * obb_extents[1] + a22 * obb_extents[2], motion[2], true)
            {
                continue;
            }

            let delta_entity_frame = rotation.transform_transposed([delta_x, delta_y, delta_z]);

            // Separate along B's local axes
            if mf.separate(u_b0, delta_entity_frame[0], extents_x * a00 + extents_y * a10 + extents_z * a20, obb_extents[0], motion2[0], false)
                || mf.separate(u_b1, delta_entity_frame[1], extents_x * a01 + extents_y * a11 + extents_z * a21, obb_extents[1], motion2[1], false)
                || mf.separate(u_b2, delta_entity_frame[2], extents_x * a02 + extents_y * a12 + extents_z * a22, obb_extents[2], motion2[2], false)
            {
                continue;
            }

            // If we reach here, the manifold has valid collision positions and normals.

            if vertical_pass && !surface_collision {
                surface_collision = true;
            }

            let mut time_of_impact = mf.time_of_impact();
            let is_temporal = time_of_impact > 0.0 && time_of_impact < 1.0;

            if !is_temporal && mf.is_discrete {
                // Create would fail on a missing axis; none is possible when the centres differ.
                if mf.step_separation <= f64::from(entity_max_step) {
                    let sep = with_signed_epsilon(mf.step_separation);
                    for i in 0..3 {
                        response[i] += mf.step_axis[i] * sep;
                    }
                } else {
                    let sep = with_signed_epsilon(mf.separation);
                    let axis = mf.axis.unwrap_or([0.0; 3]);
                    for i in 0..3 {
                        response[i] += axis[i] * sep;
                    }
                }
                time_of_impact = 0.0;
            }

            if time_of_impact >= 0.0 && temporal_response > time_of_impact {
                let scale = with_signed_epsilon(mf.normal_separation);
                let n = mf.normal_axis.unwrap_or([0.0; 3]);
                normal = [n[0] * scale, n[1] * scale, n[2] * scale];
                location = mf.collision;
            }

            if is_temporal && temporal_response > time_of_impact {
                temporal_response = time_of_impact;
            }
        }

        if vertical_pass {
            break;
        }

        let no_vertical_motion_response = temporal_response == 1.0;
        let no_vertical_collision = response[1] == 0.0;
        if no_vertical_collision && no_vertical_motion_response {
            break;
        }

        // Re-run collisions with horizontal offset
        response[0] *= 129.0 / 128.0;
        response[2] *= 129.0 / 128.0;
    }

    CollisionResponse {
        surface_collision,
        collision_response: response,
        normal,
        location,
        temporal_response,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f64, b: f64, eps: f64) -> bool {
        (a - b).abs() <= eps
    }

    #[test]
    fn signum_and_zero_follow_java() {
        assert_eq!(jsignum(0.0), 0.0);
        assert_eq!(jsignum(-3.0), -1.0);
        assert_eq!(jsignum(2.0), 1.0);
        assert!(is_zero_vec([0.0, 0.0, 0.0]));
        assert!(!is_zero_vec([-0.0, 0.0, 0.0]), "Double.compare tells -0.0 from 0.0");
    }

    #[test]
    fn sine_table_matches_cardinal_angles() {
        assert_eq!(mth_sin(0.0), 0.0);
        assert_eq!(mth_cos(0.0), 1.0);
        assert!(near(f64::from(mth_sin(std::f32::consts::FRAC_PI_2)), 1.0, 1e-6));
        assert!(near(f64::from(mth_cos(std::f32::consts::PI)), -1.0, 1e-6));
    }

    #[test]
    fn matrix_rotations_are_inverse_of_their_transpose() {
        let mut m = Matrix3d::identity();
        m.multiply(&Matrix3d::y_rotation(0.7));
        m.multiply(&Matrix3d::z_rotation(0.3));
        let v = [1.0, 2.0, 3.0];
        let back = m.transform_transposed(m.transform(v));
        for i in 0..3 {
            assert!(near(back[i], v[i], 2e-3), "{back:?}");
        }
    }

    #[test]
    fn y_rotation_matrix_layout() {
        // asYRotation: m00 = m22 = c, m02 = s, m20 = -s.
        let m = Matrix3d::y_rotation(std::f32::consts::FRAC_PI_2);
        assert!(near(m.m[0][2], 1.0, 1e-6));
        assert!(near(m.m[2][0], -1.0, 1e-6));
        assert!(near(m.m[0][0], 0.0, 1e-6));
        // (1,0,0) goes to (c, 0, -s) = (0, 0, -1).
        let v = m.transform([1.0, 0.0, 0.0]);
        assert!(near(v[2], -1.0, 1e-6));
    }

    #[test]
    fn aabb_helpers_follow_minecraft() {
        let a = Aabb::new([0.0, 0.0, 0.0], [1.0, 2.0, 3.0]);
        assert_eq!(a.center(), [0.5, 1.0, 1.5]);
        assert_eq!(a.expand_towards([-1.0, 0.0, 2.0]).min, [-1.0, 0.0, 0.0]);
        assert_eq!(a.expand_towards([-1.0, 0.0, 2.0]).max, [1.0, 2.0, 5.0]);
        assert_eq!(a.contract(0.0, 0.5, 0.0).max, [1.0, 1.5, 3.0]);
        assert_eq!(a.contract(0.0, -0.5, 0.0).min, [0.0, 0.5, 0.0]);
        assert!(a.intersects(&Aabb::new([0.9, 0.9, 0.9], [2.0, 2.0, 2.0])));
        assert!(!a.intersects(&Aabb::new([1.0, 0.0, 0.0], [2.0, 1.0, 1.0])), "touching is not intersecting");
    }

    /// A 1x1x1 block at the origin, the entity (a 0.6 x 1.8 x 0.6 box) falling onto it. Checked by
    /// hand from `separate`: centre 2.1 above the block's centre 0.5 gives TL = 1.6; rA = 0.5,
    /// rB = 0.9 + 1e-7; diff = 0.2 - 1e-7 > 0, entry = diff / 0.5.
    #[test]
    fn falling_box_meets_block_at_the_time_of_impact() {
        let mut collidable = CollisionList::new();
        collidable.append_box([0, 0, 0], [0.0; 3], [1.0; 3]);
        let bb = Aabb::new([0.2, 1.2, 0.2], [0.8, 3.0, 0.8]).inflate(1.0E-7);
        let obb = OrientedBB::from_aabb(&bb);
        let mut dense = CollisionList::new();
        let out = collide_many(&collidable, &mut dense, &obb, [0.0, -0.5, 0.0], 0.6, true);
        let sep = 1.6 - (0.5 + 0.9 + 1.0E-7);
        assert!(near(out.temporal_response, sep / 0.5, 1e-9), "{out:?}");
        assert!(out.surface_collision);
        assert_eq!(out.collision_response, [0.0; 3]);
    }

    #[test]
    fn far_box_is_not_collided() {
        let mut collidable = CollisionList::new();
        collidable.append_box([0, 0, 0], [0.0; 3], [1.0; 3]);
        let obb = OrientedBB::from_aabb(&Aabb::new([5.0, 5.0, 5.0], [5.6, 6.8, 5.6]));
        let mut dense = CollisionList::new();
        let out = collide_many(&collidable, &mut dense, &obb, [0.0, -0.1, 0.0], 0.6, true);
        assert_eq!(out.temporal_response, 1.0);
        assert!(!out.surface_collision);
        assert!(dense.is_empty());
    }
}
