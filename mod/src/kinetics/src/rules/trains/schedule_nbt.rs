//! A schedule as an NBT tree: `Schedule.write` / `fromTag`, `ScheduleEntry.write` / `fromTag`,
//! `ScheduleInstruction.write` / `fromTag`, `ScheduleWaitCondition.write` / `fromTag` and what each condition or
//! instruction keeps in its `data` tag (content/trains/schedule/). It is the form `create:train_schedule` holds on a
//! Schedule item (`ScheduleItem.getSchedule`).
//!
//! The tree is [`crate::rules::snbt::Value`]; the glue turns it into the host's `Tag`. Reading is as forgiving as
//! Java's: a missing key is the default (0, empty), an unknown instruction id is a destination (`fromTag` warns and
//! gives `DestinationInstruction`), an unknown condition id is dropped (`readCompoundList` drops the nulls).
//!
//! The filters of the cargo conditions and the frequencies of the redstone link are opaque item ids here
//! ([`super::schedule`]); they are saved as the `{id, count: 1}` compound a stack serialises to, an empty one for
//! none.

use super::schedule::{Schedule, ScheduleEntry, ScheduleInstruction, Threshold, TimeOfDay, TimedWait, WaitCondition};
use crate::rules::snbt::Value;

fn get<'a>(tag: &'a [(String, Value)], key: &str) -> Option<&'a Value> {
    tag.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn compound(tag: &[(String, Value)], key: &str) -> Vec<(String, Value)> {
    match get(tag, key) {
        Some(Value::Compound(c)) => c.clone(),
        _ => Vec::new(),
    }
}

/// `CompoundTag.getInt`: numbers of any width, 0 otherwise.
fn int(tag: &[(String, Value)], key: &str) -> i32 {
    match get(tag, key) {
        Some(Value::Int(v)) => *v,
        Some(Value::Byte(v)) => i32::from(*v),
        Some(Value::Short(v)) => i32::from(*v),
        Some(Value::Long(v)) => *v as i32,
        _ => 0,
    }
}

fn text(tag: &[(String, Value)], key: &str) -> String {
    match get(tag, key) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

fn entry(key: &str, value: Value) -> (String, Value) {
    (key.to_owned(), value)
}

/// A stack as `ItemStack.save` writes it, an item id and a count of one, or an empty compound.
fn stack_tag(item: &Option<String>) -> Value {
    match item {
        Some(id) if !id.is_empty() => Value::Compound(vec![entry("id", Value::Str(id.clone())), entry("count", Value::Int(1))]),
        _ => Value::Compound(Vec::new()),
    }
}

fn stack_of(tag: &[(String, Value)]) -> Option<String> {
    match get(tag, "id") {
        Some(Value::Str(id)) if !id.is_empty() && id != "minecraft:air" => Some(id.clone()),
        _ => None,
    }
}

// --- instructions ----------------------------------------------------------------------------------------

/// `ScheduleInstruction.write`: `Id` and `Data`.
pub fn write_instruction(instruction: &ScheduleInstruction) -> Value {
    let data = match instruction {
        ScheduleInstruction::Destination { text } | ScheduleInstruction::PackageRetrieval { text } | ScheduleInstruction::Rename { text } => {
            vec![entry("Text", Value::Str(text.clone()))]
        }
        ScheduleInstruction::Throttle { value } => vec![entry("Value", Value::Int(*value))],
        ScheduleInstruction::PackageDelivery => Vec::new(),
    };
    Value::Compound(vec![
        entry("Id", Value::Str(format!("create:{}", instruction.id()))),
        entry("Data", Value::Compound(data)),
    ])
}

/// `ScheduleInstruction.fromTag`.
pub fn read_instruction(tag: &[(String, Value)]) -> ScheduleInstruction {
    let data = compound(tag, "Data");
    let id = text(tag, "Id");
    let mut instruction = ScheduleInstruction::from_id(&id);
    match &mut instruction {
        ScheduleInstruction::Destination { text: t }
        | ScheduleInstruction::PackageRetrieval { text: t }
        | ScheduleInstruction::Rename { text: t } => *t = text(&data, "Text"),
        // `ChangeThrottleInstruction`'s constructor sets 100; `fromTag` replaces the data, so a missing value is 0.
        ScheduleInstruction::Throttle { value } => *value = int(&data, "Value"),
        ScheduleInstruction::PackageDelivery => {}
    }
    instruction
}

// --- wait conditions ---------------------------------------------------------------------------------------

/// `ScheduleWaitCondition.write`.
pub fn write_condition(condition: &WaitCondition) -> Value {
    let mut data: Vec<(String, Value)> = Vec::new();
    match condition {
        WaitCondition::Delay(t) | WaitCondition::Idle(t) => {
            data.push(entry("Value", Value::Int(t.value)));
            data.push(entry("TimeUnit", Value::Int(t.unit)));
        }
        WaitCondition::TimeOfDay(t) => {
            data.push(entry("Hour", Value::Int(t.hour)));
            data.push(entry("Minute", Value::Int(t.minute)));
            data.push(entry("Rotation", Value::Int(t.rotation)));
        }
        WaitCondition::FluidThreshold(t) => {
            data.push(entry("Threshold", Value::Str(t.threshold.clone())));
            data.push(entry("Operator", Value::Int(t.operator)));
            data.push(entry("Measure", Value::Int(t.measure)));
            data.push(entry("Bucket", stack_tag(&t.filter)));
        }
        WaitCondition::ItemThreshold(t) => {
            data.push(entry("Threshold", Value::Str(t.threshold.clone())));
            data.push(entry("Operator", Value::Int(t.operator)));
            data.push(entry("Measure", Value::Int(t.measure)));
            data.push(entry("Item", stack_tag(&t.filter)));
        }
        WaitCondition::RedstoneLink { frequency, inverted } => {
            data.push(entry("Inverted", Value::Int(*inverted)));
            let list = frequency.iter().map(|f| stack_tag(&Some(f.clone()))).collect();
            data.push(entry("Frequency", Value::List(list)));
        }
        WaitCondition::PlayerCount { count, exact } => {
            data.push(entry("Count", Value::Int(*count)));
            data.push(entry("Exact", Value::Int(*exact)));
        }
        WaitCondition::Unloaded | WaitCondition::Powered => {}
    }
    Value::Compound(vec![
        entry("Id", Value::Str(format!("create:{}", condition.id()))),
        entry("Data", Value::Compound(data)),
    ])
}

/// `ScheduleWaitCondition.fromTag`: `None` for an id that isn't registered.
pub fn read_condition(tag: &[(String, Value)]) -> Option<WaitCondition> {
    let id = text(tag, "Id");
    let mut condition = WaitCondition::from_id(&id)?;
    let data = compound(tag, "Data");
    match &mut condition {
        WaitCondition::Delay(t) | WaitCondition::Idle(t) => *t = TimedWait { value: int(&data, "Value"), unit: int(&data, "TimeUnit") },
        WaitCondition::TimeOfDay(t) => {
            *t = TimeOfDay { hour: int(&data, "Hour"), minute: int(&data, "Minute"), rotation: int(&data, "Rotation") }
        }
        WaitCondition::FluidThreshold(t) => *t = read_threshold(&data, "Bucket"),
        WaitCondition::ItemThreshold(t) => *t = read_threshold(&data, "Item"),
        WaitCondition::RedstoneLink { frequency, inverted } => {
            *inverted = int(&data, "Inverted");
            if let Some(Value::List(list)) = get(&data, "Frequency") {
                for (i, slot) in frequency.iter_mut().enumerate() {
                    if let Some(Value::Compound(c)) = list.get(i) {
                        *slot = stack_of(c).unwrap_or_default();
                    }
                }
            }
        }
        WaitCondition::PlayerCount { count, exact } => {
            *count = int(&data, "Count");
            *exact = int(&data, "Exact");
        }
        WaitCondition::Unloaded | WaitCondition::Powered => {}
    }
    Some(condition)
}

fn read_threshold(data: &[(String, Value)], filter_key: &str) -> Threshold {
    Threshold {
        // `textData("Threshold")`: a missing one is the empty text, which `getThreshold` resets to "0".
        threshold: text(data, "Threshold"),
        operator: int(data, "Operator"),
        measure: int(data, "Measure"),
        filter: stack_of(&compound(data, filter_key)),
    }
}

// --- entries and schedules ----------------------------------------------------------------------------------

/// `ScheduleEntry.write`: `Instruction`, and `Conditions` (a list of lists) when the instruction takes them.
pub fn write_entry(e: &ScheduleEntry) -> Value {
    let mut tag = vec![entry("Instruction", write_instruction(&e.instruction))];
    if e.instruction.supports_conditions() {
        let columns = e.conditions.iter().map(|column| Value::List(column.iter().map(write_condition).collect())).collect();
        tag.push(entry("Conditions", Value::List(columns)));
    }
    Value::Compound(tag)
}

/// `ScheduleEntry.fromTag`.
pub fn read_entry(tag: &[(String, Value)]) -> ScheduleEntry {
    let instruction = read_instruction(&compound(tag, "Instruction"));
    let mut conditions = Vec::new();
    if instruction.supports_conditions() {
        if let Some(Value::List(columns)) = get(tag, "Conditions") {
            for column in columns {
                if let Value::List(items) = column {
                    conditions.push(
                        items
                            .iter()
                            .filter_map(|c| match c {
                                Value::Compound(c) => read_condition(c),
                                _ => None,
                            })
                            .collect(),
                    );
                }
            }
        }
    }
    ScheduleEntry { instruction, conditions }
}

/// `Schedule.write`: `Entries`, `Cyclic` and `Progress` when there is any.
pub fn write_schedule(schedule: &Schedule) -> Value {
    let mut tag = vec![
        entry("Entries", Value::List(schedule.entries.iter().map(write_entry).collect())),
        entry("Cyclic", Value::Byte(i8::from(schedule.cyclic))),
    ];
    if schedule.saved_progress > 0 {
        tag.push(entry("Progress", Value::Int(schedule.saved_progress)));
    }
    Value::Compound(tag)
}

/// `Schedule.fromTag`.
pub fn read_schedule(tag: &[(String, Value)]) -> Schedule {
    let entries = match get(tag, "Entries") {
        Some(Value::List(list)) => list
            .iter()
            .filter_map(|e| match e {
                Value::Compound(c) => Some(read_entry(c)),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    Schedule { entries, cyclic: int(tag, "Cyclic") != 0, saved_progress: int(tag, "Progress") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compound_of(v: Value) -> Vec<(String, Value)> {
        match v {
            Value::Compound(c) => c,
            other => panic!("not a compound: {other:?}"),
        }
    }

    fn sample() -> Schedule {
        Schedule {
            entries: vec![
                ScheduleEntry::new(
                    ScheduleInstruction::Destination { text: "Home *".into() },
                    vec![
                        vec![WaitCondition::Delay(TimedWait { value: 30, unit: 1 }), WaitCondition::Powered],
                        vec![WaitCondition::ItemThreshold(Threshold {
                            threshold: "64".into(),
                            operator: 1,
                            measure: 1,
                            filter: Some("minecraft:iron_ingot".into()),
                        })],
                    ],
                ),
                ScheduleEntry::new(ScheduleInstruction::Throttle { value: 50 }, Vec::new()),
                ScheduleEntry::new(
                    ScheduleInstruction::PackageRetrieval { text: "Box".into() },
                    vec![vec![
                        WaitCondition::TimeOfDay(TimeOfDay { hour: 9, minute: 15, rotation: 3 }),
                        WaitCondition::RedstoneLink { frequency: ["minecraft:stone".into(), String::new()], inverted: 1 },
                        WaitCondition::PlayerCount { count: 2, exact: 1 },
                        WaitCondition::Idle(TimedWait::default()),
                        WaitCondition::Unloaded,
                        WaitCondition::FluidThreshold(Threshold::default()),
                    ]],
                ),
            ],
            cyclic: false,
            saved_progress: 2,
        }
    }

    #[test]
    fn a_schedule_survives_being_written_and_read() {
        let schedule = sample();
        let tag = compound_of(write_schedule(&schedule));
        assert_eq!(read_schedule(&tag), schedule);
    }

    #[test]
    fn the_keys_are_creates() {
        let tag = compound_of(write_schedule(&sample()));
        let keys: Vec<&str> = tag.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["Entries", "Cyclic", "Progress"]);
        let Some(Value::List(entries)) = get(&tag, "Entries") else { panic!() };
        let first = compound_of(entries[0].clone());
        assert_eq!(first[0].0, "Instruction");
        assert_eq!(first[1].0, "Conditions");
        let instruction = compound_of(first[0].1.clone());
        assert_eq!(instruction[0], ("Id".to_owned(), Value::Str("create:destination".into())));
        assert_eq!(compound(&instruction, "Data"), vec![("Text".to_owned(), Value::Str("Home *".into()))]);
        // A throttle takes no conditions, so the entry has none written.
        let second = compound_of(entries[1].clone());
        assert_eq!(second.len(), 1);
        // No progress is written for 0.
        let mut fresh = sample();
        fresh.saved_progress = 0;
        let tag = compound_of(write_schedule(&fresh));
        assert!(get(&tag, "Progress").is_none());
    }

    #[test]
    fn unknown_ids_read_as_java_does() {
        let bad_instruction = vec![entry("Id", Value::Str("create:nope".into())), entry("Data", Value::Compound(Vec::new()))];
        assert_eq!(read_instruction(&bad_instruction), ScheduleInstruction::Destination { text: String::new() });
        let bad_condition = vec![entry("Id", Value::Str("create:nope".into()))];
        assert!(read_condition(&bad_condition).is_none());
        // A column with only a bad condition is kept, empty.
        let entry_tag = vec![
            entry("Instruction", write_instruction(&ScheduleInstruction::Destination { text: "A".into() })),
            entry("Conditions", Value::List(vec![Value::List(vec![Value::Compound(bad_condition)])])),
        ];
        assert_eq!(read_entry(&entry_tag).conditions, vec![Vec::<WaitCondition>::new()]);
    }

    #[test]
    fn an_empty_tag_is_an_empty_cyclic_looking_schedule() {
        // `fromTag` reads `Cyclic` as `getBoolean`: absent is false.
        let schedule = read_schedule(&[]);
        assert!(schedule.entries.is_empty());
        assert!(!schedule.cyclic);
        assert_eq!(schedule.saved_progress, 0);
    }

    #[test]
    fn numbers_of_any_width_read() {
        let data = vec![entry("Value", Value::Byte(7)), entry("TimeUnit", Value::Short(2))];
        let tag = vec![entry("Id", Value::Str("create:delay".into())), entry("Data", Value::Compound(data))];
        assert_eq!(read_condition(&tag), Some(WaitCondition::Delay(TimedWait { value: 7, unit: 2 })));
    }
}
