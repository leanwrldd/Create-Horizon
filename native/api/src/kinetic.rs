//! A machine's place on its rotation network, for the modules under `machines/`.
//!
//! A machine's block entity calls [`tick`] once each server tick with what it is to the network
//! (a [`Node`]: where its shafts are, what it costs or makes) and gets its speed back. The speed
//! is kept in the block entity's NBT under [`SPEED`]; send it to clients with [`update_tag`] and
//! draw turning shafts with [`draw_shaft`] or [`draw_shaft_half`].

// Shared with the machine modules, which don't all use every helper.
#![allow(dead_code, unused_imports)]

use std::sync::MutexGuard;

use create_kinetics::{Axis, Dir, Networks, Node, Pos};
use horizon_client::mod_api::{BlockEntityDraw, Compound, Host, ModBlockEntity};

pub(crate) use crate::SPEED;
use crate::{float, set_float};

const CENTRE: [f32; 3] = [0.5; 3];

/// The mod's one table of kinetic blocks. Never hold it across a call into the game: a block
/// breaking runs hooks again.
pub fn networks() -> MutexGuard<'static, Networks> {
    crate::networks()
}

/// One server tick of a kinetic block entity at `pos`: reports `node` and answers the block's
/// speed in RPM and whether it changed this tick (then sync the block entity). `None` when the
/// network jammed and the block was destroyed, as Create's blocks are: stop there.
///
/// Call it every tick, even when the machine is idle: a block that stops reporting drops out of
/// its network.
pub fn tick(
    host: &mut dyn Host,
    pos: Pos,
    entity: &mut ModBlockEntity,
    node: Node,
) -> Option<(f32, bool)> {
    let (speed, breaks) = networks().report(host.dimension(), pos, node);
    if breaks {
        host.destroy_block(pos, None, false);
        return None;
    }
    // Compared first: writing to a block entity, even the same value, counts as a change.
    let changed = float(&entity.nbt, SPEED) != Some(speed);
    if changed {
        set_float(&mut entity.nbt, SPEED, speed);
    }
    Some((speed, changed))
}

/// The speed last stored by [`tick`], in RPM.
pub fn speed(entity: &ModBlockEntity) -> f32 {
    float(&entity.nbt, SPEED).unwrap_or(0.0)
}

/// A fresh block entity's NBT with a speed of zero: start `Hooks::block_entity` from it.
pub fn new_entity() -> Compound {
    let mut nbt = Compound::new();
    set_float(&mut nbt, SPEED, 0.0);
    nbt
}

/// An update tag holding the speed: start `Hooks::block_entity_update_tag` from it.
pub fn update_tag(entity: &ModBlockEntity) -> Compound {
    let mut tag = Compound::new();
    set_float(&mut tag, SPEED, speed(entity));
    tag
}

/// A whole shaft through the block along `axis`, turning at `rpm` (0.3 degrees a tick per RPM,
/// as Create's `KineticBlockEntityRenderer`). Leaves the draw still.
pub fn draw_shaft(host: &dyn Host, draw: &mut BlockEntityDraw, axis: Axis, rpm: f32) {
    draw.spin(axis.unit(), CENTRE, rpm * 0.3, 0.0);
    host.draw_block(draw, "create:shaft", &[("axis", axis.name())]);
    draw.still();
}

/// Half a shaft sticking out of `face`, turning at `rpm`. Leaves the draw still.
pub fn draw_shaft_half(host: &dyn Host, draw: &mut BlockEntityDraw, face: Dir, rpm: f32) {
    crate::draw::draw_shaft_half(host, draw, face, rpm * 0.3);
    draw.still();
}
