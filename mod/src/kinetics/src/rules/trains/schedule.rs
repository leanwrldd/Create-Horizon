//! Train schedules (content/trains/schedule/): the data model (`Schedule`, `ScheduleEntry`, the five
//! `ScheduleInstruction`s and nine `ScheduleWaitCondition`s), what each wait condition decides, what
//! each instruction does when it starts, and `ScheduleRuntime`'s state machine.
//!
//! Java keeps an instruction's or condition's settings in a `CompoundTag` (`Text`, `Value`,
//! `TimeUnit`, `Hour` ...) and a condition's running state in another (`Time`, `StatusVersion`,
//! `LastChecked` ...). Here both are fields: [`ScheduleInstruction`] and [`WaitCondition`] variants
//! hold the settings with Create's defaults (`new` of each), and [`ConditionContext`] is the running
//! state. Item, fluid and frequency filters are opaque ids the world side matches.
//!
//! The train and the world are not here. [`ScheduleTrain`] is what the runtime and the instructions
//! ask of a train (conductors, stations of its graph, navigation, status reports, carriage storage),
//! [`ConditionWorld`] what a wait condition reads (day time, cargo, redstone, passengers).
//!
//! Glob matching of station names and addresses is [`crate::rules::package::Glob`], itself
//! reconstructed from Catnip's `Glob` (unverified there).
//!
//! Left out: NBT and packet reading and writing (`write`, `fromTag`, stream codecs), the editing
//! screens, titles, summaries and icons, the waiting status texts (the numbers behind two of them
//! are here), `getTypeOptions`, `ScheduleItem`, `DestinationSuggestions`, hats.

use super::graph::Uuid;
use super::navigation::DiscoveredPath;
use crate::rules::package::{Glob, is_blank, match_address};

/// `Schedule.INSTRUCTION_TYPES`, in registration order (ids under `create:`).
pub const INSTRUCTION_TYPES: [&str; 5] = ["destination", "package_delivery", "package_retrieval", "rename", "throttle"];
/// `Schedule.CONDITION_TYPES`, in registration order.
pub const CONDITION_TYPES: [&str; 9] =
    ["delay", "time_of_day", "fluid_threshold", "item_threshold", "redstone_link", "player_count", "idle", "unloaded", "powered"];

/// `ScheduleRuntime.TBD`: a transit time not known yet.
pub const TBD: i32 = -1;
/// `ScheduleRuntime.INVALID`.
pub const INVALID: i32 = -2;
/// `ScheduleRuntime.INTERVAL`: ticks of cooldown after a failed start.
pub const INTERVAL: i32 = 40;

// --- conditions ------------------------------------------------------------------------------

/// A wait condition's running state (the context tag).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConditionContext {
    /// `Time`.
    pub time: i32,
    /// `StatusVersion`: raised when the waiting status text changes.
    pub status_version: i32,
    /// `LastChecked`.
    pub last_checked: Option<i32>,
    /// `CurrentDisplay` (threshold conditions).
    pub current_display: Option<i32>,
    /// `PrevPlayerCount`.
    pub prev_player_count: i32,
}

impl ConditionContext {
    /// `requestStatusToUpdate(context)`.
    pub fn request_status_to_update(&mut self) {
        self.status_version = self.status_version.wrapping_add(1);
    }

    /// `CargoThresholdCondition.requestStatusToUpdate(amount, context)`.
    fn request_threshold_display(&mut self, amount: i32) {
        self.current_display = Some(amount);
        self.request_status_to_update();
    }
}

/// `TimedWaitCondition.TimeUnit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeUnit {
    Ticks,
    Seconds,
    Minutes,
}

impl TimeUnit {
    pub const ALL: [TimeUnit; 3] = [TimeUnit::Ticks, TimeUnit::Seconds, TimeUnit::Minutes];

    pub fn ticks_per(self) -> i32 {
        match self {
            TimeUnit::Ticks => 1,
            TimeUnit::Seconds => 20,
            TimeUnit::Minutes => 20 * 60,
        }
    }

    pub fn suffix(self) -> &'static str {
        match self {
            TimeUnit::Ticks => "t",
            TimeUnit::Seconds => "s",
            TimeUnit::Minutes => "min",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            TimeUnit::Ticks => "generic.unit.ticks",
            TimeUnit::Seconds => "generic.unit.seconds",
            TimeUnit::Minutes => "generic.unit.minutes",
        }
    }

    /// `enumData("TimeUnit")`: the stored index modulo the number of units.
    pub fn from_index(i: i32) -> TimeUnit {
        TimeUnit::ALL[i.rem_euclid(3) as usize]
    }
}

/// `TimedWaitCondition`'s settings: `Value` (default 5) and `TimeUnit` (default seconds).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimedWait {
    pub value: i32,
    /// The stored `TimeUnit` index (0 ticks, 1 seconds, 2 minutes).
    pub unit: i32,
}

impl Default for TimedWait {
    fn default() -> Self {
        TimedWait { value: 5, unit: TimeUnit::Seconds as i32 }
    }
}

impl TimedWait {
    pub fn unit(&self) -> TimeUnit {
        TimeUnit::from_index(self.unit)
    }

    /// `totalWaitTicks()`.
    pub fn total_wait_ticks(&self) -> i32 {
        self.value.wrapping_mul(self.unit().ticks_per())
    }

    /// `requestDisplayIfNecessary(context, time)`.
    fn request_display_if_necessary(&self, ctx: &mut ConditionContext, time: i32) {
        let until = self.total_wait_ticks().wrapping_sub(time);
        if until < 20 * 60 && until % 100 == 0 {
            ctx.request_status_to_update();
        }
        if until >= 20 * 60 && until % (20 * 60) == 0 {
            ctx.request_status_to_update();
        }
    }

    /// `getWaitingStatus`'s number: the seconds (rounded up to 5) or minutes (rounded down) left, and
    /// whether it counts minutes.
    pub fn waiting_status(&self, time: i32) -> (i32, bool) {
        let until = self.total_wait_ticks() - time;
        let in_minutes = until >= 20 * 60;
        let num = if in_minutes {
            (f64::from(until as f32 / (20.0 * 60.0))).floor() as i32
        } else {
            ((f64::from(until as f32 / 100.0)).ceil() * 5.0) as i32
        };
        (num, in_minutes)
    }
}

/// `CargoThresholdCondition.Ops`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ops {
    Greater,
    Less,
    Equal,
}

impl Ops {
    pub const ALL: [Ops; 3] = [Ops::Greater, Ops::Less, Ops::Equal];

    pub fn formatted(self) -> &'static str {
        match self {
            Ops::Greater => ">",
            Ops::Less => "<",
            Ops::Equal => "=",
        }
    }

    pub fn test(self, current: i32, target: i32) -> bool {
        match self {
            Ops::Greater => current > target,
            Ops::Equal => current == target,
            Ops::Less => current < target,
        }
    }

    /// `enumData("Operator")`.
    pub fn from_index(i: i32) -> Ops {
        Ops::ALL[i.rem_euclid(3) as usize]
    }
}

/// `CargoThresholdCondition`'s settings: `Threshold` (text, default "10"), `Operator`, `Measure`, and
/// the filter (`Item` or `Bucket`, an opaque id; `None` is the empty filter that matches anything).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Threshold {
    pub threshold: String,
    pub operator: i32,
    pub measure: i32,
    pub filter: Option<String>,
}

impl Default for Threshold {
    fn default() -> Self {
        Threshold { threshold: "10".to_string(), operator: 0, measure: 0, filter: None }
    }
}

impl Threshold {
    pub fn operator(&self) -> Ops {
        Ops::from_index(self.operator)
    }

    /// `getThreshold()`: `Integer.valueOf` of the text, which on a bad one is reset to "0".
    pub fn threshold(&mut self) -> i32 {
        match self.threshold.parse::<i32>() {
            Ok(v) => v,
            Err(_) => {
                self.threshold = "0".to_string();
                0
            }
        }
    }

    /// `ItemThresholdCondition.inStacks()`.
    pub fn in_stacks(&self) -> bool {
        self.measure == 1
    }

    /// `getWaitingStatus`'s numbers (item and fluid): the last counted amount, and the threshold
    /// moved one toward the side the operator wants (never below 0). `None` before the first count.
    pub fn waiting_status(&mut self, ctx: &ConditionContext) -> Option<(i32, i32)> {
        let shown = ctx.current_display?;
        let offset = match self.operator() {
            Ops::Less => -1,
            Ops::Greater => 1,
            Ops::Equal => 0,
        };
        Some((shown, (self.threshold() + offset).max(0)))
    }
}

/// `TimeOfDayCondition`'s settings: `Hour` (default 8), `Minute`, `Rotation` (an index, default 5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimeOfDay {
    pub hour: i32,
    pub minute: i32,
    pub rotation: i32,
}

impl Default for TimeOfDay {
    fn default() -> Self {
        TimeOfDay { hour: 8, minute: 0, rotation: 5 }
    }
}

impl TimeOfDay {
    /// `getRotation()`: the length in ticks of the cycle the time repeats in.
    pub fn rotation_ticks(&self) -> i32 {
        match self.rotation {
            9 => 250,
            8 => 500,
            7 => 750,
            6 => 1000,
            5 => 2000,
            4 => 3000,
            3 => 4000,
            2 => 6000,
            1 => 12000,
            _ => 24000,
        }
    }

    /// The tick within the rotation the time falls on (`targetTicks`): the hour shifted 6 hours (day
    /// time 0 is 6:00), the minute in thousandths of an hour, rounded up.
    pub fn target_ticks(&self) -> i32 {
        let minute_ticks = f64::from(self.minute as f32 / 60.0 * 1000.0).ceil();
        let rotation = f64::from(self.rotation_ticks());
        ((f64::from(((self.hour + 18) % 24) * 1000) + minute_ticks) % rotation) as i32
    }

    /// `getWaitingStatus`'s departure: the `(hour, minute)` of the next time it is met.
    pub fn next_departure(&self, day_time: i64) -> (i32, i32) {
        let rotation = i64::from(self.rotation_ticks());
        let current = (day_time % rotation) as i32;
        let mut diff = self.target_ticks() - current;
        if diff < 0 {
            diff += self.rotation_ticks();
        }
        let departure_time = ((day_time + i64::from(diff)) as i32) % 24000;
        ((departure_time / 1000 + 6) % 24, (departure_time % 1000) * 60 / 1000)
    }
}

/// A `ScheduleWaitCondition`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WaitCondition {
    /// `create:delay`.
    Delay(TimedWait),
    /// `create:idle`: no cargo exchanged for the time.
    Idle(TimedWait),
    TimeOfDay(TimeOfDay),
    /// `create:fluid_threshold`.
    FluidThreshold(Threshold),
    /// `create:item_threshold`.
    ItemThreshold(Threshold),
    /// `create:redstone_link`: the two frequency item ids, and `Inverted` (1 waits for no power).
    RedstoneLink { frequency: [String; 2], inverted: i32 },
    /// `create:player_count`: `Count` and `Exact` (0 exactly, otherwise or above).
    PlayerCount { count: i32, exact: i32 },
    /// `create:unloaded`: the station's chunk is not ticking.
    Unloaded,
    /// `create:powered`: the station block has a redstone signal.
    Powered,
}

impl WaitCondition {
    /// The `create:` id's path.
    pub fn id(&self) -> &'static str {
        match self {
            WaitCondition::Delay(_) => "delay",
            WaitCondition::Idle(_) => "idle",
            WaitCondition::TimeOfDay(_) => "time_of_day",
            WaitCondition::FluidThreshold(_) => "fluid_threshold",
            WaitCondition::ItemThreshold(_) => "item_threshold",
            WaitCondition::RedstoneLink { .. } => "redstone_link",
            WaitCondition::PlayerCount { .. } => "player_count",
            WaitCondition::Unloaded => "unloaded",
            WaitCondition::Powered => "powered",
        }
    }

    /// The condition the schedule editor creates for an id (its constructor's defaults), or `None`
    /// for an id that is not registered (`fromTag` warns and gives null).
    pub fn from_id(id: &str) -> Option<WaitCondition> {
        let id = id.strip_prefix("create:").unwrap_or(id);
        Some(match id {
            "delay" => WaitCondition::Delay(TimedWait::default()),
            "idle" => WaitCondition::Idle(TimedWait::default()),
            "time_of_day" => WaitCondition::TimeOfDay(TimeOfDay::default()),
            "fluid_threshold" => WaitCondition::FluidThreshold(Threshold::default()),
            "item_threshold" => WaitCondition::ItemThreshold(Threshold::default()),
            "redstone_link" => WaitCondition::RedstoneLink { frequency: [String::new(), String::new()], inverted: 0 },
            "player_count" => WaitCondition::PlayerCount { count: 0, exact: 0 },
            "unloaded" => WaitCondition::Unloaded,
            "powered" => WaitCondition::Powered,
            _ => return None,
        })
    }

    /// `totalWaitTicks()` of a `ScheduledDelay` (an idle condition is not one, as far as stay
    /// estimates go).
    fn delay_ticks(&self) -> Option<i32> {
        match self {
            WaitCondition::Delay(t) => Some(t.total_wait_ticks()),
            _ => None,
        }
    }

    /// `tickCompletion(level, train, context)`: true when the condition is met.
    pub fn tick_completion(&mut self, world: &dyn ConditionWorld, ctx: &mut ConditionContext) -> bool {
        match self {
            WaitCondition::Delay(t) => {
                let time = ctx.time;
                if time >= t.total_wait_ticks() {
                    return true;
                }
                ctx.time = time + 1;
                t.request_display_if_necessary(ctx, time);
                false
            }
            WaitCondition::Idle(t) => {
                let idle_time = world.min_ticks_since_last_exchange();
                ctx.time = idle_time;
                t.request_display_if_necessary(ctx, idle_time);
                idle_time > t.total_wait_ticks()
            }
            WaitCondition::TimeOfDay(t) => {
                let max_tick_diff = 40;
                let day_time = (world.day_time() % i64::from(t.rotation_ticks())) as i32;
                let diff = day_time - t.target_ticks();
                diff >= 0 && max_tick_diff >= diff
            }
            WaitCondition::ItemThreshold(_) | WaitCondition::FluidThreshold(_) => {
                // `LazyTickedScheduleCondition(20)`.
                let time = ctx.time;
                let mut next = time;
                if time % 20 == 0 {
                    if self.lazy_threshold(world, ctx) {
                        return true;
                    }
                    next = 0;
                }
                ctx.time = next + 1;
                false
            }
            WaitCondition::RedstoneLink { frequency, inverted } => {
                let status = world.redstone_power_version();
                if ctx.last_checked == Some(status) {
                    return false;
                }
                ctx.last_checked = Some(status);
                world.has_any_loaded_power(frequency) != (*inverted == 1)
            }
            WaitCondition::PlayerCount { count, exact } => {
                let prev = ctx.prev_player_count;
                let present = world.count_player_passengers();
                ctx.prev_player_count = present;
                if prev != present {
                    ctx.request_status_to_update();
                }
                if *exact != 0 { present >= *count } else { present == *count }
            }
            WaitCondition::Unloaded => world.station_unloaded(),
            WaitCondition::Powered => world.station_powered(),
        }
    }

    /// `CargoThresholdCondition.lazyTickCompletion`: only when the cargo changed since the last look.
    fn lazy_threshold(&mut self, world: &dyn ConditionWorld, ctx: &mut ConditionContext) -> bool {
        let status = world.cargo_version();
        if ctx.last_checked == Some(status) {
            return false;
        }
        ctx.last_checked = Some(status);
        match self {
            WaitCondition::ItemThreshold(t) => {
                let operator = t.operator();
                let target = t.threshold();
                let stacks = t.in_stacks();
                let mut found = 0;
                for (count, max) in world.matching_item_slots(t.filter.as_deref()) {
                    if stacks {
                        found += i32::from(count == max);
                    } else {
                        found += count;
                    }
                }
                ctx.request_threshold_display(found);
                operator.test(found, target)
            }
            WaitCondition::FluidThreshold(t) => {
                let operator = t.operator();
                let target = t.threshold();
                let mut found: i32 = 0;
                for amount in world.matching_fluid_amounts(t.filter.as_deref()) {
                    found = found.wrapping_add(amount);
                }
                ctx.request_threshold_display(found / 1000);
                operator.test(found, target.wrapping_mul(1000))
            }
            _ => false,
        }
    }
}

/// What wait conditions read of the world and the train.
pub trait ConditionWorld {
    /// `level.getDayTime()`.
    fn day_time(&self) -> i64;
    /// The sum of the carriages' `storage.getVersion()`.
    fn cargo_version(&self) -> i32;
    /// The least `storage.getTicksSinceLastExchange()` over the carriages (`i32::MAX` with none).
    fn min_ticks_since_last_exchange(&self) -> i32;
    /// Every slot of every carriage's item storage that the filter accepts, as `(count, max stack
    /// size)`. A `None` filter accepts anything.
    fn matching_item_slots(&self, filter: Option<&str>) -> Vec<(i32, i32)>;
    /// The amount (mB) in every tank of every carriage that the filter accepts.
    fn matching_fluid_amounts(&self, filter: Option<&str>) -> Vec<i32>;
    /// `REDSTONE_LINK_NETWORK_HANDLER.globalPowerVersion`.
    fn redstone_power_version(&self) -> i32;
    /// `hasAnyLoadedPower(frequency)`.
    fn has_any_loaded_power(&self, frequency: &[String; 2]) -> bool;
    /// `train.countPlayerPassengers()`.
    fn count_player_passengers(&self) -> i32;
    /// The train's station block has a neighbour signal: false with no station, or where it isn't
    /// loaded.
    fn station_powered(&self) -> bool;
    /// The train's station is not entity ticking: false with no station or no level.
    fn station_unloaded(&self) -> bool;
}

// --- instructions ----------------------------------------------------------------------------

/// A `ScheduleInstruction`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScheduleInstruction {
    /// `create:destination`: go to a station whose name matches the glob.
    Destination { text: String },
    /// `create:package_delivery`: go to a station whose port takes a carried package.
    PackageDelivery,
    /// `create:package_retrieval`: go to a station with a package waiting for the address glob.
    PackageRetrieval { text: String },
    /// `create:rename`: set the schedule's title.
    Rename { text: String },
    /// `create:throttle`: `Value` percent (default 100).
    Throttle { value: i32 },
}

impl ScheduleInstruction {
    pub fn id(&self) -> &'static str {
        match self {
            ScheduleInstruction::Destination { .. } => "destination",
            ScheduleInstruction::PackageDelivery => "package_delivery",
            ScheduleInstruction::PackageRetrieval { .. } => "package_retrieval",
            ScheduleInstruction::Rename { .. } => "rename",
            ScheduleInstruction::Throttle { .. } => "throttle",
        }
    }

    /// The instruction the editor creates for an id, with its defaults. An unknown id gives a
    /// `DestinationInstruction` (as `fromTag` does).
    pub fn from_id(id: &str) -> ScheduleInstruction {
        match id.strip_prefix("create:").unwrap_or(id) {
            "package_delivery" => ScheduleInstruction::PackageDelivery,
            "package_retrieval" => ScheduleInstruction::PackageRetrieval { text: String::new() },
            "rename" => ScheduleInstruction::Rename { text: String::new() },
            "throttle" => ScheduleInstruction::Throttle { value: 100 },
            _ => ScheduleInstruction::Destination { text: String::new() },
        }
    }

    /// `supportsConditions()`: renaming and throttling are over at once.
    pub fn supports_conditions(&self) -> bool {
        !matches!(self, ScheduleInstruction::Rename { .. } | ScheduleInstruction::Throttle { .. })
    }

    /// `ChangeThrottleInstruction.getThrottle()`.
    pub fn throttle(&self) -> Option<f32> {
        match self {
            ScheduleInstruction::Throttle { value } => Some(*value as f32 / 100.0),
            _ => None,
        }
    }

    /// `start(runtime, level)`: begins the entry. A path is returned for the runtime to navigate; the
    /// other instructions act at once (and say `None`).
    pub fn start(&self, rt: &mut ScheduleRuntime, train: &mut dyn ScheduleTrain) -> Option<DiscoveredPath> {
        match self {
            ScheduleInstruction::Destination { text } => {
                if !train.has_forward_conductor() && !train.has_backward_conductor() {
                    train.report(TrainStatusEvent::MissingConductor);
                    rt.start_cooldown();
                    return None;
                }
                let glob = Glob::new(text);
                let mut any_match = false;
                let mut valid: Vec<Uuid> = Vec::new();
                for station in train.stations() {
                    if !glob.matches(&station.name) {
                        continue;
                    }
                    any_match = true;
                    valid.push(station.id);
                }
                let best = train.find_path_to(&valid);
                if best.is_none() {
                    if any_match {
                        train.report(TrainStatusEvent::FailedNavigation);
                    } else {
                        train.report(TrainStatusEvent::FailedNavigationNoTarget(text.clone()));
                    }
                    rt.start_cooldown();
                    return None;
                }
                best
            }
            ScheduleInstruction::PackageDelivery => {
                if !train.has_forward_conductor() && !train.has_backward_conductor() {
                    train.report(TrainStatusEvent::MissingConductor);
                    rt.start_cooldown();
                    return None;
                }
                let mut any_match = false;
                let mut first_package: Option<String> = None;
                let mut valid: Vec<Uuid> = Vec::new();
                let stations = train.stations();
                for address in train.carried_package_addresses() {
                    if first_package.is_none() {
                        first_package = Some(address.clone());
                    }
                    for station in &stations {
                        if station.ports.iter().any(|p| match_address(&address, &p.address)) {
                            any_match = true;
                            valid.push(station.id);
                        }
                    }
                }
                if valid.is_empty() {
                    if let Some(first) = first_package {
                        train.report(TrainStatusEvent::FailedPackageNoTarget(first));
                        rt.start_cooldown();
                    } else {
                        rt.state = State::PreTransit;
                        rt.current_entry += 1;
                    }
                    return None;
                }
                let best = train.find_path_to(&valid);
                if best.is_none() {
                    if any_match {
                        train.report(TrainStatusEvent::FailedNavigation);
                    }
                    rt.start_cooldown();
                    return None;
                }
                best
            }
            ScheduleInstruction::PackageRetrieval { text } => {
                // `getFilterForRegex()`: a blank filter is `*`.
                let glob = if is_blank(text) { Glob::new("*") } else { Glob::new(text) };
                let mut any_match = false;
                let mut valid: Vec<Uuid> = Vec::new();
                if !train.has_forward_conductor() && !train.has_backward_conductor() {
                    train.report(TrainStatusEvent::MissingConductor);
                    rt.start_cooldown();
                    return None;
                }
                for station in train.stations() {
                    if !station.dimension_loaded {
                        continue;
                    }
                    for port in &station.ports {
                        for address in &port.waiting_packages {
                            // Packages meant for this very port are not fetched.
                            if match_address(address, &port.address) {
                                continue;
                            }
                            if !glob.matches(address) {
                                continue;
                            }
                            any_match = true;
                            valid.push(station.id);
                        }
                    }
                }
                if valid.is_empty() {
                    rt.start_cooldown();
                    rt.state = State::PreTransit;
                    rt.current_entry += 1;
                    return None;
                }
                let best = train.find_path_to(&valid);
                if best.is_none() {
                    if any_match {
                        train.report(TrainStatusEvent::FailedNavigation);
                    }
                    rt.start_cooldown();
                    return None;
                }
                best
            }
            ScheduleInstruction::Rename { text } => {
                rt.current_title = text.clone();
                rt.state = State::PreTransit;
                rt.current_entry += 1;
                None
            }
            ScheduleInstruction::Throttle { value } => {
                train.set_throttle(*value as f32 / 100.0);
                rt.state = State::PreTransit;
                rt.current_entry += 1;
                None
            }
        }
    }
}

/// A station of the train's graph as an instruction sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StationInfo {
    pub id: Uuid,
    pub name: String,
    /// `server.getLevel(blockEntityDimension) != null`.
    pub dimension_loaded: bool,
    pub ports: Vec<PortInfo>,
}

/// A package port connected to a station.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortInfo {
    pub address: String,
    /// The addresses of the packages in the port's inventory (or offline buffer), slot by slot.
    pub waiting_packages: Vec<String>,
}

/// What `train.status.*` is told.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrainStatusEvent {
    MissingConductor,
    FailedNavigation,
    FailedNavigationNoTarget(String),
    FailedPackageNoTarget(String),
    SuccessfulNavigation,
    NewSchedule,
}

/// What the runtime and the instructions use of `Train`.
pub trait ScheduleTrain {
    fn derailed(&self) -> bool;
    fn has_forward_conductor(&self) -> bool;
    fn has_backward_conductor(&self) -> bool;
    /// `navigation.destination != null`.
    fn navigating(&self) -> bool;
    /// `getCurrentStation()`'s id.
    fn current_station(&self) -> Option<Uuid>;
    /// `graph.getPoints(STATION)` (none without a graph).
    fn stations(&self) -> Vec<StationInfo>;
    /// The address of every package in the carriages' inventories, carriage by carriage, slot by slot.
    fn carried_package_addresses(&self) -> Vec<String>;
    /// `navigation.findPathTo(destinations, Double.MAX_VALUE)`.
    fn find_path_to(&mut self, destinations: &[Uuid]) -> Option<DiscoveredPath>;
    /// `navigation.startNavigation(path)`: the cost, or [`TBD`] (-1) when it could not start.
    fn start_navigation(&mut self, path: DiscoveredPath) -> f64;
    /// `navigation.cancelNavigation()`.
    fn cancel_navigation(&mut self);
    fn report(&mut self, event: TrainStatusEvent);
    fn set_throttle(&mut self, throttle: f32);
    /// Every carriage's `storage.resetIdleCargoTracker()`.
    fn reset_idle_cargo_trackers(&mut self);
    /// Every carriage's `storage.tickIdleCargoTracker()`.
    fn tick_idle_cargo_trackers(&mut self);
    // For predictions.
    fn current_station_name(&self) -> Option<String>;
    fn destination_name(&self) -> Option<String>;
    fn throttle(&self) -> f64;
    fn max_speed(&self) -> f64;
    fn max_turn_speed(&self) -> f64;
    fn distance_to_destination(&self) -> f64;
    fn distance_started_at(&self) -> f64;
}

// --- schedule --------------------------------------------------------------------------------

/// `ScheduleEntry`: an instruction and its wait conditions, as columns (every column is a list of
/// conditions that must all be met in turn; the first column to finish ends the wait).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduleEntry {
    pub instruction: ScheduleInstruction,
    pub conditions: Vec<Vec<WaitCondition>>,
}

impl ScheduleEntry {
    pub fn new(instruction: ScheduleInstruction, conditions: Vec<Vec<WaitCondition>>) -> ScheduleEntry {
        ScheduleEntry { instruction, conditions }
    }
}

/// `Schedule`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Schedule {
    pub entries: Vec<ScheduleEntry>,
    pub cyclic: bool,
    pub saved_progress: i32,
}

impl Default for Schedule {
    /// `new Schedule()`: no entries, cyclic.
    fn default() -> Self {
        Schedule { entries: Vec::new(), cyclic: true, saved_progress: 0 }
    }
}

// --- runtime ---------------------------------------------------------------------------------

/// `ScheduleRuntime.State`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    PreTransit,
    InTransit,
    PostTransit,
}

/// `TrainDeparturePrediction` without the train: when (ticks) the train gets to or leaves a
/// destination, under what title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeparturePrediction {
    pub ticks: i32,
    /// The text shown for the line (the title, or the next destination's name without `*`).
    pub text: String,
    pub destination: String,
}

/// `Mth.clamp(int, min, max)`.
fn clamp_i(v: i32, min: i32, max: i32) -> i32 {
    if v < min {
        min
    } else if v > max {
        max
    } else {
        v
    }
}

/// `ScheduleRuntime`.
#[derive(Clone, Debug, PartialEq)]
pub struct ScheduleRuntime {
    pub schedule: Option<Schedule>,
    pub is_auto_schedule: bool,
    pub paused: bool,
    pub completed: bool,
    /// May be past the last entry (the end of the schedule is noticed on the next tick), or -1.
    pub current_entry: i32,
    pub state: State,
    pub condition_progress: Vec<i32>,
    pub condition_context: Vec<ConditionContext>,
    pub current_title: String,
    pub ticks_in_transit: i32,
    pub prediction_ticks: Vec<i32>,
    pub display_link_update_requested: bool,
    cooldown: i32,
}

impl Default for ScheduleRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl ScheduleRuntime {
    /// `new ScheduleRuntime(train)`: `reset()`.
    pub fn new() -> ScheduleRuntime {
        ScheduleRuntime {
            schedule: None,
            is_auto_schedule: false,
            paused: true,
            completed: false,
            current_entry: 0,
            state: State::PreTransit,
            condition_progress: Vec::new(),
            condition_context: Vec::new(),
            current_title: String::new(),
            ticks_in_transit: 0,
            prediction_ticks: Vec::new(),
            display_link_update_requested: false,
            cooldown: 0,
        }
    }

    /// `reset()`. (`ticksInTransit`, `displayLinkUpdateRequested` and the cooldown are not touched.)
    fn reset(&mut self) {
        self.paused = true;
        self.completed = false;
        self.is_auto_schedule = false;
        self.current_entry = 0;
        self.current_title = String::new();
        self.schedule = None;
        self.state = State::PreTransit;
        self.condition_progress = Vec::new();
        self.condition_context = Vec::new();
        self.prediction_ticks = Vec::new();
    }

    pub fn start_cooldown(&mut self) {
        self.cooldown = INTERVAL;
    }

    pub fn cooldown(&self) -> i32 {
        self.cooldown
    }

    /// `getSchedule()`.
    pub fn schedule(&self) -> Option<&Schedule> {
        self.schedule.as_ref()
    }

    /// `setSchedule(schedule, auto)`: the train starts at the saved progress, unpaused.
    pub fn set_schedule(&mut self, schedule: Schedule, auto: bool, train: &mut dyn ScheduleTrain) {
        self.reset();
        self.current_entry = clamp_i(schedule.saved_progress, 0, schedule.entries.len() as i32 - 1);
        self.paused = false;
        self.is_auto_schedule = auto;
        train.report(TrainStatusEvent::NewSchedule);
        self.prediction_ticks = schedule.entries.iter().map(|_| TBD).collect();
        self.schedule = Some(schedule);
        self.display_link_update_requested = true;
    }

    /// `discardSchedule()`.
    pub fn discard_schedule(&mut self, train: &mut dyn ScheduleTrain) {
        train.cancel_navigation();
        self.reset();
    }

    /// `returnSchedule()`: the schedule to hand back (with the progress saved), or `None` for an
    /// automatic one or none; either way the runtime is discarded.
    pub fn return_schedule(&mut self, train: &mut dyn ScheduleTrain) -> Option<Schedule> {
        let mut schedule = self.schedule.clone()?;
        schedule.saved_progress = self.current_entry;
        let result = if self.is_auto_schedule { None } else { Some(schedule) };
        self.discard_schedule(train);
        result
    }

    /// `destinationReached()`: the train is at its destination; the wait conditions start.
    pub fn destination_reached(&mut self, train: &mut dyn ScheduleTrain) {
        if self.state != State::InTransit {
            return;
        }
        self.state = State::PostTransit;
        self.condition_progress.clear();
        self.condition_context.clear();
        self.display_link_update_requested = true;
        train.reset_idle_cargo_trackers();

        if self.ticks_in_transit > 0 {
            if let Some(current) = self.prediction_ticks.get(self.current_entry.max(0) as usize).copied() {
                if current > 0 {
                    self.ticks_in_transit = (current + self.ticks_in_transit) / 2;
                }
                self.prediction_ticks[self.current_entry as usize] = self.ticks_in_transit;
            }
        }

        let Some(schedule) = &self.schedule else { return };
        let Some(entry) = usize::try_from(self.current_entry).ok().and_then(|i| schedule.entries.get(i)) else { return };
        for _ in 0..entry.conditions.len() {
            self.condition_progress.push(0);
            self.condition_context.push(ConditionContext::default());
        }
    }

    /// `transitInterrupted()`: the navigation was cancelled on the way.
    pub fn transit_interrupted(&mut self) {
        if self.schedule.is_none() || self.state != State::InTransit {
            return;
        }
        self.state = State::PreTransit;
        self.cooldown = 0;
    }

    /// `checkEndOfScheduleReached()`: past the last entry, back to the first; a schedule that is not
    /// cyclic is done and pauses.
    fn check_end_of_schedule_reached(&mut self) -> bool {
        let Some(schedule) = &self.schedule else { return false };
        if self.current_entry < schedule.entries.len() as i32 {
            return false;
        }
        self.current_entry = 0;
        if !schedule.cyclic {
            self.paused = true;
            self.completed = true;
        }
        true
    }

    /// `startCurrentInstruction(level)`.
    pub fn start_current_instruction(&mut self, train: &mut dyn ScheduleTrain) -> Option<DiscoveredPath> {
        if self.check_end_of_schedule_reached() {
            return None;
        }
        let instruction = {
            let schedule = self.schedule.as_ref()?;
            schedule.entries.get(usize::try_from(self.current_entry).ok()?)?.instruction.clone()
        };
        instruction.start(self, train)
    }

    /// `tick(level)`.
    pub fn tick(&mut self, train: &mut dyn ScheduleTrain, world: &dyn ConditionWorld) {
        if self.schedule.is_none() || self.paused || train.derailed() {
            return;
        }
        if train.navigating() {
            self.ticks_in_transit += 1;
            return;
        }

        if self.check_end_of_schedule_reached() {
            return;
        }
        let c = self.cooldown;
        self.cooldown -= 1;
        if c > 0 {
            return;
        }
        if self.state == State::InTransit {
            return;
        }
        if self.state == State::PostTransit {
            self.tick_conditions(train, world);
            return;
        }

        let Some(next_path) = self.start_current_instruction(train) else { return };

        train.report(TrainStatusEvent::SuccessfulNavigation);
        if train.current_station() == Some(next_path.destination) {
            self.state = State::InTransit;
            self.destination_reached(train);
            return;
        }
        if train.start_navigation(next_path) != f64::from(TBD) {
            self.state = State::InTransit;
            self.ticks_in_transit = 0;
        }
    }

    /// `tickConditions(level)`: ticks the current condition of every column; the first column to
    /// run out of conditions ends the wait.
    pub fn tick_conditions(&mut self, train: &mut dyn ScheduleTrain, world: &dyn ConditionWorld) {
        let Some(entry_index) = usize::try_from(self.current_entry).ok() else { return };
        let Some(schedule) = &mut self.schedule else { return };
        let Some(entry) = schedule.entries.get_mut(entry_index) else { return };

        if !entry.instruction.supports_conditions() {
            self.state = State::PreTransit;
            self.current_entry += 1;
            return;
        }

        for i in 0..entry.conditions.len() {
            let (Some(progress), Some(context)) =
                (self.condition_progress.get(i).copied(), self.condition_context.get(i).cloned())
            else {
                return;
            };
            let list = &mut entry.conditions[i];

            if progress >= list.len() as i32 {
                self.state = State::PreTransit;
                self.current_entry += 1;
                return;
            }

            let mut tag = context;
            let prev_version = tag.status_version;
            let done = list[progress.max(0) as usize].tick_completion(world, &mut tag);

            if done {
                self.condition_context[i] = ConditionContext::default();
                self.condition_progress[i] = progress + 1;
                self.display_link_update_requested |= i == 0;
            } else {
                self.condition_context[i] = tag.clone();
            }

            self.display_link_update_requested |= i == 0 && prev_version != tag.status_version;
        }

        train.tick_idle_cargo_trackers();
    }

    // --- predictions ---

    /// `estimateStayDuration(index)`: the ticks the first column made only of delays adds up to, or
    /// [`INVALID`] when no column is.
    fn estimate_stay_duration(&self, mut index: i32) -> i32 {
        let Some(schedule) = &self.schedule else { return INVALID };
        if index >= schedule.entries.len() as i32 {
            if !schedule.cyclic {
                return INVALID;
            }
            index = 0;
        }
        let Some(entry) = usize::try_from(index).ok().and_then(|i| schedule.entries.get(i)) else { return INVALID };
        'columns: for list in &entry.conditions {
            let mut total = 0_i32;
            for condition in list {
                let Some(ticks) = condition.delay_ticks() else { continue 'columns };
                total = total.wrapping_add(ticks);
            }
            return total;
        }
        INVALID
    }

    /// `createPrediction(index, destination, currentTitle, time)`.
    fn create_prediction(&self, mut index: i32, destination: &str, current_title: &str, time: i32) -> Option<DeparturePrediction> {
        if time == INVALID {
            return None;
        }
        let schedule = self.schedule.as_ref()?;
        let size = schedule.entries.len() as i32;
        if index >= size {
            if !schedule.cyclic {
                return Some(DeparturePrediction { ticks: time, text: " ".to_string(), destination: destination.to_string() });
            }
            index %= size;
        }

        let mut text = current_title.to_string();
        if is_blank(&text) {
            for i in 1..size {
                let j = (index + i) % size;
                if let ScheduleInstruction::Destination { text: filter } = &schedule.entries[j as usize].instruction {
                    text = filter.replace('*', "").trim().to_string();
                    break;
                }
            }
        }
        Some(DeparturePrediction { ticks: time, text, destination: destination.to_string() })
    }

    /// `predictForEntry(index, currentTitle, accumulatedTime, predictions)`.
    fn predict_for_entry(&self, index: i32, current_title: &str, mut accumulated: i32, out: &mut Vec<Option<DeparturePrediction>>) -> i32 {
        let Some(schedule) = &self.schedule else { return accumulated };
        let Some(entry) = usize::try_from(index).ok().and_then(|i| schedule.entries.get(i)) else { return accumulated };
        let ScheduleInstruction::Destination { text: filter } = &entry.instruction else { return accumulated };
        if self.prediction_ticks.len() as i32 <= self.current_entry {
            return accumulated;
        }

        let departure_time = self.estimate_stay_duration(index);

        if accumulated < 0 {
            out.push(self.create_prediction(index, filter, current_title, accumulated));
            return accumulated.min(departure_time);
        }

        let predicted_time = self.prediction_ticks.get(index as usize).copied().unwrap_or(TBD);
        accumulated += predicted_time;

        if predicted_time == TBD {
            accumulated = TBD;
        }

        out.push(self.create_prediction(index, filter, current_title, accumulated));

        if accumulated != TBD {
            accumulated += departure_time;
        }

        if departure_time == INVALID {
            accumulated = INVALID;
        }

        accumulated
    }

    /// `submitPredictions()`: when the train will be at the stations ahead.
    pub fn submit_predictions(&self, train: &dyn ScheduleTrain) -> Vec<DeparturePrediction> {
        let Some(schedule) = &self.schedule else { return Vec::new() };
        let mut predictions: Vec<Option<DeparturePrediction>> = Vec::new();
        let entry_count = schedule.entries.len() as i32;
        let mut accumulated_time = 0;
        let current = self.current_entry;

        // Current.
        if self.state == State::PostTransit || current >= entry_count {
            if let Some(name) = train.current_station_name() {
                predictions.push(self.create_prediction(current, &name, &self.current_title, 0));
            }
            let departure_time = self.estimate_stay_duration(current);
            if departure_time == INVALID {
                accumulated_time = INVALID;
            } else {
                accumulated_time += departure_time;
            }
        } else if let Some(destination) = train.destination_name() {
            let speed = f64::min(train.throttle() * train.max_speed(), (train.max_speed() + train.max_turn_speed()) / 2.0);
            let mut time_remaining = ((train.distance_to_destination() / speed) as i32).wrapping_mul(2);

            if self.prediction_ticks.len() as i32 > current && train.distance_started_at() != 0.0 {
                let mut predicted_time = self.prediction_ticks[current.max(0) as usize] as f32;
                if predicted_time > 0.0 {
                    let ratio = (train.distance_to_destination() / train.distance_started_at()).clamp(0.0, 1.0);
                    predicted_time = (f64::from(predicted_time) * ratio) as f32;
                    time_remaining = (time_remaining + predicted_time as i32) / 2;
                }
            }

            accumulated_time += time_remaining;
            predictions.push(self.create_prediction(current, &destination, &self.current_title, accumulated_time));

            let departure_time = self.estimate_stay_duration(current);
            if departure_time != INVALID {
                accumulated_time += departure_time;
            } else {
                accumulated_time = INVALID;
            }
        } else {
            self.predict_for_entry(current, &self.current_title, accumulated_time, &mut predictions);
        }

        // Upcoming.
        let mut current_title = self.current_title.clone();
        for i in 1..entry_count {
            let index = (i + current) % entry_count;
            if index == 0 && !schedule.cyclic {
                break;
            }

            if let ScheduleInstruction::Rename { text } = &schedule.entries[index as usize].instruction {
                current_title = text.clone();
                continue;
            }

            accumulated_time = self.predict_for_entry(index, &current_title, accumulated_time, &mut predictions);
        }

        predictions.into_iter().flatten().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// A world with settable readings.
    #[derive(Default)]
    struct World {
        day_time: i64,
        cargo_version: i32,
        idle: i32,
        items: Vec<(i32, i32)>,
        fluids: Vec<i32>,
        power_version: i32,
        power: bool,
        players: i32,
        powered: bool,
        unloaded: bool,
        lookups: Cell<i32>,
    }

    impl ConditionWorld for World {
        fn day_time(&self) -> i64 {
            self.day_time
        }
        fn cargo_version(&self) -> i32 {
            self.cargo_version
        }
        fn min_ticks_since_last_exchange(&self) -> i32 {
            self.idle
        }
        fn matching_item_slots(&self, _: Option<&str>) -> Vec<(i32, i32)> {
            self.lookups.set(self.lookups.get() + 1);
            self.items.clone()
        }
        fn matching_fluid_amounts(&self, _: Option<&str>) -> Vec<i32> {
            self.fluids.clone()
        }
        fn redstone_power_version(&self) -> i32 {
            self.power_version
        }
        fn has_any_loaded_power(&self, _: &[String; 2]) -> bool {
            self.power
        }
        fn count_player_passengers(&self) -> i32 {
            self.players
        }
        fn station_powered(&self) -> bool {
            self.powered
        }
        fn station_unloaded(&self) -> bool {
            self.unloaded
        }
    }

    /// A train that records what it is told.
    #[derive(Default)]
    struct Train {
        derailed: bool,
        forward: bool,
        backward: bool,
        navigating: bool,
        station: Option<Uuid>,
        stations: Vec<StationInfo>,
        packages: Vec<String>,
        path: Option<DiscoveredPath>,
        asked: Vec<Vec<Uuid>>,
        started: Vec<DiscoveredPath>,
        start_result: f64,
        events: Vec<TrainStatusEvent>,
        throttle_set: Option<f32>,
        cancelled: u32,
        idle_reset: u32,
        idle_ticked: u32,
        current_name: Option<String>,
        destination_name: Option<String>,
        to_destination: f64,
        started_at: f64,
    }

    impl ScheduleTrain for Train {
        fn derailed(&self) -> bool {
            self.derailed
        }
        fn has_forward_conductor(&self) -> bool {
            self.forward
        }
        fn has_backward_conductor(&self) -> bool {
            self.backward
        }
        fn navigating(&self) -> bool {
            self.navigating
        }
        fn current_station(&self) -> Option<Uuid> {
            self.station
        }
        fn stations(&self) -> Vec<StationInfo> {
            self.stations.clone()
        }
        fn carried_package_addresses(&self) -> Vec<String> {
            self.packages.clone()
        }
        fn find_path_to(&mut self, destinations: &[Uuid]) -> Option<DiscoveredPath> {
            self.asked.push(destinations.to_vec());
            self.path.clone()
        }
        fn start_navigation(&mut self, path: DiscoveredPath) -> f64 {
            self.started.push(path);
            self.start_result
        }
        fn cancel_navigation(&mut self) {
            self.cancelled += 1;
        }
        fn report(&mut self, event: TrainStatusEvent) {
            self.events.push(event);
        }
        fn set_throttle(&mut self, throttle: f32) {
            self.throttle_set = Some(throttle);
        }
        fn reset_idle_cargo_trackers(&mut self) {
            self.idle_reset += 1;
        }
        fn tick_idle_cargo_trackers(&mut self) {
            self.idle_ticked += 1;
        }
        fn current_station_name(&self) -> Option<String> {
            self.current_name.clone()
        }
        fn destination_name(&self) -> Option<String> {
            self.destination_name.clone()
        }
        fn throttle(&self) -> f64 {
            1.0
        }
        fn max_speed(&self) -> f64 {
            4.0
        }
        fn max_turn_speed(&self) -> f64 {
            2.0
        }
        fn distance_to_destination(&self) -> f64 {
            self.to_destination
        }
        fn distance_started_at(&self) -> f64 {
            self.started_at
        }
    }

    fn driver() -> Train {
        Train { forward: true, start_result: 5.0, ..Default::default() }
    }

    fn station(id: Uuid, name: &str) -> StationInfo {
        StationInfo { id, name: name.to_string(), dimension_loaded: true, ports: Vec::new() }
    }

    fn path_to(dest: Uuid) -> DiscoveredPath {
        DiscoveredPath { path: vec![], destination: dest, distance: 10.0, cost: 10.0 }
    }

    fn dest(text: &str) -> ScheduleInstruction {
        ScheduleInstruction::Destination { text: text.to_string() }
    }

    fn delay(seconds: i32) -> WaitCondition {
        WaitCondition::Delay(TimedWait { value: seconds, unit: 1 })
    }

    #[test]
    fn defaults_and_ids() {
        assert_eq!(TimedWait::default().total_wait_ticks(), 100);
        assert_eq!(TimeOfDay::default(), TimeOfDay { hour: 8, minute: 0, rotation: 5 });
        assert_eq!(Threshold::default().threshold, "10");
        assert_eq!(ScheduleInstruction::from_id("create:throttle"), ScheduleInstruction::Throttle { value: 100 });
        // An unknown instruction id reads as a destination; an unknown condition as none.
        assert_eq!(ScheduleInstruction::from_id("create:nope"), dest(""));
        assert!(WaitCondition::from_id("create:nope").is_none());
        for id in CONDITION_TYPES {
            assert_eq!(WaitCondition::from_id(&format!("create:{id}")).unwrap().id(), id);
        }
        for id in INSTRUCTION_TYPES {
            assert_eq!(ScheduleInstruction::from_id(id).id(), id);
        }
        assert!(dest("x").supports_conditions());
        assert!(ScheduleInstruction::PackageDelivery.supports_conditions());
        assert!(!ScheduleInstruction::Rename { text: String::new() }.supports_conditions());
        assert!(!ScheduleInstruction::Throttle { value: 50 }.supports_conditions());
        assert_eq!(ScheduleInstruction::Throttle { value: 50 }.throttle(), Some(0.5));
        let s = Schedule::default();
        assert!(s.cyclic && s.entries.is_empty() && s.saved_progress == 0);
    }

    #[test]
    fn time_units() {
        assert_eq!(TimeUnit::Minutes.ticks_per(), 1200);
        assert_eq!(TimeUnit::from_index(4), TimeUnit::Seconds);
        assert_eq!(TimedWait { value: 2, unit: 2 }.total_wait_ticks(), 2400);
        assert_eq!(TimedWait { value: 7, unit: 0 }.total_wait_ticks(), 7);
    }

    #[test]
    fn a_delay_counts_up_and_asks_for_status_updates() {
        let world = World::default();
        let mut c = delay(5); // 100 ticks
        let mut ctx = ConditionContext::default();
        let mut versions = Vec::new();
        for t in 0..100 {
            assert!(!c.tick_completion(&world, &mut ctx), "tick {t}");
            versions.push(ctx.status_version);
        }
        assert_eq!(ctx.time, 100);
        // Under a minute left: every 100 ticks (at time 0 there are 100 left).
        assert_eq!(versions[0], 1);
        assert_eq!(versions[99], 1);
        assert!(c.tick_completion(&world, &mut ctx));
        assert_eq!(ctx.time, 100);

        // Over a minute left: once a minute.
        let mut c = WaitCondition::Delay(TimedWait { value: 3, unit: 2 });
        let mut ctx = ConditionContext::default();
        for _ in 0..1200 {
            c.tick_completion(&world, &mut ctx);
        }
        // Times 0 (3600 left) and 1200 left is hit at time 2400, so only the first so far.
        assert_eq!(ctx.status_version, 1);
    }

    #[test]
    fn waiting_status_numbers() {
        let t = TimedWait { value: 10, unit: 1 }; // 200 ticks
        // Seconds, rounded up to a multiple of 5 (ceil of hundreds of ticks, times 5).
        assert_eq!(t.waiting_status(0), (10, false));
        assert_eq!(t.waiting_status(99), (10, false));
        assert_eq!(t.waiting_status(100), (5, false));
        assert_eq!(t.waiting_status(199), (5, false));
        // From a minute on it counts whole minutes, rounded down.
        let t = TimedWait { value: 2, unit: 2 };
        assert_eq!(t.waiting_status(0), (2, true));
        assert_eq!(t.waiting_status(1), (1, true));
        assert_eq!(t.waiting_status(1200), (1, true));
        assert_eq!(t.waiting_status(1201), (60, false));
    }

    #[test]
    fn idle_cargo_waits_for_the_quiet_time() {
        let mut world = World::default();
        let mut c = WaitCondition::Idle(TimedWait { value: 5, unit: 1 });
        let mut ctx = ConditionContext::default();
        world.idle = 100;
        assert!(!c.tick_completion(&world, &mut ctx));
        assert_eq!(ctx.time, 100);
        world.idle = 101;
        assert!(c.tick_completion(&world, &mut ctx));
        // No carriages: the maximum, always done.
        world.idle = i32::MAX;
        assert!(c.tick_completion(&world, &mut ctx));
    }

    #[test]
    fn time_of_day() {
        // 8:00 every 2000 ticks: (8 + 18) % 24 = 2 -> 2000 % 2000 = 0.
        let t = TimeOfDay::default();
        assert_eq!(t.rotation_ticks(), 2000);
        assert_eq!(t.target_ticks(), 0);
        let mut c = WaitCondition::TimeOfDay(t.clone());
        let mut world = World::default();
        let mut ctx = ConditionContext::default();
        // Within 40 ticks after the moment (day time modulo the rotation).
        for (dt, expected) in [(0, true), (40, true), (41, false), (1999, false), (2000, true), (4020, true), (4041, false)] {
            world.day_time = dt;
            assert_eq!(c.tick_completion(&world, &mut ctx), expected, "day time {dt}");
        }
        // 14:30 every 24 hours: 14 + 18 = 32 % 24 = 8 -> 8000, plus ceil(30/60*1000) = 500.
        let t = TimeOfDay { hour: 14, minute: 30, rotation: 0 };
        assert_eq!(t.target_ticks(), 8500);
        // Minutes round up: 1 minute is ceil(16.67) = 17.
        assert_eq!(TimeOfDay { hour: 6, minute: 1, rotation: 0 }.target_ticks(), 17);
        assert_eq!(TimeOfDay { rotation: 9, ..t.clone() }.rotation_ticks(), 250);
        assert_eq!(TimeOfDay { rotation: 1, ..t.clone() }.rotation_ticks(), 12000);
        assert_eq!(TimeOfDay { rotation: 99, ..t }.rotation_ticks(), 24000);
        // The next departure: 8:00 is at tick 2000 of the day (the day starts at 6:00).
        let eight = TimeOfDay { hour: 8, minute: 0, rotation: 0 };
        assert_eq!(eight.target_ticks(), 2000);
        assert_eq!(eight.next_departure(100), (8, 0));
        assert_eq!(eight.next_departure(2500), (8, 0));
    }

    #[test]
    fn thresholds_look_every_20_ticks_and_only_when_cargo_changed() {
        let mut world = World { items: vec![(64, 64), (10, 64)], cargo_version: 1, ..Default::default() };
        let mut c = WaitCondition::ItemThreshold(Threshold { threshold: "70".to_string(), ..Default::default() });
        let mut ctx = ConditionContext::default();
        // First tick (time 0): a look: 74 > 70.
        assert!(c.tick_completion(&world, &mut ctx));
        assert_eq!(ctx.current_display, Some(74));
        assert_eq!(world.lookups.get(), 1);

        // Fewer than needed: not met, and no look until 20 ticks on, and then only if cargo changed.
        world.items = vec![(10, 64)];
        let mut ctx = ConditionContext::default();
        assert!(!c.tick_completion(&world, &mut ctx));
        assert_eq!(ctx.time, 1);
        for _ in 0..19 {
            assert!(!c.tick_completion(&world, &mut ctx));
        }
        assert_eq!(world.lookups.get(), 2); // time 0 looked, 1..19 did not
        // Time 20: the cargo version is the same as at the look: no new look.
        c.tick_completion(&world, &mut ctx);
        assert_eq!(world.lookups.get(), 2);
        // Cargo changes (and now has enough): the next look at time 20 counts. After a look the timer
        // restarts from 0 (`time = 0` then `+ 1`).
        assert_eq!(ctx.time, 1);
        world.cargo_version = 2;
        world.items = vec![(64, 64), (64, 64)];
        for _ in 0..19 {
            assert!(!c.tick_completion(&world, &mut ctx));
        }
        assert!(c.tick_completion(&world, &mut ctx));
        assert_eq!(ctx.current_display, Some(128));
    }

    #[test]
    fn item_thresholds_count_stacks_and_use_the_operator() {
        let world = World { items: vec![(64, 64), (63, 64), (16, 16)], cargo_version: 5, ..Default::default() };
        let mut ctx = ConditionContext::default();
        // Two full stacks.
        let mut c = WaitCondition::ItemThreshold(Threshold { threshold: "2".into(), operator: 2, measure: 1, filter: None });
        assert!(c.tick_completion(&world, &mut ctx));
        assert_eq!(ctx.current_display, Some(2));
        let mut ctx = ConditionContext::default();
        let mut c = WaitCondition::ItemThreshold(Threshold { threshold: "143".into(), operator: 1, measure: 0, filter: None });
        // 143 items total, "less than 143" is false.
        assert!(!c.tick_completion(&world, &mut ctx));
        let mut ctx = ConditionContext::default();
        let mut c = WaitCondition::ItemThreshold(Threshold { threshold: "144".into(), operator: 1, measure: 0, filter: None });
        assert!(c.tick_completion(&world, &mut ctx));
        // A threshold that is not a number reads as 0 and is rewritten.
        let mut t = Threshold { threshold: "lots".into(), ..Default::default() };
        assert_eq!(t.threshold(), 0);
        assert_eq!(t.threshold, "0");
        assert_eq!(Threshold { threshold: "+7".into(), ..Default::default() }.threshold(), 7);
    }

    #[test]
    fn fluid_thresholds_compare_millibuckets() {
        let world = World { fluids: vec![1500, 600], cargo_version: 1, ..Default::default() };
        let mut c = WaitCondition::FluidThreshold(Threshold { threshold: "2".into(), operator: 0, ..Default::default() });
        let mut ctx = ConditionContext::default();
        // 2100 mB > 2000.
        assert!(c.tick_completion(&world, &mut ctx));
        // The display is in whole buckets.
        assert_eq!(ctx.current_display, Some(2));
        let mut c = WaitCondition::FluidThreshold(Threshold { threshold: "3".into(), operator: 0, ..Default::default() });
        assert!(!c.tick_completion(&world, &mut ConditionContext::default()));
    }

    #[test]
    fn threshold_waiting_status() {
        let mut t = Threshold { threshold: "10".into(), operator: 0, ..Default::default() };
        assert_eq!(t.waiting_status(&ConditionContext::default()), None);
        let ctx = ConditionContext { current_display: Some(3), ..Default::default() };
        assert_eq!(t.waiting_status(&ctx), Some((3, 11)));
        t.operator = 1;
        assert_eq!(t.waiting_status(&ctx), Some((3, 9)));
        t.operator = 2;
        assert_eq!(t.waiting_status(&ctx), Some((3, 10)));
        t.threshold = "0".into();
        t.operator = 1;
        assert_eq!(t.waiting_status(&ctx), Some((3, 0)));
    }

    #[test]
    fn redstone_link_checks_when_the_power_version_moves() {
        let mut world = World { power_version: 1, power: true, ..Default::default() };
        let mut c = WaitCondition::RedstoneLink { frequency: [String::new(), String::new()], inverted: 0 };
        let mut ctx = ConditionContext::default();
        assert!(c.tick_completion(&world, &mut ctx));
        // Same version: nothing new to look at.
        assert!(!c.tick_completion(&world, &mut ctx));
        world.power_version = 2;
        world.power = false;
        assert!(!c.tick_completion(&world, &mut ctx));
        // Inverted waits for no power.
        let mut c = WaitCondition::RedstoneLink { frequency: [String::new(), String::new()], inverted: 1 };
        let mut ctx = ConditionContext::default();
        assert!(c.tick_completion(&world, &mut ctx));
    }

    #[test]
    fn player_count_exactly_or_above() {
        let mut world = World { players: 2, ..Default::default() };
        let mut ctx = ConditionContext::default();
        let mut exact = WaitCondition::PlayerCount { count: 2, exact: 0 };
        let mut above = WaitCondition::PlayerCount { count: 2, exact: 1 };
        assert!(exact.tick_completion(&world, &mut ctx));
        assert!(above.tick_completion(&world, &mut ctx));
        // The count changed from 0 to 2: the status was asked to update once.
        assert_eq!(ctx.status_version, 1);
        world.players = 3;
        assert!(!exact.tick_completion(&world, &mut ctx));
        assert!(above.tick_completion(&world, &mut ctx));
        assert_eq!(ctx.status_version, 2);
        world.players = 1;
        assert!(!above.tick_completion(&world, &mut ctx));
    }

    #[test]
    fn station_conditions_read_the_world() {
        let mut world = World::default();
        let mut ctx = ConditionContext::default();
        assert!(!WaitCondition::Powered.tick_completion(&world, &mut ctx));
        assert!(!WaitCondition::Unloaded.tick_completion(&world, &mut ctx));
        world.powered = true;
        world.unloaded = true;
        assert!(WaitCondition::Powered.tick_completion(&world, &mut ctx));
        assert!(WaitCondition::Unloaded.tick_completion(&world, &mut ctx));
    }

    #[test]
    fn destination_goes_to_every_station_the_glob_matches() {
        let mut train = driver();
        train.stations = vec![station(1, "Main Station"), station(2, "Mine A"), station(3, "Mine B")];
        train.path = Some(path_to(2));
        let mut rt = ScheduleRuntime::new();
        let p = dest("Mine *").start(&mut rt, &mut train);
        assert_eq!(p.unwrap().destination, 2);
        assert_eq!(train.asked, vec![vec![2, 3]]);
        assert_eq!(rt.cooldown(), 0);

        // Nothing matches: a failure naming the filter, and a cooldown.
        train.path = None;
        train.asked.clear();
        assert!(dest("Port").start(&mut rt, &mut train).is_none());
        assert_eq!(train.events, vec![TrainStatusEvent::FailedNavigationNoTarget("Port".to_string())]);
        assert_eq!(rt.cooldown(), INTERVAL);

        // A match with no way there.
        train.events.clear();
        assert!(dest("Main*").start(&mut rt, &mut train).is_none());
        assert_eq!(train.events, vec![TrainStatusEvent::FailedNavigation]);
    }

    #[test]
    fn a_destination_needs_a_conductor() {
        let mut train = Train::default();
        let mut rt = ScheduleRuntime::new();
        assert!(dest("*").start(&mut rt, &mut train).is_none());
        assert_eq!(train.events, vec![TrainStatusEvent::MissingConductor]);
        assert_eq!(rt.cooldown(), INTERVAL);
        assert!(train.asked.is_empty());
        // Either direction's conductor will do here.
        train.backward = true;
        train.events.clear();
        dest("*").start(&mut rt, &mut train);
        assert!(!train.asked.is_empty());
    }

    #[test]
    fn rename_and_throttle_act_at_once() {
        let mut train = driver();
        let mut rt = ScheduleRuntime::new();
        rt.state = State::PostTransit;
        assert!(ScheduleInstruction::Rename { text: "Line 1".into() }.start(&mut rt, &mut train).is_none());
        assert_eq!(rt.current_title, "Line 1");
        assert_eq!(rt.state, State::PreTransit);
        assert_eq!(rt.current_entry, 1);
        assert!(ScheduleInstruction::Throttle { value: 25 }.start(&mut rt, &mut train).is_none());
        assert_eq!(train.throttle_set, Some(0.25));
        assert_eq!(rt.current_entry, 2);
    }

    fn port(address: &str, waiting: &[&str]) -> PortInfo {
        PortInfo { address: address.to_string(), waiting_packages: waiting.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn package_delivery_goes_where_a_carried_package_is_addressed() {
        let mut train = driver();
        let mut a = station(1, "A");
        a.ports = vec![port("Farm", &[])];
        let mut b = station(2, "B");
        b.ports = vec![port("Mine", &[])];
        train.stations = vec![a, b];
        train.packages = vec!["Mine".to_string(), "Nowhere".to_string()];
        train.path = Some(path_to(2));
        let mut rt = ScheduleRuntime::new();
        assert_eq!(ScheduleInstruction::PackageDelivery.start(&mut rt, &mut train).unwrap().destination, 2);
        assert_eq!(train.asked, vec![vec![2]]);

        // Packages that fit no port: a failure naming the first.
        train.packages = vec!["Nowhere".to_string(), "Mine".to_string()];
        train.asked.clear();
        train.path = None;
        assert!(ScheduleInstruction::PackageDelivery.start(&mut rt, &mut train).is_none());
        // (The second package still matches `Mine`, so a path was asked for, and none was found.)
        assert_eq!(train.asked, vec![vec![2]]);
        assert_eq!(train.events, vec![TrainStatusEvent::FailedNavigation]);

        train.packages = vec!["Nowhere".to_string()];
        train.events.clear();
        assert!(ScheduleInstruction::PackageDelivery.start(&mut rt, &mut train).is_none());
        assert_eq!(train.events, vec![TrainStatusEvent::FailedPackageNoTarget("Nowhere".to_string())]);
        assert_eq!(rt.cooldown(), INTERVAL);

        // No packages at all: the entry is skipped.
        train.packages.clear();
        let mut rt = ScheduleRuntime::new();
        rt.state = State::PostTransit;
        assert!(ScheduleInstruction::PackageDelivery.start(&mut rt, &mut train).is_none());
        assert_eq!((rt.state, rt.current_entry), (State::PreTransit, 1));
    }

    #[test]
    fn package_retrieval_goes_where_packages_wait() {
        let mut train = driver();
        let mut a = station(1, "A");
        a.ports = vec![port("Farm", &["Mine", "Farm"])];
        let mut b = station(2, "B");
        b.ports = vec![port("Mine", &["Town"])];
        let mut c = station(3, "C");
        c.dimension_loaded = false;
        c.ports = vec![port("X", &["Town"])];
        train.stations = vec![a, b, c];
        train.path = Some(path_to(1));
        let mut rt = ScheduleRuntime::new();
        // A blank filter is `*`: A holds a package for Mine (the one for Farm is its own), B one for Town.
        let p = ScheduleInstruction::PackageRetrieval { text: String::new() }.start(&mut rt, &mut train);
        assert_eq!(p.unwrap().destination, 1);
        assert_eq!(train.asked, vec![vec![1, 2]]);
        // A filter.
        train.asked.clear();
        ScheduleInstruction::PackageRetrieval { text: "Tow*".into() }.start(&mut rt, &mut train);
        assert_eq!(train.asked, vec![vec![2]]);
        // Nothing matches: skipped, with the cooldown too.
        train.asked.clear();
        assert!(ScheduleInstruction::PackageRetrieval { text: "Nope".into() }.start(&mut rt, &mut train).is_none());
        assert_eq!((rt.state, rt.current_entry, rt.cooldown()), (State::PreTransit, 1, INTERVAL));
        assert!(train.asked.is_empty());
    }

    fn schedule_of(entries: Vec<ScheduleEntry>) -> Schedule {
        Schedule { entries, cyclic: true, saved_progress: 0 }
    }

    fn stop(name: &str, seconds: i32) -> ScheduleEntry {
        ScheduleEntry::new(dest(name), vec![vec![delay(seconds)]])
    }

    #[test]
    fn setting_a_schedule_starts_it() {
        let mut train = driver();
        let mut rt = ScheduleRuntime::new();
        assert!(rt.paused && rt.schedule().is_none());
        let mut s = schedule_of(vec![stop("A", 1), stop("B", 1), stop("C", 1)]);
        s.saved_progress = 7;
        rt.set_schedule(s, false, &mut train);
        assert!(!rt.paused);
        // The saved progress is clamped to the last entry.
        assert_eq!(rt.current_entry, 2);
        assert_eq!(rt.prediction_ticks, vec![TBD; 3]);
        assert!(rt.display_link_update_requested);
        assert_eq!(train.events, vec![TrainStatusEvent::NewSchedule]);
        // An empty schedule clamps to -1 (Mth.clamp does not mind min > max).
        rt.set_schedule(Schedule::default(), false, &mut train);
        assert_eq!(rt.current_entry, -1);
    }

    #[test]
    fn the_runtime_runs_a_schedule_through() {
        let mut train = driver();
        train.stations = vec![station(1, "A"), station(2, "B")];
        train.path = Some(path_to(1));
        let world = World::default();
        let mut rt = ScheduleRuntime::new();
        // Paused until given a schedule.
        rt.tick(&mut train, &world);
        assert!(train.asked.is_empty());
        rt.set_schedule(schedule_of(vec![stop("A", 1), stop("B", 1)]), false, &mut train);
        train.events.clear();

        // Tick 1: the instruction starts, the navigation begins.
        rt.tick(&mut train, &world);
        assert_eq!(train.events, vec![TrainStatusEvent::SuccessfulNavigation]);
        assert_eq!(train.started.len(), 1);
        assert_eq!(rt.state, State::InTransit);
        assert_eq!(rt.ticks_in_transit, 0);

        // Under way: transit ticks count.
        train.navigating = true;
        rt.tick(&mut train, &world);
        rt.tick(&mut train, &world);
        assert_eq!(rt.ticks_in_transit, 2);

        // Arrival.
        train.navigating = false;
        rt.destination_reached(&mut train);
        assert_eq!(rt.state, State::PostTransit);
        assert_eq!(rt.condition_progress, vec![0]);
        assert_eq!(train.idle_reset, 1);
        // The transit time is remembered.
        assert_eq!(rt.prediction_ticks[0], 2);

        // The 20 tick delay passes, and the entry moves on.
        for _ in 0..20 {
            rt.tick(&mut train, &world);
            assert_eq!(rt.state, State::PostTransit);
        }
        assert_eq!(train.idle_ticked, 20);
        rt.tick(&mut train, &world);
        assert_eq!(rt.condition_progress, vec![1]);
        rt.tick(&mut train, &world);
        assert_eq!((rt.state, rt.current_entry), (State::PreTransit, 1));
    }

    #[test]
    fn transit_times_are_averaged() {
        let mut train = driver();
        let mut rt = ScheduleRuntime::new();
        rt.set_schedule(schedule_of(vec![stop("A", 1)]), false, &mut train);
        rt.state = State::InTransit;
        rt.ticks_in_transit = 100;
        rt.prediction_ticks[0] = 300;
        rt.destination_reached(&mut train);
        assert_eq!(rt.prediction_ticks[0], 200);
        // Not in transit: nothing happens.
        rt.destination_reached(&mut train);
        assert_eq!(rt.prediction_ticks[0], 200);
    }

    #[test]
    fn already_at_the_station_it_arrives_at_once() {
        let mut train = driver();
        train.stations = vec![station(1, "A")];
        train.path = Some(path_to(1));
        train.station = Some(1);
        let world = World::default();
        let mut rt = ScheduleRuntime::new();
        rt.set_schedule(schedule_of(vec![stop("A", 1)]), false, &mut train);
        rt.tick(&mut train, &world);
        assert_eq!(rt.state, State::PostTransit);
        assert!(train.started.is_empty());
    }

    #[test]
    fn a_navigation_that_cannot_start_leaves_the_state() {
        let mut train = driver();
        train.stations = vec![station(1, "A")];
        train.path = Some(path_to(1));
        train.start_result = -1.0;
        let world = World::default();
        let mut rt = ScheduleRuntime::new();
        rt.set_schedule(schedule_of(vec![stop("A", 1)]), false, &mut train);
        rt.tick(&mut train, &world);
        assert_eq!(rt.state, State::PreTransit);
        assert_eq!(train.started.len(), 1);
    }

    #[test]
    fn a_failed_start_waits_out_the_cooldown() {
        let mut train = driver();
        train.stations = vec![station(1, "A")];
        train.path = None;
        let world = World::default();
        let mut rt = ScheduleRuntime::new();
        rt.set_schedule(schedule_of(vec![stop("A", 1)]), false, &mut train);
        rt.tick(&mut train, &world);
        assert_eq!(train.asked.len(), 1);
        assert_eq!(rt.cooldown(), INTERVAL);
        // 40 ticks of waiting (the counter runs 40 -> 0), then it asks again.
        for _ in 0..40 {
            rt.tick(&mut train, &world);
        }
        assert_eq!(train.asked.len(), 1);
        rt.tick(&mut train, &world);
        assert_eq!(train.asked.len(), 2);
    }

    #[test]
    fn interrupted_transit_goes_back_to_pre_transit() {
        let mut train = driver();
        let mut rt = ScheduleRuntime::new();
        rt.transit_interrupted(); // no schedule: nothing
        rt.set_schedule(schedule_of(vec![stop("A", 1)]), false, &mut train);
        rt.start_cooldown();
        rt.transit_interrupted(); // not in transit: nothing
        assert_eq!(rt.cooldown(), INTERVAL);
        rt.state = State::InTransit;
        rt.transit_interrupted();
        assert_eq!((rt.state, rt.cooldown()), (State::PreTransit, 0));
    }

    #[test]
    fn the_end_of_a_schedule_wraps_or_completes() {
        let mut train = driver();
        let world = World::default();
        let mut rt = ScheduleRuntime::new();
        rt.set_schedule(schedule_of(vec![stop("A", 1)]), false, &mut train);
        rt.current_entry = 1;
        rt.tick(&mut train, &world);
        assert_eq!(rt.current_entry, 0);
        assert!(!rt.paused && !rt.completed);

        let mut s = schedule_of(vec![stop("A", 1)]);
        s.cyclic = false;
        rt.set_schedule(s, false, &mut train);
        rt.current_entry = 1;
        rt.tick(&mut train, &world);
        assert!(rt.paused && rt.completed);
        assert_eq!(rt.current_entry, 0);
    }

    #[test]
    fn columns_of_conditions_race_and_each_runs_in_order() {
        let mut train = driver();
        let world = World { players: 1, ..Default::default() };
        let mut rt = ScheduleRuntime::new();
        // Column 0: a 3 tick delay (value 3, unit ticks) then a player count; column 1: a long delay.
        let col0 = vec![WaitCondition::Delay(TimedWait { value: 3, unit: 0 }), WaitCondition::PlayerCount { count: 1, exact: 0 }];
        let col1 = vec![WaitCondition::Delay(TimedWait { value: 9, unit: 2 })];
        rt.set_schedule(schedule_of(vec![ScheduleEntry::new(dest("A"), vec![col0, col1]), stop("B", 1)]), false, &mut train);
        rt.state = State::InTransit;
        rt.destination_reached(&mut train);
        assert_eq!(rt.condition_progress, vec![0, 0]);
        for _ in 0..3 {
            rt.tick_conditions(&mut train, &world);
        }
        assert_eq!(rt.condition_progress, vec![0, 0]);
        rt.tick_conditions(&mut train, &world); // first delay done
        assert_eq!(rt.condition_progress, vec![1, 0]);
        // The finished condition's context was reset; the other column kept ticking.
        assert_eq!(rt.condition_context[0], ConditionContext::default());
        assert_eq!(rt.condition_context[1].time, 4);
        rt.tick_conditions(&mut train, &world); // players: met
        assert_eq!(rt.condition_progress, vec![2, 0]);
        // Column 0 has run out of conditions: the entry is over.
        rt.tick_conditions(&mut train, &world);
        assert_eq!((rt.state, rt.current_entry), (State::PreTransit, 1));
    }

    #[test]
    fn an_entry_without_conditions_is_over_when_reached() {
        let mut train = driver();
        let world = World::default();
        let mut rt = ScheduleRuntime::new();
        rt.set_schedule(schedule_of(vec![ScheduleEntry::new(dest("A"), vec![]), stop("B", 1)]), false, &mut train);
        rt.state = State::InTransit;
        rt.destination_reached(&mut train);
        // No columns: nothing can be met, so the wait never ends by itself (Java: the loop is empty).
        rt.tick_conditions(&mut train, &world);
        assert_eq!(rt.state, State::PostTransit);
        // A rename is not a stop: it moves on.
        rt.set_schedule(schedule_of(vec![ScheduleEntry::new(ScheduleInstruction::Rename { text: "x".into() }, vec![])]), false, &mut train);
        rt.state = State::PostTransit;
        rt.tick_conditions(&mut train, &world);
        assert_eq!((rt.state, rt.current_entry), (State::PreTransit, 1));
    }

    #[test]
    fn returning_and_discarding() {
        let mut train = driver();
        let mut rt = ScheduleRuntime::new();
        assert!(rt.return_schedule(&mut train).is_none());
        rt.set_schedule(schedule_of(vec![stop("A", 1), stop("B", 1)]), false, &mut train);
        rt.current_entry = 1;
        let back = rt.return_schedule(&mut train).unwrap();
        assert_eq!(back.saved_progress, 1);
        assert_eq!(train.cancelled, 1);
        assert!(rt.schedule().is_none() && rt.paused);
        // An automatic schedule is not given back.
        rt.set_schedule(schedule_of(vec![stop("A", 1)]), true, &mut train);
        assert!(rt.return_schedule(&mut train).is_none());
        assert!(rt.schedule().is_none());
    }

    #[test]
    fn stay_estimates_use_the_first_all_delay_column() {
        let mut train = driver();
        let mut rt = ScheduleRuntime::new();
        let entry = ScheduleEntry::new(
            dest("A"),
            vec![
                vec![delay(1), WaitCondition::Powered],
                vec![delay(1), delay(2)],
                vec![delay(10)],
            ],
        );
        rt.set_schedule(schedule_of(vec![entry, ScheduleEntry::new(dest("B"), vec![vec![WaitCondition::Powered]])]), false, &mut train);
        assert_eq!(rt.estimate_stay_duration(0), 60);
        assert_eq!(rt.estimate_stay_duration(1), INVALID);
        // Past the end: wraps when cyclic.
        assert_eq!(rt.estimate_stay_duration(2), 60);
        rt.schedule.as_mut().unwrap().cyclic = false;
        assert_eq!(rt.estimate_stay_duration(2), INVALID);
        // An idle condition is not a delay.
        let idle = ScheduleEntry::new(dest("A"), vec![vec![WaitCondition::Idle(TimedWait::default())]]);
        rt.schedule = Some(schedule_of(vec![idle]));
        assert_eq!(rt.estimate_stay_duration(0), INVALID);
    }

    #[test]
    fn predictions_for_a_train_at_a_station() {
        let mut train = driver();
        let mut rt = ScheduleRuntime::new();
        rt.set_schedule(schedule_of(vec![stop("A*", 5), stop("B", 10), stop("C", 1)]), false, &mut train);
        rt.state = State::PostTransit;
        rt.prediction_ticks = vec![100, 200, 300];
        train.current_name = Some("A Central".to_string());
        let p = rt.submit_predictions(&train);
        // Here now (0), then B after A's 100 ticks of waiting plus its 200 of travel, then C.
        assert_eq!(p[0], DeparturePrediction { ticks: 0, text: "B".to_string(), destination: "A Central".to_string() });
        assert_eq!(p[1], DeparturePrediction { ticks: 100 + 200, text: "C".to_string(), destination: "B".to_string() });
        assert_eq!(p[2].ticks, 100 + 200 + 200 + 300);
        assert_eq!(p[2].destination, "C");
        assert_eq!(p.len(), 3);
    }

    #[test]
    fn predictions_for_a_moving_train() {
        let mut train = driver();
        let mut rt = ScheduleRuntime::new();
        rt.set_schedule(schedule_of(vec![stop("A", 5), stop("B", 10)]), false, &mut train);
        rt.state = State::InTransit;
        train.destination_name = Some("A".to_string());
        train.to_destination = 90.0;
        train.started_at = 0.0;
        let p = rt.submit_predictions(&train);
        // speed = min(1 * 4, (4 + 2) / 2) = 3: 90 / 3 = 30, times 2 = 60 ticks to go.
        assert_eq!(p[0].ticks, 60);
        assert_eq!(p[0].destination, "A");
        // Then 100 ticks of stay, and B is not known yet (TBD): -1, which still is listed.
        assert_eq!(p[1].ticks, TBD);
        assert_eq!(p.len(), 2);

        // With a known transit time and progress: blended.
        rt.prediction_ticks = vec![400, 100];
        train.started_at = 180.0;
        let p = rt.submit_predictions(&train);
        // predicted = 400 * (90 / 180) = 200; (60 + 200) / 2 = 130.
        assert_eq!(p[0].ticks, 130);
        assert_eq!(p[1].ticks, 130 + 100 + 100);
    }

    #[test]
    fn predictions_skip_renames_and_use_titles() {
        let mut train = driver();
        let mut rt = ScheduleRuntime::new();
        let rename = ScheduleEntry::new(ScheduleInstruction::Rename { text: "Express".to_string() }, vec![]);
        rt.set_schedule(schedule_of(vec![stop("A", 5), rename, stop("B", 5)]), false, &mut train);
        rt.state = State::PostTransit;
        rt.prediction_ticks = vec![0, 0, 50];
        train.current_name = Some("A".to_string());
        let p = rt.submit_predictions(&train);
        // Current (A, 0), then B under the new title.
        assert_eq!(p.len(), 2);
        assert_eq!(p[1].text, "Express");
        assert_eq!(p[1].destination, "B");
        // A title of the runtime's own comes first.
        rt.current_title = "Local".to_string();
        let p = rt.submit_predictions(&train);
        assert_eq!(p[0].text, "Local");
    }
}
