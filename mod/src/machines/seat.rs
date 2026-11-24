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
//! - `SeatInteractionBehaviour` (clicking a seat that is part of a structure), the seat's
//!   `ENTITY_NAME` display source and waterlogging are not here.
//!
//! # Riders on moving structures
//!
//! `Contraption.getSeats` / `AbstractContraptionEntity.addSittingPassenger`: a player sitting on a
//! seat that a piston, bearing, pulley, gantry or elevator picks up stays seated and goes with it.
//! The marker the rider sits on is kept (not discarded when its block vanishes) and each tick of
//! the controller's actors (`actor::tick`, which calls [`carry`]) it is teleported to where the
//! seat block is (the structure's pose, turned), the rider following it as a passenger. When the
//! structure is set down (`actor::stop` calls [`release`]) the marker is put on the nearest free
//! seat block that now stands there (`rules::settle_cell`) and the rider stays sitting; with none
//! the rider is dismounted. Differences: the rider moves in 20 Hz steps (the marker is teleported
//! each tick; the structure is drawn smoothly), stays upright on a turning structure, and mobs
//! don't sit on seats in this port at all. A structure assembled from under a seated player is
//! matched by the seat's cell (`anchor + local`). Not played.
//! - The seat's shape: its outline is Create's 8/16, its collision 6/16, and 3/16 for a player
//!   (`AllShapes.SEAT*`).

use std::sync::Mutex;

use create_kinetics::rules::seat::{self as rules, Click, Rider};
use create_kinetics::Pos;
use crate::compat::{Compound, Flow, Hooks, Host, LevelTickPostEvent, RightClickBlockEvent, Tag};

use crate::machines::encased;
use super::portable_interface_motion::Pose;
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
    /// A seat of a moving structure (`controller`, `local`) and where its marker goes now.
    moving: Option<(Pos, Pos, [f64; 3])>,
}

struct Seat {
    dimension: String,
    pos: Pos,
    player: u128,
    id: u32,
    made: i64,
    carry: Carry,
    /// Where the marker was last put while carried.
    last: [f64; 3],
    /// Since when the seat's block isn't there (0: it is).
    missing: i64,
}

/// What carries a seat's marker.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Carry {
    /// The seat block stands in the world.
    No,
    /// The seat is part of the structure of this controller, at this local position.
    Moving { controller: Pos, local: Pos },
    /// The structure was set down at the game time (-1: not stamped yet); the marker looks for the
    /// seat block it landed on.
    Released(i64),
}

/// A seat whose block went (a structure picked it up) keeps its marker this long to be claimed.
const MISSING_TICKS: i64 = 40;
/// How long a released marker looks for its seat block.
const SETTLE_TICKS: i64 = 20;

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
                    book.pending.push(Request { dimension: dimension.to_owned(), pos, player: player.uuid, moving: None });
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
        settle_released(host, dimension, now);
        if now % SWEEP != 0 {
            return;
        }
        // Old seats.
        let seats: Vec<Seat> = match BOOK.lock() {
            Ok(book) => book.seats.iter().filter(|s| s.dimension == dimension).map(Seat::copy).collect(),
            Err(_) => return,
        };
        for seat in seats {
            let mut block_is_seat = rules::colour_of_block(&host.block(seat.pos)).is_some();
            if matches!(seat.carry, Carry::Moving { .. } | Carry::Released(_)) {
                // On a structure: its block isn't in the world (a released one is judged by
                // `settle_released`).
                block_is_seat = true;
            } else if block_is_seat {
                if seat.missing != 0 {
                    update(seat.id, |s| s.missing = 0);
                }
            } else {
                // A structure may be about to claim it (`carry`).
                let since = if seat.missing == 0 {
                    update(seat.id, |s| s.missing = now);
                    now
                } else {
                    seat.missing
                };
                block_is_seat = now - since < MISSING_TICKS;
            }
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
    if request.moving.is_none() && rules::colour_of_block(&host.block(request.pos)).is_none() {
        return;
    }
    // `seatEntity.ejectPassengers()` then the new rider: an old marker on this block goes.
    let old: Vec<u32> = match request.moving {
        Some((controller, local, _)) => BOOK
            .lock()
            .map(|book| {
                book.seats
                    .iter()
                    .filter(|s| s.dimension == dimension && s.carry == Carry::Moving { controller, local })
                    .map(|s| s.id)
                    .collect()
            })
            .unwrap_or_default(),
        None => seats_at(dimension, request.pos).iter().map(|s| s.id).collect(),
    };
    for id in old {
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
    let at = request.moving.map_or_else(|| rules::marker_position(request.pos), |(_, _, at)| at);
    let mut nbt = Compound::new();
    nbt.insert("Tags".into(), Tag::List(vec![Tag::String("create:seat".into())]));
    let Some(id) = host.summon_with_uuid(dimension, MARKER, at, nbt, uuid) else {
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
            carry: request.moving.map_or(Carry::No, |(controller, local, _)| Carry::Moving { controller, local }),
            last: at,
            missing: 0,
        });
    }
}

fn update(id: u32, f: impl FnOnce(&mut Seat)) {
    if let Ok(mut book) = BOOK.lock()
        && let Some(seat) = book.seats.iter_mut().find(|s| s.id == id)
    {
        f(seat);
    }
}

/// A structure's tick (`AbstractContraptionEntity.positionRider`): the seats among its blocks
/// (`seats`, local positions) have their riders' markers put where the blocks are at game time `now`.
pub(super) fn carry(host: &mut dyn Host, controller: Pos, anchor: Pos, seats: &[Pos], pose: &Pose<'_>, now: i64) {
    let points: Vec<(Pos, [f64; 3])> = seats
        .iter()
        .map(|local| {
            let centre = pose.centre(now as f64, *local);
            let lift = pose.rotate(now as f64, rules::carried_lift());
            (*local, [centre[0] + lift[0], centre[1] + lift[1], centre[2] + lift[2]])
        })
        .collect();
    carry_points(host, controller, Some(anchor), &points);
}

/// Puts the riders' markers of the seats of the structure known as `controller` (`points`: each seat's local position
/// and where its marker goes). A seat a player sat on before the structure was assembled is found by its cell
/// (`anchor` plus local) when `anchor` is given. Trains key their carriages by `trains::seat_key`, not a block.
pub(super) fn carry_points(host: &mut dyn Host, controller: Pos, anchor: Option<Pos>, points: &[(Pos, [f64; 3])]) {
    let dimension = host.dimension().to_owned();
    for (local, at) in points {
        let cell = anchor.map(|a| [a[0] + local[0], a[1] + local[1], a[2] + local[2]]);
        let found = BOOK.lock().ok().and_then(|book| {
            book.seats
                .iter()
                .find(|s| {
                    s.dimension == dimension
                        && (s.carry == Carry::Moving { controller, local: *local }
                            || (s.carry == Carry::No && Some(s.pos) == cell))
                })
                .map(|s| s.id)
        });
        let Some(id) = found else { continue };
        let at = *at;
        update(id, |s| {
            s.carry = Carry::Moving { controller, local: *local };
            s.last = at;
            s.missing = 0;
        });
        if let Some(entity) = host.entity(id) {
            host.entity_teleport_to(entity, at, None);
        }
    }
}

/// The structure of `controller` is being set down: its riders' markers go looking for the seat
/// blocks it places (`settle_released`, from the level tick).
pub(super) fn release(host: &dyn Host, controller: Pos) {
    let dimension = host.dimension();
    if let Ok(mut book) = BOOK.lock() {
        for seat in book.seats.iter_mut().filter(|s| s.dimension == dimension) {
            if matches!(seat.carry, Carry::Moving { controller: c, .. } if c == controller) {
                seat.carry = Carry::Released(-1);
            }
        }
    }
}

/// Puts the markers of a structure that was set down on the seat blocks it left, or takes their
/// riders off.
fn settle_released(host: &mut dyn Host, dimension: &str, now: i64) {
    let released: Vec<Seat> = match BOOK.lock() {
        Ok(book) => book
            .seats
            .iter()
            .filter(|s| s.dimension == dimension && matches!(s.carry, Carry::Released(_)))
            .map(Seat::copy)
            .collect(),
        Err(_) => return,
    };
    for seat in released {
        let Carry::Released(since) = seat.carry else { continue };
        if since < 0 {
            update(seat.id, |s| s.carry = Carry::Released(now));
            continue; // The controller places the blocks in its tick; look from the next tick.
        }
        let taken: Vec<Pos> = BOOK
            .lock()
            .map(|book| {
                book.seats.iter().filter(|s| s.dimension == dimension && s.carry == Carry::No).map(|s| s.pos).collect()
            })
            .unwrap_or_default();
        let base = seat.last.map(|c| c.floor() as i32);
        let mut cells = Vec::new();
        for dx in -2..=2 {
            for dy in -2..=2 {
                for dz in -2..=2 {
                    let cell = [base[0] + dx, base[1] + dy, base[2] + dz];
                    if !taken.contains(&cell) && rules::colour_of_block(&host.block(cell)).is_some() {
                        cells.push(cell);
                    }
                }
            }
        }
        if let Some(cell) = rules::settle_cell(seat.last, &cells) {
            update(seat.id, |s| {
                s.carry = Carry::No;
                s.pos = cell;
                s.missing = 0;
                s.made = now;
            });
            if let Some(entity) = host.entity(seat.id) {
                host.entity_teleport_to(entity, rules::marker_position(cell), None);
            }
        } else if now - since >= SETTLE_TICKS {
            // Nowhere to sit: `Contraption.removePassengers`.
            if let Some(entity) = host
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

/// `AbstractContraptionEntity.handlePlayerInteraction` for a seat: the player sits on the seat at
/// `local` of the structure of `controller`, whose cell was `cell` (anchor plus local) when it was
/// assembled and whose marker goes to `at`. False when the seat has a player on it already, or the
/// player rides something (`player.isPassenger()`).
pub(super) fn sit_on_structure(
    host: &dyn Host,
    dimension: &str,
    controller: Pos,
    local: Pos,
    cell: Pos,
    player: u128,
    at: [f64; 3],
) -> bool {
    let players = host.server_players();
    let Some(me) = players.iter().find(|p| p.uuid == player) else { return false };
    if u32::try_from(me.entity_id).ok().and_then(|id| host.vehicle_of(id)).is_some() {
        return false;
    }
    let seats: Vec<Seat> = BOOK
        .lock()
        .map(|book| {
            book.seats
                .iter()
                .filter(|s| s.dimension == dimension && s.carry == Carry::Moving { controller, local })
                .map(Seat::copy)
                .collect()
        })
        .unwrap_or_default();
    if seats.iter().any(|s| rider(host, s) == Rider::Riding) {
        return false;
    }
    if let Ok(mut book) = BOOK.lock() {
        book.pending.retain(|r| r.player != player);
        book.pending.push(Request { dimension: dimension.to_owned(), pos: cell, player, moving: Some((controller, local, at)) });
    }
    true
}
