//! The Mechanical Saw's rules: how it sits (`SawBlock`, `DirectionalAxisKineticBlock`), what it
//! saws (`SawBlockEntity.isSawable`) and the timing of cutting an item on it
//! (`SawBlockEntity.tick`, `start`, `ProcessingInventory`). Ported from Create (MIT, the Creators
//! of Create), `mc1.21.1/dev` at 4148fd8. The tree search is in [`super::saw_tree`], the breaking
//! timer in [`super::drill`].
//!
//! No game types: the glue (`native/api/src/machines/saw.rs`) owns the world and the items.

use crate::{Axis, Dir};

/// `SawBlockEntity` stores 32 slots; results beyond them are lost (`slot + 1 < getSlots()`).
pub const SLOTS: usize = 32;
/// How long an item with no cutting recipe stays on the blade (`start`: `remainingTime = 10`).
pub const NO_RECIPE_TIME: f32 = 10.0;
/// `start`'s default for a recipe that isn't a `CuttingRecipe` (a stonecutting one).
pub const DEFAULT_TIME: i32 = 50;
/// The time left when a cut is applied, and how long the result then stays on the blade
/// (`recipeDuration = 20; remainingTime = 20`).
pub const AFTER_CUT_TIME: f32 = 20.0;
/// `inventory.remainingTime < 5`: the cut happens when this little is left.
pub const CUT_AT: f32 = 5.0;
/// How many ticks apart the glue looks for items lying on the saw (Create reacts to an item
/// landing, `SawBlock.updateEntityAfterFallOn`; the `Host` has no such hook).
pub const TOP_SCAN: i64 = 2;

/// `SawBlock.isHorizontal`.
pub fn is_horizontal(facing: Dir) -> bool {
    facing.axis() != Axis::Y
}

/// `SawBlock.getRotationAxis` (`DirectionalAxisKineticBlock.getRotationAxis` when upright).
pub fn rotation_axis(facing: Dir, along_first: bool) -> Axis {
    if is_horizontal(facing) {
        return facing.axis();
    }
    // Facing along Y: along the first coordinate is X, else Z.
    if along_first { Axis::X } else { Axis::Z }
}

/// The faces with a shaft end (`SawBlock.hasShaftTowards`): out of the back sideways, both ends
/// of the rotation axis upright.
pub fn shaft_faces(facing: Dir, along_first: bool) -> Vec<Dir> {
    if is_horizontal(facing) {
        return vec![facing.opposite()];
    }
    match rotation_axis(facing, along_first) {
        Axis::X => vec![Dir::West, Dir::East],
        _ => vec![Dir::North, Dir::South],
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

/// The state a placed saw gets (`DirectionalAxisKineticBlock.getStateForPlacement` and
/// `SawBlock.getStateForPlacement`) without the shaft beside it it would prefer:
/// `(facing, axis_along_first, flipped)`. `look` is `getNearestLookingDirection`,
/// `horizontal` is `getHorizontalDirection`.
pub fn placement(look: Dir, horizontal: Dir, sneaking: bool) -> (Dir, bool, bool) {
    let mut facing = look.opposite();
    if sneaking {
        facing = facing.opposite();
    }
    let along_first = if is_horizontal(facing) {
        facing.axis() == Axis::Z
    } else {
        horizontal.axis() == Axis::X
    };
    let flipped = !is_horizontal(facing) && horizontal.sign() > 0;
    (facing, along_first, flipped)
}

/// The turn a Wrench click gives the saw (`IWrenchable.getRotatedBlockState` as
/// `SawBlock.getRotatedBlockState` finishes it): `(facing, axis_along_first, flipped)` after a
/// click on `clicked`.
pub fn wrench(facing: Dir, along_first: bool, flipped: bool, clicked: Dir) -> (Dir, bool, bool) {
    let (mut new_facing, mut new_along, mut new_flipped) = (facing, along_first, flipped);
    if facing.axis() == clicked.axis() {
        // `cycle(AXIS_ALONG_FIRST_COORDINATE)`.
        new_along = !new_along;
    } else {
        loop {
            new_facing = super::drill::clockwise(new_facing, clicked.axis());
            if clicked.axis() == Axis::Y {
                new_along = !new_along;
            }
            if new_facing.axis() != clicked.axis() {
                break;
            }
        }
    }
    // Upright, turned about Y, from the axis along the second coordinate: also flipped.
    if new_facing.axis() == Axis::Y && clicked.axis() == Axis::Y && !along_first {
        new_flipped = !new_flipped;
    }
    (new_facing, new_along, new_flipped)
}

/// `SawBlockEntity.getItemMovementVec`: the way an item crosses the blade, a unit step in x or z.
pub fn item_movement(along_first: bool, rpm: f32) -> [i32; 3] {
    let along_x = !along_first;
    let offset = if rpm < 0.0 { -1 } else { 1 };
    [
        offset * i32::from(along_x),
        0,
        offset * if along_x { 0 } else { -1 },
    ]
}

/// `Direction.getNearest` of an item movement: the face an item leaves by.
pub fn movement_face(movement: [i32; 3]) -> Dir {
    Dir::from_normal(movement).unwrap_or(Dir::East)
}

/// `SawBlockEntity.tick`: `processingSpeed = clamp(abs(speed) / 24, 1, 128)`.
pub fn processing_speed(rpm: f32) -> f32 {
    (rpm.abs() / 24.0).clamp(1.0, 128.0)
}

/// `SawRenderer.renderItems`: `clamp(abs(speed) / 32, 1, 128)`, what the drawn item moves by.
pub fn render_speed(rpm: f32) -> f32 {
    (rpm.abs() / 32.0).clamp(1.0, 128.0)
}

/// `start`'s timer for an input stack: `time * max(1, count / 5)`.
pub fn start_time(recipe_time: Option<i32>, count: u32) -> f32 {
    match recipe_time {
        Some(time) => (time as f32) * (count / 5).max(1) as f32,
        None => NO_RECIPE_TIME,
    }
}

/// The next recipe a stack with several is cut with: `start` moves on by one, wrapping
/// (`recipeIndex++; if (recipeIndex >= recipes.size()) recipeIndex = 0`). With one recipe, 0.
pub fn next_recipe(index: usize, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    let next = index + 1;
    if next >= count { 0 } else { next }
}

/// `applyRecipe`'s `if (recipeIndex >= recipes.size()) recipeIndex = 0`.
pub fn recipe_index(index: usize, count: usize) -> usize {
    if index >= count { 0 } else { index }
}

/// The cut in progress (`ProcessingInventory`): saved as `ProcessingTime`, `RecipeTime` and
/// `AppliedRecipe`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cut {
    /// `remainingTime`: -1 for nothing started.
    pub remaining: f32,
    /// `recipeDuration`.
    pub duration: f32,
    /// `appliedRecipe`.
    pub applied: bool,
}

impl Default for Cut {
    fn default() -> Self {
        Self::new()
    }
}

/// What a tick of the saw has to do next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Standing still, or nothing on the blade.
    Idle,
    /// Something lies on the blade with no cut under way: look up its recipe (`start`).
    Start,
    /// The cut goes on.
    Counting,
    /// The blade reaches the item: replace it with the recipe's results (`applyRecipe`).
    Apply,
    /// The results have crossed: hand them to the next belt or drop them (the end of `tick`).
    Leave,
}

impl Cut {
    pub const fn new() -> Self {
        Self {
            remaining: -1.0,
            duration: 0.0,
            applied: false,
        }
    }

    /// `ProcessingInventory.clear`: the timer back to 0.
    pub fn clear(&mut self) {
        self.remaining = 0.0;
        self.duration = 0.0;
        self.applied = false;
    }

    /// `start`: a cut begins taking `time` ticks (`remainingTime = recipeDuration = time`).
    pub fn begin(&mut self, time: f32) {
        self.remaining = time;
        self.duration = time;
        self.applied = false;
    }

    /// `SawBlockEntity.tick` after `canProcess` and the speed check: `occupied` says the
    /// inventory holds something.
    pub fn tick(&mut self, rpm: f32, occupied: bool) -> Step {
        if rpm == 0.0 {
            return Step::Idle;
        }
        if self.remaining == -1.0 {
            return if occupied && !self.applied {
                Step::Start
            } else {
                Step::Idle
            };
        }
        self.remaining -= processing_speed(rpm);
        if self.remaining < CUT_AT && !self.applied {
            self.applied = true;
            self.duration = AFTER_CUT_TIME;
            self.remaining = AFTER_CUT_TIME;
            return Step::Apply;
        }
        if self.remaining > 0.0 {
            return Step::Counting;
        }
        self.remaining = 0.0;
        Step::Leave
    }

    /// The inventory emptied after a cut left (`inventory.clear(); remainingTime = -1`).
    pub fn finish(&mut self) {
        self.clear();
        self.remaining = -1.0;
    }

    /// Where the item has got to for drawing, `SawRenderer.renderItems`: 0 to 1 across the
    /// block along the movement, before the speed's sign and the axis flip. `since` is the
    /// number of ticks (and the partial tick) that passed since `remaining` was valid.
    pub fn offset(&self, rpm: f32, since: f32) -> f32 {
        if rpm == 0.0 {
            return 0.5;
        }
        if self.duration == 0.0 {
            return 0.5;
        }
        let remaining = self.remaining - processing_speed(rpm) * since;
        let mut offset = remaining / self.duration;
        offset = (offset + (0.5 * render_speed(rpm)) / self.duration).clamp(0.125, 1.0);
        if !self.applied {
            offset += 1.0;
        }
        offset / 2.0
    }
}

/// `SawBlockEntity.isSawable` by block name and tag membership, for what no tag covers.
pub fn is_vertical_plant(block: &str) -> bool {
    matches!(
        block,
        "minecraft:bamboo"
            | "minecraft:cactus"
            | "minecraft:sugar_cane"
            | "minecraft:kelp_plant"
            | "minecraft:kelp"
    )
}

/// `isChorus`: `ChorusPlantBlock` or `ChorusFlowerBlock`.
pub fn is_chorus(block: &str) -> bool {
    matches!(block, "minecraft:chorus_plant" | "minecraft:chorus_flower")
}

/// `SawBlockEntity.isSawable`: what the sideways saw breaks. The tag answers are given.
pub fn is_sawable(
    block: &str,
    sapling: bool,
    log: bool,
    leaves: bool,
    root: bool,
    solid_crop: bool,
) -> bool {
    if sapling {
        return false;
    }
    if log || leaves || root {
        return true;
    }
    // `BambooStalkBlock`, `CactusBlock`, `SugarCaneBlock`, `KelpPlantBlock`, `KelpBlock`,
    // `ChorusPlantBlock` (not the flower) and `create:solid_crops`.
    is_vertical_plant(block) || block == "minecraft:chorus_plant" || solid_crop
}

/// `TreeCutter.nonDecayingLeafDistance`: `minecraft:red_mushroom_block` 2, brown 3, a wart block
/// or weeping vines 3, else -1.
pub fn non_decaying_distance(block: &str, wart_block: bool) -> i32 {
    match block {
        "minecraft:red_mushroom_block" => 2,
        "minecraft:brown_mushroom_block" => 3,
        "minecraft:weeping_vines" | "minecraft:weeping_vines_plant" => 3,
        _ if wart_block => 3,
        _ => -1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_axis_follows_the_facing_and_the_flag() {
        assert_eq!(rotation_axis(Dir::North, false), Axis::Z);
        assert_eq!(rotation_axis(Dir::East, true), Axis::X);
        assert_eq!(rotation_axis(Dir::Up, true), Axis::X);
        assert_eq!(rotation_axis(Dir::Up, false), Axis::Z);
        assert_eq!(rotation_axis(Dir::Down, true), Axis::X);
    }

    #[test]
    fn shafts_are_behind_a_sideways_saw_and_through_an_upright_one() {
        assert_eq!(shaft_faces(Dir::North, false), vec![Dir::South]);
        assert_eq!(shaft_faces(Dir::East, true), vec![Dir::West]);
        assert_eq!(shaft_faces(Dir::Up, true), vec![Dir::West, Dir::East]);
        assert_eq!(shaft_faces(Dir::Up, false), vec![Dir::North, Dir::South]);
    }

    #[test]
    fn players_face_where_their_yaw_says() {
        assert_eq!(horizontal_direction(0.0), Dir::South);
        assert_eq!(horizontal_direction(90.0), Dir::West);
        assert_eq!(horizontal_direction(180.0), Dir::North);
        assert_eq!(horizontal_direction(-90.0), Dir::East);
        assert_eq!(horizontal_direction(44.0), Dir::South);
        assert_eq!(horizontal_direction(46.0), Dir::West);
        assert_eq!(horizontal_direction(-170.0), Dir::North);
    }

    #[test]
    fn a_saw_placed_looking_down_stands_upright() {
        // Looking down at the floor, facing east: the saw faces up.
        let (facing, along_first, flipped) = placement(Dir::Down, Dir::East, false);
        assert_eq!(facing, Dir::Up);
        assert!(along_first);
        assert!(flipped);
        let (_, along_first, flipped) = placement(Dir::Down, Dir::North, false);
        assert!(!along_first);
        assert!(!flipped);
        let (_, _, flipped) = placement(Dir::Down, Dir::South, false);
        assert!(flipped);
    }

    #[test]
    fn a_saw_placed_looking_sideways_faces_the_player() {
        let (facing, along_first, flipped) = placement(Dir::North, Dir::North, false);
        assert_eq!(facing, Dir::South);
        // Facing along Z: along the first coordinate.
        assert!(along_first);
        assert!(!flipped);
        let (facing, along_first, _) = placement(Dir::East, Dir::East, true);
        assert_eq!(facing, Dir::East);
        assert!(!along_first);
    }

    #[test]
    fn a_wrench_click_along_the_facing_flips_the_axis() {
        assert_eq!(
            wrench(Dir::Up, true, false, Dir::Up),
            (Dir::Up, false, false)
        );
        // From the second coordinate the saw is also flipped.
        assert_eq!(
            wrench(Dir::Up, false, false, Dir::Up),
            (Dir::Up, true, true)
        );
    }

    #[test]
    fn a_wrench_click_turns_the_saw_round_the_face() {
        let (facing, ..) = wrench(Dir::North, false, false, Dir::Up);
        assert_eq!(facing, Dir::East);
        let (facing, ..) = wrench(Dir::Up, false, false, Dir::East);
        assert_eq!(facing, Dir::South);
    }

    #[test]
    fn items_cross_along_x_or_z_against_the_rotation() {
        assert_eq!(item_movement(false, 16.0), [1, 0, 0]);
        assert_eq!(item_movement(false, -16.0), [-1, 0, 0]);
        assert_eq!(item_movement(true, 16.0), [0, 0, -1]);
        assert_eq!(item_movement(true, -16.0), [0, 0, 1]);
        assert_eq!(movement_face([0, 0, -1]), Dir::North);
    }

    #[test]
    fn cutting_speed_is_an_rpm_over_24() {
        assert_eq!(processing_speed(8.0), 1.0);
        assert_eq!(processing_speed(48.0), 2.0);
        assert_eq!(processing_speed(-96.0), 4.0);
        assert_eq!(processing_speed(100_000.0), 128.0);
    }

    #[test]
    fn the_start_time_scales_with_the_stack() {
        assert_eq!(start_time(Some(50), 1), 50.0);
        assert_eq!(start_time(Some(50), 4), 50.0);
        assert_eq!(start_time(Some(50), 10), 100.0);
        assert_eq!(start_time(None, 10), 10.0);
    }

    #[test]
    fn several_recipes_take_turns() {
        assert_eq!(next_recipe(0, 3), 1);
        assert_eq!(next_recipe(1, 3), 2);
        assert_eq!(next_recipe(2, 3), 0);
        assert_eq!(next_recipe(0, 1), 0);
        assert_eq!(recipe_index(5, 2), 0);
        assert_eq!(recipe_index(1, 2), 1);
    }

    #[test]
    fn a_cut_runs_from_start_to_apply_to_leave() {
        let mut cut = Cut::new();
        assert_eq!(cut.tick(24.0, false), Step::Idle);
        assert_eq!(cut.tick(24.0, true), Step::Start);
        cut.begin(10.0);
        let mut applied_at = None;
        let mut left_at = None;
        for tick in 1..100 {
            match cut.tick(24.0, true) {
                Step::Apply if applied_at.is_none() => applied_at = Some(tick),
                Step::Leave => {
                    left_at = Some(tick);
                    break;
                }
                _ => {}
            }
        }
        // 10 - 1 a tick: below 5 after six ticks.
        assert_eq!(applied_at, Some(6));
        // Then 20 ticks more.
        assert_eq!(left_at, Some(26));
        cut.finish();
        assert_eq!(cut.remaining, -1.0);
        assert_eq!(cut.tick(24.0, false), Step::Idle);
    }

    #[test]
    fn a_stopped_saw_holds_its_cut() {
        let mut cut = Cut::new();
        cut.begin(50.0);
        assert_eq!(cut.tick(0.0, true), Step::Idle);
        assert_eq!(cut.remaining, 50.0);
    }

    #[test]
    fn the_drawn_item_crosses_the_blade() {
        let mut cut = Cut::new();
        cut.begin(50.0);
        let start = cut.offset(24.0, 0.0);
        // Not yet at the blade: past the half way mark (offset runs 1 down to 0.5).
        assert!(start > 0.5);
        cut.applied = true;
        cut.remaining = 10.0;
        cut.duration = 20.0;
        assert!(cut.offset(24.0, 0.0) < 0.5);
        assert_eq!(cut.offset(0.0, 0.0), 0.5);
    }

    #[test]
    fn what_a_sideways_saw_breaks() {
        assert!(is_sawable("minecraft:oak_log", false, true, false, false, false));
        assert!(is_sawable("minecraft:oak_leaves", false, false, true, false, false));
        assert!(!is_sawable("minecraft:oak_sapling", true, false, true, false, false));
        assert!(is_sawable("minecraft:bamboo", false, false, false, false, false));
        assert!(is_sawable("minecraft:cactus", false, false, false, false, false));
        assert!(is_sawable("minecraft:chorus_plant", false, false, false, false, false));
        assert!(!is_sawable("minecraft:chorus_flower", false, false, false, false, false));
        assert!(is_sawable("minecraft:pumpkin", false, false, false, false, true));
        assert!(!is_sawable("minecraft:stone", false, false, false, false, false));
        assert!(!is_sawable("minecraft:bamboo_sapling", false, false, false, false, false));
    }

    #[test]
    fn mushroom_reach() {
        assert_eq!(non_decaying_distance("minecraft:red_mushroom_block", false), 2);
        assert_eq!(non_decaying_distance("minecraft:brown_mushroom_block", false), 3);
        assert_eq!(non_decaying_distance("minecraft:nether_wart_block", true), 3);
        assert_eq!(non_decaying_distance("minecraft:weeping_vines", false), 3);
        assert_eq!(non_decaying_distance("minecraft:oak_leaves", false), -1);
    }
}
