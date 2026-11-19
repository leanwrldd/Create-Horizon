//! Moving structures made solid: what a controller (piston, pulley, gantry, bearing, elevator) hands
//! the collider so that entities stand on, ride and are pushed by it. This is the data side of
//! `Contraption.invalidateColliders`, `Contraption.bounds` / `expandBoundsAroundAxis`,
//! `AbstractContraptionEntity.canCollideWith` and the entity-facing helpers of `ContraptionCollider`
//! (`collide`'s `Entity.collideBoundingBox` and `ContraptionHandlerClient.rayTraceContraption`).
//! The collision itself is `collider.rs` / `collider_obb.rs`.
//!
//! Where Create reads Minecraft, the caller fills a plain value in:
//! - a block's collision shape: boxes relative to the block (`BlockState.getCollisionShape`), by block;
//! - a block's name, which stands in for the tags and classes Create asks about
//!   ([`block_props`]: climbable, bounce, friction);
//! - an entity's kind, which stands in for `canCollideWith`'s `instanceof` tests ([`entity_facts`]).

use std::collections::HashMap;

use super::actor::Vec3;
use super::collider::{BlockProps, CollideFacts, Rotation, StructurePose, bounce_multiplier, rotate};
use super::collider_obb::{Aabb, CollisionList, vadd, vsub};
use crate::{Axis, Pos};

/// The speed Create uses for `Contraption.bounds` is a union of unit cells; the entity's box in
/// `ContraptionCollider.collideEntities` is `bounds.move(anchor)`.
#[derive(Clone, Debug)]
pub struct Solid {
    /// `Contraption.getSimplifiedEntityColliders`: the structure's collision boxes, local space.
    pub list: CollisionList,
    /// `Contraption.bounds`: every block's cell, joined (and widened round a turning axis).
    pub local_bounds: Aabb,
    props: HashMap<Pos, BlockProps>,
}

/// One block of a structure as [`Solid::build`] takes it.
pub struct SolidBlock<'a> {
    pub local: Pos,
    /// The block's name (`create:shaft`), for [`block_props`].
    pub name: &'a str,
    /// `getCollisionShape`: boxes relative to the block, `min` to `max`, in blocks.
    pub boxes: &'a [Aabb],
}

impl Solid {
    /// `invalidateColliders` and the bounds of `addBlock` (the anchor cell is in them from the
    /// start: `new AABB(BlockPos.ZERO)`), then `expandBoundsAroundAxis` for a structure that turns.
    pub fn build(blocks: &[SolidBlock<'_>], turn_axis: Option<Axis>) -> Solid {
        let mut list = CollisionList::new();
        let mut bounds = Aabb::new([0.0; 3], [1.0; 3]);
        let mut props = HashMap::new();
        for block in blocks {
            // `bounds.minmax(new AABB(localPos))`.
            for i in 0..3 {
                bounds.min[i] = bounds.min[i].min(f64::from(block.local[i]));
                bounds.max[i] = bounds.max[i].max(f64::from(block.local[i]) + 1.0);
            }
            for b in block.boxes {
                list.append_box(block.local, b.min, b.max);
            }
            props.insert(block.local, block_props(block.name));
        }
        if let Some(axis) = turn_axis {
            bounds = expand_bounds_around_axis(&bounds, blocks.iter().map(|b| b.local), axis);
        }
        Solid { list, local_bounds: bounds, props }
    }

    /// The block at a local position, as the collider asks about it.
    pub fn props_at(&self, pos: Pos) -> Option<BlockProps> {
        self.props.get(&pos).copied()
    }

    /// The entity's box (`setPos`: `bounds.move(getAnchorVec())`).
    pub fn world_bounds(&self, pose: &StructurePose) -> Aabb {
        pose.bounding_box(&self.local_bounds)
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
}

/// `Contraption.expandBoundsAroundAxis` and `getRadius`: the box grows to the farthest block's
/// distance from the axis (plus one and two cells), the axis's own extent kept.
pub fn expand_bounds_around_axis(bounds: &Aabb, blocks: impl Iterator<Item = Pos>, axis: Axis) -> Aabb {
    let (a, b) = match axis {
        Axis::X => (1, 2),
        Axis::Y => (0, 2),
        Axis::Z => (0, 1),
    };
    let max_dist_sq = blocks
        .map(|p| i64::from(p[a]) * i64::from(p[a]) + i64::from(p[b]) * i64::from(p[b]))
        .max()
        .unwrap_or(0);
    let radius = (max_dist_sq as f64).sqrt().ceil();
    let mut min = [-radius - 1.0; 3];
    let mut max = [radius + 2.0; 3];
    let i = match axis {
        Axis::X => 0,
        Axis::Y => 1,
        Axis::Z => 2,
    };
    // `(int) bounds.minX`: truncation toward zero.
    min[i] = bounds.min[i].trunc();
    max[i] = bounds.max[i].trunc();
    Aabb::new(min, max)
}

// --- what a block is ---------------------------------------------------------------------------

/// The parts of `ContraptionCollider`'s block lookup a block's name decides: `BlockTags.CLIMBABLE`,
/// `BlockHelper.getBounceMultiplier` and `BlockState.getFriction`.
pub fn block_props(name: &str) -> BlockProps {
    let path = name.strip_prefix("minecraft:").unwrap_or(name);
    let climbable = matches!(
        path,
        "ladder"
            | "vine"
            | "scaffolding"
            | "weeping_vines"
            | "weeping_vines_plant"
            | "twisting_vines"
            | "twisting_vines_plant"
            | "cave_vines"
            | "cave_vines_plant"
    ) && name.starts_with("minecraft:");
    let is_slime = name == "minecraft:slime_block";
    let is_bed = name.starts_with("minecraft:") && path.ends_with("_bed");
    let friction = if is_slime {
        0.8
    } else if name == "minecraft:ice" || name == "minecraft:packed_ice" || name == "minecraft:frosted_ice" {
        0.98
    } else if name == "minecraft:blue_ice" {
        0.989
    } else {
        0.6
    };
    BlockProps { climbable, bounce: bounce_multiplier(is_slime, is_bed), friction, has_interactor: false }
}

// --- which entities collide --------------------------------------------------------------------

/// What an entity's kind says about `AbstractContraptionEntity.canCollideWith` and the collider's
/// special cases. `living` is `LivingEntity` (`maxUpStep` is the step height attribute for those,
/// 0 for the rest: `Entity.maxUpStep`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KindFacts {
    pub collide: CollideFacts,
    pub is_item: bool,
    pub is_player: bool,
}

/// An entity kind (`minecraft:zombie`) as the collider sees it. Only the kinds Create's tests name
/// are special: players (not collided on the server), hanging entities, minecarts, projectiles
/// (`Projectile`) and the kinds whose `getPistonPushReaction` is not `NORMAL`.
pub fn entity_facts(kind: &str) -> KindFacts {
    let path = kind.strip_prefix("minecraft:").unwrap_or(kind);
    let vanilla = kind.starts_with("minecraft:");
    let mut facts = KindFacts::default();
    facts.collide.push_reaction_normal = true;
    if !vanilla {
        return facts;
    }
    facts.is_player = path == "player";
    facts.collide.is_player = facts.is_player;
    facts.is_item = path == "item";
    facts.collide.is_hanging = matches!(path, "item_frame" | "glow_item_frame" | "painting" | "leash_knot");
    facts.collide.is_minecart = path.ends_with("minecart");
    facts.collide.is_projectile = matches!(
        path,
        "arrow"
            | "spectral_arrow"
            | "trident"
            | "snowball"
            | "egg"
            | "blue_egg"
            | "brown_egg"
            | "ender_pearl"
            | "experience_bottle"
            | "splash_potion"
            | "lingering_potion"
            | "fireball"
            | "small_fireball"
            | "dragon_fireball"
            | "wither_skull"
            | "llama_spit"
            | "shulker_bullet"
            | "firework_rocket"
            | "fishing_bobber"
            | "wind_charge"
            | "breeze_wind_charge"
            | "eye_of_ender"
    );
    // `getPistonPushReaction() != NORMAL`: a marker, the display and interaction entities, falling
    // blocks and effect clouds are ignored by pistons.
    if matches!(
        path,
        "marker"
            | "block_display"
            | "item_display"
            | "text_display"
            | "interaction"
            | "falling_block"
            | "area_effect_cloud"
            | "lightning_bolt"
    ) {
        facts.collide.push_reaction_normal = false;
    }
    facts
}

// --- Entity.collideBoundingBox -------------------------------------------------------------

/// `VoxelShape.collide` against one box along one axis: how far `bb` can move by `movement`.
fn collide_axis(axis: usize, bb: &Aabb, b: &Aabb, movement: f64) -> f64 {
    const EPS: f64 = 1.0E-7;
    // The other two axes must overlap (`findIndex(min + 1e-7)` .. `findIndex(max - 1e-7)`).
    for i in 0..3 {
        if i != axis && !(bb.min[i] + EPS < b.max[i] && bb.max[i] - EPS > b.min[i]) {
            return movement;
        }
    }
    if movement > 0.0 {
        let d = b.min[axis] - bb.max[axis];
        if d >= -EPS && d < movement {
            return d;
        }
    } else if movement < 0.0 {
        let d = b.max[axis] - bb.min[axis];
        if d <= EPS && d > movement {
            return d;
        }
    }
    movement
}

/// `Shapes.collide(axis, box, shapes, movement)`.
fn collide_shapes(axis: usize, bb: &Aabb, boxes: &[Aabb], mut movement: f64) -> f64 {
    for b in boxes {
        if movement.abs() < 1.0E-7 {
            return 0.0;
        }
        movement = collide_axis(axis, bb, b, movement);
    }
    movement
}

/// `Entity.collideWithShapes` / `collideBoundingBox` without entities: how far a box can move by a
/// vector before the blocks' collision `boxes` (world space) stop it: Y first, then the horizontal
/// axis with the larger movement (`Direction.axisStepOrder`).
pub fn collide_bounding_box(movement: Vec3, bb: &Aabb, boxes: &[Aabb]) -> Vec3 {
    if boxes.is_empty() {
        return movement;
    }
    let [mut x, mut y, mut z] = movement;
    let mut bb = *bb;
    if y != 0.0 {
        y = collide_shapes(1, &bb, boxes, y);
        if y != 0.0 {
            bb = bb.moved([0.0, y, 0.0]);
        }
    }
    let z_priority = x.abs() < z.abs();
    if z_priority && z != 0.0 {
        z = collide_shapes(2, &bb, boxes, z);
        if z != 0.0 {
            bb = bb.moved([0.0, 0.0, z]);
        }
    }
    if x != 0.0 {
        x = collide_shapes(0, &bb, boxes, x);
        if x != 0.0 {
            bb = bb.moved([x, 0.0, 0.0]);
        }
    }
    if !z_priority && z != 0.0 {
        z = collide_shapes(2, &bb, boxes, z);
    }
    [x, y, z]
}

// --- ContraptionHandlerClient.rayTraceContraption -----------------------------------------------

/// A ray against the structure's boxes in local space (the boxes of `list`): the first point the
/// ray enters one at, or `from` when it starts inside. `None` for a miss. Stands in for
/// `rayTraceContraption`'s `VoxelShape.clip` over the blocks along the ray.
pub fn ray_boxes(list: &CollisionList, from: Vec3, to: Vec3) -> Option<Vec3> {
    let d = vsub(to, from);
    let mut best: Option<f64> = None;
    for k in 0..list.len() {
        let (c, e) = (list.center[k], list.extents[k]);
        let (mut t_min, mut t_max) = (0.0f64, 1.0f64);
        let mut hit = true;
        for i in 0..3 {
            let (lo, hi) = (c[i] - e[i], c[i] + e[i]);
            if d[i].abs() < 1.0E-12 {
                if from[i] < lo || from[i] > hi {
                    hit = false;
                    break;
                }
            } else {
                let (t0, t1) = ((lo - from[i]) / d[i], (hi - from[i]) / d[i]);
                let (near, far) = if t0 < t1 { (t0, t1) } else { (t1, t0) };
                t_min = t_min.max(near);
                t_max = t_max.min(far);
                if t_min > t_max {
                    hit = false;
                    break;
                }
            }
        }
        if hit && best.is_none_or(|b| t_min < b) {
            best = Some(t_min);
        }
    }
    best.map(|t| vadd(from, [d[0] * t, d[1] * t, d[2] * t]))
}

// --- moving the entities that collided, when a structure is set down ---------------------------------

/// Ticks `collidingEntities` keeps an entity after the last surface collision: the structure's tick
/// does `removeIf(e -> e.getValue().incrementAndGet() > 3)` and `registerColliding` puts a 0.
pub const COLLIDING_TICKS: i64 = 3;

/// `AbstractContraptionEntity.collidingEntities`: the entities that stood on a structure, each with
/// the game time it was last registered at (`registerColliding`).
#[derive(Clone, Debug)]
pub struct Colliding<K> {
    seen: HashMap<K, i64>,
}

impl<K: Copy + Eq + std::hash::Hash> Default for Colliding<K> {
    fn default() -> Self {
        Self { seen: HashMap::new() }
    }
}

impl<K: Copy + Eq + std::hash::Hash> Colliding<K> {
    /// `registerColliding(entity)`: the counter goes back to 0.
    pub fn register(&mut self, entity: K, now: i64) {
        self.seen.insert(entity, now);
    }

    /// The entities still in the map at `now`: a counter that reached 4 is removed.
    pub fn alive(&self, now: i64) -> Vec<K> {
        self.seen.iter().filter(|(_, at)| now - **at <= COLLIDING_TICKS).map(|(k, _)| *k).collect()
    }

    /// Drops what has expired (the structure's per-tick `removeIf`).
    pub fn prune(&mut self, now: i64) {
        self.seen.retain(|_, at| now - *at <= COLLIDING_TICKS);
    }

    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
}

/// `StructureTransform` as a disassembly makes it (`makeStructureTransform`): the cell the anchor
/// lands in, and the right-angle turn about one axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snap {
    /// `BlockPos.containing(getAnchorVec().add(.5, .5, .5))`.
    pub offset: Pos,
    /// `rotationAxis`: `None` when the structure isn't turned.
    pub axis: Option<Axis>,
    /// `angle`, normalised as the constructor does (`%= 360`, then `+360` under -90).
    pub angle: i32,
}

impl Snap {
    /// `new StructureTransform(offset, xRot, yRot, zRot)` for `rotation` degrees about `axis`
    /// (`None` for a structure that only slides).
    pub fn new(position: Vec3, axis: Option<Axis>, rotation: f32) -> Snap {
        let offset = [0, 1, 2].map(|i| (position[i] + 0.5).floor() as i32);
        // `if (xRotation != 0) { rotationAxis = Axis.X; angle = Math.round(xRotation / 90) * 90; }`
        if rotation == 0.0 || axis.is_none() {
            return Snap { offset, axis: None, angle: 0 };
        }
        // `Math.round(float)` is floor(x + 1/2).
        let mut angle = ((f64::from(rotation / 90.0) + 0.5).floor() as i32) * 90;
        // `angle %= 360; if (angle < -90) angle += 360;`
        angle %= 360;
        if angle < -90 {
            angle += 360;
        }
        Snap { offset, axis, angle }
    }

    /// `apply(Vec3 localVec)`: `applyWithoutOffset(localVec).add(offset)`, the turn about the
    /// centre of the cell (`VecHelper.rotateCentered`).
    pub fn apply(&self, local: Vec3) -> Vec3 {
        let v = vsub(local, [0.5; 3]);
        let v = rotate(v, f64::from(self.angle), self.axis);
        let v = vadd(v, [0.5; 3]);
        vadd(v, self.offset.map(f64::from))
    }
}

/// The pose a disassembly works from, given the structure's last pose. A windmill resets its
/// angle before it disassembles (`MechanicalBearingBlockEntity.disassemble`: `angle = 0;` then
/// `applyRotation()` for a windmill), but `prevAngle` is left, and `getAngle(0)` reads that one.
pub fn disassembly_pose(last: &StructurePose, windmill: bool) -> StructurePose {
    let mut pose = *last;
    if windmill && let Rotation::Controlled { axis, prev_angle, .. } = pose.rotation {
        pose.rotation = Rotation::Controlled { axis, angle: 0.0, prev_angle };
    }
    pose
}

/// `makeStructureTransform` for a structure last at `last` (`ControlledContraptionEntity` and
/// `GantryContraptionEntity`: the cell round the anchor, the angle rounded to a right angle).
pub fn disassembly_snap(last: &StructurePose, windmill: bool) -> Snap {
    let pose = disassembly_pose(last, windmill);
    match pose.rotation {
        Rotation::Controlled { axis, angle, .. } => Snap::new(pose.position, axis, angle),
        Rotation::Oriented { .. } => Snap::new(pose.position, None, 0.0),
    }
}

/// `moveCollidedEntitiesOnDisassembly` for one entity at `position`: its place in the structure's
/// own space (`toLocalVector(entity.position(), 0)`) carried through the transform the blocks are
/// set down with, and `transformed.y + 1 / 16f` (a sixteenth up, off the new blocks' top).
pub fn moved_on_disassembly(last: &StructurePose, windmill: bool, position: Vec3) -> Vec3 {
    let pose = disassembly_pose(last, windmill);
    let snap = disassembly_snap(last, windmill);
    let moved = snap.apply(pose.to_local_vector(position, 0.0, false));
    [moved[0], moved[1] + f64::from(1.0f32 / 16.0), moved[2]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::collider::{ColliderFrame, Colliders, EntityBody, PlayerType, collide_entity};
    use crate::rules::portable_interface::rotate_vec;

    fn full(local: Pos, name: &'static str) -> (Pos, &'static str, [Aabb; 1]) {
        (local, name, [Aabb::new([0.0; 3], [1.0; 3])])
    }

    fn solid_of(blocks: &[(Pos, &'static str, [Aabb; 1])], turn: Option<Axis>) -> Solid {
        let list: Vec<SolidBlock<'_>> =
            blocks.iter().map(|(local, name, boxes)| SolidBlock { local: *local, name, boxes }).collect();
        Solid::build(&list, turn)
    }

    #[test]
    fn bounds_are_the_union_of_cells_and_include_the_anchor() {
        let s = solid_of(&[full([2, 0, 0], "minecraft:stone"), full([0, 3, -1], "minecraft:stone")], None);
        assert_eq!(s.local_bounds, Aabb::new([0.0, 0.0, -1.0], [3.0, 4.0, 1.0]));
        assert_eq!(s.list.len(), 2);
    }

    #[test]
    fn a_turning_structure_widens_round_its_axis() {
        // A block 3 from the Y axis: radius 3, so X and Z run -4..5; Y keeps its own extent.
        let s = solid_of(&[full([3, 0, 0], "minecraft:stone")], Some(Axis::Y));
        assert_eq!(s.local_bounds, Aabb::new([-4.0, 0.0, -4.0], [5.0, 1.0, 5.0]));
        // A diagonal block: sqrt(2) rounds up to 2.
        let s = solid_of(&[full([1, 0, 1], "minecraft:stone")], Some(Axis::Y));
        assert_eq!(s.local_bounds, Aabb::new([-3.0, 0.0, -3.0], [4.0, 1.0, 4.0]));
        // About X: the Y and Z distances count.
        let s = solid_of(&[full([0, 0, 5], "minecraft:stone")], Some(Axis::X));
        assert_eq!(s.local_bounds, Aabb::new([0.0, -6.0, -6.0], [1.0, 7.0, 7.0]));
    }

    #[test]
    fn block_facts_by_name() {
        assert!(block_props("minecraft:ladder").climbable);
        assert!(!block_props("create:ladder").climbable);
        assert_eq!(block_props("minecraft:slime_block").bounce, 0.8);
        assert_eq!(block_props("minecraft:slime_block").friction, 0.8);
        assert_eq!(block_props("minecraft:red_bed").bounce, 0.66 * 0.8);
        assert_eq!(block_props("minecraft:blue_ice").friction, 0.989);
        assert_eq!(block_props("minecraft:stone").friction, 0.6);
        assert_eq!(block_props("minecraft:stone").bounce, 0.0);
    }

    #[test]
    fn entity_kinds() {
        assert!(entity_facts("minecraft:zombie").collide.push_reaction_normal);
        assert!(entity_facts("minecraft:item").is_item);
        assert!(entity_facts("minecraft:player").is_player);
        assert!(entity_facts("minecraft:item_frame").collide.is_hanging);
        assert!(entity_facts("minecraft:chest_minecart").collide.is_minecart);
        assert!(entity_facts("minecraft:arrow").collide.is_projectile);
        assert!(!entity_facts("minecraft:marker").collide.push_reaction_normal);
        assert!(entity_facts("create:something").collide.push_reaction_normal);
    }

    #[test]
    fn collide_bounding_box_stops_on_a_floor_and_a_wall() {
        let floor = [Aabb::new([0.0, 0.0, 0.0], [4.0, 1.0, 4.0])];
        let bb = Aabb::new([1.0, 1.5, 1.0], [1.6, 3.3, 1.6]);
        // Falling 1 block lands on the floor after 0.5.
        let out = collide_bounding_box([0.0, -1.0, 0.0], &bb, &floor);
        assert!((out[1] + 0.5).abs() < 1e-12, "{out:?}");
        // Standing on it, walking sideways, it doesn't stop.
        let standing = Aabb::new([1.0, 1.0, 1.0], [1.6, 2.8, 1.6]);
        assert_eq!(collide_bounding_box([0.2, 0.0, 0.1], &standing, &floor), [0.2, 0.0, 0.1]);
        // A wall: walking into it stops at the face.
        let wall = [Aabb::new([2.0, 0.0, 0.0], [3.0, 3.0, 4.0])];
        let out = collide_bounding_box([1.0, 0.0, 0.0], &standing, &wall);
        assert!((out[0] - 0.4).abs() < 1e-12, "{out:?}");
        // Moving along the wall's face doesn't catch on it (touching, not overlapping).
        let beside = Aabb::new([1.4, 1.0, 1.0], [2.0, 2.8, 1.6]);
        assert_eq!(collide_bounding_box([0.0, 0.0, 0.3], &beside, &wall), [0.0, 0.0, 0.3]);
    }

    #[test]
    fn collide_bounding_box_with_no_boxes_is_the_movement() {
        let bb = Aabb::new([0.0; 3], [1.0; 3]);
        assert_eq!(collide_bounding_box([1.0, -2.0, 3.0], &bb, &[]), [1.0, -2.0, 3.0]);
    }

    #[test]
    fn a_ray_down_finds_the_top_of_the_column() {
        let s = solid_of(&[full([0, 0, 0], "minecraft:stone"), full([0, 1, 0], "minecraft:stone")], None);
        let hit = ray_boxes(&s.list, [0.5, 5.0, 0.5], [0.5, -5.0, 0.5]).expect("a hit");
        assert!((hit[1] - 2.0).abs() < 1e-12, "{hit:?}");
        assert_eq!(ray_boxes(&s.list, [3.5, 5.0, 0.5], [3.5, -5.0, 0.5]), None);
        // Starting inside a box: the start is the hit.
        assert_eq!(ray_boxes(&s.list, [0.5, 0.5, 0.5], [0.5, -5.0, 0.5]), Some([0.5, 0.5, 0.5]));
    }

    #[test]
    fn the_collider_rotation_is_the_pose_rotation() {
        // The actors' `Pose::Turning` (`rotate_vec`) and the collider's `VecHelper.rotate` turn the
        // same way, so a structure's boxes and its actors stay together.
        for axis in [Axis::X, Axis::Y, Axis::Z] {
            for degrees in [0.0, 10.0, 90.0, 133.0, -45.0] {
                let v = [1.0, 2.0, -3.0];
                let a = rotate_vec(v, axis, degrees);
                let b = crate::rules::collider::rotate(v, degrees, Some(axis));
                for i in 0..3 {
                    // `Mth.sin` is a table (1/65536 of a turn): close to the exact value, not equal.
                    assert!((a[i] - b[i]).abs() < 1e-3, "{axis:?} {degrees}: {a:?} vs {b:?}");
                }
            }
        }
    }

    #[test]
    fn standing_on_a_built_structure_rises_with_it() {
        // End to end through `Solid`: a 2x1x2 platform rising 0.1 a tick carries a player on it.
        let blocks: Vec<(Pos, &'static str, [Aabb; 1])> =
            [[0, 0, 0], [1, 0, 0], [0, 0, 1], [1, 0, 1]].into_iter().map(|p| full(p, "create:shaft")).collect();
        let s = solid_of(&blocks, None);
        let pose = StructurePose::translating([0.0, 0.1, 0.0], [0.0; 3], [0.0, 0.1, 0.0]);
        let frame = ColliderFrame::new(&pose);
        let mut e = EntityBody::new([1.0, 1.0, 1.0], 0.6, 1.8);
        e.player_type = PlayerType::None;
        e.motion = [0.0, -0.0784, 0.0];
        let mut scratch = CollisionList::new();
        let out = collide_entity(
            &frame,
            &Colliders::Simplified(&s.list),
            &mut scratch,
            &|p| s.props_at(p),
            &mut e,
            &mut |m, _| m,
        );
        assert!(out.registered_colliding && e.on_ground);
        assert!(e.position[1] > 1.09 && e.position[1] < 1.11, "{:?}", e.position);
        assert!(s.world_bounds(&pose).max[1] > 1.0);
    }

    const SIXTEENTH: f64 = 0.0625;

    fn close(a: Vec3, b: Vec3, eps: f64) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < eps)
    }

    #[test]
    fn a_pulley_stopped_below_the_middle_of_a_cell_puts_the_rider_down_with_the_blocks() {
        // The anchor is at y 10.4: the blocks are set down in the cell 10 (floor(10.4 + .5)), 0.4 lower.
        let pose = StructurePose::translating([0.0, 10.4, 0.0], [0.0, 10.4, 0.0], [0.0; 3]);
        let moved = moved_on_disassembly(&pose, false, [0.5, 11.4, 0.5]);
        assert!(close(moved, [0.5, 11.0 + SIXTEENTH, 0.5], 1e-9), "{moved:?}");
    }

    #[test]
    fn a_pulley_stopped_above_the_middle_of_a_cell_lifts_the_rider_with_the_blocks() {
        // 10.6 sets down in the cell 11: the stone the player stands on rises 0.4, so he does too.
        let pose = StructurePose::translating([0.0, 10.6, 0.0], [0.0, 10.6, 0.0], [0.0; 3]);
        let moved = moved_on_disassembly(&pose, false, [0.5, 11.6, 0.5]);
        assert!(close(moved, [0.5, 12.0 + SIXTEENTH, 0.5], 1e-9), "{moved:?}");
    }

    #[test]
    fn a_sliding_structure_moves_the_entity_only_along_its_own_axis() {
        // A piston going east: the anchor x is 5.3, the cell 5.
        let pose = StructurePose::translating([5.3, 7.0, -2.0], [5.3, 7.0, -2.0], [0.0; 3]);
        let moved = moved_on_disassembly(&pose, false, [6.1, 8.0, -1.5]);
        assert!(close(moved, [5.8, 8.0 + SIXTEENTH, -1.5], 1e-9), "{moved:?}");
        // At rest on a cell: only the sixteenth.
        let pose = StructurePose::translating([5.0, 7.0, -2.0], [5.0, 7.0, -2.0], [0.0; 3]);
        let moved = moved_on_disassembly(&pose, false, [6.1, 8.0, -1.5]);
        assert!(close(moved, [6.1, 8.0 + SIXTEENTH, -1.5], 1e-9), "{moved:?}");
    }

    #[test]
    fn the_angle_rounds_as_math_round_does() {
        let at = |deg: f32| Snap::new([0.0; 3], Some(Axis::Y), deg).angle;
        assert_eq!(at(30.0), 0);
        assert_eq!(at(45.0), 90, "Math.round(.5) is 1");
        assert_eq!(at(50.0), 90);
        assert_eq!(at(-45.0), 0, "Math.round(-.5) is 0");
        assert_eq!(at(-50.0), -90);
        assert_eq!(at(280.0), 270);
        assert_eq!(at(350.0), 0);
        assert_eq!(at(-170.0), 180, "-180 is under -90: plus 360");
        assert_eq!(Snap::new([0.0; 3], Some(Axis::Y), 0.0).axis, None, "no turn, no axis");
        assert_eq!(Snap::new([2.5, 3.49, -0.5], None, 0.0).offset, [3, 3, 0]);
    }

    #[test]
    fn a_bearing_stopped_at_thirty_degrees_turns_the_rider_back_with_the_blocks() {
        // Up bearing, anchor cell (0, 0, 0), turned 30 degrees (last tick 30 too): the blocks go back to
        // 0. An entity one block east of the axis, half a block up, is turned back by the same 30 degrees.
        let pose = StructurePose::turning([0.0; 3], Axis::Y, 30.0, 30.0);
        let moved = moved_on_disassembly(&pose, false, [1.5, 0.5, 0.5]);
        let (s, c) = 30.0f64.to_radians().sin_cos();
        assert!(close(moved, [0.5 + c, 0.5 + SIXTEENTH, 0.5 + s], 1e-3), "{moved:?}");
    }

    #[test]
    fn a_bearing_stopped_past_the_middle_of_a_quarter_turns_on_to_the_next_one() {
        // 50 degrees sets down at 90: the entity ends 40 degrees further round, still on its circle.
        let pose = StructurePose::turning([0.0; 3], Axis::Y, 50.0, 50.0);
        let moved = moved_on_disassembly(&pose, false, [0.5 + 2.0, 0.5, 0.5]);
        let (s, c) = 40.0f64.to_radians().sin_cos();
        assert!(close(moved, [0.5 + 2.0 * c, 0.5 + SIXTEENTH, 0.5 - 2.0 * s], 1e-3), "{moved:?}");
        // On the axis it only rises a sixteenth.
        let moved = moved_on_disassembly(&pose, false, [0.5, 3.0, 0.5]);
        assert!(close(moved, [0.5, 3.0 + SIXTEENTH, 0.5], 1e-9), "{moved:?}");
    }

    #[test]
    fn the_anchor_cell_is_the_centre_of_the_turn() {
        // The anchor at (4, 5, 6): the same turn about that cell's centre.
        let pose = StructurePose::turning([4.0, 5.0, 6.0], Axis::Y, 90.0, 90.0);
        let moved = moved_on_disassembly(&pose, false, [4.5, 5.5, 6.5]);
        assert!(close(moved, [4.5, 5.5 + SIXTEENTH, 6.5], 1e-9), "{moved:?}");
        // Stopped exactly on a quarter, nothing but the sixteenth moves, wherever the entity is.
        let at_90 = disassembly_snap(&pose, false);
        assert_eq!((at_90.angle, at_90.axis, at_90.offset), (90, Some(Axis::Y), [4, 5, 6]));
        let moved = moved_on_disassembly(&pose, false, [4.5, 5.5, 4.5]);
        assert!(close(moved, [4.5, 5.5 + SIXTEENTH, 4.5], 1e-3), "{moved:?}");
    }

    #[test]
    fn a_windmill_goes_back_to_its_start_whatever_it_read() {
        // `angle = 0` is set first, but `getAngle(0)` is `prevAngle`: the entity is taken back to the
        // blocks' initial frame (the sails are set down unturned).
        let pose = StructurePose::turning([0.0; 3], Axis::Y, 100.0, 99.0);
        assert_eq!(disassembly_snap(&pose, true).angle, 0);
        assert_eq!(disassembly_snap(&pose, false).angle, 90);
        let moved = moved_on_disassembly(&pose, true, [1.5, 0.5, 0.5]);
        let (s, c) = 99.0f64.to_radians().sin_cos();
        // rotate(+x, -99): x = cos, z = sin
        assert!(close(moved, [0.5 + c, 0.5 + SIXTEENTH, 0.5 + s], 1e-3), "{moved:?}");
    }

    #[test]
    fn the_colliding_map_keeps_an_entity_three_ticks() {
        let mut map: Colliding<u32> = Colliding::default();
        assert!(map.is_empty());
        map.register(7, 100);
        map.register(8, 102);
        let mut alive = map.alive(103);
        alive.sort();
        assert_eq!(alive, vec![7, 8], "the counter is 3 at the third tick");
        assert_eq!(map.alive(104), vec![8], "4 is removed");
        // Registered again, the counter starts over.
        map.register(7, 104);
        assert_eq!(map.alive(107), vec![7], "8 expired at 106");
        map.prune(107);
        assert_eq!(map.alive(107), vec![7]);
        map.prune(108);
        assert!(map.is_empty());
    }
}
