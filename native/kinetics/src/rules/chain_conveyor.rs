//! The Chain Conveyor (`ChainConveyorBlock`, `ChainConveyorBlockEntity`, `ChainConveyorPackage`,
//! `ChainConveyorRoutingTable`, `ChainConveyorConnectionHandler.validateAndConnect`,
//! `PackagePortTarget.ChainConveyorFrogportTarget`): a kinetic wheel that carries packages on
//! chains to other conveyors. Everything that decides something is here with no game types: the
//! geometry of the chains, a package's journey (looping around a conveyor's wheel, travelling
//! along a chain, handed to the next conveyor, taken off by a port), the routing table that
//! tells a package which chain leads to its address, the rules for joining two conveyors, and
//! where a Frogport's target sits. Positions are in blocks relative to the conveyor's own block
//! corner; a connection is the offset to the other conveyor's block. The glue is
//! `machines/chain_conveyor.rs`.
//!
//! Known differences from Create, for the README:
//! - Catnip's `AngleHelper` and `VecHelper` aren't published with the mod: [`shortest_angle_diff`]
//!   and [`rotate_y`] are written from their documented behaviour.
//! - The conveyors' `HashSet`/`HashMap` iteration order is undefined in Java; here a sorted order.
//! - `canAcceptMorePackages` counts `loopingPackages.size() + travellingPackages.size()` where the
//!   second is the map's number of chains that ever carried a box, not of boxes; that is kept.

use std::collections::{BTreeMap, BTreeSet};

use super::package::match_address;
use crate::Pos;

/// `CKinetics.maxChainConveyorLength`.
pub const MAX_LENGTH: f64 = 32.0;
/// `CKinetics.maxChainConveyorConnections`.
pub const MAX_CONNECTIONS: usize = 4;
/// `CLogistics.chainConveyorCapacity`.
pub const CAPACITY: usize = 20;
/// `CLogistics.packagePortRange`.
pub const PORT_RANGE: f64 = 5.0;
/// `calculateConnectionStats`'s `offBranchDistance`: how far round the wheel a chain leaves.
pub const OFF_BRANCH: f32 = 35.0;
/// `ChainConveyorBlockEntity.tick`'s wheel radius.
pub const RADIUS: f32 = 1.5;

/// `Mth.wrapDegrees`-free angle wrapping: `ChainConveyorBlockEntity.wrapAngle`, 0 to 360.
pub fn wrap_angle(angle: f32) -> f32 {
    let mut angle = angle % 360.0;
    if angle < 0.0 {
        angle += 360.0;
    }
    angle
}

/// `AngleHelper.getShortestAngleDiff(current, target)`: the signed angle from `current` to
/// `target` taking the short way, -180 to 180.
pub fn shortest_angle_diff(current: f32, target: f32) -> f32 {
    let (current, target) = (current % 360.0, target % 360.0);
    (((target - current) % 360.0 + 540.0) % 360.0) - 180.0
}

/// `VecHelper.rotate(vec, deg, Axis.Y)`: a turn about the vertical axis, so that `(0, 0, 1)`
/// goes to `(sin deg, 0, cos deg)`.
pub fn rotate_y(v: [f64; 3], degrees: f64) -> [f64; 3] {
    let (sin, cos) = degrees.to_radians().sin_cos();
    [v[0] * cos + v[2] * sin, v[1], v[2] * cos - v[0] * sin]
}

fn length(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn offset_length(p: Pos) -> f64 {
    length([f64::from(p[0]), f64::from(p[1]), f64::from(p[2])])
}

fn negate(p: Pos) -> Pos {
    [-p[0], -p[1], -p[2]]
}

/// `ChainConveyorBlockEntity.ConnectionStats`, relative to the block corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stats {
    pub tangent_angle: f32,
    pub chain_length: f32,
    pub start: [f64; 3],
    pub end: [f64; 3],
}

/// `calculateConnectionStats`: where the chain to `connection` leaves this wheel and reaches the
/// other. A wheel turning backwards (`reversed`) has its chains on the other side.
pub fn connection_stats(connection: Pos, reversed: bool) -> Stats {
    let sign = if reversed { -1.0 } else { 1.0 };
    let direction = (f64::from(connection[0]).atan2(f64::from(connection[2])).to_degrees()) as f32;
    let angle = wrap_angle(direction - OFF_BRANCH * sign);
    let opposite = wrap_angle(angle + 180.0 + 2.0 * OFF_BRANCH * sign);
    let rim = [0.0, 0.0, 1.25];
    let bottom_centre = [0.5, 0.0, 0.5];
    let at = |origin: [f64; 3], degrees: f32| {
        let turned = rotate_y(rim, f64::from(degrees));
        [origin[0] + turned[0], origin[1] + turned[1] + 6.0 / 16.0, origin[2] + turned[2]]
    };
    let start = at(bottom_centre, angle);
    let end = at(
        [0.5 + f64::from(connection[0]), f64::from(connection[1]), 0.5 + f64::from(connection[2])],
        opposite,
    );
    Stats { tangent_angle: angle, chain_length: length(sub(end, start)) as f32, start, end }
}

/// `ChainConveyorBlockEntity.getChainCost`: the chains a connection costs, about one for every
/// 2.5 blocks.
pub fn chain_cost(connection: Pos) -> i32 {
    ((offset_length(connection) / 2.5).round() as i32).max(1)
}

/// `getPackagePosition` of a package looping round the wheel.
pub fn loop_position(chain_position: f32) -> [f64; 3] {
    let turned = rotate_y([0.0, 6.0 / 16.0, 0.875], f64::from(chain_position));
    [0.5 + turned[0], turned[1], 0.5 + turned[2]]
}

/// `getPackagePosition` of a package travelling a chain.
pub fn travel_position(stats: &Stats, chain_position: f32) -> [f64; 3] {
    let direction = sub(stats.end, stats.start);
    let norm = length(direction);
    if norm == 0.0 {
        return stats.start;
    }
    let along = f64::from(chain_position.min(stats.chain_length));
    [
        stats.start[0] + direction[0] / norm * along,
        stats.start[1] + direction[1] / norm * along,
        stats.start[2] + direction[2] / norm * along,
    ]
}

/// `loopThresholdCrossed`: whether a package going round the wheel has passed `off_branch`
/// (the angle a chain or a port is at) between `previous` and `position`.
pub fn loop_threshold_crossed(position: f32, previous: f32, off_branch: f32, reversed: bool) -> bool {
    let sign = |x: f32| if x > 0.0 { 1 } else if x < 0.0 { -1 } else { 0 };
    let before = sign(shortest_angle_diff(off_branch, previous));
    let now = sign(shortest_angle_diff(off_branch, position));
    let not_crossed = before >= now && !reversed || before <= now && reversed;
    !not_crossed
}

// --- connecting two conveyors ----------------------------------------------------------------------

/// Why two conveyors can't be joined (`validateAndConnect`'s `fail`s, `chain_conveyor.*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    TooFar,
    TooClose,
    CannotConnectVertically,
    TooSteep,
    CannotAddMoreConnections,
    AlreadyConnected,
}

impl Reject {
    /// The text of `chain_conveyor.<key>` in Create's language file.
    pub fn text(self) -> &'static str {
        match self {
            Self::TooFar => "Too far away",
            Self::TooClose => "Too close",
            Self::CannotConnectVertically => "Cannot connect vertically",
            Self::TooSteep => "Too steep",
            Self::CannotAddMoreConnections => "Cannot add more connections",
            Self::AlreadyConnected => "Already connected",
        }
    }
}

/// `BlockPos.closerThan(other, distance)`.
fn closer_than(a: Pos, b: Pos, distance: f64) -> bool {
    let d = [f64::from(a[0] - b[0]), f64::from(a[1] - b[1]), f64::from(a[2] - b[2])];
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < distance * distance
}

/// `ChainConveyorConnectionHandler.validateAndConnect` for two conveyors: `first` was clicked
/// first, `second` now. `second_connections` are the second conveyor's connections (offsets).
pub fn validate_connection(first: Pos, second: Pos, second_connections: &BTreeSet<Pos>) -> Result<(), Reject> {
    if !closer_than(second, first, MAX_LENGTH) {
        return Err(Reject::TooFar);
    }
    if closer_than(second, first, 2.5) {
        return Err(Reject::TooClose);
    }
    let diff = [f64::from(second[0] - first[0]), f64::from(second[1] - first[1]), f64::from(second[2] - first[2])];
    let horizontal = (diff[0] * diff[0] + diff[2] * diff[2]).sqrt() - 1.5;
    if horizontal <= 0.0 {
        return Err(Reject::CannotConnectVertically);
    }
    if diff[1].abs() / horizontal > 1.0 {
        return Err(Reject::TooSteep);
    }
    if second_connections.len() >= MAX_CONNECTIONS {
        return Err(Reject::CannotAddMoreConnections);
    }
    if second_connections.contains(&[first[0] - second[0], first[1] - second[1], first[2] - second[2]]) {
        return Err(Reject::AlreadyConnected);
    }
    Ok(())
}

/// `ChainConveyorBlock.getStateForPlacement`: no conveyor within one block sideways of a new one
/// (the 3 by 3 around it on its own level).
pub fn can_place_beside(neighbours: &[Pos]) -> bool {
    !neighbours.iter().any(|offset| offset[1] == 0 && offset[0].abs() <= 1 && offset[2].abs() <= 1)
}

// --- routing -----------------------------------------------------------------------------------------

/// `ChainConveyorRoutingTable.ENTRY_TIMEOUT` and `PORT_ENTRY_TIMEOUT`.
pub const ENTRY_TIMEOUT: i32 = 100;
pub const PORT_ENTRY_TIMEOUT: i32 = 20;

/// `ChainConveyorRoutingTable.RoutingTableEntry`.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub port: String,
    pub distance: i32,
    pub next_connection: Pos,
    pub timeout: i32,
    pub end_of_route: bool,
}

impl Entry {
    fn copy_for_neighbour(&self, connection: Pos) -> Entry {
        Entry {
            port: self.port.clone(),
            distance: self.distance + 1,
            next_connection: negate(connection),
            timeout: ENTRY_TIMEOUT,
            end_of_route: false,
        }
    }
}

/// `ChainConveyorRoutingTable`: which chain a package for an address goes along.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RoutingTable {
    pub entries_by_distance: Vec<Entry>,
    pub last_update: i32,
    pub changed: bool,
}

impl RoutingTable {
    pub fn tick(&mut self) {
        for entry in &mut self.entries_by_distance {
            entry.timeout -= 1;
        }
        let before = self.entries_by_distance.len();
        self.entries_by_distance.retain(|e| e.timeout > 0);
        self.changed |= self.entries_by_distance.len() != before;
        self.last_update += 1;
    }

    pub fn should_advertise(&self) -> bool {
        self.changed || self.last_update > ENTRY_TIMEOUT - 20
    }

    /// `receivePortInfo`: a port on this conveyor (`connection` zero for the wheel itself).
    pub fn receive_port_info(&mut self, filter: &str, connection: Pos) {
        self.insert(Entry {
            port: filter.to_owned(),
            distance: if filter == "*" { 1000 } else { 0 },
            next_connection: connection,
            timeout: PORT_ENTRY_TIMEOUT,
            end_of_route: true,
        });
    }

    /// `getExitFor`: the chain a package takes, zero for none (it goes on circling).
    pub fn exit_for(&self, address: &str) -> Pos {
        for entry in &self.entries_by_distance {
            if match_address(address, &entry.port) {
                return entry.next_connection;
            }
        }
        [0, 0, 0]
    }

    /// `advertiseTo`: what this table knows goes to the table of the conveyor `connection` away.
    pub fn advertise_to(&self, connection: Pos, other: &mut RoutingTable) {
        let back = negate(connection);
        for entry in &self.entries_by_distance {
            if entry.end_of_route || connection != entry.next_connection {
                other.insert(entry.copy_for_neighbour(connection));
            }
        }
        other.entries_by_distance.retain(|e| !(e.timeout < ENTRY_TIMEOUT && !e.end_of_route && back == e.next_connection));
    }

    fn insert(&mut self, entry: Entry) {
        let mut target_index = 0;
        for i in 0..self.entries_by_distance.len() {
            let other = &mut self.entries_by_distance[i];
            if other.distance > entry.distance {
                // A new port, or a shorter route to a known one: it goes in here.
                break;
            }
            if other.port == entry.port {
                if other.distance == entry.distance && other.next_connection == entry.next_connection {
                    // The same route again: its life starts over.
                    other.timeout = ENTRY_TIMEOUT;
                }
                // A different route to a known port: the old entry is left to decay.
                return;
            }
            target_index = i + 1;
        }
        self.entries_by_distance.insert(target_index, entry);
        self.changed = true;
    }
}

// --- packages and the conveyor ------------------------------------------------------------------------

/// `ChainConveyorPackage`: a package on a conveyor. `position` is degrees round the wheel while
/// looping and blocks along the chain while travelling.
#[derive(Clone, Debug, PartialEq)]
pub struct Package<B> {
    pub position: f32,
    /// `PackageItem.getAddress` of the box.
    pub address: String,
    pub item: B,
    pub just_flipped: bool,
}

/// `ChainConveyorBlockEntity.ConnectedPort`: where a Frogport takes packages off.
#[derive(Clone, Debug, PartialEq)]
pub struct Port {
    pub position: f32,
    pub connection: Option<Pos>,
    pub filter: String,
}

/// What a conveyor asks of the world while it runs.
pub trait Env<B> {
    /// `exportToPort`: the port at `port` (an offset) takes the box; false when it is busy or full.
    fn export(&mut self, port: Pos, item: &B) -> bool;
    /// `notifyPortToAnticipate`.
    fn anticipate(&mut self, port: Pos);
    /// Whether a conveyor stands `connection` away (`level.getBlockEntity(..) instanceof ...`).
    fn conveyor_at(&self, connection: Pos) -> bool;
    /// `canAcceptMorePackagesFromOtherConveyor` of the conveyor `connection` away, true when
    /// there is none (the check only holds a box back for a conveyor that is full).
    fn accepts_from_other(&self, connection: Pos) -> bool;
}

/// What a tick moved off this conveyor: boxes for another conveyor.
#[derive(Clone, Debug, PartialEq)]
pub struct Tick<B> {
    /// `(connection, box)`: boxes at the end of a chain, to be added to the looping boxes of the
    /// conveyor `connection` away, their position already set.
    pub handoffs: Vec<(Pos, Package<B>)>,
    /// `(connection, box)`: boxes on a chain whose wheel reversed, to be added to the travelling
    /// boxes of the conveyor `connection` away under the connection `-connection`.
    pub flips: Vec<(Pos, Package<B>)>,
    /// A box was taken off, moved to a chain or flipped: send the conveyor's data.
    pub changed: bool,
}

impl<B> Default for Tick<B> {
    fn default() -> Self {
        Self { handoffs: Vec::new(), flips: Vec::new(), changed: false }
    }
}

/// A conveyor's state (`ChainConveyorBlockEntity`'s fields that matter to the rules).
#[derive(Clone, Debug, PartialEq)]
pub struct Conveyor<B> {
    pub connections: BTreeSet<Pos>,
    pub looping: Vec<Package<B>>,
    pub travelling: BTreeMap<Pos, Vec<Package<B>>>,
    pub loop_ports: BTreeMap<Pos, Port>,
    pub travel_ports: BTreeMap<Pos, Port>,
    pub routing: RoutingTable,
    pub reversed: bool,
}

impl<B> Default for Conveyor<B> {
    fn default() -> Self {
        Self {
            connections: BTreeSet::new(),
            looping: Vec::new(),
            travelling: BTreeMap::new(),
            loop_ports: BTreeMap::new(),
            travel_ports: BTreeMap::new(),
            routing: RoutingTable::default(),
            reversed: false,
        }
    }
}

impl<B: Clone> Conveyor<B> {
    /// `canAcceptMorePackages`.
    pub fn can_accept_more(&self) -> bool {
        self.looping.len() + self.travelling.len() < CAPACITY
    }

    /// `canAcceptMorePackagesFromOtherConveyor`.
    pub fn can_accept_from_other(&self) -> bool {
        self.looping.len() < CAPACITY
    }

    /// `addTravellingPackage`: only onto a chain this conveyor has.
    pub fn add_travelling(&mut self, boxed: Package<B>, connection: Pos) -> bool {
        if !self.connections.contains(&connection) {
            return false;
        }
        self.travelling.entry(connection).or_default().push(boxed);
        true
    }

    /// `addLoopingPackage`.
    pub fn add_looping(&mut self, boxed: Package<B>) {
        self.looping.push(boxed);
    }

    /// `removeConnectionTo`: the chain goes, and the boxes on it (returned: they drop).
    pub fn remove_connection(&mut self, connection: Pos) -> Option<Vec<Package<B>>> {
        if !self.connections.remove(&connection) {
            return None;
        }
        Some(self.travelling.remove(&connection).unwrap_or_default())
    }

    /// `tick`'s movement of packages, on the server, for a wheel turning at `rpm`
    /// (`getSpeed`): `speed / 360` blocks a tick along a chain and
    /// `speed / (pi * radius) * 360` degrees a tick round the wheel.
    pub fn tick(&mut self, rpm: f32, env: &mut dyn Env<B>) -> Tick<B> {
        let mut tick = Tick::default();
        // `prepareStats`: a wheel that turns backwards has its chains on the other side.
        let reversed_previously = self.reversed;
        if self.reversed != (rpm < 0.0) && rpm != 0.0 {
            self.reversed = rpm < 0.0;
        }
        if rpm == 0.0 {
            return tick;
        }
        let speed = rpm / 360.0;
        let distance_per_tick = speed.abs();
        let degrees_per_tick = speed / (std::f32::consts::PI * RADIUS) * 360.0;

        if reversed_previously != self.reversed {
            let offsets: Vec<Pos> = self.travelling.keys().copied().collect();
            for offset in offsets {
                if !env.conveyor_at(offset) {
                    continue;
                }
                let Some(list) = self.travelling.get_mut(&offset) else {
                    continue;
                };
                let mut kept = Vec::new();
                for mut boxed in list.drain(..) {
                    if boxed.just_flipped {
                        kept.push(boxed);
                        continue;
                    }
                    boxed.just_flipped = true;
                    let chain = offset_length(offset) as f32 - 22.0 / 16.0;
                    boxed.position = chain - boxed.position;
                    tick.flips.push((offset, boxed));
                }
                *list = kept;
            }
            tick.changed = true;
        }

        // Travelling along chains.
        let targets: Vec<Pos> = self.travelling.keys().copied().collect();
        for target in targets {
            let stats = connection_stats(target, self.reversed);
            let boxes = self.travelling.remove(&target).unwrap_or_default();
            let mut kept: Vec<Package<B>> = Vec::new();
            'travelling: for mut boxed in boxes {
                boxed.just_flipped = false;
                let previous = boxed.position;
                boxed.position = (boxed.position + distance_per_tick).min(stats.chain_length);
                let anticipate = (boxed.position + distance_per_tick * 4.0).min(stats.chain_length);

                for (port_at, port) in &self.travel_ports {
                    if previous > port.position {
                        continue;
                    }
                    if port.connection != Some(target) {
                        continue;
                    }
                    let not_at_position_yet = boxed.position < port.position;
                    if not_at_position_yet && anticipate < port.position {
                        continue;
                    }
                    if !match_address(&boxed.address, &port.filter) {
                        continue;
                    }
                    if not_at_position_yet {
                        env.anticipate(*port_at);
                        continue;
                    }
                    if !env.export(*port_at, &boxed.item) {
                        continue;
                    }
                    tick.changed = true;
                    continue 'travelling;
                }

                if boxed.position < stats.chain_length {
                    kept.push(boxed);
                    continue;
                }
                // The end of the chain: on to the other wheel, if it is there.
                if env.conveyor_at(target) {
                    let sign = if self.reversed { -1.0 } else { 1.0 };
                    boxed.position = wrap_angle(stats.tangent_angle + 180.0 + 2.0 * OFF_BRANCH * sign);
                    tick.handoffs.push((target, boxed));
                    tick.changed = true;
                } else {
                    kept.push(boxed);
                }
            }
            self.travelling.entry(target).or_default().extend(kept);
        }

        // Looping round the wheel.
        let boxes = std::mem::take(&mut self.looping);
        let mut kept: Vec<Package<B>> = Vec::new();
        'looping: for mut boxed in boxes {
            boxed.just_flipped = false;
            let previous = boxed.position;
            boxed.position = wrap_angle(boxed.position + degrees_per_tick);
            let anticipate = wrap_angle(boxed.position + degrees_per_tick * 4.0);

            for (port_at, port) in &self.loop_ports {
                let off_branch = port.position;
                let not_at_position_yet = !loop_threshold_crossed(boxed.position, previous, off_branch, self.reversed);
                if not_at_position_yet && !loop_threshold_crossed(anticipate, previous, off_branch, self.reversed) {
                    continue;
                }
                if !match_address(&boxed.address, &port.filter) {
                    continue;
                }
                if not_at_position_yet {
                    env.anticipate(*port_at);
                    continue;
                }
                if !env.export(*port_at, &boxed.item) {
                    continue;
                }
                tick.changed = true;
                continue 'looping;
            }

            let connections: Vec<Pos> = self.connections.iter().copied().collect();
            for connection in connections {
                if env.conveyor_at(connection) && !env.accepts_from_other(connection) {
                    continue;
                }
                let off_branch = connection_stats(connection, self.reversed).tangent_angle;
                if !loop_threshold_crossed(boxed.position, previous, off_branch, self.reversed) {
                    continue;
                }
                if self.routing.exit_for(&boxed.address) != connection {
                    continue;
                }
                boxed.position = 0.0;
                self.travelling.entry(connection).or_default().push(boxed);
                tick.changed = true;
                continue 'looping;
            }
            kept.push(boxed);
        }
        self.looping = kept;
        tick
    }
}

// --- where a Frogport attaches ---------------------------------------------------------------------------

/// `PackagePortTarget.ChainConveyorFrogportTarget`: the point on a conveyor a port works at.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    /// `relativePos`: the conveyor's offset from the port.
    pub relative_pos: Pos,
    /// `chainPos`: degrees round the wheel, or blocks along the chain.
    pub chain_pos: f32,
    /// `connection`: the chain, none for the wheel.
    pub connection: Option<Pos>,
    /// `flipped`: the wheel was turning backwards when it was set up.
    pub flipped: bool,
}

/// A conveyor near a port, as [`nearest_target`] needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    /// The conveyor's offset from the port.
    pub offset: Pos,
    pub connections: Vec<Pos>,
    pub reversed: bool,
}

/// The stand-in for the click on a chain that sets a Frogport's target (Create picks the point
/// on the client): the nearest point to the port on any conveyor's wheel or chains that is at or
/// above the port's bottom and within [`PORT_RANGE`] of it (`PackagePortTargetSelectionHandler
/// .validateDiff`). `None` when there is none.
pub fn nearest_target(candidates: &[Candidate]) -> Option<Target> {
    // The port's bottom centre is the origin; a conveyor at `offset` has its corner there.
    let source = [0.5, 0.0, 0.5];
    let mut best: Option<(f64, Target)> = None;
    let mut consider = |point: [f64; 3], target: Target| {
        let diff = sub(point, source);
        if diff[1] < 0.0 || length(diff) > PORT_RANGE {
            return;
        }
        let distance = length(diff);
        if best.as_ref().is_none_or(|(d, _)| distance < *d) {
            best = Some((distance, target));
        }
    };
    for candidate in candidates {
        let origin = [f64::from(candidate.offset[0]), f64::from(candidate.offset[1]), f64::from(candidate.offset[2])];
        let at = |p: [f64; 3]| [origin[0] + p[0], origin[1] + p[1], origin[2] + p[2]];
        // The wheel: the port's direction from the conveyor's centre, on the loop's circle.
        let dx = source[0] - (origin[0] + 0.5);
        let dz = source[2] - (origin[2] + 0.5);
        let angle = wrap_angle(dx.atan2(dz).to_degrees() as f32);
        consider(
            at(loop_position(angle)),
            Target { relative_pos: candidate.offset, chain_pos: angle, connection: None, flipped: candidate.reversed },
        );
        // Each chain: the nearest point along it.
        for connection in &candidate.connections {
            let stats = connection_stats(*connection, candidate.reversed);
            let direction = sub(stats.end, stats.start);
            let norm = length(direction);
            if norm == 0.0 {
                continue;
            }
            let unit = [direction[0] / norm, direction[1] / norm, direction[2] / norm];
            let from_start = sub(source, at(stats.start));
            let along = (from_start[0] * unit[0] + from_start[1] * unit[1] + from_start[2] * unit[2]).clamp(0.0, norm);
            consider(
                at(travel_position(&stats, along as f32)),
                Target {
                    relative_pos: candidate.offset,
                    chain_pos: along as f32,
                    connection: Some(*connection),
                    flipped: candidate.reversed,
                },
            );
        }
    }
    best.map(|(_, target)| target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct World {
        conveyors: BTreeSet<Pos>,
        full: BTreeSet<Pos>,
        busy_ports: BTreeSet<Pos>,
        exported: Vec<(Pos, &'static str)>,
        anticipated: Vec<Pos>,
    }

    impl Env<&'static str> for World {
        fn export(&mut self, port: Pos, item: &&'static str) -> bool {
            if self.busy_ports.contains(&port) {
                return false;
            }
            self.exported.push((port, item));
            true
        }
        fn anticipate(&mut self, port: Pos) {
            self.anticipated.push(port);
        }
        fn conveyor_at(&self, connection: Pos) -> bool {
            self.conveyors.contains(&connection)
        }
        fn accepts_from_other(&self, connection: Pos) -> bool {
            !self.full.contains(&connection)
        }
    }

    fn package(position: f32, address: &str, item: &'static str) -> Package<&'static str> {
        Package { position, address: address.to_owned(), item, just_flipped: false }
    }

    #[test]
    fn angles_wrap_and_measure_the_short_way() {
        assert_eq!(wrap_angle(370.0), 10.0);
        assert_eq!(wrap_angle(-10.0), 350.0);
        assert_eq!(shortest_angle_diff(10.0, 350.0), -20.0);
        assert_eq!(shortest_angle_diff(350.0, 10.0), 20.0);
        assert_eq!(shortest_angle_diff(0.0, 90.0), 90.0);
        assert!((shortest_angle_diff(0.0, 180.0).abs() - 180.0).abs() < 1e-4);
    }

    #[test]
    fn a_turn_about_y_takes_south_towards_east_for_a_quarter_turn() {
        let v = rotate_y([0.0, 1.0, 1.0], 90.0);
        assert!((v[0] - 1.0).abs() < 1e-9 && v[2].abs() < 1e-9 && v[1] == 1.0);
    }

    #[test]
    fn a_chain_leaves_35_degrees_off_the_direction_of_the_other_conveyor() {
        let stats = connection_stats([10, 0, 0], false);
        // East is 90 degrees; the chain starts 35 before it.
        assert!((stats.tangent_angle - 55.0).abs() < 1e-3);
        // Reversed, it starts on the other side.
        assert!((connection_stats([10, 0, 0], true).tangent_angle - 125.0).abs() < 1e-3);
        // Both ends sit a rim's width (1.25) from their wheels, 6 sixteenths up.
        assert!((stats.start[1] - 6.0 / 16.0).abs() < 1e-9);
        let from_centre = ((stats.start[0] - 0.5).powi(2) + (stats.start[2] - 0.5).powi(2)).sqrt();
        assert!((from_centre - 1.25).abs() < 1e-6);
        // The chains in the two directions are as long as each other.
        let back = connection_stats([-10, 0, 0], false);
        assert!((stats.chain_length - back.chain_length).abs() < 1e-4);
        assert!(stats.chain_length > 7.5 && stats.chain_length < 10.0);
    }

    #[test]
    fn a_chain_costs_a_chain_for_each_two_and_a_half_blocks() {
        assert_eq!(chain_cost([3, 0, 0]), 1);
        assert_eq!(chain_cost([10, 0, 0]), 4);
        assert_eq!(chain_cost([0, 0, 30]), 12);
        assert_eq!(chain_cost([1, 1, 0]), 1);
    }

    #[test]
    fn a_box_on_the_wheel_is_in_front_of_the_axle_and_on_a_chain_it_moves_along_it() {
        let p = loop_position(0.0);
        assert!((p[0] - 0.5).abs() < 1e-9 && (p[2] - 1.375).abs() < 1e-9 && (p[1] - 0.375).abs() < 1e-9);
        let stats = connection_stats([10, 0, 0], false);
        assert_eq!(travel_position(&stats, 0.0), stats.start);
        let end = travel_position(&stats, 100.0);
        assert!((end[0] - stats.end[0]).abs() < 1e-6 && (end[2] - stats.end[2]).abs() < 1e-6);
    }

    #[test]
    fn crossing_a_port_angle_is_noticed_once_in_the_direction_of_travel() {
        // Going forward past 90.
        assert!(loop_threshold_crossed(91.0, 89.0, 90.0, false));
        assert!(!loop_threshold_crossed(89.0, 87.0, 90.0, false));
        assert!(!loop_threshold_crossed(93.0, 91.0, 90.0, false));
        // Backwards past 90.
        assert!(loop_threshold_crossed(89.0, 91.0, 90.0, true));
        assert!(!loop_threshold_crossed(91.0, 89.0, 90.0, true));
        // Across the 360 seam.
        assert!(loop_threshold_crossed(1.0, 359.0, 0.0, false));
    }

    #[test]
    fn two_conveyors_join_by_create_s_rules() {
        let none = BTreeSet::new();
        assert_eq!(validate_connection([0, 0, 0], [10, 0, 0], &none), Ok(()));
        assert_eq!(validate_connection([0, 0, 0], [40, 0, 0], &none), Err(Reject::TooFar));
        assert_eq!(validate_connection([0, 0, 0], [2, 0, 0], &none), Err(Reject::TooClose));
        assert_eq!(validate_connection([0, 0, 0], [0, 5, 0], &none), Err(Reject::CannotConnectVertically));
        assert_eq!(validate_connection([0, 0, 0], [0, 6, 0], &none), Err(Reject::CannotConnectVertically));
        assert_eq!(validate_connection([0, 0, 0], [3, 10, 0], &none), Err(Reject::TooSteep));
        assert_eq!(validate_connection([0, 0, 0], [8, 4, 0], &none), Ok(()));
        let full: BTreeSet<Pos> = [[1, 1, 1], [2, 2, 2], [3, 3, 3], [4, 4, 4]].into_iter().collect();
        assert_eq!(validate_connection([0, 0, 0], [10, 0, 0], &full), Err(Reject::CannotAddMoreConnections));
        let already: BTreeSet<Pos> = [[-10, 0, 0]].into_iter().collect();
        assert_eq!(validate_connection([0, 0, 0], [10, 0, 0], &already), Err(Reject::AlreadyConnected));
    }

    #[test]
    fn a_conveyor_is_not_placed_beside_another_on_its_level() {
        assert!(can_place_beside(&[]));
        assert!(!can_place_beside(&[[1, 0, 1]]));
        assert!(can_place_beside(&[[2, 0, 0], [0, 1, 0]]));
    }

    #[test]
    fn routes_spread_from_a_port_down_the_chains_and_the_nearest_wins() {
        // A port "Farm" on B; A is 10 blocks west of B, C is 10 east of B.
        let mut a = RoutingTable::default();
        let mut b = RoutingTable::default();
        let mut c = RoutingTable::default();
        b.receive_port_info("Farm", [0, 0, 0]);
        b.advertise_to([-10, 0, 0], &mut a);
        b.advertise_to([10, 0, 0], &mut c);
        assert_eq!(a.exit_for("Farm"), [10, 0, 0]);
        assert_eq!(c.exit_for("Farm"), [-10, 0, 0]);
        // It goes on past A to a conveyor further west.
        let mut z = RoutingTable::default();
        a.advertise_to([-10, 0, 0], &mut z);
        assert_eq!(z.exit_for("Farm"), [10, 0, 0]);
        assert_eq!(z.entries_by_distance[0].distance, 2);
        // An address nobody serves goes on circling.
        assert_eq!(a.exit_for("Mine"), [0, 0, 0]);
        // A catch-all port is the last resort.
        b.receive_port_info("*", [0, 0, 0]);
        assert_eq!(b.entries_by_distance.last().unwrap().port, "*");
        assert_eq!(b.exit_for("Anything"), [0, 0, 0]);
    }

    #[test]
    fn a_route_decays_when_it_stops_being_advertised() {
        let mut table = RoutingTable::default();
        table.receive_port_info("Farm", [5, 0, 0]);
        for _ in 0..PORT_ENTRY_TIMEOUT {
            table.tick();
        }
        assert_eq!(table.exit_for("Farm"), [0, 0, 0]);
        assert!(table.changed);
    }

    #[test]
    fn a_known_route_is_refreshed_and_a_longer_one_is_ignored() {
        let mut table = RoutingTable::default();
        let entry = |distance, next| Entry { port: "Farm".into(), distance, next_connection: next, timeout: 50, end_of_route: false };
        table.insert(entry(2, [1, 0, 0]));
        table.insert(entry(2, [1, 0, 0]));
        assert_eq!(table.entries_by_distance.len(), 1);
        assert_eq!(table.entries_by_distance[0].timeout, ENTRY_TIMEOUT);
        table.insert(entry(3, [2, 0, 0]));
        assert_eq!(table.entries_by_distance.len(), 1);
        // A shorter one is added in front.
        table.insert(entry(1, [3, 0, 0]));
        assert_eq!(table.entries_by_distance[0].next_connection, [3, 0, 0]);
    }

    fn two_conveyors() -> (Conveyor<&'static str>, World) {
        let mut conveyor = Conveyor::default();
        conveyor.connections.insert([10, 0, 0]);
        conveyor.routing.receive_port_info("Farm", [10, 0, 0]);
        let mut world = World::default();
        world.conveyors.insert([10, 0, 0]);
        (conveyor, world)
    }

    #[test]
    fn a_looping_box_leaves_by_the_chain_that_leads_to_its_address() {
        let (mut conveyor, mut world) = two_conveyors();
        let off = connection_stats([10, 0, 0], false).tangent_angle;
        conveyor.looping.push(package(off - 1.0, "Farm", "box"));
        conveyor.looping.push(package(off - 1.0, "Elsewhere", "other"));
        // 16 RPM is 1.0185 degrees a tick: both boxes pass the chain's angle at once.
        conveyor.tick(16.0, &mut world);
        assert_eq!(conveyor.travelling[&[10, 0, 0]].len(), 1);
        assert_eq!(conveyor.travelling[&[10, 0, 0]][0].item, "box");
        assert_eq!(conveyor.travelling[&[10, 0, 0]][0].position, 0.0);
        // The other keeps circling.
        assert_eq!(conveyor.looping.len(), 1);
        assert_eq!(conveyor.looping[0].item, "other");
    }

    #[test]
    fn a_full_neighbour_holds_a_box_on_the_wheel() {
        let (mut conveyor, mut world) = two_conveyors();
        world.full.insert([10, 0, 0]);
        let off = connection_stats([10, 0, 0], false).tangent_angle;
        conveyor.looping.push(package(off - 1.0, "Farm", "box"));
        conveyor.tick(16.0, &mut world);
        assert_eq!(conveyor.looping.len(), 1);
        assert!(conveyor.travelling.get(&[10, 0, 0]).is_none_or(Vec::is_empty));
    }

    #[test]
    fn a_box_travels_the_chain_and_arrives_at_the_other_wheel() {
        let (mut conveyor, mut world) = two_conveyors();
        let stats = connection_stats([10, 0, 0], false);
        conveyor.travelling.entry([10, 0, 0]).or_default().push(package(stats.chain_length - 0.01, "Farm", "box"));
        let tick = conveyor.tick(16.0, &mut world);
        assert_eq!(tick.handoffs.len(), 1);
        let (to, boxed) = &tick.handoffs[0];
        assert_eq!(*to, [10, 0, 0]);
        // It starts on the other wheel at the angle the chain arrives at.
        let expected = wrap_angle(stats.tangent_angle + 180.0 + 70.0);
        assert!((boxed.position - expected).abs() < 1e-3);
        assert!(conveyor.travelling[&[10, 0, 0]].is_empty());
        assert!(tick.changed);
    }

    #[test]
    fn a_box_at_the_end_of_a_chain_with_no_wheel_beyond_it_waits() {
        let (mut conveyor, mut world) = two_conveyors();
        world.conveyors.clear();
        let stats = connection_stats([10, 0, 0], false);
        conveyor.travelling.entry([10, 0, 0]).or_default().push(package(stats.chain_length, "Farm", "box"));
        let tick = conveyor.tick(16.0, &mut world);
        assert!(tick.handoffs.is_empty());
        assert_eq!(conveyor.travelling[&[10, 0, 0]].len(), 1);
    }

    #[test]
    fn a_port_on_the_chain_takes_a_box_for_its_address_as_it_passes() {
        let (mut conveyor, mut world) = two_conveyors();
        conveyor.travel_ports.insert([0, 0, 2], Port { position: 5.0, connection: Some([10, 0, 0]), filter: "Farm*".into() });
        conveyor.travelling.entry([10, 0, 0]).or_default().push(package(4.96, "Farm 1", "mine"));
        conveyor.travelling.entry([10, 0, 0]).or_default().push(package(4.96, "Mill", "not mine"));
        let tick = conveyor.tick(16.0, &mut world);
        assert_eq!(world.exported, vec![([0, 0, 2], "mine")]);
        assert!(tick.changed);
        assert_eq!(conveyor.travelling[&[10, 0, 0]].len(), 1);
        assert_eq!(conveyor.travelling[&[10, 0, 0]][0].item, "not mine");
    }

    #[test]
    fn a_port_is_told_to_get_ready_when_a_box_is_close() {
        let (mut conveyor, mut world) = two_conveyors();
        conveyor.travel_ports.insert([0, 0, 2], Port { position: 5.0, connection: Some([10, 0, 0]), filter: "*".into() });
        // After this tick's move it is 0.106 short of the port, and four ticks ahead (4 * 0.044)
        // would pass it.
        conveyor.travelling.entry([10, 0, 0]).or_default().push(package(4.85, "Farm", "box"));
        conveyor.tick(16.0, &mut world);
        assert_eq!(world.anticipated, vec![[0, 0, 2]]);
        assert!(world.exported.is_empty());
    }

    #[test]
    fn a_busy_port_leaves_the_box_travelling() {
        let (mut conveyor, mut world) = two_conveyors();
        world.busy_ports.insert([0, 0, 2]);
        conveyor.travel_ports.insert([0, 0, 2], Port { position: 5.0, connection: Some([10, 0, 0]), filter: "*".into() });
        conveyor.travelling.entry([10, 0, 0]).or_default().push(package(4.99, "Farm", "box"));
        conveyor.tick(16.0, &mut world);
        assert_eq!(conveyor.travelling[&[10, 0, 0]].len(), 1);
    }

    #[test]
    fn a_wheel_port_takes_a_looping_box_at_its_angle() {
        let (mut conveyor, mut world) = two_conveyors();
        conveyor.loop_ports.insert([0, 0, 2], Port { position: 180.0, connection: None, filter: "Farm".into() });
        conveyor.looping.push(package(179.5, "Farm", "box"));
        conveyor.tick(16.0, &mut world);
        assert_eq!(world.exported, vec![([0, 0, 2], "box")]);
        assert!(conveyor.looping.is_empty());
    }

    #[test]
    fn nothing_moves_on_a_stopped_wheel() {
        let (mut conveyor, mut world) = two_conveyors();
        conveyor.looping.push(package(10.0, "Farm", "box"));
        let tick = conveyor.tick(0.0, &mut world);
        assert_eq!(conveyor.looping[0].position, 10.0);
        assert!(!tick.changed);
    }

    #[test]
    fn a_reversed_wheel_turns_its_boxes_round_and_sends_them_back_the_other_way() {
        let (mut conveyor, mut world) = two_conveyors();
        conveyor.travelling.entry([10, 0, 0]).or_default().push(package(3.0, "Farm", "box"));
        let tick = conveyor.tick(-16.0, &mut world);
        assert!(conveyor.reversed);
        assert_eq!(tick.flips.len(), 1);
        let (to, boxed) = &tick.flips[0];
        assert_eq!(*to, [10, 0, 0]);
        // `length - position`, with the 22 sixteenths of wheel taken off.
        assert!((boxed.position - (10.0 - 22.0 / 16.0 - 3.0)).abs() < 1e-5);
        assert!(boxed.just_flipped);
    }

    #[test]
    fn the_capacity_counts_the_looping_boxes_and_the_chains_with_boxes() {
        let (mut conveyor, _) = two_conveyors();
        assert!(conveyor.can_accept_more());
        for _ in 0..CAPACITY - 1 {
            conveyor.add_looping(package(0.0, "a", "x"));
        }
        assert!(conveyor.can_accept_more());
        conveyor.add_travelling(package(0.0, "a", "x"), [10, 0, 0]);
        assert!(!conveyor.can_accept_more());
        // A chain the conveyor doesn't have takes nothing.
        assert!(!conveyor.add_travelling(package(0.0, "a", "x"), [3, 3, 3]));
        assert!(!conveyor.can_accept_from_other() || conveyor.looping.len() < CAPACITY);
    }

    #[test]
    fn removing_a_chain_drops_the_boxes_on_it() {
        let (mut conveyor, _) = two_conveyors();
        conveyor.add_travelling(package(1.0, "a", "x"), [10, 0, 0]);
        let dropped = conveyor.remove_connection([10, 0, 0]).unwrap();
        assert_eq!(dropped.len(), 1);
        assert!(conveyor.connections.is_empty());
        assert_eq!(conveyor.remove_connection([10, 0, 0]), None);
    }

    #[test]
    fn a_frogport_attaches_to_the_nearest_point_above_it_within_range() {
        // A conveyor two blocks west of the port and at the same height: the port is on its wheel.
        let wheel = nearest_target(&[Candidate { offset: [-2, 0, 0], connections: vec![], reversed: false }]).unwrap();
        assert_eq!(wheel.connection, None);
        assert_eq!(wheel.relative_pos, [-2, 0, 0]);
        // Port east of the conveyor: 90 degrees (east).
        assert!((wheel.chain_pos - 90.0).abs() < 1e-3);
        // One with a chain passing near the port: the chain is nearer.
        let near = Candidate { offset: [-8, 0, 0], connections: vec![[16, 0, 0]], reversed: false };
        let chain = nearest_target(&[near]).unwrap();
        assert_eq!(chain.connection, Some([16, 0, 0]));
        assert!(chain.chain_pos > 5.0 && chain.chain_pos < 11.0);
        // Too far, and below.
        assert_eq!(nearest_target(&[Candidate { offset: [-9, 0, 0], connections: vec![], reversed: false }]), None);
        assert_eq!(nearest_target(&[Candidate { offset: [-2, -3, 0], connections: vec![], reversed: false }]), None);
    }
}
