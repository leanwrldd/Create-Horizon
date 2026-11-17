//! The Wand of Symmetry (`SymmetryWandItem`, `SymmetryHandler`, `SymmetryMirror`, `PlaneMirror`,
//! `CrossPlaneMirror`, `TriplePlaneMirror`, `EmptyMirror`): where a placement or a break is copied to, how a
//! click sets the mirror, what a copy costs in survival and what the mirror looks like.
//!
//! The turns of a block state are in `symmetry_wand_state`.
//!
//! Known differences from Create, for the README:
//! - no screen: a sneaking click steps the mirror type, a sneaking click with a Wrench in the other hand steps
//!   its orientation (`SymmetryWandScreen`);
//! - the mirror is drawn as outlines of its planes, not Create's models (`symmetry_effect/*`), and without the
//!   end rod particles;
//! - no `isUnobstructed` check (entities in the way don't stop a copy) and no `updateShape` pass over a copy;
//! - Create takes one more item from the inventory first when the player holds the block in survival; here only
//!   the copies cost, one item each;
//! - a copy a survival player breaks doesn't wear the held tool.

use super::symmetry_wand_state::{self as state, Turn};

/// `CEquipment.maxSymmetryWandRange` (default 50): the farthest a block may be from the mirror.
pub const MAX_RANGE: f64 = 50.0;

/// The data components (`AllDataComponents.SYMMETRY_WAND`, `SYMMETRY_WAND_ENABLE`).
pub const COMPONENT: &str = "create:symmetry_wand";
pub const ENABLE_COMPONENT: &str = "create:symmetry_wand_enable";

/// `SymmetryMirror.EMPTY`, `PLANE`, `CROSS_PLANE`, `TRIPLE_PLANE`: the codec's `type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Empty,
    Plane,
    CrossPlane,
    TriplePlane,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Empty => "empty",
            Kind::Plane => "plane",
            Kind::CrossPlane => "cross_plane",
            Kind::TriplePlane => "triple_plane",
        }
    }

    /// `SymmetryMirror.create`: an unknown type is the empty mirror.
    pub fn from_name(name: &str) -> Kind {
        match name {
            "plane" => Kind::Plane,
            "cross_plane" => Kind::CrossPlane,
            "triple_plane" => Kind::TriplePlane,
            _ => Kind::Empty,
        }
    }

    /// How many orientations the type has (`Align.values().length`; the triple plane takes none).
    pub fn orientations(self) -> i32 {
        match self {
            Kind::Plane | Kind::CrossPlane => 2,
            Kind::Empty | Kind::TriplePlane => 1,
        }
    }

    /// The screen's wording (`create.symmetry.mirror.plane`, `doublePlane`, `triplePlane`).
    pub fn label(self) -> &'static str {
        match self {
            Kind::Empty => "No mirror",
            Kind::Plane => "Mirror Once",
            Kind::CrossPlane => "Rectangular",
            Kind::TriplePlane => "Octagonal",
        }
    }

    /// `getAlignToolTips`, by orientation index (`orientation.alongZ`, ...).
    pub fn orientation_label(self, index: i32) -> &'static str {
        match (self, index) {
            (Kind::Plane, 0) => "Along Z",
            (Kind::Plane, _) => "Along X",
            (Kind::CrossPlane, 0) => "Orthogonal",
            (Kind::CrossPlane, _) => "Diagonal",
            (Kind::TriplePlane, _) => "Horizontal",
            (Kind::Empty, _) => "",
        }
    }
}

/// A wand's mirror: `SymmetryMirror` with its codec's fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mirror {
    pub kind: Kind,
    /// `orientation_index`.
    pub orientation: i32,
    /// `position`: always a block corner (`BlockPos` as a `Vec3`).
    pub position: [i32; 3],
    /// The mirror's own `enable`.
    pub enable: bool,
}

impl Mirror {
    /// `checkComponents`: a wand with no mirror holds an empty one at the origin.
    pub const EMPTY: Mirror = Mirror { kind: Kind::Empty, orientation: 0, position: [0; 3], enable: true };

    /// `SymmetryMirror.create(orientationIndex, position, type, enable)`: the index is kept inside the type's range.
    pub fn from_fields(orientation: i32, position: [f64; 3], kind: &str, enable: bool) -> Mirror {
        let kind = Kind::from_name(kind);
        Mirror {
            kind,
            orientation: orientation.clamp(0, kind.orientations() - 1),
            position: [position[0].floor() as i32, position[1].floor() as i32, position[2].floor() as i32],
            enable,
        }
    }

    /// A fresh mirror of `kind` at `position` (`new PlaneMirror(pos)`): orientation 0, enabled.
    pub fn new(kind: Kind, position: [i32; 3]) -> Mirror {
        Mirror { kind, orientation: 0, position, enable: true }
    }

    /// `getAlignToolTips().get(orientation)`'s wording.
    pub fn describe(&self) -> String {
        match self.kind {
            Kind::Empty => "No mirror set".to_owned(),
            kind => format!(
                "{} ({}) at {} {} {}",
                kind.label(),
                kind.orientation_label(self.orientation),
                self.position[0],
                self.position[1],
                self.position[2]
            ),
        }
    }
}

// ---- Placing the mirror -------------------------------------------------------------------------------------------

/// `Direction.fromYRot`: the horizontal direction a yaw looks along (`south` at 0, `west` at 90).
pub fn direction_of_yaw(yaw: f32) -> &'static str {
    match ((f64::from(yaw) / 90.0 + 0.5).floor() as i64).rem_euclid(4) {
        0 => "south",
        1 => "west",
        2 => "north",
        _ => "east",
    }
}

/// `SymmetryWandItem.useOn` without a sneak: the mirror after a click on the block `clicked` from `face`, `previous`
/// being what the wand held. `facing` is `player.getDirection()` and `head_yaw` `getYHeadRot`.
pub fn click(previous: Mirror, clicked: [i32; 3], face: [i32; 3], facing: &str, head_yaw: f32) -> Mirror {
    let position = [clicked[0] + face[0], clicked[1] + face[1], clicked[2] + face[2]];
    let along_z = facing == "north" || facing == "south";
    if previous.kind == Kind::Empty {
        // A new plane mirror: XY when the player faces north or south, YZ otherwise.
        return Mirror { kind: Kind::Plane, orientation: i32::from(!along_z), position, enable: true };
    }
    let mut mirror = Mirror { position, ..previous };
    match previous.kind {
        Kind::Plane => mirror.orientation = i32::from(!along_z),
        Kind::CrossPlane => {
            let abs = (head_yaw % 90.0).abs();
            let diagonal = abs > 22.0 && abs < 45.0 + 22.0;
            mirror.orientation = i32::from(diagonal);
        }
        Kind::TriplePlane | Kind::Empty => {}
    }
    mirror
}

/// The screen's type selector, stood in by a sneaking click: the next of plane, cross plane and triple plane, at the
/// same position (`SymmetryWandScreen`: an empty mirror shows as a plane at the origin).
pub fn next_kind(mirror: Mirror) -> Mirror {
    let kind = match mirror.kind {
        Kind::Empty => Kind::Plane,
        Kind::Plane => Kind::CrossPlane,
        Kind::CrossPlane => Kind::TriplePlane,
        Kind::TriplePlane => Kind::Plane,
    };
    Mirror::new(kind, mirror.position)
}

/// The screen's orientation selector, stood in by a sneaking click with a Wrench in the other hand
/// (`SymmetryMirror.rotate`, wrapping).
pub fn next_orientation(mirror: Mirror) -> Mirror {
    let count = mirror.kind.orientations();
    Mirror { orientation: (mirror.orientation + 1).rem_euclid(count), ..mirror }
}

// ---- Copies ---------------------------------------------------------------------------------------------------------

/// `SymmetryMirror.flipX(BlockPos)` and the others: a position turned about the mirror.
pub fn flip(mirror: [i32; 3], pos: [i32; 3], turn: Turn) -> [i32; 3] {
    let (dx, dz) = (pos[0] - mirror[0], pos[2] - mirror[2]);
    match turn {
        Turn::X => [pos[0] - 2 * dx, pos[1], pos[2]],
        Turn::Z => [pos[0], pos[1], pos[2] - 2 * dz],
        Turn::D2 => [pos[0] - dx + dz, pos[1], pos[2] - dz + dx],
        Turn::D1 => [pos[0] - dx - dz, pos[1], pos[2] - dz - dx],
    }
}

/// A copy of a block: where, and the turns its state takes, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Copy {
    pub pos: [i32; 3],
    pub turns: Vec<Turn>,
}

/// The turn lists of `SymmetryMirror.process(pos, block)`, in the order each type puts them in its result map. A copy
/// is `flipA(flipB(x))`: the turns apply innermost first.
fn recipes(kind: Kind, orientation: i32) -> Vec<Vec<Turn>> {
    use Turn::{D1, D2, X, Z};
    match (kind, orientation) {
        (Kind::Empty, _) => vec![],
        // `PlaneMirror`: XY (0) flips z, YZ (1) flips x.
        (Kind::Plane, 0) => vec![vec![Z]],
        (Kind::Plane, _) => vec![vec![X]],
        // `CrossPlaneMirror`: Y (0) flips x and z, D (1) the two diagonals.
        (Kind::CrossPlane, 0) => vec![vec![X], vec![Z], vec![Z, X]],
        (Kind::CrossPlane, _) => vec![vec![D1], vec![D2], vec![D2, D1]],
        // `TriplePlaneMirror`.
        (Kind::TriplePlane, _) => {
            vec![vec![X], vec![Z], vec![Z, X], vec![D1], vec![X, D1], vec![Z, D1], vec![Z, X, D1]]
        }
    }
}

/// `SymmetryMirror.process(blocks)` for one block at `pos`: the other places it is copied to. Where two flips land
/// on one place the later one wins (they go into a map in order), and the block's own place is never a copy
/// (`SymmetryWandItem.apply` skips it).
pub fn copies(mirror: &Mirror, pos: [i32; 3]) -> Vec<Copy> {
    let mut result: Vec<Copy> = Vec::new();
    for turns in recipes(mirror.kind, mirror.orientation) {
        let mut at = pos;
        for turn in &turns {
            at = flip(mirror.position, at, *turn);
        }
        let copy = Copy { pos: at, turns };
        match result.iter_mut().find(|c| c.pos == at) {
            Some(existing) => *existing = copy,
            None => result.push(copy),
        }
    }
    result.retain(|c| c.pos != pos);
    result
}

/// The copies of a placed state: where each goes and the state's text turned.
pub fn copies_of_state(mirror: &Mirror, pos: [i32; 3], state_text: &str) -> Vec<([i32; 3], String)> {
    copies(mirror, pos).into_iter().map(|c| (c.pos, state::turned(state_text, &c.turns))).collect()
}

/// `mirrorPos.distanceTo(Vec3.atLowerCornerOf(pos)) > range` returns without doing anything.
pub fn in_range(mirror: &Mirror, pos: [i32; 3]) -> bool {
    let d: [f64; 3] = std::array::from_fn(|i| f64::from(pos[i] - mirror.position[i]));
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() <= MAX_RANGE
}

// ---- Survival ---------------------------------------------------------------------------------------------------

/// How many items `BlockHelper.findAndRemoveInInventory` wants for a block state: a double slab takes two, eggs,
/// pickles and candles one per count, and a vine or glow lichen one per side it grows on.
pub fn item_cost(state_text: &str) -> u32 {
    let (name, props) = state::parse(state_text);
    let mut amount: u32 = 1;
    if props.get("type").is_some_and(|t| t == "double") {
        amount *= 2;
    }
    for key in ["eggs", "pickles", "candles"] {
        if let Some(n) = props.get(key).and_then(|v| v.parse::<u32>().ok()) {
            amount *= n;
        }
    }
    if name == "minecraft:vine" || name == "minecraft:glow_lichen" {
        let sides = ["up", "north", "east", "south", "west", "down"]
            .iter()
            .filter(|s| props.get(**s).is_some_and(|v| v == "true"))
            .count() as u32;
        amount = (amount + sides).saturating_sub(1).max(1);
    }
    amount
}

/// Which slots to take `need` items from, `slots` being (slot, count) of every stack of the item in the order to try
/// them (the held slot first, as `findAndRemoveInInventory` does). `None` when they don't add up to `need`.
pub fn take_plan(slots: &[(usize, u32)], need: u32) -> Option<Vec<(usize, u32)>> {
    let mut left = need;
    let mut plan = Vec::new();
    for &(slot, count) in slots {
        if left == 0 {
            break;
        }
        let taken = count.min(left);
        if taken > 0 {
            plan.push((slot, taken));
            left -= taken;
        }
    }
    (left == 0).then_some(plan)
}

// ---- Effects and drawing ------------------------------------------------------------------------------------

/// `SymmetryHandler.drawEffect`: the dust along the line from the mirror to a copy (every 0.8 blocks, from the third
/// step to the second last), as points.
pub fn effect_points(from: [i32; 3], to: [i32; 3]) -> Vec<[f64; 3]> {
    let start: [f64; 3] = std::array::from_fn(|i| f64::from(from[i]) + 0.5);
    let end: [f64; 3] = std::array::from_fn(|i| f64::from(to[i]) + 0.5);
    let diff: [f64; 3] = std::array::from_fn(|i| end[i] - start[i]);
    let length = (diff[0] * diff[0] + diff[1] * diff[1] + diff[2] * diff[2]).sqrt();
    if length == 0.0 {
        return vec![];
    }
    let density = 0.8;
    let steps = (length / density) as i32;
    (3..steps - 1)
        .map(|i| std::array::from_fn(|k| start[k] + diff[k] / length * density * f64::from(i)))
        .collect()
}

/// `SymmetryHandler.onRenderWorld`'s bob: `sin(time / 16) / 5`.
pub fn bob(render_time: f64) -> f64 {
    (render_time / 16.0).sin() / 5.0
}

/// A line to draw, in world coordinates.
pub type Segment = ([f64; 3], [f64; 3]);

/// The unit directions (x, z) the mirror's vertical planes run along: a plane that flips z runs along x, one that
/// flips x along z, and the diagonal flips run along (1, 1) and (1, -1).
pub fn plane_directions(mirror: &Mirror) -> Vec<[f64; 2]> {
    let d = std::f64::consts::FRAC_1_SQRT_2;
    match (mirror.kind, mirror.orientation) {
        (Kind::Empty, _) => vec![],
        (Kind::Plane, 0) => vec![[1.0, 0.0]],
        (Kind::Plane, _) => vec![[0.0, 1.0]],
        (Kind::CrossPlane, 0) => vec![[1.0, 0.0], [0.0, 1.0]],
        (Kind::CrossPlane, _) => vec![[d, d], [d, -d]],
        (Kind::TriplePlane, _) => vec![[1.0, 0.0], [0.0, 1.0], [d, d], [d, -d]],
    }
}

/// The mirror as lines: each plane is a square a block wide (the models are 1 block, `symmetry_effect/plane`),
/// 14 pixels across, standing 0.2 above the block's floor and bobbing by `bob`.
pub fn outline(mirror: &Mirror, bob: f64) -> Vec<Segment> {
    let centre = [f64::from(mirror.position[0]) + 0.5, f64::from(mirror.position[1]) + 0.2 + bob, f64::from(mirror.position[2]) + 0.5];
    let (half, low, high) = (7.0 / 16.0, 1.0 / 16.0, 15.0 / 16.0);
    let mut lines = Vec::new();
    for [dx, dz] in plane_directions(mirror) {
        let at = |across: f64, up: f64| [centre[0] + dx * across, centre[1] + up, centre[2] + dz * across];
        let corners = [at(-half, low), at(half, low), at(half, high), at(-half, high)];
        for i in 0..4 {
            lines.push((corners[i], corners[(i + 1) % 4]));
        }
        // The cross inside the square.
        lines.push((at(-half, 0.5), at(half, 0.5)));
        lines.push((at(0.0, low), at(0.0, high)));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(kind: Kind, orientation: i32) -> Mirror {
        Mirror { kind, orientation, position: [10, 64, 10], enable: true }
    }

    fn positions(m: &Mirror, pos: [i32; 3]) -> Vec<[i32; 3]> {
        let mut out: Vec<[i32; 3]> = copies(m, pos).into_iter().map(|c| c.pos).collect();
        out.sort();
        out
    }

    #[test]
    fn a_plane_reflects_across_its_block() {
        // XY flips z: the plane stands through the block at z = 10.
        assert_eq!(positions(&at(Kind::Plane, 0), [12, 65, 13]), vec![[12, 65, 7]]);
        assert_eq!(positions(&at(Kind::Plane, 1), [12, 65, 13]), vec![[8, 65, 13]]);
        // A block on the plane has no copy.
        assert!(positions(&at(Kind::Plane, 0), [12, 65, 10]).is_empty());
    }

    #[test]
    fn a_cross_copies_to_every_quarter() {
        assert_eq!(positions(&at(Kind::CrossPlane, 0), [12, 65, 13]), vec![[8, 65, 7], [8, 65, 13], [12, 65, 7]]);
        // Diagonal: swapping (dx, dz), (-dz, -dx) and (-dx, -dz).
        assert_eq!(positions(&at(Kind::CrossPlane, 1), [12, 65, 13]), vec![[7, 65, 8], [8, 65, 7], [13, 65, 12]]);
    }

    #[test]
    fn a_block_on_a_crosss_axis_has_fewer_copies_and_the_later_flip_wins() {
        // On the x axis (z = 10): flipX gives itself (skipped), flipZ itself too, flipX(flipZ) the far side.
        let found = copies(&at(Kind::CrossPlane, 0), [12, 65, 10]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].pos, [8, 65, 10]);
        assert_eq!(found[0].turns, vec![Turn::Z, Turn::X]);
        // Off the axes, the one at (-dx, dz) takes its flip alone and the one at (-dx, -dz) both.
        let found = copies(&at(Kind::CrossPlane, 0), [12, 65, 13]);
        let far = found.iter().find(|c| c.pos == [8, 65, 7]).unwrap();
        assert_eq!(far.turns, vec![Turn::Z, Turn::X]);
    }

    #[test]
    fn a_triple_plane_makes_the_other_seven() {
        let found = positions(&at(Kind::TriplePlane, 0), [13, 65, 11]);
        assert_eq!(found.len(), 7);
        // The eight form the square's symmetries of (3, 1): (+-3, +-1) and (+-1, +-3).
        let mut offsets: Vec<[i32; 2]> = found.iter().map(|p| [p[0] - 10, p[2] - 10]).collect();
        offsets.push([3, 1]);
        offsets.sort();
        assert_eq!(
            offsets,
            vec![[-3, -1], [-3, 1], [-1, -3], [-1, 3], [1, -3], [1, 3], [3, -1], [3, 1]]
        );
    }

    #[test]
    fn an_empty_mirror_copies_nothing() {
        assert!(copies(&Mirror::EMPTY, [1, 2, 3]).is_empty());
    }

    #[test]
    fn states_are_turned_with_their_copies() {
        let found = copies_of_state(&at(Kind::Plane, 0), [12, 65, 13], "minecraft:furnace[facing=north,lit=false]");
        assert_eq!(found, vec![([12, 65, 7], "minecraft:furnace[facing=south,lit=false]".to_owned())]);
        let found = copies_of_state(&at(Kind::Plane, 1), [12, 65, 13], "minecraft:oak_log[axis=x]");
        assert_eq!(found, vec![([8, 65, 13], "minecraft:oak_log[axis=x]".to_owned())]);
    }

    #[test]
    fn the_range_is_fifty_blocks() {
        let m = at(Kind::Plane, 0);
        assert!(in_range(&m, [10, 64, 60]));
        assert!(!in_range(&m, [10, 64, 61]));
        assert!(in_range(&m, [40, 64, 50]), "a 30-40-50 triangle is just in");
        assert!(!in_range(&m, [41, 64, 50]));
    }

    #[test]
    fn a_first_click_makes_a_plane_facing_the_player() {
        let made = click(Mirror::EMPTY, [5, 64, 5], [0, 1, 0], "north", 180.0);
        assert_eq!((made.kind, made.orientation, made.position), (Kind::Plane, 0, [5, 65, 5]));
        let made = click(Mirror::EMPTY, [5, 64, 5], [1, 0, 0], "east", -90.0);
        assert_eq!((made.kind, made.orientation, made.position), (Kind::Plane, 1, [6, 64, 5]));
    }

    #[test]
    fn a_later_click_keeps_the_type_and_moves_the_mirror() {
        let tri = at(Kind::TriplePlane, 0);
        assert_eq!(click(tri, [0, 0, 0], [0, 1, 0], "west", 90.0).position, [0, 1, 0]);
        // A cross: diagonal when the head yaw is a diagonal one.
        let cross = at(Kind::CrossPlane, 0);
        assert_eq!(click(cross, [0, 0, 0], [0, 1, 0], "south", 45.0).orientation, 1);
        assert_eq!(click(cross, [0, 0, 0], [0, 1, 0], "south", 90.0).orientation, 0);
        assert_eq!(click(cross, [0, 0, 0], [0, 1, 0], "south", -135.0).orientation, 1);
        assert_eq!(click(cross, [0, 0, 0], [0, 1, 0], "south", 20.0).orientation, 0);
        // A plane follows the facing again.
        let plane = at(Kind::Plane, 0);
        assert_eq!(click(plane, [0, 0, 0], [0, 1, 0], "east", -90.0).orientation, 1);
    }

    #[test]
    fn yaw_names_the_direction() {
        assert_eq!(direction_of_yaw(0.0), "south");
        assert_eq!(direction_of_yaw(90.0), "west");
        assert_eq!(direction_of_yaw(180.0), "north");
        assert_eq!(direction_of_yaw(-90.0), "east");
        assert_eq!(direction_of_yaw(44.0), "south");
        assert_eq!(direction_of_yaw(46.0), "west");
        assert_eq!(direction_of_yaw(-180.0), "north");
    }

    #[test]
    fn the_stand_ins_step_the_type_and_the_orientation() {
        let m = next_kind(at(Kind::Plane, 1));
        assert_eq!((m.kind, m.orientation, m.position), (Kind::CrossPlane, 0, [10, 64, 10]));
        assert_eq!(next_kind(m).kind, Kind::TriplePlane);
        assert_eq!(next_kind(next_kind(m)).kind, Kind::Plane);
        assert_eq!(next_kind(Mirror::EMPTY).kind, Kind::Plane);
        assert_eq!(next_orientation(at(Kind::Plane, 0)).orientation, 1);
        assert_eq!(next_orientation(at(Kind::Plane, 1)).orientation, 0);
        assert_eq!(next_orientation(at(Kind::TriplePlane, 0)).orientation, 0);
    }

    #[test]
    fn fields_round_trip_and_stay_in_range() {
        let m = Mirror::from_fields(1, [3.0, 70.0, -4.0], "cross_plane", true);
        assert_eq!(m, Mirror { kind: Kind::CrossPlane, orientation: 1, position: [3, 70, -4], enable: true });
        assert_eq!(Mirror::from_fields(9, [0.0; 3], "triple_plane", false).orientation, 0);
        assert_eq!(Mirror::from_fields(0, [0.0; 3], "nonsense", true).kind, Kind::Empty);
        assert_eq!(Kind::from_name(Kind::TriplePlane.name()), Kind::TriplePlane);
    }

    #[test]
    fn costs_follow_the_block_helper() {
        assert_eq!(item_cost("minecraft:stone"), 1);
        assert_eq!(item_cost("minecraft:oak_slab[type=double]"), 2);
        assert_eq!(item_cost("minecraft:oak_slab[type=bottom]"), 1);
        assert_eq!(item_cost("minecraft:candle[candles=3,lit=false]"), 3);
        assert_eq!(item_cost("minecraft:vine[up=true,north=true,east=false,south=true,west=false]"), 3);
        assert_eq!(item_cost("minecraft:vine[up=false,north=true]"), 1);
    }

    #[test]
    fn a_take_plan_walks_the_slots() {
        assert_eq!(take_plan(&[(3, 2), (0, 5)], 4), Some(vec![(3, 2), (0, 2)]));
        assert_eq!(take_plan(&[(3, 2)], 3), None);
        assert_eq!(take_plan(&[], 0), Some(vec![]));
    }

    #[test]
    fn the_effect_line_leaves_the_ends_free() {
        let pts = effect_points([0, 0, 0], [10, 0, 0]);
        // 10 / 0.8 = 12 steps: i = 3..=10.
        assert_eq!(pts.len(), 8);
        assert!((pts[0][0] - (0.5 + 0.8 * 3.0)).abs() < 1e-9);
        assert!(effect_points([1, 1, 1], [1, 1, 1]).is_empty());
        assert!(effect_points([0, 0, 0], [1, 0, 0]).is_empty());
    }

    #[test]
    fn the_outline_has_six_lines_a_plane() {
        assert_eq!(outline(&at(Kind::Plane, 0), 0.0).len(), 6);
        assert_eq!(outline(&at(Kind::TriplePlane, 0), 0.0).len(), 24);
        assert!(outline(&Mirror::EMPTY, 0.0).is_empty());
        // A plane that flips z runs along x: its corners differ in x, not z.
        let lines = outline(&at(Kind::Plane, 0), 0.0);
        assert!((lines[0].0[2] - lines[0].1[2]).abs() < 1e-9);
        assert!((lines[0].0[0] - lines[0].1[0]).abs() > 0.5);
    }

    #[test]
    fn it_describes_itself() {
        assert_eq!(Mirror::EMPTY.describe(), "No mirror set");
        assert_eq!(at(Kind::Plane, 0).describe(), "Mirror Once (Along Z) at 10 64 10");
    }
}
