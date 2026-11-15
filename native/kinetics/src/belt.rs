//! Create's mechanical belt: the chain of belt blocks, and the items riding on it.
//!
//! Ported from Create 6.0.10 (`mc1.21.1/dev` at `4148fd8`, MIT, the Creators of Create):
//! `BeltBlock`, `BeltBlockEntity`, `BeltConnectorItem`, `transport/BeltInventory`,
//! `transport/TransportedItemStack` and `transport/ItemHandlerBeltSegment`.
//!
//! This slice is the straight horizontal belt. Items move along it, queue one block apart,
//! and at the end either fly off as item entities or wait at a block whose face closes the
//! way. Belt processing (presses, fans), funnels, tunnels, crushers, belt-to-belt insertion,
//! slopes and casings are not here yet.
//!
//! An item's position counts blocks from the belt's start (its controller, `index` 0) along
//! the belt's facing: 0 is the start block's back edge, `length` the end block's front edge.

use crate::{Axis, Dir, Pos};

/// `CKinetics.maxBeltLength`'s default: the two shafts must be closer than this.
pub const MAX_LENGTH: i32 = 20;
/// `BeltInventory.tick`'s `spacing`: items keep this far apart.
pub const SPACING: f32 = 1.0;

/// `BeltPart`: which piece of the chain a belt block is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Part {
    /// The first block, where the controller lives. Has a pulley.
    Start,
    Middle,
    /// A middle block a shaft was put through: it has a pulley too.
    Pulley,
    /// The last block. Has a pulley.
    End,
}

impl Part {
    pub const ALL: [Part; 4] = [Part::Start, Part::Middle, Part::Pulley, Part::End];

    pub const fn name(self) -> &'static str {
        match self {
            Part::Start => "start",
            Part::Middle => "middle",
            Part::Pulley => "pulley",
            Part::End => "end",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == name)
    }

    /// `BeltBlockEntity.hasPulley`: every part but the middle one turns a shaft.
    pub const fn has_pulley(self) -> bool {
        !matches!(self, Part::Middle)
    }
}

/// `BeltBlock.getRotationAxis` for a horizontal belt: `facing.getClockWise().getAxis()`.
pub const fn rotation_axis(facing: Dir) -> Axis {
    match facing {
        Dir::North | Dir::South => Axis::X,
        _ => Axis::Z,
    }
}

/// `Direction.getClockWise` of a horizontal direction.
pub const fn clockwise(facing: Dir) -> Dir {
    match facing {
        Dir::North => Dir::East,
        Dir::East => Dir::South,
        Dir::South => Dir::West,
        Dir::West => Dir::North,
        other => other,
    }
}

/// `BeltBlockEntity.getBeltMovementSpeed`: blocks per tick at `rpm`.
pub fn movement_speed(rpm: f32) -> f32 {
    rpm / 480.0
}

/// `getDirectionAwareBeltMovementSpeed`: blocks per tick, positive towards the belt's end.
pub fn chain_speed(rpm: f32, facing: Dir) -> f32 {
    let mut offset = facing.sign() as f32;
    if facing.axis() == Axis::X {
        offset = -offset;
    }
    movement_speed(rpm) * offset
}

/// `getMovementFacing`: the way the belt's top moves.
pub fn movement_facing(rpm: f32, facing: Dir) -> Dir {
    let negative = (movement_speed(rpm) < 0.0) ^ (facing.axis() == Axis::X);
    let positive_dir = match facing.axis() {
        Axis::X => Dir::East,
        _ => Dir::South,
    };
    if negative {
        positive_dir.opposite()
    } else {
        positive_dir
    }
}

fn add(a: Pos, b: Pos) -> Pos {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: Pos, by: i32) -> Pos {
    [a[0] * by, a[1] * by, a[2] * by]
}

/// `BeltBlock.nextSegmentPosition` for a horizontal belt: the next block of the chain, or the
/// previous one, or none past either end.
pub fn next_segment(pos: Pos, facing: Dir, part: Part, forward: bool) -> Option<Pos> {
    if (part == Part::End && forward) || (part == Part::Start && !forward) {
        return None;
    }
    Some(add(pos, scale(facing.normal(), if forward { 1 } else { -1 })))
}

/// `BeltHelper.getPositionForOffset` for a horizontal belt.
pub fn segment_position(controller: Pos, facing: Dir, offset: i32) -> Pos {
    add(controller, scale(facing.normal(), offset))
}

/// Why two shafts can't be joined (`BeltConnectorItem.canConnect` refuses).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Not closer than [`MAX_LENGTH`].
    TooFar,
    /// The same shaft twice.
    Same,
    /// Not in a line that crosses both shafts' axis, or the axes differ.
    NotInLine,
    /// A slope, a vertical belt or a sideways one: not in this slice yet.
    NotHorizontal,
}

/// Where a belt between the shafts at `first` and `second` goes, both shafts on `axis`, as
/// `BeltConnectorItem.canConnect` and `createBelts` place it: its facing (from `first`, the
/// start, to `second`, the end) and every block of it in order. Only the geometry: the game
/// still checks the blocks between and the shafts' speeds.
pub fn connect(first: Pos, second: Pos, axis: Axis, second_axis: Axis) -> Result<(Dir, Vec<Pos>), Refusal> {
    let diff = [second[0] - first[0], second[1] - first[1], second[2] - first[2]];
    if diff == [0, 0, 0] {
        return Err(Refusal::Same);
    }
    // `Vec3i.closerThan`: distSqr < maxLength².
    let distance = diff.iter().map(|c| c * c).sum::<i32>();
    if distance >= MAX_LENGTH * MAX_LENGTH {
        return Err(Refusal::TooFar);
    }
    let [x, y, z] = diff.map(i32::abs);
    let sames = i32::from(x == y) + i32::from(y == z) + i32::from(z == x);
    if diff[axis.index()] != 0 || sames != 1 || axis != second_axis {
        return Err(Refusal::NotInLine);
    }
    if axis == Axis::Y || diff[1] != 0 {
        return Err(Refusal::NotHorizontal);
    }
    // `getFacingFromTo`: along x unless the two share an x.
    let along = if first[0] == second[0] { Axis::Z } else { Axis::X };
    let sign = diff[along.index()].signum();
    let facing = match (along, sign > 0) {
        (Axis::X, true) => Dir::East,
        (Axis::X, false) => Dir::West,
        (_, true) => Dir::South,
        (_, false) => Dir::North,
    };
    let steps = diff[along.index()].abs();
    let chain = (0..=steps)
        .map(|i| add(first, scale(facing.normal(), i)))
        .collect();
    Ok((facing, chain))
}

/// The part a block of a new belt gets (`createBelts`): `pulley` when a shaft stood there.
pub fn part_at(index: usize, length: usize, pulley: bool) -> Part {
    if index == 0 {
        Part::Start
    } else if index + 1 == length {
        Part::End
    } else if pulley {
        Part::Pulley
    } else {
        Part::Middle
    }
}

/// `BeltInventory.Ending`: what is past the belt's end, and how far before the edge items stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    /// Open: items fly off.
    Eject,
    /// A block whose face towards the belt is solid: items wait.
    Blocked,
}

impl Ending {
    pub const fn margin(self) -> f32 {
        match self {
            Ending::Eject => 0.0,
            Ending::Blocked => 0.45,
        }
    }

    pub const fn id(self) -> i8 {
        match self {
            Ending::Eject => 0,
            Ending::Blocked => 1,
        }
    }

    pub const fn from_id(id: i8) -> Self {
        match id {
            1 => Ending::Blocked,
            _ => Ending::Eject,
        }
    }
}

/// `TransportedItemStack`: one stack riding the belt. `S` is the game's item stack.
#[derive(Clone, Debug, PartialEq)]
pub struct Item<S> {
    pub stack: S,
    /// `beltPosition`.
    pub pos: f32,
    /// `sideOffset`: across the belt, eased towards [`Item::target_side`].
    pub side: f32,
    /// `angle`, in degrees: how it lies, and where across the belt it settles.
    pub angle: i32,
    /// `insertedAt`: the segment it was put on.
    pub inserted_at: i32,
    /// `insertedFrom`.
    pub inserted_from: Dir,
}

impl<S> Item<S> {
    /// A stack dropped onto segment `segment` (`ItemHandlerBeltSegment.insertItem`), lying at
    /// `angle` (`new TransportedItemStack`: random, or 180 for an upright item).
    pub fn dropped(stack: S, segment: i32, angle: i32, positive: bool) -> Self {
        let mut item = Self {
            stack,
            pos: segment as f32 + 0.5 + if positive { -1.0 } else { 1.0 } / 16.0,
            side: 0.0,
            angle,
            inserted_at: segment,
            inserted_from: Dir::Up,
        };
        item.side = item.target_side();
        item
    }

    /// `getTargetSideOffset`.
    pub fn target_side(&self) -> f32 {
        (self.angle - 180) as f32 / (360.0 * 3.0)
    }
}

/// What one tick did to a belt's items.
#[derive(Debug, PartialEq)]
pub struct Moved<S> {
    /// Stacks that went off the end (`BeltInventory.eject`), with the position they left at.
    pub ejected: Vec<Item<S>>,
    /// Anything moved, so the saved items changed.
    pub moved: bool,
    /// The list changed in a way a client can't foresee: an item left or the order turned.
    pub reordered: bool,
}

/// `BeltInventory`: the items of one belt, kept on its controller.
#[derive(Clone, Debug, PartialEq)]
pub struct Inventory<S> {
    /// The item furthest along the belt first.
    pub items: Vec<Item<S>>,
    /// `beltMovementPositive`: the belt last moved towards its end.
    pub positive: bool,
}

impl<S> Default for Inventory<S> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            positive: false,
        }
    }
}

impl<S> Inventory<S> {
    /// `BeltInventory.tick` on the server, for a belt `length` blocks long moving `speed`
    /// blocks a tick towards its end ([`chain_speed`]). `ending` is asked only once an item
    /// gets near the end in this tick.
    pub fn tick(&mut self, speed: f32, length: i32, mut ending: impl FnMut() -> Ending) -> Moved<S> {
        let mut moved = Moved {
            ejected: Vec::new(),
            moved: false,
            reordered: false,
        };
        if speed == 0.0 {
            return moved;
        }
        // The belt just reversed: the item furthest along is now at the other end of the list.
        if self.positive != (speed > 0.0) {
            self.positive = !self.positive;
            self.items.reverse();
            moved.reordered = true;
        }
        let positive = self.positive;
        let mut resolved: Option<Ending> = None;
        // The position of the item just handled, the one in front; an ejected item counts too.
        let mut front: Option<f32> = None;
        let mut kept = Vec::with_capacity(self.items.len());
        for mut item in self.items.drain(..) {
            let mut movement = speed;
            // Don't move if other items are waiting in front.
            let mut no_movement = false;
            if let Some(front) = front {
                let diff = front - item.pos;
                if diff.abs() <= SPACING {
                    no_movement = true;
                }
                movement = if positive {
                    movement.min(diff - SPACING)
                } else {
                    movement.max(diff + SPACING)
                };
            }
            // Don't move beyond the edge.
            let mut to_end = if positive {
                length as f32 - item.pos
            } else {
                -item.pos
            };
            if to_end.abs() < movement.abs() + 1.0 {
                let margin = resolved.get_or_insert_with(&mut ending).margin();
                to_end += if positive { -margin } else { margin };
            }
            let limited = if positive {
                movement.min(to_end)
            } else {
                movement.max(to_end)
            };
            if no_movement {
                front = Some(item.pos);
                kept.push(item);
                continue;
            }
            item.pos += limited;
            let to_middle = item.target_side() - item.side;
            item.side += (to_middle * limited.abs() * 6.0).clamp(-to_middle.abs(), to_middle.abs());
            if limited != 0.0 {
                moved.moved = true;
            }
            front = Some(item.pos);
            // Movement successful, or it waits at a closed end.
            if limited == movement || resolved != Some(Ending::Eject) {
                kept.push(item);
                continue;
            }
            moved.ejected.push(item);
            moved.reordered = true;
        }
        self.items = kept;
        moved
    }

    /// `canInsertAtFromSide(segment, UP)`: an item dropped on `segment` lately is still too close.
    pub fn can_insert_at(&self, segment: i32) -> bool {
        let at = segment as f32 + 0.5;
        !self.items.iter().any(|item| {
            item.inserted_at == segment
                && item.inserted_from == Dir::Up
                && if self.positive {
                    item.pos <= at + 1.0
                } else {
                    item.pos >= at - 1.0
                }
        })
    }

    /// `BeltInventory.insert`: keeps the list ordered from the item furthest along.
    pub fn insert(&mut self, item: Item<S>) {
        let index = self
            .items
            .iter()
            .position(|other| (other.pos < item.pos) == self.positive)
            .unwrap_or(self.items.len());
        self.items.insert(index, item);
    }

    /// Takes every item within `distance` of `position` (`applyToEachWithin` removing them, as
    /// an empty hand does at `index + .5` within .55).
    pub fn take_within(&mut self, position: f32, distance: f32) -> Vec<Item<S>> {
        let (taken, kept) = std::mem::take(&mut self.items)
            .into_iter()
            .partition(|item| (position - item.pos).abs() < distance);
        self.items = kept;
        taken
    }
}

/// Where a client draws the items `ticks` after the server last sent them: each moves at
/// `speed` until the end (less `margin`) or the item in front stops it, as the server's
/// [`Inventory::tick`] will move them while nothing new happens. `positions` are front first.
pub fn foresee(positions: &[f32], speed: f32, ticks: f32, length: i32, margin: f32) -> Vec<f32> {
    let positive = speed > 0.0;
    let end = if positive {
        length as f32 - margin
    } else {
        margin
    };
    let mut front: Option<f32> = None;
    positions
        .iter()
        .map(|&pos| {
            let mut limit = end;
            if let Some(front) = front {
                limit = if positive {
                    limit.min(front - SPACING)
                } else {
                    limit.max(front + SPACING)
                };
            }
            let ahead = pos + speed * ticks.max(0.0);
            let drawn = if positive {
                ahead.min(limit).max(pos)
            } else {
                ahead.max(limit).min(pos)
            };
            front = Some(drawn);
            drawn
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn belt(positions: &[f32], positive: bool) -> Inventory<u32> {
        Inventory {
            items: positions
                .iter()
                .enumerate()
                .map(|(i, &pos)| Item {
                    stack: i as u32,
                    pos,
                    side: 0.0,
                    angle: 180,
                    inserted_at: 0,
                    inserted_from: Dir::Up,
                })
                .collect(),
            positive,
        }
    }

    fn positions(inventory: &Inventory<u32>) -> Vec<f32> {
        inventory.items.iter().map(|i| i.pos).collect()
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn speeds_and_facings_follow_create() {
        // 480 RPM would be a block a tick.
        assert_eq!(movement_speed(48.0), 0.1);
        // A belt facing south moves towards its end when the speed is positive; one facing
        // east the other way round, as `getDirectionAwareBeltMovementSpeed` flips x.
        assert_eq!(chain_speed(48.0, Dir::South), 0.1);
        assert_eq!(chain_speed(48.0, Dir::North), -0.1);
        assert_eq!(chain_speed(48.0, Dir::East), -0.1);
        assert_eq!(chain_speed(48.0, Dir::West), 0.1);
        // The top moves the way the chain speed says.
        for facing in [Dir::North, Dir::South, Dir::East, Dir::West] {
            for rpm in [32.0, -32.0] {
                let towards_end = chain_speed(rpm, facing) > 0.0;
                let expected = if towards_end { facing } else { facing.opposite() };
                assert_eq!(movement_facing(rpm, facing), expected, "{facing:?} {rpm}");
            }
        }
        assert_eq!(rotation_axis(Dir::North), Axis::X);
        assert_eq!(rotation_axis(Dir::East), Axis::Z);
    }

    #[test]
    fn a_stack_rides_to_the_end_and_flies_off() {
        let mut inventory = Inventory::default();
        inventory.insert(Item::dropped(7u32, 0, 180, true));
        let speed = chain_speed(256.0, Dir::South);
        let mut ejected = None;
        for _ in 0..100 {
            let moved = inventory.tick(speed, 3, || Ending::Eject);
            if let Some(item) = moved.ejected.into_iter().next() {
                assert!(moved.reordered);
                ejected = Some(item);
                break;
            }
        }
        let item = ejected.expect("the stack leaves the belt");
        assert_eq!(item.stack, 7);
        // It left at the end's front edge.
        assert!(close(item.pos, 3.0), "{}", item.pos);
        assert!(inventory.items.is_empty());
    }

    #[test]
    fn a_closed_end_holds_the_stack_and_queues_the_next() {
        let mut inventory = belt(&[1.5, 0.5], true);
        let speed = chain_speed(256.0, Dir::South);
        for _ in 0..200 {
            let moved = inventory.tick(speed, 4, || Ending::Blocked);
            assert!(moved.ejected.is_empty());
        }
        let at = positions(&inventory);
        assert!(close(at[0], 4.0 - 0.45), "{at:?}");
        assert!(close(at[1], 4.0 - 0.45 - SPACING), "{at:?}");
        // Opened again: the front one goes, then the one behind.
        let mut gone = 0;
        for _ in 0..200 {
            gone += inventory.tick(speed, 4, || Ending::Eject).ejected.len();
        }
        assert_eq!(gone, 2);
    }

    #[test]
    fn a_stopped_belt_moves_nothing() {
        let mut inventory = belt(&[1.0], true);
        let moved = inventory.tick(0.0, 4, || unreachable!());
        assert!(!moved.moved && !moved.reordered);
        assert_eq!(positions(&inventory), vec![1.0]);
    }

    #[test]
    fn the_end_is_only_asked_for_near_it() {
        let mut inventory = belt(&[0.5], true);
        inventory.tick(0.1, 10, || panic!("far from the end"));
        let mut asked = false;
        let mut near = belt(&[9.5], true);
        near.tick(0.1, 10, || {
            asked = true;
            Ending::Eject
        });
        assert!(asked);
    }

    #[test]
    fn reversing_turns_the_order_and_runs_back() {
        let mut inventory = belt(&[2.5, 0.5], true);
        let moved = inventory.tick(-0.1, 4, || Ending::Blocked);
        assert!(moved.reordered);
        assert!(!inventory.positive);
        // The one nearest the start leads now, and stops short of the closed start.
        assert!(close(inventory.items[0].pos, 0.45));
        assert!(close(inventory.items[1].pos, 2.4));
        for _ in 0..100 {
            inventory.tick(-0.1, 4, || Ending::Blocked);
        }
        let at = positions(&inventory);
        assert!(close(at[0], 0.45) && close(at[1], 1.45), "{at:?}");
    }

    #[test]
    fn a_drop_waits_for_the_last_drop_on_that_segment_to_move_on() {
        let mut inventory = belt(&[], true);
        assert!(inventory.can_insert_at(1));
        inventory.insert(Item::dropped(1u32, 1, 180, true));
        assert!(!inventory.can_insert_at(1));
        // Others segments are free.
        assert!(inventory.can_insert_at(0));
        assert!(inventory.can_insert_at(2));
        inventory.items[0].pos = 2.6;
        assert!(inventory.can_insert_at(1));
    }

    #[test]
    fn inserting_keeps_the_front_first() {
        let mut forward = belt(&[3.0, 1.0], true);
        forward.insert(Item::dropped(9, 2, 180, true));
        assert_eq!(forward.items.iter().map(|i| i.stack).collect::<Vec<_>>(), vec![0, 9, 1]);
        let mut back = belt(&[1.0, 3.0], false);
        back.insert(Item::dropped(9, 2, 180, false));
        assert_eq!(back.items.iter().map(|i| i.stack).collect::<Vec<_>>(), vec![0, 9, 1]);
    }

    #[test]
    fn a_hand_takes_what_is_on_its_segment() {
        let mut inventory = belt(&[2.5, 1.6, 0.4], true);
        let taken = inventory.take_within(1.5, 0.55);
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].pos, 1.6);
        assert_eq!(positions(&inventory), vec![2.5, 0.4]);
    }

    #[test]
    fn a_drop_settles_across_the_belt_by_its_angle() {
        let item = Item::dropped((), 0, 0, true);
        assert!(close(item.side, -1.0 / 6.0));
        assert!(close(item.pos, 0.5 - 1.0 / 16.0));
        assert!(close(Item::dropped((), 0, 0, false).pos, 0.5 + 1.0 / 16.0));
    }

    #[test]
    fn the_client_foresees_what_the_server_does() {
        let speed = chain_speed(128.0, Dir::South);
        let mut inventory = belt(&[3.2, 2.5, 0.5], true);
        let start = positions(&inventory);
        for ticks in 1..=40 {
            inventory.tick(speed, 5, || Ending::Blocked);
            let drawn = foresee(&start, speed, ticks as f32, 5, Ending::Blocked.margin());
            for (a, b) in drawn.iter().zip(positions(&inventory)) {
                assert!((a - b).abs() < 1e-3, "tick {ticks}: {drawn:?} vs {:?}", positions(&inventory));
            }
        }
    }

    #[test]
    fn connecting_two_shafts_in_a_line() {
        let (facing, chain) = connect([0, 0, 0], [3, 0, 0], Axis::Z, Axis::Z).unwrap();
        assert_eq!(facing, Dir::East);
        assert_eq!(chain, vec![[0, 0, 0], [1, 0, 0], [2, 0, 0], [3, 0, 0]]);
        let (facing, chain) = connect([0, 0, 0], [0, 0, -2], Axis::X, Axis::X).unwrap();
        assert_eq!(facing, Dir::North);
        assert_eq!(chain.len(), 3);
        // Along the shafts' own axis, or with different axes, there is no belt.
        assert_eq!(connect([0, 0, 0], [3, 0, 0], Axis::X, Axis::X), Err(Refusal::NotInLine));
        assert_eq!(connect([0, 0, 0], [3, 0, 0], Axis::Z, Axis::Y), Err(Refusal::NotInLine));
        assert_eq!(connect([0, 0, 0], [3, 0, 1], Axis::Y, Axis::Y), Err(Refusal::NotInLine));
        assert_eq!(connect([0, 0, 0], [0, 0, 0], Axis::Z, Axis::Z), Err(Refusal::Same));
        // Closer than 20: a belt is at most 20 blocks long.
        assert!(connect([0, 0, 0], [19, 0, 0], Axis::Z, Axis::Z).is_ok());
        assert_eq!(connect([0, 0, 0], [20, 0, 0], Axis::Z, Axis::Z), Err(Refusal::TooFar));
        // Slopes and vertical belts come later.
        assert_eq!(connect([0, 0, 0], [2, 2, 0], Axis::Z, Axis::Z), Err(Refusal::NotHorizontal));
        assert_eq!(connect([0, 0, 0], [0, 3, 0], Axis::Z, Axis::Z), Err(Refusal::NotHorizontal));
    }

    #[test]
    fn a_belt_turns_as_one_and_drives_shafts_at_its_pulleys() {
        use crate::{Kind, Node, solve};
        use std::collections::BTreeMap;
        let belt = |part| Node::passive(Kind::Belt(Dir::East, part));
        let mut motor = Node::passive(Kind::Motor(Dir::South));
        motor.generated = 16.0;
        motor.capacity = 16384.0;
        let nodes: BTreeMap<Pos, Node> = [
            ([0, 0, -1], motor),
            ([0, 0, 0], belt(Part::Start)),
            ([1, 0, 0], belt(Part::Middle)),
            ([2, 0, 0], belt(Part::End)),
            // Beside the middle block: no pulley there.
            ([1, 0, 1], Node::passive(Kind::Shaft(Axis::Z))),
            // Beside the end's pulley.
            ([2, 0, 1], Node::passive(Kind::Shaft(Axis::Z))),
            // Another belt starting right after the end is a belt of its own.
            ([3, 0, 0], belt(Part::Start)),
            ([4, 0, 0], belt(Part::End)),
        ]
        .into_iter()
        .collect();
        let solution = solve(&nodes);
        assert!(solution.breaks.is_empty());
        let speeds = solution.speeds;
        for pos in [[0, 0, 0], [1, 0, 0], [2, 0, 0], [2, 0, 1]] {
            assert_eq!(speeds[&pos], 16.0, "{pos:?}");
        }
        for pos in [[1, 0, 1], [3, 0, 0], [4, 0, 0]] {
            assert_eq!(speeds[&pos], 0.0, "{pos:?}");
        }
    }

    #[test]
    fn parts_and_chain_steps() {
        assert_eq!(part_at(0, 4, true), Part::Start);
        assert_eq!(part_at(3, 4, true), Part::End);
        assert_eq!(part_at(1, 4, false), Part::Middle);
        assert_eq!(part_at(2, 4, true), Part::Pulley);
        assert_eq!(next_segment([0, 0, 0], Dir::East, Part::Start, true), Some([1, 0, 0]));
        assert_eq!(next_segment([0, 0, 0], Dir::East, Part::Start, false), None);
        assert_eq!(next_segment([0, 0, 0], Dir::East, Part::End, true), None);
        assert_eq!(segment_position([5, 1, 5], Dir::North, 3), [5, 1, 2]);
        for part in Part::ALL {
            assert_eq!(Part::from_name(part.name()), Some(part));
        }
    }
}
