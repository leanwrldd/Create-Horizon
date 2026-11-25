//! `TrackBlockOutline.drawCustomBlockSelection`, `walkShapes` and `renderShape` (content/trains/track/TrackBlockOutline.java,
//! TrackVoxelShapes.java, `AllShapes.TRACK_*`): the selection box a straight track block shows under the cursor, as lines.
//!
//! A track block's outline is not its one box: it is the long, low shapes of the directions its rails run in, turned to them
//! (`walkShapes`). Each shape is a union (and for the diagonal, a difference) of boxes; [`shape_edges`] gives the edges of such
//! a union as `VoxelShape.forAllEdges` does (an edge is drawn where one cell, three cells, or two diagonal cells of the four
//! around it are filled), merged into runs.
//!
//! Left out: the curve's own selection, which is `track_overlay::curve_outline`.

use super::track_overlay::Affine;
use super::track_shape::{Facing, TrackShape};
use super::vec::{Vec3, add};

/// `renderShape`'s colour for a hovered track: black at `.4` alpha.
pub const COLOUR_PLAIN: u32 = 0x6600_0000;
/// While holding track and a curve could start here: white at `.4`.
pub const COLOUR_CAN_CONNECT: u32 = 0x66FF_FFFF;
/// While holding track and it can't: `(1, .25, .125)` at `.4`.
pub const COLOUR_CANNOT: u32 = 0x66FF_4020;

/// `drawCustomBlockSelection`'s colour: nothing held (`valid` null) is plain, track held is white where a curve can start from the
/// block and reddish where it can't.
pub fn colour(holding_track: bool, can_connect_from: bool) -> u32 {
    match (holding_track, can_connect_from) {
        (false, _) => COLOUR_PLAIN,
        (true, true) => COLOUR_CAN_CONNECT,
        (true, false) => COLOUR_CANNOT,
    }
}

/// `canConnectFrom`: not a junction, and not tilted (`TrackBlockEntity.isTilted`).
pub fn can_connect_from(shape: TrackShape, tilted: bool) -> bool {
    !shape.is_junction() && !tilted
}

/// A voxel shape as the boxes it is built of, in blocks: `true` adds a box (`Shapes.or`), `false` takes it away
/// (`Shapes.join(.., ONLY_FIRST)`), in order.
#[derive(Clone, Debug, PartialEq)]
pub struct Shape(pub Vec<(bool, [Vec3; 2])>);

/// `Block.box(x1, y1, z1, x2, y2, z2)`, in sixteenths.
fn px(x1: f64, y1: f64, z1: f64, x2: f64, y2: f64, z2: f64) -> [Vec3; 2] {
    [[x1 / 16.0, y1 / 16.0, z1 / 16.0], [x2 / 16.0, y2 / 16.0, z2 / 16.0]]
}

fn boxes(list: &[[Vec3; 2]]) -> Shape {
    Shape(list.iter().map(|b| (true, *b)).collect())
}

/// `AllShapes.TRACK_ORTHO.get(SOUTH)`: `TrackVoxelShapes.orthogonal()`, long along x.
pub fn ortho_south() -> Shape {
    boxes(&[px(-14.0, 0.0, 0.0, 30.0, 4.0, 16.0)])
}

/// `AllShapes.TRACK_ORTHO.get(EAST)`: the same turned a quarter, long along z.
pub fn ortho_east() -> Shape {
    boxes(&[px(0.0, 0.0, -14.0, 16.0, 4.0, 30.0)])
}

/// `AllShapes.TRACK_CROSS`: the two orthogonal shapes together.
pub fn cross() -> Shape {
    boxes(&[px(-14.0, 0.0, 0.0, 30.0, 4.0, 16.0), px(0.0, 0.0, -14.0, 16.0, 4.0, 30.0)])
}

/// `TrackVoxelShapes.longOrthogonalZ()`.
pub fn long_ortho() -> Shape {
    boxes(&[px(-14.0, 0.0, -3.3, 30.0, 4.0, 19.3)])
}

/// `TrackVoxelShapes.longOrthogonalZOffset()`.
pub fn long_ortho_offset() -> Shape {
    boxes(&[px(-14.0, 0.0, 0.0, 30.0, 4.0, 24.0)])
}

/// `LONG_CROSS`: `longOrthogonalZ` and `longOrthogonalX` together.
pub fn long_cross() -> Shape {
    boxes(&[px(-14.0, 0.0, -3.3, 30.0, 4.0, 19.3), px(-3.3, 0.0, -14.0, 19.3, 4.0, 30.0)])
}

const EPS: f64 = 1.0e-9;

fn axis_cuts(shape: &Shape, axis: usize) -> Vec<f64> {
    let mut cuts: Vec<f64> = shape.0.iter().flat_map(|(_, b)| [b[0][axis], b[1][axis]]).collect();
    cuts.sort_by(f64::total_cmp);
    cuts.dedup_by(|a, b| (*a - *b).abs() < EPS);
    cuts
}

fn contains(shape: &Shape, p: Vec3) -> bool {
    let mut filled = false;
    for (add, b) in &shape.0 {
        if (0..3).all(|i| p[i] > b[0][i] && p[i] < b[1][i]) {
            filled = *add;
        }
    }
    filled
}

/// `VoxelShape.forAllEdges`: the edges of the union, a straight run of them as one line.
pub fn shape_edges(shape: &Shape) -> Vec<[Vec3; 2]> {
    let cuts = [axis_cuts(shape, 0), axis_cuts(shape, 1), axis_cuts(shape, 2)];
    if cuts.iter().any(|c| c.len() < 2) {
        return Vec::new();
    }
    let n = [cuts[0].len() - 1, cuts[1].len() - 1, cuts[2].len() - 1];
    let mut cells = vec![false; n[0] * n[1] * n[2]];
    let index = |i: usize, j: usize, k: usize| (i * n[1] + j) * n[2] + k;
    for i in 0..n[0] {
        for j in 0..n[1] {
            for k in 0..n[2] {
                let centre = [
                    (cuts[0][i] + cuts[0][i + 1]) / 2.0,
                    (cuts[1][j] + cuts[1][j + 1]) / 2.0,
                    (cuts[2][k] + cuts[2][k + 1]) / 2.0,
                ];
                cells[index(i, j, k)] = contains(shape, centre);
            }
        }
    }
    // A cell by signed index; outside the grid is empty.
    let cell = |c: [i64; 3]| {
        (0..3).all(|a| c[a] >= 0 && (c[a] as usize) < n[a]) && cells[index(c[0] as usize, c[1] as usize, c[2] as usize)]
    };
    let mut lines = Vec::new();
    for axis in 0..3 {
        let (b, c) = ((axis + 1) % 3, (axis + 2) % 3);
        // For each grid line along `axis` (a point of the other two axes) the runs of its steps that are edges.
        for lb in 0..cuts[b].len() {
            for lc in 0..cuts[c].len() {
                let mut run: Option<(usize, usize)> = None;
                for step in 0..=n[axis] {
                    let is_edge = step < n[axis] && {
                        let at = |db: i64, dc: i64| {
                            let mut idx = [0_i64; 3];
                            idx[axis] = step as i64;
                            idx[b] = lb as i64 + db;
                            idx[c] = lc as i64 + dc;
                            cell(idx)
                        };
                        let (a, bb, cc, d) = (at(-1, -1), at(-1, 0), at(0, -1), at(0, 0));
                        let count = [a, bb, cc, d].iter().filter(|f| **f).count();
                        count == 1 || count == 3 || (count == 2 && ((a && d) || (bb && cc)))
                    };
                    match (&mut run, is_edge) {
                        (Some((_, end)), true) => *end = step + 1,
                        (None, true) => run = Some((step, step + 1)),
                        (Some(_), false) => {
                            let (from, to) = run.take().expect("a run");
                            let mut p = [0.0; 3];
                            p[b] = cuts[b][lb];
                            p[c] = cuts[c][lc];
                            let (mut start, mut end) = (p, p);
                            start[axis] = cuts[axis][from];
                            end[axis] = cuts[axis][to];
                            lines.push([start, end]);
                        }
                        (None, false) => {}
                    }
                }
            }
        }
    }
    lines
}

/// `AngleHelper.horizontalAngle` in degrees (an x-axis direction has its `toYRot` negated): south 0, west -90, north 180, east -270.
fn horizontal_angle(d: Facing) -> f64 {
    match d {
        Facing::South => 0.0,
        Facing::West => -90.0,
        Facing::North => 180.0,
        Facing::East => -270.0,
    }
}

/// `walkShapes`: each shape the block shows with the pose it is drawn in (a pose acts in the frame the earlier ones made, as
/// the `TransformStack` does, and the poses carry over from one shape to the next).
pub fn walk_shapes(shape: TrackShape) -> Vec<(Shape, Affine)> {
    use TrackShape::*;
    let angle45 = std::f64::consts::FRAC_PI_4;
    let mut msr = Affine::IDENTITY;
    let mut out = Vec::new();
    if matches!(shape, Xo | CrNdx | CrPdx) {
        out.push((ortho_east(), msr));
    } else if matches!(shape, Zo | CrNdz | CrPdz) {
        out.push((ortho_south(), msr));
    }
    if shape.is_portal() {
        for d in [Facing::North, Facing::South, Facing::East, Facing::West] {
            if TrackShape::as_portal(d) != shape {
                continue;
            }
            msr = msr.rotate_centered_y(horizontal_angle(d).to_radians());
            out.push((long_ortho_offset(), msr));
            return out;
        }
    }
    if matches!(shape, Pd | CrPdx | CrPdz) {
        msr = msr.rotate_centered_y(angle45);
        out.push((long_ortho(), msr));
    } else if matches!(shape, Nd | CrNdx | CrNdz) {
        msr = msr.rotate_centered_y(-angle45);
        out.push((long_ortho(), msr));
    }
    if shape == CrO {
        out.push((cross(), msr));
    } else if shape == CrD {
        msr = msr.rotate_centered_y(angle45);
        out.push((long_cross(), msr));
    }
    if !matches!(shape, Ae | An | Aw | As) {
        return out;
    }
    msr = msr
        .translate([0.0, 1.0, 0.0])
        .rotate_centered_y(std::f64::consts::PI - f64::from(shape.model_rotation()).to_radians())
        .rotate_x(angle45)
        .translate([0.0, -3.0 / 16.0, 1.0 / 16.0]);
    out.push((long_ortho(), msr));
    out
}

/// The lines of a track block's selection, in world space for the block at `pos`: every edge of every shape `walkShapes` gives.
pub fn selection_lines(shape: TrackShape, pos: [i32; 3]) -> Vec<[Vec3; 2]> {
    let origin = pos.map(f64::from);
    walk_shapes(shape)
        .into_iter()
        .flat_map(|(s, pose)| {
            shape_edges(&s).into_iter().map(move |[a, b]| [add(pose.apply(a), origin), add(pose.apply(b), origin)])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-9)
    }

    #[test]
    fn a_box_has_twelve_edges() {
        let lines = shape_edges(&ortho_south());
        assert_eq!(lines.len(), 12);
        let len: f64 = lines.iter().map(|[a, b]| (0..3).map(|i| (b[i] - a[i]).abs()).sum::<f64>()).sum();
        // Four edges of 44/16, four of 16/16 and four of 4/16.
        assert!((len - (4.0 * 44.0 + 4.0 * 16.0 + 4.0 * 4.0) / 16.0).abs() < 1e-9, "{len}");
    }

    #[test]
    fn a_cross_of_two_boxes_has_the_outline_of_the_union() {
        // Twelve corners on the top, twelve on the bottom, twelve verticals.
        assert_eq!(shape_edges(&cross()).len(), 36);
    }

    #[test]
    fn a_taken_out_box_leaves_a_notch() {
        let mut shape = ortho_south();
        shape.0.push((false, px(0.0, -1.0, -1.0, 16.0, 5.0, 17.0)));
        // Two separate pieces, each a box.
        assert_eq!(shape_edges(&shape).len(), 24);
        // Nothing is left of a shape taken away entirely.
        shape.0.push((false, px(-20.0, -1.0, -1.0, 40.0, 5.0, 17.0)));
        assert!(shape_edges(&shape).is_empty());
    }

    #[test]
    fn two_boxes_that_touch_by_an_edge_only_keep_the_edge() {
        let shape = boxes(&[px(0.0, 0.0, 0.0, 8.0, 8.0, 8.0), px(8.0, 8.0, 0.0, 16.0, 16.0, 8.0)]);
        // 24 edges; the one they share (two diagonal cells) is drawn once, and the four pairs that continue each other in a line
        // (two along x, two up) join: 24 - 1 - 4.
        assert_eq!(shape_edges(&shape).len(), 19);
    }

    #[test]
    fn the_colour_follows_what_is_held() {
        assert_eq!(colour(false, true), COLOUR_PLAIN);
        assert_eq!(colour(false, false), COLOUR_PLAIN);
        assert_eq!(colour(true, true), COLOUR_CAN_CONNECT);
        assert_eq!(colour(true, false), COLOUR_CANNOT);
        assert!(can_connect_from(TrackShape::Xo, false));
        assert!(!can_connect_from(TrackShape::Xo, true));
        assert!(!can_connect_from(TrackShape::CrO, false));
    }

    #[test]
    fn a_straight_track_shows_its_orthogonal_box() {
        // `XO` draws `TRACK_ORTHO` east (long along z), `ZO` the south one (long along x).
        let xo = selection_lines(TrackShape::Xo, [10, 64, 20]);
        assert_eq!(xo.len(), 12);
        assert!(xo.iter().any(|[a, b]| close(*a, [10.0, 64.0, 20.0 - 14.0 / 16.0]) || close(*b, [10.0, 64.0, 20.0 - 14.0 / 16.0])));
        let zo = selection_lines(TrackShape::Zo, [0, 0, 0]);
        let xs: Vec<f64> = zo.iter().flat_map(|[a, b]| [a[0], b[0]]).collect();
        assert!(xs.iter().cloned().fold(f64::MAX, f64::min) < -0.8 && xs.iter().cloned().fold(f64::MIN, f64::max) > 1.8);
    }

    #[test]
    fn a_diagonal_is_the_long_box_turned_an_eighth_about_the_centre() {
        let lines = selection_lines(TrackShape::Pd, [0, 0, 0]);
        assert_eq!(lines.len(), 12);
        // The centre of the block stays inside the box's footprint: the bottom edges are symmetrical about it.
        let (mut sum, mut n) = ([0.0; 3], 0.0);
        for [a, b] in &lines {
            for p in [a, b] {
                for i in 0..3 {
                    sum[i] += p[i];
                }
                n += 1.0;
            }
        }
        assert!((sum[0] / n - 0.5).abs() < 1e-9 && (sum[2] / n - 0.5).abs() < 1e-9, "{sum:?}");
    }

    #[test]
    fn a_junction_shows_every_shape_it_is_made_of() {
        assert_eq!(selection_lines(TrackShape::CrO, [0, 0, 0]).len(), 36);
        // An orthogonal and a diagonal.
        assert_eq!(walk_shapes(TrackShape::CrPdx).len(), 2);
        assert_eq!(walk_shapes(TrackShape::CrD).len(), 1);
        assert_eq!(walk_shapes(TrackShape::None).len(), 0);
    }

    #[test]
    fn a_slope_is_raised_and_pitched() {
        let lines = selection_lines(TrackShape::As, [0, 0, 0]);
        assert_eq!(lines.len(), 12);
        let top = lines.iter().flat_map(|[a, b]| [a[1], b[1]]).fold(f64::MIN, f64::max);
        let bottom = lines.iter().flat_map(|[a, b]| [a[1], b[1]]).fold(f64::MAX, f64::min);
        assert!(top > 1.0 && bottom < 0.5, "{bottom} {top}");
    }

    #[test]
    fn a_portal_track_shows_the_long_offset_box() {
        let lines = selection_lines(TrackShape::Tn, [0, 0, 0]);
        assert_eq!(lines.len(), 12);
        assert_eq!(walk_shapes(TrackShape::Te).len(), 1);
    }
}
