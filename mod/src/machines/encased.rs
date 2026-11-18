//! Encased shafts and encased cogwheels in the game (`EncasedShaftBlock`, `EncasedCogwheelBlock`,
//! `ShaftRenderer`, `EncasedCogRenderer`), and the helpers the other transmission modules
//! (clutch, gearshift, chain drive, gauges) share: placement along a neighbouring shaft, the
//! Wrench's quarter turn, switching a block for another, and the turning shaft.
//!
//! The rules are `create_kinetics::rules::encased`; its header lists the known differences. One
//! more here: a sneaking player placing one of these beside a shaft gets the axis they look along,
//! where Create gives the face clicked (a placement hook can't read the face).

use create_kinetics::generators::preferred_shaft;
use create_kinetics::rules::encased::{
    self, ANDESITE_CASING, BRASS_CASING, CogWrench, VARIANTS, cog_wrench, encased_by,
};
use create_kinetics::{Axis, Dir, Kind, Node, Pos, looking};
use crate::compat::{
    BlockEntityDraw, BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent,
    BlockEntityTicksEvent, BlockEntityUpdateTagEvent, BlockStateId, Compound, Flow, Hooks, Host,
    LevelHost, PlaceBlockEvent, StandaloneModelsEvent, StateForPlacementEvent, UseItemOnEvent,
};

use crate::{kinetic, util};

const CENTRE: [f32; 3] = [0.5; 3];
const SHAFT_HALF: &str = "create:block/shaft_half";
const SHAFTLESS_COGWHEEL: &str = "create:block/cogwheel_shaftless";
const SHAFTLESS_LARGE_COGWHEEL: &str = "create:block/large_cogwheel_shaftless";

struct Encased;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Encased)
}

// --- shared with the other transmission modules ---------------------------------------------

/// `KineticBlockEntityVisual.shouldOffset`: the sum of the position's coordinates across the
/// axis is even.
fn offset(axis: Axis, pos: Pos) -> bool {
    let sum: i32 = (0..3).filter(|i| *i != axis.index()).map(|i| pos[i]).sum();
    sum % 2 == 0
}

/// The turn a shaft along `axis` starts from at `pos` (`KineticBlockEntityVisual.rotationOffset`
/// without the large-cogwheel half step).
pub(crate) fn shaft_phase(axis: Axis, pos: Pos) -> f32 {
    if offset(axis, pos) { 22.5 } else { 0.0 }
}

/// The turn a cogwheel along `axis` starts from at `pos`: its teeth interleave with a neighbour's.
fn cog_phase(axis: Axis, pos: Pos, large: bool) -> f32 {
    match (offset(axis, pos), large) {
        (true, _) => 22.5,
        (false, true) => 11.25,
        (false, false) => 0.0,
    }
}

/// A whole shaft through the block along `axis` at `rpm` (`ShaftRenderer`).
pub(crate) fn draw_shaft(
    host: &dyn Host,
    draw: &mut BlockEntityDraw,
    axis: Axis,
    pos: Pos,
    rpm: f32,
) {
    draw.spin(axis.unit(), CENTRE, rpm * 0.3, shaft_phase(axis, pos));
    host.draw_block(draw, "create:shaft", &[("axis", axis.name())]);
    draw.still();
}

/// Half a shaft out of `face` at `rpm`, from `phase` degrees.
pub(crate) fn draw_half(
    host: &dyn Host,
    draw: &mut BlockEntityDraw,
    face: Dir,
    rpm: f32,
    phase: f32,
) {
    draw.spin(face.axis().unit(), CENTRE, rpm * 0.3, phase);
    draw.push();
    crate::draw::face_south(draw, face);
    host.draw_model(draw, SHAFT_HALF, 0xFF_FFFF);
    draw.pop();
    draw.still();
}

/// What the block at `side` of `pos` has told the network it is (`IRotate`).
pub(crate) fn neighbour_kind(host: &dyn Host, pos: Pos, side: Dir) -> Option<Kind> {
    kinetic::networks().kind_at(host.dimension(), util::add(pos, side.normal()))
}

/// `RotatedPillarKineticBlock.getPreferredAxis`: the axis the shaft ends beside `pos` point
/// along, when they agree.
fn preferred_axis(host: &dyn Host, pos: Pos) -> Option<Axis> {
    preferred_shaft(|side| neighbour_kind(host, pos, side)).map(Dir::axis)
}

/// `state_for_placement` of a block with an `axis`, when its item is one of `ids`: the axis the
/// player looks along (`getNearestLookingDirection`); the shaft beside it is looked for after
/// placing ([`align_axis`]).
pub(crate) fn placement_state(
    level: &dyn LevelHost,
    event: &StateForPlacementEvent<'_>,
    ids: &[&str],
) -> Option<Option<BlockStateId>> {
    let context = event.context;
    if !ids.contains(&context.item.as_str()) {
        return None;
    }
    let axis = looking(context.yaw, context.pitch).axis();
    level
        .state_with_property(event.state, "axis", axis.name())
        .map(Some)
}

/// After a block with an `axis` is placed in `ids`: lines it up with the shaft ends beside it
/// (`RotatedPillarKineticBlock.getStateForPlacement`; not for a sneaking player, see the module
/// note).
pub(crate) fn align_axis(host: &mut dyn Host, event: &PlaceBlockEvent<'_>, ids: &[&str]) {
    if !ids.contains(&&*event.stack.item) {
        return;
    }
    if host.acting_player().is_none_or(|player| player.sneaking) {
        return;
    }
    let Some(axis) = preferred_axis(host, event.pos) else {
        return;
    };
    if host.block_property(event.pos, "axis").as_deref() != Some(axis.name()) {
        host.set_block_property(event.pos, "axis", axis.name());
    }
}

/// The Wrench's quarter turn of a block with an `axis`, about the axis of the face clicked.
pub(crate) fn rotate_block(host: &mut dyn Host, pos: Pos, face: Dir) {
    let Some(axis) = host
        .block_property(pos, "axis")
        .as_deref()
        .and_then(Axis::from_name)
    else {
        return;
    };
    let next = encased::rotate_axis(axis, face.axis());
    if next != axis {
        host.set_block_property(pos, "axis", next.name());
    }
}

/// `KineticBlockEntity.switchToBlockState`: replaces the block at `pos` by `block` with the
/// state properties given (the rest default); the new block makes its own block entity.
pub(crate) fn switch_block(
    host: &mut dyn Host,
    pos: Pos,
    block: &str,
    properties: &[(&str, &str)],
) -> bool {
    let Some(state) = host.default_state(block) else {
        return false;
    };
    let dimension = host.dimension();
    if !host.set_block(dimension, pos, state, util::UPDATE_ALL) {
        return false;
    }
    for (key, value) in properties {
        host.set_block_property(pos, key, value);
    }
    true
}

fn is_mine(block: &str) -> bool {
    VARIANTS.iter().any(|(id, _, _)| *id == block)
}

fn bool_name(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

// --- the hooks -------------------------------------------------------------------------------

impl Hooks for Encased {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.push(SHAFTLESS_COGWHEEL.to_owned());
        event.models.push(SHAFTLESS_LARGE_COGWHEEL.to_owned());
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        is_mine(event.block).then(|| (event.block.to_owned(), kinetic::new_entity()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if is_mine(event.block) {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if !is_mine(&event.entity.id) {
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
        let kind = match encased::cog_kind(&event.entity.id) {
            None => Kind::Shaft(axis),
            Some(large) => {
                let on = |key| host.block_property(pos, key).as_deref() == Some("true");
                Kind::encased_cog(axis, large, on("top_shaft"), on("bottom_shaft"))
            }
        };
        // `CStress.setNoImpact`: no stress of its own.
        let Some((_, changed)) = kinetic::tick(host, pos, event.entity, Node::passive(kind)) else {
            return;
        };
        if changed {
            host.block_entity_sync(pos);
        }
    }

    fn block_entity_update_tag(
        &self,
        event: &mut BlockEntityUpdateTagEvent<'_>,
    ) -> Option<Compound> {
        is_mine(&event.entity.id).then(|| kinetic::update_tag(event.entity))
    }

    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if !is_mine(event.block) {
            return;
        }
        let Some(axis) = host
            .block_state_property(event.state, "axis")
            .and_then(Axis::from_name)
        else {
            return;
        };
        let rpm = kinetic::speed(event.entity);
        let pos = event.pos;
        let draw = &mut *event.draw;
        match encased::cog_kind(event.block) {
            // `ShaftRenderer`.
            None => draw_shaft(host, draw, axis, pos, rpm),
            Some(large) => {
                // `EncasedCogRenderer`: the cogwheel without its shaft, then a half shaft out
                // of each end that has one (`hasShaftTowards`).
                draw.spin(axis.unit(), CENTRE, rpm * 0.3, cog_phase(axis, pos, large));
                draw.push();
                // The partial models are drawn for the y axis (`partialFacingVertical`).
                match axis {
                    Axis::X => draw.rotate_about(Axis::Z.unit(), -90.0, CENTRE),
                    Axis::Z => draw.rotate_about(Axis::X.unit(), 90.0, CENTRE),
                    Axis::Y => {}
                }
                let model = if large {
                    SHAFTLESS_LARGE_COGWHEEL
                } else {
                    SHAFTLESS_COGWHEEL
                };
                host.draw_model(draw, model, 0xFF_FFFF);
                draw.pop();
                draw.still();
                let kind_on = |key| host.block_state_property(event.state, key) == Some("true");
                for (face, key) in [(positive(axis), "top_shaft"), (negative(axis), "bottom_shaft")]
                {
                    if kind_on(key) {
                        // A large cogwheel's shaft isn't offset by the half step
                        // (`getAngleForLargeCogShaft`).
                        let phase = if large {
                            shaft_phase(axis, pos)
                        } else {
                            cog_phase(axis, pos, false)
                        };
                        draw_half(host, draw, face, rpm, phase);
                    }
                }
            }
        }
        // It only turns at a steady speed, through `spin`.
        draw.keep();
    }

    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let ids: Vec<&str> = VARIANTS.iter().map(|(id, _, _)| *id).collect();
        placement_state(level, event, &ids)
    }

    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        let ids: Vec<&str> = VARIANTS.iter().map(|(id, _, _)| *id).collect();
        align_axis(host, event, &ids);
    }

    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let item = &*event.stack.item;
        let pos = event.pos;
        if item == ANDESITE_CASING || item == BRASS_CASING {
            return encase(host, pos, item);
        }
        if item != util::WRENCH {
            return Flow::Continue;
        }
        let id = host.block(pos);
        if !is_mine(&id) {
            return Flow::Continue;
        }
        let Some(face) = Dir::from_normal(event.face) else {
            return Flow::Continue;
        };
        wrench(host, pos, &id, face)
    }
}

fn positive(axis: Axis) -> Dir {
    [Dir::Up, Dir::South, Dir::East]
        .into_iter()
        .find(|d| d.axis() == axis)
        .expect("a positive end")
}

fn negative(axis: Axis) -> Dir {
    positive(axis).opposite()
}

/// `EncasableBlock.tryEncase` through `ShaftBlock.useItemOn` and `CogWheelBlock.useItemOn`: a
/// casing right-clicked on a shaft or cogwheel, not sneaking. The casing isn't used up, as in
/// Create.
fn encase(host: &mut dyn Host, pos: Pos, casing: &str) -> Flow {
    if host.acting_player().is_none_or(|player| player.sneaking) {
        return Flow::Continue;
    }
    let plain = host.block(pos);
    let Some(block) = encased_by(casing, &plain) else {
        return Flow::Continue;
    };
    let Some(axis) = host
        .block_property(pos, "axis")
        .as_deref()
        .and_then(Axis::from_name)
    else {
        return Flow::Continue;
    };
    if encased::cog_kind(block).is_some() {
        // `EncasedCogwheelBlock.handleEncasing`: a shaft end for each side that has one pointing in.
        let (top, bottom) = encased::encased_cog_shafts(axis, |side| {
            neighbour_kind(host, pos, side).is_some_and(|kind| kind.has_shaft(side.opposite()))
        });
        switch_block(
            host,
            pos,
            block,
            &[
                ("axis", axis.name()),
                ("top_shaft", bool_name(top)),
                ("bottom_shaft", bool_name(bottom)),
            ],
        );
    } else {
        switch_block(host, pos, block, &[("axis", axis.name())]);
    }
    Flow::Cancel
}

/// `EncasedShaftBlock.onSneakWrenched` and `EncasedCogwheelBlock.onWrenched`/`onSneakWrenched`.
fn wrench(host: &mut dyn Host, pos: Pos, id: &str, face: Dir) -> Flow {
    let Some(axis) = host
        .block_property(pos, "axis")
        .as_deref()
        .and_then(Axis::from_name)
    else {
        return Flow::Continue;
    };
    let cog = encased::cog_kind(id);
    if host.acting_player().is_some_and(|player| player.sneaking) {
        // Back to the plain block, on the same axis; the casing is lost.
        if let Some(plain) = encased::plain_of(id) {
            switch_block(host, pos, plain, &[("axis", axis.name())]);
            return Flow::Cancel;
        }
        // A plain shaft or cogwheel: `IWrenchable.onSneakWrenched` picks the block up.
        return util::wrench(host, pos, |_| {});
    }
    if cog.is_none() {
        rotate_block(host, pos, face);
        return Flow::Cancel;
    }
    let on = |host: &dyn Host, key| host.block_property(pos, key).as_deref() == Some("true");
    let (top, bottom) = (on(host, "top_shaft"), on(host, "bottom_shaft"));
    match cog_wrench(axis, face) {
        CogWrench::ToggleTop => {
            host.set_block_property(pos, "top_shaft", bool_name(!top));
        }
        CogWrench::ToggleBottom => {
            host.set_block_property(pos, "bottom_shaft", bool_name(!bottom));
        }
        CogWrench::Turn => {
            let (next, top, bottom) = encased::rotate_cog(axis, top, bottom, face.axis());
            host.set_block_property(pos, "axis", next.name());
            host.set_block_property(pos, "top_shaft", bool_name(top));
            host.set_block_property(pos, "bottom_shaft", bool_name(bottom));
        }
    }
    Flow::Cancel
}
