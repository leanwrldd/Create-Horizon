//! Driving a train by hand: `CarriageContraptionEntity.startControlling` / `control`
//! (content/trains/entity/CarriageContraptionEntity.java), `ControlsServerHandler`
//! (contraptions/actors/trainControls/) and the key numbers `ControlsUtil.getControls` gives
//! (foundation/utility/ControlsUtil.java).
//!
//! A player at the Train Controls sends the movement keys they hold (`ControlsHandler`, every 5 ticks while held,
//! and the keys that changed); the server keeps each key alive for 30 ticks without news
//! (`ControlsServerHandler.ManuallyPressedKey`) and runs [`control`] each tick with the keys alive. The prompts of
//! Java's `TrainPromptPacket` come back as [`Prompt`]s.
//!
//! Left out: `backwardsDriver` (the honk's owner), the HUD packet (`TrainHUDUpdatePacket`), honking.

use super::graph::Uuid;
use super::network::RailNetworks;
use super::train::Train;
use super::travelling_point::SteerDirection;
use std::collections::BTreeMap;

/// `ControlsUtil.getControls()`: the key each number stands for.
pub const FORWARD: i32 = 0;
pub const BACK: i32 = 1;
pub const LEFT: i32 = 2;
pub const RIGHT: i32 = 3;
/// Jump: the space bar approaches a station, or confirms the arrival.
pub const SPACE: i32 = 4;
/// Sneak: stops controlling (`heldControls.contains(5)` fails `control`).
pub const SNEAK: i32 = 5;

/// The vanilla names of the six keys, in `ControlsUtil`'s order.
pub const KEYS: [&str; 6] = [
    "key.keyboard.w",
    "key.keyboard.s",
    "key.keyboard.a",
    "key.keyboard.d",
    "key.keyboard.space",
    "key.keyboard.left.shift",
];

/// `ControlsHandler.PACKET_RATE`: ticks between a held key's packets.
pub const PACKET_RATE: i32 = 5;
/// `ControlsServerHandler.TIMEOUT`: ticks a key stays pressed with no packet.
pub const TIMEOUT: i32 = 30;
/// How far a player may be from the controls (`canInteractWithBlock(.., 8)`).
pub const RANGE: f64 = 8.0;
/// `TrainHUD`'s prompt bar is this many ticks long.
pub const PROGRESS_BARS: i32 = 30;

/// What the keys held by one player are, kept alive on the server (`ControlsServerHandler`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeldKeys {
    /// Key number to the ticks it has left.
    keys: BTreeMap<i32, i32>,
}

impl HeldKeys {
    /// `receivePressed`: the keys of a packet. A pressed key is kept alive (or newly pressed), a released one is
    /// let go at once.
    pub fn receive(&mut self, keys: &[i32], pressed: bool) {
        for key in keys {
            if !(0..=5).contains(key) {
                continue;
            }
            match self.keys.get_mut(key) {
                Some(left) => *left = if pressed { TIMEOUT } else { 0 },
                None if pressed => {
                    self.keys.insert(*key, TIMEOUT);
                }
                None => {}
            }
        }
    }

    /// `ControlsServerHandler.tick`: every key ages a tick and goes when it is out of time. The keys still held.
    pub fn tick(&mut self) -> Vec<i32> {
        for left in self.keys.values_mut() {
            *left -= 1;
        }
        self.keys.retain(|_, left| *left > 0);
        self.keys.keys().copied().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// What `TrainPromptPacket` puts on the player's action bar.
#[derive(Clone, Debug, PartialEq)]
pub enum Prompt {
    /// `train.arrived_at`.
    ArrivedAt(String),
    /// `train.departing_from`.
    DepartingFrom(String),
    /// `contraption.controls.approach_station`: hold the jump key to approach it.
    ApproachStation(String),
    /// The approach bar: `progress` bars of the 30 are white; `arrived` when none is left.
    Progress { progress: i32, arrived: bool },
}

/// How the controls stand to the carriage's direction: `inverted` when the Controls block faces against the
/// carriage's initial orientation turned counter-clockwise (`!facing.equals(initialOrientation)`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlsSide {
    pub inverted: bool,
}

/// `inverted` for a Controls block `facing` (a horizontal direction as `(x, z)`), the carriage's assembly direction
/// as `(x, z)`: `getInitialOrientation().getCounterClockWise()` is the assembly direction (the initial orientation is
/// its clockwise), so the controls are the right way round when they face the assembly direction.
pub fn controls_side(facing: [i32; 2], assembly_direction: [i32; 2]) -> ControlsSide {
    ControlsSide { inverted: facing != assembly_direction }
}

/// What a tick of `control` came to.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ControlOutcome {
    /// `control` returned true (the player keeps controlling).
    pub keep: bool,
    pub prompt: Option<Prompt>,
    /// `cleanUpApproachStationMessage`: clear the action bar line of the approach prompt.
    pub clear_prompt: bool,
}

/// The state of a player's driving that `CarriageContraptionEntity` keeps.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Driver {
    /// `navDistanceTotal`.
    pub nav_distance_total: f64,
    /// `stationMessage`: the approach prompt is showing.
    pub station_message: bool,
}

/// `startControlling`: a spectator or a derailed train can't be driven. The driver takes over the train: a schedule
/// that was running pauses (`train.status.manualControls()`), the navigation is cancelled and the signal wait ends.
pub fn start_controlling(train: &mut Train, nets: &mut RailNetworks) -> bool {
    if train.derailed {
        return false;
    }
    if train.runtime.schedule.is_some() && !train.runtime.paused {
        train.status.manual_controls();
    }
    train.cancel_navigation(nets);
    train.runtime.paused = true;
    train.navigation.waiting_for_signal = None;
    true
}

fn signum(v: f64) -> f64 {
    if v > 0.0 { 1.0 } else if v < 0.0 { -1.0 } else { 0.0 }
}

fn station_name(nets: &RailNetworks, train: &Train, station: Uuid) -> String {
    train
        .graph
        .and_then(|g| nets.graphs.get(&g))
        .and_then(|g| g.point(super::graph::EdgePointType::Station, station))
        .and_then(|p| p.station())
        .map_or_else(String::new, |s| s.name.clone())
}

/// `CarriageContraptionEntity.control` for the carriage the controls are on: the keys held now, which way round the
/// Controls block is, and what is near (the player's range is the caller's: a player further than [`RANGE`], a
/// spectator and a sneak ([`SNEAK`] held) are `keep: false`).
pub fn control(
    train: &mut Train,
    nets: &mut RailNetworks,
    others: &BTreeMap<Uuid, Train>,
    held: &[i32],
    side: ControlsSide,
    driver: &mut Driver,
    leading_on_turn: bool,
) -> ControlOutcome {
    let mut out = ControlOutcome { keep: true, ..Default::default() };
    if train.derailed || held.contains(&SNEAK) {
        out.keep = false;
        return out;
    }
    let inverted = side.inverted;
    let mut target_speed: i32 = 0;
    if held.contains(&FORWARD) {
        target_speed += 1;
    }
    if held.contains(&BACK) {
        target_speed -= 1;
    }
    let mut target_steer: i32 = 0;
    if held.contains(&LEFT) {
        target_steer += 1;
    }
    if held.contains(&RIGHT) {
        target_steer -= 1;
    }
    if inverted {
        target_speed *= -1;
        target_steer *= -1;
    }
    if target_speed != 0 {
        train.burn_fuel_manual();
    }
    let slow = inverted ^ (target_speed < 0);
    let space_down = held.contains(&SPACE);
    let current_station = train.current_station_id();
    if let (Some(station), true) = (current_station, space_down) {
        out.prompt = Some(Prompt::ArrivedAt(station_name(nets, train, station)));
        return out;
    }
    if let Some(before) = train.speed_before_stall {
        if target_speed != 0 && signum(before) != f64::from(target_speed) {
            train.cancel_stall();
        }
    }
    if let (Some(station), true) = (current_station, target_speed != 0) {
        driver.station_message = false;
        out.prompt = Some(Prompt::DepartingFrom(station_name(nets, train, station)));
    }

    if current_station.is_none() {
        if let Some(destination) = train.navigation.destination {
            let _ = destination;
            if !space_down {
                train.cancel_navigation(nets);
            }
            if space_down && train.navigation.destination.is_some() {
                let f = train.navigation.distance_to_destination / driver.nav_distance_total;
                let progress = ((1.0 - (1.0 - f) * (1.0 - f)).clamp(0.0, 1.0) * f64::from(PROGRESS_BARS)) as i32;
                out.prompt = Some(Prompt::Progress { progress, arrived: progress == 0 });
                train.manual_tick = true;
                return out;
            }
        }
        let directed_speed = if target_speed != 0 { f64::from(target_speed) } else { train.speed };
        let forward = !train.double_ended || (if directed_speed != 0.0 { directed_speed > 0.0 } else { !inverted });
        let look_ahead = train.find_nearest_approachable(nets, others, forward);
        if let Some(station) = look_ahead {
            if space_down {
                train.manual_tick = true;
                let path = train.find_path_to(nets, others, station);
                train.start_navigation(nets, path);
                train.manual_tick = false;
                driver.nav_distance_total = train.navigation.distance_to_destination;
                return out;
            }
            out.prompt = Some(Prompt::ApproachStation(station_name(nets, train, station)));
            driver.station_message = true;
        } else if driver.station_message {
            driver.station_message = false;
            out.clear_prompt = true;
        }
    }

    train.manual_steer = match target_steer {
        s if s < 0 => SteerDirection::Right,
        s if s > 0 => SteerDirection::Left,
        _ => SteerDirection::None,
    };
    let mut top_speed = train.max_speed() * f64::from(train.config.manual_train_speed_modifier);
    let capped_top_speed = top_speed * train.throttle;
    if leading_on_turn {
        top_speed = train.max_turn_speed();
    }
    if slow {
        top_speed /= 4.0;
    }
    train.target_speed = top_speed.min(capped_top_speed) * f64::from(target_speed);
    let countering_acceleration = (signum(f64::from(target_speed)) - signum(train.speed)).abs() > 1.5;
    train.manual_tick = true;
    train.approach_target_speed_manual(nets, if countering_acceleration { 2.0 } else { 1.0 });
    out
}

/// Whether the carriage's leading or trailing wheel is on a curve (`getLeadingPoint().edge.isTurn()`).
pub fn on_turn(train: &Train, carriage: usize, nets: &RailNetworks) -> bool {
    let Some(graph) = train.graph.and_then(|g| nets.graphs.get(&g)) else { return false };
    let Some(c) = train.carriages.get(carriage) else { return false };
    [c.leading_point(), c.trailing_point()].iter().any(|p| p.edge(graph).is_some_and(|e| e.is_turn()))
}

#[cfg(test)]
mod tests {
    use super::super::bogey::BogeyType;
    use super::super::carriage::{Carriage, CarriageBogey};
    use super::super::network::tests::line;
    use super::super::railways::Railways;
    use super::super::travelling_point::TravellingPoint;
    use super::*;

    fn train(rw: &Railways, g: Uuid) -> Train {
        let node = |x: f64| {
            rw.nets.graphs[&g].locate_node(&super::super::network::tests::loc(x, 0.0)).unwrap().net_id
        };
        let (a, b) = (node(0.0), node(100.0));
        let pt = |p: f64| TravellingPoint::new(a, b, p, false);
        let bogey = CarriageBogey::new(BogeyType::small(), false, vec![], pt(50.0), pt(48.0), 0);
        let mut c = Carriage::new(1, bogey, None, 0);
        c.present_conductors = [true, true];
        Train::new(1, None, Some(g), vec![c], vec![], false, "T", "create:traditional", 0)
    }

    fn setup() -> (Railways, Uuid) {
        let mut rw = Railways::new();
        let g = line(&mut rw.nets, &[0.0, 100.0]);
        (rw, g)
    }

    #[test]
    fn holding_forward_accelerates_to_the_manual_top_speed() {
        let (mut rw, g) = setup();
        let mut t = train(&rw, g);
        let mut driver = Driver::default();
        assert!(start_controlling(&mut t, &mut rw.nets));
        let others = BTreeMap::new();
        for _ in 0..500 {
            let out = control(&mut t, &mut rw.nets, &others, &[FORWARD], ControlsSide { inverted: false }, &mut driver, false);
            assert!(out.keep);
        }
        let top = t.max_speed() * f64::from(t.config.manual_train_speed_modifier);
        assert!((t.speed - top).abs() < 1e-6, "{} {}", t.speed, top);
        assert!(t.runtime.paused);
    }

    #[test]
    fn back_is_a_quarter_of_the_speed_and_inverted_controls_flip_the_keys() {
        let (mut rw, g) = setup();
        let mut t = train(&rw, g);
        let mut driver = Driver::default();
        let others = BTreeMap::new();
        control(&mut t, &mut rw.nets, &others, &[BACK], ControlsSide { inverted: false }, &mut driver, false);
        let top = t.max_speed() * f64::from(t.config.manual_train_speed_modifier);
        assert!((t.target_speed + top / 4.0).abs() < 1e-6, "{}", t.target_speed);
        // The same key on controls facing the other way moves the train the other way, still slowly (`slow` is
        // `inverted ^ (targetSpeed < 0)`, true for a back key on either side).
        control(&mut t, &mut rw.nets, &others, &[BACK], ControlsSide { inverted: true }, &mut driver, false);
        assert!((t.target_speed - top / 4.0).abs() < 1e-6, "{}", t.target_speed);
        control(&mut t, &mut rw.nets, &others, &[FORWARD], ControlsSide { inverted: true }, &mut driver, false);
        assert!((t.target_speed + top).abs() < 1e-6, "{}", t.target_speed);
    }

    #[test]
    fn steering_keys_turn_the_switches() {
        let (mut rw, g) = setup();
        let mut t = train(&rw, g);
        let mut driver = Driver::default();
        let others = BTreeMap::new();
        control(&mut t, &mut rw.nets, &others, &[LEFT], ControlsSide { inverted: false }, &mut driver, false);
        assert_eq!(t.manual_steer, SteerDirection::Left);
        control(&mut t, &mut rw.nets, &others, &[RIGHT], ControlsSide { inverted: false }, &mut driver, false);
        assert_eq!(t.manual_steer, SteerDirection::Right);
        control(&mut t, &mut rw.nets, &others, &[LEFT], ControlsSide { inverted: true }, &mut driver, false);
        assert_eq!(t.manual_steer, SteerDirection::Right);
        control(&mut t, &mut rw.nets, &others, &[], ControlsSide { inverted: false }, &mut driver, false);
        assert_eq!(t.manual_steer, SteerDirection::None);
    }

    #[test]
    fn sneaking_or_a_derailed_train_ends_it() {
        let (mut rw, g) = setup();
        let mut t = train(&rw, g);
        let mut driver = Driver::default();
        let others = BTreeMap::new();
        assert!(!control(&mut t, &mut rw.nets, &others, &[SNEAK], ControlsSide { inverted: false }, &mut driver, false).keep);
        t.derailed = true;
        assert!(!start_controlling(&mut t, &mut rw.nets));
        assert!(!control(&mut t, &mut rw.nets, &others, &[], ControlsSide { inverted: false }, &mut driver, false).keep);
    }

    #[test]
    fn a_turn_caps_the_speed_to_the_turning_speed() {
        let (mut rw, g) = setup();
        let mut t = train(&rw, g);
        let mut driver = Driver::default();
        let others = BTreeMap::new();
        control(&mut t, &mut rw.nets, &others, &[FORWARD], ControlsSide { inverted: false }, &mut driver, true);
        assert!((t.target_speed - t.max_turn_speed()).abs() < 1e-9);
    }

    #[test]
    fn a_key_is_alive_for_thirty_ticks_and_a_release_ends_it() {
        let mut keys = HeldKeys::default();
        keys.receive(&[FORWARD, LEFT], true);
        for _ in 0..(TIMEOUT - 1) {
            assert_eq!(keys.tick(), vec![FORWARD, LEFT]);
        }
        assert!(keys.tick().is_empty());
        keys.receive(&[FORWARD], true);
        keys.receive(&[FORWARD], false);
        assert!(keys.tick().is_empty());
        // A release of a key never pressed does nothing; a number outside the six is ignored.
        keys.receive(&[BACK, 9], false);
        keys.receive(&[9], true);
        assert!(keys.is_empty());
    }

    #[test]
    fn controls_facing_the_assembly_direction_are_not_inverted() {
        assert!(!controls_side([1, 0], [1, 0]).inverted);
        assert!(controls_side([-1, 0], [1, 0]).inverted);
    }
}
