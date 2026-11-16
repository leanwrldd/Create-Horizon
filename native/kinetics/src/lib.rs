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
/// One file per machine's rules (`tools/gen_modules.py` writes its `mod.rs`).
pub mod rules;
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

    /// Vanilla `Direction.getClockWise(Axis)`, the one turn every wrench and placement rule uses:
    /// a quarter turn of `self` about `axis`, unchanged when it lies along the axis. Y is clockwise
    /// seen from above (north to east to south to west), Z up to east to down to west
    /// (`getClockWiseZ`), and X up to south to down to north (`getClockWiseX`): X is not the mirror
    /// of Z, so a table written from memory is wrong half the time.
    pub const fn clockwise_about(self, axis: Axis) -> Self {
        if self.axis() as u8 == axis as u8 {
            return self;
        }
        match axis {
            Axis::Y => match self {
                Dir::North => Dir::East,
                Dir::East => Dir::South,
                Dir::South => Dir::West,
                _ => Dir::North,
            },
            Axis::X => match self {
                Dir::Up => Dir::South,
                Dir::South => Dir::Down,
                Dir::Down => Dir::North,
                _ => Dir::Up,
            },
            Axis::Z => match self {
                Dir::Up => Dir::East,
                Dir::East => Dir::Down,
                Dir::Down => Dir::West,
                _ => Dir::Up,
            },
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
    /// A machine with shaft ends on some faces and a rule for how they relate ([`Ends`]).
    /// What a press, a fan or a mixer is to its network ([`Kind::shafts`]: every end turning as
    /// one), and the casings of the transmission blocks ([`Kind::gearshift`], [`Kind::clutch`],
    /// [`Kind::encased_cog`], [`Kind::chain`]).
    Shafts(Ends),
}

/// A part of an encased chain drive (`ChainDriveBlock.Part`): which way along its connection
/// axis the chain runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChainPart {
    Start,
    Middle,
    End,
    /// Create's `NONE`: no chain to a neighbour.
    Alone,
}

impl ChainPart {
    pub const ALL: [ChainPart; 4] = [
        ChainPart::Start,
        ChainPart::Middle,
        ChainPart::End,
        ChainPart::Alone,
    ];

    /// The state's `part` value.
    pub const fn name(self) -> &'static str {
        match self {
            ChainPart::Start => "start",
            ChainPart::Middle => "middle",
            ChainPart::End => "end",
            ChainPart::Alone => "none",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == name)
    }
}

/// An encased chain drive or adjustable chain gearshift as the network sees it
/// (`ChainDriveBlock`, `ChainGearshiftBlockEntity`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chain {
    pub axis: Axis,
    /// The state's `axis_along_first`.
    pub along_first: bool,
    pub part: ChainPart,
    /// The redstone signal an adjustable chain gearshift reads, 0 to 15 (0 for a plain drive).
    pub signal: u8,
}

impl Chain {
    /// `ChainDriveBlock.getConnectionAxis`: the axis the chain runs along.
    pub const fn connection_axis(self) -> Axis {
        if self.along_first {
            if matches!(self.axis, Axis::X) {
                Axis::Y
            } else {
                Axis::X
            }
        } else if matches!(self.axis, Axis::Z) {
            Axis::Y
        } else {
            Axis::Z
        }
    }

    /// The part test of `ChainDriveBlock.areBlocksConnected`: whether this block's chain runs
    /// out of its `face`.
    pub fn runs_towards(self, face: Dir) -> bool {
        if face.axis() != self.connection_axis() {
            return false;
        }
        if face.sign() > 0 {
            matches!(self.part, ChainPart::Middle | ChainPart::Start)
        } else {
            matches!(self.part, ChainPart::Middle | ChainPart::End)
        }
    }

    /// `ChainGearshiftBlockEntity.getModifierForSignal`; a plain chain drive is 1.
    pub fn modifier(self) -> f32 {
        if self.signal == 0 {
            1.0
        } else {
            1.0 + (f32::from(self.signal) + 1.0) / 16.0
        }
    }
}

/// How the shaft ends of a [`Kind::Shafts`] block relate to one another and to its neighbours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Joint {
    /// Every end turns as one.
    Plain,
    /// A powered gearshift: the two ends on its axis turn opposite ways
    /// (`GearshiftBlockEntity.getRotationSpeedModifier`).
    Reversing,
    /// A powered clutch: it takes rotation from whichever side reaches it first and passes none
    /// on (`ClutchBlockEntity.getRotationSpeedModifier`). An unpowered one is a [`Kind::Shaft`].
    Disengaged,
    /// A cogwheel in a casing: meshes as a small or a large cogwheel on `axis`, with shaft ends
    /// only where `faces` says (`EncasedCogwheelBlock`).
    Cog { axis: Axis, large: bool },
    /// An encased chain drive: ends on its axis, and a chain to the next drive.
    Chain(Chain),
}

/// The shaft ends of a [`Kind::Shafts`] block: a bit per face, in [`Dir::ALL`] order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ends {
    pub faces: u8,
    pub joint: Joint,
}

impl Kind {
    /// A machine with a shaft end on each of `faces` and nothing else.
    pub fn shafts(faces: &[Dir]) -> Self {
        Kind::Shafts(Ends {
            faces: Self::face_bits(faces),
            joint: Joint::Plain,
        })
    }

    fn face_bits(faces: &[Dir]) -> u8 {
        faces.iter().fold(0, |bits, face| bits | 1 << *face as u8)
    }

    fn axis_ends(axis: Axis) -> [Dir; 2] {
        match axis {
            Axis::X => [Dir::West, Dir::East],
            Axis::Y => [Dir::Down, Dir::Up],
            Axis::Z => [Dir::North, Dir::South],
        }
    }

    /// A gearshift with redstone power on it (`GearshiftBlock`): unpowered it is a [`Kind::Shaft`].
    pub fn gearshift(axis: Axis) -> Self {
        Kind::Shafts(Ends {
            faces: Self::face_bits(&Self::axis_ends(axis)),
            joint: Joint::Reversing,
        })
    }

    /// A clutch with redstone power on it (`ClutchBlock`): unpowered it is a [`Kind::Shaft`].
    pub fn clutch(axis: Axis) -> Self {
        Kind::Shafts(Ends {
            faces: Self::face_bits(&Self::axis_ends(axis)),
            joint: Joint::Disengaged,
        })
    }

    /// An encased cogwheel (`EncasedCogwheelBlock`); `top` and `bottom` are its `top_shaft` and
    /// `bottom_shaft`, the shaft ends at the positive and the negative end of its axis.
    pub fn encased_cog(axis: Axis, large: bool, top: bool, bottom: bool) -> Self {
        let [negative, positive] = Self::axis_ends(axis);
        let mut faces = 0;
        if top {
            faces |= Self::face_bits(&[positive]);
        }
        if bottom {
            faces |= Self::face_bits(&[negative]);
        }
        Kind::Shafts(Ends {
            faces,
            joint: Joint::Cog { axis, large },
        })
    }

    /// An encased chain drive or adjustable chain gearshift.
    pub fn chain(chain: Chain) -> Self {
        Kind::Shafts(Ends {
            faces: Self::face_bits(&Self::axis_ends(chain.axis)),
            joint: Joint::Chain(chain),
        })
    }

    /// The cogwheel this block meshes as: its axis, and whether it is a large one
    /// (`ICogWheel.isSmallCog`, `isLargeCog`).
    pub fn cog(self) -> Option<(Axis, bool)> {
        match self {
            Kind::Cogwheel(axis) => Some((axis, false)),
            Kind::LargeCogwheel(axis) => Some((axis, true)),
            Kind::Shafts(Ends {
                joint: Joint::Cog { axis, large },
                ..
            }) => Some((axis, large)),
            _ => None,
        }
    }

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
            Kind::Shafts(ends) => ends.faces & (1 << face as u8) != 0,
        }
    }
}

/// How the shaft on `face` turns relative to the block's own speed.
///
/// Always 1 but for a gearbox, where it is Create's `getAxisModifier` made independent of which
/// face the power comes in by: opposite faces differ, and of the two shaft axes one counts its
/// positive face as 1 and the other its negative face.
pub fn face_sign(kind: Kind, face: Dir) -> f32 {
    // A powered gearshift reverses the end opposite the one power comes in by, whichever it is:
    // the two ends turn opposite ways (`GearshiftBlockEntity.getRotationSpeedModifier`).
    if let Kind::Shafts(Ends {
        joint: Joint::Reversing,
        ..
    }) = kind
    {
        return face.sign() as f32;
    }
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
                // Small cogwheels, bare or encased (`ICogWheel.isSmallCog`), side by side.
                _ if matches!((a.cog(), b.cog()), (Some((x, false)), Some((y, false)))
                    if x == y && face.axis() != x) =>
                {
                    Some(-1.0)
                }
                // Encased chain drives joined by their chain (`ChainDriveBlock.areBlocksConnected`
                // and `getRotationSpeedModifier`): `from` and `to` turn with speeds in the
                // inverse ratio of their modifiers.
                (
                    Kind::Shafts(Ends {
                        joint: Joint::Chain(ca),
                        ..
                    }),
                    Kind::Shafts(Ends {
                        joint: Joint::Chain(cb),
                        ..
                    }),
                ) if ca.connection_axis() == cb.connection_axis()
                    && (ca.runs_towards(face) || cb.runs_towards(face.opposite())) =>
                {
                    Some(ca.modifier() / cb.modifier())
                }
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
            _ if matches!((a.cog(), b.cog()), (Some((x, true)), Some((y, false)))
                if x == y && diff[x.index()] == 0) =>
            {
                Some(-2.0)
            }
            _ if matches!((a.cog(), b.cog()), (Some((x, false)), Some((y, true)))
                if x == y && diff[x.index()] == 0) =>
            {
                Some(-0.5)
            }
            _ if matches!((a.cog(), b.cog()), (Some((x, true)), Some((y, true))) if x != y) => {
                let ((x, _), (y, _)) = (a.cog().unwrap(), b.cog().unwrap());
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

/// A powered clutch (`ClutchBlock`).
fn is_cut(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Shafts(Ends {
            joint: Joint::Disengaged,
            ..
        })
    )
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-4 * a.abs().max(b.abs()).max(1.0)
}

/// Every block's speed. The same blocks always give the same answer: positions are walked in
/// order, so which block of a jammed network breaks doesn't depend on a hash.
pub fn solve(nodes: &BTreeMap<Pos, Node>) -> Solution {
    let mut solution = Solution::default();
    let mut seen = BTreeSet::new();
    // Generators first, and powered clutches last: a clutch belongs to the first network that
    // reaches it, which should be one that turns (see [`Joint::Disengaged`]).
    let cut = |pos: &Pos| is_cut(nodes[pos].kind);
    let mut order: Vec<Pos> = nodes
        .iter()
        .filter(|(pos, node)| node.generated != 0.0 && !cut(pos))
        .map(|(pos, _)| *pos)
        .collect();
    order.extend(nodes.keys().filter(|pos| !cut(pos)));
    order.extend(nodes.keys().filter(|pos| cut(pos)));
    for root in order {
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
            // A powered clutch is a dead end: it passes nothing on.
            if is_cut(kind) {
                continue;
            }
            for offset in offsets() {
                let next = add(pos, offset);
                let Some(node) = nodes.get(&next) else {
                    continue;
                };
                let Some(ratio) = ratio(kind, node.kind, offset) else {
                    continue;
                };
                // A powered clutch already in a network (this one or another) isn't joined again.
                if is_cut(node.kind) && seen.contains(&next) {
                    continue;
                }
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

    fn line(x: Axis, middle: Kind) -> BTreeMap<Pos, Node> {
        world([
            ([-2, 0, 0], motor(Dir::East, 16.0)),
            passive([-1, 0, 0], Kind::Shaft(x)),
            passive([0, 0, 0], middle),
            passive([1, 0, 0], Kind::Shaft(x)),
            passive([2, 0, 0], Kind::Shaft(x)),
        ])
    }

    #[test]
    fn a_powered_gearshift_reverses_whichever_side_the_power_comes_from() {
        let speeds = solve(&line(Axis::X, Kind::gearshift(Axis::X))).speeds;
        assert_eq!(speeds[&[-1, 0, 0]], 16.0);
        assert_eq!(speeds[&[1, 0, 0]], -16.0);
        assert_eq!(speeds[&[2, 0, 0]], -16.0);
        // From the other end the same: the far side turns against the near one.
        let mut nodes = world([
            ([-1, 0, 0], passive([0, 0, 0], Kind::Shaft(Axis::X)).1),
            ([0, 0, 0], passive([0, 0, 0], Kind::gearshift(Axis::X)).1),
            ([1, 0, 0], passive([0, 0, 0], Kind::Shaft(Axis::X)).1),
            ([2, 0, 0], motor(Dir::West, 16.0)),
        ]);
        let speeds = solve(&nodes).speeds;
        assert_eq!(speeds[&[1, 0, 0]], 16.0);
        assert_eq!(speeds[&[-1, 0, 0]], -16.0);
        // Unpowered it is a shaft.
        nodes.insert([0, 0, 0], Node::passive(Kind::Shaft(Axis::X)));
        assert_eq!(solve(&nodes).speeds[&[-1, 0, 0]], 16.0);
        // Not turned along its axis: no connection.
        assert_eq!(
            solve(&line(Axis::X, Kind::gearshift(Axis::Z))).speeds[&[1, 0, 0]],
            0.0
        );
    }

    #[test]
    fn a_powered_clutch_passes_rotation_only_from_the_side_that_turns() {
        let clutch = Kind::clutch(Axis::X);
        let speeds = solve(&line(Axis::X, clutch)).speeds;
        assert_eq!(speeds[&[-1, 0, 0]], 16.0);
        assert_eq!(speeds[&[0, 0, 0]], 16.0);
        assert_eq!(speeds[&[1, 0, 0]], 0.0);
        assert_eq!(speeds[&[2, 0, 0]], 0.0);
        // The motor on the other side: that side turns, this one stops.
        let mut nodes = line(Axis::X, clutch);
        nodes.remove(&[-2, 0, 0]);
        nodes.insert([3, 0, 0], motor(Dir::West, 32.0));
        let speeds = solve(&nodes).speeds;
        assert_eq!(speeds[&[1, 0, 0]], 32.0);
        assert_eq!(speeds[&[0, 0, 0]], 32.0);
        assert_eq!(speeds[&[-1, 0, 0]], 0.0);
        // Motors on both sides: each turns its own side, and nothing breaks.
        nodes.insert([-2, 0, 0], motor(Dir::East, -16.0));
        let solution = solve(&nodes);
        assert!(solution.breaks.is_empty());
        assert_eq!(solution.speeds[&[-1, 0, 0]], -16.0);
        assert_eq!(solution.speeds[&[1, 0, 0]], 32.0);
        // A clutch nothing reaches stands alone.
        let alone = world([passive([0, 0, 0], clutch)]);
        assert_eq!(solve(&alone).speeds[&[0, 0, 0]], 0.0);
    }

    #[test]
    fn an_encased_cogwheel_meshes_like_a_cogwheel_and_shows_only_its_shafts() {
        let y = Axis::Y;
        let encased = Kind::encased_cog(y, false, true, false);
        assert!(encased.has_shaft(Dir::Up));
        assert!(!encased.has_shaft(Dir::Down));
        assert!(!encased.has_shaft(Dir::East));
        let nodes = world([
            ([0, -1, 0], motor(Dir::Up, 16.0)),
            passive([0, 0, 0], Kind::Cogwheel(y)),
            passive([1, 0, 0], encased),
            passive([1, 1, 0], Kind::Shaft(y)),
            passive([1, -1, 0], Kind::Shaft(y)),
        ]);
        let speeds = solve(&nodes).speeds;
        assert_eq!(speeds[&[1, 0, 0]], -16.0);
        // Only its top end has a shaft.
        assert_eq!(speeds[&[1, 1, 0]], -16.0);
        assert_eq!(speeds[&[1, -1, 0]], 0.0);
        // Large and encased, driving a small one on the diagonal.
        let nodes = world([
            ([0, -1, 0], motor(Dir::Up, 16.0)),
            passive([0, 0, 0], Kind::encased_cog(y, true, false, false)),
            passive([1, 0, 1], Kind::encased_cog(y, false, false, false)),
        ]);
        assert_eq!(solve(&nodes).speeds[&[1, 0, 1]], 0.0);
        let nodes = world([
            ([0, -1, 0], motor(Dir::Up, 16.0)),
            passive([0, 0, 0], Kind::encased_cog(y, true, false, true)),
            passive([1, 0, 1], Kind::encased_cog(y, false, false, false)),
        ]);
        assert_eq!(solve(&nodes).speeds[&[1, 0, 1]], -32.0);
        // Stacked along the axis they only join through shafts.
        let stacked = |top| {
            world([
                ([0, -1, 0], motor(Dir::Up, 16.0)),
                passive([0, 0, 0], Kind::encased_cog(y, false, top, true)),
                passive([0, 1, 0], Kind::encased_cog(y, false, true, true)),
            ])
        };
        assert_eq!(solve(&stacked(true)).speeds[&[0, 1, 0]], 16.0);
        assert_eq!(solve(&stacked(false)).speeds[&[0, 1, 0]], 0.0);
    }

    fn drive(part: ChainPart, signal: u8) -> Kind {
        Kind::chain(Chain {
            axis: Axis::Y,
            along_first: true,
            part,
            signal,
        })
    }

    #[test]
    fn chain_drives_turn_together_and_a_gearshift_scales_the_far_one() {
        // Axis Y, along the first coordinate: the chain runs along X.
        assert_eq!(
            Chain {
                axis: Axis::Y,
                along_first: true,
                part: ChainPart::Start,
                signal: 0
            }
            .connection_axis(),
            Axis::X
        );
        let nodes = world([
            ([0, -1, 0], motor(Dir::Up, 16.0)),
            passive([0, 0, 0], drive(ChainPart::Start, 0)),
            passive([1, 0, 0], drive(ChainPart::Middle, 0)),
            passive([2, 0, 0], drive(ChainPart::End, 0)),
            passive([3, 0, 0], drive(ChainPart::Alone, 0)),
        ]);
        let speeds = solve(&nodes).speeds;
        assert_eq!(speeds[&[1, 0, 0]], 16.0);
        assert_eq!(speeds[&[2, 0, 0]], 16.0);
        // The next one has no chain to it.
        assert_eq!(speeds[&[3, 0, 0]], 0.0);
        // Signal 15 is a modifier of 2: the gearshift end turns at half the other's speed.
        let mut nodes = nodes;
        nodes.insert([2, 0, 0], passive([2, 0, 0], drive(ChainPart::End, 15)).1);
        let speeds = solve(&nodes).speeds;
        assert_eq!(speeds[&[2, 0, 0]], 8.0);
        // Chain drives side by side along their axis still share the shaft.
        let nodes = world([
            ([0, -1, 0], motor(Dir::Up, 16.0)),
            passive([0, 0, 0], drive(ChainPart::Alone, 0)),
            passive([0, 1, 0], drive(ChainPart::Alone, 0)),
        ]);
        assert_eq!(solve(&nodes).speeds[&[0, 1, 0]], 16.0);
        assert_eq!(drive_modifier(1), 1.0 + 2.0 / 16.0);
    }

    fn drive_modifier(signal: u8) -> f32 {
        Chain {
            axis: Axis::Y,
            along_first: false,
            part: ChainPart::Alone,
            signal,
        }
        .modifier()
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
