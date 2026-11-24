//! What the trains tell the Display Link: `GlobalTrainDisplayData` (the departure board's predictions), the text of
//! `StationSummaryDisplaySource`, `TrainStatusDisplaySource` and `ObservedTrainNameSource`, and the waiting status of each
//! wait condition (`ScheduleRuntime.getWaitingStatus`, `ScheduleWaitCondition.getWaitingStatus`).
//!
//! Ported from `content/trains/display/GlobalTrainDisplayData.java`, `content/redstone/displayLink/source/{StationSummary,
//! TrainStatus}DisplaySource.java` and `ObservedTrainNameSource.java`, and `getWaitingStatus` in `content/trains/schedule`.
//! Text is plain strings in Create's English (`create.display_source.*`, `create.schedule.condition.*.status`).
//!
//! Differences from Create, for the README:
//! - Java keeps the data in one global manager; here one [`GlobalTrainDisplayData`] is kept for each dimension's
//!   [`Railways`], and a summary lists the trains of its own dimension.
//! - The table is a `BTreeMap` by destination where Java has a `HashMap`: predictions with the same ticks and train name
//!   (the full sort key) keep the order of their destinations, which Java leaves to the hash.
//! - The tie break between trains of the same ticks is `String.compareTo`'s order of UTF-16 units.
//! - A station summary's two column scroll inputs (`NameColumn` 0 to 73, `PlatformColumn` 0 to 16) are [`LAYOUTS`], a short
//!   list of pairs the Wrench steps through (see `display_link.rs`'s header).
//! - The DISPLAY_BOARD advancement isn't awarded.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use super::graph::Uuid;
use super::railways::Railways;
use super::schedule::{ConditionContext, ScheduleRuntime, State, WaitCondition};
use super::train::{ScheduleCtx, Train, TrainWorld};
use crate::Pos;
use crate::rules::flap_display::{Layout, MONOSPACE, Section};
use crate::rules::package::Glob;

/// `StationSummaryDisplaySource.UNPREDICTABLE`.
pub const UNPREDICTABLE: &str = " ~ ";
/// `DisplaySource.WHITESPACE`.
pub const WHITESPACE: &str = " ";
/// `GlobalRailwayManager.tick`: the table is refreshed when `gameTime % 100 == 0`.
pub const REFRESH_EVERY: i64 = 100;
/// `StationSummaryDisplaySource.populateData`: the default of `PlatformColumn` and `NameColumn`.
pub const DEFAULT_PLATFORM_COLUMN: i32 = 3;
pub const DEFAULT_NAME_COLUMN: i32 = 50;

/// The `(NameColumn, PlatformColumn)` pairs the Wrench offers for a station summary, the defaults first (a stand-in for the
/// two scroll inputs: ranges 0 to 73 and 0 to 16 in Create).
pub const LAYOUTS: [(i32, i32); 6] = [(50, 3), (25, 3), (73, 3), (50, 0), (50, 6), (0, 3)];

/// `(NameColumn, PlatformColumn)` of layout `index` (wrapped).
pub fn layout_columns(index: i32) -> (i32, i32) {
    LAYOUTS[index.rem_euclid(LAYOUTS.len() as i32) as usize]
}

// --- GlobalTrainDisplayData ---------------------------------------------------------------------------------------

/// `GlobalTrainDisplayData.TrainDeparturePrediction`, with the train's name in place of the train.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prediction {
    pub train_name: String,
    pub ticks: i32,
    pub schedule_title: String,
    pub destination: String,
}

impl Prediction {
    /// `getCompareTicks()`.
    fn compare_ticks(&self) -> i32 {
        if self.ticks == -1 {
            i32::MAX
        } else if self.ticks < 200 {
            0
        } else {
            self.ticks
        }
    }

    /// `compareTo`: by `getCompareTicks`, then by the train's name (`String.compareTo`, UTF-16 units).
    pub fn compare(&self, other: &Prediction) -> Ordering {
        self.compare_ticks()
            .cmp(&other.compare_ticks())
            .then_with(|| self.train_name.encode_utf16().cmp(other.train_name.encode_utf16()))
    }
}

/// What a train asks of the world while its predictions are made: nothing that matters (`submitPredictions` reads only the
/// train, its navigation and its schedule).
pub struct NoWorld;

impl TrainWorld for NoWorld {
    fn day_time(&self) -> i64 {
        0
    }
    fn redstone_power_version(&self) -> i32 {
        0
    }
    fn has_any_loaded_power(&self, _: &[String; 2]) -> bool {
        false
    }
    fn station_powered(&self, _: Uuid) -> bool {
        false
    }
    fn station_unloaded(&self, _: Uuid) -> bool {
        false
    }
    fn dimension_loaded(&self, _: u32) -> bool {
        true
    }
    fn port_packages(&self, _: Pos, _: u32) -> Vec<String> {
        Vec::new()
    }
    fn count_player_passengers(&self, _: Uuid) -> i32 {
        0
    }
    fn check_conductors(&self, _: Uuid, _: i32) -> Option<[bool; 2]> {
        None
    }
}

/// `GlobalTrainDisplayData`: `statusByDestination`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GlobalTrainDisplayData {
    status_by_destination: BTreeMap<String, Vec<Prediction>>,
}

impl GlobalTrainDisplayData {
    pub fn new() -> GlobalTrainDisplayData {
        GlobalTrainDisplayData::default()
    }

    /// `statusByDestination.clear()`.
    pub fn clear(&mut self) {
        self.status_by_destination.clear();
    }

    /// `GlobalTrainDisplayData.refresh()`: every train with a running schedule that is on its graph asks its runtime for its
    /// predictions.
    pub fn refresh(&mut self, rails: &mut Railways) {
        self.status_by_destination.clear();
        let Railways { nets, trains, .. } = rails;
        let ids: Vec<Uuid> = trains.keys().copied().collect();
        for id in ids {
            let Some(mut train) = trains.remove(&id) else { continue };
            if !(train.runtime.paused || train.runtime.schedule().is_none() || train.derailed || train.graph.is_none()) {
                let predictions = predictions_of(&mut train, nets, trains);
                for p in predictions {
                    self.status_by_destination.entry(p.destination.clone()).or_default().push(p);
                }
            }
            trains.insert(id, train);
        }
    }

    /// `prepare(filter, maxLines)`: the predictions for the stations the glob `filter` matches, soonest first.
    pub fn prepare(&self, filter: &str, max_lines: usize) -> Vec<Prediction> {
        let glob = Glob::new(filter);
        let mut found: Vec<Prediction> = self
            .status_by_destination
            .iter()
            .filter(|(destination, _)| glob.matches(destination))
            .flat_map(|(_, list)| list.iter().cloned())
            .collect();
        found.sort_by(Prediction::compare);
        found.truncate(max_lines);
        found
    }

    /// Every prediction there is (the table as the display source reads it).
    pub fn all(&self) -> Vec<Prediction> {
        self.status_by_destination.values().flatten().cloned().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.status_by_destination.is_empty()
    }

    /// The table filled with `predictions` (for a reading made of an earlier refresh).
    pub fn from_predictions(predictions: Vec<Prediction>) -> GlobalTrainDisplayData {
        let mut data = GlobalTrainDisplayData::default();
        for p in predictions {
            data.status_by_destination.entry(p.destination.clone()).or_default().push(p);
        }
        data
    }
}

/// `train.runtime.submitPredictions()`, as `TrainDeparturePrediction`s of the train.
fn predictions_of(train: &mut Train, nets: &mut super::network::RailNetworks, others: &BTreeMap<Uuid, Train>) -> Vec<Prediction> {
    let runtime = std::mem::take(&mut train.runtime);
    let mut nav = std::mem::take(&mut train.navigation);
    let found = {
        let ctx = ScheduleCtx { train: &mut *train, nav: &mut nav, nets, others, world: &NoWorld };
        runtime.submit_predictions(&ctx)
    };
    train.navigation = nav;
    train.runtime = runtime;
    found
        .into_iter()
        .map(|p| Prediction { train_name: train.name.clone(), ticks: p.ticks, schedule_title: p.text, destination: p.destination })
        .collect()
}

// --- StationSummaryDisplaySource ------------------------------------------------------------------------------------

/// `StationSummaryDisplaySource.populateData`: the filter is the station's name when the source block is a station.
pub fn default_filter(station_name: Option<&str>) -> Option<String> {
    station_name.map(str::to_owned)
}

/// The first two columns of a row: the minutes and the unit, from the ticks until the train is there.
pub fn time_columns(ticks: i32) -> [String; 2] {
    if ticks == -1 || ticks >= 12000 - 15 * 20 {
        return [WHITESPACE.to_owned(), UNPREDICTABLE.to_owned()];
    }
    if ticks < 200 {
        return [WHITESPACE.to_owned(), "now".to_owned()];
    }
    let mut min = ticks / 1200;
    let mut sec = (ticks / 20) % 60;
    sec = (sec as f32 / 15.0).ceil() as i32 * 15;
    if sec == 60 {
        min += 1;
        sec = 0;
    }
    if min > 0 {
        [min.to_string(), " min".to_owned()]
    } else {
        [WHITESPACE.to_owned(), format!("{sec}s")]
    }
}

/// `provideFlapDisplayText`: a row for each prediction `data` has for `filter` (up to `rows`), the rest of the board padded with
/// the empty entries. A row is the texts of the sections of its line: minutes, unit, train, title and, when the filter has a `*`,
/// the platform.
pub fn station_summary_rows(data: &GlobalTrainDisplayData, filter: &str, rows: usize) -> Vec<Vec<String>> {
    let has_platform = filter.contains('*');
    let mut list: Vec<Vec<String>> = Vec::new();
    for prediction in data.prepare(filter, rows) {
        let mut lines: Vec<String> = time_columns(prediction.ticks).to_vec();
        lines.push(prediction.train_name.clone());
        lines.push(prediction.schedule_title.clone());
        if has_platform {
            let mut platform = prediction.destination.clone();
            for part in filter.split('*') {
                if !part.is_empty() {
                    platform = platform.replace(part, "");
                }
            }
            platform = platform.replace('*', "?");
            lines.push(platform.trim().to_owned());
        }
        list.push(lines);
    }
    let pad = rows.saturating_sub(list.len());
    for _ in 0..pad {
        let mut entry = vec![WHITESPACE.to_owned(), " . ".to_owned(), WHITESPACE.to_owned(), WHITESPACE.to_owned()];
        if has_platform {
            entry.push(WHITESPACE.to_owned());
        }
        list.push(entry);
    }
    list
}

/// `loadFlapDisplayLayout` of the station summary: minutes, the arrival unit, the train, the destination (title) and the
/// platform column, sized from the board's width in blocks (`x_size`).
pub fn station_summary_layout(layout: &mut Layout, filter: &str, name_column: i32, platform_column: i32, x_size: i32) {
    let has_platform = filter.contains('*');
    let name = format!("StationSummary{name_column}{has_platform}{platform_column}");
    if layout.is_layout(&name) {
        return;
    }
    let time_width = 20.0_f32;
    let gap_size = 8.0_f32;
    let mut platform_width = platform_column as f32 * MONOSPACE;

    let minutes = Section::new(MONOSPACE, "numeric", false, false);
    let time = Section::new(time_width, "arrival_time", true, true);

    let mut total_size = x_size as f32 * 32.0 - 4.0 - gap_size * 2.0;
    total_size = total_size - time_width - MONOSPACE;
    platform_width = platform_width.min(total_size - gap_size);
    platform_width = ((platform_width / MONOSPACE) as i32) as f32 * MONOSPACE;

    if has_platform {
        total_size = total_size - gap_size - platform_width;
    }
    if platform_width == 0.0 && has_platform {
        total_size += gap_size;
    }

    let train_name_width = ((name_column as f32 / 100.0) * total_size / MONOSPACE) as i32;
    let destination_width = ((1.0 - name_column as f32 / 100.0) * total_size / MONOSPACE + 0.5).floor() as i32;

    let train_name = Section::new(train_name_width as f32 * MONOSPACE, "alphabet", false, train_name_width > 0);
    let destination = Section::new(
        destination_width as f32 * MONOSPACE,
        "alphabet",
        false,
        has_platform && destination_width > 0 && platform_width > 0.0,
    );
    let platform = Section::new(platform_width, "numeric", false, false).right_aligned();

    let mut sections = vec![minutes, time, train_name, destination];
    if has_platform {
        sections.push(platform);
    }
    layout.configure(&name, sections);
}

// --- TrainStatusDisplaySource, ObservedTrainNameSource -----------------------------------------------------------------

/// `TrainStatusDisplaySource.provideLine` for the train at the station: `None` is `EMPTY_LINE`. `day_time` is the level's day
/// time and `passengers` `train.countPlayerPassengers()`.
pub fn train_status_line(train: Option<&Train>, day_time: i64, passengers: i32) -> Option<String> {
    let runtime = &train?.runtime;
    let schedule = runtime.schedule()?;
    if runtime.paused {
        return None;
    }
    if runtime.state != State::PostTransit {
        return None;
    }
    if runtime.current_entry == schedule.entries.len() as i32 - 1 && !schedule.cyclic {
        return None;
    }
    Some(waiting_status(runtime, day_time, passengers))
}

/// `ObservedTrainNameSource.provideLine`: the name of the train the observer sees, `None` for `EMPTY_LINE`.
pub fn observed_train_name(train: Option<&Train>) -> Option<String> {
    train.map(|t| t.name.clone())
}

/// `ScheduleRuntime.getWaitingStatus(level)`: the status text of the first column's current condition, empty when there is none.
pub fn waiting_status(runtime: &ScheduleRuntime, day_time: i64, passengers: i32) -> String {
    let Some(schedule) = runtime.schedule() else { return String::new() };
    let Some(entry) = usize::try_from(runtime.current_entry).ok().and_then(|i| schedule.entries.get(i)) else {
        return String::new();
    };
    if entry.conditions.is_empty() || runtime.condition_progress.is_empty() || runtime.condition_context.is_empty() {
        return String::new();
    }
    let list = &entry.conditions[0];
    let progress = runtime.condition_progress[0];
    let Some(condition) = usize::try_from(progress).ok().and_then(|p| list.get(p)) else { return String::new() };
    condition_status(condition, &runtime.condition_context[0], day_time, passengers)
}

/// `getWaitingStatus` of one wait condition.
pub fn condition_status(condition: &WaitCondition, ctx: &ConditionContext, day_time: i64, passengers: i32) -> String {
    match condition {
        // `TimedWaitCondition.getWaitingStatus`: `schedule.condition.<id>.status` with "5 Seconds".
        WaitCondition::Delay(t) => format!("Departs in {}", timed_text(t.waiting_status(ctx.time))),
        WaitCondition::Idle(t) => format!("Cargo Idle for {}", timed_text(t.waiting_status(ctx.time))),
        WaitCondition::TimeOfDay(t) => {
            let (hour, minute) = t.next_departure(day_time);
            format!("Departs at {}", digital_display(hour, minute))
        }
        WaitCondition::FluidThreshold(t) => match t.clone().waiting_status(ctx) {
            Some((shown, threshold)) => format!("Cargo: {shown}/{threshold} Buckets"),
            None => String::new(),
        },
        WaitCondition::ItemThreshold(t) => {
            let in_stacks = t.in_stacks();
            match t.clone().waiting_status(ctx) {
                Some((shown, threshold)) => {
                    format!("Cargo: {shown}/{threshold} {}", if in_stacks { "Stacks" } else { "Items" })
                }
                None => String::new(),
            }
        }
        WaitCondition::RedstoneLink { .. } => "Waiting for redstone link".to_owned(),
        WaitCondition::PlayerCount { count, .. } => format!("Passengers: {passengers}/{count}"),
        WaitCondition::Unloaded => "Waiting for chunk unload".to_owned(),
        WaitCondition::Powered => "Waiting for redstone".to_owned(),
    }
}

/// `Component.literal(num + " ").append(translate(key))`: "1 Minute", "5 Seconds".
fn timed_text((num, in_minutes): (i32, bool)) -> String {
    let unit = match (in_minutes, num == 1) {
        (true, true) => "Minute",
        (true, false) => "Minutes",
        (false, true) => "Second",
        (false, false) => "Seconds",
    };
    format!("{num} {unit}")
}

/// `TimeOfDayCondition.getDigitalDisplay(hour, minute, false)`: "7:05 pm".
pub fn digital_display(hour: i32, minute: i32) -> String {
    let hour12 = if hour % 12 == 0 { 12 } else { hour % 12 };
    let suffix = if hour > 11 { "pm" } else { "am" };
    format!("{hour12}:{minute:02} {suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::trains::schedule::{Schedule, ScheduleEntry, ScheduleInstruction, TBD, TimedWait};

    fn prediction(name: &str, ticks: i32, destination: &str) -> Prediction {
        Prediction {
            train_name: name.to_owned(),
            ticks,
            schedule_title: format!("To {destination}"),
            destination: destination.to_owned(),
        }
    }

    fn data(predictions: Vec<Prediction>) -> GlobalTrainDisplayData {
        GlobalTrainDisplayData::from_predictions(predictions)
    }

    #[test]
    fn predictions_sort_by_ticks_with_the_near_ones_tied_and_the_unknown_last() {
        let d = data(vec![
            prediction("Zed", 100, "A"),
            prediction("Bob", 150, "A"),
            prediction("Cat", 5000, "A"),
            prediction("Dan", -1, "A"),
            prediction("Eve", 300, "A"),
        ]);
        let names: Vec<String> = d.prepare("A", 10).into_iter().map(|p| p.train_name).collect();
        // Under 200 ticks all count as 0, so the names order Bob before Zed.
        assert_eq!(names, vec!["Bob", "Zed", "Eve", "Cat", "Dan"]);
        assert_eq!(d.prepare("A", 2).len(), 2);
    }

    #[test]
    fn the_filter_is_a_glob_over_the_whole_destination() {
        let d = data(vec![
            prediction("T1", 500, "Central Platform 1"),
            prediction("T2", 600, "Central Platform 2"),
            prediction("T3", 700, "Harbour"),
        ]);
        assert_eq!(d.prepare("Central *", 10).len(), 2);
        assert_eq!(d.prepare("Harbour", 10).len(), 1);
        // Not a prefix match: the glob must cover the name.
        assert_eq!(d.prepare("Central", 10).len(), 0);
        assert_eq!(d.prepare("*", 10).len(), 3);
        assert_eq!(d.prepare("", 10).len(), 0);
    }

    #[test]
    fn the_time_columns_round_the_seconds_up_to_fifteen() {
        assert_eq!(time_columns(-1), [" ", " ~ "]);
        assert_eq!(time_columns(12000 - 300), [" ", " ~ "]);
        assert_eq!(time_columns(0), [" ", "now"]);
        assert_eq!(time_columns(199), [" ", "now"]);
        // 200 ticks is 10 seconds, shown as 15s.
        assert_eq!(time_columns(200), [" ", "15s"]);
        // 20 seconds shows as 30s, 46 as a minute.
        assert_eq!(time_columns(20 * 20), [" ", "30s"]);
        assert_eq!(time_columns(46 * 20), ["1", " min"]);
        assert_eq!(time_columns(1200 + 3 * 20), ["1", " min"]);
        assert_eq!(time_columns(1200 * 5), ["5", " min"]);
        // 59 seconds rounds to 60: the minute carries.
        assert_eq!(time_columns(1200 * 2 + 59 * 20), ["3", " min"]);
    }

    #[test]
    fn a_summary_without_a_platform_has_four_sections() {
        let d = data(vec![prediction("Express", 100, "Central"), prediction("Local", 2400, "Central")]);
        let rows = station_summary_rows(&d, "Central", 4);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0], vec![" ", "now", "Express", "To Central"]);
        assert_eq!(rows[1], vec!["2", " min", "Local", "To Central"]);
        // The rest of the board is padded.
        assert_eq!(rows[2], vec![" ", " . ", " ", " "]);
        assert_eq!(rows[3].len(), 4);
    }

    #[test]
    fn a_star_in_the_filter_adds_the_platform() {
        let d = data(vec![prediction("Express", 100, "Central 3"), prediction("Local", 900, "Central 12")]);
        let rows = station_summary_rows(&d, "Central *", 3);
        assert_eq!(rows[0], vec![" ", "now", "Express", "To Central 3", "3"]);
        assert_eq!(rows[1].last().map(String::as_str), Some("12"));
        assert_eq!(rows[2], vec![" ", " . ", " ", " ", " "]);
        // Several stars: every piece between them is taken out, the star itself becomes a question mark.
        let d = data(vec![prediction("T", 100, "A Terminal 7 North")]);
        assert_eq!(station_summary_rows(&d, "A * North", 1)[0][4], "Terminal 7");
        assert_eq!(station_summary_rows(&d, "*", 1)[0][4], "A Terminal 7 North");
    }

    #[test]
    fn the_layout_follows_the_board_and_the_columns() {
        let mut layout = Layout::new(30);
        station_summary_layout(&mut layout, "Central", 50, 3, 4);
        assert!(layout.is_layout("StationSummary50false3"));
        assert_eq!(layout.sections.len(), 4);
        assert_eq!(layout.sections[0].cycle, "numeric");
        assert_eq!(layout.sections[1].cycle, "arrival_time");
        assert!(layout.sections[1].single_flap);
        // 4 blocks: 128 - 4 - 16 = 108, less 20 and 7 is 81; half of it in flaps of 7.
        assert_eq!(layout.sections[2].flaps(), 5);
        assert_eq!(layout.sections[3].flaps(), 6);
        // With a platform the third column is cut out of the width and the section is right aligned.
        station_summary_layout(&mut layout, "Central *", 50, 3, 4);
        assert!(layout.is_layout("StationSummary50true3"));
        assert_eq!(layout.sections.len(), 5);
        assert!(layout.sections[4].right_aligned);
        assert_eq!(layout.sections[4].flaps(), 3);
        // A platform column of 0 gives the space back.
        station_summary_layout(&mut layout, "Central *", 50, 0, 4);
        assert_eq!(layout.sections[4].flaps(), 0);
        // The same layout is kept.
        let before = layout.clone();
        station_summary_layout(&mut layout, "Central *", 50, 0, 4);
        assert_eq!(layout, before);
    }

    #[test]
    fn the_layouts_start_at_the_defaults() {
        assert_eq!(layout_columns(0), (DEFAULT_NAME_COLUMN, DEFAULT_PLATFORM_COLUMN));
        assert_eq!(layout_columns(LAYOUTS.len() as i32), LAYOUTS[0]);
        assert_eq!(layout_columns(-1), LAYOUTS[LAYOUTS.len() - 1]);
    }

    fn runtime_with(entries: Vec<ScheduleEntry>, cyclic: bool, current: i32, condition: WaitCondition) -> (ScheduleRuntime, ConditionContext) {
        let _ = condition;
        let mut rt = ScheduleRuntime::new();
        rt.schedule = Some(Schedule { entries, cyclic, saved_progress: 0 });
        rt.paused = false;
        rt.current_entry = current;
        rt.state = State::PostTransit;
        rt.prediction_ticks = vec![TBD; 2];
        (rt, ConditionContext::default())
    }

    fn stop(name: &str, conditions: Vec<Vec<WaitCondition>>) -> ScheduleEntry {
        ScheduleEntry::new(ScheduleInstruction::Destination { text: name.to_owned() }, conditions)
    }

    fn train_with(rt: ScheduleRuntime) -> Train {
        let mut train = Train::new(1, None, Some(9), Vec::new(), Vec::new(), false, "Express", "", 0);
        train.runtime = rt;
        train
    }

    #[test]
    fn waiting_status_texts() {
        let ctx = ConditionContext::default();
        let delay = WaitCondition::Delay(TimedWait { value: 30, unit: 1 });
        // 30 seconds is 600 ticks; nothing waited yet: 600 / 100 * 5 = 30.
        assert_eq!(condition_status(&delay, &ctx, 0, 0), "Departs in 30 Seconds");
        let one = WaitCondition::Delay(TimedWait { value: 1, unit: 2 });
        assert_eq!(condition_status(&one, &ConditionContext { time: 0, ..ctx.clone() }, 0, 0), "Departs in 1 Minute");
        let two = WaitCondition::Delay(TimedWait { value: 2, unit: 2 });
        assert_eq!(condition_status(&two, &ctx, 0, 0), "Departs in 2 Minutes");
        let idle = WaitCondition::Idle(TimedWait { value: 5, unit: 1 });
        assert_eq!(condition_status(&idle, &ctx, 0, 0), "Cargo Idle for 5 Seconds");
        assert_eq!(condition_status(&WaitCondition::Powered, &ctx, 0, 0), "Waiting for redstone");
        assert_eq!(condition_status(&WaitCondition::Unloaded, &ctx, 0, 0), "Waiting for chunk unload");
        let link = WaitCondition::RedstoneLink { frequency: [String::new(), String::new()], inverted: 0 };
        assert_eq!(condition_status(&link, &ctx, 0, 0), "Waiting for redstone link");
        let players = WaitCondition::PlayerCount { count: 3, exact: 0 };
        assert_eq!(condition_status(&players, &ctx, 0, 2), "Passengers: 2/3");
    }

    #[test]
    fn threshold_status_waits_for_the_first_count() {
        use crate::rules::trains::schedule::Threshold;
        let items = WaitCondition::ItemThreshold(Threshold { threshold: "10".into(), operator: 0, measure: 0, filter: None });
        assert_eq!(condition_status(&items, &ConditionContext::default(), 0, 0), "");
        let counted = ConditionContext { current_display: Some(4), ..Default::default() };
        // Greater: the threshold moves one up.
        assert_eq!(condition_status(&items, &counted, 0, 0), "Cargo: 4/11 Items");
        let stacks = WaitCondition::ItemThreshold(Threshold { threshold: "2".into(), operator: 1, measure: 1, filter: None });
        assert_eq!(condition_status(&stacks, &counted, 0, 0), "Cargo: 4/1 Stacks");
        let fluids = WaitCondition::FluidThreshold(Threshold { threshold: "2".into(), operator: 2, measure: 0, filter: None });
        assert_eq!(condition_status(&fluids, &counted, 0, 0), "Cargo: 4/2 Buckets");
    }

    #[test]
    fn the_time_of_day_status_names_the_next_departure() {
        use crate::rules::trains::schedule::TimeOfDay;
        // 19:05 on a 24 hour rotation, asked at day time 0 (6:00): the same day at 19:05.
        let at = WaitCondition::TimeOfDay(TimeOfDay { hour: 19, minute: 5, rotation: 0 });
        assert_eq!(condition_status(&at, &ConditionContext::default(), 0, 0), "Departs at 7:05 pm");
        assert_eq!(digital_display(0, 0), "12:00 am");
        assert_eq!(digital_display(12, 30), "12:30 pm");
        assert_eq!(digital_display(8, 9), "8:09 am");
    }

    #[test]
    fn the_train_status_shows_only_while_the_train_waits_at_a_station() {
        let delay = WaitCondition::Delay(TimedWait { value: 30, unit: 1 });
        let entries = vec![stop("A", vec![vec![delay.clone()]]), stop("B", vec![])];
        let (mut rt, ctx) = runtime_with(entries.clone(), true, 0, delay.clone());
        rt.condition_progress = vec![0];
        rt.condition_context = vec![ctx.clone()];
        let train = train_with(rt.clone());
        assert_eq!(train_status_line(Some(&train), 0, 0).as_deref(), Some("Departs in 30 Seconds"));
        assert_eq!(train_status_line(None, 0, 0), None);

        // Paused, in transit and no schedule are empty lines.
        let mut paused = rt.clone();
        paused.paused = true;
        assert_eq!(train_status_line(Some(&train_with(paused)), 0, 0), None);
        let mut moving = rt.clone();
        moving.state = State::InTransit;
        assert_eq!(train_status_line(Some(&train_with(moving)), 0, 0), None);
        assert_eq!(train_status_line(Some(&train_with(ScheduleRuntime::new())), 0, 0), None);

        // The last stop of a schedule that isn't cyclic.
        let (mut last, ctx) = runtime_with(entries.clone(), false, 1, delay.clone());
        last.condition_progress = vec![0];
        last.condition_context = vec![ctx];
        assert_eq!(train_status_line(Some(&train_with(last)), 0, 0), None);
        let (mut cyclic, ctx) = runtime_with(entries, true, 1, delay);
        cyclic.condition_progress = vec![0];
        cyclic.condition_context = vec![ctx];
        // The second stop has no conditions: the status is empty text, not an empty line.
        assert_eq!(train_status_line(Some(&train_with(cyclic)), 0, 0).as_deref(), Some(""));
    }

    #[test]
    fn a_finished_column_has_no_status() {
        let delay = WaitCondition::Delay(TimedWait { value: 30, unit: 1 });
        let (mut rt, ctx) = runtime_with(vec![stop("A", vec![vec![delay.clone()]])], true, 0, delay);
        rt.condition_progress = vec![1];
        rt.condition_context = vec![ctx];
        assert_eq!(waiting_status(&rt, 0, 0), "");
    }

    #[test]
    fn the_observed_name_is_the_trains() {
        let train = train_with(ScheduleRuntime::new());
        assert_eq!(observed_train_name(Some(&train)).as_deref(), Some("Express"));
        assert_eq!(observed_train_name(None), None);
    }

    #[test]
    fn only_trains_with_a_running_schedule_on_a_graph_are_listed() {
        let delay = WaitCondition::Delay(TimedWait { value: 30, unit: 1 });
        let entries = vec![stop("Central", vec![vec![delay.clone()]]), stop("Harbour", vec![])];
        let make = |id: Uuid, name: &str, mutate: &dyn Fn(&mut Train)| {
            let (rt, _) = runtime_with(entries.clone(), true, 0, delay.clone());
            let mut train = Train::new(id, None, Some(9), Vec::new(), Vec::new(), false, name, "", 0);
            train.runtime = rt;
            mutate(&mut train);
            train
        };
        let mut rails = Railways::new();
        for train in [
            make(1, "Running", &|_| {}),
            make(2, "Paused", &|t| t.runtime.paused = true),
            make(3, "Derailed", &|t| t.derailed = true),
            make(4, "Detached", &|t| t.graph = None),
            make(5, "Scheduleless", &|t| t.runtime = ScheduleRuntime::new()),
        ] {
            rails.trains.insert(train.id, train);
        }
        let mut display = GlobalTrainDisplayData::new();
        display.refresh(&mut rails);
        let names: Vec<String> = display.all().into_iter().map(|p| p.train_name).collect();
        assert!(!names.is_empty(), "the running train makes predictions");
        assert!(names.iter().all(|n| n == "Running"), "{names:?}");
        // A refresh starts again from nothing.
        rails.trains.clear();
        display.refresh(&mut rails);
        assert!(display.is_empty());
        // The runtimes and navigations came back to the trains.
        let mut rails = Railways::new();
        let train = make(1, "Running", &|_| {});
        rails.trains.insert(1, train);
        display.refresh(&mut rails);
        assert!(rails.trains[&1].runtime.schedule().is_some());
    }
}
