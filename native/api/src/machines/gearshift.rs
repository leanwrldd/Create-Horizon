//! The Gearshift in the game (`GearshiftBlock`, `GearshiftBlockEntity`, `SplitShaftRenderer`),
//! and the split shaft it shares with the Clutch (`machines/clutch.rs`): a casing with a shaft
//! out of each end that redstone power reverses or cuts.
//!
//! The rules are `create_kinetics::rules::gearshift` and `Kind::gearshift`/`Kind::clutch`; the
//! header of the rules file lists the known differences.

use create_kinetics::rules::gearshift::{Split, clutch_input, ends, half_speed, powered};
use create_kinetics::{Axis, Dir, Kind, Node, Pos, face_sign};
use horizon_client::mod_api::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, BlockStateId, Compound, Flow, Hooks, Host, LevelHost,
    PlaceBlockEvent, StateForPlacementEvent, Tag, UseItemOnEvent,
};

use super::encased;
use crate::{kinetic, util};

/// The end a powered clutch takes its rotation from, for the drawing (`getSourceFacing`, which
/// Create keeps as the block entity's `Source`).
const INPUT: &str = "InputFace";

pub const GEARSHIFT: &str = "create:gearshift";

/// One kind of split shaft: the Gearshift here, the Clutch in `clutch.rs`.
pub(crate) struct SplitShaft {
    id: &'static str,
    split: Split,
}

impl SplitShaft {
    pub(crate) fn new(id: &'static str, split: Split) -> Self {
        SplitShaft { id, split }
    }

    fn input_of(entity: &Compound) -> Option<Dir> {
        match util::get(entity, INPUT) {
            Some(Tag::String(name)) => Dir::from_name(&name.to_string_lossy()),
            _ => None,
        }
    }
}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(SplitShaft::new(GEARSHIFT, Split::Gearshift))
}

/// The speed of the shaft end the block at `side` of `pos` points at it, if it has one.
fn neighbour_speed(host: &dyn Host, pos: Pos, side: Dir) -> Option<f32> {
    let at = util::add(pos, side.normal());
    let dimension = host.dimension();
    let networks = kinetic::networks();
    let kind = networks.kind_at(dimension, at)?;
    let end = side.opposite();
    if !kind.has_shaft(end) {
        return None;
    }
    networks
        .speed_at(dimension, at)
        .map(|speed| speed * face_sign(kind, end))
}

impl Hooks for SplitShaft {
    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == self.id).then(|| (self.id.to_owned(), kinetic::new_entity()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == self.id {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != self.id {
            return;
        }
        let pos = event.pos;
        let Some(axis) = host
            .block_property(pos, "axis")
            .as_deref()
            .and_then(Axis::from_name)
        else {
            return;
        };
        // `neighborChanged`: the block follows the signal. Create re-attaches the network a tick
        // after a change; here the next solve sees the new kind.
        let signal = powered(host.neighbor_signal(host.dimension(), pos));
        let was = host.block_property(pos, "powered").as_deref() == Some("true");
        if was != signal {
            host.set_block_property(pos, "powered", if signal { "true" } else { "false" });
        }
        let kind = match (signal, self.split) {
            (false, _) => Kind::Shaft(axis),
            (true, Split::Gearshift) => Kind::gearshift(axis),
            (true, Split::Clutch) => Kind::clutch(axis),
        };
        // `CStress.setNoImpact`.
        let Some((speed, mut changed)) =
            kinetic::tick(host, pos, event.entity, Node::passive(kind))
        else {
            return;
        };
        if self.split == Split::Clutch {
            let before = Self::input_of(&event.entity.nbt);
            let input = if signal {
                clutch_input(axis, speed, before, |side| neighbour_speed(host, pos, side))
            } else {
                None
            };
            if input != before {
                let name = input.map_or("", Dir::name);
                event.entity.nbt.insert(INPUT.into(), Tag::String(name.into()));
                changed = true;
            }
        }
        if changed {
            host.block_entity_sync(pos);
        }
    }

    fn block_entity_update_tag(
        &self,
        event: &mut BlockEntityUpdateTagEvent<'_>,
    ) -> Option<Compound> {
        if event.entity.id != self.id {
            return None;
        }
        let mut tag = kinetic::update_tag(event.entity);
        if self.split == Split::Clutch {
            let name = Self::input_of(&event.entity.nbt).map_or("", Dir::name);
            tag.insert(INPUT.into(), Tag::String(name.into()));
        }
        Some(tag)
    }

    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != self.id {
            return;
        }
        let Some(axis) = host
            .block_state_property(event.state, "axis")
            .and_then(Axis::from_name)
        else {
            return;
        };
        let powered = host.block_state_property(event.state, "powered") == Some("true");
        let rpm = kinetic::speed(event.entity);
        let input = Self::input_of(&event.entity.nbt);
        // `SplitShaftRenderer`: each half at the speed times the face's modifier.
        for face in ends(axis) {
            let speed = half_speed(self.split, powered, rpm, face, input);
            encased::draw_half(host, &mut *event.draw, face, speed, encased::shaft_phase(axis, event.pos));
        }
        event.draw.keep();
    }

    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        encased::placement_state(level, event, &[self.id])
    }

    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        encased::align_axis(host, event, &[self.id]);
    }

    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != self.id {
            return Flow::Continue;
        }
        let Some(face) = Dir::from_normal(event.face) else {
            return Flow::Continue;
        };
        // `IWrenchable`: sneaking picks it up, otherwise a quarter turn about the face's axis.
        util::wrench(host, event.pos, |host| {
            encased::rotate_block(host, event.pos, face);
        })
    }
}
