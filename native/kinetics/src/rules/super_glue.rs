//! Super Glue's rules: the glued boxes, which blocks hold together through them, how two clicks
//! make one, what a structure takes along and how it comes back. Ported from Create (MIT, the
//! Creators of Create), `mc1.21.1/dev` at 4148fd8: `SuperGlueEntity` (`span`, `isGlued`,
//! `isValidFace`, `isSideSticky`, `contains`, `rotate`), `SuperGlueSelectionHelper`
//! (`searchGlueGroup`, `collectGlueFromInventory`), `SuperGlueSelectionHandler` (the click
//! sequence), `SuperGlueSelectionPacket.handle` (the server's checks), `SuperGlueRemovalPacket`
//! and `Contraption.removeBlocksFromWorld` / `addBlocksToWorld` (the glue a structure carries).
//!
//! No game types: the glue is `native/api/src/machines/super_glue.rs`, which keeps the boxes of a
//! level in a level attachment.
//!
//! Known differences from Create:
//! - A glue is a box of whole blocks (`min` and `max`, both inclusive) in integers; Create's is an
//!   entity with a `double` bounding box that always lies on the grid.
//! - The corners are picked on the server from the click, not from the client's ray, and the
//!   selection is shown with dust, not Create's outline (see the glue module).

use std::collections::{HashSet, VecDeque};

use crate::rules::chassis;
use crate::rules::contraption::{
    State, Transform, World, attached_towards, movement_necessary, not_supportive, relative,
};
use crate::{Dir, Pos};

/// `AllItems.SUPER_GLUE`: `durability(99)`.
pub const MAX_DAMAGE: u32 = 99;
/// `SuperGlueSelectionPacket`: the corners are `closerThan(from, 25)` (`BlockPos.closerThan`).
pub const MAX_DISTANCE: f64 = 25.0;
/// `Attributes.BLOCK_INTERACTION_RANGE + 1` (`SuperGlueSelectionHandler.tick`): how far the glue
/// in hand reaches for a glue to take off. Survival's reach is 4.5, creative's 5.
pub fn removal_range(creative: bool) -> f64 {
    if creative { 6.0 } else { 5.5 }
}

pub const ITEM: &str = "create:super_glue";

// --- the boxes ------------------------------------------------------------------------------------

/// A glue: the blocks from `min` to `max`, both corners included (`SuperGlueEntity`'s bounding
/// box, `span`, in whole blocks).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GlueBox {
    pub min: Pos,
    pub max: Pos,
}

impl GlueBox {
    /// `SuperGlueEntity.span`.
    pub fn span(a: Pos, b: Pos) -> GlueBox {
        GlueBox {
            min: [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])],
            max: [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])],
        }
    }

    /// `SuperGlueEntity.contains`: the block's centre is inside the box.
    pub fn contains(&self, pos: Pos) -> bool {
        (0..3).all(|i| self.min[i] <= pos[i] && pos[i] <= self.max[i])
    }

    /// Whether the box covers more than one block (`getSize() > 1.01` of
    /// `removeBlocksFromWorld`).
    pub fn is_more_than_a_block(&self) -> bool {
        self.min != self.max
    }

    /// The same box with its corners moved by `transform` (`Contraption.addBlocksToWorld`, and
    /// `SuperGlueEntity.rotate`).
    pub fn transformed(&self, transform: &Transform) -> GlueBox {
        GlueBox::span(transform.apply(self.min), transform.apply(self.max))
    }

    /// The box relative to `origin`.
    pub fn relative_to(&self, origin: Pos) -> GlueBox {
        GlueBox {
            min: [0, 1, 2].map(|i| self.min[i] - origin[i]),
            max: [0, 1, 2].map(|i| self.max[i] - origin[i]),
        }
    }

    /// The box moved by `origin`.
    pub fn moved(&self, origin: Pos) -> GlueBox {
        GlueBox {
            min: [0, 1, 2].map(|i| self.min[i] + origin[i]),
            max: [0, 1, 2].map(|i| self.max[i] + origin[i]),
        }
    }

    /// The distance along a ray from `eye` along `look` at which it enters the box's outline
    /// (the box is `min` to `max + 1` in space), within `range`: `AABB.clip`.
    pub fn clip(&self, eye: [f64; 3], look: [f64; 3], range: f64) -> Option<f64> {
        let (mut near, mut far) = (0.0_f64, range);
        for i in 0..3 {
            let (lo, hi) = (f64::from(self.min[i]), f64::from(self.max[i]) + 1.0);
            if look[i].abs() < 1e-12 {
                if eye[i] < lo || eye[i] > hi {
                    return None;
                }
                continue;
            }
            let (a, b) = ((lo - eye[i]) / look[i], (hi - eye[i]) / look[i]);
            let (t0, t1) = if a < b { (a, b) } else { (b, a) };
            near = near.max(t0);
            far = far.min(t1);
            if near > far {
                return None;
            }
        }
        Some(near)
    }
}

impl GlueBox {
    /// The points that outline the box (`min` to `max + 1` in space) along its twelve edges, a
    /// step apart and at each corner, at most 16 along an edge, without repeats.
    pub fn outline_points(&self, step: f64) -> Vec<[f64; 3]> {
        let lo = [0, 1, 2].map(|i| f64::from(self.min[i]));
        let hi = [0, 1, 2].map(|i| f64::from(self.max[i]) + 1.0);
        let mut points: Vec<[f64; 3]> = Vec::new();
        for axis in 0..3 {
            let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
            let length = hi[axis] - lo[axis];
            let count = ((length / step).ceil() as usize).clamp(1, 16);
            for i in 0..=count {
                let along = lo[axis] + length * i as f64 / count as f64;
                for (ca, cb) in [(lo[a], lo[b]), (lo[a], hi[b]), (hi[a], lo[b]), (hi[a], hi[b])] {
                    let mut p = [0.0; 3];
                    p[axis] = along;
                    p[a] = ca;
                    p[b] = cb;
                    if !points.contains(&p) {
                        points.push(p);
                    }
                }
            }
        }
        points
    }
}

/// All the glue of one level.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GlueSet {
    pub boxes: Vec<GlueBox>,
}

impl GlueSet {
    pub fn new(boxes: Vec<GlueBox>) -> Self {
        GlueSet { boxes }
    }

    /// `SuperGlueEntity.isGlued`: a glue holds the block at `pos` to its neighbour on `face`
    /// when its box contains both.
    pub fn glued(&self, pos: Pos, face: Dir) -> bool {
        let other = relative(pos, face, 1);
        self.boxes.iter().any(|b| b.contains(pos) && b.contains(other))
    }

    /// Adds a box; a box already there isn't added twice. True when it was new.
    pub fn add(&mut self, glue: GlueBox) -> bool {
        if self.boxes.contains(&glue) {
            return false;
        }
        self.boxes.push(glue);
        true
    }

    pub fn remove(&mut self, glue: &GlueBox) -> bool {
        let before = self.boxes.len();
        self.boxes.retain(|b| b != glue);
        self.boxes.len() != before
    }

    /// The boxes within `reach` blocks of `at` along each axis (`inflate(32, 16, 32)`).
    pub fn near(&self, at: [f64; 3], reach: [f64; 3]) -> Vec<GlueBox> {
        self.boxes
            .iter()
            .filter(|b| {
                (0..3).all(|i| {
                    f64::from(b.max[i]) + 1.0 >= at[i] - reach[i] && f64::from(b.min[i]) <= at[i] + reach[i]
                })
            })
            .copied()
            .collect()
    }

    /// The glue the look ray meets first within `range` (`SuperGlueSelectionHandler.tick`).
    pub fn picked(&self, eye: [f64; 3], look: [f64; 3], range: f64) -> Option<GlueBox> {
        self.boxes
            .iter()
            .filter_map(|b| b.clip(eye, look, range).map(|d| (d, *b)))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, b)| b)
    }
}

// --- which blocks hold together --------------------------------------------------------------------

/// `SuperGlueEntity.isValidFace`: glue can hold this face of a block.
pub fn is_valid_face(world: &dyn World, pos: Pos, face: Dir) -> bool {
    let Some(state) = world.state(pos) else { return false };
    if attached_towards(world, &state, face) {
        return true;
    }
    if !movement_necessary(world, &state, pos) {
        return false;
    }
    !not_supportive(world, &state, face)
}

/// `SuperGlueEntity.isSideSticky`: the blocks that already hold on to what is on one face.
pub fn is_side_sticky(state: &State, face: Dir) -> bool {
    if state.name.starts_with("create:") {
        match state.path() {
            "sticky_mechanical_piston" | "sticker" | "gantry_carriage" | "mechanical_bearing"
            | "windmill_bearing" | "clockwork_bearing" => {
                return state.dir("facing") == Some(face);
            }
            "cart_assembler" => return face == Dir::Up,
            _ => {}
        }
        return chassis::is_chassis(&state.name) && chassis::is_sticky(state, face);
    }
    state.is("minecraft:slime_block") || state.is("minecraft:honey_block")
}

/// `SuperGlueSelectionHelper.searchGlueGroup`: the blocks inside the box from `start` to `end`
/// that hold together (by face), reached from `start`. None for a single block. `include_other`
/// also counts glue and sticky sides already there (the client's preview; the server leaves it
/// out).
pub fn search_group(
    world: &dyn World,
    glue: &GlueSet,
    start: Pos,
    end: Pos,
    include_other: bool,
) -> Option<HashSet<Pos>> {
    let bounds = GlueBox::span(start, end);
    let mut frontier: VecDeque<Pos> = VecDeque::from([start]);
    let mut visited: HashSet<Pos> = HashSet::from([start]);
    let mut attached: HashSet<Pos> = HashSet::new();
    while let Some(current) = frontier.pop_front() {
        attached.insert(current);
        for d in Dir::ALL {
            let offset = relative(current, d, 1);
            let glue_present = include_other && glue.glued(current, d);
            let sticky = |pos: Pos, face: Dir| world.state(pos).is_some_and(|s| is_side_sticky(&s, face));
            let already_sticky = (include_other && sticky(current, d)) || sticky(offset, d.opposite());
            if !already_sticky && !glue_present && !bounds.contains(offset) {
                continue;
            }
            let Some(state) = world.state(offset) else { continue };
            if !movement_necessary(world, &state, offset) {
                continue;
            }
            if !is_valid_face(world, current, d) || !is_valid_face(world, offset, d.opposite()) {
                continue;
            }
            if visited.insert(offset) {
                frontier.push_back(offset);
            }
        }
    }
    if attached.len() < 2 && attached.contains(&end) {
        return None;
    }
    Some(attached)
}

// --- the clicks ---------------------------------------------------------------------------------------

/// What a right click with the glue came to (`SuperGlueSelectionHandler.onMouseInput` and
/// `SuperGlueSelectionPacket.handle`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Click {
    /// Not the glue's business: the click goes on to the block.
    Ignore,
    /// `firstPos` set to this block.
    FirstCorner(Pos),
    /// Sneaking: `discard`.
    Discard,
    /// `super_glue.too_far`.
    TooFar,
    /// `super_glue.cannot_reach`: the second block isn't joined to the first through the box.
    CannotReach,
    /// `super_glue.not_enough`.
    NotEnough,
    /// A glue is made over this box (`super_glue.success`).
    Glue(GlueBox),
}

/// `closerThan(from, to, 25)`: the squared distance under 25 squared.
pub fn closer_than(a: Pos, b: Pos, distance: f64) -> bool {
    let d: f64 = (0..3).map(|i| f64::from(a[i] - b[i]).powi(2)).sum();
    d < distance * distance
}

/// The click sequence: `first` is the corner picked so far, `pos` the block clicked now. `group`
/// answers `searchGlueGroup`, `can_afford` whether a charge is at hand.
pub fn click(
    first: Option<Pos>,
    pos: Pos,
    sneaking: bool,
    group: &dyn Fn(Pos, Pos) -> Option<HashSet<Pos>>,
    can_afford: bool,
) -> Click {
    if sneaking {
        return if first.is_some() { Click::Discard } else { Click::Ignore };
    }
    let Some(first) = first else {
        return Click::FirstCorner(pos);
    };
    if !closer_than(first, pos, MAX_DISTANCE) {
        return Click::TooFar;
    }
    // A cluster of a single block doesn't exist: the click starts again from here.
    let Some(cluster) = group(first, pos) else {
        return Click::FirstCorner(pos);
    };
    if !cluster.contains(&pos) {
        return Click::CannotReach;
    }
    if !can_afford {
        return Click::NotEnough;
    }
    Click::Glue(GlueBox::span(first, pos))
}

/// `collectGlueFromInventory` for one charge on a stack with `damage`: the damage after, or none
/// when the glue is used up (`hurtAndBreak`) or already was.
pub fn use_charge(damage: u32) -> Option<u32> {
    (damage < MAX_DAMAGE && damage + 1 < MAX_DAMAGE).then_some(damage + 1)
}

/// Whether a charge can be had from a stack with `damage` left (`maxDamage - damage >= 1`).
pub fn has_charge(damage: u32) -> bool {
    damage < MAX_DAMAGE
}

// --- the glue a structure carries -----------------------------------------------------------------

/// The glue a structure of `found` blocks takes (`glueToRemove`): every box that holds one of
/// them to a neighbour. The indexes into `glue.boxes`.
pub fn captured(glue: &GlueSet, found: &HashSet<Pos>) -> Vec<usize> {
    glue.boxes
        .iter()
        .enumerate()
        .filter(|(_, b)| {
            found.iter().any(|p| {
                b.contains(*p) && Dir::ALL.iter().any(|d| b.contains(relative(*p, *d, 1)))
            })
        })
        .map(|(i, _)| i)
        .collect()
}

/// `removeBlocksFromWorld`'s minimising: the box shrunk to the structure's blocks inside it, kept
/// when it covers more than one block. Positions in any one frame.
pub fn minimised(glue: &GlueBox, structure: &[Pos]) -> Option<GlueBox> {
    let inside: Vec<Pos> = structure.iter().copied().filter(|p| glue.contains(*p)).collect();
    let first = *inside.first()?;
    let shrunk = inside.iter().fold(GlueBox { min: first, max: first }, |acc, p| GlueBox {
        min: [0, 1, 2].map(|i| acc.min[i].min(p[i])),
        max: [0, 1, 2].map(|i| acc.max[i].max(p[i])),
    });
    shrunk.is_more_than_a_block().then_some(shrunk)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Axis;
    use std::collections::HashMap;

    struct Grid(HashMap<Pos, State>);

    impl Grid {
        fn solid(cells: &[Pos]) -> Grid {
            Grid(cells.iter().map(|p| (*p, State::new("minecraft:stone"))).collect())
        }
    }

    impl World for Grid {
        fn state(&self, pos: Pos) -> Option<State> {
            Some(self.0.get(&pos).cloned().unwrap_or_else(|| State::new("minecraft:air")))
        }
        fn collision_empty(&self, pos: Pos) -> bool {
            self.0.get(&pos).is_none_or(State::is_air)
        }
    }

    #[test]
    fn a_box_spans_two_corners_either_way_round() {
        let b = GlueBox::span([3, 5, -2], [1, 7, -4]);
        assert_eq!((b.min, b.max), ([1, 5, -4], [3, 7, -2]));
        assert!(b.contains([2, 6, -3]) && !b.contains([4, 6, -3]));
        assert!(b.is_more_than_a_block() && !GlueBox::span([1; 3], [1; 3]).is_more_than_a_block());
    }

    #[test]
    fn the_outline_runs_along_the_edges_and_through_every_corner() {
        let one = GlueBox::span([0, 0, 0], [0, 0, 0]);
        let points = one.outline_points(1.0);
        // The eight corners and nothing else: an edge a block long has no point between.
        assert_eq!(points.len(), 8);
        assert!(points.contains(&[0.0, 0.0, 0.0]) && points.contains(&[1.0, 1.0, 1.0]));
        let long = GlueBox::span([0, 0, 0], [3, 0, 0]);
        let points = long.outline_points(1.0);
        assert!(points.contains(&[2.0, 0.0, 1.0]) && points.contains(&[4.0, 1.0, 0.0]));
        assert!(points.iter().all(|p| p[0] >= 0.0 && p[0] <= 4.0 && p[1] >= 0.0 && p[1] <= 1.0));
        // A huge box is capped.
        let huge = GlueBox::span([0, 0, 0], [24, 24, 24]);
        assert!(huge.outline_points(1.0).len() <= 12 * 17);
    }

    #[test]
    fn glue_holds_a_block_to_a_neighbour_inside_the_same_box() {
        let set = GlueSet::new(vec![GlueBox::span([0, 0, 0], [1, 0, 0])]);
        assert!(set.glued([0, 0, 0], Dir::East) && set.glued([1, 0, 0], Dir::West));
        assert!(!set.glued([1, 0, 0], Dir::East) && !set.glued([0, 0, 0], Dir::Up));
        // The two blocks have to be in one box: two boxes touching don't hold.
        let two = GlueSet::new(vec![GlueBox::span([0, 0, 0], [0, 0, 0]), GlueBox::span([1, 0, 0], [1, 0, 0])]);
        assert!(!two.glued([0, 0, 0], Dir::East));
    }

    #[test]
    fn a_glued_box_joins_the_blocks_in_it_and_not_the_rest() {
        // Searching from the corner: the 2x2 inside the box hold together, the block beyond
        // doesn't join.
        let cells = [[0, 0, 0], [1, 0, 0], [0, 0, 1], [1, 0, 1], [2, 0, 0]];
        let grid = Grid::solid(&cells);
        let group = search_group(&grid, &GlueSet::default(), [0, 0, 0], [1, 0, 1], false).unwrap();
        assert_eq!(group.len(), 4);
        assert!(!group.contains(&[2, 0, 0]));
    }

    #[test]
    fn a_gap_in_the_box_cannot_be_reached() {
        let grid = Grid::solid(&[[0, 0, 0], [2, 0, 0]]);
        let group = search_group(&grid, &GlueSet::default(), [0, 0, 0], [2, 0, 0], false).unwrap();
        // The corner is alone: the far block isn't in the group.
        assert!(!group.contains(&[2, 0, 0]));
        // A single block is no selection.
        assert!(search_group(&grid, &GlueSet::default(), [0, 0, 0], [0, 0, 0], false).is_none());
    }

    #[test]
    fn a_sticky_block_joins_what_is_outside_the_box() {
        // Slime holds on to the stone beyond the box (`alreadySticky`).
        let mut grid = Grid::solid(&[[0, 0, 0], [1, 0, 0]]);
        grid.0.insert([1, 0, 0], State::new("minecraft:slime_block"));
        grid.0.insert([2, 0, 0], State::new("minecraft:stone"));
        // The client's preview counts it (`includeOther`); the server's check leaves out what
        // this block holds on to by itself, and counts only what holds from the far side.
        let group = search_group(&grid, &GlueSet::default(), [0, 0, 0], [1, 0, 0], true).unwrap();
        assert!(group.contains(&[2, 0, 0]));
        let group = search_group(&grid, &GlueSet::default(), [0, 0, 0], [1, 0, 0], false).unwrap();
        assert!(!group.contains(&[2, 0, 0]));
        // A sticky block beyond the box holds what is inside it from its own side.
        let mut grid = Grid::solid(&[[0, 0, 0], [1, 0, 0]]);
        grid.0.insert([2, 0, 0], State::new("minecraft:slime_block"));
        let group = search_group(&grid, &GlueSet::default(), [0, 0, 0], [1, 0, 0], false).unwrap();
        assert!(group.contains(&[2, 0, 0]));
    }

    #[test]
    fn the_click_sequence() {
        let all = |a: Pos, b: Pos| {
            let mut s = HashSet::new();
            s.insert(a);
            s.insert(b);
            Some(s)
        };
        // First corner, then the second makes the glue.
        assert_eq!(click(None, [0, 0, 0], false, &all, true), Click::FirstCorner([0, 0, 0]));
        assert_eq!(
            click(Some([0, 0, 0]), [2, 1, 0], false, &all, true),
            Click::Glue(GlueBox::span([0, 0, 0], [2, 1, 0]))
        );
        assert_eq!(click(Some([0, 0, 0]), [2, 1, 0], false, &all, false), Click::NotEnough);
        // Sneaking discards a selection, and does nothing without one.
        assert_eq!(click(Some([0, 0, 0]), [2, 1, 0], true, &all, true), Click::Discard);
        assert_eq!(click(None, [2, 1, 0], true, &all, true), Click::Ignore);
        // Too far, unreachable, and a lone block starting again.
        assert_eq!(click(Some([0, 0, 0]), [30, 0, 0], false, &all, true), Click::TooFar);
        let apart = |a: Pos, _: Pos| Some(HashSet::from([a]));
        assert_eq!(click(Some([0, 0, 0]), [2, 0, 0], false, &apart, true), Click::CannotReach);
        let none = |_: Pos, _: Pos| None;
        assert_eq!(click(Some([0, 0, 0]), [2, 0, 0], false, &none, true), Click::FirstCorner([2, 0, 0]));
        // The reach is a distance under 25, not a box.
        assert!(closer_than([0, 0, 0], [24, 0, 0], 25.0) && !closer_than([0, 0, 0], [25, 0, 0], 25.0));
    }

    #[test]
    fn the_glue_item_wears_out_after_ninety_eight_more_uses() {
        // `durability(99)`: the stack breaks with the 99th use.
        assert_eq!(use_charge(0), Some(1));
        assert_eq!(use_charge(97), Some(98));
        assert_eq!(use_charge(98), None);
        assert!(has_charge(98) && !has_charge(99));
    }

    #[test]
    fn the_look_picks_the_nearest_glue() {
        let near = GlueBox::span([0, 0, 2], [0, 0, 3]);
        let far = GlueBox::span([0, 0, 5], [0, 0, 6]);
        let set = GlueSet::new(vec![far, near]);
        let eye = [0.5, 0.5, -1.0];
        assert_eq!(set.picked(eye, [0.0, 0.0, 1.0], 5.5), Some(near));
        // Out of reach, and looking elsewhere.
        assert_eq!(set.picked(eye, [0.0, 0.0, 1.0], 2.0), None);
        assert_eq!(set.picked(eye, [1.0, 0.0, 0.0], 5.5), None);
        assert!(set.near([0.0, 0.0, 0.0], [32.0, 16.0, 32.0]).len() == 2);
        assert!(set.near([100.0, 0.0, 0.0], [32.0, 16.0, 32.0]).is_empty());
    }

    #[test]
    fn a_structure_takes_the_glue_holding_its_blocks_and_brings_it_back_turned() {
        let set = GlueSet::new(vec![
            GlueBox::span([0, 0, 0], [3, 0, 0]),
            GlueBox::span([10, 0, 0], [12, 0, 0]),
        ]);
        let found: HashSet<Pos> = HashSet::from([[1, 0, 0], [2, 0, 0]]);
        assert_eq!(captured(&set, &found), vec![0]);
        // Shrunk to the blocks the structure holds, in the structure's own frame.
        let structure = [[1, 0, 0], [2, 0, 0]];
        let shrunk = minimised(&set.boxes[0], &structure).unwrap();
        assert_eq!((shrunk.min, shrunk.max), ([1, 0, 0], [2, 0, 0]));
        // One block of the box in the structure leaves nothing to glue.
        assert_eq!(minimised(&set.boxes[0], &[[1, 0, 0]]), None);
        // Relative to an anchor and back, and turned a quarter about Y: x runs into -z.
        let local = shrunk.relative_to([1, 0, 0]);
        assert_eq!((local.min, local.max), ([0, 0, 0], [1, 0, 0]));
        let turned = local.transformed(&Transform { offset: [5, 5, 5], axis: Axis::Y, quarters: 1 });
        assert_eq!((turned.min, turned.max), ([5, 5, 4], [5, 5, 5]));
        assert_eq!(local.moved([1, 0, 0]), shrunk);
    }

    #[test]
    fn sides_that_already_hold_on() {
        assert!(is_side_sticky(&State::new("minecraft:slime_block"), Dir::Up));
        let piston = State::parse("create:sticky_mechanical_piston[facing=east]");
        assert!(is_side_sticky(&piston, Dir::East) && !is_side_sticky(&piston, Dir::West));
        let chassis = State::parse("create:linear_chassis[axis=y,sticky_top=true,sticky_bottom=false]");
        assert!(is_side_sticky(&chassis, Dir::Up) && !is_side_sticky(&chassis, Dir::Down));
        assert!(!is_side_sticky(&State::new("minecraft:stone"), Dir::Up));
    }
}
