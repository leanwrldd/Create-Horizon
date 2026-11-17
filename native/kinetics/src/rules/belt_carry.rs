//! Players, mobs and other entities riding a belt (Belts III).
//!
//! Ported from Create 6.0.10 (`mc1.21.1/dev` at `4148fd8`, MIT): `BeltBlock.entityInside`
//! (who is taken), `BeltBlock.canTransportObjects`, `BeltMovementHandler.canBeTransported` and
//! `transportEntity` (how far each tick), and the `TransportedEntityInfo` lifetime that the
//! controller's tick keeps (`BeltBlockEntity.tick`).
//!
//! Create moves the entity itself each tick (`Entity.move(MoverType.SELF, movement)`), on the
//! client for the player too. Horizon lets a server mod do that for no one: a player's own game
//! moves them and a mob moves itself. What a mod can add is velocity (`Entity.push`, and a message
//! to a player's client that pushes it), so this works out the move Create makes this tick
//! ([`carry`]) and the push that makes the entity travel that far each tick through ground friction
//! ([`push`]).

use crate::belt::{Part, Slope};
use crate::{Axis, Dir, Pos};

/// `transportEntity`'s "Too slow": a belt turning slower than this carries nothing.
pub const MIN_RPM: f32 = 1.0;
/// `Math.abs(diffCenter) > 48 / 64f`: further than this from the belt's line, an entity is no
/// longer on it.
pub const REACH_ACROSS: f64 = 48.0 / 64.0;
/// `transportEntity`'s "Not on top": the entity's feet must be this far above the belt block's base.
pub const MIN_HEIGHT: f64 = 0.25;
/// `top`: where the belt's flat part ends in the block, 13 pixels.
const TOP: f64 = 13.0 / 16.0;
/// How far above its base block an entity may be and still count as standing on a belt (the
/// belt's collision mask, 19 pixels, and a little more for a step).
pub const MAX_HEIGHT: f64 = 1.3125;
/// The horizontal drag of an entity on the ground in a tick: block friction 0.6 times 0.91
/// (`LivingEntity.travel`).
pub const GROUND_FRICTION: f64 = 0.6 * 0.91;

/// The belt block an entity stands on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Belt {
    pub pos: Pos,
    pub facing: Dir,
    pub slope: Slope,
    pub part: Part,
    /// The belt's speed, RPM, signed (`BeltBlockEntity.getSpeed`).
    pub rpm: f32,
}

/// `BeltBlock.entityInside`'s first checks and `BeltMovementHandler.canBeTransported`: a player
/// who sneaks (Create's cardboard boots aside) or flies is not carried.
pub fn can_ride(sneaking: bool, flying: bool) -> bool {
    !sneaking && !flying
}

/// The blocks an entity with its feet at `feet` may be riding: the one its feet are in
/// (`BeltBlock.entityInside`) and the one below (`updateEntityAfterFallOn`), in that order.
pub fn cells_under(feet: [f64; 3]) -> [Pos; 2] {
    let at = feet.map(|v| v.floor() as i32);
    [at, [at[0], at[1] - 1, at[2]]]
}

/// One tick's carry, in blocks (`transportEntity`'s `movement`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Carry {
    /// Create's move: along the belt, and up or down it on a slope (as many blocks up as along),
    /// plus the pull towards the belt's line.
    pub movement: [f64; 3],
    pub moving_up: bool,
    pub moving_down: bool,
    /// The entity is on the sloped part of the block (`onSlope`).
    pub on_slope: bool,
}

/// `BeltMovementHandler.transportEntity` for an entity with its feet at `feet` on `belt`: what it
/// is moved by this tick, or none where Create leaves it alone (too slow, not on top, too far
/// to the side, a belt that only turns).
///
/// `centered` leaves out the pull towards the belt's line, which Create adds only for an entity that
/// isn't walking (`livingEntity.zza == 0 && livingEntity.xxa == 0`), something a mod can't read.
pub fn carry(belt: &Belt, feet: [f64; 3], centered: bool) -> Option<Carry> {
    if !belt.slope.transports() || belt.rpm.abs() < MIN_RPM {
        return None;
    }
    let pos = belt.pos;
    let height = feet[1] - f64::from(pos[1]);
    // "Not on top": `entityIn.getY() - .25f < pos.getY()`.
    if height - MIN_HEIGHT < 0.0 {
        return None;
    }
    let axis = belt.facing.axis();
    let speed = f64::from(belt.rpm / 480.0);
    // `Direction.get(axis == Axis.X ? NEGATIVE : POSITIVE, axis)`.
    let along = if axis == Axis::X { Dir::West } else { Dir::South }.normal().map(f64::from);
    let mut movement = [along[0] * speed, 0.0, along[2] * speed];
    let diff_center = if axis == Axis::Z {
        f64::from(pos[0]) + 0.5 - feet[0]
    } else {
        f64::from(pos[2]) + 0.5 - feet[2]
    };
    if diff_center.abs() > REACH_ACROSS {
        return None;
    }
    // `onSlope`.
    let upward = belt.slope == Slope::Upward;
    let on_slope = belt.slope.is_diagonal()
        && (matches!(belt.part, Part::Middle | Part::Pulley)
            || (belt.part == if upward { Part::End } else { Part::Start } && height < TOP)
            || (belt.part == if upward { Part::Start } else { Part::End } && height > TOP));
    // `movementFacing`: along the axis, the positive way for a negative speed (not the way the
    // entity moves along z: the swap below makes up for it, as Create's does).
    let facing_positive = belt.facing.sign() > 0;
    let movement_positive = belt.rpm < 0.0;
    let same = movement_positive == facing_positive;
    let mut moving_down = on_slope && belt.slope == if same { Slope::Downward } else { Slope::Upward };
    let mut moving_up = on_slope && belt.slope == if same { Slope::Upward } else { Slope::Downward };
    if axis == Axis::Z {
        std::mem::swap(&mut moving_down, &mut moving_up);
    }
    let along_speed = movement[axis.index()].abs();
    if moving_up {
        movement[1] += along_speed;
    }
    if moving_down {
        movement[1] -= along_speed;
    }
    if centered {
        // `centeringDirection`: positive along the axis across the belt.
        let across = crate::belt::rotation_axis(belt.facing).index();
        movement[across] += diff_center * speed.abs().min(0.1) * 4.0;
    }
    Some(Carry { movement, moving_up, moving_down, on_slope })
}

/// The velocity to add to an entity that stands on ground so that, with the drag of `friction`
/// each tick, it travels `carry.movement` a tick: added every tick it settles at that speed
/// (`push / (1 - friction)`). Only the horizontal part is pushed: up a slope the entity climbs the
/// belt's stair-stepped collision as it is pushed against it, and down it falls onto the next step.
pub fn push(carry: &Carry, friction: f64) -> [f64; 3] {
    let share = 1.0 - friction;
    [carry.movement[0] * share, 0.0, carry.movement[2] * share]
}

/// `TransportedEntityInfo.getTicksSinceLastCollision` against `BeltBlockEntity.tick`'s
/// `leftTheBelt`: whether an entity last touched a belt `ticks` ago is still carried. Kept for a
/// caller that remembers its passengers; one that checks each tick doesn't need it.
pub fn still_riding(ticks_since_contact: i32, slope: Slope) -> bool {
    ticks_since_contact <= if slope != Slope::Horizontal { 3 } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn belt(facing: Dir, slope: Slope, part: Part, rpm: f32) -> Belt {
        Belt { pos: [10, 4, 10], facing, slope, part, rpm }
    }

    /// Feet on top of the block at the middle of its width, `height` up.
    fn on(b: &Belt, height: f64) -> [f64; 3] {
        [f64::from(b.pos[0]) + 0.5, f64::from(b.pos[1]) + height, f64::from(b.pos[2]) + 0.5]
    }

    #[test]
    fn a_flat_belt_moves_what_stands_on_it_at_its_speed_along_it() {
        // 48 RPM is a tenth of a block a tick; along x the top moves west at a positive speed
        // and along z south (`Direction.get(axis == X ? NEGATIVE : POSITIVE, axis)`).
        let east = belt(Dir::East, Slope::Horizontal, Part::Middle, 48.0);
        let c = carry(&east, on(&east, 0.8125), false).unwrap();
        assert!((c.movement[0] + 0.1).abs() < 1e-6 && c.movement[1] == 0.0 && c.movement[2] == 0.0);
        let south = belt(Dir::South, Slope::Horizontal, Part::Middle, 48.0);
        let c = carry(&south, on(&south, 0.8125), false).unwrap();
        assert!((c.movement[2] - 0.1).abs() < 1e-6 && c.movement[0] == 0.0);
        // A reversed belt runs the other way, and the facing doesn't matter along an axis.
        let back = belt(Dir::North, Slope::Horizontal, Part::Start, -48.0);
        let c = carry(&back, on(&back, 0.8125), false).unwrap();
        assert!((c.movement[2] + 0.1).abs() < 1e-6);
        assert!(!c.moving_up && !c.moving_down && !c.on_slope);
    }

    #[test]
    fn nothing_is_carried_when_too_slow_off_the_top_or_off_to_the_side() {
        let b = belt(Dir::South, Slope::Horizontal, Part::Middle, 0.5);
        assert!(carry(&b, on(&b, 0.8125), false).is_none());
        let b = belt(Dir::South, Slope::Horizontal, Part::Middle, 16.0);
        assert!(carry(&b, on(&b, 0.8125), false).is_some());
        // Feet lower than a quarter block above the block's base: not on top.
        assert!(carry(&b, on(&b, 0.2), false).is_none());
        assert!(carry(&b, on(&b, 0.25), false).is_some());
        // Further across than 48/64 from the belt's middle.
        let mut feet = on(&b, 0.8125);
        feet[0] += 0.76;
        assert!(carry(&b, feet, false).is_none());
        feet[0] -= 0.1;
        assert!(carry(&b, feet, false).is_some());
        // A belt that only turns carries nothing.
        for slope in [Slope::Vertical, Slope::Sideways] {
            let b = belt(Dir::South, slope, Part::Middle, 16.0);
            assert!(carry(&b, on(&b, 0.8125), false).is_none());
        }
    }

    #[test]
    fn a_sneaking_or_flying_player_is_not_carried() {
        assert!(can_ride(false, false));
        assert!(!can_ride(true, false));
        assert!(!can_ride(false, true));
    }

    #[test]
    fn on_a_slope_the_move_climbs_as_far_as_it_goes_along() {
        // Facing east and turning backwards (the top moves east), up: with the facing, so up.
        let up = belt(Dir::East, Slope::Upward, Part::Middle, -96.0);
        let c = carry(&up, on(&up, 0.8), false).unwrap();
        assert!(c.moving_up && !c.moving_down && c.on_slope);
        assert!((c.movement[0] - 0.2).abs() < 1e-6 && (c.movement[1] - 0.2).abs() < 1e-6, "{c:?}");
        // The same belt turning the other way takes things west, down the slope.
        let down = belt(Dir::East, Slope::Upward, Part::Middle, 96.0);
        let c = carry(&down, on(&down, 0.8), false).unwrap();
        assert!(c.moving_down && !c.moving_up);
        assert!((c.movement[0] + 0.2).abs() < 1e-6 && (c.movement[1] + 0.2).abs() < 1e-6);
        // Along z the top moves south at a positive speed: up a belt facing south.
        let south = belt(Dir::South, Slope::Upward, Part::Middle, 96.0);
        let c = carry(&south, on(&south, 0.8), false).unwrap();
        assert!(c.moving_up && c.movement[2] > 0.0 && c.movement[1] > 0.0);
        let north = belt(Dir::North, Slope::Upward, Part::Middle, 96.0);
        let c = carry(&north, on(&north, 0.8), false).unwrap();
        assert!(c.moving_down && c.movement[2] > 0.0 && c.movement[1] < 0.0);
        // A downward belt is the upward one the other way round.
        let downward = belt(Dir::South, Slope::Downward, Part::Middle, 96.0);
        assert!(carry(&downward, on(&downward, 0.8), false).unwrap().moving_down);
    }

    #[test]
    fn the_flat_half_of_a_sloped_end_block_does_not_climb() {
        // Upward, so its last block's flat half is the higher one: below the 13 pixel line the
        // entity is on the slope, above it on the flat top.
        let end = belt(Dir::East, Slope::Upward, Part::End, -96.0);
        assert!(carry(&end, on(&end, 0.5), false).unwrap().on_slope);
        assert!(!carry(&end, on(&end, 0.9), false).unwrap().on_slope);
        // The start block is the other way round.
        let start = belt(Dir::East, Slope::Upward, Part::Start, -96.0);
        assert!(!start_on_slope(&start, 0.5));
        assert!(start_on_slope(&start, 0.9));
    }

    fn start_on_slope(b: &Belt, height: f64) -> bool {
        carry(b, on(b, height), false).unwrap().on_slope
    }

    #[test]
    fn the_belt_pulls_a_still_entity_towards_its_line() {
        let b = belt(Dir::South, Slope::Horizontal, Part::Middle, 48.0);
        let mut feet = on(&b, 0.8125);
        feet[0] += 0.3;
        let free = carry(&b, feet, false).unwrap().movement;
        let pulled = carry(&b, feet, true).unwrap().movement;
        // Across the belt (x for a belt along z): towards the middle, by diff * min(speed, .1) * 4.
        assert_eq!(free[0], 0.0);
        assert!((pulled[0] + 0.3 * 0.1 * 4.0).abs() < 1e-6, "{pulled:?}");
        assert_eq!(pulled[2], free[2]);
        // Along x the pull is along z.
        let east = belt(Dir::East, Slope::Horizontal, Part::Middle, 48.0);
        let mut feet = on(&east, 0.8125);
        feet[2] -= 0.2;
        assert!(carry(&east, feet, true).unwrap().movement[2] > 0.0);
    }

    #[test]
    fn a_push_settles_the_entity_at_the_belts_speed_through_ground_drag() {
        let b = belt(Dir::South, Slope::Horizontal, Part::Middle, 96.0);
        let c = carry(&b, on(&b, 0.8125), false).unwrap();
        let kick = push(&c, GROUND_FRICTION);
        assert_eq!(kick[1], 0.0);
        // The entity's velocity each tick: pushed, moved, then dragged.
        let mut velocity = 0.0;
        let mut moved = 0.0;
        for _ in 0..200 {
            velocity += kick[2];
            moved = velocity;
            velocity *= GROUND_FRICTION;
        }
        assert!((moved - c.movement[2]).abs() < 1e-6, "{moved} against {}", c.movement[2]);
        // Nothing climbs by push: the slope's stairs do it.
        let up = belt(Dir::South, Slope::Upward, Part::Middle, 96.0);
        let c = carry(&up, on(&up, 0.8), false).unwrap();
        assert!(c.movement[1] > 0.0);
        assert_eq!(push(&c, GROUND_FRICTION)[1], 0.0);
    }

    #[test]
    fn the_feet_look_in_their_own_block_and_the_one_below() {
        assert_eq!(cells_under([3.4, 5.8, -1.2]), [[3, 5, -2], [3, 4, -2]]);
        assert_eq!(cells_under([3.4, 5.0, 0.0]), [[3, 5, 0], [3, 4, 0]]);
    }

    #[test]
    fn an_entity_off_the_belt_is_let_go_after_a_moment() {
        // `leftTheBelt`: more than 1 tick without touching a flat belt, 3 on a slope.
        assert!(still_riding(1, Slope::Horizontal));
        assert!(!still_riding(2, Slope::Horizontal));
        assert!(still_riding(3, Slope::Upward));
        assert!(!still_riding(4, Slope::Downward));
    }
}
