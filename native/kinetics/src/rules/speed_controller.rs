//! The Rotation Speed Controller's place in the rotation network (`SpeedControllerBlockEntity`,
//! `SpeedControllerBlock`, `RotationPropagator.isLargeCogToSpeedController` and
//! `getConveyedSpeed`).
//!
//! The controller is an ordinary shaft through its two ends. A large cogwheel directly above it
//! on a horizontal axis across the controller's own is the output: while the controller's shaft
//! turns at all, that cogwheel (and everything it meshes with) turns at the controller's target
//! speed, whatever the input is and either way round. In Create the two sides are one
//! `KineticNetwork`, so what the output side costs is paid out of what the input side's
//! generators provide (stress is impact times the speed each block turns at, capacity is
//! per generated RPM). Here the cogwheel is made a generator of the target speed in a second
//! solve of the snapshot, and the two networks are then counted as one: their capacities and
//! stresses added, and everything stopped if the sum is over.
//!
//! Known differences from Create, for the README:
//! - Create's `getConveyedSpeed` also lets a cogwheel that something else drives *power the
//!   controller* (the speed flows backwards, to the controller's shaft); here the output only
//!   ever goes from the controller to the cogwheel.
//! - Create compares the conveyed speed with the cogwheel's present speed (`Math.max` of the two
//!   when they point the same way), so a faster motor on the output side wins; here the cogwheel
//!   is one more generator in its network, which is the same: the fastest sets the speed and two
//!   against each other jam.
//! - Create's targets are any whole number from -256 to 256 (a scroll value box); the Wrench's
//!   stand-in steps through [`TARGETS`].

use std::collections::{BTreeMap, BTreeSet};

use crate::{Axis, Dir, Ends, Joint, Kind, Network, Node, Pos, Solution, solve_once};

/// `SpeedControllerBlockEntity.DEFAULT_SPEED`.
pub const DEFAULT_SPEED: i32 = 16;
/// The scroll value's limits: `-maxRotationSpeed` to `maxRotationSpeed`.
pub const MAX_TARGET: i32 = 256;

/// The values the Wrench steps the target through (Create's scroll box takes any whole number in
/// range).
pub const TARGETS: [i32; 27] = [
    -256, -192, -128, -96, -64, -48, -32, -24, -16, -8, -4, -2, -1, 0, 1, 2, 4, 8, 16, 24, 32, 48,
    64, 96, 128, 192, 256,
];

/// The next target up (`up`) or down the list from `current`, stopping at the ends. A value that
/// isn't in the list (set with `/data`) goes to the next one in that direction.
pub fn step_target(current: i32, up: bool) -> i32 {
    let current = current.clamp(-MAX_TARGET, MAX_TARGET);
    if up {
        TARGETS.iter().copied().find(|t| *t > current).unwrap_or(MAX_TARGET)
    } else {
        TARGETS.iter().rev().copied().find(|t| *t < current).unwrap_or(-MAX_TARGET)
    }
}

/// `SpeedControllerBlock.getStateForPlacement`: a controller put under a large cogwheel on a
/// horizontal axis turns across it, its shaft along the other horizontal axis.
pub fn axis_under(cog_axis: Axis) -> Option<Axis> {
    match cog_axis {
        Axis::X => Some(Axis::Z),
        Axis::Z => Some(Axis::X),
        Axis::Y => None,
    }
}

/// `isLargeCogToSpeedController` for the cogwheel above a controller along `axis`: a large
/// cogwheel on a horizontal axis that isn't the controller's.
pub fn is_output_cog(controller: Axis, cog: Kind) -> bool {
    matches!(cog.cog(), Some((axis, true)) if axis != Axis::Y && axis != controller)
}

/// `ICogWheel.isLargeCog` of a block name: the large cogwheel and its two encased variants.
pub fn is_large_cog_block(name: &str) -> bool {
    name == "create:large_cogwheel" || super::encased::cog_kind(name) == Some(true)
}

/// `HorizontalAxisKineticBlock.getStateForPlacement` without a neighbour: the axis clockwise of the
/// way the player faces (`getHorizontalDirection().getClockWise().getAxis()`).
pub fn look_axis(facing: Dir) -> Axis {
    facing.clockwise_about(Axis::Y).axis()
}

/// `HorizontalAxisKineticBlock.getPreferredHorizontalAxis`: the axis the shaft ends beside the block
/// point along, when they agree (`has_shaft(side)`: the block on that horizontal side has a shaft
/// end pointing at this one).
pub fn preferred_horizontal_axis(mut has_shaft: impl FnMut(Dir) -> bool) -> Option<Axis> {
    let mut preferred: Option<Dir> = None;
    for side in [Dir::North, Dir::South, Dir::West, Dir::East] {
        if !has_shaft(side) {
            continue;
        }
        if preferred.is_some_and(|p| p.axis() != side.axis()) {
            return None;
        }
        preferred = Some(side);
    }
    preferred.map(Dir::axis)
}

/// `SpeedControllerBlock.getStateForPlacement`: across a large cogwheel above on a horizontal axis;
/// otherwise along the shaft ends beside it; otherwise by `look_axis`.
pub fn placement_axis(
    above: Option<(&str, Axis)>,
    preferred: Option<Axis>,
    facing: Dir,
) -> Axis {
    if let Some((block, axis)) = above
        && is_large_cog_block(block)
        && let Some(across) = axis_under(axis)
    {
        return across;
    }
    preferred.unwrap_or_else(|| look_axis(facing))
}

/// `SpeedControllerBlockEntity.isCogwheelPresent` for the client's bracket: the block above is
/// a (bare) large cogwheel on a horizontal axis.
pub fn has_bracket(block_above: &str, axis_above: Option<Axis>) -> bool {
    block_above == "create:large_cogwheel" && matches!(axis_above, Some(Axis::X | Axis::Z))
}

/// What `getDesiredOutputSpeed` comes to when the controller is powered through its shaft: the
/// target, as long as the shaft turns at all (`!hasSource` gives nothing, and a zero target none).
pub fn output_speed(target: i32, input_speed: f32) -> f32 {
    if target == 0 || input_speed == 0.0 {
        0.0
    } else {
        target as f32
    }
}

fn controllers(nodes: &BTreeMap<Pos, Node>) -> impl Iterator<Item = (Pos, Axis, i16)> + '_ {
    nodes.iter().filter_map(|(pos, node)| match node.kind {
        Kind::Shafts(Ends {
            joint: Joint::SpeedController { axis, target },
            ..
        }) => Some((*pos, axis, target)),
        _ => None,
    })
}

/// Where a controller's output cogwheel stands: the block above.
pub fn cog_pos(controller: Pos) -> Pos {
    [controller[0], controller[1] + 1, controller[2]]
}

/// Solves again with each working controller's cogwheel a generator of its target, and counts
/// the networks it joins as one. `plain` is [`solve_once`] of `nodes`.
pub fn resolve(nodes: &BTreeMap<Pos, Node>, plain: Solution) -> Solution {
    // The controllers that have something to give: a shaft that turns, a target, a cogwheel.
    let mut outputs: Vec<(Pos, Pos, f32)> = Vec::new();
    for (pos, axis, target) in controllers(nodes) {
        let cog = cog_pos(pos);
        let input = plain.theoretical.get(&pos).copied().unwrap_or(0.0);
        let Some(above) = nodes.get(&cog) else { continue };
        if !is_output_cog(axis, above.kind) || above.generated != 0.0 {
            continue;
        }
        let speed = output_speed(i32::from(target), input);
        if speed != 0.0 {
            outputs.push((pos, cog, speed));
        }
    }
    if outputs.is_empty() {
        return plain;
    }

    let mut driven = nodes.clone();
    for (_, cog, speed) in &outputs {
        if let Some(node) = driven.get_mut(cog) {
            // No capacity of its own: what the input side gives is counted below.
            node.generated = *speed;
            node.capacity = 0.0;
        }
    }
    let mut solution = solve_once(&driven);

    // The networks a controller joins: its input's and its cogwheel's.
    let mut parent: Vec<usize> = (0..solution.networks.len()).collect();
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for (pos, cog, _) in &outputs {
        let (Some(&a), Some(&b)) = (solution.network_of.get(pos), solution.network_of.get(cog)) else {
            continue;
        };
        let (a, b) = (find(&mut parent, a), find(&mut parent, b));
        if a != b {
            parent[a.max(b)] = a.min(b);
        }
    }
    let roots: Vec<usize> = (0..parent.len()).map(|i| find(&mut parent, i)).collect();
    let joined: BTreeSet<usize> = (0..roots.len())
        .filter(|i| roots.iter().filter(|root| **root == roots[*i]).count() > 1)
        .collect();
    if joined.is_empty() {
        return solution;
    }

    // One network for each group: added up, and stopped together.
    let mut groups: BTreeMap<usize, Network> = BTreeMap::new();
    for index in &joined {
        let root = roots[*index];
        let own = solution.networks[*index];
        let group = groups.entry(root).or_default();
        group.capacity += own.capacity;
        group.stress += own.stress;
    }
    for group in groups.values_mut() {
        group.overstressed = group.stress > group.capacity + 1e-3;
    }
    let members: Vec<(Pos, usize)> = solution
        .network_of
        .iter()
        .filter(|(_, index)| joined.contains(index))
        .map(|(pos, index)| (*pos, *index))
        .collect();
    for (pos, index) in members {
        let root = roots[index];
        let group = groups[&root];
        solution.network_of.insert(pos, root);
        // A network the sums say can carry it turns at what it would; one that can't, stands still.
        let theory = solution.theoretical.get(&pos).copied().unwrap_or(0.0);
        solution
            .speeds
            .insert(pos, if group.overstressed { 0.0 } else { theory });
    }
    for (root, group) in groups {
        solution.networks[root] = group;
    }
    solution
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Dir, solve};

    fn motor(rpm: f32) -> Node {
        Node {
            kind: Kind::Motor(Dir::East),
            generated: rpm,
            capacity: 100.0,
            impact: 0.0,
        }
    }

    fn passive(kind: Kind) -> Node {
        Node::passive(kind)
    }

    /// A motor at x=0, a shaft at 1, the controller (shaft along x) at 2 and a large cogwheel on
    /// z above it, with a small cogwheel meshed beside that.
    fn rig(target: i32, input: f32) -> BTreeMap<Pos, Node> {
        let mut load = passive(Kind::Cogwheel(Axis::Z));
        load.impact = 1.0;
        BTreeMap::from([
            ([0, 0, 0], motor(input)),
            ([1, 0, 0], passive(Kind::Shaft(Axis::X))),
            ([2, 0, 0], passive(Kind::speed_controller(Axis::X, target))),
            ([2, 1, 0], passive(Kind::LargeCogwheel(Axis::Z))),
            // A small cogwheel diagonal to the large one on the same axis: -2x its speed.
            ([3, 2, 0], load),
        ])
    }

    #[test]
    fn the_cogwheel_turns_at_the_target_whatever_the_input() {
        for (input, target) in [(16.0, 64), (16.0, 8), (128.0, 32), (16.0, -64)] {
            let solution = solve(&rig(target, input));
            assert_eq!(solution.speeds[&[1, 0, 0]], input);
            // The controller's own shaft turns at the input speed.
            assert_eq!(solution.speeds[&[2, 0, 0]], input, "{input} {target}");
            assert_eq!(solution.speeds[&[2, 1, 0]], target as f32, "{input} {target}");
            assert!(solution.breaks.is_empty());
        }
    }

    #[test]
    fn a_stopped_input_gives_nothing() {
        let solution = solve(&rig(64, 0.0));
        assert_eq!(solution.speeds[&[2, 1, 0]], 0.0);
        let mut zero = rig(0, 16.0);
        assert_eq!(solve(&zero).speeds[&[2, 1, 0]], 0.0);
        zero.remove(&[0, 0, 0]);
        assert_eq!(solve(&zero).speeds[&[2, 1, 0]], 0.0);
    }

    #[test]
    fn the_cogwheel_must_be_large_horizontal_and_across_the_controller() {
        let mut nodes = rig(64, 16.0);
        // Along the controller's own axis: no.
        nodes.insert([2, 1, 0], passive(Kind::LargeCogwheel(Axis::X)));
        assert_eq!(solve(&nodes).speeds[&[2, 1, 0]], 0.0);
        // Vertical: no.
        nodes.insert([2, 1, 0], passive(Kind::LargeCogwheel(Axis::Y)));
        assert_eq!(solve(&nodes).speeds[&[2, 1, 0]], 0.0);
        // A small one: no.
        nodes.insert([2, 1, 0], passive(Kind::Cogwheel(Axis::Z)));
        assert_eq!(solve(&nodes).speeds[&[2, 1, 0]], 0.0);
        // An encased large cogwheel counts (`ICogWheel.isLargeCog`).
        nodes.insert([2, 1, 0], passive(Kind::encased_cog(Axis::Z, true, true, true)));
        assert_eq!(solve(&nodes).speeds[&[2, 1, 0]], 64.0);
    }

    #[test]
    fn what_the_output_costs_comes_out_of_the_input_generator() {
        // The small cogwheel is diagonal to the large one: it turns at -2 x 64, and costs 1 su
        // per RPM = 128 su. The motor gives 100 per RPM at 16 RPM = 1600 su: plenty.
        let solution = solve(&rig(64, 16.0));
        assert_eq!(solution.speeds[&[3, 2, 0]], -128.0);
        let network = solution.networks[solution.network_of[&[0, 0, 0]]];
        assert_eq!(solution.network_of[&[0, 0, 0]], solution.network_of[&[2, 1, 0]]);
        assert_eq!(network.capacity, 1600.0);
        assert_eq!(network.stress, 128.0);
        assert!(!network.overstressed);

        // A faster output costs more than the input provides: everything stops, both sides.
        let slow = solve(&rig(256, 1.0));
        // 512 RPM on the small cog: 512 su against 100 su.
        assert!(slow.breaks.contains(&[3, 2, 0]) || slow.speeds[&[2, 1, 0]] == 0.0);
        let mut weak = rig(128, 4.0);
        weak.get_mut(&[0, 0, 0]).unwrap().capacity = 20.0;
        let solution = solve(&weak);
        let network = solution.networks[solution.network_of[&[0, 0, 0]]];
        assert_eq!(solution.network_of[&[0, 0, 0]], solution.network_of[&[3, 2, 0]]);
        // 256 RPM x 1 su against 20 x 4 = 80 su.
        assert!(network.overstressed);
        assert_eq!(network.stress, 256.0);
        assert!(solution.speeds.values().all(|speed| *speed == 0.0));
        // The goggles still read what each block would turn at.
        assert_eq!(solution.theoretical[&[2, 1, 0]], 128.0);
        assert_eq!(solution.theoretical[&[1, 0, 0]], 4.0);
    }

    #[test]
    fn a_faster_generator_on_the_output_side_sets_its_speed() {
        // A motor driving the small cogwheel at 200 RPM beats the target (the cogwheel is just
        // one more generator, `Math.max`): the large one turns at -100, the way -8 points.
        let mut nodes = rig(-8, 16.0);
        nodes.insert(
            [3, 2, 0],
            Node {
                kind: Kind::Cogwheel(Axis::Z),
                generated: 200.0,
                capacity: 10.0,
                impact: 0.0,
            },
        );
        let solution = solve(&nodes);
        assert_eq!(solution.speeds[&[3, 2, 0]], 200.0);
        assert_eq!(solution.speeds[&[2, 1, 0]], -100.0);
    }

    #[test]
    fn the_wrench_steps_the_target() {
        assert_eq!(step_target(16, true), 24);
        assert_eq!(step_target(16, false), 8);
        assert_eq!(step_target(1, false), 0);
        assert_eq!(step_target(0, false), -1);
        assert_eq!(step_target(256, true), 256);
        assert_eq!(step_target(-256, false), -256);
        // Between steps goes the way asked.
        assert_eq!(step_target(20, true), 24);
        assert_eq!(step_target(20, false), 16);
        assert_eq!(step_target(-9999, true), -192);
        assert_eq!(TARGETS.len(), 27);
        assert!(TARGETS.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn placement_and_the_bracket_follow_the_cogwheel() {
        assert_eq!(axis_under(Axis::X), Some(Axis::Z));
        assert_eq!(axis_under(Axis::Z), Some(Axis::X));
        assert_eq!(axis_under(Axis::Y), None);
        assert!(has_bracket("create:large_cogwheel", Some(Axis::X)));
        assert!(!has_bracket("create:large_cogwheel", Some(Axis::Y)));
        assert!(!has_bracket("create:andesite_encased_large_cogwheel", Some(Axis::X)));
        assert_eq!(output_speed(-64, 3.0), -64.0);
        assert_eq!(output_speed(64, 0.0), 0.0);
        assert_eq!(output_speed(0, 3.0), 0.0);
    }

    #[test]
    fn the_controller_is_placed_across_the_cogwheel_above_or_along_the_shafts() {
        // Under a large cogwheel on x: across it (z); on z: x; a vertical one doesn't count.
        let cog = "create:large_cogwheel";
        assert_eq!(placement_axis(Some((cog, Axis::X)), None, Dir::North), Axis::Z);
        assert_eq!(placement_axis(Some((cog, Axis::Z)), Some(Axis::X), Dir::North), Axis::X);
        assert_eq!(
            placement_axis(Some(("create:brass_encased_large_cogwheel", Axis::X)), None, Dir::East),
            Axis::Z
        );
        assert_eq!(placement_axis(Some((cog, Axis::Y)), None, Dir::North), Axis::X);
        assert_eq!(placement_axis(Some(("create:cogwheel", Axis::X)), None, Dir::North), Axis::X);
        // A small cogwheel or a stone above leaves it to the shafts, then the player: facing
        // north the clockwise axis is x; facing east, z.
        assert_eq!(placement_axis(None, Some(Axis::Z), Dir::North), Axis::Z);
        assert_eq!(placement_axis(None, None, Dir::North), Axis::X);
        assert_eq!(placement_axis(None, None, Dir::East), Axis::Z);
        assert!(is_large_cog_block("create:andesite_encased_large_cogwheel"));
        assert!(!is_large_cog_block("create:andesite_encased_cogwheel"));
        // Shaft ends on two different horizontal axes agree on nothing.
        assert_eq!(preferred_horizontal_axis(|s| s == Dir::East), Some(Axis::X));
        assert_eq!(preferred_horizontal_axis(|s| s == Dir::East || s == Dir::West), Some(Axis::X));
        assert_eq!(preferred_horizontal_axis(|s| s == Dir::East || s == Dir::North), None);
        assert_eq!(preferred_horizontal_axis(|_| false), None);
    }
}
