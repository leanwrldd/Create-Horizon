//! The Spout (`create:spout`; `SpoutBlock`, `SpoutBlockEntity`, `SpoutRenderer`,
//! `FillingBySpout`): two blocks over a belt or a depot, it fills the items that stop under it
//! with the fluid in its tank, which pipes fill. The timing is
//! `create_kinetics::rules::spout`; what an item takes and gives is `spout_fill`. The belt
//! and the depot hand it their items through `belt::register_processor`, as they do the press.
//!
//! It also fills the blocks Create gives a `BlockSpoutingBehaviour` (`rules/spout_blocks.rs`): dirt to mud,
//! farmland, a water cauldron and an empty cauldron. Create 6.0.10 has none for a basin or a campfire.
//!
//! Known differences from Create: see `rules/spout.rs` (the fill happens in the Spout's own tick),
//! `rules/spout_blocks.rs` (no Tinkers' Construct `SpoutCasting`, no splash) and `spout_fill.rs` (the
//! containers known). Also:
//! - Create's Spout fills items on a belt or a depot only (`BeltProcessingBehaviour` over a
//!   `TransportedItemStackHandlerBehaviour`), not items lying on the ground, and neither does this one.
//! - The fluid in the tank rises at once where Create's `LerpedFloat` chases it; there are no
//!   splash or stream particles (`FluidFX`, `sendSplash`), no comparator signal
//!   (`ComparatorUtil.levelOfSmartFluidTank`: the hook gets no block entity), no advancement and no waterlogging.

use std::collections::HashSet;
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

use create_kinetics::belt::Processing;
use create_kinetics::rules::spout::{self as rules, Held, Item, Spout};
use create_kinetics::rules::spout_blocks as blocks;
use crate::compat::{
    BlockEntityEvent, BlockEntityFluidsEvent, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent,
    BlockEntityTickEvent, BlockEntityTicksEvent, BlockEntityUpdateTagEvent, Compound, Hooks, Host,
    ItemStack, ModFluidTanks, StandaloneModelsEvent, Tag,
};

use super::fluids;
use super::spout_fill as fill;
use crate::belt::{self, BeltProcessor};
use crate::{depot, util};

type Pos = [i32; 3];

pub const ID: &str = "create:spout";
/// `SpoutBlockEntity.write`.
const TICKS: &str = "ProcessingTicks";
/// `AllPartialModels.SPOUT_TOP`, `SPOUT_MIDDLE`, `SPOUT_BOTTOM`: drawn in this order.
const BITS: [&str; 3] = [
    "create:block/spout/top",
    "create:block/spout/middle",
    "create:block/spout/bottom",
];

struct Spouts;

pub fn hooks() -> Box<dyn Hooks> {
    belt::register_processor(BeltProcessor {
        claims: |block| block == ID,
        received,
        held,
    });
    Box::new(Spouts)
}

/// What a saved Spout holds.
struct Saved {
    spout: Spout,
    fluid: Option<String>,
    amount: i32,
}

fn read(nbt: &Compound) -> Saved {
    let (fluid, amount) = fill::tank_of(nbt);
    let ticks = match util::get(nbt, TICKS) {
        Some(Tag::Int(n)) => *n,
        _ => -1,
    };
    Saved { spout: Spout { processing_ticks: ticks }, fluid, amount }
}

fn new_entity() -> Compound {
    let mut nbt = Compound::new();
    nbt.insert(fill::FLUIDS.into(), fill::tank_tag(None, 0));
    nbt.insert(TICKS.into(), Tag::Int(-1));
    nbt
}

/// The tank, in the container's shape: `SpoutBlockEntity.registerCapabilities` gives
/// the tank to every side but down, to fill and to drain.
fn tanks() -> ModFluidTanks {
    let mut faces: [Vec<usize>; 6] = [(); 6].map(|()| vec![0]);
    // `Direction.DOWN` is the first face.
    faces[0] = Vec::new();
    ModFluidTanks { tanks: vec![rules::CAPACITY], faces, inputs: vec![0], outputs: vec![0] }
}

/// The item the Spout looks at, as `FillingBySpout` sees it against the tank.
fn facts(host: &dyn Host, stack: &ItemStack, saved: &Saved) -> Item {
    Item {
        can_be_filled: fill::can_be_filled(host, stack),
        tank_empty: saved.fluid.is_none(),
        required: saved.fluid.as_deref().and_then(|f| fill::required_amount(host, stack, f)),
        amount: saved.amount,
    }
}

fn saved_of(host: &dyn Host, pos: Pos) -> Option<Saved> {
    let entity = host.mod_block_entity(pos).filter(|e| e.id == ID)?;
    Some(read(&entity.nbt))
}

// --- the belt's processor -----------------------------------------------------------------------

/// `SpoutBlockEntity.onItemReceived`.
fn received(host: &mut dyn Host, pos: Pos, stack: &ItemStack) -> Processing {
    let Some(saved) = saved_of(&*host, pos) else {
        return Processing::Pass;
    };
    rules::received(&facts(&*host, stack, &saved))
}

/// `SpoutBlockEntity.whenItemHeld`, up to the start of a fill (the fill is in [`fill_below`]).
fn held(host: &mut dyn Host, pos: Pos, stack: &ItemStack) -> Processing {
    let Some(mut saved) = saved_of(&*host, pos) else {
        return Processing::Pass;
    };
    let item = facts(&*host, stack, &saved);
    match saved.spout.held(&item) {
        Held::Pass => Processing::Pass,
        Held::Hold => Processing::Hold,
        Held::Start => {
            host.change_mod_block_entity(pos, &mut |entity| {
                entity.nbt.insert(TICKS.into(), Tag::Int(rules::FILLING_TIME));
            });
            host.block_entity_sync(pos);
            // `AllSoundEvents.SPOUTING.playOnServer(level, pos, 0.75f, 0.9f + 0.2f * random)`.
            let pitch = 0.9 + 0.2 * (util::random() % 1000) as f32 / 1000.0;
            util::create_sound_at(host, "spout", pos, 0.75, pitch);
            Processing::Hold
        }
    }
}

/// The rest of `whenItemHeld` at the tick `processingTicks` reaches 5: fills the item held in the
/// middle of the belt or the depot two blocks down. Answers the fluid left in the tank, or `None`
/// where nothing was filled (nothing is held, or `whenItemHeld` would pass or hold it).
fn fill_below(host: &mut dyn Host, pos: Pos, saved: &Saved) -> Option<i32> {
    let fluid = saved.fluid.clone()?;
    let target = [pos[0], pos[1] - 2, pos[2]];
    let name = host.block(target);
    let held = if name == belt::ID {
        belt::locked_stack(&*host, target)?
    } else if name == depot::ID {
        depot::locked_stack(&*host, target)?
    } else {
        return None;
    };
    let item = facts(&*host, &held, saved);
    let required = item.required?;
    if !item.can_be_filled || required > saved.amount {
        return None;
    }
    let mut results = |host: &mut dyn Host, probe: &ItemStack| -> Option<Vec<ItemStack>> {
        fill::fill_item(host, probe, &fluid, required).map(|out| vec![out])
    };
    let done = if name == belt::ID {
        belt::process_held(host, target, &mut results)
    } else {
        depot::process_held(host, target, &mut results)
    };
    done.then_some(saved.amount - required)
}

/// The Spouts whose fill is a block's (`customProcess`), by dimension and position: not saved, as Create's isn't.
static CUSTOM: LazyLock<Mutex<HashSet<(&'static str, Pos)>>> = LazyLock::new(Default::default);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `BlockSpoutingBehaviour.fillBlock(.., simulate = true)` for the block two below the Spout: what it would take and
/// become (`rules::spout_blocks`).
fn block_fill(host: &dyn Host, pos: Pos, saved: &Saved) -> Option<blocks::Fill> {
    let fluid = saved.fluid.as_deref()?;
    let target = [pos[0], pos[1] - 2, pos[2]];
    let text = crate::compat::state_text(host, host.level(host.dimension())?, target)?;
    blocks::fill(&text, fluid, saved.amount)
}

/// The same with `simulate = false`, at the tick the fill happens: the block changes. Answers the fluid left in the
/// tank, `None` when the behaviour no longer takes it.
fn fill_block(host: &mut dyn Host, pos: Pos, saved: &Saved) -> Option<i32> {
    let fill = block_fill(&*host, pos, saved)?;
    let target = [pos[0], pos[1] - 2, pos[2]];
    let line = format!(
        "execute in {} run setblock {} {} {} {} replace",
        host.dimension(),
        target[0],
        target[1],
        target[2],
        fill.state
    );
    host.run_command(&line).then_some(saved.amount - fill.amount)
}

// --- the hooks ----------------------------------------------------------------------------------

impl Hooks for Spouts {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.extend(BITS.map(str::to_owned));
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == ID).then(|| (ID.to_owned(), new_entity()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    fn block_entity_fluids(
        &self,
        _: &dyn Host,
        event: &mut BlockEntityFluidsEvent<'_>,
    ) -> Option<ModFluidTanks> {
        (event.block == ID).then(tanks)
    }

    /// `SpoutBlockEntity.tick`.
    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        let pos = event.pos;
        let mut saved = read(&event.entity.nbt);
        let before = saved.spout;
        let dimension = host.dimension();
        // An idle Spout with fluid asks the block two below for a behaviour (`BlockSpoutingBehaviour.get`): if it
        // would take some, the 20 ticks start and the behaviour is the one that fills.
        if saved.spout.processing_ticks == -1 && block_fill(&*host, pos, &saved).is_some() {
            saved.spout.processing_ticks = rules::FILLING_TIME;
            lock(&CUSTOM).insert((dimension, pos));
        }
        let filled = saved.spout.tick();
        let mut changed = false;
        if filled {
            let left = if lock(&CUSTOM).remove(&(dimension, pos)) {
                fill_block(host, pos, &saved)
            } else {
                fill_below(host, pos, &saved)
            };
            if let Some(left) = left {
                let fluid = saved.fluid.clone();
                fill::set_tank(&mut event.entity.nbt, fluid.as_deref(), left);
                changed = true;
            }
        }
        if saved.spout != before {
            event.entity.nbt.insert(TICKS.into(), Tag::Int(saved.spout.processing_ticks));
            changed = true;
        }
        if changed {
            host.block_entity_sync(pos);
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        if event.entity.id != ID {
            return None;
        }
        let mut tag = Compound::new();
        // Always sent, so a client that had a fluid hears it is gone.
        if let Some(fluids) = util::get(&event.entity.nbt, fill::FLUIDS) {
            tag.insert(fill::FLUIDS.into(), fluids.clone());
        }
        let ticks = read(&event.entity.nbt).spout.processing_ticks;
        tag.insert(TICKS.into(), Tag::Int(ticks));
        Some(tag)
    }

    /// `SpoutBlockEntity.createRenderBoundingBox`: the stream reaches two blocks down.
    fn block_entity_render_bounding_box(
        &self,
        _: &dyn Host,
        event: &mut BlockEntityRenderBoundingBoxEvent<'_>,
    ) {
        if event.block == ID {
            event.bounds[1] = event.bounds[1].min(-2.0);
        }
    }

    /// `SpoutRenderer`: the fluid in the body, the stream under the nozzle while filling, and the
    /// nozzle's three bits squeezed together.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let saved = read(&event.entity.nbt);
        let ticks = saved.spout.processing_ticks;
        let draw = &mut *event.draw;
        if let Some(fluid) = saved.fluid.as_deref() {
            // `level = Math.max(level, 0.175f)`; the box is `min..max` wide and `11/16 * level` tall.
            let level = (saved.amount as f32 / rules::CAPACITY as f32).max(0.175);
            let min = 2.5 / 16.0;
            let max = min + 11.0 / 16.0;
            let height = 11.0 / 16.0 * level;
            fluids::draw_fluid(host, draw, fluid, saved.amount, [min, min, min, max, min + height, max], false);
        }
        let (radius, squeeze) = rules::stroke(ticks, draw.partial);
        if let Some(fluid) = saved.fluid.as_deref()
            && ticks != -1
        {
            // `new AABB(0.5, 0.0, 0.5, 0.5, -1.2, 0.5).inflate(radius / 32f)`, corners sorted.
            let r = radius / 32.0;
            let (x0, x1) = (0.5 + r, 0.5 - r);
            let (y0, y1) = (-1.2 - r, r);
            let sorted = |a: f32, b: f32| (a.min(b), a.max(b));
            let (x0, x1) = sorted(x0, x1);
            let (y0, y1) = sorted(y0, y1);
            if x1 > x0 {
                fluids::draw_fluid(host, draw, fluid, saved.amount, [x0, y0, x0, x1, y1, x1], true);
            }
        }
        draw.push();
        for model in BITS {
            host.draw_model(draw, model, 0xFF_FFFF);
            draw.translate([0.0, -3.0 * squeeze / 32.0, 0.0]);
        }
        draw.pop();
        // At rest the picture depends on the saved data only; mid-fill it reads the partial tick.
        if ticks == -1 {
            draw.keep();
        }
    }
}
