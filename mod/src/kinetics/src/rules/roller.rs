//! The Mechanical Roller on a moving structure: which blocks it breaks in front of it, which cells it
//! paves or fills for each of its three modes, in what order, and what the filter accepts
//! (`RollerMovementBehaviour`, `RollerBlockEntity.RollingMode`, `RollerBlockEntity.isValidMaterial`,
//! `CKinetics.rollerFillDepth`). Plain values only: the glue (`machines/roller_actor.rs`) reads the world,
//! asks these and applies the answer.
//!
//! The rest of a roller's behaviour is the block breaking every breaker shares (`actor.rs`: `Breaker`,
//! `breaking_speed`, `roller_breaking_speed`, `is_active`, `active_area_offset`).
//!
//! Known differences from Create, for the README:
//! - The Roller on a Train (`createHeightProfileForTracks`, `TrackPaverV2`, `RollerTravellingPoint`) isn't
//!   ported: there are no trains here, so the paving set is always the visited cell alone.

use crate::Pos;

use super::roller_facts::PAVE_MATERIALS;

/// `CKinetics.rollerFillDepth`: "Maximum depth of blocks filled in using a Mechanical Roller", 12 by default.
pub const FILL_DEPTH: i32 = 12;

/// `RollerBlockEntity.RollingMode`, in ordinal order: what `ScrollValue` of the block entity counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Clears the blocks above the layer it paves, and paves it.
    TunnelPave,
    /// Fills a column straight down to the surface; breaks nothing.
    StraightFill,
    /// Fills in layers that grow with the depth; breaks nothing.
    WideFill,
}

/// `RollingMode.getTranslationKey` (`create.contraptions.roller_mode.<name>`), in ordinal order.
pub const MODE_KEYS: [&str; 3] = [
    "create.contraptions.roller_mode.tunnel_pave",
    "create.contraptions.roller_mode.straight_fill",
    "create.contraptions.roller_mode.wide_fill",
];

/// The language key of the mode box's label (`CreateLang.translateDirect("contraptions.roller_mode")`).
pub const MODE_LABEL: &str = "create.contraptions.roller_mode";

/// The filter box's label (`contraptions.mechanical_roller.pave_material`).
pub const FILTER_LABEL: &str = "create.contraptions.mechanical_roller.pave_material";

impl Mode {
    /// `RollingMode.values()[ScrollValue]`. Create throws on a value out of range; here it is the first mode.
    pub fn from_ordinal(value: i32) -> Mode {
        match value {
            1 => Mode::StraightFill,
            2 => Mode::WideFill,
            _ => Mode::TunnelPave,
        }
    }

    pub fn ordinal(self) -> i32 {
        match self {
            Mode::TunnelPave => 0,
            Mode::StraightFill => 1,
            Mode::WideFill => 2,
        }
    }
}

// --- the filter ----------------------------------------------------------------------------------------

/// `RollerBlockEntity.isValidMaterial` for a non-empty filter: a block item whose paving state is a full
/// cube with a collision shape and neither an `EntityBlock` nor stairs. Vanilla's answer is baked
/// (`roller_facts.rs`); another mod's block is taken as valid, as Horizon can't ask it before it is placed.
pub fn is_valid_material(item: &str, is_block_item: bool) -> bool {
    if !is_block_item {
        return false;
    }
    if item.starts_with("minecraft:") {
        return PAVE_MATERIALS.binary_search(&item).is_ok();
    }
    true
}

/// `defaultBlockState.hasProperty(SlabBlock.TYPE)`: a slab's default `type` is `bottom` (a chest's is
/// `single`, a piston head's `normal`), so a block whose default state has that value is one.
pub fn is_slab_default(default_type: Option<&str>) -> bool {
    default_type == Some("bottom")
}

/// `getStateToPaveWithAsSlab` for a block that isn't a slab: the block names (in its namespace) tried in
/// order, the first that exists being the slab to put in half-height spots. `name` is the block's path.
pub fn slab_candidates(name: &str) -> Vec<String> {
    let length = name.len();
    let mut candidates = vec![format!("{name}_slab")];
    if name.ends_with('s') && length > 1 {
        candidates.push(format!("{}_slab", &name[..length - 1]));
    }
    // `blockName.substring(0, nameLength - 7)`: "oak_planks" becomes "oak".
    if name.ends_with("planks") && length > 7 {
        candidates.push(format!("{}_slab", &name[..length - 7]));
    }
    candidates
}

// --- rollers side by side --------------------------------------------------------------------------------

/// `Iterate.positiveAndNegative`: the two ways along a line of rollers.
pub const SIDES: [i32; 2] = [1, -1];

/// How far `shareValuesToAdjacent` follows a line of rollers (`for (int i = 1; i < 100; i++)`).
pub const SHARE_REACH: i32 = 100;

/// `worldPosition.relative(facing.getClockWise(), side * i)`: the cell `i` blocks from `pos` along a line of
/// rollers, `across` being the normal of the facing turned clockwise.
pub fn line_cell(pos: Pos, across: Pos, side: i32, i: i32) -> Pos {
    [pos[0] + across[0] * side * i, pos[1] + across[1] * side * i, pos[2] + across[2] * side * i]
}

// --- breaking ------------------------------------------------------------------------------------------

/// `getPositionsToBreak`'s `startingY`: 1, or 0 when there is something to pave with (the filter's block is not
/// air) and the structure's storage holds an item that passes the filter, so the layer itself is cleared as well.
pub fn starting_y(paves_with_a_block: bool, storage_has_item: bool) -> i32 {
    if paves_with_a_block && storage_has_item { 0 } else { 1 }
}

/// `testBreakerTarget`'s two early outs: the block already in the paved layer (`columnY` 0) or the slab in the
/// layer above it (`columnY` 1) is left alone.
pub fn leaves_alone(column_y: i32, is_pave_block: bool, has_slab: bool, is_slab_block: bool) -> bool {
    (column_y == 0 && is_pave_block) || (has_slab && column_y == 1 && is_slab_block)
}

/// `getPositionsToBreak` away from tracks: only the `TUNNEL_PAVE` mode breaks, the cells `starting_y` to 2 above
/// the visited one (`columnY`), those `test` says to break (`testBreakerTarget`).
pub fn positions_to_break(
    mode: Mode,
    start_y: i32,
    visited: Pos,
    test: &mut dyn FnMut(Pos, i32) -> bool,
) -> Vec<Pos> {
    let mut positions = Vec::new();
    if mode != Mode::TunnelPave {
        return positions;
    }
    for i in start_y..=2 {
        let target = [visited[0], visited[1] + i, visited[2]];
        if test(target, i) {
            positions.push(target);
        }
    }
    positions
}

/// `visitNewPosition`'s choice of the block to crack: the one with the highest destroy speed, the later of
/// two equal ones (`if (hardness < max) continue`, `max` starting at -1).
pub fn hardest(positions: &[(Pos, f32)]) -> Option<Pos> {
    let mut max = -1.0_f64;
    let mut at = None;
    for (pos, hardness) in positions {
        if f64::from(*hardness) < max {
            continue;
        }
        max = f64::from(*hardness);
        at = Some(*pos);
    }
    at
}

/// `destroyBlock`'s consumer: a drop is kept (into the structure, or thrown) when the block doesn't need an
/// iron, stone or diamond tool and the coin (`random.nextBoolean()`) came up false. Half of the rest is lost.
pub fn keeps_drop(needs_tool: bool, coin: bool) -> bool {
    !(needs_tool || coin)
}

// --- paving --------------------------------------------------------------------------------------------

/// `PaveResult`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fill {
    Fail,
    Pass,
    Success,
}

/// What `tryFill` sees at the cell it is to fill, before it asks the storage for the item.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Existing {
    /// `existing.is(toPlace.getBlock())`.
    pub same_block: bool,
    /// `BlockTags.LEAVES`.
    pub leaves: bool,
    /// `canBeReplaced()`.
    pub replaceable: bool,
    /// `getCollisionShape().isEmpty()`.
    pub collision_empty: bool,
    /// `BlockTags.PORTALS`.
    pub portal: bool,
}

/// `tryFill` up to the storage: already the block is a pass, a cell that can't be built into a fail, and
/// `None` where an item has to be taken (a fail if there is none, a success when the block is placed).
pub fn judge(existing: &Existing) -> Option<Fill> {
    if existing.same_block {
        return Some(Fill::Pass);
    }
    if !existing.leaves && !existing.replaceable && (!existing.collision_empty || existing.portal) {
        return Some(Fill::Fail);
    }
    None
}

/// `triggerPaver`'s first check: a fill mode with nothing to fill with does nothing.
pub fn should_pave(mode: Mode, paves_with_air: bool) -> bool {
    !(mode != Mode::TunnelPave && paves_with_air)
}

/// One layer of the cells `triggerPaver` fills at depth `y_offset` under the `anchors` (each with whether a slab
/// goes on top of it): the cell straight down, or in `WIDE_FILL` every cell within `(y_offset + 1) / 2`
/// blocks of it by Manhattan distance, `y_offset` lower. A cell of two anchors is there once.
pub fn layer(mode: Mode, anchors: &[(Pos, bool)], y_offset: i32) -> Vec<(Pos, bool)> {
    let mut cells: Vec<(Pos, bool)> = Vec::new();
    let mut add = |cell: (Pos, bool)| {
        if !cells.contains(&cell) {
            cells.push(cell);
        }
    };
    for (anchor, slab) in anchors {
        if mode == Mode::WideFill {
            let radius = (y_offset + 1) / 2;
            for i in -radius..=radius {
                for j in -radius..=radius {
                    if i.abs() + j.abs() <= radius {
                        add(([anchor[0] + i, anchor[1] - y_offset, anchor[2] + j], *slab));
                    }
                }
            }
        } else {
            add(([anchor[0], anchor[1] - y_offset, anchor[2]], *slab));
        }
    }
    cells
}

/// What `triggerPaver` needs to know of the block it paves with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Paving {
    pub mode: Mode,
    /// `CKinetics.rollerFillDepth`.
    pub depth: i32,
    /// The block is a `FallingBlock`: placed at the top, it would drop, so a column goes on.
    pub falling: bool,
    /// There is a slab to put in the half-height spots (`stateToPaveWithAsSlab != null`).
    pub has_slab: bool,
}

/// `triggerPaver`'s loop over the layers: `fill(cell, as_slab)` is `tryFill` with the paving block (or its slab).
/// Returns whether a block was placed this trigger (`PaveResult.SUCCESS`), the structure then waiting two ticks.
pub fn pave(paving: &Paving, anchors: &[(Pos, bool)], fill: &mut dyn FnMut(Pos, bool) -> Fill) -> bool {
    if anchors.is_empty() {
        return false;
    }
    let mut result = Fill::Pass;
    let mut y_offset = 0;
    while result == Fill::Pass {
        if y_offset > paving.depth {
            result = Fill::Fail;
            break;
        }
        let mut completely_blocked = true;
        let mut any_placed = false;
        for (cell, slab_on_top) in layer(paving.mode, anchors, y_offset) {
            if paving.has_slab && y_offset == 0 && slab_on_top {
                fill([cell[0], cell[1] + 1, cell[2]], true);
            }
            result = fill(cell, false);
            if result != Fill::Fail {
                completely_blocked = false;
            }
            if result == Fill::Success {
                any_placed = true;
            }
        }
        if any_placed {
            result = Fill::Success;
        } else if !completely_blocked || y_offset == 0 {
            result = Fill::Pass;
        }
        if result == Fill::Success && paving.falling {
            result = Fill::Pass;
        }
        if result != Fill::Pass {
            break;
        }
        if paving.mode == Mode::TunnelPave {
            break;
        }
        y_offset += 1;
    }
    result == Fill::Success
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const AT: Pos = [10, 64, 10];

    fn paving(mode: Mode) -> Paving {
        Paving { mode, depth: FILL_DEPTH, falling: false, has_slab: false }
    }

    #[test]
    fn the_modes_are_in_createss_order() {
        assert_eq!(Mode::from_ordinal(0), Mode::TunnelPave);
        assert_eq!(Mode::from_ordinal(1), Mode::StraightFill);
        assert_eq!(Mode::from_ordinal(2), Mode::WideFill);
        assert_eq!(Mode::from_ordinal(7), Mode::TunnelPave);
        assert_eq!(Mode::WideFill.ordinal(), 2);
        assert_eq!(MODE_KEYS[1], "create.contraptions.roller_mode.straight_fill");
    }

    #[test]
    fn only_a_full_block_item_is_a_material() {
        assert!(is_valid_material("minecraft:stone", true));
        assert!(is_valid_material("minecraft:oak_slab", true));
        assert!(is_valid_material("minecraft:soul_sand", true));
        assert!(is_valid_material("create:cut_andesite", true));
        // Not a block item, a block entity, stairs, not a full cube, no collision.
        assert!(!is_valid_material("minecraft:stick", false));
        assert!(!is_valid_material("minecraft:chest", true));
        assert!(!is_valid_material("minecraft:oak_stairs", true));
        assert!(!is_valid_material("minecraft:oak_fence", true));
        assert!(!is_valid_material("minecraft:farmland", true));
        assert!(!is_valid_material("minecraft:powder_snow", true));
        assert!(!is_valid_material("minecraft:air", true));
    }

    #[test]
    fn the_baked_materials_are_sorted_for_the_search() {
        assert!(PAVE_MATERIALS.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn a_slab_is_told_by_its_default_type() {
        assert!(is_slab_default(Some("bottom")));
        assert!(!is_slab_default(Some("single")));
        assert!(!is_slab_default(None));
    }

    #[test]
    fn a_slab_is_looked_for_under_the_names_create_tries() {
        assert_eq!(slab_candidates("stone"), ["stone_slab"]);
        assert_eq!(slab_candidates("oak_planks"), ["oak_planks_slab", "oak_plank_slab", "oak_slab"]);
        assert_eq!(slab_candidates("stone_bricks"), ["stone_bricks_slab", "stone_brick_slab"]);
        // A name of one letter, or of "planks" alone, has no shortened guess.
        assert_eq!(slab_candidates("s"), ["s_slab"]);
        assert_eq!(slab_candidates("planks"), ["planks_slab", "plank_slab"]);
    }

    #[test]
    fn rollers_share_along_the_line_across_their_facing() {
        // Facing north, clockwise is east: +x.
        assert_eq!(line_cell(AT, [1, 0, 0], 1, 1), [11, 64, 10]);
        assert_eq!(line_cell(AT, [1, 0, 0], -1, 3), [7, 64, 10]);
        assert_eq!(line_cell(AT, [0, 0, 1], -1, 2), [10, 64, 8]);
        assert_eq!(SIDES, [1, -1]);
        assert_eq!(SHARE_REACH, 100);
    }

    #[test]
    fn the_layer_itself_is_cleared_only_with_something_to_pave_with() {
        assert_eq!(starting_y(true, true), 0);
        assert_eq!(starting_y(true, false), 1);
        assert_eq!(starting_y(false, true), 1);
        assert_eq!(starting_y(false, false), 1);
    }

    #[test]
    fn the_column_above_is_what_tunnel_pave_breaks() {
        let mut asked = Vec::new();
        let mut test = |pos: Pos, i: i32| {
            asked.push((pos, i));
            true
        };
        assert_eq!(positions_to_break(Mode::TunnelPave, 1, AT, &mut test), vec![[10, 65, 10], [10, 66, 10]]);
        assert_eq!(asked, vec![([10, 65, 10], 1), ([10, 66, 10], 2)]);
        let mut test = |_: Pos, _: i32| true;
        assert_eq!(
            positions_to_break(Mode::TunnelPave, 0, AT, &mut test),
            vec![[10, 64, 10], [10, 65, 10], [10, 66, 10]]
        );
        // Only the cells the test lets through; the fill modes break nothing.
        let mut only_top = |_: Pos, i: i32| i == 2;
        assert_eq!(positions_to_break(Mode::TunnelPave, 1, AT, &mut only_top), vec![[10, 66, 10]]);
        let mut all = |_: Pos, _: i32| true;
        assert!(positions_to_break(Mode::StraightFill, 1, AT, &mut all).is_empty());
        assert!(positions_to_break(Mode::WideFill, 0, AT, &mut all).is_empty());
    }

    #[test]
    fn the_paved_block_and_its_slab_are_left_standing() {
        assert!(leaves_alone(0, true, false, false));
        assert!(!leaves_alone(1, true, false, false));
        assert!(leaves_alone(1, false, true, true));
        assert!(!leaves_alone(1, false, false, true));
        assert!(!leaves_alone(2, true, true, true));
        assert!(!leaves_alone(0, false, true, true));
    }

    #[test]
    fn the_hardest_block_is_cracked_and_the_later_of_equals() {
        let a: Pos = [0, 1, 0];
        let b: Pos = [0, 2, 0];
        assert_eq!(hardest(&[(a, 1.5), (b, 0.5)]), Some(a));
        assert_eq!(hardest(&[(a, 0.5), (b, 1.5)]), Some(b));
        assert_eq!(hardest(&[(a, 1.0), (b, 1.0)]), Some(b));
        assert_eq!(hardest(&[]), None);
        // Even an unbreakable -1 is taken from the start (`max` is -1, not above it).
        assert_eq!(hardest(&[(a, -1.0)]), Some(a));
    }

    #[test]
    fn half_the_drops_are_lost_and_all_of_a_hard_block() {
        assert!(keeps_drop(false, false));
        assert!(!keeps_drop(false, true));
        assert!(!keeps_drop(true, false));
        assert!(!keeps_drop(true, true));
    }

    #[test]
    fn what_stands_in_the_way_of_a_fill() {
        let air = Existing { replaceable: true, collision_empty: true, ..Existing::default() };
        let stone = Existing::default();
        let leaves = Existing { leaves: true, ..Existing::default() };
        let portal = Existing { collision_empty: true, portal: true, ..Existing::default() };
        let flower = Existing { collision_empty: true, ..Existing::default() };
        let same = Existing { same_block: true, ..Existing::default() };
        assert_eq!(judge(&same), Some(Fill::Pass));
        assert_eq!(judge(&stone), Some(Fill::Fail));
        assert_eq!(judge(&portal), Some(Fill::Fail));
        // Air, replaceable things, leaves and things with no collision (a flower) are built over.
        assert_eq!(judge(&air), None);
        assert_eq!(judge(&leaves), None);
        assert_eq!(judge(&flower), None);
    }

    #[test]
    fn a_fill_mode_without_a_block_does_nothing() {
        assert!(!should_pave(Mode::StraightFill, true));
        assert!(!should_pave(Mode::WideFill, true));
        assert!(should_pave(Mode::TunnelPave, true));
        assert!(should_pave(Mode::StraightFill, false));
    }

    #[test]
    fn a_straight_fill_goes_down_the_column_and_a_pave_is_one_layer() {
        let cells = layer(Mode::StraightFill, &[(AT, false)], 3);
        assert_eq!(cells, vec![([10, 61, 10], false)]);
        assert_eq!(layer(Mode::TunnelPave, &[(AT, true)], 0), vec![(AT, true)]);
    }

    #[test]
    fn a_wide_fill_grows_by_half_a_block_a_layer() {
        // Radius (y + 1) / 2: 0, 1, 1, 2 blocks at depths 0 to 3, by Manhattan distance (a diamond).
        assert_eq!(layer(Mode::WideFill, &[(AT, false)], 0).len(), 1);
        assert_eq!(layer(Mode::WideFill, &[(AT, false)], 1).len(), 5);
        assert_eq!(layer(Mode::WideFill, &[(AT, false)], 2).len(), 5);
        assert_eq!(layer(Mode::WideFill, &[(AT, false)], 3).len(), 13);
        let two = layer(Mode::WideFill, &[(AT, false)], 1);
        assert!(two.contains(&([11, 63, 10], false)));
        assert!(two.contains(&([10, 63, 9], false)));
        assert!(!two.contains(&([11, 63, 11], false)));
        // Two anchors a block apart share cells.
        let overlapping = layer(Mode::WideFill, &[(AT, false), ([11, 64, 10], false)], 1);
        assert_eq!(overlapping.len(), 8);
    }

    /// A world of cells that say what `fill` answers there; the cells placed are recorded.
    struct World {
        answers: HashMap<Pos, Fill>,
        asked: Vec<(Pos, bool)>,
    }

    impl World {
        fn new(answers: &[(Pos, Fill)]) -> World {
            World { answers: answers.iter().copied().collect(), asked: Vec::new() }
        }

        fn fill(&mut self, pos: Pos, slab: bool) -> Fill {
            self.asked.push((pos, slab));
            self.answers.get(&pos).copied().unwrap_or(Fill::Fail)
        }
    }

    #[test]
    fn tunnel_pave_places_its_one_layer_and_waits() {
        let mut world = World::new(&[(AT, Fill::Success)]);
        let placed = pave(&paving(Mode::TunnelPave), &[(AT, false)], &mut |p, s| world.fill(p, s));
        assert!(placed);
        assert_eq!(world.asked, vec![(AT, false)]);
        // Over a block that is already there (a pass) it neither places nor goes further down.
        let mut world = World::new(&[(AT, Fill::Pass), ([10, 63, 10], Fill::Success)]);
        assert!(!pave(&paving(Mode::TunnelPave), &[(AT, false)], &mut |p, s| world.fill(p, s)));
        assert_eq!(world.asked, vec![(AT, false)]);
        // Over solid ground (a fail at depth 0) it is a pass: nothing placed.
        let mut world = World::new(&[]);
        assert!(!pave(&paving(Mode::TunnelPave), &[(AT, false)], &mut |p, s| world.fill(p, s)));
    }

    #[test]
    fn a_straight_fill_goes_down_to_the_first_block_it_can_place() {
        // Air at the visited cell and the two below, the ground under that: a pass on air-less cells
        // is a success only where something is placed.
        let mut world = World::new(&[
            (AT, Fill::Success),
            ([10, 63, 10], Fill::Success),
            ([10, 62, 10], Fill::Success),
        ]);
        // Every layer places a block, so each one succeeds and the first success stops the loop.
        assert!(pave(&paving(Mode::StraightFill), &[(AT, false)], &mut |p, s| world.fill(p, s)));
        assert_eq!(world.asked, vec![(AT, false)]);
        // The visited cell already has the block (pass), the next is air that gets filled.
        let mut world = World::new(&[(AT, Fill::Pass), ([10, 63, 10], Fill::Success)]);
        assert!(pave(&paving(Mode::StraightFill), &[(AT, false)], &mut |p, s| world.fill(p, s)));
        assert_eq!(world.asked, vec![(AT, false), ([10, 63, 10], false)]);
    }

    #[test]
    fn a_fill_stops_when_every_cell_of_a_layer_below_the_first_is_blocked() {
        // Depth 0 passes (it's there already), depth 1 is solid all over: the fill is over.
        let mut world = World::new(&[(AT, Fill::Pass)]);
        assert!(!pave(&paving(Mode::StraightFill), &[(AT, false)], &mut |p, s| world.fill(p, s)));
        assert_eq!(world.asked, vec![(AT, false), ([10, 63, 10], false)]);
        // Depth 0 failing (solid ground, as when paving a road) is a pass, so it goes on to depth 1.
        let mut world = World::new(&[([10, 63, 10], Fill::Success)]);
        assert!(pave(&paving(Mode::StraightFill), &[(AT, false)], &mut |p, s| world.fill(p, s)));
        assert_eq!(world.asked.len(), 2);
    }

    #[test]
    fn a_fill_gives_up_below_the_depth_limit() {
        // Every cell passes, forever: the loop ends at depth 13 with a fail.
        let mut asked = 0;
        let placed = pave(&paving(Mode::StraightFill), &[(AT, false)], &mut |_, _| {
            asked += 1;
            Fill::Pass
        });
        assert!(!placed);
        assert_eq!(asked, FILL_DEPTH + 1);
        let mut asked = 0;
        let shallow = Paving { depth: 1, ..paving(Mode::StraightFill) };
        pave(&shallow, &[(AT, false)], &mut |_, _| {
            asked += 1;
            Fill::Pass
        });
        assert_eq!(asked, 2);
    }

    #[test]
    fn a_wide_fill_fills_a_diamond_and_one_placed_block_is_enough() {
        // Depth 0 passes, depth 1 has five cells and places in two of them.
        let mut world = World::new(&[
            (AT, Fill::Pass),
            ([11, 63, 10], Fill::Success),
            ([9, 63, 10], Fill::Success),
        ]);
        assert!(pave(&paving(Mode::WideFill), &[(AT, false)], &mut |p, s| world.fill(p, s)));
        assert_eq!(world.asked.len(), 1 + 5);
    }

    #[test]
    fn a_falling_block_is_not_a_success_so_the_column_goes_on() {
        let falling = Paving { falling: true, ..paving(Mode::StraightFill) };
        // Sand placed at depth 0 is a pass in the end; the next layer then places too, and the column goes on
        // until a layer is blocked all over (depth 2 here).
        let mut world = World::new(&[(AT, Fill::Success), ([10, 63, 10], Fill::Success)]);
        assert!(!pave(&falling, &[(AT, false)], &mut |p, s| world.fill(p, s)));
        assert_eq!(world.asked.len(), 3);
        // In the paving mode (one layer) it ends there, with the block placed but no wait.
        let falling = Paving { falling: true, ..paving(Mode::TunnelPave) };
        let mut world = World::new(&[(AT, Fill::Success)]);
        assert!(!pave(&falling, &[(AT, false)], &mut |p, s| world.fill(p, s)));
        assert_eq!(world.asked.len(), 1);
    }

    #[test]
    fn a_slab_goes_on_the_first_layer_where_asked() {
        let slab = Paving { has_slab: true, ..paving(Mode::TunnelPave) };
        let mut world = World::new(&[(AT, Fill::Success), ([10, 65, 10], Fill::Success)]);
        assert!(pave(&slab, &[(AT, true)], &mut |p, s| world.fill(p, s)));
        // The slab one up first, then the block.
        assert_eq!(world.asked, vec![([10, 65, 10], true), (AT, false)]);
        // Not where the anchor asks for none, and not with no slab to place.
        let mut world = World::new(&[(AT, Fill::Success)]);
        pave(&slab, &[(AT, false)], &mut |p, s| world.fill(p, s));
        assert_eq!(world.asked, vec![(AT, false)]);
        let mut world = World::new(&[(AT, Fill::Success)]);
        pave(&paving(Mode::TunnelPave), &[(AT, true)], &mut |p, s| world.fill(p, s));
        assert_eq!(world.asked, vec![(AT, false)]);
    }

    #[test]
    fn nothing_is_paved_without_anchors() {
        assert!(!pave(&paving(Mode::TunnelPave), &[], &mut |_, _| Fill::Success));
    }
}
