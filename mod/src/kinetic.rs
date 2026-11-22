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
use crate::compat::{BlockEntityDraw, Compound, Host, ModBlockEntity, Tag};

pub(crate) use crate::SPEED;
use crate::{float, set_float, util};

/// `KineticBlockEntity.overStressed`, saved and sent to clients for the goggles' warning.
pub(crate) const OVERSTRESSED: &str = "Overstressed";
/// `getTheoreticalSpeed` while overstressed, when `Speed` is 0: what the goggles' stats count.
pub(crate) const THEORY: &str = "Theory";

/// Keeps `Overstressed` and `Theory` in `nbt` as the network says; true when they changed.
/// Written only on a change, as `ModBlockEntity` marks every write.
pub(crate) fn note_stress(nbt: &mut Compound, overstressed: bool, theory: f32) -> bool {
    if overstressed {
        if util::boolean(nbt, OVERSTRESSED) && float(nbt, THEORY) == Some(theory) {
            return false;
        }
        nbt.insert(OVERSTRESSED.into(), Tag::Byte(1));
        set_float(nbt, THEORY, theory);
        true
    } else if util::has(nbt, OVERSTRESSED) {
        util::remove(nbt, OVERSTRESSED);
        util::remove(nbt, THEORY);
        true
    } else {
        false
    }
}

/// What the network says of the block at `pos`: overstressed, and the speed it would turn at.
pub(crate) fn stress_at(host: &dyn Host, pos: Pos) -> (bool, f32) {
    let networks = networks();
    let over = networks.network_at(host.dimension(), pos).is_some_and(|n| n.overstressed);
    (over, networks.theoretical_at(host.dimension(), pos).unwrap_or(0.0))
}

/// The goggles' keys in an update tag. Always sent, so a client drops them when they go.
pub(crate) fn copy_stress(from: &Compound, to: &mut Compound) {
    to.insert(OVERSTRESSED.into(), Tag::Byte(i8::from(util::boolean(from, OVERSTRESSED))));
    set_float(to, THEORY, float(from, THEORY).unwrap_or(0.0));
}

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
    tick_linked(host, pos, entity, node, None)
}

/// [`tick`] for a Chain Conveyor, whose chains join it to other conveyors (`links`, world
/// positions of the conveyors they reach): the network solves with them.
pub fn tick_linked(
    host: &mut dyn Host,
    pos: Pos,
    entity: &mut ModBlockEntity,
    node: Node,
    links: Option<&[Pos]>,
) -> Option<(f32, bool)> {
    let (speed, breaks) = match links {
        Some(links) => networks().report_linked(host.dimension(), pos, node, links),
        None => networks().report(host.dimension(), pos, node),
    };
    if breaks {
        host.destroy_block(pos, None, false);
        return None;
    }
    // Compared first: writing to a block entity, even the same value, counts as a change.
    let mut changed = float(&entity.nbt, SPEED) != Some(speed);
    if changed {
        set_float(&mut entity.nbt, SPEED, speed);
    }
    let (over, theory) = stress_at(host, pos);
    changed |= note_stress(&mut entity.nbt, over, theory);
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
    copy_stress(&entity.nbt, &mut tag);
    tag
}

/// A whole shaft through the block along `axis`, turning at `rpm` (0.3 degrees a tick per RPM,
/// as Create's `KineticBlockEntityRenderer`). Leaves the draw still.
/// It starts from the position's 22.5 degree offset (`getRotationOffsetForPosition`), as a plain shaft does.
pub fn draw_shaft(host: &dyn Host, draw: &mut BlockEntityDraw, axis: Axis, rpm: f32, pos: [i32; 3]) {
    draw.spin(axis.unit(), CENTRE, rpm * 0.3, crate::machines::encased::shaft_phase(axis, pos));
    host.draw_block(draw, "create:shaft", &[("axis", axis.name())]);
    draw.still();
}

/// Half a shaft sticking out of `face`, turning at `rpm`, from the position's offset. Leaves the draw still.
pub fn draw_shaft_half(host: &dyn Host, draw: &mut BlockEntityDraw, face: Dir, rpm: f32, pos: [i32; 3]) {
    crate::draw::draw_shaft_half(host, draw, face, rpm * 0.3, pos);
    draw.still();
}
