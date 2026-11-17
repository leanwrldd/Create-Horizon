//! The rules of Create's block breakers: the Mechanical Drill and the Mechanical Saw facing
//! sideways. Ported from `BlockBreakingKineticBlockEntity`, `DrillBlock`, `DrillBlockEntity`
//! and `IWrenchable` (MIT, the Creators of Create), checked against `mc1.21.1/dev` at 4148fd8.
//!
//! No game types: the glue (`native/api/src/machines/drill.rs`) reads and breaks the blocks.

use crate::{Axis, Dir};

pub use super::drill_hardness::{HARDNESS, LIQUIDS};

/// Stress units per RPM (`AllBlocks.MECHANICAL_DRILL`: `CStress.setImpact(4.0)`; the saw's is the
/// same).
pub const IMPACT: f32 = 4.0;
/// The hardness of a block no table knows (another mod's): a stone-like 2, as
/// `SharedProperties.stone` blocks have. Horizon's `Host` cannot ask a block its hardness.
pub const UNKNOWN_HARDNESS: f32 = 2.0;
/// `ItemEntity.setDefaultPickUpDelay`: `ItemEntity.pickupDelay = 10`.
pub const PICKUP_DELAY: i16 = 10;
/// `VecHelper.offsetRandomly(center, random, .125f)`: how far a drop starts from the block's
/// centre on each axis.
pub const DROP_SPREAD: f32 = 0.125;
/// How many ticks apart the glue looks for entities in the head. Create asks each tick an entity
/// overlaps (`Block.entityInside`), which a mod's `Host` cannot hear: the scan is
/// `Host::entities_in`, which walks every entity. Entities are hurt at most every 10 ticks anyway
/// (`LivingEntity.invulnerableTime`).
pub const DAMAGE_SCAN: i64 = 4;

/// The block's hardness, `BlockState.getDestroySpeed`: -1 for unbreakable.
pub fn hardness(block: &str) -> f32 {
    HARDNESS
        .binary_search_by(|(name, _)| (*name).cmp(block))
        .map_or(UNKNOWN_HARDNESS, |i| HARDNESS[i].1)
}

/// `BlockState.liquid()`.
pub fn is_liquid(block: &str) -> bool {
    LIQUIDS.contains(&block)
}

/// `block instanceof AirBlock`.
pub fn is_air(block: &str) -> bool {
    matches!(
        block,
        "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
    )
}

/// `BlockBreakingKineticBlockEntity.isBreakable`: not a liquid, not air, not unbreakable
/// (hardness -1) and not in `create:non_breakable`.
pub fn is_breakable(block: &str, hardness: f32, non_breakable: bool) -> bool {
    !(is_liquid(block) || is_air(block) || hardness == -1.0 || non_breakable)
}

/// `BlockBreakingKineticBlockEntity.getBreakSpeed`: `abs(speed / 100)`.
pub fn break_speed(rpm: f32) -> f32 {
    (rpm / 100.0).abs()
}

/// `DrillBlock.getDamage`: what an entity in the head takes each hit.
pub fn damage(rpm: f32) -> f32 {
    let speed = f64::from(rpm.abs());
    let sub1 = (speed / 16.0).min(2.0);
    let sub2 = (speed / 32.0).min(4.0);
    let sub3 = (speed / 64.0).min(4.0);
    (sub1 + sub2 + sub3).clamp(1.0, 10.0) as f32
}

/// `DrillBlock.entityInside`: an entity hurts only where its box meets the block's, shrunk by
/// 0.1 on every side (`new AABB(pos).deflate(.1f).intersects(box)`, strict on every axis).
/// `bounds` is the entity's lowest and highest corner (`Host::entity_bounding_box`).
pub fn in_head(bounds: [[f64; 3]; 2], pos: [i32; 3]) -> bool {
    (0..3).all(|i| {
        let lo = f64::from(pos[i]) + f64::from(0.1_f32);
        let hi = f64::from(pos[i]) + 1.0 - f64::from(0.1_f32);
        bounds[0][i] < hi && bounds[1][i] > lo
    })
}

/// A breaker's block entity state (`BlockBreakingKineticBlockEntity`): saved as `Progress` and
/// `NextTick`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Breaker {
    /// `destroyProgress`: the crack's stage, 0 to 9.
    pub progress: i32,
    /// `ticksUntilNextProgress`: -1 waits for the next lazy tick.
    pub next: i32,
}

/// What the block in front is, when a tick looks at it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    /// `BlockState.getDestroySpeed`.
    pub hardness: f32,
    /// `canBreak`: for the saw, also `isSawable`.
    pub breakable: bool,
}

/// What one tick of a breaker did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing to do this tick.
    Wait,
    /// The block in front can't be broken: `clear` says a crack was showing, to remove.
    Blocked { clear: bool },
    /// The crack moved on to `stage` (0 to 9).
    Cracking { stage: i32 },
    /// The block breaks: drop it, and take the crack away.
    Broken,
}

impl Breaker {
    /// `destroyNextTick`.
    pub fn destroy_next_tick(&mut self) {
        self.next = 1;
    }

    /// `lazyTick`: a breaker that finished (or found nothing to break) tries again. Create's
    /// `lazyTickRate` is 0, so this runs every tick, before the rest of `tick`.
    pub fn lazy_tick(&mut self) {
        if self.next == -1 {
            self.destroy_next_tick();
        }
    }

    /// The part of `BlockBreakingKineticBlockEntity.tick` after the checks that the breaker
    /// should run and turns (`shouldRun`, `getSpeed() != 0`): `target` looks at the block in
    /// front, only when this tick gets that far.
    pub fn tick(&mut self, rpm: f32, target: impl FnOnce() -> Target) -> Step {
        if self.next < 0 {
            return Step::Wait;
        }
        // `ticksUntilNextProgress-- > 0`.
        let waiting = self.next > 0;
        self.next -= 1;
        if waiting {
            return Step::Wait;
        }
        let target = target();
        if !target.breakable {
            let clear = self.progress != 0;
            self.progress = 0;
            return Step::Blocked { clear };
        }
        let speed = break_speed(rpm);
        // `Mth.clamp((int) (breakSpeed / blockHardness), 1, 10 - destroyProgress)`; the cast
        // saturates (and sends NaN to 0), as Java's does.
        let max = 10 - self.progress;
        self.progress += ((speed / target.hardness) as i32).clamp(1, max.max(1));
        if self.progress >= 10 {
            self.progress = 0;
            self.next = -1;
            return Step::Broken;
        }
        self.next = (target.hardness / speed) as i32;
        Step::Cracking {
            stage: self.progress,
        }
    }
}

/// `DirectionalKineticBlock.getStateForPlacement`, without the neighbouring shaft it prefers
/// (`getPreferredFacing`): the block faces the player, or away from them when sneaking.
/// `look` is `Direction.orderedByNearest(player)[0]`.
pub fn placement_facing(look: Dir, sneaking: bool) -> Dir {
    if sneaking { look } else { look.opposite() }
}

/// `Direction.getClockWise(Axis)`: `facing` a quarter turn clockwise seen from the positive end
/// of `axis`; unchanged when it lies along the axis.
pub fn clockwise(facing: Dir, axis: Axis) -> Dir {
    facing.clockwise_about(axis)
}

/// The drill's `facing` after a Wrench click on `clicked` face (`IWrenchable.getRotatedBlockState`
/// for a `DirectionalKineticBlock`): it turns round that face's axis, and stays when it points
/// along it.
pub fn wrench_facing(facing: Dir, clicked: Dir) -> Dir {
    if facing.axis() == clicked.axis() {
        facing
    } else {
        clockwise(facing, clicked.axis())
    }
}

/// A stable negative id for the crack of the breaker at `pos`
/// (`BlockBreakingKineticBlockEntity.breakerId` is a fresh negative number per block entity;
/// Create-Horizon's has no per-entity state to hold one, so it comes from the position).
pub fn breaker_id(pos: [i32; 3]) -> i32 {
    let hash = (pos[0].wrapping_mul(73_856_093)
        ^ pos[1].wrapping_mul(19_349_663)
        ^ pos[2].wrapping_mul(83_492_791))
        & 0x3FFF_FFFF;
    -(hash + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stone() -> Target {
        Target {
            hardness: 1.5,
            breakable: true,
        }
    }

    /// Runs the breaker like the block entity's tick does and counts ticks until it breaks.
    fn ticks_to_break(rpm: f32, hardness: f32, limit: i32) -> Option<i32> {
        let mut breaker = Breaker::default();
        for tick in 1..=limit {
            breaker.lazy_tick();
            let target = Target {
                hardness,
                breakable: true,
            };
            if breaker.tick(rpm, || target) == Step::Broken {
                return Some(tick);
            }
        }
        None
    }

    #[test]
    fn hardness_comes_from_the_table() {
        assert_eq!(hardness("minecraft:stone"), 1.5);
        assert_eq!(hardness("minecraft:obsidian"), 50.0);
        assert_eq!(hardness("minecraft:bedrock"), -1.0);
        assert_eq!(hardness("minecraft:dirt"), 0.5);
        assert_eq!(hardness("mymod:unknown"), UNKNOWN_HARDNESS);
    }

    #[test]
    fn the_table_is_sorted_for_the_search() {
        assert!(HARDNESS.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn what_can_be_broken() {
        assert!(is_breakable("minecraft:stone", 1.5, false));
        assert!(!is_breakable("minecraft:water", 100.0, false));
        assert!(!is_breakable("minecraft:lava", 100.0, false));
        assert!(!is_breakable("minecraft:air", 0.0, false));
        assert!(!is_breakable("minecraft:bedrock", -1.0, false));
        assert!(!is_breakable("minecraft:stone", 1.5, true));
        // Not unbreakable: obsidian is slow, not refused.
        assert!(is_breakable("minecraft:obsidian", 50.0, false));
    }

    #[test]
    fn break_speed_is_a_hundredth_of_the_rpm() {
        assert_eq!(break_speed(32.0), 0.32);
        assert_eq!(break_speed(-64.0), 0.64);
    }

    #[test]
    fn damage_grows_with_speed_up_to_ten() {
        // 16/16 + 16/32 + 16/64 = 1.75
        assert_eq!(damage(16.0), 1.75);
        assert_eq!(damage(-16.0), 1.75);
        // A crawl still hurts: clamped up to 1.
        assert_eq!(damage(1.0), 1.0);
        // 2 + 4 + 4 at 256 RPM, the most the three terms give.
        assert_eq!(damage(256.0), 10.0);
        assert_eq!(damage(100.0), 2.0 + 100.0 / 32.0 + 100.0 / 64.0);
    }

    #[test]
    fn the_head_is_a_block_shrunk_by_a_tenth() {
        let pos = [10, 64, 10];
        // Standing inside.
        assert!(in_head([[10.2, 64.0, 10.2], [10.8, 65.8, 10.8]], pos));
        // Only brushing the shrunk edge.
        assert!(!in_head([[10.95, 64.0, 10.2], [11.5, 65.8, 10.8]], pos));
        assert!(in_head([[10.85, 64.0, 10.2], [11.5, 65.8, 10.8]], pos));
        // Next door.
        assert!(!in_head([[11.2, 64.0, 10.2], [11.8, 65.8, 10.8]], pos));
    }

    #[test]
    fn a_new_breaker_starts_at_once() {
        let mut breaker = Breaker::default();
        breaker.lazy_tick();
        // `ticksUntilNextProgress` starts at 0: the first tick evaluates.
        assert_eq!(
            breaker.tick(32.0, stone),
            Step::Cracking { stage: 1 }
        );
    }

    #[test]
    fn stone_at_32_rpm_takes_a_few_steps() {
        // breakSpeed 0.32, hardness 1.5: (int) (0.32 / 1.5) = 0, clamped to 1 a step, then
        // (int) (1.5 / 0.32) = 4 ticks until the next step.
        let mut breaker = Breaker::default();
        assert_eq!(breaker.tick(32.0, stone), Step::Cracking { stage: 1 });
        assert_eq!(breaker.next, 4);
        // Four waiting ticks, the fifth looks again.
        for _ in 0..4 {
            assert_eq!(breaker.tick(32.0, stone), Step::Wait);
        }
        assert_eq!(breaker.tick(32.0, stone), Step::Cracking { stage: 2 });
    }

    #[test]
    fn a_quick_drill_skips_steps() {
        // 256 RPM on dirt (hardness 0.5): 2.56 / 0.5 = 5 stages a step.
        let mut breaker = Breaker::default();
        let dirt = || Target {
            hardness: 0.5,
            breakable: true,
        };
        assert_eq!(breaker.tick(256.0, dirt), Step::Cracking { stage: 5 });
        assert_eq!(breaker.next, 0);
        // The next tick looks again at once, and finishes (5 + 5).
        assert_eq!(breaker.tick(256.0, dirt), Step::Broken);
        assert_eq!(breaker.progress, 0);
        assert_eq!(breaker.next, -1);
    }

    #[test]
    fn the_last_step_never_overshoots() {
        let mut breaker = Breaker {
            progress: 9,
            next: 0,
        };
        // Even a slow one finishes with the one step left.
        assert_eq!(breaker.tick(1.0, stone), Step::Broken);
    }

    #[test]
    fn a_block_with_no_hardness_breaks_in_one_look() {
        let mut breaker = Breaker::default();
        let leaves = || Target {
            hardness: 0.0,
            breakable: true,
        };
        // speed / 0 is infinity, saturating to the clamp's top.
        assert_eq!(breaker.tick(8.0, leaves), Step::Broken);
    }

    #[test]
    fn finished_breakers_look_again_every_other_tick() {
        let mut breaker = Breaker {
            progress: 0,
            next: -1,
        };
        let nothing = || Target {
            hardness: 0.0,
            breakable: false,
        };
        let mut looks = 0;
        for _ in 0..10 {
            breaker.lazy_tick();
            let mut looked = false;
            let step = breaker.tick(32.0, || {
                looked = true;
                nothing()
            });
            if looked {
                looks += 1;
                assert_eq!(step, Step::Blocked { clear: false });
            }
        }
        // -1 -> 1 -> 0 (wait), then 0 -> -1 (look): one look in two ticks.
        assert_eq!(looks, 5);
    }

    #[test]
    fn blocked_clears_a_crack_once() {
        let mut breaker = Breaker {
            progress: 3,
            next: 0,
        };
        let gone = || Target {
            hardness: 0.0,
            breakable: false,
        };
        assert_eq!(breaker.tick(32.0, gone), Step::Blocked { clear: true });
        assert_eq!(breaker.progress, 0);
        breaker.next = 0;
        assert_eq!(breaker.tick(32.0, gone), Step::Blocked { clear: false });
    }

    #[test]
    fn obsidian_takes_a_long_while_at_speed() {
        // 256 RPM: breakSpeed 2.56; obsidian 50 / 2.56 = 19 ticks a step, ten steps of 1.
        let ticks = ticks_to_break(256.0, 50.0, 10_000).expect("it breaks");
        // Nine waits of 19 ticks between ten steps, each step a tick: about 9 * 20 + 1.
        assert_eq!(ticks, 9 * 20 + 1);
    }

    #[test]
    fn breaker_ids_are_negative_and_stable() {
        let id = breaker_id([5, 64, -3]);
        assert!(id < 0);
        assert_eq!(id, breaker_id([5, 64, -3]));
        assert_ne!(id, breaker_id([6, 64, -3]));
        assert!(breaker_id([i32::MAX, i32::MIN, 17]) < 0);
    }

    #[test]
    fn the_drill_faces_the_player() {
        assert_eq!(placement_facing(Dir::North, false), Dir::South);
        assert_eq!(placement_facing(Dir::Down, false), Dir::Up);
        assert_eq!(placement_facing(Dir::North, true), Dir::North);
    }

    #[test]
    fn the_wrench_turns_the_drill_round_the_clicked_face() {
        assert_eq!(wrench_facing(Dir::North, Dir::Up), Dir::East);
        assert_eq!(wrench_facing(Dir::West, Dir::Up), Dir::North);
        assert_eq!(wrench_facing(Dir::Up, Dir::East), Dir::South);
        assert_eq!(wrench_facing(Dir::Up, Dir::Up), Dir::Up);
        let mut f = Dir::North;
        for _ in 0..4 {
            f = wrench_facing(f, Dir::East);
        }
        assert_eq!(f, Dir::North);
    }
}
