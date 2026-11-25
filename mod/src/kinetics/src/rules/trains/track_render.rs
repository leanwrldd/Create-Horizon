//! `TrackRenderer.getModelAngles` and `BezierConnection.SegmentAngles` (content/trains/track/TrackRenderer.java,
//! BezierConnection.java): where each tie and each rail piece of a curve is set down. The glue turns a [`Pose`] into
//! the draw calls (translate, three turns, translate, scale) and draws the tie and the two rail models with it.
//!
//! `GirderAngles` (the girder beams under a curve with a girder) is [`baked_girders`].
//!
//! Left out: the light position of each piece (`lightPosition`).

use super::bezier::BezierConnection;
use super::vec::{Vec3, add, cross, dot, length, normalize, scale, sub};

/// One part's place, as the chain `translate(at).rotateY(yaw).rotateX(pitch).rotateZ(roll).translate(then)` and then
/// a scale of the z axis. Angles in radians, as Create stores them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub at: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub then: Vec3,
    pub scale_z: f32,
}

/// The tie and the two rails of one piece of a curve (`SegmentAngles.tieTransform`, `railTransforms`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SegmentPose {
    pub index: usize,
    pub tie: Pose,
    /// The first rail (`+normal` side, the left segment model) and the second.
    pub rails: [Pose; 2],
}

/// `VecHelper.rotate(vec, degrees, Axis.X)`.
fn rotate_x(v: Vec3, degrees: f64) -> Vec3 {
    let (s, c) = degrees.to_radians().sin_cos();
    [v[0], v[1] * c - v[2] * s, v[2] * c + v[1] * s]
}

/// `VecHelper.rotate(vec, degrees, Axis.Y)`.
fn rotate_y(v: Vec3, degrees: f64) -> Vec3 {
    let (s, c) = degrees.to_radians().sin_cos();
    [v[0] * c + v[2] * s, v[1], v[2] * c - v[0] * s]
}

fn signum(v: f64) -> f64 {
    if v > 0.0 {
        1.0
    } else if v < 0.0 {
        -1.0
    } else {
        0.0
    }
}

/// `TrackRenderer.getModelAngles(normal, diff)`: (pitch, yaw, roll) in radians for a model that lies along `diff`.
/// `normal` is the curve segment's `normal`, the vector across the track (`faceNormal x derivative`), not its up.
pub fn model_angles(normal: Vec3, diff: Vec3) -> Vec3 {
    let len = (diff[0] * diff[0] + diff[2] * diff[2]).sqrt();
    let yaw = diff[0].atan2(diff[2]);
    let pitch = len.atan2(diff[1]) - std::f64::consts::PI * 0.5;
    let yaw_pitch_normal = rotate_y(rotate_x([0.0, 1.0, 0.0], pitch.to_degrees()), yaw.to_degrees());
    let mut sign = signum(dot(yaw_pitch_normal, normal));
    if sign.abs() < 0.5 {
        let d = sub(yaw_pitch_normal, normal);
        sign = if dot(d, d) < 0.5 { -1.0 } else { 1.0 };
    }
    let across = normalize(cross(diff, normal));
    let roll = dot(across, yaw_pitch_normal).clamp(-1.0, 1.0).acos() * sign;
    [pitch, yaw, roll]
}

fn pose(at: Vec3, angles: Vec3, then: Vec3, scale_z: f32) -> Pose {
    Pose { at, yaw: angles[1] as f32, pitch: angles[0] as f32, roll: angles[2] as f32, then, scale_z }
}

/// `SegmentAngles`: the ties and rails of a curve, relative to the first block entity (`Bezierator`'s offset).
/// The first sample only sets the start; each later one gives a piece back to the last.
pub fn baked_segments(bc: &BezierConnection) -> Vec<SegmentPose> {
    let count = bc.segment_count();
    let mut out = Vec::with_capacity(count);
    let mut previous: Option<[Vec3; 2]> = None;
    for segment in bc.segments() {
        let rail_offsets = [
            add(segment.position, scale(segment.normal, f64::from(0.965_f32))),
            sub(segment.position, scale(segment.normal, f64::from(0.965_f32))),
        ];
        let rail_middle = scale(add(rail_offsets[0], rail_offsets[1]), 0.5);
        let Some(prev) = previous else {
            previous = Some(rail_offsets);
            continue;
        };
        let end = segment.index == 0 || segment.index == count;
        let prev_middle = scale(add(prev[0], prev[1]), 0.5);
        let tie_angles = model_angles(segment.normal, sub(rail_middle, prev_middle));
        let low = -2.0 / 16.0 - 1.0 / 256.0;
        let tie = pose(prev_middle, tie_angles, [-0.5, low, 0.0], 1.0);
        let rail_scale = if end { 2.2_f32 } else { 2.1_f32 };
        let rail = |i: usize| {
            let diff = sub(rail_offsets[i], prev[i]);
            let angles = model_angles(segment.normal, diff);
            pose(prev[i], angles, [0.0, low, -1.0 / 32.0], length(diff) as f32 * rail_scale)
        };
        out.push(SegmentPose { index: segment.index, tie, rails: [rail(0), rail(1)] });
        previous = Some(rail_offsets);
    }
    out
}

/// One piece of girder under a curve (`GirderAngles.beams[i]`, `beamCaps[i]`): the middle model of the left and the right beam
/// and, for each of those, the top and the bottom cap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GirderPose {
    pub index: usize,
    /// `beams[i].get(first)`: `[left, right]` (`first` true is the left one).
    pub beams: [Pose; 2],
    /// `beamCaps[i].get(top).get(first)`: `[top, bottom][left, right]`.
    pub caps: [[Pose; 2]; 2],
}

/// `GirderAngles`: the girder beams of a curve with a girder, relative to the first block entity. Nothing for a curve without one.
/// As for the ties, the first sample only sets the start.
pub fn baked_girders(bc: &BezierConnection) -> Vec<GirderPose> {
    if !bc.has_girder {
        return Vec::new();
    }
    let count = bc.segment_count();
    let mut out = Vec::with_capacity(count);
    // `offsets`: `Couple(Couple(leftTop, rightTop), Couple(leftBottom, rightBottom))`, as `[top, bottom][left, right]`.
    let mut previous: Option<[[Vec3; 2]; 2]> = None;
    for segment in bc.segments() {
        let end = segment.index == 0 || segment.index == count;
        let up_normal = cross(normalize(segment.derivative), segment.normal);
        let first_offset = scale(up_normal, f64::from(-8.0_f32 / 16.0));
        let second_offset = scale(up_normal, f64::from(-10.0_f32 / 16.0));
        let left_top = add(add(segment.position, scale(segment.normal, 1.0)), first_offset);
        let right_top = add(sub(segment.position, scale(segment.normal, 1.0)), first_offset);
        let left_bottom = add(left_top, second_offset);
        let right_bottom = add(right_top, second_offset);
        let offsets = [[left_top, right_top], [left_bottom, right_bottom]];
        let Some(prev) = previous else {
            previous = Some(offsets);
            continue;
        };
        let scale_z = if end { 2.3_f32 } else { 2.2_f32 };
        let sign = if segment.index % 2 == 0 { 1.0_f32 } else { -1.0_f32 };
        let then = [0.0, f64::from(2.0_f32 / 16.0 + sign / 2048.0 - 1.0 / 1024.0), f64::from(-1.0_f32 / 32.0)];
        let mut beams = [None; 2];
        let mut caps = [[None; 2]; 2];
        for side in 0..2 {
            // Middle: halfway between the top and the bottom of the beam.
            let current_beam = scale(add(offsets[0][side], offsets[1][side]), 0.5);
            let previous_beam = scale(add(prev[0][side], prev[1][side]), 0.5);
            let beam_diff = sub(current_beam, previous_beam);
            let angles = model_angles(segment.normal, beam_diff);
            beams[side] = Some(pose(previous_beam, angles, then, length(beam_diff) as f32 * scale_z));
            for (cap, slot) in caps.iter_mut().enumerate() {
                let (current, before) = (offsets[cap][side], prev[cap][side]);
                let diff = sub(current, before);
                let angles = model_angles(segment.normal, diff);
                slot[side] = Some(pose(before, angles, then, length(diff) as f32 * scale_z));
            }
        }
        let beams = beams.map(|b| b.expect("both sides"));
        let caps = caps.map(|c| c.map(|p| p.expect("both sides")));
        out.push(GirderPose { index: segment.index, beams, caps });
        previous = Some(offsets);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn straight_curve(length: i32) -> BezierConnection {
        // A straight run along +x: the ends' axes point at each other.
        let a = [0.5 + 0.5, 64.0, 0.5];
        let b = [f64::from(length) + 0.5 - 0.5, 64.0, 0.5];
        BezierConnection::new(
            [[0, 64, 0], [length - 1, 64, 0]],
            [a, b],
            [[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0]],
            [[0.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
            true,
            false,
            "create:andesite",
        )
    }

    #[test]
    fn a_model_along_x_turns_a_quarter() {
        // Along +x with the up on y, the across vector is -z.
        let angles = model_angles([0.0, 0.0, -1.0], [1.0, 0.0, 0.0]);
        assert!(angles[0].abs() < 1e-9, "no pitch on the level: {angles:?}");
        assert!((angles[1] - std::f64::consts::FRAC_PI_2).abs() < 1e-9, "yaw {angles:?}");
        assert!(angles[2].abs() < 1e-6, "no roll on a level track: {angles:?}");
        // Along z: no turn at all.
        let along_z = model_angles([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
        assert!(along_z[1].abs() < 1e-9 && along_z[0].abs() < 1e-9);
    }

    #[test]
    fn a_climb_pitches_the_model() {
        let angles = model_angles([1.0, 0.0, 0.0], normalize([0.0, 1.0, 1.0]));
        assert!(angles[0].abs() > 0.5, "a slope pitches: {angles:?}");
        assert!(angles[2].abs() < 1e-3, "and does not roll: {angles:?}");
    }

    #[test]
    fn one_piece_is_baked_for_every_sample_after_the_first() {
        let bc = straight_curve(8);
        let pieces = baked_segments(&bc);
        assert_eq!(pieces.len(), bc.segment_count());
        assert_eq!(pieces[0].index, 1);
        assert_eq!(pieces.last().unwrap().index, bc.segment_count());
        // The ties lie along the run, in the first block entity's frame.
        for window in pieces.windows(2) {
            assert!(window[1].tie.at[0] > window[0].tie.at[0]);
        }
        // The last rail piece is longer (2.2 against 2.1 times its length).
        let last = pieces.last().unwrap().rails[0].scale_z;
        let before = pieces[pieces.len() - 2].rails[0].scale_z;
        assert!(last > before);
    }

    #[test]
    fn the_rails_sit_a_little_less_than_a_block_apart() {
        let bc = straight_curve(8);
        let piece = baked_segments(&bc)[2];
        let gap = (piece.rails[0].at[2] - piece.rails[1].at[2]).abs();
        assert!((gap - 2.0 * 0.965).abs() < 1e-5, "{gap}");
    }
    #[test]
    fn a_curve_without_a_girder_has_none() {
        assert!(baked_girders(&straight_curve(8)).is_empty());
    }

    #[test]
    fn a_girder_curve_has_a_pair_of_beams_for_each_piece_below_the_rails() {
        let mut bc = straight_curve(8);
        bc.has_girder = true;
        let pieces = baked_girders(&bc);
        assert_eq!(pieces.len(), bc.segment_count());
        let segments = baked_segments(&bc);
        for (girder, rails) in pieces.iter().zip(&segments) {
            assert_eq!(girder.index, rails.index);
            // The beams hang under the track (half a block and 10/16 below the rails' ties) at the rails' distance.
            assert!(girder.beams[0].at[1] < rails.rails[0].at[1] - 0.4, "{:?} {:?}", girder.beams[0].at, rails.rails[0].at);
            let gap = (girder.beams[0].at[2] - girder.beams[1].at[2]).abs();
            assert!((gap - 2.0).abs() < 1e-5, "{gap}");
            // The top cap is above the bottom one by the second offset (10/16).
            let rise = girder.caps[0][0].at[1] - girder.caps[1][0].at[1];
            assert!((rise - 10.0 / 16.0).abs() < 1e-5, "{rise}");
        }
        // The last piece is longer in its scale (2.3 against 2.2 times its length).
        let last = pieces.last().unwrap().beams[0].scale_z;
        let before = pieces[pieces.len() - 2].beams[0].scale_z;
        assert!(last > before);
        // Neighbouring pieces alternate by 1/1024 so their faces don't fight (the shift is in `then`).
        let (a, b) = (pieces[1].beams[0].then[1], pieces[2].beams[0].then[1]);
        assert!((a - b).abs() > 1e-4, "{a} {b}");
    }
}
