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
//! - Mobs that walk or fall onto a seat don't sit (`updateEntityAfterFallOn`): the hook for that doesn't exist. A
//!   player's click seats the player, or a mob they lead (`SeatBlock.getLeashed`: [`leashed_by`]).
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
use create_kinetics::rules::trains::train_passengers::{Candidate, LeashCandidate, first_leashed, leash_search_box};
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
    /// Who rides: a player's uuid, or a mob's (`mob` is then its entity id).
    player: u128,
    /// A mob seated by a carriage (`SeatInteractionBehaviour.handleEntityCollision`), by entity id.
    mob: Option<u32>,
    /// A seat of a moving structure (`controller`, `local`) and where its marker goes now.
    moving: Option<(Pos, Pos, [f64; 3])>,
}

struct Seat {
    dimension: String,
    pos: Pos,
    player: u128,
    /// The rider is a mob, by entity id.
    mob: Option<u32>,
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
    if let Some(mob) = seat.mob {
        // A mob that is gone (killed, unloaded) rides nothing.
        return match host.vehicle_of(mob) {
            Some(vehicle) if vehicle == seat.id => Rider::Riding,
            _ => Rider::Off,
        };
    }
    let players = host.server_players();
    let Some(player) = players.iter().find(|p| p.uuid == seat.player) else {
        return Rider::Unknown;
    };
    match u32::try_from(player.entity_id).ok().and_then(|id| host.vehicle_of(id)) {
        Some(vehicle) if vehicle == seat.id => Rider::Riding,
        _ => Rider::Off,
    }
}

/// The rider's entity id: a mob's, or the player's.
fn rider_entity(host: &dyn Host, seat: &Seat) -> Option<u32> {
    seat.mob.or_else(|| {
        host.server_players().iter().find(|p| p.uuid == seat.player).and_then(|p| u32::try_from(p.entity_id).ok())
    })
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

/// `SeatBlock.getLeashed`: the first mob within ten blocks of the player that is on the player's lead and can be picked
/// up, as its entity id and uuid. `player_entity` is the player's entity id.
pub(super) fn leashed_by(host: &mut dyn Host, player: u128, player_entity: i32) -> Option<(u32, u128)> {
    let _ = player;
    let me = u32::try_from(player_entity).ok()?;
    let player_ref = host.entity(me)?;
    let [lo, hi] = leash_search_box(host.entity_bounding_box(player_ref)?);
    let dimension = host.dimension().to_owned();
    let mut found: Vec<(u32, String, bool, bool)> = Vec::new();
    for id in host.entities_in(&dimension, lo, hi) {
        let Some(entity) = host.entity(id) else { continue };
        let Some(kind) = host.entity_kind(entity) else { continue };
        // `mob.getLeashHolder() == player`: the holder is read from the game's own lead.
        let led = matches!(entity.leash_holder(host.new_host()), Some(mod_api::LeashHolder::Entity(holder)) if holder.id() == me);
        found.push((id, kind.clone(), led, host.living(entity).is_some()));
    }
    let candidates: Vec<LeashCandidate<'_>> = found
        .iter()
        .map(|(_, kind, led, living)| LeashCandidate {
            // `instanceof Mob`: a boat takes a lead too but is not living, and a player or armor stand is never led.
            mob: *living && kind != "minecraft:player" && kind != "minecraft:armor_stand",
            led_by_player: *led,
            candidate: Candidate { kind, player: kind == "minecraft:player", living: *living, ignored: false },
        })
        .collect();
    let index = first_leashed(&candidates)?;
    let id = found[index].0;
    let uuid = host.entity(id).and_then(|e| host.entity_uuid(e))?;
    Some((id, uuid))
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
                // `sitDown(level, pos, getLeashed(level, player).or(player))`: a mob on the player's lead sits instead.
                let who = leashed_by(host, player.uuid, player.entity_id).map_or((player.uuid, None), |(id, uuid)| (uuid, Some(id)));
                if let Ok(mut book) = BOOK.lock()
                    && !book.pending.iter().any(|r| r.player == who.0 && r.pos == pos)
                {
                    book.pending.push(Request { dimension: dimension.to_owned(), pos, player: who.0, mob: who.1, moving: None });
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
                && let Some(entity) = rider_entity(host, &seat)
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
            mob: request.mob,
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
            if let Some(entity) = rider_entity(host, &seat) {
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
    // A player on the seat keeps it; a mob gives way to a player (`addSittingPassenger`: `entity.stopRiding()`).
    if seats.iter().any(|s| s.mob.is_none() && rider(host, s) == Rider::Riding) {
        return false;
    }
    if let Ok(mut book) = BOOK.lock() {
        book.pending.retain(|r| r.player != player);
        book.pending.push(Request { dimension: dimension.to_owned(), pos: cell, player, mob: None, moving: Some((controller, local, at)) });
    }
    true
}

/// Who sits on the seats of the structure known as `controller` (a carriage: `trains::seat_key`) now: the seat's local
/// position, the rider's uuid and whether it is a player (`AbstractContraptionEntity.getPassengers` with
/// `getSeatOf`).
pub(super) fn passengers_of(host: &dyn Host, dimension: &str, controller: Pos) -> Vec<(Pos, u128, bool)> {
    let seats: Vec<Seat> = BOOK
        .lock()
        .map(|book| {
            book.seats
                .iter()
                .filter(|s| s.dimension == dimension && matches!(s.carry, Carry::Moving { controller: c, .. } if c == controller))
                .map(Seat::copy)
                .collect()
        })
        .unwrap_or_default();
    seats
        .iter()
        .filter(|s| rider(host, s) == Rider::Riding)
        .filter_map(|s| match s.carry {
            Carry::Moving { local, .. } => Some((local, s.player, s.mob.is_none())),
            _ => None,
        })
        .collect()
}

/// [`passengers_of`] with the riders' entity ids instead of uuids: the seat's local position, the entity id and whether it is a
/// player (the Train Hat goes to the clients by entity id).
pub(super) fn riders_with_ids(host: &dyn Host, dimension: &str, controller: Pos) -> Vec<(Pos, u32, bool)> {
    let seats: Vec<Seat> = BOOK
        .lock()
        .map(|book| {
            book.seats
                .iter()
                .filter(|s| s.dimension == dimension && matches!(s.carry, Carry::Moving { controller: c, .. } if c == controller))
                .map(Seat::copy)
                .collect()
        })
        .unwrap_or_default();
    seats
        .iter()
        .filter(|s| rider(host, s) == Rider::Riding)
        .filter_map(|s| match s.carry {
            Carry::Moving { local, .. } => Some((local, rider_entity(host, s)?, s.mob.is_none())),
            _ => None,
        })
        .collect()
}

/// `AbstractContraptionEntity.addSittingPassenger` for a mob: `mob` (entity id and uuid) sits on the seat at `local`

/// of the structure of `controller`, whose marker goes to `at`. A mob already on that seat is not replaced, a player
/// on it is not either (`rules::trains::train_passengers::add_sitting_passenger`); a mob that rides something is left
/// alone. False when nothing was done.
#[allow(clippy::too_many_arguments)]
pub(super) fn sit_mob_on_structure(
    host: &dyn Host,
    dimension: &str,
    controller: Pos,
    local: Pos,
    cell: Pos,
    mob: u32,
    mob_uuid: u128,
    at: [f64; 3],
) -> bool {
    if host.vehicle_of(mob).is_some() {
        return false;
    }
    let occupant = passengers_of(host, dimension, controller).into_iter().find(|(l, _, _)| *l == local).map(|(_, _, player)| player);
    // A mob never takes a seat from anyone: only a player replaces a mob, and that is the player's own click.
    if create_kinetics::rules::trains::train_passengers::add_sitting_passenger(occupant, false)
        != (create_kinetics::rules::trains::train_passengers::Seating::Sit { eject: false })
    {
        return false;
    }
    if let Ok(mut book) = BOOK.lock() {
        if book.pending.iter().any(|r| r.mob == Some(mob) || matches!(r.moving, Some((c, l, _)) if c == controller && l == local)) {
            return false;
        }
        book.pending.push(Request { dimension: dimension.to_owned(), pos: cell, player: mob_uuid, mob: Some(mob), moving: Some((controller, local, at)) });
    }
    true
}
