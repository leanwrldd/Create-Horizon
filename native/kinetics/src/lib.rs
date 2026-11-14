//! Create's rotation network, as pure data in and data out.
//!
//! The game glue hands [`solve`] every kinetic block it knows of and gets back the speed of
//! each one. Nothing here touches Horizon, so the rules are tested without building the game.
//!
//! Rules follow Create's `RotationPropagator` (MIT, the Creators of Create):
//! - blocks joined shaft to shaft turn together;
//! - two small cogwheels side by side turn in opposite directions;
//! - a large cogwheel drives a diagonal small one at twice the speed, reversed;
//! - two large cogwheels on different axes mesh at a corner;
//! - a gearbox reverses the shaft opposite its input and turns the side shafts.
//!
//! A speed is in RPM, signed: positive is counterclockwise seen from the positive end of the
//! block's axis. A gearbox has four shafts that don't share one speed, so its speed is that of
//! its "positive" faces; [`face_sign`] gives each face's share.

pub mod belt;
pub mod generators;
pub mod press;
mod table;

pub use table::{Networks, looking};

use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub type Pos = [i32; 3];

/// Create's `maxRotationSpeed`: a block asked to turn faster breaks.
pub const MAX_SPEED: f32 = 256.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub const ALL: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

    pub const fn index(self) -> usize {
        match self {
            Axis::X => 0,
            Axis::Y => 1,
            Axis::Z => 2,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Axis::X => "x",
            Axis::Y => "y",
            Axis::Z => "z",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.name() == name)
    }

    /// The unit vector along the axis.
    pub const fn unit(self) -> [f32; 3] {
        match self {
            Axis::X => [1.0, 0.0, 0.0],
            Axis::Y => [0.0, 1.0, 0.0],
            Axis::Z => [0.0, 0.0, 1.0],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Dir {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

impl Dir {
    pub const ALL: [Dir; 6] = [
        Dir::Down,
        Dir::Up,
        Dir::North,
        Dir::South,
        Dir::West,
        Dir::East,
    ];

    pub const fn axis(self) -> Axis {
        match self {
            Dir::Down | Dir::Up => Axis::Y,
            Dir::North | Dir::South => Axis::Z,
            Dir::West | Dir::East => Axis::X,
        }
    }

    /// 1 towards the positive end of the axis, -1 towards the negative.
    pub const fn sign(self) -> i32 {
        match self {
            Dir::Up | Dir::South | Dir::East => 1,
            Dir::Down | Dir::North | Dir::West => -1,
        }
    }

    pub const fn opposite(self) -> Self {
        match self {
            Dir::Down => Dir::Up,
            Dir::Up => Dir::Down,
            Dir::North => Dir::South,
            Dir::South => Dir::North,
            Dir::West => Dir::East,
            Dir::East => Dir::West,
        }
    }

    pub const fn normal(self) -> Pos {
        match self {
            Dir::Down => [0, -1, 0],
            Dir::Up => [0, 1, 0],
            Dir::North => [0, 0, -1],
            Dir::South => [0, 0, 1],
            Dir::West => [-1, 0, 0],
            Dir::East => [1, 0, 0],
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Dir::Down => "down",
            Dir::Up => "up",
            Dir::North => "north",
            Dir::South => "south",
            Dir::West => "west",
            Dir::East => "east",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.name() == name)
    }

    pub fn from_normal(normal: Pos) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.normal() == normal)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Shaft(Axis),
    Cogwheel(Axis),
    LargeCogwheel(Axis),
    /// The axis is the one with no shafts; the four other faces have one each.
    Gearbox(Axis),
    /// The shaft comes out of the face named.
    Motor(Dir),
    /// The handle points out of the face named; the shaft comes out of the opposite one.
    Crank(Dir),
    /// A shaft along the axis, both ends.
    WaterWheel(Axis),
    /// One block of a horizontal belt facing from its start to its end. Every block of a belt
    /// turns with the next (`BeltBlockEntity.propagateRotationTo`); those with a pulley also
    /// take a shaft on both sides, across the belt.
    Belt(Dir, belt::Part),
}

impl Kind {
    /// Whether the block has a shaft end on `face`.
    pub fn has_shaft(self, face: Dir) -> bool {
        match self {
            Kind::Shaft(axis)
            | Kind::Cogwheel(axis)
            | Kind::LargeCogwheel(axis)
            | Kind::WaterWheel(axis) => face.axis() == axis,
            Kind::Gearbox(axis) => face.axis() != axis,
            Kind::Motor(facing) => face == facing,
            Kind::Crank(facing) => face == facing.opposite(),
            // `BeltBlock.hasShaftTowards`.
            Kind::Belt(facing, part) => {
                part.has_pulley() && face.axis() == belt::rotation_axis(facing)
            }
        }
    }
}

/// How the shaft on `face` turns relative to the block's own speed.
///
/// Always 1 but for a gearbox, where it is Create's `getAxisModifier` made independent of which
/// face the power comes in by: opposite faces differ, and of the two shaft axes one counts its
/// positive face as 1 and the other its negative face.
pub fn face_sign(kind: Kind, face: Dir) -> f32 {
    let Kind::Gearbox(axis) = kind else {
        return 1.0;
    };
    let first = Axis::ALL.into_iter().find(|a| *a != axis);
    let sign = face.sign() as f32;
    if Some(face.axis()) == first {
        sign
    } else {
        -sign
    }
}

/// One kinetic block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Node {
    pub kind: Kind,
    /// RPM the block makes by itself; 0 for one that only passes rotation on.
    pub generated: f32,
    /// Stress units it adds to its network's capacity per RPM generated.
    pub capacity: f32,
    /// Stress units it costs its network per RPM it turns at.
    pub impact: f32,
}

impl Node {
    pub const fn passive(kind: Kind) -> Self {
        Self {
            kind,
            generated: 0.0,
            capacity: 0.0,
            impact: 0.0,
        }
    }
}

/// `b`'s speed over `a`'s when the two are connected, `b` being at `a`'s position plus `diff`.
pub fn ratio(a: Kind, b: Kind, diff: Pos) -> Option<f32> {
    let nonzero = diff.iter().filter(|c| **c != 0).count();
    if diff.iter().any(|c| c.abs() > 1) {
        return None;
    }
    match nonzero {
        1 => {
            let face = Dir::from_normal(diff)?;
            if a.has_shaft(face) && b.has_shaft(face.opposite()) {
                return Some(face_sign(a, face) * face_sign(b, face.opposite()));
            }
            match (a, b) {
                (Kind::Cogwheel(x), Kind::Cogwheel(y)) if x == y && face.axis() != x => Some(-1.0),
                // The next block of the same belt: `b` follows `a`, or `a` follows `b`.
                (Kind::Belt(x, pa), Kind::Belt(y, pb)) if x == y => {
                    let follows = |from: belt::Part, to: belt::Part| {
                        from != belt::Part::End && to != belt::Part::Start
                    };
                    let linked = (face == x && follows(pa, pb))
                        || (face == x.opposite() && follows(pb, pa));
                    linked.then_some(1.0)
                }
                _ => None,
            }
        }
        2 => match (a, b) {
            (Kind::LargeCogwheel(x), Kind::Cogwheel(y)) if x == y && diff[x.index()] == 0 => {
                Some(-2.0)
            }
            (Kind::Cogwheel(x), Kind::LargeCogwheel(y)) if x == y && diff[x.index()] == 0 => {
                Some(-0.5)
            }
            (Kind::LargeCogwheel(x), Kind::LargeCogwheel(y)) if x != y => {
                let (along_a, along_b) = (diff[x.index()], diff[y.index()]);
                if along_a == 0 || along_b == 0 {
                    return None;
                }
                Some(if (along_a > 0) ^ (along_b > 0) {
                    -1.0
                } else {
                    1.0
                })
            }
            _ => None,
        },
        _ => None,
    }
}

/// What a network adds up to.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Network {
    pub capacity: f32,
    pub stress: f32,
    /// More stress than capacity: every block in it stands still.
    pub overstressed: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Solution {
    /// Every block's speed in RPM; 0 for one that stands still.
    pub speeds: BTreeMap<Pos, f32>,
    /// Blocks that can't turn as their neighbours ask (two speeds at once, the wrong way round,
    /// or past [`MAX_SPEED`]) and break, as Create's do. At most one per network: the caller
    /// removes it and solves again.
    pub breaks: Vec<Pos>,
    /// The network each block is in, as an index into `networks`.
    pub network_of: BTreeMap<Pos, usize>,
    pub networks: Vec<Network>,
}

fn add(a: Pos, b: Pos) -> Pos {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// The 6 faces and 12 edges a connection can go through.
fn offsets() -> impl Iterator<Item = Pos> {
    (-1..=1).flat_map(|x| {
        (-1..=1).flat_map(move |y| {
            (-1..=1).filter_map(move |z| {
                let nonzero = [x, y, z].iter().filter(|c| **c != 0).count();
                (nonzero == 1 || nonzero == 2).then_some([x, y, z])
            })
        })
    })
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-4 * a.abs().max(b.abs()).max(1.0)
}

/// Every block's speed. The same blocks always give the same answer: positions are walked in
/// order, so which block of a jammed network breaks doesn't depend on a hash.
pub fn solve(nodes: &BTreeMap<Pos, Node>) -> Solution {
    let mut solution = Solution::default();
    let mut seen = BTreeSet::new();
    for &root in nodes.keys() {
        if !seen.insert(root) {
            continue;
        }
        // Each block's speed as a multiple of the root's.
        let mut factors = BTreeMap::from([(root, 1.0f32)]);
        let mut queue = VecDeque::from([root]);
        let mut jammed = None;
        while let Some(pos) = queue.pop_front() {
            let kind = nodes[&pos].kind;
            let factor = factors[&pos];
            for offset in offsets() {
                let next = add(pos, offset);
                let Some(node) = nodes.get(&next) else {
                    continue;
                };
                let Some(ratio) = ratio(kind, node.kind, offset) else {
                    continue;
                };
                let wanted = factor * ratio;
                match factors.get(&next) {
                    Some(&had) if !close(had, wanted) => {
                        jammed.get_or_insert(next);
                    }
                    Some(_) => {}
                    None => {
                        factors.insert(next, wanted);
                        seen.insert(next);
                        queue.push_back(next);
                    }
                }
            }
        }

        let index = solution.networks.len();
        let mut network = Network::default();
        // The root speed each generator asks for.
        let mut asks: Vec<(Pos, f32)> = Vec::new();
        for (&pos, &factor) in &factors {
            solution.network_of.insert(pos, index);
            let node = &nodes[&pos];
            if node.generated != 0.0 {
                network.capacity += node.capacity * node.generated.abs();
                asks.push((pos, node.generated / factor));
            }
        }
        let fastest = asks
            .iter()
            .copied()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()));
        let mut root_speed = fastest.map_or(0.0, |(_, speed)| speed);
        if jammed.is_none() {
            // Generators turning against the fastest one: the slowest of them breaks.
            jammed = asks
                .iter()
                .filter(|(_, speed)| (*speed > 0.0) != (root_speed > 0.0))
                .min_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
                .map(|(pos, _)| *pos);
        }
        if jammed.is_none() {
            jammed = factors
                .iter()
                .find(|(_, factor)| (*factor * root_speed).abs() > MAX_SPEED + 1e-3)
                .map(|(pos, _)| *pos);
        }
        if let Some(pos) = jammed {
            solution.breaks.push(pos);
            root_speed = 0.0;
        }
        network.stress = factors
            .iter()
            .map(|(pos, factor)| nodes[pos].impact * (factor * root_speed).abs())
            .sum();
        network.overstressed = network.stress > network.capacity + 1e-3;
        if network.overstressed {
            root_speed = 0.0;
        }
        for (&pos, &factor) in &factors {
            solution.speeds.insert(pos, factor * root_speed);
        }
        solution.networks.push(network);
    }
    solution
}

#[cfg(test)]
mod tests {
    use super::*;

    fn motor(facing: Dir, rpm: f32) -> Node {
        Node {
            kind: Kind::Motor(facing),
            generated: rpm,
            capacity: 16384.0,
            impact: 0.0,
        }
    }

    fn world(blocks: impl IntoIterator<Item = (Pos, Node)>) -> BTreeMap<Pos, Node> {
        blocks.into_iter().collect()
    }

    fn passive(pos: Pos, kind: Kind) -> (Pos, Node) {
        (pos, Node::passive(kind))
    }

    #[test]
    fn a_line_of_shafts_turns_with_its_motor() {
        let nodes = world([
            ([0, 0, 0], motor(Dir::East, 16.0)),
            passive([1, 0, 0], Kind::Shaft(Axis::X)),
            passive([2, 0, 0], Kind::Shaft(Axis::X)),
            // Not in line: stands still, in a network of its own.
            passive([2, 1, 0], Kind::Shaft(Axis::X)),
            // In line but turned the wrong way.
            passive([3, 0, 0], Kind::Shaft(Axis::Z)),
        ]);
        let solution = solve(&nodes);
        assert_eq!(solution.speeds[&[1, 0, 0]], 16.0);
        assert_eq!(solution.speeds[&[2, 0, 0]], 16.0);
        assert_eq!(solution.speeds[&[2, 1, 0]], 0.0);
        assert_eq!(solution.speeds[&[3, 0, 0]], 0.0);
        assert!(solution.breaks.is_empty());
        assert_eq!(solution.networks.len(), 3);
    }

    #[test]
    fn a_motor_only_drives_from_its_shaft_face() {
        let nodes = world([
            ([0, 0, 0], motor(Dir::East, 16.0)),
            passive([-1, 0, 0], Kind::Shaft(Axis::X)),
        ]);
        assert_eq!(solve(&nodes).speeds[&[-1, 0, 0]], 0.0);
    }

    #[test]
    fn small_cogwheels_side_by_side_alternate() {
        let nodes = world([
            ([0, -1, 0], motor(Dir::Up, 32.0)),
            passive([0, 0, 0], Kind::Cogwheel(Axis::Y)),
            passive([1, 0, 0], Kind::Cogwheel(Axis::Y)),
            passive([2, 0, 0], Kind::Cogwheel(Axis::Y)),
        ]);
        let speeds = solve(&nodes).speeds;
        assert_eq!(speeds[&[0, 0, 0]], 32.0);
        assert_eq!(speeds[&[1, 0, 0]], -32.0);
        assert_eq!(speeds[&[2, 0, 0]], 32.0);
    }

    #[test]
    fn cogwheels_stacked_along_their_axis_share_a_shaft() {
        assert_eq!(
            ratio(Kind::Cogwheel(Axis::Y), Kind::Cogwheel(Axis::Y), [0, 1, 0]),
            Some(1.0)
        );
    }

    #[test]
    fn a_large_cogwheel_doubles_a_diagonal_small_one() {
        let nodes = world([
            ([0, -1, 0], motor(Dir::Up, 16.0)),
            passive([0, 0, 0], Kind::LargeCogwheel(Axis::Y)),
            passive([1, 0, 1], Kind::Cogwheel(Axis::Y)),
        ]);
        assert_eq!(solve(&nodes).speeds[&[1, 0, 1]], -32.0);
        // And the other way round it halves.
        assert_eq!(
            ratio(
                Kind::Cogwheel(Axis::Y),
                Kind::LargeCogwheel(Axis::Y),
                [-1, 0, -1]
            ),
            Some(-0.5)
        );
        // Straight beside it they'd overlap, not mesh.
        assert_eq!(
            ratio(
                Kind::LargeCogwheel(Axis::Y),
                Kind::Cogwheel(Axis::Y),
                [1, 0, 0]
            ),
            None
        );
        // Two large ones on one axis never mesh.
        assert_eq!(
            ratio(
                Kind::LargeCogwheel(Axis::Y),
                Kind::LargeCogwheel(Axis::Y),
                [1, 0, 1]
            ),
            None
        );
    }

    #[test]
    fn large_cogwheels_on_different_axes_mesh_at_a_corner() {
        let (y, x) = (Kind::LargeCogwheel(Axis::Y), Kind::LargeCogwheel(Axis::X));
        // Rims meet at (1, 0, 0), moving the same way when both speeds have one sign.
        assert_eq!(ratio(y, x, [1, 1, 0]), Some(1.0));
        assert_eq!(ratio(y, x, [1, -1, 0]), Some(-1.0));
        assert_eq!(ratio(y, x, [-1, 1, 0]), Some(-1.0));
        assert_eq!(ratio(y, x, [-1, -1, 0]), Some(1.0));
        // Seen from the other wheel it is the same connection.
        assert_eq!(ratio(x, y, [-1, -1, 0]), Some(1.0));
        assert_eq!(ratio(x, y, [-1, 1, 0]), Some(-1.0));
        // The corner has to lie along both axes.
        assert_eq!(ratio(y, x, [0, 1, 1]), None);
    }

    #[test]
    fn a_gearbox_reverses_across_and_turns_the_sides() {
        let gearbox = Kind::Gearbox(Axis::Y);
        // Create's getAxisModifier, with the power coming in by the west face.
        let source = Dir::West;
        for face in [Dir::East, Dir::North, Dir::South] {
            // Reversed across, and on the side face that points the same way as the input.
            let reversed = face.axis() == source.axis() || face.sign() == source.sign();
            let expected = if reversed { -1.0 } else { 1.0 };
            assert_eq!(
                face_sign(gearbox, face) / face_sign(gearbox, source),
                expected,
                "{face:?}"
            );
        }
        let nodes = world([
            ([-2, 0, 0], motor(Dir::East, 16.0)),
            passive([-1, 0, 0], Kind::Shaft(Axis::X)),
            passive([0, 0, 0], gearbox),
            passive([1, 0, 0], Kind::Shaft(Axis::X)),
            passive([0, 0, 1], Kind::Shaft(Axis::Z)),
            passive([0, 0, -1], Kind::Shaft(Axis::Z)),
            // No shaft on the gearbox's own axis.
            passive([0, 1, 0], Kind::Shaft(Axis::Y)),
        ]);
        let speeds = solve(&nodes).speeds;
        assert_eq!(speeds[&[1, 0, 0]], -16.0);
        assert_eq!(speeds[&[0, 0, 1]], 16.0);
        assert_eq!(speeds[&[0, 0, -1]], -16.0);
        assert_eq!(speeds[&[0, 1, 0]], 0.0);
    }

    #[test]
    fn the_faster_motor_sets_the_speed() {
        let nodes = world([
            ([0, 0, 0], motor(Dir::East, 16.0)),
            passive([1, 0, 0], Kind::Shaft(Axis::X)),
            ([2, 0, 0], motor(Dir::West, 64.0)),
        ]);
        let solution = solve(&nodes);
        assert!(solution.breaks.is_empty());
        assert_eq!(solution.speeds[&[1, 0, 0]], 64.0);
        assert_eq!(solution.networks[0].capacity, 16384.0 * 80.0);
    }

    #[test]
    fn motors_turning_against_each_other_break_the_slower() {
        let nodes = world([
            ([0, 0, 0], motor(Dir::East, 16.0)),
            passive([1, 0, 0], Kind::Shaft(Axis::X)),
            ([2, 0, 0], motor(Dir::West, -64.0)),
        ]);
        let solution = solve(&nodes);
        assert_eq!(solution.breaks, vec![[0, 0, 0]]);
        assert!(solution.speeds.values().all(|speed| *speed == 0.0));
    }

    #[test]
    fn a_loop_asking_for_two_speeds_breaks_one_block() {
        // Two shaft lines, joined by small cogwheels at one end (1 to -1) and by a large one
        // driving a small one at the other (1 to -2): the last cogwheel is asked for both.
        let x = Axis::X;
        let nodes = world([
            ([-1, 0, 0], motor(Dir::East, 16.0)),
            passive([0, 0, 0], Kind::Cogwheel(x)),
            passive([0, 1, 0], Kind::Cogwheel(x)),
            passive([1, 0, 0], Kind::Shaft(x)),
            passive([1, 1, 0], Kind::Shaft(x)),
            passive([2, 0, 0], Kind::LargeCogwheel(x)),
            passive([2, 1, 0], Kind::Cogwheel(x)),
            passive([2, 1, 1], Kind::Cogwheel(x)),
        ]);
        let solution = solve(&nodes);
        assert_eq!(solution.breaks.len(), 1);
        assert!(solution.speeds.values().all(|speed| *speed == 0.0));
        // Without the block that broke, the rest turns.
        let mut rest = nodes.clone();
        rest.remove(&solution.breaks[0]);
        let again = solve(&rest);
        assert!(again.breaks.is_empty());
        assert_eq!(again.speeds[&[1, 0, 0]], 16.0);
    }

    #[test]
    fn a_block_asked_past_the_limit_breaks() {
        let nodes = world([
            ([0, -1, 0], motor(Dir::Up, 256.0)),
            passive([0, 0, 0], Kind::LargeCogwheel(Axis::Y)),
            passive([1, 0, 1], Kind::Cogwheel(Axis::Y)),
        ]);
        assert_eq!(solve(&nodes).breaks, vec![[1, 0, 1]]);
    }

    #[test]
    fn more_stress_than_capacity_stops_the_network() {
        let mut consumer = Node::passive(Kind::Shaft(Axis::X));
        consumer.impact = 8.0;
        let mut weak = motor(Dir::East, 16.0);
        weak.capacity = 4.0;
        let nodes = world([([0, 0, 0], weak), ([1, 0, 0], consumer)]);
        let solution = solve(&nodes);
        assert!(solution.networks[0].overstressed);
        assert_eq!(solution.networks[0].capacity, 64.0);
        assert_eq!(solution.networks[0].stress, 128.0);
        assert_eq!(solution.speeds[&[1, 0, 0]], 0.0);
        assert!(solution.breaks.is_empty());

        let mut strong = weak;
        strong.capacity = 8.0;
        let nodes = world([([0, 0, 0], strong), ([1, 0, 0], consumer)]);
        assert_eq!(solve(&nodes).speeds[&[1, 0, 0]], 16.0);
    }

    #[test]
    fn names_round_trip() {
        for axis in Axis::ALL {
            assert_eq!(Axis::from_name(axis.name()), Some(axis));
        }
        for dir in Dir::ALL {
            assert_eq!(Dir::from_name(dir.name()), Some(dir));
            assert_eq!(Dir::from_normal(dir.normal()), Some(dir));
            assert_eq!(dir.opposite().opposite(), dir);
        }
    }
}
