//! The Encased Chain Drive and the Adjustable Chain Gearshift in the game (`ChainDriveBlock`,
//! `ChainGearshiftBlock`, `ChainGearshiftBlockEntity`, `ShaftRenderer`): casings with a shaft
//! through them that turn together with the next casing along a chain, the gearshift at a speed
//! its redstone signal scales.
//!
//! The rules are `create_kinetics::rules::chain_drive` and `create_kinetics::Chain`; the header of
//! the rules file lists the known differences.

use create_kinetics::rules::chain_drive::{self, SIGNAL, State, powered, settle, update_shape};
use create_kinetics::{Axis, Chain, ChainPart, Dir, Kind, Node, Pos};
use horizon_client::mod_api::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, BlockStateId, Compound, Flow, Hooks, Host, LevelHost,
    PlaceBlockEvent, StateForPlacementEvent, Tag, UseItemOnEvent,
};

use super::encased;
use crate::{kinetic, util};

const DRIVE: &str = "create:encased_chain_drive";
const GEARSHIFT: &str = "create:adjustable_chain_gearshift";
const IDS: [&str; 2] = [DRIVE, GEARSHIFT];

struct ChainDrive;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(ChainDrive)
}

/// The chain state of the block at `pos`, if it is a chain drive or a chain gearshift (both are
/// `ChainDriveBlock`s to `updateShape`).
fn state_at(host: &dyn Host, pos: Pos) -> Option<State> {
    if !IDS.contains(&host.block(pos).as_str()) {
        return None;
    }
    Some(State {
        axis: Axis::from_name(&host.block_property(pos, "axis")?)?,
        part: ChainPart::from_name(&host.block_property(pos, "part")?)?,
        along_first: host.block_property(pos, "axis_along_first")? == "true",
    })
}

fn side_state(host: &dyn Host, pos: Pos, face: Dir) -> Option<State> {
    state_at(host, util::add(pos, face.normal()))
}

fn set_state(host: &mut dyn Host, pos: Pos, state: State) {
    host.set_block_property(pos, "axis", state.axis.name());
    host.set_block_property(pos, "part", state.part.name());
    host.set_block_property(
        pos,
        "axis_along_first",
        if state.along_first { "true" } else { "false" },
    );
}

impl Hooks for ChainDrive {
    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        if !IDS.contains(&event.block) {
            return None;
        }
        let mut nbt = kinetic::new_entity();
        if event.block == GEARSHIFT {
            nbt.insert(SIGNAL.into(), Tag::Int(0));
        }
        Some((event.block.to_owned(), nbt))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if IDS.contains(&event.block) {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if !IDS.contains(&&*event.entity.id) {
            return;
        }
        let pos = event.pos;
        let Some(state) = state_at(host, pos) else {
            return;
        };
        let mut sync = false;

        // `ChainGearshiftBlockEntity.neighbourChanged` and `ChainGearshiftBlock.neighborChanged`.
        let mut signal = 0;
        if event.entity.id == GEARSHIFT {
            signal = host.neighbor_signal(host.dimension(), pos).clamp(0, 15);
            let was = host.block_property(pos, "powered").as_deref() == Some("true");
            if was != powered(signal) {
                host.set_block_property(pos, "powered", if powered(signal) { "true" } else { "false" });
            }
            if util::int(&event.entity.nbt, SIGNAL) != signal {
                event.entity.nbt.insert(SIGNAL.into(), Tag::Int(signal));
            }
        }

        // `updateShape` for each side, as if the neighbours had just changed.
        let mut now = state;
        for face in Dir::ALL.into_iter().filter(|face| face.axis() != state.axis) {
            now = update_shape(now, face, side_state(host, pos, face));
        }
        if now != state {
            set_state(host, pos, now);
        }

        let chain = Chain {
            axis: now.axis,
            along_first: now.along_first,
            part: now.part,
            signal: signal as u8,
        };
        // `CStress.setNoImpact`.
        let Some((_, changed)) =
            kinetic::tick(host, pos, event.entity, Node::passive(Kind::chain(chain)))
        else {
            return;
        };
        sync |= changed;
        if sync {
            host.block_entity_sync(pos);
        }
    }

    fn block_entity_update_tag(
        &self,
        event: &mut BlockEntityUpdateTagEvent<'_>,
    ) -> Option<Compound> {
        IDS.contains(&&*event.entity.id)
            .then(|| kinetic::update_tag(event.entity))
    }

    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if !IDS.contains(&event.block) {
            return;
        }
        let Some(axis) = host
            .block_state_property(event.state, "axis")
            .and_then(Axis::from_name)
        else {
            return;
        };
        // `ShaftRenderer`: the shaft through the casing.
        let rpm = kinetic::speed(event.entity);
        encased::draw_shaft(host, &mut *event.draw, axis, event.pos, rpm);
        event.draw.keep();
    }

    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        // `part` and `axis_along_first` settle in the block entity's first tick.
        encased::placement_state(level, event, &IDS)
    }

    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        encased::align_axis(host, event, &IDS);
    }

    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        if &*event.stack.item != util::WRENCH || !IDS.contains(&host.block(pos).as_str()) {
            return Flow::Continue;
        }
        let Some(face) = Dir::from_normal(event.face) else {
            return Flow::Continue;
        };
        util::wrench(host, pos, |host| {
            // `getRotatedBlockState` then `updateAfterWrenched`: turned, then settled afresh
            // against the neighbours.
            let Some(state) = state_at(host, pos) else {
                return;
            };
            let axis = chain_drive::wrenched_axis(state, face.axis());
            let fresh = settle(axis, |side| side_state(host, pos, side));
            set_state(host, pos, fresh);
        })
    }
}
