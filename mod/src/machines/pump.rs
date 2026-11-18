//! The Mechanical Pump (`PumpBlock`, `PumpBlockEntity`, `PumpRenderer`): a small cogwheel turning
//! about its `facing` axis that pulls fluid in from the tank behind it and pushes it out to the
//! one in front, through the pipes between. Which pipes and tanks it reaches, and how much
//! moves each tick, are `create_kinetics::rules::pump`; this is the glue.
//!
//! It costs 4 su per RPM (`CStress.setImpact(4.0)`) and, being an `ICogWheel` with no shaft ends
//! of its own, turns only with a small cogwheel beside it (`Kind::encased_cog` with no shaft).
//! Its direction is its `facing`, not the rotation (`PumpBlockEntity.isPullingOnSide`): the
//! Wrench turns it round.
//!
//! Known differences from Create: see `rules/pump.rs`. Also, the pump's `facing` at placement
//! follows the player (`DirectionalKineticBlock.getStateForPlacement`) but not the pipe beside it
//! (`PumpBlock.getStateForPlacement` reads neighbours a placement hook can't), the network is
//! solved every second rather than on each neighbour's update, and the pump isn't waterlogged.
//! A closed Fluid Valve and a Smart Fluid Pipe whose filter refuses the fluid are no pipe to the
//! network (`fluids::classify_for`); with a filtered pipe in it, each source's fluid gets the
//! network worked out for itself, every second like the rest.

use std::collections::HashMap;
use std::sync::Mutex;

use create_kinetics::rules::pump::{
    self as rules, Cell, Endpoint, Side, distribute, solve_side, transfer_speed, warmup_ticks,
};
use create_kinetics::{Dir, Kind, Node, looking};
use crate::compat::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, BlockStateId, Compound, Flow, Hooks, Host, LevelHost,
    StandaloneModelsEvent, StateForPlacementEvent, UseItemOnEvent,
};

use super::fluids::{self, MECHANICAL_PUMP, direction};
use crate::{kinetic, util};

type Pos = [i32; 3];

/// `AllPartialModels.MECHANICAL_PUMP_COG`.
const COG: &str = "create:block/mechanical_pump/cog";
const CENTRE: [f32; 3] = [0.5; 3];
/// How often the network is looked at again, in ticks.
const SOLVE: i64 = 20;

struct Pump;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Pump)
}

/// What a running pump works from between ticks. Not saved: a pump solves again on its first tick.
struct State {
    facing: Dir,
    speed: f32,
    solved: i64,
    seen: i64,
    pull: Side,
    push: Side,
    /// Ticks before the fluid reaches (`FluidNetwork.pauseBeforePropagation` and the flow's progress).
    warmup: i32,
    /// Whether a Smart Fluid Pipe with a filter lies in the network: then which tanks the fluid of
    /// each source reaches is worked out for that fluid (`by_fluid`).
    filtered: bool,
    by_fluid: HashMap<String, (Side, Side)>,
}

static STATES: Mutex<Option<HashMap<(String, Pos), State>>> = Mutex::new(None);

fn with_states<R>(f: impl FnOnce(&mut HashMap<(String, Pos), State>) -> R) -> R {
    let mut guard = STATES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    f(guard.get_or_insert_with(HashMap::new))
}

fn node(facing: Dir) -> Node {
    Node {
        impact: rules::IMPACT,
        ..Node::passive(Kind::encased_cog(facing.axis(), false, false, false))
    }
}

/// `distributePressureTo` for both sides of the pump, for `fluid` passing (a Smart Fluid Pipe's
/// filter lets only its own through). Also whether the network holds a filtered pipe.
fn solve(host: &dyn Host, pos: Pos, facing: Dir, speed: f32, fluid: Option<&str>) -> (Side, Side, bool) {
    let mut memo: HashMap<Pos, Cell> = HashMap::new();
    let (pull, push) = {
        let mut cells = |p: Pos| *memo.entry(p).or_insert_with(|| fluids::classify_for(host, p, fluid));
        let pull = solve_side(&mut cells, pos, facing.opposite(), true, speed, rules::RANGE);
        let push = solve_side(&mut cells, pos, facing, false, speed, rules::RANGE);
        (pull, push)
    };
    let filtered = fluid.is_none() && memo.keys().any(|p| fluids::is_filtered(host, *p));
    (pull, push, filtered)
}

fn same_ends(a: &[Endpoint], b: &[Endpoint]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x == y)
}

/// `FluidNetwork.tick`'s transfer, for each source behind the pump: up to its `transferSpeed`
/// from the tank, shared among the tanks in front by how much room each has. With a filtered
/// pipe in the network each source's fluid sees the network its own way.
fn transfer(host: &mut dyn Host, pos: Pos, state: &mut State) {
    if state.push.endpoints.is_empty() {
        return;
    }
    let sources = state.pull.endpoints.clone();
    for source in &sources {
        let face = direction(source.face);
        let Some((fluid, have)) = fluids::contents(&*host, source.handler, face) else {
            continue;
        };
        let (pressure, ends) = if state.filtered {
            let (facing, speed) = (state.facing, state.speed);
            let entry = state.by_fluid.entry(fluid.clone()).or_insert_with(|| {
                let (pull, push, _) = solve(&*host, pos, facing, speed, Some(&fluid));
                (pull, push)
            });
            let (pull, push) = entry.clone();
            let Some(own) = pull.endpoints.iter().find(|e| e.handler == source.handler && e.face == source.face)
            else {
                continue;
            };
            (own.pressure, push.endpoints)
        } else {
            (source.pressure, state.push.endpoints.clone())
        };
        let want = transfer_speed(pressure).min(have);
        let targets: Vec<&Endpoint> = ends.iter().filter(|t| t.handler != source.handler).collect();
        let rooms: Vec<i32> = targets
            .iter()
            .map(|t| fluids::room(&*host, t.handler, direction(t.face), &fluid))
            .collect();
        let given = distribute(want, &rooms);
        let total: i32 = given.iter().sum();
        if total <= 0 {
            continue;
        }
        let dimension = host.dimension();
        let Some((taken, amount)) = host.fluid_extract(dimension, source.handler, face, total) else {
            continue;
        };
        let mut remaining = amount;
        for (target, share) in targets.iter().zip(&given) {
            let share = (*share).min(remaining);
            if share <= 0 {
                continue;
            }
            let rest = host.fluid_insert(target.handler, direction(target.face), &taken, share);
            remaining -= share - rest;
        }
        if remaining > 0 {
            // What no target took goes back where it came from.
            host.fluid_insert(source.handler, face, &taken, remaining);
        }
    }
}

fn flow(host: &mut dyn Host, pos: Pos, facing: Dir, rpm: f32) {
    let key = (host.dimension().to_owned(), pos);
    let now = util::game_time(host);
    let speed = rpm.abs();
    // `PumpFluidTransferBehaviour`: no pressure, no flow, at rest.
    if speed == 0.0 {
        with_states(|states| states.remove(&key));
        return;
    }
    let mut state = with_states(|states| states.remove(&key));
    let stale = state
        .as_ref()
        .is_none_or(|s| s.facing != facing || s.speed != speed || now - s.solved >= SOLVE);
    if stale {
        let (pull, push, filtered) = solve(&*host, pos, facing, speed, None);
        let unchanged = state.as_ref().is_some_and(|s| {
            s.facing == facing
                && s.speed == speed
                && same_ends(&s.pull.endpoints, &pull.endpoints)
                && same_ends(&s.push.endpoints, &push.endpoints)
        });
        let warmup = if unchanged {
            state.as_ref().map_or(0, |s| s.warmup)
        } else {
            warmup_ticks(&[&pull, &push])
        };
        state = Some(State {
            facing,
            speed,
            solved: now,
            seen: now,
            pull,
            push,
            warmup,
            filtered,
            by_fluid: HashMap::new(),
        });
    }
    let Some(mut state) = state else { return };
    state.seen = now;
    if state.warmup > 0 {
        state.warmup -= 1;
    } else {
        transfer(host, pos, &mut state);
    }
    with_states(|states| {
        states.insert(key, state);
        // A pump that stopped ticking was broken or unloaded.
        if now.rem_euclid(200) == 0 {
            states.retain(|_, s| now - s.seen < 400);
        }
    });
}

/// `KineticBlockEntityRenderer.getRotationOffsetForPosition`: a cog is turned 22.5 degrees where
/// the two other coordinates add up even, so neighbours mesh.
fn phase(pos: Pos, axis: create_kinetics::Axis) -> f32 {
    let sum: i32 = (0..3).filter(|i| *i != axis.index()).map(|i| pos[i]).sum();
    if sum.rem_euclid(2) == 0 { 22.5 } else { 0.0 }
}

impl Hooks for Pump {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.push(COG.to_owned());
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == MECHANICAL_PUMP).then(|| (MECHANICAL_PUMP.to_owned(), kinetic::new_entity()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == MECHANICAL_PUMP {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != MECHANICAL_PUMP {
            return;
        }
        let pos = event.pos;
        let Some(facing) = util::facing(&*host, pos, "facing") else {
            return;
        };
        let Some((rpm, changed)) = kinetic::tick(host, pos, event.entity, node(facing)) else {
            // The network jammed and the pump broke.
            let key = (host.dimension().to_owned(), pos);
            with_states(|states| states.remove(&key));
            return;
        };
        if changed {
            host.block_entity_sync(pos);
        }
        flow(host, pos, facing, rpm);
    }

    fn block_entity_update_tag(
        &self,
        event: &mut BlockEntityUpdateTagEvent<'_>,
    ) -> Option<Compound> {
        (event.entity.id == MECHANICAL_PUMP).then(|| {
            let mut tag = kinetic::update_tag(event.entity);
            // `PipeAttachmentModel`: the drain (`fluid_pipe_attachments.rs`).
            super::fluid_pipe_attachments::copy_rims(&mut tag, &event.entity.nbt);
            tag
        })
    }

    /// `PumpRenderer`: the casing is in the chunk mesh; the cog turns about the facing axis.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != MECHANICAL_PUMP {
            return;
        }
        let Some(facing) = host
            .block_state_property(event.state, "facing")
            .and_then(Dir::from_name)
        else {
            return;
        };
        let rpm = kinetic::speed(event.entity);
        let axis = facing.axis();
        let draw = &mut *event.draw;
        draw.spin(axis.unit(), CENTRE, rpm * 0.3, phase(event.pos, axis));
        draw.push();
        crate::draw::face_south(draw, facing);
        host.draw_model(draw, COG, 0xFF_FFFF);
        draw.pop();
        draw.still();
        draw.keep();
    }

    /// `DirectionalKineticBlock.getStateForPlacement`: facing away from the player, towards them
    /// when sneaking (Create first looks for a shaft beside it, which a placement hook can't see).
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        if &*event.context.item != MECHANICAL_PUMP {
            return None;
        }
        let look = looking(event.context.yaw, event.context.pitch);
        let facing = if event.context.sneaking { look } else { look.opposite() };
        level.state_with_property(event.state, "facing", facing.name()).map(Some)
    }

    /// The Wrench: sneak-click picks the pump up; a click reverses it (`PumpBlock.getRotatedBlockState`).
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != MECHANICAL_PUMP {
            return Flow::Continue;
        }
        let pos = event.pos;
        util::wrench(host, pos, |host| {
            if let Some(facing) = util::facing(&*host, pos, "facing") {
                host.set_block_property(pos, "facing", facing.opposite().name());
            }
        })
    }
}
