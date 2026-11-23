//! The rest of `StationBlockEntity.assemble` (content/trains/station/StationBlockEntity.java): from
//! the scanned assembly track and the contraptions the glue gathered, the carriages and the
//! [`Train`]; and what the station does about trains around it (`enterAssemblyMode`'s renavigation,
//! `tick`'s train presence).
//!
//! `station.rs` has the first half (the scan, the wheel offsets, putting the wheel points on the
//! graph, the error keys). Here [`assemble_train`] walks the bogeys in order: each carriage's
//! contraption is assembled by the glue (a callback; `CarriageContraption.assemble` reads blocks),
//! which says whether it found anything, whether it has forward or backward controls, the bogey
//! blocks' data and whether a second bogey sits where the next scanned bogey is. The rules do the
//! rest: carriage spacing, one or two bogeys per carriage, `single_bogey_carriage`,
//! `not_connected_in_order`, `no_controls`, then the `Train` with the name of the last disassembled
//! one. [`Railways::add_assembled_train`] is the end of `assemble`: the station reserved for the
//! train, its signal blocks collected, the train added.
//!
//! Left out: `CarriageContraption` (blocks, actors, storage, seats), removing the blocks from the
//! world, `setContraption`, the advancement awards, the packets, `exception(...)` bookkeeping on the
//! block entity (the error is the return value), `StationEditPacket`, the schedule item
//! (`applyAutoSchedule` reads the stack; the glue calls [`Train::set_schedule`]), `dropSchedule`.

use super::bogey::BogeyType;
use super::carriage::{Carriage, CarriageBogey};
use super::graph::{EdgePointType, Uuid};
use super::navigation::DiscoveredPath;
use super::railways::Railways;
use super::station::{AssemblyError, AssemblyScan};
use super::train::{Train, TrainWorld};
use super::travelling_point::TravellingPoint;
use super::vec::{Vec3, distance_to, equal, normalize, sub};
use std::collections::BTreeMap;

/// `Train.getUnnamed`: `CreateLang.translateDirect("train.unnamed")` in English.
pub const UNNAMED_TRAIN: &str = "Unnamed Train";
/// `TrainIconType.getDefault()`'s id.
pub const DEFAULT_ICON: &str = "create:traditional";

/// A second bogey a contraption turned out to have, as the glue found it.
#[derive(Clone, Debug, PartialEq)]
pub struct SecondBogeyFound {
    /// It sits where the next scanned bogey is (`secondBogeyPos` equals the expected position,
    /// upside down or not as that bogey is).
    pub matches_next_bogey: bool,
    /// Its block entity's `bogeyData`.
    pub data: Vec<u8>,
}

/// What `CarriageContraption.assemble` found for one carriage.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContraptionResult {
    /// `assemble` returned true (something is attached to the bogey).
    pub success: bool,
    pub has_forward_controls: bool,
    pub has_backward_controls: bool,
    /// The first bogey block entity's `bogeyData`.
    pub first_bogey_data: Vec<u8>,
    pub second_bogey: Option<SecondBogeyFound>,
}

/// A failed assembly: the error and the carriage it belongs to (1-based; -1 for none), as
/// `exception(error, carriage)` shows them.
#[derive(Clone, Debug, PartialEq)]
pub struct AssemblyFailure {
    pub error: AssemblyError,
    pub carriage: i32,
}

/// What `assemble` needs besides the contraptions.
pub struct AssemblyInput<'a> {
    pub scan: &'a AssemblyScan<BogeyType>,
    /// The wheel points, two per scanned bogey, as `station::place_travelling_points` made them.
    pub points: Vec<TravellingPoint>,
    pub graph: Uuid,
    pub train_id: Uuid,
    pub player: Option<Uuid>,
    /// `lastDisassembledTrainName` and `lastDisassembledMapColorIndex`.
    pub last_disassembled: Option<(String, i32)>,
    /// The id of the first carriage (`Carriage.netIdGenerator`); the rest follow.
    pub first_carriage_id: i32,
}

/// What `assemble` made.
#[derive(Debug)]
pub struct AssembledTrain {
    pub train: Train,
    /// For each carriage, the scanned bogey indices it holds (the second is none for a single one).
    pub bogey_indices: Vec<(usize, Option<usize>)>,
}

/// `assemble`'s `location` search: the connected end of the station's track block that lies in the
/// assembly direction. `ends` are the end locations (`getConnected`), `center` the middle of the
/// block, `direction` the direction's normal.
pub fn find_assembly_start<T: Copy>(ends: &[(T, Vec3)], center: Vec3, direction: Vec3) -> Option<T> {
    let mut found = None;
    for (loc, at) in ends {
        let d = distance_to(direction, normalize(sub(*at, center)));
        if equal(0.0, d * d) {
            found = Some(*loc);
        }
    }
    found
}

/// `assemble(playerUUID)` after the points are placed.
///
/// `assemble_contraption(bogey_index, offset)` assembles the carriage whose first bogey is scanned
/// bogey `bogey_index` (`offset` blocks along the track from the station, `loc + 1`); an `Err` is
/// the `AssemblyException` it threw. `derail_angle()` is `Create.RANDOM.nextInt(60) - 30`.
pub fn assemble_train(
    input: AssemblyInput,
    assemble_contraption: &mut dyn FnMut(usize, i32) -> Result<ContraptionResult, AssemblyError>,
    derail_angle: &mut dyn FnMut() -> i32,
) -> Result<AssembledTrain, AssemblyFailure> {
    let scan = input.scan;
    let fail = |error: AssemblyError, carriage: i32| Err(AssemblyFailure { error, carriage });

    if input.points.is_empty() {
        return fail(AssemblyError::NoBogeys, -1);
    }

    let bogey_count = scan.bogey_count();
    let mut points = input.points.into_iter();
    let mut carriages: Vec<Carriage> = Vec::new();
    let mut indices: Vec<(usize, Option<usize>)> = Vec::new();
    let mut spacing: Vec<i32> = Vec::new();
    let mut at_least_one_forward_controls = false;
    let mut any_backward_controls = false;
    let mut next_id = input.first_carriage_id;

    let mut bogey_index = 0;
    while bogey_index < bogey_count {
        if bogey_index > 0 {
            spacing.push(scan.bogey_locations[bogey_index] - scan.bogey_locations[bogey_index - 1]);
        }
        let offset = scan.bogey_locations[bogey_index] + 1;
        let result = match assemble_contraption(bogey_index, offset) {
            Ok(r) => r,
            Err(e) => return fail(e, carriages.len() as i32 + 1),
        };
        at_least_one_forward_controls |= result.has_forward_controls;
        if !result.success {
            return fail(AssemblyError::NothingAttached { carriage: bogey_index + 1 }, -1);
        }
        any_backward_controls |= result.has_backward_controls;

        let type_of_first = &scan.bogey_types[bogey_index];
        let (Some(p1), Some(p2)) = (points.next(), points.next()) else {
            return fail(AssemblyError::NoBogeys, -1);
        };
        let first_bogey = CarriageBogey::new(type_of_first.clone(), scan.upside_down[bogey_index], result.first_bogey_data, p1, p2, derail_angle());
        let mut second_bogey = None;
        let mut bogey_spacing = 0;
        let first_index = bogey_index;
        let mut second_index = None;

        if let Some(second) = result.second_bogey {
            if bogey_index == bogey_count - 1 || !second.matches_next_bogey {
                return fail(AssemblyError::NotConnectedInOrder, carriages.len() as i32 + 1);
            }
            bogey_spacing = scan.bogey_locations[bogey_index + 1] - scan.bogey_locations[bogey_index];
            let (Some(p3), Some(p4)) = (points.next(), points.next()) else {
                return fail(AssemblyError::NoBogeys, -1);
            };
            second_bogey = Some(CarriageBogey::new(
                scan.bogey_types[bogey_index + 1].clone(),
                scan.upside_down[bogey_index + 1],
                second.data,
                p3,
                p4,
                derail_angle(),
            ));
            second_index = Some(bogey_index + 1);
            bogey_index += 1;
        } else if !type_of_first.allows_single_bogey_carriage {
            return fail(AssemblyError::SingleBogeyCarriage, carriages.len() as i32 + 1);
        }

        carriages.push(Carriage::new(next_id, first_bogey, second_bogey, bogey_spacing));
        indices.push((first_index, second_index));
        next_id += 1;
        bogey_index += 1;
    }

    if !at_least_one_forward_controls {
        return fail(AssemblyError::NoControls, -1);
    }

    let mut train = Train::new(input.train_id, input.player, Some(input.graph), carriages, spacing, any_backward_controls, UNNAMED_TRAIN, DEFAULT_ICON, 0);
    if let Some((name, color)) = input.last_disassembled {
        train.name = name;
        train.map_color_index = color;
    }
    Ok(AssembledTrain { train, bogey_indices: indices })
}

impl Railways {
    /// The end of `assemble`: the train stands at the station (`setCurrentStation`, `reserveFor`),
    /// works out the signal blocks it occupies, and joins the manager.
    pub fn add_assembled_train(&mut self, mut train: Train, station: Option<Uuid>) {
        if let Some(s) = station {
            train.set_current_station(s);
            if let Some(g) = train.graph {
                let (id, distance) = (train.id, train.navigation.distance_to_destination);
                let view = train_view(&train, &self.trains);
                if let Some(st) = self.nets.graphs.get_mut(&g).and_then(|g| g.edge_points.get_mut(EdgePointType::Station, s)).and_then(|p| p.station_mut()) {
                    st.reserve_for(id, distance, &view);
                }
            }
        }
        train.collect_initially_occupied_signal_blocks(&self.nets);
        self.add_train(train);
    }

    /// `enterAssemblyMode`'s last step: every train whose destination is this station finds its path
    /// to it again (the station now refuses approach, so the path goes).
    pub fn station_entered_assembly(&mut self, station: Uuid, world: &dyn TrainWorld) {
        let ids: Vec<Uuid> = self.trains.iter().filter(|(_, t)| t.navigation.destination == Some(station)).map(|(id, _)| *id).collect();
        for id in ids {
            let Some(mut train) = self.trains.remove(&id) else { continue };
            let nets = &mut self.nets;
            let others = &self.trains;
            let preferred = train.start_current_instruction(nets, others, world);
            let path: Option<DiscoveredPath> = match preferred {
                Some(p) => Some(p),
                None => train.find_path_to(nets, others, station),
            };
            train.start_navigation(nets, path);
            self.trains.insert(id, train);
        }
    }

    /// What `StationBlockEntity.tick` works out about the trains at a station each tick.
    pub fn station_train_state(&self, station: Uuid, entity_of: &dyn Fn(&Carriage) -> Option<(f32, f32)>) -> StationTrainState {
        let mut state = StationTrainState::default();
        let Some(graph) = self.nets.graphs.values().find(|g| g.point(EdgePointType::Station, station).is_some()) else {
            return state;
        };
        let Some(gs) = graph.point(EdgePointType::Station, station).and_then(|p| p.station()) else { return state };
        let Some(imminent) = gs.imminent_train(station, &self.trains) else { return state };
        let Some(train) = self.trains.get(&imminent) else { return state };
        state.imminent_train = Some(imminent);
        state.train_present = train.current_station_id() == Some(station);
        state.can_disassemble = state.train_present && train.can_disassemble(entity_of);
        state.has_schedule = state.train_present && train.runtime.schedule().is_some();
        state.has_auto_schedule = state.has_schedule && train.runtime.is_auto_schedule;
        state.backwards = train.currently_backwards;
        state.display_link_update_requested = state.train_present && train.runtime.display_link_update_requested;
        state
    }
}

/// `StationBlockEntity.tick`'s picture of the trains at a station.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StationTrainState {
    pub imminent_train: Option<Uuid>,
    pub train_present: bool,
    pub can_disassemble: bool,
    pub has_schedule: bool,
    pub has_auto_schedule: bool,
    pub backwards: bool,
    /// The runtime asks the display links to refresh (the glue clears `display_link_update_requested`).
    pub display_link_update_requested: bool,
}

fn train_view<'a>(t: &Train, others: &'a BTreeMap<Uuid, Train>) -> super::train::TrainView<'a> {
    super::train::TrainView {
        others,
        me: Some((t.id, t.current_station_id(), t.navigation.is_active(), t.navigation.distance_to_destination)),
    }
}

#[cfg(test)]
mod tests {
    use super::super::graph::TrackEdgePoint;
    use super::super::network::tests::{line, loc};
    use super::super::station::{bogey_point_offsets, place_travelling_points, scan_assembly};
    use super::*;

    fn scan_of(locations: &[i32]) -> AssemblyScan<BogeyType> {
        AssemblyScan {
            length: 30,
            bogey_locations: locations.to_vec(),
            bogey_types: locations.iter().map(|_| BogeyType::small()).collect(),
            upside_down: locations.iter().map(|_| false).collect(),
        }
    }

    fn pts(n: usize) -> Vec<TravellingPoint> {
        (0..n).map(|i| TravellingPoint::new(1, 2, 10.0 - i as f64, false)).collect()
    }

    fn input(scan: &AssemblyScan<BogeyType>) -> AssemblyInput<'_> {
        AssemblyInput {
            scan,
            points: pts(scan.bogey_count() * 2),
            graph: 1,
            train_id: 9,
            player: Some(3),
            last_disassembled: None,
            first_carriage_id: 100,
        }
    }

    fn ok(forward: bool, backward: bool, second: Option<bool>) -> ContraptionResult {
        ContraptionResult {
            success: true,
            has_forward_controls: forward,
            has_backward_controls: backward,
            first_bogey_data: vec![1],
            second_bogey: second.map(|m| SecondBogeyFound { matches_next_bogey: m, data: vec![2] }),
        }
    }

    #[test]
    fn single_bogey_carriages_and_spacing() {
        let scan = scan_of(&[0, 5, 11]);
        let mut calls = Vec::new();
        let r = assemble_train(
            input(&scan),
            &mut |i, off| {
                calls.push((i, off));
                Ok(ok(i == 0, i == 2, None))
            },
            &mut || 7,
        )
        .unwrap();
        assert_eq!(calls, vec![(0, 1), (1, 6), (2, 12)]);
        let t = &r.train;
        assert_eq!(t.carriages.len(), 3);
        assert_eq!(t.carriage_spacing, vec![5, 6]);
        assert!(t.double_ended);
        assert_eq!(t.owner, Some(3));
        assert_eq!(t.graph, Some(1));
        assert_eq!(t.name, UNNAMED_TRAIN);
        assert_eq!(t.carriages.iter().map(|c| c.id).collect::<Vec<_>>(), vec![100, 101, 102]);
        assert_eq!(r.bogey_indices, vec![(0, None), (1, None), (2, None)]);
        assert_eq!(t.carriages[0].leading_bogey().derail_angle, 7);
        assert_eq!(t.carriages[0].leading_bogey().bogey_data, vec![1]);
        assert_eq!(t.carriages[0].leading_point(), &TravellingPoint::new(1, 2, 10.0, false));
    }

    #[test]
    fn two_bogey_carriages() {
        let scan = scan_of(&[0, 6, 14]);
        let r = assemble_train(
            input(&scan),
            &mut |i, _| Ok(if i == 0 { ok(true, false, Some(true)) } else { ok(false, false, None) }),
            &mut || 0,
        )
        .unwrap();
        let t = &r.train;
        assert_eq!(t.carriages.len(), 2);
        assert_eq!(t.carriages[0].bogey_spacing, 6);
        assert!(t.carriages[0].is_on_two_bogeys());
        assert_eq!(t.carriages[0].bogeys.1.as_ref().unwrap().bogey_data, vec![2]);
        assert_eq!(t.carriages[0].trailing_point(), &TravellingPoint::new(1, 2, 7.0, false));
        assert_eq!(t.carriage_spacing, vec![8]);
        assert_eq!(r.bogey_indices, vec![(0, Some(1)), (2, None)]);
        assert!(!t.double_ended);
    }

    #[test]
    fn errors_come_with_their_carriage() {
        let scan = scan_of(&[0, 6]);
        // Second bogey not where the next one is.
        let e = assemble_train(input(&scan), &mut |_, _| Ok(ok(true, false, Some(false))), &mut || 0).unwrap_err();
        assert_eq!(e, AssemblyFailure { error: AssemblyError::NotConnectedInOrder, carriage: 1 });
        // A second bogey with none after it.
        let scan1 = scan_of(&[0]);
        let e = assemble_train(input(&scan1), &mut |_, _| Ok(ok(true, false, Some(true))), &mut || 0).unwrap_err();
        assert_eq!(e.error, AssemblyError::NotConnectedInOrder);
        // Nothing attached to the second carriage's bogey: no carriage index.
        let e = assemble_train(
            input(&scan),
            &mut |i, _| Ok(if i == 0 { ok(true, false, None) } else { ContraptionResult::default() }),
            &mut || 0,
        )
        .unwrap_err();
        assert_eq!(e, AssemblyFailure { error: AssemblyError::NothingAttached { carriage: 2 }, carriage: -1 });
        // The contraption threw.
        let e = assemble_train(input(&scan), &mut |i, _| if i == 1 { Err(AssemblyError::NoBogeys) } else { Ok(ok(true, false, None)) }, &mut || 0).unwrap_err();
        assert_eq!(e, AssemblyFailure { error: AssemblyError::NoBogeys, carriage: 2 });
        // No controls.
        let e = assemble_train(input(&scan), &mut |_, _| Ok(ok(false, true, None)), &mut || 0).unwrap_err();
        assert_eq!(e, AssemblyFailure { error: AssemblyError::NoControls, carriage: -1 });
        // No points.
        let mut inp = input(&scan);
        inp.points.clear();
        assert_eq!(assemble_train(inp, &mut |_, _| Ok(ok(true, false, None)), &mut || 0).unwrap_err().error, AssemblyError::NoBogeys);
    }

    #[test]
    fn single_bogey_carriage_can_be_refused() {
        let mut scan = scan_of(&[0]);
        scan.bogey_types[0].allows_single_bogey_carriage = false;
        let e = assemble_train(input(&scan), &mut |_, _| Ok(ok(true, false, None)), &mut || 0).unwrap_err();
        assert_eq!(e, AssemblyFailure { error: AssemblyError::SingleBogeyCarriage, carriage: 1 });
    }

    #[test]
    fn the_last_disassembled_name_is_kept() {
        let scan = scan_of(&[0]);
        let mut inp = input(&scan);
        inp.last_disassembled = Some(("Express".into(), 4));
        let r = assemble_train(inp, &mut |_, _| Ok(ok(true, false, None)), &mut || 0).unwrap();
        assert_eq!((r.train.name.as_str(), r.train.map_color_index), ("Express", 4));
    }

    #[test]
    fn start_location_is_the_end_in_the_direction() {
        let center = [0.5, 64.0, 0.5];
        let ends = [(1, [1.5, 64.0, 0.5]), (2, [-0.5, 64.0, 0.5])];
        assert_eq!(find_assembly_start(&ends, center, [1.0, 0.0, 0.0]), Some(1));
        assert_eq!(find_assembly_start(&ends, center, [-1.0, 0.0, 0.0]), Some(2));
        assert_eq!(find_assembly_start(&ends, center, [0.0, 0.0, 1.0]), None);
    }

    /// The whole of assembling on a real line: scan, offsets, points, carriages, the manager.
    #[test]
    fn assembling_on_a_track_end_to_end() {
        let mut rw = Railways::new();
        let g = line(&mut rw.nets, &[0.0, 60.0]);
        let station = rw.nets.new_uuid();
        rw.nets.add_point(g, TrackEdgePoint::new_station(station, [loc(0.0, 0.0), loc(60.0, 0.0)], 0.5));
        let scan = scan_assembly(30, 20, |i| i < 20, |i, ud| if !ud && (i == 0 || i == 5) { Some((BogeyType::small(), false)) } else { None });
        assert_eq!(scan.bogey_locations, vec![0, 5]);
        let offsets = bogey_point_offsets(&scan.bogey_locations, |i| scan.bogey_types[i].wheel_point_spacing).unwrap();
        let graph = &rw.nets.graphs[&g];
        let points = place_travelling_points(graph, loc(0.0, 0.0), [1.0, 0.0, 0.0], scan.length, &offsets).unwrap();
        assert_eq!(points.len(), 4);
        let inp = AssemblyInput { scan: &scan, points, graph: g, train_id: 5, player: None, last_disassembled: None, first_carriage_id: 1 };
        let r = assemble_train(inp, &mut |i, _| Ok(ok(i == 0, false, None)), &mut || 0).unwrap();
        rw.add_assembled_train(r.train, Some(station));
        let t = &rw.trains[&5];
        assert_eq!(t.current_station, Some(station));
        assert!(t.occupied_signal_blocks.contains_key(&g));
        let gs = rw.nets.graphs[&g].point(EdgePointType::Station, station).unwrap().station().unwrap();
        assert_eq!(gs.nearest_train(&rw.trains), Some(5));
        let st = rw.station_train_state(station, &|_: &Carriage| Some((0.0, 0.0)));
        assert!(st.train_present && st.can_disassemble && !st.has_schedule);
        assert_eq!(st.imminent_train, Some(5));
    }
}
