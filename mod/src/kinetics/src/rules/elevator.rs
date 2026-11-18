//! The Elevator Pulley and the Elevator Contacts' rules: the column of floors, how a contact calls
//! the cabin, how the cabin accelerates to the floor, which cabin may start. Ported from Create
//! (MIT, the Creators of Create), `mc1.21.1/dev` at 4148fd8: `ElevatorColumn` (`add`, `remove`,
//! `target`, `contactAt`, `initNames`), `ElevatorContactBlock` (`neighborChanged`,
//! `callToContactAndUpdate`, `getSignal`, `getColumnCoords`), `ElevatorPulleyBlockEntity`
//! (`getMovementSpeed`, `getTargetOffset`, `assemble`), `ElevatorContraption` (`capture`,
//! `assemble`, `isTargetUnreachable`) and `AllShapes.ELEVATOR_PULLEY`.
//!
//! No game types: the glue is `native/api/src/machines/elevator.rs`.
//!
//! Known differences from Create:
//! - The cabin's contact is a Redstone Contact, as Create's; the Wrench makes one that faces the
//!   column of floors an Elevator Contact (`RedstoneContactBlock.onWrenched`), but the item doesn't
//!   do so when placed (`RedstoneContactItem`).
//! - No floor screen (`ElevatorContactScreen`), no door controls, no Contraption Controls panel
//!   and no Display Link source: a contact's names are only kept and numbered by `initNames`.
//! - A contact keeps being a signal source only for the pulse an arrival gives it
//!   (`RedstoneContactBlock.hasValidContact` has no Redstone Contact to find).

use std::collections::BTreeSet;

use crate::rules::contraption::{Fail, State, relative};
use crate::rules::piston::convert_to_linear;
use crate::{Dir, Pos};

pub const PULLEY: &str = "create:elevator_pulley";
pub const CONTACT: &str = "create:elevator_contact";

/// `ElevatorContactBlock.getLight`: lit while it powers.
pub const POWERING_LIGHT: u8 = 10;

/// `ElevatorContactBlock.ColumnCoords`: the line of floors, by its x and z and the side the
/// contacts face.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ColumnCoords {
    pub x: i32,
    pub z: i32,
    pub side: Dir,
}

impl ColumnCoords {
    /// `ColumnCoords.relative(anchor)`.
    pub fn relative(self, anchor: Pos) -> ColumnCoords {
        ColumnCoords { x: self.x + anchor[0], z: self.z + anchor[2], side: self.side }
    }

    /// `ElevatorColumn.contactAt`.
    pub fn contact_at(self, y: i32) -> Pos {
        [self.x, y, self.z]
    }
}

/// `ElevatorContactBlock.getColumnCoords`: the column a contact at `pos` facing `facing` is in.
pub fn column_of(pos: Pos, facing: Dir) -> ColumnCoords {
    ColumnCoords { x: pos[0], z: pos[2], side: facing }
}

/// `ElevatorColumn`: the floors (the heights of the contacts) of one column, which the cabin
/// is sent to, and whether a cabin runs in it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Column {
    contacts: BTreeSet<i32>,
    target: i32,
    target_available: bool,
    active: bool,
}

impl Column {
    pub fn add(&mut self, y: i32) {
        self.contacts.insert(y);
    }

    pub fn remove(&mut self, y: i32) {
        self.contacts.remove(&y);
    }

    /// The floors, lowest first.
    pub fn floors(&self) -> Vec<i32> {
        self.contacts.iter().copied().collect()
    }

    pub fn target(&mut self, y: i32) {
        self.target = y;
        self.target_available = true;
    }

    pub fn targeted(&self) -> Option<i32> {
        self.target_available.then_some(self.target)
    }

    pub fn target_value(&self) -> i32 {
        self.target
    }

    pub fn set_active(&mut self, active: bool) {
        self.active = active;
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// `checkEmpty`: a column with no floors and no cabin is dropped.
    pub fn is_empty(&self) -> bool {
        self.contacts.is_empty() && !self.active
    }
}

/// `ElevatorContraption.isTargetUnreachable`.
pub fn target_unreachable(target: i32, min_contact_y: i32, max_contact_y: i32) -> bool {
    target < min_contact_y || target > max_contact_y
}

/// `ElevatorPulleyBlockEntity.getTargetOffset`: how far below the pulley the cabin has to be to
/// stand at floor `target_y` (`contact_y_offset`: the cabin's contact above its anchor).
pub fn target_offset(pulley_y: i32, target_y: i32, contact_y_offset: i32) -> i32 {
    pulley_y - target_y + contact_y_offset - 1
}

/// `ElevatorPulleyBlockEntity.getMovementSpeed`: the speed this tick and whether the cabin has
/// arrived. `rpm` is the pulley's, `target` the offset it is sent to, `offset` where it is and
/// `previous` the last tick's speed: it accelerates and brakes smoothly.
pub fn movement_speed(rpm: f32, target: i32, offset: f32, previous: f32) -> (f32, bool) {
    let diff = target as f32 - offset;
    let movement = convert_to_linear(rpm * 2.0).clamp(-1.99, 1.99);
    let limit = movement.abs();
    // `Mth.lerp(|movement|, 0.0075, 0.0175)`.
    let acceleration = 0.0075 + limit * (0.0175 - 0.0075);
    let deceleration = (2.0 * diff.abs() * acceleration).sqrt();
    let mut speed = diff;
    speed = speed.clamp(-limit, limit);
    speed = speed.clamp(previous - acceleration, previous + acceleration);
    speed = speed.clamp(-deceleration, deceleration);
    (speed, diff.abs() < 0.5)
}

/// Where the cabin was and how fast it moved at a tick (the pulley saves one when its target or
/// its rotation changes; the cabin's offset on any later tick is replayed from it, the same on
/// the server and on every client).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snapshot {
    /// Blocks below the pulley the cabin's anchor is (`offset`).
    pub offset: f32,
    pub speed: f32,
    /// The game time it was true at.
    pub time: i64,
    /// The offset it is sent to.
    pub target: i32,
}

/// One tick of the cabin's movement: the new offset and speed, and whether it has arrived
/// (`getMovementSpeed`, then `ElevatorPulleyBlockEntity.tick` easing it onto the floor).
pub fn step(rpm: f32, target: i32, offset: f32, speed: f32) -> (f32, f32, bool) {
    let (next_speed, arrived) = movement_speed(rpm, target, offset, speed);
    let mut next = offset + next_speed;
    if arrived {
        // The contraption is moved by a quarter of what is left each tick, and the rest at the end.
        let diff = target as f32 - next;
        next += if diff.abs() > 1.0 / 128.0 { diff * 0.25 } else { diff };
    }
    (next, next_speed, arrived)
}

/// The cabin `ticks` after the snapshot, replayed at `rpm`. After a long time it has settled.
pub fn replay(snapshot: Snapshot, rpm: f32, ticks: i64) -> (f32, f32, bool) {
    let (mut offset, mut speed, mut arrived) = (snapshot.offset, snapshot.speed, false);
    for _ in 0..ticks.clamp(0, 4000) {
        (offset, speed, arrived) = step(rpm, snapshot.target, offset, speed);
    }
    (offset, speed, arrived)
}

/// What `ElevatorContactBlock.neighborChanged` does when the redstone signal on a contact
/// changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reaction {
    /// The new `powered`, when it changes.
    pub powered: Option<bool>,
    /// The contact calls the cabin to its floor.
    pub call: bool,
}

pub fn neighbour_changed(powered: bool, has_signal: bool, calling: bool) -> Reaction {
    if powered == has_signal {
        return Reaction { powered: None, call: false };
    }
    // The contact toggles; only a new signal calls, and not while it already is.
    Reaction { powered: Some(!powered), call: !powered && !calling }
}

/// `callToContactAndUpdate`: calling the cabin to the contact at `y` clears the others' `calling`
/// and sets the column's target. Returns the floors whose contacts stop calling.
pub fn call(column: &mut Column, y: i32) -> Vec<i32> {
    let others = column.floors().into_iter().filter(|f| *f != y).collect();
    column.target(y);
    others
}

/// `ElevatorContactBlock.getSignal`: 15 from a powering contact, none into another contact.
pub fn signal(powering: bool, reader_is_contact: bool) -> u8 {
    if reader_is_contact || !powering { 0 } else { 15 }
}

/// `ElevatorContraption.capture` and `assemble`: the cabin's contacts must be exactly one,
/// horizontal. `contacts` are the cabin's contact blocks (position relative to the anchor, and
/// facing). Returns the column (relative to the anchor) and the contact's height above the
/// anchor.
pub fn cabin_column(contacts: &[(Pos, Dir)]) -> Result<(ColumnCoords, i32), Fail> {
    let horizontal: Vec<&(Pos, Dir)> = contacts.iter().filter(|(_, f)| f.axis() != crate::Axis::Y).collect();
    match horizontal.as_slice() {
        [] => Err(Fail::NoContacts),
        [(pos, facing)] => {
            let front = relative(*pos, *facing, 1);
            Ok((ColumnCoords { x: front[0], z: front[2], side: facing.opposite() }, front[1]))
        }
        _ => Err(Fail::TooManyContacts),
    }
}

/// Whether a block in a cabin is the contact `cabin_column` reads: a horizontal Redstone Contact
/// (`ElevatorContraption.capture` counts `AllBlocks.REDSTONE_CONTACT`; the floors are Elevator Contacts).
pub fn is_contact(state: &State) -> bool {
    state.is(crate::rules::redstone_contact::ID) && state.dir("facing").is_some_and(|f| f.axis() != crate::Axis::Y)
}

/// `ElevatorColumn.initNames`: the contacts with no name of their own are numbered by the ones
/// that have a number, upward from the one before and downward from the one after. `names` are
/// the short names of the contacts from the lowest floor up; the result is the name each takes.
pub fn number_floors(names: &[String]) -> Vec<String> {
    let value = |s: &str| s.parse::<i32>().ok();
    let mut out: Vec<String> = Vec::with_capacity(names.len());
    let mut previous: Option<i32> = None;
    for (i, name) in names.iter().enumerate() {
        let mut current: Option<i32> = None;
        if !name.trim().is_empty() {
            match value(name) {
                Some(v) => current = Some(v),
                None => {
                    // A name that isn't a number stays as it is and doesn't number its neighbours.
                    out.push(name.clone());
                    continue;
                }
            }
        }
        if let Some(p) = previous {
            current = Some(p + 1);
        }
        let mut next: Option<i32> = None;
        for peek in &names[i + 1..] {
            let Some(v) = value(peek) else { continue };
            if current.is_some_and(|c| c >= v) {
                break;
            }
            next = Some(v);
            break;
        }
        let current = current.unwrap_or_else(|| next.map_or(0, |n| n - 1));
        out.push(current.to_string());
        previous = Some(current);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_column_keeps_its_floors_in_order_and_one_target() {
        let mut column = Column::default();
        for y in [70, 64, 80, 64] {
            column.add(y);
        }
        assert_eq!(column.floors(), vec![64, 70, 80]);
        assert_eq!(column.targeted(), None);
        column.target(70);
        assert_eq!(column.targeted(), Some(70));
        column.remove(64);
        assert_eq!(column.floors(), vec![70, 80]);
        assert!(!column.is_empty());
        column.remove(70);
        column.remove(80);
        assert!(column.is_empty());
        column.set_active(true);
        assert!(!column.is_empty() && column.is_active());
    }

    #[test]
    fn a_call_makes_it_the_target_and_quiets_the_others() {
        let mut column = Column::default();
        for y in [64, 70, 80] {
            column.add(y);
        }
        assert_eq!(call(&mut column, 70), vec![64, 80]);
        assert_eq!(column.targeted(), Some(70));
        assert_eq!(column.target_value(), 70);
    }

    #[test]
    fn redstone_on_a_contact_calls_once_and_toggles_it() {
        // A new signal: powered, and a call unless it is calling already.
        assert_eq!(neighbour_changed(false, true, false), Reaction { powered: Some(true), call: true });
        assert_eq!(neighbour_changed(false, true, true), Reaction { powered: Some(true), call: false });
        // The signal going: powered off, no call.
        assert_eq!(neighbour_changed(true, false, false), Reaction { powered: Some(false), call: false });
        // Nothing changed.
        assert_eq!(neighbour_changed(true, true, false), Reaction { powered: None, call: false });
        assert_eq!(neighbour_changed(false, false, true), Reaction { powered: None, call: false });
    }

    #[test]
    fn a_contact_powers_what_isnt_a_contact() {
        assert_eq!(signal(true, false), 15);
        assert_eq!(signal(true, true), 0);
        assert_eq!(signal(false, false), 0);
    }

    #[test]
    fn the_cabin_sets_its_column_and_its_height() {
        // A contact one up and to the side of the anchor, facing east: the column is the cell
        // east of it, and the floors' contacts face back west at the cabin.
        let (column, height) = cabin_column(&[([0, 1, 0], Dir::East), ([1, 0, 0], Dir::Up)]).unwrap();
        assert_eq!(column, ColumnCoords { x: 1, z: 0, side: Dir::West });
        assert_eq!(height, 1);
        assert_eq!(column.relative([10, 64, 5]), ColumnCoords { x: 11, z: 5, side: Dir::West });
        assert_eq!(column.contact_at(70), [1, 70, 0]);
        // No contact, a vertical one only, or two: it can't be assembled.
        assert_eq!(cabin_column(&[]), Err(Fail::NoContacts));
        assert_eq!(cabin_column(&[([0, 0, 0], Dir::Up)]), Err(Fail::NoContacts));
        assert_eq!(
            cabin_column(&[([0, 0, 0], Dir::East), ([0, 1, 0], Dir::West)]),
            Err(Fail::TooManyContacts)
        );
        assert!(is_contact(&State::parse("create:redstone_contact[facing=north]")));
        assert!(!is_contact(&State::parse("create:redstone_contact[facing=up]")));
        // The floors' own block is not the cabin's contact.
        assert!(!is_contact(&State::parse("create:elevator_contact[facing=north]")));
    }

    #[test]
    fn the_cabin_stands_where_the_contact_meets_the_floor() {
        // A pulley at y 100, a floor at 70, the cabin's contact 3 above its anchor: the anchor
        // is 100 - 70 + 3 - 1 = 32 below the pulley.
        assert_eq!(target_offset(100, 70, 3), 32);
        assert!(target_unreachable(60, 64, 90) && target_unreachable(91, 64, 90) && !target_unreachable(64, 64, 90));
    }

    #[test]
    fn the_cabin_speeds_up_and_brakes_into_the_floor() {
        // From rest at 64 rpm: a gentle start, no faster than the speed allows.
        let (first, arrived) = movement_speed(64.0, 20, 0.0, 0.0);
        assert!(!arrived && first > 0.0 && first <= 0.0075 + 0.25 * 0.01 + 1e-6);
        // It keeps gaining, then slows as the floor comes close.
        let mut speed = first;
        let mut offset = first;
        let mut fastest: f32 = 0.0;
        for _ in 0..2000 {
            let (next, _) = movement_speed(64.0, 20, offset, speed);
            speed = next;
            offset += speed;
            fastest = fastest.max(speed);
            if (20.0 - offset).abs() < 0.001 {
                break;
            }
        }
        assert!(fastest <= 0.25 + 1e-6, "no faster than rpm / 256 ({fastest})");
        assert!((20.0 - offset).abs() < 0.05, "it reached the floor ({offset})");
        // Arrived within half a block; going the other way is negative; no rotation, no way.
        assert!(movement_speed(64.0, 20, 19.7, 0.01).1);
        assert!(movement_speed(64.0, 0, 5.0, 0.0).0 < 0.0);
        assert_eq!(movement_speed(0.0, 20, 0.0, 0.0).0, 0.0);
    }

    #[test]
    fn a_replayed_cabin_ends_on_its_floor_and_the_same_every_time() {
        let snapshot = Snapshot { offset: 2.0, speed: 0.0, time: 100, target: 12 };
        let (offset, _, arrived) = replay(snapshot, 64.0, 600);
        assert!(arrived && (offset - 12.0).abs() < 0.01, "{offset}");
        // Part way, it is between, and the first tick is the first step.
        let (half, _, _) = replay(snapshot, 64.0, 40);
        assert!(half > 2.0 && half < 12.0);
        assert_eq!(replay(snapshot, 64.0, 1), {
            let (o, s, a) = step(64.0, 12, 2.0, 0.0);
            (o, s, a)
        });
        // Nothing before the snapshot.
        assert_eq!(replay(snapshot, 64.0, -5), (2.0, 0.0, false));
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn floors_without_a_number_are_numbered_from_their_neighbours() {
        // Nothing named: 0, 1, 2 (the first with nothing after it is 0).
        assert_eq!(number_floors(&names(&["", "", ""])), names(&["0", "1", "2"]));
        // A named floor sets the count for those after it, and the ones before count up to it.
        assert_eq!(number_floors(&names(&["", "5", "", ""])), names(&["4", "5", "6", "7"]));
        // A name that isn't a number stays.
        assert_eq!(number_floors(&names(&["1", "Roof"])), names(&["1", "Roof"]));
    }
}
