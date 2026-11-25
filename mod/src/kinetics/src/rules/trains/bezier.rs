//! `BezierConnection` (content/trains/track/BezierConnection.java): a curved piece of track between
//! two track ends, as a cubic Bezier whose handles are sized so a turn follows a circle.
//!
//! Ported: the control points (`Runtime.determineHandles`), the length (`computeLength`, 16 straight
//! pieces), the sleeper spacing table (`stepLUT`, `getSegmentT`), `getPosition`, `incrementT`,
//! `getNormal`, the bounds, the segment iterator (`Bezierator`, as [`BezierConnection::segments`]),
//! `rasterise` (the columns the curve crosses), `secondary`, `equalsSansMaterial`, the item costs.
//!
//! `yOffsetAt` is [`BezierConnection::y_offset_at`] (with `track_tilt.rs`'s `compareHandles`).
//!
//! Left out (render and world side, for the glue): spawning items and particles, `addItemsToPlayer`, the NBT and packet
//! formats. The baked render transforms (`SegmentAngles`, `GirderAngles`) are in `track_render.rs`.
//!
//! Known differences: `Mth.atan2` is a lookup-table approximation in Minecraft; `f64::atan2` is used.

use super::vec::{
    Aabb, MTH_PI, Plane, UP, Vec3, add, bezier, bezier_derivative, block_containing, cross, distance_to, equal,
    intersect, length, normalize, scale, slerp, sub,
};
use crate::Pos;
use std::collections::BTreeMap;

/// One sample along the curve (`BezierConnection.Segment`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub index: usize,
    pub position: Vec3,
    pub derivative: Vec3,
    pub face_normal: Vec3,
    pub normal: Vec3,
}

/// What `new Runtime(starts, axes)` works out once.
#[derive(Clone, Debug, PartialEq)]
struct Runtime {
    finish1: Vec3,
    finish2: Vec3,
    length: f64,
    step_lut: Vec<f32>,
    segments: usize,
    radius: f64,
    handle_length: f64,
    bounds: Aabb,
}

/// A curve between two track ends. `starts` are where the track leaves each end, `axes` the
/// direction it leaves in (toward the curve), `normals` the track's up at each end.
#[derive(Clone, Debug, PartialEq)]
pub struct BezierConnection {
    pub be_positions: [Pos; 2],
    pub starts: [Vec3; 2],
    pub axes: [Vec3; 2],
    pub normals: [Vec3; 2],
    pub smoothing: Option<[i32; 2]>,
    pub primary: bool,
    pub has_girder: bool,
    /// The track material's id (`TrackMaterial.id`), e.g. `create:andesite`.
    pub material: String,
    runtime: Runtime,
}

impl BezierConnection {
    /// The constructor: also resolves the runtime.
    pub fn new(
        be_positions: [Pos; 2],
        starts: [Vec3; 2],
        axes: [Vec3; 2],
        normals: [Vec3; 2],
        primary: bool,
        has_girder: bool,
        material: impl Into<String>,
    ) -> BezierConnection {
        let runtime = Runtime::new(starts, axes);
        BezierConnection {
            be_positions,
            starts,
            axes,
            normals,
            smoothing: None,
            primary,
            has_girder,
            material: material.into(),
            runtime,
        }
    }

    /// `secondary()`: the same curve seen from the other end.
    pub fn secondary(&self) -> BezierConnection {
        let mut c = BezierConnection::new(
            [self.be_positions[1], self.be_positions[0]],
            [self.starts[1], self.starts[0]],
            [self.axes[1], self.axes[0]],
            [self.normals[1], self.normals[0]],
            !self.primary,
            self.has_girder,
            self.material.clone(),
        );
        c.smoothing = self.smoothing.map(|s| [s[1], s[0]]);
        c
    }

    /// `getKey()`: the block entity position that keys the connection.
    pub fn key(&self) -> Pos {
        self.be_positions[1]
    }

    pub fn is_primary(&self) -> bool {
        self.primary
    }

    /// `yOffsetAt(end)`: the smoothing of the end that is at `end` (`TrackBlockEntityTilt.compareHandles`), 0 without any.
    pub fn y_offset_at(&self, end: Vec3) -> i32 {
        let Some(smoothing) = self.smoothing else { return 0 };
        if super::track_tilt::compare_handles(self.starts[0], end) {
            smoothing[0]
        } else if super::track_tilt::compare_handles(self.starts[1], end) {
            smoothing[1]
        } else {
            0
        }
    }

    /// `coupleEquals`: equal, or each vector within 1e-6 (`closerThan`).
    fn couple_eq_vec(a: &[Vec3; 2], b: &[Vec3; 2]) -> bool {
        (a[0] == b[0] && a[1] == b[1]) || (distance_to(a[0], b[0]) < 1e-6 && distance_to(a[1], b[1]) < 1e-6)
    }

    fn equals_sans_material_inner(&self, other: &BezierConnection) -> bool {
        self.be_positions == other.be_positions
            && Self::couple_eq_vec(&self.starts, &other.starts)
            && Self::couple_eq_vec(&self.axes, &other.axes)
            && Self::couple_eq_vec(&self.normals, &other.normals)
            && self.has_girder == other.has_girder
    }

    /// `equalsSansMaterial`: the same curve from either end.
    pub fn equals_sans_material(&self, other: &BezierConnection) -> bool {
        self.equals_sans_material_inner(other) || self.equals_sans_material_inner(&other.secondary())
    }

    // --- runtime information ---------------------------------------------------------------

    pub fn length(&self) -> f64 {
        self.runtime.length
    }
    pub fn step_lut(&self) -> &[f32] {
        &self.runtime.step_lut
    }
    pub fn segment_count(&self) -> usize {
        self.runtime.segments
    }
    /// `getRadius()`: the circle's radius; 0 for an S-bend or a straight run.
    pub fn radius(&self) -> f64 {
        self.runtime.radius
    }
    pub fn handle_length(&self) -> f64 {
        self.runtime.handle_length
    }
    /// The far points of the two handles (`finish1`, `finish2`).
    pub fn finishes(&self) -> [Vec3; 2] {
        [self.runtime.finish1, self.runtime.finish2]
    }
    /// `getBounds()`: the curve's box, inflated by 1.375.
    pub fn bounds(&self) -> Aabb {
        self.runtime.bounds
    }

    /// `getPosition(t)`.
    pub fn position(&self, t: f64) -> Vec3 {
        bezier(self.starts[0], self.starts[1], self.runtime.finish1, self.runtime.finish2, t as f32)
    }

    /// `getSegmentT(index)`.
    pub fn segment_t(&self, index: usize) -> f32 {
        self.runtime.segment_t(index)
    }

    /// `incrementT(currentT, distance)`: the `t` further along by `distance` (a share of the length).
    pub fn increment_t(&self, current_t: f64, distance: f64) -> f64 {
        let dx = length(bezier_derivative(
            self.starts[0],
            self.starts[1],
            self.runtime.finish1,
            self.runtime.finish2,
            current_t as f32,
        )) / self.length();
        current_t + distance / dx
    }

    /// `getNormal(t)`: the track's up along the curve, between the end normals.
    pub fn normal(&self, t: f64) -> Vec3 {
        let [fn1, fn2] = self.normals;
        let derivative = normalize(bezier_derivative(
            self.starts[0],
            self.starts[1],
            self.runtime.finish1,
            self.runtime.finish2,
            t as f32,
        ));
        let face_normal = if fn1 == fn2 { fn1 } else { slerp(t as f32, fn1, fn2) };
        let normal = normalize(cross(face_normal, derivative));
        cross(derivative, normal)
    }

    /// `getTrackItemCost()`.
    pub fn track_item_cost(&self) -> usize {
        (self.segment_count() + 1) / 2
    }

    /// `getGirderItemCost()`.
    pub fn girder_item_cost(&self) -> usize {
        if self.has_girder { self.track_item_cost() * 2 } else { 0 }
    }

    /// The `Bezierator` iterator, collected: one sample per sleeper, index 0 through the segment
    /// count. `iterator()` shifts the curve by `-bePositions.first + (0, 3/16, 0)`.
    pub fn segments(&self) -> Vec<Segment> {
        let first = self.be_positions[0];
        let offset = [-f64::from(first[0]), -f64::from(first[1]) + f64::from(3.0_f32 / 16.0), -f64::from(first[2])];
        self.segments_with_offset(offset)
    }

    /// The iterator with an offset of its own.
    pub fn segments_with_offset(&self, offset: Vec3) -> Vec<Segment> {
        let end1 = add(self.starts[0], offset);
        let end2 = add(self.starts[1], offset);
        let finish1 = add(scale(self.axes[0], self.runtime.handle_length), end1);
        let finish2 = add(scale(self.axes[1], self.runtime.handle_length), end2);
        let [fn1, fn2] = self.normals;
        (0..=self.runtime.segments)
            .map(|index| {
                let t = self.runtime.segment_t(index);
                let position = bezier(end1, end2, finish1, finish2, t);
                let derivative = normalize(bezier_derivative(end1, end2, finish1, finish2, t));
                let face_normal = if fn1 == fn2 { fn1 } else { slerp(t, fn1, fn2) };
                let normal = normalize(cross(face_normal, derivative));
                Segment { index, position, derivative, face_normal, normal }
            })
            .collect()
    }

    /// `rasterise()`: for each (x, z) column (relative to the first end's block) the curve's rail
    /// crosses, the height of the rail middle; the column that only doubles a diagonal step is dropped.
    pub fn rasterise(&self) -> BTreeMap<(i32, i32), f64> {
        let mut y_levels: BTreeMap<(i32, i32), f64> = BTreeMap::new();
        let te = self.be_positions[0];
        let te_corner = [f64::from(te[0]), f64::from(te[1]), f64::from(te[2])];
        let lift = [0.0, f64::from(3.0_f32 / 16.0), 0.0];
        let end1 = add(sub(self.starts[0], te_corner), lift);
        let end2 = add(sub(self.starts[1], te_corner), lift);
        let handle = self.handle_length();
        let finish1 = add(scale(self.axes[0], handle), end1);
        let finish2 = add(scale(self.axes[1], handle), end2);
        let [fn1, fn2] = self.normals;
        let seg_count = self.segment_count();
        let lut = self.step_lut();
        let mut samples: Vec<Vec3> = Vec::with_capacity(seg_count);
        for i in 0..seg_count {
            let t = ((i as f32 + 0.5) * lut[i] / seg_count as f32).clamp(0.0, 1.0);
            let result = bezier(end1, end2, finish1, finish2, t);
            let derivative = normalize(bezier_derivative(end1, end2, finish1, finish2, t));
            let face_normal = if fn1 == fn2 { fn1 } else { slerp(t, fn1, fn2) };
            let normal = normalize(cross(face_normal, derivative));
            let below = add(result, scale(face_normal, f64::from(-0.25_f32)));
            let rail1 = add(below, scale(normal, f64::from(0.05_f32)));
            let rail2 = sub(below, scale(normal, f64::from(0.05_f32)));
            samples.push(scale(add(rail1, rail2), 0.5));
        }
        let center = scale(add(end1, end2), 0.5);
        let diff = |k: (i32, i32)| {
            let (dx, dz) = (f64::from(k.0) + 0.5 - center[0], f64::from(k.1) + 0.5 - center[2]);
            dx * dx + dz * dz
        };
        let doubled = |from: (i32, i32), via: (i32, i32), to: (i32, i32)| {
            let (d1x, d1z) = (via.0 - from.0, via.1 - from.1);
            let (d2x, d2z) = (to.0 - via.0, to.1 - via.1);
            d1x.abs() + d1z.abs() == 1 && d2x.abs() + d2z.abs() == 1 && d1x != d2x && d1z != d2z
        };
        let mut prev: Option<(i32, i32)> = None;
        let mut prev2: Option<(i32, i32)> = None;
        let mut prev3: Option<(i32, i32)> = None;
        for rail_middle in samples {
            let pos = block_containing(rail_middle);
            let key = (pos[0], pos[2]);
            let already = y_levels.get(&key).copied();
            if let Some(y) = already {
                if y <= rail_middle[1] {
                    continue;
                }
            }
            y_levels.insert(key, rail_middle[1]);
            if already.is_some() {
                continue;
            }
            if let (Some(p1), Some(p2), Some(p3)) = (prev, prev2, prev3) {
                let doubled_via_prev = doubled(p2, p1, key);
                let doubled_via_prev2 = doubled(p3, p2, p1);
                let prev_closer = diff(p1) > diff(p2);
                if doubled_via_prev2 && (!doubled_via_prev || !prev_closer) {
                    y_levels.remove(&p2);
                    prev2 = prev;
                    prev = Some(key);
                    continue;
                } else if doubled_via_prev && doubled_via_prev2 && prev_closer {
                    y_levels.remove(&p1);
                    prev = Some(key);
                    continue;
                }
            }
            prev3 = prev2;
            prev2 = prev;
            prev = Some(key);
        }
        y_levels
    }
}

impl Runtime {
    fn new(starts: [Vec3; 2], axes: [Vec3; 2]) -> Runtime {
        let (end1, end2) = (starts[0], starts[1]);
        let axis1 = normalize(axes[0]);
        let axis2 = normalize(axes[1]);
        let (handle_length, radius) = determine_handles(end1, end2, axis1, axis2);
        let finish1 = add(scale(axis1, handle_length), end1);
        let finish2 = add(scale(axis2, handle_length), end2);

        let scan_count = 16;
        let length = compute_length(finish1, finish2, end1, end2, scan_count);

        let segments = (length * 2.0) as usize;
        let mut step_lut = vec![0.0_f32; segments + 1];
        step_lut[0] = 1.0;
        let mut combined_distance = 0.0_f32;
        let mut lo = [end1[0].min(end2[0]), end1[1].min(end2[1]), end1[2].min(end2[2])];
        let mut hi = [end1[0].max(end2[0]), end1[1].max(end2[1]), end1[2].max(end2[2])];
        let mut previous = end1;
        for i in 0..=segments {
            let t = i as f32 / segments as f32;
            let result = bezier(end1, end2, finish1, finish2, t);
            for k in 0..3 {
                lo[k] = lo[k].min(result[k]);
                hi[k] = hi[k].max(result[k]);
            }
            if i > 0 {
                // `float += double`: added in double, narrowed back.
                combined_distance = (f64::from(combined_distance) + distance_to(previous, result) / length) as f32;
                step_lut[i] = t / combined_distance;
            }
            previous = result;
        }
        let grow = f64::from(1.375_f32);
        let bounds = [
            [lo[0] - grow, lo[1] - grow, lo[2] - grow],
            [hi[0] + grow, hi[1] + grow, hi[2] + grow],
        ];
        Runtime { finish1, finish2, length, step_lut, segments, radius, handle_length, bounds }
    }

    fn segment_t(&self, index: usize) -> f32 {
        if index == self.segments {
            1.0
        } else {
            index as f32 * self.step_lut[index] / self.segments as f32
        }
    }
}

fn compute_length(finish1: Vec3, finish2: Vec3, end1: Vec3, end2: Vec3, scan_count: i32) -> f64 {
    let mut length = 0.0;
    let mut previous = end1;
    for i in 0..=scan_count {
        let t = i as f32 / scan_count as f32;
        let result = bezier(end1, end2, finish1, finish2, t);
        length += distance_to(previous, result);
        previous = result;
    }
    length
}

/// `Runtime.determineHandles`, as `(handle_length, radius)`.
fn determine_handles(end1: Vec3, end2: Vec3, axis1: Vec3, axis2: Vec3) -> (f64, f64) {
    let cross1 = cross(axis1, UP);
    let cross2 = cross(axis2, UP);

    let a1 = (-axis2[2]).atan2(-axis2[0]);
    let a2 = axis1[2].atan2(axis1[0]);
    let mut angle = a1 - a2;

    let circle = 2.0 * MTH_PI;
    angle = (angle + circle) % circle;
    if (circle - angle).abs() < angle.abs() {
        angle = circle - angle;
    }

    if equal(angle, 0.0) {
        if let Some(hit) = intersect(end1, end2, axis1, cross2, Plane::Y) {
            let t = hit[0].abs();
            let u = hit[1].abs();
            let min = t.min(u);
            let max = t.max(u);
            if min > 1.2 && max / min > 1.0 && max / min < 3.0 {
                return (max - min, 0.0);
            }
        }
        return (distance_to(end1, end2) / 3.0, 0.0);
    }

    let n = circle / angle;
    let factor = 4.0 / 3.0 * (std::f64::consts::PI / (2.0 * n)).tan();
    let Some(hit) = intersect(end1, end2, cross1, cross2, Plane::Y) else {
        return (distance_to(end1, end2) / 3.0, 0.0);
    };
    let radius = hit[1].abs();
    let mut handle_length = radius * factor;
    if equal(handle_length, 0.0) {
        handle_length = 1.0;
    }
    (handle_length, radius)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quarter_turn() -> BezierConnection {
        // From (0, 0, 0) heading +x, to (8, 0, 8) where the track leaves back toward -z.
        BezierConnection::new(
            [[0, 0, 0], [8, 0, 8]],
            [[0.0; 3], [8.0, 0.0, 8.0]],
            [[1.0, 0.0, 0.0], [0.0, 0.0, -1.0]],
            [UP, UP],
            true,
            false,
            "create:andesite",
        )
    }

    #[test]
    fn a_quarter_turn_follows_its_circle() {
        let c = quarter_turn();
        assert!((c.radius() - 8.0).abs() < 1e-9, "{}", c.radius());
        // 4/3 tan(pi/8) x 8, the quarter-circle handle.
        assert!((c.handle_length() - 8.0 * 0.552_284_75).abs() < 1e-4, "{}", c.handle_length());
        // A quarter of a circle of 8 is 12.566; 16 straight pieces fall just short.
        assert!((c.length() - 12.566).abs() < 0.02, "{}", c.length());
        assert_eq!(c.segment_count(), 25);
        assert_eq!(c.step_lut().len(), 26);
        let mid = c.position(0.5);
        let r = (mid[0].powi(2) + (mid[2] - 8.0).powi(2)).sqrt();
        assert!((r - 8.0).abs() < 0.01, "{r}");
        assert_eq!(c.position(0.0), [0.0; 3]);
        // The ends: t = 0 and 1 hit the track ends exactly (to float rounding).
        let end = c.position(1.0);
        assert!((end[0] - 8.0).abs() < 1e-9 && (end[2] - 8.0).abs() < 1e-9);
        // Flat track stays upright.
        let n = c.normal(0.3);
        assert!((n[1] - 1.0).abs() < 1e-6, "{n:?}");
        // Bounds: both ends and the curve, grown by 1.375.
        let b = c.bounds();
        assert!(b[0][0] <= -1.375 + 1e-9 && b[1][2] >= 8.0 + 1.375 - 1e-9);
    }

    #[test]
    fn sleepers_are_evenly_spaced() {
        let c = quarter_turn();
        for i in 0..c.segment_count() {
            let a = c.position(f64::from(c.segment_t(i)));
            let b = c.position(f64::from(c.segment_t(i + 1)));
            let gap = distance_to(a, b);
            assert!((gap - 0.5).abs() < 0.03, "{i}: {gap}");
        }
        assert_eq!(c.segment_t(c.segment_count()), 1.0);
        assert_eq!(c.segment_t(0), 0.0);
    }

    #[test]
    fn increment_t_takes_distance_as_a_share_of_length() {
        let c = quarter_turn();
        let t = c.increment_t(0.0, 1.0 / c.length());
        assert!((length(c.position(t)) - 1.0).abs() < 0.05, "{t}");
    }

    #[test]
    fn an_s_bend_and_a_straight_run() {
        let s = BezierConnection::new(
            [[0, 0, 0], [12, 0, 3]],
            [[0.0; 3], [12.0, 0.0, 3.0]],
            [[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0]],
            [UP, UP],
            true,
            false,
            "create:andesite",
        );
        assert_eq!(s.radius(), 0.0);
        assert!(s.handle_length() > 0.0);
        assert!(s.length() > 12.0 && s.length() < 13.5, "{}", s.length());
        let mid = s.position(0.5);
        assert!((mid[0] - 6.0).abs() < 1e-9 && (mid[2] - 1.5).abs() < 1e-9, "{mid:?}");

        let line = BezierConnection::new(
            [[0, 0, 0], [9, 0, 0]],
            [[0.0; 3], [9.0, 0.0, 0.0]],
            [[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0]],
            [UP, UP],
            true,
            false,
            "create:andesite",
        );
        assert!((line.handle_length() - 3.0).abs() < 1e-9);
        assert!((line.length() - 9.0).abs() < 1e-9);
    }

    #[test]
    fn secondary_swaps_the_ends_and_equals_itself() {
        let c = quarter_turn();
        let s = c.secondary();
        assert_eq!(s.starts[0], c.starts[1]);
        assert_eq!(s.be_positions, [[8, 0, 8], [0, 0, 0]]);
        assert!(!s.primary);
        assert!((s.length() - c.length()).abs() < 1e-9);
        assert!(c.equals_sans_material(&s));
        assert_eq!(c.track_item_cost(), 13);
        assert_eq!(c.girder_item_cost(), 0);
        assert_eq!(s.key(), [0, 0, 0]);
    }

    #[test]
    fn segments_cover_the_curve() {
        let c = quarter_turn();
        let segs = c.segments();
        assert_eq!(segs.len(), c.segment_count() + 1);
        assert_eq!(segs[0].index, 0);
        // The iterator lifts the curve 3/16 above the track end.
        assert!((segs[0].position[1] - 0.1875).abs() < 1e-9);
        assert!((length(segs[3].derivative) - 1.0).abs() < 1e-9);
        // Flat: face normal up, normal across the track.
        assert_eq!(segs[3].face_normal, UP);
        assert!(segs[3].normal[1].abs() < 1e-9);
    }

    #[test]
    fn a_curve_rasterises_into_one_column_a_step() {
        let c = BezierConnection::new(
            [[3, 64, 0], [10, 64, 7]],
            [[3.0, 64.0, 0.5], [10.5, 64.0, 8.0]],
            [[1.0, 0.0, 0.0], [0.0, 0.0, -1.0]],
            [UP, UP],
            true,
            false,
            "create:andesite",
        );
        let cols = c.rasterise();
        assert!(cols.len() >= 8 && cols.len() <= 12, "{}", cols.len());
        for (k, y) in &cols {
            // 3/16 up, then the quarter below along the flat curve's normal: relative to y 64.
            assert!((y - (64.0 - 64.0 + 0.1875 - 0.25)).abs() < 1e-6, "{k:?} {y}");
        }
        // Relative to the first end's block: starts at (0, 0), ends near (7, 7).
        assert!(cols.contains_key(&(0, 0)));
        assert!(cols.keys().any(|k| k.0 >= 6 && k.1 >= 6));
    }
}
