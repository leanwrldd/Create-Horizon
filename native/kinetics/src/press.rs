//! The Mechanical Press: its pressing cycle, what it costs, and how its head moves. Ported from
//! Create's `PressingBehaviour`, `MechanicalPressBlockEntity` and `AllBlocks.MECHANICAL_PRESS`
//! (MIT, the Creators of Create), checked against Create `mc1.21.1/dev` at 4148fd8.
//!
//! This slice presses items lying in the world under the press (`Mode.WORLD`). Belts, depots
//! and basins come later. The game glue scans for items, applies recipes and spawns results;
//! this module only decides when. On the kinetic network a press is a shaft along its facing's
//! axis (`MechanicalPressBlock.getRotationAxis`, `hasShaftTowards`) that costs [`IMPACT`].

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

/// What the glue does after a tick of the press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing this tick.
    Wait,
    /// Look for items to press under the press; call [`Press::start`] if one has a recipe.
    Scan,
    /// The head is down: press what lies under it, then call [`Press::pressed`] if anything was
    /// there. Tell clients.
    Apply,
    /// The press went back up and stopped. Tell clients.
    Finished,
}

/// One press's state. Saved as Create saves it (`Running`, `Ticks`, `Finished`); the scan
/// cooldown isn't saved, as in Create.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Press {
    pub running: bool,
    /// Progress through the cycle, 0 to just past [`CYCLE`].
    pub ticks: i32,
    pub finished: bool,
    pub scan_cooldown: i32,
}

impl Default for Press {
    fn default() -> Self {
        Self {
            running: false,
            ticks: 0,
            finished: false,
            scan_cooldown: ENTITY_SCAN,
        }
    }
}

impl Press {
    /// `PressingBehaviour.start(Mode.WORLD)`. Tell clients.
    pub fn start(&mut self) {
        self.running = true;
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
    fn chances_roll_as_create_s() {
        assert!(rolls(1.0, 0.99));
        assert!(rolls(0.5, 0.5));
        assert!(!rolls(0.5, 0.51));
    }
}
