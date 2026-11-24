//! The people (and mobs) on a train: seated passengers and seated conductors, handing a Schedule to a conductor
//! (`ScheduleItemEntityInteraction`, `ScheduleItem.handScheduleTo`) and the Blaze Burner that conducts
//! (`ConductorBlockInteractionBehavior.BlazeBurner`). The decisions are `create_kinetics::rules::trains::{train_passengers,
//! schedule_hand}`; the markers and their riders are `seat.rs`, the carriages and their trains `trains.rs`.
//!
//! - **Seating** ([`sweep`], called by `trains::carry_riders` each tick): a living mob whose feet touch a seat of a carriage
//!   sits on it, as `SeatInteractionBehaviour.handleEntityCollision` seats what the contraption's collision meets. A player
//!   sits by clicking the seat (`structure_click.rs`), and replaces a mob there.
//! - **Conductors**: `trains::build_world` asks `check_conductors` of the passengers and the carriage's burner conductors; a
//!   mob on a seat in front of Train Controls is a conductor for that side, so a train with one can run a schedule on its own.
//! - **Handing a Schedule over**: a right click on a seated mob with a Schedule in the main hand
//!   gives it to the train when the mob's seat is in front of controls; an empty hand takes the schedule back; a click
//!   on a paused train resumes it. Clicking the Blaze Burner in front of the controls does the same.
//!
//! Differences from Create, for the README:
//! - A mob is seated only by touching a seat, not by a lead (`SeatBlock.getLeashed`: no leash read), and only on a carriage's
//!   seats here; mobs don't walk onto a seat of a block in the world.
//! - `TamableAnimal.setInSittingPose` isn't done. Hostile mobs are seated too (`seatHostileMobs` defaults to true).
//! - The Schedule item's cooldown (5 ticks) is kept per player here (Horizon's cooldown call isn't on the compat layer).
//! - The hat (`EntityHats`, `BlazeBurnerRenderer`'s Train Hat) is decided by `schedule_hand::wears_train_hat` and
//!   `burner_wears_hat` but not drawn: nothing here draws on a mob, and a carriage's burner head isn't drawn at all.
//! - Messages go to the action bar in plain text (no colour: `applied_to_train` is green in Create), and the sounds are
//!   `create:confirm` and `create:deny`.
//!
//! Type-checked and unit-tested only (in the rules): not played.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use create_kinetics::rules::seat as seat_rules;
use create_kinetics::rules::trains::carriage_contraption as cc;
use create_kinetics::rules::trains::schedule_hand::{
    self as rules, BlockClick, BlockReaction, Cue, EntityClick, Hand, Message, Reaction, Target,
};
use create_kinetics::rules::trains::train_passengers::{self as passengers, Candidate, Seating};
use create_kinetics::{Dir, Pos};

use crate::compat::{Flow, Hooks, Host, InteractEntityEvent, ItemStack, Text};
use crate::util;

use super::schedule::{self, SCHEDULE};
use super::trains;

struct Riders;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Riders)
}

/// `SoundSource.BLOCKS` and `PLAYERS`.
const BLOCKS: usize = 4;
const PLAYERS: usize = 7;
/// `player.getCooldowns().addCooldown(SCHEDULE, 5)`.
const COOLDOWN: i64 = 5;

/// When each player's Schedule cooldown ends (game time).
static COOLDOWNS: LazyLock<Mutex<HashMap<u128, i64>>> = LazyLock::new(Mutex::default);

fn on_cooldown(who: u128, now: i64) -> bool {
    COOLDOWNS.lock().ok().and_then(|m| m.get(&who).copied()).is_some_and(|end| now < end)
}

fn start_cooldown(who: u128, now: i64) {
    if let Ok(mut m) = COOLDOWNS.lock() {
        m.retain(|_, end| *end > now);
        m.insert(who, now + COOLDOWN);
    }
}

// --- sounds and messages --------------------------------------------------------------------------------------------

fn cue(host: &mut dyn Host, at: [f64; 3], cue: Cue) {
    match cue {
        Cue::Confirm => host.play_sound("create:confirm", at, BLOCKS, 1.0, 1.0),
        Cue::Deny => host.play_sound("create:deny", at, BLOCKS, 1.0, 1.0),
        // `AllSoundEvents.playItemPickup`: `ITEM_PICKUP`, .2, 1 + random.
        Cue::ItemPickup => host.play_sound("minecraft:entity.item.pickup", at, PLAYERS, 0.2, 1.0 + (util::random() % 1000) as f32 / 1000.0),
    };
}

fn say(host: &mut dyn Host, who: u128, message: Message) {
    host.action_bar(who, Text::literal(message.text().to_owned()));
}

// --- applying what the rules decided ----------------------------------------------------------------------------------

/// `train.runtime.setSchedule(schedule, false)` with the held Schedule, the item shrunk by one.
fn apply_schedule(host: &mut dyn Host, who: u128, at: [f64; 3], train: create_kinetics::rules::trains::graph::Uuid, stack: &ItemStack) {
    let Some(schedule) = schedule::schedule_of(stack) else { return };
    if !trains::give_schedule(host, train, schedule, false) {
        return;
    }
    host.take_items(SCHEDULE, 1);
    cue(host, at, Cue::Confirm);
    say(host, who, Message::Applied);
}

/// `player.getInventory().placeItemBackInInventory(train.runtime.returnSchedule(..))`. An Auto-Schedule gives nothing back.
fn return_schedule(host: &mut dyn Host, who: u128, at: [f64; 3], train: create_kinetics::rules::trains::graph::Uuid, auto: bool) {
    cue(host, at, Cue::ItemPickup);
    say(host, who, rules::returned_message(auto));
    let Some(schedule) = trains::take_schedule(host, train) else { return };
    let Some(stack) = schedule::stack_with(&*host, &schedule) else { return };
    if host.give_stack(who, stack.clone()).is_some() {
        util::spawn_item(host, at, &stack, [0.0; 3], 10);
    }
}

fn held_schedule_stops(stack: Option<&ItemStack>) -> Option<usize> {
    let stack = stack.filter(|s| &*s.item == SCHEDULE)?;
    schedule::schedule_of(stack).map(|s| s.entries.len())
}

// --- the hooks ---------------------------------------------------------------------------------------------------------------

impl Hooks for Riders {
    /// `ScheduleItemEntityInteraction.interactWithConductor`: a click on a mob seated on a carriage.
    fn interact_entity(&self, host: &mut dyn Host, event: &mut InteractEntityEvent<'_>) -> Flow {
        // The server decides (the client's result is the same click, and cancelling it there needs the same answer).
        if host.window() {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else { return Flow::Continue };
        let Ok(id) = u32::try_from(event.entity) else { return Flow::Continue };
        let Some(entity) = host.entity(id) else { return Flow::Continue };
        let living = host.living(entity).is_some();
        let Some(uuid) = host.entity_uuid(entity) else { return Flow::Continue };
        let dimension = host.dimension().to_owned();
        let Some((train, carriage, seat)) = trains::carriage_of_rider(&*host, &dimension, uuid) else { return Flow::Continue };
        let now = util::game_time(&*host);
        let target = Target {
            on_carriage: true,
            train: trains::runtime_view(&dimension, train),
            // It sits on a seat of the carriage (it is a passenger there), in front of controls or not.
            seated: true,
            conductor_seat: trains::carriage_info(&dimension, train, carriage, Some(seat)).is_some_and(|c| c.conductor_seat),
        };
        let click = EntityClick {
            spectator: player.game_mode == util::SPECTATOR,
            living,
            on_cooldown: on_cooldown(player.uuid, now),
            main_hand: event.main_hand,
            hand_empty: event.stack.is_none(),
            server: true,
            held_schedule: held_schedule_stops(event.stack),
            target,
        };
        let at = player.position;
        let who = player.uuid;
        let taken = match rules::entity_click(&click) {
            Reaction::Ignore => return Flow::Continue,
            Reaction::Handed(hand) => {
                match hand {
                    Hand::Pass | Hand::Nothing => {}
                    Hand::Deny(message) => {
                        cue(host, at, Cue::Deny);
                        say(host, who, message);
                    }
                    Hand::Apply => {
                        if let Some(stack) = event.stack {
                            apply_schedule(host, who, at, train, stack);
                        }
                    }
                }
                true
            }
            Reaction::Resume => {
                if trains::resume_schedule(&dimension, train) {
                    cue(host, at, Cue::Confirm);
                    say(host, who, Message::Continued);
                }
                true
            }
            Reaction::Refuse => {
                cue(host, at, Cue::Deny);
                say(host, who, Message::RemoveWithEmptyHand);
                // No cooldown for a refusal.
                return Flow::Cancel;
            }
            Reaction::Return { auto } => {
                return_schedule(host, who, at, train, auto);
                true
            }
        };
        if taken {
            start_cooldown(who, now);
        }
        Flow::Cancel
    }
}

// --- the burner -----------------------------------------------------------------------------------------------------------------

/// `ConductorBlockInteractionBehavior.handlePlayerInteraction` for the burner at `local` of the carriage whose anchor is
/// `controller`. True when the click was taken.
pub fn burner_click(host: &mut dyn Host, controller: Pos, local: Pos, who: u128) -> bool {
    let dimension = host.dimension().to_owned();
    let Some((train, carriage)) = trains::carriage_of_anchor(&*host, controller) else { return false };
    let Some(info) = trains::carriage_info(&dimension, train, carriage, None) else { return false };
    let Some(state) = info.blocks.iter().find(|(p, _)| *p == local).map(|(_, s)| s.clone()) else { return false };
    let in_control = Dir::ALL
        .into_iter()
        .filter(|d| d.axis() == info.direction.axis())
        .any(|d| cc::in_control(&info.blocks, local, d));
    let held = host.held_stack(who);
    let at = host.server_players().into_iter().find(|p| p.uuid == who).map_or([0.0; 3], |p| p.position);
    let click = BlockClick {
        main_hand: true,
        valid: cc::is_valid_conductor(&state),
        in_control,
        train: trains::runtime_view(&dimension, train),
        server: true,
        hand_empty: held.is_none(),
        held_schedule: held_schedule_stops(held.as_ref()),
        holds_schedule_item: held.as_ref().is_some_and(|s| &*s.item == SCHEDULE),
    };
    match rules::block_click(&click) {
        BlockReaction::NotTaken => false,
        BlockReaction::Taken => true,
        BlockReaction::Deny(message) => {
            cue(host, at, Cue::Deny);
            say(host, who, message);
            true
        }
        BlockReaction::Resume => {
            if trains::resume_schedule(&dimension, train) {
                cue(host, at, Cue::Confirm);
                say(host, who, Message::Continued);
            }
            true
        }
        BlockReaction::Return { auto } => {
            return_schedule(host, who, at, train, auto);
            true
        }
        BlockReaction::Apply => {
            if let Some(stack) = held {
                apply_schedule(host, who, at, train, &stack);
            }
            true
        }
    }
}

// --- seating mobs ------------------------------------------------------------------------------------------------------------------

/// Every few ticks, the mobs that touch a seat of the carriage known as `key` sit on it. `points` are each seat's local
/// position and where its marker goes (the seat's centre, lifted).
pub fn sweep(host: &mut dyn Host, key: Pos, anchor: Pos, points: &[(Pos, [f64; 3])]) {
    let now = util::game_time(&*host);
    if now % 5 != 0 || points.is_empty() {
        return;
    }
    let dimension = host.dimension().to_owned();
    let sitting = super::seat::passengers_of(&*host, &dimension, key);
    let lift = seat_rules::carried_lift();
    for (local, at) in points {
        // A seat with a rider (a mob never takes a seat from anyone: `addSittingPassenger`).
        let occupant = sitting.iter().find(|(l, _, _)| l == local).map(|(_, _, player)| *player);
        if passengers::add_sitting_passenger(occupant, false) != (Seating::Sit { eject: false }) {
            continue;
        }
        let centre = [at[0] - lift[0], at[1] - lift[1], at[2] - lift[2]];
        let lo = [centre[0] - 1.0, centre[1] - 1.5, centre[2] - 1.0];
        let hi = [centre[0] + 1.0, centre[1] + 1.5, centre[2] + 1.0];
        for id in host.entities_in(&dimension, lo, hi) {
            let Some(entity) = host.entity(id) else { continue };
            let Some(kind) = host.entity_kind(entity) else { continue };
            let candidate = Candidate { kind: &kind, player: kind == "minecraft:player", living: host.living(entity).is_some(), ignored: false };
            if !passengers::can_be_picked_up(&candidate) {
                continue;
            }
            let Some(feet) = host.entity_position(entity) else { continue };
            if !passengers::touches_seat(centre, feet) {
                continue;
            }
            let Some(uuid) = host.entity_uuid(entity) else { continue };
            let cell = [anchor[0] + local[0], anchor[1] + local[1], anchor[2] + local[2]];
            if super::seat::sit_mob_on_structure(&*host, &dimension, key, *local, cell, id, uuid, *at) {
                break;
            }
        }
    }
}
