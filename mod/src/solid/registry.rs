//! The structures that are solid right now: one entry per running controller, per side.
//!
//! A controller reports its structure each tick it runs (`put`); the passes read the table
//! (`active`). The server and a client share a process in single player, so the two sides keep
//! their own entries.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use create_kinetics::rules::collider::StructurePose;
use create_kinetics::rules::solid::Solid;
use create_kinetics::Pos;

use super::{STAY, Sample};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Server,
    Client,
}

/// A running structure.
#[derive(Clone)]
pub struct Entry {
    pub solid: Arc<Solid>,
    /// `Contraption.anchor`.
    pub anchor: Pos,
    /// Where it was at `seen` (game time) and a tick before.
    pub sample: Sample,
    pub seen: i64,
    /// What `solid` was built from: a new one rebuilds it.
    pub token: u64,
}

impl Entry {
    /// The structure's pose at game time `now` (`StructurePose`). A structure no controller
    /// reported this tick has stood still since it did.
    pub fn pose(&self, now: i64) -> StructurePose {
        let stale = now > self.seen;
        match self.sample {
            Sample::Slide { now: at, prev } => {
                let prev = if stale { at } else { prev };
                StructurePose::translating(at, prev, [at[0] - prev[0], at[1] - prev[1], at[2] - prev[2]])
            }
            Sample::Turn { axis, now: angle, prev } => {
                let prev = if stale { angle } else { prev };
                StructurePose::turning(self.anchor.map(f64::from), axis, angle, prev)
            }
        }
    }
}

type Key = (Side, String, Pos);

static ENTRIES: LazyLock<Mutex<HashMap<Key, Entry>>> = LazyLock::new(Mutex::default);

/// The newest game time any client entry was reported at: the client's "now" for its passes.
static CLIENT_NOW: AtomicI64 = AtomicI64::new(i64::MIN);

fn table() -> MutexGuard<'static, HashMap<Key, Entry>> {
    ENTRIES.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The token of the entry kept for a controller, if any.
pub fn token(side: Side, dimension: &str, pos: Pos) -> Option<(u64, Arc<Solid>)> {
    table().get(&(side, dimension.to_owned(), pos)).map(|e| (e.token, e.solid.clone()))
}

/// Reports a structure.
pub fn put(side: Side, dimension: &str, pos: Pos, entry: Entry) {
    if side == Side::Client {
        CLIENT_NOW.fetch_max(entry.seen, Ordering::Relaxed);
    }
    table().insert((side, dimension.to_owned(), pos), entry);
}

/// A controller that no longer runs a structure (it was set down, or the block went away).
pub fn forget(side: Side, dimension: &str, pos: Pos) {
    table().remove(&(side, dimension.to_owned(), pos));
}

/// Takes a controller's entry out (a structure that was just set down), for the pose it was last in.
pub fn take(side: Side, dimension: &str, pos: Pos) -> Option<Entry> {
    table().remove(&(side, dimension.to_owned(), pos))
}

/// Both sides forget a controller: the server's set-down.
pub fn forget_both(dimension: &str, pos: Pos) {
    let mut t = table();
    t.remove(&(Side::Server, dimension.to_owned(), pos));
    t.remove(&(Side::Client, dimension.to_owned(), pos));
}

/// The client's game time as the registry last saw it.
pub fn client_now() -> Option<i64> {
    let now = CLIENT_NOW.load(Ordering::Relaxed);
    (now != i64::MIN).then_some(now)
}

/// The structures of a side and dimension that are still solid at `now`; the entries long gone are
/// dropped on the way.
pub fn active(side: Side, dimension: &str, now: i64) -> Vec<(Pos, Entry)> {
    let mut t = table();
    let mut out = Vec::new();
    t.retain(|(s, d, pos), entry| {
        if *s != side {
            return true;
        }
        let age = now - entry.seen;
        if age > 20 * STAY {
            return false;
        }
        if d == dimension && age <= STAY {
            out.push((*pos, entry.clone()));
        }
        true
    });
    out
}

/// Whether the client has any structure to collide with: one cheap check for the mixin's every tick.
pub fn client_any() -> bool {
    table().keys().any(|(side, ..)| *side == Side::Client)
}

#[cfg(test)]
mod tests {
    use super::*;
    use create_kinetics::Axis;
    use create_kinetics::rules::solid::Solid;

    fn entry(sample: Sample, seen: i64) -> Entry {
        Entry { solid: Arc::new(Solid::build(&[], None)), anchor: [1, 2, 3], sample, seen, token: 0 }
    }

    #[test]
    fn a_slide_is_a_translating_pose_with_its_step() {
        let e = entry(Sample::Slide { now: [0.0, 1.1, 0.0], prev: [0.0, 1.0, 0.0] }, 10);
        let pose = e.pose(10);
        assert!(pose.translating);
        assert!((pose.delta_movement[1] - 0.1).abs() < 1e-12);
        assert_eq!(pose.prev_position, [0.0, 1.0, 0.0]);
    }

    #[test]
    fn a_structure_nobody_reported_this_tick_stands_still() {
        let e = entry(Sample::Slide { now: [0.0, 1.1, 0.0], prev: [0.0, 1.0, 0.0] }, 10);
        let pose = e.pose(11);
        assert_eq!(pose.delta_movement, [0.0; 3]);
        assert_eq!(pose.prev_position, pose.position);
        let e = entry(Sample::Turn { axis: Axis::Y, now: 20.0, prev: 19.0 }, 10);
        let pose = e.pose(11);
        assert!(!pose.translating);
        assert_eq!(pose.position, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn the_table_keeps_sides_apart_and_forgets() {
        let e = entry(Sample::Slide { now: [0.0; 3], prev: [0.0; 3] }, 5);
        put(Side::Server, "test:dim", [9, 9, 9], e.clone());
        put(Side::Client, "test:dim", [9, 9, 9], e);
        assert_eq!(active(Side::Server, "test:dim", 6).len(), 1);
        assert_eq!(active(Side::Server, "test:other", 6).len(), 0);
        assert_eq!(active(Side::Server, "test:dim", 6 + STAY + 1).len(), 0, "stale");
        forget(Side::Server, "test:dim", [9, 9, 9]);
        assert_eq!(active(Side::Server, "test:dim", 6).len(), 0);
        assert_eq!(active(Side::Client, "test:dim", 6).len(), 1);
        forget_both("test:dim", [9, 9, 9]);
        assert_eq!(active(Side::Client, "test:dim", 6).len(), 0);
    }
}
