//! The Redstone Contact: two facing each other power each other, and one on a moving structure
//! gives a signal to the contact it passes (`RedstoneContactBlock`, `RedstoneContactItem`,
//! `ContactMovementBehaviour`). Plain values: the glue reads the blocks and applies the answer.

use crate::{Dir, Pos};

pub const ID: &str = "create:redstone_contact";
/// The Elevator Contact also counts as a contact to face (`hasValidContact`).
pub const ELEVATOR_ID: &str = "create:elevator_contact";

/// What stands in front of a contact: a contact block and the way it faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Front {
    pub is_contact: bool,
    pub facing: Option<Dir>,
}

/// `RedstoneContactBlock.hasValidContact`: the block in front of a contact facing `direction` is a
/// contact facing back at it.
pub fn has_valid_contact(front: Front, direction: Dir) -> bool {
    front.is_contact && front.facing == Some(direction.opposite())
}

/// `RedstoneContactBlock.getStateForPlacement`: `(facing, powered)`. `looking` is the nearest
/// looking direction, `clicked` the face clicked; `valid` says whether a contact faces back from the
/// cell in front in a direction.
pub fn placement(looking: Dir, clicked: Dir, sneaking: bool, valid: &dyn Fn(Dir) -> bool) -> (Dir, bool) {
    let mut facing = looking.opposite();
    let toward = clicked.opposite();
    if sneaking || valid(toward) {
        facing = toward;
    }
    (facing, valid(facing))
}

/// `RedstoneContactBlock.getSignal`: 15 on every side but the one it faces, while powered. `side` is
/// Java's side (the one the reader is on, seen from the contact: the opposite of the face that touches
/// the reader).
pub fn signal(powered: bool, facing: Dir, side: Dir) -> i32 {
    if powered && side != facing.opposite() { 15 } else { 0 }
}

/// `RedstoneContactBlock.isSignalSource`: `state.getValue(POWERED)`. Redstone wire beside an unpowered
/// contact does not connect to it.
pub fn is_signal_source(powered: bool) -> bool {
    powered
}

/// `ContactMovementBehaviour.visitNewPosition`'s decision.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Visit {
    /// The contact the moving one let go of: it is no longer held powered.
    pub release: Option<Pos>,
    /// The contact to power and hold.
    pub power: Option<Pos>,
    /// The `lastContact` to remember from now.
    pub last: Option<Pos>,
}

/// What a contact on a structure does on entering the cell `pos`. `first_movement` is the context's
/// flag; `own` the way the contact faces now (turned with the structure, to the nearest direction);
/// `visited` the contact in the cell, if it is one, with whether it is a Redstone Contact (an Elevator
/// Contact is told of the cabin by the elevator, not powered here) and its facing.
pub fn visit(first_movement: bool, last: Option<Pos>, pos: Pos, own: Dir, visited: Option<(bool, Dir)>) -> Visit {
    if first_movement {
        return Visit { last, ..Visit::default() };
    }
    // `deactivateLastVisitedContact`.
    let mut out = Visit { release: last, power: None, last: None };
    let Some((redstone, facing)) = visited else { return out };
    if facing != own.opposite() {
        return out;
    }
    if redstone {
        out.power = Some(pos);
    }
    out.last = Some(pos);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contact(facing: Dir) -> Front {
        Front { is_contact: true, facing: Some(facing) }
    }

    #[test]
    fn two_contacts_facing_each_other_are_valid() {
        assert!(has_valid_contact(contact(Dir::West), Dir::East));
        assert!(!has_valid_contact(contact(Dir::East), Dir::East));
        assert!(!has_valid_contact(Front { is_contact: false, facing: Some(Dir::West) }, Dir::East));
    }

    #[test]
    fn placed_it_faces_away_from_the_player_or_the_wall_it_is_on() {
        let none = |_: Dir| false;
        // Looking north: faces south, the opposite of what is looked at.
        assert_eq!(placement(Dir::North, Dir::South, false, &none), (Dir::South, false));
        // Sneaking: against the face clicked.
        assert_eq!(placement(Dir::North, Dir::Up, true, &none), (Dir::Down, false));
        // A contact there to face: against it, and powered.
        let valid = |d: Dir| d == Dir::Down;
        assert_eq!(placement(Dir::North, Dir::Up, false, &valid), (Dir::Down, true));
    }

    #[test]
    fn it_is_a_signal_source_only_while_powered() {
        assert!(is_signal_source(true));
        assert!(!is_signal_source(false));
    }

    /// The in-game pair (east-facing and west-facing, lamps above and north of both): every lamp
    /// reads 15 from both contacts, the lamp's side being `Down`/`South` (it reads towards the
    /// contact), so a lamp that stays dark was never told to look (a `setblock`'d lamp isn't).
    #[test]
    fn a_facing_pair_lights_lamps_above_and_beside_both() {
        for facing in [Dir::East, Dir::West] {
            for side in [Dir::Down, Dir::South] {
                assert_eq!(signal(true, facing, side), 15, "{facing:?} read from {side:?}");
            }
        }
        // Only the side it faces through (the reader in front, looking back) is dark.
        assert_eq!(signal(true, Dir::East, Dir::West), 0);
        assert_eq!(signal(true, Dir::West, Dir::East), 0);
    }

    #[test]
    fn it_powers_every_side_but_the_one_it_faces_through() {
        assert_eq!(signal(true, Dir::North, Dir::East), 15);
        assert_eq!(signal(true, Dir::North, Dir::North), 15);
        assert_eq!(signal(true, Dir::North, Dir::South), 0);
        assert_eq!(signal(false, Dir::North, Dir::East), 0);
    }

    #[test]
    fn a_moving_contact_powers_the_one_it_faces_and_lets_go_of_the_last() {
        // The first movement does nothing, and keeps what it had.
        assert_eq!(
            visit(true, Some([9, 9, 9]), [1, 0, 0], Dir::East, Some((true, Dir::West))),
            Visit { release: None, power: None, last: Some([9, 9, 9]) }
        );
        // A contact facing back: power it and remember it; the old one is let go.
        let v = visit(false, Some([0, 0, 0]), [1, 0, 0], Dir::East, Some((true, Dir::West)));
        assert_eq!(v, Visit { release: Some([0, 0, 0]), power: Some([1, 0, 0]), last: Some([1, 0, 0]) });
        // One facing the wrong way, or no contact: only the old one is let go.
        let v = visit(false, Some([0, 0, 0]), [1, 0, 0], Dir::East, Some((true, Dir::East)));
        assert_eq!(v, Visit { release: Some([0, 0, 0]), power: None, last: None });
        let v = visit(false, None, [1, 0, 0], Dir::East, None);
        assert_eq!(v, Visit::default());
        // An Elevator Contact is remembered but not powered here.
        let v = visit(false, None, [1, 0, 0], Dir::East, Some((false, Dir::West)));
        assert_eq!(v, Visit { release: None, power: None, last: Some([1, 0, 0]) });
    }
}
