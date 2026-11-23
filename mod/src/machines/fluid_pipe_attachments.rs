//! The arms, rims and drains of the pipes (`PipeAttachmentModel`): Create's baked model wrapper
//! adds, to the chunk model of a Fluid Pipe, a Glass Fluid Pipe, an Encased Fluid Pipe, a Smart
//! Fluid Pipe, a Fluid Valve and a Mechanical Pump, a partial model on each side
//! (`block/fluid_pipe/{connection,rim_connector,rim,drain}/<dir>`) and a casing on a junction. The
//! chunk model here draws only the core, so the block entity draws the rest.
//!
//! The choice per side is `create_kinetics::rules::fluid_pipe_attachments`; this is the glue. A
//! block entity's tick reads what is beside each side, asks the rule, and keeps the answer in its
//! NBT under `Rims` (seven bytes: six sides and the casing), written only when it changes. The
//! block entity draws the answer and keeps the geometry (`block_entity_submit`).
//!
//! The Fluid Pipe, the Valve, the Smart Pipe and the Pump make their own block entities in their
//! modules; this one makes the Glass and the Encased Fluid Pipe's, which had none, and ticks all of
//! them. Their update tags carry `Rims` too: `copy_rims` is what a module that answers its own
//! update tag (the valve, the pump, the bracket for a pipe) calls.
//!
//! Known differences from Create: see `rules/fluid_pipe_attachments.rs`. Also, the attachments are
//! looked at every few ticks, not at each neighbour's change, so an arm may show a moment late
//! after a pipe is placed or removed, and a pipe's partials are lit as a block entity's are, not by
//! the chunk's ambient occlusion (`PipeAttachmentModel.withAO`).

use create_kinetics::rules::fluid_pipe::{Sides, straight_axis};
use create_kinetics::rules::fluid_pipe_attachments::{self as rules, Beside, Block, Rims};
use create_kinetics::Dir;
use crate::compat::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, Compound, Hooks, Host, StandaloneModelsEvent, Tag,
};

use super::fluids::{
    self, ENCASED_FLUID_PIPE, FLUID_PIPE, FLUID_VALVE, GLASS_FLUID_PIPE, ITEM_DRAIN, MECHANICAL_PUMP,
    SMART_FLUID_PIPE, SPOUT, direction,
};
use crate::util;

type Pos = [i32; 3];

/// The block entity's key: [`rules::encode`].
pub const KEY: &str = "Rims";
/// How often a block looks at what is beside it, in ticks.
const REFRESH: i64 = 4;

struct Attachments;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Attachments)
}

/// The blocks this module draws on.
fn is_ours(block: &str) -> bool {
    matches!(
        block,
        FLUID_PIPE | GLASS_FLUID_PIPE | ENCASED_FLUID_PIPE | SMART_FLUID_PIPE | FLUID_VALVE | MECHANICAL_PUMP
    )
}

/// The blocks whose block entity this module makes: the ones with none of their own.
fn is_made_here(block: &str) -> bool {
    matches!(block, GLASS_FLUID_PIPE | ENCASED_FLUID_PIPE)
}

/// `Rims` from a block entity's NBT into an update tag, when it holds one.
pub fn copy_rims(tag: &mut Compound, nbt: &Compound) {
    if let Some(rims) = util::get(nbt, KEY) {
        tag.insert(KEY.into(), rims.clone());
    }
}

fn sides_of(host: &dyn Host, pos: Pos) -> Sides {
    Dir::ALL.map(|d| host.block_property(pos, d.name()).as_deref() == Some("true"))
}

fn facing(host: &dyn Host, pos: Pos) -> Option<Dir> {
    host.block_property(pos, "facing").as_deref().and_then(Dir::from_name)
}

/// The block at `pos`, as the rule knows it.
fn own_block(host: &dyn Host, pos: Pos) -> Option<Block> {
    Some(match host.block(pos).as_str() {
        FLUID_PIPE => Block::Pipe(sides_of(host, pos)),
        ENCASED_FLUID_PIPE => Block::Encased(sides_of(host, pos)),
        GLASS_FLUID_PIPE => Block::Glass(fluids::pipe_axis(host, pos)?),
        FLUID_VALVE => Block::Valve(fluids::pipe_axis(host, pos)?),
        SMART_FLUID_PIPE => Block::Smart(fluids::pipe_axis(host, pos)?),
        MECHANICAL_PUMP => Block::Pump(facing(host, pos)?),
        _ => return None,
    })
}

/// Whether the block at `pos` holds a bracket (`BracketedBlockEntityBehaviour.isBracketPresent`):
/// the pipe's block entity keeps it under `Bracket`, an empty compound for none.
fn has_bracket(host: &dyn Host, pos: Pos) -> bool {
    host.mod_block_entity(pos)
        .is_some_and(|entity| matches!(util::get(&entity.nbt, "Bracket"), Some(Tag::Compound(b)) if !b.is_empty()))
}

/// What stands at `pos`, beside the block that looks at it on `side`.
fn beside(host: &dyn Host, pos: Pos) -> Beside {
    match host.block(pos).as_str() {
        FLUID_PIPE => {
            let straight = straight_axis(&sides_of(host, pos));
            Beside::Pipe { bracket: has_bracket(host, pos), straight }
        }
        // `EncasedPipeBlock.FACING_TO_PROPERTY_MAP.get(side.getOpposite())` is read by the caller.
        ENCASED_FLUID_PIPE => Beside::Encased { open_toward_us: false },
        GLASS_FLUID_PIPE => fluids::pipe_axis(host, pos).map_or(Beside::Other, Beside::Glass),
        FLUID_VALVE => fluids::pipe_axis(host, pos).map_or(Beside::Other, Beside::Valve),
        SMART_FLUID_PIPE => fluids::pipe_axis(host, pos).map_or(Beside::Other, Beside::Smart),
        MECHANICAL_PUMP => facing(host, pos).map_or(Beside::Other, Beside::Pump),
        _ => Beside::Other,
    }
}

/// `FluidPropagator.hasFluidCapability(world, pos, face)`: a block entity with a fluid handler on
/// that face. The pipes and the pump have none; the Spout's tank is closed below and the drain's
/// above (an empty list there).
fn has_capability(host: &mut dyn Host, pos: Pos, face: Dir) -> bool {
    match host.block(pos).as_str() {
        "minecraft:air" | FLUID_PIPE | GLASS_FLUID_PIPE | ENCASED_FLUID_PIPE | SMART_FLUID_PIPE
        | FLUID_VALVE | MECHANICAL_PUMP => false,
        SPOUT | ITEM_DRAIN => host
            .fluid_tanks(host.dimension(), pos, direction(face))
            .is_some_and(|tanks| !tanks.is_empty()),
        _ => fluids::has_fluid_capability(host, pos, direction(face)),
    }
}

/// `PipeAttachmentModel.gatherModelData` for the block at `pos`.
fn read(host: &mut dyn Host, pos: Pos) -> Option<Rims> {
    let own = own_block(&*host, pos)?;
    let mut rims = Rims::NONE;
    for side in Dir::ALL {
        let next = util::add(pos, side.normal());
        let mut around = beside(&*host, next);
        if matches!(around, Beside::Encased { .. }) {
            let open = host.block_property(next, side.opposite().name()).as_deref() == Some("true");
            around = Beside::Encased { open_toward_us: open };
        }
        let capability = has_capability(host, next, side.opposite());
        rims.attachments[side as usize] = rules::attachment(&own, side, around, capability);
    }
    // `FluidPipeBlock.shouldDrawCasing` is for a pipe only.
    if let Block::Pipe(sides) = own {
        rims.casing = rules::draws_casing(&sides);
    }
    Some(rims)
}

/// Draws a block's `Rims`, if its NBT holds them: the partial models, kept.
fn draw(host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
    let Some(Tag::ByteArray(bytes)) = util::get(&event.entity.nbt, KEY) else {
        return;
    };
    let Some(rims) = rules::decode(bytes) else {
        return;
    };
    event.draw.still();
    for model in rules::models(&rims) {
        host.draw_model(&mut *event.draw, &model, 0xFF_FFFF);
    }
    // The partials change only with the entity's data, and a new `Rims` bumps its revision.
    event.draw.keep();
}

impl Hooks for Attachments {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.extend(rules::all_models());
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        is_made_here(event.block).then(|| (event.block.to_owned(), Compound::new()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if is_ours(event.block) {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if !is_ours(&event.entity.id) {
            return;
        }
        let pos = event.pos;
        let salt = i64::from(pos[0]) + 3 * i64::from(pos[1]) + 7 * i64::from(pos[2]);
        let known = util::has(&event.entity.nbt, KEY);
        if known && (util::game_time(host) + salt).rem_euclid(REFRESH) != 0 {
            return;
        }
        let Some(rims) = read(&mut *host, pos) else {
            return;
        };
        let value = Tag::ByteArray(rules::encode(&rims));
        // Comparing first matters: any `&mut` to the entity bumps its revision and rebuilds its
        // kept geometry.
        if !util::differs(&event.entity.nbt, KEY, &value) {
            return;
        }
        event.entity.nbt.insert(KEY.into(), value);
        host.block_entity_sync(pos);
    }

    /// The update tag of the blocks whose block entity is made here and of the Smart Fluid Pipe
    /// (which has none of its own); the others add `Rims` to theirs with [`copy_rims`].
    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        if !matches!(event.entity.id.as_str(), GLASS_FLUID_PIPE | ENCASED_FLUID_PIPE | SMART_FLUID_PIPE) {
            return None;
        }
        let mut tag = Compound::new();
        copy_rims(&mut tag, &event.entity.nbt);
        Some(tag)
    }

    /// `PipeAttachmentModel.addQuads`: the attachments and the casing, after the block's own model.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if is_ours(event.block) {
            draw(host, event);
        }
    }
}
