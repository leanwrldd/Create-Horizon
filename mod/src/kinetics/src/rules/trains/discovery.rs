//! How the glue learns about track it was not told about (Create: `TrackBlock.onPlace` schedules a tick, `tick` runs
//! `TrackPropagator.onRailAdded`; `onRemove` runs `onRailRemoved`). Horizon has no `onPlace` or `onRemove` hook for a
//! block placed by `setblock`, `fill`, a structure or a piston, so the glue looks at the track it can see (each track
//! block's own block entity ticks, and a slow sweep over what is known) and asks the two decisions here.
//!
//! Play-test 2026-11-26: a track put down by `/fill` or `/setblock` never reached the graph (nothing looked for new
//! blocks, the sweep only looked for vanished ones), and a block that was replaced and put back lost its node for good.
//! The level's saved token also reset the whole graph whenever the attachment read back empty.

use super::graph::DiscoveredLocation;

/// What to do with a track block that is in the world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Registration {
    /// The graph already has what the block gives it.
    Keep,
    /// `onRailAdded`: the block is not in the graph.
    Add,
    /// `onRailRemoved` with what it had, then `onRailAdded`: its ends are not what the graph was built from (the shape or
    /// its curves changed under it, as a `setblock` over it does).
    Retrack,
}

fn same_ends(a: &[DiscoveredLocation], b: &[DiscoveredLocation]) -> bool {
    a.len() == b.len() && a.iter().all(|x| b.contains(x))
}

/// `tracked`: the glue has the block among the track it added. `known`: the ends it had when it was added. `current`: the
/// ends the block gives now (`getConnected(pos, state, false, null)`).
pub fn registration(tracked: bool, known: Option<&[DiscoveredLocation]>, current: &[DiscoveredLocation]) -> Registration {
    match (tracked, known) {
        (false, _) | (true, None) => Registration::Add,
        (true, Some(known)) if same_ends(known, current) => Registration::Keep,
        (true, Some(_)) => Registration::Retrack,
    }
}

/// What the level's saved token says about the graphs kept in memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sync {
    /// The same world: nothing to do.
    Keep,
    /// The same world, but the attachment did not read back: keep the graphs and write the token again.
    Resave,
    /// Another world (or a saved one): drop what is in memory and adopt this token; its positions are added again.
    Adopt(i64),
    /// No world known yet: start from nothing with a new token.
    Fresh,
}

/// `stored`: the token the level's attachment holds (0 when it holds none). `kept`: the token of the graphs in memory (0
/// before the first sync). `went_back`: the game time is earlier than the last sync (a different world was opened).
pub fn sync(stored: i64, kept: i64, went_back: bool) -> Sync {
    match (stored, kept) {
        (s, k) if s != 0 && s == k && !went_back => Sync::Keep,
        (s, _) if s != 0 => Sync::Adopt(s),
        (_, k) if k != 0 && !went_back => Sync::Resave,
        _ => Sync::Fresh,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f64) -> DiscoveredLocation {
        DiscoveredLocation::new(0, [x, 64.0, 0.0])
    }

    #[test]
    fn a_block_nobody_added_is_added() {
        // Reproduces the play-test: a `/fill`ed block is tracked by nobody, and nothing used to add it.
        assert_eq!(registration(false, None, &[at(0.0), at(1.0)]), Registration::Add);
    }

    #[test]
    fn a_tracked_block_with_no_remembered_ends_is_added() {
        assert_eq!(registration(true, None, &[at(0.0)]), Registration::Add);
    }

    #[test]
    fn a_block_put_back_after_removal_is_added_again() {
        // Removal drops it from the tracked set and the known ends; discovery then sees it as new.
        assert_eq!(registration(false, None, &[at(0.0), at(1.0)]), Registration::Add);
    }

    #[test]
    fn unchanged_ends_keep_the_node() {
        let known = [at(0.0), at(1.0)];
        assert_eq!(registration(true, Some(&known), &[at(1.0), at(0.0)]), Registration::Keep);
    }

    #[test]
    fn changed_ends_retrack() {
        let known = [at(0.0), at(1.0)];
        assert_eq!(registration(true, Some(&known), &[at(0.0), at(2.0)]), Registration::Retrack);
        assert_eq!(registration(true, Some(&known), &[at(0.0)]), Registration::Retrack);
    }

    #[test]
    fn the_same_world_keeps_its_graphs_even_when_the_attachment_reads_back_empty() {
        assert_eq!(sync(7, 7, false), Sync::Keep);
        // The old code reset everything here, every second.
        assert_eq!(sync(0, 7, false), Sync::Resave);
    }

    #[test]
    fn another_world_is_adopted_or_started_fresh() {
        assert_eq!(sync(9, 7, false), Sync::Adopt(9));
        assert_eq!(sync(7, 7, true), Sync::Adopt(7));
        assert_eq!(sync(0, 7, true), Sync::Fresh);
        assert_eq!(sync(0, 0, false), Sync::Fresh);
        assert_eq!(sync(9, 0, false), Sync::Adopt(9));
    }
}
