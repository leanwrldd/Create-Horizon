//! Editing a schedule by command, standing in for `ScheduleScreen` (content/trains/schedule/ScheduleScreen.java):
//! the edits the screen makes (add a stop, remove one, move it, add a condition to a stop or an alternative column,
//! switch the loop, retitle, limit the speed), what a new stop starts as, and a one-line summary of each stop
//! (`getSummary` of the instruction, `getTitleAs("condition")` of each condition, in English).
//!
//! A new stop is `ScheduleScreen`'s `addEntry`: a `DestinationInstruction` with one column holding a `ScheduledDelay`
//! (5 seconds). Conditions in one column all have to be met in turn; the columns are alternatives (the first to finish
//! ends the wait), so `when` adds to the last column of a stop and `or` starts a column.
//!
//! Not here: the screen's drag and drop, the instruction and condition editors' widgets, the suggestions, the icons.
//! The cargo conditions take an item id (a filter item isn't a thing a command can hold); the redstone link takes none.

use super::schedule::{
    Ops, Schedule, ScheduleEntry, ScheduleInstruction, Threshold, TimeOfDay, TimeUnit, TimedWait, WaitCondition,
};

/// What `/schedule` can do to the schedule in hand.
#[derive(Clone, Debug, PartialEq)]
pub enum ScheduleCommand {
    Show,
    /// `stop <station name, * wildcards>`: a new travel stop.
    Stop(String),
    /// `retrieve <address>`: a `create:package_retrieval` stop.
    Retrieve(String),
    /// `deliver`: a `create:package_delivery` stop.
    Deliver,
    /// `title <text>`: a `create:rename` stop.
    Title(String),
    /// `throttle <percent>`: a `create:throttle` stop.
    Throttle(i32),
    /// `remove <n>` (1-based).
    Remove(usize),
    /// `move <n> <up|down>`.
    Move { index: usize, up: bool },
    /// `when <n> <condition>`: the condition goes on the last column of the stop; `or <n> <condition>` begins a column.
    Condition { index: usize, alternative: bool, condition: WaitCondition },
    /// `unwait <n>`: the stop's last condition goes (`removeCondition`), an emptied column with it.
    DropCondition(usize),
    /// `loop [on|off]`: cyclic or not (`cyclicButton`).
    Cyclic(Option<bool>),
    /// `reset`: the saved progress back to the first stop.
    Reset,
    Clear,
}

/// The default of `ScheduledDelay`, as `new ScheduledDelay()` makes it.
fn default_delay() -> WaitCondition {
    WaitCondition::Delay(TimedWait::default())
}

/// `ScheduleScreen.addEntry`'s result for a destination.
pub fn new_stop(instruction: ScheduleInstruction) -> ScheduleEntry {
    let conditions = if instruction.supports_conditions() { vec![vec![default_delay()]] } else { Vec::new() };
    ScheduleEntry { instruction, conditions }
}

fn number(token: Option<&&str>, what: &str) -> Result<i32, String> {
    token.ok_or_else(|| format!("Missing {what}"))?.parse::<i32>().map_err(|_| format!("{what} must be a whole number"))
}

fn index(token: Option<&&str>) -> Result<usize, String> {
    let n = number(token, "the stop number")?;
    if n < 1 {
        return Err("Stops count from 1".into());
    }
    Ok(n as usize)
}

fn unit(token: Option<&&str>) -> Result<i32, String> {
    Ok(match token.map(|t| t.to_ascii_lowercase()).as_deref() {
        None | Some("seconds") | Some("second") | Some("s") => TimeUnit::Seconds as i32,
        Some("ticks") | Some("tick") | Some("t") => TimeUnit::Ticks as i32,
        Some("minutes") | Some("minute") | Some("m") => TimeUnit::Minutes as i32,
        Some(other) => return Err(format!("'{other}' is not a time unit (ticks, seconds, minutes)")),
    })
}

fn operator(token: Option<&&str>) -> Result<i32, String> {
    Ok(match token.map(|t| t.to_ascii_lowercase()).as_deref() {
        Some("less") | Some("<") => Ops::Less as i32,
        Some("more") | Some("greater") | Some(">") => Ops::Greater as i32,
        Some("exactly") | Some("equal") | Some("=") => Ops::Equal as i32,
        _ => return Err("Use less, more or exactly".into()),
    })
}

/// A condition from the words after `when <n>` / `or <n>`.
fn condition(words: &[&str]) -> Result<WaitCondition, String> {
    let kind = words.first().map(|w| w.to_ascii_lowercase()).ok_or("Name a condition: delay, idle, time, powered, unloaded, players, items, fluid")?;
    let rest = &words[1..];
    Ok(match kind.as_str() {
        "delay" | "wait" => WaitCondition::Delay(TimedWait { value: number(rest.first(), "the time")?, unit: unit(rest.get(1))? }),
        "idle" => WaitCondition::Idle(TimedWait { value: number(rest.first(), "the time")?, unit: unit(rest.get(1))? }),
        "time" => {
            let hour = number(rest.first(), "the hour")?;
            let minute = number(rest.get(1), "the minute")?;
            if !(0..24).contains(&hour) || !(0..60).contains(&minute) {
                return Err("The time is 0-23 hours and 0-59 minutes".into());
            }
            let rotation = match rest.get(2) {
                Some(t) => t.parse::<i32>().map_err(|_| "The rotation is a number from 0 to 9".to_string())?,
                None => TimeOfDay::default().rotation,
            };
            if !(0..=9).contains(&rotation) {
                return Err("The rotation is a number from 0 to 9".into());
            }
            WaitCondition::TimeOfDay(TimeOfDay { hour, minute, rotation })
        }
        "powered" => WaitCondition::Powered,
        "unloaded" => WaitCondition::Unloaded,
        "players" => {
            let count = number(rest.first(), "the number of players")?;
            let exact = i32::from(rest.get(1).is_some_and(|w| w.eq_ignore_ascii_case("exact")));
            WaitCondition::PlayerCount { count, exact }
        }
        "items" | "fluid" => {
            let operator = operator(rest.first())?;
            let amount = number(rest.get(1), "the amount")?;
            if amount < 0 {
                return Err("The amount can't be negative".into());
            }
            let mut measure = 0;
            let mut filter = None;
            for word in &rest[2..] {
                match word.to_ascii_lowercase().as_str() {
                    "stacks" | "buckets" => measure = 1,
                    "items" | "mb" => measure = 0,
                    _ => filter = Some((*word).to_owned()),
                }
            }
            let threshold = Threshold { threshold: amount.to_string(), operator, measure, filter };
            if kind == "items" { WaitCondition::ItemThreshold(threshold) } else { WaitCondition::FluidThreshold(threshold) }
        }
        other => return Err(format!("'{other}' is not a condition (delay, idle, time, powered, unloaded, players, items, fluid)")),
    })
}

/// Reads one `/schedule` line (what comes after the command's name).
pub fn parse(line: &str) -> Result<ScheduleCommand, String> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let Some(first) = words.first().map(|w| w.to_ascii_lowercase()) else { return Ok(ScheduleCommand::Show) };
    let tail = || line.trim_start()[words[0].len()..].trim().to_owned();
    Ok(match first.as_str() {
        "show" | "list" => ScheduleCommand::Show,
        "stop" | "add" => {
            let name = tail();
            if name.is_empty() {
                return Err("Name the station (use * as a wildcard)".into());
            }
            ScheduleCommand::Stop(name)
        }
        "retrieve" => ScheduleCommand::Retrieve(tail()),
        "deliver" => ScheduleCommand::Deliver,
        "title" => ScheduleCommand::Title(tail()),
        "throttle" => {
            let percent = number(words.get(1), "the speed in percent")?;
            if !(0..=100).contains(&percent) {
                return Err("The speed is 0 to 100 percent".into());
            }
            ScheduleCommand::Throttle(percent)
        }
        "remove" => ScheduleCommand::Remove(index(words.get(1))?),
        "move" => {
            let index = index(words.get(1))?;
            match words.get(2).map(|w| w.to_ascii_lowercase()).as_deref() {
                Some("up") => ScheduleCommand::Move { index, up: true },
                Some("down") => ScheduleCommand::Move { index, up: false },
                _ => return Err("Move up or down".into()),
            }
        }
        "when" | "or" => {
            let index = index(words.get(1))?;
            ScheduleCommand::Condition { index, alternative: first == "or", condition: condition(&words[2.min(words.len())..])? }
        }
        "unwait" => ScheduleCommand::DropCondition(index(words.get(1))?),
        "loop" => match words.get(1).map(|w| w.to_ascii_lowercase()).as_deref() {
            None => ScheduleCommand::Cyclic(None),
            Some("on") | Some("true") => ScheduleCommand::Cyclic(Some(true)),
            Some("off") | Some("false") => ScheduleCommand::Cyclic(Some(false)),
            Some(_) => return Err("loop on or loop off".into()),
        },
        "reset" => ScheduleCommand::Reset,
        "clear" => ScheduleCommand::Clear,
        other => return Err(format!("Unknown schedule command '{other}'")),
    })
}

fn entry_mut(schedule: &mut Schedule, index: usize) -> Result<&mut ScheduleEntry, String> {
    let count = schedule.entries.len();
    schedule.entries.get_mut(index - 1).ok_or_else(|| format!("There is no stop {index} (the schedule has {count})"))
}

/// Applies an edit; the answer is what to tell the player. `Show` changes nothing.
pub fn apply(schedule: &mut Schedule, command: ScheduleCommand) -> Result<String, String> {
    match command {
        ScheduleCommand::Show => Ok(summary(schedule).join("\n")),
        ScheduleCommand::Stop(name) => {
            schedule.entries.push(new_stop(ScheduleInstruction::Destination { text: name.clone() }));
            Ok(format!("Added stop {}: travel to {name}", schedule.entries.len()))
        }
        ScheduleCommand::Retrieve(address) => {
            schedule.entries.push(new_stop(ScheduleInstruction::PackageRetrieval { text: address }));
            Ok(format!("Added stop {}: retrieve a package", schedule.entries.len()))
        }
        ScheduleCommand::Deliver => {
            schedule.entries.push(new_stop(ScheduleInstruction::PackageDelivery));
            Ok(format!("Added stop {}: deliver a package", schedule.entries.len()))
        }
        ScheduleCommand::Title(text) => {
            schedule.entries.push(new_stop(ScheduleInstruction::Rename { text }));
            Ok(format!("Added stop {}: update the title", schedule.entries.len()))
        }
        ScheduleCommand::Throttle(value) => {
            schedule.entries.push(new_stop(ScheduleInstruction::Throttle { value }));
            Ok(format!("Added stop {}: limit the speed to {value}%", schedule.entries.len()))
        }
        ScheduleCommand::Remove(index) => {
            entry_mut(schedule, index)?;
            schedule.entries.remove(index - 1);
            Ok(format!("Removed stop {index}"))
        }
        ScheduleCommand::Move { index, up } => {
            entry_mut(schedule, index)?;
            let target = if up { index.checked_sub(1).filter(|t| *t >= 1) } else { Some(index + 1).filter(|t| *t <= schedule.entries.len()) };
            let Some(target) = target else { return Err(format!("Stop {index} can't move {}", if up { "up" } else { "down" })) };
            schedule.entries.swap(index - 1, target - 1);
            Ok(format!("Stop {index} is now stop {target}"))
        }
        ScheduleCommand::Condition { index, alternative, condition } => {
            let entry = entry_mut(schedule, index)?;
            if !entry.instruction.supports_conditions() {
                return Err("This kind of stop takes no conditions".into());
            }
            if alternative || entry.conditions.is_empty() {
                entry.conditions.push(vec![condition]);
            } else if let Some(column) = entry.conditions.last_mut() {
                column.push(condition);
            }
            Ok(format!("Stop {index}: {}", describe_conditions(&entry.conditions)))
        }
        ScheduleCommand::DropCondition(index) => {
            let entry = entry_mut(schedule, index)?;
            let Some(column) = entry.conditions.last_mut() else { return Err("The stop has no conditions".into()) };
            column.pop();
            if column.is_empty() {
                entry.conditions.pop();
            }
            Ok(format!("Stop {index}: {}", describe_conditions(&entry.conditions)))
        }
        ScheduleCommand::Cyclic(value) => {
            schedule.cyclic = value.unwrap_or(!schedule.cyclic);
            Ok(if schedule.cyclic { "The schedule starts over when completed".into() } else { "The schedule ends when completed".into() })
        }
        ScheduleCommand::Reset => {
            schedule.saved_progress = 0;
            Ok("Progress reset".into())
        }
        ScheduleCommand::Clear => {
            schedule.entries.clear();
            schedule.saved_progress = 0;
            Ok("The schedule is empty".into())
        }
    }
}

// --- summaries (`getTitleAs`, `getSummary`, English) --------------------------------------------------------------

fn time_text(t: &TimedWait) -> String {
    let unit = match t.unit() {
        TimeUnit::Ticks => "ticks",
        TimeUnit::Seconds => "seconds",
        TimeUnit::Minutes => "minutes",
    };
    format!("{} {unit}", t.value)
}

/// One condition in a line (`create.schedule.condition.*`).
pub fn describe_condition(condition: &WaitCondition) -> String {
    match condition {
        WaitCondition::Delay(t) => format!("wait {}", time_text(t)),
        WaitCondition::Idle(t) => format!("cargo idle for {}", time_text(t)),
        WaitCondition::TimeOfDay(t) => format!("time of day {}:{:02}", t.hour, t.minute),
        WaitCondition::FluidThreshold(t) => format!("fluid cargo {}", describe_threshold(t, "buckets", "mB")),
        WaitCondition::ItemThreshold(t) => format!("item cargo {}", describe_threshold(t, "stacks", "items")),
        WaitCondition::RedstoneLink { inverted, .. } => {
            if *inverted == 1 { "redstone link off".into() } else { "redstone link on".into() }
        }
        WaitCondition::PlayerCount { count, exact } => {
            format!("{count} player{} seated{}", if *count == 1 { "" } else { "s" }, if *exact != 0 { " exactly" } else { " or more" })
        }
        WaitCondition::Unloaded => "chunk unloaded".into(),
        WaitCondition::Powered => "station powered".into(),
    }
}

fn describe_threshold(t: &Threshold, stacks: &str, items: &str) -> String {
    let operator = match t.operator() {
        Ops::Less => "less than",
        Ops::Greater => "more than",
        Ops::Equal => "exactly",
    };
    let measure = if t.measure == 1 { stacks } else { items };
    let of = t.filter.as_deref().map_or(String::new(), |f| format!(" of {f}"));
    format!("{operator} {} {measure}{of}", t.threshold)
}

/// All the columns of a stop: `|` between the conditions that all must be met, ` or ` between the alternatives.
pub fn describe_conditions(columns: &[Vec<WaitCondition>]) -> String {
    if columns.is_empty() {
        return "no conditions".into();
    }
    columns.iter().map(|c| c.iter().map(describe_condition).collect::<Vec<_>>().join(", then ")).collect::<Vec<_>>().join(" or ")
}

/// A stop's instruction in a line (`create.schedule.instruction.*`).
pub fn describe_instruction(instruction: &ScheduleInstruction) -> String {
    match instruction {
        ScheduleInstruction::Destination { text } => format!("Travel to Station: {text}"),
        ScheduleInstruction::PackageDelivery => "Deliver Package".into(),
        ScheduleInstruction::PackageRetrieval { text } => format!("Retrieve Package addressed to: {text}"),
        ScheduleInstruction::Rename { text } => format!("Update Schedule Title: {text}"),
        ScheduleInstruction::Throttle { value } => format!("Limit Max Speed to {value}%"),
    }
}

/// The lines `show` prints: the loop, then each stop with its conditions.
pub fn summary(schedule: &Schedule) -> Vec<String> {
    let mut lines = vec![format!(
        "Train Schedule: {} stop{}, {}",
        schedule.entries.len(),
        if schedule.entries.len() == 1 { "" } else { "s" },
        if schedule.cyclic { "loops forever" } else { "runs once" }
    )];
    for (i, entry) in schedule.entries.iter().enumerate() {
        let mut line = format!("{}. {}", i + 1, describe_instruction(&entry.instruction));
        if entry.instruction.supports_conditions() {
            line.push_str(&format!(" | {}", describe_conditions(&entry.conditions)));
        }
        lines.push(line);
    }
    lines
}

/// `ScheduleItem.appendHoverText`: a line per destination, the current stop (`savedProgress`, with more than one stop)
/// marked `-> ` and shown yellow, the others `> ` and gold.
pub fn tooltip(schedule: &Schedule) -> Vec<(String, bool)> {
    let mut lines = Vec::new();
    for (i, entry) in schedule.entries.iter().enumerate() {
        let ScheduleInstruction::Destination { text } = &entry.instruction else { continue };
        let current = i as i32 == schedule.saved_progress && schedule.entries.len() > 1;
        lines.push((format!("{}{text}", if current { "-> " } else { "> " }), current));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(schedule: &mut Schedule, line: &str) -> Result<String, String> {
        apply(schedule, parse(line)?)
    }

    #[test]
    fn a_new_stop_waits_five_seconds() {
        let mut s = Schedule::default();
        run(&mut s, "stop Home *").unwrap();
        assert_eq!(s.entries[0].instruction, ScheduleInstruction::Destination { text: "Home *".into() });
        assert_eq!(s.entries[0].conditions, vec![vec![WaitCondition::Delay(TimedWait { value: 5, unit: 1 })]]);
        assert!(s.cyclic);
    }

    #[test]
    fn conditions_go_on_the_last_column_or_start_one() {
        let mut s = Schedule::default();
        run(&mut s, "stop A").unwrap();
        run(&mut s, "when 1 powered").unwrap();
        run(&mut s, "or 1 time 8 30").unwrap();
        run(&mut s, "when 1 players 2 exact").unwrap();
        assert_eq!(
            s.entries[0].conditions,
            vec![
                vec![WaitCondition::Delay(TimedWait::default()), WaitCondition::Powered],
                vec![
                    WaitCondition::TimeOfDay(TimeOfDay { hour: 8, minute: 30, rotation: 5 }),
                    WaitCondition::PlayerCount { count: 2, exact: 1 }
                ],
            ]
        );
        run(&mut s, "unwait 1").unwrap();
        run(&mut s, "unwait 1").unwrap();
        // The emptied column goes with its last condition.
        assert_eq!(s.entries[0].conditions.len(), 1);
        assert_eq!(describe_conditions(&s.entries[0].conditions), "wait 5 seconds, then station powered");
    }

    #[test]
    fn cargo_and_time_units() {
        let mut s = Schedule::default();
        run(&mut s, "stop A").unwrap();
        run(&mut s, "or 1 items more 64 stacks minecraft:iron_ingot").unwrap();
        run(&mut s, "or 1 delay 2 minutes").unwrap();
        run(&mut s, "or 1 idle 20 ticks").unwrap();
        let columns = &s.entries[0].conditions;
        assert_eq!(
            columns[1][0],
            WaitCondition::ItemThreshold(Threshold {
                threshold: "64".into(),
                operator: Ops::Greater as i32,
                measure: 1,
                filter: Some("minecraft:iron_ingot".into())
            })
        );
        assert_eq!(columns[2][0], WaitCondition::Delay(TimedWait { value: 2, unit: TimeUnit::Minutes as i32 }));
        assert_eq!(columns[3][0], WaitCondition::Idle(TimedWait { value: 20, unit: TimeUnit::Ticks as i32 }));
        assert_eq!(describe_condition(&columns[1][0]), "item cargo more than 64 stacks of minecraft:iron_ingot");
    }

    #[test]
    fn stops_without_conditions_refuse_them() {
        let mut s = Schedule::default();
        run(&mut s, "throttle 50").unwrap();
        assert!(s.entries[0].conditions.is_empty());
        assert!(run(&mut s, "when 1 powered").is_err());
        assert!(run(&mut s, "throttle 150").is_err());
        run(&mut s, "title Express").unwrap();
        assert!(s.entries[1].conditions.is_empty());
    }

    #[test]
    fn removing_moving_and_the_loop() {
        let mut s = Schedule::default();
        for name in ["A", "B", "C"] {
            run(&mut s, &format!("stop {name}")).unwrap();
        }
        run(&mut s, "move 3 up").unwrap();
        let names: Vec<_> = s.entries.iter().map(|e| describe_instruction(&e.instruction)).collect();
        assert_eq!(names[1], "Travel to Station: C");
        assert!(run(&mut s, "move 1 up").is_err());
        assert!(run(&mut s, "move 3 down").is_err());
        run(&mut s, "remove 2").unwrap();
        assert_eq!(s.entries.len(), 2);
        assert!(run(&mut s, "remove 9").is_err());
        run(&mut s, "loop").unwrap();
        assert!(!s.cyclic);
        run(&mut s, "loop on").unwrap();
        assert!(s.cyclic);
        s.saved_progress = 1;
        run(&mut s, "reset").unwrap();
        assert_eq!(s.saved_progress, 0);
        run(&mut s, "clear").unwrap();
        assert!(s.entries.is_empty());
    }

    #[test]
    fn bad_lines_say_what_is_wrong() {
        assert_eq!(parse(""), Ok(ScheduleCommand::Show));
        assert!(parse("stop").is_err());
        assert!(parse("when x powered").is_err());
        assert!(parse("when 1 sparkle").is_err());
        assert!(parse("when 1 time 25 0").is_err());
        assert!(parse("move 1 sideways").is_err());
        assert!(parse("remove 0").is_err());
        assert!(parse("dance").is_err());
    }

    #[test]
    fn the_tooltip_marks_the_current_stop_only_with_several() {
        let mut s = Schedule::default();
        run(&mut s, "stop A").unwrap();
        assert_eq!(tooltip(&s), vec![("> A".to_owned(), false)]);
        run(&mut s, "stop B").unwrap();
        s.saved_progress = 1;
        assert_eq!(tooltip(&s), vec![("> A".to_owned(), false), ("-> B".to_owned(), true)]);
    }

    #[test]
    fn the_summary_lists_each_stop() {
        let mut s = Schedule::default();
        run(&mut s, "stop Home").unwrap();
        run(&mut s, "throttle 40").unwrap();
        let lines = summary(&s);
        assert_eq!(lines[0], "Train Schedule: 2 stops, loops forever");
        assert_eq!(lines[1], "1. Travel to Station: Home | wait 5 seconds");
        assert_eq!(lines[2], "2. Limit Max Speed to 40%");
    }
}
