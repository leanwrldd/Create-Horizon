//! Items riding belts and resting on depots in a Ponder scene: `world.createItemOnBelt`, `createItemOnBeltLike`,
//! `removeItemsFromBelt`, `stallBeltItem` and `changeBeltItemTo` (Create 6.0.10, `CreateSceneBuilder.WorldInstructions`
//! and `BeltItemElement`). No game types.
//!
//! The scene does not script an item's travel: `createItemOnBelt` hands the stack to the belt's
//! `DirectBeltInputBehaviour.handleInsertion` (the stack lands on its segment, as a stack dropped from above does) and
//! the belt's own `BeltInventory` moves it every tick at the belt's speed. [`crate::belt::Inventory`] is that
//! inventory, so the replay below runs it: the same spacing, the same stop short of the end, the same direction-aware
//! speed. `removeItemsFromBelt` is `handleCenteredProcessingOnAllItems(.52f, removeItem)`: every item within .52 of the
//! middle of the block. `stallBeltItem` sets `locked` on the item. A block that is not a belt (a depot) holds a stack
//! where it is put until it is removed.
//!
//! What a scene's belt does that this leaves out: processors over the belt (presses, sprays, funnels, tunnels), which
//! need the scene's block entities; items they would act on ride past them.

use std::collections::BTreeSet;

use crate::belt::{Ending, Inventory, Item, Part, Processing, Slope, Surroundings, chain_speed};
use crate::rules::belt_slope::{Chain, next_segment, vector_for_offset};
use crate::{Dir, Pos};

pub type Link = u32;

/// A belt block as the scene's structure has it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    pub facing: Dir,
    pub slope: Slope,
    pub part: Part,
}

/// One instruction of the scene's, by the tick it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// `createItemOnBelt(pos, side, stack)` and `createItemOnBeltLike`: `stack` is the scene's own index of the item.
    Put { link: Link, stack: u32, pos: Pos },
    /// `removeItemsFromBelt(pos)`.
    Remove { pos: Pos },
    /// `stallBeltItem(link, stalled)`.
    Stall { link: Link, stalled: bool },
    /// `changeBeltItemTo(link, stack)`.
    Change { link: Link, stack: u32 },
}

/// Where a stack is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub stack: u32,
    pub at: [f64; 3],
}

/// What the scene's blocks say.
pub struct World<'a> {
    /// The belt block at a position, if it is one.
    pub belt: &'a dyn Fn(Pos) -> Option<Segment>,
    /// The speed of the belt block at a position at a tick (rpm).
    pub rpm: &'a dyn Fn(Pos, i64) -> f32,
    /// What is past the belt's end: `Eject` for air.
    pub ending: &'a dyn Fn(Pos) -> Ending,
}

/// The belt a block is on: its controller, how many blocks it has, and which block this is (`BeltBlockEntity.getController`).
pub fn chain_at(world: &World<'_>, pos: Pos) -> Option<(Chain, i32)> {
    let here = (world.belt)(pos)?;
    let mut controller = pos;
    let mut segment = here;
    let mut index = 0;
    while let Some(back) = next_segment(controller, segment.facing, segment.slope, segment.part, false) {
        match (world.belt)(back) {
            Some(before) if before.facing == here.facing && before.slope == here.slope => {
                controller = back;
                segment = before;
                index += 1;
            }
            _ => break,
        }
    }
    let mut length = 1;
    let mut end = controller;
    let mut part = segment;
    while let Some(next) = next_segment(end, part.facing, part.slope, part.part, true) {
        match (world.belt)(next) {
            Some(after) if after.facing == here.facing && after.slope == here.slope => {
                end = next;
                part = after;
                length += 1;
            }
            _ => break,
        }
    }
    Some((Chain { controller, facing: here.facing, slope: here.slope, length }, index))
}

struct Riding {
    chain: Chain,
    items: Inventory<Link>,
}

struct Around<'a> {
    ending: Ending,
    stalled: &'a BTreeSet<Link>,
    horizontal: bool,
}

impl Surroundings<Link> for Around<'_> {
    fn ending(&mut self) -> Ending {
        self.ending
    }

    fn horizontal(&self) -> bool {
        self.horizontal
    }

    fn held(&mut self, _segment: i32, item: &Item<Link>) -> Option<Processing> {
        self.stalled.contains(&item.stack).then_some(Processing::Hold)
    }
}

/// The stacks on belts and depots after scene tick `until`, for the actions in the order they run. An item rests on its
/// segment's belt as it was put there; a stack `Put` on a block that is no belt rests on that block.
pub fn run(world: &World<'_>, actions: &[(i64, Action)], until: i64) -> Vec<Placed> {
    let mut belts: Vec<Riding> = Vec::new();
    let mut resting: Vec<(Link, Pos)> = Vec::new();
    let mut stacks = std::collections::BTreeMap::<Link, u32>::new();
    let mut stalled = BTreeSet::<Link>::new();
    let Some(&(first, _)) = actions.first() else {
        return Vec::new();
    };
    let mut next = 0;
    let mut tick = first;
    while tick <= until {
        while let Some((_, action)) = actions.get(next).filter(|(at, _)| *at <= tick) {
            next += 1;
            match action {
                Action::Put { link, stack, pos } => {
                    stacks.insert(*link, *stack);
                    if let Some((chain, index)) = chain_at(world, *pos) {
                        let slot = match belts.iter().position(|b| b.chain == chain) {
                            Some(slot) => slot,
                            None => {
                                belts.push(Riding { chain, items: Inventory::default() });
                                belts.len() - 1
                            }
                        };
                        let rpm = (world.rpm)(chain.controller, tick);
                        let positive = chain_speed(rpm, chain.facing) > 0.0;
                        belts[slot].items.insert(Item::dropped(*link, index, 180, positive));
                    } else {
                        resting.push((*link, *pos));
                    }
                }
                Action::Remove { pos } => {
                    if let Some((chain, index)) = chain_at(world, *pos) {
                        for belt in belts.iter_mut().filter(|belt| belt.chain == chain) {
                            belt.items.take_within(index as f32 + 0.5, 0.52);
                        }
                    }
                    resting.retain(|(_, at)| at != pos);
                }
                Action::Stall { link, stalled: on } => {
                    if *on {
                        stalled.insert(*link);
                        for item in belts.iter_mut().flat_map(|belt| belt.items.items.iter_mut()) {
                            if item.stack == *link {
                                item.locked = true;
                            }
                        }
                    } else {
                        stalled.remove(link);
                    }
                }
                Action::Change { link, stack } => {
                    stacks.insert(*link, *stack);
                }
            }
        }
        for belt in &mut belts {
            let chain = belt.chain;
            let rpm = (world.rpm)(chain.controller, tick);
            let speed = chain_speed(rpm, chain.facing);
            // The block past the end the belt is moving towards.
            let beyond = if speed > 0.0 { chain.at(chain.length) } else { chain.at(-1) };
            let mut around = Around {
                ending: (world.ending)(beyond),
                stalled: &stalled,
                horizontal: chain.slope == Slope::Horizontal,
            };
            belt.items.tick_with(speed, chain.length, &mut around);
        }
        tick += 1;
    }
    let mut placed = Vec::new();
    for belt in &belts {
        for item in &belt.items.items {
            let at = vector_for_offset(belt.chain, item.pos);
            // The stack lies on the belt's surface: the block's own height plus how far up the slope it is.
            placed.push(Placed { stack: stacks[&item.stack], at: [at[0], at[1] - 0.5 + 0.92, at[2]] });
        }
    }
    for &(link, pos) in &resting {
        placed.push(Placed {
            stack: stacks[&link],
            at: [f64::from(pos[0]) + 0.5, f64::from(pos[1]) + 0.92, f64::from(pos[2]) + 0.5],
        });
    }
    placed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A belt of `length` blocks along +x from `[0, 1, 0]`, moving at `rpm`.
    fn lay(length: i32, rpm: f32, ending: Ending) -> (impl Fn(Pos) -> Option<Segment>, f32, Ending) {
        let belt = move |pos: Pos| {
            (pos[1] == 1 && pos[2] == 0 && (0..length).contains(&pos[0])).then(|| Segment {
                facing: Dir::East,
                slope: Slope::Horizontal,
                part: if pos[0] == 0 {
                    Part::Start
                } else if pos[0] == length - 1 {
                    Part::End
                } else {
                    Part::Middle
                },
            })
        };
        (belt, rpm, ending)
    }

    fn put(link: Link, pos: Pos) -> Action {
        Action::Put { link, stack: 7, pos }
    }

    fn run_on(length: i32, rpm: f32, ending: Ending, actions: &[(i64, Action)], until: i64) -> Vec<Placed> {
        let (belt, rpm, ending) = lay(length, rpm, ending);
        let world = World { belt: &belt, rpm: &move |_, _| rpm, ending: &move |_| ending };
        run(&world, actions, until)
    }

    #[test]
    fn a_belt_is_found_whole_from_any_of_its_blocks() {
        let (belt, _, _) = lay(4, 16.0, Ending::Eject);
        let world = World { belt: &belt, rpm: &|_, _| 16.0, ending: &|_| Ending::Eject };
        let (chain, index) = chain_at(&world, [2, 1, 0]).unwrap();
        assert_eq!((chain.controller, chain.length, index), ([0, 1, 0], 4, 2));
        assert!(chain_at(&world, [9, 1, 0]).is_none());
    }

    #[test]
    fn an_item_put_on_a_running_belt_rides_it_at_the_belts_speed() {
        // -16 RPM on a belt facing east moves towards its end: 1/30 of a block a tick (`getDirectionAwareBeltMovementSpeed`).
        let here = run_on(5, -16.0, Ending::Blocked, &[(10, put(1, [1, 1, 0]))], 40);
        assert_eq!(here.len(), 1);
        // Dropped a sixteenth short of the block's middle, then moved on each of the 31 ticks from 10 to 40.
        let expected = 1.5 - 1.0 / 16.0 + 31.0 / 30.0;
        assert!((here[0].at[0] - expected).abs() < 1e-3);
        assert!((here[0].at[1] - 1.92).abs() < 1e-6);
        // Turning the other way it goes towards the start.
        let back = run_on(5, 16.0, Ending::Blocked, &[(10, put(1, [3, 1, 0]))], 40);
        assert!(back[0].at[0] < 3.5);
    }

    #[test]
    fn an_item_stops_short_of_a_blocked_end_and_one_over_open_air_flies_off() {
        let held = run_on(3, -256.0, Ending::Blocked, &[(0, put(1, [0, 1, 0]))], 400);
        assert_eq!(held.len(), 1);
        assert!(held[0].at[0] <= 3.0 - 0.45 + 1e-3 && held[0].at[0] > 2.0);
        let off = run_on(3, -256.0, Ending::Eject, &[(0, put(1, [0, 1, 0]))], 400);
        assert!(off.is_empty());
    }

    #[test]
    fn removing_clears_the_blocks_items_and_a_depot_holds_until_then() {
        let belt = run_on(
            4,
            0.0,
            Ending::Blocked,
            &[(0, put(1, [1, 1, 0])), (5, Action::Remove { pos: [1, 1, 0] })],
            9,
        );
        assert!(belt.is_empty());
        let depot = run_on(
            4,
            16.0,
            Ending::Blocked,
            &[(0, put(1, [8, 1, 8])), (20, Action::Remove { pos: [8, 1, 8] })],
            10,
        );
        assert_eq!(depot, vec![Placed { stack: 7, at: [8.5, 1.92, 8.5] }]);
        let gone = run_on(
            4,
            16.0,
            Ending::Blocked,
            &[(0, put(1, [8, 1, 8])), (20, Action::Remove { pos: [8, 1, 8] })],
            20,
        );
        assert!(gone.is_empty());
    }

    #[test]
    fn a_stalled_item_waits_and_goes_on_when_released() {
        let stalled = [
            (0, put(1, [0, 1, 0])),
            (0, Action::Stall { link: 1, stalled: true }),
            (40, Action::Stall { link: 1, stalled: false }),
        ];
        let early = run_on(6, -128.0, Ending::Blocked, &stalled, 30)[0].at[0];
        let late = run_on(6, -128.0, Ending::Blocked, &stalled, 70)[0].at[0];
        let still = run_on(6, -128.0, Ending::Blocked, &stalled, 10)[0].at[0];
        assert!((early - still).abs() < 1e-6);
        assert!((late - early).abs() > 0.5);
    }

    #[test]
    fn changing_an_items_stack_changes_what_is_drawn() {
        let both = [(0, put(1, [0, 1, 0])), (5, Action::Change { link: 1, stack: 9 })];
        assert_eq!(run_on(4, 16.0, Ending::Blocked, &both, 20)[0].stack, 9);
    }
}
