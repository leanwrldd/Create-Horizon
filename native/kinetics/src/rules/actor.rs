//! Actors on a moving structure: which block acts and where, the context each carries, and the
//! block breaking the Drill, Saw, Plough and Roller share (`MovementBehaviour`, `MovementContext`,
//! `AbstractContraptionEntity.tickActors` / `shouldActorTrigger`, `BlockBreakingMovementBehaviour`,
//! `Contraption.startMoving` / `setActorsActive`). Plain values only: the glue
//! (`native/api/src/machines/actor*.rs`) reads the world, asks these and applies the answer.
//!
//! The Harvester's, Plough's and Roller's own rules are `harvester.rs`, `plough.rs` and
//! `roller.rs`.

use crate::{Dir, Pos};

pub type Vec3 = [f64; 3];

/// The kind of block that acts on a structure, `MovementBehaviour.REGISTRY`'s entries this port has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Drill,
    Saw,
    Harvester,
    Plough,
    Roller,
    Deployer,
    /// An Andesite (`brass: false`) or Brass Funnel.
    Funnel { brass: bool },
    Seat,
    /// `ContraptionControlsMovement`: it acts on no block, it only decides which others may.
    Controls,
    /// `ContactMovementBehaviour`.
    Contact,
}

pub const DRILL: &str = "create:mechanical_drill";
pub const SAW: &str = "create:mechanical_saw";
pub const HARVESTER: &str = "create:mechanical_harvester";
pub const PLOUGH: &str = "create:mechanical_plough";
pub const ROLLER: &str = "create:mechanical_roller";
pub const DEPLOYER: &str = "create:deployer";
pub const ANDESITE_FUNNEL: &str = "create:andesite_funnel";
pub const BRASS_FUNNEL: &str = "create:brass_funnel";
pub const CONTROLS: &str = "create:contraption_controls";
pub const CONTACT: &str = "create:redstone_contact";

/// The behaviour a block has on a structure, if any (`MovementBehaviour.REGISTRY.get(state)`).
pub fn kind_of(block: &str) -> Option<Kind> {
    Some(match block {
        DRILL => Kind::Drill,
        SAW => Kind::Saw,
        HARVESTER => Kind::Harvester,
        PLOUGH => Kind::Plough,
        ROLLER => Kind::Roller,
        DEPLOYER => Kind::Deployer,
        ANDESITE_FUNNEL => Kind::Funnel { brass: false },
        BRASS_FUNNEL => Kind::Funnel { brass: true },
        CONTROLS => Kind::Controls,
        CONTACT => Kind::Contact,
        other if super::seat::colour_of_block(other).is_some() => Kind::Seat,
        _ => return None,
    })
}

// --- vectors --------------------------------------------------------------------------------------

fn length(v: Vec3) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn scale(v: Vec3, s: f64) -> Vec3 {
    v.map(|c| c * s)
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn normal(dir: Dir) -> Vec3 {
    dir.normal().map(f64::from)
}

/// `Vec3.normalize`: the zero vector below 1e-5.
fn normalised(v: Vec3) -> Vec3 {
    let l = length(v);
    if l < 1.0e-5 { [0.0; 3] } else { scale(v, 1.0 / l) }
}

/// `VecHelper.isVecPointingTowards`: the normalised vector within 0.75 of the direction's normal.
pub fn pointing_towards(v: Vec3, dir: Dir) -> bool {
    length(sub(normal(dir), normalised(v))) < 0.75
}

/// `BlockPos.containing`.
pub fn floor(v: Vec3) -> Pos {
    v.map(|c| c.floor() as i32)
}

// --- where an actor is ----------------------------------------------------------------------------

/// `.scale(.65f)` and `.scale(.45f)`: the float widened to a double.
const SIXTY_FIVE: f64 = 0.65f32 as f64;
const FORTY_FIVE: f64 = 0.45f32 as f64;
const FIFTEEN: f64 = 0.15;

/// `MovementBehaviour.getActiveAreaOffset`: where in front of its block an actor acts, in the
/// structure's own frame. `state` reads the block state's properties.
pub fn active_area_offset(kind: Kind, facing: Option<Dir>, extracting: bool) -> Vec3 {
    let Some(facing) = facing else { return [0.0; 3] };
    let n = normal(facing);
    match kind {
        // `DrillMovementBehaviour`, `SawMovementBehaviour`.
        Kind::Drill | Kind::Saw => scale(n, SIXTY_FIVE),
        // `HarvesterMovementBehaviour`, `PloughMovementBehaviour`.
        Kind::Harvester | Kind::Plough => scale(n, FORTY_FIVE),
        // `RollerMovementBehaviour`: `.scale(.45).subtract(0, 2, 0)`.
        Kind::Roller => {
            let v = scale(n, FORTY_FIVE);
            [v[0], v[1] - 2.0, v[2]]
        }
        // `DeployerMovementBehaviour`: `.scale(2)`.
        Kind::Deployer => scale(n, 2.0),
        // `FunnelMovementBehaviour`.
        Kind::Funnel { .. } => {
            if facing != Dir::Up {
                scale(n, if extracting { FIFTEEN } else { SIXTY_FIVE })
            } else {
                scale(n, SIXTY_FIVE)
            }
        }
        // `ContactMovementBehaviour.getActiveAreaOffset`.
        Kind::Contact => scale(n, SIXTY_FIVE),
        Kind::Seat | Kind::Controls => [0.0; 3],
    }
}

/// Whether an actor is active for the motion it has relative to the structure
/// (`MovementBehaviour.isActive` and each behaviour's own addition): the Drill, Saw, Harvester and
/// Plough sleep while the structure moves with their back first, the Roller only works going
/// forward and not on a Rope Pulley's structure (`PulleyContraption`).
pub fn is_active(kind: Kind, disabled: bool, facing: Option<Dir>, relative_motion: Vec3, on_pulley: bool) -> bool {
    if disabled {
        return false;
    }
    let Some(facing) = facing else { return true };
    match kind {
        Kind::Drill | Kind::Saw | Kind::Harvester | Kind::Plough => {
            !pointing_towards(relative_motion, facing.opposite())
        }
        Kind::Roller => !on_pulley && pointing_towards(relative_motion, facing),
        _ => true,
    }
}

/// `MovementBehaviour.mustTickWhileDisabled`: none of these.
pub fn must_tick_while_disabled(_kind: Kind) -> bool {
    false
}

// --- the context ------------------------------------------------------------------------------------

/// `MovementContext`'s motion state for one actor.
#[derive(Clone, Debug, PartialEq)]
pub struct Context {
    pub position: Option<Vec3>,
    pub motion: Vec3,
    pub relative_motion: Vec3,
    pub stall: bool,
    pub disabled: bool,
    pub first_movement: bool,
}

impl Default for Context {
    fn default() -> Self {
        Context {
            position: None,
            motion: [0.0; 3],
            relative_motion: [0.0; 3],
            stall: false,
            disabled: false,
            first_movement: true,
        }
    }
}

impl Context {
    /// The first half of `tickActors` for one actor: its position this tick. Works out the motion
    /// from the last one, unless the actor stalls the structure (`!context.stall && shouldActorTrigger`:
    /// a stalled actor keeps its last motion), and answers the grid cell when a new one was
    /// visited. `reverse` turns a world motion into the structure's own frame
    /// (`reverseRotation(motion, 1)`); `ignore_first_motion` is the Portable Interface's case.
    pub fn arrive(
        &mut self,
        position: Vec3,
        reverse: &dyn Fn(Vec3) -> Vec3,
        ignore_first_motion: bool,
    ) -> Option<Pos> {
        let mut visited = false;
        if !self.stall
            && let Some(previous) = self.position
        {
            self.motion = sub(position, previous);
            self.relative_motion = reverse(self.motion);
            visited = floor(previous) != floor(position)
                || ((length(self.relative_motion) > 0.0 || ignore_first_motion) && self.first_movement);
        }
        self.position = Some(position);
        visited.then(|| floor(position))
    }

    /// `visitNewPosition` happened: `context.firstMovement = false`.
    pub fn visited(&mut self) {
        self.first_movement = false;
    }

    /// `stopMoving` in `Contraption.stop`: the position and the motion are forgotten.
    pub fn stop(&mut self) {
        self.position = None;
        self.motion = [0.0; 3];
        self.relative_motion = [0.0; 3];
    }

    /// `MovementContext.getAnimationSpeed`, on the server.
    pub fn animation_speed(&self) -> f32 {
        animation_speed(self.motion, self.disabled)
    }
}

/// `MovementContext.getAnimationSpeed`: the length of the motion in thousandths, rounded to
/// hundreds away from zero, negative as it is in Create.
pub fn animation_speed(motion: Vec3, disabled: bool) -> f32 {
    let length = -length(motion);
    if disabled || length.abs() < 1.0 / 512.0 {
        return 0.0;
    }
    // `(((int) (length * 1000 + 100 * signum(length))) / 100) * 100`, in Java's int arithmetic.
    let signed = (length * 1000.0 + 100.0 * length.signum()) as i32;
    ((signed / 100) * 100) as f32
}

// --- the Controls' say ------------------------------------------------------------------------------

/// Which kind of structure it is, for the break speed's lower limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Carrier {
    /// A piston, bearing, gantry, pulley or elevator.
    Controlled,
    /// A minecart (`MountedContraption`).
    Minecart,
}

/// `BlockBreakingMovementBehaviour.getBlockBreakingSpeed`: the structure's animation speed over
/// 500, between the carrier's lower limit and 16.
pub fn breaking_speed(animation: f32, carrier: Carrier) -> f32 {
    let lower = match carrier {
        Carrier::Controlled => 1.0 / 128.0,
        Carrier::Minecart => 1.0,
    };
    (animation.abs() / 500.0).clamp(lower, 16.0)
}

/// The Roller breaks 1.5 times faster (`RollerMovementBehaviour.getBlockBreakingSpeed`).
pub fn roller_breaking_speed(speed: f32) -> f32 {
    (speed * 1.5).clamp(1.0 / 128.0, 16.0)
}

/// `Contraption.disabledActors`: the filters (a block id, or "" for all) a structure starts with
/// disabled, from its Controls (`disableActorOnStart`: each Controls saved `Disabled`, with its
/// `Filter`; a filter already in the list is not added twice).
pub fn disabled_filters(controls: &[(bool, Option<&str>)]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (disabled, filter) in controls {
        if !disabled {
            continue;
        }
        let key = filter.unwrap_or("").to_owned();
        if !out.contains(&key) {
            out.push(key);
        }
    }
    out
}

/// `Contraption.setActorsActive(filter, false)` for every disabled filter: whether an actor of
/// `block` is disabled. An empty filter reaches every actor that can be disabled
/// (`canBeDisabledVia` is the block's item; the Controls themselves cannot).
pub fn is_disabled(disabled: &[String], block: &str) -> bool {
    if block == CONTROLS {
        return false;
    }
    disabled.iter().any(|filter| filter.is_empty() || filter == block)
}

// --- block breaking -----------------------------------------------------------------------------------

/// What a breaker asks about the block in front of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    /// `getDestroySpeed`.
    pub hardness: f32,
    /// `canBreak`.
    pub breakable: bool,
}

/// What a tick of the breaker did that the world must be told.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// Nothing for the world (waiting, or no block to break).
    Idle,
    /// The crack at the block is gone (`destroyBlockProgress(id, pos, -1)`).
    Cleared(Pos),
    /// The block cracked to a stage 0 to 9 (a hit sound too).
    Cracking { pos: Pos, stage: i32 },
    /// The block broke: clear the crack at `cleared`, then break `pos` (the top of a column of
    /// falling blocks, `shouldDestroyStartBlock` permitting) and run `onBlockBroken` for `original`.
    Broken { cleared: Pos, pos: Pos, original: Pos },
}

/// `BlockBreakingMovementBehaviour`'s `context.data`: `BreakingPos`, `Progress`,
/// `TicksUntilNextProgress`, `WaitingTicks`, `LastPos` (and the Roller's `ReferencePos`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Breaker {
    pub breaking: Option<Pos>,
    pub progress: i32,
    /// `TicksUntilNextProgress`: absent reads as 0.
    pub ticks_until_next: i32,
    pub waiting: Option<(i32, Pos)>,
    pub reference: Option<Pos>,
}

impl Breaker {
    /// `visitNewPosition`'s last two lines: the block to break, and the structure waits for it.
    pub fn start(&mut self, pos: Pos) {
        self.breaking = Some(pos);
    }

    /// `cancelStall`'s data part: `Progress`, `TicksUntilNextProgress`, `BreakingPos` go. The
    /// crack to clear, if there was a block being broken.
    pub fn cancel(&mut self) -> Option<Pos> {
        let at = self.breaking.take();
        if at.is_some() {
            self.progress = 0;
            self.ticks_until_next = 0;
        }
        at
    }

    fn forget(&mut self) {
        self.progress = 0;
        self.ticks_until_next = 0;
        self.breaking = None;
    }

    /// `tickBreaker`. Returns whether the structure still waits (`context.stall`) as well as
    /// the step. `above_is_falling` says whether the block above a position falls
    /// (`FallingBlock`), to find the top of a column.
    pub fn tick_breaker(
        &mut self,
        ctx: &mut Context,
        speed: f32,
        target: &dyn Fn(Pos) -> Target,
        above_is_falling: &dyn Fn(Pos) -> bool,
    ) -> Step {
        let Some(breaking) = self.breaking else {
            ctx.stall = false;
            return Step::Idle;
        };
        if ctx.relative_motion == [0.0; 3] {
            ctx.stall = false;
            return Step::Idle;
        }
        // `if (ticksUntilNextProgress-- > 0)`: the decrement happens either way, but only a
        // positive value is stored and returned on.
        let waiting = self.ticks_until_next > 0;
        self.ticks_until_next -= 1;
        if waiting {
            return Step::Idle;
        }
        let aim = target(breaking);
        if !aim.breakable {
            let had_progress = self.progress != 0;
            if had_progress {
                self.forget();
            }
            ctx.stall = false;
            return if had_progress { Step::Cleared(breaking) } else { Step::Idle };
        }
        let step = ((speed / aim.hardness) as i32).clamp(1, (10 - self.progress).max(1));
        self.progress += step;
        if self.progress >= 10 {
            let original = breaking;
            let mut top = breaking;
            while above_is_falling([top[0], top[1] + 1, top[2]]) {
                top[1] += 1;
            }
            ctx.stall = false;
            self.forget();
            return Step::Broken { cleared: original, pos: top, original };
        }
        self.ticks_until_next = (aim.hardness / speed) as i32;
        Step::Cracking { pos: breaking, stage: self.progress }
    }

    /// `tick`'s tail: the wait after a falling block broke or the Roller paved
    /// (`WaitingTicks`): counts down while the structure waits, then the position is visited
    /// again. Returns that position.
    pub fn tick_waiting(&mut self, ctx: &mut Context) -> Option<Pos> {
        let (ticks, last) = self.waiting?;
        if ticks > 0 {
            self.waiting = Some((ticks - 1, last));
            ctx.stall = true;
            return None;
        }
        self.waiting = None;
        ctx.stall = false;
        Some(last)
    }

    /// `onBlockBroken` of a falling block: wait ten ticks (`WaitingTicks`), keeping `last`.
    pub fn wait(&mut self, ctx: &mut Context, ticks: i32, last: Pos) {
        self.waiting = Some((ticks, last));
        ctx.stall = true;
    }
}

/// `instanceof FallingBlock`.
pub fn is_falling(block: &str) -> bool {
    super::actor_facts::FALLING.contains(&block)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vector_within_three_quarters_of_a_direction_points_towards_it() {
        assert!(pointing_towards([1.0, 0.0, 0.0], Dir::East));
        assert!(pointing_towards([1.0, 0.5, 0.0], Dir::East));
        // 45 degrees: the distance is 0.765.
        assert!(!pointing_towards([1.0, 1.0, 0.0], Dir::East));
        assert!(!pointing_towards([-1.0, 0.0, 0.0], Dir::East));
        // Standing still points nowhere.
        assert!(!pointing_towards([0.0; 3], Dir::East));
    }

    #[test]
    fn a_drill_sleeps_with_its_back_to_the_way_it_goes() {
        let f = Some(Dir::East);
        assert!(is_active(Kind::Drill, false, f, [1.0, 0.0, 0.0], false));
        assert!(!is_active(Kind::Drill, false, f, [-1.0, 0.0, 0.0], false));
        assert!(!is_active(Kind::Drill, true, f, [1.0, 0.0, 0.0], false));
        // Standing still: not pointing backwards, so awake.
        assert!(is_active(Kind::Drill, false, f, [0.0; 3], false));
    }

    #[test]
    fn a_roller_only_works_going_forward_and_not_on_a_pulley() {
        let f = Some(Dir::North);
        assert!(is_active(Kind::Roller, false, f, [0.0, 0.0, -1.0], false));
        assert!(!is_active(Kind::Roller, false, f, [0.0, 0.0, 1.0], false));
        assert!(!is_active(Kind::Roller, false, f, [0.0, 0.0, -1.0], true));
        assert!(!is_active(Kind::Roller, false, f, [0.0; 3], false));
    }

    #[test]
    fn the_active_areas() {
        let v = active_area_offset(Kind::Drill, Some(Dir::East), false);
        assert!((v[0] - 0.65).abs() < 1e-6 && v[1] == 0.0);
        let v = active_area_offset(Kind::Roller, Some(Dir::South), false);
        assert!((v[2] - 0.45).abs() < 1e-6 && v[1] == -2.0);
        assert_eq!(active_area_offset(Kind::Deployer, Some(Dir::Up), false), [0.0, 2.0, 0.0]);
        let funnel = Kind::Funnel { brass: true };
        assert!((active_area_offset(funnel, Some(Dir::West), true)[0] + 0.15).abs() < 1e-9);
        assert!((active_area_offset(funnel, Some(Dir::West), false)[0] + 0.65).abs() < 1e-6);
        // Up is always 0.65, extracting or not.
        assert!((active_area_offset(funnel, Some(Dir::Up), true)[1] - 0.65).abs() < 1e-6);
    }

    #[test]
    fn a_new_cell_or_the_first_motion_visits() {
        let mut ctx = Context::default();
        let same = |v: Vec3| v;
        // The first tick only learns where it is.
        assert_eq!(ctx.arrive([0.5, 0.5, 0.5], &same, false), None);
        // Moving inside the cell: the first movement counts.
        assert_eq!(ctx.arrive([0.6, 0.5, 0.5], &same, false), Some([0, 0, 0]));
        ctx.visited();
        assert_eq!(ctx.arrive([0.7, 0.5, 0.5], &same, false), None);
        assert_eq!(ctx.arrive([1.1, 0.5, 0.5], &same, false), Some([1, 0, 0]));
    }

    #[test]
    fn a_stalled_actor_keeps_its_last_motion() {
        let mut ctx = Context::default();
        let same = |v: Vec3| v;
        ctx.arrive([0.5; 3], &same, false);
        ctx.arrive([1.5, 0.5, 0.5], &same, false);
        ctx.stall = true;
        assert_eq!(ctx.arrive([1.5, 0.5, 0.5], &same, false), None);
        assert_eq!(ctx.motion, [1.0, 0.0, 0.0]);
        assert_eq!(ctx.position, Some([1.5, 0.5, 0.5]));
    }

    #[test]
    fn the_animation_speed_is_in_hundreds_and_negative() {
        // 0.0625 blocks a tick: -62.5 - 100 = -162 -> -1 -> -100.
        assert_eq!(animation_speed([0.0625, 0.0, 0.0], false), -100.0);
        assert_eq!(animation_speed([0.0, 0.0, 0.0], false), 0.0);
        assert_eq!(animation_speed([0.001, 0.0, 0.0], false), 0.0);
        assert_eq!(animation_speed([0.5, 0.0, 0.0], true), 0.0);
        assert_eq!(animation_speed([0.25, 0.0, 0.0], false), -300.0);
    }

    #[test]
    fn the_breaking_speed_has_a_floor_by_carrier() {
        assert_eq!(breaking_speed(-100.0, Carrier::Controlled), 0.2);
        assert_eq!(breaking_speed(0.0, Carrier::Controlled), 1.0 / 128.0);
        assert_eq!(breaking_speed(-100.0, Carrier::Minecart), 1.0);
        assert_eq!(breaking_speed(-90_000.0, Carrier::Controlled), 16.0);
        assert_eq!(roller_breaking_speed(0.2), 0.3);
    }

    fn still(ctx: &mut Context) {
        ctx.relative_motion = [1.0, 0.0, 0.0];
    }

    #[test]
    fn a_block_breaks_in_stages_and_the_structure_then_moves_on() {
        let mut ctx = Context::default();
        still(&mut ctx);
        let mut b = Breaker::default();
        b.start([5, 0, 0]);
        ctx.stall = true;
        let stone = |_: Pos| Target { hardness: 1.5, breakable: true };
        let nothing_falls = |_: Pos| false;
        // Speed 0.5: progress (0.5/1.5) = 0 clamps to 1 a stage, waiting 3 ticks between.
        let mut stages = Vec::new();
        for _ in 0..100 {
            match b.tick_breaker(&mut ctx, 0.5, &stone, &nothing_falls) {
                Step::Cracking { stage, .. } => stages.push(stage),
                Step::Broken { pos, .. } => {
                    assert_eq!(pos, [5, 0, 0]);
                    break;
                }
                _ => {}
            }
        }
        assert_eq!(stages, vec![1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert!(!ctx.stall);
        assert_eq!(b.breaking, None);
        assert_eq!(b.ticks_until_next, 0);
    }

    #[test]
    fn a_fast_breaker_takes_bigger_steps() {
        let mut ctx = Context::default();
        still(&mut ctx);
        let mut b = Breaker::default();
        b.start([1, 0, 0]);
        let dirt = |_: Pos| Target { hardness: 0.5, breakable: true };
        let nothing_falls = |_: Pos| false;
        // 4 / 0.5 = 8 stages at once.
        assert_eq!(
            b.tick_breaker(&mut ctx, 4.0, &dirt, &nothing_falls),
            Step::Cracking { pos: [1, 0, 0], stage: 8 }
        );
        // Then clamped to what is left: 2.
        b.ticks_until_next = 0;
        assert!(matches!(b.tick_breaker(&mut ctx, 4.0, &dirt, &nothing_falls), Step::Broken { .. }));
    }

    #[test]
    fn something_that_cannot_be_broken_lets_the_structure_go() {
        let mut ctx = Context::default();
        still(&mut ctx);
        ctx.stall = true;
        let mut b = Breaker::default();
        b.start([1, 0, 0]);
        b.progress = 3;
        let bedrock = |_: Pos| Target { hardness: -1.0, breakable: false };
        let none = |_: Pos| false;
        assert_eq!(b.tick_breaker(&mut ctx, 1.0, &bedrock, &none), Step::Cleared([1, 0, 0]));
        assert!(!ctx.stall);
        assert_eq!(b.breaking, None);
    }

    #[test]
    fn a_column_of_sand_is_broken_from_the_top() {
        let mut ctx = Context::default();
        still(&mut ctx);
        let mut b = Breaker::default();
        b.start([0, 0, 0]);
        b.progress = 9;
        let sand = |_: Pos| Target { hardness: 0.5, breakable: true };
        let falls = |p: Pos| p[1] <= 2;
        assert_eq!(
            b.tick_breaker(&mut ctx, 1.0, &sand, &falls),
            Step::Broken { cleared: [0, 0, 0], pos: [0, 2, 0], original: [0, 0, 0] }
        );
    }

    #[test]
    fn a_stopped_structure_does_not_break_and_lets_go() {
        let mut ctx = Context::default();
        ctx.stall = true;
        let mut b = Breaker::default();
        b.start([1, 0, 0]);
        let stone = |_: Pos| Target { hardness: 1.5, breakable: true };
        let none = |_: Pos| false;
        assert_eq!(b.tick_breaker(&mut ctx, 1.0, &stone, &none), Step::Idle);
        assert!(!ctx.stall);
    }

    #[test]
    fn waiting_ticks_revisit_the_last_position() {
        let mut ctx = Context::default();
        let mut b = Breaker::default();
        b.wait(&mut ctx, 2, [4, 4, 4]);
        assert_eq!(b.tick_waiting(&mut ctx), None);
        assert!(ctx.stall);
        assert_eq!(b.tick_waiting(&mut ctx), None);
        assert_eq!(b.tick_waiting(&mut ctx), Some([4, 4, 4]));
        assert!(!ctx.stall);
        assert_eq!(b.tick_waiting(&mut ctx), None);
    }

    #[test]
    fn controls_disable_the_actors_they_name() {
        let off = disabled_filters(&[(true, Some("create:mechanical_drill")), (false, None), (true, Some("create:mechanical_drill"))]);
        assert_eq!(off, vec!["create:mechanical_drill".to_owned()]);
        assert!(is_disabled(&off, DRILL));
        assert!(!is_disabled(&off, SAW));
        let all = disabled_filters(&[(true, None)]);
        assert!(is_disabled(&all, SAW) && is_disabled(&all, DEPLOYER));
        // The Controls can't be disabled.
        assert!(!is_disabled(&all, CONTROLS));
    }

    #[test]
    fn the_blocks_with_a_behaviour() {
        assert_eq!(kind_of(DRILL), Some(Kind::Drill));
        assert_eq!(kind_of("create:white_seat"), Some(Kind::Seat));
        assert_eq!(kind_of("create:brass_funnel"), Some(Kind::Funnel { brass: true }));
        assert_eq!(kind_of("minecraft:stone"), None);
    }

    /// The in-game "harvester on a bearing reaps nothing" report: a Harvester two blocks out from a
    /// bearing's axis, facing south, on an up bearing turning at +16 RPM (4.8 degrees a tick,
    /// right-handed about +y). It moves north, back first, so `isActive` keeps it asleep as in
    /// Create; faced north it works and visits cells on the circle of radius 2.
    #[test]
    fn a_harvester_on_a_turning_structure_must_face_the_way_it_goes() {
        use super::super::portable_interface::rotate_vec;
        let run = |facing: Dir| {
            let mut ctx = Context::default();
            let local = [2.0, 0.0, 0.0];
            let offset = active_area_offset(Kind::Harvester, Some(facing), false);
            let (mut awake, mut cells) = (0, Vec::new());
            for t in 0..40 {
                let angle = 4.8 * f64::from(t);
                let turned = rotate_vec(local, crate::Axis::Y, angle);
                let ahead = rotate_vec(offset, crate::Axis::Y, angle);
                let position = [0.5 + turned[0] + ahead[0], 0.5, 0.5 + turned[2] + ahead[2]];
                let visited = ctx.arrive(position, &|v| rotate_vec(v, crate::Axis::Y, -angle), false);
                if !is_active(Kind::Harvester, false, Some(facing), ctx.relative_motion, false) {
                    continue;
                }
                awake += 1;
                cells.extend(visited);
            }
            (awake, cells)
        };
        let (awake, cells) = run(Dir::South);
        // (The first tick has no motion yet, so it counts as awake once.)
        assert!(awake <= 1 && cells.is_empty(), "{awake} {cells:?}");
        let (awake, cells) = run(Dir::North);
        assert!(awake > 30 && cells.len() > 3, "{awake} {cells:?}");
        for cell in cells {
            let d = f64::from(cell[0]).hypot(f64::from(cell[2]));
            assert!((1.0..=3.5).contains(&d), "{cell:?}");
        }
    }
}
