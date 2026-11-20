//! The Mechanical Roller on a moving structure (`RollerMovementBehaviour`): it breaks what is above the layer
//! it rolls over like a Drill, takes the paving block from the structure's storage and puts it down, or fills the
//! ground below it. The engine hook is `actor.rs`, the shared block breaking is in `actor_tools.rs`; the rules
//! (which cells, which order, what the filter takes) are `create_kinetics::rules::roller`. This file reads the
//! world, asks the rules and applies the answer.
//!
//! Known differences from Create:
//! - A Roller on a Train (`createHeightProfileForTracks`) isn't ported, as there are no trains.
//! - The pave and fill states are placed with `setblock`, so a block with a block entity or a state the command
//!   can't take isn't paved; the filter refuses those anyway (`roller.rs`).
//! - As in Create, a Roller with no filter in its paving mode swaps a replaceable block (grass, snow) for air and
//!   uses up an item of the structure's storage (`tryFill` with air as the block to place and an empty filter
//!   that passes anything).

use create_kinetics::rules::actor as actor_rules;
use create_kinetics::rules::contraption::{self as contraption_rules, State};
use create_kinetics::rules::drill as drill_rules;
use create_kinetics::rules::actor::Kind;
use create_kinetics::rules::roller::{self as rules, Existing, Fill, Mode, Paving};
use create_kinetics::Pos;
use crate::compat::{Hooks, Host, ItemStack, RandomSource};

use super::actor::{Actor, Cx};
use super::actor_tools as tools;
use super::chute_inv::{self, Filter};
use super::contraption as engine;
use super::value_box::SCROLL_VALUE;
use crate::int;

struct Shared;

impl Hooks for Shared {}

/// A module of shared code: no hooks of its own.
pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

/// `WaitingTicks` after a block was paved.
const PAVE_WAIT: i32 = 2;
const AIR: &str = "minecraft:air";

// --- what the roller has to work with -------------------------------------------------------------------

/// `getStateToPaveWith` and `getStateToPaveWithAsSlab`, worked out from the filter item.
struct Material {
    /// The state placed (`None`: air, nothing to pave with). A slab is placed as a double slab.
    state: Option<State>,
    /// The state placed in half-height spots, where there is one.
    slab: Option<State>,
    falling: bool,
}

impl Material {
    fn block(&self) -> &str {
        self.state.as_ref().map_or(AIR, |state| state.name.as_str())
    }

    fn slab_block(&self) -> Option<&str> {
        self.slab.as_ref().map(|state| state.name.as_str())
    }

    fn place(&self) -> State {
        self.state.clone().unwrap_or_else(|| State::new(AIR))
    }
}

/// The block an item stands for, if it is a block item: its own id where the block of that name exists.
fn material_of(host: &dyn Host, filter: &Filter) -> Material {
    let none = Material { state: None, slab: None, falling: false };
    let Some(item) = &filter.item else { return none };
    let id = &*item.item;
    let Some(default) = host.default_state(id) else { return none };
    // `defaultBlockState.hasProperty(SlabBlock.TYPE)`: a double slab to pave with, a bottom one for the spots a
    // slab goes.
    if rules::is_slab_default(host.block_state_property(default, "type")) {
        return Material {
            state: Some(State::new(id).with("type", "double")),
            slab: Some(State::new(id).with("type", "bottom")),
            falling: false,
        };
    }
    let (namespace, name) = id.split_once(':').unwrap_or(("minecraft", id));
    let slab = rules::slab_candidates(name)
        .into_iter()
        .map(|candidate| format!("{namespace}:{candidate}"))
        .find(|candidate| host.default_state(candidate).is_some())
        .map(|candidate| State::new(&candidate));
    Material { state: Some(State::new(id)), slab, falling: actor_rules::is_falling(id) }
}

fn mode_of(actor: &Actor) -> Mode {
    Mode::from_ordinal(int(&actor.data, SCROLL_VALUE))
}

fn in_tag(host: &dyn Host, block: &str, tag: &str) -> bool {
    host.block_name_in_tag(block, tag).unwrap_or(false)
}

fn state_at(host: &dyn Host, pos: Pos) -> State {
    host.level(host.dimension())
        .and_then(|level| crate::compat::state_text(host, level, pos))
        .map_or_else(|| State::new(AIR), |text| State::parse(&text))
}

/// `random.nextBoolean()`.
fn coin(host: &mut dyn Host) -> bool {
    let dimension = host.dimension();
    host.level_random(dimension).map_or(0.5, |random| f64::from(random.next_float())) >= 0.5
}

// --- breaking -------------------------------------------------------------------------------------------

/// `getPositionsToBreak`: the cells above the visited one that the Roller clears first.
fn positions_to_break(cx: &mut Cx<'_>, actor: &Actor, visited: Pos) -> Vec<Pos> {
    let mode = mode_of(actor);
    if mode != Mode::TunnelPave {
        return Vec::new();
    }
    let filter = chute_inv::filter_of(&actor.data);
    let material = material_of(&*cx.host, &filter);
    // `ItemHelper.extract(storage, filter, 1, simulate)`: only asked when there is something to pave with.
    let stored = material.state.is_some() && {
        let host: &dyn Host = &*cx.host;
        let test = |stack: &ItemStack| filter.test(host, stack);
        cx.store.has(host, &test)
    };
    let start = rules::starting_y(material.state.is_some(), stored);
    let host: &dyn Host = &*cx.host;
    rules::positions_to_break(mode, start, visited, &mut |target, column_y| {
        // `testBreakerTarget`.
        let existing = host.block(target);
        let is_pave = existing == material.block();
        let is_slab = material.slab_block() == Some(existing.as_str());
        if rules::leaves_alone(column_y, is_pave, material.slab.is_some(), is_slab) {
            return false;
        }
        tools::can_break(host, Kind::Roller, target)
    })
}

/// `visitNewPosition`.
pub(super) fn visit(cx: &mut Cx<'_>, actor: &mut Actor, pos: Pos) {
    // `!stateVisited.isRedstoneConductor`: a full block has nobody inside it.
    let solid = cx.host.block_collision(pos).iter().any(|b| *b == [0.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
    if !solid {
        tools::damage_entities(cx, actor, pos);
    }
    let positions = positions_to_break(cx, actor, pos);
    if positions.is_empty() {
        trigger_paver(cx, actor, pos);
        return;
    }
    let hardness: Vec<(Pos, f32)> =
        positions.iter().map(|at| (*at, drill_rules::hardness(&cx.host.block(*at)))).collect();
    let Some(hardest) = rules::hardest(&hardness) else {
        trigger_paver(cx, actor, pos);
        return;
    };
    actor.breaker.reference = Some(pos);
    actor.breaker.start(hardest);
    actor.ctx.stall = true;
}

/// `destroyBlock`: the block goes with half of what it drops, none of it if it needs an iron, stone or diamond tool.
pub(super) fn destroy(cx: &mut Cx<'_>, actor: &Actor, pos: Pos) {
    let block = cx.host.block(pos);
    let needs_tool = ["minecraft:needs_iron_tool", "minecraft:needs_stone_tool", "minecraft:needs_diamond_tool"]
        .iter()
        .any(|tag| in_tag(&*cx.host, &block, tag));
    let Some(drops) = cx.host.destroy_block(pos, None, true) else { return };
    for stack in drops {
        let toss = coin(&mut *cx.host);
        if rules::keeps_drop(needs_tool, toss) {
            cx.collect_or_drop(&actor.ctx, stack);
        }
    }
}

/// `onBlockBroken`: what `BlockBreakingMovementBehaviour` does for a falling block, then the rest of the column
/// the Roller was to clear, and the paving of the layer it was visiting.
pub(super) fn on_broken(cx: &mut Cx<'_>, actor: &mut Actor, pos: Pos, broken: &str) {
    if actor_rules::is_falling(broken) {
        actor.breaker.wait(&mut actor.ctx, tools::FALLING_WAIT, pos);
    }
    let Some(reference) = actor.breaker.reference else { return };
    for other in positions_to_break(cx, actor, reference) {
        if other != pos {
            destroy(cx, actor, other);
        }
    }
    trigger_paver(cx, actor, reference);
    actor.breaker.reference = None;
}

// --- paving -------------------------------------------------------------------------------------------

/// `tryFill`: the block put at `target` if the cell can be built into and the structure holds an item the filter
/// passes.
fn try_fill(cx: &mut Cx<'_>, filter: &Filter, target: Pos, place: &State) -> Fill {
    let existing = {
        let host: &dyn Host = &*cx.host;
        let state = state_at(host, target);
        Existing {
            same_block: state.name == place.name,
            leaves: in_tag(host, &state.name, "minecraft:leaves"),
            replaceable: contraption_rules::replaceable(&state),
            collision_empty: host.block_collision(target).is_empty(),
            portal: in_tag(host, &state.name, "minecraft:portals"),
        }
    };
    if let Some(fill) = rules::judge(&existing) {
        return fill;
    }
    let taken = {
        let host: &dyn Host = &*cx.host;
        let test = |stack: &ItemStack| filter.test(host, stack);
        cx.store.extract(host, &test, 1, false)
    };
    if taken.is_none() {
        return Fill::Fail;
    }
    if engine::place_state(cx.host, target, place) { Fill::Success } else { Fill::Fail }
}

/// `triggerPaver`: paves or fills under the visited cell, and holds the structure for two ticks if a block went down.
fn trigger_paver(cx: &mut Cx<'_>, actor: &mut Actor, pos: Pos) {
    let mode = mode_of(actor);
    let filter = chute_inv::filter_of(&actor.data);
    let material = material_of(&*cx.host, &filter);
    if !rules::should_pave(mode, material.state.is_none()) {
        return;
    }
    let paving = Paving {
        mode,
        depth: rules::FILL_DEPTH,
        falling: material.falling,
        has_slab: material.slab.is_some(),
    };
    let main = material.place();
    let slab = material.slab.clone();
    let placed = rules::pave(&paving, &[(pos, false)], &mut |target, as_slab| {
        match (&slab, as_slab) {
            (Some(slab), true) => try_fill(cx, &filter, target, slab),
            _ => try_fill(cx, &filter, target, &main),
        }
    });
    if placed {
        actor.breaker.wait(&mut actor.ctx, PAVE_WAIT, pos);
    }
}
