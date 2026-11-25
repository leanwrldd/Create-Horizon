//! Portal tracks: `AllPortalTracks`, `PortalTrackProvider` (api/contraption/train/), and the portal halves of `TrackBlock`
//! (`connectToPortal`, `updateShape`, `getConnected`) and `TrackBlockEntity` (`bind`, `boundLocation`). No game types: the
//! glue would ask the world through [`PortalWorld`] and apply the [`Plan`].
//!
//! A track block beside a portal block, running toward it (`TrackShape.XO` or `ZO`, with the portal one block along the
//! rails), becomes a portal track (`TE`/`TN`/`TS`/`TW`) and gets a partner in the other dimension, next to the portal there. The two
//! blocks are bound to each other (`boundLocation`) and the graphs join them through [`bound_ends`]: two ends in different
//! dimensions, both forced nodes. A train that crosses is the carriages' business (`carriage.rs`: `findPivot`, `cutoff`), and
//! what is hidden of a carriage while it is in the portal is `portal_cutoff.rs`.
//!
//! # What of the vanilla destination is here
//!
//! Create asks the game where a portal leads (`Portal.getPortalDestination(level, probe, pos)`: `NetherPortalBlock`, with the
//! `PortalForcer`'s point-of-interest search, and a new portal built when there is none), with a probe entity the size of an
//! entity (0.6 by 1.8: `SuperGlueEntity` has no `sized` in `AllEntityTypes`). Horizon's mod API has **no call for that**: no portal
//! link read, no search of the portal points of interest and no way to build a portal (its `portal_create` and `portal_spawn` hooks
//! only listen). So [`find_exit`] rebuilds the maths from what Horizon's own port of vanilla does (`horizon-world`'s
//! `portal_travel.rs`: `BlockUtil.getLargestRectangleAround`, `PortalShape.getRelativePosition`, `NetherPortalBlock.createDimensionTransition`,
//! `PortalForcer.findClosestPortalPosition`) over the portal blocks the glue lists in the other dimension, and with these differences:
//! - no portal is created when the other side has none: the answer is `None` and the track pops with "Target portal not generated
//!   yet" (`portal_track.missing`), where Create builds one;
//! - the world border (`clampToBounds`, `isWithinBounds`) is ignored;
//! - the collision-free adjustment (`PortalShape.findCollisionFreePosition`) is left out: it only moves a probe that would touch a
//!   block, which the inside of a portal does not;
//! - the portal blocks are the glue's list of loaded blocks, not the points of interest (an unloaded part of the other dimension
//!   has none), and equal-distance ties go to the lower `y`, then `x`, then `z` (vanilla keeps its set's order).
//!
//! Catnip's `Iterate.directionsInAxis` is not in the Java tree: it is east then west, south then north (the positive direction
//! first).

use crate::{Axis, Dir, Pos};

use super::graph::{DiscoveredLocation, TrackNodeLocation};
use super::propagator::add_to_list_if_connected;
use super::track_shape::{Facing, TrackShape};
use super::vec::{Vec3, add, scale};

pub const OVERWORLD: &str = "minecraft:overworld";
pub const NETHER: &str = "minecraft:the_nether";
pub const END: &str = "minecraft:the_end";
pub const NETHER_PORTAL: &str = "minecraft:nether_portal";

/// `AllPortalTracks.registerDefaults`: a portal block and the dimension it leads to from the Overworld (and back), and the mod
/// whose block it is (`tryRegisterSimpleInteraction`; a portal of an absent mod is not registered).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Integration {
    pub portal: &'static str,
    pub dimension: &'static str,
    pub requires_mod: Option<&'static str>,
}

pub const DEFAULTS: [Integration; 4] = [
    Integration { portal: NETHER_PORTAL, dimension: NETHER, requires_mod: None },
    Integration { portal: "aether:aether_portal", dimension: "aether:the_aether", requires_mod: Some("aether") },
    Integration { portal: "aether_ii:aether_portal", dimension: "aether_ii:aether_highlands", requires_mod: Some("aether_ii") },
    Integration { portal: "betterend:end_portal_block", dimension: END, requires_mod: Some("betterend") },
];

/// `PortalTrackProvider.isSupportedPortal` / `REGISTRY.get(block)`: the integration of a portal block, if its mod is there.
pub fn integration(block: &str, mod_loaded: &dyn Fn(&str) -> bool) -> Option<Integration> {
    DEFAULTS.into_iter().find(|i| i.portal == block && i.requires_mod.is_none_or(mod_loaded))
}

pub fn is_supported_portal(block: &str, mod_loaded: &dyn Fn(&str) -> bool) -> bool {
    integration(block, mod_loaded).is_some()
}

/// `BlockFace`: a block position and a face; `getConnectedPos` is the neighbour on that face.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockFace {
    pub pos: Pos,
    pub face: Dir,
}

impl BlockFace {
    pub fn connected_pos(self) -> Pos {
        relative(self.pos, self.face)
    }
}

fn relative(pos: Pos, d: Dir) -> Pos {
    let n = d.normal();
    [pos[0] + n[0], pos[1] + n[1], pos[2] + n[2]]
}

/// `Direction.getClockWise()` for a horizontal direction.
fn clockwise(d: Dir) -> Dir {
    match d {
        Dir::North => Dir::East,
        Dir::East => Dir::South,
        Dir::South => Dir::West,
        Dir::West => Dir::North,
        other => other,
    }
}

/// `PortalTrackProvider.Exit`: the track block on the other side and the face of it that looks at the portal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exit {
    pub dimension: String,
    pub face: BlockFace,
}

/// What the rules ask of the game (the glue reads the loaded blocks of each dimension).
pub trait PortalWorld {
    /// `server.getLevel(dimension) != null && server.isLevelEnabled(..)`.
    fn dimension_enabled(&self, dimension: &str) -> bool;
    /// The block at a position of a dimension and, if it has a `horizontal_axis`, whether that axis is X (a portal block).
    fn portal_block(&self, dimension: &str, pos: Pos) -> Option<PortalBlock>;
    /// Every block named `block` in the loaded part of `dimension` within `radius` blocks of the column `centre` (a square).
    fn portals_around(&self, dimension: &str, block: &str, centre: [i32; 2], radius: i32) -> Vec<Pos>;
    /// `DimensionType.coordinateScale()`: 1 for the Overworld, 8 for the Nether.
    fn coordinate_scale(&self, dimension: &str) -> f64;
}

/// A portal block: its name and the axis its plane lies along.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortalBlock {
    pub name: String,
    pub axis_x: bool,
}

/// `BlockUtil.FoundRectangle`: the low corner and the sizes along the two axes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoundRectangle {
    pub min_corner: Pos,
    pub axis1_size: i32,
    pub axis2_size: i32,
}

fn step(pos: Pos, axis: Axis, n: i32) -> Pos {
    let mut p = pos;
    match axis {
        Axis::X => p[0] += n,
        Axis::Y => p[1] += n,
        Axis::Z => p[2] += n,
    }
    p
}

/// `BlockUtil.getLimit`: how many steps from `start` along `direction` (`+1` or `-1` on `axis`) pass `test`, up to `limit`.
fn limit_of(test: &mut impl FnMut(Pos) -> bool, start: Pos, axis: Axis, sign: i32, limit: i32) -> i32 {
    let mut max = 0;
    let mut pos = start;
    while max < limit {
        pos = step(pos, axis, sign);
        if !test(pos) {
            break;
        }
        max += 1;
    }
    max
}

/// `BlockUtil.getMaxRectangleLocation`: the largest-area run of `columns` (start, end inclusive) and its height.
fn max_rectangle_location(columns: &[i32]) -> (i32, i32, i32) {
    let (mut max_start, mut max_end, mut max_height) = (0usize, 0usize, 0i32);
    let mut stack: Vec<usize> = vec![0];
    for column in 1..=columns.len() {
        let height = if column == columns.len() { 0 } else { columns[column] };
        while let Some(&top) = stack.last() {
            let stack_height = columns[top];
            if height >= stack_height {
                stack.push(column);
                break;
            }
            stack.pop();
            let start = stack.last().map_or(0, |t| t + 1);
            if i64::from(stack_height) * (column - start) as i64 > i64::from(max_height) * (max_end - max_start) as i64 {
                max_end = column;
                max_start = start;
                max_height = stack_height;
            }
        }
        if stack.is_empty() {
            stack.push(column);
        }
    }
    (max_start as i32, max_end as i32 - 1, max_height)
}

/// `BlockUtil.getLargestRectangleAround`: the largest rectangle of positions passing `test` in the plane of `axis1` and `axis2`
/// that contains `center`.
pub fn largest_rectangle_around(
    center: Pos,
    axis1: Axis,
    limit1: i32,
    axis2: Axis,
    limit2: i32,
    mut test: impl FnMut(Pos) -> bool,
) -> FoundRectangle {
    let negative_delta1 = limit_of(&mut test, center, axis1, -1, limit1);
    let positive_delta1 = limit_of(&mut test, center, axis1, 1, limit1);
    let center_index1 = negative_delta1 as usize;
    let mut bounds = vec![(0, 0); center_index1 + 1 + positive_delta1 as usize];
    bounds[center_index1] =
        (limit_of(&mut test, center, axis2, -1, limit2), limit_of(&mut test, center, axis2, 1, limit2));
    let center_index2 = bounds[center_index1].0;
    for i in 1..=negative_delta1 as usize {
        let last = bounds[center_index1 - (i - 1)];
        let at = step(center, axis1, -(i as i32));
        bounds[center_index1 - i] = (limit_of(&mut test, at, axis2, -1, last.0), limit_of(&mut test, at, axis2, 1, last.1));
    }
    for i in 1..=positive_delta1 as usize {
        let last = bounds[center_index1 + i - 1];
        let at = step(center, axis1, i as i32);
        bounds[center_index1 + i] = (limit_of(&mut test, at, axis2, -1, last.0), limit_of(&mut test, at, axis2, 1, last.1));
    }
    let (mut min_axis1, mut min_axis2, mut size_axis1, mut size_axis2) = (0, 0, 0, 0);
    let mut columns = vec![0; bounds.len()];
    for i2 in (0..=center_index2).rev() {
        for (i1, b) in bounds.iter().enumerate() {
            let min2 = center_index2 - b.0;
            let max2 = center_index2 + b.1;
            columns[i1] = if i2 >= min2 && i2 <= max2 { max2 + 1 - i2 } else { 0 };
        }
        let (start, end, height) = max_rectangle_location(&columns);
        let new_size1 = 1 + end - start;
        if new_size1 * height > size_axis1 * size_axis2 {
            min_axis1 = start;
            min_axis2 = i2;
            size_axis1 = new_size1;
            size_axis2 = height;
        }
    }
    let corner = step(center, axis1, min_axis1 - center_index1 as i32);
    let corner = step(corner, axis2, min_axis2 - center_index2);
    FoundRectangle { min_corner: corner, axis1_size: size_axis1, axis2_size: size_axis2 }
}

/// The rectangle of portal blocks (`getLargestRectangleAround(pos, axis, 21, Y, 21, same state)`) around a portal block.
fn portal_rectangle(world: &dyn PortalWorld, dimension: &str, pos: Pos, block: &PortalBlock) -> FoundRectangle {
    let axis = if block.axis_x { Axis::X } else { Axis::Z };
    largest_rectangle_around(pos, axis, 21, Axis::Y, 21, |p| world.portal_block(dimension, p).as_ref() == Some(block))
}

/// A probe of Create's `SuperGlueEntity` (`EntityType` default size).
pub const PROBE_WIDTH: f64 = 0.6;
pub const PROBE_HEIGHT: f64 = 1.8;

/// `PortalShape.getRelativePosition` for an entity of `width` by `height` whose feet are at `position`: how far right and up
/// in the portal rectangle it stands (fractions), and its forward offset in blocks.
pub fn relative_position(rect: FoundRectangle, axis_x: bool, position: Vec3, width: f64, height: f64) -> Vec3 {
    let room_width = f64::from(rect.axis1_size) - width;
    let room_height = f64::from(rect.axis2_size) - height;
    let bottom = rect.min_corner;
    let clamp = |v: f64| v.clamp(0.0, 1.0);
    let (along, forward_at) = if axis_x { (position[0], position[2]) } else { (position[2], position[0]) };
    let (bottom_along, bottom_forward) = if axis_x { (bottom[0], bottom[2]) } else { (bottom[2], bottom[0]) };
    let right = if room_width > 0.0 { clamp((along - (f64::from(bottom_along) + width / 2.0)) / room_width) } else { 0.5 };
    let up = if room_height > 0.0 { clamp((position[1] - f64::from(bottom[1])) / room_height) } else { 0.0 };
    [right, up, forward_at - (f64::from(bottom_forward) + 0.5)]
}

/// `NetherPortalBlock.createDimensionTransition` before the collision-free step: where an entity with the relative position
/// `offset` stands in the exit rectangle.
pub fn exit_position(exit: FoundRectangle, exit_axis_x: bool, offset: Vec3, width: f64, height: f64) -> Vec3 {
    let right = width / 2.0 + (f64::from(exit.axis1_size) - width) * offset[0];
    let up = (f64::from(exit.axis2_size) - height) * offset[1];
    let forward = 0.5 + offset[2];
    let b = exit.min_corner;
    [
        f64::from(b[0]) + if exit_axis_x { right } else { forward },
        f64::from(b[1]) + up,
        f64::from(b[2]) + if exit_axis_x { forward } else { right },
    ]
}

/// `PortalForcer.findClosestPortalPosition`: the nearest of `portals` to `approximate` within 16 blocks (going to the Nether) or
/// 128 (out of it), nearest first, then the lowest.
pub fn closest_portal(portals: &[Pos], approximate: Pos, to_nether: bool) -> Option<Pos> {
    let radius = if to_nether { 16 } else { 128 };
    let mut best: Option<(i64, Pos)> = None;
    for &p in portals {
        if (p[0] - approximate[0]).abs() > radius || (p[2] - approximate[2]).abs() > radius {
            continue;
        }
        let d: i64 = (0..3).map(|i| i64::from(p[i] - approximate[i]).pow(2)).sum();
        let better = match best {
            None => true,
            Some((bd, bp)) => d < bd || (d == bd && (p[1], p[0], p[2]) < (bp[1], bp[0], bp[2])),
        };
        if better {
            best = Some((d, p));
        }
    }
    best.map(|(_, p)| p)
}

/// The position `getPortalDestination` looks for a portal near: the probe's position scaled by the dimensions' coordinate scales
/// (`DimensionType.getTeleportationScale`), `y` unchanged.
pub fn approximate_position(from_scale: f64, to_scale: f64, pos: Pos) -> Pos {
    let s = from_scale / to_scale;
    [((f64::from(pos[0]) + 0.5) * s).floor() as i32, pos[1], ((f64::from(pos[2]) + 0.5) * s).floor() as i32]
}

/// The block that contains a position (`BlockPos.containing`).
fn containing(v: Vec3) -> Pos {
    [v[0].floor() as i32, v[1].floor() as i32, v[2].floor() as i32]
}

/// `AllPortalTracks.fromPortal`: where the track on the other side of the portal goes, for the track block face `inbound` in
/// `dimension` and a portal that leads between `first` and `second` (`PortalTrackProvider.fromPortal`). `None` when the other
/// dimension isn't there, the portal leads nowhere, or what stands where the probe arrives isn't the same portal block.
pub fn find_exit(world: &dyn PortalWorld, dimension: &str, inbound: BlockFace, first: &str, second: &str) -> Option<Exit> {
    let other = if dimension == second { first } else { second };
    if !world.dimension_enabled(other) {
        return None;
    }
    let portal_pos = inbound.connected_pos();
    let portal = world.portal_block(dimension, portal_pos)?;
    // `portal.getPortalDestination(level, probe, probe.blockPosition())`.
    let entry = portal_rectangle(world, dimension, portal_pos, &portal);
    let probe = [f64::from(portal_pos[0]) + 0.5, f64::from(portal_pos[1]), f64::from(portal_pos[2]) + 0.5];
    let rel_pos = relative_position(entry, portal.axis_x, probe, PROBE_WIDTH, PROBE_HEIGHT);
    let approximate = approximate_position(world.coordinate_scale(dimension), world.coordinate_scale(other), portal_pos);
    let to_nether = other == NETHER;
    let candidates = world.portals_around(other, &portal.name, [approximate[0], approximate[2]], if to_nether { 16 } else { 128 });
    let found = closest_portal(&candidates, approximate, to_nether)?;
    let found_block = world.portal_block(other, found)?;
    let exit_rect = portal_rectangle(world, other, found, &found_block);
    let at = exit_position(exit_rect, found_block.axis_x, rel_pos, PROBE_WIDTH, PROBE_HEIGHT);
    let other_portal_pos = containing(at);
    // `otherPortalState.is(portalState.getBlock())`.
    let other_state = world.portal_block(other, other_portal_pos)?;
    if other_state.name != portal.name {
        return None;
    }
    let mut target = inbound.face;
    let other_axis = if other_state.axis_x { Axis::X } else { Axis::Z };
    if target.axis() == other_axis {
        target = clockwise(target);
    }
    let other_pos = relative(other_portal_pos, target);
    Some(Exit { dimension: other.to_owned(), face: BlockFace { pos: other_pos, face: target.opposite() } })
}

// --- TrackBlock.connectToPortal --------------------------------------------------------------------------------------

/// What went wrong with a portal track (`TrackBlock.connectToPortal`'s `fail`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// `portal_track.missing`: no exit found.
    Missing,
    /// `portal_track.blocked`: something stands where the partner would go.
    Blocked(Pos),
}

/// One portal this track block links to: it becomes the portal shape of `direction`, its partner stands at `exit`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub direction: Dir,
    pub exit: Exit,
    /// `TrackShape.asPortal(d)` for this block.
    pub shape: TrackShape,
    /// `TrackShape.asPortal(otherTrack.getFace())` for the partner.
    pub partner_shape: TrackShape,
}

/// What `connectToPortal` does.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Each link made, in order (`setBlock` of both blocks and `bind` of each to the other).
    pub links: Vec<Link>,
    /// The block is broken (`level.destroyBlock(pos, true)`): a portal was beside it and nothing linked.
    pub pop: bool,
    /// The message to show the nearest player within ten blocks, if something failed.
    pub failure: Option<Failure>,
}

fn facing_of(d: Dir) -> Option<Facing> {
    match d {
        Dir::North => Some(Facing::North),
        Dir::South => Some(Facing::South),
        Dir::East => Some(Facing::East),
        Dir::West => Some(Facing::West),
        _ => None,
    }
}

/// `TrackBlock.connectToPortal`: for the plain track block at `pos` of `shape` in `dimension`. `can_replace` is
/// `otherLevel.getBlockState(otherTrackPos).canBeReplaced()` for a block of another dimension; `mod_loaded` the loaded mods.
pub fn connect_to_portal(
    world: &dyn PortalWorld,
    dimension: &str,
    pos: Pos,
    shape: TrackShape,
    can_replace: &dyn Fn(&str, Pos) -> bool,
    mod_loaded: &dyn Fn(&str) -> bool,
) -> Plan {
    let axis = match shape {
        TrackShape::Xo => Axis::X,
        TrackShape::Zo => Axis::Z,
        _ => return Plan::default(),
    };
    let mut plan = Plan::default();
    // `Iterate.directionsInAxis(axis)`.
    let directions = if axis == Axis::X { [Dir::East, Dir::West] } else { [Dir::South, Dir::North] };
    for d in directions {
        let portal_pos = relative(pos, d);
        let Some(portal) = world.portal_block(dimension, portal_pos) else { continue };
        let Some(integration) = integration(&portal.name, mod_loaded) else { continue };
        plan.pop = true;
        let Some(exit) = find_exit(world, dimension, BlockFace { pos, face: d }, OVERWORLD, integration.dimension) else {
            plan.failure = Some(Failure::Missing);
            continue;
        };
        let other_pos = exit.face.pos;
        if !can_replace(&exit.dimension, other_pos) {
            plan.failure = Some(Failure::Blocked(other_pos));
            continue;
        }
        let (Some(f), Some(partner)) = (facing_of(d), facing_of(exit.face.face)) else { continue };
        plan.links.push(Link { direction: d, shape: TrackShape::as_portal(f), partner_shape: TrackShape::as_portal(partner), exit });
        plan.pop = false;
    }
    plan
}

/// The two lines shown to the player: `<!> Cannot place portal track:` and ` - <reason>` (`portal_track.failed`, `.missing`,
/// `.blocked`; Create's lang file).
pub fn failure_lines(failure: Failure) -> [String; 2] {
    let reason = match failure {
        Failure::Missing => "Target portal not generated yet".to_owned(),
        Failure::Blocked(p) => format!("Target location blocked ({},{},{})", p[0], p[1], p[2]),
    };
    ["<!> Cannot place portal track:".to_owned(), format!(" - {reason}")]
}

/// `TrackBlock.updateShape`: a portal track whose portal is gone from the side it faces becomes air. `changed` is the side of
/// the neighbour update, `portal` whether the block there is a supported portal.
pub fn portal_track_survives(shape: TrackShape, changed: Dir, portal_still_there: bool) -> bool {
    if !shape.is_portal() {
        return true;
    }
    for d in [Dir::North, Dir::East, Dir::South, Dir::West] {
        let Some(f) = facing_of(d) else { continue };
        if TrackShape::as_portal(f) != shape || changed != d {
            continue;
        }
        return portal_still_there;
    }
    true
}

// --- TrackBlock.getConnected, the bound half ---------------------------------------------------------------------------

/// The track block a portal track is bound to (`TrackBlockEntity.boundLocation`), read from the other dimension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bound {
    pub dimension: u32,
    pub pos: Pos,
    pub shape: TrackShape,
}

/// `getElevationAtCenter` for a block of `shape`: half a block for a slope, else none.
fn elevation(shape: TrackShape) -> f64 {
    if shape.axes()[0][1] != 0.0 { 0.5 } else { 0.0 }
}

/// `TrackBlock.getConnected`'s `boundLocation` part: for each of this block's axes, one end at this block and one at the bound
/// block, in their own dimensions (so both are forced nodes). `material(on_this_block, point)` is
/// `ITrackBlock.getMaterialSimple(b ? level : otherLevel, v)`.
pub fn bound_ends(
    dimension: u32,
    pos: Pos,
    shape: TrackShape,
    bound: &Bound,
    connected_to: Option<&TrackNodeLocation>,
    material: &dyn Fn(bool, Vec3) -> String,
) -> Vec<DiscoveredLocation> {
    let bottom_centre = |p: Pos| [f64::from(p[0]) + 0.5, f64::from(p[1]), f64::from(p[2]) + 0.5];
    let center = add(bottom_centre(pos), [0.0, elevation(shape), 0.0]);
    let bound_center = add(bottom_centre(bound.pos), [0.0, elevation(bound.shape), 0.0]);
    let bound_axis = bound.shape.axes()[0];
    let mut list = Vec::new();
    for axis in shape.axes() {
        let offset = |d: f64, b: bool| add(scale(if b { axis } else { bound_axis }, d), if b { center } else { bound_center });
        let make = |b: bool| {
            let simple = |v: Vec3| Some(material(b, v));
            DiscoveredLocation::new(if b { dimension } else { bound.dimension }, offset(0.5, b))
                .via_turn(None)
                .materials(simple(offset(0.0, b)), simple(offset(1.0, b)))
                .with_normal(if b { shape } else { bound.shape }.normal())
                .with_direction(Some(axis))
                .with_y_offset(0)
        };
        add_to_list_if_connected(connected_to, &mut list, make(true), make(false));
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct World {
        blocks: HashMap<(String, Pos), PortalBlock>,
        disabled: Vec<String>,
    }

    impl World {
        /// A rectangle of portal blocks `w` wide along x or z and `h` tall with its low corner at `at`.
        fn portal(&mut self, dim: &str, at: Pos, axis_x: bool, w: i32, h: i32) {
            for i in 0..w {
                for j in 0..h {
                    let p = if axis_x { [at[0] + i, at[1] + j, at[2]] } else { [at[0], at[1] + j, at[2] + i] };
                    self.blocks.insert((dim.to_owned(), p), PortalBlock { name: NETHER_PORTAL.into(), axis_x });
                }
            }
        }
    }

    impl PortalWorld for World {
        fn dimension_enabled(&self, d: &str) -> bool {
            !self.disabled.iter().any(|x| x == d)
        }
        fn portal_block(&self, d: &str, pos: Pos) -> Option<PortalBlock> {
            self.blocks.get(&(d.to_owned(), pos)).cloned()
        }
        fn portals_around(&self, d: &str, block: &str, c: [i32; 2], r: i32) -> Vec<Pos> {
            self.blocks
                .iter()
                .filter(|((dim, p), b)| dim == d && b.name == block && (p[0] - c[0]).abs() <= r && (p[2] - c[1]).abs() <= r)
                .map(|((_, p), _)| *p)
                .collect()
        }
        fn coordinate_scale(&self, d: &str) -> f64 {
            if d == NETHER { 8.0 } else { 1.0 }
        }
    }

    fn mods(_: &str) -> bool {
        false
    }

    #[test]
    fn the_defaults_are_the_nether_and_the_mods_that_are_there() {
        assert!(is_supported_portal(NETHER_PORTAL, &mods));
        assert!(!is_supported_portal("aether:aether_portal", &mods));
        assert!(is_supported_portal("aether:aether_portal", &|m| m == "aether"));
        assert!(!is_supported_portal("aether:aether_portal", &|m| m == "aether_ii"));
        assert_eq!(integration("betterend:end_portal_block", &|_| true).map(|i| i.dimension), Some(END));
        assert!(!is_supported_portal("minecraft:end_portal", &|_| true));
    }

    #[test]
    fn the_largest_rectangle_is_the_portal_interior() {
        let mut w = World::default();
        w.portal("a", [100, 64, 50], true, 2, 3);
        let rect = portal_rectangle(&w, "a", [101, 65, 50], &w.portal_block("a", [101, 65, 50]).unwrap());
        assert_eq!(rect, FoundRectangle { min_corner: [100, 64, 50], axis1_size: 2, axis2_size: 3 });
        let mut z = World::default();
        z.portal("a", [7, 70, 20], false, 3, 3);
        let rect = portal_rectangle(&z, "a", [7, 71, 21], &z.portal_block("a", [7, 71, 21]).unwrap());
        assert_eq!(rect, FoundRectangle { min_corner: [7, 70, 20], axis1_size: 3, axis2_size: 3 });
    }

    #[test]
    fn a_probe_at_the_bottom_left_of_a_portal_has_zero_relative_position_but_for_the_width() {
        let rect = FoundRectangle { min_corner: [100, 64, 50], axis1_size: 2, axis2_size: 3 };
        let r = relative_position(rect, true, [100.5, 64.0, 50.5], PROBE_WIDTH, PROBE_HEIGHT);
        assert!((r[0] - 0.2 / 1.4).abs() < 1e-9 && r[1] == 0.0 && r[2] == 0.0);
        // The far column and the top row.
        let r = relative_position(rect, true, [101.5, 66.0, 50.5], PROBE_WIDTH, PROBE_HEIGHT);
        assert!(r[0] > 0.85 && r[1] == 1.0);
        // A portal as narrow as the probe has no room: the middle.
        let narrow = FoundRectangle { min_corner: [0, 0, 0], axis1_size: 1, axis2_size: 2 };
        assert_eq!(relative_position(narrow, true, [0.5, 0.0, 0.5], PROBE_WIDTH, PROBE_HEIGHT)[0], 0.5);
    }

    #[test]
    fn the_closest_portal_is_nearest_then_lowest() {
        let portals = [[12, 70, 6], [13, 70, 6], [12, 71, 6], [200, 70, 6]];
        assert_eq!(closest_portal(&portals, [12, 64, 6], true), Some([12, 70, 6]));
        // Out of the Nether the search reaches 128; into it, 16.
        assert_eq!(closest_portal(&[[100, 64, 0]], [0, 64, 0], true), None);
        assert_eq!(closest_portal(&[[100, 64, 0]], [0, 64, 0], false), Some([100, 64, 0]));
        // A tie goes to the lower block.
        assert_eq!(closest_portal(&[[1, 66, 0], [1, 62, 0]], [1, 64, 0], true), Some([1, 62, 0]));
    }

    #[test]
    fn the_nether_position_is_one_eighth_and_the_overworld_eight_times() {
        assert_eq!(approximate_position(1.0, 8.0, [100, 64, 50]), [12, 64, 6]);
        assert_eq!(approximate_position(8.0, 1.0, [12, 70, 6]), [100, 70, 52]);
    }

    /// An Overworld portal (axis x, 2 by 3) at x 100..101, and a Nether one (axis x, 2 by 3) at x 12..13.
    fn two_worlds() -> World {
        let mut w = World::default();
        w.portal(OVERWORLD, [100, 64, 50], true, 2, 3);
        w.portal(NETHER, [12, 70, 6], true, 2, 3);
        w
    }

    #[test]
    fn a_track_beside_an_overworld_portal_finds_its_partner_beside_the_nether_one() {
        let w = two_worlds();
        let exit = find_exit(&w, OVERWORLD, BlockFace { pos: [99, 64, 50], face: Dir::East }, OVERWORLD, NETHER).expect("exit");
        // The probe stood at the left of the bottom row, so it arrives at the left of the bottom row; the exit portal's axis is x,
        // as the track's direction is, so the exit turns a quarter clockwise: east becomes south.
        assert_eq!(exit.dimension, NETHER);
        assert_eq!(exit.face, BlockFace { pos: [12, 70, 7], face: Dir::North });
    }

    #[test]
    fn the_exit_turns_only_when_the_portals_axis_is_the_directions() {
        let mut w = World::default();
        w.portal(OVERWORLD, [100, 64, 50], true, 2, 3);
        // The Nether portal's plane is along z: an eastward track doesn't turn.
        w.portal(NETHER, [12, 70, 6], false, 2, 3);
        let exit = find_exit(&w, OVERWORLD, BlockFace { pos: [99, 64, 50], face: Dir::East }, OVERWORLD, NETHER).expect("exit");
        assert_eq!(exit.face.face, Dir::West);
        assert_eq!(exit.face.pos, [13, 70, 6]);
    }

    #[test]
    fn there_is_no_exit_without_a_portal_on_the_other_side_or_a_dimension() {
        let mut w = World::default();
        w.portal(OVERWORLD, [100, 64, 50], true, 2, 3);
        assert_eq!(find_exit(&w, OVERWORLD, BlockFace { pos: [99, 64, 50], face: Dir::East }, OVERWORLD, NETHER), None);
        let mut w = two_worlds();
        w.disabled.push(NETHER.into());
        assert_eq!(find_exit(&w, OVERWORLD, BlockFace { pos: [99, 64, 50], face: Dir::East }, OVERWORLD, NETHER), None);
        // No portal block where the face looks.
        let w = two_worlds();
        assert_eq!(find_exit(&w, OVERWORLD, BlockFace { pos: [90, 64, 50], face: Dir::East }, OVERWORLD, NETHER), None);
    }

    #[test]
    fn and_back_from_the_nether() {
        let w = two_worlds();
        // The other way: the track beside the Nether portal, to the Overworld one at 8 times the coordinates.
        let exit = find_exit(&w, NETHER, BlockFace { pos: [11, 70, 6], face: Dir::East }, OVERWORLD, NETHER).expect("exit");
        assert_eq!(exit.dimension, OVERWORLD);
        assert_eq!(exit.face.face, Dir::North);
        assert_eq!(exit.face.pos, [100, 64, 51]);
    }

    fn free(_: &str, _: Pos) -> bool {
        true
    }

    #[test]
    fn connecting_makes_a_portal_shape_pair() {
        let w = two_worlds();
        let plan = connect_to_portal(&w, OVERWORLD, [99, 64, 50], TrackShape::Xo, &free, &mods);
        assert!(!plan.pop && plan.failure.is_none());
        assert_eq!(plan.links.len(), 1);
        let link = &plan.links[0];
        assert_eq!(link.direction, Dir::East);
        assert_eq!(link.shape, TrackShape::Te);
        assert_eq!(link.partner_shape, TrackShape::Tn);
        assert_eq!(link.exit.face.pos, [12, 70, 7]);
    }

    #[test]
    fn a_track_with_no_portal_beside_it_does_nothing() {
        let w = two_worlds();
        let plan = connect_to_portal(&w, OVERWORLD, [90, 64, 50], TrackShape::Xo, &free, &mods);
        assert_eq!(plan, Plan::default());
        // A track that isn't straight along an axis doesn't look.
        assert_eq!(connect_to_portal(&w, OVERWORLD, [99, 64, 50], TrackShape::Pd, &free, &mods), Plan::default());
        // A track along z beside an x portal's end has no portal on its own axis.
        assert_eq!(connect_to_portal(&w, OVERWORLD, [99, 64, 50], TrackShape::Zo, &free, &mods), Plan::default());
    }

    #[test]
    fn a_missing_exit_pops_the_track_with_a_message() {
        let mut w = World::default();
        w.portal(OVERWORLD, [100, 64, 50], true, 2, 3);
        let plan = connect_to_portal(&w, OVERWORLD, [99, 64, 50], TrackShape::Xo, &free, &mods);
        assert!(plan.pop && plan.links.is_empty());
        assert_eq!(plan.failure, Some(Failure::Missing));
        assert_eq!(failure_lines(Failure::Missing)[1], " - Target portal not generated yet");
    }

    #[test]
    fn a_blocked_exit_says_where() {
        let w = two_worlds();
        let plan = connect_to_portal(&w, OVERWORLD, [99, 64, 50], TrackShape::Xo, &|_, p| p != [12, 70, 7], &mods);
        assert!(plan.pop);
        assert_eq!(plan.failure, Some(Failure::Blocked([12, 70, 7])));
        assert_eq!(failure_lines(Failure::Blocked([12, 70, 7]))[1], " - Target location blocked (12,70,7)");
        assert_eq!(failure_lines(Failure::Missing)[0], "<!> Cannot place portal track:");
    }

    #[test]
    fn a_portal_of_a_mod_that_is_not_loaded_is_not_a_portal() {
        let mut w = World::default();
        w.blocks.insert((OVERWORLD.into(), [100, 64, 50]), PortalBlock { name: "aether:aether_portal".into(), axis_x: true });
        assert_eq!(connect_to_portal(&w, OVERWORLD, [99, 64, 50], TrackShape::Xo, &free, &mods), Plan::default());
    }

    #[test]
    fn a_portal_track_goes_when_its_portal_does() {
        // TE faces east: only a change on the east side matters.
        assert!(!portal_track_survives(TrackShape::Te, Dir::East, false));
        assert!(portal_track_survives(TrackShape::Te, Dir::East, true));
        assert!(portal_track_survives(TrackShape::Te, Dir::West, false));
        assert!(!portal_track_survives(TrackShape::Tn, Dir::North, false));
        assert!(portal_track_survives(TrackShape::Xo, Dir::East, false));
    }

    #[test]
    fn bound_ends_join_the_two_dimensions_with_forced_nodes() {
        let bound = Bound { dimension: 1, pos: [12, 70, 7], shape: TrackShape::Tn };
        let ends = bound_ends(0, [99, 64, 50], TrackShape::Te, &bound, None, &|_, _| "create:andesite".into());
        assert_eq!(ends.len(), 2);
        // This block's end half a block east of its centre; the partner's, half a block north of its own.
        assert_eq!(ends[0].loc.location(), [100.0, 64.0, 50.5]);
        assert_eq!(ends[1].loc.location(), [12.5, 70.0, 7.0]);
        assert_eq!(ends[0].loc.dimension, 0);
        assert_eq!(ends[1].loc.dimension, 1);
        assert!(ends.iter().all(|e| e.force_node));
        // From one end only the other is offered.
        let from = ends[0].loc.clone();
        let only = bound_ends(0, [99, 64, 50], TrackShape::Te, &bound, Some(&from), &|_, _| "create:andesite".into());
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].loc.dimension, 1);
    }
}
