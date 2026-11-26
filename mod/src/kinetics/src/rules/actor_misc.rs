//! The simple movement behaviours Create registers for vanilla and its own blocks on a moving structure:
//! bells (`BellMovementBehaviour`, `HauntedBellMovementBehaviour`), campfires (`CampfireMovementBehaviour`)
//! and dispensers and droppers (`DispenserMovementBehaviour`, `DropperMovementBehaviour`,
//! `DefaultMountedDispenseBehavior`, `AllMountedDispenseItemBehaviors`). Registered in
//! `AllMovementBehaviours` (`Blocks.BELL`, the campfires, the dispenser and the dropper) and in `AllBlocks`
//! (`.onRegister(movementBehaviour(..))` of the Peculiar Bell, the Haunted Bell and the Desk Bell).
//!
//! Plain values only: the glue (`machines/actor_more.rs`) reads the world and the structure, asks these and
//! applies the answer.
//!
//! Known differences from Create:
//! - `BellMovementBehaviour.isActive` also turns the bell off on a train's carriage; there are no trains here.

use crate::rules::actor::Vec3;
use crate::Pos;

pub const BELL: &str = "minecraft:bell";
pub const PECULIAR_BELL: &str = "create:peculiar_bell";
pub const HAUNTED_BELL: &str = "create:haunted_bell";
pub const DESK_BELL: &str = "create:desk_bell";
pub const CAMPFIRE: &str = "minecraft:campfire";
pub const SOUL_CAMPFIRE: &str = "minecraft:soul_campfire";
pub const DISPENSER: &str = "minecraft:dispenser";
pub const DROPPER: &str = "minecraft:dropper";

/// The dice the rules roll: `RandomSource`'s calls, in the order Create makes them.
pub trait Rng {
    /// `nextFloat` and `nextDouble`: [0, 1).
    fn unit(&mut self) -> f64;
    /// `nextInt(bound)`.
    fn int(&mut self, bound: i32) -> i32;
    /// `nextBoolean`.
    fn boolean(&mut self) -> bool {
        self.unit() >= 0.5
    }
}

// --- bells -----------------------------------------------------------------------------------------

/// Which bell, for its sound and its own additions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BellKind {
    /// `Blocks.BELL`.
    Vanilla,
    /// `PeculiarBellBlock`, an `AbstractBellBlock`.
    Peculiar,
    /// `HauntedBellBlock`, whose behaviour adds a recharge and a pulse (`HauntedBellMovementBehaviour`).
    Haunted,
    /// `DeskBellBlock`.
    Desk,
}

pub fn bell_kind(block: &str) -> Option<BellKind> {
    Some(match block {
        BELL => BellKind::Vanilla,
        PECULIAR_BELL => BellKind::Peculiar,
        HAUNTED_BELL => BellKind::Haunted,
        DESK_BELL => BellKind::Desk,
        _ => return None,
    })
}

/// A sound to play: id, volume, pitch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sound {
    pub id: &'static str,
    pub volume: f32,
    pub pitch: f32,
}

/// `BellMovementBehaviour.playSound`: the desk bell's own (`AllSoundEvents.DESK_BELL_USE.play`, 1 and 1), the
/// peculiar bell's (`PECULIAR_BELL_USE.playOnServer(world, pos, 2f, 0.94f)`, vanilla's `BELL_BLOCK`), the haunted
/// bell's (`HAUNTED_BELL_USE.playOnServer(world, pos, 4f, 1f)`) and vanilla's (`BELL_BLOCK`, 2 and 1).
pub fn bell_sound(kind: BellKind) -> Sound {
    match kind {
        BellKind::Desk => Sound { id: "create:desk_bell", volume: 1.0, pitch: 1.0 },
        BellKind::Peculiar => Sound { id: "create:peculiar_bell_use", volume: 2.0, pitch: 0.94 },
        BellKind::Haunted => Sound { id: "create:haunted_bell_use", volume: 4.0, pitch: 1.0 },
        BellKind::Vanilla => Sound { id: "minecraft:block.bell.use", volume: 2.0, pitch: 1.0 },
    }
}

/// `BellMovementBehaviour.onSpeedChanged` (not on an elevator): the bell rings when the structure turns
/// round (`oldMotion . motion <= 0` while it moves relative to itself) or at the very first movement.
pub fn rings_on_speed_change(old: Vec3, new: Vec3, moving_relatively: bool, first_movement: bool) -> bool {
    let dot = old[0] * new[0] + old[1] * new[1] + old[2] * new[2];
    (dot <= 0.0 && moving_relatively) || first_movement
}

/// `BellMovementBehaviour.tick`: `moved` is `temporaryData` (an elevator sets it while the cabin has not
/// arrived and rings once it has). `arrived` is `Some` on an elevator's structure. Answers the new `moved`
/// and whether the bell rings now.
pub fn bell_tick(moved: bool, arrived: Option<bool>) -> (bool, bool) {
    match arrived {
        Some(false) => (true, false),
        _ if moved => (false, true),
        _ => (moved, false),
    }
}

/// `HauntedBellBlockEntity.RECHARGE_TICKS`: the wait after a haunted bell's pulse.
pub const HAUNTED_RECHARGE: i32 = 65;

/// `HauntedBellMovementBehaviour.tick`: the recharge counts down.
pub fn haunted_tick(recharge: i32) -> i32 {
    if recharge > 0 { recharge - 1 } else { recharge }
}

/// `HauntedBellMovementBehaviour.visitNewPosition`: with the recharge at 0 the bell pulses, rings and starts
/// recharging. Answers the new recharge if it did.
pub fn haunted_visit(recharge: i32) -> Option<i32> {
    (recharge == 0).then_some(HAUNTED_RECHARGE)
}

// --- campfires -------------------------------------------------------------------------------------

/// One puff of smoke.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Smoke {
    /// `CAMPFIRE_SIGNAL_SMOKE`, else `CAMPFIRE_COSY_SMOKE`.
    pub signal: bool,
    pub at: Vec3,
}

/// The puffs' velocity: `0.0D, 0.07D, 0.0D`.
pub const SMOKE_MOTION: Vec3 = [0.0, 0.07, 0.0];

/// `CampfireMovementBehaviour.tick` (the client's half; `context.disabled` and an unlit campfire do nothing):
/// 11% of the ticks, two to three puffs ("`i < random.nextInt(2) + 2`" is rolled again for every puff, as in
/// Create), each near the campfire's position.
pub fn campfire_smoke(lit: bool, disabled: bool, signal: bool, position: Vec3, rng: &mut dyn Rng) -> Vec<Smoke> {
    let mut out = Vec::new();
    if !lit || disabled {
        return out;
    }
    if rng.unit() < f64::from(0.11f32) {
        let mut i = 0;
        while i < rng.int(2) + 2 {
            // `random.nextDouble() / (random.nextBoolean() ? 3D : -3D)`, in that order.
            let side = |rng: &mut dyn Rng| {
                let d = rng.unit();
                d / if rng.boolean() { 3.0 } else { -3.0 }
            };
            let x = position[0] + side(rng);
            let y = position[1] + rng.unit() + rng.unit();
            let z = position[2] + side(rng);
            out.push(Smoke { signal, at: [x, y, z] });
            i += 1;
        }
    }
    out
}

// --- dispensers ------------------------------------------------------------------------------------

pub const DISPENSE_SOUND: Sound = Sound { id: "minecraft:block.dispenser.dispense", volume: 1.0, pitch: 1.0 };
pub const FAIL_SOUND: Sound = Sound { id: "minecraft:block.dispenser.fail", volume: 1.0, pitch: 1.2 };
pub const TNT_SOUND: Sound = Sound { id: "minecraft:entity.tnt.primed", volume: 1.0, pitch: 1.0 };

/// The dispenser's slots (`DispenserBlockEntity`: 3 by 3).
pub const DISPENSER_SLOTS: usize = 9;

/// `DropperMovementBehaviour.getSlot`, the top-off: a stack of one that could be bigger takes what the
/// structure's inventory has of the same item up to the stack's maximum. Answers how many to ask for.
pub fn top_off_wanted(count: i32, max: i32) -> Option<i32> {
    (count == 1 && max != 1).then_some(max - count)
}

/// `DropperMovementBehaviour.getSlot`'s choice among the filled slots: none, the only one, or a random one
/// (`Util.getRandom(list, random)`, one `nextInt(size)`).
pub fn pick_slot(filled: &[usize], rng: &mut dyn Rng) -> Option<usize> {
    match filled.len() {
        0 => None,
        1 => Some(filled[0]),
        n => Some(filled[rng.int(n as i32) as usize]),
    }
}

/// `Direction.getNearest`.
pub fn closest(facing: Vec3) -> crate::Dir {
    crate::rules::portable_interface::nearest_direction(facing)
}

/// A gaussian from two uniform draws (Box-Muller; Java's `nextGaussian` is the polar method, the same law).
pub fn gaussian(u1: f64, u2: f64) -> f64 {
    (-2.0 * (1.0 - u1).ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
}

/// `DefaultMountedDispenseBehavior.spawnItem`: where the item appears (`pos + facing + .5`, lowered by .125 on
/// a vertical axis, .15625 on a horizontal one) and its motion (`nextDouble * .1 + .2` along the facing, a
/// `nextGaussian * .0075 * speed` scatter, the structure's motion). `speed` is 6.
pub fn spawn_item(cell: Pos, facing: Vec3, motion: Vec3, rng: &mut dyn Rng) -> (Vec3, Vec3) {
    const SPEED: f64 = 6.0;
    let vertical = matches!(closest(facing), crate::Dir::Up | crate::Dir::Down);
    let at = [
        f64::from(cell[0]) + facing[0] + 0.5,
        f64::from(cell[1]) + facing[1] + 0.5 - if vertical { 0.125 } else { 0.156_25 },
        f64::from(cell[2]) + facing[2] + 0.5,
    ];
    let d3 = rng.unit() * 0.1 + 0.2;
    let mut out = [0.0; 3];
    for axis in 0..3 {
        let g = gaussian(rng.unit(), rng.unit());
        out[axis] = g * 0.0075 * SPEED + facing[axis] * d3 + motion[axis];
    }
    (at, out)
}

/// `AllMountedDispenseItemBehaviors.TNT`: `pos + facing * .7 + .5`.
pub fn tnt_origin(cell: Pos, facing: Vec3) -> Vec3 {
    [0, 1, 2].map(|a| f64::from(cell[a]) + facing[a] * 0.7 + 0.5)
}

/// `AllMountedDispenseItemBehaviors.SPAWN_EGG`: the block the mob is spawned at (`pos.offset(BlockPos
/// .containing(facing + .7))`), whether it is aligned (`facing.y < .5`), and the motion it leaves with (twice
/// the structure's).
pub fn spawn_egg_cell(cell: Pos, facing: Vec3) -> Pos {
    [0, 1, 2].map(|a| cell[a] + (facing[a] + 0.7).floor() as i32)
}

pub fn spawn_egg_motion(motion: Vec3) -> Vec3 {
    motion.map(|c| c * 2.0)
}

/// The entity a spawn egg item makes (`SpawnEggItem.getType`): the item's name without `_spawn_egg`.
pub fn egg_entity(item: &str) -> Option<String> {
    let (namespace, name) = item.split_once(':')?;
    name.strip_suffix("_spawn_egg").map(|mob| format!("{namespace}:{mob}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(Vec<f64>, usize);

    impl Fixed {
        fn new(values: &[f64]) -> Fixed {
            Fixed(values.to_vec(), 0)
        }
    }

    impl Rng for Fixed {
        fn unit(&mut self) -> f64 {
            let v = self.0[self.1 % self.0.len()];
            self.1 += 1;
            v
        }
        fn int(&mut self, bound: i32) -> i32 {
            ((self.unit() * f64::from(bound)) as i32).min(bound - 1)
        }
    }

    #[test]
    fn the_blocks_have_their_behaviours() {
        use crate::rules::actor::{kind_of, Kind};
        assert_eq!(kind_of("minecraft:bell"), Some(Kind::Bell(BellKind::Vanilla)));
        assert_eq!(kind_of("create:haunted_bell"), Some(Kind::Bell(BellKind::Haunted)));
        assert_eq!(kind_of("minecraft:campfire"), Some(Kind::Campfire));
        assert_eq!(kind_of("minecraft:soul_campfire"), Some(Kind::Campfire));
        assert_eq!(kind_of("minecraft:dispenser"), Some(Kind::Dispenser { dropper: false }));
        assert_eq!(kind_of("minecraft:dropper"), Some(Kind::Dispenser { dropper: true }));
    }

    #[test]
    fn the_registered_bells() {
        assert_eq!(bell_kind("minecraft:bell"), Some(BellKind::Vanilla));
        assert_eq!(bell_kind("create:peculiar_bell"), Some(BellKind::Peculiar));
        assert_eq!(bell_kind("create:haunted_bell"), Some(BellKind::Haunted));
        assert_eq!(bell_kind("create:desk_bell"), Some(BellKind::Desk));
        assert_eq!(bell_kind("minecraft:campfire"), None);
    }

    #[test]
    fn each_bell_has_its_sound() {
        assert_eq!(bell_sound(BellKind::Vanilla), Sound { id: "minecraft:block.bell.use", volume: 2.0, pitch: 1.0 });
        assert_eq!(bell_sound(BellKind::Peculiar).pitch, 0.94);
        assert_eq!(bell_sound(BellKind::Haunted).volume, 4.0);
        assert_eq!(bell_sound(BellKind::Desk).id, "create:desk_bell");
    }

    #[test]
    fn a_bell_rings_when_the_motion_turns_round_or_at_the_first_movement() {
        // Going on in the same direction: no.
        assert!(!rings_on_speed_change([0.0, 0.1, 0.0], [0.0, 0.2, 0.0], true, false));
        // Reversing (dot < 0) and stopping (dot = 0) while it moves relatively: yes.
        assert!(rings_on_speed_change([0.0, 0.1, 0.0], [0.0, -0.1, 0.0], true, false));
        assert!(rings_on_speed_change([0.0, 0.1, 0.0], [0.0, 0.0, 0.0], true, false));
        // Reversing with no relative motion: no, unless it is the first movement.
        assert!(!rings_on_speed_change([0.0, 0.1, 0.0], [0.0, -0.1, 0.0], false, false));
        assert!(rings_on_speed_change([0.0, 0.1, 0.0], [0.0, 0.2, 0.0], false, true));
    }

    #[test]
    fn an_elevator_bell_rings_once_the_cabin_arrives() {
        assert_eq!(bell_tick(false, Some(false)), (true, false));
        assert_eq!(bell_tick(true, Some(false)), (true, false));
        assert_eq!(bell_tick(true, Some(true)), (false, true));
        assert_eq!(bell_tick(false, Some(true)), (false, false));
        // Another structure never sets `moved`.
        assert_eq!(bell_tick(false, None), (false, false));
    }

    #[test]
    fn a_haunted_bell_pulses_when_recharged() {
        assert_eq!(haunted_visit(0), Some(65));
        assert_eq!(haunted_visit(3), None);
        assert_eq!(haunted_tick(3), 2);
        assert_eq!(haunted_tick(0), 0);
        // 65 ticks to count down.
        let mut r = HAUNTED_RECHARGE;
        let mut ticks = 0;
        while r > 0 {
            r = haunted_tick(r);
            ticks += 1;
        }
        assert_eq!(ticks, 65);
    }

    #[test]
    fn a_campfire_smokes_one_tick_in_nine_when_lit() {
        let at = [10.0, 20.0, 30.0];
        // 0.5 is over 0.11: nothing.
        assert!(campfire_smoke(true, false, false, at, &mut Fixed::new(&[0.5])).is_empty());
        // 0.05: puffs. nextInt(2) = 0 (0.05 * 2 -> 0) so the loop bound is 2: two puffs.
        let puffs = campfire_smoke(true, false, true, at, &mut Fixed::new(&[0.05, 0.05, 0.05]));
        assert_eq!(puffs.len(), 2);
        assert!(puffs.iter().all(|p| p.signal && p.at[1] >= 20.0 && p.at[1] < 22.0));
        assert!(puffs.iter().all(|p| (p.at[0] - 10.0).abs() <= 1.0 / 3.0 && (p.at[2] - 30.0).abs() <= 1.0 / 3.0));
        // Unlit or disabled: nothing, however lucky.
        assert!(campfire_smoke(false, false, false, at, &mut Fixed::new(&[0.0])).is_empty());
        assert!(campfire_smoke(true, true, false, at, &mut Fixed::new(&[0.0])).is_empty());
    }

    #[test]
    fn the_dropper_tops_off_a_stack_of_one() {
        assert_eq!(top_off_wanted(1, 64), Some(63));
        assert_eq!(top_off_wanted(1, 16), Some(15));
        assert_eq!(top_off_wanted(1, 1), None);
        assert_eq!(top_off_wanted(2, 64), None);
    }

    #[test]
    fn a_slot_is_picked_among_the_filled_ones() {
        assert_eq!(pick_slot(&[], &mut Fixed::new(&[0.5])), None);
        assert_eq!(pick_slot(&[4], &mut Fixed::new(&[0.99])), Some(4));
        assert_eq!(pick_slot(&[1, 5, 7], &mut Fixed::new(&[0.5])), Some(5));
        assert_eq!(pick_slot(&[1, 5, 7], &mut Fixed::new(&[0.0])), Some(1));
    }

    #[test]
    fn a_dispensed_item_leaves_along_the_facing_with_the_structures_motion() {
        // A random of 0.5: d3 = 0.25; the gaussian of (0.5, 0.25) is 0 (cos 90 degrees).
        let (at, motion) = spawn_item([1, 2, 3], [1.0, 0.0, 0.0], [0.1, 0.0, 0.0], &mut Fixed::new(&[0.5, 0.5, 0.25, 0.5, 0.25, 0.5, 0.25]));
        assert_eq!(at, [2.0 + 0.5, 2.0 + 0.5 - 0.156_25, 3.5]);
        assert!((motion[0] - (0.25 + 0.1)).abs() < 1e-9 && motion[1].abs() < 1e-9 && motion[2].abs() < 1e-9);
        // Up: lowered by 0.125.
        let (at, _) = spawn_item([0, 0, 0], [0.0, 1.0, 0.0], [0.0; 3], &mut Fixed::new(&[0.5, 0.5, 0.25, 0.5, 0.25, 0.5, 0.25]));
        assert_eq!(at, [0.5, 1.0 + 0.5 - 0.125, 0.5]);
    }

    #[test]
    fn tnt_and_eggs_leave_from_in_front() {
        assert_eq!(tnt_origin([0, 0, 0], [0.0, 0.0, -1.0]), [0.5, 0.5, 0.5 - 0.7]);
        // `BlockPos.containing(facing + .7)`: facing east is (1.7, 0.7, 0.7): (1, 0, 0).
        assert_eq!(spawn_egg_cell([5, 5, 5], [1.0, 0.0, 0.0]), [6, 5, 5]);
        // facing west: (-0.3, .7, .7) floors to (-1, 0, 0).
        assert_eq!(spawn_egg_cell([5, 5, 5], [-1.0, 0.0, 0.0]), [4, 5, 5]);
        assert_eq!(spawn_egg_motion([0.1, 0.0, -0.2]), [0.2, 0.0, -0.4]);
        assert_eq!(egg_entity("minecraft:cow_spawn_egg").as_deref(), Some("minecraft:cow"));
        assert_eq!(egg_entity("minecraft:stone"), None);
    }
}
