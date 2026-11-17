//! The actors that need more than a block break or a pick-up: the Contraption Controls (which
//! only decide what is disabled, `actor.rs`), the Seat, the Redstone Contact and the Roller. The
//! rest are in `actor_tools.rs`.

use create_kinetics::Pos;
use horizon_client::mod_api::{Hooks, Host};

use super::actor::{Actor, Cx};

struct Shared;

impl Hooks for Shared {}

/// A module of shared code: no hooks of its own.
pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

pub(super) fn visit(_cx: &mut Cx<'_>, _actor: &mut Actor, _cell: Pos) {}

pub(super) fn tick(_cx: &mut Cx<'_>, _actor: &mut Actor) {}

pub(super) fn stop(_cx: &mut Cx<'_>, _actor: &mut Actor) {}

#[allow(dead_code)]
fn unused(_: &dyn Host) {}
