//! `TrackPlacement.tryConnect` (content/trains/track/TrackPlacement.java): is a curve between two
//! track ends allowed, and how long are the straight runs added at each end.
//!
//! This is the geometry only: the rules about minimum radius, turn angles, S-curves and slopes, and
//! the translation keys of each refusal ([`PlacementError::key`]). The world side stays with the
//! glue: reading the clicked block's axis and normal, the inventory and pavement bookkeeping
//! (`placeTracks`, `paveTracks`), the player's look-angle cache and the client overlay. The glue
//! hands in the two ends ([`PlacementInput`]) and gets back a [`PlacementInfo`].
//!
//! Known differences: where Java would throw (a null `VecHelper.intersect` in the slope branch) the
//! result is `too_sharp`; `Mth.atan2` is `f64::atan2`.

use super::bezier::BezierConnection;
use super::vec::{
    Plane, UP, Vec3, add, center_of, cross, deg, distance_to, dot, equal, floor, intersect, length, normalize, round,
    scale, sub, block_containing,
};
use crate::Pos;

/// Why a connection was refused. [`key`](PlacementError::key) is the `track.<key>` lang key
/// `PlacementInfo.withMessage` stores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlacementError {
    SecondPoint,
    TooFar,
    OriginalMissing,
    TurnStart,
    Perpendicular,
    AscendingSCurve,
    TooSharp,
    SlopeTurn,
    OpposingSlopes,
    LeaveSlopeAscending,
    LeaveSlopeDescending,
    TooSteep,
    Turn90,
    /// Raised by the inventory check, which the glue runs: kept so the keys live in one place.
    NotEnoughTracks,
    NotEnoughPavement,
}

impl PlacementError {
    /// `"track." + message`.
    pub fn key(self) -> &'static str {
        match self {
            PlacementError::SecondPoint => "track.second_point",
            PlacementError::TooFar => "track.too_far",
            PlacementError::OriginalMissing => "track.original_missing",
            PlacementError::TurnStart => "track.turn_start",
            PlacementError::Perpendicular => "track.perpendicular",
            PlacementError::AscendingSCurve => "track.ascending_s_curve",
            PlacementError::TooSharp => "track.too_sharp",
            PlacementError::SlopeTurn => "track.slope_turn",
            PlacementError::OpposingSlopes => "track.opposing_slopes",
            PlacementError::LeaveSlopeAscending => "track.leave_slope_ascending",
            PlacementError::LeaveSlopeDescending => "track.leave_slope_descending",
            PlacementError::TooSteep => "track.too_steep",
            PlacementError::Turn90 => "track.turn_90",
            PlacementError::NotEnoughTracks => "track.not_enough_tracks",
            PlacementError::NotEnoughPavement => "track.not_enough_pavement",
        }
    }
}

/// `TrackPlacement.ConnectingFrom`: where the player started the connection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConnectingFrom {
    pub pos: Pos,
    pub axis: Vec3,
    pub normal: Vec3,
    pub end: Vec3,
}

/// Everything `tryConnect` reads from the world and the player, as plain values.
pub struct PlacementInput<'a> {
    /// The end the player started from (`TRACK_CONNECTING_FROM`).
    pub from: ConnectingFrom,
    /// Whether the block at `from.pos` still has the `HAS_BE` property (a track block).
    pub original_has_be: bool,
    /// The clicked end: its block, `getNearestTrackAxis` already turned to point away from the
    /// look direction (`axis.scale(POSITIVE ? -1 : 1)`), `getUpNormal` normalised, and `getCurveStart`.
    pub pos2: Pos,
    pub axis2: Vec3,
    pub normal2: Vec3,
    pub end2: Vec3,
    /// Whether the clicked block's entity is tilted (`TrackBlockEntity.isTilted`).
    pub target_tilted: bool,
    /// `getCurveStart(level, pos, state, axis)` of the starting block (for a flipped `axis1`) and
    /// of the clicked block (for a flipped `axis2`).
    pub curve_start1: &'a dyn Fn(Vec3) -> Vec3,
    pub curve_start2: &'a dyn Fn(Vec3) -> Vec3,
    /// `maxTrackPlacementLength` (config, default 32).
    pub max_length: i32,
    /// Sprint held: the longest turn instead of the standard-sized one.
    pub maximise_turn: bool,
    pub girder: bool,
    pub material: String,
    /// `level.isClientSide`: the curve is set up front for the preview, even if the result is a refusal.
    pub client: bool,
}

/// `TrackPlacement.PlacementInfo`, the geometric part.
#[derive(Clone, Debug)]
pub struct PlacementInfo {
    pub curve: Option<BezierConnection>,
    pub valid: bool,
    pub end1_extent: i32,
    pub end2_extent: i32,
    pub error: Option<PlacementError>,
    pub end1: Vec3,
    pub end2: Vec3,
    pub normal1: Vec3,
    pub normal2: Vec3,
    pub axis1: Vec3,
    pub axis2: Vec3,
    pub pos1: Pos,
    pub pos2: Pos,
    /// The block entities the curve joins (`pos1 + extent`, `pos2 + extent`) once valid.
    pub target_pos1: Pos,
    pub target_pos2: Pos,
}

fn offset_pos(pos: Pos, by: Pos) -> Pos {
    [pos[0] + by[0], pos[1] + by[1], pos[2] + by[2]]
}

fn pos_dist_sqr(a: Pos, b: Pos) -> f64 {
    let d = [f64::from(a[0] - b[0]), f64::from(a[1] - b[1]), f64::from(a[2] - b[2])];
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
}

/// `tryConnect`, up to and including `info.valid = true` and the positions: everything that decides
/// whether the two ends can be joined and with what straight extensions. Returns the info with
/// `valid` and `error` set.
pub fn try_connect(input: &PlacementInput) -> PlacementInfo {
    let from = input.from;
    let (pos1, mut axis1, mut end1, normal1) = (from.pos, from.axis, from.end, from.normal);
    let pos2 = input.pos2;
    let normal2 = input.normal2;
    let mut axis2 = input.axis2;
    let mut end2 = input.end2;

    let mut normed_axis1 = normalize(axis1);
    let mut normed_axis2 = normalize(axis2);

    let mut info = PlacementInfo {
        curve: None,
        valid: false,
        end1_extent: 0,
        end2_extent: 0,
        error: None,
        end1,
        end2,
        normal1,
        normal2,
        axis1,
        axis2,
        pos1,
        pos2,
        target_pos1: pos1,
        target_pos2: pos2,
    };

    let max_length = i64::from(input.max_length);
    if pos1 == pos2 {
        info.error = Some(PlacementError::SecondPoint);
        return info;
    }
    if pos_dist_sqr(pos1, pos2) > (max_length * max_length) as f64 {
        info.error = Some(PlacementError::TooFar);
        info.curve = None;
        return info;
    }
    if !input.original_has_be {
        info.error = Some(PlacementError::OriginalMissing);
        return info;
    }
    if input.target_tilted {
        info.error = Some(PlacementError::TurnStart);
        return info;
    }

    if dot(axis1, sub(end2, end1)) < 0.0 {
        axis1 = scale(axis1, -1.0);
        normed_axis1 = scale(normed_axis1, -1.0);
        end1 = (input.curve_start1)(axis1);
        info.end1 = end1;
        info.axis1 = axis1;
    }

    let mut hit = intersect(end1, end2, normed_axis1, normed_axis2, Plane::Y);
    let parallel = hit.is_none();
    let mut skip_curve = false;

    if (parallel && dot(normed_axis1, normed_axis2) > 0.0)
        || (!parallel && hit.is_some_and(|h| h[0] < 0.0 || h[1] < 0.0))
    {
        axis2 = scale(axis2, -1.0);
        normed_axis2 = scale(normed_axis2, -1.0);
        end2 = (input.curve_start2)(axis2);
        info.end2 = end2;
        info.axis2 = axis2;
    }

    let cross2 = cross(normed_axis2, UP);

    let a1 = normed_axis2[2].atan2(normed_axis2[0]);
    let a2 = normed_axis1[2].atan2(normed_axis1[0]);
    let angle = a1 - a2;
    let ascend = sub(end2, end1)[1];
    let abs_ascend = ascend.abs();
    let slope = normal1 != normal2;

    let make_curve = |info: &PlacementInfo, end1: Vec3, end2: Vec3, axis1: Vec3, axis2: Vec3| {
        let offset1 = scale(axis1, f64::from(info.end1_extent));
        let offset2 = scale(axis2, f64::from(info.end2_extent));
        let target1 = offset_pos(pos1, block_containing(offset1));
        let target2 = offset_pos(pos2, block_containing(offset2));
        BezierConnection::new(
            [target1, target2],
            [add(end1, offset1), add(end2, offset2)],
            [normed_axis1, normed_axis2],
            [normal1, normal2],
            true,
            input.girder,
            input.material.clone(),
        )
    };

    if input.client {
        info.curve = Some(make_curve(&info, end1, end2, axis1, axis2));
    }

    macro_rules! refuse {
        ($e:expr) => {{
            info.error = Some($e);
            return info;
        }};
        ($e:expr, jumbly) => {{
            info.error = Some($e);
            info.curve = None;
            return info;
        }};
    }

    // S curve or straight

    let mut dist = 0.0;

    if parallel {
        if let Some(s_test) = intersect(end1, end2, normed_axis1, cross2, Plane::Y) {
            let t = s_test[0].abs();
            let u = s_test[1].abs();

            skip_curve = equal(u, 0.0);

            if !skip_curve && s_test[0] < 0.0 {
                refuse!(PlacementError::Perpendicular, jumbly);
            }

            if skip_curve {
                dist = distance_to(center_of(pos1), center_of(pos2));
                info.end1_extent = round((dist + 1.0) / length(axis1)) as i32;
            } else {
                if !equal(ascend, 0.0) || normed_axis1[1] != 0.0 {
                    refuse!(PlacementError::AscendingSCurve);
                }

                let target_t = if u <= 1.0 { 3.0 } else { u * 2.0 };

                if t < target_t {
                    refuse!(PlacementError::TooSharp);
                }

                // This is for standardising s curve sizes
                if t > target_t {
                    let correction = ((t - target_t) / length(axis1)) as i32;
                    info.end1_extent = if input.maximise_turn { 0 } else { correction / 2 + (correction % 2) };
                    info.end2_extent = if input.maximise_turn { 0 } else { correction / 2 };
                }
            }
        }
    }

    // Slope

    if slope {
        if !skip_curve {
            refuse!(PlacementError::SlopeTurn);
        }
        if equal(dot(normal1, normal2), 0.0) {
            refuse!(PlacementError::OpposingSlopes);
        }
        if (axis1[1] < 0.0 || axis2[1] > 0.0) && ascend > 0.0 {
            refuse!(PlacementError::LeaveSlopeAscending);
        }
        if (axis1[1] > 0.0 || axis2[1] < 0.0) && ascend < 0.0 {
            refuse!(PlacementError::LeaveSlopeDescending);
        }

        skip_curve = false;
        info.end1_extent = 0;
        info.end2_extent = 0;

        let plane = if equal(axis1[0], 0.0) { Plane::X } else { Plane::Z };
        hit = intersect(end1, end2, normed_axis1, normed_axis2, plane);
        let Some(h) = hit else {
            // Java dereferences the null here; refuse as too sharp instead.
            refuse!(PlacementError::TooSharp, jumbly);
        };
        let dist1 = (h[0] / length(axis1)).abs();
        let dist2 = (h[1] / length(axis2)).abs();

        if dist1 > dist2 {
            info.end1_extent = round(dist1 - dist2) as i32;
        }
        if dist2 > dist1 {
            info.end2_extent = round(dist2 - dist1) as i32;
        }

        let mut turn_size = dist1.min(dist2);
        if h[0] < 0.0 || h[1] < 0.0 {
            refuse!(PlacementError::TooSharp, jumbly);
        }
        if turn_size < 2.0 {
            refuse!(PlacementError::TooSharp);
        }

        // This is for standardising curve sizes
        if turn_size > 2.0 && !input.maximise_turn {
            // `int += double` narrows toward zero.
            info.end1_extent = (f64::from(info.end1_extent) + (turn_size - 2.0)) as i32;
            info.end2_extent = (f64::from(info.end2_extent) + (turn_size - 2.0)) as i32;
            turn_size = 2.0;
        }
        let _ = turn_size;
    }

    // Straight ascend

    if skip_curve && !equal(ascend, 0.0) {
        let h_distance = info.end1_extent;
        if axis1[1] == 0.0 || !equal(abs_ascend + 1.0, dist / length(axis1)) {
            if axis1[1] != 0.0 && axis1[1] == -axis2[1] {
                refuse!(PlacementError::AscendingSCurve);
            }

            info.end1_extent = 0;
            let min_h_distance =
                (if abs_ascend < 4.0 { abs_ascend * 4.0 } else { abs_ascend * 3.0 }).max(6.0) / length(axis1);
            if f64::from(h_distance) < min_h_distance {
                refuse!(PlacementError::TooSteep);
            }
            if f64::from(h_distance) > min_h_distance {
                let correction = (f64::from(h_distance) - min_h_distance) as i32;
                info.end1_extent = if input.maximise_turn { 0 } else { correction / 2 + (correction % 2) };
                info.end2_extent = if input.maximise_turn { 0 } else { correction / 2 };
            }

            skip_curve = false;
        }
    }

    // Turn

    if !parallel {
        let abs_angle = deg(angle).abs();
        if abs_angle < 60.0 || abs_angle > 300.0 {
            refuse!(PlacementError::Turn90, jumbly);
        }

        let Some(h) = intersect(end1, end2, normed_axis1, normed_axis2, Plane::Y) else {
            refuse!(PlacementError::TooSharp, jumbly);
        };
        let dist1 = h[0].abs();
        let dist2 = h[1].abs();
        let mut ex1: f32 = 0.0;
        let mut ex2: f32 = 0.0;

        if dist1 > dist2 {
            ex1 = ((dist1 - dist2) / length(axis1)) as f32;
        }
        if dist2 > dist1 {
            ex2 = ((dist2 - dist1) / length(axis2)) as f32;
        }

        let turn_size = dist1.min(dist2) - 0.1;
        let ninety = (abs_angle + 0.25_f32) % 90.0 < 1.0;

        if h[0] < 0.0 || h[1] < 0.0 {
            refuse!(PlacementError::TooSharp, jumbly);
        }

        let min_turn_size = if ninety { 7.0 } else { 3.25 };
        let turn_size_to_fit_ascend = min_turn_size
            + if ninety {
                (abs_ascend - 3.0).max(0.0) * 2.0
            } else {
                (abs_ascend - 1.5).max(0.0) * 1.5
            };

        if turn_size < min_turn_size {
            refuse!(PlacementError::TooSharp);
        }
        if turn_size < turn_size_to_fit_ascend {
            refuse!(PlacementError::TooSteep);
        }

        // This is for standardising curve sizes
        if !input.maximise_turn {
            // `float += double`.
            ex1 = (f64::from(ex1) + (turn_size - turn_size_to_fit_ascend) / length(axis1)) as f32;
            ex2 = (f64::from(ex2) + (turn_size - turn_size_to_fit_ascend) / length(axis2)) as f32;
        }
        info.end1_extent = floor(f64::from(ex1));
        info.end2_extent = floor(f64::from(ex2));
    }

    let offset1 = scale(axis1, f64::from(info.end1_extent));
    let offset2 = scale(axis2, f64::from(info.end2_extent));
    let target_pos1 = offset_pos(pos1, block_containing(offset1));
    let target_pos2 = offset_pos(pos2, block_containing(offset2));

    info.curve = if skip_curve { None } else { Some(make_curve(&info, end1, end2, axis1, axis2)) };
    info.valid = true;
    info.pos1 = pos1;
    info.pos2 = pos2;
    info.axis1 = axis1;
    info.axis2 = axis2;
    info.end1 = end1;
    info.end2 = end2;
    info.target_pos1 = target_pos1;
    info.target_pos2 = target_pos2;
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat ZO/XO track block: `getCurveStart` is the block's centre, half a block down, half the
    /// axis out (`TrackBlock.getCurveStart`).
    fn curve_start(pos: Pos) -> impl Fn(Vec3) -> Vec3 {
        move |axis| {
            let vertical = axis[1] != 0.0;
            add(add(center_of(pos), [0.0, if vertical { 0.0 } else { -0.5 }, 0.0]), scale(axis, 0.5))
        }
    }

    fn flat(from_pos: Pos, from_axis: Vec3, to_pos: Pos, to_axis: Vec3, maximise: bool) -> PlacementInfo {
        let cs1 = curve_start(from_pos);
        let cs2 = curve_start(to_pos);
        let input = PlacementInput {
            from: ConnectingFrom { pos: from_pos, axis: from_axis, normal: UP, end: cs1(from_axis) },
            original_has_be: true,
            pos2: to_pos,
            axis2: to_axis,
            normal2: UP,
            end2: cs2(to_axis),
            target_tilted: false,
            curve_start1: &cs1,
            curve_start2: &cs2,
            max_length: 32,
            maximise_turn: maximise,
            girder: false,
            material: "create:andesite".into(),
            client: false,
        };
        try_connect(&input)
    }

    #[test]
    fn straight_line_has_no_curve_and_extends_between() {
        let info = flat([0, 64, 0], [1.0, 0.0, 0.0], [10, 64, 0], [-1.0, 0.0, 0.0], false);
        assert!(info.valid, "{:?}", info.error);
        assert!(info.curve.is_none());
        // 10 blocks apart: (10 + 1) / 1 extension on the first end.
        assert_eq!(info.end1_extent, 11);
        assert_eq!(info.end2_extent, 0);
    }

    #[test]
    fn same_block_and_too_far_are_refused() {
        let same = flat([0, 64, 0], [1.0, 0.0, 0.0], [0, 64, 0], [1.0, 0.0, 0.0], false);
        assert_eq!(same.error, Some(PlacementError::SecondPoint));
        let far = flat([0, 64, 0], [1.0, 0.0, 0.0], [40, 64, 0], [1.0, 0.0, 0.0], false);
        assert_eq!(far.error, Some(PlacementError::TooFar));
        assert_eq!(PlacementError::TooFar.key(), "track.too_far");
    }

    #[test]
    fn a_ninety_degree_turn_needs_room() {
        // A quarter turn with a seven-block leg each way: valid.
        let ok = flat([0, 64, 0], [1.0, 0.0, 0.0], [9, 64, 9], [0.0, 0.0, 1.0], true);
        assert!(ok.valid, "{:?}", ok.error);
        let curve = ok.curve.expect("a turn has a curve");
        assert!(curve.radius() > 5.0);
        // Too tight for a ninety: min turn size 7.
        let tight = flat([0, 64, 0], [1.0, 0.0, 0.0], [5, 64, 5], [0.0, 0.0, 1.0], true);
        assert_eq!(tight.error, Some(PlacementError::TooSharp));
    }

    #[test]
    fn a_standard_turn_is_trimmed_unless_maximised() {
        let std = flat([0, 64, 0], [1.0, 0.0, 0.0], [16, 64, 16], [0.0, 0.0, 1.0], false);
        let max = flat([0, 64, 0], [1.0, 0.0, 0.0], [16, 64, 16], [0.0, 0.0, 1.0], true);
        assert!(std.valid && max.valid);
        assert!(std.end1_extent + std.end2_extent >= max.end1_extent + max.end2_extent);
        assert!(max.curve.unwrap().radius() > std.curve.unwrap().radius() - 1e-9);
    }

    #[test]
    fn an_s_curve_needs_length() {
        // Parallel tracks one block apart sideways: needs t >= 3.
        let short = flat([0, 64, 0], [1.0, 0.0, 0.0], [2, 64, 2], [-1.0, 0.0, 0.0], false);
        assert_eq!(short.error, Some(PlacementError::TooSharp));
        let long = flat([0, 64, 0], [1.0, 0.0, 0.0], [12, 64, 2], [-1.0, 0.0, 0.0], false);
        assert!(long.valid, "{:?}", long.error);
        let c = long.curve.expect("an S-curve is a curve");
        assert_eq!(c.radius(), 0.0);
    }

    #[test]
    fn a_missing_original_and_a_tilted_target_are_refused() {
        let cs = curve_start([0, 64, 0]);
        let mut input = PlacementInput {
            from: ConnectingFrom { pos: [0, 64, 0], axis: [1.0, 0.0, 0.0], normal: UP, end: cs([1.0, 0.0, 0.0]) },
            original_has_be: false,
            pos2: [10, 64, 0],
            axis2: [-1.0, 0.0, 0.0],
            normal2: UP,
            end2: curve_start([10, 64, 0])([-1.0, 0.0, 0.0]),
            target_tilted: false,
            curve_start1: &cs,
            curve_start2: &cs,
            max_length: 32,
            maximise_turn: false,
            girder: false,
            material: String::new(),
            client: false,
        };
        assert_eq!(try_connect(&input).error, Some(PlacementError::OriginalMissing));
        input.original_has_be = true;
        input.target_tilted = true;
        assert_eq!(try_connect(&input).error, Some(PlacementError::TurnStart));
    }

    #[test]
    fn a_turn_through_more_than_a_right_angle_and_a_half_is_refused() {
        // Turning back on itself: the axes into the curve only 45 degrees apart.
        let cs1 = curve_start([0, 64, 0]);
        let cs2 = curve_start([10, 64, 10]);
        let a2 = normalize([1.0, 0.0, -1.0]);
        let input = PlacementInput {
            from: ConnectingFrom { pos: [0, 64, 0], axis: [1.0, 0.0, 0.0], normal: UP, end: cs1([1.0, 0.0, 0.0]) },
            original_has_be: true,
            pos2: [10, 64, 10],
            axis2: a2,
            normal2: UP,
            end2: cs2(a2),
            target_tilted: false,
            curve_start1: &cs1,
            curve_start2: &cs2,
            max_length: 32,
            maximise_turn: false,
            girder: false,
            material: String::new(),
            client: false,
        };
        let info = try_connect(&input);
        assert!(!info.valid);
        assert_eq!(info.error, Some(PlacementError::Turn90));
    }

    #[test]
    fn a_climb_needs_run() {
        // Two up over five blocks is too steep: it needs at least max(2 x 4, 6) = 8 blocks.
        let steep = flat([0, 64, 0], [1.0, 0.0, 0.0], [5, 66, 0], [-1.0, 0.0, 0.0], false);
        assert_eq!(steep.error, Some(PlacementError::TooSteep));
        // Over twelve it is fine, and becomes a curve (the ramp) with the spare length shared out.
        let ok = flat([0, 64, 0], [1.0, 0.0, 0.0], [12, 66, 0], [-1.0, 0.0, 0.0], false);
        assert!(ok.valid, "{:?}", ok.error);
        assert!(ok.curve.is_some());
        assert_eq!(ok.end1_extent, 3);
        assert_eq!(ok.end2_extent, 2);
        // Maximised, no spare is spent.
        let max = flat([0, 64, 0], [1.0, 0.0, 0.0], [12, 66, 0], [-1.0, 0.0, 0.0], true);
        assert!(max.valid);
        assert_eq!((max.end1_extent, max.end2_extent), (0, 0));
        assert_eq!(PlacementError::TooSteep.key(), "track.too_steep");
    }
}
