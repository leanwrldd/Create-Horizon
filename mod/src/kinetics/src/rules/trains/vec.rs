//! The vector helpers the train code calls: `net.minecraft.world.phys.Vec3`, `Mth`, and Catnip's
//! `VecHelper` (`bezier`, `bezierDerivative`, `intersect`, `intersectRanged`, `slerp`, `lerp`,
//! `getCenterOf`) and `AngleHelper.deg`.
//!
//! Catnip is a separate library and is not in `create-java-src`, so `VecHelper` is written from the
//! calls Create makes (`BezierConnection`, `TrackPlacement`, `TrackEdge`) and cross-checked against
//! the existing `create-horizon-ola` port (`bezier.rs`); `intersect_ranged` in particular is
//! reconstructed from how `TrackEdge.getIntersection` uses it (see its doc) and is the one function
//! here that could not be checked against any source.

pub type Vec3 = [f64; 3];

pub const ZERO: Vec3 = [0.0, 0.0, 0.0];
pub const UP: Vec3 = [0.0, 1.0, 0.0];

pub fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn scale(a: Vec3, s: f64) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
pub fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
/// `Vec3.cross`.
pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub fn length(a: Vec3) -> f64 {
    dot(a, a).sqrt()
}
pub fn distance_to(a: Vec3, b: Vec3) -> f64 {
    length(sub(b, a))
}
/// `Vec3.normalize`: the zero vector for anything shorter than 1e-4.
pub fn normalize(a: Vec3) -> Vec3 {
    let d = length(a);
    if d < 1.0e-4 {
        ZERO
    } else {
        [a[0] / d, a[1] / d, a[2] / d]
    }
}
/// `Mth.equal(a, b)`: within 1e-5 (as a float).
pub fn equal(a: f64, b: f64) -> bool {
    (b - a).abs() < f64::from(1.0e-5_f32)
}
/// `Mth.floor(double)`.
pub fn floor(v: f64) -> i32 {
    let i = v as i32;
    if v < f64::from(i) { i - 1 } else { i }
}
/// `Math.round(double)` (half up).
pub fn round(v: f64) -> i64 {
    (v + 0.5).floor() as i64
}
/// `Mth.PI`, a float.
pub const MTH_PI: f64 = std::f32::consts::PI as f64;

/// `VecHelper.lerp(float p, from, to)`.
pub fn lerp(p: f32, from: Vec3, to: Vec3) -> Vec3 {
    add(from, scale(sub(to, from), f64::from(p)))
}

/// `VecHelper.getCenterOf(Vec3i)`.
pub fn center_of(pos: [i32; 3]) -> Vec3 {
    [f64::from(pos[0]) + 0.5, f64::from(pos[1]) + 0.5, f64::from(pos[2]) + 0.5]
}

/// `BlockPos.containing(Vec3)`: the block holding a point.
pub fn block_containing(v: Vec3) -> [i32; 3] {
    [floor(v[0]), floor(v[1]), floor(v[2])]
}

/// `AngleHelper.deg(double)`: radians to degrees, as a float.
pub fn deg(angle: f64) -> f32 {
    if angle == 0.0 {
        return 0.0;
    }
    (angle * 180.0 / std::f64::consts::PI) as f32
}

/// `VecHelper.bezier(p1, p2, q1, q2, t)`: ends `p1`, `p2`, handles `q1` (after `p1`) and `q2`
/// (before `p2`), by de Casteljau. `t` is a float in Java; the caller passes it already rounded.
pub fn bezier(p1: Vec3, p2: Vec3, q1: Vec3, q2: Vec3, t: f32) -> Vec3 {
    let v1 = lerp(t, p1, q1);
    let v2 = lerp(t, q1, q2);
    let v3 = lerp(t, q2, p2);
    lerp(t, lerp(t, v1, v2), lerp(t, v2, v3))
}

/// `VecHelper.bezierDerivative(p1, p2, q1, q2, t)`.
pub fn bezier_derivative(p1: Vec3, p2: Vec3, q1: Vec3, q2: Vec3, t: f32) -> Vec3 {
    let t = f64::from(t);
    let a = scale(p1, -3.0 * t * t + 6.0 * t - 3.0);
    let b = scale(q1, 9.0 * t * t - 12.0 * t + 3.0);
    let c = scale(q2, -9.0 * t * t + 6.0 * t);
    let d = scale(p2, 3.0 * t * t);
    add(add(a, b), add(c, d))
}

/// `VecHelper.slerp(t, v1, v2)`.
pub fn slerp(t: f32, v1: Vec3, v2: Vec3) -> Vec3 {
    let p = f64::from(t);
    let theta = dot(v1, v2).clamp(-1.0, 1.0).acos();
    let a = scale(v1, f64::from(((1.0 - p) as f32).sin()) * theta);
    let b = scale(v2, f64::from(((theta * p) as f32).sin()));
    scale(add(a, b), 1.0 / f64::from((theta as f32).sin()))
}

/// The plane `VecHelper.intersect` works in: the two axes left after projecting out this one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plane {
    /// Drop x: work in (y, z).
    X,
    /// Drop y: work in (x, z).
    Y,
    /// Drop z: work in (x, y).
    Z,
}

fn project(v: Vec3, plane: Plane) -> [f64; 2] {
    match plane {
        Plane::X => [v[1], v[2]],
        Plane::Y => [v[0], v[2]],
        Plane::Z => [v[0], v[1]],
    }
}

/// `VecHelper.intersect(p, q, r, s, plane)`: where `p + t·r` meets `q + u·s` in the plane, as
/// `[t, u]`; `None` when the lines are parallel.
pub fn intersect(p: Vec3, q: Vec3, r: Vec3, s: Vec3, plane: Plane) -> Option<[f64; 2]> {
    let (p, q, r, s) = (project(p, plane), project(q, plane), project(r, plane), project(s, plane));
    let qp = [q[0] - p[0], q[1] - p[1]];
    let rcs = r[0] * s[1] - r[1] * s[0];
    if equal(rcs, 0.0) {
        return None;
    }
    let rd = [r[0] / rcs, r[1] / rcs];
    let sd = [s[0] / rcs, s[1] / rcs];
    Some([qp[0] * sd[1] - qp[1] * sd[0], qp[0] * rd[1] - qp[1] * rd[0]])
}

/// `VecHelper.intersectRanged(p, q, r, s, plane)` as `TrackEdge.getIntersection` uses it: the
/// segment `p..r` against the segment `q..s`; `None` when they are parallel or miss, otherwise how
/// far along each one they cross, in blocks (the callers add these to a distance already walked).
/// Reconstructed, not copied: Catnip's source is not in the tree.
pub fn intersect_ranged(p: Vec3, q: Vec3, r: Vec3, s: Vec3, plane: Plane) -> Option<[f64; 2]> {
    let d1 = sub(r, p);
    let d2 = sub(s, q);
    let [t, u] = intersect(p, q, d1, d2, plane)?;
    if !(0.0..=1.0).contains(&t) || !(0.0..=1.0).contains(&u) {
        return None;
    }
    Some([t * length(d1), u * length(d2)])
}

/// `AABB.minmax`-style box: `[min, max]`.
pub type Aabb = [Vec3; 2];

/// `AABB.intersects(Vec3, Vec3)`: strict overlap with the box spanned by two points.
pub fn aabb_intersects_segment(bb: &Aabb, a: Vec3, b: Vec3) -> bool {
    let lo = [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])];
    let hi = [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])];
    bb[0][0] < hi[0] && bb[1][0] > lo[0] && bb[0][1] < hi[1] && bb[1][1] > lo[1] && bb[0][2] < hi[2] && bb[1][2] > lo[2]
}

/// `AABB.intersects(AABB)`.
pub fn aabb_intersects(a: &Aabb, b: &Aabb) -> bool {
    a[0][0] < b[1][0] && a[1][0] > b[0][0] && a[0][1] < b[1][1] && a[1][1] > b[0][1] && a[0][2] < b[1][2] && a[1][2] > b[0][2]
}

/// Catnip `VecHelper.intersectSphere(origin, lineDirection, sphereCenter, radius)`: where the line
/// leaves the sphere (the far intersection), or `None` when it misses or has no direction.
/// Reconstructed from memory of Catnip (not in `create-java-src`): check it when Catnip is at hand.
pub fn intersect_sphere(origin: Vec3, line_direction: Vec3, sphere_center: Vec3, radius: f64) -> Option<Vec3> {
    if length(line_direction) == 0.0 {
        return None;
    }
    let dir = if length(line_direction) != 1.0 { normalize(line_direction) } else { line_direction };
    let diff = sub(origin, sphere_center);
    let line_dot_diff = dot(dir, diff);
    let delta = line_dot_diff * line_dot_diff - (dot(diff, diff) - radius * radius);
    if delta < 0.0 {
        return None;
    }
    let t = -line_dot_diff + delta.sqrt();
    Some(add(origin, scale(dir, t)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bezier_hits_its_ends_and_middle() {
        let (p1, p2, q1, q2) = ([0.0, 0.0, 0.0], [3.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]);
        assert_eq!(bezier(p1, p2, q1, q2, 0.0), p1);
        assert_eq!(bezier(p1, p2, q1, q2, 1.0), p2);
        assert!((bezier(p1, p2, q1, q2, 0.5)[0] - 1.5).abs() < 1e-12);
        // Collinear handles: the derivative is constant, 3 per unit t.
        assert!((bezier_derivative(p1, p2, q1, q2, 0.3)[0] - 3.0).abs() < 1e-9);
    }

    #[test]
    fn intersect_finds_both_distances() {
        let hit = intersect([0.0; 3], [8.0, 0.0, 8.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0], Plane::Y).unwrap();
        assert!((hit[0] - 8.0).abs() < 1e-12 && (hit[1] - 8.0).abs() < 1e-12);
        assert!(intersect([0.0; 3], [0.0, 0.0, 5.0], [1.0, 0.0, 0.0], [1.0, 0.0, 0.0], Plane::Y).is_none());
        // Behind the start: negative t.
        let behind = intersect([0.0; 3], [-4.0, 0.0, 3.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0], Plane::Y).unwrap();
        assert!(behind[0] < 0.0);
        // The X plane works in (y, z).
        let up = intersect([0.0; 3], [5.0, 4.0, 4.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0], Plane::X).unwrap();
        assert!((up[0] - 4.0).abs() < 1e-12 && (up[1] - 4.0).abs() < 1e-12);
    }

    #[test]
    fn ranged_intersection_is_in_blocks() {
        // The segment (0,0)-(4,0) crossed by (2,-1)-(2,1) at 2 and 1.
        let hit = intersect_ranged([0.0; 3], [2.0, 0.0, -1.0], [4.0, 0.0, 0.0], [2.0, 0.0, 1.0], Plane::Y).unwrap();
        assert!((hit[0] - 2.0).abs() < 1e-12 && (hit[1] - 1.0).abs() < 1e-12);
        assert!(intersect_ranged([0.0; 3], [5.0, 0.0, -1.0], [4.0, 0.0, 0.0], [5.0, 0.0, 1.0], Plane::Y).is_none());
    }

    #[test]
    fn mth_rounding() {
        assert_eq!(floor(-0.5), -1);
        assert_eq!(floor(2.0), 2);
        assert_eq!(round(0.5), 1);
        assert_eq!(round(-0.5), 0);
        assert!(equal(1.0, 1.000001));
        assert!(!equal(1.0, 1.0001));
        assert_eq!(normalize([0.00001, 0.0, 0.0]), ZERO);
    }
}
