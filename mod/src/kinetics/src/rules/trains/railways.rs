//! `GlobalRailwayManager`'s train half (content/trains/GlobalRailwayManager.java): the trains, their
//! tick order, the signal groups' occupancy each game tick, and what the track graph's changes mean
//! for the trains on it (`TrackGraph.removeNode`, `transfer`, `transferAll`,
//! `SignalPropagator.notifyTrains`).
//!
//! The track half (graphs, groups, edge point ticks) is [`RailNetworks`]. [`Railways::tick`] is the
//! whole `GlobalRailwayManager.tick(level)` for the overworld: every signal group forgets its trains
//! and reservation, the points tick (before the trains), the trains tick (`earlyTick` of the
//! waiting ones, then the moving ones, then `tick` in the same order, so the ones that waited longest
//! go first), the points tick again. Then trains whose navigation now waits for a signal move to the
//! waiting list and the ones that no longer do move back. `invalid` trains are dropped.
//!
//! Collisions between trains are found in [`Train::tick`] (the first carriage that moves against the
//! other trains' carriages, see [`super::train::find_colliding_train`]).
//!
//! Left out: saved data (`RailwaySavedData`), the sync layer (`TrackGraphSync`, `AddTrainPacket`
//! etc.: the glue reads [`Railways::events`] and the networks' events), `GlobalTrainDisplayData`,
//! login/logout, the client manager (`sided`). Keeping the graph's changes in step with the trains is
//! [`Railways::apply_rail_events`], to be fed what [`RailNetworks::drain_events`] returns.

use super::graph::Uuid;
use super::network::{RailEvent, RailNetworks};
use super::train::{Train, TrainEvent, TrainWorld};
use crate::Pos;
use std::collections::BTreeMap;

/// Where a train's carriages go when it is disassembled, and what the station block remembers.
#[derive(Clone, Debug, PartialEq)]
pub struct DisassemblyPlan {
    /// `(carriage index, carriage id, block position)` of each carriage's entity.
    pub placements: Vec<(usize, i32, Pos)>,
    /// The station the train stood at (the block entity keeps `lastDisassembledTrainName`).
    pub station: Option<Uuid>,
    pub train_name: String,
    pub map_color_index: i32,
}

/// The train half of `GlobalRailwayManager`.
#[derive(Debug, Default)]
pub struct Railways {
    pub nets: RailNetworks,
    pub trains: BTreeMap<Uuid, Train>,
    moving: Vec<Uuid>,
    waiting: Vec<Uuid>,
    /// What the trains did that the world must be told, for the glue to drain.
    pub events: Vec<TrainEvent>,
}

impl Railways {
    pub fn new() -> Railways {
        Railways { nets: RailNetworks::new(), trains: BTreeMap::new(), moving: Vec::new(), waiting: Vec::new(), events: Vec::new() }
    }

    /// `addTrain(train)`: it starts on the moving list.
    pub fn add_train(&mut self, train: Train) {
        let id = train.id;
        self.trains.insert(id, train);
        self.moving.push(id);
    }

    /// `removeTrain(id)`.
    pub fn remove_train(&mut self, id: Uuid) -> Option<Train> {
        let removed = self.trains.remove(&id)?;
        self.moving.retain(|t| *t != id);
        self.waiting.retain(|t| *t != id);
        Some(removed)
    }

    /// Every loaded train goes on the moving list (`loadTrackData`'s `movingTrains.addAll`).
    pub fn load_all_moving(&mut self) {
        self.moving = self.trains.keys().copied().collect();
        self.waiting.clear();
    }

    /// The trains in tick order: waiting first, then moving.
    pub fn tick_order(&self) -> Vec<Uuid> {
        self.waiting.iter().chain(self.moving.iter()).copied().collect()
    }

    pub fn is_waiting(&self, id: Uuid) -> bool {
        self.waiting.contains(&id)
    }

    /// `tick(level)`.
    pub fn tick(&mut self, world: &dyn TrainWorld) {
        self.nets.clear_group_occupancy();
        self.nets.tick_all_points(true);
        self.tick_trains(world);
        self.nets.tick_all_points(false);
    }

    fn tick_trains(&mut self, world: &dyn TrainWorld) {
        // keeping two lists ensures a tick order starting at longest waiting
        let order = self.tick_order();
        for id in &order {
            self.with_train(*id, |t, nets, _others, _events| t.early_tick(nets, world));
        }
        for id in &order {
            self.with_train(*id, |t, nets, others, events| t.tick(nets, others, world, events));
        }

        let mut i = 0;
        while i < self.waiting.len() {
            let id = self.waiting[i];
            let Some(train) = self.trains.get(&id) else {
                self.waiting.remove(i);
                continue;
            };
            if train.invalid {
                self.waiting.remove(i);
                self.trains.remove(&id);
                self.events.push(TrainEvent::TrainRemoved { train: id });
                continue;
            }
            if train.navigation.waiting_for_signal.is_some() {
                i += 1;
                continue;
            }
            self.moving.push(id);
            self.waiting.remove(i);
        }

        let mut i = 0;
        while i < self.moving.len() {
            let id = self.moving[i];
            let Some(train) = self.trains.get(&id) else {
                self.moving.remove(i);
                continue;
            };
            if train.invalid {
                self.moving.remove(i);
                self.trains.remove(&id);
                self.events.push(TrainEvent::TrainRemoved { train: id });
                continue;
            }
            if train.navigation.waiting_for_signal.is_none() {
                i += 1;
                continue;
            }
            self.waiting.push(id);
            self.moving.remove(i);
        }
    }

    /// Lends one train, taken out of the map, with the networks and the others.
    fn with_train(&mut self, id: Uuid, f: impl FnOnce(&mut Train, &mut RailNetworks, &mut BTreeMap<Uuid, Train>, &mut Vec<TrainEvent>)) {
        let Some(mut train) = self.trains.remove(&id) else { return };
        f(&mut train, &mut self.nets, &mut self.trains, &mut self.events);
        self.trains.insert(id, train);
    }

    /// What the track's changes do to the trains: a node removed under a train detaches it
    /// (`removeNode`), a node or a whole graph moved to another graph takes the trains on it along
    /// (`transfer`, `transferAll`), and a signal change on an edge a train is on makes it work out its
    /// signal blocks again (`SignalPropagator.notifyTrains`). Feed it what the networks' `drain_events`
    /// returned.
    pub fn apply_rail_events(&mut self, events: &[RailEvent]) {
        for e in events {
            match e {
                RailEvent::TrainsOnNodeRemoved { graph, node, edges } => {
                    let ids: Vec<Uuid> = self
                        .trains
                        .iter()
                        .filter(|(_, t)| t.graph == Some(*graph) && t.is_travelling_on(*node))
                        .map(|(id, _)| *id)
                        .collect();
                    for id in ids {
                        // `detachFromTracks()`: cancel the navigation, then remember the wheels.
                        self.with_train(id, |t, nets, _o, _e| {
                            t.cancel_navigation(nets);
                            if let Some(g) = nets.graphs.get(graph) {
                                t.detach_from_tracks(g, edges);
                            }
                        });
                    }
                }
                RailEvent::NodeTransferred { from, to, node } => {
                    for t in self.trains.values_mut() {
                        if t.graph == Some(*from) && t.is_travelling_on(*node) {
                            t.graph = Some(*to);
                            t.point_graph = Some(*to);
                        }
                    }
                }
                RailEvent::GraphTransferred { from, to } => {
                    for t in self.trains.values_mut() {
                        if t.graph == Some(*from) {
                            t.graph = Some(*to);
                            t.point_graph = Some(*to);
                        }
                    }
                }
                RailEvent::SignalBlocksStale { graph, edges } => {
                    for (from, to) in edges {
                        for t in self.trains.values_mut() {
                            if t.graph != Some(*graph) || t.update_signal_blocks {
                                continue;
                            }
                            if t.travelling_points().iter().any(|tp| tp.node1 == Some(*from) && tp.node2 == Some(*to)) {
                                t.update_signal_blocks = true;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// `Train.disassemble(assemblyDirection, pos)`: when the train can (see [`Train::can_disassemble`])
    /// it is removed from the manager and the places of its carriages are returned; the station's
    /// reservation is cancelled. `direction` is the assembly direction's normal, `pos` the block above
    /// the station's track.
    pub fn disassemble_train(
        &mut self,
        id: Uuid,
        direction: [i32; 3],
        pos: Pos,
        entity_of: &dyn Fn(&super::carriage::Carriage) -> Option<(f32, f32)>,
    ) -> Option<DisassemblyPlan> {
        let train = self.trains.get(&id)?;
        if !train.can_disassemble(entity_of) {
            return None;
        }
        let placements = train.disassembly_positions(direction, pos).into_iter().map(|(i, p)| (i, train.carriages[i].id, p)).collect();
        let station = train.current_station_in(&self.nets).map(|p| p.id);
        let plan = DisassemblyPlan { placements, station, train_name: train.name.clone(), map_color_index: train.map_color_index };
        if let (Some(s), Some(g)) = (station, train.graph) {
            if let Some(st) = self.nets.graphs.get_mut(&g).and_then(|g| g.edge_points.get_mut(super::graph::EdgePointType::Station, s)).and_then(|p| p.station_mut()) {
                st.cancel_reservation(id);
            }
        }
        self.remove_train(id);
        self.events.push(TrainEvent::TrainRemoved { train: id });
        Some(plan)
    }
}
