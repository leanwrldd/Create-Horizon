//! The Mechanical Pump's pipe network: which pipes the pump reaches on each side, which fluid
//! handlers (tanks) are at the ends, how much each moves per tick, and how the fluid divides.
//! No game types: the glue says what each block is and moves the fluid.
//!
//! Ported from Create `content/fluids/pump/PumpBlockEntity` (`distributePressureTo`,
//! `searchForEndpointRecursively`, `hasReachedValidEndpoint`), `FluidNetwork.tick` (the transfer
//! speed and how it divides among the targets), `PipeConnection.tickFlowProgress` (how fast a
//! flow reaches along a pipe) and `FluidPropagator` (`getPumpRange`, which sides a pipe opens).
//!
//! Known differences from Create:
//! - A pipe network holds no pressure or flow state between ticks: the glue solves the graph
//!   now and then and moves fluid each tick from the sources behind the pump to the targets in
//!   front, after a start-up delay worked out from the same flow speeds.
//! - Open pipe ends (spilling into the world, picking up sources), other pumps joined into the
//!   network (their throughputs adding) and fluid reactions are not part of the graph: they
//!   count as dead ends.

use std::collections::{BTreeMap, BTreeSet};

use crate::{Axis, Dir};

pub type Pos = [i32; 3];

/// `CStress` gives the pump no extra: `AllBlocks.MECHANICAL_PUMP` is `CStress.setImpact(4.0)`.
pub const IMPACT: f32 = 4.0;
/// `CFluids.mechanicalPumpRange`: how far a pump pushes or pulls on either side, in blocks.
pub const RANGE: i32 = 16;

/// What the glue says a block is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    /// Nothing that carries or holds fluid.
    Nothing,
    /// A pipe, open on the faces `Faces` names (`FluidTransportBehaviour.canHaveFlowToward`).
    Pipe(Faces),
    /// A pump facing a direction (`PumpBlock.FACING`).
    Pump(Dir),
    /// A block with fluid tanks on that side (`FluidPropagator.hasFluidCapability`).
    Handler,
}

/// A set of faces, a bit per [`Dir::ALL`] entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Faces(pub u8);

impl Faces {
    pub fn of(open: &[Dir]) -> Self {
        Self(open.iter().fold(0, |bits, d| bits | 1 << *d as u8))
    }

    pub fn has(self, face: Dir) -> bool {
        self.0 & (1 << face as u8) != 0
    }

    /// A straight pipe along `axis` (`AxisPipeBlock`).
    pub fn axis(axis: Axis) -> Self {
        Self::of(&Dir::ALL.into_iter().filter(|d| d.axis() == axis).collect::<Vec<_>>())
    }
}

fn add(pos: Pos, dir: Dir) -> Pos {
    let n = dir.normal();
    [pos[0] + n[0], pos[1] + n[1], pos[2] + n[2]]
}

/// `FluidTransportBehaviour.canHaveFlowToward`.
fn can_flow(cell: Cell, face: Dir) -> bool {
    match cell {
        Cell::Pipe(open) => open.has(face),
        // `PumpFluidTransferBehaviour.canHaveFlowToward`: `isSideAccessible`.
        Cell::Pump(facing) => facing.axis() == face.axis(),
        _ => false,
    }
}

/// `PumpBlockEntity.isPullingOnSide(isFront(side))`: a pump pulls on every side but its front.
pub fn pulls_on(facing: Dir, side: Dir) -> bool {
    side != facing
}

/// A tank at the end of a network.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Endpoint {
    /// The block holding the fluid.
    pub handler: Pos,
    /// Its face the network touches (`FlowSource.FluidHandler`: `location.getOppositeFace()`).
    pub face: Dir,
    /// The pressure on the pipe face there (`PipeConnection.pressure`).
    pub pressure: f32,
}

/// What one side of a pump reaches.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Side {
    pub endpoints: Vec<Endpoint>,
    /// The pressure at each distance from the pump, for [`warmup_ticks`].
    pub pressures: Vec<(i32, f32)>,
}

/// `FluidNetwork`'s `transferSpeed`: `(int) Math.max(1, pressure / 2f)` mB each tick.
pub fn transfer_speed(pressure: f32) -> i32 {
    (pressure / 2.0).max(1.0) as i32
}

/// `PipeConnection.tickFlowProgress`: how far a flow gets along a pipe face each tick.
pub fn flow_speed(pressure: f32) -> f32 {
    1.0 / 32.0 + (pressure / 128.0).clamp(0.0, 1.0) * 31.0 / 32.0
}

/// Ticks before fluid arrives: `FluidNetwork.pauseBeforePropagation` (2), then a flow has to
/// reach along each pipe face, in then out, one distance after the other.
pub fn warmup_ticks(sides: &[&Side]) -> i32 {
    let mut ticks = 2;
    for side in sides {
        let mut strongest: BTreeMap<i32, f32> = BTreeMap::new();
        for (distance, pressure) in &side.pressures {
            let entry = strongest.entry(*distance).or_insert(0.0);
            *entry = entry.max(*pressure);
        }
        for pressure in strongest.values() {
            ticks += 2 * (1.0 / flow_speed(*pressure)).ceil() as i32;
        }
    }
    ticks
}

/// `hasReachedValidEndpoint`: whether the block past `face` of `pos` ends a network that pulls
/// (`pull`) or pushes.
fn reached_endpoint(cells: &mut impl FnMut(Pos) -> Cell, pos: Pos, face: Dir, pull: bool) -> bool {
    let connected = cells(add(pos, face));
    match connected {
        // Facing a pump: an end only if it works the opposite way to this side.
        Cell::Pump(facing) if facing.axis() == face.axis() => {
            pulls_on(facing, face.opposite()) != pull
        }
        // Another pipe is no end.
        Cell::Pipe(_) if can_flow(connected, face.opposite()) => false,
        Cell::Handler => true,
        _ => false,
    }
}

type Graph = BTreeMap<Pos, (i32, BTreeMap<Dir, bool>)>;

fn graph_set(graph: &mut Graph, pos: Pos, distance: i32, face: Dir, pull: bool) {
    graph
        .entry(pos)
        .or_insert_with(|| (distance, BTreeMap::new()))
        .1
        .insert(face, pull);
}

/// `searchForEndpointRecursively`.
fn search(
    graph: &Graph,
    targets: &BTreeSet<(Pos, Dir)>,
    valid: &mut BTreeMap<i32, BTreeSet<(Pos, Dir)>>,
    current: (Pos, Dir),
    pull: bool,
    depth: usize,
) -> bool {
    let Some((distance, map)) = graph.get(&current.0) else {
        return false;
    };
    if depth > 4096 {
        return false;
    }
    let mut any = false;
    for next in Dir::ALL {
        if next == current.1 {
            continue;
        }
        let Some(&flag) = map.get(&next) else {
            continue;
        };
        let local = (current.0, next);
        if targets.contains(&local) {
            valid.entry(*distance).or_default().insert(local);
            any = true;
            continue;
        }
        if flag != pull {
            continue;
        }
        if !search(graph, targets, valid, (add(current.0, next), next.opposite()), pull, depth + 1) {
            continue;
        }
        valid.entry(*distance).or_default().insert(local);
        any = true;
    }
    if any {
        valid.entry(*distance).or_default().insert(current);
    }
    any
}

/// `distributePressureTo(side)` of the pump at `pump`: the pipes reached from its `side` within
/// `range`, and the tanks at their ends. `pull` is whether the side pulls (every side but the
/// front); `speed` is the pump's rotation speed in RPM.
pub fn solve_side(
    cells: &mut impl FnMut(Pos) -> Cell,
    pump: Pos,
    side: Dir,
    pull: bool,
    speed: f32,
    range: i32,
) -> Side {
    let mut result = Side::default();
    let pressure = speed.abs();
    if pressure == 0.0 {
        return result;
    }
    // A tank right against the pump: the pump's own connection has all the pressure.
    if reached_endpoint(cells, pump, side, pull) {
        if cells(add(pump, side)) == Cell::Handler {
            result.endpoints.push(Endpoint {
                handler: add(pump, side),
                face: side.opposite(),
                pressure,
            });
            result.pressures.push((0, pressure));
        }
        return result;
    }

    let start = add(pump, side);
    let mut graph = Graph::new();
    let mut targets: BTreeSet<(Pos, Dir)> = BTreeSet::new();
    graph_set(&mut graph, pump, 0, side, pull);
    graph_set(&mut graph, start, 1, side.opposite(), !pull);

    let mut frontier = std::collections::VecDeque::from([(1, start)]);
    let mut visited: BTreeSet<Pos> = BTreeSet::new();
    while let Some((distance, current)) = frontier.pop_front() {
        if !visited.insert(current) {
            continue;
        }
        let cell = cells(current);
        if !matches!(cell, Cell::Pipe(_)) {
            continue;
        }
        for face in Dir::ALL {
            if !can_flow(cell, face) {
                continue;
            }
            let connected = add(current, face);
            // The boundary back to the pump.
            if current == start && face == side.opposite() {
                continue;
            }
            if reached_endpoint(cells, current, face, pull) {
                graph_set(&mut graph, current, distance, face, pull);
                targets.insert((current, face));
                continue;
            }
            if !matches!(cells(connected), Cell::Pipe(_)) || visited.contains(&connected) {
                continue;
            }
            if distance + 1 >= range {
                graph_set(&mut graph, current, distance, face, pull);
                targets.insert((current, face));
                continue;
            }
            graph_set(&mut graph, current, distance, face, pull);
            graph_set(&mut graph, connected, distance + 1, face.opposite(), !pull);
            frontier.push_back((distance + 1, connected));
        }
    }

    let mut valid: BTreeMap<i32, BTreeSet<(Pos, Dir)>> = BTreeMap::new();
    search(&graph, &targets, &mut valid, (pump, side.opposite()), pull, 0);

    for (distance, set) in &valid {
        let parallel = (set.len() as i32 - 1).max(1) as f32;
        let share = pressure / parallel;
        result.pressures.push((*distance, share));
        for (pos, face) in set {
            if *pos == pump || !targets.contains(&(*pos, *face)) {
                continue;
            }
            if cells(add(*pos, *face)) == Cell::Handler {
                result.endpoints.push(Endpoint {
                    handler: add(*pos, *face),
                    face: face.opposite(),
                    pressure: share,
                });
            }
        }
    }
    result
}

/// How a transfer divides among targets (`FluidNetwork.tick`'s output loop): `amount` is shared
/// equally with the remainder going to the first ones, and what a full target cannot take goes
/// to the rest. `rooms` is what each target has room for; the answer is what each gets.
pub fn distribute(amount: i32, rooms: &[i32]) -> Vec<i32> {
    let mut given = vec![0; rooms.len()];
    let mut left_room: Vec<i32> = rooms.iter().map(|r| (*r).max(0)).collect();
    let mut available: Vec<usize> = (0..rooms.len()).collect();
    let mut remaining = amount.max(0);
    while !available.is_empty() && remaining > 0 {
        let divided = remaining / available.len() as i32;
        let mut remainder = remaining % available.len() as i32;
        let mut index = 0;
        while index < available.len() {
            if remaining <= 0 {
                break;
            }
            let target = available[index];
            let mut to_transfer = divided;
            if remainder > 0 {
                to_transfer += 1;
                remainder -= 1;
            }
            let fill = to_transfer.min(left_room[target]);
            given[target] += fill;
            left_room[target] -= fill;
            remaining -= fill;
            if fill < to_transfer {
                available.remove(index);
            } else {
                index += 1;
            }
        }
    }
    given
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct World(HashMap<Pos, Cell>);

    impl World {
        fn cell(&mut self, pos: Pos) -> Cell {
            self.0.get(&pos).copied().unwrap_or(Cell::Nothing)
        }
    }

    /// A straight line along x: a tank at 0, pipes at 1..=pipes, then the pump, pipes, a tank.
    fn line(before: i32, after: i32) -> (World, Pos) {
        let mut map = HashMap::new();
        map.insert([0, 0, 0], Cell::Handler);
        for x in 1..=before {
            map.insert([x, 0, 0], Cell::Pipe(Faces::axis(Axis::X)));
        }
        let pump = [before + 1, 0, 0];
        map.insert(pump, Cell::Pump(Dir::East));
        for x in 1..=after {
            map.insert([pump[0] + x, 0, 0], Cell::Pipe(Faces::axis(Axis::X)));
        }
        map.insert([pump[0] + after + 1, 0, 0], Cell::Handler);
        (World(map), pump)
    }

    #[test]
    fn a_pump_between_two_pipes_pulls_from_the_tank_behind_and_pushes_to_the_one_ahead() {
        let (mut world, pump) = line(3, 4);
        let mut cells = |p| world.cell(p);
        let back = solve_side(&mut cells, pump, Dir::West, true, 64.0, RANGE);
        let front = solve_side(&mut cells, pump, Dir::East, false, 64.0, RANGE);
        assert_eq!(back.endpoints.len(), 1);
        assert_eq!(back.endpoints[0].handler, [0, 0, 0]);
        assert_eq!(back.endpoints[0].face, Dir::East);
        assert_eq!(front.endpoints.len(), 1);
        assert_eq!(front.endpoints[0].handler, [pump[0] + 5, 0, 0]);
        assert_eq!(front.endpoints[0].face, Dir::West);
        // One branch: the pressure is the pump's speed, and half of it is the mB each tick.
        assert_eq!(back.endpoints[0].pressure, 64.0);
        assert_eq!(transfer_speed(back.endpoints[0].pressure), 32);
    }

    #[test]
    fn tanks_right_against_the_pump_are_reached_with_its_full_pressure() {
        let (mut world, pump) = line(0, 0);
        let mut cells = |p| world.cell(p);
        let back = solve_side(&mut cells, pump, Dir::West, true, 16.0, RANGE);
        let front = solve_side(&mut cells, pump, Dir::East, false, 16.0, RANGE);
        assert_eq!(back.endpoints[0].handler, [0, 0, 0]);
        assert_eq!(front.endpoints[0].handler, [2, 0, 0]);
        assert_eq!(transfer_speed(16.0), 8);
        assert_eq!(transfer_speed(1.0), 1);
    }

    #[test]
    fn a_still_pump_reaches_nothing() {
        let (mut world, pump) = line(1, 1);
        let mut cells = |p| world.cell(p);
        assert!(solve_side(&mut cells, pump, Dir::East, false, 0.0, RANGE).endpoints.is_empty());
    }

    #[test]
    fn a_pipe_out_of_range_reaches_no_tank() {
        // 16 pipes: the last is at distance 16, so the tank behind it is out of reach.
        let (mut world, pump) = line(RANGE, 1);
        let mut cells = |p| world.cell(p);
        let back = solve_side(&mut cells, pump, Dir::West, true, 32.0, RANGE);
        assert!(back.endpoints.is_empty());
        let (mut world, pump) = line(RANGE - 2, 1);
        let mut cells = |p| world.cell(p);
        let back = solve_side(&mut cells, pump, Dir::West, true, 32.0, RANGE);
        assert_eq!(back.endpoints.len(), 1);
    }

    #[test]
    fn a_tee_splits_the_pressure_between_two_tanks() {
        // Pump at x = 1 facing east, a pipe at x = 2 that opens east, up and down, tanks above,
        // below and ahead of it.
        let mut map = HashMap::new();
        map.insert([0, 0, 0], Cell::Handler);
        map.insert([1, 0, 0], Cell::Pump(Dir::East));
        map.insert([2, 0, 0], Cell::Pipe(Faces::of(&[Dir::West, Dir::Up, Dir::Down])));
        map.insert([2, 1, 0], Cell::Handler);
        map.insert([2, -1, 0], Cell::Handler);
        let mut world = World(map);
        let mut cells = |p| world.cell(p);
        let front = solve_side(&mut cells, [1, 0, 0], Dir::East, false, 60.0, RANGE);
        let mut handlers: Vec<Pos> = front.endpoints.iter().map(|e| e.handler).collect();
        handlers.sort();
        assert_eq!(handlers, vec![[2, -1, 0], [2, 1, 0]]);
        // `Math.max(1, set.size() - 1)` branches: the in face and two out faces make 3, so 2.
        assert!(front.endpoints.iter().all(|e| e.pressure == 30.0));
    }

    #[test]
    fn a_pump_facing_the_other_way_swaps_the_sides() {
        let (mut world, pump) = line(2, 2);
        world.0.insert(pump, Cell::Pump(Dir::West));
        let mut cells = |p| world.cell(p);
        // The west side is its front now: it pushes there.
        assert!(!pulls_on(Dir::West, Dir::West));
        assert!(pulls_on(Dir::West, Dir::East));
        let front = solve_side(&mut cells, pump, Dir::West, false, 32.0, RANGE);
        assert_eq!(front.endpoints[0].handler, [0, 0, 0]);
    }

    #[test]
    fn two_pumps_pushing_at_each_other_do_not_pass_fluid() {
        let mut map = HashMap::new();
        map.insert([0, 0, 0], Cell::Pump(Dir::East));
        map.insert([1, 0, 0], Cell::Pipe(Faces::axis(Axis::X)));
        map.insert([2, 0, 0], Cell::Pump(Dir::West));
        let mut world = World(map);
        let mut cells = |p| world.cell(p);
        let front = solve_side(&mut cells, [0, 0, 0], Dir::East, false, 32.0, RANGE);
        assert!(front.endpoints.is_empty());
    }

    #[test]
    fn fluid_divides_equally_and_a_full_target_passes_its_share_on() {
        assert_eq!(distribute(10, &[100, 100]), vec![5, 5]);
        assert_eq!(distribute(11, &[100, 100]), vec![6, 5]);
        assert_eq!(distribute(10, &[2, 100]), vec![2, 8]);
        assert_eq!(distribute(10, &[0, 0]), vec![0, 0]);
        assert_eq!(distribute(10, &[3]), vec![3]);
        assert_eq!(distribute(0, &[5]), vec![0]);
    }

    #[test]
    fn a_faster_pump_starts_the_flow_sooner() {
        let slow = Side { endpoints: vec![], pressures: vec![(1, 8.0), (2, 8.0)] };
        let fast = Side { endpoints: vec![], pressures: vec![(1, 256.0), (2, 256.0)] };
        assert!(warmup_ticks(&[&slow]) > warmup_ticks(&[&fast]));
        assert!((flow_speed(256.0) - 1.0).abs() < 1e-6);
        assert!((flow_speed(0.0) - 1.0 / 32.0).abs() < 1e-6);
    }
}
