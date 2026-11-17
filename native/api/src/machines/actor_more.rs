//! The actors that need more than a block break or a pick-up: the Redstone Contact
//! (`ContactMovementBehaviour`) and the Contraption Controls (which only decide what is disabled,
//! `actor.rs`). The Roller and the Seat's riders are not ported. The rest are in `actor_tools.rs`.

use create_kinetics::rules::actor::Kind;
use create_kinetics::rules::redstone_contact as rules;
use create_kinetics::Pos;
use horizon_client::mod_api::{Hooks, Host};

use super::actor::{Actor, Cx};
use super::actor_tools::Extra;
use super::diodes_signal as signal;
use super::redstone_contact as contact;
use crate::util;

struct Shared;

impl Hooks for Shared {}

/// A module of shared code: no hooks of its own.
pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

fn last_of(actor: &Actor) -> Option<Pos> {
    match actor.extra {
        Extra::Contact(last) => last,
        _ => None,
    }
}

/// `ContactMovementBehaviour.visitNewPosition`.
pub(super) fn visit(cx: &mut Cx<'_>, actor: &mut Actor, cell: Pos) {
    if actor.kind != Kind::Contact {
        return;
    }
    let Some(facing) = actor.state.dir("facing") else { return };
    let own = super::actor::nearest(cx.rotated(facing.normal().map(f64::from)));
    let visited = {
        let host: &dyn Host = &*cx.host;
        let block = host.block(cell);
        let is_contact = block == rules::ID || block == rules::ELEVATOR_ID;
        if is_contact { util::facing(host, cell, "facing").map(|f| (block == rules::ID, f)) } else { None }
    };
    let level = signal::host_level(&*cx.host);
    let result = rules::visit(actor.ctx.first_movement, last_of(actor), cell, own, visited);
    if let Some(old) = result.release {
        contact::release(level, old);
    }
    if let Some(new) = result.power {
        contact::hold(level, new);
    }
    actor.extra = Extra::Contact(result.last);
}

pub(super) fn tick(_cx: &mut Cx<'_>, _actor: &mut Actor) {}

/// `ContactMovementBehaviour.stopMoving`: `deactivateLastVisitedContact`.
pub(super) fn stop(cx: &mut Cx<'_>, actor: &mut Actor) {
    if let Some(old) = last_of(actor) {
        contact::release(signal::host_level(&*cx.host), old);
    }
    actor.extra = Extra::None;
}
