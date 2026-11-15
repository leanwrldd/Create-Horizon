//! The rules of the Chute, the Smart Chute and the item filter the funnels share: which way an
//! item moves, how the fans' pull and push add up along a chain of chutes, how a chute's shape
//! follows its neighbours, and how a stack is taken out of an inventory. Ported from Create's
//! `ChuteBlockEntity`, `ChuteBlock`, `SmartChuteBlockEntity`, `FilteringBehaviour` and
//! `ItemHelper.extract` (MIT, the Creators of Create), checked against `mc1.21.1/dev` at 4148fd8.
//!
//! No game types: the glue (`native/api/src/machines/chute.rs`) owns the world and the stacks.

use crate::Dir;

/// `ChuteBlockEntity.getItemMotion`: `fanSpeedModifier`, chutes per second a fan's speed moves
/// an item.
pub const FAN_SPEED_MODIFIER: f32 = 1.0 / 64.0;
/// `ChuteBlockEntity.getItemMotion`: `maxItemSpeed`, chutes per second.
pub const MAX_ITEM_SPEED: f32 = 20.0;
/// `ChuteBlockEntity.getItemMotion`: `gravity`, chutes per second.
pub const GRAVITY: f32 = 4.0;
/// `ChuteBlockEntity.tickAirStreams`: ticks between looks for item entities to pull in.
pub const ENTITY_SEARCH: i64 = 5;
/// `ChuteBlockEntity.handleDownwardOutput`: how far below the centre a dropped item appears
/// (blocks, `-12 / 16`).
pub const DROP_BELOW: f64 = -12.0 / 16.0;
/// `ChuteBlockEntity.handleUpwardOutput`: how far above the centre an item rising out of the top
/// appears (`8 / 16`).
pub const DROP_ABOVE: f64 = 8.0 / 16.0;
/// `ChuteBlockEntity.handleDownwardOutput`: the downward speed of a dropped item
/// (blocks a tick, `-.25f`).
pub const DROP_SPEED: f64 = -0.25;
/// `ItemEntity.setDefaultPickUpDelay`.
pub const PICKUP_DELAY: i16 = 10;
/// `CKinetics.fanBlockCheckRate`: ticks between a chute's looks at what is below it.
pub const AIR_CHECK_RATE: i64 = 30;
/// `ChuteBlock.canSurvive`'s `FACING_HOPPER`: down and the four horizontal sides.
pub const FACINGS: [Dir; 5] = [Dir::Down, Dir::North, Dir::South, Dir::West, Dir::East];
/// `Iterate.horizontalDirections`.
pub const HORIZONTALS: [Dir; 4] = [Dir::South, Dir::West, Dir::North, Dir::East];

/// `ChuteBlock.Shape`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Intersection,
    Window,
    Normal,
    Encased,
}

impl Shape {
    pub const fn name(self) -> &'static str {
        match self {
            Shape::Intersection => "intersection",
            Shape::Window => "window",
            Shape::Normal => "normal",
            Shape::Encased => "encased",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [Shape::Intersection, Shape::Window, Shape::Normal, Shape::Encased]
            .into_iter()
            .find(|s| s.name() == name)
    }
}

/// `ChuteBlockEntity.getItemMotion`: chutes an item moves each tick, negative downwards. `push`
/// and `pull` are the fans' speeds (RPM, signed) added up along the chain.
pub fn item_motion(push: f32, pull: f32) -> f32 {
    let motion = (push + pull) * FAN_SPEED_MODIFIER;
    (motion.clamp(-MAX_ITEM_SPEED, MAX_ITEM_SPEED) + if motion <= 0.0 { -GRAVITY } else { 0.0 })
        / 20.0
}

/// `ChuteBlockEntity.updateAirFlow`: how far below the chute a fan below it pulls items up, from
/// `speed` (`pull - push`). Not yet limited by what is in the way.
pub fn max_pull_distance(speed: f32) -> f32 {
    if speed >= 128.0 {
        3.0
    } else if speed >= 64.0 {
        2.0
    } else if speed >= 32.0 {
        1.0
    } else {
        // `Mth.lerp(speed / 32, 0, 1)`.
        speed / 32.0
    }
}

/// `ChuteBlockEntity.calculatePull`: a fan right above, facing down, gives its speed; else the
/// pulls of the chutes feeding this one add up.
pub fn total_pull(fan_above: Option<f32>, input_pulls: impl IntoIterator<Item = f32>) -> f32 {
    match fan_above {
        Some(speed) => speed,
        None => input_pulls.into_iter().sum(),
    }
}

/// `ChuteBlockEntity.calculatePush`: a fan right below, facing up, gives its speed; else the
/// push of the chute this one empties into, shared among the `branches` chutes feeding it.
pub fn total_push(fan_below: Option<f32>, target_push: Option<f32>, branches: usize) -> f32 {
    match (fan_below, target_push) {
        (Some(speed), _) => speed,
        (None, Some(push)) => push / branches.max(1) as f32,
        (None, None) => 0.0,
    }
}

/// What `ChuteBlock.updateChuteState` looks at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Around {
    /// `getChuteFacing(above)`: the facing of the chute above, if the block above is one (a
    /// Smart Chute faces down).
    pub above_chute_facing: Option<Dir>,
    /// The block above is a funnel facing down (`FunnelBlock.getFunnelFacing(above) == DOWN`).
    pub above_funnel_down: bool,
    /// For each of [`HORIZONTALS`]: the block diagonally above on that side is a chute (not a
    /// Smart Chute) facing that way, so it empties into this one.
    pub feeding: [bool; 4],
    /// For a diagonal chute: the block it leads to, below and behind it, is a chute.
    pub target_is_chute: bool,
}

impl Around {
    fn connection(&self, side: Dir) -> bool {
        HORIZONTALS
            .iter()
            .position(|d| *d == side)
            .is_some_and(|i| self.feeding[i])
    }
}

/// `ChuteBlock.updateChuteState`: the facing and shape a chute takes given what is around it.
pub fn update_chute_state(facing: Dir, shape: Shape, around: &Around) -> (Dir, Shape) {
    let vertical = facing == Dir::Down;
    if !vertical && !around.target_is_chute {
        return (Dir::Down, Shape::Normal);
    }
    let amount = around.feeding.iter().filter(|c| **c).count();
    if vertical {
        let shape = if amount == 0 {
            if shape == Shape::Intersection {
                Shape::Normal
            } else {
                shape
            }
        } else {
            Shape::Intersection
        };
        return (facing, shape);
    }
    if amount == 0
        || (around.connection(Dir::North) && around.connection(Dir::South))
        || (around.connection(Dir::East) && around.connection(Dir::West))
    {
        return (facing, Shape::Intersection);
    }
    if amount == 1
        && around.connection(facing)
        && around.above_chute_facing != Some(Dir::Down)
        && !around.above_funnel_down
    {
        return (
            facing,
            if shape == Shape::Encased {
                Shape::Encased
            } else {
                Shape::Normal
            },
        );
    }
    (facing, Shape::Intersection)
}

/// `ChuteBlock.canSurvive`: a chute doesn't hang under one that isn't pointing down.
pub fn can_survive(above_chute_facing: Option<Dir>) -> bool {
    above_chute_facing.is_none_or(|facing| facing == Dir::Down)
}

/// `ChuteBlock.onWrenched` for a chute facing down: a window chute or a plain one swap; an
/// encased one only loses its casing (`Some(shape)`); an intersection does nothing.
pub fn wrenched(shape: Shape, down: bool) -> Option<Shape> {
    match shape {
        Shape::Intersection => None,
        Shape::Encased => Some(Shape::Normal),
        _ if down => Some(if shape != Shape::Normal { Shape::Normal } else { Shape::Window }),
        _ => None,
    }
}

/// `ChuteItem.place` and `ChuteBlock.getStateForPlacement` give a diagonal chute only where one
/// can lead: whether a chute clicked from the side with a chute in hand makes a diagonal one
/// above it (`face` horizontal and not sneaking).
pub fn diagonal_placement(face: Dir, sneaking: bool) -> bool {
    face.axis() != crate::Axis::Y && !sneaking
}

// --- the filter (`FilteringBehaviour`) ---------------------------------------------------------

/// How much a funnel or a smart chute takes at once: `FilteringBehaviour.count` and `upTo`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Amount {
    /// `FilteringBehaviour.count`, default 64.
    pub count: i32,
    /// `FilteringBehaviour.upTo`, default true: take up to `count`, not exactly `count`.
    pub up_to: bool,
}

impl Default for Amount {
    fn default() -> Self {
        Amount { count: 64, up_to: true }
    }
}

/// The amounts a click with the filter item itself steps through, standing in for Create's
/// scroll value box (`ValueSettingsBoard`: row 0 "up to", row 1 "exactly").
pub const AMOUNT_STEPS: [Amount; 8] = [
    Amount { count: 64, up_to: true },
    Amount { count: 1, up_to: false },
    Amount { count: 2, up_to: false },
    Amount { count: 4, up_to: false },
    Amount { count: 8, up_to: false },
    Amount { count: 16, up_to: false },
    Amount { count: 32, up_to: false },
    Amount { count: 64, up_to: false },
];

/// The next amount after `now`, one that fits a stack of `max_stack` (`FilteringBehaviour`
/// clamps an "exactly" amount to the filter item's stack size).
pub fn next_amount(now: Amount, max_stack: i32) -> Amount {
    let at = AMOUNT_STEPS.iter().position(|a| *a == now).unwrap_or(AMOUNT_STEPS.len() - 1);
    for step in 1..=AMOUNT_STEPS.len() {
        let candidate = AMOUNT_STEPS[(at + step) % AMOUNT_STEPS.len()];
        if candidate.up_to || candidate.count <= max_stack {
            return candidate;
        }
    }
    Amount::default()
}

/// `FilteringBehaviour.test`: an empty filter passes anything, otherwise the same item
/// (`ItemHelper.sameItem` compares the item, not its components).
pub fn filter_passes(filter: Option<&str>, item: &str) -> bool {
    filter.is_none_or(|f| f == item)
}

/// `ExtractionCountMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Exactly,
    UpTo,
}

/// `FilteringBehaviour.getAmount` and `upTo` as an extraction (`InvManipulationBehaviour
/// .getAmountFromFilter` / `getModeFromFilter`): the count and the mode.
pub fn extraction_of(amount: Amount) -> (u32, Mode) {
    (
        amount.count.max(1) as u32,
        if amount.up_to { Mode::UpTo } else { Mode::Exactly },
    )
}

/// `SmartChuteBlockEntity.getExtractionAmount` and `getExtractionMode`: the filter's amount only
/// where the count is shown (an inventory on the extracting side and an item that stacks).
pub fn smart_extraction(amount: Amount, count_visible: bool) -> (u32, Mode) {
    if count_visible && amount.count != 0 {
        extraction_of(amount)
    } else {
        (64, Mode::UpTo)
    }
}

// --- `ItemHelper.extract` ----------------------------------------------------------------------

/// One slot of an inventory, as far as `ItemHelper.extract` needs it. `key` stands for the item
/// and its components, whatever the count.
#[derive(Clone, Debug, PartialEq)]
pub struct Slot<K> {
    pub slot: usize,
    pub key: K,
    pub count: u32,
    /// `ItemStack.getMaxStackSize`.
    pub max: u32,
}

/// What `ItemHelper.extract` takes: the item and how much, from which slots.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan<K> {
    pub key: K,
    pub count: u32,
    pub takes: Vec<(usize, u32)>,
}

/// `ItemHelper.extract(inv, test, mode, amount, simulate = false)` as a plan: the slots to take
/// from, in order, and how much from each. `test` is asked of an item and the count that would
/// be taken from one slot (Create asks the simulated `extractItem`'s stack). `None` where nothing
/// is taken: no slot passes, or `Exactly` can't reach `amount`.
pub fn extract_plan<K: Clone + PartialEq>(
    slots: &[Slot<K>],
    test: &mut dyn FnMut(&K, u32) -> bool,
    mode: Mode,
    amount: u32,
) -> Option<Plan<K>> {
    let amount_required = mode == Mode::Exactly;
    let mut check_has_enough = amount_required;
    let mut has_enough = !check_has_enough;
    let mut potential_other_match = false;
    let mut blacklisted: Vec<K> = Vec::new();
    let max_extraction = amount;
    let mut guard = 0;
    let (mut key, mut total, mut takes): (Option<K>, u32, Vec<(usize, u32)>) =
        (None, 0, Vec::new());
    loop {
        guard += 1;
        if guard > 8 {
            // Create's loop ends because each restart either flips a flag or blacklists an item
            // (of which there are only so many); this keeps a mistake from spinning.
            break;
        }
        key = None;
        total = 0;
        takes.clear();
        let mut restart = false;
        for slot in slots {
            if slot.count == 0 {
                continue;
            }
            let from_slot = max_extraction.saturating_sub(total).min(slot.max);
            let stack_count = slot.count.min(from_slot);
            if stack_count == 0 {
                continue;
            }
            if blacklisted.contains(&slot.key) || !test(&slot.key, stack_count) {
                continue;
            }
            // `canItemStackAmountsStack(stack, extracting)`.
            if let Some(have) = &key
                && !(*have == slot.key && stack_count + total <= slot.max)
            {
                potential_other_match = true;
                continue;
            }
            if key.is_none() {
                key = Some(slot.key.clone());
            }
            total += stack_count;
            if has_enough {
                takes.push((slot.slot, stack_count));
            }
            if total >= max_extraction {
                if check_has_enough {
                    has_enough = true;
                    check_has_enough = false;
                    restart = true;
                    break;
                }
                break;
            }
        }
        if restart {
            continue;
        }
        if key.is_some() && !has_enough && potential_other_match {
            blacklisted.push(key.clone().expect("checked"));
            continue;
        }
        if check_has_enough {
            check_has_enough = false;
        } else {
            break;
        }
    }
    let key = key?;
    if total == 0 || (amount_required && total < amount) {
        return None;
    }
    Some(Plan { key, count: total, takes })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(slot: usize, key: &'static str, count: u32) -> Slot<&'static str> {
        Slot { slot, key, count, max: 64 }
    }

    #[test]
    fn items_fall_four_chutes_a_second_and_a_fan_changes_that() {
        assert_eq!(item_motion(0.0, 0.0), -4.0 / 20.0);
        // 256 RPM is 4 chutes a second up, and gravity no longer adds to it.
        assert_eq!(item_motion(256.0, 0.0), 4.0 / 20.0);
        assert!(item_motion(0.0, 512.0) > 0.0);
        // Capped at 20 chutes a second.
        assert_eq!(item_motion(100_000.0, 0.0), 1.0);
        // A fan pushing down: below -4 the item falls faster, gravity included.
        assert_eq!(item_motion(-256.0, 0.0), (-4.0 - 4.0) / 20.0);
    }

    #[test]
    fn a_fans_reach_below_a_chute_follows_its_speed() {
        assert_eq!(max_pull_distance(256.0), 3.0);
        assert_eq!(max_pull_distance(128.0), 3.0);
        assert_eq!(max_pull_distance(64.0), 2.0);
        assert_eq!(max_pull_distance(32.0), 1.0);
        assert_eq!(max_pull_distance(16.0), 0.5);
        assert!(max_pull_distance(-16.0) < 0.0);
    }

    #[test]
    fn pull_and_push_add_up_along_a_chain() {
        assert_eq!(total_pull(Some(64.0), [10.0]), 64.0);
        assert_eq!(total_pull(None, [10.0, 20.0]), 30.0);
        assert_eq!(total_pull(None, []), 0.0);
        assert_eq!(total_push(Some(32.0), Some(100.0), 2), 32.0);
        assert_eq!(total_push(None, Some(100.0), 2), 50.0);
        assert_eq!(total_push(None, None, 1), 0.0);
    }

    fn around(feeding: [bool; 4]) -> Around {
        Around {
            above_chute_facing: None,
            above_funnel_down: false,
            feeding,
            target_is_chute: true,
        }
    }

    #[test]
    fn a_vertical_chute_becomes_an_intersection_when_a_diagonal_one_feeds_it() {
        // `HORIZONTALS`: south, west, north, east.
        let feeding = [false, false, true, false];
        assert_eq!(
            update_chute_state(Dir::Down, Shape::Normal, &around(feeding)),
            (Dir::Down, Shape::Intersection)
        );
        assert_eq!(
            update_chute_state(Dir::Down, Shape::Intersection, &around([false; 4])),
            (Dir::Down, Shape::Normal)
        );
        // A window or an encased chute keeps its shape while nothing feeds it.
        assert_eq!(
            update_chute_state(Dir::Down, Shape::Window, &around([false; 4])),
            (Dir::Down, Shape::Window)
        );
    }

    #[test]
    fn a_diagonal_chute_without_a_target_goes_back_to_vertical() {
        let mut a = around([false; 4]);
        a.target_is_chute = false;
        assert_eq!(
            update_chute_state(Dir::East, Shape::Intersection, &a),
            (Dir::Down, Shape::Normal)
        );
    }

    #[test]
    fn a_diagonal_chute_is_plain_only_with_one_chute_feeding_it_from_behind() {
        // Facing east, fed from the east side (the one it faces).
        let east = [false, false, false, true];
        assert_eq!(
            update_chute_state(Dir::East, Shape::Intersection, &around(east)),
            (Dir::East, Shape::Normal)
        );
        // An encased one stays encased.
        assert_eq!(
            update_chute_state(Dir::East, Shape::Encased, &around(east)),
            (Dir::East, Shape::Encased)
        );
        // Nothing feeding it, or two opposite chutes, make an intersection.
        assert_eq!(
            update_chute_state(Dir::East, Shape::Normal, &around([false; 4])).1,
            Shape::Intersection
        );
        assert_eq!(
            update_chute_state(Dir::East, Shape::Normal, &around([false, true, false, true])).1,
            Shape::Intersection
        );
        // A chute pointing down above it, or a funnel pointing down, does too.
        let mut a = around(east);
        a.above_chute_facing = Some(Dir::Down);
        assert_eq!(update_chute_state(Dir::East, Shape::Normal, &a).1, Shape::Intersection);
        let mut a = around(east);
        a.above_funnel_down = true;
        assert_eq!(update_chute_state(Dir::East, Shape::Normal, &a).1, Shape::Intersection);
    }

    #[test]
    fn a_chute_doesnt_hang_from_a_diagonal_one() {
        assert!(can_survive(None));
        assert!(can_survive(Some(Dir::Down)));
        assert!(!can_survive(Some(Dir::East)));
    }

    #[test]
    fn the_wrench_cycles_a_vertical_chutes_shape() {
        assert_eq!(wrenched(Shape::Normal, true), Some(Shape::Window));
        assert_eq!(wrenched(Shape::Window, true), Some(Shape::Normal));
        assert_eq!(wrenched(Shape::Encased, true), Some(Shape::Normal));
        assert_eq!(wrenched(Shape::Encased, false), Some(Shape::Normal));
        assert_eq!(wrenched(Shape::Intersection, true), None);
        assert_eq!(wrenched(Shape::Normal, false), None);
    }

    #[test]
    fn an_empty_filter_passes_everything() {
        assert!(filter_passes(None, "minecraft:stone"));
        assert!(filter_passes(Some("minecraft:stone"), "minecraft:stone"));
        assert!(!filter_passes(Some("minecraft:stone"), "minecraft:dirt"));
    }

    #[test]
    fn the_amount_steps_skip_what_the_stack_cant_hold() {
        let any = Amount::default();
        assert_eq!(next_amount(any, 64), Amount { count: 1, up_to: false });
        assert_eq!(next_amount(Amount { count: 32, up_to: false }, 64), Amount { count: 64, up_to: false });
        assert_eq!(next_amount(Amount { count: 64, up_to: false }, 64), any);
        // Ender pearls stack to 16: 32 and 64 exactly are skipped.
        assert_eq!(next_amount(Amount { count: 16, up_to: false }, 16), any);
        assert_eq!(extraction_of(any), (64, Mode::UpTo));
        assert_eq!(extraction_of(Amount { count: 8, up_to: false }), (8, Mode::Exactly));
        assert_eq!(smart_extraction(Amount { count: 8, up_to: false }, false), (64, Mode::UpTo));
        assert_eq!(smart_extraction(Amount { count: 8, up_to: false }, true), (8, Mode::Exactly));
    }

    #[test]
    fn up_to_takes_the_first_matching_stack_and_tops_up_from_the_next() {
        let slots = [slot(0, "dirt", 5), slot(1, "stone", 30), slot(2, "stone", 40)];
        let mut any = |_: &&str, _: u32| true;
        // The first stack is dirt; the stone doesn't stack with it, so only dirt is taken.
        let plan = extract_plan(&slots, &mut any, Mode::UpTo, 64).unwrap();
        assert_eq!(plan, Plan { key: "dirt", count: 5, takes: vec![(0, 5)] });
        let mut stone = |k: &&str, _: u32| *k == "stone";
        let plan = extract_plan(&slots, &mut stone, Mode::UpTo, 64).unwrap();
        // 30 from the first stone slot, then 34 of the second (the 64 stack limit).
        assert_eq!(plan.count, 64);
        assert_eq!(plan.takes, vec![(1, 30), (2, 34)]);
    }

    #[test]
    fn exactly_takes_nothing_when_it_cannot_reach_the_amount() {
        let slots = [slot(0, "stone", 3), slot(1, "stone", 4)];
        let mut stone = |k: &&str, _: u32| *k == "stone";
        assert_eq!(extract_plan(&slots, &mut stone, Mode::Exactly, 8), None);
        let plan = extract_plan(&slots, &mut stone, Mode::Exactly, 6).unwrap();
        assert_eq!(plan.count, 6);
        assert_eq!(plan.takes, vec![(0, 3), (1, 3)]);
    }

    #[test]
    fn exactly_retries_without_an_item_that_comes_up_short() {
        // The dirt (5) is first but can't make 8; the stone can.
        let slots = [slot(0, "dirt", 5), slot(1, "stone", 9)];
        let mut any = |_: &&str, _: u32| true;
        let plan = extract_plan(&slots, &mut any, Mode::Exactly, 8).unwrap();
        assert_eq!(plan, Plan { key: "stone", count: 8, takes: vec![(1, 8)] });
    }

    #[test]
    fn nothing_is_taken_when_the_test_refuses_everything() {
        let slots = [slot(0, "dirt", 5)];
        let mut none = |_: &&str, _: u32| false;
        assert_eq!(extract_plan(&slots, &mut none, Mode::UpTo, 64), None);
        let mut any = |_: &&str, _: u32| true;
        assert_eq!(extract_plan::<&str>(&[], &mut any, Mode::UpTo, 64), None);
    }

    #[test]
    fn a_stack_is_cut_to_the_amount_asked_for() {
        let slots = [slot(0, "stone", 64)];
        let mut any = |_: &&str, _: u32| true;
        let plan = extract_plan(&slots, &mut any, Mode::UpTo, 1).unwrap();
        assert_eq!(plan.takes, vec![(0, 1)]);
    }

    #[test]
    fn a_diagonal_chute_is_placed_from_a_side_unless_sneaking() {
        assert!(diagonal_placement(Dir::East, false));
        assert!(!diagonal_placement(Dir::East, true));
        assert!(!diagonal_placement(Dir::Up, false));
    }
}
