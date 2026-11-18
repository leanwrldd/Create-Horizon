//! The Rope Pulley's rules: how long its rope may be, how fast it lowers and raises, which blocks
//! hang from the rope, where it stops and how the rope and magnet are drawn. Ported from Create
//! (MIT, the Creators of Create), `mc1.21.1/dev` at 4148fd8: `PulleyBlockEntity` (`assemble`,
//! `disassemble`, `getExtensionRange`, `toPosition`, `visitNewPosition`), `PulleyContraption`
//! (`isAnchoringBlockAt`), `LinearActuatorBlockEntity` (`getMovementSpeed`, `getGridOffset`),
//! `HorizontalAxisKineticBlock.getStateForPlacement`, `AbstractPulleyRenderer` and `CKinetics`
//! (`maxRopeLength`).
//!
//! No game types: the glue is `native/api/src/machines/pulley.rs`.
//!
//! Known differences from Create:
//! - Only `MovementMode.MOVE_PLACE`, as the piston: the structure is set down where the pulley
//!   stops.
//! - No mirrored pulleys (`mirrorParent`, several pulleys on one structure): each runs its own.
//! - The rope isn't waterlogged where it passes through water (no waterlogging, as elsewhere), and
//!   a world's lowest block is taken as -64 in the Overworld and 0 elsewhere.
//! - The coil's texture doesn't scroll.

use std::collections::HashSet;

use crate::rules::contraption::{Config, Fail, Found, State, World, is_brittle, movement_necessary, search};
use crate::rules::piston::convert_to_linear;
use crate::{Axis, Dir, Pos};

pub const PULLEY: &str = "create:rope_pulley";
pub const ROPE: &str = "create:rope";
pub const MAGNET: &str = "create:pulley_magnet";
/// `CKinetics.maxRopeLength`.
pub const MAX_ROPE_LENGTH: i32 = 384;
/// `CStress.setImpact(4.0)` (`AllBlocks.ROPE_PULLEY`).
pub const IMPACT: f32 = 4.0;

/// `Level.getMinBuildHeight`: the Overworld's -64, the other dimensions' 0.
pub fn min_build_height(dimension: &str) -> i32 {
    if dimension == "minecraft:overworld" { -64 } else { 0 }
}

/// `PulleyBlockEntity.getExtensionRange`: the rope's longest reach from a pulley at height `y`.
pub fn extension_range(y: i32, min_y: i32) -> i32 {
    MAX_ROPE_LENGTH.min((y - 1) - min_y).max(0)
}

/// `getMovementSpeed`: blocks a tick the rope pays out (down) at `rpm`, within +-0.49.
pub fn speed(rpm: f32) -> f32 {
    convert_to_linear(rpm).clamp(-0.49, 0.49)
}

/// How far the rope is out `now` ticks, from a snapshot at `time`, kept within `0..=range`.
pub fn offset_at(offset: f32, time: i64, rpm: f32, now: f64, range: i32) -> f32 {
    (offset + speed(rpm) * (now - time as f64) as f32).clamp(0.0, range as f32)
}

/// `getGridOffset`: the length the rope is set to when it stops, `(int) (offset + .5)`.
pub fn grid_offset(offset: f32, range: i32) -> i32 {
    ((offset + 0.5) as i32).clamp(0, range)
}

/// How many blocks of rope and magnet hang below the pulley at `pos` (`PulleyBlockEntity.assemble`
/// counts them from below the pulley to the first block that is neither).
pub fn rope_length(read: &dyn Fn(Pos) -> Option<State>, pos: Pos) -> i32 {
    let mut i = 1;
    while i <= MAX_ROPE_LENGTH {
        let below = [pos[0], pos[1] - i, pos[2]];
        let Some(state) = read(below) else { break };
        if !state.is(ROPE) && !state.is(MAGNET) {
            break;
        }
        i += 1;
    }
    i - 1
}

/// Whether a rope of `offset` blocks may start moving at `rpm` (`offset >= range && speed > 0` and
/// `offset <= 0 && speed < 0` return early).
pub fn can_start(offset: i32, range: i32, rpm: f32) -> bool {
    if rpm == 0.0 {
        return false;
    }
    !(offset >= range && rpm > 0.0) && !(offset <= 0 && rpm < 0.0)
}

/// The cell the structure's anchor is in: just below the magnet (`worldPosition.below(floor(offset
/// + 1))`).
pub fn anchor(pos: Pos, offset: i32) -> Pos {
    [pos[0], pos[1] - (offset + 1), pos[2]]
}

/// `PulleyContraption.isAnchoringBlockAt`: the rope's own cells above the anchor never join.
pub fn anchoring(anchor: Pos, initial: i32) -> impl Fn(Pos) -> bool {
    move |p: Pos| {
        p[0] == anchor[0] && p[2] == anchor[2] && p[1] > anchor[1] && p[1] <= anchor[1] + initial + 1
    }
}

/// `PulleyContraption.assemble`: what hangs from the magnet.
pub fn assemble(world: &dyn World, anchor_cell: Pos, initial: i32) -> Result<Found, Fail> {
    let column = anchoring(anchor_cell, initial);
    let mut frontier = Vec::new();
    if let Some(state) = world.state(anchor_cell) {
        if !is_brittle(world, &state) {
            frontier.push(anchor_cell);
        }
    }
    let config = Config {
        forced: None,
        anchoring: &column,
        frontier,
        preexisting: 0,
        known: HashSet::new(),
    };
    search(world, &config)
}

/// `toPosition`: how far above the anchor the structure is when the rope is `offset` out and
/// started `initial` out (it sinks as the rope pays out).
pub fn rise(initial: i32, offset: f32) -> f32 {
    initial as f32 - offset
}

/// `visitNewPosition` for a rope with no structure paying out: whether the block the magnet is
/// about to meet is one to pick up (`isMovementNecessary` and not brittle).
pub fn picks_up(world: &dyn World, state: &State, pos: Pos) -> bool {
    movement_necessary(world, state, pos) && !is_brittle(world, state)
}

/// `HorizontalAxisKineticBlock.getStateForPlacement` without a shaft beside it it would prefer:
/// the pulley's axis is the horizontal direction's, a quarter turn clockwise.
pub fn placement_axis(horizontal: Dir) -> Axis {
    if horizontal.axis() == Axis::X { Axis::Z } else { Axis::X }
}

/// What `AbstractPulleyRenderer.renderSafe` draws below the pulley for a rope `offset` out:
/// heights below the pulley (positive down) of the magnet (and whether it is the half magnet), of
/// the half rope that shows as the magnet passes a block boundary, and of each rope block.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub magnet: Option<(f32, bool)>,
    pub half_rope: Option<f32>,
    pub ropes: Vec<f32>,
}

pub fn plan(offset: f32, running: bool) -> Plan {
    let magnet = (running || offset == 0.0).then(|| (offset, offset <= 0.25));
    let f = offset % 1.0;
    let half_rope = (offset > 0.75 && (f < 0.25 || f > 0.75)).then(|| if f > 0.75 { f - 1.0 } else { f });
    let mut ropes = Vec::new();
    if running {
        let mut i = 0;
        while (i as f32) < offset - 1.25 {
            ropes.push(offset - i as f32 - 1.0);
            i += 1;
        }
    }
    Plan { magnet, half_rope, ropes }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn the_rope_reaches_as_far_as_the_config_and_the_world_floor_allow() {
        assert_eq!(extension_range(100, -64), 163);
        assert_eq!(extension_range(500, -64), MAX_ROPE_LENGTH);
        assert_eq!(extension_range(-63, -64), 0);
        assert_eq!(extension_range(-70, -64), 0);
        assert_eq!(min_build_height("minecraft:the_nether"), 0);
    }

    #[test]
    fn the_rope_pays_out_a_five_hundred_and_twelfth_of_the_rpm_a_tick() {
        assert!((speed(16.0) - 16.0 / 512.0).abs() < 1e-6);
        assert_eq!(speed(1000.0), 0.49);
        assert_eq!(speed(-1000.0), -0.49);
        // Kept inside its range.
        assert_eq!(offset_at(3.0, 0, 256.0, 1000.0, 5), 5.0);
        assert_eq!(offset_at(3.0, 0, -256.0, 1000.0, 5), 0.0);
        assert!((offset_at(1.0, 10, 16.0, 42.0, 10) - 2.0).abs() < 1e-6);
        assert_eq!(grid_offset(2.4, 10), 2);
        assert_eq!(grid_offset(2.5, 10), 3);
        assert_eq!(grid_offset(12.0, 10), 10);
        assert_eq!(grid_offset(-0.2, 10), 0);
    }

    #[test]
    fn a_rope_counts_its_blocks_down_to_the_first_that_is_not_rope() {
        let mut grid = Grid(HashMap::new());
        let pulley = [0, 10, 0];
        for y in 7..=9 {
            grid.0.insert([0, y, 0], State::new(ROPE));
        }
        grid.0.insert([0, 6, 0], State::new(MAGNET));
        grid.0.insert([0, 5, 0], State::new("minecraft:stone"));
        assert_eq!(rope_length(&|p| grid.state(p), pulley), 4);
        grid.0.clear();
        assert_eq!(rope_length(&|p| grid.state(p), pulley), 0);
        assert_eq!(anchor(pulley, 4), [0, 5, 0]);
    }

    #[test]
    fn a_rope_only_starts_where_it_can_go() {
        assert!(can_start(0, 10, 16.0));
        assert!(!can_start(10, 10, 16.0));
        assert!(can_start(10, 10, -16.0));
        assert!(!can_start(0, 10, -16.0));
        assert!(!can_start(3, 10, 0.0));
    }

    #[test]
    fn what_hangs_from_the_magnet_is_the_structure_and_the_rope_is_not() {
        let mut grid = Grid(HashMap::new());
        let pulley = [0, 10, 0];
        for y in 8..=9 {
            grid.0.insert([0, y, 0], State::new(ROPE));
        }
        grid.0.insert([0, 7, 0], State::new(MAGNET));
        // Below the magnet: a slime block with stone beside it, and a torch hanging off the stone.
        grid.0.insert([0, 6, 0], State::new("minecraft:slime_block"));
        grid.0.insert([1, 6, 0], State::new("minecraft:stone"));
        grid.0.insert([2, 6, 0], State::new("minecraft:stone"));
        let at = anchor(pulley, rope_length(&|p| grid.state(p), pulley));
        assert_eq!(at, [0, 6, 0]);
        let found = assemble(&grid, at, 3).expect("found");
        let cells: HashSet<Pos> = found.blocks.iter().map(|(p, _)| *p).collect();
        assert!(cells.contains(&[0, 6, 0]) && cells.contains(&[1, 6, 0]));
        assert!(!cells.contains(&[2, 6, 0]) && !cells.contains(&[0, 7, 0]));
        // The column the rope fills never joins, even if the search could reach it.
        let column = anchoring(at, 3);
        assert!(column([0, 7, 0]) && column([0, 10, 0]) && !column([0, 11, 0]) && !column([0, 6, 0]));
        assert!(!column([1, 8, 0]));
        // Nothing below: nothing to carry, which is still a rope.
        grid.0.remove(&[0, 6, 0]);
        grid.0.remove(&[1, 6, 0]);
        grid.0.remove(&[2, 6, 0]);
        assert!(assemble(&grid, at, 3).expect("found").blocks.is_empty());
    }

    #[test]
    fn the_structure_sinks_as_the_rope_pays_out() {
        assert_eq!(rise(4, 4.0), 0.0);
        assert_eq!(rise(4, 6.5), -2.5);
        assert_eq!(rise(4, 0.0), 4.0);
    }

    #[test]
    fn the_pulley_sits_across_the_way_the_player_faces() {
        assert_eq!(placement_axis(Dir::North), Axis::X);
        assert_eq!(placement_axis(Dir::East), Axis::Z);
    }

    #[test]
    fn the_rope_is_drawn_as_a_magnet_rope_blocks_and_a_half_rope_at_a_boundary() {
        // Retracted: the half magnet under the coil, nothing else.
        let p = plan(0.0, false);
        assert_eq!(p.magnet, Some((0.0, true)));
        assert!(p.half_rope.is_none() && p.ropes.is_empty());
        // A rope that isn't running isn't drawn out.
        assert!(plan(3.0, false).magnet.is_none());
        // Running at 3.5: the magnet, no half rope (it shows near whole numbers), rope blocks.
        let p = plan(3.5, true);
        assert_eq!(p.magnet, Some((3.5, false)));
        assert!(p.half_rope.is_none());
        assert_eq!(p.ropes, vec![2.5, 1.5, 0.5]);
        // Just past a whole block the half rope shows under the coil.
        assert!((plan(3.1, true).half_rope.unwrap() - 0.1).abs() < 1e-5);
        let near_next = plan(3.9, true).half_rope.unwrap();
        assert!(near_next < 0.0 && near_next > -0.25);
    }

    #[test]
    fn only_a_block_that_moves_is_picked_up_off_the_magnet() {
        let grid = Grid(HashMap::new());
        let stone = State::new("minecraft:stone");
        assert!(picks_up(&Grid(HashMap::from([([0, 0, 0], stone.clone())])), &stone, [0, 0, 0]));
        assert!(!picks_up(&grid, &State::new("minecraft:air"), [0, 0, 0]));
        assert!(!picks_up(&grid, &State::new("minecraft:torch"), [0, 0, 0]));
    }
}
