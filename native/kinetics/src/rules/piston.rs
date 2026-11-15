//! The Mechanical Piston's rules: how it gathers its poles and the blocks it pushes, how fast
//! it moves and where it stops. Ported from Create (MIT, the Creators of Create), `mc1.21.1/dev`
//! at 4148fd8: `PistonContraption` (`collectExtensions`, `addToInitialFrontier`,
//! `isAnchoringBlockAt`), `LinearActuatorBlockEntity` (`tick`, `getGridOffset`),
//! `MechanicalPistonBlockEntity` (`getMovementSpeed`, `assemble`, `toPosition`),
//! `ContraptionCollider.collideBlocks` and `KineticBlockEntity.convertToLinear`.
//!
//! No game types: the glue is `native/api/src/machines/piston.rs`.
//!
//! Known differences from Create:
//! - Only `MovementMode.MOVE_PLACE` (the scroll value's default): the piston always sets its
//!   structure down where it stops; "move, never place" and "place only when returned" are not
//!   offered.
//! - `ServerSpeedProvider` (the client slowing with the server's tick rate) is not modelled; a
//!   client estimates the offset from the game time it reads.

use std::collections::HashSet;

use crate::rules::contraption::{
    self as contraption, Config, Fail, Found, State, World, is_brittle, movement_allowed,
    movement_necessary, not_supportive, push_only, relative, sub,
};
use crate::{Axis, Dir, Pos};

/// `CStress.setImpact(4.0)` (`BuilderTransformers.mechanicalPiston`).
pub const IMPACT: f32 = 4.0;

/// `KineticBlockEntity.convertToLinear`: RPM to blocks a tick, before the clamp.
pub fn convert_to_linear(rpm: f32) -> f32 {
    rpm / 512.0
}

/// The poles, the head and the geometry `collectExtensions` finds.
#[derive(Clone, Debug)]
pub struct Collected {
    /// `anchor`: the cell in front of the piston.
    pub anchor: Pos,
    /// Poles behind the piston plus those in front (`extensionLength`).
    pub extension_length: i32,
    /// Poles in front of the piston (`initialExtensionProgress`): how far it is extended.
    pub initial: i32,
    /// The pole and head blocks, at the cells they stand in now, each with its state.
    pub poles: Vec<(Pos, State)>,
}

fn air() -> State {
    State::new("minecraft:air")
}

/// `PistonContraption.collectExtensions` for the piston at `pos` facing `direction`.
pub fn collect_extensions(
    world: &dyn World,
    pos: Pos,
    direction: Dir,
    piston: &State,
) -> Result<Collected, Fail> {
    let sticky = piston.is("create:sticky_mechanical_piston");
    let pole_on = |state: &State| {
        state.is("create:piston_extension_pole")
            && state.dir("facing").is_some_and(|f| f.axis() == direction.axis())
    };
    let head_facing = |state: &State| {
        state.is("create:mechanical_piston_head") && state.dir("facing") == Some(direction)
    };
    let mut poles: Vec<(Pos, State)> = Vec::new();
    let mut actual_start = pos;
    let mut in_front = 0;
    let mut next = world.state(relative(pos, direction, 1)).unwrap_or_else(air);
    if piston.get("state") == Some("extended") {
        while pole_on(&next) || head_facing(&next) {
            actual_start = relative(actual_start, direction, 1);
            poles.push((actual_start, next.clone().with("facing", direction.name())));
            in_front += 1;
            if next.is("create:mechanical_piston_head") {
                break;
            }
            next = world.state(relative(actual_start, direction, 1)).unwrap_or_else(air);
            if in_front > contraption::MAX_PISTON_POLES {
                return Err(Fail::TooManyPistonPoles);
            }
        }
    }
    if in_front == 0 {
        poles.push((
            pos,
            State::new("create:mechanical_piston_head")
                .with("facing", direction.name())
                .with("type", if sticky { "sticky" } else { "normal" }),
        ));
    } else {
        poles.push((
            pos,
            State::new("create:piston_extension_pole").with("facing", direction.name()),
        ));
    }
    let mut end = pos;
    let mut next = world.state(relative(end, direction, -1)).unwrap_or_else(air);
    let mut in_back = 0;
    while pole_on(&next) {
        end = relative(end, direction, -1);
        poles.push((end, next.clone().with("facing", direction.name())));
        in_back += 1;
        next = world.state(relative(end, direction, -1)).unwrap_or_else(air);
        if in_front + in_back > contraption::MAX_PISTON_POLES {
            return Err(Fail::TooManyPistonPoles);
        }
    }
    let extension_length = in_back + in_front;
    if extension_length == 0 {
        return Err(Fail::NoPistonPoles);
    }
    Ok(Collected {
        anchor: relative(pos, direction, 1),
        extension_length,
        initial: in_front,
        poles,
    })
}

/// `isAnchoringBlockAt`: the column behind the anchor that the poles fill (the extension's
/// collision box), never part of the search.
pub fn anchoring(anchor: Pos, direction: Dir, extension_length: i32) -> impl Fn(Pos) -> bool {
    move |pos: Pos| {
        let local = sub(pos, anchor);
        let along = local[direction.axis().index()] * direction.sign();
        let others_zero = (0..3)
            .filter(|i| *i != direction.axis().index())
            .all(|i| local[i] == 0);
        others_zero && (-(extension_length + 1)..=-1).contains(&along)
    }
}

/// `PistonContraption.addToInitialFrontier`: the cells the push starts from, in front of the
/// head. `direction` is the way it searches (the piston's facing, or the opposite when
/// retracting), `sticky` whether it is a sticky piston.
pub fn initial_frontier(
    world: &dyn World,
    anchor: Pos,
    orientation: Dir,
    initial: i32,
    direction: Dir,
    sticky: bool,
) -> Result<Vec<Pos>, Fail> {
    let mut frontier = Vec::new();
    let retracting = direction != orientation;
    if retracting && !sticky {
        return Ok(frontier);
    }
    for offset in 0..=contraption::MAX_CHASSIS_RANGE {
        if offset == 1 && retracting {
            return Ok(frontier);
        }
        let current = relative(anchor, orientation, offset + initial);
        let Some(state) = world.state(current) else {
            if retracting {
                return Ok(frontier);
            }
            return Err(Fail::UnloadedChunk(current));
        };
        if !movement_necessary(world, &state, current) {
            return Ok(frontier);
        }
        if is_brittle(world, &state) && !state.path().ends_with("_carpet") {
            return Ok(frontier);
        }
        if state.is("create:mechanical_piston_head")
            && state.dir("facing") == Some(direction.opposite())
        {
            return Ok(frontier);
        }
        if !movement_allowed(world, &state, current) {
            if retracting {
                return Ok(frontier);
            }
            return Err(Fail::UnmovableBlock(current, state.name.clone()));
        }
        if retracting && push_only(&state) {
            return Ok(frontier);
        }
        frontier.push(current);
        if not_supportive(world, &state, orientation) {
            return Ok(frontier);
        }
    }
    Ok(frontier)
}

/// `PistonContraption.assemble`: the poles and what the push finds, at the cells they stand in
/// now.
pub fn assemble(
    world: &dyn World,
    pos: Pos,
    direction: Dir,
    retract: bool,
    piston: &State,
) -> Result<(Collected, Found), Fail> {
    let collected = collect_extensions(world, pos, direction, piston)?;
    let search_direction = if retract { direction.opposite() } else { direction };
    let sticky = piston.is("create:sticky_mechanical_piston");
    let frontier = initial_frontier(
        world,
        collected.anchor,
        direction,
        collected.initial,
        search_direction,
        sticky,
    )?;
    let anchoring = anchoring(collected.anchor, direction, collected.extension_length);
    let known: HashSet<Pos> = collected.poles.iter().map(|(p, _)| *p).collect();
    let config = Config {
        forced: Some(search_direction),
        anchoring: &anchoring,
        frontier,
        preexisting: collected.poles.len(),
        known,
    };
    let found = contraption::search(world, &config)?;
    Ok((collected, found))
}

/// `MechanicalPistonBlockEntity.getMovementSpeed` without the client's correction: blocks a tick
/// along the facing, positive extending, clamped so the offset stays within `0..=range`.
pub fn movement_speed(rpm: f32, facing: Dir, offset: f32, range: i32) -> f32 {
    let linear = convert_to_linear(rpm).clamp(-0.49, 0.49);
    let step = facing.sign();
    let modifier = step * if facing.axis() == Axis::Z { -1 } else { 1 };
    let speed = linear * -(modifier as f32);
    speed.max(0.0 - offset).min(range as f32 - offset)
}

/// The way the structure moves for a rotation speed (`MechanicalPistonBlockEntity.assemble`'s
/// `movementDirection`): the speed's sign and the axis decide, not the facing.
pub fn movement_direction(rpm: f32, axis: Axis) -> Dir {
    let positive = match axis {
        Axis::X => Dir::East,
        Axis::Y => Dir::Up,
        Axis::Z => Dir::South,
    };
    if (rpm > 0.0) ^ (axis != Axis::Z) {
        positive
    } else {
        positive.opposite()
    }
}

/// `LinearActuatorBlockEntity.getGridOffset`.
pub fn grid_offset(offset: f32, range: i32) -> i32 {
    ((offset + 0.5).floor() as i32).clamp(0, range)
}

/// Where the structure's anchor cell is at `offset` (`toPosition`), along the facing.
pub fn position(anchor: Pos, facing: Dir, offset: f32) -> [f64; 3] {
    let n = facing.normal();
    [0, 1, 2].map(|i| f64::from(anchor[i]) + f64::from(n[i]) * f64::from(offset))
}

/// `ContraptionCollider.collideBlocks`'s cell: `BlockPos.containing(position)`, one on in the
/// movement direction when that is positive. The structure's colliders are tested from there.
pub fn collision_anchor(position: [f64; 3], movement: Dir) -> Pos {
    let mut cell = [0, 1, 2].map(|i| position[i].floor() as i32);
    if movement.sign() > 0 {
        cell = relative(cell, movement, 1);
    }
    cell
}

// --- the block (DirectionalAxisKineticBlock) ----------------------------------------------------

/// `DirectionalAxisKineticBlock.getRotationAxis`: the shaft lies across the facing, along the
/// first coordinate or not.
pub fn rotation_axis(facing: Dir, along_first: bool) -> Axis {
    match (facing.axis(), along_first) {
        (Axis::X, true) => Axis::Y,
        (Axis::X, false) => Axis::Z,
        (Axis::Y, true) => Axis::X,
        (Axis::Y, false) => Axis::Z,
        (Axis::Z, true) => Axis::X,
        (Axis::Z, false) => Axis::Y,
    }
}

/// The two shaft ends (`hasShaftTowards`: both faces on the rotation axis).
pub fn shaft_faces(facing: Dir, along_first: bool) -> [Dir; 2] {
    match rotation_axis(facing, along_first) {
        Axis::X => [Dir::West, Dir::East],
        Axis::Y => [Dir::Down, Dir::Up],
        Axis::Z => [Dir::North, Dir::South],
    }
}

/// `DirectionalAxisKineticBlock.getStateForPlacement` without the shaft beside it it would
/// prefer (`prefersConnectionTo` needs a neighbour's class): `(facing, axis_along_first)`.
/// `look` is `getNearestLookingDirection`, `horizontal` is `getHorizontalDirection`.
pub fn placement(look: Dir, horizontal: Dir, sneaking: bool) -> (Dir, bool) {
    let mut facing = look.opposite();
    if sneaking {
        facing = facing.opposite();
    }
    let along_first = if facing.axis() == Axis::Y {
        horizontal.axis() == Axis::X
    } else {
        facing.axis() == Axis::Z
    };
    (facing, along_first)
}

/// The turn a Wrench click on `clicked` gives the piston (`IWrenchable.getRotatedBlockState` of
/// a `DirectionalAxisKineticBlock`): `(facing, axis_along_first)`.
pub fn wrench(facing: Dir, along_first: bool, clicked: Dir) -> (Dir, bool) {
    let (mut new_facing, mut along) = (facing, along_first);
    if facing.axis() == clicked.axis() {
        along = !along;
    } else {
        loop {
            new_facing = crate::rules::drill::clockwise(new_facing, clicked.axis());
            if clicked.axis() == Axis::Y {
                along = !along;
            }
            if new_facing.axis() != clicked.axis() {
                break;
            }
        }
    }
    (new_facing, along)
}

/// The draw's offset at `game_time` (with the frame's `partial`) from a snapshot taken at tick
/// `time` at `offset`, moving `velocity` blocks a tick, kept within `0..=range`: how a client
/// (and the server, between sync points) follows the piston without being told every tick.
pub fn offset_at(offset: f32, time: i64, velocity: f32, now: f64, range: i32) -> f32 {
    let dt = (now - time as f64) as f32;
    (offset + velocity * dt).clamp(0.0, range as f32)
}

/// The unclamped speed of the offset (`getMovementSpeed` without its clamp to the range).
pub fn velocity(rpm: f32, facing: Dir) -> f32 {
    movement_speed(rpm, facing, 1.0e6, 2_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Grid(HashMap<Pos, State>);

    impl Grid {
        fn set(&mut self, pos: Pos, text: &str) {
            self.0.insert(pos, State::parse(text));
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

    fn piston_with_poles(back: i32, facing: Dir) -> (Grid, State) {
        let mut grid = Grid(HashMap::new());
        let piston = State::new("create:mechanical_piston")
            .with("facing", facing.name())
            .with("state", "retracted")
            .with("axis_along_first", "false");
        grid.0.insert([0, 0, 0], piston.clone());
        for i in 1..=back {
            grid.set(
                relative([0, 0, 0], facing, -i),
                &format!("create:piston_extension_pole[facing={}]", facing.name()),
            );
        }
        (grid, piston)
    }

    #[test]
    fn a_retracted_piston_with_poles() {
        let (mut grid, piston) = piston_with_poles(3, Dir::East);
        grid.set([1, 0, 0], "minecraft:stone");
        grid.set([2, 0, 0], "minecraft:dirt");
        let (collected, found) =
            assemble(&grid, [0, 0, 0], Dir::East, false, &piston).expect("assembles");
        assert_eq!(collected.extension_length, 3);
        assert_eq!(collected.initial, 0);
        assert_eq!(collected.anchor, [1, 0, 0]);
        assert_eq!(collected.poles.len(), 4);
        assert!(collected
            .poles
            .iter()
            .any(|(p, s)| *p == [0, 0, 0] && s.is("create:mechanical_piston_head")));
        let found: Vec<Pos> = found.blocks.iter().map(|(p, _)| *p).collect();
        assert_eq!(found.len(), 2);
        assert!(found.contains(&[1, 0, 0]) && found.contains(&[2, 0, 0]));
    }

    #[test]
    fn no_poles_no_piston() {
        let (grid, piston) = piston_with_poles(0, Dir::East);
        assert_eq!(
            assemble(&grid, [0, 0, 0], Dir::East, false, &piston).unwrap_err(),
            Fail::NoPistonPoles
        );
    }

    #[test]
    fn an_extended_piston_finds_its_head() {
        let (mut grid, _) = piston_with_poles(1, Dir::East);
        let piston = State::new("create:mechanical_piston")
            .with("facing", "east")
            .with("state", "extended");
        grid.0.insert([0, 0, 0], piston.clone());
        grid.set([1, 0, 0], "create:piston_extension_pole[facing=east]");
        grid.set([2, 0, 0], "create:mechanical_piston_head[facing=east,type=normal]");
        grid.set([3, 0, 0], "minecraft:stone");
        let (collected, found) =
            assemble(&grid, [0, 0, 0], Dir::East, false, &piston).expect("assembles");
        assert_eq!(collected.initial, 2);
        assert_eq!(collected.extension_length, 3);
        assert_eq!(found.blocks.len(), 1);
        assert!(collected
            .poles
            .iter()
            .any(|(p, s)| *p == [0, 0, 0] && s.is("create:piston_extension_pole")));
    }

    #[test]
    fn retracting_pulls_only_with_a_sticky_piston() {
        let (mut grid, _) = piston_with_poles(2, Dir::East);
        let piston = State::new("create:sticky_mechanical_piston")
            .with("facing", "east")
            .with("state", "extended");
        grid.0.insert([0, 0, 0], piston.clone());
        grid.set([1, 0, 0], "create:piston_extension_pole[facing=east]");
        grid.set([2, 0, 0], "create:mechanical_piston_head[facing=east,type=sticky]");
        grid.set([3, 0, 0], "minecraft:stone");
        grid.set([4, 0, 0], "minecraft:stone");
        let (_, found) = assemble(&grid, [0, 0, 0], Dir::East, true, &piston).expect("assembles");
        assert_eq!(found.blocks.len(), 1, "a sticky piston pulls one block");
        let mut plain = piston.clone();
        plain.name = "create:mechanical_piston".to_owned();
        grid.0.insert([0, 0, 0], plain.clone());
        let (_, found) = assemble(&grid, [0, 0, 0], Dir::East, true, &plain).expect("assembles");
        assert!(found.blocks.is_empty());
    }

    #[test]
    fn an_unmovable_block_in_front_stops_a_push() {
        let (mut grid, piston) = piston_with_poles(2, Dir::East);
        grid.set([1, 0, 0], "minecraft:stone");
        grid.set([2, 0, 0], "minecraft:obsidian");
        assert!(matches!(
            assemble(&grid, [0, 0, 0], Dir::East, false, &piston).unwrap_err(),
            Fail::UnmovableBlock([2, 0, 0], _)
        ));
    }

    #[test]
    fn a_push_takes_the_whole_line_in_front() {
        let (mut grid, piston) = piston_with_poles(4, Dir::East);
        for x in 1..=20 {
            grid.set([x, 0, 0], "minecraft:stone");
        }
        let (_, found) = assemble(&grid, [0, 0, 0], Dir::East, false, &piston).expect("assembles");
        // The first 17 start the frontier (`maxChassisRange` + 1) and each block takes the next
        // one in the forced direction (`offset == forcedDirection`), so the whole line moves.
        assert_eq!(found.blocks.len(), 20);
    }

    #[test]
    fn the_anchoring_column_is_behind_the_anchor() {
        let anchoring_east = anchoring([1, 0, 0], Dir::East, 3);
        assert!(anchoring_east([0, 0, 0]) && anchoring_east([-3, 0, 0]));
        assert!(!anchoring_east([-4, 0, 0]) && !anchoring_east([1, 0, 0]) && !anchoring_east([0, 1, 0]));
        let west = anchoring([-1, 0, 0], Dir::West, 2);
        assert!(west([0, 0, 0]) && west([2, 0, 0]) && !west([-1, 0, 0]));
    }

    #[test]
    fn speed_and_direction_follow_create() {
        // 64 RPM is 0.125 blocks a tick.
        assert!((movement_speed(-64.0, Dir::East, 0.0, 3) - 0.125).abs() < 1e-6);
        assert_eq!(movement_speed(64.0, Dir::East, 0.0, 3), 0.0, "can't retract past zero");
        assert!((movement_speed(1024.0, Dir::West, 0.0, 3) - 0.49).abs() < 1e-6);
        // Facing north (a negative step on z): the z flip cancels the step, so a negative speed extends.
        assert!(movement_speed(-64.0, Dir::North, 0.0, 3) > 0.0);
        // A speed that would carry it past the end stops exactly there.
        assert!((movement_speed(-1024.0, Dir::East, 2.9, 3) - 0.1).abs() < 1e-5);
        assert_eq!(movement_direction(64.0, Axis::X), Dir::West);
        assert_eq!(movement_direction(-64.0, Axis::X), Dir::East);
        assert_eq!(movement_direction(64.0, Axis::Z), Dir::South);
    }

    #[test]
    fn the_movement_direction_agrees_with_the_signed_speed() {
        // Whatever the facing, the way `assemble` says the structure moves is the way
        // `movement_speed` moves the offset: extending goes along the facing.
        for facing in Dir::ALL {
            for rpm in [64.0_f32, -64.0] {
                let speed = movement_speed(rpm, facing, 1.0, 3);
                let world = if speed > 0.0 { facing } else { facing.opposite() };
                assert_eq!(movement_direction(rpm, facing.axis()), world, "{facing:?} {rpm}");
            }
        }
    }

    #[test]
    fn placement_and_wrench_follow_the_directional_axis_block() {
        assert_eq!(placement(Dir::South, Dir::South, false), (Dir::North, true));
        assert_eq!(placement(Dir::Down, Dir::East, false), (Dir::Up, true));
        assert_eq!(placement(Dir::Down, Dir::North, true), (Dir::Down, false));
        assert_eq!(rotation_axis(Dir::Up, true), Axis::X);
        assert_eq!(rotation_axis(Dir::North, false), Axis::Y);
        assert_eq!(shaft_faces(Dir::East, false), [Dir::North, Dir::South]);
        // A click on its own axis flips the shaft's axis; another turns it round that face.
        assert_eq!(wrench(Dir::Up, false, Dir::Up), (Dir::Up, true));
        assert_eq!(wrench(Dir::North, false, Dir::Up), (Dir::East, true));
        assert_eq!(wrench(Dir::North, false, Dir::East), (Dir::Up, false));
    }

    #[test]
    fn the_offset_follows_a_snapshot() {
        assert!((offset_at(1.0, 100, 0.1, 110.0, 3) - 2.0).abs() < 1e-5);
        assert_eq!(offset_at(1.0, 100, 0.1, 500.0, 3), 3.0);
        assert_eq!(offset_at(1.0, 100, -0.1, 500.0, 3), 0.0);
        assert!((velocity(-64.0, Dir::East) - 0.125).abs() < 1e-6);
    }

    #[test]
    fn positions_and_grid_offsets() {
        assert_eq!(grid_offset(1.49, 3), 1);
        assert_eq!(grid_offset(1.5, 3), 2);
        assert_eq!(grid_offset(-0.2, 3), 0);
        assert_eq!(grid_offset(7.0, 3), 3);
        assert_eq!(position([5, 1, 2], Dir::West, 2.0), [3.0, 1.0, 2.0]);
        assert_eq!(collision_anchor([5.1, 1.0, 2.0], Dir::East), [6, 1, 2]);
        assert_eq!(collision_anchor([4.9, 1.0, 2.0], Dir::West), [4, 1, 2]);
    }
}
