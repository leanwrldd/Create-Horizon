//! How a schematic is turned, mirrored and moved: vanilla's `StructureTemplate.transform` and
//! `BlockState.rotate`/`mirror` for the properties blocks turn with, and Create's
//! `SchematicTransformation` (the maths behind the hologram's tools: its anchor, its rotation about
//! the centre and its mirrors). No game types.
//!
//! Known differences from Create, for the README:
//! - `BlockState.rotate` and `mirror` are not callable, so [`transform_state`] turns the properties
//!   vanilla's and Create's blocks turn with (`facing`, `axis`, `rotation`, the four sides, rails,
//!   stairs' shape, doors' hinge, `orientation`, `axis_along_first`, `horizontal_axis`) by name; a
//!   block of another mod with a property of its own that needs turning isn't.
//! - Create's own `transform` of a few blocks (the chassis' sticky sides, vertical belts) is not
//!   applied: those blocks keep the turn their `facing` gets.

use crate::rules::contraption::State;
use crate::rules::schematic_structure::Pos;

/// `net.minecraft.world.level.block.Rotation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rotation {
    None,
    Clockwise90,
    Clockwise180,
    Counterclockwise90,
}

impl Rotation {
    /// Quarter turns clockwise seen from above (the enum's ordinal).
    pub fn quarters(self) -> i32 {
        match self {
            Rotation::None => 0,
            Rotation::Clockwise90 => 1,
            Rotation::Clockwise180 => 2,
            Rotation::Counterclockwise90 => 3,
        }
    }

    pub fn from_quarters(quarters: i32) -> Self {
        match quarters.rem_euclid(4) {
            0 => Rotation::None,
            1 => Rotation::Clockwise90,
            2 => Rotation::Clockwise180,
            _ => Rotation::Counterclockwise90,
        }
    }

    /// The name vanilla's codec writes (`StringRepresentable`).
    pub fn name(self) -> &'static str {
        match self {
            Rotation::None => "none",
            Rotation::Clockwise90 => "clockwise_90",
            Rotation::Clockwise180 => "180",
            Rotation::Counterclockwise90 => "counterclockwise_90",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [Rotation::None, Rotation::Clockwise90, Rotation::Clockwise180, Rotation::Counterclockwise90]
            .into_iter()
            .find(|r| r.name() == name || (*r == Rotation::Clockwise180 && name == "clockwise_180"))
    }

    pub fn then(self, other: Rotation) -> Rotation {
        Rotation::from_quarters(self.quarters() + other.quarters())
    }
}

/// `net.minecraft.world.level.block.Mirror`: left-right negates z, front-back negates x.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mirror {
    None,
    LeftRight,
    FrontBack,
}

impl Mirror {
    pub fn name(self) -> &'static str {
        match self {
            Mirror::None => "none",
            Mirror::LeftRight => "left_right",
            Mirror::FrontBack => "front_back",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [Mirror::None, Mirror::LeftRight, Mirror::FrontBack].into_iter().find(|m| m.name() == name)
    }
}

/// `StructureTemplate.transform(pos, mirror, rotation, pivot)` with the pivot at the origin: the
/// position of a template block relative to where the template is placed.
pub fn transform_pos(pos: Pos, mirror: Mirror, rotation: Rotation) -> Pos {
    let [mut x, y, mut z] = pos;
    match mirror {
        Mirror::LeftRight => z = -z,
        Mirror::FrontBack => x = -x,
        Mirror::None => {}
    }
    match rotation {
        Rotation::Counterclockwise90 => [z, y, -x],
        Rotation::Clockwise90 => [-z, y, x],
        Rotation::Clockwise180 => [-x, y, -z],
        Rotation::None => [x, y, z],
    }
}

/// `StructureTemplate.getSize(rotation)`.
pub fn rotate_size(size: Pos, rotation: Rotation) -> Pos {
    match rotation {
        Rotation::Clockwise90 | Rotation::Counterclockwise90 => [size[2], size[1], size[0]],
        _ => size,
    }
}

// --- block states ---------------------------------------------------------------------------------

const HORIZONTAL: [&str; 4] = ["north", "east", "south", "west"];

fn turn_name(name: &str, quarters: i32) -> Option<&'static str> {
    let i = HORIZONTAL.iter().position(|d| *d == name)?;
    Some(HORIZONTAL[(i + quarters.rem_euclid(4) as usize) % 4])
}

fn reflect_name(name: &str, mirror: Mirror) -> Option<&'static str> {
    let reflected = match (mirror, name) {
        (Mirror::LeftRight, "north") => "south",
        (Mirror::LeftRight, "south") => "north",
        (Mirror::FrontBack, "east") => "west",
        (Mirror::FrontBack, "west") => "east",
        (_, "north") => "north",
        (_, "south") => "south",
        (_, "east") => "east",
        (_, "west") => "west",
        _ => return None,
    };
    Some(reflected)
}

/// A direction word of a state's value, through a mapping of the four horizontal ones: `up` and
/// `down` stay.
fn map_word(word: &str, f: &dyn Fn(&str) -> Option<&'static str>) -> String {
    f(word).map_or_else(|| word.to_owned(), str::to_owned)
}

/// `north_east` and the like under a mapping of the words; a rail shape's words are kept in the
/// order vanilla names them (`north_south`, `east_west`, `south_east`...).
fn map_shape(shape: &str, f: &dyn Fn(&str) -> Option<&'static str>) -> Option<String> {
    if let Some(rest) = shape.strip_prefix("ascending_") {
        return Some(format!("ascending_{}", map_word(rest, f)));
    }
    let (a, b) = shape.split_once('_')?;
    let (a, b) = (map_word(a, f), map_word(b, f));
    let vertical = |w: &str| w == "north" || w == "south";
    let (first, second) = if vertical(&a) == vertical(&b) {
        // north_south or east_west: the same axis twice.
        if vertical(&a) { ("north".to_owned(), "south".to_owned()) } else { ("east".to_owned(), "west".to_owned()) }
    } else if vertical(&a) {
        (a, b)
    } else {
        (b, a)
    };
    Some(format!("{first}_{second}"))
}

fn is_rail_shape(shape: &str) -> bool {
    shape.starts_with("ascending_")
        || matches!(
            shape,
            "north_south" | "east_west" | "south_east" | "south_west" | "north_west" | "north_east"
        )
}

/// The four sides of a fence, a wall, a pane, redstone dust, a vine or a multiface block, moved by
/// `f` (each value goes where its side goes).
fn move_sides(state: &mut State, f: &dyn Fn(&str) -> Option<&'static str>) {
    let sides: Vec<(&str, Option<String>)> =
        HORIZONTAL.iter().map(|side| (*side, state.get(side).map(str::to_owned))).collect();
    for (side, value) in sides {
        if let (Some(value), Some(to)) = (value, f(side)) {
            state.set(to, &value);
        }
    }
}

/// `Block.rotate(state, rotation)` by property names.
fn rotate_state(state: &State, rotation: Rotation) -> State {
    let q = rotation.quarters();
    if q == 0 {
        return state.clone();
    }
    let turn = |name: &str| turn_name(name, q);
    let mut out = state.clone();
    if let Some(facing) = state.get("facing") {
        if let Some(to) = turn(facing) {
            out.set("facing", to);
        }
    }
    for key in ["axis", "horizontal_axis"] {
        if let Some(axis) = state.get(key) {
            if q % 2 == 1 && (axis == "x" || axis == "z") {
                out.set(key, if axis == "x" { "z" } else { "x" });
            }
        }
    }
    // Create's `DirectionalAxisKineticBlock`: the axis flips on a quarter turn.
    if state.has("axis_along_first") && q % 2 == 1 {
        out.set("axis_along_first", if state.flag("axis_along_first") { "false" } else { "true" });
    }
    // Signs, banners and heads: 16 steps, 4 to a quarter turn.
    if let Some(r) = state.get("rotation").and_then(|r| r.parse::<i32>().ok()) {
        out.set("rotation", &(r + 4 * q).rem_euclid(16).to_string());
    }
    // `orientation` (the crafter, the jigsaw): a front and a top, each turned if it is horizontal.
    if let Some(orientation) = state.get("orientation") {
        if let Some((front, top)) = orientation.split_once('_') {
            out.set("orientation", &format!("{}_{}", map_word(front, &turn), map_word(top, &turn)));
        }
    }
    if state.has("north") || state.has("east") || state.has("south") || state.has("west") {
        // Clear the old assignment first so a side with no source doesn't keep its value.
        let mut moved = state.clone();
        move_sides(&mut moved, &turn);
        for side in HORIZONTAL {
            if let Some(value) = moved.get(side) {
                out.set(side, value);
            }
        }
    }
    if let Some(shape) = state.get("shape") {
        if is_rail_shape(shape) {
            if let Some(shape) = map_shape(shape, &turn) {
                out.set("shape", &shape);
            }
        }
    }
    out
}

/// `Block.mirror(state, mirror)` by property names.
fn mirror_state(state: &State, mirror: Mirror) -> State {
    if mirror == Mirror::None {
        return state.clone();
    }
    let reflect = |name: &str| reflect_name(name, mirror);
    let mut out = state.clone();
    let facing = state.get("facing").map(str::to_owned);
    if let Some(f) = &facing {
        if let Some(to) = reflect(f) {
            out.set("facing", to);
        }
    }
    if let Some(r) = state.get("rotation").and_then(|r| r.parse::<i32>().ok()) {
        // `Mirror.mirror(rotation, 16)`.
        let j = if r > 8 { r - 16 } else { r };
        let mirrored = match mirror {
            Mirror::FrontBack => (16 - j) % 16,
            _ => (8 - j + 16) % 16,
        };
        out.set("rotation", &mirrored.rem_euclid(16).to_string());
    }
    if let Some(orientation) = state.get("orientation") {
        if let Some((front, top)) = orientation.split_once('_') {
            out.set("orientation", &format!("{}_{}", map_word(front, &reflect), map_word(top, &reflect)));
        }
    }
    if state.has("north") || state.has("east") || state.has("south") || state.has("west") {
        let mut moved = state.clone();
        move_sides(&mut moved, &reflect);
        for side in HORIZONTAL {
            if let Some(value) = moved.get(side) {
                out.set(side, value);
            }
        }
    }
    if let Some(shape) = state.get("shape") {
        if is_rail_shape(shape) {
            if let Some(shape) = map_shape(shape, &reflect) {
                out.set("shape", &shape);
            }
        } else if state.has("half") && state.path().ends_with("_stairs") {
            // `StairBlock.mirror`: along the mirrored axis the facing turns half way round and the
            // corners swap sides.
            let along = match (mirror, facing.as_deref()) {
                (Mirror::LeftRight, Some("north" | "south")) => true,
                (Mirror::FrontBack, Some("east" | "west")) => true,
                _ => false,
            };
            if along {
                let swapped = match shape {
                    "inner_left" => "inner_right",
                    "inner_right" => "inner_left",
                    "outer_left" => "outer_right",
                    "outer_right" => "outer_left",
                    other => other,
                };
                out.set("shape", swapped);
            }
        }
    }
    // `DoorBlock.mirror`: a mirrored door swaps its hinge.
    if state.has("hinge") {
        out.set("hinge", if state.get("hinge") == Some("left") { "right" } else { "left" });
    }
    out
}

/// What `StructureTemplate.placeInWorld` does to a state: `mirror`, then `rotate`.
pub fn transform_state(state: &State, mirror: Mirror, rotation: Rotation) -> State {
    rotate_state(&mirror_state(state, mirror), rotation)
}

// --- SchematicTransformation ----------------------------------------------------------------------

/// `VecHelper.rotate(vec, degrees, Axis.Y)` for a multiple of 90 degrees (exact).
fn rotate_y(x: f64, z: f64, degrees: i32) -> (f64, f64) {
    let (s, c) = match degrees.rem_euclid(360) {
        90 => (1.0, 0.0),
        180 => (0.0, -1.0),
        270 => (-1.0, 0.0),
        _ => (0.0, 1.0),
    };
    (x * c + z * s, z * c - x * s)
}

/// `SchematicTransformation`: where the hologram stands, how it is turned about its centre and
/// which way it is mirrored; and from those the anchor and settings a Schematic stores.
#[derive(Clone, Debug, PartialEq)]
pub struct Transformation {
    /// `target`: the world position the schematic's unturned lower corner is shown at.
    pub target: Pos,
    /// The rotation chase target in degrees: `-(Rotation ordinal * 90)`, turning by 90 as the tools do.
    pub rotation: i32,
    /// `scaleFrontBack` and `scaleLeftRight` chase targets: 1 or -1.
    pub scale_fb: i32,
    pub scale_lr: i32,
    x_origin: f64,
    z_origin: f64,
}

impl Transformation {
    /// `SchematicTransformation.init(anchor, settings, bounds)`: `size` is the template's size
    /// (`SCHEMATIC_BOUNDS`).
    pub fn new(anchor: Pos, rotation: Rotation, mirror: Mirror, size: Pos) -> Self {
        let mut t = Transformation {
            target: [0; 3],
            rotation: -(rotation.quarters() * 90),
            scale_fb: if mirror == Mirror::FrontBack { -1 } else { 1 },
            scale_lr: if mirror == Mirror::LeftRight { -1 } else { 1 },
            x_origin: f64::from(size[0]) / 2.0,
            z_origin: f64::from(size[2]) / 2.0,
        };
        t.target = t.from_anchor(anchor);
        t
    }

    /// `getRotationOffset`: half a block when the two sides differ in parity, so the turn
    /// pivots about a block corner.
    pub fn rotation_offset(&self, ignore_mirrors: bool) -> [f64; 2] {
        if (self.z_origin * 2.0) as i32 % 2 == (self.x_origin * 2.0) as i32 % 2 {
            return [0.0, 0.0];
        }
        let x_greater = self.x_origin > self.z_origin;
        let mut x_in = if x_greater { 0.0 } else { 0.5 };
        let mut z_in = if x_greater { 0.5 } else { 0.0 };
        if !ignore_mirrors {
            x_in *= f64::from(self.scale_fb);
            z_in *= f64::from(self.scale_lr);
        }
        [x_in, z_in]
    }

    fn anchor_offset(&self) -> [i32; 2] {
        let [ox, oz] = self.rotation_offset(false);
        let mut x = 0.5 - self.x_origin - ox;
        let mut z = 0.5 - self.z_origin - oz;
        x *= f64::from(self.scale_fb);
        z *= f64::from(self.scale_lr);
        let (x, z) = rotate_y(x, z, self.rotation);
        [(x + self.x_origin).floor() as i32, (z + self.z_origin).floor() as i32]
    }

    /// `fromAnchor`: the target that puts the anchor where it is.
    pub fn from_anchor(&self, anchor: Pos) -> Pos {
        let [dx, dz] = self.anchor_offset();
        [anchor[0] - dx, anchor[1], anchor[2] - dz]
    }

    /// `getAnchor`: the block the template's origin ends up in.
    pub fn anchor(&self) -> Pos {
        let [dx, dz] = self.anchor_offset();
        [self.target[0] + dx, self.target[1], self.target[2] + dz]
    }

    /// `toSettings`: the rotation and mirror a Schematic stores.
    pub fn settings(&self) -> (Rotation, Mirror) {
        let mut turn = self.rotation;
        let mut mirror_lr = self.scale_lr < 0;
        let mut mirror_fb = self.scale_fb < 0;
        if mirror_lr && mirror_fb {
            mirror_lr = false;
            mirror_fb = false;
            turn += 180;
        }
        let turn = turn.rem_euclid(360);
        let rotation = match turn {
            90 => Rotation::Counterclockwise90,
            180 => Rotation::Clockwise180,
            270 => Rotation::Clockwise90,
            _ => Rotation::None,
        };
        let mirror = if mirror_lr {
            Mirror::LeftRight
        } else if mirror_fb {
            Mirror::FrontBack
        } else {
            Mirror::None
        };
        (rotation, mirror)
    }

    /// `rotate90`.
    pub fn rotate90(&mut self, clockwise: bool) {
        self.rotation += if clockwise { -90 } else { 90 };
    }

    /// `flip(axis)`: the east-west axis flips left-right, the north-south one front-back.
    pub fn flip_x(&mut self) {
        self.scale_lr = -self.scale_lr;
    }

    pub fn flip_z(&mut self) {
        self.scale_fb = -self.scale_fb;
    }

    /// `move(x, y, z)`.
    pub fn shift(&mut self, by: Pos) {
        self.target = [self.target[0] + by[0], self.target[1] + by[1], self.target[2] + by[2]];
    }

    /// Where the hologram's lower corner goes when the player deploys it with `selected` as the
    /// block clicked: `DeployTool.handleRightClick` puts the centre of its footprint there.
    pub fn deploy_target(selected: Pos, size: Pos) -> Pos {
        // `bounds.getCenter()` truncated: half the size, rounded down.
        [selected[0] - size[0] / 2, selected[1], selected[2] - size[2] / 2]
    }

    /// Where the centre of the block at template position `p` (a block of the template) is shown,
    /// by `applyTransformations`: the maths the hologram follows. Used to check the anchor against
    /// the cells a printer places.
    pub fn shown_centre(&self, p: Pos, size: Pos) -> [f64; 3] {
        let (fb, lr) = (self.scale_fb < 0, self.scale_lr < 0);
        let mut x = f64::from(p[0]) + 0.5;
        let mut z = f64::from(p[2]) + 0.5;
        // The renderers of the mirrored structures are placed to fill the unmirrored box again.
        let both = fb && lr;
        if fb && !both {
            x = f64::from(size[0]) - x;
        }
        if lr && !both {
            z = f64::from(size[2]) - z;
        }
        let turn = self.rotation + if both { 180 } else { 0 };
        let [ox, oz] = self.rotation_offset(true);
        let (rx, rz) = rotate_y(x - self.x_origin - ox, z - self.z_origin - oz, turn);
        [
            f64::from(self.target[0]) + self.x_origin + ox + rx,
            f64::from(p[1] + self.target[1]) + 0.5,
            f64::from(self.target[2]) + self.z_origin + oz + rz,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(text: &str) -> State {
        State::parse(text)
    }

    #[test]
    fn positions_turn_as_vanilla_turns_them() {
        // East of the origin, clockwise from above: south.
        assert_eq!(transform_pos([1, 0, 0], Mirror::None, Rotation::Clockwise90), [0, 0, 1]);
        assert_eq!(transform_pos([1, 0, 0], Mirror::None, Rotation::Counterclockwise90), [0, 0, -1]);
        assert_eq!(transform_pos([1, 2, 3], Mirror::None, Rotation::Clockwise180), [-1, 2, -3]);
        assert_eq!(transform_pos([1, 2, 3], Mirror::FrontBack, Rotation::None), [-1, 2, 3]);
        assert_eq!(transform_pos([1, 2, 3], Mirror::LeftRight, Rotation::None), [1, 2, -3]);
        // Mirrored first, then turned.
        assert_eq!(transform_pos([1, 0, 2], Mirror::FrontBack, Rotation::Clockwise90), [-2, 0, -1]);
        assert_eq!(rotate_size([2, 3, 5], Rotation::Clockwise90), [5, 3, 2]);
    }

    #[test]
    fn facings_axes_and_sides_turn_with_the_block() {
        let lever = s("minecraft:lever[face=wall,facing=north]");
        assert_eq!(transform_state(&lever, Mirror::None, Rotation::Clockwise90).get("facing"), Some("east"));
        assert_eq!(transform_state(&lever, Mirror::None, Rotation::Counterclockwise90).get("facing"), Some("west"));
        let piston = s("minecraft:piston[facing=up]");
        assert_eq!(transform_state(&piston, Mirror::None, Rotation::Clockwise90).get("facing"), Some("up"));
        let log = s("minecraft:oak_log[axis=x]");
        assert_eq!(transform_state(&log, Mirror::None, Rotation::Clockwise90).get("axis"), Some("z"));
        assert_eq!(transform_state(&log, Mirror::None, Rotation::Clockwise180).get("axis"), Some("x"));
        let fence = s("minecraft:oak_fence[north=true,east=false,south=false,west=false]");
        let turned = transform_state(&fence, Mirror::None, Rotation::Clockwise90);
        assert_eq!(turned.get("east"), Some("true"));
        assert_eq!(turned.get("north"), Some("false"));
        let wall = s("minecraft:cobblestone_wall[north=tall,east=none,south=low,west=none,up=true]");
        let turned = transform_state(&wall, Mirror::None, Rotation::Clockwise180);
        assert_eq!((turned.get("south"), turned.get("north")), (Some("tall"), Some("low")));
        let sign = s("minecraft:oak_sign[rotation=0]");
        assert_eq!(transform_state(&sign, Mirror::None, Rotation::Clockwise90).get("rotation"), Some("4"));
        assert_eq!(transform_state(&sign, Mirror::None, Rotation::Counterclockwise90).get("rotation"), Some("12"));
    }

    #[test]
    fn mirrors_swap_what_they_reflect() {
        let stairs = s("minecraft:oak_stairs[facing=north,half=bottom,shape=inner_left]");
        let mirrored = transform_state(&stairs, Mirror::LeftRight, Rotation::None);
        assert_eq!(mirrored.get("facing"), Some("south"));
        assert_eq!(mirrored.get("shape"), Some("inner_right"));
        // Across the other axis the stairs stay as they are.
        let same = transform_state(&stairs, Mirror::FrontBack, Rotation::None);
        assert_eq!(same.get("facing"), Some("north"));
        assert_eq!(same.get("shape"), Some("inner_left"));
        let door = s("minecraft:oak_door[facing=east,half=lower,hinge=left,open=false]");
        assert_eq!(transform_state(&door, Mirror::FrontBack, Rotation::None).get("hinge"), Some("right"));
        assert_eq!(transform_state(&door, Mirror::FrontBack, Rotation::None).get("facing"), Some("west"));
        let sign = s("minecraft:oak_sign[rotation=4]");
        // 4 is west: front-back (x negated) makes it east, 12; left-right keeps west.
        assert_eq!(transform_state(&sign, Mirror::FrontBack, Rotation::None).get("rotation"), Some("12"));
        assert_eq!(transform_state(&sign, Mirror::LeftRight, Rotation::None).get("rotation"), Some("4"));
    }

    #[test]
    fn rails_keep_vanilla_s_names() {
        let rail = s("minecraft:rail[shape=north_south]");
        assert_eq!(transform_state(&rail, Mirror::None, Rotation::Clockwise90).get("shape"), Some("east_west"));
        let corner = s("minecraft:rail[shape=south_east]");
        // Clockwise: south -> west, east -> south.
        assert_eq!(transform_state(&corner, Mirror::None, Rotation::Clockwise90).get("shape"), Some("south_west"));
        assert_eq!(transform_state(&corner, Mirror::FrontBack, Rotation::None).get("shape"), Some("south_west"));
        let slope = s("minecraft:powered_rail[shape=ascending_east,powered=true]");
        assert_eq!(transform_state(&slope, Mirror::None, Rotation::Clockwise90).get("shape"), Some("ascending_south"));
        assert_eq!(transform_state(&slope, Mirror::None, Rotation::Clockwise90).get("powered"), Some("true"));
    }

    #[test]
    fn the_crafter_s_orientation_turns() {
        let crafter = s("minecraft:crafter[orientation=north_up,crafting=false]");
        assert_eq!(
            transform_state(&crafter, Mirror::None, Rotation::Clockwise90).get("orientation"),
            Some("east_up")
        );
        let down = s("minecraft:crafter[orientation=down_east]");
        assert_eq!(transform_state(&down, Mirror::None, Rotation::Clockwise90).get("orientation"), Some("down_south"));
    }

    #[test]
    fn rotation_and_mirror_have_names() {
        assert_eq!(Rotation::from_name("clockwise_90"), Some(Rotation::Clockwise90));
        assert_eq!(Rotation::from_name(Rotation::Clockwise180.name()), Some(Rotation::Clockwise180));
        assert_eq!(Rotation::from_name("sideways"), None);
        assert_eq!(Mirror::from_name("left_right"), Some(Mirror::LeftRight));
        assert_eq!(Rotation::Clockwise90.then(Rotation::Clockwise180), Rotation::Counterclockwise90);
    }

    /// The anchor the tools store puts the cells the printer fills (anchor + `transform_pos`) where
    /// the hologram shows them, for every size, turn and mirror.
    #[test]
    fn the_anchor_matches_the_hologram() {
        let rotations = [Rotation::None, Rotation::Clockwise90, Rotation::Clockwise180, Rotation::Counterclockwise90];
        let mirrors = [Mirror::None, Mirror::LeftRight, Mirror::FrontBack];
        for sx in 1..=5 {
            for sz in 1..=5 {
                let size = [sx, 2, sz];
                for rotation in rotations {
                    for mirror in mirrors {
                        let t = Transformation::new([10, 64, -20], rotation, mirror, size);
                        let (rotation_back, mirror_back) = t.settings();
                        assert_eq!((rotation_back, mirror_back), (rotation, mirror), "settings {size:?}");
                        let anchor = t.anchor();
                        assert_eq!(anchor, [10, 64, -20], "anchor {size:?} {rotation:?} {mirror:?}");
                        for x in 0..sx {
                            for z in 0..sz {
                                let at = transform_pos([x, 1, z], mirror_back, rotation_back);
                                let centre = [
                                    f64::from(anchor[0] + at[0]) + 0.5,
                                    f64::from(anchor[1] + at[1]) + 0.5,
                                    f64::from(anchor[2] + at[2]) + 0.5,
                                ];
                                assert_eq!(
                                    t.shown_centre([x, 1, z], size),
                                    centre,
                                    "{size:?} {rotation:?} {mirror:?} block {x},{z}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_tools_keep_the_hologram_about_its_centre() {
        let size = [4, 2, 3];
        let mut t = Transformation::new([0, 70, 0], Rotation::None, Mirror::None, size);
        t.shift([5, 0, 5]);
        let before = t.anchor();
        t.rotate90(true);
        assert_eq!(t.settings().0, Rotation::Clockwise90);
        // The anchor moved so the turn is about the centre, and the cells still agree.
        assert_ne!(t.anchor(), before);
        let (r, m) = t.settings();
        for x in 0..4 {
            for z in 0..3 {
                let at = transform_pos([x, 0, z], m, r);
                let a = t.anchor();
                assert_eq!(
                    t.shown_centre([x, 0, z], size),
                    [f64::from(a[0] + at[0]) + 0.5, f64::from(a[1]) + 0.5, f64::from(a[2] + at[2]) + 0.5]
                );
            }
        }
        t.rotate90(false);
        assert_eq!(t.anchor(), before);
        t.flip_x();
        assert_eq!(t.settings().1, Mirror::LeftRight);
        t.flip_z();
        // Both mirrors are a half turn.
        assert_eq!(t.settings(), (Rotation::Clockwise180, Mirror::None));
        assert_eq!(Transformation::deploy_target([20, 64, 20], [5, 3, 4]), [18, 64, 18]);
    }
}
