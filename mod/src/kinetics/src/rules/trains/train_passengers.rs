//! Who sits where on a carriage: the passengers of `CarriageContraptionEntity` and the conductors among them
//! (`checkConductors`), `Train.countPlayerPassengers`, `SeatBlock.canBePickedUp`,
//! `SeatInteractionBehaviour.handleEntityCollision` and `AbstractContraptionEntity.addSittingPassenger`.
//!
//! A carriage here has no entity, so its passengers are what `seat.rs` keeps (a marker on each seat, a rider on
//! the marker). The glue lists them as [`Passenger`]s (the carriage seat they sit on and whether they are a player);
//! the rules answer which sides of the carriage a driver is present for, and whether a mob walking over a seat sits on it.
//!
//! `SeatBlock.getLeashed` is [`first_leashed`]: a player's click on a seat seats a mob they lead, if one is within
//! [`LEASH_SEARCH`] blocks, in the player's place.
//!
//! Left out: the config `seatHostileMobs` (always on, its default: every mob is picked up unless it is a Shulker or in
//! `#c:capturing_not_supported`), and `TamableAnimal.setInSittingPose`.

use crate::Pos;
use std::collections::BTreeMap;

/// Someone on a seat of the carriage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Passenger {
    /// The seat's local position (`contraption.getSeatOf(uuid)`).
    pub seat: Pos,
    pub player: bool,
}

/// `CarriageContraptionEntity.checkConductors`: `(forward, backward)` for a carriage whose controls found a
/// block conductor on each side (`blockConductors`, a Blaze Burner next to the controls) and whose seats lead to
/// the controls (`conductorSeats`). Players don't count (they drive with the controls, not as conductors); a
/// passenger on a seat that isn't in front of controls adds nothing.
pub fn check_conductors(block_conductors: [bool; 2], conductor_seats: &BTreeMap<Pos, [bool; 2]>, passengers: &[Passenger]) -> [bool; 2] {
    let mut sides = block_conductors;
    for passenger in passengers {
        if passenger.player {
            continue;
        }
        let Some(valid) = conductor_seats.get(&passenger.seat) else { continue };
        sides[0] |= valid[0];
        sides[1] |= valid[1];
    }
    sides
}

/// `Train.countPlayerPassengers` for one carriage: the players seated on it.
pub fn count_player_passengers(passengers: &[Passenger]) -> i32 {
    passengers.iter().filter(|p| p.player).count() as i32
}

/// What `SeatBlock.canBePickedUp` looks at in an entity.
#[derive(Clone, Copy, Debug)]
pub struct Candidate<'a> {
    /// The entity type's id.
    pub kind: &'a str,
    pub player: bool,
    /// `instanceof LivingEntity`.
    pub living: bool,
    /// `AllEntityTags.IGNORE_SEAT` (`#c:capturing_not_supported`) matches it. NeoForge's tag isn't in the prepared
    /// data, so the glue passes `false`: whatever that tag names (the bosses) is picked up here when it comes near.
    pub ignored: bool,
}

/// `SeatBlock.canBePickedUp`.
pub fn can_be_picked_up(c: &Candidate<'_>) -> bool {
    if c.kind == "minecraft:shulker" || c.player || c.ignored {
        return false;
    }
    c.living
}

/// What happens when something sits on a seat that may hold someone already
/// (`AbstractContraptionEntity.addSittingPassenger`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seating {
    /// Nothing changes: a player is on the seat, or a mob would take a mob's place.
    Refuse,
    /// The new passenger sits; the old one (if any) is put off (`entity.stopRiding()`).
    Sit { eject: bool },
}

/// `occupant`: `Some(is_player)` for whoever sits on that seat already.
pub fn add_sitting_passenger(occupant: Option<bool>, newcomer_is_player: bool) -> Seating {
    match occupant {
        None => Seating::Sit { eject: false },
        Some(true) => Seating::Refuse,
        Some(false) if !newcomer_is_player => Seating::Refuse,
        Some(false) => Seating::Sit { eject: true },
    }
}

/// How far from a seat's centre a mob's feet must be for the seat's block to touch it
/// (`handleEntityCollision` is called for an entity whose box meets the block's collision shape): the cell's
/// half width and a mob's half width.
pub const TOUCH_HORIZONTAL: f64 = 0.5 + 0.3;
/// A seat's collision is its lower 6/16 (`AllShapes.SEAT_COLLISION`): feet from a little under the cell to the top of that.
pub const TOUCH_BELOW: f64 = 0.5 + 0.1;
pub const TOUCH_ABOVE: f64 = -0.5 + 6.0 / 16.0;

/// Whether a mob standing with its feet at `feet` meets the seat whose centre is `centre`. An approximation of the
/// box test (`handleEntityCollision` is called with the colliding entity): Create runs it from the contraption's
/// entity collision, which does not exist here.
pub fn touches_seat(centre: [f64; 3], feet: [f64; 3]) -> bool {
    let dx = (feet[0] - centre[0]).abs();
    let dz = (feet[2] - centre[2]).abs();
    let dy = feet[1] - centre[1];
    dx <= TOUCH_HORIZONTAL && dz <= TOUCH_HORIZONTAL && dy >= -TOUCH_BELOW && dy <= TOUCH_ABOVE
}

/// `SeatBlock.getLeashed` looks at the entities in `player.getBoundingBox().inflate(10)`.
pub const LEASH_SEARCH: f64 = 10.0;

/// The box `getLeashed` searches: the player's box, grown by [`LEASH_SEARCH`] on every side.
pub fn leash_search_box(player_box: [[f64; 3]; 2]) -> [[f64; 3]; 2] {
    [player_box[0].map(|c| c - LEASH_SEARCH), player_box[1].map(|c| c + LEASH_SEARCH)]
}

/// One entity of that box, as `getLeashed` tests it.
#[derive(Clone, Copy, Debug)]
pub struct LeashCandidate<'a> {
    /// `e instanceof Mob`: only a mob takes a lead.
    pub mob: bool,
    /// `mob.getLeashHolder() == player`.
    pub led_by_player: bool,
    pub candidate: Candidate<'a>,
}

/// `SeatBlock.getLeashed`: the first entity (in the world's order) that is a mob led by the player and can be picked up
/// (`canBePickedUp`). Its index in `entities`.
pub fn first_leashed(entities: &[LeashCandidate<'_>]) -> Option<usize> {
    entities.iter().position(|e| e.mob && e.led_by_player && can_be_picked_up(&e.candidate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lead_search_grows_the_players_box_by_ten() {
        let b = leash_search_box([[0.0, 64.0, 0.0], [0.6, 65.8, 0.6]]);
        assert_eq!(b, [[-10.0, 54.0, -10.0], [10.6, 75.8, 10.6]]);
    }

    #[test]
    fn a_led_mob_that_can_be_picked_up_is_seated_in_the_players_place() {
        let cow = Candidate { kind: "minecraft:cow", player: false, living: true, ignored: false };
        let led = |mob, led_by_player, candidate| LeashCandidate { mob, led_by_player, candidate };
        // Not led by the player (or by anyone): skipped, and the first one that is led wins.
        let list = [
            led(true, false, cow),
            led(true, true, Candidate { kind: "minecraft:shulker", ..cow }),
            led(true, true, Candidate { ignored: true, ..cow }),
            led(false, true, cow),
            led(true, true, cow),
            led(true, true, Candidate { kind: "minecraft:horse", ..cow }),
        ];
        assert_eq!(first_leashed(&list), Some(4));
        assert_eq!(first_leashed(&list[..4]), None);
        assert_eq!(first_leashed(&[]), None);
    }

    fn seats() -> BTreeMap<Pos, [bool; 2]> {
        let mut m = BTreeMap::new();
        m.insert([1, 1, 0], [true, false]);
        m.insert([3, 1, 0], [false, true]);
        m
    }

    #[test]
    fn an_empty_carriage_has_no_conductor() {
        assert_eq!(check_conductors([false, false], &seats(), &[]), [false, false]);
    }

    #[test]
    fn a_mob_on_a_conductor_seat_is_a_conductor_for_its_sides() {
        let p = [Passenger { seat: [1, 1, 0], player: false }];
        assert_eq!(check_conductors([false, false], &seats(), &p), [true, false]);
        let both = [Passenger { seat: [1, 1, 0], player: false }, Passenger { seat: [3, 1, 0], player: false }];
        assert_eq!(check_conductors([false, false], &seats(), &both), [true, true]);
    }

    #[test]
    fn a_player_is_not_a_conductor_and_a_plain_seat_adds_nothing() {
        let p = [Passenger { seat: [1, 1, 0], player: true }, Passenger { seat: [9, 1, 0], player: false }];
        assert_eq!(check_conductors([false, false], &seats(), &p), [false, false]);
    }

    #[test]
    fn a_block_conductor_counts_without_a_rider() {
        assert_eq!(check_conductors([false, true], &seats(), &[]), [false, true]);
        let p = [Passenger { seat: [1, 1, 0], player: false }];
        assert_eq!(check_conductors([false, true], &seats(), &p), [true, true]);
    }

    #[test]
    fn players_are_counted() {
        let p = [
            Passenger { seat: [1, 1, 0], player: true },
            Passenger { seat: [2, 1, 0], player: false },
            Passenger { seat: [3, 1, 0], player: true },
        ];
        assert_eq!(count_player_passengers(&p), 2);
    }

    #[test]
    fn who_is_picked_up() {
        let mob = Candidate { kind: "minecraft:villager", player: false, living: true, ignored: false };
        assert!(can_be_picked_up(&mob));
        assert!(!can_be_picked_up(&Candidate { kind: "minecraft:shulker", ..mob }));
        assert!(!can_be_picked_up(&Candidate { player: true, kind: "minecraft:player", ..mob }));
        assert!(!can_be_picked_up(&Candidate { ignored: true, ..mob }));
        assert!(!can_be_picked_up(&Candidate { living: false, kind: "minecraft:item", ..mob }));
    }

    #[test]
    fn sitting_down_follows_who_is_there() {
        assert_eq!(add_sitting_passenger(None, false), Seating::Sit { eject: false });
        // A player on the seat is never replaced.
        assert_eq!(add_sitting_passenger(Some(true), true), Seating::Refuse);
        assert_eq!(add_sitting_passenger(Some(true), false), Seating::Refuse);
        // A mob gives way to a player, not to another mob.
        assert_eq!(add_sitting_passenger(Some(false), true), Seating::Sit { eject: true });
        assert_eq!(add_sitting_passenger(Some(false), false), Seating::Refuse);
    }

    #[test]
    fn a_mob_standing_on_the_seat_touches_it_and_one_beside_it_does_not() {
        let seat = [10.5, 65.5, 4.5];
        assert!(touches_seat(seat, [10.5, 65.0, 4.5]));
        assert!(touches_seat(seat, [10.9, 65.0, 4.5]));
        assert!(!touches_seat(seat, [11.5, 65.0, 4.5]));
        // Far above or below the seat.
        assert!(!touches_seat(seat, [10.5, 66.5, 4.5]));
        assert!(!touches_seat(seat, [10.5, 64.0, 4.5]));
    }
}
