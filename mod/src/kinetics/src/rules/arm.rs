//! The Mechanical Arm's rules (`ArmBlockEntity`, `ArmAngleTarget`, `ArmRenderer`), from Create
//! `mc1.21.1/dev` at 4148fd8 (MIT, the Creators of Create). No game types: the world is reached
//! through [`Env`], so the whole cycle (search, move, collect, move, deposit) runs in unit tests.
//!
//! The points an arm works with, what each takes and gives, and the selection a player makes
//! with the item are in `arm_points`.

use crate::{Dir, Pos};

/// `CLogistics.mechanicalArmRange`: how far (in blocks, exclusive) a point may be from the arm.
pub const RANGE: i32 = 5;
/// `AllBlocks.MECHANICAL_ARM`: `CStress.setImpact(2.0)`.
pub const IMPACT: f32 = 2.0;
/// `SmartBlockEntity.setLazyTickRate(10)`: `lazyTick` runs once every 11 ticks.
pub const LAZY_PERIOD: i64 = 11;
/// Create's arm looks for an input every tick; here every second tick while it has nothing to do
/// (each look reads every point), so a waiting arm starts up to a tick late.
pub const SEARCH_RATE: i64 = 2;

/// Whether the arm may sit out this tick: it is at rest searching (nothing moves or gets decided
/// but the search, and the lazy tick's) and neither the search nor the lazy tick is due.
pub fn may_skip(phase: Phase, progress: f32, search_due: bool, lazy_due: bool) -> bool {
    progress >= 1.0
        && matches!(phase, Phase::SearchInputs | Phase::Dancing | Phase::SearchOutputs)
        && !lazy_due
        && !(search_due && phase != Phase::SearchOutputs)
}

/// `ArmBlockEntity.Phase`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    SearchInputs,
    MoveToInput,
    SearchOutputs,
    MoveToOutput,
    Dancing,
}

impl Phase {
    const ALL: [Phase; 5] = [
        Phase::SearchInputs,
        Phase::MoveToInput,
        Phase::SearchOutputs,
        Phase::MoveToOutput,
        Phase::Dancing,
    ];

    /// `NBTHelper.writeEnum`: the constant's name.
    pub const fn name(self) -> &'static str {
        match self {
            Phase::SearchInputs => "SEARCH_INPUTS",
            Phase::MoveToInput => "MOVE_TO_INPUT",
            Phase::SearchOutputs => "SEARCH_OUTPUTS",
            Phase::MoveToOutput => "MOVE_TO_OUTPUT",
            Phase::Dancing => "DANCING",
        }
    }

    /// `NBTHelper.readEnum`: the first constant for an unknown name.
    pub fn from_name(name: &str) -> Self {
        Self::ALL.into_iter().find(|p| p.name() == name).unwrap_or(Phase::SearchInputs)
    }
}

/// `ArmBlockEntity.SelectionMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionMode {
    RoundRobin,
    ForcedRoundRobin,
    PreferFirst,
}

impl SelectionMode {
    const ALL: [SelectionMode; 3] =
        [SelectionMode::RoundRobin, SelectionMode::ForcedRoundRobin, SelectionMode::PreferFirst];

    /// `ScrollOptionBehaviour`'s saved `ScrollValue`.
    pub fn from_ordinal(value: i32) -> Self {
        usize::try_from(value).ok().and_then(|i| Self::ALL.get(i)).copied().unwrap_or(Self::ALL[0])
    }

    pub fn ordinal(self) -> i32 {
        Self::ALL.iter().position(|m| *m == self).unwrap_or(0) as i32
    }

    /// The next mode, as the Wrench steps it here (Create scrolls its value box).
    pub fn next(self) -> Self {
        Self::ALL[(self.ordinal() as usize + 1) % Self::ALL.len()]
    }

    /// `create.mechanical_arm.selection_mode.*` in `en_us`.
    pub const fn title(self) -> &'static str {
        match self {
            SelectionMode::RoundRobin => "Round Robin",
            SelectionMode::ForcedRoundRobin => "Forced Round Robin",
            SelectionMode::PreferFirst => "Prefer First Target",
        }
    }
}

// --- movement -----------------------------------------------------------------------------------

/// How far the arm moves towards its point each tick (`tickMovementProgress`): the speed up to
/// 256 RPM, over 1024.
pub fn rate(speed: f32) -> f32 {
    speed.abs().min(256.0) / 1024.0
}

/// `chasedPointProgress` at game time `now` plus `partial` ticks, given that it was `start` at game
/// time `since` and the arm has turned at `speed` since: the server and the clients share this,
/// so nothing is written between transitions.
pub fn progress_at(start: f32, since: i64, speed: f32, now: i64, partial: f32) -> f32 {
    let elapsed = ((now - since) as f32 + partial).max(0.0);
    (start + elapsed * rate(speed)).clamp(0.0, 1.0)
}

// --- the angles the arm takes to reach a point (`ArmAngleTarget`) --------------------------------

/// The four joint angles, in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Angles {
    pub base: f32,
    pub lower: f32,
    pub upper: f32,
    pub head: f32,
}

/// `ArmAngleTarget.NO_TARGET`: the arm folded at rest.
pub const NO_TARGET: Angles = Angles { base: 0.0, lower: 135.0, upper: 45.0, head: 0.0 };

impl Angles {
    pub fn to_array(self) -> [f32; 4] {
        [self.base, self.lower, self.upper, self.head]
    }

    pub fn from_array(a: [f32; 4]) -> Self {
        Self { base: a[0], lower: a[1], upper: a[2], head: a[3] }
    }
}

/// `AngleHelper.deg`: radians to degrees as a float, 0 for 0.
fn deg(radians: f64) -> f32 {
    if radians == 0.0 {
        return 0.0;
    }
    (radians * 180.0 / std::f64::consts::PI) as f32
}

/// `VecHelper.rotate(vec, deg, Axis.X)` (axis 0) or `Axis.Y` (axis 1).
fn rotate(v: [f64; 3], degrees: f64, axis: u8) -> [f64; 3] {
    if degrees == 0.0 {
        return v;
    }
    let angle = (degrees / 180.0 * std::f64::consts::PI) as f32;
    let (sin, cos) = (f64::from(angle.sin()), f64::from(angle.cos()));
    let [x, y, z] = v;
    if axis == 0 {
        [x, y * cos - z * sin, z * cos + y * sin]
    } else {
        [x * cos + z * sin, y, z * cos - x * sin]
    }
}

/// `new ArmAngleTarget(armPos, pointTarget, clawFacing, ceiling)`: the two arm segments (14 and 15
/// sixteenths) reach the point backed off half a block against the claw's facing, by the law of
/// cosines; the head then tilts to point at the point itself.
pub fn target_angles(arm: Pos, point: [f64; 3], claw: Dir, ceiling: bool) -> Angles {
    let origin = [
        f64::from(arm[0]) + 0.5,
        f64::from(arm[1]) + 0.5 + if ceiling { -6.0 / 16.0 } else { 6.0 / 16.0 },
        f64::from(arm[2]) + 0.5,
    ];
    let back = claw.opposite().normal();
    let reach = [0, 1, 2].map(|i| point[i] + f64::from(back[i]) * 0.5);
    let mut diff = [0, 1, 2].map(|i| reach[i] - origin[i]);
    let horizontal = (diff[0] * diff[0] + diff[2] * diff[2]).sqrt() as f32;

    let mut base = deg(diff[0].atan2(diff[2])) + 180.0;
    if ceiling {
        diff[1] = -diff[1];
        base = 180.0 - base;
    }
    let alpha_offset = deg(diff[1].atan2(f64::from(horizontal)));

    let a = 14.0f32 / 16.0;
    let b = 15.0f32 / 16.0;
    let (a2, b2) = (a * a, b * b);
    let length = ((diff[1] * diff[1]) as f32 + horizontal * horizontal)
        .sqrt()
        .clamp(1.0 / 8.0, a + b);
    let length2 = length * length;
    let alpha_ratio = (-b2 + a2 + length2) / (2.0 * a * length);
    let mut alpha = deg(f64::from(alpha_ratio).acos()) + alpha_offset;
    let beta_ratio = (-length2 + a2 + b2) / (2.0 * b * a);
    let mut beta = deg(f64::from(beta_ratio).acos());
    if alpha.is_nan() {
        alpha = 0.0;
    }
    if beta.is_nan() {
        beta = 0.0;
    }

    let mut head = rotate([0.0, f64::from(b), 0.0], f64::from(beta + 180.0), 0);
    head[1] += f64::from(a);
    head = rotate(head, f64::from(alpha - 90.0), 0);
    head = rotate(head, f64::from(base), 1);
    head = rotate(head, if ceiling { 180.0 } else { 0.0 }, 0);
    let head_pos = [0, 1, 2].map(|i| head[i] + origin[i]);
    let mut head_diff = [0, 1, 2].map(|i| point[i] - head_pos[i]);
    if ceiling {
        head_diff[1] = -head_diff[1];
    }
    let horizontal_head = (head_diff[0] * head_diff[0] + head_diff[2] * head_diff[2]).sqrt() as f32;
    let head_angle = (f64::from(alpha) + f64::from(beta) + 135.0
        - f64::from(deg(head_diff[1].atan2(f64::from(horizontal_head))))) as f32;
    Angles { base, lower: alpha, upper: beta, head: -head_angle }
}

/// `AngleHelper.getShortestAngleDiff`.
pub fn shortest_angle_diff(current: f32, target: f32) -> f32 {
    let (c, t) = (current % 360.0, target % 360.0);
    (((t - c) % 360.0 + 540.0) % 360.0) - 180.0
}

/// `AngleHelper.angleLerp`.
pub fn angle_lerp(pct: f32, current: f32, target: f32) -> f32 {
    current + shortest_angle_diff(current, target) * pct
}

/// Where the arm came from and is going, as a client animates it (`ArmBlockEntity.previousTarget`,
/// `previousBaseAngle`, the targeted point's angles).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    /// The angles of the point the arm left (`NO_TARGET` for none).
    pub previous: Angles,
    /// `previousBaseAngle`: the base angle of the last point the arm left (it keeps it while idle).
    pub previous_base: f32,
    /// The point being moved to, if the phase has one.
    pub target: Option<Angles>,
}

impl Motion {
    pub const REST: Motion = Motion { previous: NO_TARGET, previous_base: 0.0, target: None };

    /// What `ArmBlockEntity.read` does when the phase or the target index changes: the point
    /// the arm was at (if it was moving to one) becomes the previous target.
    pub fn after(self, left: Option<Angles>, now_target: Option<Angles>) -> Motion {
        Motion {
            previous: left.unwrap_or(NO_TARGET),
            previous_base: left.map_or(self.previous_base, |a| a.base),
            target: now_target,
        }
    }
}

/// The four angles `tickMovementProgress` sets at `progress`: the base turns the shortest way
/// from where it was to the point, while the lower arm, upper arm and head first fold back to
/// rest (the first half) and then unfold to the point.
pub fn pose(progress: f32, motion: &Motion) -> Angles {
    let target = motion.target;
    let base = angle_lerp(progress, motion.previous_base, target.map_or(motion.previous_base, |t| t.base));
    let (from, to) = if progress < 0.5 {
        (motion.previous, NO_TARGET)
    } else {
        (NO_TARGET, target.unwrap_or(NO_TARGET))
    };
    let p = if progress == 1.0 { 1.0 } else { (progress % 0.5) * 2.0 };
    let lerp = |a: f32, b: f32| a + (b - a) * p;
    Angles {
        base,
        lower: lerp(from.lower, to.lower),
        upper: lerp(from.upper, to.upper),
        head: angle_lerp(p, from.head % 360.0, to.head % 360.0),
    }
}

/// What `ArmRenderer` hands its transforms: the base angle, the lower arm minus 135, the upper
/// arm minus 90 and the head angle.
pub fn render_angles(a: Angles) -> [f32; 4] {
    [a.base, a.lower - 135.0, a.upper - 90.0, a.head]
}

/// `ArmRenderer`'s dance while `DANCING` at a nonzero speed: the same four numbers from the
/// render time (`getRenderTime + hashCode % 64`), and the colour (`Color.rainbowColor(ticks *
/// 100)`) of the arm parts.
pub fn rave(time: f32, salt: i32, ticks: i64) -> ([f32; 4], u32) {
    let t = time + (salt % 64) as f32;
    let mix = |p: f32, a: f32, b: f32| a + (b - a) * p;
    let lower = mix(((t / 4.0).sin() + 1.0) / 2.0, -45.0, 15.0);
    let upper = mix(((t / 8.0).sin() + 1.0) / 4.0, -45.0, 95.0);
    ([(t * 10.0) % 360.0, lower, upper, -lower], rainbow_color((ticks * 100) as i32))
}

/// Catnip's `Color.rainbowColor`: a hue round the wheel by `step` (1536 to a turn), 0xRRGGBB.
pub fn rainbow_color(step: i32) -> u32 {
    fn in_phase(phase: i32, progress: i32) -> i32 {
        match phase % 6 {
            0 | 1 => 0,
            2 => progress,
            3 | 4 => 255,
            _ => 255 - progress,
        }
    }
    let local = step.unsigned_abs() % 1536;
    let (progress, blue_phase) = ((local % 256) as i32, (local / 256) as i32);
    let r = in_phase(blue_phase + 4, progress);
    let g = in_phase(blue_phase + 2, progress);
    let b = in_phase(blue_phase, progress);
    (r as u32) << 16 | (g as u32) << 8 | b as u32
}

/// `ArmRenderer.transformClawHalf`'s offset for one grip (`flip` is 1 or -1): how far the claw
/// opens, by what it holds.
pub fn claw_offset(has_item: bool, is_block_item: bool, flip: i32) -> [f32; 3] {
    let open = if !has_item {
        1.0 / 16.0
    } else if is_block_item {
        3.0 / 16.0
    } else {
        5.0 / 64.0
    };
    [0.0, -(flip as f32) * open, -6.0 / 16.0]
}

/// `ArmRenderer`'s scale and drop of the item in the claw: `(scale, down)`.
pub fn held_item_fit(is_block_item: bool) -> (f32, f32) {
    if is_block_item { (0.5, 9.0 / 16.0) } else { (0.625, 10.0 / 16.0) }
}

// --- the cycle (`ArmBlockEntity.tick` and what it calls) ----------------------------------------

/// What the arm needs to know of the world and of the stacks it moves; `S` is the game's stack.
/// Inputs and outputs are numbered as `inputs` and `outputs` are in Create.
pub trait Env<S> {
    fn count(&self, stack: &S) -> u32;
    /// `ItemStack.isSameItem`.
    fn same_item(&self, a: &S, b: &S) -> bool;
    /// `ItemStack.matches`: item, count and components.
    fn matches(&self, a: &S, b: &S) -> bool;

    fn input_count(&self) -> usize;
    fn output_count(&self) -> usize;
    /// `ArmInteractionPoint.isValid`.
    fn input_valid(&mut self, index: usize) -> bool;
    fn output_valid(&mut self, index: usize) -> bool;
    /// `getSlotCount` of an input.
    fn input_slots(&mut self, index: usize) -> usize;
    /// `extract(slot, amount, simulate)` of an input: `None` for nothing.
    fn extract(&mut self, index: usize, slot: usize, amount: u32, simulate: bool) -> Option<S>;
    /// `insert(stack, simulate)` of an output: what is left.
    fn insert(&mut self, index: usize, stack: S, simulate: bool) -> S;
    /// `ArmInteractionPoint.keepAlive` of the input being moved to.
    fn keep_alive(&mut self, _index: usize) {}
    /// `checkForMusicAmong`: a jukebox among the points with a record in it.
    fn has_music(&mut self) -> bool {
        false
    }
}

/// The fields of the arm the cycle reads and changes.
#[derive(Clone, Debug, PartialEq)]
pub struct Arm<S> {
    pub phase: Phase,
    /// `chasedPointIndex`, -1 for none.
    pub index: i32,
    pub progress: f32,
    pub held: Option<S>,
    pub last_input: i32,
    pub last_output: i32,
    pub mode: SelectionMode,
    /// `redstoneLocked`.
    pub locked: bool,
}

impl<S> Arm<S> {
    /// A new arm (`ArmBlockEntity` constructor): searching, progress 0, nothing held.
    pub fn new() -> Self {
        Arm {
            phase: Phase::SearchInputs,
            index: -1,
            progress: 0.0,
            held: None,
            last_input: -1,
            last_output: -1,
            mode: SelectionMode::RoundRobin,
            locked: false,
        }
    }
}

impl<S> Default for Arm<S> {
    fn default() -> Self {
        Self::new()
    }
}

/// What a step of the arm did that the glue has to tell the world.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Events {
    /// `sendData` and `setChanged`: write the state, with the new progress.
    pub changed: bool,
    /// `SoundEvents.ITEM_PICKUP` at the arm.
    pub picked_up: bool,
}

fn point_index(arm_index: i32, count: usize) -> Option<usize> {
    usize::try_from(arm_index).ok().filter(|i| *i < count)
}

/// `getTargetedInteractionPoint`'s index: an input while moving to one, an output while moving
/// to one.
pub fn targeted<S, E: Env<S>>(arm: &Arm<S>, env: &E) -> Option<usize> {
    match arm.phase {
        Phase::MoveToInput => point_index(arm.index, env.input_count()),
        Phase::MoveToOutput => point_index(arm.index, env.output_count()),
        _ => None,
    }
}

/// `simulateInsertion`: what is left of `stack` after offering it to every valid output.
fn simulate_insertion<S, E: Env<S>>(env: &mut E, mut stack: S) -> S {
    for i in 0..env.output_count() {
        if env.output_valid(i) {
            stack = env.insert(i, stack, true);
        }
        if env.count(&stack) == 0 {
            break;
        }
    }
    stack
}

/// `getDistributableAmount`: how much of what slot `slot` of input `index` holds the outputs
/// would take.
pub fn distributable<S: Clone, E: Env<S>>(env: &mut E, index: usize, slot: usize) -> u32 {
    let Some(stack) = env.extract(index, slot, 64, true) else {
        return 0;
    };
    let total = env.count(&stack);
    if total == 0 {
        return 0;
    }
    let remainder = simulate_insertion(env, stack.clone());
    if env.same_item(&stack, &remainder) {
        total.saturating_sub(env.count(&remainder))
    } else {
        total
    }
}

/// The search in `searchForItem` and `searchForDestination`: where to start, how far to look.
fn scan(mode: SelectionMode, last: i32, count: usize) -> (usize, usize) {
    let start = if mode == SelectionMode::PreferFirst { 0 } else { (last + 1).max(0) as usize };
    let range = if mode == SelectionMode::ForcedRoundRobin { (last + 2).max(0) as usize } else { count };
    (start, range.min(count))
}

/// `if (!found && mode == ROUND_ROBIN) last = -1; if (last == count - 1) last = -1`.
fn wrap(mode: SelectionMode, found: bool, mut last: i32, count: usize) -> i32 {
    if !found && mode == SelectionMode::RoundRobin {
        last = -1;
    }
    if last == count as i32 - 1 {
        last = -1;
    }
    last
}

/// `selectIndex`.
fn select<S>(arm: &mut Arm<S>, input: bool, index: usize, ev: &mut Events) {
    arm.phase = if input { Phase::MoveToInput } else { Phase::MoveToOutput };
    arm.index = index as i32;
    arm.progress = 0.0;
    if input {
        arm.last_input = index as i32;
    } else {
        arm.last_output = index as i32;
    }
    ev.changed = true;
}

/// `searchForItem`.
pub fn search_for_item<S: Clone, E: Env<S>>(arm: &mut Arm<S>, env: &mut E, ev: &mut Events) {
    if arm.locked {
        return;
    }
    let count = env.input_count();
    let (start, range) = scan(arm.mode, arm.last_input, count);
    let mut found = false;
    'points: for i in start..range {
        if !env.input_valid(i) {
            continue;
        }
        for slot in 0..env.input_slots(i) {
            if distributable(env, i, slot) == 0 {
                continue;
            }
            select(arm, true, i, ev);
            found = true;
            break 'points;
        }
    }
    arm.last_input = wrap(arm.mode, found, arm.last_input, count);
}

/// `searchForDestination`.
pub fn search_for_destination<S: Clone, E: Env<S>>(arm: &mut Arm<S>, env: &mut E, ev: &mut Events) {
    let Some(held) = arm.held.clone() else {
        return;
    };
    let count = env.output_count();
    let (start, range) = scan(arm.mode, arm.last_output, count);
    let mut found = false;
    for i in start..range {
        if !env.output_valid(i) {
            continue;
        }
        let remainder = env.insert(i, held.clone(), true);
        if env.matches(&remainder, &held) {
            continue;
        }
        select(arm, false, i, ev);
        found = true;
        break;
    }
    arm.last_output = wrap(arm.mode, found, arm.last_output, count);
}

/// `collectItem`.
fn collect_item<S: Clone, E: Env<S>>(arm: &mut Arm<S>, env: &mut E, ev: &mut Events) {
    if let Some(i) = targeted(arm, env)
        && env.input_valid(i)
    {
        for slot in 0..env.input_slots(i) {
            let amount = distributable(env, i, slot);
            if amount == 0 {
                continue;
            }
            let previous = arm.held.take();
            arm.held = env.extract(i, slot, amount, false).filter(|s| env.count(s) > 0);
            arm.phase = Phase::SearchOutputs;
            arm.progress = 0.0;
            arm.index = -1;
            ev.changed = true;
            ev.picked_up = match (&previous, &arm.held) {
                (Some(old), Some(new)) => !env.same_item(new, old),
                (None, Some(_)) => true,
                _ => false,
            };
            return;
        }
    }
    arm.phase = Phase::SearchInputs;
    arm.progress = 0.0;
    arm.index = -1;
    ev.changed = true;
}

/// `depositItem`.
fn deposit_item<S: Clone, E: Env<S>>(arm: &mut Arm<S>, env: &mut E, ev: &mut Events) {
    if let Some(i) = targeted(arm, env)
        && env.output_valid(i)
        && let Some(held) = arm.held.take()
    {
        let remainder = env.insert(i, held, false);
        arm.held = Some(remainder).filter(|s| env.count(s) > 0);
    }
    arm.phase = if arm.held.is_none() { Phase::SearchInputs } else { Phase::SearchOutputs };
    arm.progress = 0.0;
    arm.index = -1;
    ev.changed = true;
}

/// `lazyTick`.
fn lazy_tick<S: Clone, E: Env<S>>(arm: &mut Arm<S>, env: &mut E, ev: &mut Events) {
    if arm.progress < 0.5 {
        return;
    }
    match arm.phase {
        Phase::SearchInputs | Phase::Dancing => {
            // `checkForMusic`.
            let music = env.has_music();
            if music != (arm.phase == Phase::Dancing) {
                arm.phase = if music { Phase::Dancing } else { Phase::SearchInputs };
                ev.changed = true;
            }
        }
        Phase::SearchOutputs => search_for_destination(arm, env, ev),
        _ => {}
    }
}

/// One server tick of `ArmBlockEntity.tick` after the kinetic part: `progress` is what it was
/// last tick, `rate` this tick's step (see [`rate`]) and `lazy_due` whether the lazy tick
/// counter ran out.
pub fn tick<S: Clone, E: Env<S>>(arm: &mut Arm<S>, env: &mut E, rate: f32, lazy_due: bool) -> Events {
    let mut ev = Events::default();
    if lazy_due {
        lazy_tick(arm, env, &mut ev);
    }
    let reached_before = arm.progress >= 1.0;
    arm.progress = (arm.progress + rate).min(1.0);
    let reached = !reached_before && arm.progress >= 1.0;
    if arm.progress < 1.0 {
        if arm.phase == Phase::MoveToInput
            && let Some(i) = targeted(arm, env)
        {
            env.keep_alive(i);
        }
        return ev;
    }
    match arm.phase {
        Phase::MoveToInput => collect_item(arm, env, &mut ev),
        Phase::MoveToOutput => deposit_item(arm, env, &mut ev),
        Phase::SearchInputs | Phase::Dancing => search_for_item(arm, env, &mut ev),
        Phase::SearchOutputs => {}
    }
    if reached {
        lazy_tick(arm, env, &mut ev);
    }
    ev
}

/// `redstoneUpdate`: the arm's redstone lock follows the signal; unlocked, it looks for an input
/// at once. (Create looks whatever the phase; here only while the arm searches, so a held stack
/// is never overwritten.)
pub fn redstone_update<S: Clone, E: Env<S>>(arm: &mut Arm<S>, env: &mut E, powered: bool) -> Events {
    let mut ev = Events::default();
    if powered == arm.locked {
        return ev;
    }
    arm.locked = powered;
    ev.changed = true;
    if !powered && matches!(arm.phase, Phase::SearchInputs | Phase::Dancing) {
        search_for_item(arm, env, &mut ev);
    }
    ev
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stack is (item, count).
    type St = (&'static str, u32);

    #[derive(Default)]
    struct World {
        /// Each input's slots.
        inputs: Vec<Vec<Option<St>>>,
        /// Each output's room: what it takes, and how many.
        outputs: Vec<(&'static str, u32)>,
        taken: Vec<St>,
        music: bool,
        kept: usize,
    }

    impl Env<St> for World {
        fn count(&self, s: &St) -> u32 {
            s.1
        }
        fn same_item(&self, a: &St, b: &St) -> bool {
            a.0 == b.0
        }
        fn matches(&self, a: &St, b: &St) -> bool {
            a == b
        }
        fn input_count(&self) -> usize {
            self.inputs.len()
        }
        fn output_count(&self) -> usize {
            self.outputs.len()
        }
        fn input_valid(&mut self, _: usize) -> bool {
            true
        }
        fn output_valid(&mut self, _: usize) -> bool {
            true
        }
        fn input_slots(&mut self, i: usize) -> usize {
            self.inputs[i].len()
        }
        fn extract(&mut self, i: usize, slot: usize, amount: u32, simulate: bool) -> Option<St> {
            let have = self.inputs[i][slot]?;
            let n = have.1.min(amount);
            if !simulate {
                self.inputs[i][slot] = (have.1 > n).then_some((have.0, have.1 - n));
            }
            Some((have.0, n))
        }
        fn insert(&mut self, i: usize, stack: St, simulate: bool) -> St {
            let (item, room) = self.outputs[i];
            if item != stack.0 || room == 0 {
                return stack;
            }
            let put = room.min(stack.1);
            if !simulate {
                self.outputs[i].1 -= put;
                self.taken.push((stack.0, put));
            }
            (stack.0, stack.1 - put)
        }
        fn keep_alive(&mut self, _: usize) {
            self.kept += 1;
        }
        fn has_music(&mut self) -> bool {
            self.music
        }
    }

    fn run(arm: &mut Arm<St>, world: &mut World, ticks: usize, rate: f32) {
        for t in 0..ticks {
            tick(arm, world, rate, t as i64 % LAZY_PERIOD == 0);
        }
    }

    #[test]
    fn it_carries_a_stack_from_an_input_to_an_output() {
        let mut world = World {
            inputs: vec![vec![Some(("iron", 5))]],
            outputs: vec![("iron", 64)],
            ..World::default()
        };
        let mut arm = Arm::new();
        arm.progress = 1.0;
        run(&mut arm, &mut world, 200, 0.25);
        assert_eq!(world.taken, vec![("iron", 5)]);
        assert!(arm.held.is_none());
        assert_eq!(arm.phase, Phase::SearchInputs);
        assert!(world.kept > 0);
    }

    #[test]
    fn it_ignores_what_no_output_takes() {
        let mut world = World {
            inputs: vec![vec![Some(("gold", 5))]],
            outputs: vec![("iron", 64)],
            ..World::default()
        };
        let mut arm = Arm::new();
        arm.progress = 1.0;
        run(&mut arm, &mut world, 100, 0.25);
        assert_eq!(arm.phase, Phase::SearchInputs);
        assert!(world.taken.is_empty());
    }

    #[test]
    fn it_takes_only_what_the_outputs_have_room_for() {
        let world = World {
            inputs: vec![vec![Some(("iron", 40))]],
            outputs: vec![("iron", 16)],
            ..World::default()
        };
        let mut world = world;
        assert_eq!(distributable(&mut world, 0, 0), 16);
    }

    #[test]
    fn round_robin_cycles_and_prefer_first_does_not() {
        let mut world = World {
            inputs: vec![vec![Some(("a", 9))], vec![Some(("a", 9))], vec![Some(("a", 9))]],
            outputs: vec![("a", 999)],
            ..World::default()
        };
        let mut arm: Arm<St> = Arm::new();
        let mut ev = Events::default();
        let mut picks = Vec::new();
        for _ in 0..4 {
            search_for_item(&mut arm, &mut world, &mut ev);
            picks.push(arm.index);
        }
        assert_eq!(picks, [0, 1, 2, 0]);
        arm.mode = SelectionMode::PreferFirst;
        arm.last_input = 1;
        search_for_item(&mut arm, &mut world, &mut ev);
        assert_eq!(arm.index, 0);
    }

    #[test]
    fn forced_round_robin_waits_for_the_next_point() {
        let mut world = World {
            inputs: vec![vec![None], vec![Some(("a", 1))]],
            outputs: vec![("a", 5)],
            ..World::default()
        };
        let mut arm: Arm<St> = Arm::new();
        arm.mode = SelectionMode::ForcedRoundRobin;
        let mut ev = Events::default();
        // It looks at input 0 only (nothing there), then at input 1.
        search_for_item(&mut arm, &mut world, &mut ev);
        assert_eq!(arm.phase, Phase::SearchInputs);
        assert_eq!(arm.last_input, -1);
        arm.last_input = 0;
        search_for_item(&mut arm, &mut world, &mut ev);
        assert_eq!(arm.index, 1);
    }

    #[test]
    fn a_powered_arm_does_not_search_and_looks_again_when_released() {
        let mut world = World {
            inputs: vec![vec![Some(("a", 1))]],
            outputs: vec![("a", 5)],
            ..World::default()
        };
        let mut arm: Arm<St> = Arm::new();
        arm.progress = 1.0;
        redstone_update(&mut arm, &mut world, true);
        run(&mut arm, &mut world, 10, 0.25);
        assert_eq!(arm.phase, Phase::SearchInputs);
        let ev = redstone_update(&mut arm, &mut world, false);
        assert!(ev.changed);
        assert_eq!(arm.phase, Phase::MoveToInput);
    }

    #[test]
    fn a_record_makes_it_dance() {
        let mut world = World { music: true, ..World::default() };
        let mut arm: Arm<St> = Arm::new();
        arm.progress = 1.0;
        let ev = tick(&mut arm, &mut world, 0.25, true);
        assert!(ev.changed);
        assert_eq!(arm.phase, Phase::Dancing);
        world.music = false;
        tick(&mut arm, &mut world, 0.25, true);
        assert_eq!(arm.phase, Phase::SearchInputs);
    }

    #[test]
    fn a_resting_arm_sits_out_ticks_where_nothing_is_due() {
        assert!(may_skip(Phase::SearchInputs, 1.0, false, false));
        assert!(!may_skip(Phase::SearchInputs, 1.0, true, false));
        assert!(!may_skip(Phase::SearchInputs, 1.0, false, true));
        assert!(may_skip(Phase::SearchOutputs, 1.0, true, false));
        assert!(!may_skip(Phase::SearchInputs, 0.5, false, false));
        assert!(!may_skip(Phase::MoveToInput, 1.0, false, false));
    }

    #[test]
    fn progress_follows_the_speed() {
        assert_eq!(rate(0.0), 0.0);
        assert_eq!(rate(-64.0), 64.0 / 1024.0);
        assert_eq!(rate(1000.0), 0.25);
        assert_eq!(progress_at(0.0, 10, 256.0, 12, 0.0), 0.5);
        assert_eq!(progress_at(0.5, 0, 256.0, 100, 0.0), 1.0);
        assert_eq!(progress_at(0.0, 10, 256.0, 11, 0.5), 0.375);
    }

    #[test]
    fn the_arm_reaches_the_block_below_it() {
        // A point on the top of the block beside the arm, claw pointing down.
        let a = target_angles([0, 0, 0], [1.5, 0.875, 0.5], Dir::Down, false);
        assert!(a.lower.is_finite() && a.upper.is_finite() && a.head.is_finite());
        // It faces +x: the base angle is 180 + atan2(1, 0) = 270.
        assert!((a.base - 270.0).abs() < 1e-3, "{a:?}");
        // The same point from a ceiling arm mirrors the base angle.
        let c = target_angles([0, 0, 0], [1.5, 0.125, 0.5], Dir::Down, true);
        assert!((c.base - (180.0 - 270.0)).abs() < 1e-3, "{c:?}");
    }

    #[test]
    fn the_pose_folds_back_before_it_unfolds() {
        let target = Angles { base: 90.0, lower: 60.0, upper: 80.0, head: -30.0 };
        let motion = Motion::REST.after(None, Some(target));
        let start = pose(0.0, &motion);
        assert_eq!((start.lower, start.upper), (NO_TARGET.lower, NO_TARGET.upper));
        let half = pose(0.5, &motion);
        assert_eq!((half.lower, half.upper), (NO_TARGET.lower, NO_TARGET.upper));
        let end = pose(1.0, &motion);
        assert_eq!((end.lower, end.upper, end.base), (60.0, 80.0, 90.0));
        // Leaving a point: it folds to rest and the base keeps the angle it had.
        let back = Motion { previous: target, previous_base: 90.0, target: None };
        assert_eq!(pose(0.0, &back).lower, 60.0);
        assert_eq!(pose(1.0, &back).lower, NO_TARGET.lower);
        assert_eq!(pose(1.0, &back).base, 90.0);
    }

    #[test]
    fn the_rainbow_goes_red_yellow_green() {
        assert_eq!(rainbow_color(0), 0xFF0000);
        assert_eq!(rainbow_color(256), 0xFFFF00);
        assert_eq!(rainbow_color(512), 0x00FF00);
    }

    #[test]
    fn modes_and_phases_round_trip() {
        for p in Phase::ALL {
            assert_eq!(Phase::from_name(p.name()), p);
        }
        assert_eq!(SelectionMode::PreferFirst.next(), SelectionMode::RoundRobin);
        assert_eq!(SelectionMode::from_ordinal(1), SelectionMode::ForcedRoundRobin);
        assert_eq!(SelectionMode::from_ordinal(9), SelectionMode::RoundRobin);
    }
}
