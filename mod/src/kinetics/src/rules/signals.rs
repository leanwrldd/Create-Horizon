//! When a block entity asks the world for its redstone signal, and when it reuses the last answer.
//!
//! Create reads `hasNeighborSignal` from `neighborChanged` (and once at placement or load). A block
//! entity of the port ticks instead, so the glue (`mod/src/signals.rs`) keeps the last answer and asks
//! only when this says so: a neighbour update marked the position dirty, no answer was ever kept, or
//! the slow heartbeat falls on this tick (it stands in for updates Horizon may not send, such as a
//! power change two blocks away through a conductor).

/// Ticks between reads made whatever the events said.
pub const HEARTBEAT: i64 = 20;

/// Whether the heartbeat falls on `game_time` for the block at `pos`: positions are spread over the
/// period by their coordinates, so a field of machines does not all read on the same tick.
pub fn beat(game_time: i64, pos: [i32; 3]) -> bool {
    let spread = i64::from(pos[0]) + 3 * i64::from(pos[1]) + 7 * i64::from(pos[2]);
    (game_time + spread).rem_euclid(HEARTBEAT) == 0
}

/// Whether to ask the world for the signal now (`true`) or use the kept answer (`false`).
/// `dirty`: `neighborChanged` fired at this position since the last read. `known`: an answer is kept.
pub fn should_read(dirty: bool, known: bool, game_time: i64, pos: [i32; 3]) -> bool {
    dirty || !known || beat(game_time, pos)
}

#[cfg(test)]
mod tests {
    use super::*;

    const POS: [i32; 3] = [4, 70, -9];

    /// A tick on which the heartbeat is not due at `POS`.
    fn quiet() -> i64 {
        (0..HEARTBEAT).find(|t| !beat(*t, POS)).unwrap()
    }

    #[test]
    fn a_neighbour_update_forces_a_read() {
        assert!(should_read(true, true, quiet(), POS));
    }

    #[test]
    fn a_first_read_is_always_made() {
        assert!(should_read(false, false, quiet(), POS));
    }

    #[test]
    fn a_quiet_known_position_uses_the_cache() {
        assert!(!should_read(false, true, quiet(), POS));
    }

    #[test]
    fn the_heartbeat_reads_once_per_period() {
        let due = (0..HEARTBEAT * 3).filter(|t| should_read(false, true, *t, POS)).count();
        assert_eq!(due, 3);
    }

    #[test]
    fn the_heartbeat_is_spread_by_position() {
        let at = |pos| (0..HEARTBEAT).find(|t| beat(*t, pos)).unwrap();
        assert_ne!(at([0, 0, 0]), at([1, 0, 0]));
        assert_ne!(at([0, 0, 0]), at([0, 1, 0]));
        // Negative coordinates and times stay in range.
        assert!((-40..0).any(|t| beat(t, [-5, -64, -5])));
    }
}
