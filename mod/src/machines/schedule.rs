//! The Train Schedule item (`ScheduleItem`, `AllDataComponents.TRAIN_SCHEDULE`): a list of stops, each a destination or another
//! instruction with the conditions that end the wait there, that a train follows once it is given the item. The data and the
//! edits are `create_kinetics::rules::trains::{schedule, schedule_nbt, schedule_edit}`; this is the item.
//!
//! The item holds the schedule as the component `create:train_schedule` (`Schedule.write`: `Entries`, `Cyclic`, `Progress`).
//!
//! Known differences from Create, for the README:
//! - **No schedule screen** (`ScheduleScreen`, `ScheduleMenu`): a right click with the schedule in hand prints its stops, and
//!   `/schedule` stands in for the screen: `stop <station>`, `retrieve <address>`, `deliver`, `title <text>`, `throttle <percent>`,
//!   `when <n> <condition>`, `or <n> <condition>`, `unwait <n>`, `move <n> up|down`, `remove <n>`, `loop [on|off]`, `reset`,
//!   `clear`. It works on the schedule in the main hand and writes it to the stack on the next inventory tick (a command can't
//!   change a stack), so a second command in the same tick still sees the first.
//! - No stop suggestions (`DestinationSuggestions`), no icons, no redstone link frequencies in a condition (it needs two item
//!   slots), and a cargo condition's item is an id typed in the command.
//! - Handing the schedule to a conductor (`handScheduleTo`, `ScheduleItemEntityInteraction`) needs a train with seated conductors
//!   (part 3): a click on a seated mob does nothing special yet.
//! - The tooltip lists the destinations as Create's does (`appendHoverText`), the current stop yellow with an arrow.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};

use create_kinetics::rules::snbt::Value;
use create_kinetics::rules::trains::schedule::Schedule;
use create_kinetics::rules::trains::schedule_edit::{self as edit, ScheduleCommand};
use create_kinetics::rules::trains::schedule_nbt::{read_schedule, write_schedule};
use crate::compat::{
    ComponentTypesEvent, Compound, Flow, Hooks, Host, InventoryTickEvent, ItemStack, NativeCommand, PerformCommandEvent,
    RegisterCommandsEvent, Tag, Text, TooltipEvent, UseItemEvent, UseItemOnEvent,
};

use super::train_net as net;

pub const SCHEDULE: &str = "create:schedule";
/// `AllDataComponents.TRAIN_SCHEDULE`.
pub const COMPONENT: &str = "create:train_schedule";
/// Chat colours of the tooltip: gold for a stop, yellow for the current one.
const GOLD: u32 = 0xFFFF_AA00;
const YELLOW: u32 = 0xFFFF_FF55;

struct Schedules;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Schedules)
}

/// What each player's schedule in hand should be (written by the next inventory tick, read first by every command).
static PENDING: LazyLock<Mutex<HashMap<u128, Tag>>> = LazyLock::new(Mutex::default);
static PENDING_COUNT: AtomicUsize = AtomicUsize::new(0);

// --- the tree and the tag -------------------------------------------------------------------------------------------------

fn value_of(tag: &Tag) -> Option<Value> {
    Some(match tag {
        Tag::Byte(v) => Value::Byte(*v),
        Tag::Short(v) => Value::Short(*v),
        Tag::Int(v) => Value::Int(*v),
        Tag::Long(v) => Value::Long(*v),
        Tag::String(s) => Value::Str(s.to_string_lossy()),
        Tag::List(items) => Value::List(items.iter().filter_map(value_of).collect()),
        Tag::Compound(c) => Value::Compound(c.iter().filter_map(|(k, v)| Some((k.to_string_lossy(), value_of(v)?))).collect()),
        _ => return None,
    })
}

fn tag_of(value: Value) -> Tag {
    match value {
        Value::Byte(v) => Tag::Byte(v),
        Value::Short(v) => Tag::Short(v),
        Value::Int(v) => Tag::Int(v),
        Value::Long(v) => Tag::Long(v),
        Value::Str(s) => Tag::String(s.as_str().into()),
        Value::List(items) => Tag::List(items.into_iter().map(tag_of).collect()),
        Value::Compound(fields) => {
            let mut out = Compound::new();
            for (key, value) in fields {
                out.insert(key.as_str().into(), tag_of(value));
            }
            Tag::Compound(out)
        }
        // The codec writes none of the others.
        _ => Tag::Byte(0),
    }
}

/// `ScheduleItem.getSchedule`: the stack's schedule, none for a stack without the component.
fn schedule_of(stack: &ItemStack) -> Option<Schedule> {
    let tag = stack.data()?.other(COMPONENT)?;
    match value_of(tag)? {
        Value::Compound(fields) => Some(read_schedule(&fields)),
        _ => None,
    }
}

fn tag_for(schedule: &Schedule) -> Tag {
    tag_of(write_schedule(schedule))
}

fn set_pending(uuid: u128, tag: Tag) {
    let mut pending = net::lock(&PENDING);
    pending.insert(uuid, tag);
    PENDING_COUNT.store(pending.len(), Ordering::Relaxed);
}

/// The schedule a player holds: an edit not yet on the stack first, the stack's, a new one (`new Schedule()`).
fn held(host: &dyn Host, uuid: u128) -> Option<Schedule> {
    let stack = host.held_stack(uuid).filter(|s| &*s.item == SCHEDULE)?;
    if let Some(pending) = net::lock(&PENDING).get(&uuid)
        && let Some(Value::Compound(fields)) = value_of(pending)
    {
        return Some(read_schedule(&fields));
    }
    Some(schedule_of(&stack).unwrap_or_default())
}

fn show(host: &mut dyn Host, uuid: u128, schedule: &Schedule) {
    for line in edit::summary(schedule) {
        net::tell(host, uuid, line);
    }
    net::tell(host, uuid, "Edit it with /schedule stop <station>, when <n> <condition>, or <n> <condition>, remove <n>, move <n> up|down, loop, clear");
}

impl Hooks for Schedules {
    fn component_types(&self, event: &mut ComponentTypesEvent<'_>) {
        if !event.types.iter().any(|t| t == COMPONENT) {
            event.types.push(COMPONENT.to_owned());
        }
    }

    /// `ScheduleItem.useOn` and `use`: a right click without sneaking opens the screen; here it prints the stops.
    fn use_item(&self, host: &mut dyn Host, event: &mut UseItemEvent<'_>) -> Flow {
        if &*event.stack.item != SCHEDULE {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else { return Flow::Continue };
        if player.sneaking {
            return Flow::Continue;
        }
        if let Some(schedule) = held(&*host, player.uuid) {
            show(host, player.uuid, &schedule);
        }
        Flow::Cancel
    }

    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != SCHEDULE {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else { return Flow::Continue };
        if player.sneaking {
            return Flow::Continue;
        }
        if crate::util::first_click(&*host, player.uuid, event.pos)
            && let Some(schedule) = held(&*host, player.uuid)
        {
            show(host, player.uuid, &schedule);
        }
        Flow::Cancel
    }

    /// `ScheduleItem.appendHoverText`.
    fn tooltip(&self, _: &dyn Host, event: &mut TooltipEvent<'_>) {
        if &*event.stack.item != SCHEDULE {
            return;
        }
        let Some(schedule) = schedule_of(event.stack) else { return };
        for (line, current) in edit::tooltip(&schedule) {
            // The caret or the arrow is grey and the name gold or yellow in Create: one colour a line here, the name's.
            event.lines.push((Text::literal(line), if current { YELLOW } else { GOLD }));
        }
    }

    fn register_commands(&self, _: &mut dyn Host, event: &mut RegisterCommandsEvent<'_>) {
        event.commands.push(NativeCommand::new("schedule"));
        event.commands.push(NativeCommand::new("schedule <text:text>"));
    }

    fn perform_command(&self, host: &mut dyn Host, event: &mut PerformCommandEvent<'_>) -> Flow {
        let line = event.command.trim();
        let Some(rest) = (if line == "schedule" { Some("") } else { line.strip_prefix("schedule ") }) else {
            return Flow::Continue;
        };
        let uuid = event.player.uuid;
        let Some(mut schedule) = held(&*host, uuid) else {
            net::tell(host, uuid, "Hold a Train Schedule in your main hand");
            return Flow::Cancel;
        };
        let command = match edit::parse(rest) {
            Ok(command) => command,
            Err(message) => {
                net::tell(host, uuid, message);
                return Flow::Cancel;
            }
        };
        let is_show = command == ScheduleCommand::Show;
        match edit::apply(&mut schedule, command) {
            Ok(message) => {
                if !is_show {
                    set_pending(uuid, tag_for(&schedule));
                }
                for line in message.lines() {
                    net::tell(host, uuid, line);
                }
            }
            Err(message) => net::tell(host, uuid, message),
        }
        Flow::Cancel
    }

    /// Writes an edit of the held schedule to the stack.
    fn inventory_tick(&self, host: &mut dyn Host, event: &mut InventoryTickEvent) {
        if !event.selected || PENDING_COUNT.load(Ordering::Relaxed) == 0 {
            return;
        }
        let Some(player) = host.acting_player() else { return };
        let Some(tag) = net::lock(&PENDING).get(&player.uuid).cloned() else { return };
        let Some(mut stack) = host.carried_stack(event.slot).map(|s| s.clone()) else { return };
        if &*stack.item != SCHEDULE {
            return;
        }
        let _ = host.set_component(&mut stack, COMPONENT, tag);
        if let Some(slot) = host.carried_stack(event.slot) {
            *slot = stack;
        }
        let mut pending = net::lock(&PENDING);
        pending.remove(&player.uuid);
        PENDING_COUNT.store(pending.len(), Ordering::Relaxed);
    }
}
