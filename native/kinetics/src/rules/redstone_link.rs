//! The Redstone Link's rules: the frequency network of `RedstoneLinkNetworkHandler` and
//! `LinkBehaviour` (links on the same pair of items share a signal within range), the two
//! frequency slots' places on the block (`RedstoneLinkFrequencySlot`) and the shape the block fills
//! (`AllShapes.REDSTONE_BRIDGE`). No game types: the game glue is `machines/redstone_link.rs`.
//!
//! Known differences from Create, for the README:
//! - Create keeps a `Set<IRedstoneLinkable>` per world and drops a link lazily, when an update
//!   finds it `!isAlive()`; here every link reports in each tick it runs (`Links::touch`) and the
//!   glue prunes the ones that stopped (`Links::prune`), which re-solves what they left.
//! - Create's `newPosition` flag (a loaded link ignores updates until it has re-registered) is not
//!   needed: a link takes part in the network from its first tick, and a loaded receiver keeps the
//!   `Receive` it saved until then.
//! - The frequency slots are told apart by the nearer of the two slot centres (the way
//!   `LinkHandler` picks one for a fake player) rather than by Create's small value-box hit test.

use std::collections::BTreeMap;

use crate::Pos;

/// `CLogistics.linkRange`: "Maximum possible range in blocks of redstone link connections", 256
/// by default (`RedstoneLinkNetworkHandler.withinRange` takes `closerThan`).
pub const LINK_RANGE: i64 = 256;

/// The signal a transmitter can give at most.
pub const MAX_SIGNAL: i32 = 15;

/// `RedstoneLinkNetworkHandler.Frequency`: the item and, for a dyed one, its colour. Items count
/// as the same frequency whatever other components they carry (`Frequency.equals`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Frequency {
    /// The item's id, empty for `Frequency.EMPTY`.
    pub item: String,
    /// `DataComponents.DYED_COLOR` rgb, -1 for none.
    pub color: i32,
}

impl Frequency {
    /// `Frequency.EMPTY`.
    pub fn empty() -> Self {
        Frequency { item: String::new(), color: -1 }
    }

    pub fn of(item: &str, color: i32) -> Self {
        Frequency { item: item.to_owned(), color }
    }

    pub fn is_empty(&self) -> bool {
        self.item.is_empty()
    }
}

/// `IRedstoneLinkable.getNetworkKey`: a `Couple` of the two slots' frequencies. Links with both
/// slots empty share a network of their own, as in Create.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key(pub Frequency, pub Frequency);

impl Key {
    pub fn new(first: Frequency, last: Frequency) -> Self {
        Key(first, last)
    }
}

/// `RedstoneLinkNetworkHandler.withinRange`: the same link, or `BlockPos.closerThan(range)`
/// (`distSqr < range * range`).
pub fn within_range(from: Pos, to: Pos) -> bool {
    if from == to {
        return true;
    }
    let d = |i: usize| i64::from(from[i]) - i64::from(to[i]);
    d(0) * d(0) + d(1) * d(1) + d(2) * d(2) < LINK_RANGE * LINK_RANGE
}

#[derive(Clone, Debug)]
struct Entry {
    pos: Pos,
    /// `IRedstoneLinkable.isListening`: a receiver.
    listening: bool,
    /// `getTransmittedStrength`: what a transmitter gives, 0 for a receiver.
    transmit: i32,
    /// What the network last told a receiver (`setReceivedStrength`).
    received: i32,
    /// The glue's tick count when it last reported in.
    seen: i64,
}

/// What a link is registered as, for [`Links::registered`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registration {
    pub key: Key,
    pub listening: bool,
}

/// Every link of every level the glue ticks, by level (a small index the glue chooses) and
/// network key: `RedstoneLinkNetworkHandler.connections`.
#[derive(Debug, Default)]
pub struct Links {
    networks: BTreeMap<(u8, Key), Vec<Entry>>,
    index: BTreeMap<(u8, Pos), Registration>,
}

impl Links {
    /// What the link at `pos` is registered under, if it is.
    pub fn registered(&self, level: u8, pos: Pos) -> Option<&Registration> {
        self.index.get(&(level, pos))
    }

    /// Whether any link is registered.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// `RedstoneLinkNetworkHandler.addToNetwork`: puts a link in its network (once: the network
    /// is a `LinkedHashSet`) and updates the network.
    pub fn add(&mut self, level: u8, key: &Key, pos: Pos, listening: bool, transmit: i32, now: i64) {
        let network = self.networks.entry((level, key.clone())).or_default();
        match network.iter_mut().find(|e| e.pos == pos) {
            Some(entry) => {
                entry.listening = listening;
                entry.transmit = if listening { 0 } else { transmit };
                entry.seen = now;
            }
            None => network.push(Entry {
                pos,
                listening,
                transmit: if listening { 0 } else { transmit },
                received: 0,
                seen: now,
            }),
        }
        self.index.insert(
            (level, pos),
            Registration { key: key.clone(), listening },
        );
        self.update_network_of(level, key, pos);
    }

    /// `RedstoneLinkNetworkHandler.removeFromNetwork`: takes a link out; an emptied network is
    /// dropped, otherwise what it left is worked out again.
    pub fn remove(&mut self, level: u8, key: &Key, pos: Pos) {
        self.index.remove(&(level, pos));
        let Some(network) = self.networks.get_mut(&(level, key.clone())) else {
            return;
        };
        network.retain(|e| e.pos != pos);
        if network.is_empty() {
            self.networks.remove(&(level, key.clone()));
            return;
        }
        self.update_network_of(level, key, pos);
    }

    /// `LinkBehaviour.notifySignalChange` after `RedstoneLinkBlockEntity.transmit`: the signal a
    /// transmitter gives changed.
    pub fn set_transmit(&mut self, level: u8, key: &Key, pos: Pos, strength: i32) {
        let Some(network) = self.networks.get_mut(&(level, key.clone())) else {
            return;
        };
        let Some(entry) = network.iter_mut().find(|e| e.pos == pos) else {
            return;
        };
        if entry.listening || entry.transmit == strength {
            return;
        }
        entry.transmit = strength;
        self.update_network_of(level, key, pos);
    }

    /// The signal a receiver has been told, if it is in the network.
    pub fn received(&self, level: u8, key: &Key, pos: Pos) -> Option<i32> {
        self.networks
            .get(&(level, key.clone()))?
            .iter()
            .find(|e| e.pos == pos)
            .map(|e| e.received)
    }

    /// The link at `pos` reports that it is still there (it ticked at `now`).
    pub fn touch(&mut self, level: u8, key: &Key, pos: Pos, now: i64) {
        if let Some(entry) = self
            .networks
            .get_mut(&(level, key.clone()))
            .and_then(|network| network.iter_mut().find(|e| e.pos == pos))
        {
            entry.seen = now;
        }
    }

    /// Takes out the links that haven't reported in for `max_age` ticks (`!isAlive`: their
    /// chunk unloaded or the block went), works out again what they left, and answers where
    /// they were.
    pub fn prune(&mut self, now: i64, max_age: i64) -> Vec<(u8, Pos)> {
        let stale: Vec<(u8, Key, Pos)> = self
            .networks
            .iter()
            .flat_map(|((level, key), network)| {
                network
                    .iter()
                    .filter(move |e| now - e.seen >= max_age)
                    .map(move |e| (*level, key.clone(), e.pos))
            })
            .collect();
        for (level, key, pos) in &stale {
            self.remove(*level, key, *pos);
        }
        stale.into_iter().map(|(level, _, pos)| (level, pos)).collect()
    }

    /// `RedstoneLinkNetworkHandler.updateNetworkOf`: the strongest transmitter within range of
    /// `actor` becomes the signal of every receiver within range of `actor` (and of the actor
    /// itself, if it is one), as Create works it out.
    pub fn update_network_of(&mut self, level: u8, key: &Key, actor: Pos) {
        let Some(network) = self.networks.get_mut(&(level, key.clone())) else {
            return;
        };
        let mut power = 0;
        for other in network.iter() {
            if !within_range(actor, other.pos) {
                continue;
            }
            if power < MAX_SIGNAL {
                power = power.max(other.transmit);
            }
        }
        for other in network.iter_mut() {
            if other.listening && within_range(actor, other.pos) {
                other.received = power;
            }
        }
    }
}

// --- the slots ----------------------------------------------------------------------------------

/// `AngleHelper.horizontalAngle`: south 0, east 90, north 180, west 270, 0 for up and down.
pub fn horizontal_angle(facing: [i32; 3]) -> f64 {
    match facing {
        [0, 0, 1] => 0.0,
        [1, 0, 0] => 90.0,
        [0, 0, -1] => 180.0,
        [-1, 0, 0] => 270.0,
        _ => 0.0,
    }
}

/// `RedstoneLinkFrequencySlot.getLocalOffset`: where a frequency slot's centre is in the block,
/// for a link facing `facing` (a unit normal). The first slot is the upper one on a wall and the
/// one further south when it lies flat.
pub fn slot_centre(facing: [i32; 3], first: bool) -> [f64; 3] {
    let voxel = |x: f64, y: f64, z: f64| [x / 16.0, y / 16.0, z / 16.0];
    if facing[1] == 0 {
        // `VecHelper.voxelSpace(8, 5.5, 3.01)`, the first slot 5/16 higher, then
        // `rotateHorizontally`: turned about the block's centre by `horizontalAngle(facing)`.
        let mut at = voxel(8.0, 5.5, 3.01);
        if first {
            at[1] += 5.0 / 16.0;
        }
        let angle = horizontal_angle(facing).to_radians();
        let (sin, cos) = angle.sin_cos();
        let (x, z) = (at[0] - 0.5, at[2] - 0.5);
        return [x * cos + z * sin + 0.5, at[1], z * cos - x * sin + 0.5];
    }
    // `voxelSpace(8, 3.01, 5.5)`, the first slot 5/16 further in z; a link facing down turns it
    // 180 degrees about x through the block's centre.
    let mut at = voxel(8.0, 3.01, 5.5);
    if first {
        at[2] += 5.0 / 16.0;
    }
    if facing[1] < 0 {
        at[1] = 1.0 - at[1];
        at[2] = 1.0 - at[2];
    }
    at
}

/// Which slot a click lands on: the one whose centre is nearer to where the click crossed the
/// front of the link (its two coordinates across the facing). True for the first slot.
pub fn first_slot_hit(facing: [i32; 3], local_hit: [f64; 3]) -> bool {
    let distance = |first: bool| {
        let centre = slot_centre(facing, first);
        (0..3)
            .filter(|&i| facing[i] == 0)
            .map(|i| (centre[i] - local_hit[i]).powi(2))
            .sum::<f64>()
    };
    distance(true) <= distance(false)
}

/// Where the eye ray `origin` + t * `dir` crosses the plane of the link's front, in the block's
/// own coordinates (`block` is its position), or `None` when it doesn't reach it. The front is
/// 3/16 from the side the link sits on (`AllShapes.REDSTONE_BRIDGE` is 3 texels deep).
pub fn front_hit(facing: [i32; 3], block: Pos, origin: [f64; 3], dir: [f64; 3]) -> Option<[f64; 3]> {
    let axis = (0..3).find(|&i| facing[i] != 0)?;
    if dir[axis].abs() < 1e-9 {
        return None;
    }
    let depth = 3.0 / 16.0;
    let plane = f64::from(block[axis]) + if facing[axis] > 0 { depth } else { 1.0 - depth };
    let t = (plane - origin[axis]) / dir[axis];
    if t < 0.0 {
        return None;
    }
    let mut local = [0.0; 3];
    for i in 0..3 {
        local[i] = origin[i] + dir[i] * t - f64::from(block[i]);
    }
    Some(local)
}

/// The direction a player looks along: `Entity.calculateViewVector` for `yaw` and `pitch` in
/// degrees.
pub fn view_vector(yaw: f32, pitch: f32) -> [f64; 3] {
    let (yaw, pitch) = (f64::from(yaw).to_radians(), f64::from(pitch).to_radians());
    [-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()]
}

// --- the shape ----------------------------------------------------------------------------------

/// `AllShapes.REDSTONE_BRIDGE`, in blocks, for a link facing `facing`: a tablet 3 texels deep on
/// the side it sits on, `VERTICAL_TABLET_SHAPE` = `cuboid(3, 1, -1, 13, 15, 3)` for the four sides
/// (turned as `forDirectional(SOUTH)` does) and `SQUARE_TABLET_SHAPE` = `cuboid(2, 2, -1, 14, 14, 3)`
/// for up and down (`withVerticalShapes`). Cut to the block's own cell: the 1 texel Create lets
/// stick out behind is left out.
pub fn shape(facing: &str) -> [f64; 6] {
    let s = |v: f64| v / 16.0;
    match facing {
        "south" => [s(3.0), s(1.0), 0.0, s(13.0), s(15.0), s(3.0)],
        "north" => [s(3.0), s(1.0), s(13.0), s(13.0), s(15.0), 1.0],
        "east" => [0.0, s(1.0), s(3.0), s(3.0), s(15.0), s(13.0)],
        "west" => [s(13.0), s(1.0), s(3.0), 1.0, s(15.0), s(13.0)],
        "up" => [s(2.0), 0.0, s(2.0), s(14.0), s(3.0), s(14.0)],
        _ => [s(2.0), s(13.0), s(2.0), s(14.0), 1.0, s(14.0)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(a: &str, b: &str) -> Key {
        let f = |n: &str| if n.is_empty() { Frequency::empty() } else { Frequency::of(n, -1) };
        Key::new(f(a), f(b))
    }

    #[test]
    fn a_receiver_hears_a_transmitter_on_the_same_frequencies() {
        let mut links = Links::default();
        let k = key("minecraft:stone", "minecraft:dirt");
        links.add(0, &k, [0, 0, 0], true, 0, 0);
        links.add(0, &k, [10, 0, 0], false, 0, 0);
        assert_eq!(links.received(0, &k, [0, 0, 0]), Some(0));
        links.set_transmit(0, &k, [10, 0, 0], 9);
        assert_eq!(links.received(0, &k, [0, 0, 0]), Some(9));
        links.set_transmit(0, &k, [10, 0, 0], 0);
        assert_eq!(links.received(0, &k, [0, 0, 0]), Some(0));
    }

    #[test]
    fn the_strongest_transmitter_wins() {
        let mut links = Links::default();
        let k = key("a", "");
        links.add(0, &k, [0, 0, 0], true, 0, 0);
        links.add(0, &k, [1, 0, 0], false, 4, 0);
        links.add(0, &k, [2, 0, 0], false, 11, 0);
        links.add(0, &k, [3, 0, 0], false, 7, 0);
        assert_eq!(links.received(0, &k, [0, 0, 0]), Some(11));
    }

    #[test]
    fn other_frequencies_and_other_levels_are_other_networks() {
        let mut links = Links::default();
        let (a, b) = (key("a", "b"), key("b", "a"));
        links.add(0, &a, [0, 0, 0], true, 0, 0);
        links.add(0, &b, [1, 0, 0], false, 15, 0);
        assert_eq!(links.received(0, &a, [0, 0, 0]), Some(0));
        links.add(1, &a, [2, 0, 0], false, 15, 0);
        assert_eq!(links.received(0, &a, [0, 0, 0]), Some(0));
        links.add(0, &a, [3, 0, 0], false, 5, 0);
        assert_eq!(links.received(0, &a, [0, 0, 0]), Some(5));
    }

    #[test]
    fn a_pair_of_empty_slots_is_a_network_too() {
        let mut links = Links::default();
        let k = key("", "");
        links.add(0, &k, [0, 0, 0], true, 0, 0);
        links.add(0, &k, [4, 0, 0], false, 15, 0);
        assert_eq!(links.received(0, &k, [0, 0, 0]), Some(15));
    }

    #[test]
    fn the_order_of_the_two_items_matters() {
        assert_ne!(key("a", "b"), key("b", "a"));
        assert_ne!(Frequency::of("a", -1), Frequency::of("a", 0xFF0000));
    }

    #[test]
    fn range_is_closer_than_256() {
        assert!(within_range([0, 0, 0], [255, 0, 0]));
        assert!(!within_range([0, 0, 0], [256, 0, 0]));
        assert!(!within_range([0, 0, 0], [200, 0, 200]));
        assert!(within_range([5, 5, 5], [5, 5, 5]));
        assert!(!within_range([-128, -128, -128], [128, 128, 128]));
    }

    #[test]
    fn a_receiver_out_of_range_of_the_actor_is_left_alone() {
        let mut links = Links::default();
        let k = key("a", "a");
        links.add(0, &k, [0, 0, 0], true, 0, 0);
        links.add(0, &k, [300, 0, 0], false, 15, 0);
        assert_eq!(links.received(0, &k, [0, 0, 0]), Some(0));
    }

    #[test]
    fn removing_the_transmitter_switches_the_receiver_off() {
        let mut links = Links::default();
        let k = key("a", "");
        links.add(0, &k, [0, 0, 0], true, 0, 0);
        links.add(0, &k, [5, 0, 0], false, 15, 0);
        assert_eq!(links.received(0, &k, [0, 0, 0]), Some(15));
        links.remove(0, &k, [5, 0, 0]);
        assert_eq!(links.received(0, &k, [0, 0, 0]), Some(0));
        assert!(links.registered(0, [5, 0, 0]).is_none());
        links.remove(0, &k, [0, 0, 0]);
        assert!(links.is_empty());
    }

    #[test]
    fn links_that_stop_reporting_are_pruned() {
        let mut links = Links::default();
        let k = key("a", "");
        links.add(0, &k, [0, 0, 0], true, 0, 10);
        links.add(0, &k, [5, 0, 0], false, 15, 10);
        links.touch(0, &k, [0, 0, 0], 12);
        assert!(links.prune(12, 3).is_empty());
        assert_eq!(links.prune(13, 3), vec![(0, [5, 0, 0])]);
        assert_eq!(links.received(0, &k, [0, 0, 0]), Some(0));
        assert_eq!(links.prune(15, 3), vec![(0, [0, 0, 0])]);
        assert!(links.is_empty());
    }

    #[test]
    fn adding_a_link_twice_keeps_one() {
        let mut links = Links::default();
        let k = key("a", "");
        links.add(0, &k, [0, 0, 0], false, 3, 0);
        links.add(0, &k, [0, 0, 0], false, 3, 1);
        links.add(0, &k, [1, 0, 0], true, 0, 1);
        assert_eq!(links.received(0, &k, [1, 0, 0]), Some(3));
        links.remove(0, &k, [0, 0, 0]);
        assert_eq!(links.received(0, &k, [1, 0, 0]), Some(0));
    }

    #[test]
    fn switching_a_link_to_a_receiver_stops_it_transmitting() {
        let mut links = Links::default();
        let k = key("a", "");
        links.add(0, &k, [0, 0, 0], false, 15, 0);
        links.add(0, &k, [3, 0, 0], true, 0, 0);
        assert_eq!(links.received(0, &k, [3, 0, 0]), Some(15));
        // The same block becomes a receiver: re-added with no transmission.
        links.remove(0, &k, [0, 0, 0]);
        links.add(0, &k, [0, 0, 0], true, 15, 0);
        assert_eq!(links.received(0, &k, [3, 0, 0]), Some(0));
    }

    #[test]
    fn slot_centres_follow_the_facing() {
        // Facing south: nothing turns; the first slot is the upper one, near the front of the
        // block's back wall.
        let first = slot_centre([0, 0, 1], true);
        let last = slot_centre([0, 0, 1], false);
        assert!((first[0] - 0.5).abs() < 1e-9 && (first[1] - 0.65625).abs() < 1e-9);
        assert!((first[2] - 3.01 / 16.0).abs() < 1e-9);
        assert!((last[1] - 0.34375).abs() < 1e-9);
        // Facing north: turned 180 degrees about the centre, so the slot is near z = 1.
        let north = slot_centre([0, 0, -1], true);
        assert!((north[2] - (1.0 - 3.01 / 16.0)).abs() < 1e-9);
        assert!((north[1] - 0.65625).abs() < 1e-9);
        // Facing east: the front is at x = 3/16 and the slots keep their heights.
        let east = slot_centre([1, 0, 0], false);
        assert!((east[0] - 3.01 / 16.0).abs() < 1e-9 && (east[2] - 0.5).abs() < 1e-9);
        // Lying flat, the first slot is further south; facing down it is mirrored.
        let up = slot_centre([0, 1, 0], true);
        assert!((up[1] - 3.01 / 16.0).abs() < 1e-9 && (up[2] - 0.65625).abs() < 1e-9);
        let down = slot_centre([0, -1, 0], true);
        assert!((down[1] - (1.0 - 3.01 / 16.0)).abs() < 1e-9 && (down[2] - (1.0 - 0.65625)).abs() < 1e-9);
    }

    #[test]
    fn the_upper_half_of_a_wall_link_is_the_first_slot() {
        assert!(first_slot_hit([0, 0, 1], [0.5, 0.8, 0.19]));
        assert!(!first_slot_hit([0, 0, 1], [0.5, 0.3, 0.19]));
        assert!(first_slot_hit([1, 0, 0], [0.19, 0.7, 0.5]));
        // Lying flat, the south half (up) is the first, the north half the last.
        assert!(first_slot_hit([0, 1, 0], [0.5, 0.19, 0.8]));
        assert!(!first_slot_hit([0, 1, 0], [0.5, 0.19, 0.2]));
    }

    #[test]
    fn the_ray_crosses_the_front_of_the_link() {
        // A link on the north side of a cell at the origin, facing south, looked at from the south.
        let hit = front_hit([0, 0, 1], [0, 0, 0], [0.5, 0.5, 4.5], [0.0, 0.0, -1.0]).unwrap();
        assert!((hit[0] - 0.5).abs() < 1e-9 && (hit[2] - 3.0 / 16.0).abs() < 1e-9);
        // A ray going away from it doesn't.
        assert!(front_hit([0, 0, 1], [0, 0, 0], [0.5, 0.5, 4.5], [0.0, 0.0, 1.0]).is_none());
        // Facing north the front is at z = 13/16.
        let hit = front_hit([0, 0, -1], [0, 0, 0], [0.5, 0.5, -3.5], [0.0, 0.0, 1.0]).unwrap();
        assert!((hit[2] - 13.0 / 16.0).abs() < 1e-9);
    }

    #[test]
    fn the_view_vector_points_where_the_player_looks() {
        let south = view_vector(0.0, 0.0);
        assert!((south[2] - 1.0).abs() < 1e-9);
        let west = view_vector(90.0, 0.0);
        assert!((west[0] + 1.0).abs() < 1e-9);
        let down = view_vector(0.0, 90.0);
        assert!((down[1] + 1.0).abs() < 1e-9);
    }

    #[test]
    fn the_shape_sits_on_the_side_it_is_attached_to() {
        assert_eq!(shape("south")[2], 0.0);
        assert!((shape("south")[5] - 3.0 / 16.0).abs() < 1e-9);
        assert_eq!(shape("north")[5], 1.0);
        assert_eq!(shape("west")[3], 1.0);
        assert_eq!(shape("east")[0], 0.0);
        assert_eq!(shape("up")[1], 0.0);
        assert_eq!(shape("down")[4], 1.0);
    }
}
