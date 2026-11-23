//! What `CarriageContraption` (content/trains/entity/CarriageContraption.java) works out of the blocks a carriage
//! gathered around a bogey: how many bogeys it holds and where the second is, which way its Train Controls face,
//! which seats are next to them (conductor seats), and whether it is a carriage at all (`assemble`'s verdict).
//!
//! The blocks come from [`crate::rules::contraption::search`] (the bogey's sticky sides are there). The glue
//! reads the world, this sorts out what was found, in the order the blocks were found (`capture` is called for
//! each block as `searchMovedStructure` adds it, and the second bogey is the second one met).
//!
//! Left out: the blaze burner conductors (`ConductorBlockInteractionBehavior`), the portal cutoffs and the sound
//! queue (`ArrivalSoundQueue`).

use super::station::AssemblyError;
use crate::rules::contraption::State;
use crate::{Dir, Pos};
use std::collections::BTreeMap;

/// `AllBlocks.TRAIN_CONTROLS`.
pub const CONTROLS: &str = "create:controls";
/// The two bogey blocks (`AbstractBogeyBlock`).
pub const BOGEYS: [&str; 2] = ["create:small_bogey", "create:large_bogey"];

/// `AllBlockTags.SEATS`: the seat blocks (Create's sixteen cushions).
pub fn is_seat(state: &State) -> bool {
    state.name.starts_with("create:") && state.path().ends_with("_seat")
}

pub fn is_bogey(state: &State) -> bool {
    BOGEYS.contains(&state.name.as_str())
}

/// What `capture` and `assemble` noted about a carriage's blocks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    /// `bogeys`.
    pub bogeys: i32,
    /// `secondBogeyPos`: where the second bogey met stands.
    pub second_bogey: Option<Pos>,
    /// `forwardControls`, `backwardControls` (controls that face along the assembly direction, or against it).
    pub forward_controls: bool,
    pub backward_controls: bool,
    /// `sidewaysControls`.
    pub sideways_controls: bool,
    /// `conductorSeats`: seat position to `(forward, backward)`.
    pub conductor_seats: BTreeMap<Pos, [bool; 2]>,
    /// `getSeats()`: every seat, in the order found.
    pub seats: Vec<Pos>,
}

fn step(dir: Dir) -> [i32; 3] {
    dir.normal()
}

fn offset(pos: Pos, dir: Dir) -> Pos {
    let n = step(dir);
    [pos[0] + n[0], pos[1] + n[1], pos[2] + n[2]]
}

/// `CarriageContraption.inControl(pos, direction)`: the controls next to `pos` on that side face back at it.
pub fn in_control(blocks: &[(Pos, State)], pos: Pos, direction: Dir) -> bool {
    let controls_pos = offset(pos, direction);
    blocks
        .iter()
        .find(|(p, _)| *p == controls_pos)
        .is_some_and(|(_, state)| state.is(CONTROLS) && state.dir("facing") == Some(direction.opposite()))
}

/// The directions along an axis (`Iterate.directionsInAxis`: negative, then positive).
fn along(axis: crate::Axis) -> [Dir; 2] {
    match axis {
        crate::Axis::X => [Dir::West, Dir::East],
        crate::Axis::Y => [Dir::Down, Dir::Up],
        crate::Axis::Z => [Dir::North, Dir::South],
    }
}

/// `capture` for every block, then the end of `assemble`: the counts and the conductor seats.
pub fn summarize(blocks: &[(Pos, State)], assembly_direction: Dir) -> Summary {
    let mut summary = Summary::default();
    for (pos, state) in blocks {
        if is_bogey(state) {
            summary.bogeys += 1;
            if summary.bogeys == 2 {
                summary.second_bogey = Some(*pos);
            }
        }
        if is_seat(state) {
            summary.seats.push(*pos);
        }
        if state.is(CONTROLS) {
            match state.dir("facing") {
                Some(facing) if facing.axis() == assembly_direction.axis() => {
                    if facing == assembly_direction {
                        summary.forward_controls = true;
                    } else {
                        summary.backward_controls = true;
                    }
                }
                _ => summary.sideways_controls = true,
            }
        }
    }
    for seat in summary.seats.clone() {
        for direction in along(assembly_direction.axis()) {
            if in_control(blocks, seat, direction) {
                // `.set(direction != assemblyDirection, true)`: the first slot is forward.
                let slot = usize::from(direction != assembly_direction);
                summary.conductor_seats.entry(seat).or_insert([false, false])[slot] = true;
            }
        }
    }
    summary
}

/// `CarriageContraption.assemble`'s verdict on what was found: `Ok(true)` when it is a carriage, `Ok(false)` for
/// nothing worth moving (one block or no bogey: "nothing attached"), the error for too many bogeys or controls
/// that face across the track.
pub fn verdict(block_count: usize, summary: &Summary) -> Result<bool, AssemblyError> {
    if block_count <= 1 || summary.bogeys == 0 {
        return Ok(false);
    }
    if summary.bogeys > 2 {
        return Err(AssemblyError::TooManyBogeys { count: summary.bogeys as usize });
    }
    if summary.sideways_controls {
        return Err(AssemblyError::SidewaysControls);
    }
    Ok(true)
}

/// The local position of a block (`toLocalPos`): relative to the anchor, which is the first bogey.
pub fn local(pos: Pos, anchor: Pos) -> Pos {
    [pos[0] - anchor[0], pos[1] - anchor[1], pos[2] - anchor[2]]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(text: &str) -> State {
        State::parse(text)
    }

    fn blocks() -> Vec<(Pos, State)> {
        vec![
            ([0, 0, 0], state("create:small_bogey[axis=x]")),
            ([0, 1, 0], state("create:controls[facing=east]")),
            ([0, 1, 1], state("minecraft:stone")),
            ([1, 1, 1], state("create:white_seat")),
            ([2, 0, 0], state("create:small_bogey[axis=x]")),
        ]
    }

    #[test]
    fn it_counts_the_bogeys_and_names_the_second() {
        let s = summarize(&blocks(), Dir::East);
        assert_eq!(s.bogeys, 2);
        assert_eq!(s.second_bogey, Some([2, 0, 0]));
    }

    #[test]
    fn controls_along_the_assembly_direction_are_forward() {
        let s = summarize(&blocks(), Dir::East);
        assert!(s.forward_controls && !s.backward_controls && !s.sideways_controls);
        let s = summarize(&blocks(), Dir::West);
        assert!(s.backward_controls && !s.forward_controls);
    }

    #[test]
    fn controls_across_the_track_are_sideways_and_an_error() {
        let mut blocks = blocks();
        blocks.push(([5, 1, 0], state("create:controls[facing=north]")));
        let s = summarize(&blocks, Dir::East);
        assert!(s.sideways_controls);
        assert_eq!(verdict(blocks.len(), &s), Err(AssemblyError::SidewaysControls));
    }

    #[test]
    fn a_seat_in_front_of_facing_back_controls_is_a_forward_conductor_seat() {
        // `inControl(seat, direction)`: the controls at `seat + direction` face `direction.opposite()`.
        let blocks = vec![
            ([0, 0, 0], state("create:small_bogey[axis=x]")),
            ([1, 1, 0], state("create:white_seat")),
            ([2, 1, 0], state("create:controls[facing=west]")),
            ([0, 1, 0], state("create:controls[facing=east]")),
        ];
        let s = summarize(&blocks, Dir::East);
        // Controls at +x facing west: the seat is in control towards the east (`direction == assemblyDirection`):
        // slot 0 (forward). Controls at -x facing east: towards the west, slot 1 (backward).
        assert_eq!(s.conductor_seats.get(&[1, 1, 0]), Some(&[true, true]));
    }

    #[test]
    fn a_lone_block_or_a_bogeyless_pile_is_nothing_attached() {
        let s = summarize(&[([0, 0, 0], state("create:small_bogey[axis=x]"))], Dir::East);
        assert_eq!(verdict(1, &s), Ok(false));
        let s = summarize(&[([0, 0, 0], state("minecraft:stone")), ([1, 0, 0], state("minecraft:stone"))], Dir::East);
        assert_eq!(verdict(2, &s), Ok(false));
    }

    #[test]
    fn three_bogeys_are_too_many() {
        let mut blocks = blocks();
        blocks.push(([4, 0, 0], state("create:large_bogey[axis=x]")));
        let s = summarize(&blocks, Dir::East);
        assert_eq!(verdict(blocks.len(), &s), Err(AssemblyError::TooManyBogeys { count: 3 }));
    }

    #[test]
    fn a_good_carriage_passes() {
        let s = summarize(&blocks(), Dir::East);
        assert_eq!(verdict(5, &s), Ok(true));
    }

    #[test]
    fn a_bogey_pulls_in_the_blocks_beside_it_along_the_track() {
        use crate::rules::contraption::{Config, World, search};
        struct Row;
        impl World for Row {
            fn state(&self, pos: Pos) -> Option<State> {
                match pos {
                    [0, 0, 0] => Some(State::parse("create:small_bogey[axis=x]")),
                    [1, 0, 0] | [-1, 0, 0] => Some(State::parse("minecraft:stone")),
                    [0, 0, 1] => Some(State::parse("minecraft:stone")),
                    _ => Some(State::new("minecraft:air")),
                }
            }
            fn collision_empty(&self, pos: Pos) -> bool {
                self.state(pos).is_some_and(|s| s.is_air())
            }
        }
        let anchoring = |_: Pos| false;
        let config = Config { forced: None, anchoring: &anchoring, frontier: vec![[0, 0, 0]], preexisting: 0, known: Default::default() };
        let found = search(&Row, &config).expect("search");
        let at: Vec<Pos> = found.blocks.iter().map(|(p, _)| *p).collect();
        assert!(at.contains(&[1, 0, 0]) && at.contains(&[-1, 0, 0]), "{at:?}");
        // Not the block across the track: nothing sticks it (no glue, no slime).
        assert!(!at.contains(&[0, 0, 1]), "{at:?}");
    }
}
