//! What the tunnels do to the items of the belt under them as they cross from one block of the
//! belt into the next (`BeltTunnelInteractionHandler`): flap, split a stack off an Andesite
//! Tunnel's sides, take a stack into a Brass Tunnel or hold it back. The belt calls these through
//! `belt::register_tunnels`; the decisions are `create_kinetics::rules::tunnel`.

use create_kinetics::belt::{Crossing, Item};
use create_kinetics::rules::tunnel::{self as rules, BrassAhead, Offer, Verdict};
use create_kinetics::{Dir, Pos};
use horizon_client::mod_api::{Hooks, Host, ItemStack};

use super::tunnel_distribute as distribute;
use super::tunnel_world::{self as world, View};
use crate::{belt, util};

struct Shared;

impl Hooks for Shared {}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

/// Gives the belt the tunnels' two hand-offs.
pub fn register() {
    belt::register_tunnels(belt::TunnelHandler { cross, left, covers });
}

/// `BeltTunnelInteractionHandler.getTunnelOnPosition` for the block over the belt block at `belt`.
fn tunnel_over(host: &dyn Host, belt: Pos) -> Option<(Pos, View)> {
    let at = world::above(belt);
    world::view(host, at).map(|view| (at, view))
}

/// `flapTunnelsAndCheckIfStuck` for an item moving towards `movement` from the belt block `from`
/// into `to`.
fn cross(host: &mut dyn Host, item: &mut Item<ItemStack>, from: Pos, to: Pos, movement: Dir) -> Crossing {
    let ahead = tunnel_over(&*host, to);
    let mut verdict = Crossing::Pass;

    if let Some((at, view)) = ahead {
        let Some(entity) = world::entity_at(&*host, at) else {
            return Crossing::Pass;
        };
        if view.brass {
            let (state, _) = world::read_state(at, &entity.nbt);
            let entry = movement.opposite();
            let brass = BrassAhead {
                entry_allowed: world::filter_allows(&entity.nbt, entry, &item.stack),
                distributes: world::distributes(&entity.nbt, view.axis),
                holding: state.count > 0,
                can_take: state.can_take(),
            };
            match rules::enter_brass(&brass) {
                Verdict::Stuck => return Crossing::Stuck,
                Verdict::Wait => return Crossing::Wait,
                Verdict::Take => {
                    distribute::give_stack(host, at, &item.stack, Some(entry));
                    item.stack.count = 0;
                    verdict = Crossing::Taken;
                }
                Verdict::Pass => {}
            }
        } else if rules::splits(item.stack.count, view.shape, movement.axis(), view.axis) {
            let flaps = world::dirs_of(&entity.nbt, world::FLAPS);
            let split = rules::split(item.stack.count, movement.axis(), &flaps, |side| {
                offer(host, to, side, &item.stack)
            });
            for side in &split.given {
                world::flap(host, at, *side, false);
            }
            item.stack.count -= split.given.len() as u32;
            if split.wait {
                return Crossing::Wait;
            }
            if !split.given.is_empty() {
                verdict = Crossing::Split;
            }
        }
    }

    // Both tunnels flap: the one the item leaves outwards, the one it enters inwards.
    world::flap(host, world::above(from), movement, false);
    if let Some((at, _)) = ahead {
        world::flap(host, at, movement.opposite(), true);
    }
    verdict
}

/// An Andesite Tunnel over the belt block `to` offering one item of `stack` to its `side`.
fn offer(host: &mut dyn Host, to: Pos, side: Dir, stack: &ItemStack) -> Offer {
    let out = world::add(to, side);
    if !host.chunk_is_loaded(host.dimension(), [out[0] >> 4, out[2] >> 4]) {
        return Offer::Wait;
    }
    if belt::can_insert_from_side(&*host, out, side) != Some(true) {
        return Offer::Skip;
    }
    let mut one = stack.clone();
    one.count = 1;
    let angle = (util::random() % 360) as i32;
    let item = Item::dropped(one, 0, angle, true);
    match belt::handle_insertion(host, out, side, &item, false, false) {
        Some(rest) if rest.count == 0 => Offer::Gave,
        Some(_) => Offer::Wait,
        None => Offer::Skip,
    }
}

/// `BeltTunnelInteractionHandler.getTunnelOnPosition(level, belt) != null`.
fn covers(host: &dyn Host, belt: Pos) -> bool {
    tunnel_over(host, belt).is_some()
}

/// `flapTunnel(inventory, lastOffset, movementFacing, false)`: an item left the end of the belt.
fn left(host: &mut dyn Host, last: Pos, movement: Dir) {
    world::flap(host, world::above(last), movement, false);
}
