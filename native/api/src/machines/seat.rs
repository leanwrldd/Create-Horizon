//! Seats (`SeatBlock`, `SeatEntity`): sixteen coloured cushions a player sits on; a dye recolours one.
//! The rules are `create_kinetics::rules::seat`; this reads the world and applies what they say.
//!
//! How a player sits: Create makes a `SeatEntity` and `startRiding`s it. Horizon has no entity for
//! a mod, but `/ride` seats a player on a `minecraft:marker` (an entity it keeps as data, with no
//! size and no model), so a click queues a request, and the level's tick summons the marker where
//! `SeatEntity.setPos` stands and runs `ride <player> mount <marker>` (a command can't run inside
//! a player's hook). The marker goes when the rider gets off, as `SeatEntity.tick` discards the
//! entity. Type-checked and unit-tested only: not yet played.
//!
//! Known differences from Create, for the README:
//! - Mobs that walk or fall onto a seat don't sit (`updateEntityAfterFallOn`, `getLeashed`): the
//!   hook for that doesn't exist. Only a player's click seats anyone.
//! - A rider is taken off a seat that was broken or recoloured to nothing within a few ticks; a
//!   seat whose rider the server can't see (a player missing from `server_players`) keeps its marker
//!   until the next click on that seat replaces it. A marker saved with the world while someone sits
//!   there stays in the save (Create's `SeatEntity` is saved the same way, and discards itself
//!   on its first tick; this one has no tick to do that).
//! - A rider gets off by sneaking, as Create's; they land on the seat, not 0.5 above it.
//! - `SeatMovementBehaviour` and `SeatInteractionBehaviour` (seats on contraptions), the seat's
//!   `ENTITY_NAME` display source and waterlogging are not here.
//! - The seat's shape: its outline is Create's 8/16, its collision 6/16, and 3/16 for a player
//!   (`AllShapes.SEAT*`).

use std::sync::Mutex;

use create_kinetics::rules::seat::{self as rules, Click, Rider};
use create_kinetics::Pos;
use horizon_client::mod_api::{Compound, Flow, Hooks, Host, LevelTickPostEvent, RightClickBlockEvent, Tag};

use crate::machines::encased;
use crate::util;

const MARKER: &str = "minecraft:marker";
/// How often the riders are looked at, in ticks.
const SWEEP: i64 = 5;

struct Seats;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Seats)
}

struct Request {
    dimension: String,
    pos: Pos,
    player: u128,
}

struct Seat {
    dimension: String,
    pos: Pos,
    player: u128,
    id: u32,
    made: i64,
}

#[derive(Default)]
struct Book {
    pending: Vec<Request>,
    seats: Vec<Seat>,
    made: u64,
}

static BOOK: Mutex<Book> = Mutex::new(Book { pending: Vec::new(), seats: Vec::new(), made: 0 });

/// Whether the player of `seat` still rides its marker.
fn rider(host: &dyn Host, seat: &Seat) -> Rider {
    let players = host.server_players();
    let Some(player) = players.iter().find(|p| p.uuid == seat.player) else {
        return Rider::Unknown;
    };
    match u32::try_from(player.entity_id).ok().and_then(|id| host.vehicle_of(id)) {
        Some(vehicle) if vehicle == seat.id => Rider::Riding,
        _ => Rider::Off,
    }
}

impl Seat {
    fn copy(&self) -> Seat {
        Seat { dimension: self.dimension.clone(), ..*self }
    }
}

/// The markers made for the seat block at `pos`.
fn seats_at(dimension: &str, pos: Pos) -> Vec<Seat> {
    BOOK.lock()
        .map(|book| book.seats.iter().filter(|s| s.dimension == dimension && s.pos == pos).map(Seat::copy).collect())
        .unwrap_or_default()
}

/// The players now riding the seat block at `pos`, for the Entity Name display source
/// (`EntityNameDisplaySource`).
pub(super) fn riders_at(host: &dyn Host, dimension: &str, pos: Pos) -> Vec<u128> {
    seats_at(dimension, pos)
        .iter()
        .filter(|seat| rider(host, seat) == Rider::Riding)
        .map(|seat| seat.player)
        .collect()
}

impl Hooks for Seats {
    /// `SeatBlock.useItemOn`.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        let block = host.block(pos);
        let Some(colour) = rules::colour_of_block(&block) else {
            return Flow::Continue;
        };
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        let held = host.held_stack(player.uuid).map(|stack| stack.item.to_string());
        let dimension = host.dimension();
        // Only a marker with somebody confirmed on it makes the seat taken.
        let occupied = seats_at(dimension, pos).iter().any(|s| rider(host, s) == Rider::Riding);
        match rules::click(player.sneaking, held.as_deref(), colour, occupied) {
            Click::Pass | Click::Occupied => Flow::Continue,
            Click::Recolour(new) => {
                encased::switch_block(host, pos, &new, &[]);
                Flow::Cancel
            }
            Click::Sit => {
                if let Ok(mut book) = BOOK.lock()
                    && !book.pending.iter().any(|r| r.player == player.uuid && r.pos == pos)
                {
                    book.pending.push(Request { dimension: dimension.to_owned(), pos, player: player.uuid });
                }
                Flow::Cancel
            }
        }
    }

    /// `SeatBlock.sitDown` (the marker and the ride), `SeatEntity.tick` (taking it away).
    fn level_tick_post(&self, host: &mut dyn Host, event: &mut LevelTickPostEvent<'_>) {
        let dimension = event.dimension;
        let now = util::game_time(host);
        // New riders.
        let requests: Vec<Request> = match BOOK.lock() {
            Ok(mut book) => {
                let (mine, rest): (Vec<_>, Vec<_>) = book.pending.drain(..).partition(|r| r.dimension == dimension);
                book.pending = rest;
                mine
            }
            Err(_) => return,
        };
        for request in requests {
            sit(host, dimension, &request, now);
        }
        if now % SWEEP != 0 {
            return;
        }
        // Old seats.
        let seats: Vec<Seat> = match BOOK.lock() {
            Ok(book) => book.seats.iter().filter(|s| s.dimension == dimension).map(Seat::copy).collect(),
            Err(_) => return,
        };
        for seat in seats {
            let block_is_seat = rules::colour_of_block(&host.block(seat.pos)).is_some();
            let rider = rider(host, &seat);
            if rules::keep(now - seat.made, block_is_seat, rider) {
                continue;
            }
            if rider == Rider::Riding
                && let Some(entity) = host
                    .server_players()
                    .iter()
                    .find(|p| p.uuid == seat.player)
                    .and_then(|p| u32::try_from(p.entity_id).ok())
            {
                host.dismount(entity);
            }
            host.remove_entity(dimension, seat.id);
            if let Ok(mut book) = BOOK.lock() {
                book.seats.retain(|s| s.id != seat.id);
            }
        }
    }
}

/// Makes the seat entity and puts the player on it, replacing one already on that block.
fn sit(host: &mut dyn Host, dimension: &str, request: &Request, now: i64) {
    if rules::colour_of_block(&host.block(request.pos)).is_none() {
        return;
    }
    // `seatEntity.ejectPassengers()` then the new rider: an old marker on this block goes.
    for id in seats_at(dimension, request.pos).iter().map(|s| s.id) {
        host.remove_entity(dimension, id);
        if let Ok(mut book) = BOOK.lock() {
            book.seats.retain(|s| s.id != id);
        }
    }
    let n = BOOK
        .lock()
        .map(|mut book| {
            book.made += 1;
            book.made
        })
        .unwrap_or(1);
    let uuid = rules::marker_uuid(n);
    let mut nbt = Compound::new();
    nbt.insert("Tags".into(), Tag::List(vec![Tag::String("create:seat".into())]));
    let Some(id) = host.summon_with_uuid(dimension, MARKER, rules::marker_position(request.pos), nbt, uuid) else {
        return;
    };
    if !host.run_command(&rules::ride_command(dimension, request.player, uuid)) {
        host.remove_entity(dimension, id);
        return;
    }
    if let Ok(mut book) = BOOK.lock() {
        book.seats.push(Seat {
            dimension: dimension.to_owned(),
            pos: request.pos,
            player: request.player,
            id,
            made: now,
        });
    }
}
