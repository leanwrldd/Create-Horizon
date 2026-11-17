//! The Windmill Bearing and the sails: how many sails make how much speed, which blocks count as a
//! sail, how a sail is dyed and how a row of sails is laid. Ported from Create (MIT, the Creators
//! of Create), `mc1.21.1/dev` at 4148fd8: `WindmillBearingBlockEntity` (`getGeneratedSpeed`,
//! `RotationDirection`), `BearingContraption` (`sailBlocks`, `assemble`), `CKinetics`
//! (`minimumWindmillSails`, `windmillSailsPerRPM`), `AllBlocks.WINDMILL_BEARING`
//! (`CStress.setCapacity(512)`), `SailBlock` (`applyDye`, `PlacementHelper`, `getRotatedBlockState`),
//! the `create:windmill_sails` tag and `AllShapes.SAIL`.
//!
//! No game types: the glue is `native/api/src/machines/windmill.rs` and `bearing.rs`.
//!
//! Known differences from Create:
//! - The rotation direction (`ScrollOptionBehaviour`) is toggled with the Wrench on a side of the
//!   bearing (see the README's Settings); a click on its front or back turns it as usual.
//! - The structure turns at the network's speed at the bearing, which is the windmill's own
//!   output unless a faster source drives the network (Create always turns it at its own).
//! - A Copycat Panel counts as a sail by its material in Create (`getSailBlock`); here it doesn't.

use std::collections::HashSet;

use crate::rules::contraption::{Fail, State};
use crate::rules::girder::ordered_by_distance;
use crate::{Axis, Dir, Pos};

pub const BEARING: &str = "create:windmill_bearing";
pub const FRAME: &str = "create:sail_frame";
pub const WHITE_SAIL: &str = "create:white_sail";

/// `CStress.setCapacity(512.0)`: su per RPM.
pub const CAPACITY: f32 = 512.0;
/// `CKinetics.minimumWindmillSails`.
pub const MINIMUM_SAILS: i32 = 8;
/// `CKinetics.windmillSailsPerRPM`.
pub const SAILS_PER_RPM: i32 = 8;
/// The fastest a windmill turns: `Mth.clamp(sails, 1, 16)`.
pub const MAX_SPEED: i32 = 16;
/// The saved key of the rotation direction (`ScrollOptionBehaviour`).
pub const DIRECTION_KEY: &str = "ScrollValue";

/// `DyeColor`'s names, in its order.
pub const COLOURS: [&str; 16] = [
    "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan",
    "purple", "blue", "brown", "green", "red", "black",
];

/// `RotationDirection`: the saved value is its ordinal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rotation {
    Clockwise,
    CounterClockwise,
}

impl Rotation {
    pub fn from_value(value: i32) -> Self {
        if value == 1 { Rotation::CounterClockwise } else { Rotation::Clockwise }
    }

    pub fn value(self) -> i32 {
        match self {
            Rotation::Clockwise => 0,
            Rotation::CounterClockwise => 1,
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            Rotation::Clockwise => Rotation::CounterClockwise,
            Rotation::CounterClockwise => Rotation::Clockwise,
        }
    }

    /// `getAngleSpeedDirection`.
    pub fn sign(self) -> f32 {
        match self {
            Rotation::Clockwise => 1.0,
            Rotation::CounterClockwise => -1.0,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Rotation::Clockwise => "Clockwise",
            Rotation::CounterClockwise => "Counter-Clockwise",
        }
    }
}

/// `WindmillBearingBlockEntity.getGeneratedSpeed` of a running windmill: the sails over
/// `windmillSailsPerRPM`, in 1 to 16, signed by the rotation direction. A windmill that isn't
/// running makes none.
pub fn generated_speed(running: bool, sails: i32, rotation: Rotation) -> f32 {
    if !running {
        return 0.0;
    }
    (sails / SAILS_PER_RPM).clamp(1, MAX_SPEED) as f32 * rotation.sign()
}

/// The `create:windmill_sails` tag: wool and Create's sails and sail frame.
pub fn is_sail(state: &State) -> bool {
    if state.name.starts_with("minecraft:") {
        return state.path().ends_with("_wool");
    }
    state.name.starts_with("create:") && (state.path() == "sail_frame" || state.path().ends_with("_sail"))
}

/// `BearingContraption.sailBlocks` over the blocks a structure holds.
pub fn count_sails<'a>(states: impl IntoIterator<Item = &'a State>) -> i32 {
    states.into_iter().filter(|s| is_sail(s)).count() as i32
}

/// `BearingContraption.assemble`: a windmill with too few sails doesn't assemble.
pub fn check_sails(sails: i32) -> Result<(), Fail> {
    if sails < MINIMUM_SAILS { Err(Fail::NotEnoughSails(sails)) } else { Ok(()) }
}

/// What a Wrench click on the bearing does: a side toggles the rotation direction (the stand-in
/// for the value box on a side, `getMovementModeSlot`), the ends turn it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wrench {
    ToggleDirection,
    Turn,
}

pub fn wrench(facing: Dir, clicked: Dir) -> Wrench {
    if clicked.axis() == facing.axis() { Wrench::Turn } else { Wrench::ToggleDirection }
}

// --- the sails ------------------------------------------------------------------------------------

/// The block of a dyed sail: white is `white_sail`, the rest `<colour>_sail`.
pub fn dyed_sail(colour: &str) -> String {
    format!("create:{colour}_sail")
}

/// `DyeColor.getColor(stack)` for a dye item: its colour.
pub fn dye_colour(item: &str) -> Option<&'static str> {
    let path = item.strip_prefix("minecraft:")?.strip_suffix("_dye")?;
    COLOURS.iter().copied().find(|c| *c == path)
}

fn is_sail_block(state: &State) -> bool {
    state.name.starts_with("create:") && (state.path() == "sail_frame" || state.path().ends_with("_sail"))
}

/// `SailBlock.frame`.
fn is_frame(state: &State) -> bool {
    state.is(FRAME)
}

/// `SailBlock.applyDye`: dyeing the sail at `pos` with `colour` (shears: `None`, back to a frame).
/// `read` is the world; `hit` the click's offset from the block's centre. The changes to make
/// (position, new state), in order.
pub fn apply_dye(
    read: &dyn Fn(Pos) -> Option<State>,
    pos: Pos,
    hit: [f64; 3],
    colour: Option<&str>,
) -> Vec<(Pos, State)> {
    let Some(state) = read(pos) else { return Vec::new() };
    let Some(facing) = state.dir("facing") else { return Vec::new() };
    let new_name = colour.map_or_else(|| FRAME.to_owned(), |c| dyed_sail(c));
    let new_state = State::new(&new_name).with("facing", facing.name());

    // Dye the block itself.
    if state != new_state {
        return vec![(pos, new_state)];
    }

    // Dye one adjacent block, the nearest to the click.
    let others: Vec<Dir> = Dir::ALL.into_iter().filter(|d| d.axis() != facing.axis()).collect();
    for d in ordered_by_distance(hit, &others) {
        let offset = crate::rules::contraption::relative(pos, d, 1);
        let Some(adjacent) = read(offset) else { continue };
        if !is_sail_block(&adjacent) || is_frame(&adjacent) {
            continue;
        }
        if adjacent.dir("facing") != Some(facing) || state == adjacent {
            continue;
        }
        return vec![(offset, new_state)];
    }

    // Dye all the things: every sail joined to this one on the same plane.
    let mut changes = Vec::new();
    let mut frontier: Vec<Pos> = vec![pos];
    let mut visited: HashSet<Pos> = HashSet::new();
    let mut timeout = 100;
    while !frontier.is_empty() {
        timeout -= 1;
        if timeout < 0 {
            break;
        }
        let current = frontier.remove(0);
        visited.insert(current);
        for d in Dir::ALL {
            if d.axis() == facing.axis() {
                continue;
            }
            let offset = crate::rules::contraption::relative(current, d, 1);
            if visited.contains(&offset) {
                continue;
            }
            let Some(adjacent) = read(offset) else { continue };
            if !is_sail_block(&adjacent) || (is_frame(&adjacent) && colour.is_some()) {
                continue;
            }
            if adjacent.dir("facing") != Some(facing) {
                continue;
            }
            if state != adjacent && adjacent != new_state {
                changes.push((offset, new_state.clone()));
            }
            frontier.push(offset);
            visited.insert(offset);
        }
    }
    changes
}

/// `SailBlock.PlacementHelper.getOffset`: where the next sail of a row goes, beside the clicked
/// sail on its own plane, the nearest side to the click that is free; it takes the clicked
/// sail's facing.
pub fn row_offset(
    facing: Dir,
    pos: Pos,
    hit: [f64; 3],
    replaceable: &dyn Fn(Pos) -> bool,
) -> Option<(Pos, Dir)> {
    let others: Vec<Dir> = Dir::ALL
        .into_iter()
        .filter(|d| d.axis() != facing.axis() && replaceable(crate::rules::contraption::relative(pos, *d, 1)))
        .collect();
    ordered_by_distance(hit, &others)
        .first()
        .map(|d| (crate::rules::contraption::relative(pos, *d, 1), facing))
}

/// `SailBlock.getStateForPlacement`: `WrenchableDirectionalBlock` faces the nearest looking
/// direction and the sail turns that round: it faces back toward the player. `nearest` is the
/// direction nearest the look.
pub fn placement_facing(nearest: Dir) -> Dir {
    nearest.opposite()
}

/// `WrenchableDirectionalBlock.getRotatedBlockState`: a quarter turn about the face clicked.
pub fn wrenched_facing(facing: Dir, clicked: Dir) -> Dir {
    if facing.axis() == clicked.axis() { facing } else { facing.clockwise_about(clicked.axis()) }
}

/// Which axis a sail's plate lies across (the plate is perpendicular to its facing).
pub fn plate_axis(facing: Dir) -> Axis {
    facing.axis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn eight_sails_a_rpm_up_to_sixteen_either_way() {
        let r = Rotation::Clockwise;
        // The speed is the sails over eight, at least one, at most sixteen.
        assert_eq!(generated_speed(true, 8, r), 1.0);
        assert_eq!(generated_speed(true, 7, r), 1.0);
        assert_eq!(generated_speed(true, 40, r), 5.0);
        assert_eq!(generated_speed(true, 128, r), 16.0);
        assert_eq!(generated_speed(true, 500, r), 16.0);
        assert_eq!(generated_speed(true, 40, Rotation::CounterClockwise), -5.0);
        // A windmill that isn't running makes nothing.
        assert_eq!(generated_speed(false, 100, r), 0.0);
        assert_eq!(Rotation::from_value(1).toggled(), Rotation::Clockwise);
        assert_eq!(Rotation::from_value(0).value(), 0);
    }

    #[test]
    fn wool_and_every_sail_count_but_other_blocks_dont() {
        for name in ["minecraft:red_wool", "create:sail_frame", "create:white_sail", "create:black_sail"] {
            assert!(is_sail(&State::new(name)), "{name}");
        }
        for name in ["minecraft:red_carpet", "minecraft:stone", "create:andesite_casing", "minecraft:sail"] {
            assert!(!is_sail(&State::new(name)), "{name}");
        }
        let states = [State::new("minecraft:white_wool"), State::new("minecraft:stone"), State::new("create:sail_frame")];
        assert_eq!(count_sails(&states), 2);
    }

    #[test]
    fn a_windmill_needs_eight_sails() {
        assert_eq!(check_sails(7), Err(Fail::NotEnoughSails(7)));
        assert_eq!(check_sails(8), Ok(()));
        assert!(Fail::NotEnoughSails(7).message().contains("minimum of 8"));
    }

    #[test]
    fn a_side_click_toggles_the_direction_and_an_end_turns_it() {
        assert_eq!(wrench(Dir::Up, Dir::North), Wrench::ToggleDirection);
        assert_eq!(wrench(Dir::Up, Dir::Up), Wrench::Turn);
        assert_eq!(wrench(Dir::Up, Dir::Down), Wrench::Turn);
    }

    #[test]
    fn dyes_are_told_by_name() {
        assert_eq!(dye_colour("minecraft:light_blue_dye"), Some("light_blue"));
        assert_eq!(dye_colour("minecraft:bone_meal"), None);
        assert_eq!(dye_colour("create:red_dye"), None);
        assert_eq!(dyed_sail("red"), "create:red_sail");
    }

    fn sails(cells: &[(Pos, &str)]) -> impl Fn(Pos) -> Option<State> {
        let map: HashMap<Pos, State> = cells
            .iter()
            .map(|(p, name)| (*p, State::new(name).with("facing", "up")))
            .collect();
        move |p| Some(map.get(&p).cloned().unwrap_or_else(|| State::new("minecraft:air")))
    }

    #[test]
    fn dyeing_changes_the_block_itself_first() {
        let read = sails(&[([0, 0, 0], "create:white_sail"), ([1, 0, 0], "create:white_sail")]);
        let changes = apply_dye(&read, [0, 0, 0], [0.0; 3], Some("red"));
        assert_eq!(changes, vec![([0, 0, 0], State::new("create:red_sail").with("facing", "up"))]);
        // Shears take the canvas off.
        let changes = apply_dye(&read, [0, 0, 0], [0.0; 3], None);
        assert_eq!(changes[0].1.name, FRAME);
    }

    #[test]
    fn dyeing_again_spreads_to_the_sail_beside_the_click_and_then_to_the_whole_plane() {
        let read = sails(&[
            ([0, 0, 0], "create:red_sail"),
            ([1, 0, 0], "create:white_sail"),
            ([-1, 0, 0], "create:white_sail"),
            ([2, 0, 0], "create:white_sail"),
            ([0, 1, 0], "create:white_sail"), // along the facing: another plane
        ]);
        // The click toward +x: the neighbour that way is dyed, one at a time.
        let changes = apply_dye(&read, [0, 0, 0], [0.4, 0.0, 0.0], Some("red"));
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].0, [1, 0, 0]);
        // With the near neighbours done, the rest of the plane follows at once.
        let read = sails(&[
            ([0, 0, 0], "create:red_sail"),
            ([1, 0, 0], "create:red_sail"),
            ([2, 0, 0], "create:white_sail"),
            ([3, 0, 0], "create:white_sail"),
        ]);
        let changes = apply_dye(&read, [0, 0, 0], [0.4, 0.0, 0.0], Some("red"));
        let at: HashSet<Pos> = changes.iter().map(|(p, _)| *p).collect();
        assert_eq!(at, HashSet::from([[2, 0, 0], [3, 0, 0]]));
        // A frame isn't coloured, and doesn't stop the flood from reaching what's behind it.
        let read = sails(&[
            ([0, 0, 0], "create:red_sail"),
            ([1, 0, 0], "create:sail_frame"),
            ([2, 0, 0], "create:white_sail"),
        ]);
        assert!(apply_dye(&read, [0, 0, 0], [0.4, 0.0, 0.0], Some("red")).is_empty());
    }

    #[test]
    fn a_row_of_sails_extends_toward_the_click() {
        let free = |p: Pos| p != [0, 0, 1];
        // Facing up: the plate's sides are x and z. A click toward +z with that side taken
        // goes to the next nearest.
        let (at, facing) = row_offset(Dir::Up, [0, 0, 0], [0.0, 0.0, 0.45], &free).unwrap();
        assert_eq!(facing, Dir::Up);
        assert_ne!(at, [0, 0, 1]);
        assert_eq!(row_offset(Dir::Up, [0, 0, 0], [0.45, 0.0, 0.0], &|_| true).unwrap().0, [1, 0, 0]);
        // Never along the facing.
        assert!(row_offset(Dir::Up, [0, 0, 0], [0.0, 0.5, 0.0], &|_| true).unwrap().0[1] == 0);
        assert!(row_offset(Dir::Up, [0, 0, 0], [0.0; 3], &|_| false).is_none());
    }

    #[test]
    fn the_wrench_turns_a_sail_about_the_face() {
        assert_eq!(wrenched_facing(Dir::Up, Dir::Up), Dir::Up);
        assert_ne!(wrenched_facing(Dir::Up, Dir::North), Dir::Up);
        assert_eq!(placement_facing(Dir::Down), Dir::Up);
    }
}
