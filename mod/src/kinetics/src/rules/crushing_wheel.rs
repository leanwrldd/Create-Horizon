//! The Crushing Wheels' rules: where the controller between two wheels exists and faces, how fast
//! it crushes, and the processing inventory's clock. Ported from Create's `CrushingWheelBlock`,
//! `CrushingWheelControllerBlock`, `CrushingWheelControllerBlockEntity` and `ProcessingInventory`
//! (MIT, the Creators of Create), checked against `mc1.21.1/dev` at 4148fd8.
//!
//! No game types: the glue is `native/api/src/machines/crushing_wheel.rs`.

use crate::{Axis, Dir};

/// Stress units per RPM (`AllBlocks.CRUSHING_WHEEL`: `CStress.setImpact(8.0)`).
pub const IMPACT: f32 = 8.0;
/// `CKinetics.crushingDamage`, the default of the config.
pub const CRUSHING_DAMAGE: f32 = 4.0;
/// `CrushingWheelBlockEntity` calls `setLazyTickRate(20)`: wheels re-check their controllers this often.
pub const LAZY_TICK: i64 = 20;
/// `ProcessingInventory(32)`: slot 0 is the input, the rest the outputs.
pub const SLOTS: usize = 32;
/// `itemInserted`: the time with no recipe.
pub const NO_RECIPE_TIME: f32 = 100.0;
/// `tick`: results are made once `remainingTime` falls under this.
pub const APPLY_BELOW: f32 = 20.0;
/// `CrushingWheelControllerBlockEntity.tick`: `getBoundingBox().intersects(new AABB(pos).inflate(.5f))`.
pub const HOLD_RANGE: f64 = 0.5;

/// `CrushingWheelControllerBlock.updateSpeed`: `abs(wheel speed / 50)`.
pub fn crushing_speed(wheel_rpm: f32) -> f32 {
    (wheel_rpm / 50.0).abs()
}

/// Two wheels crush when both turn and in opposite senses
/// (`(be.getSpeed() > 0) != (otherBE.getSpeed() > 0) && both != 0`).
pub fn turn_against_each_other(a: f32, b: f32) -> bool {
    (a > 0.0) != (b > 0.0) && a != 0.0 && b != 0.0
}

/// The controller's `FACING` between the wheel at the origin (turning `wheel_rpm` about `axis`)
/// and the other wheel two blocks along `side` (`CrushingWheelBlock.updateControllers`):
/// the wheel axis crossed with the side axis, flipped by the sign of the speed and of the side.
pub fn controller_facing(axis: Axis, side: Dir, wheel_rpm: f32) -> Dir {
    let ado = if wheel_rpm > 0.0 {
        1
    } else if wheel_rpm < 0.0 {
        -1
    } else {
        0
    } * side.sign();
    let a = axis.unit().map(|v| v as i32);
    let b = side.axis().unit().map(|v| v as i32);
    // `Vec3.cross`.
    let cross = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    let v = cross.map(|c| c * ado);
    // `Direction.getNearest`: the largest component wins, ties to the earlier of Down..East
    // as Create's `Direction.getNearest` walks the directions; for an axis-aligned vector there is one.
    Dir::ALL
        .into_iter()
        .map(|d| (d, d.normal().iter().zip(v).map(|(n, c)| n * c).sum::<i32>()))
        .max_by_key(|(_, dot)| *dot)
        .map_or(Dir::North, |(d, _)| d)
}

/// What a controller should be, from the wheels around it (`updateControllers` for one side).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Controller {
    pub valid: bool,
    pub facing: Dir,
}

/// The controller wanted at `side` of the wheel at the origin, or `None` for no controller:
/// `other_axis` is the axis of the wheel two blocks away, if one stands there.
pub fn controller_wanted(
    axis: Axis,
    side: Dir,
    rpm: f32,
    other: Option<(Axis, f32)>,
) -> Option<Controller> {
    let (other_axis, other_rpm) = other?;
    if other_axis != axis {
        return None;
    }
    if turn_against_each_other(rpm, other_rpm) {
        Some(Controller {
            valid: true,
            facing: controller_facing(axis, side, rpm),
        })
    } else {
        // `controllerNewDirection = Direction.DOWN`, `controllerShouldBeValid = false`.
        Some(Controller {
            valid: false,
            facing: Dir::Down,
        })
    }
}

/// `CrushingWheelControllerBlockEntity.tick`: where items leave. `Vec3 outSpeed`, in blocks a tick.
pub fn out_speed(facing: Dir) -> [f64; 3] {
    let offset = f64::from(facing.sign());
    let axis = facing.axis();
    [
        if axis == Axis::X { 0.25 * offset } else { 0.0 },
        // Increased upwards speed so upwards crushing wheels shoot the item out properly; no
        // downwards speed, so downwards ones drop items as before.
        if facing.sign() == 1 && axis == Axis::Y { 0.5 } else { 0.0 },
        if axis == Axis::Z { 0.25 * offset } else { 0.0 },
    ]
}

/// `outPos`: the block's centre pushed `.55f` out along the facing.
pub fn out_pos(centre: [f64; 3], facing: Dir) -> [f64; 3] {
    along(centre, facing, f64::from(0.55f32))
}

/// `entityOutPos`: another `.5f` further, where a crushed mob is moved to so its drops land there.
pub fn entity_out_pos(centre: [f64; 3], facing: Dir) -> [f64; 3] {
    along(out_pos(centre, facing), facing, f64::from(0.5f32))
}

fn along(mut at: [f64; 3], facing: Dir, by: f64) -> [f64; 3] {
    at[facing.axis().index()] += by * f64::from(facing.sign());
    at
}

/// The velocity a crushed entity is given: along the facing axis `max(-speed / 4, -.5) * -offset`
/// (`speed = crushingspeed * 4`), and towards the block's centre across the other two when
/// `centre_pull` is on (a sneaking entity is not pulled). `entity` and `centre` are positions.
pub fn pull(
    facing: Dir,
    crushing_speed: f32,
    entity: [f64; 3],
    centre: [f64; 3],
    centre_pull: bool,
) -> [f64; 3] {
    let speed = crushing_speed * 4.0;
    let movement = f64::from((-speed / 4.0).max(-0.5)) * f64::from(-facing.sign());
    let mut v = [0.0; 3];
    for i in 0..3 {
        v[i] = if i == facing.axis().index() {
            movement
        } else if i == 1 {
            // "Do not move entities upwards or downwards for horizontal crushers."
            0.0
        } else if centre_pull {
            (centre[i] - entity[i]) / 2.0
        } else {
            0.0
        };
    }
    v
}

/// Whether the box `a` (min and max corners) meets the box `b` (`AABB.intersects`).
pub fn boxes_overlap(a: [[f64; 3]; 2], b: [[f64; 3]; 2]) -> bool {
    (0..3).all(|i| a[0][i] < b[1][i] && a[1][i] > b[0][i])
}

/// The box of the block at `pos` grown by `by` on every side (`new AABB(pos).inflate(by)`).
pub fn cell(pos: crate::Pos, by: f64) -> [[f64; 3]; 2] {
    let at = pos.map(f64::from);
    [at.map(|v| v - by), at.map(|v| v + 1.0 + by)]
}

/// `ProcessingInventory`'s clock: `remainingTime` and `appliedRecipe`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clock {
    pub remaining: f32,
    pub applied: bool,
}

/// What the controller does after a tick of the clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Still crushing.
    Wait,
    /// Roll the recipe's results into the output slots now, then call [`Clock::recipe_applied`].
    Apply,
    /// Done: push the outputs out and clear the inventory.
    Output,
}

impl Clock {
    /// `itemInserted`: the recipe's duration, or 100 with none.
    pub fn inserted(recipe_time: Option<i32>) -> Self {
        Clock {
            remaining: recipe_time.map_or(NO_RECIPE_TIME, |t| t as f32),
            applied: false,
        }
    }

    /// `processingSpeed = clamp(speed / (!appliedRecipe ? log2(count) : 1), .25f, 20)` with
    /// `speed = crushingspeed * 4`. One item makes `log2 = 0`, and so 20 (infinity, clamped).
    pub fn speed(&self, crushing_speed: f32, count: u32) -> f32 {
        let divisor = if self.applied {
            1.0
        } else {
            (f64::from(count).ln() as f32) / (std::f64::consts::LN_2 as f32)
        };
        (crushing_speed * 4.0 / divisor).clamp(0.25, 20.0)
    }

    /// One server tick without a processing entity, for `count` items in the input slot.
    pub fn tick(&mut self, crushing_speed: f32, count: u32) -> Phase {
        self.remaining -= self.speed(crushing_speed, count);
        if self.remaining < APPLY_BELOW && !self.applied {
            return Phase::Apply;
        }
        if self.remaining > 0.0 {
            return Phase::Wait;
        }
        self.remaining = 0.0;
        Phase::Output
    }

    /// After `applyRecipe`, which `clear()`s the inventory (the time is zeroed) and `tick` flags it.
    pub fn recipe_applied(&mut self) {
        self.remaining = 0.0;
        self.applied = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crushing_speed_is_a_fiftieth_of_the_wheel() {
        assert_eq!(crushing_speed(-256.0), 5.12);
        assert_eq!(crushing_speed(0.0), 0.0);
    }

    #[test]
    fn only_opposite_turning_wheels_crush() {
        assert!(turn_against_each_other(16.0, -16.0));
        assert!(!turn_against_each_other(16.0, 16.0));
        assert!(!turn_against_each_other(0.0, -16.0));
        assert!(!turn_against_each_other(16.0, 0.0));
    }

    #[test]
    fn the_controller_faces_where_the_wheels_squeeze_to() {
        // Wheels along X (axis X), the other one to the east (side east), this one turning
        // positive: (1,0,0) x (1,0,0)... the sides are across the axis, so take axis Z and side east.
        // axis z x side x = (0,0,1) x (1,0,0) = (0, 1, 0); ado = +1 * +1: up.
        assert_eq!(controller_facing(Axis::Z, Dir::East, 16.0), Dir::Up);
        // The other wheel, seen from the other end (side west) and turning the other way, agrees.
        assert_eq!(controller_facing(Axis::Z, Dir::West, -16.0), Dir::Up);
        // Turning the other way round flips it.
        assert_eq!(controller_facing(Axis::Z, Dir::East, -16.0), Dir::Down);
        // Axis X, side up: (1,0,0) x (0,1,0) = (0,0,1): south when ado is positive.
        assert_eq!(controller_facing(Axis::X, Dir::Up, 8.0), Dir::South);
        assert_eq!(controller_facing(Axis::X, Dir::Down, 8.0), Dir::North);
        // Axis Y, side east: (0,1,0) x (1,0,0) = (0,0,-1): north.
        assert_eq!(controller_facing(Axis::Y, Dir::East, 8.0), Dir::North);
    }

    #[test]
    fn both_wheels_ask_for_the_same_controller() {
        for axis in [Axis::X, Axis::Y, Axis::Z] {
            for side in Dir::ALL.into_iter().filter(|s| s.axis() != axis) {
                let this = controller_wanted(axis, side, 24.0, Some((axis, -24.0)));
                let that = controller_wanted(axis, side.opposite(), -24.0, Some((axis, 24.0)));
                assert_eq!(this, that, "{axis:?} {side:?}");
                assert!(this.unwrap().valid);
            }
        }
    }

    #[test]
    fn a_controller_exists_between_wheels_on_one_axis_and_is_valid_when_they_oppose() {
        let still = controller_wanted(Axis::X, Dir::Up, 0.0, Some((Axis::X, 0.0)));
        assert_eq!(
            still,
            Some(Controller {
                valid: false,
                facing: Dir::Down
            })
        );
        assert_eq!(controller_wanted(Axis::X, Dir::Up, 8.0, None), None);
        assert_eq!(
            controller_wanted(Axis::X, Dir::Up, 8.0, Some((Axis::Z, -8.0))),
            None
        );
    }

    #[test]
    fn items_leave_by_the_facing() {
        assert_eq!(out_speed(Dir::Down), [0.0; 3]);
        assert_eq!(out_speed(Dir::Up), [0.0, 0.5, 0.0]);
        assert_eq!(out_speed(Dir::West), [-0.25, 0.0, 0.0]);
        assert_eq!(out_speed(Dir::South), [0.0, 0.0, 0.25]);
        let at = out_pos([0.5; 3], Dir::Down);
        assert!((at[1] - -0.05).abs() < 1e-6 && at[0] == 0.5);
        let mob = entity_out_pos([0.5; 3], Dir::Down);
        assert!((mob[1] - -0.55).abs() < 1e-6);
    }

    #[test]
    fn entities_are_pulled_along_the_facing_and_to_the_centre() {
        // A downward crusher at crushing speed 1.28 (64 RPM): speed 5.12, -min(5.12 / 4, .5) down.
        let v = pull(Dir::Down, 1.28, [0.2, 0.7, 0.9], [0.5, 0.5, 0.5], true);
        assert_eq!(v[1], -0.5);
        assert!((v[0] - 0.15).abs() < 1e-9 && (v[2] - -0.2).abs() < 1e-9);
        // Up goes up, and a sneaking entity is not centred.
        let v = pull(Dir::Up, 0.2, [0.2, 0.7, 0.9], [0.5; 3], false);
        assert!((v[1] - 0.2).abs() < 1e-6 && v[0] == 0.0 && v[2] == 0.0);
        // Horizontal: no vertical motion.
        let v = pull(Dir::East, 1.0, [0.2, 0.7, 0.9], [0.5; 3], true);
        assert!((v[0] - 1.0f64.min(0.5)).abs() < 1e-9 && v[1] == 0.0);
    }

    #[test]
    fn a_single_item_crushes_at_the_top_speed_and_a_stack_slower() {
        let clock = Clock::inserted(Some(250));
        assert_eq!(clock.speed(1.0, 1), 20.0);
        assert_eq!(clock.speed(1.0, 2), 4.0);
        assert_eq!(clock.speed(1.0, 64), 4.0 / 6.0);
        // Never under a quarter: a crawl on a big stack.
        assert_eq!(clock.speed(0.01, 64), 0.25);
        // Once the results are made it goes on at crushingspeed * 4.
        let mut done = clock;
        done.recipe_applied();
        assert_eq!(done.speed(1.0, 64), 4.0);
    }

    #[test]
    fn the_clock_makes_the_results_at_twenty_and_then_outputs_at_once() {
        let mut clock = Clock::inserted(Some(100));
        let mut ticks = 0;
        // One item, crushing speed 1: 20 a tick.
        while clock.tick(1.0, 1) == Phase::Wait {
            ticks += 1;
        }
        // 100 -> 80 -> 60 -> 40 -> 20 wait (20 is not under 20); the fifth takes it to 0.
        assert_eq!(ticks, 4);
        assert_eq!(clock.remaining, 0.0);
        clock.recipe_applied();
        assert_eq!(clock.tick(1.0, 1), Phase::Output);
        assert_eq!(clock.remaining, 0.0);
    }

    #[test]
    fn boxes_meet_only_when_they_overlap() {
        let block = cell([0, 0, 0], 0.0);
        assert!(boxes_overlap([[0.9, 0.5, 0.5], [1.4, 1.0, 0.7]], block));
        // Touching is not meeting.
        assert!(!boxes_overlap([[1.0, 0.0, 0.0], [1.5, 1.0, 1.0]], block));
        assert!(boxes_overlap([[1.0, 0.0, 0.0], [1.5, 1.0, 1.0]], cell([0, 0, 0], HOLD_RANGE)));
    }

    #[test]
    fn no_recipe_waits_a_hundred() {
        assert_eq!(Clock::inserted(None).remaining, 100.0);
    }
}
