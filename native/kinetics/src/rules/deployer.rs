//! The Deployer's rules: its stroke, what it does to an item on a belt, where its hand is, and the
//! numbers of mining and attacking with what it holds. No game types.
//!
//! Ported from Create `content/kinetics/deployer`: `DeployerBlockEntity` (`State`, `Mode`, `tick`,
//! `start`, `getTimerSpeed`, `getHandOffset`, `getHandPose`, `startFistBump`), `DeployerBlock`
//! (`getShape`, `onWrenched`), `DeployerHandler` (`activateInner`, `shouldActivate`,
//! `tryHarvestBlock`), `BeltDeployerCallbacks` (`onItemReceived`, `whenItemHeld`),
//! `DirectionalAxisKineticBlock` (the placement and the shaft), `IWrenchable` (the turn) and
//! `AllShapes.DEPLOYER_INTERACTION`, `CStress` (4 su per RPM), checked against `mc1.21.1/dev`
//! at 4148fd8. The facts about items and blocks that Horizon's `Host` cannot ask are baked into
//! `deployer_items.rs` from the game's own data.
//!
//! Known differences from Create (the glue, `machines/deployer*.rs`, lists the rest):
//! - No fake player entity: its work goes through `Host::use_as_player` (use), `hurt_entity`
//!   (attack) and `destroy_block` (break). It has no inventory beyond the held stack and the
//!   overflow, no attributes, and mobs don't retaliate against it.
//! - Mining speed is the tool's `Tool` component and the block's hardness, with no enchantments,
//!   no Haste, and no `/ 5` for a player off the ground (a fake player never lands: Create's
//!   own value there is not checked here).
//! - Attacks deal `1 + the item's mainhand attack_damage` and wear a weapon by its
//!   `item_damage_per_attack`: no enchantment damage, sweep, critical hit or knockback.

use crate::belt::Processing;
use crate::{Axis, Dir};

pub use super::deployer_items::{self as facts, Item, NO_SPEED, Rule};

/// `CStress.setImpact(4.0)` for `AllBlocks.DEPLOYER`.
pub const IMPACT: f32 = 4.0;
/// The timer of an extended or retracting hand: 1000 units, spent at `getTimerSpeed()` a tick.
pub const STROKE: i32 = 1000;
/// The pause after a stroke (`state = WAITING; timer = 500`).
pub const PAUSE: i32 = 500;
/// A deployer that can't activate (`shouldActivate` false, overflow, a held item that fails the
/// filter) waits `getTimerSpeed() * 10`.
pub const BACKOFF: i32 = 10;
/// The overflow takes up to eleven stacks (`overflowItems.size() > 10` stops the shuffle).
pub const OVERFLOW: usize = 11;
/// The most the hand reaches (`Math.min(.., 21 / 16f)`).
pub const MAX_OFFSET: f32 = 21.0 / 16.0;

// --- the stroke -------------------------------------------------------------------------------

/// `DeployerBlockEntity.State`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Waiting,
    Expanding,
    Retracting,
    Dumping,
}

impl State {
    /// `NBTHelper.writeEnum`: the constant's name.
    pub const fn name(self) -> &'static str {
        match self {
            State::Waiting => "WAITING",
            State::Expanding => "EXPANDING",
            State::Retracting => "RETRACTING",
            State::Dumping => "DUMPING",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [State::Waiting, State::Expanding, State::Retracting, State::Dumping]
            .into_iter()
            .find(|s| s.name() == name)
    }
}

/// `DeployerBlockEntity.Mode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Punch,
    Use,
}

impl Mode {
    pub const fn name(self) -> &'static str {
        match self {
            Mode::Punch => "PUNCH",
            Mode::Use => "USE",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [Mode::Punch, Mode::Use].into_iter().find(|m| m.name() == name)
    }

    /// `changeMode`.
    pub const fn toggled(self) -> Self {
        match self {
            Mode::Punch => Mode::Use,
            Mode::Use => Mode::Punch,
        }
    }
}

/// `getTimerSpeed`: `speed == 0 ? 0 : clamp(abs(speed * 2), 8, 512)`, as an int.
pub fn timer_speed(rpm: f32) -> i32 {
    if rpm == 0.0 {
        0
    } else {
        (rpm * 2.0).abs().clamp(8.0, 512.0) as i32
    }
}

/// The part of a Deployer's block entity the stroke is made of.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke {
    pub state: State,
    pub timer: i32,
    /// How far the hand reaches at the end of the stroke, from `start`.
    pub reach: f32,
    pub fist_bump: bool,
}

impl Default for Stroke {
    /// `state = WAITING`, nothing counting.
    fn default() -> Self {
        Stroke { state: State::Waiting, timer: 0, reach: 0.0, fist_bump: false }
    }
}

impl Stroke {
    /// The start of `tick` after the speed check: a timer still running counts down by the timer
    /// speed and ends the tick. True when it did.
    pub fn count_down(&mut self, rpm: f32) -> bool {
        if self.timer > 0 {
            self.timer -= timer_speed(rpm);
            true
        } else {
            false
        }
    }

    /// `timer = getTimerSpeed() * 10`: wait before looking again.
    pub fn back_off(&mut self, rpm: f32) {
        self.timer = timer_speed(rpm) * BACKOFF;
    }

    /// `start`: the hand sets out, to a depth `reach` found by the ray (see [`front_hit`]).
    pub fn start(&mut self, reach: f32) {
        self.state = State::Expanding;
        self.reach = reach;
        self.timer = STROKE;
    }

    /// `startFistBump`, for each of the two deployers.
    pub fn start_fist_bump(&mut self, reach: f32) {
        self.fist_bump = true;
        self.reach = reach;
        self.timer = STROKE;
        self.state = State::Expanding;
    }

    /// The end of the extending hand (`activate()` has run): `state = RETRACTING; timer = 1000`.
    pub fn expanded(&mut self) {
        self.state = State::Retracting;
        self.timer = STROKE;
    }

    /// The end of the retracting hand: `state = WAITING; timer = 500`.
    pub fn retracted(&mut self) {
        self.state = State::Waiting;
        self.timer = PAUSE;
    }

    /// `triggerFistBump` on the partner that is still extended and has run its timer out: it
    /// retracts at once.
    pub fn bumped(&mut self) {
        self.fist_bump = false;
        self.state = State::Retracting;
        self.timer = STROKE;
    }
}

/// `getHandPose`: which model the hand is drawn with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pose {
    Pointing,
    Punching,
    Holding,
}

impl Pose {
    pub fn of(mode: Mode, holding: bool) -> Self {
        match mode {
            Mode::Punch => Pose::Punching,
            Mode::Use if holding => Pose::Holding,
            Mode::Use => Pose::Pointing,
        }
    }

    /// `AllPartialModels.DEPLOYER_HAND_*`, without the `create:` namespace.
    pub const fn model(self) -> &'static str {
        match self {
            Pose::Pointing => "block/deployer/hand_pointing",
            Pose::Punching => "block/deployer/hand_punching",
            Pose::Holding => "block/deployer/hand_holding",
        }
    }

    /// `getHandOffset`'s `handLength`.
    pub const fn length(self) -> f32 {
        match self {
            Pose::Pointing => 0.0,
            Pose::Holding => 4.0 / 16.0,
            Pose::Punching => 3.0 / 16.0,
        }
    }
}

/// `getHandOffset` for a hand in `state` with `timer` left (as a client sees it: the saved timer
/// less the timer speed times the ticks since, partial tick included).
pub fn hand_offset(state: State, timer: f32, fist_bump: bool, reach: f32, pose: Pose) -> f32 {
    let mut progress = 0.0;
    if state == State::Expanding {
        progress = 1.0 - timer / STROKE as f32;
        if fist_bump {
            progress *= progress;
        }
    }
    if state == State::Retracting {
        progress = timer / STROKE as f32;
    }
    (progress.clamp(0.0, 1.0) * (reach + pose.length())).min(MAX_OFFSET)
}

/// `start`: the hand reaches `0.5 + min(distance, .75)` where `distance` is how far from the ray's
/// origin (`centre + 1.5 * facing`) the ray to `centre + 2.5 * facing` meets the outline.
pub fn reach_of(distance: f32) -> f32 {
    0.5 + distance.min(0.75)
}

// --- the shape and the ray ---------------------------------------------------------------------

/// `AllShapes.DEPLOYER_INTERACTION` for `facing`, in blocks: `CASING_12PX.get(UP)` (the casing,
/// 12 texels deep at the back) plus `SIX_VOXEL_POLE.get(Y)` (the pole, 6 texels square, through
/// the block), turned with `forDirectional(UP)`. Every box is square across, so a turn only
/// moves its depth onto the facing's axis.
pub fn interaction_boxes(facing: Dir) -> [[f64; 6]; 2] {
    [turned([0.0, 0.0, 0.0, 1.0, 12.0 / 16.0, 1.0], facing), turned([5.0 / 16.0, 0.0, 5.0 / 16.0, 11.0 / 16.0, 1.0, 11.0 / 16.0], facing)]
}

/// `AllShapes.CASING_12PX` for `facing`: what the Deployer collides with.
pub fn collision_boxes(facing: Dir) -> [[f64; 6]; 1] {
    [turned([0.0, 0.0, 0.0, 1.0, 12.0 / 16.0, 1.0], facing)]
}

/// A box written for facing up, turned so up points along `facing` (its other two sides keep the
/// bounds the box has across).
fn turned(box_: [f64; 6], facing: Dir) -> [f64; 6] {
    let [x0, y0, _, x1, y1, _] = box_;
    let (lo, hi) = if facing.sign() > 0 { (y0, y1) } else { (1.0 - y1, 1.0 - y0) };
    let mut out = [x0, x0, x0, x1, x1, x1];
    out[facing.axis().index()] = lo;
    out[facing.axis().index() + 3] = hi;
    out
}

/// Where a ray from `origin` along `direction` first meets `box_` (slab method): the distance
/// along `direction`, or `None`. A ray that starts inside meets it at 0.
pub fn ray_box(origin: [f64; 3], direction: [f64; 3], box_: [f64; 6]) -> Option<f64> {
    let (mut near, mut far) = (f64::NEG_INFINITY, f64::INFINITY);
    for axis in 0..3 {
        let (lo, hi) = (box_[axis], box_[axis + 3]);
        if direction[axis].abs() < 1e-12 {
            if origin[axis] < lo || origin[axis] > hi {
                return None;
            }
            continue;
        }
        let (a, b) = ((lo - origin[axis]) / direction[axis], (hi - origin[axis]) / direction[axis]);
        near = near.max(a.min(b));
        far = far.min(a.max(b));
    }
    (near <= far && far >= 0.0).then_some(near.max(0.0))
}

/// The nearest distance a ray meets any of `boxes`.
pub fn ray_boxes(origin: [f64; 3], direction: [f64; 3], boxes: &[[f64; 6]]) -> Option<f64> {
    boxes
        .iter()
        .filter_map(|b| ray_box(origin, direction, *b))
        .min_by(f64::total_cmp)
}

/// The direction a player looks along (`Entity.calculateViewVector`) for `yaw` and `pitch` in degrees.
pub fn view_vector(yaw: f32, pitch: f32) -> [f64; 3] {
    let (yaw, pitch) = (f64::from(yaw).to_radians(), f64::from(pitch).to_radians());
    [-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()]
}

/// `DeployerBlock.useItemOn` and `onWrenched`: how far back from the casing's rear face the
/// click met the block, along the facing, in blocks. A click at 0.75 or more is on the hand.
pub fn along_from_back(facing: Dir, hit: [f64; 3], block: [i32; 3]) -> f64 {
    let axis = facing.axis().index();
    let local = hit[axis] - f64::from(block[axis]);
    if facing.sign() > 0 { local } else { 1.0 - local }
}

/// `location.length() < .75f` passes to the default interaction; anything else is the hand.
/// (The casing's own top face, at exactly 0.75, counts as the hand, as it does in Create; the
/// tolerance is for the ray's rounding.)
pub fn is_hand(along: f64) -> bool {
    along >= 0.75 - 1e-6
}

/// The hit of the ray `activateInner` and `start` shoot through the cell two blocks in front of
/// the Deployer: from the near face of that cell to its far face, through its middle, against
/// the boxes the block there has (in the block's own coordinates, 0 to 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrontHit {
    /// Whether the ray met something (otherwise its end counts as the hit, as `BlockHitResult.miss`).
    pub hit: bool,
    /// From the ray's origin to the hit, 0 to 1.
    pub distance: f32,
    /// Where the hit is along the facing's axis, in the cell's own coordinates.
    pub along: f64,
}

pub fn front_hit(facing: Dir, boxes: &[[f64; 6]]) -> FrontHit {
    let axis = facing.axis().index();
    let positive = facing.sign() > 0;
    let mut best: Option<(f64, f64)> = None;
    for b in boxes {
        let lateral = (0..3).filter(|i| *i != axis).all(|i| b[i] <= 0.5 && b[i + 3] >= 0.5);
        if !lateral {
            continue;
        }
        // The ray runs from along 0 (or 1 facing the other way) across the cell.
        let (entry, distance) = if positive { (b[axis], b[axis]) } else { (b[axis + 3], 1.0 - b[axis + 3]) };
        let reaches = if positive { b[axis + 3] > 0.0 } else { b[axis] < 1.0 };
        if reaches && best.is_none_or(|(d, _)| distance < d) {
            best = Some((distance, entry));
        }
    }
    match best {
        Some((distance, along)) => FrontHit { hit: true, distance: distance as f32, along },
        None => FrontHit { hit: false, distance: 1.0, along: if positive { 1.0 } else { 0.0 } },
    }
}

// --- the block's state ---------------------------------------------------------------------------

/// `DirectionalAxisKineticBlock.getRotationAxis`: a deployer's shaft runs across its facing.
pub fn rotation_axis(facing: Dir, along_first: bool) -> Axis {
    match facing.axis() {
        Axis::X => if along_first { Axis::Y } else { Axis::Z },
        Axis::Y => if along_first { Axis::X } else { Axis::Z },
        Axis::Z => if along_first { Axis::X } else { Axis::Y },
    }
}

/// `hasShaftTowards`: both ends of the rotation axis.
pub fn shaft_faces(facing: Dir, along_first: bool) -> [Dir; 2] {
    match rotation_axis(facing, along_first) {
        Axis::X => [Dir::West, Dir::East],
        Axis::Y => [Dir::Down, Dir::Up],
        Axis::Z => [Dir::North, Dir::South],
    }
}

/// `Direction.fromYRot`: the horizontal direction a player turned `yaw` degrees looks.
pub fn horizontal_direction(yaw: f32) -> Dir {
    match (((yaw / 90.0 + 0.5).floor() as i32) & 3) as u8 {
        0 => Dir::South,
        1 => Dir::West,
        2 => Dir::North,
        _ => Dir::East,
    }
}

/// `DirectionalAxisKineticBlock.getStateForPlacement` without the shaft beside it that Create
/// would line up with: `(facing, axis_along_first)`. `look` is `getNearestLookingDirection`,
/// `horizontal` is `getHorizontalDirection`.
pub fn placement(look: Dir, horizontal: Dir, sneaking: bool) -> (Dir, bool) {
    let mut facing = look.opposite();
    if sneaking {
        facing = facing.opposite();
    }
    let along_first = if facing.axis() == Axis::Y {
        horizontal.axis() == Axis::X
    } else {
        facing.axis() == Axis::Z
    };
    (facing, along_first)
}

/// `IWrenchable.getRotatedBlockState` for `facing` and `axis_along_first`, clicked on `clicked`.
pub fn wrench(facing: Dir, along_first: bool, clicked: Dir) -> (Dir, bool) {
    if facing.axis() == clicked.axis() {
        return (facing, !along_first);
    }
    let (mut next, mut along) = (facing, along_first);
    loop {
        next = super::drill::clockwise(next, clicked.axis());
        if clicked.axis() == Axis::Y {
            along = !along;
        }
        if next.axis() != clicked.axis() {
            return (next, along);
        }
    }
}

// --- what the Deployer says to a belt ------------------------------------------------------------

/// What `BeltDeployerCallbacks` looks at.
#[derive(Clone, Copy, Debug)]
pub struct BeltView {
    pub rpm: f32,
    pub mode: Mode,
    pub facing: Dir,
    pub state: State,
    pub redstone_locked: bool,
    /// The fake player's main hand is empty.
    pub holding_nothing: bool,
}

/// `onItemReceived`: what the Deployer says of an item that reached the middle of the block two
/// under it, and whether it starts a stroke (`blockEntity.start()`). `has_recipe` is asked only
/// when it can matter.
pub fn on_item_received(v: &BeltView, has_recipe: impl FnOnce() -> bool) -> (Processing, bool) {
    if v.rpm == 0.0 || v.mode == Mode::Punch || v.facing != Dir::Down {
        return (Processing::Pass, false);
    }
    if v.state != State::Waiting {
        return (Processing::Hold, false);
    }
    if v.redstone_locked {
        return (Processing::Pass, false);
    }
    if v.holding_nothing {
        return (Processing::Hold, false);
    }
    if !has_recipe() {
        return (Processing::Pass, false);
    }
    (Processing::Hold, true)
}

/// `whenItemHeld`, without the stroke's end (the glue applies the recipe in the Deployer's own
/// tick, where `state == RETRACTING && timer == 1000` is the moment it ends): what the Deployer
/// says of a held item, and whether it starts a stroke.
pub fn while_item_held(v: &BeltView, has_recipe: impl FnOnce() -> bool) -> (Processing, bool) {
    if v.rpm == 0.0 || v.facing != Dir::Down {
        return (Processing::Pass, false);
    }
    if v.holding_nothing {
        return (Processing::Hold, false);
    }
    if !has_recipe() {
        return (Processing::Pass, false);
    }
    if v.state == State::Waiting {
        if v.redstone_locked {
            return (Processing::Pass, false);
        }
        return (Processing::Hold, true);
    }
    (Processing::Hold, false)
}

/// `AllItemTags.UPRIGHT_ON_BELT` of Create's datagen: what stands up on a belt (not read: the
/// angle of what a Deployer makes is random, except for these).
pub const UPRIGHT_ON_BELT: [&str; 11] = [
    "minecraft:glass_bottle",
    "minecraft:potion",
    "minecraft:splash_potion",
    "minecraft:lingering_potion",
    "minecraft:honey_bottle",
    "minecraft:cake",
    "minecraft:bowl",
    "minecraft:mushroom_stew",
    "minecraft:rabbit_stew",
    "minecraft:beetroot_soup",
    "minecraft:suspicious_stew",
];

// --- `shouldActivate` ----------------------------------------------------------------------------

/// What `DeployerHandler.shouldActivate` asks: whether a deployer that holds `held` and faces
/// `facing` should start a stroke on the block `target`. `held_block` is the block the held item
/// places (the item's own id for the blocks that share it), `bucket_fluid` the fluid block the
/// held bucket holds, `transports` whether the target holds items moving on it (a belt or a depot).
pub fn should_activate(
    held: Option<&str>,
    held_block: Option<&str>,
    bucket_fluid: Option<&str>,
    target: &str,
    facing: Dir,
    transports: bool,
) -> bool {
    if held_block.is_some_and(|block| block == target) {
        return false;
    }
    if bucket_fluid.is_some_and(|fluid| fluid == target) {
        return false;
    }
    if held.is_some() && facing == Dir::Down && transports {
        return false;
    }
    true
}

/// `BucketItem.content` of a vanilla bucket, as the block its fluid fills: only the source
/// fluids the game has (`minecraft:water_bucket`, `minecraft:lava_bucket`).
pub fn bucket_fluid(item: &str) -> Option<&'static str> {
    match item {
        "minecraft:water_bucket" => Some("minecraft:water"),
        "minecraft:lava_bucket" => Some("minecraft:lava"),
        _ => None,
    }
}

// --- the fake player's pose ----------------------------------------------------------------------

/// `direction.toYRot()` for `player.setYRot` (`(data2d & 3) * 90`: 270 for up and down).
pub fn fake_yaw(facing: Dir) -> f32 {
    match facing {
        Dir::South => 0.0,
        Dir::West => 90.0,
        Dir::North => 180.0,
        Dir::East | Dir::Up | Dir::Down => 270.0,
    }
}

/// `player.setXRot`: -90 up, 90 down, level otherwise.
pub fn fake_pitch(facing: Dir) -> f32 {
    match facing {
        Dir::Up => -90.0,
        Dir::Down => 90.0,
        _ => 0.0,
    }
}

// --- items worn and swung ------------------------------------------------------------------------

/// The item's entry in the table of what the game says of items, if it has one.
pub fn item(id: &str) -> Option<&'static Item> {
    facts::ITEMS
        .binary_search_by(|item| item.id.cmp(id))
        .ok()
        .map(|i| &facts::ITEMS[i])
}

/// A player's own attack damage, before what the item adds (`Attributes.ATTACK_DAMAGE` base).
pub const FIST_DAMAGE: f32 = 1.0;

/// What `player.attack` deals with `held` in the main hand: the base 1 plus the item's mainhand
/// attack damage modifier, at full strength (`getCurrentItemAttackStrengthDelay` is 1/64).
pub fn attack_damage(held: Option<&str>) -> f32 {
    FIST_DAMAGE + held.and_then(item).map_or(0.0, |item| item.attack)
}

/// What an attack costs the held item (`Item.hurtEnemy` through `Weapon.itemDamagePerAttack`),
/// 0 for an item that isn't a weapon.
pub fn wear_per_attack(held: &str) -> u32 {
    item(held).filter(|item| item.weapon).map_or(0, |item| item.damage_per_attack)
}

/// What breaking a block costs the held item (`ItemStack.mineBlock`: `Tool.damagePerBlock` unless
/// the block has no hardness), 0 for an item that isn't a tool.
pub fn wear_per_block(held: &str, hardness: f32) -> u32 {
    if hardness == 0.0 {
        return 0;
    }
    item(held).filter(|item| item.tool).map_or(0, |item| item.damage_per_block)
}

/// `ItemStack.getMaxDamage`: 0 for an item that doesn't wear.
pub fn max_damage(held: &str) -> u32 {
    item(held).map_or(0, |item| item.max_damage)
}

/// What wearing a stack by `amount` comes to (`hurtAndBreak`): its new damage, or `None` when it
/// breaks.
pub fn worn(damage: u32, max: u32, amount: u32) -> Option<u32> {
    let total = damage.saturating_add(amount);
    (total < max).then_some(total)
}

// --- mining --------------------------------------------------------------------------------------

/// Whether `block` is in what a tool rule names: a block id, or `#tag` read by `in_tag`.
fn named(spec: &str, block: &str, in_tag: &impl Fn(&str) -> bool) -> bool {
    match spec.strip_prefix('#') {
        Some(tag) => in_tag(tag),
        None => spec == block,
    }
}

/// `ItemStack.getDestroySpeed` (`Tool.getMiningSpeed`): the first rule with a speed that names
/// the block; the tool's default (1) otherwise, and 1 for an item with no `Tool`.
pub fn mining_speed(held: Option<&str>, block: &str, in_tag: impl Fn(&str) -> bool) -> f32 {
    let Some(tool) = held.and_then(item).filter(|item| item.tool) else {
        return 1.0;
    };
    tool.rules
        .iter()
        .find(|rule| rule.speed != NO_SPEED && named(rule.blocks, block, &in_tag))
        .map_or(tool.default_speed, |rule| rule.speed)
}

/// `ItemStack.isCorrectToolForDrops` (`Tool.isCorrectForDrops`): the first rule that says either
/// way and names the block; false otherwise.
pub fn correct_for_drops(held: Option<&str>, block: &str, in_tag: impl Fn(&str) -> bool) -> bool {
    let Some(tool) = held.and_then(item).filter(|item| item.tool) else {
        return false;
    };
    tool.rules
        .iter()
        .find(|rule| rule.correct.is_some() && named(rule.blocks, block, &in_tag))
        .and_then(|rule| rule.correct)
        .unwrap_or(false)
}

/// `BlockBehaviour.requiresCorrectToolForDrops`, for the vanilla and Create blocks the table knows.
pub fn requires_tool(block: &str) -> bool {
    facts::REQUIRES_TOOL.binary_search(&block).is_ok()
}

/// `Player.hasCorrectToolForDrops`.
pub fn has_correct_tool(held: Option<&str>, block: &str, in_tag: impl Fn(&str) -> bool) -> bool {
    !requires_tool(block) || correct_for_drops(held, block, in_tag)
}

/// `BlockBehaviour.getDestroyProgress`: `speed / hardness / (30 with the right tool, else 100)`;
/// 0 for an unbreakable block (hardness -1).
pub fn destroy_progress(speed: f32, hardness: f32, correct_tool: bool) -> f32 {
    if hardness == -1.0 {
        return 0.0;
    }
    speed / hardness / if correct_tool { 30.0 } else { 100.0 }
}

/// `activateInner`'s multiplier: a punch counts for sixteen ticks of mining.
pub const PUNCH_TICKS: f32 = 16.0;

/// What a punch did to the block it hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Punch {
    /// `progress >= 1`: harvest the block and take the crack away.
    Break,
    /// `progress <= 0`: nothing to show, the progress is dropped.
    Reset,
    /// The progress is kept; `stage` is the crack to show when it moved (`(int)(progress * 10)`).
    Crack { progress: f32, stage: Option<i32> },
}

/// `activateInner`'s mining half: `destroy` is `getDestroyProgress`, `before` what the last punch
/// left.
pub fn punch(destroy: f32, before: f32) -> Punch {
    let progress = destroy * PUNCH_TICKS + before;
    if progress >= 1.0 {
        return Punch::Break;
    }
    if progress <= 0.0 {
        return Punch::Reset;
    }
    let stage = ((before * 10.0) as i32 != (progress * 10.0) as i32).then_some((progress * 10.0) as i32);
    Punch::Crack { progress, stage }
}

/// The sound a finished application makes: `SoundEvents.ITEM_BREAK` for a held item that was
/// used up, then `ITEM_PICKUP` (`BeltDeployerCallbacks.activate`).
pub const BREAK_SOUND: (&str, f32, f32) = ("minecraft:entity.item.break", 0.25, 1.0);
pub const PICKUP_SOUND: (&str, f32, f32) = ("minecraft:entity.item.pickup", 0.25, 0.75);
/// `ManualApplicationRecipe`: `SoundEvents.COPPER_BREAK`, volume 1, pitch 1.45.
pub const MANUAL_SOUND: (&str, f32, f32) = ("minecraft:block.copper.break", 1.0, 1.45);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_speed_is_twice_the_rpm_between_8_and_512() {
        assert_eq!(timer_speed(0.0), 0);
        assert_eq!(timer_speed(1.0), 8);
        assert_eq!(timer_speed(16.0), 32);
        assert_eq!(timer_speed(-16.0), 32);
        assert_eq!(timer_speed(256.0), 512);
        assert_eq!(timer_speed(1000.0), 512);
        assert_eq!(timer_speed(7.7), 15);
    }

    #[test]
    fn a_stroke_counts_down_extends_retracts_and_pauses() {
        let mut s = Stroke::default();
        assert!(!s.count_down(16.0));
        s.start(1.0);
        assert_eq!((s.state, s.timer), (State::Expanding, STROKE));
        // 1000 at 32 a tick: 32 ticks to run out (31.25 rounded up).
        let mut ticks = 0;
        while s.count_down(16.0) {
            ticks += 1;
        }
        assert_eq!(ticks, 32);
        assert!(s.timer <= 0);
        s.expanded();
        assert_eq!((s.state, s.timer), (State::Retracting, STROKE));
        s.retracted();
        assert_eq!((s.state, s.timer), (State::Waiting, PAUSE));
        s.back_off(16.0);
        assert_eq!(s.timer, 320);
    }

    #[test]
    fn the_hand_goes_out_and_back() {
        let pose = Pose::Holding;
        // Not moving: in.
        assert_eq!(hand_offset(State::Waiting, 0.0, false, 1.0, pose), 0.0);
        // Just set out (timer 1000): still in; all done (timer 0): out by reach + hand.
        assert_eq!(hand_offset(State::Expanding, 1000.0, false, 1.0, pose), 0.0);
        assert!((hand_offset(State::Expanding, 0.0, false, 0.5, pose) - 0.75).abs() < 1e-6);
        // Retracting from the same place.
        assert!((hand_offset(State::Retracting, 1000.0, false, 0.5, pose) - 0.75).abs() < 1e-6);
        assert_eq!(hand_offset(State::Retracting, 0.0, false, 0.5, pose), 0.0);
        // The pointing hand has no length, the punching one 3 texels.
        assert!((hand_offset(State::Expanding, 0.0, false, 1.25, Pose::Pointing) - 1.25).abs() < 1e-6);
        // Never past 21/16.
        assert_eq!(hand_offset(State::Expanding, 0.0, false, 5.0, pose), MAX_OFFSET);
        // A fist bump eases in.
        let plain = hand_offset(State::Expanding, 500.0, false, 1.0, pose);
        let bump = hand_offset(State::Expanding, 500.0, true, 1.0, pose);
        assert!(bump < plain);
    }

    #[test]
    fn the_pose_follows_the_mode_and_the_hand() {
        assert_eq!(Pose::of(Mode::Punch, false), Pose::Punching);
        assert_eq!(Pose::of(Mode::Punch, true), Pose::Punching);
        assert_eq!(Pose::of(Mode::Use, true), Pose::Holding);
        assert_eq!(Pose::of(Mode::Use, false), Pose::Pointing);
    }

    #[test]
    fn the_interaction_shape_is_a_casing_and_a_pole() {
        let up = interaction_boxes(Dir::Up);
        assert_eq!(up[0], [0.0, 0.0, 0.0, 1.0, 0.75, 1.0]);
        assert_eq!(up[1], [0.3125, 0.0, 0.3125, 0.6875, 1.0, 0.6875]);
        let down = interaction_boxes(Dir::Down);
        assert_eq!(down[0], [0.0, 0.25, 0.0, 1.0, 1.0, 1.0]);
        let east = interaction_boxes(Dir::East);
        assert_eq!(east[0], [0.0, 0.0, 0.0, 0.75, 1.0, 1.0]);
        assert_eq!(east[1], [0.0, 0.3125, 0.3125, 1.0, 0.6875, 0.6875]);
        let north = interaction_boxes(Dir::North);
        assert_eq!(north[0], [0.0, 0.0, 0.25, 1.0, 1.0, 1.0]);
        assert_eq!(collision_boxes(Dir::West)[0], [0.25, 0.0, 0.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn a_click_on_the_pole_or_the_casing_top_is_the_hand() {
        // A player looking straight down at a block facing up: hits the pole's end at the top.
        let boxes = interaction_boxes(Dir::Up);
        let t = ray_boxes([0.5, 3.0, 0.5], [0.0, -1.0, 0.0], &boxes).unwrap();
        let hit = [0.5, 3.0 - t, 0.5];
        assert!(is_hand(along_from_back(Dir::Up, hit, [0, 0, 0])));
        // Beside the pole, on the casing's top face at 0.75: still the hand.
        let t = ray_boxes([0.1, 3.0, 0.1], [0.0, -1.0, 0.0], &boxes).unwrap();
        let hit = [0.1, 3.0 - t, 0.1];
        assert!(is_hand(along_from_back(Dir::Up, hit, [0, 0, 0])));
        // The casing's side is not.
        let t = ray_boxes([-2.0, 0.4, 0.5], [1.0, 0.0, 0.0], &boxes).unwrap();
        let hit = [-2.0 + t, 0.4, 0.5];
        assert!(!is_hand(along_from_back(Dir::Up, hit, [0, 0, 0])));
        // Facing down the hand is the low end, and the back is the top.
        assert!(is_hand(along_from_back(Dir::Down, [0.5, 0.2, 0.5], [0, 0, 0])));
        assert!(!is_hand(along_from_back(Dir::Down, [0.5, 0.8, 0.5], [0, 0, 0])));
    }

    #[test]
    fn rays_meet_the_nearest_box() {
        let boxes = [[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]];
        assert_eq!(ray_boxes([0.5, 0.5, -2.0], [0.0, 0.0, 1.0], &boxes), Some(2.0));
        assert_eq!(ray_boxes([0.5, 0.5, 3.0], [0.0, 0.0, 1.0], &boxes), None);
        assert_eq!(ray_boxes([2.5, 0.5, -2.0], [0.0, 0.0, 1.0], &boxes), None);
        assert_eq!(ray_boxes([0.5, 0.5, 0.5], [0.0, 0.0, 1.0], &boxes), Some(0.0));
    }

    #[test]
    fn the_hand_reaches_to_what_is_in_front() {
        // A full block: the ray meets it at once, the hand goes the least (0.5).
        let full = [[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]];
        let hit = front_hit(Dir::East, &full);
        assert!(hit.hit);
        assert_eq!(hit.distance, 0.0);
        assert_eq!(reach_of(hit.distance), 0.5);
        // Nothing there: the ray's end, and the hand goes 0.75 further.
        let miss = front_hit(Dir::East, &[]);
        assert!(!miss.hit);
        assert_eq!(miss.distance, 1.0);
        assert_eq!(reach_of(miss.distance), 1.25);
        assert_eq!(miss.along, 1.0);
        assert_eq!(front_hit(Dir::West, &[]).along, 0.0);
        // Something that doesn't reach the middle of the cell (a low carpet beside the line) is missed.
        let carpet = [[0.0, 0.0, 0.0, 1.0, 0.0625, 1.0]];
        assert!(!front_hit(Dir::East, &carpet).hit);
        // A carpet-like floor facing down: the ray from above meets its top.
        let floor = [[0.0, 0.0, 0.0, 1.0, 0.0625, 1.0]];
        let hit = front_hit(Dir::Down, &floor);
        assert!(hit.hit);
        assert!((hit.distance - (1.0 - 0.0625)).abs() < 1e-6);
        assert!((hit.along - 0.0625).abs() < 1e-9);
        // A half block it meets at its near face when the ray comes from the far side.
        let top_slab = [[0.0, 0.5, 0.0, 1.0, 1.0, 1.0]];
        let hit = front_hit(Dir::Down, &top_slab);
        assert!(hit.hit);
        assert_eq!(hit.distance, 0.0);
        assert_eq!(hit.along, 1.0);
    }

    #[test]
    fn the_shaft_runs_across_the_facing() {
        assert_eq!(rotation_axis(Dir::East, true), Axis::Y);
        assert_eq!(rotation_axis(Dir::East, false), Axis::Z);
        assert_eq!(rotation_axis(Dir::North, true), Axis::X);
        assert_eq!(rotation_axis(Dir::South, false), Axis::Y);
        assert_eq!(rotation_axis(Dir::Up, true), Axis::X);
        assert_eq!(rotation_axis(Dir::Down, false), Axis::Z);
        assert_eq!(shaft_faces(Dir::Up, true), [Dir::West, Dir::East]);
        assert_eq!(shaft_faces(Dir::North, false), [Dir::Down, Dir::Up]);
    }

    #[test]
    fn placement_faces_the_player_and_aligns_the_shaft() {
        // Looking south at a wall: the deployer faces north, shaft along first for Z.
        assert_eq!(placement(Dir::South, Dir::South, false), (Dir::North, true));
        assert_eq!(placement(Dir::South, Dir::South, true), (Dir::South, true));
        // East or west: not along the first coordinate.
        assert_eq!(placement(Dir::East, Dir::East, false), (Dir::West, false));
        // Looking down: faces up, along first when the player faces east or west.
        assert_eq!(placement(Dir::Down, Dir::East, false), (Dir::Up, true));
        assert_eq!(placement(Dir::Down, Dir::North, false), (Dir::Up, false));
        assert_eq!(horizontal_direction(0.0), Dir::South);
        assert_eq!(horizontal_direction(90.0), Dir::West);
        assert_eq!(horizontal_direction(180.0), Dir::North);
        assert_eq!(horizontal_direction(-90.0), Dir::East);
    }

    #[test]
    fn the_wrench_turns_about_the_face_clicked() {
        // On the facing's own axis: the shaft turns a quarter.
        assert_eq!(wrench(Dir::North, true, Dir::South), (Dir::North, false));
        assert_eq!(wrench(Dir::Up, false, Dir::Down), (Dir::Up, true));
        // Round the vertical axis: a quarter turn of the facing, and the shaft with it.
        assert_eq!(wrench(Dir::North, true, Dir::Up), (Dir::East, false));
        // Round a horizontal one: no change of the alignment.
        assert_eq!(wrench(Dir::Up, true, Dir::East), (Dir::South, true));
        // Four turns come home.
        let mut state = (Dir::West, true);
        for _ in 0..4 {
            state = wrench(state.0, state.1, Dir::Up);
        }
        assert_eq!(state, (Dir::West, true));
    }

    fn view(state: State) -> BeltView {
        BeltView {
            rpm: 16.0,
            mode: Mode::Use,
            facing: Dir::Down,
            state,
            redstone_locked: false,
            holding_nothing: false,
        }
    }

    #[test]
    fn a_belt_item_starts_a_stroke_when_the_deployer_has_a_recipe() {
        assert_eq!(on_item_received(&view(State::Waiting), || true), (Processing::Hold, true));
        assert_eq!(on_item_received(&view(State::Waiting), || false), (Processing::Pass, false));
        assert_eq!(on_item_received(&view(State::Retracting), || false), (Processing::Hold, false));
        let mut v = view(State::Waiting);
        v.holding_nothing = true;
        assert_eq!(on_item_received(&v, || false), (Processing::Hold, false));
        v = view(State::Waiting);
        v.redstone_locked = true;
        assert_eq!(on_item_received(&v, || true), (Processing::Pass, false));
        v = view(State::Waiting);
        v.mode = Mode::Punch;
        assert_eq!(on_item_received(&v, || true), (Processing::Pass, false));
        v = view(State::Waiting);
        v.facing = Dir::Up;
        assert_eq!(on_item_received(&v, || true), (Processing::Pass, false));
        v = view(State::Waiting);
        v.rpm = 0.0;
        assert_eq!(on_item_received(&v, || true), (Processing::Pass, false));
    }

    #[test]
    fn a_held_belt_item_is_held_through_the_stroke() {
        assert_eq!(while_item_held(&view(State::Waiting), || true), (Processing::Hold, true));
        assert_eq!(while_item_held(&view(State::Expanding), || true), (Processing::Hold, false));
        assert_eq!(while_item_held(&view(State::Retracting), || true), (Processing::Hold, false));
        assert_eq!(while_item_held(&view(State::Waiting), || false), (Processing::Pass, false));
        let mut v = view(State::Waiting);
        v.redstone_locked = true;
        assert_eq!(while_item_held(&v, || true), (Processing::Pass, false));
        // Punch mode doesn't let go of a held item the way it doesn't take one in.
        v = view(State::Waiting);
        v.mode = Mode::Punch;
        assert_eq!(while_item_held(&v, || true), (Processing::Hold, true));
        v = view(State::Waiting);
        v.holding_nothing = true;
        assert_eq!(while_item_held(&v, || false), (Processing::Hold, false));
    }

    #[test]
    fn it_doesnt_activate_where_it_would_do_nothing() {
        // A block item on the block it places.
        assert!(!should_activate(Some("minecraft:stone"), Some("minecraft:stone"), None, "minecraft:stone", Dir::North, false));
        assert!(should_activate(Some("minecraft:stone"), Some("minecraft:stone"), None, "minecraft:air", Dir::North, false));
        // A full bucket in its own fluid.
        assert!(!should_activate(Some("minecraft:water_bucket"), None, bucket_fluid("minecraft:water_bucket"), "minecraft:water", Dir::North, false));
        assert!(should_activate(Some("minecraft:bucket"), None, bucket_fluid("minecraft:bucket"), "minecraft:water", Dir::North, false));
        // Over a belt or a depot with something in hand: the belt callbacks drive it.
        assert!(!should_activate(Some("create:andesite_alloy"), None, None, "create:belt", Dir::Down, true));
        assert!(should_activate(None, None, None, "create:belt", Dir::Down, true));
        assert!(should_activate(Some("create:andesite_alloy"), None, None, "create:belt", Dir::North, true));
    }

    #[test]
    fn the_fake_player_looks_where_the_deployer_faces() {
        assert_eq!(fake_yaw(Dir::South), 0.0);
        assert_eq!(fake_yaw(Dir::West), 90.0);
        assert_eq!(fake_yaw(Dir::North), 180.0);
        assert_eq!(fake_yaw(Dir::East), 270.0);
        assert_eq!(fake_yaw(Dir::Up), 270.0);
        assert_eq!((fake_pitch(Dir::Up), fake_pitch(Dir::Down), fake_pitch(Dir::North)), (-90.0, 90.0, 0.0));
    }

    #[test]
    fn items_are_found_by_id_in_the_table() {
        // The table is sorted for the binary search.
        assert!(facts::ITEMS.windows(2).all(|w| w[0].id < w[1].id));
        assert!(facts::REQUIRES_TOOL.windows(2).all(|w| w[0] < w[1]));
        let iron_axe = item("minecraft:iron_axe").expect("an iron axe");
        assert!(iron_axe.tool && iron_axe.weapon);
        assert_eq!(iron_axe.max_damage, 250);
        assert!(item("minecraft:stick").is_none());
        assert_eq!(max_damage("minecraft:iron_axe"), 250);
        assert_eq!(max_damage("minecraft:stick"), 0);
    }

    #[test]
    fn attacks_are_one_plus_the_weapon() {
        // `minecraft:attribute_modifiers` of the game's own data.
        assert_eq!(attack_damage(None), 1.0);
        assert_eq!(attack_damage(Some("minecraft:stick")), 1.0);
        assert_eq!(attack_damage(Some("minecraft:iron_sword")), 6.0);
        assert_eq!(attack_damage(Some("minecraft:diamond_sword")), 7.0);
        assert_eq!(attack_damage(Some("minecraft:wooden_sword")), 4.0);
        assert_eq!(attack_damage(Some("minecraft:iron_axe")), 9.0);
        assert_eq!(wear_per_attack("minecraft:iron_sword"), 1);
        assert_eq!(wear_per_attack("minecraft:iron_axe"), 2);
        assert_eq!(wear_per_attack("minecraft:stick"), 0);
    }

    #[test]
    fn tools_dig_fast_where_they_are_the_right_tool() {
        let in_tag = |tag: &str| tag == "minecraft:mineable/pickaxe";
        assert_eq!(mining_speed(Some("minecraft:iron_pickaxe"), "minecraft:stone", in_tag), 6.0);
        assert_eq!(mining_speed(Some("minecraft:stick"), "minecraft:stone", in_tag), 1.0);
        assert_eq!(mining_speed(None, "minecraft:stone", in_tag), 1.0);
        // A pickaxe on dirt: no rule names it, so the default.
        assert_eq!(mining_speed(Some("minecraft:iron_pickaxe"), "minecraft:dirt", |_| false), 1.0);
        assert!(correct_for_drops(Some("minecraft:iron_pickaxe"), "minecraft:stone", in_tag));
        assert!(!correct_for_drops(Some("minecraft:iron_pickaxe"), "minecraft:dirt", |_| false));
        assert!(!correct_for_drops(None, "minecraft:stone", in_tag));
        // A pickaxe too weak for the block (`incorrect_for_*_tool` comes first).
        let weak = |tag: &str| tag == "minecraft:incorrect_for_wooden_tool" || tag == "minecraft:mineable/pickaxe";
        assert!(!correct_for_drops(Some("minecraft:wooden_pickaxe"), "minecraft:diamond_ore", weak));
    }

    #[test]
    fn stone_needs_a_tool_and_dirt_does_not() {
        assert!(requires_tool("minecraft:stone"));
        assert!(requires_tool("minecraft:iron_ore"));
        assert!(!requires_tool("minecraft:dirt"));
        assert!(!requires_tool("minecraft:oak_log"));
        let none = |_: &str| false;
        assert!(has_correct_tool(None, "minecraft:dirt", none));
        assert!(!has_correct_tool(None, "minecraft:stone", none));
    }

    #[test]
    fn mining_progress_follows_vanilla() {
        // Stone (1.5) by an iron pickaxe (6): 6 / 1.5 / 30 per tick, sixteen ticks a punch.
        let per_tick = destroy_progress(6.0, 1.5, true);
        assert!((per_tick - 0.13333).abs() < 1e-4);
        assert!(destroy_progress(1.0, 1.5, false) < per_tick);
        assert_eq!(destroy_progress(6.0, -1.0, true), 0.0);
        assert_eq!(punch(per_tick, 0.0), Punch::Break);
        // Obsidian (50) by the same tool: slow, and the crack builds up.
        let slow = destroy_progress(6.0, 50.0, true);
        let Punch::Crack { progress, stage } = punch(slow, 0.0) else { panic!("a crack") };
        assert!((progress - 0.064).abs() < 1e-4);
        assert_eq!(stage, None);
        // A crack that moves from 0.35 to 0.414 shows stage 4.
        let Punch::Crack { progress: next, stage } = punch(slow, 0.35) else { panic!("a crack") };
        assert!((next - 0.414).abs() < 1e-4);
        assert_eq!(stage, Some(4));
        // Nothing to mine with: nothing happens.
        assert_eq!(punch(0.0, 0.0), Punch::Reset);
    }

    #[test]
    fn tools_wear_by_what_the_data_says() {
        assert_eq!(wear_per_block("minecraft:iron_pickaxe", 1.5), 1);
        assert_eq!(wear_per_block("minecraft:iron_sword", 1.5), 2);
        assert_eq!(wear_per_block("minecraft:iron_pickaxe", 0.0), 0);
        assert_eq!(wear_per_block("minecraft:stick", 1.5), 0);
        assert_eq!(worn(0, 250, 1), Some(1));
        assert_eq!(worn(249, 250, 1), None);
        assert_eq!(worn(248, 250, 1), Some(249));
    }
}
