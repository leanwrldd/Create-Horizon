//! The Mechanical Press: its pressing cycle, what it costs, and how its head moves. Ported from
//! Create's `PressingBehaviour`, `MechanicalPressBlockEntity` and `AllBlocks.MECHANICAL_PRESS`
//! (MIT, the Creators of Create), checked against Create `mc1.21.1/dev` at 4148fd8.
//!
//! The press works in three places (`PressingBehaviour.Mode`): on items lying in the world under
//! it (`WORLD`), on items carried by a belt or lying on a depot two blocks under it (`BELT`,
//! `BeltPressingCallbacks`) and on a basin two blocks under it (`BASIN`, compacting). The game
//! glue scans for items, applies recipes and spawns results; this module only decides when.
//! On the kinetic network a press is a shaft along its facing's axis
//! (`MechanicalPressBlock.getRotationAxis`, `hasShaftTowards`) that costs [`IMPACT`].

use crate::belt::Processing;

/// Ticks of progress in one press, down and up (`PressingBehaviour.CYCLE`).
pub const CYCLE: i32 = 240;
/// Progress at which the head reaches the item and the recipe applies.
pub const HALF: i32 = CYCLE / 2;
/// Ticks between looks for something to press (`PressingBehaviour.ENTITY_SCAN`).
pub const ENTITY_SCAN: i32 = 10;
/// Stress units per RPM (`AllBlocks.MECHANICAL_PRESS`: `CStress.setImpact(8.0)`).
pub const IMPACT: f32 = 8.0;
/// How far the head travels for an item in the world, in blocks (`Mode.WORLD.headOffset`).
pub const WORLD_HEAD_TRAVEL: f32 = 1.0;
/// `Mode.BELT.headOffset`: 19/16 of a block, down to an item on a belt or a depot.
pub const BELT_HEAD_TRAVEL: f32 = 19.0 / 16.0;
/// `Mode.BASIN.headOffset`: 22/16 of a block, down onto a basin's contents.
pub const BASIN_HEAD_TRAVEL: f32 = 22.0 / 16.0;
/// Ticks before a player may pick up an item the press made (`ItemEntity.setDefaultPickUpDelay`).
pub const PICKUP_DELAY: i32 = 10;
/// How hard a made item is tossed, at most, along each axis (`VecHelper.offsetRandomly(.05f)`).
pub const TOSS: f64 = 0.05;
/// Create's `bulkPressing` server option defaults to false: a stack of several items is
/// pressed one at a time.
pub const BULK: bool = false;

/// Progress per tick at `rpm` (`PressingBehaviour.getRunningTickSpeed`): from 1 at the slowest to
/// 60 at 512 RPM and over, 0 standing still.
pub fn progress_per_tick(rpm: f32) -> i32 {
    if rpm == 0.0 {
        return 0;
    }
    let t = (rpm.abs() / 512.0).clamp(0.0, 1.0);
    // `(int) Mth.lerp(...)`: truncated toward zero.
    (1.0 + t * (60.0 - 1.0)) as i32
}

/// `PressingBehaviour.Mode`: what the running cycle works on. Saved as its ordinal under `Mode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Items lying under the press.
    World,
    /// An item on a belt or a depot two blocks under it.
    Belt,
    /// The contents of a basin two blocks under it.
    Basin,
}

impl Mode {
    pub const fn id(self) -> i32 {
        match self {
            Mode::World => 0,
            Mode::Belt => 1,
            Mode::Basin => 2,
        }
    }

    /// `Mode.values()[ordinal]`; anything else reads as the first, as a fresh press is.
    pub const fn from_id(id: i32) -> Self {
        match id {
            1 => Mode::Belt,
            2 => Mode::Basin,
            _ => Mode::World,
        }
    }

    /// How far the head comes down, in blocks (`Mode.headOffset`).
    pub const fn head_travel(self) -> f32 {
        match self {
            Mode::World => WORLD_HEAD_TRAVEL,
            Mode::Belt => BELT_HEAD_TRAVEL,
            Mode::Basin => BASIN_HEAD_TRAVEL,
        }
    }
}

/// What the glue does after a tick of the press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing this tick.
    Wait,
    /// Look for items to press under the press; call [`Press::start`] if one has a recipe.
    Scan,
    /// The head is down: work on what is under it, as [`Press::mode`] says (items in the world
    /// first, then call [`Press::pressed`] if any lay there; the held item on a belt or a depot;
    /// the basin's recipe). Tell clients.
    Apply,
    /// The press went back up and stopped. Tell clients.
    Finished,
}

/// One press's state. Saved as Create saves it (`Running`, `Ticks`, `Finished`); the scan
/// cooldown isn't saved, as in Create.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Press {
    pub running: bool,
    /// What the cycle works on (`PressingBehaviour.mode`), set by [`Press::start_in`].
    pub mode: Mode,
    /// Progress through the cycle, 0 to just past [`CYCLE`].
    pub ticks: i32,
    pub finished: bool,
    pub scan_cooldown: i32,
}

impl Default for Press {
    fn default() -> Self {
        Self {
            running: false,
            mode: Mode::World,
            ticks: 0,
            finished: false,
            scan_cooldown: ENTITY_SCAN,
        }
    }
}

impl Press {
    /// `PressingBehaviour.start(Mode.WORLD)`. Tell clients.
    pub fn start(&mut self) {
        self.start_in(Mode::World);
    }

    /// `PressingBehaviour.start(mode)`. Tell clients.
    pub fn start_in(&mut self, mode: Mode) {
        self.mode = mode;
        self.running = true;
        self.ticks = 0;
    }

    /// `MechanicalPressBlockEntity.onBasinRemoved`: the basin below was emptied or taken away, so
    /// the head goes back up at once. Tell clients.
    pub fn basin_removed(&mut self) {
        self.running = false;
        self.ticks = 0;
    }

    /// The press found items on the ground at [`Step::Apply`]: look again straight after this
    /// press, as `applyInWorld` zeroes the cooldown.
    pub fn pressed(&mut self) {
        self.scan_cooldown = 0;
    }

    /// One server tick at `rpm` (`PressingBehaviour.tick`, server side, world mode).
    pub fn tick(&mut self, rpm: f32) -> Step {
        if !self.running {
            if rpm == 0.0 {
                return Step::Wait;
            }
            if self.scan_cooldown > 0 {
                self.scan_cooldown -= 1;
            }
            if self.scan_cooldown <= 0 {
                self.scan_cooldown = ENTITY_SCAN;
                return Step::Scan;
            }
            return Step::Wait;
        }

        let apply = self.ticks == HALF && rpm != 0.0;

        if self.ticks > CYCLE {
            self.finished = true;
            self.running = false;
            return Step::Finished;
        }

        let before = self.ticks;
        self.ticks += progress_per_tick(rpm);
        // The head stops at the item for one tick whatever the speed.
        if before < HALF && self.ticks >= HALF {
            self.ticks = HALF;
        }
        if apply { Step::Apply } else { Step::Wait }
    }
}

/// How far down the head is, 0 at rest to 1 on the item (`getRenderedHeadOffset`), at progress
/// `ticks` (fractional between ticks). Times [`WORLD_HEAD_TRAVEL`] for blocks.
pub fn head_offset(running: bool, ticks: f32) -> f32 {
    if !running {
        return 0.0;
    }
    let ticks = ticks.abs();
    let cycle = CYCLE as f32;
    if ticks < (CYCLE * 2 / 3) as f32 {
        return (ticks / cycle * 2.0).powi(3).clamp(0.0, 1.0);
    }
    ((cycle - ticks) / cycle * 3.0).clamp(0.0, 1.0)
}

/// The progress of a press in a Ponder scene `elapsed` ticks after `start(...)`
/// (`PressingBehaviour.tick`). A scene's world is virtual, so the client's wait at the item for the
/// server's packet (`runningTicks = -CYCLE / 2`) doesn't happen: the head runs on through the
/// half-way tick, which only holds the count at [`HALF`] for that tick. Fractional `elapsed` is
/// the partial tick (`Mth.lerp(partialTicks, prevRunningTicks, runningTicks)`).
pub fn virtual_ticks(rpm: f32, elapsed: f32) -> f32 {
    let speed = progress_per_tick(rpm);
    let whole = elapsed.max(0.0).floor() as i32;
    let (mut previous, mut ticks) = (0, 0);
    for _ in 0..whole {
        if ticks > CYCLE {
            // The head is back up for good: it reads 0 from here on, whatever the count.
            return ticks as f32;
        }
        previous = ticks;
        ticks += speed;
        if previous < HALF && ticks >= HALF {
            ticks = HALF;
        }
    }
    previous as f32 + (ticks - previous) as f32 * (elapsed.max(0.0) - whole as f32)
}

/// The progress a client draws, from the last progress the server sent (`synced`, at game time
/// `since`) and the speed then: it runs on at the press's pace but waits at the item until the
/// server says it pressed, as Create's client pauses for the packet.
pub fn client_ticks(synced: i32, since: i64, now: f64, rpm: f32) -> f32 {
    let elapsed = (now - since as f64).max(0.0) as f32;
    let ticks = synced as f32 + elapsed * progress_per_tick(rpm) as f32;
    let cap = if synced < HALF { HALF } else { CYCLE };
    ticks.min(cap as f32)
}

/// What pressing an item stack of `count` does (`MechanicalPressBlockEntity.tryProcessInWorld`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InWorld {
    /// The whole stack turns into the results: the entity holds the first, the rest are
    /// dropped beside it (`RecipeApplier.applyRecipeOn(ItemEntity, ...)`).
    Convert,
    /// One item is pressed: the results drop as new items and the stack shrinks by one.
    TakeOne,
}

pub fn in_world(count: u32) -> InWorld {
    if BULK || count == 1 {
        InWorld::Convert
    } else {
        InWorld::TakeOne
    }
}

/// What the press tells a belt or a depot about an item that has just come to the middle of the
/// block under it (`BeltPressingCallbacks.onItemReceived`). `recipe` says a pressing recipe takes
/// the item (`tryProcessOnBelt` simulated).
pub fn on_item_received(rpm: f32, running: bool, recipe: bool) -> Processing {
    if rpm == 0.0 {
        return Processing::Pass;
    }
    if running {
        return Processing::Hold;
    }
    if !recipe {
        return Processing::Pass;
    }
    Processing::Hold
}

/// Whether [`on_item_received`] starts the press (`behaviour.start(Mode.BELT)`): it was idle,
/// turning, and took the item.
pub fn starts_on_item(rpm: f32, running: bool, recipe: bool) -> bool {
    rpm != 0.0 && !running && recipe
}

/// What the press tells a belt or a depot about an item it holds while the press runs
/// (`BeltPressingCallbacks.whenItemHeld`, less the pressing at the item itself: that is
/// [`Step::Apply`], done by the press's own tick whatever order the blocks tick in).
pub fn while_item_held(rpm: f32, running: bool) -> Processing {
    if rpm == 0.0 || !running {
        Processing::Pass
    } else {
        Processing::Hold
    }
}

/// How a stack of `count` on a belt or a depot is pressed (`whenItemHeld`): the number of items
/// the recipe takes this time, and the number left held in its place for the next press.
/// Without `bulkPressing` (the default) a stack is pressed one item at a time.
pub fn belt_split(count: u32) -> (u32, u32) {
    match in_world(count) {
        InWorld::Convert => (count, 0),
        InWorld::TakeOne => (1, count - 1),
    }
}

/// `BasinOperatingBlockEntity.updateBasin`: whether to look for a recipe in the basin. The press
/// needs no minimum speed (`isSpeedRequirementFulfilled` of `SpeedLevel.NONE`).
pub fn looks_at_basin(rpm: f32, running: bool, basin_can_continue: bool) -> bool {
    rpm != 0.0 && !running && basin_can_continue
}

/// What `MechanicalPressBlockEntity.onPressingCompleted` does after a cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// `startProcessingBasin`: the recipe still matches, press again at once.
    Again,
    /// `basinChecker.scheduleUpdate()`: look at the basin afresh.
    Recheck,
}

pub fn after_cycle(mode: Mode, recipe_matches: bool, basin_can_continue: bool) -> Next {
    if mode == Mode::Basin && recipe_matches && basin_can_continue {
        Next::Again
    } else {
        Next::Recheck
    }
}

/// `MechanicalPressBlockEntity.startProcessingBasin`'s guard: a cycle still on its way down is
/// not started again.
pub fn may_start_on_basin(running: bool, ticks: i32) -> bool {
    !(running && ticks <= HALF)
}

/// Ticks between the glue's looks at the basin while the press is idle. Create is told the
/// moment a basin's contents change (`contentsChanged`), which the glue approximates with this
/// poll; the mixer polls at the same rate.
pub const BASIN_POLL: i64 = 5;

/// Whether a result with `chance` comes out, given a uniform draw in 0..1
/// (`ProcessingOutput.rollOutput`: `chance == 1` always, else `random.nextFloat() <= chance`).
pub fn rolls(chance: f32, draw: f32) -> bool {
    chance >= 1.0 || draw <= chance
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_follows_create_s_lerp() {
        assert_eq!(progress_per_tick(0.0), 0);
        assert_eq!(progress_per_tick(1.0), 1);
        assert_eq!(progress_per_tick(16.0), 2); // 1 + 59 * 16/512 = 2.84
        assert_eq!(progress_per_tick(-32.0), 4); // 4.69
        assert_eq!(progress_per_tick(256.0), 30); // 30.5
        assert_eq!(progress_per_tick(512.0), 60);
        assert_eq!(progress_per_tick(1024.0), 60);
    }

    #[test]
    fn an_idle_press_scans_every_ten_ticks_while_turning() {
        let mut press = Press::default();
        assert_eq!(press.tick(0.0), Step::Wait);
        let steps: Vec<_> = (0..20).map(|_| press.tick(32.0)).collect();
        let scans: Vec<_> = steps
            .iter()
            .enumerate()
            .filter(|(_, s)| **s == Step::Scan)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(scans, [9, 19]);
    }

    /// Runs a started press at `rpm` and returns the tick indexes of `Apply` and `Finished`.
    fn cycle(rpm: f32) -> (usize, usize) {
        let mut press = Press::default();
        press.start();
        let mut apply = None;
        for i in 0..10_000 {
            match press.tick(rpm) {
                Step::Apply => {
                    assert!(apply.is_none(), "applies once");
                    apply = Some(i);
                }
                Step::Finished => {
                    assert!(!press.running && press.finished);
                    return (apply.expect("applied before finishing"), i);
                }
                _ => {}
            }
        }
        panic!("never finished");
    }

    #[test]
    fn a_cycle_presses_once_halfway() {
        // 32 RPM is 4 a tick: 120 is reached in 30 ticks, applied in the next, and the
        // count passes 240 thirty ticks later and finishes the tick after.
        assert_eq!(cycle(32.0), (30, 61));
        // At 512 RPM and up, 60 a tick: 2 ticks down, a tick on the item.
        assert_eq!(cycle(512.0), (2, 5));
        // At 1 RPM, 1 a tick.
        assert_eq!(cycle(1.0), (120, 241));
    }

    #[test]
    fn the_head_stops_at_the_item_whatever_the_speed() {
        let mut press = Press::default();
        press.start();
        press.ticks = 100;
        press.tick(512.0);
        assert_eq!(press.ticks, HALF);
    }

    #[test]
    fn a_stalled_press_waits_at_the_item_without_pressing() {
        let mut press = Press::default();
        press.start();
        press.ticks = HALF;
        assert_eq!(press.tick(0.0), Step::Wait);
        assert_eq!(press.ticks, HALF);
        assert_eq!(press.tick(32.0), Step::Apply);
    }

    #[test]
    fn the_head_goes_down_fast_at_the_end_and_back_up_steadily() {
        assert_eq!(head_offset(false, 100.0), 0.0);
        assert_eq!(head_offset(true, 0.0), 0.0);
        assert_eq!(head_offset(true, 60.0), 0.125);
        assert_eq!(head_offset(true, 120.0), 1.0);
        assert_eq!(head_offset(true, 159.0), 1.0);
        assert_eq!(head_offset(true, 200.0), 0.5);
        assert_eq!(head_offset(true, 240.0), 0.0);
    }

    #[test]
    fn a_client_waits_at_the_item_for_the_server() {
        assert_eq!(client_ticks(0, 100, 110.5, 32.0), 42.0);
        assert_eq!(client_ticks(0, 100, 200.0, 32.0), HALF as f32);
        assert_eq!(client_ticks(HALF, 200, 210.0, 32.0), 160.0);
        assert_eq!(client_ticks(HALF, 200, 400.0, 32.0), CYCLE as f32);
    }

    #[test]
    fn one_item_converts_and_a_stack_gives_one_at_a_time() {
        assert_eq!(in_world(1), InWorld::Convert);
        assert_eq!(in_world(5), InWorld::TakeOne);
    }

    #[test]
    fn a_hand_crank_runs_one_press_but_not_two() {
        use crate::generators::{CRANK_CAPACITY, CRANK_SPEED};
        use crate::{Axis, Dir, Kind, Node, solve};
        let crank = Node {
            kind: Kind::Crank(Dir::West),
            generated: CRANK_SPEED * Dir::West.sign() as f32,
            capacity: CRANK_CAPACITY,
            impact: 0.0,
        };
        let press = Node {
            impact: IMPACT,
            ..Node::passive(Kind::Shaft(Axis::X))
        };
        // 8 su/RPM of capacity at 32 RPM is 256 su; a press at 32 RPM costs 8 x 32 = 256.
        let one = [([0, 0, 0], crank), ([1, 0, 0], press)]
            .into_iter()
            .collect();
        let solution = solve(&one);
        assert_eq!(solution.speeds[&[1, 0, 0]].abs(), 32.0);
        assert_eq!(solution.networks[0].stress, 256.0);
        let two = [([0, 0, 0], crank), ([1, 0, 0], press), ([2, 0, 0], press)]
            .into_iter()
            .collect();
        let solution = solve(&two);
        assert!(solution.networks[0].overstressed);
        assert_eq!(solution.speeds[&[1, 0, 0]], 0.0);
    }

    #[test]
    fn a_scene_press_runs_on_through_the_half_way_tick() {
        // 32 RPM is 4 a tick: 30 ticks to HALF, then on to the cycle's end.
        assert_eq!(progress_per_tick(32.0), 4);
        assert_eq!(virtual_ticks(32.0, 0.0), 0.0);
        // A frame shows the blend of the last two ticks, as Create's partial tick does.
        assert_eq!(virtual_ticks(32.0, 10.0), 36.0);
        assert_eq!(virtual_ticks(32.0, 10.5), 38.0);
        assert_eq!(virtual_ticks(32.0, 30.0), (HALF - 4) as f32);
        assert_eq!(virtual_ticks(32.0, 31.0), HALF as f32);
        assert_eq!(virtual_ticks(32.0, 31.5), 122.0);
        // Over the end the head is up (offset 0) and stays so.
        assert_eq!(head_offset(true, virtual_ticks(32.0, 200.0)), 0.0);
        // A press that doesn't turn never moves.
        assert_eq!(virtual_ticks(0.0, 50.0), 0.0);
    }

    #[test]
    fn the_head_comes_down_as_far_as_the_mode_says() {
        // Mode.WORLD 1, Mode.BELT 19/16, Mode.BASIN 22/16, saved as the ordinal.
        assert_eq!(Mode::World.head_travel(), 1.0);
        assert_eq!(Mode::Belt.head_travel(), 1.1875);
        assert_eq!(Mode::Basin.head_travel(), 1.375);
        for mode in [Mode::World, Mode::Belt, Mode::Basin] {
            assert_eq!(Mode::from_id(mode.id()), mode);
        }
        assert_eq!([Mode::World.id(), Mode::Belt.id(), Mode::Basin.id()], [0, 1, 2]);
        assert_eq!(Mode::from_id(7), Mode::World);
    }

    #[test]
    fn a_basin_cycle_times_as_a_world_one_and_remembers_its_mode() {
        let mut press = Press::default();
        press.start_in(Mode::Basin);
        assert_eq!(press.mode, Mode::Basin);
        assert!(press.running && press.ticks == 0);
        // The same 240 progress, applied at the item's height whatever the mode.
        assert_eq!(cycle(32.0), (30, 61));
        // A basin emptied under a running press stops it where it is and puts the head up.
        press.ticks = 150;
        press.basin_removed();
        assert!(!press.running);
        assert_eq!(press.ticks, 0);
    }

    #[test]
    fn a_belt_item_is_held_while_the_press_runs_and_taken_when_it_has_a_recipe() {
        // `BeltPressingCallbacks.onItemReceived`.
        assert_eq!(on_item_received(0.0, false, true), Processing::Pass);
        assert_eq!(on_item_received(32.0, true, false), Processing::Hold);
        assert_eq!(on_item_received(32.0, false, false), Processing::Pass);
        assert_eq!(on_item_received(32.0, false, true), Processing::Hold);
        assert!(starts_on_item(32.0, false, true));
        assert!(!starts_on_item(32.0, true, true));
        assert!(!starts_on_item(0.0, false, true));
        assert!(!starts_on_item(32.0, false, false));
        // `whenItemHeld`: pass when stopped or idle, else hold.
        assert_eq!(while_item_held(32.0, true), Processing::Hold);
        assert_eq!(while_item_held(32.0, false), Processing::Pass);
        assert_eq!(while_item_held(0.0, true), Processing::Pass);
    }

    #[test]
    fn a_stack_on_a_belt_is_pressed_one_item_at_a_time() {
        assert_eq!(belt_split(1), (1, 0));
        assert_eq!(belt_split(5), (1, 4));
    }

    #[test]
    fn a_press_over_a_basin_starts_and_repeats_as_create_s() {
        assert!(looks_at_basin(32.0, false, true));
        assert!(!looks_at_basin(0.0, false, true));
        assert!(!looks_at_basin(32.0, true, true));
        assert!(!looks_at_basin(32.0, false, false));
        // `startProcessingBasin` leaves a cycle on its way down alone.
        assert!(!may_start_on_basin(true, 0));
        assert!(!may_start_on_basin(true, HALF));
        assert!(may_start_on_basin(true, HALF + 4));
        assert!(may_start_on_basin(false, 0));
        // `onPressingCompleted`.
        assert_eq!(after_cycle(Mode::Basin, true, true), Next::Again);
        assert_eq!(after_cycle(Mode::Basin, false, true), Next::Recheck);
        assert_eq!(after_cycle(Mode::Basin, true, false), Next::Recheck);
        assert_eq!(after_cycle(Mode::Belt, true, true), Next::Recheck);
    }

    #[test]
    fn chances_roll_as_create_s() {
        assert!(rolls(1.0, 0.99));
        assert!(rolls(0.5, 0.5));
        assert!(!rolls(0.5, 0.51));
    }
}
