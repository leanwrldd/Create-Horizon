//! The Mechanical Bearing's rules: how fast it turns its structure, when it lets go and how the
//! structure is put down. Ported from Create (MIT, the Creators of Create), `mc1.21.1/dev` at
//! 4148fd8: `MechanicalBearingBlockEntity` (`tick`, `assemble`, `disassemble`,
//! `isNearInitialAngle`, `getAngularSpeed`), `BearingContraption`, `BearingBlock`,
//! `ControlledContraptionEntity.makeStructureTransform` and `KineticBlockEntity.convertToAngular`.
//!
//! No game types: the glue is `native/api/src/machines/bearing.rs`.
//!
//! Known differences from Create:
//! - Only `RotationMode.ROTATE_PLACE` (the scroll value's default): the bearing sets its
//!   structure down at the nearest right angle whenever it stops; "rotate, never place" and
//!   "place only when near the start" are not offered.
//! - A Mechanical Bearing inside a structure turning about its own axis is carried as one rigid
//!   block with what it holds; Create makes a stabilised sub-contraption that keeps its
//!   orientation (`StabilizedContraption`).

use std::collections::HashSet;

use crate::rules::contraption::{self as contraption, Config, Fail, Found, State, Transform, World, relative};
use crate::{Axis, Dir, Pos};

/// `CStress.setImpact(4.0)` (`AllBlocks.MECHANICAL_BEARING`).
pub const IMPACT: f32 = 4.0;

/// `KineticBlockEntity.convertToAngular`: RPM to degrees a tick (`rpm * 360 / 60 / 20`).
pub fn convert_to_angular(rpm: f32) -> f32 {
    rpm * 360.0 / 60.0 / 20.0
}

/// `MechanicalBearingBlockEntity.tick`: the angle after a tick at `rpm`, `angle % 360`.
pub fn advance(angle: f32, rpm: f32) -> f32 {
    (angle + convert_to_angular(rpm)) % 360.0
}

/// `isNearInitialAngle`.
pub fn near_initial_angle(angle: f32) -> bool {
    angle.abs() < 22.5 || angle.abs() > 360.0 - 22.5
}

/// `BearingBlock.hasShaftTowards`: the one shaft end is out of the back.
pub fn shaft_face(facing: Dir) -> Dir {
    facing.opposite()
}

/// `BearingContraption.assemble`: the blocks in front of the bearing at `pos` facing `facing`
/// that hang together. The cell in front is the anchor; the bearing's own cell never joins.
pub fn assemble(world: &dyn World, pos: Pos, facing: Dir) -> Result<(Pos, Found), Fail> {
    let anchor = relative(pos, facing, 1);
    let bearing = pos;
    let anchoring = move |p: Pos| p == bearing;
    let mut frontier = Vec::new();
    // `searchMovedStructure`: the anchor cell starts the frontier unless it is brittle.
    if let Some(state) = world.state(anchor) {
        if !contraption::is_brittle(world, &state) {
            frontier.push(anchor);
        }
    }
    let config = Config {
        forced: None,
        anchoring: &anchoring,
        frontier,
        preexisting: 0,
        known: HashSet::new(),
    };
    let found = contraption::search(world, &config)?;
    Ok((anchor, found))
}

/// `ControlledContraptionEntity.makeStructureTransform`: the structure turned `angle` degrees
/// (rounded to a right angle) about the facing's axis through the anchor cell.
pub fn disassembly(anchor: Pos, facing: Dir, angle: f32) -> Transform {
    Transform::from_angle(anchor, facing.axis(), angle)
}

/// `BearingContraption`'s sail count: blocks in the `create:windmill_sails` tag
/// (`rules::windmill`).
pub fn is_sail(state: &State) -> bool {
    crate::rules::windmill::is_sail(state)
}

/// The turn that takes a model made facing up to face `face`: `(axis, degrees)` right-handed,
/// as the bearing's top is drawn (`BearingRenderer`).
pub fn up_to(face: Dir) -> (Axis, f32) {
    match face {
        Dir::Up => (Axis::Y, 0.0),
        Dir::Down => (Axis::X, 180.0),
        Dir::North => (Axis::X, -90.0),
        Dir::South => (Axis::X, 90.0),
        Dir::East => (Axis::Z, -90.0),
        Dir::West => (Axis::Z, 90.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::contraption::rotate_pos;
    use std::collections::HashMap;

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
    fn sixteen_rpm_is_four_point_eight_degrees_a_tick() {
        assert!((convert_to_angular(16.0) - 4.8).abs() < 1e-6);
        assert!((advance(359.0, 16.0) - 3.8).abs() < 1e-4);
        assert!(advance(0.0, -16.0) < 0.0);
        assert!(near_initial_angle(10.0) && near_initial_angle(-10.0) && near_initial_angle(350.0));
        assert!(!near_initial_angle(90.0));
    }

    #[test]
    fn the_bearing_takes_what_hangs_on_the_block_in_front_of_it() {
        let mut grid = Grid(HashMap::new());
        grid.0.insert([0, 0, 0], State::parse("create:mechanical_bearing[facing=up]"));
        grid.0.insert([0, 1, 0], State::new("minecraft:slime_block"));
        grid.0.insert([1, 1, 0], State::new("minecraft:stone"));
        grid.0.insert([5, 1, 0], State::new("minecraft:stone"));
        let (anchor, found) = assemble(&grid, [0, 0, 0], Dir::Up).expect("assembles");
        assert_eq!(anchor, [0, 1, 0]);
        let positions: Vec<Pos> = found.blocks.iter().map(|(p, _)| *p).collect();
        // The bearing itself isn't taken; the slime drags the stone beside it.
        assert!(positions.contains(&[0, 1, 0]) && positions.contains(&[1, 1, 0]));
        assert!(!positions.contains(&[0, 0, 0]) && !positions.contains(&[5, 1, 0]));
    }

    #[test]
    fn a_lone_block_is_a_structure_and_air_is_not() {
        let mut grid = Grid(HashMap::new());
        grid.0.insert([0, 1, 0], State::new("minecraft:stone"));
        assert_eq!(assemble(&grid, [0, 0, 0], Dir::Up).unwrap().1.blocks.len(), 1);
        grid.0.clear();
        assert!(assemble(&grid, [0, 0, 0], Dir::Up).unwrap().1.blocks.is_empty());
    }

    #[test]
    fn a_turned_structure_lands_on_the_right_cells() {
        // A bearing facing up, anchor above it. A quarter turn counterclockwise from above
        // takes +x to -z.
        let t = disassembly([0, 1, 0], Dir::Up, 92.0);
        assert_eq!(t.apply([2, 0, 0]), [0, 1, -2]);
        let t = disassembly([0, 1, 0], Dir::Up, 178.0);
        assert_eq!(t.apply([2, 0, 3]), [-2, 1, -3]);
        // A bearing on the side turns about x: the block above the anchor goes to +z.
        let t = disassembly([5, 5, 5], Dir::East, 90.0);
        assert_eq!(t.apply([0, 1, 0]), [5, 5, 6]);
        assert_eq!(rotate_pos([0, 1, 0], Axis::X, 1), [0, 0, 1]);
    }

    #[test]
    fn model_turns_put_up_where_asked() {
        for face in Dir::ALL {
            let (axis, degrees) = up_to(face);
            let turned = rotate_pos([0, 1, 0], axis, (degrees / 90.0).round() as i32);
            assert_eq!(turned, face.normal(), "{face:?}");
        }
    }

    #[test]
    fn sails_are_wool_and_create_sails() {
        assert!(is_sail(&State::new("minecraft:white_wool")));
        assert!(is_sail(&State::new("create:white_sail")));
        assert!(!is_sail(&State::new("minecraft:stone")));
    }
}
