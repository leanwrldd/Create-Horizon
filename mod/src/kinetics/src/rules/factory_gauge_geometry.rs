//! Where a Factory Gauge's panels are on the block and how the arrows between them run (`FactoryPanelBlock.getTargetedSlot`,
//! `getXRot`, `getYRot`, `FactoryPanelSlotPositioning.getCenterOfSlot`, `FactoryPanelConnection.getPath` and `calculatePathDiff`,
//! `FactoryPanelRenderer.renderPath`). All in block units; no game types. The collision test of the path search is a closure.
//!
//! Known differences from Create, for the README:
//! - `getPath`'s cache (the path kept while the bend mode is the one it was found with) is the caller's.
//! - `noCollision(AABB(v, v).inflate(1/128))` is the caller's `blocked`: a point test against the block collision boxes.

use super::factory_gauge::{PanelPosition, Slot};
use super::stock_link::AttachFace;
use super::value_box::{Vec3, horizontal_angle, rotate, rotate_centered};
use crate::{Axis, Dir};

/// `FactoryPanelBlock.getXRot` in degrees.
pub fn x_rot(face: AttachFace) -> f64 {
    match face {
        AttachFace::Ceiling => 90.0,
        AttachFace::Floor => -90.0,
        AttachFace::Wall => 0.0,
    }
}

/// `FactoryPanelBlock.getYRot` in degrees.
pub fn y_rot(face: AttachFace, facing: Dir) -> f64 {
    (if face == AttachFace::Ceiling { 180.0 } else { 0.0 }) + horizontal_angle(facing)
}

/// The panel's spot on the face, `lift` above the surface, in the block's own coordinates (`getCenterOfSlot` for `lift` 1.5/16;
/// `getTargetedSlot` uses 0).
pub fn slot_point(face: AttachFace, facing: Dir, slot: Slot, lift: f64) -> Vec3 {
    let vec = [0.25 + f64::from(slot.x_offset()) * 0.5, lift, 0.25 + f64::from(slot.y_offset()) * 0.5];
    let vec = rotate_centered(vec, 180.0, Axis::Y);
    let vec = rotate_centered(vec, x_rot(face) + 90.0, Axis::X);
    rotate_centered(vec, y_rot(face, facing), Axis::Y)
}

/// `getCenterOfSlot`.
pub fn center_of_slot(face: AttachFace, facing: Dir, slot: Slot) -> Vec3 {
    slot_point(face, facing, slot, 1.5 / 16.0)
}

/// `getTargetedSlot`: the panel nearest to a click (`local_click` in the block's coordinates). A tie goes to the later slot.
pub fn targeted_slot(face: AttachFace, facing: Dir, local_click: Vec3) -> Slot {
    let mut best = f64::MAX;
    let mut best_slot = Slot::BottomLeft;
    for slot in Slot::ALL {
        let at = slot_point(face, facing, slot, 0.0);
        let diff: f64 = (0..3).map(|i| (at[i] - local_click[i]).powi(2)).sum();
        if diff > best {
            continue;
        }
        best = diff;
        best_slot = slot;
    }
    best_slot
}

/// `FactoryPanelConnection.calculatePathDiff`: from the connection's source panel `from` to the panel `to` it feeds, in the
/// face's own frame (y dropped).
pub fn path_diff(face: AttachFace, facing: Dir, from: PanelPosition, to: PanelPosition) -> Vec3 {
    let slot_x = f64::from(to.slot.x_offset() - from.slot.x_offset());
    let slot_y = f64::from(to.slot.y_offset() - from.slot.y_offset());
    let mut diff = [
        f64::from(to.pos[0] - from.pos[0]),
        f64::from(to.pos[1] - from.pos[1]),
        f64::from(to.pos[2] - from.pos[2]),
    ];
    diff = rotate(diff, -y_rot(face, facing), Axis::Y);
    diff = rotate(diff, -x_rot(face) - 90.0, Axis::X);
    diff = rotate(diff, -180.0, Axis::Y);
    [diff[0] + slot_x * 0.5, 0.0, diff[2] + slot_y * 0.5]
}

fn manhattan(a: (i32, i32)) -> i32 {
    a.0.abs() + a.1.abs()
}

fn step(at: (i32, i32), d: Dir) -> (i32, i32) {
    let n = d.normal();
    (at.0 + n[0], at.1 + n[2])
}

/// `Mth.positiveCeilDiv(x, 2)`.
fn ceil_half(x: i32) -> i32 {
    -(-x).div_euclid(2)
}

/// `FactoryPanelConnection.getPath`: the half-block steps (horizontal directions in the face's frame) from the target panel towards
/// the source. `bend_mode` -1 searches the modes 0 to 4 for one whose path doesn't pass through anything (`blocked(point)` is told
/// the world point of each corner, `start` being the target's panel centre in the world).
pub fn connection_path(diff: Vec3, bend_mode: i32, start: Vec3, face: AttachFace, facing: Dir, blocked: &dyn Fn(Vec3) -> bool) -> Vec<Dir> {
    let find_suitable = bend_mode == -1;
    let mut path: Vec<Dir> = Vec::new();
    for mode in 0..=4 {
        path.clear();
        if !find_suitable && mode != bend_mode {
            continue;
        }
        let desperate = mode == 4;
        let mut first = (0, 0);
        let mut last = ((diff[0] * 2.0 + 0.1).floor() as i32, (diff[2] * 2.0 + 0.1).floor() as i32);
        if mode > 1 {
            let flip_x = (diff[0] > 0.0) ^ (mode % 2 == 1);
            let flip_z = (diff[2] > 0.0) ^ (mode % 2 == 0);
            let (ceil_x, ceil_z) = (ceil_half(last.0), ceil_half(last.1));
            let (floor_x, floor_z) = (last.0.div_euclid(2), last.1.div_euclid(2));
            first = (if flip_x { floor_x } else { ceil_x }, if flip_z { floor_z } else { ceil_z });
            last = (if !flip_x { floor_x } else { ceil_x }, if !flip_z { floor_z } else { ceil_z });
        }
        let mut last_direction: Option<Dir> = None;
        let mut current: Option<Dir> = None;
        for mut travel in [first, last] {
            let z_is_farther = travel.1.abs() > travel.0.abs();
            let z_preferred = if desperate { z_is_farther } else { mode % 2 == 1 };
            let order = if z_preferred {
                [Dir::South, Dir::North, Dir::West, Dir::East]
            } else {
                [Dir::West, Dir::East, Dir::South, Dir::North]
            };
            for _ in 0..100 {
                if travel == (0, 0) {
                    break;
                }
                for d in order {
                    if last_direction.is_some_and(|l| d == l.opposite()) {
                        continue;
                    }
                    if current.is_none_or(|c| manhattan(step(travel, d)) < manhattan(step(travel, c))) {
                        current = Some(d);
                    }
                }
                let Some(c) = current else {
                    break;
                };
                last_direction = Some(c);
                travel = step(travel, c);
                path.push(c);
            }
        }
        if find_suitable && !desperate {
            let mut travelled = (0, 0);
            let mut clear = true;
            for d in path.iter().take(path.len().saturating_sub(1)) {
                travelled = step(travelled, *d);
                let offset = [f64::from(travelled.0) * 0.5, 0.0, f64::from(travelled.1) * 0.5];
                let offset = rotate(offset, 180.0, Axis::Y);
                let offset = rotate(offset, x_rot(face) + 90.0, Axis::X);
                let offset = rotate(offset, y_rot(face, facing), Axis::Y);
                if blocked([start[0] + offset[0], start[1] + offset[1], start[2] + offset[2]]) {
                    clear = false;
                    break;
                }
            }
            if !clear {
                continue;
            }
        }
        break;
    }
    path
}

/// What `renderPath` shows of a panel's arrows besides the path: the colour (`getIngredientStatusColor`, RGB) and the height of the
/// strip above the surface in 1/512 blocks (`yOffset`, before the per-direction 1/4 pixel).
pub fn arrow_color(count: i32, missing_address: bool, redstone_powered: bool, waiting: bool, satisfied: bool, promised_satisfied: bool) -> u32 {
    if count == 0 || missing_address || redstone_powered {
        0x88_8898
    } else if waiting {
        0x5B_3B3B
    } else if satisfied {
        0x9E_FF7F
    } else if promised_satisfied {
        0x22_AFAF
    } else {
        0x3D_6EBD
    }
}

/// `renderPath`'s `yOffset` for an ingredient arrow.
pub fn arrow_height(satisfied: bool, promised_satisfied: bool) -> f64 {
    1.0 + if promised_satisfied { 1.0 } else if satisfied { 0.0 } else { 2.0 }
}

/// The pose of one path segment (`renderPath`'s translations, before the three centred turns): the segment's offset on the panel's
/// own plane, for step `i` of `path`, and the direction its model is named after (`direction.getOpposite()`).
pub fn segment(target: Slot, path: &[Dir], i: usize, height: f64) -> (Vec3, Dir) {
    let (mut x, mut z) = (0.0, 0.0);
    for d in &path[..=i] {
        let n = d.normal();
        x += f64::from(n[0]) * 0.5;
        z += f64::from(n[2]) * 0.5;
    }
    let d = path[i];
    let two_d = match d {
        Dir::South => 0,
        Dir::West => 1,
        Dir::North => 2,
        _ => 3,
    };
    (
        [f64::from(target.x_offset()) * 0.5 + 0.25 + x, (height + f64::from(two_d % 2) * 0.125) / 512.0, f64::from(target.y_offset()) * 0.5 + 0.25 + z],
        d.opposite(),
    )
}

/// Why `FactoryPanelBehaviour.moveTo` (with `FactoryPanelConnectionHandler.clientTick`'s checks) leaves a panel where it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveRefusal {
    /// `FactoryPanelBehaviour.at(level, newPos) != null`.
    Occupied,
    /// The block there is neither air nor a gauge in the same state (this port only moves onto a gauge block).
    NotAGauge,
    /// A gauge block with another face or facing.
    OtherState,
    /// A connection would be 24 or more blocks long (`closerThan(newPos.pos(), 24)`).
    TooFar,
    /// Not on the same plane (`facing.getAxis().choose(diff) != 0`).
    OffSurface,
    /// A Packager behind the new spot.
    OnPackager,
}

/// `moveTo`'s checks. `connected` are the blocks of every panel and link the panel is connected to.
pub fn check_move(
    target_active: bool,
    target_is_gauge: bool,
    same_state: bool,
    connected: &[[i32; 3]],
    new_pos: [i32; 3],
    diff_along_face_axis: i32,
    packager_behind: bool,
) -> Option<MoveRefusal> {
    if target_active {
        return Some(MoveRefusal::Occupied);
    }
    if !target_is_gauge {
        return Some(MoveRefusal::NotAGauge);
    }
    if !same_state {
        return Some(MoveRefusal::OtherState);
    }
    if diff_along_face_axis != 0 {
        return Some(MoveRefusal::OffSurface);
    }
    if packager_behind {
        return Some(MoveRefusal::OnPackager);
    }
    let far = |p: &[i32; 3]| (0..3).map(|i| i64::from(p[i] - new_pos[i]).pow(2)).sum::<i64>() >= 24 * 24;
    if connected.iter().any(far) {
        return Some(MoveRefusal::TooFar);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_move_is_checked() {
        let ok = |active, gauge, same, conn: &[[i32; 3]], along, pack| check_move(active, gauge, same, conn, [0, 0, 0], along, pack);
        assert_eq!(ok(false, true, true, &[[3, 0, 0]], 0, false), None);
        assert_eq!(ok(true, true, true, &[], 0, false), Some(MoveRefusal::Occupied));
        assert_eq!(ok(false, false, true, &[], 0, false), Some(MoveRefusal::NotAGauge));
        assert_eq!(ok(false, true, false, &[], 0, false), Some(MoveRefusal::OtherState));
        assert_eq!(ok(false, true, true, &[], 1, false), Some(MoveRefusal::OffSurface));
        assert_eq!(ok(false, true, true, &[], 0, true), Some(MoveRefusal::OnPackager));
        assert_eq!(ok(false, true, true, &[[24, 0, 0]], 0, false), Some(MoveRefusal::TooFar));
        assert_eq!(ok(false, true, true, &[[23, 0, 0]], 0, false), None);
    }

    fn near(a: Vec3, b: Vec3) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-9)
    }

    #[test]
    fn a_wall_gauge_has_its_four_panels_on_the_face() {
        // A gauge on the north face of its block's south neighbour... facing south: the face is the z = 0 plane or the z = 1 plane;
        // either way every slot centre is on one plane and the four are distinct, a half block apart.
        let points: Vec<Vec3> = Slot::ALL.iter().map(|s| slot_point(AttachFace::Wall, Dir::South, *s, 0.0)).collect();
        let z = points[0][2];
        assert!(points.iter().all(|p| (p[2] - z).abs() < 1e-9));
        assert!(z.abs() < 1e-9 || (z - 1.0).abs() < 1e-9, "z = {z}");
        for (i, a) in points.iter().enumerate() {
            for b in &points[i + 1..] {
                assert!(!near(*a, *b));
            }
        }
    }

    #[test]
    fn a_click_picks_the_nearest_panel() {
        for face in [AttachFace::Wall, AttachFace::Floor, AttachFace::Ceiling] {
            for facing in [Dir::North, Dir::East, Dir::South, Dir::West] {
                for slot in Slot::ALL {
                    let at = slot_point(face, facing, slot, 0.0);
                    assert_eq!(targeted_slot(face, facing, at), slot, "{face:?} {facing:?} {slot:?}");
                    // A click a little to the side of the centre still means that panel.
                    let c = center_of_slot(face, facing, slot);
                    assert_eq!(targeted_slot(face, facing, c), slot);
                }
            }
        }
    }

    #[test]
    fn the_rotations_are_the_blocks() {
        assert_eq!(x_rot(AttachFace::Floor), -90.0);
        assert_eq!(x_rot(AttachFace::Ceiling), 90.0);
        assert_eq!(y_rot(AttachFace::Ceiling, Dir::East), 270.0);
        assert_eq!(y_rot(AttachFace::Wall, Dir::West), 270.0);
    }

    #[test]
    fn a_path_between_two_panels_walks_the_difference() {
        let from = PanelPosition { pos: [0, 64, 0], slot: Slot::BottomLeft };
        let to = PanelPosition { pos: [2, 64, 0], slot: Slot::BottomLeft };
        let diff = path_diff(AttachFace::Wall, Dir::South, from, to);
        assert!(near([diff[0].abs(), diff[1], diff[2]], [2.0, 0.0, 0.0]), "{diff:?}");
        let path = connection_path(diff, -1, [0.0; 3], AttachFace::Wall, Dir::South, &|_| false);
        // Two blocks apart is four half-block steps, none of them backwards, all along the one axis.
        assert_eq!(path.len(), 4);
        assert!(path.iter().all(|d| *d == path[0]));
        assert!(matches!(path[0], Dir::East | Dir::West));
    }

    #[test]
    fn a_blocked_straight_path_bends() {
        let from = PanelPosition { pos: [0, 64, 0], slot: Slot::BottomLeft };
        let to = PanelPosition { pos: [2, 66, 0], slot: Slot::BottomLeft };
        let diff = path_diff(AttachFace::Wall, Dir::South, from, to);
        let free = connection_path(diff, -1, [0.0; 3], AttachFace::Wall, Dir::South, &|_| false);
        assert_eq!(free.len(), 8);
        // Everything blocked: the search settles on the last mode, still a full path.
        let stuck = connection_path(diff, -1, [0.0; 3], AttachFace::Wall, Dir::South, &|_| true);
        assert_eq!(stuck.len(), 8);
        // A set bend mode is used as it is.
        for mode in 0..4 {
            let path = connection_path(diff, mode, [0.0; 3], AttachFace::Wall, Dir::South, &|_| true);
            assert_eq!(path.len(), 8, "mode {mode}");
        }
    }

    #[test]
    fn panels_on_the_same_block_are_a_half_block_apart() {
        let from = PanelPosition { pos: [0, 64, 0], slot: Slot::BottomLeft };
        let to = PanelPosition { pos: [0, 64, 0], slot: Slot::BottomRight };
        let diff = path_diff(AttachFace::Wall, Dir::South, from, to);
        assert!(near([diff[0].abs(), diff[1], diff[2]], [0.5, 0.0, 0.0]), "{diff:?}");
        let path = connection_path(diff, -1, [0.0; 3], AttachFace::Wall, Dir::South, &|_| false);
        assert_eq!(path.len(), 1);
    }

    #[test]
    fn arrow_colours_follow_the_state() {
        assert_eq!(arrow_color(0, false, false, false, true, true), 0x88_8898);
        assert_eq!(arrow_color(2, true, false, false, true, true), 0x88_8898);
        assert_eq!(arrow_color(2, false, false, true, true, true), 0x5B_3B3B);
        assert_eq!(arrow_color(2, false, false, false, true, false), 0x9E_FF7F);
        assert_eq!(arrow_color(2, false, false, false, false, true), 0x22_AFAF);
        assert_eq!(arrow_color(2, false, false, false, false, false), 0x3D_6EBD);
        assert_eq!(arrow_height(true, false), 1.0);
        assert_eq!(arrow_height(false, true), 2.0);
        assert_eq!(arrow_height(false, false), 3.0);
    }

    #[test]
    fn segments_advance_half_a_block_each() {
        let path = [Dir::East, Dir::East, Dir::South];
        let (a, model) = segment(Slot::TopRight, &path, 0, 0.0);
        assert_eq!(model, Dir::West);
        let (b, _) = segment(Slot::TopRight, &path, 1, 0.0);
        assert!((b[0] - a[0] - 0.5).abs() < 1e-9);
        let (c, _) = segment(Slot::TopRight, &path, 2, 0.0);
        assert!((c[2] - b[2] - 0.5).abs() < 1e-9);
    }
}
