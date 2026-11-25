//! `TrackBlockEntityTilt` and `TrackModel` (content/trains/track/TrackBlockEntityTilt.java, TrackModel.java): a flat track block
//! between two curves that climb to opposite sides is tilted so the slope is smooth, and its two curves' ends are raised in
//! 16ths of a block (`BezierConnection.smoothing`, read back as `yOffsetAt`).
//!
//! Ported: `tryApplySmoothing` (as [`try_apply_smoothing`], which works out what the two curves become and the angle; the glue
//! writes them to the block entities), `restoreToOriginalCurve`, the decision of `undoSmoothing` ([`should_undo`]),
//! `getYOffsetForAxisEnd`, `compareHandles`, `BezierConnection.yOffsetAt` (in `bezier.rs`) and `TrackModel`'s vertex transform
//! ([`tilt_point`]).
//!
//! Known differences: `Mth.atan2` is a lookup table in Minecraft, `f64::atan2` here; the curves are rebuilt through
//! [`BezierConnection::new`] (Java mutates them and keeps their old runtime until the next load, which sees the new values, as this does).

use super::bezier::BezierConnection;
use super::graph::TrackNodeLocation;
use super::track_shape::TrackShape;
use super::vec::{Vec3, add, dot, normalize, scale, sub};

/// `TrackBlockEntityTilt.compareHandles`: the two points are the same node column (half-block grid) within `1 / 512` (squared, y ignored).
pub fn compare_handles(a: Vec3, b: Vec3) -> bool {
    let flat = |v: Vec3| {
        let l = TrackNodeLocation::new(v, 0).location();
        [l[0], 0.0, l[2]]
    };
    let d = sub(flat(a), flat(b));
    dot(d, d) < f64::from(1.0_f32 / 512.0)
}

/// `TrackBlockEntityTilt.getYOffsetForAxisEnd(end)`: `tilted` is `smoothingAngle.isPresent()`, `previous` the handles
/// `captureSmoothingHandles` kept (start and first smoothing value of each curve) from before a curve was removed.
pub fn y_offset_for_axis_end(tilted: bool, curves: &[BezierConnection], previous: &[(Vec3, i32)], end: Vec3) -> i32 {
    if !tilted {
        return 0;
    }
    for bc in curves {
        if compare_handles(bc.starts[0], end) {
            return bc.y_offset_at(end);
        }
    }
    for (handle, offset) in previous {
        if compare_handles(*handle, end) {
            return *offset;
        }
    }
    0
}

/// `captureSmoothingHandles`: each curve's first start and first smoothing value (0 without any).
pub fn capture_handles(curves: &[BezierConnection]) -> Vec<(Vec3, i32)> {
    curves.iter().map(|bc| (bc.starts[0], bc.smoothing.map_or(0, |s| s[0]))).collect()
}

/// What `tryApplySmoothing` made: the angle (`smoothingAngle`, degrees, negative when flipped) and the two curves it changed, the
/// lower one first. The glue sets both on the block entity and each one's `secondary()` on the block at its other end.
#[derive(Clone, Debug, PartialEq)]
pub struct Smoothed {
    pub angle: f64,
    pub curves: [BezierConnection; 2],
}

/// The same curve with other starts, axes and smoothing (the runtime is worked out again).
fn rebuilt(bc: &BezierConnection, starts: [Vec3; 2], axes: [Vec3; 2], smoothing: Option<[i32; 2]>) -> BezierConnection {
    let mut out = BezierConnection::new(bc.be_positions, starts, axes, bc.normals, bc.primary, bc.has_girder, bc.material.clone());
    out.smoothing = smoothing;
    out
}

/// `TrackBlockEntityTilt.tryApplySmoothing()` for a block that isn't tilted yet: `axes` are the block's track axes
/// (`getTrackAxes`), `bound` whether it has a `boundLocation` (a portal), `connections` its curves.
///
/// `None` when the block isn't a flat run of one axis with exactly two curves that go opposite ways, one falling and one climbing
/// away from it.
pub fn try_apply_smoothing(axes: &[Vec3], bound: bool, connections: &[BezierConnection]) -> Option<Smoothed> {
    if axes.len() != 1 || axes[0][1] != 0.0 || bound {
        return None;
    }
    let mut first: Option<&BezierConnection> = None;
    let mut second: Option<&BezierConnection> = None;
    let mut axis: Option<Vec3> = None;
    for bc in connections {
        if bc.starts[0][1] == bc.starts[1][1] {
            continue;
        }
        let normed = normalize(bc.axes[0]);
        if let Some(axis) = axis {
            if second.is_some() {
                return None;
            }
            if dot(normed, axis) > -1.0 + f64::from(1.0_f32 / 64.0) {
                return None;
            }
            second = Some(bc);
            continue;
        }
        axis = Some(normed);
        first = Some(bc);
    }
    let (mut low, mut high) = (first?, second?);
    if low.starts[1][1] > high.starts[1][1] {
        std::mem::swap(&mut low, &mut high);
    }
    let (low_starts, high_starts) = (low.starts, high.starts);
    let (lowest, highest) = (low_starts[1], high_starts[1]);
    if lowest[1] > low_starts[0][1] || highest[1] < high_starts[0][1] {
        return None;
    }

    let h_distance = low.length() + high.length();
    let base_axis = low.axes[0];
    let base_axis_length = if base_axis[0] != 0.0 && base_axis[2] != 0.0 { std::f64::consts::SQRT_2 } else { 1.0 };
    let v_distance = highest[1] - lowest[1];
    let m = v_distance / h_distance;

    let diff = sub(high_starts[0], low_starts[0]);
    let flip_rotation = dot(diff, normalize([1.0, 0.0, 2.0])) <= 0.0;
    let angle = m.atan2(1.0).to_degrees() * if flip_rotation { -1.0 } else { 1.0 };

    let param = ((m * base_axis_length * 16.0) as i32).clamp(0, 15);
    let raised = scale(
        normalize(add(normalize(diff), [0.0, m.clamp(0.0, 1.0 - f64::from(1.0_f32 / 512.0)), 0.0])),
        base_axis_length,
    );
    let raised_start = add(low_starts[0], raised);

    let make = |bc: &BezierConnection, is_first: bool| {
        let mut starts = bc.starts;
        if !is_first {
            starts[0] = raised_start;
        }
        let apply = if is_first { 0 } else { param };
        let smoothing = Some([apply, bc.smoothing.map_or(0, |s| s[1])]);
        let mut axes = bc.axes;
        axes[0] = normalize(add(axes[0], [0.0, (if is_first { 1.0 } else { -1.0 }) * -m, 0.0]));
        rebuilt(bc, starts, axes, smoothing)
    };
    Some(Smoothed { angle, curves: [make(low, true), make(high, false)] })
}

/// `restoreToOriginalCurve`: the first end back to the grid node and flat, no smoothing at it (none at all once both are 0).
pub fn restore_to_original_curve(bc: &BezierConnection) -> BezierConnection {
    let mut smoothing = bc.smoothing;
    if let Some(s) = &mut smoothing {
        s[0] = 0;
        if s[0] == 0 && s[1] == 0 {
            smoothing = None;
        }
    }
    let mut starts = bc.starts;
    starts[0] = TrackNodeLocation::new(starts[0], 0).location();
    let mut axes = bc.axes;
    axes[0] = normalize([axes[0][0], 0.0, axes[0][2]]);
    rebuilt(bc, starts, axes, smoothing)
}

/// `TrackBlockEntity.transform`'s restore of both ends: `restore(restore(secondary()).secondary())`.
pub fn restore_both_ends(bc: &BezierConnection) -> BezierConnection {
    restore_to_original_curve(&restore_to_original_curve(&bc.secondary()).secondary())
}

/// `undoSmoothing`'s guard: a tilted block that remembers its handles and no longer has exactly two curves lets go of the tilt.
pub fn should_undo(tilted: bool, has_previous_handles: bool, connection_count: usize) -> bool {
    tilted && has_previous_handles && connection_count != 2
}

fn rotate_axis(v: Vec3, degrees: f64, axis: usize) -> Vec3 {
    let (s, c) = degrees.to_radians().sin_cos();
    match axis {
        0 => [v[0], v[1] * c - v[2] * s, v[2] * c + v[1] * s],
        1 => [v[0] * c + v[2] * s, v[1], v[2] * c - v[0] * s],
        _ => [v[0] * c - v[1] * s, v[1] * c + v[0] * s, v[2]],
    }
}

fn rotate_centered(v: Vec3, degrees: f64, axis: usize) -> Vec3 {
    let centre = [0.5; 3];
    add(rotate_axis(sub(v, centre), degrees, axis), centre)
}

/// `TrackModel.getQuads`' vertex transform: where a corner of a straight track's model goes when the block is tilted by `angle_in`
/// (`smoothingAngle`, degrees).
pub fn tilt_point(v: Vec3, shape: TrackShape, angle_in: f64) -> Vec3 {
    let angle = angle_in.abs();
    let flip = angle_in < 0.0;
    let h_angle = match shape {
        TrackShape::Pd => 45.0,
        TrackShape::Zo => 90.0,
        TrackShape::Nd => 135.0,
        _ => 0.0,
    };
    let vertical = [0.0, -0.25, 0.0];
    let diagonal = if matches!(shape, TrackShape::Nd | TrackShape::Pd) {
        [(std::f64::consts::SQRT_2 - 1.0) / 2.0, 0.0, 0.0]
    } else {
        [0.0; 3]
    };
    let mut v = add(v, vertical);
    v = rotate_centered(v, h_angle, 1);
    v = add(v, diagonal);
    v = rotate_axis(v, angle, 2);
    v = sub(v, diagonal);
    v = rotate_centered(v, -h_angle + if flip { 180.0 } else { 0.0 }, 1);
    sub(v, vertical)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-9)
    }

    /// A block at x 0 to 1 on a run along x: a curve falling to the west and one climbing to the east.
    fn slope() -> Vec<BezierConnection> {
        let west = BezierConnection::new(
            [[0, 64, 0], [-8, 63, 0]],
            [[0.0, 64.0, 0.5], [-8.0, 63.0, 0.5]],
            [[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
            [[0.0, 1.0, 0.0]; 2],
            true,
            false,
            "create:andesite",
        );
        let east = BezierConnection::new(
            [[0, 64, 0], [9, 65, 0]],
            [[1.0, 64.0, 0.5], [9.0, 65.0, 0.5]],
            [[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0]],
            [[0.0, 1.0, 0.0]; 2],
            true,
            false,
            "create:andesite",
        );
        vec![east, west]
    }

    #[test]
    fn a_flat_block_between_a_fall_and_a_climb_is_smoothed() {
        let smoothed = try_apply_smoothing(&[[1.0, 0.0, 0.0]], false, &slope()).expect("a slope");
        let [low, high] = &smoothed.curves;
        // The lower curve (to the west) keeps its end; the climbing one's end rises by a share of the block and gets the offset.
        assert_eq!(low.starts[0], [0.0, 64.0, 0.5]);
        assert_eq!(low.smoothing, Some([0, 0]));
        assert!(high.starts[0][1] > 64.0, "{:?}", high.starts[0]);
        let param = high.smoothing.expect("smoothing")[0];
        assert!((1..=15).contains(&param), "{param}");
        // The angle is the slope's: 2 up over the two curves' length, about 7 degrees; it points one way for east.
        assert!(smoothed.angle.abs() > 3.0 && smoothed.angle.abs() < 12.0, "{}", smoothed.angle);
        // The axes lean with the slope: the low one down, the high one up.
        assert!(low.axes[0][1] < 0.0 && high.axes[0][1] > 0.0, "{:?} {:?}", low.axes[0], high.axes[0]);
    }

    #[test]
    fn nothing_is_smoothed_without_a_slope_either_side_or_on_a_portal_or_a_slope() {
        let curves = slope();
        assert!(try_apply_smoothing(&[[1.0, 0.0, 0.0]], false, &curves[..1]).is_none());
        assert!(try_apply_smoothing(&[[1.0, 0.0, 0.0]], true, &curves).is_none());
        assert!(try_apply_smoothing(&[[1.0, 1.0, 0.0]], false, &curves).is_none());
        assert!(try_apply_smoothing(&[[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]], false, &curves).is_none());
        // Two curves going the same way are not a through slope.
        let same = vec![curves[0].clone(), curves[0].clone()];
        assert!(try_apply_smoothing(&[[1.0, 0.0, 0.0]], false, &same).is_none());
    }

    #[test]
    fn a_curve_that_does_not_fall_away_on_the_low_side_is_not_smoothed() {
        // Both curves climb away: the lower one's far end is above its start.
        let mut curves = slope();
        curves[1] = BezierConnection::new(
            [[0, 64, 0], [-8, 65, 0]],
            [[0.0, 64.0, 0.5], [-8.0, 65.0, 0.5]],
            [[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
            [[0.0, 1.0, 0.0]; 2],
            true,
            false,
            "create:andesite",
        );
        assert!(try_apply_smoothing(&[[1.0, 0.0, 0.0]], false, &curves).is_none());
    }

    #[test]
    fn the_y_offset_is_the_smoothing_of_the_end_asked_about() {
        let smoothed = try_apply_smoothing(&[[1.0, 0.0, 0.0]], false, &slope()).unwrap();
        let high = &smoothed.curves[1];
        let param = high.smoothing.unwrap()[0];
        assert_eq!(high.y_offset_at([1.0, 64.0, 0.5]), param);
        assert_eq!(high.y_offset_at(high.starts[1]), 0);
        assert_eq!(slope()[0].y_offset_at([1.0, 64.0, 0.5]), 0, "a curve without smoothing never offsets");
        // The block answers for an end through its curves, and from what it kept once the curve is gone.
        let curves = smoothed.curves.to_vec();
        assert_eq!(y_offset_for_axis_end(true, &curves, &[], [1.0, 64.0, 0.5]), param);
        assert_eq!(y_offset_for_axis_end(false, &curves, &[], [1.0, 64.0, 0.5]), 0);
        let kept = capture_handles(&curves);
        assert_eq!(y_offset_for_axis_end(true, &[], &kept, [1.0, 64.0, 0.5]), param);
        assert_eq!(y_offset_for_axis_end(true, &[], &[], [1.0, 64.0, 0.5]), 0);
    }

    #[test]
    fn handles_compare_on_the_half_block_grid_without_height() {
        assert!(compare_handles([1.0, 64.0, 0.5], [1.01, 70.0, 0.49]));
        assert!(!compare_handles([1.0, 64.0, 0.5], [1.5, 64.0, 0.5]));
    }

    #[test]
    fn restoring_a_curve_puts_its_first_end_back_flat() {
        let smoothed = try_apply_smoothing(&[[1.0, 0.0, 0.0]], false, &slope()).unwrap();
        let high = &smoothed.curves[1];
        let restored = restore_to_original_curve(high);
        assert_eq!(restored.smoothing, None);
        assert_eq!(restored.starts[0][1], 64.0);
        assert_eq!(restored.axes[0][1], 0.0);
        assert!((restored.axes[0][0] - 1.0).abs() < 1e-9);
        // A curve smoothed at its far end keeps that.
        let mut both = high.clone();
        both.smoothing = Some([3, 2]);
        assert_eq!(restore_to_original_curve(&both).smoothing, Some([0, 2]));
    }

    #[test]
    fn the_tilt_lets_go_only_of_a_remembering_block_without_two_curves() {
        assert!(should_undo(true, true, 1));
        assert!(should_undo(true, true, 0));
        assert!(!should_undo(true, true, 2));
        assert!(!should_undo(true, false, 1));
        assert!(!should_undo(false, true, 1));
    }

    #[test]
    fn a_tilted_model_turns_about_its_middle_and_flips_with_the_angle() {
        // The model turns about the end of the track at the block's low-x edge (at the rail's height): that end stays, the other rises.
        let angle = 10.0;
        let ahead = tilt_point([1.0, 0.25, 0.5], TrackShape::Xo, angle);
        let behind = tilt_point([0.0, 0.25, 0.5], TrackShape::Xo, angle);
        assert!(close(behind, [0.0, 0.25, 0.5]), "{behind:?}");
        assert!((ahead[1] - (0.25 + angle.to_radians().sin())).abs() < 1e-9, "{ahead:?}");
        assert!((ahead[0] - angle.to_radians().cos()).abs() < 1e-9 && (ahead[2] - 0.5).abs() < 1e-9, "{ahead:?}");
        // A negative angle tips it the other way: the same lift, mirrored about the block's middle.
        let flipped = tilt_point([1.0, 0.25, 0.5], TrackShape::Xo, -angle);
        assert!((flipped[1] - ahead[1]).abs() < 1e-9 && (flipped[0] - (1.0 - ahead[0])).abs() < 1e-9, "{flipped:?}");
        // Along z (turned a quarter) the lift is along z instead.
        let along_z = tilt_point([0.5, 0.25, 1.0], TrackShape::Zo, angle);
        assert!((along_z[1] - ahead[1]).abs() < 1e-9 && (along_z[0] - 0.5).abs() < 1e-9, "{along_z:?}");
    }
}
