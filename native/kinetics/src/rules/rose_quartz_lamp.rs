//! The Rose Quartz Lamp's rules (`RoseQuartzLampBlock`): it lights on a redstone signal, keeps
//! giving power afterwards, turns the lamps next to it off when it lights, and gives a comparator a
//! signal that falls with the distance to the lit one. No game types: the game glue is
//! `machines/rose_quartz_lamp.rs`.
//!
//! Known differences from Create, for the README:
//! - Create reacts to `neighborChanged` and schedules a tick; each lamp has a block entity here
//!   that reads `hasNeighborSignal` once a tick and runs the scheduled tick on the next one.
//! - The lamps of the cluster that a lighting lamp turns off are told to their neighbours at once,
//!   not a tick later.
//! - Vanilla's `getStateForPlacement` sets `POWERED` from the signal beside the lamp: that is done
//!   when it is placed (`place_block`), not in the placement state.

use std::collections::{BTreeSet, VecDeque};

use crate::Pos;

/// `RoseQuartzLampBlock.forEachInCluster`: lamps farther than this (`distManhattan`) from the one
/// that lit are not part of its cluster.
pub const CLUSTER_RADIUS: i32 = 16;

/// The six neighbours, in `Iterate.directions` order (down, up, north, south, west, east).
pub const DIRECTIONS: [Pos; 6] = [
    [0, -1, 0],
    [0, 1, 0],
    [0, 0, -1],
    [0, 0, 1],
    [-1, 0, 0],
    [1, 0, 0],
];

fn manhattan(a: Pos, b: Pos) -> i32 {
    (a[0] - b[0]).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs()
}

/// `RoseQuartzLampBlock.forEachInCluster`: every lamp connected to `origin` through lamps, within
/// 16 blocks of it by Manhattan distance, in the order a breadth first search finds them (the
/// origin itself is left out). `is_lamp` says whether the block at a position is a Rose Quartz Lamp.
pub fn cluster(origin: Pos, is_lamp: impl Fn(Pos) -> bool) -> Vec<Pos> {
    let mut found = Vec::new();
    let mut frontier = VecDeque::from([origin]);
    let mut visited = BTreeSet::from([origin]);
    while let Some(pos) = frontier.pop_front() {
        for d in DIRECTIONS {
            let at = [pos[0] + d[0], pos[1] + d[1], pos[2] + d[2]];
            if manhattan(at, origin) > CLUSTER_RADIUS {
                continue;
            }
            if !visited.insert(at) {
                continue;
            }
            if !is_lamp(at) {
                continue;
            }
            found.push(at);
            frontier.push_back(at);
        }
    }
    found
}

/// What a lamp's three states are: `POWERED` (a signal reaches it), `POWERING` (it gives power)
/// and `ACTIVATE` (the scheduled tick is to set `POWERING`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Lamp {
    pub powered: bool,
    pub powering: bool,
    pub activate: bool,
}

/// What the lamp's neighbours do about it: `neighborChanged` with the signal now beside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reaction {
    /// Nothing changes.
    None,
    /// The signal went away: `POWERED` is cleared (`POWERING` stays).
    Unpower,
    /// A signal arrived: the cluster is switched off and this lamp lights.
    Light,
}

/// `RoseQuartzLampBlock.neighborChanged`.
pub fn react(lamp: Lamp, signal: bool) -> Reaction {
    if lamp.powered == signal {
        Reaction::None
    } else if lamp.powered {
        Reaction::Unpower
    } else {
        Reaction::Light
    }
}

/// The state a lamp takes when it lights (`neighborChanged`): powered, powering and activating.
pub fn lit() -> Lamp {
    Lamp { powered: true, powering: true, activate: true }
}

/// `RoseQuartzLampBlock.tick`: where a lamp is after its scheduled tick, if the tick changes its
/// state (`wasPowering || shouldBePowering`): `ACTIVATE` is cleared and `POWERING` takes what
/// `ACTIVATE` said.
pub fn scheduled_tick(lamp: Lamp) -> Option<Lamp> {
    if lamp.powering || lamp.activate {
        Some(Lamp { activate: false, powering: lamp.activate, ..lamp })
    } else {
        None
    }
}

/// `RoseQuartzLampBlock.getDistanceToPowered`: a comparator reads 15 on the lit lamp's own
/// side and one less for each lamp further along `column`, up to 15 lamps. `lamp_at` gives, for
/// each position starting at the lamp itself and moving along `column`, whether there is a lamp
/// there and whether it is powering.
pub fn distance_to_powered(mut lamp_at: impl FnMut(i32) -> Option<bool>) -> i32 {
    for (step, power) in (1..=15).rev().enumerate() {
        match lamp_at(step as i32) {
            None => return 0,
            Some(true) => return power,
            Some(false) => {}
        }
    }
    0
}

/// `RoseQuartzLampBlock.getSignal` for a block `reader` the lamp powers: 0 for another lamp, the
/// distance for a comparator, else 15 when powering.
pub fn signal(powering: bool, reader_is_lamp: bool, reader_is_comparator: bool, distance: i32) -> i32 {
    if reader_is_lamp {
        0
    } else if reader_is_comparator {
        distance
    } else if powering {
        15
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signal_lights_an_unpowered_lamp_and_dims_nothing_else() {
        let off = Lamp::default();
        assert_eq!(react(off, false), Reaction::None);
        assert_eq!(react(off, true), Reaction::Light);
        let on = lit();
        assert_eq!(on, Lamp { powered: true, powering: true, activate: true });
        assert_eq!(react(on, true), Reaction::None);
        assert_eq!(react(on, false), Reaction::Unpower);
    }

    #[test]
    fn the_scheduled_tick_settles_the_activation() {
        let on = lit();
        assert_eq!(
            scheduled_tick(on),
            Some(Lamp { powered: true, powering: true, activate: false })
        );
        // A lamp that was switched off by the cluster keeps being off.
        let dimmed = Lamp { powered: true, powering: false, activate: false };
        assert_eq!(scheduled_tick(dimmed), None);
        // A lit lamp ticked without `activate` goes out (it was never meant to stay).
        let lit_idle = Lamp { powered: true, powering: true, activate: false };
        assert_eq!(
            scheduled_tick(lit_idle),
            Some(Lamp { powered: true, powering: false, activate: false })
        );
    }

    #[test]
    fn the_cluster_is_the_lamps_joined_to_the_origin_within_16_blocks() {
        // A row of lamps along x from 1 to 20, and a lamp that touches nothing.
        let is_lamp = |p: Pos| (p[1] == 0 && p[2] == 0 && (1..=20).contains(&p[0])) || p == [0, 5, 5];
        let found = cluster([0, 0, 0], is_lamp);
        assert_eq!(found.len(), 16);
        assert_eq!(found[0], [1, 0, 0]);
        assert_eq!(*found.last().unwrap(), [16, 0, 0]);
        assert!(!found.contains(&[0, 5, 5]));
        assert!(!found.contains(&[17, 0, 0]));
    }

    #[test]
    fn the_cluster_runs_through_corners_and_leaves_the_origin_out() {
        let is_lamp = |p: Pos| matches!(p, [1, 0, 0] | [1, 1, 0] | [1, 1, 1] | [0, 0, 0]);
        let found = cluster([0, 0, 0], is_lamp);
        assert_eq!(found, vec![[1, 0, 0], [1, 1, 0], [1, 1, 1]]);
    }

    #[test]
    fn a_comparator_reads_the_distance_to_a_lit_lamp() {
        // Lamps at steps 0 to 4; the lit one is step 2: 15 - 2 = 13.
        let lamps = [false, false, true, false, false];
        assert_eq!(
            distance_to_powered(|step| lamps.get(step as usize).copied()),
            13
        );
        // The first lamp is lit: 15. No lamp where one should be: 0.
        assert_eq!(distance_to_powered(|step| (step < 3).then_some(step == 0)), 15);
        assert_eq!(distance_to_powered(|_| None), 0);
        // Fifteen unlit lamps in a row: nothing.
        assert_eq!(distance_to_powered(|_| Some(false)), 0);
    }

    #[test]
    fn the_signal_is_15_except_into_lamps_and_comparators() {
        assert_eq!(signal(true, false, false, 0), 15);
        assert_eq!(signal(false, false, false, 0), 0);
        assert_eq!(signal(true, true, false, 0), 0);
        assert_eq!(signal(true, false, true, 9), 9);
        assert_eq!(signal(false, false, true, 0), 0);
    }
}
