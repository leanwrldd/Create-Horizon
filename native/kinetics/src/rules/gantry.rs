//! The Gantry Shaft and the Gantry Carriage's rules: how fast the carriage moves, where it may
//! go, how the shafts join into parts and take redstone together, how a carriage is turned and
//! when it breaks, and what a gantry takes along. Ported from Create (MIT, the Creators of
//! Create), `mc1.21.1/dev` at 4148fd8: `GantryShaftBlock` (`updateShape`, `neighborChanged`,
//! `shouldBePowered`, `getStateForPlacement`), `GantryShaftBlockEntity` (`canAssembleOn`,
//! `getPinionMovementSpeed`, `checkAttachedCarriageBlocks`), `GantryCarriageBlock` (`canSurvive`,
//! `cycleAxisIfNecessary`, `getStateForPlacement`), `GantryCarriageBlockEntity`
//! (`getGantryPinionModifier`, `tryAssemble`, `shouldAssemble`), `GantryContraption`,
//! `GantryContraptionEntity.checkPinionShaft` and `Contraption.moveGantryPinion` /
//! `moveGantryShaft`.
//!
//! No game types: the glue is `native/api/src/machines/gantry.rs`.
//!
//! Known differences from Create:
//! - Like every contraption here, nothing collides with a moving gantry (no one rides it) and it
//!   is always "move and place". The carriage's wheels (`GANTRY_COGS`) aren't drawn.
//! - The structure is kept by the gantry shaft the carriage started on (Create keeps it in an
//!   entity): that shaft can't be moved, and one shaft runs one carriage at a time. It stops where
//!   the shaft says it can't go on, a block is in its way, the shaft behind it is gone or
//!   powered, or the rotation stops.
//! - The shaft placement assist (`PoleHelper`) and Sequenced Gearshift limits are not here.
//! - Powering (`neighborChanged`) and a carriage breaking without its shaft (`canSurvive`) are
//!   polled from the shaft's and the carriage's ticks, not driven by block updates.

use std::collections::HashSet;

use crate::rules::contraption::{Config, Fail, Found, State, World, relative, search};
use crate::rules::piston::rotation_axis;
use crate::{Axis, Dir, Pos};

pub const SHAFT: &str = "create:gantry_shaft";
pub const CARRIAGE: &str = "create:gantry_carriage";

/// `GantryShaftBlockEntity.getPinionMovementSpeed`: the carriage's blocks a tick along the shaft's
/// facing, `convertToLinear(-speed)` (`speed / 512`) kept within +-0.49.
pub fn pinion_speed(rpm: f32) -> f32 {
    (-rpm / 512.0).clamp(-0.49, 0.49)
}

/// `GantryShaftBlock.Part`: `START` is the end the shaft's facing points away from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Start,
    Middle,
    End,
    Single,
}

impl Part {
    pub const fn name(self) -> &'static str {
        match self {
            Part::Start => "start",
            Part::Middle => "middle",
            Part::End => "end",
            Part::Single => "single",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [Part::Start, Part::Middle, Part::End, Part::Single].into_iter().find(|p| p.name() == name)
    }
}

/// `GantryShaftBlockEntity.canAssembleOn`: whether a carriage may travel on this shaft at
/// `pinion_speed` (an unpowered shaft only): a start only forward, an end only back, a middle
/// either way, a single shaft not at all.
pub fn can_assemble_on(powered: bool, part: Part, pinion_speed: f32) -> bool {
    if powered {
        return false;
    }
    match part {
        Part::End => pinion_speed < 0.0,
        Part::Middle => pinion_speed != 0.0,
        Part::Start => pinion_speed > 0.0,
        Part::Single => false,
    }
}

/// `GantryCarriageBlockEntity.getGantryPinionModifier`: how the carriage and a powered shaft turn
/// against each other (1 or -1).
pub fn pinion_modifier(shaft: Dir, pinion: Dir) -> f32 {
    let step = shaft.sign() as f32;
    let flipped = match shaft.axis() {
        Axis::Y => matches!(pinion, Dir::North | Dir::East),
        Axis::X => matches!(pinion, Dir::Down | Dir::South),
        Axis::Z => matches!(pinion, Dir::Up | Dir::West),
    };
    if flipped { -step } else { step }
}

/// `GantryShaftBlock.updateShape`: the part after the neighbour on `towards` changed; `connect` is
/// whether that neighbour is a gantry shaft facing the same way.
pub fn update_part(part: Part, facing: Dir, towards: Dir, connect: bool) -> Part {
    if towards.axis() != facing.axis() {
        return part;
    }
    if towards.sign() == facing.sign() {
        match (connect, part) {
            (true, Part::End) => Part::Middle,
            (true, Part::Single) => Part::Start,
            (false, Part::Middle) => Part::End,
            (false, Part::Start) => Part::Single,
            _ => part,
        }
    } else {
        match (connect, part) {
            (true, Part::Start) => Part::Middle,
            (true, Part::Single) => Part::End,
            (false, Part::Middle) => Part::Start,
            (false, Part::End) => Part::Single,
            _ => part,
        }
    }
}

/// The part a shaft has from what is on each side along its axis (`updateShape` of both).
pub fn part_from_neighbours(_facing: Dir, front: bool, back: bool) -> Part {
    match (front, back) {
        (true, true) => Part::Middle,
        (true, false) => Part::Start,
        (false, true) => Part::End,
        (false, false) => Part::Single,
    }
}

/// A gantry shaft as the powering rules read one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShaftState {
    pub facing: Dir,
    pub powered: bool,
}

/// `GantryShaftBlock.shouldBePowered`: a signal on the shaft, or a powered shaft of its line
/// beside it.
pub fn should_be_powered(
    state: ShaftState,
    pos: Pos,
    read: &dyn Fn(Pos) -> Option<ShaftState>,
    has_signal: &dyn Fn(Pos) -> bool,
) -> bool {
    let mut power = has_signal(pos);
    for d in state.facing.axis_dirs() {
        if let Some(neighbour) = read(relative(pos, d, 1)) {
            if neighbour.facing == state.facing {
                power |= neighbour.powered;
            }
        }
    }
    power
}

/// `GantryShaftBlock.neighborChanged`: the shafts whose `powered` changes (position, new value)
/// when something happens to the shaft at `pos`. A line of shafts takes power together: a signal
/// on any shaft powers the whole line, and it stays powered while any shaft of it has a signal.
pub fn power_changes(
    state: ShaftState,
    pos: Pos,
    read: &dyn Fn(Pos) -> Option<ShaftState>,
    has_signal: &dyn Fn(Pos) -> bool,
) -> Vec<(Pos, bool)> {
    let previously = state.powered;
    let should = has_signal(pos);
    if !previously && !should && should_be_powered(state, pos, read, has_signal) {
        return vec![(pos, true)];
    }
    if previously == should {
        return Vec::new();
    }
    let mut changes = Vec::new();
    for d in state.facing.axis_dirs() {
        let mut current = relative(pos, d, 1);
        loop {
            let Some(shaft) = read(current) else { break };
            if shaft.facing != state.facing {
                break;
            }
            if !should && shaft.powered && has_signal(current) {
                // Another shaft of the line is still powered by a signal of its own.
                return Vec::new();
            }
            if shaft.powered == should {
                break;
            }
            changes.push((current, should));
            current = relative(current, d, 1);
        }
    }
    changes.push((pos, should));
    changes
}

/// `GantryShaftBlock.getStateForPlacement`: the facing. `base` is the one the look gives
/// (`DirectionalKineticBlock`), `behind` the facing of the gantry shaft at the new block's back
/// if there is one, `clicked_on` that of the block clicked.
pub fn shaft_placement(base: Dir, sneaking: bool, behind: Option<Dir>, clicked_on: Option<Dir>) -> Dir {
    let neighbour = behind.or(clicked_on);
    match neighbour {
        Some(facing) if facing.axis() == base.axis() => {
            if sneaking { facing.opposite() } else { facing }
        }
        _ => base,
    }
}

// --- the carriage ----------------------------------------------------------------------------------------

/// `GantryCarriageBlock.canSurvive`: the shaft behind it runs across its facing.
pub fn carriage_can_survive(facing: Dir, shaft_behind: Option<Dir>) -> bool {
    shaft_behind.is_some_and(|shaft| shaft.axis() != facing.axis())
}

/// `GantryCarriageBlock.getValidGantryShaftAxis`: the axis a shaft may run along, the one that is
/// neither the carriage's rotation axis nor its facing's.
pub fn valid_shaft_axis(facing: Dir, along_first: bool) -> Axis {
    let rotation = rotation_axis(facing, along_first);
    Axis::ALL
        .into_iter()
        .find(|a| *a != rotation && *a != facing.axis())
        .unwrap_or(Axis::Y)
}

/// `GantryCarriageBlock.cycleAxisIfNecessary`: whether `axis_along_first` flips so the carriage
/// takes the shaft on `towards` (`shaft`: its facing).
pub fn needs_axis_flip(facing: Dir, along_first: bool, towards: Dir, shaft: Option<Dir>) -> bool {
    let Some(shaft) = shaft else { return false };
    if shaft.axis() == towards.axis() {
        return false;
    }
    valid_shaft_axis(facing, along_first) != shaft.axis()
}

/// `GantryCarriageBlock.getStateForPlacement`: `(facing, axis_along_first)`. The carriage faces
/// the face clicked; `horizontal` is the player's horizontal direction and `shaft_behind` the
/// gantry shaft's facing behind the new block.
pub fn carriage_placement(clicked: Dir, horizontal: Dir, shaft_behind: Option<Dir>) -> (Dir, bool) {
    // `DirectionalAxisKineticBlock.getStateForPlacement` without a neighbour it would prefer.
    let along_first = if clicked.axis() == Axis::Y {
        horizontal.axis() == Axis::X
    } else {
        clicked.axis() == Axis::Z
    };
    let flip = needs_axis_flip(clicked, along_first, clicked.opposite(), shaft_behind);
    (clicked, along_first ^ flip)
}

fn axis_dirs(axis: Axis) -> [Dir; 2] {
    match axis {
        Axis::X => [Dir::West, Dir::East],
        Axis::Y => [Dir::Down, Dir::Up],
        Axis::Z => [Dir::North, Dir::South],
    }
}

// --- what a gantry takes along ---------------------------------------------------------------------------

/// What `Contraption.moveGantryPinion` and `moveGantryShaft` add to the frontier for the block at
/// `pos`: a carriage takes the block it faces and the shafts along its rotation axis (a second
/// rail), a shaft the rest of its line and the carriages facing out of it.
pub fn targets(world: &dyn World, pos: Pos, state: &State) -> Vec<Pos> {
    let mut out = Vec::new();
    if state.is(CARRIAGE) {
        let (Some(facing), along_first) = (state.dir("facing"), state.flag("axis_along_first")) else {
            return out;
        };
        out.push(relative(pos, facing, 1));
        let rotation = rotation_axis(facing, along_first);
        for d in axis_dirs(rotation) {
            let offset = relative(pos, d, 1);
            if let Some(other) = world.state(offset) {
                if other.is(SHAFT) && other.dir("facing").is_some_and(|f| f.axis() == d.axis()) {
                    out.push(offset);
                }
            }
        }
    } else if state.is(SHAFT) {
        let Some(facing) = state.dir("facing") else { return out };
        for d in Dir::ALL {
            let offset = relative(pos, d, 1);
            let Some(other) = world.state(offset) else { continue };
            if d.axis() == facing.axis() && other.is(SHAFT) && other.dir("facing") == Some(facing) {
                out.push(offset);
            } else if other.is(CARRIAGE) && other.dir("facing") == Some(d) {
                out.push(offset);
            }
        }
    }
    out
}

/// `GantryContraption.assemble`: the blocks the carriage at `pos` (facing `facing`) takes. The
/// shaft behind it is the anchor and never joins.
pub fn assemble(world: &dyn World, pos: Pos, facing: Dir) -> Result<Found, Fail> {
    let shaft = relative(pos, facing, -1);
    let anchoring = move |p: Pos| p == shaft;
    let config = Config {
        forced: None,
        anchoring: &anchoring,
        frontier: vec![pos],
        preexisting: 0,
        known: HashSet::new(),
    };
    search(world, &config)
}

// --- the movement ----------------------------------------------------------------------------------------

/// Where the structure is along the shaft `now` ticks, from a snapshot at `time` (blocks from the
/// start cell, along the shaft's facing), moving `speed` blocks a tick.
pub fn offset_at(offset: f32, time: i64, speed: f32, now: f64) -> f32 {
    offset + speed * (now - time as f64) as f32
}

/// `StructureTransform(BlockPos.containing(anchorVec + .5))`: the cell the structure is set down
/// in, the nearest.
pub fn grid_offset(offset: f32) -> i32 {
    (offset + 0.5).floor() as i32
}

/// The cell whose shaft has to let the carriage on this tick (`checkPinionShaft`'s crossing
/// test): going forward once the step takes it past the centre of its cell, back once it is at or
/// below the centre and the step doesn't rise above it. None when no shaft is asked.
pub fn shaft_to_check(offset: f32, speed: f32) -> Option<i32> {
    if speed == 0.0 {
        return None;
    }
    let cell = grid_offset(offset);
    let next = offset + speed;
    let asked = if speed > 0.0 { next > cell as f32 } else { next <= cell as f32 };
    asked.then_some(cell)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn the_carriage_moves_a_five_hundred_and_twelfth_of_the_rpm_a_tick_up_to_half_a_block() {
        assert!((pinion_speed(-16.0) - 16.0 / 512.0).abs() < 1e-6);
        assert!((pinion_speed(16.0) + 16.0 / 512.0).abs() < 1e-6);
        assert_eq!(pinion_speed(256.0), -0.49);
        assert_eq!(pinion_speed(-1000.0), 0.49);
        assert_eq!(pinion_speed(0.0), 0.0);
    }

    #[test]
    fn a_carriage_travels_only_where_the_part_lets_it() {
        // A start goes forward, an end back, a middle either way, a lone shaft nowhere.
        assert!(can_assemble_on(false, Part::Start, 0.1) && !can_assemble_on(false, Part::Start, -0.1));
        assert!(can_assemble_on(false, Part::End, -0.1) && !can_assemble_on(false, Part::End, 0.1));
        assert!(can_assemble_on(false, Part::Middle, 0.1) && can_assemble_on(false, Part::Middle, -0.1));
        assert!(!can_assemble_on(false, Part::Middle, 0.0));
        assert!(!can_assemble_on(false, Part::Single, 0.1));
        // Redstone stops it.
        assert!(!can_assemble_on(true, Part::Middle, 0.1));
    }

    #[test]
    fn shafts_join_into_parts() {
        let facing = Dir::East;
        // A lone shaft takes a neighbour ahead (it is a start), another behind (an end), then both.
        assert_eq!(update_part(Part::Single, facing, Dir::East, true), Part::Start);
        assert_eq!(update_part(Part::Single, facing, Dir::West, true), Part::End);
        assert_eq!(update_part(Part::Start, facing, Dir::West, true), Part::Middle);
        assert_eq!(update_part(Part::End, facing, Dir::East, true), Part::Middle);
        // And loses them again.
        assert_eq!(update_part(Part::Middle, facing, Dir::East, false), Part::End);
        assert_eq!(update_part(Part::Middle, facing, Dir::West, false), Part::Start);
        assert_eq!(update_part(Part::Start, facing, Dir::East, false), Part::Single);
        assert_eq!(update_part(Part::End, facing, Dir::West, false), Part::Single);
        // A neighbour across the axis changes nothing.
        assert_eq!(update_part(Part::Single, facing, Dir::Up, true), Part::Single);
        // Both together give the part outright.
        assert_eq!(part_from_neighbours(facing, true, true), Part::Middle);
        assert_eq!(part_from_neighbours(facing, false, true), Part::End);
        assert_eq!(Part::from_name("middle"), Some(Part::Middle));
    }

    #[test]
    fn the_pinion_turns_one_way_or_the_other_by_how_it_sits() {
        assert_eq!(pinion_modifier(Dir::Up, Dir::South), 1.0);
        assert_eq!(pinion_modifier(Dir::Up, Dir::North), -1.0);
        assert_eq!(pinion_modifier(Dir::Down, Dir::North), 1.0);
        assert_eq!(pinion_modifier(Dir::East, Dir::Down), -1.0);
        assert_eq!(pinion_modifier(Dir::West, Dir::Down), 1.0);
        assert_eq!(pinion_modifier(Dir::South, Dir::Up), -1.0);
        assert_eq!(pinion_modifier(Dir::South, Dir::Down), 1.0);
    }

    fn line(cells: &[(Pos, bool)]) -> impl Fn(Pos) -> Option<ShaftState> {
        let map: HashMap<Pos, ShaftState> = cells
            .iter()
            .map(|(p, powered)| (*p, ShaftState { facing: Dir::East, powered: *powered }))
            .collect();
        move |p| map.get(&p).copied()
    }

    #[test]
    fn a_signal_on_one_shaft_powers_the_whole_line() {
        let read = line(&[([0, 0, 0], false), ([1, 0, 0], false), ([2, 0, 0], false), ([3, 0, 0], false)]);
        let signal = |p: Pos| p == [1, 0, 0];
        let state = ShaftState { facing: Dir::East, powered: false };
        let mut changes = power_changes(state, [1, 0, 0], &read, &signal);
        changes.sort();
        assert_eq!(
            changes,
            vec![([0, 0, 0], true), ([1, 0, 0], true), ([2, 0, 0], true), ([3, 0, 0], true)]
        );
        // A shaft the line doesn't reach (another facing, or a gap) isn't touched.
        let gap = line(&[([0, 0, 0], false), ([1, 0, 0], false), ([3, 0, 0], false)]);
        let mut changes = power_changes(state, [1, 0, 0], &gap, &signal);
        changes.sort();
        assert_eq!(changes, vec![([0, 0, 0], true), ([1, 0, 0], true)]);
    }

    #[test]
    fn the_line_stays_powered_while_any_shaft_has_a_signal() {
        let read = line(&[([0, 0, 0], true), ([1, 0, 0], true), ([2, 0, 0], true)]);
        let state = ShaftState { facing: Dir::East, powered: true };
        // The signal at 0 goes, but 2 has its own.
        assert!(power_changes(state, [0, 0, 0], &read, &|p| p == [2, 0, 0]).is_empty());
        // None left: the line goes quiet.
        let mut changes = power_changes(state, [0, 0, 0], &read, &|_| false);
        changes.sort();
        assert_eq!(changes, vec![([0, 0, 0], false), ([1, 0, 0], false), ([2, 0, 0], false)]);
        // An unpowered shaft beside a powered one with no signal of its own is powered by it.
        let read = line(&[([0, 0, 0], true), ([1, 0, 0], false)]);
        let quiet = ShaftState { facing: Dir::East, powered: false };
        assert_eq!(power_changes(quiet, [1, 0, 0], &read, &|_| false), vec![([1, 0, 0], true)]);
    }

    #[test]
    fn placement_continues_a_line_and_sneaking_turns_the_new_shaft_round() {
        assert_eq!(shaft_placement(Dir::Up, false, None, None), Dir::Up);
        // Against a shaft along the same axis it takes its facing; sneaking, the opposite.
        assert_eq!(shaft_placement(Dir::Down, false, Some(Dir::Up), None), Dir::Up);
        assert_eq!(shaft_placement(Dir::Down, true, None, Some(Dir::Up)), Dir::Down);
        // One across the axis is left alone.
        assert_eq!(shaft_placement(Dir::Down, false, Some(Dir::East), None), Dir::Down);
    }

    #[test]
    fn a_carriage_needs_a_shaft_behind_it_that_runs_across_its_facing() {
        assert!(carriage_can_survive(Dir::North, Some(Dir::Up)));
        assert!(carriage_can_survive(Dir::North, Some(Dir::East)));
        assert!(!carriage_can_survive(Dir::North, Some(Dir::South)));
        assert!(!carriage_can_survive(Dir::North, None));
    }

    #[test]
    fn the_carriage_takes_the_axis_its_shaft_runs_along() {
        // Facing north, rotation along X (along_first = true), so the valid shaft axis is Y.
        assert_eq!(valid_shaft_axis(Dir::North, true), Axis::Y);
        assert_eq!(valid_shaft_axis(Dir::North, false), Axis::X);
        // A vertical shaft behind a carriage that wants X: it flips.
        assert!(needs_axis_flip(Dir::North, false, Dir::South, Some(Dir::Up)));
        assert!(!needs_axis_flip(Dir::North, true, Dir::South, Some(Dir::Up)));
        assert!(!needs_axis_flip(Dir::North, false, Dir::South, None));
        // A shaft along the facing isn't a shaft for it.
        assert!(!needs_axis_flip(Dir::North, false, Dir::South, Some(Dir::South)));
        // Placed on a vertical shaft facing the player.
        let (facing, along_first) = carriage_placement(Dir::North, Dir::North, Some(Dir::Up));
        assert_eq!(facing, Dir::North);
        assert_eq!(valid_shaft_axis(facing, along_first), Axis::Y);
    }

    #[test]
    fn the_structure_settles_on_the_nearest_cell() {
        assert_eq!(grid_offset(0.2), 0);
        assert_eq!(grid_offset(0.5), 1);
        assert_eq!(grid_offset(-0.4), 0);
        assert_eq!(grid_offset(-0.6), -1);
        assert!((offset_at(2.0, 100, 0.25, 104.0) - 3.0).abs() < 1e-6);
    }

    #[test]
    fn the_shaft_is_asked_when_the_step_goes_past_the_centre_of_the_cell() {
        // Forward: only the step that takes it past the centre.
        assert_eq!(shaft_to_check(0.6, 0.3), None);
        assert_eq!(shaft_to_check(0.8, 0.3), Some(1));
        assert_eq!(shaft_to_check(3.0, 0.1), Some(3));
        // Back: from the centre of a cell downward, and anywhere below it.
        assert_eq!(shaft_to_check(3.0, -0.1), Some(3));
        assert_eq!(shaft_to_check(2.8, -0.1), Some(3));
        assert_eq!(shaft_to_check(3.2, -0.1), None);
        assert_eq!(shaft_to_check(3.2, -0.3), Some(3));
        assert_eq!(shaft_to_check(1.0, 0.0), None);
    }

    struct Grid(HashMap<Pos, State>);

    impl World for Grid {
        fn state(&self, pos: Pos) -> Option<State> {
            Some(self.0.get(&pos).cloned().unwrap_or_else(|| State::new("minecraft:air")))
        }
        fn collision_empty(&self, pos: Pos) -> bool {
            self.0.get(&pos).is_none_or(State::is_air)
        }
    }

    #[test]
    fn a_gantry_takes_the_carriage_and_what_it_faces_but_not_its_shaft() {
        let mut grid = Grid(HashMap::new());
        // The carriage faces east, its shaft is behind it (west) and runs along Y.
        grid.0.insert([0, 0, 0], State::parse("create:gantry_shaft[facing=up,part=single,powered=false]"));
        grid.0.insert([1, 0, 0], State::parse("create:gantry_carriage[facing=east,axis_along_first=false]"));
        grid.0.insert([2, 0, 0], State::new("minecraft:stone"));
        grid.0.insert([3, 0, 0], State::new("minecraft:stone"));
        grid.0.insert([1, 1, 0], State::new("minecraft:dirt"));
        let found = assemble(&grid, [1, 0, 0], Dir::East).expect("found");
        let at: HashSet<Pos> = found.blocks.iter().map(|(p, _)| *p).collect();
        assert!(at.contains(&[1, 0, 0]) && at.contains(&[2, 0, 0]));
        // Stone beside stone doesn't stick; the shaft is the anchor.
        assert!(!at.contains(&[3, 0, 0]) && !at.contains(&[0, 0, 0]) && !at.contains(&[1, 1, 0]));
    }

    #[test]
    fn a_shaft_in_a_structure_takes_its_line_and_its_carriages() {
        let mut grid = Grid(HashMap::new());
        grid.0.insert([0, 0, 0], State::parse("create:gantry_shaft[facing=up,part=start,powered=false]"));
        grid.0.insert([0, 1, 0], State::parse("create:gantry_shaft[facing=up,part=end,powered=false]"));
        grid.0.insert([1, 0, 0], State::parse("create:gantry_carriage[facing=east,axis_along_first=false]"));
        let state = grid.state([0, 0, 0]).unwrap();
        let mut got = targets(&grid, [0, 0, 0], &state);
        got.sort();
        assert_eq!(got, vec![[0, 1, 0], [1, 0, 0]]);
    }
}
