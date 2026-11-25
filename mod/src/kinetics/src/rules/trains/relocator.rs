//! `TrainRelocator` (content/trains/entity/TrainRelocator.java, `TrainRelocationPacket.java`): picking a derailed or stopped
//! train up with the Wrench and putting it back on track.
//!
//! [`relocate`] is `TrainRelocator.relocate(train, level, pos, bezier, direction, lookAngle, simulate)` from the point where the
//! graph location of the clicked track is known (`TrackGraphHelper.getGraphLocationAt`, the glue's, in `graph_helper.rs`): a
//! probe walks the train's wheels backwards from the clicked spot, recording where each wheel lands; the spot is refused when the
//! probe is blocked on the way (the end of the track, a portal) or a stretch of the new line crosses another train; otherwise the
//! train takes the recorded places, forgets its signals and its destination, and stops. The session around it (the client's
//! `relocatingTrain`, the messages and the checks that end it) is [`check_session`] and [`click`].
//!
//! Differences from Create, for the README:
//! - Create runs the session on the client (the hover shows the line the wheels would take, the click sends
//!   `TrainRelocationPacket`); here the server keeps it and the click acts at once. The client says where the cursor is and
//!   the server answers with [`preview`] (the line the wheels would take) and the `train.relocate.valid` message.
//! - A curve is clicked by the client's ray at its segments (`track_overlay::pick_curves`), which sends the segment; the location is
//!   `TrackGraphHelper.getBezierGraphLocationAt` (`graph_helper.rs`).

//! - The probe's `ignoreEdgePoints` and `ignoreTurns` are the listener-less default of [`Listeners`].

use std::collections::BTreeMap;

use super::graph::{NodeId, Uuid};
use super::network::RailNetworks;
use super::train::{Train, find_colliding_train};
use super::travelling_point::{Listeners, SteerDirection, TrackGraphLocation, TrackSelector, TravellingPoint};
use super::vec::Vec3;
use crate::Pos;

/// `player.canInteractWithBlock(relocatingOrigin, 24)`: the block interaction range (4.5) and the buffer.
pub const SESSION_REACH: f64 = 4.5 + 24.0;
/// `AllConfigs.server().trains.maxTrackPlacementLength.get() * 2`, the packet's buffer: the clicked track may be this far
/// beyond the usual reach.
pub const CLICK_BUFFER: f64 = 64.0;
/// The usual block interaction range.
pub const BLOCK_REACH: f64 = 4.5;

pub const MSG_ABORT: &str = "Relocation aborted";
pub const MSG_INVALID: &str = "Cannot relocate Train to here";
/// `create.train.relocate.valid`, shown while the cursor is on a spot the train fits.
pub const MSG_VALID: &str = "Can relocate to here, Click to Confirm";
pub const MSG_SUCCESS: &str = "Relocation successful";
pub const MSG_TOO_FAR: &str = "Cannot relocate Train this far away";
pub const MSG_MOVING: &str = "Cannot relocate a moving Train";

/// `create.train.relocate`: "Click a Track to Relocate %1$s to. Sneak-Click to abort".
pub fn message_start(train_name: &str) -> String {
    format!("Click a Track to Relocate {train_name} to. Sneak-Click to abort")
}

/// `canInteractWithBlock(pos, buffer)`: the eye is within `BLOCK_REACH + buffer` of the block's box.
pub fn can_interact_with_block(eye: [f64; 3], pos: Pos, buffer: f64) -> bool {
    let mut distance_sqr = 0.0;
    for i in 0..3 {
        let low = f64::from(pos[i]);
        let d = (low - eye[i]).max(eye[i] - (low + 1.0)).max(0.0);
        distance_sqr += d * d;
    }
    distance_sqr < (BLOCK_REACH + buffer).powi(2)
}

/// The test of `clientTick` for a carriage that has moved since the last tick (`|delta|^2 > 1/1024`): a train is moving when it
/// travels more than 1/32 of a block a tick.
pub fn is_moving(speed: f64) -> bool {
    speed * speed > 1.0 / 1024.0
}

/// What the running session does this tick (`clientTick`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    /// Nothing to say: waiting for the click.
    Keep,
    /// The carriage moves (`cannot_relocate_moving`): the session ends.
    Moving,
    /// The player no longer holds the Wrench (`relocate.abort`): the session ends.
    Abort,
    /// The player is too far from where the session began (`relocate.too_far`): the session stays.
    TooFar,
}

/// `clientTick`'s checks of a session, in Create's order.
pub fn check_session(moving: bool, holds_wrench: bool, within_session_reach: bool) -> Check {
    if moving {
        Check::Moving
    } else if !holds_wrench {
        Check::Abort
    } else if !within_session_reach {
        Check::TooFar
    } else {
        Check::Keep
    }
}

/// What a click does in a session (`onClicked`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Click {
    /// A sneak-click, or one from beyond the session's reach: the session ends (`relocate.abort`).
    Abort,
    /// A click on a track: try to relocate there.
    Relocate,
}

pub fn click(sneaking: bool, within_session_reach: bool) -> Click {
    if sneaking || !within_session_reach { Click::Abort } else { Click::Relocate }
}

/// Where each wheel lands when the train stands at a spot, front to back as `forEachTravellingPointBackwards` walks them
/// (the last wheel first in the vector's order: the vector holds them in walk order).
struct Walk {
    places: Vec<(NodeId, NodeId, f64)>,
    positions: Vec<Vec3>,
    blocked: bool,
    /// `blockingIndex`: the wheels reached before the probe was blocked.
    reached: i32,
}

/// The probe of `relocate`: from the graph location, travel the distance each wheel is behind the one before.
fn walk(train: &Train, nets: &RailNetworks, location: &TrackGraphLocation, up_normal: Vec3) -> Option<Walk> {
    let graph = nets.graphs.get(&location.graph)?;
    let node1 = graph.locate_node(&location.edge[0])?.net_id;
    let node2 = graph.locate_node(&location.edge[1])?.net_id;
    graph.edge(node1, node2)?;

    let mut probe = TravellingPoint::new(node1, node2, location.position, false);
    let mut steer = TrackSelector::steer(SteerDirection::None, up_normal);
    let mut out = Walk { places: Vec::new(), positions: Vec::new(), blocked: false, reached: 0 };
    for (_, distance) in train.travelling_points_backwards() {
        if out.blocked {
            continue;
        }
        let mut portal = false;
        {
            // `ignoreEdgePoints` and `ignoreTurns` are the default listeners; a portal blocks.
            let mut on_portal = |_: [super::graph::TrackNodeLocation; 2]| {
                portal = true;
                true
            };
            let mut listeners = Listeners { edge_point: None, turn: None, portal: Some(&mut on_portal) };
            probe.travel(graph, distance, &mut steer, &mut listeners);
        }
        out.places.push((probe.node1?, probe.node2?, probe.position));
        out.positions.push(probe.get_position(Some(graph), false)?);
        if probe.blocked || portal {
            out.blocked = true;
            continue;
        }
        out.reached += 1;
    }
    Some(out)
}

/// What the client's preview of a spot shows (`relocate(.., simulate = true)` on the client records `toVisualise`): whether the train
/// can stand there, and the places its wheels would take, from the first wheel to the last one reached (the walk stops at the first
/// stretch that is blocked or crossed by another train, which is included). Empty when the location is not on the graph.
#[allow(clippy::too_many_arguments)]
pub fn preview(
    train: &Train,
    nets: &RailNetworks,
    others: &BTreeMap<Uuid, Train>,
    location: &TrackGraphLocation,
    up_normal: Vec3,
    dimension: u32,
    max_assembly_length: f64,
) -> (bool, Vec<Vec3>) {
    let Some(found) = walk(train, nets, location, up_normal) else { return (false, Vec::new()) };
    let Some(graph) = nets.graphs.get(&location.graph) else { return (false, Vec::new()) };
    let mut points: Vec<Vec3> = found.positions.first().copied().into_iter().collect();
    for i in 0..found.positions.len().saturating_sub(1) {
        let blocking = i as i32 >= found.reached - 1;
        let collided = !found.blocked
            && find_colliding_train(
                train.id,
                train.graph,
                graph,
                found.positions[i],
                found.positions[i + 1],
                dimension,
                others,
                max_assembly_length,
                None,
            )
            .is_some();
        points.push(found.positions[i + 1]);
        if collided || blocking {
            return (false, points);
        }
    }
    (!found.blocked, points)
}

/// `TrainRelocator.relocate(..)` once the graph location is known: whether the train can stand there and, unless `simulate`, puts
/// it there. `others` are the other trains, `dimension` the clicked track's, `max_assembly_length` the train config's.
#[allow(clippy::too_many_arguments)]
pub fn relocate(
    train: &mut Train,
    nets: &mut RailNetworks,
    others: &BTreeMap<Uuid, Train>,
    location: &TrackGraphLocation,
    up_normal: Vec3,
    dimension: u32,
    max_assembly_length: f64,
    simulate: bool,
) -> bool {
    let Some(found) = walk(train, nets, location, up_normal) else { return false };
    {
        let Some(graph) = nets.graphs.get(&location.graph) else { return false };
        for i in 0..found.positions.len().saturating_sub(1) {
            let blocking = i as i32 >= found.reached - 1;
            let collided = !found.blocked
                && find_colliding_train(
                    train.id,
                    train.graph,
                    graph,
                    found.positions[i],
                    found.positions[i + 1],
                    dimension,
                    others,
                    max_assembly_length,
                    None,
                )
                .is_some();
            if collided || blocking {
                return false;
            }
        }
    }
    if found.blocked {
        return false;
    }
    if simulate {
        return true;
    }

    train.leave_station(nets);
    train.derailed = false;
    train.navigation.waiting_for_signal = None;
    train.occupied_signal_blocks.clear();
    train.graph = Some(location.graph);
    train.point_graph = Some(location.graph);
    train.speed = 0.0;
    train.migrating_points.clear();
    train.cancel_stall();

    if train.navigation.destination.is_some() {
        train.cancel_navigation(nets);
    }

    // `forEachTravellingPoint`: the last wheel the probe reached goes to the first wheel.
    let mut places = found.places;
    for carriage in &mut train.carriages {
        let (first, second) = (&mut carriage.bogeys.0, &mut carriage.bogeys.1);
        let points = first.points.iter_mut().chain(second.iter_mut().flat_map(|b| b.points.iter_mut()));
        for point in points {
            let Some((n1, n2, position)) = places.pop() else { break };
            point.node1 = Some(n1);
            point.node2 = Some(n2);
            point.position = position;
        }
    }

    if let Some(graph) = nets.graphs.get(&location.graph) {
        for carriage in &mut train.carriages {
            carriage.update_contraption_anchors(graph);
        }
    }
    train.status.successful_migration();
    train.collect_initially_occupied_signal_blocks(nets);
    true
}

#[cfg(test)]
mod tests {
    use super::super::bogey::BogeyType;
    use super::super::carriage::{Carriage, CarriageBogey};
    use super::super::network::tests::{line, loc};
    use super::super::railways::Railways;
    use super::*;

    const UP: Vec3 = [0.0, 1.0, 0.0];

    fn node(nets: &RailNetworks, g: Uuid, x: f64) -> NodeId {
        nets.graphs[&g].locate_node(&loc(x, 0.0)).unwrap().net_id
    }

    /// A one carriage train on the line, its leading wheel at `x`, standing on a graph of 0 to `end`.
    fn train_at(nets: &RailNetworks, g: Uuid, id: Uuid, end: f64, x: f64) -> Train {
        let (a, b) = (node(nets, g, 0.0), node(nets, g, end));
        let pt = |p: f64| TravellingPoint::new(a, b, p, false);
        let bogey = CarriageBogey::new(BogeyType::small(), false, vec![], pt(x), pt(x - 2.0), 0);
        Train::new(id, None, Some(g), vec![Carriage::new(1, bogey, None, 0)], vec![], false, "Train", "create:traditional", 0)
    }

    /// The location `x` blocks along the line, as `getGraphLocationAt` gives it for a forward click.
    fn at(g: Uuid, x: f64, end: f64) -> TrackGraphLocation {
        TrackGraphLocation { graph: g, edge: [loc(0.0, 0.0), loc(end, 0.0)], position: x }
    }

    fn setup(end: f64) -> (Railways, Uuid) {
        let mut rails = Railways::new();
        let g = line(&mut rails.nets, &[0.0, end]);
        (rails, g)
    }

    fn try_relocate(rails: &mut Railways, id: Uuid, location: &TrackGraphLocation, simulate: bool) -> bool {
        let Railways { nets, trains, .. } = rails;
        let mut train = trains.remove(&id).unwrap();
        let ok = relocate(&mut train, nets, trains, location, UP, 0, 128.0, simulate);
        trains.insert(id, train);
        ok
    }

    #[test]
    fn a_derailed_train_goes_back_on_track() {
        let (mut rails, g) = setup(100.0);
        let mut train = train_at(&rails.nets, g, 1, 100.0, 12.0);
        train.derailed = true;
        train.graph = None;
        train.speed = 0.3;
        rails.trains.insert(1, train);
        assert!(try_relocate(&mut rails, 1, &at(g, 60.0, 100.0), false));
        let t = &rails.trains[&1];
        assert!(!t.derailed);
        assert_eq!(t.graph, Some(g));
        assert_eq!(t.speed, 0.0);
        // The probe starts at the click with the trailing wheel and goes on for the wheel spacing.
        assert_eq!(t.carriages[0].trailing_point().position, 60.0);
        assert_eq!(t.carriages[0].leading_point().position, 62.0);
    }

    #[test]
    fn the_preview_lists_the_wheels_places_and_says_whether_they_fit() {
        let (mut rails, g) = setup(100.0);
        rails.trains.insert(1, train_at(&rails.nets, g, 1, 100.0, 12.0));
        let (ok, points) = {
            let train = &rails.trains[&1];
            preview(train, &rails.nets, &BTreeMap::new(), &at(g, 60.0, 100.0), UP, 0, 128.0)
        };
        assert!(ok);
        // Two wheels: the first place, then the second two blocks on.
        assert_eq!(points.len(), 2);
        assert!((points[1][0] - points[0][0]).abs() > 1.5);
        // At the end of the track the probe is blocked: the line is still shown, and it is not valid.
        let (ok, points) = {
            let train = &rails.trains[&1];
            preview(train, &rails.nets, &BTreeMap::new(), &at(g, 99.5, 100.0), UP, 0, 128.0)
        };
        assert!(!ok);
        assert!(!points.is_empty());
        // And a simulated relocation agrees about validity.
        assert!(!try_relocate(&mut rails, 1, &at(g, 99.5, 100.0), true));
    }

    #[test]
    fn a_simulation_changes_nothing() {
        let (mut rails, g) = setup(100.0);
        let mut train = train_at(&rails.nets, g, 1, 100.0, 12.0);
        train.derailed = true;
        train.graph = None;
        rails.trains.insert(1, train.clone());
        assert!(try_relocate(&mut rails, 1, &at(g, 60.0, 100.0), true));
        let t = &rails.trains[&1];
        assert!(t.derailed);
        assert_eq!(t.graph, None);
        assert_eq!(t.carriages[0].leading_point().position, 12.0);
    }

    #[test]
    fn the_end_of_the_track_refuses() {
        let (mut rails, g) = setup(100.0);
        rails.trains.insert(1, train_at(&rails.nets, g, 1, 100.0, 12.0));
        // The leading wheel would be 2 blocks past the click: not on the 100 block line.
        assert!(!try_relocate(&mut rails, 1, &at(g, 99.0, 100.0), false));
        assert_eq!(rails.trains[&1].carriages[0].leading_point().position, 12.0);
        // Just in time.
        assert!(try_relocate(&mut rails, 1, &at(g, 97.5, 100.0), false));
        assert_eq!(rails.trains[&1].carriages[0].leading_point().position, 99.5);
    }

    #[test]
    fn a_train_in_the_way_refuses() {
        let (mut rails, g) = setup(100.0);
        rails.trains.insert(1, train_at(&rails.nets, g, 1, 100.0, 12.0));
        rails.trains.insert(2, train_at(&rails.nets, g, 2, 100.0, 61.0));
        assert!(!try_relocate(&mut rails, 1, &at(g, 60.0, 100.0), true));
        assert!(try_relocate(&mut rails, 1, &at(g, 30.0, 100.0), true));
    }

    #[test]
    fn the_train_forgets_its_destination_and_station() {
        let (mut rails, g) = setup(100.0);
        let mut train = train_at(&rails.nets, g, 1, 100.0, 12.0);
        train.navigation.destination = Some(77);
        train.navigation.waiting_for_signal = Some((5, true));
        train.occupied_signal_blocks.insert(3, None);
        train.current_station = Some(9);
        train.status.track = true;
        rails.trains.insert(1, train);
        assert!(try_relocate(&mut rails, 1, &at(g, 40.0, 100.0), false));
        let t = &rails.trains[&1];
        assert_eq!(t.navigation.destination, None);
        assert_eq!(t.navigation.waiting_for_signal, None);
        assert_eq!(t.current_station, None);
        // `status.successfulMigration()` says so once, because the track was missing.
        assert!(!t.status.track);
    }

    #[test]
    fn a_location_off_the_graph_refuses() {
        let (mut rails, g) = setup(100.0);
        rails.trains.insert(1, train_at(&rails.nets, g, 1, 100.0, 12.0));
        let nowhere = TrackGraphLocation { graph: g, edge: [loc(500.0, 0.0), loc(600.0, 0.0)], position: 3.0 };
        assert!(!try_relocate(&mut rails, 1, &nowhere, false));
        let other_graph = TrackGraphLocation { graph: 999, edge: [loc(0.0, 0.0), loc(100.0, 0.0)], position: 3.0 };
        assert!(!try_relocate(&mut rails, 1, &other_graph, false));
    }

    #[test]
    fn the_session_ends_as_create_has_it() {
        assert_eq!(check_session(false, true, true), Check::Keep);
        assert_eq!(check_session(true, true, true), Check::Moving);
        assert_eq!(check_session(false, false, true), Check::Abort);
        assert_eq!(check_session(false, true, false), Check::TooFar);
        // The moving check goes first.
        assert_eq!(check_session(true, false, false), Check::Moving);
        assert_eq!(click(true, true), Click::Abort);
        assert_eq!(click(false, false), Click::Abort);
        assert_eq!(click(false, true), Click::Relocate);
        assert!(is_moving(0.05));
        assert!(!is_moving(0.03));
        assert_eq!(message_start("Express"), "Click a Track to Relocate Express to. Sneak-Click to abort");
    }

    #[test]
    fn reach_is_measured_to_the_blocks_box() {
        // 28 blocks from the middle of the block's face is within 4.5 + 24, 30 is not.
        assert!(can_interact_with_block([0.5, 0.5, 28.0], [0, 0, 0], 24.0));
        assert!(!can_interact_with_block([0.5, 0.5, 30.0], [0, 0, 0], 24.0));
        // The box's edge counts: eye right beside the block's far face.
        assert!(can_interact_with_block([0.5, 0.5, 29.4], [0, 0, 0], 24.0));
        assert!(can_interact_with_block([0.5, 0.5, 4.4], [0, 0, 0], 0.0));
        assert!(!can_interact_with_block([0.5, 0.5, 6.0], [0, 0, 0], 0.0));
    }
}
