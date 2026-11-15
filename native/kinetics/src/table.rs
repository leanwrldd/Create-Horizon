//! The mod's table of kinetic blocks: what the server's block entities report each tick, and
//! the speeds the last solve gave them. No game types, so it is tested here.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::{Dir, Kind, Network, Node, Pos, solve};

/// `Direction.getNearest` of a player's look (`getNearestLookingDirection`).
pub fn looking(yaw: f32, pitch: f32) -> Dir {
    let (yaw, pitch) = (yaw.to_radians(), pitch.to_radians());
    let look = [
        -yaw.sin() * pitch.cos(),
        -pitch.sin(),
        yaw.cos() * pitch.cos(),
    ];
    let (axis, _) = look
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .expect("three components");
    match (axis, look[axis] > 0.0) {
        (0, true) => Dir::East,
        (0, false) => Dir::West,
        (1, true) => Dir::Up,
        (1, false) => Dir::Down,
        (_, true) => Dir::South,
        (_, false) => Dir::North,
    }
}

/// Every kinetic block the server ticked lately, by dimension, and what solving them gave.
#[derive(Default)]
pub struct Networks {
    /// Counts the server's running ticks.
    tick: u64,
    /// Each block with the tick it last reported in.
    nodes: HashMap<&'static str, BTreeMap<Pos, (Node, u64)>>,
    speeds: HashMap<&'static str, BTreeMap<Pos, f32>>,
    /// Blocks the last solve found jammed: each breaks in its next tick.
    breaks: HashMap<&'static str, BTreeSet<Pos>>,
    /// The network each block was in at the last solve, and what each added up to
    /// (`Solution::network_of`, `Solution::networks`).
    stress: HashMap<&'static str, (BTreeMap<Pos, usize>, Vec<Network>)>,
    /// A block appeared, changed or went away since the last solve.
    dirty: bool,
}

impl Networks {
    /// A block entity's tick: records the block and answers its speed and whether it breaks.
    pub fn report(&mut self, dimension: &'static str, pos: Pos, node: Node) -> (f32, bool) {
        let tick = self.tick;
        let nodes = self.nodes.entry(dimension).or_default();
        if nodes.insert(pos, (node, tick)).map(|(old, _)| old) != Some(node) {
            self.dirty = true;
        }
        let breaks = self
            .breaks
            .get_mut(dimension)
            .is_some_and(|breaks| breaks.remove(&pos));
        let speed = self
            .speeds
            .get(dimension)
            .and_then(|speeds| speeds.get(&pos))
            .copied()
            .unwrap_or(0.0);
        (speed, breaks)
    }

    /// What the block at `pos` last reported to be, if it ticked lately: a placing block
    /// reads its neighbours' shaft ends from it (`has_shaft`).
    pub fn kind_at(&self, dimension: &str, pos: Pos) -> Option<Kind> {
        Some(self.nodes.get(dimension)?.get(&pos)?.0.kind)
    }

    /// The speed the last solve gave the block at `pos`, in RPM.
    pub fn speed_at(&self, dimension: &str, pos: Pos) -> Option<f32> {
        self.speeds.get(dimension)?.get(&pos).copied()
    }

    /// The stress, capacity and overstress of the network the block at `pos` was in at the
    /// last solve: what Create's stressometer reads (`KineticNetwork`).
    pub fn network_at(&self, dimension: &str, pos: Pos) -> Option<Network> {
        let (network_of, networks) = self.stress.get(dimension)?;
        networks.get(*network_of.get(&pos)?).copied()
    }

    /// The start of a server tick: forgets blocks that stopped ticking (broken, or their chunk
    /// unloaded) and solves again if anything changed.
    pub fn advance(&mut self) {
        self.tick += 1;
        let tick = self.tick;
        for nodes in self.nodes.values_mut() {
            let before = nodes.len();
            nodes.retain(|_, (_, seen)| *seen + 1 >= tick);
            self.dirty |= nodes.len() != before;
        }
        if !std::mem::take(&mut self.dirty) {
            return;
        }
        self.speeds.clear();
        self.breaks.clear();
        self.stress.clear();
        for (&dimension, nodes) in &self.nodes {
            let nodes = nodes.iter().map(|(pos, (node, _))| (*pos, *node)).collect();
            let solution = solve(&nodes);
            self.breaks
                .insert(dimension, solution.breaks.into_iter().collect());
            self.speeds.insert(dimension, solution.speeds);
            self.stress
                .insert(dimension, (solution.network_of, solution.networks));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Axis, Kind};

    const MOTOR_CAPACITY: f32 = 16384.0;

    #[test]
    fn looking_follows_minecrafts_yaw_and_pitch() {
        assert_eq!(looking(0.0, 0.0), Dir::South);
        assert_eq!(looking(90.0, 0.0), Dir::West);
        assert_eq!(looking(180.0, 0.0), Dir::North);
        assert_eq!(looking(-90.0, 0.0), Dir::East);
        assert_eq!(looking(45.0, 80.0), Dir::Down);
        assert_eq!(looking(45.0, -80.0), Dir::Up);
    }

    fn motor() -> Node {
        Node {
            kind: Kind::Motor(Dir::East),
            generated: 16.0,
            capacity: MOTOR_CAPACITY,
            impact: 0.0,
        }
    }

    #[test]
    fn a_block_turns_the_tick_after_it_reports_and_stops_when_its_motor_goes() {
        let dimension = "minecraft:overworld";
        let shaft = Node::passive(Kind::Shaft(Axis::X));
        let mut networks = Networks::default();
        networks.advance();
        assert_eq!(networks.report(dimension, [0, 0, 0], motor()), (0.0, false));
        assert_eq!(networks.report(dimension, [1, 0, 0], shaft), (0.0, false));
        networks.advance();
        assert_eq!(
            networks.report(dimension, [0, 0, 0], motor()),
            (16.0, false)
        );
        assert_eq!(networks.report(dimension, [1, 0, 0], shaft), (16.0, false));
        // The motor is broken: it reports no more.
        networks.advance();
        assert_eq!(networks.report(dimension, [1, 0, 0], shaft), (16.0, false));
        networks.advance();
        assert_eq!(networks.report(dimension, [1, 0, 0], shaft), (0.0, false));
    }

    #[test]
    fn a_block_can_ask_what_its_neighbour_is_and_what_its_network_carries() {
        let dimension = "minecraft:overworld";
        let shaft = Node::passive(Kind::Shaft(Axis::X));
        let mut load = shaft;
        load.impact = 256.0;
        let mut networks = Networks::default();
        networks.advance();
        networks.report(dimension, [0, 0, 0], motor());
        networks.report(dimension, [1, 0, 0], load);
        networks.report(dimension, [2, 0, 0], shaft);
        assert_eq!(networks.network_at(dimension, [2, 0, 0]), None);
        networks.advance();
        assert_eq!(networks.kind_at(dimension, [1, 0, 0]), Some(Kind::Shaft(Axis::X)));
        assert_eq!(networks.kind_at(dimension, [5, 0, 0]), None);
        let network = networks.network_at(dimension, [2, 0, 0]).unwrap();
        assert_eq!(network.capacity, MOTOR_CAPACITY * 16.0);
        assert_eq!(network.stress, 256.0 * 16.0);
        assert!(!network.overstressed);
        assert_eq!(networks.network_at(dimension, [9, 9, 9]), None);
    }

    #[test]
    fn a_jammed_block_is_told_to_break_once() {
        let dimension = "minecraft:overworld";
        let mut against = motor();
        against.kind = Kind::Motor(Dir::West);
        against.generated = -64.0;
        let mut networks = Networks::default();
        networks.advance();
        networks.report(dimension, [0, 0, 0], motor());
        networks.report(dimension, [1, 0, 0], against);
        networks.advance();
        assert_eq!(networks.report(dimension, [0, 0, 0], motor()), (0.0, true));
        assert_eq!(networks.report(dimension, [0, 0, 0], motor()), (0.0, false));
    }
}
