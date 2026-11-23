//! What the blocks that sit on an edge point decide: the signal (`SignalBlock`, `SignalBlockEntity`), the observer
//! (`TrackObserverBlock`, `TrackObserverBlockEntity`) and the station (`StationBlock`, `StationBlockEntity`,
//! `StationEditPacket`). The glue reads the world, asks these, and applies the answer; the graph side is
//! `signal.rs`, `observer.rs`, `station.rs`, `assembly.rs` and `targeting.rs`.
//!
//! Left out: the computer behaviour (`computerBehaviour.hasAttachedComputer()` is always false here), the door
//! controls, the depot behaviour that holds an auto-schedule, map markers (`StationMarker`), the flag's animation.

use super::graph::Uuid;
use super::observer::TrackObserver;
use super::signal::{OverlayState, SignalBoundary, SignalState, SignalType};
use super::station::AssemblyError;
use super::track_shape::TrackShape;
use crate::Pos;

// --- the signal ----------------------------------------------------------------------------------------------

impl SignalType {
    /// The `type` property's value (`SignalBlock.SignalType.getSerializedName`).
    pub fn property_value(self) -> &'static str {
        match self {
            SignalType::EntrySignal => "entry_signal",
            SignalType::CrossSignal => "cross_signal",
        }
    }

    pub fn from_property(value: &str) -> Option<SignalType> {
        match value {
            "entry_signal" => Some(SignalType::EntrySignal),
            "cross_signal" => Some(SignalType::CrossSignal),
            _ => None,
        }
    }
}

impl SignalState {
    /// `NBTHelper.writeEnum`: the constant's name.
    pub fn nbt_name(self) -> &'static str {
        match self {
            SignalState::Red => "RED",
            SignalState::Yellow => "YELLOW",
            SignalState::Green => "GREEN",
            SignalState::Invalid => "INVALID",
        }
    }

    /// `NBTHelper.readEnum`: by name, the first constant (red) for anything else.
    pub fn from_nbt_name(name: &str) -> SignalState {
        match name {
            "YELLOW" => SignalState::Yellow,
            "GREEN" => SignalState::Green,
            "INVALID" => SignalState::Invalid,
            _ => SignalState::Red,
        }
    }

    /// `isRedLight(renderTime)`: red, or invalid for the first three ticks of every forty.
    pub fn is_red_light(self, render_time: f32) -> bool {
        self == SignalState::Red || (self == SignalState::Invalid && render_time % 40.0 < 3.0)
    }
}

impl OverlayState {
    pub fn nbt_name(self) -> &'static str {
        match self {
            OverlayState::Render => "RENDER",
            OverlayState::Skip => "SKIP",
            OverlayState::Dual => "DUAL",
        }
    }

    pub fn from_nbt_name(name: &str) -> OverlayState {
        match name {
            "SKIP" => OverlayState::Skip,
            "DUAL" => OverlayState::Dual,
            _ => OverlayState::Render,
        }
    }
}

/// `SignalBlockEntity.enterState`'s delay: a signal that turned green or yellow stays out of red this many ticks.
pub const SWITCH_TO_RED_DELAY: i32 = 15;

/// What a signal block entity tick found it must do besides keeping its own state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SignalTick {
    /// The block's `powered` property differs from the last reported: tell the boundary
    /// (`updateBlockEntityPower`).
    pub report_power: Option<bool>,
    /// The block's `type` differs from the boundary's: set it.
    pub set_type: Option<SignalType>,
    /// Something a client shows changed (`notifyUpdate`).
    pub changed: bool,
}

/// `SignalBlockEntity`'s fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SignalLights {
    pub state: SignalState,
    pub overlay: OverlayState,
    /// `switchToRedAfterTrainEntered`: ticks a green or yellow light keeps red off (not saved by Create).
    pub switch_to_red_after_train_entered: i32,
    pub last_reported_power: bool,
}

impl Default for SignalLights {
    fn default() -> Self {
        SignalLights::new()
    }
}

impl SignalLights {
    /// The constructor: invalid, skipping its overlay, no power reported.
    pub fn new() -> SignalLights {
        SignalLights { state: SignalState::Invalid, overlay: OverlayState::Skip, switch_to_red_after_train_entered: 0, last_reported_power: false }
    }

    /// `SignalBlockEntity.read`.
    pub fn from_saved(state: SignalState, overlay: OverlayState, power: bool) -> SignalLights {
        SignalLights { state, overlay, switch_to_red_after_train_entered: 0, last_reported_power: power }
    }

    /// `isPowered()`: what the comparator reads (15) and what the boundary's red is.
    pub fn is_powered(&self) -> bool {
        self.state == SignalState::Red
    }

    /// `getAnalogOutputSignal`.
    pub fn analog_output(&self) -> i32 {
        if self.is_powered() { 15 } else { 0 }
    }

    /// `enterState(state)`: true when the state changed.
    pub fn enter_state(&mut self, state: SignalState) -> bool {
        if self.switch_to_red_after_train_entered > 0 {
            self.switch_to_red_after_train_entered -= 1;
        }
        if self.state == state {
            return false;
        }
        if state == SignalState::Red && self.switch_to_red_after_train_entered > 0 {
            return false;
        }
        self.state = state;
        self.switch_to_red_after_train_entered = if matches!(state, SignalState::Green | SignalState::Yellow) { SWITCH_TO_RED_DELAY } else { 0 };
        true
    }

    /// `setOverlay(state)`: true when it changed.
    pub fn set_overlay(&mut self, overlay: OverlayState) -> bool {
        if self.overlay == overlay {
            return false;
        }
        self.overlay = overlay;
        true
    }

    /// `SignalBlockEntity.tick` (server): `boundary` is the signal's edge point, `pos` this block, `powered` and
    /// `block_type` what its block state says.
    pub fn tick(&mut self, boundary: Option<&SignalBoundary>, pos: Pos, powered: Option<bool>, block_type: Option<SignalType>) -> SignalTick {
        let mut out = SignalTick::default();
        let Some(boundary) = boundary else {
            out.changed |= self.enter_state(SignalState::Invalid);
            out.changed |= self.set_overlay(OverlayState::Render);
            return out;
        };
        if let Some(powered) = powered {
            if self.last_reported_power != powered {
                self.last_reported_power = powered;
                out.report_power = Some(powered);
                out.changed = true;
            }
        }
        if let Some(state_type) = block_type {
            let target = boundary.type_for(pos);
            if state_type != target {
                out.set_type = Some(target);
            }
        }
        out.changed |= self.enter_state(boundary.state_for(pos));
        out.changed |= self.set_overlay(boundary.overlay_for(pos));
        out
    }
}

/// How long `SignalBlock` waits (scheduled tick) before the power it was given goes out.
pub const SIGNAL_POWER_OFF_DELAY: i32 = 4;

/// `SignalBlock.neighborChanged` and `tick`, step by step: `powered` is the block's property, `neighbor` whether a
/// neighbour gives it a signal (`hasNeighborSignal`), `pending` the ticks left of a scheduled power-off (0 for none).
/// A neighbour's signal switches it on at once; one that stopped switches it off after four ticks if it hasn't come
/// back. Answers the new `powered` and `pending`.
pub fn signal_power_step(powered: bool, neighbor: bool, pending: i32) -> (bool, i32) {
    if powered == neighbor {
        return (powered, 0);
    }
    if !powered {
        return (true, 0);
    }
    // Powered, and the neighbour gave up: the scheduled tick (4 ticks) clears it if it stays so.
    if pending <= 0 {
        return (true, SIGNAL_POWER_OFF_DELAY);
    }
    if pending == 1 { (false, 0) } else { (true, pending - 1) }
}

/// `SignalBlock.onWrenched`'s message key.
pub fn signal_wrench_message(signal: Option<&SignalBoundary>, pos: Pos) -> String {
    match signal {
        Some(s) => format!("create.track_signal.mode_change.{}", s.type_for(pos).property_value()),
        None => "create.track_signal.cannot_change_mode".to_owned(),
    }
}

// --- the observer --------------------------------------------------------------------------------------------

/// `TrackObserverBlockEntity.tick`: whether the block's `powered` should change, and to what.
pub fn observer_power_step(block_powered: bool, observer: Option<&TrackObserver>) -> Option<bool> {
    let should_be_powered = observer.is_some_and(TrackObserver::is_activated);
    (block_powered != should_be_powered).then_some(should_be_powered)
}

/// `TrackObserverBlock.getSignal`.
pub fn observer_signal(powered: bool) -> i32 {
    if powered { 15 } else { 0 }
}

// --- the station ---------------------------------------------------------------------------------------------

/// `StationEditPacket.name`'s limit (`stringUtf8(256)`).
pub const MAX_STATION_NAME: usize = 256;

/// `StationEditPacket.applySettings`: a name only counts when it isn't blank.
pub fn station_name(name: &str) -> Option<String> {
    if name.trim().is_empty() || name.chars().count() > MAX_STATION_NAME { None } else { Some(name.to_owned()) }
}

/// What `StationEditPacket` carries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StationEdit {
    pub drop_schedule: bool,
    /// `assemblyMode`: enter (true) or leave (false) assembly mode once the rest is done.
    pub assembly_mode: bool,
    /// `tryAssemble`: `Some(true)` assemble, `Some(false)` disassemble, `None` neither.
    pub try_assemble: Option<bool>,
    pub name: Option<String>,
}

/// One step of what `applySettings` does, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditStep {
    DropSchedule,
    Rename(String),
    Assemble,
    /// `tryDisassembleTrain` and, when it did, `tryEnterAssemblyMode` and `refreshAssemblyInfo`.
    Disassemble,
    /// After `Assemble`: carry on only when the station has a present train now (`assemblyComplete`).
    ContinueIfTrainPresent,
    EnterAssembly,
    ExitAssembly,
}

/// `applySettings(player, be)`. `is_station` says the block is still a station, `assembling` its `assembling` state.
pub fn plan_edit(edit: &StationEdit, is_station: bool, assembling: bool) -> Vec<EditStep> {
    let mut steps = Vec::new();
    if edit.drop_schedule {
        steps.push(EditStep::DropSchedule);
        return steps;
    }
    if let Some(name) = edit.name.as_deref().and_then(station_name) {
        steps.push(EditStep::Rename(name));
    }
    if !is_station {
        return steps;
    }
    if let Some(assemble) = edit.try_assemble {
        if !assembling {
            return steps;
        }
        if assemble {
            steps.push(EditStep::Assemble);
            steps.push(EditStep::ContinueIfTrainPresent);
        } else {
            steps.push(EditStep::Disassemble);
            return steps;
        }
    }
    steps.push(if edit.assembly_mode { EditStep::EnterAssembly } else { EditStep::ExitAssembly });
    steps
}

/// The lines the `/station` command stands in for `StationScreen` and `AssemblyScreen`'s buttons with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StationCommand {
    Show,
    /// `name <text>`.
    Name(String),
    /// `assembly`: the "Assemble Train" button of the station screen: enter assembly mode.
    Assembly,
    /// `cancel`: leave assembly mode.
    Cancel,
    /// `assemble`: the assembly screen's "Assemble Train" (`tryAssemble`).
    Assemble,
    /// `disassemble`: `StationEditPacket.tryDisassemble`.
    Disassemble,
    /// `schedule`: `StationEditPacket.dropSchedule`, "Retrieve Schedule".
    Schedule,
}

impl StationCommand {
    pub fn parse(line: &str) -> Result<StationCommand, String> {
        let line = line.trim();
        let (word, rest) = line.split_once(char::is_whitespace).map_or((line, ""), |(w, r)| (w, r.trim()));
        Ok(match word.to_ascii_lowercase().as_str() {
            "" | "show" => StationCommand::Show,
            "name" => match station_name(rest) {
                Some(name) => StationCommand::Name(name),
                None => return Err("Give the station a name".into()),
            },
            "assembly" => StationCommand::Assembly,
            "cancel" => StationCommand::Cancel,
            "assemble" => StationCommand::Assemble,
            "disassemble" => StationCommand::Disassemble,
            "schedule" => StationCommand::Schedule,
            other => return Err(format!("Unknown station command '{other}' (name, assembly, assemble, cancel, disassemble, schedule)")),
        })
    }

    /// What the button sends.
    pub fn edit(&self) -> Option<StationEdit> {
        Some(match self {
            StationCommand::Show => return None,
            StationCommand::Name(name) => StationEdit { name: Some(name.clone()), ..StationEdit::default() },
            StationCommand::Assembly => StationEdit { assembly_mode: true, ..StationEdit::default() },
            StationCommand::Cancel => StationEdit::default(),
            StationCommand::Assemble => StationEdit { try_assemble: Some(true), ..StationEdit::default() },
            StationCommand::Disassemble => StationEdit { try_assemble: Some(false), ..StationEdit::default() },
            StationCommand::Schedule => StationEdit { drop_schedule: true, ..StationEdit::default() },
        })
    }
}

/// What `StationBlockEntity.tick` last told the clients (`imminentTrain`, `trainPresent`, ...).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StationShown {
    pub imminent_train: Option<Uuid>,
    pub train_present: bool,
    pub can_disassemble: bool,
    pub backwards: bool,
    pub has_schedule: bool,
    pub has_auto_schedule: bool,
}

/// `StationBlockEntity.tick`'s test for sending a fresh picture: `newlyArrived` or any shown value different.
/// Answers the new picture when it must go out, and whether the train just arrived (`applyAutoSchedule`).
pub fn station_tick(shown: &StationShown, now: &StationShown) -> (Option<StationShown>, bool) {
    let newly_arrived = shown.train_present != now.train_present;
    let differs = newly_arrived
        || shown.can_disassemble != now.can_disassemble
        || shown.imminent_train != now.imminent_train
        || shown.has_schedule != now.has_schedule
        || shown.has_auto_schedule != now.has_auto_schedule;
    (differs.then(|| now.clone()), newly_arrived)
}

/// `StationBlock.getAnalogOutputSignal`: a train is here.
pub fn station_analog_output(train_present: bool) -> i32 {
    if train_present { 15 } else { 0 }
}

/// `StationBlockEntity.tryEnterAssemblyMode`'s verdict on a track: it needs the station's track to be straight and
/// along one axis, and a station that targets a track.
pub fn can_enter_assembly_mode(has_valid_track: bool, track_axis: Option<[f64; 3]>) -> bool {
    has_valid_track && track_axis.is_some_and(super::station::track_axis_is_aligned)
}

/// The error a failed assembly shows, with its numbers: the lang key (`AssemblyError::key` is the part after `create.`) and
/// the arguments.
pub fn assembly_error_message(error: &AssemblyError) -> (String, Vec<usize>) {
    (format!("create.{}", error.key()), error.args())
}

// --- the targeting item ------------------------------------------------------------------------------------

/// `TrackTargetingBlockItem.useOn`'s reach: the selected track must be closer than 16 blocks to the placed block
/// (`closerThan`, over `maxTrackPlacementLength + 16` for a curve).
pub fn targeting_reach(bezier: bool, max_track_placement_length: i32) -> f64 {
    if bezier { f64::from(max_track_placement_length + 16) } else { 16.0 }
}

/// `Vec3i.closerThan(other, distance)`: the squared distance is below the square of `distance`.
pub fn closer_than(a: Pos, b: Pos, distance: f64) -> bool {
    let d = [f64::from(a[0] - b[0]), f64::from(a[1] - b[1]), f64::from(a[2] - b[2])];
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < distance * distance
}

// --- the messages ----------------------------------------------------------------------------------------------

/// The English of the lang keys the blocks and items of this part show (`create.*` in `en_us.json`, the same text the
/// content's lang file holds), for the places the host takes a literal. `None` for a key not listed.
pub fn message_text(key: &str) -> Option<&'static str> {
    Some(match key {
        "create.track_target.clear" => "Cleared track selection",
        "create.track_target.invalid" => "Cannot target this track here",
        "create.track_target.missing" => "Right-click the targeted train track first",
        "create.track_target.no_junctions" => "Targeted track cannot be an intersection",
        "create.track_target.occupied" => "Targeted track is occupied",
        "create.track_target.set" => "Targeted track selected",
        "create.track_target.success" => "Successfully bound to targeted track",
        "create.track_target.too_far" => "Targeted track is too far from here",
        "create.track_signal.cannot_change_mode" => "Unable to switch mode of this Signal",
        "create.track_signal.mode_change.cross_signal" => "-> Allow passage if section fully traversable",
        "create.track_signal.mode_change.entry_signal" => "-> Allow passage if section unoccupied",
        "create.train_assembly.bogey_created" => "Bogey created. Click again to cycle type",
        "create.train_assembly.bogeys_too_close" => "Bogeys %1$s and %2$s are too close to each other",
        "create.train_assembly.frontmost_bogey_at_station" => "Frontmost Bogey must be at Station Marker",
        "create.train_assembly.no_bogeys" => "No Bogeys Found",
        "create.train_assembly.no_controls" => "Attach at least one forward-facing Train Controls block (Are you missing Super Glue?)",
        "create.train_assembly.not_connected_in_order" => "Bogeys are not connected in order",
        "create.train_assembly.nothing_attached" => "No structure attached to Bogey %1$s",
        "create.train_assembly.requires_casing" => "Use Train Casing to create bogeys on tracks",
        "create.train_assembly.single_bogey_carriage" => "This Bogey type cannot support a carriage on its own",
        "create.bogey.style.no_other_sizes" => "No other sizes",
        "create.schedule.applied_to_train" => "Train is now following this Schedule",
        "create.schedule.no_stops" => "This Schedule does not have any Stops yet",
        _ => return None,
    })
}

/// A lang string with its `%1$s`, `%2$s` filled in; the key itself when it isn't listed.
pub fn format_message(key: &str, args: &[String]) -> String {
    let mut text = message_text(key).unwrap_or(key).to_owned();
    for (i, arg) in args.iter().enumerate() {
        text = text.replace(&format!("%{}$s", i + 1), arg);
    }
    text
}

/// `CTrains.maxAssemblyLength`: the longest assembly track of a station (default 128).
pub const MAX_ASSEMBLY_LENGTH: i32 = 128;
/// `CTrains.maxBogeyCount`: the most bogeys a train takes (default 20).
pub const MAX_BOGEY_COUNT: usize = 20;

// --- the assembly area and the bogeys -------------------------------------------------------------------------

/// `assemblyAreas`' box: `BoundingBox.fromCorners(startPosition, trackEnd)` with the start next to the station's
/// target and the end `length - 1` further along the assembly direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AssemblyArea {
    pub min: Pos,
    pub max: Pos,
}

impl AssemblyArea {
    pub fn new(target: Pos, direction: [i32; 3], length: i32) -> AssemblyArea {
        let start = [target[0] + direction[0], target[1] + direction[1], target[2] + direction[2]];
        let n = length - 1;
        let end = [start[0] + direction[0] * n, start[1] + direction[1] * n, start[2] + direction[2] * n];
        AssemblyArea {
            min: [start[0].min(end[0]), start[1].min(end[1]), start[2].min(end[2])],
            max: [start[0].max(end[0]), start[1].max(end[1]), start[2].max(end[2])],
        }
    }

    /// `BoundingBox.isInside`.
    pub fn contains(&self, p: Pos) -> bool {
        (0..3).all(|i| p[i] >= self.min[i] && p[i] <= self.max[i])
    }
}

/// `BlockPos.distManhattan`.
pub fn dist_manhattan(a: Pos, b: Pos) -> i32 {
    (a[0] - b[0]).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs()
}

/// What a click on the track in the assembly area comes to (`StationBlockEntity.trackClicked`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackClick {
    /// Not in the assembly area: not handled.
    Ignored,
    /// The spot is no valid bogey offset: a bogey beside it (within one block along the assembly direction, above
    /// the track, then below) changes size, if the glue finds one.
    CycleSize,
    /// "Use Train Casing to create bogeys on tracks" (`train_assembly.requires_casing`).
    NeedsCasing,
    /// A bogey goes above the track, taking a casing unless the player is in creative.
    PlaceBogey { consume_casing: bool },
}

/// `trackClicked`'s decisions up to the bogey being placed.
pub fn track_click(in_area: bool, valid_offset: bool, creative: bool, holds_casing: bool) -> TrackClick {
    if !in_area {
        return TrackClick::Ignored;
    }
    if !valid_offset {
        return TrackClick::CycleSize;
    }
    if !creative && !holds_casing {
        return TrackClick::NeedsCasing;
    }
    TrackClick::PlaceBogey { consume_casing: !creative }
}

/// `trackClicked`'s search for a bogey to change: along the assembly direction -1, 0, 1, above the track
/// (`false`) and then below (`true`).
pub fn bogey_search_order() -> Vec<(bool, i32)> {
    let mut order = Vec::new();
    for upside_down in [false, true] {
        for i in -1..=1 {
            order.push((upside_down, i));
        }
    }
    order
}

/// `AbstractBogeyBlockEntity`'s next size for the standard style (`style.getNextBlock(size)`): small and large take
/// turns. `None` for another block.
pub fn next_bogey_block(block: &str) -> Option<&'static str> {
    match block {
        "create:small_bogey" => Some("create:large_bogey"),
        "create:large_bogey" => Some("create:small_bogey"),
        _ => None,
    }
}

/// `TrackBlock.getBogeyAnchor`: a bogey over `shape`, turned `x` over an x track and `z` otherwise.
pub fn bogey_axis(shape: TrackShape) -> &'static str {
    if shape == TrackShape::Xo { "x" } else { "z" }
}

#[cfg(test)]
mod tests {
    use super::super::graph::EdgePointType;
    use super::super::network::RailNetworks;
    use super::super::network::tests::line;
    use super::*;

    #[test]
    fn a_signal_entering_green_stays_out_of_red_for_fifteen_ticks() {
        let mut s = SignalLights::new();
        assert_eq!(s.state, SignalState::Invalid);
        assert!(s.enter_state(SignalState::Green));
        assert_eq!(s.analog_output(), 0);
        // Red is refused until the delay has run down.
        for _ in 0..14 {
            assert!(!s.enter_state(SignalState::Red));
            assert_eq!(s.state, SignalState::Green);
        }
        assert!(s.enter_state(SignalState::Red));
        assert!(s.is_powered());
        assert_eq!(s.analog_output(), 15);
        // Back to red at once from invalid.
        let mut s = SignalLights::new();
        assert!(s.enter_state(SignalState::Red));
        // Same state: unchanged.
        assert!(!s.enter_state(SignalState::Red));
    }

    #[test]
    fn yellow_also_holds_red_off() {
        let mut s = SignalLights::from_saved(SignalState::Red, OverlayState::Render, false);
        assert!(s.enter_state(SignalState::Yellow));
        assert!(!s.enter_state(SignalState::Red));
    }

    #[test]
    fn the_red_light_blinks_when_invalid() {
        assert!(SignalState::Red.is_red_light(100.0));
        assert!(SignalState::Invalid.is_red_light(41.0));
        assert!(!SignalState::Invalid.is_red_light(10.0));
        assert!(!SignalState::Green.is_red_light(1.0));
    }

    #[test]
    fn enum_names_round_trip() {
        for s in [SignalState::Red, SignalState::Yellow, SignalState::Green, SignalState::Invalid] {
            assert_eq!(SignalState::from_nbt_name(s.nbt_name()), s);
        }
        for o in [OverlayState::Render, OverlayState::Skip, OverlayState::Dual] {
            assert_eq!(OverlayState::from_nbt_name(o.nbt_name()), o);
        }
        for t in [SignalType::EntrySignal, SignalType::CrossSignal] {
            assert_eq!(SignalType::from_property(t.property_value()), Some(t));
        }
    }

    #[test]
    fn a_signal_without_a_boundary_is_invalid_and_draws_its_overlay() {
        let mut s = SignalLights::from_saved(SignalState::Green, OverlayState::Skip, false);
        let out = s.tick(None, [0, 0, 0], Some(false), Some(SignalType::EntrySignal));
        assert_eq!((s.state, s.overlay), (SignalState::Invalid, OverlayState::Render));
        assert!(out.changed);
        assert_eq!(out.report_power, None);
    }

    #[test]
    fn a_signal_reports_power_and_follows_the_boundarys_type_and_state() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 100.0]);
        let mut point = super::super::graph::TrackEdgePoint::new_signal(5, [super::super::network::tests::loc(0.0, 0.0), super::super::network::tests::loc(100.0, 0.0)], 30.0);
        let boundary = point.signal_mut().unwrap();
        boundary.block_entity_added([1, 2, 3], true, Some(SignalType::CrossSignal), false);
        nets.add_point(g, point);
        let boundary = nets.graphs[&g].point(EdgePointType::Signal, 5).unwrap().signal().unwrap().clone();
        let mut s = SignalLights::new();
        let out = s.tick(Some(&boundary), [1, 2, 3], Some(true), Some(SignalType::EntrySignal));
        assert_eq!(out.report_power, Some(true));
        assert_eq!(out.set_type, Some(SignalType::CrossSignal));
        assert!(s.last_reported_power);
        // Told once: the same power again says nothing.
        let out = s.tick(Some(&boundary), [1, 2, 3], Some(true), Some(SignalType::CrossSignal));
        assert_eq!(out.report_power, None);
        assert_eq!(out.set_type, None);
        // The wrench message names the type.
        assert_eq!(signal_wrench_message(Some(&boundary), [1, 2, 3]), "create.track_signal.mode_change.cross_signal");
        assert_eq!(signal_wrench_message(None, [1, 2, 3]), "create.track_signal.cannot_change_mode");
    }

    #[test]
    fn redstone_powers_a_signal_at_once_and_releases_it_after_four_ticks() {
        assert_eq!(signal_power_step(false, true, 0), (true, 0));
        assert_eq!(signal_power_step(true, true, 3), (true, 0));
        assert_eq!(signal_power_step(false, false, 0), (false, 0));
        let mut state = (true, 0);
        let mut ticks = 0;
        while state.0 {
            state = signal_power_step(state.0, false, state.1);
            ticks += 1;
            assert!(ticks <= 10);
        }
        assert_eq!(ticks, SIGNAL_POWER_OFF_DELAY as usize + 1 - 0);
        // The neighbour coming back cancels the release.
        let (p, pending) = signal_power_step(true, false, 0);
        assert_eq!((p, pending), (true, 4));
        assert_eq!(signal_power_step(p, true, pending), (true, 0));
    }

    #[test]
    fn an_observer_is_powered_while_activated() {
        assert_eq!(observer_power_step(false, None), None);
        assert_eq!(observer_power_step(true, None), Some(false));
        let mut o = TrackObserver::new();
        assert_eq!(observer_power_step(false, Some(&o)), None);
        o.keep_alive(1);
        assert_eq!(observer_power_step(false, Some(&o)), Some(true));
        assert_eq!(observer_power_step(true, Some(&o)), None);
        assert_eq!(observer_signal(true), 15);
        assert_eq!(observer_signal(false), 0);
    }

    #[test]
    fn station_edits_follow_the_packet() {
        let rename = StationEdit { name: Some("Home".into()), ..StationEdit::default() };
        assert_eq!(plan_edit(&rename, true, false), vec![EditStep::Rename("Home".into()), EditStep::ExitAssembly]);
        let blank = StationEdit { name: Some("  ".into()), ..StationEdit::default() };
        assert_eq!(plan_edit(&blank, true, false), vec![EditStep::ExitAssembly]);
        assert_eq!(plan_edit(&rename, false, false), vec![EditStep::Rename("Home".into())]);
        let enter = StationEdit { assembly_mode: true, ..StationEdit::default() };
        assert_eq!(plan_edit(&enter, true, false), vec![EditStep::EnterAssembly]);
        let assemble = StationEdit { try_assemble: Some(true), ..StationEdit::default() };
        assert_eq!(plan_edit(&assemble, true, true), vec![EditStep::Assemble, EditStep::ContinueIfTrainPresent, EditStep::ExitAssembly]);
        assert!(plan_edit(&assemble, true, false).is_empty());
        let disassemble = StationEdit { try_assemble: Some(false), ..StationEdit::default() };
        assert_eq!(plan_edit(&disassemble, true, true), vec![EditStep::Disassemble]);
        let drop = StationEdit { drop_schedule: true, name: Some("x".into()), ..StationEdit::default() };
        assert_eq!(plan_edit(&drop, true, true), vec![EditStep::DropSchedule]);
    }

    #[test]
    fn station_commands() {
        assert_eq!(StationCommand::parse("name Central Station"), Ok(StationCommand::Name("Central Station".into())));
        assert!(StationCommand::parse("name  ").is_err());
        assert_eq!(StationCommand::parse(""), Ok(StationCommand::Show));
        assert_eq!(StationCommand::parse("ASSEMBLE").unwrap().edit(), Some(StationEdit { try_assemble: Some(true), ..StationEdit::default() }));
        assert_eq!(StationCommand::parse("cancel").unwrap().edit(), Some(StationEdit::default()));
        assert!(StationCommand::parse("fly").is_err());
        assert_eq!(StationCommand::Show.edit(), None);
    }

    #[test]
    fn the_station_tells_clients_only_of_changes() {
        let idle = StationShown::default();
        let (send, arrived) = station_tick(&idle, &idle);
        assert!(send.is_none() && !arrived);
        let here = StationShown { imminent_train: Some(4), train_present: true, can_disassemble: true, ..StationShown::default() };
        let (send, arrived) = station_tick(&idle, &here);
        assert_eq!(send, Some(here.clone()));
        assert!(arrived);
        let (send, arrived) = station_tick(&here, &StationShown { has_schedule: true, ..here.clone() });
        assert!(send.is_some() && !arrived);
        assert_eq!(station_analog_output(true), 15);
    }

    #[test]
    fn assembly_mode_needs_a_straight_axis_track() {
        assert!(can_enter_assembly_mode(true, Some([1.0, 0.0, 0.0])));
        assert!(can_enter_assembly_mode(true, Some([0.0, 0.0, -1.0])));
        assert!(!can_enter_assembly_mode(true, Some([1.0, 0.0, 1.0])));
        assert!(!can_enter_assembly_mode(false, Some([1.0, 0.0, 0.0])));
        assert!(!can_enter_assembly_mode(true, None));
    }

    #[test]
    fn the_assembly_area_starts_next_to_the_station() {
        let area = AssemblyArea::new([10, 64, 0], [1, 0, 0], 5);
        assert_eq!((area.min, area.max), ([11, 64, 0], [15, 64, 0]));
        let west = AssemblyArea::new([10, 64, 0], [-1, 0, 0], 5);
        assert_eq!((west.min, west.max), ([5, 64, 0], [9, 64, 0]));
        assert!(area.contains([12, 64, 0]) && !area.contains([16, 64, 0]) && !area.contains([12, 65, 0]));
        assert_eq!(dist_manhattan([10, 64, 0], [13, 64, 0]) - 1, 2);
    }

    #[test]
    fn messages_fill_their_numbers() {
        let (key, args) = assembly_error_message(&AssemblyError::BogeysTooClose { a: 2, b: 3 });
        let args: Vec<String> = args.iter().map(ToString::to_string).collect();
        assert_eq!(format_message(&key, &args), "Bogeys 2 and 3 are too close to each other");
        assert_eq!(format_message("create.track_target.set", &[]), "Targeted track selected");
        assert_eq!(format_message("create.nothing", &[]), "create.nothing");
        for error in [AssemblyError::NoBogeys, AssemblyError::NoControls, AssemblyError::SingleBogeyCarriage, AssemblyError::NotConnectedInOrder, AssemblyError::FrontmostBogeyAtStation, AssemblyError::NothingAttached { carriage: 1 }] {
            let (key, _) = assembly_error_message(&error);
            assert!(message_text(&key).is_some(), "{key}");
        }
    }

    #[test]
    fn the_targeting_item_reaches_sixteen_blocks() {
        assert!(closer_than([0, 0, 0], [15, 0, 0], targeting_reach(false, 32)));
        assert!(!closer_than([0, 0, 0], [16, 0, 0], targeting_reach(false, 32)));
        assert!(!closer_than([0, 0, 0], [12, 12, 0], targeting_reach(false, 32)));
        assert!(closer_than([0, 0, 0], [40, 0, 0], targeting_reach(true, 32)));
    }

    #[test]
    fn clicking_the_track() {
        assert_eq!(track_click(false, true, true, true), TrackClick::Ignored);
        assert_eq!(track_click(true, false, false, false), TrackClick::CycleSize);
        assert_eq!(track_click(true, true, false, false), TrackClick::NeedsCasing);
        assert_eq!(track_click(true, true, false, true), TrackClick::PlaceBogey { consume_casing: true });
        assert_eq!(track_click(true, true, true, false), TrackClick::PlaceBogey { consume_casing: false });
        assert_eq!(bogey_search_order().len(), 6);
        assert_eq!(bogey_search_order()[0], (false, -1));
        assert_eq!(bogey_search_order()[5], (true, 1));
        assert_eq!(next_bogey_block("create:small_bogey"), Some("create:large_bogey"));
        assert_eq!(next_bogey_block("create:large_bogey"), Some("create:small_bogey"));
        assert_eq!(next_bogey_block("create:track"), None);
        assert_eq!(bogey_axis(TrackShape::Xo), "x");
        assert_eq!(bogey_axis(TrackShape::Zo), "z");
        assert_eq!(bogey_axis(TrackShape::Pd), "z");
    }
}
