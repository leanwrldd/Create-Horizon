//! The Display Link's rules (`DisplayLinkBlock`, `DisplayLinkBlockEntity`, `DisplaySource` and the
//! sources under `displayLink/source`, `DisplayTarget` and the targets under `displayLink/target`,
//! `ClickToLinkBlockItem`): which blocks can be read and written, the text each source makes of
//! what it read, how that text is cut to what each target holds, and when a link refreshes.
//!
//! The glue reads the world into a [`Reading`], gives it to [`provide_text`] (or
//! [`provide_flap_text`]) and writes the answer to the target; nothing here knows the game.
//!
//! Known differences from Create, for the README:
//! - Only the sources of blocks this port has: speedometers, stressometers, cuckoo clocks (time
//!   and stopwatch), smart observers (items and fluids), threshold switches, belts, depots and
//!   weighted ejectors (item names), seats, a target block (redstone power) and nixie tubes. The
//!   boiler, scoreboard, death count, enchanting power, tunnel, train, elevator, package and
//!   factory gauge sources are not here (their blocks aren't, or Horizon gives no way to read them).
//! - Text is plain: Create's styled components (colours in the boiler's lines, translatable
//!   names) become plain strings, and a name is the item's id's last part where the game's own
//!   name for it can't be asked.
//! - The configuration screen is replaced by clicks (see the glue's header).

use super::flap_display::{Layout, MONOSPACE, Section, WIDE_MONOSPACE};

/// `CLogistics.displayLinkRange` (`displayLinkRange`, 64): blocks the target may be away.
pub const RANGE: i32 = 64;
/// `DisplaySource.getPassiveRefreshTicks`.
pub const PASSIVE_REFRESH: i32 = 100;
/// `ValueListDisplaySource.ENTRIES_PER_PAGE`.
pub const ENTRIES_PER_PAGE: usize = 8;

// --- sources -------------------------------------------------------------------------------------

/// A `DisplaySource` of `AllDisplaySources` this port has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    KineticSpeed,
    KineticStress,
    TimeOfDay,
    Stopwatch,
    CountItems,
    ListItems,
    CountFluids,
    ListFluids,
    FillLevel,
    ItemNames,
    EntityName,
    RedstonePower,
    NixieTube,
    /// `FactoryGaugeDisplaySource` (`gauge_status`): the panels a Display Link is attached to, one row each.
    GaugeStatus,
}

/// How a source builds its flap display layout (`loadFlapDisplayLayout`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    /// `SingleLineDisplaySource` with the "Default" alphabet layout.
    Line,
    /// `NumericSingleLineDisplaySource`: "Number".
    Numeric,
    /// `PercentOrProgressBarDisplaySource`: "Number" or "Progress".
    Percent,
    /// "Instant", for the clocks.
    Instant,
    /// `ItemNameDisplaySource`: key "Number" but alphabet flaps, as Create has it.
    Names,
    /// `NixieTubeDisplaySource`: "Number" when the text is a number.
    Nixie,
    ValueList,
    FluidList,
}

impl Source {
    pub const ALL: [Source; 14] = [
        Source::KineticSpeed,
        Source::KineticStress,
        Source::TimeOfDay,
        Source::Stopwatch,
        Source::CountItems,
        Source::ListItems,
        Source::CountFluids,
        Source::ListFluids,
        Source::FillLevel,
        Source::ItemNames,
        Source::EntityName,
        Source::RedstonePower,
        Source::NixieTube,
        Source::GaugeStatus,
    ];

    /// The id Create registers it under (`AllDisplaySources`).
    pub const fn id(self) -> &'static str {
        match self {
            Source::KineticSpeed => "create:kinetic_speed",
            Source::KineticStress => "create:kinetic_stress",
            Source::TimeOfDay => "create:time_of_day",
            Source::Stopwatch => "create:stopwatch",
            Source::CountItems => "create:count_items",
            Source::ListItems => "create:list_items",
            Source::CountFluids => "create:count_fluids",
            Source::ListFluids => "create:list_fluids",
            Source::FillLevel => "create:fill_level",
            Source::ItemNames => "create:item_names",
            Source::EntityName => "create:entity_name",
            Source::RedstonePower => "create:redstone_power",
            Source::NixieTube => "create:nixie_tube",
            Source::GaugeStatus => "create:gauge_status",
        }
    }

    pub fn from_id(id: &str) -> Option<Source> {
        Source::ALL.into_iter().find(|s| s.id() == id)
    }

    /// `DisplaySource.getName`: `create.display_source.*` in Create's English.
    pub const fn name(self) -> &'static str {
        match self {
            Source::KineticSpeed => "Rotation Speed (RPM)",
            Source::KineticStress => "Network Stress",
            Source::TimeOfDay => "Time of Day",
            Source::Stopwatch => "Stopwatch",
            Source::CountItems => "Amount of matching Items",
            Source::ListItems => "List matching Items",
            Source::CountFluids => "Amount of matching Fluids",
            Source::ListFluids => "List matching Fluids",
            Source::FillLevel => "Container Fill Level",
            Source::ItemNames => "Combine Item Names",
            Source::EntityName => "Entity Name",
            Source::RedstonePower => "Redstone Power",
            Source::NixieTube => "Copy Nixie Tubes",
            Source::GaugeStatus => "Factory Gauge Status",
        }
    }

    fn shape(self) -> Shape {
        match self {
            Source::KineticSpeed | Source::CountItems => Shape::Numeric,
            Source::KineticStress | Source::FillLevel | Source::RedstonePower => Shape::Percent,
            Source::TimeOfDay | Source::Stopwatch => Shape::Instant,
            Source::CountFluids | Source::EntityName => Shape::Line,
            Source::ItemNames => Shape::Names,
            Source::NixieTube => Shape::Nixie,
            Source::ListItems | Source::GaugeStatus => Shape::ValueList,
            Source::ListFluids => Shape::FluidList,
        }
    }

    /// Whether it makes a single line (`SingleLineDisplaySource`).
    pub fn is_single_line(self) -> bool {
        !matches!(self.shape(), Shape::ValueList | Shape::FluidList)
    }

    /// `getPassiveRefreshTicks`.
    pub const fn refresh_ticks(self) -> i32 {
        match self {
            Source::Stopwatch => 20,
            _ => PASSIVE_REFRESH,
        }
    }

    /// `allowsLabeling` for a link writing to `target`.
    pub fn allows_labeling(self, target: Target) -> bool {
        match self {
            Source::ListItems | Source::ListFluids | Source::GaugeStatus => false,
            Source::NixieTube => target != Target::NixieTube,
            _ => true,
        }
    }

    /// The one option the configuration screen offers besides the label: its key in the saved
    /// config and the names of its values (`initConfigurationWidgets`).
    pub fn option(self) -> Option<(&'static str, &'static [&'static str])> {
        Some(match self {
            Source::KineticSpeed => ("Directional", &["Ignore Direction", "Include Direction"]),
            Source::KineticStress => (
                "Mode",
                &["Progress Bar", "Percentage", "Stress in SU", "Total Capacity", "Remaining SU"],
            ),
            Source::TimeOfDay => ("Cycle", &["12-hour", "24-hour"]),
            Source::FillLevel => ("Mode", &["Percent", "Progress Bar"]),
            Source::RedstonePower => ("Mode", &["Number", "Progress Bar"]),
            Source::ListItems | Source::ListFluids | Source::GaugeStatus => ("Format", &["Shortened", "Full Number"]),
            _ => return None,
        })
    }
}

/// `DisplaySource.BY_BLOCK` and `BY_BLOCK_ENTITY`: the sources that read the block `block`, in
/// the order Create registers them (the order the screen lists them in).
pub fn sources_for(block: &str) -> Vec<Source> {
    match block {
        "create:speedometer" => vec![Source::KineticSpeed],
        "create:stressometer" => vec![Source::KineticStress],
        "create:cuckoo_clock" | "create:mysterious_cuckoo_clock" => vec![Source::TimeOfDay, Source::Stopwatch],
        "create:content_observer" => {
            vec![Source::CountItems, Source::ListItems, Source::CountFluids, Source::ListFluids]
        }
        "create:stockpile_switch" => vec![Source::FillLevel],
        "create:belt" | "create:depot" | "create:weighted_ejector" => vec![Source::ItemNames],
        "minecraft:target" => vec![Source::RedstonePower],
        // `AllDisplaySources`: `factory_gauge_source_gauge_status`.
        "create:factory_gauge" => vec![Source::GaugeStatus],
        b if b.starts_with("create:") && b.ends_with("_seat") => vec![Source::EntityName],
        b if super::nixie_tube::dye_of_block(b).is_some() => vec![Source::NixieTube],
        _ => Vec::new(),
    }
}

// --- targets -------------------------------------------------------------------------------------

/// A `DisplayTarget` of `AllDisplayTargets`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Sign,
    Lectern,
    DisplayBoard,
    NixieTube,
}

impl Target {
    /// `DisplayTarget.get(level, pos)` for the block there (`is_sign` is a block entity that is a
    /// sign, the case Create's "modded signs are common" covers).
    pub fn of_block(block: &str, is_sign: bool) -> Option<Target> {
        if block == "create:display_board" {
            Some(Target::DisplayBoard)
        } else if super::nixie_tube::dye_of_block(block).is_some() {
            Some(Target::NixieTube)
        } else if block == "minecraft:lectern" {
            Some(Target::Lectern)
        } else if is_sign || is_sign_block(block) {
            Some(Target::Sign)
        } else {
            None
        }
    }

    /// The `getLineOptionText` of line `line` (counted from 0).
    pub fn line_text(self, line: usize) -> String {
        match self {
            Target::NixieTube => "Single Line".to_owned(),
            Target::Lectern => format!("Page {}", line + 1),
            _ => format!("Line {}", line + 1),
        }
    }
}

/// A standing, wall or hanging sign block (`SignBlockEntity`'s blocks).
pub fn is_sign_block(block: &str) -> bool {
    block.starts_with("minecraft:")
        && (block.ends_with("_sign") || block.ends_with("_wall_sign") || block.ends_with("_hanging_sign"))
}

/// `DisplayTargetStats`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stats {
    pub rows: usize,
    pub columns: usize,
}

impl Stats {
    pub const SIGN: Stats = Stats { rows: 4, columns: 15 };
    pub const LECTERN: Stats = Stats { rows: 50, columns: 256 };
}

// --- configuration -------------------------------------------------------------------------------

/// The source's saved configuration (`DisplayLinkBlockEntity.sourceConfig`): the label, the one
/// option and the stopwatch's start.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Config {
    pub label: String,
    pub option: i32,
    pub start_time: Option<i64>,
}

impl Config {
    /// The next value of the source's option (the stand-in for its scroll input).
    pub fn step_option(&mut self, source: Source) -> bool {
        match source.option() {
            Some((_, values)) => {
                self.option = (self.option + 1).rem_euclid(values.len() as i32);
                true
            }
            None => false,
        }
    }

    /// `StopWatchDisplaySource.onSignalReset` and the like: forget what the source accumulated.
    pub fn reset(&mut self, source: Source, now: i64) {
        if source == Source::Stopwatch {
            self.start_time = Some(now);
        }
    }
}

// --- what the glue read --------------------------------------------------------------------------

/// What the glue read at the source position, for each kind of source.
#[derive(Clone, Debug, PartialEq)]
pub enum Reading {
    /// The block entity the source needs isn't there.
    Missing,
    Speed(f32),
    Stress { stress: f32, capacity: f32 },
    /// A cuckoo clock: whether it turns, the world's time of day, whether the dimension has one
    /// (`dimensionType().natural()`), and two random numbers below 70 and 40 for one that doesn't.
    Clock { running: bool, day_time: i64, natural: bool, random: (i32, i32) },
    Stopwatch { running: bool, game_time: i64 },
    /// A sum: items counted, or fluid in millibuckets.
    Total(i64),
    /// Names and amounts, one entry for each different name.
    List(Vec<(String, i64)>),
    Level { current: i32, min: i32, max: i32 },
    Names(Vec<String>),
    Name(Option<String>),
    Power(i32),
    Text(String),
}

/// What a source makes of a reading, for a line of text. `None` is Create's `EMPTY_LINE`.
type Line = Option<String>;

/// `CreateLang.number`: up to two decimals, grouped by thousands (`LangNumberFormat`).
pub fn format_number(value: f64) -> String {
    let rounded = (value * 100.0).round_ties_even() / 100.0;
    let negative = rounded < 0.0;
    let text = format!("{:.2}", rounded.abs());
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, "00"));
    let mut grouped = String::new();
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let fraction = fraction.trim_end_matches('0');
    let mut out = if negative && (rounded != 0.0) { "-".to_owned() } else { String::new() };
    out.push_str(&grouped);
    if !fraction.is_empty() {
        out.push('.');
        out.push_str(fraction);
    }
    out
}

/// `FluidFormatter.asComponents`: the number and the unit.
pub fn fluid_parts(amount: i64, shorten: bool) -> (String, &'static str) {
    if shorten && amount >= 1000 {
        (format!("{:.1}", amount as f64 / 1000.0), "B")
    } else {
        (amount.to_string(), "mB")
    }
}

/// `FluidFormatter.asString`.
pub fn fluid_string(amount: i64, shorten: bool) -> String {
    let (number, unit) = fluid_parts(amount, shorten);
    format!("{number} {unit}")
}

/// `TimeOfDayDisplaySource.provideLine`.
pub fn time_text(day_time: i64, twelve_hour: bool, natural: bool, random: (i32, i32)) -> String {
    let day = day_time.rem_euclid(24000) as i32;
    let mut hours = (day / 1000 + 6) % 24;
    let mut minutes = (day % 1000) * 60 / 1000;
    let suffix = if hours > 11 { "pm" } else { "am" };
    minutes = minutes / 5 * 5;
    if twelve_hour {
        hours %= 12;
        if hours == 0 {
            hours = 12;
        }
    }
    if !natural {
        hours = random.0 + 24;
        minutes = random.1 + 60;
    }
    let mut text = format!(
        "{}{}:{}{}{}",
        if hours < 10 { " " } else { "" },
        hours,
        if minutes < 10 { "0" } else { "" },
        minutes,
        if twelve_hour { " " } else { "" },
    );
    if twelve_hour {
        text.push_str(suffix);
    }
    text
}

/// `StopWatchDisplaySource.provideLine`: the time since `start`, `now` and `start` in game ticks.
pub fn stopwatch_text(start: i64, now: i64) -> String {
    let diff = (now - start) as i32;
    let hours = diff / 60 / 60 / 20;
    let minutes = (diff / 60 / 20) % 60;
    let seconds = (diff / 20) % 60;
    let mut text = String::new();
    if hours != 0 {
        text.push_str(&format!("{}{}:", if hours < 10 { " " } else { "" }, hours));
    }
    if minutes < 10 {
        text.push_str(if hours == 0 { " " } else { "0" });
    }
    text.push_str(&format!("{minutes}:{}{seconds}", if seconds < 10 { "0" } else { "" }));
    text
}

/// What a progress bar or a percentage reads from, as `PercentOrProgressBarDisplaySource` takes it.
struct Progress {
    value: Option<f32>,
    bar: bool,
}

/// `PercentOrProgressBarDisplaySource.provideLine`.
fn percent_or_bar(
    progress: &Progress,
    label: &str,
    stats: &Stats,
    target: Target,
    numeric: &dyn Fn(f32) -> String,
) -> Line {
    let raw = progress.value?;
    if !progress.bar {
        return Some(numeric(raw));
    }
    let label_size = if label.is_empty() { 0 } else { label.chars().count() + 1 };
    let mut length = stats.columns.saturating_sub(label_size).min(128);
    if target == Target::Sign {
        length = (length as f32 * 6.0 / 9.0) as usize;
    }
    if target == Target::DisplayBoard {
        length = (length as f32 * MONOSPACE / WIDE_MONOSPACE) as usize;
    }
    let level = raw.clamp(0.0, 1.0);
    let filled = (level * length as f32) as usize;
    if length < 1 {
        return None;
    }
    Some("\u{2588}".repeat(filled) + &"\u{2592}".repeat(length - filled))
}

/// The progress a percentage or bar source reads, and whether it shows a bar.
fn progress_of(source: Source, config: &Config, reading: &Reading) -> Progress {
    match (source, reading) {
        (Source::KineticStress, Reading::Stress { stress, capacity }) => {
            let value = if *capacity == 0.0 {
                0.0
            } else {
                match config.option {
                    0 | 1 => stress / capacity,
                    2 => *stress,
                    3 => *capacity,
                    4 => capacity - stress,
                    _ => 0.0,
                }
            };
            Progress { value: Some(value), bar: config.option == 0 }
        }
        (Source::KineticStress, _) => Progress { value: None, bar: config.option == 0 },
        (Source::FillLevel, Reading::Level { current, min, max }) => {
            let span = (max - min) as f32;
            let level = if span > 0.0 { ((current - min) as f32 / span).max(0.0) } else { 0.0 };
            Progress { value: Some(level), bar: config.option != 0 }
        }
        (Source::FillLevel, _) => Progress { value: None, bar: config.option != 0 },
        (Source::RedstonePower, Reading::Power(power)) => {
            Progress { value: Some(*power as f32 / 15.0), bar: config.option != 0 }
        }
        (_, _) => Progress { value: None, bar: config.option != 0 },
    }
}

/// `provideLine` of the single-line sources.
fn line_of(source: Source, config: &mut Config, reading: &Reading, stats: &Stats, target: Target) -> Line {
    match source {
        Source::KineticSpeed => Some(match reading {
            Reading::Speed(rpm) => {
                let speed = if config.option == 0 { rpm.abs() } else { *rpm };
                format!("{} RPM", format_number(f64::from(speed)))
            }
            _ => "0".to_owned(),
        }),
        Source::KineticStress | Source::FillLevel | Source::RedstonePower => {
            let progress = progress_of(source, config, reading);
            let flap = target == Target::DisplayBoard;
            let mode = config.option;
            let numeric = move |level: f32| match source {
                Source::KineticStress if mode != 1 => {
                    format!("{}{}su", format_number(f64::from(level)), if flap { " " } else { "" })
                }
                Source::RedstonePower => ((level * 15.0) as i32).to_string(),
                _ => format!("{}%", ((level * 100.0) as i32).clamp(0, 100)),
            };
            percent_or_bar(&progress, &config.label, stats, target, &numeric)
        }
        Source::TimeOfDay => Some(match reading {
            Reading::Clock { running: true, day_time, natural, random } => {
                time_text(*day_time, config.option == 0, *natural, *random)
            }
            _ => "--:--".to_owned(),
        }),
        Source::Stopwatch => Some(match reading {
            Reading::Stopwatch { running: true, game_time } => {
                let start = *config.start_time.get_or_insert(*game_time);
                stopwatch_text(start, *game_time)
            }
            _ => "--:--".to_owned(),
        }),
        Source::CountItems => Some(match reading {
            Reading::Total(count) => count.to_string(),
            _ => "0".to_owned(),
        }),
        Source::CountFluids => match reading {
            Reading::Total(amount) => Some(fluid_string(*amount, false)),
            _ => None,
        },
        Source::ItemNames => match reading {
            Reading::Names(names) if !names.is_empty() => Some(names.concat()),
            _ => None,
        },
        Source::EntityName => match reading {
            Reading::Name(Some(name)) => Some(name.clone()),
            _ => None,
        },
        Source::NixieTube => match reading {
            Reading::Text(text) => Some(text.clone()),
            _ => None,
        },
        Source::ListItems | Source::ListFluids | Source::GaugeStatus => None,
    }
}

/// `SingleLineDisplaySource.provideText`'s label: `"label " + line`.
fn labelled(source: Source, config: &Config, target: Target, line: String) -> String {
    if source.allows_labeling(target) && !config.label.is_empty() {
        format!("{} {line}", config.label)
    } else {
        line
    }
}

/// The entries of a list source, largest first, `limit` of them (`CountedItemStackList.getTopNames`
/// and `FluidListDisplaySource` sort by amount, biggest first; equal ones keep their order).
pub fn top_entries(mut entries: Vec<(String, i64)>, limit: usize) -> Vec<(String, i64)> {
    entries.sort_by(|a, b| b.1.cmp(&a.1));
    entries.truncate(limit);
    entries
}

/// The rows a list source writes: the biggest first for the item and fluid lists, but the gauge status keeps its panels' order
/// (`FactoryGaugeDisplaySource.provideEntries` has its `sorted` commented out), `limit` of them.
fn listed(source: Source, entries: Vec<(String, i64)>, limit: usize) -> Vec<(String, i64)> {
    if source == Source::GaugeStatus {
        entries.into_iter().take(limit).collect()
    } else {
        top_entries(entries, limit)
    }
}

/// `ValueListDisplaySource.shorten`: the number and the suffix a long number is written with.
fn shortened(number: i64) -> (String, &'static str) {
    if number >= 1_000_000 {
        ((number / 1_000_000).to_string(), "m ")
    } else if number >= 1000 {
        ((number / 1000).to_string(), "k ")
    } else {
        (number.to_string(), " ")
    }
}

/// `createComponentsFromEntry`: the parts of one entry's row.
pub fn entry_parts(source: Source, config: &Config, entry: &(String, i64)) -> Vec<String> {
    let shorten = config.option == 0;
    let name = format!("{} ", entry.0);
    if source == Source::ListFluids {
        let (number, unit) = fluid_parts(entry.1, shorten);
        return vec![number, unit.to_owned(), name];
    }
    let value_first = matches!(source, Source::ListItems | Source::GaugeStatus);
    if shorten {
        let (number, suffix) = shortened(entry.1);
        if value_first {
            vec![number, suffix.to_owned(), name]
        } else {
            vec![name, number, suffix.to_owned()]
        }
    } else {
        let number = format!("{} ", entry.1);
        if value_first { vec![number, name] } else { vec![name, number] }
    }
}

/// `ValueListDisplaySource.provideText`: a row of text for each entry, or for a lectern a page of
/// eight entries joined by new lines.
fn list_text(source: Source, config: &Config, reading: &Reading, stats: &Stats, target: Target) -> Vec<String> {
    let Reading::List(entries) = reading else {
        return Vec::new();
    };
    let book = target == Target::Lectern;
    let limit = stats.rows * if book { ENTRIES_PER_PAGE } else { 1 };
    let rows: Vec<String> = listed(source, entries.clone(), limit)
        .iter()
        .map(|entry| entry_parts(source, config, entry).concat())
        .collect();
    if !book {
        return rows;
    }
    rows.chunks(ENTRIES_PER_PAGE).map(|page| page.join("\n")).collect()
}

/// `DisplaySource.provideText`: the lines a source writes to a target. An empty answer is
/// Create's `EMPTY` (one empty line), as `transferData` makes it.
pub fn provide_text(
    source: Source,
    config: &mut Config,
    reading: &Reading,
    stats: &Stats,
    target: Target,
) -> Vec<String> {
    let text = if source.is_single_line() {
        match line_of(source, config, reading, stats, target) {
            Some(line) => vec![labelled(source, config, target, line)],
            None => Vec::new(),
        }
    } else {
        list_text(source, config, reading, stats, target)
    };
    if text.is_empty() { vec![String::new()] } else { text }
}

/// `provideFlapDisplayText`: the rows of a board, each the texts of its line's sections.
pub fn provide_flap_text(source: Source, config: &mut Config, reading: &Reading, stats: &Stats) -> Vec<Vec<String>> {
    if !source.is_single_line() {
        let Reading::List(entries) = reading else {
            return Vec::new();
        };
        return listed(source, entries.clone(), stats.rows)
            .iter()
            .map(|entry| entry_parts(source, config, entry))
            .collect();
    }
    let line = line_of(source, config, reading, stats, Target::DisplayBoard);
    if source.allows_labeling(Target::DisplayBoard) && !config.label.is_empty() {
        return vec![vec![format!("{} ", config.label), line.unwrap_or_default()]];
    }
    vec![vec![line.unwrap_or_default()]]
}

// --- the flap display layouts ----------------------------------------------------------------------

/// `NumericSingleLineDisplaySource` and friends: the section a value goes in.
fn value_section(shape: Shape, bar: bool, numeric_text: bool, size: i32) -> Section {
    let width = size.max(0) as f32 * MONOSPACE;
    match shape {
        Shape::Numeric => Section::new(width, "numeric", false, false),
        Shape::Percent if bar => Section::new(width, "pixel", false, false).wide_flaps(),
        Shape::Percent => Section::new(width, "numeric", false, false),
        Shape::Instant => Section::new(width, "instant", false, false),
        Shape::Nixie if numeric_text => Section::new(width, "numeric", false, false),
        _ => Section::new(width, "alphabet", false, false),
    }
}

/// `loadFlapDisplayLayout` of a source: configures `layout` for the row `row` it is about to get,
/// for a board that holds `max_chars(gaps)` characters a line. `reading` is what the rows are
/// made from (a list source sizes its columns by the largest value).
pub fn load_flap_layout(
    source: Source,
    config: &Config,
    reading: &Reading,
    layout: &mut Layout,
    max_chars: &dyn Fn(i32) -> i32,
) {
    let shape = source.shape();
    let bar = progress_of(source, config, reading).bar;
    let numeric_text = matches!(reading, Reading::Text(t) if t.parse::<i32>().is_ok());
    match shape {
        Shape::ValueList => {
            let value_first = true;
            let shorten = config.option == 0;
            let highest = match reading {
                Reading::List(entries) => entries.iter().map(|e| e.1).max().unwrap_or(0),
                _ => 0,
            };
            let digits = if highest > 0 { 1 + (highest as f64).log10() as i32 } else { 0 };
            let value_format = if shorten { 0 } else { 4.max(digits) };
            let key = format!("ValueList_{value_first}_{value_format}");
            if layout.is_layout(&key) {
                return;
            }
            let max = max_chars(1);
            let number_length = max.min(3.max(value_format));
            let name_length = (max - number_length - i32::from(shorten)).max(0);
            let name = Section::new(MONOSPACE * name_length as f32, "alphabet", false, !value_first);
            let value = Section::new(MONOSPACE * number_length as f32, "number", false, !shorten && value_first)
                .right_aligned();
            if shorten {
                let suffix = Section::new(MONOSPACE, "shortened_numbers", false, value_first);
                layout.configure(&key, vec![value, suffix, name]);
            } else {
                layout.configure(&key, vec![value, name]);
            }
        }
        Shape::FluidList => {
            let shorten = config.option == 0;
            let highest = match reading {
                Reading::List(entries) => entries.iter().map(|e| e.1).max().unwrap_or(0),
                _ => 0,
            };
            let length = fluid_string(highest, shorten).chars().count() as i32;
            let key = format!("FluidList_{length}");
            if layout.is_layout(&key) {
                return;
            }
            let max = max_chars(1);
            let number_length = max.min(3.max(length - 2));
            let name_length = (max - number_length - 2).max(0);
            let value = Section::new(MONOSPACE * number_length as f32, "number", false, false).right_aligned();
            let unit = Section::new(MONOSPACE * 2.0, "fluid_units", true, true);
            let name = Section::new(MONOSPACE * name_length as f32, "alphabet", false, false);
            layout.configure(&key, vec![value, unit, name]);
        }
        _ => {
            let key = match shape {
                Shape::Numeric | Shape::Names => "Number",
                Shape::Percent if bar => "Progress",
                Shape::Percent => "Number",
                Shape::Instant => "Instant",
                Shape::Nixie if numeric_text => "Number",
                _ => "Default",
            };
            let label = if source.allows_labeling(Target::DisplayBoard) { config.label.as_str() } else { "" };
            let max = max_chars(0);
            if label.is_empty() {
                if !layout.is_layout(key) {
                    layout.configure(key, vec![value_section(shape, bar, numeric_text, max)]);
                }
                return;
            }
            let length = label.chars().count() as i32;
            let name = format!("{length}_Labeled_{key}");
            if layout.is_layout(&name) {
                return;
            }
            let label_section = Section::new((max.min(length + 1)) as f32 * MONOSPACE, "alphabet", false, false);
            if length + 1 < max {
                layout.configure(&name, vec![label_section, value_section(shape, bar, numeric_text, max - length - 1)]);
            } else {
                layout.configure(&name, vec![label_section]);
            }
        }
    }
}

// --- writing to the targets ----------------------------------------------------------------------

/// `SignDisplayTarget.acceptText`: the lines to write to a sign, as (line index, text). `reserve`
/// is called for the first, and `reserved` asked for the others: a line another link wrote stops
/// the run.
pub fn sign_writes(
    text: &[String],
    line: usize,
    mut reserve: impl FnMut(usize),
    mut reserved: impl FnMut(usize) -> bool,
) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (i, row) in text.iter().enumerate() {
        if i + line >= 4 {
            break;
        }
        if i == 0 {
            reserve(i + line);
        }
        if i > 0 && reserved(i + line) {
            break;
        }
        // `getString(sign.getMaxTextLineWidth())`: at most 90 characters.
        out.push((i + line, row.chars().take(90).collect()));
    }
    out
}

/// `LecternDisplayTarget.acceptText`: the pages of a book after the text is written from page
/// `line`. `pages` are the book's pages as they are; the result is `None` when nothing changed.
pub fn lectern_pages(
    pages: &[String],
    text: &[String],
    line: usize,
    mut reserve: impl FnMut(usize),
    mut reserved: impl FnMut(usize) -> bool,
) -> Option<Vec<String>> {
    let mut pages = pages.to_vec();
    let mut changed = false;
    let mut i = 0;
    while i < 50 && (i as i64 - line as i64) < text.len() as i64 {
        if pages.len() <= i {
            pages.push(if i < line { String::new() } else { text[i - line].clone() });
        } else if i >= line {
            if i - line == 0 {
                reserve(i);
            }
            if i - line > 0 && reserved(i - line) {
                break;
            }
            pages[i] = text[i - line].clone();
        }
        changed = true;
        i += 1;
    }
    changed.then_some(pages)
}

/// `LinkBulbRenderer`: the brightness (0 to 200) the bulb glows with `ticks` game ticks after the
/// link wrote, `None` once it is out. The glow is set to 2 by the pulse and eases to 0 by half each
/// tick (`LerpedFloat.chase(0, 0.5, EXP)`), the frame between two ticks lerped.
pub fn bulb_glow(ticks: f32) -> Option<u8> {
    if ticks < 0.0 {
        return None;
    }
    let k = ticks.floor();
    let at = |n: f32| 2.0 * 0.5f32.powf(n);
    let glow = at(k) + (at(k + 1.0) - at(k)) * (ticks - k);
    if glow < 0.125 {
        return None;
    }
    let shaped = (1.0 - 2.0 * (glow - 0.75).powi(2)).clamp(-1.0, 1.0);
    Some((200.0 * shaped).clamp(0.0, 255.0) as u8)
}

/// `ClickToLinkBlockItem`'s distance rule: the selected block must be within `RANGE` of where the
/// link is placed (`closerThan`, a Euclidean distance) and in the same dimension.
pub fn within_range(selected: [i32; 3], placed: [i32; 3], same_dimension: bool) -> bool {
    let d: [f64; 3] = std::array::from_fn(|i| f64::from(selected[i] - placed[i]));
    same_dimension && d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < f64::from(RANGE) * f64::from(RANGE)
}

/// Where a ray from `eye` along `look` meets the face `face` of the block at `pos`
/// (`BlockHitResult.getLocation`), for the clicks that need to know where on a face they were.
pub fn hit_on_face(eye: [f64; 3], look: [f64; 3], pos: [i32; 3], face: [i32; 3]) -> [f64; 3] {
    let axis = (0..3).find(|&a| face[a] != 0).unwrap_or(1);
    let plane = f64::from(pos[axis]) + if face[axis] > 0 { 1.0 } else { 0.0 };
    let t = if look[axis].abs() < 1e-9 { 0.0 } else { (plane - eye[axis]) / look[axis] };
    let mut hit = [eye[0] + look[0] * t, eye[1] + look[1] * t, eye[2] + look[2] * t];
    hit[axis] = plane;
    hit
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(columns: usize) -> Stats {
        Stats { rows: 4, columns }
    }

    fn text(source: Source, config: &mut Config, reading: Reading, target: Target) -> Vec<String> {
        let stats = match target {
            Target::Sign => Stats::SIGN,
            Target::Lectern => Stats::LECTERN,
            _ => stats(20),
        };
        provide_text(source, config, &reading, &stats, target)
    }

    #[test]
    fn numbers_group_and_round_like_the_game() {
        assert_eq!(format_number(16.0), "16");
        assert_eq!(format_number(-32.0), "-32");
        assert_eq!(format_number(12.5), "12.5");
        assert_eq!(format_number(1234567.891), "1,234,567.89");
        assert_eq!(format_number(0.004), "0");
        assert_eq!(format_number(-0.004), "0");
        assert_eq!(format_number(100.0), "100");
    }

    #[test]
    fn the_clock_reads_hours_and_the_stopwatch_minutes() {
        // Tick 0 is 6 o'clock.
        assert_eq!(time_text(0, false, true, (0, 0)), " 6:00");
        assert_eq!(time_text(0, true, true, (0, 0)), " 6:00 am");
        assert_eq!(time_text(6000, true, true, (0, 0)), "12:00 pm");
        assert_eq!(time_text(18000, false, true, (0, 0)), " 0:00");
        assert_eq!(time_text(13000 + 24000, true, true, (0, 0)), " 7:00 pm");
        // Minutes go in fives: 100 ticks in an hour step is 6 minutes, shown as 5.
        assert_eq!(time_text(100, false, true, (0, 0)), " 6:05");
        // A dimension without a day shows nonsense from the random numbers.
        assert_eq!(time_text(0, false, false, (10, 5)), "34:65");
        assert_eq!(stopwatch_text(0, 20 * 5), " 0:05");
        assert_eq!(stopwatch_text(0, 20 * 65), " 1:05");
        assert_eq!(stopwatch_text(0, 20 * 60 * 60 * 2 + 20 * 60 * 3 + 20 * 9), " 2:03:09");
    }

    #[test]
    fn a_speedometer_reads_rpm_with_or_without_the_direction() {
        let mut cfg = Config::default();
        let line = |cfg: &mut Config, rpm| text(Source::KineticSpeed, cfg, Reading::Speed(rpm), Target::Sign);
        assert_eq!(line(&mut cfg, -32.0), vec!["32 RPM"]);
        cfg.option = 1;
        assert_eq!(line(&mut cfg, -32.0), vec!["-32 RPM"]);
        assert_eq!(text(Source::KineticSpeed, &mut cfg, Reading::Missing, Target::Sign), vec!["0"]);
        cfg.label = "Motor".to_owned();
        assert_eq!(line(&mut cfg, 8.0), vec!["Motor 8 RPM"]);
    }

    #[test]
    fn a_stressometer_has_five_displays() {
        let reading = Reading::Stress { stress: 4096.0, capacity: 16384.0 };
        let mut cfg = Config::default();
        let board = |cfg: &mut Config| text(Source::KineticStress, cfg, reading.clone(), Target::DisplayBoard);
        // The bar: 20 columns, 25 percent full, in the board's wide flaps (7/9 of the length).
        let bar = &board(&mut cfg)[0];
        assert_eq!(bar.chars().count(), 15);
        assert_eq!(bar.chars().filter(|&c| c == '\u{2588}').count(), 3);
        cfg.option = 1;
        assert_eq!(board(&mut cfg), vec!["25%"]);
        cfg.option = 2;
        assert_eq!(board(&mut cfg), vec!["4,096 su"]);
        assert_eq!(text(Source::KineticStress, &mut cfg, reading.clone(), Target::Sign), vec!["4,096su"]);
        cfg.option = 3;
        assert_eq!(board(&mut cfg), vec!["16,384 su"]);
        cfg.option = 4;
        assert_eq!(board(&mut cfg), vec!["12,288 su"]);
        // No capacity reads as nothing used, and no gauge as an empty line.
        cfg.option = 2;
        let none = Reading::Stress { stress: 10.0, capacity: 0.0 };
        assert_eq!(text(Source::KineticStress, &mut cfg, none, Target::Sign), vec!["0su"]);
        assert_eq!(text(Source::KineticStress, &mut cfg, Reading::Missing, Target::Sign), vec![""]);
    }

    #[test]
    fn a_bar_is_shorter_on_a_sign_and_leaves_room_for_the_label() {
        let mut cfg = Config { option: 1, ..Config::default() };
        let power = Reading::Power(15);
        let bar = text(Source::RedstonePower, &mut cfg, power.clone(), Target::Sign);
        // 15 columns * 6/9 = 10.
        assert_eq!(bar[0], "\u{2588}".repeat(10));
        cfg.label = "ab".to_owned();
        let labelled = text(Source::RedstonePower, &mut cfg, Reading::Power(0), Target::Sign);
        // 12 columns left, 8 of them as a sign shows; the label stands in front.
        assert_eq!(labelled[0], format!("ab {}", "\u{2592}".repeat(8)));
        cfg.option = 0;
        assert_eq!(text(Source::RedstonePower, &mut cfg, Reading::Power(9), Target::Sign), vec!["ab 9"]);
    }

    #[test]
    fn a_threshold_switch_reads_as_percent_or_bar() {
        let mut cfg = Config::default();
        let level = Reading::Level { current: 30, min: 10, max: 50 };
        assert_eq!(text(Source::FillLevel, &mut cfg, level.clone(), Target::Sign), vec!["50%"]);
        let below = Reading::Level { current: 5, min: 10, max: 50 };
        assert_eq!(text(Source::FillLevel, &mut cfg, below, Target::Sign), vec!["0%"]);
    }

    #[test]
    fn item_and_fluid_counts_and_names() {
        let mut cfg = Config::default();
        assert_eq!(text(Source::CountItems, &mut cfg, Reading::Total(37), Target::Sign), vec!["37"]);
        assert_eq!(text(Source::CountItems, &mut cfg, Reading::Missing, Target::Sign), vec!["0"]);
        assert_eq!(text(Source::CountFluids, &mut cfg, Reading::Total(1500), Target::Sign), vec!["1500 mB"]);
        assert_eq!(text(Source::CountFluids, &mut cfg, Reading::Missing, Target::Sign), vec![""]);
        let names = Reading::Names(vec!["Iron Ingot".into(), "Cobblestone".into()]);
        assert_eq!(text(Source::ItemNames, &mut cfg, names, Target::Sign), vec!["Iron IngotCobblestone"]);
        assert_eq!(text(Source::EntityName, &mut cfg, Reading::Name(Some("Steve".into())), Target::Sign), vec!["Steve"]);
        assert_eq!(text(Source::EntityName, &mut cfg, Reading::Name(None), Target::Sign), vec![""]);
    }

    #[test]
    fn a_list_is_sorted_cut_and_shortened() {
        let list = Reading::List(vec![
            ("Dirt".into(), 12),
            ("Iron".into(), 2500),
            ("Gold".into(), 2_000_000),
            ("Coal".into(), 1),
            ("Sand".into(), 5),
        ]);
        let mut cfg = Config::default();
        // Four rows on a sign, biggest first, numbers shortened, value before the name.
        let rows = text(Source::ListItems, &mut cfg, list.clone(), Target::Sign);
        assert_eq!(rows, vec!["2m Gold ", "2k Iron ", "12 Dirt ", "5 Sand "]);
        cfg.option = 1;
        let full = text(Source::ListItems, &mut cfg, list.clone(), Target::Sign);
        assert_eq!(full[1], "2500 Iron ");
        // A lectern gathers eight entries to a page.
        let many = Reading::List((0..20).map(|i| (format!("I{i}"), 100 - i)).collect());
        let pages = text(Source::ListItems, &mut cfg, many, Target::Lectern);
        assert_eq!(pages.len(), 3);
        assert_eq!(pages[0].lines().count(), 8);
        assert_eq!(pages[2].lines().count(), 4);
        // A fluid list writes the number, the unit and the name with no space, as Create does.
        let fluids = Reading::List(vec![("Water".into(), 1500)]);
        cfg.option = 0;
        assert_eq!(text(Source::ListFluids, &mut cfg, fluids.clone(), Target::Sign), vec!["1.5BWater "]);
        cfg.option = 1;
        assert_eq!(text(Source::ListFluids, &mut cfg, fluids, Target::Sign), vec!["1500mBWater "]);
    }

    #[test]
    fn gauge_status_keeps_the_panels_order_and_is_offered_by_a_gauge() {
        // `FactoryGaugeDisplaySource`: a `ValueListDisplaySource` with the value first and no sorting, found on the gauge block.
        assert_eq!(sources_for("create:factory_gauge"), vec![Source::GaugeStatus]);
        assert!(!Source::GaugeStatus.is_single_line());
        let list = Reading::List(vec![
            ("\u{2714} Iron".into(), 20),
            ("\u{25aa} Gold".into(), 3000),
            ("  Coal".into(), 5),
            ("\u{2191} Sand".into(), 8),
            ("\u{2714} Dirt".into(), 9),
        ]);
        let mut cfg = Config::default();
        let rows = text(Source::GaugeStatus, &mut cfg, list.clone(), Target::Sign);
        assert_eq!(rows, vec!["20 \u{2714} Iron ", "3k \u{25aa} Gold ", "5   Coal ", "8 \u{2191} Sand "]);
        // A board gets the same rows as its flap sections.
        let flaps = provide_flap_text(Source::GaugeStatus, &mut cfg, &list, &Stats { rows: 2, columns: 12 });
        assert_eq!(flaps.len(), 2);
        assert_eq!(flaps[0].concat(), "20 \u{2714} Iron ");
    }

    #[test]
    fn a_link_with_no_text_writes_one_empty_line() {
        let mut cfg = Config::default();
        assert_eq!(text(Source::ListItems, &mut cfg, Reading::Missing, Target::Sign), vec![""]);
        assert_eq!(text(Source::NixieTube, &mut cfg, Reading::Missing, Target::Sign), vec![""]);
    }

    #[test]
    fn the_stopwatch_starts_when_first_read() {
        let mut cfg = Config::default();
        let at = |now| Reading::Stopwatch { running: true, game_time: now };
        assert_eq!(text(Source::Stopwatch, &mut cfg, at(1000), Target::Sign), vec![" 0:00"]);
        assert_eq!(cfg.start_time, Some(1000));
        assert_eq!(text(Source::Stopwatch, &mut cfg, at(1000 + 20 * 7), Target::Sign), vec![" 0:07"]);
        cfg.reset(Source::Stopwatch, 5000);
        assert_eq!(cfg.start_time, Some(5000));
        let stopped = Reading::Stopwatch { running: false, game_time: 0 };
        assert_eq!(text(Source::Stopwatch, &mut cfg, stopped, Target::Sign), vec!["--:--"]);
    }

    #[test]
    fn sources_and_targets_are_found_by_block() {
        assert_eq!(sources_for("create:speedometer"), vec![Source::KineticSpeed]);
        assert_eq!(
            sources_for("create:content_observer"),
            vec![Source::CountItems, Source::ListItems, Source::CountFluids, Source::ListFluids]
        );
        assert_eq!(sources_for("create:red_seat"), vec![Source::EntityName]);
        assert_eq!(sources_for("create:lime_nixie_tube"), vec![Source::NixieTube]);
        assert_eq!(sources_for("create:shaft"), Vec::<Source>::new());
        assert_eq!(Target::of_block("minecraft:oak_wall_sign", false), Some(Target::Sign));
        assert_eq!(Target::of_block("minecraft:oak_hanging_sign", false), Some(Target::Sign));
        assert_eq!(Target::of_block("minecraft:lectern", false), Some(Target::Lectern));
        assert_eq!(Target::of_block("create:display_board", false), Some(Target::DisplayBoard));
        assert_eq!(Target::of_block("create:nixie_tube", false), Some(Target::NixieTube));
        assert_eq!(Target::of_block("minecraft:stone", false), None);
        assert_eq!(Target::of_block("some:modded_sign", true), Some(Target::Sign));
        for source in Source::ALL {
            assert_eq!(Source::from_id(source.id()), Some(source));
        }
        // A nixie tube can't label itself into another nixie tube.
        assert!(!Source::NixieTube.allows_labeling(Target::NixieTube));
        assert!(Source::NixieTube.allows_labeling(Target::Sign));
    }

    #[test]
    fn options_wrap_around() {
        let mut cfg = Config::default();
        for expected in [1, 2, 3, 4, 0] {
            assert!(cfg.step_option(Source::KineticStress));
            assert_eq!(cfg.option, expected);
        }
        assert!(!cfg.step_option(Source::CountItems));
    }

    #[test]
    fn a_sign_takes_four_lines_and_stops_at_a_reserved_one() {
        let text: Vec<String> = ["a", "b", "c", "d", "e"].iter().map(|s| s.to_string()).collect();
        let mut reserved = Vec::new();
        let writes = sign_writes(&text, 1, |l| reserved.push(l), |_| false);
        assert_eq!(writes.len(), 3);
        assert_eq!(writes[0], (1, "a".to_owned()));
        assert_eq!(reserved, vec![1]);
        let stopped = sign_writes(&text, 0, |_| {}, |l| l == 2);
        assert_eq!(stopped.len(), 2);
        let long = sign_writes(&["x".repeat(200)], 0, |_| {}, |_| false);
        assert_eq!(long[0].1.len(), 90);
    }

    #[test]
    fn a_lectern_fills_its_pages() {
        let write = |pages: &[&str], text: &[&str], line| {
            let pages: Vec<String> = pages.iter().map(|s| s.to_string()).collect();
            let text: Vec<String> = text.iter().map(|s| s.to_string()).collect();
            lectern_pages(&pages, &text, line, |_| {}, |_| false)
        };
        assert_eq!(write(&[], &["one", "two"], 0), Some(vec!["one".into(), "two".into()]));
        assert_eq!(write(&["x"], &["one"], 2), Some(vec!["x".into(), "".into(), "one".into()]));
        assert_eq!(write(&["a", "b", "c"], &["new"], 1), Some(vec!["a".into(), "new".into(), "c".into()]));
        assert_eq!(write(&["a"], &[], 0), None);
    }

    #[test]
    fn the_bulb_flashes_and_goes_out() {
        let at = |t| bulb_glow(t);
        // The glow starts at 2, past the peak: dark at first, brightest as it eases through 0.75.
        assert_eq!(at(0.0), Some(0));
        assert!(at(1.0).is_some_and(|c| c > 150));
        assert!(at(2.0).is_some_and(|c| c > 100));
        assert_eq!(at(6.0), None);
        assert_eq!(at(-1.0), None);
    }

    #[test]
    fn the_target_must_be_in_range() {
        assert!(within_range([0, 0, 0], [10, 0, 0], true));
        assert!(!within_range([0, 0, 0], [64, 0, 0], true));
        assert!(!within_range([0, 0, 0], [1, 0, 0], false));
    }

    #[test]
    fn a_ray_meets_the_face_it_hits() {
        // Standing at the origin looking along +z at the north face of the block at z = 5.
        let hit = hit_on_face([0.5, 1.6, 0.5], [0.0, -0.1, 1.0], [0, 1, 5], [0, 0, -1]);
        assert_eq!(hit[2], 5.0);
        assert!((hit[1] - (1.6 - 0.45)).abs() < 1e-9);
    }

    #[test]
    fn layouts_follow_the_source_and_its_label() {
        let max: &dyn Fn(i32) -> i32 = &|gaps| 12 - gaps;
        let mut layout = Layout::new(12);
        let mut cfg = Config::default();
        load_flap_layout(Source::KineticSpeed, &cfg, &Reading::Speed(1.0), &mut layout, max);
        assert!(layout.is_layout("Number"));
        assert_eq!(layout.sections[0].cycle, "numeric");
        assert_eq!(layout.sections[0].flaps(), 12);
        cfg.label = "Speed".to_owned();
        load_flap_layout(Source::KineticSpeed, &cfg, &Reading::Speed(1.0), &mut layout, max);
        assert!(layout.is_layout("5_Labeled_Number"));
        assert_eq!(layout.sections.len(), 2);
        assert_eq!(layout.sections[0].flaps(), 6);
        assert_eq!(layout.sections[1].flaps(), 6);
        // A label that fills the line leaves no room for the value.
        cfg.label = "A very long label".to_owned();
        load_flap_layout(Source::KineticSpeed, &cfg, &Reading::Speed(1.0), &mut layout, max);
        assert_eq!(layout.sections.len(), 1);
        // A bar uses the wide pixel flaps.
        let bar = Config { option: 0, ..Config::default() };
        load_flap_layout(Source::KineticStress, &bar, &Reading::Stress { stress: 1.0, capacity: 2.0 }, &mut layout, max);
        assert!(layout.is_layout("Progress"));
        assert!(layout.sections[0].wide);
        // Lists size their number column by the largest number.
        let list = Reading::List(vec![("Iron".into(), 123_456)]);
        let full = Config { option: 1, ..Config::default() };
        load_flap_layout(Source::ListItems, &full, &list, &mut layout, max);
        assert!(layout.is_layout("ValueList_true_6"));
        assert_eq!(layout.sections.len(), 2);
        assert!(layout.sections[0].right_aligned);
        let short = Config { option: 0, ..Config::default() };
        load_flap_layout(Source::ListItems, &short, &list, &mut layout, max);
        assert!(layout.is_layout("ValueList_true_0"));
        assert_eq!(layout.sections.len(), 3);
        load_flap_layout(Source::ListFluids, &short, &Reading::List(vec![("Water".into(), 1500)]), &mut layout, max);
        assert!(layout.is_layout("FluidList_5"));
        assert_eq!(layout.sections[1].cycle, "fluid_units");
        assert!(layout.sections[1].single_flap);
    }

    #[test]
    fn rows_for_a_board_come_in_sections() {
        let mut cfg = Config { label: "Rpm".to_owned(), ..Config::default() };
        let rows = provide_flap_text(Source::KineticSpeed, &mut cfg, &Reading::Speed(16.0), &Stats { rows: 2, columns: 12 });
        assert_eq!(rows, vec![vec!["Rpm ".to_owned(), "16 RPM".to_owned()]]);
        cfg.label.clear();
        let plain = provide_flap_text(Source::KineticSpeed, &mut cfg, &Reading::Speed(16.0), &Stats { rows: 2, columns: 12 });
        assert_eq!(plain, vec![vec!["16 RPM".to_owned()]]);
        let list = Reading::List(vec![("Iron".into(), 5), ("Gold".into(), 9)]);
        let rows = provide_flap_text(Source::ListItems, &mut cfg, &list, &Stats { rows: 2, columns: 12 });
        assert_eq!(rows[0], vec!["9".to_owned(), " ".to_owned(), "Gold ".to_owned()]);
    }
}
