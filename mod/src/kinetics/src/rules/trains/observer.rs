//! `TrackObserver` (content/trains/observer/TrackObserver.java), the edge point a train observer
//! block puts on the track, and the `SingleBlockEntityEdgePoint` state it extends.
//!
//! A train passing over it calls [`TrackObserver::keep_alive`] every tick; the observer stays
//! activated for 8 more ticks. Java's `tick(graph, preTrains)` ignores `preTrains`, and the graph
//! ticks its points twice a game tick (before and after the trains), so it counts down by 2 a tick:
//! kept alive, it is activated for 4 ticks. Kept as is.
//!
//! The filter is `FilterItemStack`; here it is an optional filter key the world answers for
//! ([`super::train::TrainWorld::item_matches`]). `None` is the empty filter.
//!
//! Left out: NBT, packets, `notifyTrains` (the glue raises `SignalBlocksStale` for the observer's edge).

use super::graph::Uuid;
use crate::Pos;

/// `TrackObserver`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TrackObserver {
    /// `SingleBlockEntityEdgePoint.blockEntityPos`.
    pub block_entity_pos: Option<Pos>,
    /// `blockEntityDimension` (an index the caller keeps).
    pub block_entity_dimension: Option<u32>,
    activated: i32,
    /// `filter`: `None` is `FilterItemStack.empty()`.
    pub filter: Option<String>,
    current_train: Option<Uuid>,
}

impl TrackObserver {
    /// `new TrackObserver()`.
    pub fn new() -> TrackObserver {
        TrackObserver::default()
    }

    /// `blockEntityAdded(blockEntity, front)`, with the filter its filtering behaviour holds.
    pub fn block_entity_added(&mut self, pos: Pos, dimension: u32, filter: Option<String>) {
        self.block_entity_pos = Some(pos);
        self.block_entity_dimension = Some(dimension);
        self.filter = filter;
    }

    /// `tick(graph, preTrains)`: counts down; no activation, no train.
    pub fn tick(&mut self) {
        if self.is_activated() {
            self.activated -= 1;
        }
        if !self.is_activated() {
            self.current_train = None;
        }
    }

    /// `setFilterAndNotify(level, filter)`'s state change.
    pub fn set_filter(&mut self, filter: Option<String>) {
        self.filter = filter;
    }

    pub fn current_train(&self) -> Option<Uuid> {
        self.current_train
    }

    pub fn is_activated(&self) -> bool {
        self.activated > 0
    }

    /// `activated` as the block entity reads it (NBT `Activated`).
    pub fn activated_ticks(&self) -> i32 {
        self.activated
    }

    /// `keepAlive(train)`.
    pub fn keep_alive(&mut self, train: Uuid) {
        self.activated = 8;
        self.current_train = Some(train);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kept_alive_it_lasts_eight_counts() {
        let mut o = TrackObserver::new();
        assert!(!o.is_activated());
        o.keep_alive(7);
        assert!(o.is_activated());
        assert_eq!(o.current_train(), Some(7));
        for _ in 0..7 {
            o.tick();
        }
        assert!(o.is_activated());
        o.tick();
        assert!(!o.is_activated());
        assert_eq!(o.current_train(), None);
    }

    #[test]
    fn block_entity_added_takes_position_and_filter() {
        let mut o = TrackObserver::new();
        o.block_entity_added([1, 2, 3], 0, Some("create:cog".into()));
        assert_eq!(o.block_entity_pos, Some([1, 2, 3]));
        assert_eq!(o.filter.as_deref(), Some("create:cog"));
    }
}
