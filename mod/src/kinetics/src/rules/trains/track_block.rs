//! The block side of the track: `TrackBlock` (content/trains/track/TrackBlock.java), `ITrackBlock`'s defaults,
//! `TrackBlockItem` (the click that picks a curve's start) and `TrackPlacement.placeTracks` (what a valid curve sets
//! down), all as decisions over plain values. The glue reads the world, asks these, and applies what they say.
//!
//! Ported: `getStateForPlacement`'s shape choice and its slope fix ([`shape_for_look`], [`slope_fix`]),
//! `getNearestTrackAxis`, `getCurveStart`, `TrackBlockItem.select` ([`select`]), the clicked end of `tryConnect`
//! ([`target_end`]), `overlay` (a crossing is made when two shapes meet, [`overlay`]), `getConnected` for a track with
//! its curves ([`track_ends`]), `placeTracks` ([`plan_tracks`]), `removeConnection`'s `HAS_BE` rule and
//! `BezierConnection.spawnItems`' drop points ([`drop_points`]), `getRequiredItems` for the curve cost.
//!
//! Known differences: the slope smoothing of `TrackBlockEntityTilt` is not ported, so `yOffsetAt` is 0 and a track is
//! never tilted; `getBlockPathType`, waterlogging and `AllPortalTracks` are left out; `TrackMaterial` is the
//! andesite one only.

use super::bezier::BezierConnection;
use super::graph::{DiscoveredLocation, TrackNodeLocation};
use super::placement::{ConnectingFrom, PlacementInfo};
use super::propagator::{StraightTrack, add_to_list_if_connected, connected_straight};
use super::track_shape::TrackShape;
use super::vec::{
    Vec3, add, block_containing, center_of, distance_to, dot, length, normalize, scale, sub,
};
use crate::Pos;

/// `AllBlocks.TRACK`.
pub const TRACK: &str = "create:track";
/// `TrackMaterial.ANDESITE`'s id.
pub const MATERIAL: &str = "create:andesite";
/// The data component of `AllDataComponents.TRACK_CONNECTING_FROM`.
pub const COMPONENT: &str = "create:track_connecting_from";

/// The block entity's key for the curves (`TrackBlockEntity.write`'s `Connections`).
pub const CONNECTIONS: &str = "Connections";

// --- the shape of a block being placed ------------------------------------------------------------

/// `TrackBlock.getStateForPlacement`, the first half: the shape whose axis lies nearest the player's horizontal look,
/// either way along it. `look` is `getLookAngle`, `yaw` is the player's `getYRot` in degrees (used when looking
/// straight up or down). Junction and portal shapes are skipped; the later shape wins a tie, as Java's `>` does.
pub fn shape_for_look(look: Vec3, yaw: f32) -> TrackShape {
    let mut flat = [look[0], 0.0, look[2]];
    if length(flat) < f64::from(1.0e-5_f32) {
        // `VecHelper.rotate(new Vec3(0, 0, 1), -yaw, Y)`.
        let angle = f64::from(-yaw).to_radians();
        flat = [(angle.sin()) * 1.0, 0.0, angle.cos()];
    }
    let flat = normalize(flat);
    let mut best = TrackShape::Zo;
    let mut best_value = f64::from(f32::MAX);
    for shape in TrackShape::ALL {
        if shape.is_junction() || shape.is_portal() {
            continue;
        }
        let axis = shape.axes()[0];
        let d1 = dist_sqr(axis, flat);
        let d2 = dist_sqr(scale(normalize(axis), -1.0), flat);
        let distance = d1.min(d2);
        if distance > best_value {
            continue;
        }
        best_value = distance;
        best = shape;
    }
    best
}

fn dist_sqr(a: Vec3, b: Vec3) -> f64 {
    let d = sub(a, b);
    dot(d, d)
}

/// `getStateForPlacement`, the second half: an orthogonal track next to a block with a sturdy top and open air above
/// it climbs toward it. `top_sturdy(pos)` is `isFaceSturdy(level, pos, UP)` of the block there, `above_sturdy(pos)`
/// is whether the block over it has a sturdy bottom (`isFaceSturdy(level, offset, DOWN)`).
pub fn slope_fix(
    mut best: TrackShape,
    clicked: Pos,
    top_sturdy: &dyn Fn(Pos) -> bool,
    above_sturdy: &dyn Fn(Pos) -> bool,
) -> TrackShape {
    let axis = best.axes()[0];
    if (dot(axis, axis) - 1.0).abs() > 1e-9 {
        return best;
    }
    for neg in [true, false] {
        let by = block_containing(scale(axis, if neg { -1.0 } else { 1.0 }));
        let offset = [clicked[0] + by[0], clicked[1] + by[1], clicked[2] + by[2]];
        let above = [offset[0], offset[1] + 1, offset[2]];
        if top_sturdy(offset) && !above_sturdy(above) {
            if best == TrackShape::Xo {
                best = if neg { TrackShape::Aw } else { TrackShape::Ae };
            }
            if best == TrackShape::Zo {
                best = if neg { TrackShape::An } else { TrackShape::As };
            }
        }
    }
    best
}

// --- ends of a block ------------------------------------------------------------------------------

/// `ITrackBlock.getNearestTrackAxis`: the axis (of the block's axes, either way) whose direction is closest to `look`,
/// and whether the look points against it (`lookVec.dot(best.multiply(1, 0, 1).normalize()) < 0`, Java's
/// `POSITIVE`).
pub fn nearest_track_axis(axes: &[Vec3], look: Vec3) -> (Vec3, bool) {
    let mut best: Option<Vec3> = None;
    let mut best_diff = f64::MAX;
    for axis in axes {
        for opposite in [1.0, -1.0] {
            let distance = distance_to(normalize(*axis), scale(look, opposite));
            if distance > best_diff {
                continue;
            }
            best_diff = distance;
            best = Some(*axis);
        }
    }
    let best = best.unwrap_or([0.0; 3]);
    let flat = normalize([best[0], 0.0, best[2]]);
    (best, dot(look, flat) < 0.0)
}

/// `TrackBlock.getCurveStart`: where a curve leaves the block along `axis`.
pub fn curve_start(pos: Pos, axis: Vec3) -> Vec3 {
    let vertical = axis[1] != 0.0;
    let centre = center_of(pos);
    add(add(centre, [0.0, if vertical { 0.0 } else { -0.5_f32 as f64 }, 0.0]), scale(axis, 0.5))
}

/// The clicked end as `tryConnect` and `TrackBlockItem.select` read it: the axis pointing away from the look (the
/// nearest one, turned when the look points against it), the track's up normal and the curve start on that axis.
pub fn target_end(pos: Pos, shape: TrackShape, look: Vec3) -> (Vec3, Vec3, Vec3) {
    let (axis, positive) = nearest_track_axis(&shape.axes(), look);
    let axis = scale(axis, if positive { -1.0 } else { 1.0 });
    let normal = normalize(shape.normal());
    (axis, normal, curve_start(pos, axis))
}

/// `TrackBlockItem.select`: what the stack stores when a track end is clicked first.
pub fn select(pos: Pos, shape: TrackShape, look: Vec3) -> ConnectingFrom {
    let (axis, normal, end) = target_end(pos, shape, look);
    ConnectingFrom { pos, axis, normal, end }
}

/// `TrackBlock.overlay`: two shapes put in one block make a crossing.
pub fn overlay(existing: TrackShape, placed: TrackShape) -> TrackShape {
    use TrackShape::{CrD, CrNdx, CrNdz, CrO, CrPdx, CrPdz, Nd, Pd, Xo, Zo};
    let mut combined = None;
    for flip in [true, false] {
        let (s1, s2) = if flip { (existing, placed) } else { (placed, existing) };
        if s1 == Xo && s2 == Zo {
            combined = Some(CrO);
        }
        if s1 == Pd && s2 == Nd {
            combined = Some(CrD);
        }
        if s1 == Xo && s2 == Pd {
            combined = Some(CrPdx);
        }
        if s1 == Zo && s2 == Pd {
            combined = Some(CrPdz);
        }
        if s1 == Xo && s2 == Nd {
            combined = Some(CrNdx);
        }
        if s1 == Zo && s2 == Nd {
            combined = Some(CrNdz);
        }
    }
    combined.unwrap_or(existing)
}

/// What a track block is, for `getConnected`.
pub struct TrackView<'a> {
    pub pos: Pos,
    pub shape: TrackShape,
    /// The `turn` property: the block entity holds curves.
    pub has_be: bool,
    pub dimension: u32,
    /// The curves its block entity holds (`TrackBlockEntity.connections`), each as seen from this block.
    pub curves: &'a [BezierConnection],
    /// `ITrackBlock.getMaterialSimple(world, v)` for a point: the id of the track material there, or `default`.
    pub material: &'a dyn Fn(Vec3, &str) -> String,
}

/// `TrackBlock.getConnected(world, pos, state, linear, connectedTo)`: the track ends of the block (empty when
/// the block is not track).
pub fn track_ends(view: &TrackView, linear: bool, connected_to: Option<&TrackNodeLocation>) -> Vec<DiscoveredLocation> {
    let axes = view.shape.axes();
    let bottom_centre = [f64::from(view.pos[0]) + 0.5, f64::from(view.pos[1]), f64::from(view.pos[2]) + 0.5];
    let normal = view.shape.normal();
    let simple = |v: Vec3| Some((view.material)(v, MATERIAL));
    let mut list: Vec<DiscoveredLocation> = Vec::new();
    if axes.len() > 1 {
        // `getElevationAtCenter`: a junction is never a slope.
        for axis in &axes {
            for from_center in [true, false] {
                let offset = |d: f64, b: bool| {
                    add(scale(*axis, if b { 0.0 } else if from_center { -d } else { d }), bottom_centre)
                };
                let make = |b: bool| {
                    DiscoveredLocation::new(view.dimension, offset(0.5, b))
                        .via_turn(None)
                        .materials(simple(offset(0.0, b)), simple(offset(1.0, b)))
                        .with_normal(normal)
                        .with_direction(Some(*axis))
                        .with_y_offset(0)
                };
                add_to_list_if_connected(connected_to, &mut list, make(true), make(false));
            }
        }
    } else {
        // `ITrackBlock.getConnected`: a straight or sloped block; a slope's centre is half a block up.
        let elevation = if axes[0][1] != 0.0 { 0.5 } else { 0.0 };
        let straight = StraightTrack {
            center: add(bottom_centre, [0.0, elevation, 0.0]),
            axes: axes.clone(),
            normal,
            dimension: view.dimension,
            material: &simple,
            y_offset: &|_| 0,
        };
        list = connected_straight(&straight, connected_to);
    }
    if !view.has_be || linear {
        return list;
    }
    for bc in view.curves {
        let offset = |d: f64, b: bool| {
            let i = usize::from(!b);
            if d == 1.0 {
                [f64::from(bc.be_positions[i][0]), f64::from(bc.be_positions[i][1]), f64::from(bc.be_positions[i][2])]
            } else {
                bc.starts[i]
            }
        };
        let curve_material = |v: Vec3| Some((view.material)(v, &bc.material));
        let make = |b: bool| {
            let i = usize::from(!b);
            DiscoveredLocation::new(view.dimension, offset(0.5, b))
                .via_turn(Some(bc.clone()))
                .materials(curve_material(offset(0.0, b)), curve_material(offset(1.0, b)))
                .with_normal(bc.normals[i])
                .with_direction(None)
                .with_y_offset(0)
        };
        add_to_list_if_connected(connected_to, &mut list, make(true), make(false));
    }
    list
}

// --- placing a curve ----------------------------------------------------------------------------------

/// One block `placeTracks` sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlannedBlock {
    pub pos: Pos,
    pub shape: TrackShape,
    /// The `turn` property (a curve end).
    pub has_be: bool,
}

/// What a valid placement sets down, and what it costs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The blocks to set, straight pieces first (in the order Java writes them), then the curve ends.
    pub blocks: Vec<PlannedBlock>,
    /// `info.requiredTracks`.
    pub required_tracks: i32,
}

/// What `placeTracks` needs to ask of the world for one position.
pub struct WorldAsk<'a> {
    /// `stateAtPos.canBeReplaced() || stateAtPos.is(BlockTags.FLOWERS)`.
    pub can_replace: &'a dyn Fn(Pos) -> bool,
    /// The shape of the track block at a position, if it is one.
    pub track_at: &'a dyn Fn(Pos) -> Option<TrackShape>,
    /// Whether both curve ends already have a block entity and the first holds a connection to the second
    /// (`tte1.getConnections().containsKey(tte2.getBlockPos())`).
    pub already_connected: bool,
}

/// `TrackPlacement.placeTracks`: the cells `info` extends each end over (`extent + 1` when there is a curve), then
/// the curve ends, and the track items it takes. `shape1`/`shape2` are the shapes at the two ends (a clicked
/// track's, or the one chosen for a block being placed).
pub fn plan_tracks(info: &PlacementInfo, shape1: TrackShape, shape2: TrackShape, ask: &WorldAsk) -> Plan {
    let mut blocks = Vec::new();
    let mut required = 0;
    for first in [true, false] {
        let extent = if first { info.end1_extent } else { info.end2_extent };
        let axis = if first { info.axis1 } else { info.axis2 };
        let pos = if first { info.pos1 } else { info.pos2 };
        let mut shape = if first { shape1 } else { shape2 };
        shape = match shape {
            TrackShape::Te | TrackShape::Tw => TrackShape::Xo,
            TrackShape::Tn | TrackShape::Ts => TrackShape::Zo,
            other => other,
        };
        let count = if info.curve.is_some() { extent + 1 } else { extent };
        for i in 0..count {
            let by = block_containing(scale(axis, f64::from(i)));
            let at = [pos[0] + by[0], pos[1] + by[1], pos[2] + by[2]];
            let mut can_place = (ask.can_replace)(at);
            if can_place {
                required += 1;
            }
            let mut put = shape;
            let mut be = false;
            if let Some(existing) = (ask.track_at)(at) {
                put = overlay(existing, put);
                can_place = true;
                // an existing block keeps its own `turn`: the glue leaves the property alone
                be = false;
            }
            if can_place {
                blocks.push(PlannedBlock { pos: at, shape: put, has_be: be });
            }
        }
    }
    if let Some(curve) = &info.curve {
        // The block the cells just set (`stateAtPos`, read after the loop), else the track already there, else the
        // end's own shape as it was clicked.
        for (target, shape) in [(info.target_pos1, shape1), (info.target_pos2, shape2)] {
            let planned = blocks.iter().rev().find(|b| b.pos == target).map(|b| b.shape);
            let shape = planned.or_else(|| (ask.track_at)(target)).unwrap_or(shape);
            blocks.push(PlannedBlock { pos: target, shape, has_be: true });
        }
        if !ask.already_connected {
            required += (curve.segment_count() as i32 + 1) / 2;
        }
    }
    Plan { blocks, required_tracks: required }
}

/// `TrackBlockEntity.removeConnection`: whether the block lets go of its block entity (`HAS_BE` false) once the last
/// curve is gone; a portal track keeps it.
pub fn drops_block_entity(connections_left: usize, shape: TrackShape) -> bool {
    connections_left == 0 && !shape.is_portal()
}

/// `BezierConnection.spawnItems`: where the items of a broken curve appear, one for every other sleeper (the last one
/// left out), at the curve's local position (add the first block entity's corner); with a girder two more each.
pub fn drop_points(bc: &BezierConnection) -> Vec<(Vec3, usize)> {
    let count = bc.segment_count();
    bc.segments_with_offset([0.0; 3])
        .into_iter()
        .filter(|s| s.index % 2 == 0 && s.index != count)
        .map(|s| (s.position, if bc.has_girder { 2 } else { 0 }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::trains::bezier::BezierConnection;
    use crate::rules::trains::placement::{PlacementInput, try_connect};

    fn view<'a>(pos: Pos, shape: TrackShape, has_be: bool, curves: &'a [BezierConnection]) -> TrackView<'a> {
        TrackView {
            pos,
            shape,
            has_be,
            dimension: 0,
            curves,
            material: &|_, default| default.to_owned(),
        }
    }

    #[test]
    fn the_shape_follows_the_look() {
        // Looking south (+z): a Z track; east (+x): an X track; the diagonals in between.
        assert_eq!(shape_for_look([0.0, 0.0, 1.0], 0.0), TrackShape::Zo);
        assert_eq!(shape_for_look([0.0, -0.3, -1.0], 180.0), TrackShape::Zo);
        assert_eq!(shape_for_look([1.0, 0.0, 0.0], 270.0), TrackShape::Xo);
        assert_eq!(shape_for_look([0.7, 0.0, 0.7], 315.0), TrackShape::Pd);
        assert_eq!(shape_for_look([-0.7, 0.0, 0.7], 45.0), TrackShape::Nd);
        // Straight down: the yaw decides (yaw 90 faces west).
        assert_eq!(shape_for_look([0.0, -1.0, 0.0], 90.0), TrackShape::Xo);
    }

    #[test]
    fn a_track_beside_a_ledge_climbs() {
        let ledge = [4, 64, 3];
        let top = |p: Pos| p == ledge;
        let none = |_: Pos| false;
        // At (4, 64, 4) a Z track next to a sturdy block to its north (-z) with air above climbs north.
        assert_eq!(slope_fix(TrackShape::Zo, [4, 64, 4], &top, &none), TrackShape::An);
        // A sturdy block above the ledge keeps it flat.
        let roof = |p: Pos| p == [4, 65, 3];
        assert_eq!(slope_fix(TrackShape::Zo, [4, 64, 4], &top, &roof), TrackShape::Zo);
        // Diagonals never climb (their axis is longer than one).
        assert_eq!(slope_fix(TrackShape::Pd, [4, 64, 4], &top, &none), TrackShape::Pd);
        // The east side of an X track.
        let east = |p: Pos| p == [5, 64, 4];
        assert_eq!(slope_fix(TrackShape::Xo, [4, 64, 4], &east, &none), TrackShape::Ae);
        let west = |p: Pos| p == [3, 64, 4];
        assert_eq!(slope_fix(TrackShape::Xo, [4, 64, 4], &west, &none), TrackShape::Aw);
    }

    #[test]
    fn the_clicked_end_points_away_from_the_player() {
        // A Z track at (0, 64, 0), the player north of it looking south: the end clicked is the far (south) one.
        let (axis, normal, end) = target_end([0, 64, 0], TrackShape::Zo, [0.0, 0.0, 1.0]);
        assert_eq!(axis, [0.0, 0.0, 1.0]);
        assert_eq!(normal, [0.0, 1.0, 0.0]);
        // The curve leaves half a block south of the centre, at the rail's height (the block's bottom).
        assert_eq!(end, [0.5, 64.0, 1.0]);
        // Looking north reaches the north end.
        let (axis, _, end) = target_end([0, 64, 0], TrackShape::Zo, [0.0, 0.0, -1.0]);
        assert_eq!(axis, [0.0, 0.0, -1.0]);
        assert_eq!(end, [0.5, 64.0, 0.0]);
        let selected = select([0, 64, 0], TrackShape::Zo, [0.0, 0.0, 1.0]);
        assert_eq!(selected.pos, [0, 64, 0]);
        assert_eq!(selected.end, end_of(1.0));
    }

    fn end_of(z: f64) -> Vec3 {
        [0.5, 64.0, z]
    }

    #[test]
    fn a_slope_starts_at_its_centre_height() {
        // A vertical axis starts at the block's centre height.
        let start = curve_start([0, 64, 0], [0.0, 1.0, 1.0]);
        assert_eq!(start, [0.5, 65.0, 1.0]);
    }

    #[test]
    fn crossings_form() {
        assert_eq!(overlay(TrackShape::Xo, TrackShape::Zo), TrackShape::CrO);
        assert_eq!(overlay(TrackShape::Zo, TrackShape::Xo), TrackShape::CrO);
        assert_eq!(overlay(TrackShape::Pd, TrackShape::Nd), TrackShape::CrD);
        assert_eq!(overlay(TrackShape::Zo, TrackShape::Pd), TrackShape::CrPdz);
        assert_eq!(overlay(TrackShape::Nd, TrackShape::Xo), TrackShape::CrNdx);
        // Anything else leaves the existing shape.
        assert_eq!(overlay(TrackShape::Xo, TrackShape::Xo), TrackShape::Xo);
        assert_eq!(overlay(TrackShape::Zo, TrackShape::As), TrackShape::Zo);
    }

    #[test]
    fn a_straight_block_has_two_ends_and_a_junction_four() {
        let straight = track_ends(&view([0, 64, 0], TrackShape::Xo, false, &[]), false, None);
        assert_eq!(straight.len(), 2);
        let xs: Vec<f64> = straight.iter().map(|l| l.loc.location()[0]).collect();
        assert!(xs.contains(&1.0) && xs.contains(&0.0), "{xs:?}");
        let cross = track_ends(&view([0, 64, 0], TrackShape::CrO, false, &[]), false, None);
        assert_eq!(cross.len(), 8, "two axes, each a centre end and a side end for both ways");
        // From one end only the others are given.
        let from = straight[0].loc.clone();
        let other = track_ends(&view([0, 64, 0], TrackShape::Xo, false, &[]), false, Some(&from));
        assert_eq!(other.len(), 1);
        assert_ne!(other[0].loc, from);
        // Not reached from a point that is not one of its ends.
        let far = TrackNodeLocation::new([50.0, 64.0, 50.0], 0);
        assert!(track_ends(&view([0, 64, 0], TrackShape::Xo, false, &[]), false, Some(&far)).is_empty());
    }

    fn curve() -> (BezierConnection, PlacementInfo) {
        let cs = |pos: Pos| move |axis: Vec3| curve_start(pos, axis);
        let (c1, c2) = (cs([0, 64, 0]), cs([9, 64, 9]));
        let a1 = [1.0, 0.0, 0.0];
        let a2 = [0.0, 0.0, 1.0];
        let input = PlacementInput {
            from: ConnectingFrom { pos: [0, 64, 0], axis: a1, normal: [0.0, 1.0, 0.0], end: c1(a1) },
            original_has_be: true,
            pos2: [9, 64, 9],
            axis2: a2,
            normal2: [0.0, 1.0, 0.0],
            end2: c2(a2),
            target_tilted: false,
            curve_start1: &c1,
            curve_start2: &c2,
            max_length: 32,
            maximise_turn: false,
            girder: false,
            material: MATERIAL.to_owned(),
            client: false,
        };
        let info = try_connect(&input);
        assert!(info.valid, "{:?}", info.error);
        (info.curve.clone().expect("a turn is a curve"), info)
    }

    #[test]
    fn a_ninety_degree_turn_is_planned_with_its_ends_and_cost() {
        let (bc, info) = curve();
        let ask = WorldAsk { can_replace: &|_| true, track_at: &|_| None, already_connected: false };
        let plan = plan_tracks(&info, TrackShape::Xo, TrackShape::Zo, &ask);
        let ends: Vec<_> = plan.blocks.iter().filter(|b| b.has_be).collect();
        assert_eq!(ends.len(), 2);
        assert_eq!(ends[0].pos, info.target_pos1);
        assert_eq!(ends[1].pos, info.target_pos2);
        let cells = (info.end1_extent + 1 + info.end2_extent + 1) as usize;
        assert_eq!(plan.blocks.len(), cells + 2, "the cells then the two ends (the ends repeat their cell)");
        assert_eq!(plan.required_tracks, cells as i32 + (bc.segment_count() as i32 + 1) / 2);
        // An existing connection costs nothing more for the curve.
        let ask = WorldAsk { can_replace: &|_| true, track_at: &|_| None, already_connected: true };
        assert_eq!(plan_tracks(&info, TrackShape::Xo, TrackShape::Zo, &ask).required_tracks, cells as i32);
        // Existing track in the way crosses rather than costs.
        let ask = WorldAsk {
            can_replace: &|p| p != info.pos1,
            track_at: &|p| (p == info.pos1).then_some(TrackShape::Zo),
            already_connected: false,
        };
        let plan = plan_tracks(&info, TrackShape::Xo, TrackShape::Zo, &ask);
        assert_eq!(plan.blocks[0].shape, TrackShape::CrO);
        assert_eq!(plan.required_tracks, cells as i32 - 1 + (bc.segment_count() as i32 + 1) / 2);
    }

    #[test]
    fn a_portal_shape_becomes_a_plain_one() {
        let (_, info) = curve();
        let ask = WorldAsk { can_replace: &|_| true, track_at: &|_| None, already_connected: false };
        let plan = plan_tracks(&info, TrackShape::Te, TrackShape::Tn, &ask);
        assert!(plan.blocks.iter().all(|b| matches!(b.shape, TrackShape::Xo | TrackShape::Zo)));
    }

    #[test]
    fn a_curve_ends_up_in_the_graph_through_its_ends() {
        let (bc, _) = curve();
        let curves = [bc.clone()];
        let ends = track_ends(&view(bc.be_positions[0], TrackShape::Xo, true, &curves), false, None);
        // The straight's two ends plus the curve's two.
        assert_eq!(ends.len(), 4);
        assert_eq!(ends.iter().filter(|l| l.turn.is_some()).count(), 2);
        // Linear walks leave the curve out.
        assert_eq!(track_ends(&view(bc.be_positions[0], TrackShape::Xo, true, &curves), true, None).len(), 2);
    }

    #[test]
    fn removing_the_last_curve_releases_the_entity() {
        assert!(drops_block_entity(0, TrackShape::Xo));
        assert!(!drops_block_entity(1, TrackShape::Xo));
        assert!(!drops_block_entity(0, TrackShape::Te));
    }

    #[test]
    fn a_broken_curve_drops_every_other_sleeper() {
        let (bc, _) = curve();
        let points = drop_points(&bc);
        assert_eq!(points.len(), bc.segment_count().div_ceil(2), "even indexes below the count");
        assert!(points.iter().all(|(_, girders)| *girders == 0));
    }
}
