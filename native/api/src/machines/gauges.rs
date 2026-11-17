//! The Speedometer and the Stressometer in the game (`GaugeBlock`, `SpeedGaugeBlockEntity`,
//! `StressGaugeBlockEntity`, `GaugeRenderer`): a gauge sits on the network like a shaft and
//! shows the speed it turns at, or how much of the network's capacity is used, on a dial.
//!
//! The rules are `create_kinetics::rules::gauges`; its header lists the known differences. The
//! dial's value is the block entity's `Value` (as in Create); `From` and `Since` are this port's,
//! so a client can ease the dial from where it was on the server's clock.

use create_kinetics::rules::gauges::{
    Context, Placing, dial_after, head_faces, head_turn, horizontal, placement, rotation_axis,
    speed_dial, stress_dial, wrenched,
};
use create_kinetics::{Axis, Dir, Kind, Node, Pos, looking};
use horizon_client::mod_api::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, BlockStateId, Compound, Flow, Hooks, Host, LevelHost,
    PlaceBlockEvent, StandaloneModelsEvent, StateForPlacementEvent, Tag, UseItemOnEvent,
};

use super::encased;
use crate::{kinetic, util};

const SPEEDOMETER: &str = "create:speedometer";
const STRESSOMETER: &str = "create:stressometer";
const IDS: [&str; 2] = [SPEEDOMETER, STRESSOMETER];

const DIAL: &str = "create:block/gauge/dial";
const HEAD_SPEED: &str = "create:block/gauge/speedometer/head";
const HEAD_STRESS: &str = "create:block/gauge/stressometer/head";

/// The dial's target (`GaugeBlockEntity.dialTarget`, saved as `Value`), where it was when the
/// target last changed, and the game time that was.
const VALUE: &str = "Value";
const FROM: &str = "From";
const SINCE: &str = "Since";
/// The network's stress and capacity in su (`Network.Stress`, `Network.Capacity` in Create's tag).
const STRESS: &str = "Stress";
const CAPACITY: &str = "Capacity";
/// Ticks after which the dial has settled (an eighth of the way a tick: 0.875^120 is under 1e-7).
const SETTLED: f32 = 120.0;
/// `GaugeRenderer`'s `dialPivot`: 5.75 texels.
const DIAL_PIVOT: f32 = 5.75 / 16.0;
const CENTRE: [f32; 3] = [0.5; 3];

struct Gauges;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Gauges)
}

fn flag(host: &dyn Host, pos: Pos, key: &str) -> bool {
    host.block_property(pos, key).as_deref() == Some("true")
}

fn flag_name(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

impl Hooks for Gauges {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        for model in [DIAL, HEAD_SPEED, HEAD_STRESS] {
            event.models.push(model.to_owned());
        }
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        if !IDS.contains(&event.block) {
            return None;
        }
        let mut nbt = kinetic::new_entity();
        util::set_float(&mut nbt, VALUE, 0.0);
        util::set_float(&mut nbt, FROM, 0.0);
        nbt.insert(SINCE.into(), Tag::Long(0));
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
        let Some(facing) = util::facing(host, pos, "facing") else {
            return;
        };
        let axis = rotation_axis(facing, flag(host, pos, "axis_along_first"));
        // `CStress.setNoImpact`: a gauge joins the network without costing it anything.
        let Some((speed, mut changed)) = kinetic::tick(
            host,
            pos,
            event.entity,
            Node::passive(Kind::Shaft(axis)),
        ) else {
            return;
        };

        let target = if event.entity.id == SPEEDOMETER {
            speed_dial(speed)
        } else {
            let network = kinetic::networks().network_at(host.dimension(), pos);
            network.map_or(0.0, |n| stress_dial(n.stress, n.capacity, n.overstressed, speed))
        };
        let nbt = &event.entity.nbt;
        let had = util::float(nbt, VALUE).unwrap_or(0.0);
        if had != target {
            // The dial now stands where it had got to on its way to the old target.
            let now = util::game_time(host);
            let since = util::long(nbt, SINCE);
            let from = util::float(nbt, FROM).unwrap_or(0.0);
            let eased = dial_after(from, had, (now - since) as f32);
            util::set_float(&mut event.entity.nbt, VALUE, target);
            util::set_float(&mut event.entity.nbt, FROM, eased);
            event.entity.nbt.insert(SINCE.into(), Tag::Long(now));
            changed = true;
        }
        if event.entity.id == STRESSOMETER {
            // `StressGaugeBlockEntity.stress` and `capacity`, for the goggles' readout in su.
            let n = kinetic::networks().network_at(host.dimension(), pos).unwrap_or_default();
            for (key, value) in [(STRESS, n.stress), (CAPACITY, n.capacity)] {
                if util::float(&event.entity.nbt, key) != Some(value) {
                    util::set_float(&mut event.entity.nbt, key, value);
                    changed = true;
                }
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
        if !IDS.contains(&&*event.entity.id) {
            return None;
        }
        let nbt = &event.entity.nbt;
        let mut tag = kinetic::update_tag(event.entity);
        util::set_float(&mut tag, VALUE, util::float(nbt, VALUE).unwrap_or(0.0));
        util::set_float(&mut tag, FROM, util::float(nbt, FROM).unwrap_or(0.0));
        tag.insert(SINCE.into(), Tag::Long(util::long(nbt, SINCE)));
        for key in [STRESS, CAPACITY] {
            util::set_float(&mut tag, key, util::float(nbt, key).unwrap_or(0.0));
        }
        Some(tag)
    }

    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if !IDS.contains(&event.block) {
            return;
        }
        let Some(facing) = host
            .block_state_property(event.state, "facing")
            .and_then(Dir::from_name)
        else {
            return;
        };
        let along_first = host.block_state_property(event.state, "axis_along_first") == Some("true");
        let nbt = &event.entity.nbt;
        let rpm = kinetic::speed(event.entity);
        let value = util::float(nbt, VALUE).unwrap_or(0.0);
        let from = util::float(nbt, FROM).unwrap_or(0.0);
        let age = (event.draw.game_time - util::long(nbt, SINCE)) as f32 + event.draw.partial;
        let progress = dial_after(from, value, age);
        let head = if event.block == SPEEDOMETER {
            HEAD_SPEED
        } else {
            HEAD_STRESS
        };

        let draw = &mut *event.draw;
        // `ShaftRenderer`: the shaft through the gauge.
        encased::draw_shaft(
            host,
            draw,
            rotation_axis(facing, along_first),
            event.pos,
            rpm,
        );
        // `GaugeRenderer.renderSafe`: on each face that shows one, the head and the dial, both
        // modelled facing east. The dial turns a quarter about the x axis at its pivot as the
        // value goes from 0 to 1.
        for face in head_faces(facing, along_first) {
            draw.push();
            draw.rotate_about(Axis::Y.unit(), head_turn(face), CENTRE);
            draw.push();
            draw.rotate_about(
                Axis::X.unit(),
                -90.0 * progress,
                [0.0, DIAL_PIVOT, DIAL_PIVOT],
            );
            host.draw_model(draw, DIAL, 0xFF_FFFF);
            draw.pop();
            host.draw_model(draw, head, 0xFF_FFFF);
            draw.pop();
        }
        // The dial moves with the clock until it has settled; only then may the frame be kept.
        if age > SETTLED {
            draw.keep();
        }
    }

    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if !IDS.contains(&context.item.as_str()) {
            return None;
        }
        // The shafts beside it are looked for after placing (`place_block`).
        let placed = placement(
            Context {
                face: util::dir(context.face),
                horizontal: horizontal(context.yaw),
                looking: looking(context.yaw, context.pitch),
            },
            false,
            |_| false,
        );
        let state = level.state_with_property(event.state, "facing", placed.facing.name())?;
        level
            .state_with_property(state, "axis_along_first", flag_name(placed.along_first))
            .map(Some)
    }

    /// `GaugeBlock.getStateForPlacement` against the blocks that are there: a gauge placed on the
    /// end of a shaft turns to take it, and lines up with shafts beside it.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if !IDS.contains(&&*event.stack.item) {
            return;
        }
        let Some(player) = host.acting_player() else {
            return;
        };
        let pos = event.pos;
        // The clicked face is the facing the state for placement gave it.
        let Some(face) = util::facing(host, pos, "facing") else {
            return;
        };
        let beside = |host: &dyn Host, side: Dir| {
            encased::neighbour_kind(host, pos, side).is_some_and(|kind| kind.has_shaft(side.opposite()))
        };
        let on_shaft = encased::neighbour_kind(host, pos, face.opposite())
            .is_some_and(|kind| kind.has_shaft(face));
        let placed = placement(
            Context {
                face,
                horizontal: horizontal(player.yaw),
                looking: looking(player.yaw, player.pitch),
            },
            on_shaft,
            |side| beside(host, side),
        );
        set_placing(host, pos, placed);
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
            let Some(facing) = util::facing(host, pos, "facing") else {
                return;
            };
            let state = Placing {
                facing,
                along_first: flag(host, pos, "axis_along_first"),
            };
            set_placing(host, pos, wrenched(state, face.axis()));
        })
    }
}

fn set_placing(host: &mut dyn Host, pos: Pos, placing: Placing) {
    host.set_block_property(pos, "facing", placing.facing.name());
    host.set_block_property(pos, "axis_along_first", flag_name(placing.along_first));
}
