//! Collision between entities and moving structures, as pure data in and data out
//! (`ContraptionCollider.collideEntities`, the parts of `AbstractContraptionEntity`,
//! `ControlledContraptionEntity` and `OrientedContraptionEntity` it calls, and the pose maths:
//! `getRotationState`, `applyRotation`, `reverseRotation`, `toGlobalVector`, `toLocalVector`,
//! `getContactPointMotion`, `getPrevPositionVec`). The box maths is in `collider_obb.rs`.
//!
//! Create runs this every tick from the structure's side: for each entity near the structure it
//! moves the entity into the structure's local space, collides the entity's box with the
//! structure's collision boxes (`ContinuousOBBCollider.collideMany`), then pushes the entity out,
//! stops its motion into the structure, marks it standing and carries it with the structure's
//! motion. [`collide_entity`] is one pass of that loop body for one entity; [`collide_entities`] is
//! the loop; [`broad_phase`] and [`can_collide_with`] are the entity query that feeds it.
//!
//! Where Create reads Minecraft the caller fills a plain value in:
//! - the entity: [`EntityBody`] (position, last position, box, motion, `maxUpStep`, `onGround`, ...),
//!   which [`collide_entity`] changes the way Create changes the `Entity` (position, motion,
//!   `onGround`, `fallDistance`, `hurtMarked`); what else Create does is in [`EntityOutcome`];
//! - the structure: [`StructurePose`] (position now and last tick, rotation now and last tick,
//!   `getDeltaMovement`, whether it is a `TranslatingContraption`) and its collision boxes as a
//!   [`CollisionList`] (`Contraption.getSimplifiedEntityColliders`; build it once with
//!   `CollisionList::from_blocks` and keep it while the structure's blocks don't change);
//! - a block of the structure: [`BlockProps`] (climbable, bounce, friction, has a moving
//!   interaction), looked up by local position;
//! - the world: a closure for `Entity.collideBoundingBox` (the vanilla collision of a box moving by
//!   a vector against the blocks and entity shapes around it), which `ContraptionCollider.collide`
//!   calls to keep a pushed or carried entity out of the world's blocks.
//!
//! Left out (see the doc of each): the train's run-over damage (`handleDamageFromTrain`, only for
//! `CarriageContraptionEntity`), the remote player's clipping rescue, the packets and sounds (the
//! outcome says which are due), `collideBlocks` / `isCollidingWithWorld` (the structure against
//! the world, not the entity side).

use super::actor::Vec3;
use super::collider_obb::{
    Aabb, CollisionList, Matrix3d, OrientedBB, block_containing, collide_many, is_zero_vec, mth_cos, mth_sin,
    vadd, vcross, vdot, vhorizontal_sqr, vlength, vlength_sqr, vmul, vnormalize, vscale, vsub,
};
use crate::{Axis, Pos};

// --- rotation --------------------------------------------------------------------------------

/// `AngleHelper.rad`: degrees to radians as a float, 0 for 0.
fn rad(degrees: f32) -> f32 {
    if degrees == 0.0 {
        return 0.0;
    }
    (f64::from(degrees) / 180.0 * std::f64::consts::PI) as f32
}

/// `AngleHelper.getShortestAngleDiff`.
fn shortest_angle_diff(current: f32, target: f32) -> f32 {
    let (c, t) = (current % 360.0, target % 360.0);
    (((t - c) % 360.0 + 540.0) % 360.0) - 180.0
}

/// `AngleHelper.angleLerp`.
fn angle_lerp(pct: f32, current: f32, target: f32) -> f32 {
    current + shortest_angle_diff(current, target) * pct
}

/// `VecHelper.rotate(vec, deg, axis)`: a turn of `degrees` about an axis (the vector as it is for
/// no axis or no angle). Catnip's source is not in the local clone; this is the formula the crate's
/// other ports of it use (`arm.rs`), with `Mth`'s sine table.
pub fn rotate(v: Vec3, degrees: f64, axis: Option<Axis>) -> Vec3 {
    if degrees == 0.0 {
        return v;
    }
    let Some(axis) = axis else { return v };
    let angle = (degrees / 180.0 * std::f64::consts::PI) as f32;
    let sin = f64::from(mth_sin(angle));
    let cos = f64::from(mth_cos(angle));
    let [x, y, z] = v;
    match axis {
        Axis::X => [x, y * cos - z * sin, z * cos + y * sin],
        Axis::Y => [x * cos + z * sin, y, z * cos - x * sin],
        Axis::Z => [x * cos - y * sin, y * cos + x * sin, z],
    }
}

/// `VecHelper.CENTER_OF_ORIGIN`.
const CENTER_OF_ORIGIN: Vec3 = [0.5, 0.5, 0.5];

/// `ContraptionRotationState`: the structure's turn as angles in degrees about each axis, the
/// turn about Y that comes after the others (`secondYRotation`) and the matrix the collider uses.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RotationState {
    pub x_rotation: f32,
    pub y_rotation: f32,
    pub z_rotation: f32,
    pub second_y_rotation: f32,
}

impl RotationState {
    /// `ContraptionRotationState.NONE`.
    pub const NONE: RotationState = RotationState { x_rotation: 0.0, y_rotation: 0.0, z_rotation: 0.0, second_y_rotation: 0.0 };

    /// `ContraptionRotationState.asMatrix`.
    pub fn as_matrix(&self) -> Matrix3d {
        let mut matrix = Matrix3d::identity();
        if self.x_rotation != 0.0 {
            matrix.multiply(&Matrix3d::x_rotation(rad(-self.x_rotation)));
        }
        if self.y_rotation != 0.0 {
            matrix.multiply(&Matrix3d::y_rotation(rad(-self.y_rotation)));
        }
        if self.z_rotation != 0.0 {
            matrix.multiply(&Matrix3d::z_rotation(rad(-self.z_rotation)));
        }
        matrix
    }

    /// `ContraptionRotationState.hasVerticalRotation`.
    pub fn has_vertical_rotation(&self) -> bool {
        self.x_rotation != 0.0 || self.z_rotation != 0.0
    }

    /// `ContraptionRotationState.getYawOffset`.
    pub fn yaw_offset(&self) -> f32 {
        self.second_y_rotation
    }
}

/// How a structure is turned: the two kinds of contraption entity Create has.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rotation {
    /// `ControlledContraptionEntity`: pistons, bearings, pulleys, gantries. A turn of `angle`
    /// degrees about `axis` (`None` for a structure that only translates), `prev_angle` last tick
    /// (`prevAngle`, which Create sets to `angle` at the start of the entity's tick).
    Controlled { axis: Option<Axis>, angle: f32, prev_angle: f32 },
    /// `OrientedContraptionEntity`: carts and trains. `initial_yaw` is `getInitialYaw`
    /// (the initial orientation's `toYRot`), `yaw` and `pitch` the turn in degrees and
    /// `prev_yaw` / `prev_pitch` last tick's.
    Oriented { initial_yaw: f32, pitch: f32, yaw: f32, prev_pitch: f32, prev_yaw: f32 },
}

/// `AbstractContraptionEntity`'s state the collider reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StructurePose {
    /// `position()`.
    pub position: Vec3,
    /// `xo, yo, zo`: the position last tick.
    pub prev_position: Vec3,
    /// `prevPosInvalid`: true until the entity's first tick (the previous position is then its
    /// position, and the contact point motion zero).
    pub prev_pos_invalid: bool,
    /// `getDeltaMovement()`: what a piston or pulley sets with `setContraptionMotion`.
    pub delta_movement: Vec3,
    pub rotation: Rotation,
    /// `contraption instanceof TranslatingContraption` (pistons, pulleys, gantries, elevators).
    pub translating: bool,
}

impl StructurePose {
    /// A structure that only translates (a piston, pulley or gantry).
    pub fn translating(position: Vec3, prev_position: Vec3, delta_movement: Vec3) -> Self {
        Self {
            position,
            prev_position,
            prev_pos_invalid: false,
            delta_movement,
            rotation: Rotation::Controlled { axis: None, angle: 0.0, prev_angle: 0.0 },
            translating: true,
        }
    }

    /// A structure that turns about an axis (a bearing).
    pub fn turning(position: Vec3, axis: Axis, angle: f32, prev_angle: f32) -> Self {
        Self {
            position,
            prev_position: position,
            prev_pos_invalid: false,
            delta_movement: [0.0; 3],
            rotation: Rotation::Controlled { axis: Some(axis), angle, prev_angle },
            translating: false,
        }
    }

    /// `getPrevPositionVec`.
    pub fn prev_position_vec(&self) -> Vec3 {
        if self.prev_pos_invalid { self.position } else { self.prev_position }
    }

    /// `getAnchorVec`: the position, less half a block in X and Z for an oriented structure.
    pub fn anchor_vec(&self) -> Vec3 {
        match self.rotation {
            Rotation::Controlled { .. } => self.position,
            Rotation::Oriented { .. } => vsub(self.position, [0.5, 0.0, 0.5]),
        }
    }

    /// `getPrevAnchorVec`.
    pub fn prev_anchor_vec(&self) -> Vec3 {
        match self.rotation {
            Rotation::Controlled { .. } => self.prev_position_vec(),
            Rotation::Oriented { .. } => vsub(self.prev_position_vec(), [0.5, 0.0, 0.5]),
        }
    }

    /// The entity's bounding box (`setPos`: `contraption.bounds.move(getAnchorVec())`) given the
    /// structure's local bounds (`Contraption.bounds`).
    pub fn bounding_box(&self, local_bounds: &Aabb) -> Aabb {
        local_bounds.moved(self.anchor_vec())
    }

    /// `position().subtract(getPrevPositionVec())`: how far the structure moved this tick.
    pub fn motion(&self) -> Vec3 {
        vsub(self.position, self.prev_position_vec())
    }

    /// `getRotationState`.
    pub fn rotation_state(&self) -> RotationState {
        match self.rotation {
            Rotation::Controlled { axis, angle, .. } => {
                let mut crs = RotationState::NONE;
                match axis {
                    Some(Axis::X) => crs.x_rotation = angle,
                    Some(Axis::Y) => crs.y_rotation = angle,
                    Some(Axis::Z) => crs.z_rotation = angle,
                    None => {}
                }
                crs
            }
            Rotation::Oriented { initial_yaw, pitch, yaw, .. } => {
                let mut crs = RotationState::NONE;
                let yaw_offset = initial_yaw;
                crs.z_rotation = pitch;
                crs.y_rotation = -yaw + yaw_offset;
                if pitch != 0.0 && yaw != 0.0 {
                    crs.second_y_rotation = -yaw;
                    crs.y_rotation = yaw_offset;
                }
                crs
            }
        }
    }

    /// `ControlledContraptionEntity.getAngle(partialTicks)`.
    fn controlled_angle(angle: f32, prev_angle: f32, partial_ticks: f32) -> f32 {
        if partial_ticks == 1.0 { angle } else { angle_lerp(partial_ticks, prev_angle, angle) }
    }

    /// `OrientedContraptionEntity.getViewYRot`.
    fn view_y_rot(yaw: f32, prev_yaw: f32, partial_ticks: f32) -> f32 {
        -(if partial_ticks == 1.0 { yaw } else { angle_lerp(partial_ticks, prev_yaw, yaw) })
    }

    /// `OrientedContraptionEntity.getViewXRot`.
    fn view_x_rot(pitch: f32, prev_pitch: f32, partial_ticks: f32) -> f32 {
        if partial_ticks == 1.0 { pitch } else { angle_lerp(partial_ticks, prev_pitch, pitch) }
    }

    /// `applyRotation(localPos, partialTicks)`.
    pub fn apply_rotation(&self, local: Vec3, partial_ticks: f32) -> Vec3 {
        match self.rotation {
            Rotation::Controlled { axis, angle, prev_angle } => {
                rotate(local, f64::from(Self::controlled_angle(angle, prev_angle, partial_ticks)), axis)
            }
            Rotation::Oriented { initial_yaw, pitch, yaw, prev_pitch, prev_yaw } => {
                let mut v = rotate(local, f64::from(initial_yaw), Some(Axis::Y));
                v = rotate(v, f64::from(Self::view_x_rot(pitch, prev_pitch, partial_ticks)), Some(Axis::Z));
                rotate(v, f64::from(Self::view_y_rot(yaw, prev_yaw, partial_ticks)), Some(Axis::Y))
            }
        }
    }

    /// `reverseRotation(localPos, partialTicks)`.
    pub fn reverse_rotation(&self, local: Vec3, partial_ticks: f32) -> Vec3 {
        match self.rotation {
            Rotation::Controlled { axis, angle, prev_angle } => {
                rotate(local, f64::from(-Self::controlled_angle(angle, prev_angle, partial_ticks)), axis)
            }
            Rotation::Oriented { initial_yaw, pitch, yaw, prev_pitch, prev_yaw } => {
                let mut v = rotate(local, f64::from(-Self::view_y_rot(yaw, prev_yaw, partial_ticks)), Some(Axis::Y));
                v = rotate(v, f64::from(-Self::view_x_rot(pitch, prev_pitch, partial_ticks)), Some(Axis::Z));
                rotate(v, f64::from(-initial_yaw), Some(Axis::Y))
            }
        }
    }

    /// `toGlobalVector(localVec, partialTicks, prevAnchor)`: a point in the structure's local
    /// space to world space. `partial_ticks` is 0 for last tick's turn, 1 for this tick's.
    pub fn to_global_vector(&self, local: Vec3, partial_ticks: f32, prev_anchor: bool) -> Vec3 {
        let anchor = if prev_anchor { self.prev_anchor_vec() } else { self.anchor_vec() };
        let rotation_offset = CENTER_OF_ORIGIN;
        let v = vsub(local, rotation_offset);
        let v = self.apply_rotation(v, partial_ticks);
        vadd(vadd(v, rotation_offset), anchor)
    }

    /// `toLocalVector(globalVec, partialTicks, prevAnchor)`: a world point in the structure's
    /// local space.
    pub fn to_local_vector(&self, global: Vec3, partial_ticks: f32, prev_anchor: bool) -> Vec3 {
        let anchor = if prev_anchor { self.prev_anchor_vec() } else { self.anchor_vec() };
        let rotation_offset = CENTER_OF_ORIGIN;
        let v = vsub(vsub(global, anchor), rotation_offset);
        let v = self.reverse_rotation(v, partial_ticks);
        vadd(v, rotation_offset)
    }

    /// `getContactPointMotion(globalContactPoint)`: how far the structure carries a point at rest
    /// on it this tick. A translating `ControlledContraptionEntity` answers with its
    /// `getDeltaMovement`; the others with the point's move from last tick's pose to this tick's
    /// (the turn included) plus the anchor's move. Zero before the first tick.
    pub fn contact_point_motion(&self, global_contact_point: Vec3) -> Vec3 {
        if matches!(self.rotation, Rotation::Controlled { .. }) && self.translating {
            return self.delta_movement;
        }
        if self.prev_pos_invalid {
            return [0.0; 3];
        }
        let contact_point = self.to_global_vector(self.to_local_vector(global_contact_point, 0.0, true), 1.0, true);
        let contraption_local_movement = vsub(contact_point, global_contact_point);
        let contraption_anchor_movement = vsub(self.position, self.prev_position_vec());
        vadd(contraption_local_movement, contraption_anchor_movement)
    }
}

// --- the structure's frame for a tick --------------------------------------------------------

/// What `collideEntities` works out once for a structure before its entity loop: the anchor, the
/// structure's motion, the rotation state and its matrix.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColliderFrame {
    pub pose: StructurePose,
    /// `contraptionEntity.getAnchorVec()`.
    pub anchor_vec: Vec3,
    /// `contraptionPosition.subtract(getPrevPositionVec())`.
    pub contraption_motion: Vec3,
    pub rotation: RotationState,
    pub matrix: Matrix3d,
}

impl ColliderFrame {
    pub fn new(pose: &StructurePose) -> Self {
        let rotation = pose.rotation_state();
        Self {
            pose: *pose,
            anchor_vec: pose.anchor_vec(),
            contraption_motion: pose.motion(),
            rotation,
            matrix: rotation.as_matrix(),
        }
    }
}

/// `ContraptionCollider.worldToLocalPos(worldPos, anchorVec, rotationMatrix, yawOffset)`.
pub fn world_to_local_pos(world_pos: Vec3, anchor_vec: Vec3, rotation_matrix: &Matrix3d, yaw_offset: f32) -> Vec3 {
    let mut local = world_pos;
    local = vsub(local, anchor_vec);
    local = vsub(local, CENTER_OF_ORIGIN);
    local = rotate(local, f64::from(-yaw_offset), Some(Axis::Y));
    local = rotation_matrix.transform(local);
    vadd(local, CENTER_OF_ORIGIN)
}

/// `ContraptionCollider.getWorldToLocalTranslation(entity, anchorVec, rotationMatrix, yawOffset)`:
/// how far the entity's box moves to go into local space, turning about the middle of its height.
pub fn get_world_to_local_translation(
    entity_position: Vec3,
    entity_box: &Aabb,
    anchor_vec: Vec3,
    rotation_matrix: &Matrix3d,
    yaw_offset: f32,
) -> Vec3 {
    let center_y = [0.0, entity_box.y_size() / 2.0, 0.0];
    let mut position = entity_position;
    position = vadd(position, center_y);
    position = world_to_local_pos(position, anchor_vec, rotation_matrix, yaw_offset);
    position = vsub(position, center_y);
    vsub(position, entity_position)
}

// --- the entity side -------------------------------------------------------------------------

/// `ContraptionCollider.PlayerType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerType {
    /// Not a player.
    None,
    /// The local player on a client (`LocalPlayer`).
    Client,
    /// Another player seen on a client (`RemotePlayer`).
    Remote,
    /// A player on the server (`ServerPlayer`).
    Server,
}

/// An entity as `collideEntities` sees and changes it. The caller fills it from the game, calls
/// the collider, and writes `position`, `motion`, `on_ground`, `fall_distance` and `hurt_marked`
/// back (setting `position` moves the box with it, `set_pos`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityBody {
    /// `position()`.
    pub position: Vec3,
    /// `xo, yo, zo`.
    pub prev_position: Vec3,
    /// `getBoundingBox()`.
    pub bounding_box: Aabb,
    /// `getDeltaMovement()`.
    pub motion: Vec3,
    /// `maxUpStep()`.
    pub max_up_step: f32,
    /// `onGround()`.
    pub on_ground: bool,
    /// `fallDistance`.
    pub fall_distance: f64,
    /// `hurtMarked`.
    pub hurt_marked: bool,
    pub player_type: PlayerType,
    /// `entity instanceof ItemEntity`.
    pub is_item: bool,
    /// `isSuppressingBounce()`.
    pub suppresses_bounce: bool,
    /// Whether one of `getIndirectPassengers()` is the client's player.
    pub has_client_player_rider: bool,
}

impl EntityBody {
    /// A plain entity: box `width` x `height` standing at `position`, at rest, not a player.
    pub fn new(position: Vec3, width: f64, height: f64) -> Self {
        let half = width / 2.0;
        Self {
            position,
            prev_position: position,
            bounding_box: Aabb::new(
                [position[0] - half, position[1], position[2] - half],
                [position[0] + half, position[1] + height, position[2] + half],
            ),
            motion: [0.0; 3],
            max_up_step: 0.6,
            on_ground: false,
            fall_distance: 0.0,
            hurt_marked: false,
            player_type: PlayerType::None,
            is_item: false,
            suppresses_bounce: false,
            has_client_player_rider: false,
        }
    }

    /// `Entity.setPos`: the box goes with the position.
    pub fn set_pos(&mut self, pos: Vec3) {
        let by = vsub(pos, self.position);
        self.bounding_box = self.bounding_box.moved(by);
        self.position = pos;
    }
}

/// A block of the structure, as the collider asks about it (by local position).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BlockProps {
    /// `blockState.is(BlockTags.CLIMBABLE)`.
    pub climbable: bool,
    /// `BlockHelper.getBounceMultiplier(block)`: see [`bounce_multiplier`].
    pub bounce: f64,
    /// `blockState.getFriction(...)`: 0.6 for most blocks, 0.8 slime, 0.98 ice.
    pub friction: f32,
    /// The structure has a `MovingInteractionBehaviour` here (`contraption.interactors`).
    pub has_interactor: bool,
}

/// `BlockHelper.getBounceMultiplier`: 0.8 for a slime block, 0.66 * 0.8 for a bed, else 0.
pub fn bounce_multiplier(is_slime_block: bool, is_bed: bool) -> f64 {
    if is_slime_block {
        0.8
    } else if is_bed {
        0.66 * 0.8
    } else {
        0.0
    }
}

/// Where the collision boxes come from.
pub enum Colliders<'a> {
    /// `contraption.getSimplifiedEntityColliders()`: every box of the structure, cached.
    Simplified(&'a CollisionList),
    /// `null` from it (a carriage inside a portal): `getPotentiallyCollidedShapes` looks the blocks
    /// near the entity up one by one. The closure gives the collision boxes (relative to the block)
    /// of the structure's block at a local position: none for no block, and none for one that
    /// `isHiddenInPortal`.
    Nearby(&'a dyn Fn(Pos) -> Vec<Aabb>),
}

/// `ContraptionCollider.getPotentiallyCollidedShapes`: the boxes of the blocks around `local_bb`
/// (already grown by the motion), in local space.
pub fn potentially_collided_shapes(shape_at: &dyn Fn(Pos) -> Vec<Aabb>, local_bb: &Aabb) -> CollisionList {
    let height = local_bb.y_size();
    let width = local_bb.x_size();
    let horizontal_factor = if height > width && width != 0.0 { height / width } else { 1.0 };
    let vertical_factor = if width > height && height != 0.0 { width / height } else { 1.0 };
    let block_scan_bb = local_bb.inflate(f64::from(0.5f32)).inflate_xyz(horizontal_factor, vertical_factor, horizontal_factor);

    let min = block_containing(block_scan_bb.min);
    let max = block_containing(block_scan_bb.max);

    let mut out = CollisionList::new();
    // `BlockPos.betweenClosed`: X changes fastest, then Y, then Z.
    for z in min[2]..=max[2] {
        for y in min[1]..=max[1] {
            for x in min[0]..=max[0] {
                for b in shape_at([x, y, z]) {
                    out.append_box([x, y, z], b.min, b.max);
                }
            }
        }
    }
    out
}

/// What `collideEntities` did besides changing the entity.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EntityOutcome {
    /// This entity isn't collided here (a server player or a remote player): Create skips it.
    pub skipped: bool,
    /// `((ServerPlayer) e).connection.aboveGroundTickCount = 0` for the entity and its passengers
    /// (done for every entity but a remote player, before the server-player skip).
    pub reset_above_ground_tick_count: bool,
    /// The collision changed anything (`false` where Create `continue`s: no hard collision and no
    /// surface, or a bounce, or a skip).
    pub collided: bool,
    /// The entity bounced off a slime block or bed: play `SLIME_BLOCK_FALL` (volume .5, pitch 1),
    /// for the local player to that player only.
    pub bounced: bool,
    /// `contraptionEntity.registerColliding(entity)`: the entity stands on the structure.
    pub registered_colliding: bool,
    /// The entity's `fallDistance` was reset.
    pub fall_distance_reset: bool,
    /// `entity.setOnGround(true)`.
    pub set_on_ground: bool,
    /// An item's horizontal motion was halved (`ItemEntity` standing on it).
    pub item_slowed: bool,
    /// The local player rides this entity: send `ClientMotionPacket(rider.getDeltaMovement(), true, 0)`
    /// for each such rider.
    pub notify_client_riders: bool,
    /// `getContactPointMotion(entityPosition)`: the carry from standing on it (zero off the
    /// structure). Create has moved the entity by it already (horizontally, through `collide`).
    pub contact_point_motion: Vec3,
    /// How far the entity was moved by the collider, `position` after less `position` before.
    pub position_correction: Vec3,
    /// For the local player: send `ClientMotionPacket(motion, true, limbSwing)`.
    pub client_motion: Option<ClientMotion>,
    /// For the local player standing on a translating structure: the safety lock
    /// (`safetyLock.right`, `entity.getY() - contraptionEntity.getY()`).
    pub safety_lock: Option<f64>,
    /// `movingInteractionBehaviour.handleEntityCollision(entity, pos, contraptionEntity)` is due
    /// for the block at this local position.
    pub interaction_at: Option<Pos>,
}

/// `new ClientMotionPacket(motion, onGround, limbSwing)`'s data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClientMotion {
    pub motion: Vec3,
    pub limb_swing: f32,
}

/// `ContraptionCollider.collide(motion, entity)`: `Entity.collide`'s copy, with step-up. `collide_bb`
/// is `Entity.collideBoundingBox(entity, motion, box, level, list)`: how far a box can move by a
/// vector before the world's blocks and entity shapes stop it. Returns the move that is allowed.
pub fn collide(movement: Vec3, max_up_step: f32, aabb: &Aabb, collide_bb: &mut dyn FnMut(Vec3, &Aabb) -> Vec3) -> Vec3 {
    let max_step = f64::from(max_up_step);
    let vec3 = if vlength_sqr(movement) == 0.0 { movement } else { collide_bb(movement, aabb) };
    let flag = movement[0] != vec3[0];
    let flag1 = movement[1] != vec3[1];
    let flag2 = movement[2] != vec3[2];
    let flag3 = flag1 && movement[1] < 0.0;
    if max_up_step > 0.0 && flag3 && (flag || flag2) {
        let mut vec31 = collide_bb([movement[0], max_step, movement[2]], aabb);
        let vec32 = collide_bb([0.0, max_step, 0.0], &aabb.expand_towards([movement[0], 0.0, movement[2]]));
        if vec32[1] < max_step {
            let vec33 = vadd(collide_bb([movement[0], 0.0, movement[2]], &aabb.moved(vec32)), vec32);
            if vhorizontal_sqr(vec33) > vhorizontal_sqr(vec31) {
                vec31 = vec33;
            }
        }

        if vhorizontal_sqr(vec31) > vhorizontal_sqr(vec3) {
            return vadd(vec31, collide_bb([0.0, -vec31[1] + movement[1], 0.0], &aabb.moved(vec31)));
        }
    }

    vec3
}

/// `ContraptionCollider.bounceEntity`: bounce the entity off a bouncy block (the normal points out
/// of the block). Changes the entity's motion and returns whether it bounced.
pub fn bounce_entity(entity: &mut EntityBody, normal: Vec3, pose: &StructurePose, factor: f64) -> bool {
    if factor == 0.0 {
        return false;
    }
    if entity.suppresses_bounce {
        return false;
    }

    let contact_point_motion = pose.contact_point_motion(entity.position);
    let motion = vsub(entity.motion, contact_point_motion);
    let deltav = vscale(normal, factor * 2.0 * vdot(motion, normal));
    if vdot(deltav, deltav) < f64::from(0.1f32) {
        return false;
    }
    entity.motion = vsub(entity.motion, deltav);
    true
}

/// `AbstractContraptionEntity.canCollideWith`'s inputs about an entity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CollideFacts {
    pub is_player: bool,
    pub is_spectator: bool,
    /// `noPhysics`.
    pub no_physics: bool,
    /// `HangingEntity` (item frames, paintings).
    pub is_hanging: bool,
    /// `AbstractMinecart`.
    pub is_minecart: bool,
    pub is_super_glue: bool,
    /// `SeatEntity`.
    pub is_seat: bool,
    pub is_projectile: bool,
    /// `getVehicle() != null`.
    pub has_vehicle: bool,
    /// The entity is the structure's vehicle or one further up its vehicle chain.
    pub is_vehicle_of_structure: bool,
    /// `getPistonPushReaction() == PushReaction.NORMAL`.
    pub push_reaction_normal: bool,
}

/// `AbstractContraptionEntity.canCollideWith(entity)`. `structure_is_mounted` is
/// `contraption instanceof MountedContraption` (a cart assembler's structure).
pub fn can_collide_with(e: &CollideFacts, structure_is_mounted: bool) -> bool {
    if e.is_player && e.is_spectator {
        return false;
    }
    if e.no_physics {
        return false;
    }
    if e.is_hanging {
        return false;
    }
    if e.is_minecart {
        return !structure_is_mounted;
    }
    if e.is_super_glue {
        return false;
    }
    if e.is_seat {
        return false;
    }
    if e.is_projectile {
        return false;
    }
    if e.has_vehicle {
        return false;
    }
    if e.is_vehicle_of_structure {
        return false;
    }
    e.push_reaction_normal
}

/// The broad phase of `collideEntities`: the box Create asks the world for entities in
/// (`bounds.inflate(2).expandTowards(0, 32, 0)`, `bounds` being the structure's bounding box,
/// `StructurePose::bounding_box`).
pub fn broad_phase_area(structure_bounds: &Aabb) -> Aabb {
    structure_bounds.inflate(2.0).expand_towards([0.0, 32.0, 0.0])
}

/// Whether an entity's box is within `broad_phase_area` (`getEntitiesOfClass`'s box test), so the
/// glue can skip every other entity.
pub fn broad_phase(structure_bounds: &Aabb, entity_box: &Aabb) -> bool {
    broad_phase_area(structure_bounds).intersects(entity_box)
}

/// `ContraptionCollider.collideEntities` for one entity: the body of Create's loop. `blocks` looks
/// a block of the structure up by local position (`contraption.getBlocks()`); `scratch` is the
/// reusable list `collideMany` filters into (`denseViableColliders`); `world_collide` is
/// `Entity.collideBoundingBox` for this entity.
///
/// The caller has already filtered the entity as Create's loop does (`canCollideWith`, alive, not
/// frozen) and, for the local player, skipped a second reference to it (`skipClientPlayer`;
/// [`collide_entities`] does that). Changes `entity` as Create changes the `Entity`; the rest is in
/// the outcome.
pub fn collide_entity(
    frame: &ColliderFrame,
    colliders: &Colliders,
    scratch: &mut CollisionList,
    blocks: &dyn Fn(Pos) -> Option<BlockProps>,
    entity: &mut EntityBody,
    world_collide: &mut dyn FnMut(Vec3, &Aabb) -> Vec3,
) -> EntityOutcome {
    let mut out = EntityOutcome::default();
    let player_type = entity.player_type;
    let pose = &frame.pose;
    let contraption_motion = frame.contraption_motion;
    let rotation = &frame.rotation;
    let rotation_matrix = &frame.matrix;

    if player_type == PlayerType::Remote {
        // Create returns here for everything but a translating structure, where the client
        // rescues the remote player from clipping (`saveRemotePlayerFromClipping`); no collision.
        out.skipped = true;
        return out;
    }

    out.reset_above_ground_tick_count = true;

    if player_type == PlayerType::Server {
        out.skipped = true;
        return out;
    }

    // Transform entity position and motion to local space
    let start_position = entity.position;
    let mut entity_position = entity.position;
    let mut entity_bounds = entity.bounding_box;
    let mut motion = entity.motion;
    let yaw_offset = rotation.yaw_offset();
    let position = get_world_to_local_translation(entity.position, &entity.bounding_box, frame.anchor_vec, rotation_matrix, yaw_offset);

    // Make player 'shorter' to make it less likely to become stuck
    if player_type == PlayerType::Client && entity_bounds.y_size() > 1.0 {
        entity_bounds = entity_bounds.contract(0.0, f64::from(2.0f32 / 16.0f32), 0.0);
    }

    motion = vsub(motion, contraption_motion);
    motion = rotation_matrix.transform(motion);

    // Prepare entity bounds
    let local_bb = entity_bounds.moved(position).inflate(1.0E-7);

    let mut obb = OrientedBB::from_aabb(&local_bb);
    obb.rotation = *rotation_matrix;

    // Use simplified bbs when present
    let nearby;
    let collidable_bbs: &CollisionList = match colliders {
        Colliders::Simplified(list) => list,
        Colliders::Nearby(shape_at) => {
            // Else find 'nearby' individual block shapes to collide with
            nearby = potentially_collided_shapes(*shape_at, &local_bb.expand_towards(motion));
            &nearby
        }
    };

    let collision_result = collide_many(collidable_bbs, scratch, &obb, motion, entity.max_up_step, !rotation.has_vertical_rotation());

    // Resolve collision
    let mut entity_motion = entity.motion;
    let entity_motion_no_temporal = entity_motion;
    let mut collision_normal = collision_result.normal;
    let mut collision_location = collision_result.location;
    let mut total_response = collision_result.collision_response;
    let mut surface_collision = collision_result.surface_collision;
    let hard_collision = !is_zero_vec(total_response);
    let temporal_collision = collision_result.temporal_response != 1.0;
    let mut motion_response = if !temporal_collision {
        motion
    } else {
        vscale(vnormalize(motion), vlength(motion) * collision_result.temporal_response)
    };

    motion_response = vadd(rotation_matrix.transform_transposed(motion_response), contraption_motion);
    total_response = rotation_matrix.transform_transposed(total_response);
    total_response = rotate(total_response, f64::from(yaw_offset), Some(Axis::Y));
    collision_normal = rotation_matrix.transform_transposed(collision_normal);
    collision_normal = rotate(collision_normal, f64::from(yaw_offset), Some(Axis::Y));
    collision_normal = vnormalize(collision_normal);
    collision_location = rotation_matrix.transform_transposed(collision_location);
    collision_location = rotate(collision_location, f64::from(yaw_offset), Some(Axis::Y));

    let mut bounce = 0.0f64;
    let mut slide = 0.0f64;

    if !is_zero_vec(collision_location) {
        collision_location = vadd(
            collision_location,
            vscale(vadd(entity.position, entity.bounding_box.center()), f64::from(0.5f32)),
        );
        if temporal_collision {
            collision_location = vadd(collision_location, [0.0, motion_response[1], 0.0]);
        }

        let pos = block_containing(pose.to_local_vector(entity.position, 0.0, false));
        if let Some(props) = blocks(pos)
            && props.climbable
        {
            surface_collision = true;
            total_response = vadd(total_response, [0.0, f64::from(0.1f32), 0.0]);
        }

        let pos = block_containing(pose.to_local_vector(collision_location, 0.0, false));
        if let Some(props) = blocks(pos) {
            if props.has_interactor {
                out.interaction_at = Some(pos);
            }

            bounce = props.bounce;
            // `Math.max(0, friction - .6f)`: float arithmetic.
            slide = f64::from((props.friction - 0.6f32).max(0.0));
        }
    }

    let has_normal = !is_zero_vec(collision_normal);
    let any_collision = hard_collision || temporal_collision;

    if bounce > 0.0 && has_normal && any_collision && bounce_entity(entity, collision_normal, pose, bounce) {
        out.bounced = true;
        return out;
    }

    if temporal_collision {
        let ideal_vertical_motion = motion_response[1];
        if ideal_vertical_motion != entity_motion[1] {
            entity.motion = vadd(vmul(entity_motion, [1.0, 0.0, 1.0]), [0.0, ideal_vertical_motion, 0.0]);
            entity_motion = entity.motion;
        }
    }

    if hard_collision {
        let motion_x = entity_motion[0];
        let motion_y = entity_motion[1];
        let motion_z = entity_motion[2];
        let intersect_x = total_response[0];
        let intersect_y = total_response[1];
        let intersect_z = total_response[2];

        let horizonal_epsilon = f64::from(1.0f32 / 128.0f32);
        if motion_x != 0.0 && intersect_x.abs() > horizonal_epsilon && (motion_x > 0.0) == (intersect_x < 0.0) {
            entity_motion = vmul(entity_motion, [0.0, 1.0, 1.0]);
        }
        if motion_y != 0.0 && intersect_y != 0.0 && (motion_y > 0.0) == (intersect_y < 0.0) {
            entity_motion = vadd(vmul(entity_motion, [1.0, 0.0, 1.0]), [0.0, contraption_motion[1], 0.0]);
        }
        if motion_z != 0.0 && intersect_z.abs() > horizonal_epsilon && (motion_z > 0.0) == (intersect_z < 0.0) {
            entity_motion = vmul(entity_motion, [1.0, 1.0, 0.0]);
        }
    }

    if bounce == 0.0 && slide > 0.0 && has_normal && any_collision && rotation.has_vertical_rotation() {
        let slide_factor = vlength(vmul(collision_normal, [1.0, 0.0, 1.0])) * f64::from(1.25f32);
        let motion_in = vadd(vmul(entity_motion_no_temporal, [0.0, 0.9, 0.0]), [0.0, f64::from(-0.01f32), 0.0]);
        let slide_normal = vnormalize(vcross(collision_normal, vcross(motion_in, collision_normal)));
        let new_motion = vadd(
            vmul(entity_motion, [0.85, 0.0, 0.85]),
            vadd(
                vscale(slide_normal, (f64::from(0.2f32) + slide) * vlength(motion_in) * slide_factor),
                [0.0, f64::from(-0.1f32) - collision_normal[1] * f64::from(0.125f32), 0.0],
            ),
        );
        entity.motion = new_motion;
        entity_motion = entity.motion;
    }

    if !hard_collision && !surface_collision {
        return out;
    }
    out.collided = true;

    let mut allowed_movement = collide(total_response, entity.max_up_step, &entity.bounding_box, world_collide);
    entity.set_pos([
        entity_position[0] + allowed_movement[0],
        entity_position[1] + allowed_movement[1],
        entity_position[2] + allowed_movement[2],
    ]);
    entity_position = entity.position;

    // `handleDamageFromTrain` is left out: it does nothing unless the structure is a
    // `CarriageContraptionEntity`.

    entity.hurt_marked = true;
    let mut contact_point_motion = [0.0; 3];

    if surface_collision {
        out.registered_colliding = true;
        entity.fall_distance = 0.0;
        out.fall_distance_reset = true;
        if entity.has_client_player_rider {
            out.notify_client_riders = true;
        }
        let can_walk = bounce != 0.0 || slide == 0.0;
        if can_walk || !rotation.has_vertical_rotation() {
            if can_walk {
                entity.on_ground = true;
                out.set_on_ground = true;
            }
            if entity.is_item {
                entity_motion = vmul(entity_motion, [f64::from(0.5f32), 1.0, f64::from(0.5f32)]);
                out.item_slowed = true;
            }
        }
        contact_point_motion = pose.contact_point_motion(entity_position);
        allowed_movement = collide(contact_point_motion, entity.max_up_step, &entity.bounding_box, world_collide);
        entity.set_pos([
            entity_position[0] + allowed_movement[0],
            entity_position[1],
            entity_position[2] + allowed_movement[2],
        ]);
    }

    entity.motion = entity_motion;
    out.contact_point_motion = contact_point_motion;
    out.position_correction = vsub(entity.position, start_position);

    if player_type != PlayerType::Client {
        return out;
    }

    let d0 = entity.position[0] - entity.prev_position[0] - contact_point_motion[0];
    let d1 = entity.position[2] - entity.prev_position[2] - contact_point_motion[2];
    let mut limb_swing = ((d0 * d0 + d1 * d1) as f32).sqrt() * 4.0;
    if limb_swing > 1.0 {
        limb_swing = 1.0;
    }
    out.client_motion = Some(ClientMotion { motion: entity_motion, limb_swing });

    if entity.on_ground && pose.translating {
        out.safety_lock = Some(entity.position[1] - pose.position[1]);
    }
    out
}

/// `ContraptionCollider.collideEntities`'s entity loop over entities already found with
/// [`broad_phase`] and filtered with [`can_collide_with`] (alive, not frozen): runs
/// [`collide_entity`] on each, in order, skipping a second reference to the local player
/// (`skipClientPlayer`: "after death, multiple refs to the client player may show up"). The
/// closure is `Entity.collideBoundingBox` for the entity at the given index.
pub fn collide_entities(
    frame: &ColliderFrame,
    colliders: &Colliders,
    scratch: &mut CollisionList,
    blocks: &dyn Fn(Pos) -> Option<BlockProps>,
    entities: &mut [EntityBody],
    world_collide: &mut dyn FnMut(usize, Vec3, &Aabb) -> Vec3,
) -> Vec<EntityOutcome> {
    let mut skip_client_player = false;
    let mut outcomes = Vec::with_capacity(entities.len());
    for (index, entity) in entities.iter_mut().enumerate() {
        if entity.player_type == PlayerType::Client {
            if skip_client_player {
                outcomes.push(EntityOutcome { skipped: true, ..EntityOutcome::default() });
                continue;
            }
            skip_client_player = true;
        }
        let mut collide_bb = |motion: Vec3, bb: &Aabb| world_collide(index, motion, bb);
        outcomes.push(collide_entity(frame, colliders, scratch, blocks, entity, &mut collide_bb));
    }
    outcomes
}

// --- the local player's clipping rescue ------------------------------------------------------

/// `ContraptionCollider.savePlayerFromClipping`: the player has sunk into a structure that moved
/// up faster than the collider could push; put them back on the highest surface under them.
/// `ray_trace(start, end)` is `ContraptionHandlerClient.rayTraceContraption`: the structure's
/// shapes hit by a world-space ray (leaving out top faces), giving the hit in the structure's
/// local space. Returns the player's new Y, or `None` if nothing was found (Create returns false).
pub fn save_player_from_clipping(
    player_box: &Aabb,
    player_y: f64,
    max_up_step: f32,
    pose: &StructurePose,
    y_start_offset: f64,
    ray_trace: &mut dyn FnMut(Vec3, Vec3) -> Option<Vec3>,
) -> Option<f64> {
    // `deflate(1 / 4f, 0, 1 / 4f)`
    let bb = player_box.inflate_xyz(-f64::from(0.25f32), -0.0, -f64::from(0.25f32));
    let mut shortest_distance = f64::MAX;
    let y_start = f64::from(max_up_step) + pose.position[1] + y_start_offset;
    let ray_length = f64::max(5.0, (player_y - y_start).abs());

    for ray_index in 0..4 {
        let start = [
            if ray_index / 2 == 0 { bb.min[0] } else { bb.max[0] },
            y_start,
            if ray_index % 2 == 0 { bb.min[2] } else { bb.max[2] },
        ];
        let end = vadd(start, [0.0, -ray_length, 0.0]);

        let Some(hit_local) = ray_trace(start, end) else { continue };

        let hit = pose.to_global_vector(hit_local, 1.0, false);
        let hit_diff = start[1] - hit[1];
        if shortest_distance > hit_diff {
            shortest_distance = hit_diff;
        }
    }

    if shortest_distance > ray_length {
        return None;
    }
    Some(y_start - shortest_distance)
}

/// What `saveClientPlayerFromClipping` decided.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClippingDecision {
    /// Move the player to this Y (`entity.setPos(x, y, z)`).
    pub new_y: Option<f64>,
    /// `safetyLock.setLeft(null)`: forget the lock.
    pub clear_lock: bool,
    /// Send `ContraptionColliderLockPacketRequest(contraptionId, offset)` with this offset.
    pub lock_request_offset: Option<f64>,
}

/// `ContraptionCollider.saveClientPlayerFromClipping`, for the local player, run each tick on the
/// structure the safety lock is on (`safetyLock.left`). `lock_offset` is `safetyLock.right`;
/// `packet_cooldown` is the static `packetCooldown`; `multiplayer` is
/// `handler.getOnlinePlayers().size() > 1`.
#[allow(clippy::too_many_arguments)]
pub fn save_client_player_from_clipping(
    pose: &StructurePose,
    contraption_motion: Vec3,
    player_box: &Aabb,
    player_position: Vec3,
    player_motion: Vec3,
    player_max_up_step: f32,
    player_is_passenger: bool,
    lock_offset: f64,
    multiplayer: bool,
    packet_cooldown: &mut i32,
    ray_trace: &mut dyn FnMut(Vec3, Vec3) -> Option<Vec3>,
) -> ClippingDecision {
    let mut out = ClippingDecision::default();
    if player_is_passenger {
        return out;
    }

    let prev_diff = lock_offset;
    let current_diff = player_position[1] - pose.position[1];
    let motion = vsub(contraption_motion, player_motion)[1];
    let trend = super::collider_obb::jsignum(current_diff - prev_diff);

    if multiplayer {
        if *packet_cooldown > 0 {
            *packet_cooldown -= 1;
        }
        if *packet_cooldown == 0 {
            out.lock_request_offset = Some(current_diff);
            *packet_cooldown = 3;
        }
    }

    if trend == 0.0 {
        return out;
    }
    if trend == super::collider_obb::jsignum(motion) {
        return out;
    }

    let speed = vlength_sqr(vmul(contraption_motion, [0.0, 1.0, 0.0]));
    if trend > 0.0 && speed < 0.1 {
        return out;
    }
    if speed < 0.05 {
        return out;
    }

    match save_player_from_clipping(player_box, player_position[1], player_max_up_step, pose, prev_diff, ray_trace) {
        Some(y) => out.new_y = Some(y),
        None => out.clear_lock = true,
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A player-sized entity (0.6 x 1.8, step height 0.6) standing at `position`.
    fn player(position: Vec3) -> EntityBody {
        EntityBody::new(position, 0.6, 1.8)
    }

    /// The world has nothing in the way: `Entity.collideBoundingBox` lets everything through.
    fn open_world(motion: Vec3, _bb: &Aabb) -> Vec3 {
        motion
    }

    fn no_blocks(_: Pos) -> Option<BlockProps> {
        None
    }

    fn near(a: f64, b: f64, eps: f64) -> bool {
        (a - b).abs() <= eps
    }

    fn near3(a: Vec3, b: Vec3, eps: f64) -> bool {
        (0..3).all(|i| near(a[i], b[i], eps))
    }

    /// One full cube at local `(x, y, z)`.
    fn cube(list: &mut CollisionList, x: i32, y: i32, z: i32) {
        list.append_box([x, y, z], [0.0; 3], [1.0; 3]);
    }

    fn run(pose: &StructurePose, list: &CollisionList, entity: &mut EntityBody) -> EntityOutcome {
        let frame = ColliderFrame::new(pose);
        let mut scratch = CollisionList::new();
        collide_entity(&frame, &Colliders::Simplified(list), &mut scratch, &no_blocks, entity, &mut |m, b| open_world(m, b))
    }

    // The inflation Create gives the entity's box before colliding.
    const EPS: f64 = 1.0E-7;
    // `withSignedEpsilon`'s nudge.
    const NUDGE: f64 = 1.0E-4;

    #[test]
    fn box_lands_on_a_still_platform_and_stands() {
        // A one-block platform at the origin; its top is y = 1. The player's feet are 0.2 above it
        // and fall by 0.5 in a tick.
        //
        // By hand from `collideMany` (the box centre is 1.2 + 0.9 = 2.1, the block's 0.5): TL = 1.6,
        // rA = 0.5, rB = 0.9 + 1e-7, so diff = 0.2 - 1e-7 > 0 and the entry time is diff / 0.5
        // = 0.4 - 2e-7: the entity reaches the top after 0.4 - 2e-7 of its motion. No push-out, a
        // surface collision. `collideEntities` then cuts the motion to that part:
        // y = -0.5 * (0.4 - 2e-7) = -0.2 + 1e-7.
        let pose = StructurePose::translating([0.0; 3], [0.0; 3], [0.0; 3]);
        let mut list = CollisionList::new();
        cube(&mut list, 0, 0, 0);
        let mut e = player([0.5, 1.2, 0.5]);
        e.motion = [0.0, -0.5, 0.0];
        let out = run(&pose, &list, &mut e);

        assert!(out.collided);
        assert!(out.registered_colliding && out.set_on_ground && out.fall_distance_reset);
        assert!(e.on_ground && e.hurt_marked);
        assert!(near(e.motion[1], -0.2 + EPS, 1e-9), "{:?}", e.motion);
        assert_eq!(e.position, [0.5, 1.2, 0.5], "it lands on the next tick, nothing to push yet");
        assert_eq!(out.contact_point_motion, [0.0; 3]);
    }

    #[test]
    fn resting_box_is_held_up_by_the_platform() {
        // Feet at y = 1.0 exactly, falling at -0.0784 (gravity). By hand:
        // centre 1.0 + 0.9 = 1.9, minus the block's 0.5 = 1.4 = TL; rA + rB = 0.5 + 0.9 + 1e-7, so
        // diff = -1e-7 (a discrete collision); the step separation along Y is 1e-7, within the step
        // height, so the response is 1e-7 + 1e-4 (withSignedEpsilon) up. The second (vertical) pass
        // then sees diff = +1e-4 and a time of impact of 1e-4 / 0.0784 on the motion.
        let pose = StructurePose::translating([0.0; 3], [0.0; 3], [0.0; 3]);
        let mut list = CollisionList::new();
        cube(&mut list, 0, 0, 0);
        let mut e = player([0.5, 1.0, 0.5]);
        e.motion = [0.0, -0.0784, 0.0];
        let out = run(&pose, &list, &mut e);

        let up = EPS + NUDGE;
        assert!(out.collided && e.on_ground);
        assert!(near(e.position[1], 1.0 + up, 1e-12), "{:?}", e.position);
        assert!(near3(out.position_correction, [0.0, up, 0.0], 1e-12));
        // The downward motion into the platform is stopped (`motionY > 0 == intersectY < 0`).
        assert_eq!(e.motion, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn platform_moving_up_carries_the_entity() {
        // The platform went from y = 0 to y = 0.1 this tick, the entity stood at y = 1.0 (and falls
        // at -0.0784). In local space its feet are at 0.9, inside the block by 0.1 + 1e-7: TL =
        // (0.9 + 0.9) - 0.5 = 1.3, rA + rB = 1.4 + 1e-7, so the step separation is 0.1 + 1e-7 and the
        // response 0.1 + 1e-7 + 1e-4 up. The entity ends 0.1001001 higher: y = 1.1001001.
        // The relative motion is -0.0784 - 0.1 = -0.1784; the vertical pass gives it a time of
        // impact of 1e-4 / 0.1784, so the ideal motion is -1e-4 + the platform's 0.1 = 0.0999.
        // The carry (`getContactPointMotion` of a translating structure) is its delta movement.
        let pose = StructurePose::translating([0.0, 0.1, 0.0], [0.0; 3], [0.0, 0.1, 0.0]);
        let mut list = CollisionList::new();
        cube(&mut list, 0, 0, 0);
        let mut e = player([0.5, 1.0, 0.5]);
        e.motion = [0.0, -0.0784, 0.0];
        let out = run(&pose, &list, &mut e);

        assert!(out.collided && e.on_ground);
        assert!(near(e.position[1], 1.0 + 0.1 + EPS + NUDGE, 1e-9), "{:?}", e.position);
        assert!(near(e.motion[1], 0.1 - NUDGE, 1e-9), "{:?}", e.motion);
        assert_eq!(out.contact_point_motion, [0.0, 0.1, 0.0]);
        // Horizontal position isn't changed by an upward carry.
        assert!(near(e.position[0], 0.5, 1e-12) && near(e.position[2], 0.5, 1e-12));
    }

    #[test]
    fn platform_moving_sideways_carries_the_entity_along() {
        // From x = 0 to x = 0.1. The entity stands on it at x = 0.5 (local 0.4, still on the
        // block) and is carried by the contact point motion (0.1, 0, 0): x = 0.6. It is held up the
        // same way as on a still platform (+1e-7 + 1e-4).
        let pose = StructurePose::translating([0.1, 0.0, 0.0], [0.0; 3], [0.1, 0.0, 0.0]);
        let mut list = CollisionList::new();
        cube(&mut list, 0, 0, 0);
        let mut e = player([0.5, 1.0, 0.5]);
        e.motion = [0.0, -0.0784, 0.0];
        let out = run(&pose, &list, &mut e);

        assert!(e.on_ground);
        assert_eq!(out.contact_point_motion, [0.1, 0.0, 0.0]);
        assert!(near(e.position[0], 0.6, 1e-12), "{:?}", e.position);
        assert!(near(e.position[1], 1.0 + EPS + NUDGE, 1e-12));
    }

    #[test]
    fn piston_face_pushes_an_entity() {
        // The block moved from x = 0 to x = 0.1 (right face at 1.1); the entity (x 0.95..1.55,
        // feet at y = 0, the block's bottom) is 0.15 inside it. Local centre x = 1.15, TL = 0.65,
        // rA + rB = 0.8 + 1e-7: diff = -0.15 - 1e-7. The step separation (up) is 1.0 + 1e-7, more
        // than the 0.6 step height, so Create pushes along the axis of least separation, X:
        // 0.15 + 1e-7 + 1e-4. This is not a standing: no surface, no onGround.
        let pose = StructurePose::translating([0.1, 0.0, 0.0], [0.0; 3], [0.1, 0.0, 0.0]);
        let mut list = CollisionList::new();
        cube(&mut list, 0, 0, 0);
        let mut e = player([1.25, 0.0, 0.5]);
        let out = run(&pose, &list, &mut e);

        let push = 0.15 + EPS + NUDGE;
        assert!(out.collided);
        assert!(!out.registered_colliding && !e.on_ground, "pushed sideways, not standing");
        assert!(near3(out.position_correction, [push, 0.0, 0.0], 1e-9), "{:?}", out.position_correction);
        assert!(near(e.position[0], 1.25 + push, 1e-9));
        assert!(e.hurt_marked);
    }

    #[test]
    fn push_stops_the_motion_into_the_face() {
        // The same entity walking into the face (-x) at 0.1 a tick loses that motion.
        let pose = StructurePose::translating([0.1, 0.0, 0.0], [0.0; 3], [0.1, 0.0, 0.0]);
        let mut list = CollisionList::new();
        cube(&mut list, 0, 0, 0);
        let mut e = player([1.25, 0.0, 0.5]);
        e.motion = [-0.1, 0.0, 0.05];
        run(&pose, &list, &mut e);
        assert_eq!(e.motion, [0.0, 0.0, 0.05]);
    }

    #[test]
    fn bearing_turns_an_entity_standing_on_it() {
        // A bearing structure at the origin with one block at local (1, 0, 0), turning about Y from
        // 0 to 10 degrees this tick. The entity stands on the block at its old place (1.5, 1, .5),
        // 1 block from the axis. Create's `applyRotation` is `VecHelper.rotate(v, 10, Y)`:
        // (x, z) -> (x cos + z sin, z cos - x sin), so the point goes to (cos 10, -sin 10) from
        // the axis: a move of (cos 10 - 1, 0, -sin 10) = (-0.015192, 0, -0.173648) (clockwise seen
        // from above). That is `getContactPointMotion`, and the standing entity is carried by it.
        let pose = StructurePose::turning([0.0; 3], Axis::Y, 10.0, 0.0);
        let mut list = CollisionList::new();
        cube(&mut list, 1, 0, 0);
        let start = [1.5, 1.0, 0.5];
        let mut e = player(start);
        e.motion = [0.0, -0.0784, 0.0];
        let out = run(&pose, &list, &mut e);

        let (s, c) = 10.0f64.to_radians().sin_cos();
        let want = [c - 1.0, 0.0, -s];
        assert!(e.on_ground && out.registered_colliding);
        assert!(near3(out.contact_point_motion, want, 5e-4), "{:?} vs {want:?}", out.contact_point_motion);
        // The carry moves the entity by it, horizontally.
        assert!(near(e.position[0], start[0] + want[0], 5e-4) && near(e.position[2], start[2] + want[2], 5e-4), "{:?}", e.position);
        // The turn tilts nothing: the entity keeps standing at the top (plus the nudge).
        assert!(near(e.position[1], 1.0 + EPS + NUDGE, 1e-3), "{:?}", e.position);
        assert_eq!(out.client_motion, None);
    }

    #[test]
    fn contact_point_motion_of_a_turning_structure() {
        // Same numbers as above, straight from `getContactPointMotion`, for a structure that turned
        // 10 -> 20 degrees about Y: the point (1.5, 1, .5) is 1 from the axis; rotate the local
        // vector (1, .5, 0) (about the block-centre) from the 10 degree pose to the 20 degree one.
        let mut pose = StructurePose::turning([0.0; 3], Axis::Y, 20.0, 10.0);
        pose.prev_position = [0.0; 3];
        let (s10, c10) = 10.0f64.to_radians().sin_cos();
        let (s20, c20) = 20.0f64.to_radians().sin_cos();
        // The point as the 10 degree pose has it in world space for local vector (1, .5, 0):
        let p = [0.5 + c10, 1.0, 0.5 - s10];
        let motion = pose.contact_point_motion(p);
        let want = [c20 - c10, 0.0, -(s20 - s10)];
        assert!(near3(motion, want, 5e-4), "{motion:?} vs {want:?}");
    }

    #[test]
    fn invalid_previous_position_gives_no_carry() {
        // `prevPosInvalid` (the structure's first tick): a turning structure carries nothing.
        let mut pose = StructurePose::turning([0.0; 3], Axis::Y, 10.0, 0.0);
        pose.prev_pos_invalid = true;
        assert_eq!(pose.contact_point_motion([1.5, 1.0, 0.5]), [0.0; 3]);
    }

    #[test]
    fn entity_walks_through_a_gap_in_the_structure() {
        // Two walls of two blocks (x 0..1 and x 2..3), a one-block gap between them. The player
        // (0.6 wide) in the middle of the gap touches neither wall, and nothing changes.
        let pose = StructurePose::translating([0.0; 3], [0.0; 3], [0.0; 3]);
        let mut list = CollisionList::new();
        for y in 0..2 {
            cube(&mut list, 0, y, 0);
            cube(&mut list, 2, y, 0);
        }
        let mut e = player([1.5, 0.0, 0.5]);
        e.motion = [0.1, 0.0, 0.0];
        let before = e;
        let out = run(&pose, &list, &mut e);
        assert!(!out.collided && !out.registered_colliding);
        assert_eq!(e, before, "walking in the gap is untouched");

        // Stepping through the gap lengthwise (along z) is untouched as well.
        let mut e = player([1.5, 0.0, -2.0]);
        e.motion = [0.0, 0.0, 0.2];
        let before = e;
        run(&pose, &list, &mut e);
        assert_eq!(e, before);
    }

    #[test]
    fn gap_edge_still_stops_the_entity() {
        // At the gap's right side the box (x 1.5..2.1) is 0.1 into the wall at x = 2: pushed back
        // out. By hand: centre 1.8, the block's 2.5, TL = -0.7; rA + rB = 0.8 + 1e-7:
        // diff = -0.1 - 1e-7, and the step separation up is 1 + 1e-7 > 0.6, so the push is along X:
        // -(0.1 + 1e-7 + 1e-4).
        let pose = StructurePose::translating([0.0; 3], [0.0; 3], [0.0; 3]);
        let mut list = CollisionList::new();
        for y in 0..2 {
            cube(&mut list, 0, y, 0);
            cube(&mut list, 2, y, 0);
        }
        let mut e = player([1.8, 0.0, 0.5]);
        let out = run(&pose, &list, &mut e);
        assert!(out.collided);
        assert!(near(e.position[0], 1.8 - (0.1 + EPS + NUDGE), 1e-9), "{:?}", e.position);
        assert!(!e.on_ground);
    }

    #[test]
    fn entity_steps_up_onto_a_half_slab() {
        // A bottom half slab (y 0..0.5) at local (1, 0, 0); the entity's feet are at the structure's
        // bottom (y = 0) and its box (x 0.75..1.35) is 0.35 into it. By hand: the Y step separation
        // is deltaY - (rA + rB): TL = 0.9 - 0.25 = 0.65, rA + rB = 0.25 + 0.9 + 1e-7, so 0.5 + 1e-7,
        // under the 0.6 step height, so Create lifts the entity by it (+1e-4) rather than push it
        // out along X. It also stops the motion into the ground, not the walking.
        let pose = StructurePose::translating([0.0; 3], [0.0; 3], [0.0; 3]);
        let mut list = CollisionList::new();
        list.append_box([1, 0, 0], [0.0; 3], [1.0, 0.5, 1.0]);
        let mut e = player([1.05, 0.0, 0.5]);
        e.motion = [0.1, -0.0784, 0.0];
        let out = run(&pose, &list, &mut e);

        let lift = 0.5 + EPS + NUDGE;
        assert!(out.collided);
        assert!(near(e.position[1], lift, 1e-9), "{:?}", e.position);
        assert!(near(e.position[0], 1.05, 1e-9), "stepped up, not pushed: {:?}", e.position);
        assert!(e.on_ground);
        assert!(near(e.motion[0], 0.1, 1e-12), "keeps walking: {:?}", e.motion);
        assert_eq!(e.motion[1], 0.0);
    }

    #[test]
    fn full_block_is_too_high_to_step_up() {
        // The same entity against a full block (1 high): the step separation 1 + 1e-7 is more than
        // 0.6, so it is pushed out sideways (-X, the side it entered from: the block's x 1..2 and
        // the box's 0.75..1.35, so back by 0.35 + 1e-7 + 1e-4) and its walking into the face stops.
        let pose = StructurePose::translating([0.0; 3], [0.0; 3], [0.0; 3]);
        let mut list = CollisionList::new();
        cube(&mut list, 1, 0, 0);
        let mut e = player([1.05, 0.0, 0.5]);
        e.motion = [0.1, 0.0, 0.0];
        run(&pose, &list, &mut e);
        assert!(near(e.position[0], 1.05 - (0.35 + EPS + NUDGE), 1e-9), "{:?}", e.position);
        assert_eq!(e.motion[0], 0.0);
        assert!(e.position[1] == 0.0);
    }

    #[test]
    fn remote_and_server_players_are_skipped() {
        let pose = StructurePose::translating([0.0; 3], [0.0; 3], [0.0; 3]);
        let mut list = CollisionList::new();
        cube(&mut list, 0, 0, 0);
        for kind in [PlayerType::Server, PlayerType::Remote] {
            let mut e = player([0.5, 0.5, 0.5]);
            e.player_type = kind;
            let before = e;
            let out = run(&pose, &list, &mut e);
            assert!(out.skipped && !out.collided);
            assert_eq!(e, before);
        }
    }

    #[test]
    fn local_player_gets_a_motion_packet_and_a_safety_lock() {
        // The local player standing on a translating platform: the outcome carries the
        // `ClientMotionPacket` (the limb swing is the walked distance less the carry, x 4, at most
        // 1; here the player didn't move, so 0) and the safety lock (feet height over the structure).
        let pose = StructurePose::translating([0.0; 3], [0.0; 3], [0.0; 3]);
        let mut list = CollisionList::new();
        cube(&mut list, 0, 0, 0);
        let mut e = player([0.5, 1.0, 0.5]);
        e.player_type = PlayerType::Client;
        e.motion = [0.0, -0.0784, 0.0];
        let out = run(&pose, &list, &mut e);
        let cm = out.client_motion.expect("a motion packet");
        assert_eq!(cm.limb_swing, 0.0);
        assert!(near(out.safety_lock.unwrap(), 1.0 + EPS + NUDGE, 1e-12));
    }

    #[test]
    fn local_player_box_is_made_shorter() {
        // For the local player a box taller than 1 loses 2/16 at the top, so a ceiling brushing
        // the top 0.1 of the head doesn't catch them: a block whose underside is at 1.75 is missed
        // by a 1.8 tall player at y = 0 (box up to 1.8 - 0.125 = 1.675) but hit by a server-side one.
        let pose = StructurePose::translating([0.0; 3], [0.0; 3], [0.0; 3]);
        let mut list = CollisionList::new();
        list.append_box([0, 1, 0], [0.0, 0.75, 0.0], [1.0, 1.0, 1.0]);
        let mut client = player([0.5, 0.0, 0.5]);
        client.player_type = PlayerType::Client;
        let out = run(&pose, &list, &mut client);
        assert!(!out.collided, "{out:?}");
        let mut mob = player([0.5, 0.0, 0.5]);
        let out = run(&pose, &list, &mut mob);
        assert!(out.collided);
    }

    #[test]
    fn slime_block_bounces_a_falling_entity() {
        let pose = StructurePose::translating([0.0; 3], [0.0; 3], [0.0; 3]);
        let mut list = CollisionList::new();
        cube(&mut list, 0, 0, 0);
        let slime = |p: Pos| (p == [0, 0, 0]).then_some(BlockProps { bounce: bounce_multiplier(true, false), friction: 0.8, ..BlockProps::default() });
        // Landing (feet 0.2 over the top, falling fast): the contact location is in the block.
        let mut e = player([0.5, 1.0, 0.5]);
        e.motion = [0.0, -1.0, 0.0];
        let frame = ColliderFrame::new(&pose);
        let mut scratch = CollisionList::new();
        let out = collide_entity(&frame, &Colliders::Simplified(&list), &mut scratch, &slime, &mut e, &mut |m, b| open_world(m, b));
        assert!(out.bounced, "{out:?}");
        assert!(e.motion[1] > 0.0, "thrown back up: {:?}", e.motion);
    }

    #[test]
    fn climbable_block_adds_a_lift() {
        // Ladders and vines in a structure: standing counts and the response gets +0.1f up.
        let pose = StructurePose::translating([0.0; 3], [0.0; 3], [0.0; 3]);
        let mut list = CollisionList::new();
        cube(&mut list, 0, 0, 0);
        let ladder = |p: Pos| (p == [0, 0, 0] || p == [0, 1, 0]).then_some(BlockProps { climbable: true, friction: 0.6, ..BlockProps::default() });
        let mut e = player([0.5, 1.0, 0.5]);
        e.motion = [0.0, -0.0784, 0.0];
        let frame = ColliderFrame::new(&pose);
        let mut scratch = CollisionList::new();
        collide_entity(&frame, &Colliders::Simplified(&list), &mut scratch, &ladder, &mut e, &mut |m, b| open_world(m, b));
        // 1e-7 + 1e-4 for the touch, plus the climbable's float 0.1.
        assert!(near(e.position[1], 1.0 + EPS + NUDGE + f64::from(0.1f32), 1e-9), "{:?}", e.position);
    }

    #[test]
    fn world_blocks_keep_a_pushed_entity_out() {
        // The push goes through the world's collision (`collide`): a wall of the world 0.05 beyond
        // the entity's right side limits how far the piston face can push it.
        let pose = StructurePose::translating([0.1, 0.0, 0.0], [0.0; 3], [0.1, 0.0, 0.0]);
        let mut list = CollisionList::new();
        cube(&mut list, 0, 0, 0);
        let mut e = player([1.25, 0.0, 0.5]);
        let frame = ColliderFrame::new(&pose);
        let mut scratch = CollisionList::new();
        // Entity right face at 1.55; a wall from x = 1.6: at most 0.05 more.
        let mut wall = |m: Vec3, b: &Aabb| {
            let room = 1.6 - b.max[0];
            [if m[0] > room { room } else { m[0] }, m[1], m[2]]
        };
        collide_entity(&frame, &Colliders::Simplified(&list), &mut scratch, &no_blocks, &mut e, &mut wall);
        assert!(near(e.position[0], 1.3, 1e-9), "{:?}", e.position);
    }

    #[test]
    fn collide_steps_up_like_entity_collide() {
        // `ContraptionCollider.collide`: walking into a 0.5 high ledge (a world box x 1..2, y 0..0.5)
        // on a floor (y < 0) with a 0.6 step height climbs it. A stand-in for `collideBoundingBox`
        // clips a move against those boxes axis by axis (Y, then X, then Z).
        let boxes = [
            Aabb::new([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0]),
            Aabb::new([1.0, 0.0, 0.0], [2.0, 0.5, 1.0]),
        ];
        let mut clip = |m: Vec3, bb: &Aabb| -> Vec3 {
            let mut m = m;
            let mut bb = *bb;
            for axis in [1usize, 0, 2] {
                for b in &boxes {
                    let overlap = (0..3).filter(|o| *o != axis).all(|o| bb.max[o] > b.min[o] && bb.min[o] < b.max[o]);
                    if !overlap {
                        continue;
                    }
                    if m[axis] > 0.0 && bb.max[axis] <= b.min[axis] + 1e-9 {
                        m[axis] = m[axis].min(b.min[axis] - bb.max[axis]);
                    } else if m[axis] < 0.0 && bb.min[axis] >= b.max[axis] - 1e-9 {
                        m[axis] = m[axis].max(b.max[axis] - bb.min[axis]);
                    }
                }
                let mut d = [0.0; 3];
                d[axis] = m[axis];
                bb = bb.moved(d);
            }
            m
        };
        let bb = Aabb::new([0.2, 0.0, 0.2], [0.8, 1.8, 0.8]);
        // Plain clipping: the ledge stops the walk after 0.2 (box edge 0.8 to the ledge at 1.0).
        let plain = clip([0.4, -0.08, 0.0], &bb);
        assert!(near(plain[0], 0.2, 1e-9) && plain[1] == 0.0, "{plain:?}");
        // With the step: by hand, up 0.6 (clear of the ledge), 0.4 across, then the drop of
        // -0.6 + -0.08 lands on the ledge top 0.5, 0.1 down: (0.4, 0.5, 0).
        let out = collide([0.4, -0.08, 0.0], 0.6, &bb, &mut clip);
        assert!(near3(out, [0.4, 0.5, 0.0], 1e-9), "{out:?}");
        // No step height, no climbing.
        let out = collide([0.4, -0.08, 0.0], 0.0, &bb, &mut clip);
        assert!(near(out[0], 0.2, 1e-9), "{out:?}");
    }

    #[test]
    fn broad_phase_covers_the_structure_and_the_column_above() {
        let pose = StructurePose::translating([10.0, 64.0, 10.0], [10.0, 64.0, 10.0], [0.0; 3]);
        let bounds = pose.bounding_box(&Aabb::new([0.0; 3], [3.0, 2.0, 3.0]));
        assert_eq!(bounds, Aabb::new([10.0, 64.0, 10.0], [13.0, 66.0, 13.0]));
        let area = broad_phase_area(&bounds);
        assert_eq!(area, Aabb::new([8.0, 62.0, 8.0], [15.0, 100.0, 15.0]));
        assert!(broad_phase(&bounds, &Aabb::new([11.0, 90.0, 11.0], [11.6, 91.8, 11.6])), "32 up still counts");
        assert!(!broad_phase(&bounds, &Aabb::new([11.0, 101.0, 11.0], [11.6, 102.8, 11.6])));
        assert!(!broad_phase(&bounds, &Aabb::new([20.0, 64.0, 11.0], [20.6, 65.8, 11.6])));
    }

    #[test]
    fn oriented_structure_anchor_is_half_a_block_back() {
        let pose = StructurePose {
            position: [5.0, 70.0, 5.0],
            prev_position: [4.0, 70.0, 5.0],
            prev_pos_invalid: false,
            delta_movement: [1.0, 0.0, 0.0],
            rotation: Rotation::Oriented { initial_yaw: 0.0, pitch: 0.0, yaw: 0.0, prev_pitch: 0.0, prev_yaw: 0.0 },
            translating: false,
        };
        assert_eq!(pose.anchor_vec(), [4.5, 70.0, 4.5]);
        assert_eq!(pose.prev_anchor_vec(), [3.5, 70.0, 4.5]);
        // A cart moving 1 in x carries things at rest by 1 in x (the anchor's movement).
        assert!(near3(pose.contact_point_motion([5.0, 71.0, 5.0]), [1.0, 0.0, 0.0], 1e-9));
    }

    #[test]
    fn rotation_state_of_a_cart() {
        let mut pose = StructurePose::translating([0.0; 3], [0.0; 3], [0.0; 3]);
        pose.rotation = Rotation::Oriented { initial_yaw: 90.0, pitch: 0.0, yaw: 30.0, prev_pitch: 0.0, prev_yaw: 0.0 };
        let r = pose.rotation_state();
        assert_eq!((r.y_rotation, r.z_rotation, r.second_y_rotation), (60.0, 0.0, 0.0));
        assert!(!r.has_vertical_rotation());
        pose.rotation = Rotation::Oriented { initial_yaw: 90.0, pitch: 10.0, yaw: 30.0, prev_pitch: 0.0, prev_yaw: 0.0 };
        let r = pose.rotation_state();
        assert_eq!((r.y_rotation, r.z_rotation, r.second_y_rotation), (90.0, 10.0, -30.0));
        assert!(r.has_vertical_rotation());
        assert_eq!(r.yaw_offset(), -30.0);
    }

    #[test]
    fn local_and_global_vectors_round_trip() {
        let pose = StructurePose::turning([3.0, 64.0, -2.0], Axis::Y, 37.0, 37.0);
        let p = [4.2, 65.5, -1.1];
        let back = pose.to_global_vector(pose.to_local_vector(p, 1.0, false), 1.0, false);
        assert!(near3(back, p, 5e-3), "{back:?}");
        // At partial ticks 0 the previous angle is used.
        let pose = StructurePose::turning([0.0; 3], Axis::Y, 90.0, 0.0);
        let at0 = pose.to_global_vector([1.5, 0.5, 0.5], 0.0, false);
        let at1 = pose.to_global_vector([1.5, 0.5, 0.5], 1.0, false);
        assert!(near3(at0, [1.5, 0.5, 0.5], 1e-9));
        assert!(near3(at1, [0.5, 0.5, -0.5], 1e-5), "{at1:?}");
    }

    #[test]
    fn can_collide_with_follows_create() {
        let base = CollideFacts { push_reaction_normal: true, ..CollideFacts::default() };
        assert!(can_collide_with(&base, false));
        assert!(!can_collide_with(&CollideFacts { is_player: true, is_spectator: true, ..base }, false));
        assert!(!can_collide_with(&CollideFacts { no_physics: true, ..base }, false));
        assert!(!can_collide_with(&CollideFacts { is_hanging: true, ..base }, false));
        assert!(!can_collide_with(&CollideFacts { is_projectile: true, ..base }, false));
        assert!(!can_collide_with(&CollideFacts { has_vehicle: true, ..base }, false));
        assert!(!can_collide_with(&CollideFacts { is_vehicle_of_structure: true, ..base }, false));
        assert!(!can_collide_with(&CollideFacts { push_reaction_normal: false, ..base }, false));
        let cart = CollideFacts { is_minecart: true, push_reaction_normal: false, ..base };
        assert!(can_collide_with(&cart, false), "a minecart collides with all but a mounted structure");
        assert!(!can_collide_with(&cart, true));
    }

    #[test]
    fn nearby_shapes_path_matches_the_simplified_one() {
        // `getPotentiallyCollidedShapes` finds the same blocks as the cached list.
        let pose = StructurePose::translating([0.0; 3], [0.0; 3], [0.0; 3]);
        let mut list = CollisionList::new();
        cube(&mut list, 0, 0, 0);
        let shape_at = |p: Pos| if p == [0, 0, 0] { vec![Aabb::new([0.0; 3], [1.0; 3])] } else { vec![] };
        let frame = ColliderFrame::new(&pose);
        let mut scratch = CollisionList::new();
        let mut a = player([0.5, 1.0, 0.5]);
        a.motion = [0.0, -0.0784, 0.0];
        let mut b = a;
        collide_entity(&frame, &Colliders::Simplified(&list), &mut scratch, &no_blocks, &mut a, &mut |m, bb| open_world(m, bb));
        collide_entity(&frame, &Colliders::Nearby(&shape_at), &mut scratch, &no_blocks, &mut b, &mut |m, bb| open_world(m, bb));
        assert_eq!(a, b);
    }

    #[test]
    fn collide_entities_skips_a_second_local_player() {
        let pose = StructurePose::translating([0.0; 3], [0.0; 3], [0.0; 3]);
        let mut list = CollisionList::new();
        cube(&mut list, 0, 0, 0);
        let frame = ColliderFrame::new(&pose);
        let mut scratch = CollisionList::new();
        let mut a = player([0.5, 1.0, 0.5]);
        a.player_type = PlayerType::Client;
        a.motion = [0.0, -0.0784, 0.0];
        let mut entities = [a, a, player([0.5, 1.0, 0.5])];
        entities[2].motion = [0.0, -0.0784, 0.0];
        let outs = collide_entities(&frame, &Colliders::Simplified(&list), &mut scratch, &no_blocks, &mut entities, &mut |_, m, b| open_world(m, b));
        assert!(outs[0].collided && !outs[0].skipped);
        assert!(outs[1].skipped, "the second reference to the local player");
        assert_eq!(entities[1], a, "and it is left alone");
        assert!(outs[2].collided);
    }

    #[test]
    fn clipping_rescue_puts_the_player_back_on_the_surface() {
        // Platform at y = 10 (feet standing offset 1 above it: lock offset 1.0). The player sank to
        // y = 10.4. The rays start `maxUpStep + platform y + offset` = 0.6 + 10 + 1 = 11.6 above and
        // find the top at 11.0 (local y = 1.0): the player goes to 11.6 - (11.6 - 11.0) = 11.0.
        let pose = StructurePose::translating([0.0, 10.0, 0.0], [0.0, 10.0, 0.0], [0.0; 3]);
        let bb = Aabb::new([0.2, 10.4, 0.2], [0.8, 12.2, 0.8]);
        let mut ray = |start: Vec3, _end: Vec3| {
            // The structure's top at local y = 1, over x, z in 0..1.
            (start[0] > 0.0 && start[0] < 1.0).then_some([start[0], 1.0, start[2]])
        };
        let y = save_player_from_clipping(&bb, 10.4, 0.6, &pose, 1.0, &mut ray).unwrap();
        assert!(near(y, 11.0, 1e-9), "{y}");
        assert_eq!(save_player_from_clipping(&bb, 10.4, 0.6, &pose, 1.0, &mut |_, _| None), None);
    }
}
