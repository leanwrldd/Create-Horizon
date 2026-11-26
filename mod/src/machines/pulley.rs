//! The Rope Pulley, its Rope and its Pulley Magnet (`PulleyBlock`, `PulleyBlockEntity`,
//! `LinearActuatorBlockEntity`, `PulleyContraption`, `PulleyRenderer`) on the contraption engine
//! (`contraption.rs`, where the design and the differences common to every contraption are
//! written; the rules, with the list of what differs from Create, are
//! `create_kinetics::rules::pulley`).
//!
//! A pulley with rope (and a magnet) under it takes the blocks that hang from the magnet when it
//! starts turning or an empty hand clicks it, pays the rope out (turning one way) or in (the other)
//! at RPM / 512 blocks a tick, drawn as it goes, and sets the structure down and the rope back as
//! blocks when it stops, reaches its length or meets something. With nothing hanging from the
//! magnet it lets the rope down on its own and picks up what it comes to.
//!
//! Actors on the structure (drills, saws, harvesters, ploughs, deployers, funnels: `actor.rs`) run
//! on it, and so do the Portable Interfaces: each tick the controller gives them the structure's
//! `Pose` (it slides up and down from the anchor, `toPosition`) and holds the rope still while one
//! stalls it (`Stalled`, `psi_motion`), as `piston.rs` does. A rope that turns round or stops
//! stops its actors (`onSpeedChanged`), and setting the structure down stops them first. The
//! Roller sleeps on a pulley (`PulleyContraption`). The drills' heads turn with the rope's speed.

use create_kinetics::rules::contraption::{self as rules, Transform, add};
use create_kinetics::rules::drill::hardness;
use create_kinetics::rules::pulley as pulley_rules;
use create_kinetics::rules::piston as piston_rules;
use create_kinetics::{Axis, Dir, Kind, Node, Pos};
use crate::compat::{
    BlockEntityEvent, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent, BlockEntityTickEvent,
    BlockEntityTicksEvent, BlockEntityUpdateTagEvent, BlockStateId, Compound, DestroyBlockEvent, Flow,
    Hooks, Host, LevelHost, ModBlockEntity, RightClickBlockEvent, StandaloneModelsEvent,
    StateForPlacementEvent, Tag, UseItemOnEvent,
};

use super::actor;
use super::actor_draw;
use super::contraption::{
    self as engine, ASSEMBLE, CONTRAPTION, HostWorld, LAST_EXCEPTION, RUNNING, SYNC_TIME,
};
use super::portable_interface_motion as psi_motion;
use create_kinetics::rules::actor::{Carrier, Structure, speed_reversed};
use crate::{kinetic, util};

const PULLEY: &str = pulley_rules::PULLEY;
const ROPE: &str = pulley_rules::ROPE;
const MAGNET: &str = pulley_rules::MAGNET;
/// `AllPartialModels.ROPE_COIL`, `ROPE_HALF`, `ROPE_HALF_MAGNET`.
const COIL: &str = "create:block/rope_pulley/rope_coil";
const ROPE_HALF: &str = "create:block/rope_pulley/rope_half";
const MAGNET_HALF: &str = "create:block/rope_pulley/rope_half_magnet";

// Saved as `LinearActuatorBlockEntity.write` and `PulleyBlockEntity.write` name them.
const OFFSET: &str = "Offset";
const INITIAL_OFFSET: &str = "InitialOffset";
const NEEDS_CONTRAPTION: &str = "NeedsContraption";
/// A click of the Wrench on a running pulley: set the structure down at the next tick.
const DISASSEMBLE: &str = "Disassemble";

struct Pulley;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Pulley)
}

fn flag(value: bool) -> Tag {
    Tag::Byte(i8::from(value))
}

fn put(entity: &mut ModBlockEntity, key: &str, value: Tag) {
    if util::differs(&entity.nbt, key, &value) {
        entity.nbt.insert(key.into(), value);
    }
}

struct Saved {
    running: bool,
    offset: f32,
    time: i64,
    initial: i32,
    assemble: bool,
    needs: bool,
}

fn saved(entity: &ModBlockEntity) -> Saved {
    Saved {
        running: util::boolean(&entity.nbt, RUNNING),
        offset: util::float(&entity.nbt, OFFSET).unwrap_or(0.0),
        time: util::long(&entity.nbt, SYNC_TIME),
        initial: util::int(&entity.nbt, INITIAL_OFFSET),
        assemble: util::boolean(&entity.nbt, ASSEMBLE),
        needs: util::boolean(&entity.nbt, NEEDS_CONTRAPTION),
    }
}

fn new_entity() -> Compound {
    let mut nbt = kinetic::new_entity();
    nbt.insert(RUNNING.into(), flag(false));
    nbt.insert(ASSEMBLE.into(), flag(false));
    nbt.insert(NEEDS_CONTRAPTION.into(), flag(true));
    nbt.insert(INITIAL_OFFSET.into(), Tag::Int(0));
    nbt.insert(SYNC_TIME.into(), Tag::Long(0));
    nbt.insert(LAST_EXCEPTION.into(), Tag::String("".into()));
    util::set_float(&mut nbt, OFFSET, 0.0);
    nbt
}

fn axis_of(host: &dyn Host, pos: Pos) -> Option<Axis> {
    host.block_property(pos, "axis").as_deref().and_then(Axis::from_name)
}

fn range_at(host: &dyn Host, pos: Pos) -> i32 {
    pulley_rules::extension_range(pos[1], pulley_rules::min_build_height(host.dimension()))
}

/// Where the structure is at game time `t` and a tick before (`solid`): `toPosition`, the anchor
/// `initialOffset - offset` up, the offset following its snapshot and speed.
pub(crate) fn solid_sample(
    host: &dyn Host,
    pos: Pos,
    entity: &ModBlockEntity,
    anchor: Pos,
    t: f64,
) -> Option<crate::solid::Sample> {
    if entity.id != PULLEY {
        return None;
    }
    let state = saved(entity);
    if !state.running {
        return None;
    }
    let spin = psi_motion::moving_rpm(entity, kinetic::speed(entity));
    let range = range_at(host, pos);
    let at = |t: f64| {
        let offset = pulley_rules::offset_at(state.offset, state.time, spin, t, range);
        [f64::from(anchor[0]), f64::from(anchor[1]) + f64::from(pulley_rules::rise(state.initial, offset)), f64::from(anchor[2])]
    };
    Some(crate::solid::Sample::Slide { now: at(t), prev: at(t - 1.0) })
}

fn below(pos: Pos, i: i32) -> Pos {
    [pos[0], pos[1] - i, pos[2]]
}

/// `PulleyBlockEntity.removeRopes`: the rope blocks and the magnet leave the world.
fn remove_ropes(host: &mut dyn Host, pos: Pos, length: i32) {
    let dimension = host.dimension();
    let Some(air) = host.default_state("minecraft:air") else { return };
    for i in 1..=length {
        let at = below(pos, i);
        let block = host.block(at);
        if block == ROPE || block == MAGNET {
            host.set_block(dimension, at, air, util::UPDATE_ALL);
        }
    }
}

/// Puts the rope and the magnet back (`PulleyBlockEntity.disassemble`): the magnet `length` below
/// the pulley, rope above it. What stands there breaks, unless it can't.
fn make_ropes(host: &mut dyn Host, pos: Pos, length: i32) {
    for i in (1..=length).rev() {
        let at = below(pos, i);
        let existing = host.block(at);
        if existing == ROPE || existing == MAGNET {
            continue;
        }
        if hardness(&existing) < 0.0 {
            continue;
        }
        if !rules::State::new(&existing).is_air() {
            host.destroy_block(at, None, false);
        }
        let state = rules::State::new(if i == length { MAGNET } else { ROPE });
        engine::place_state(host, at, &state);
    }
}

/// `PulleyBlockEntity.assemble`. Returns whether the pulley runs now.
fn assemble(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity, rpm: f32, now: i64, manual: bool) -> bool {
    let dimension = host.dimension().to_owned();
    let range = range_at(&*host, pos);
    let length = {
        let Some(world) = HostWorld::new(&*host) else { return false };
        use create_kinetics::rules::contraption::World;
        pulley_rules::rope_length(&|p| world.state(p), pos)
    };
    if !pulley_rules::can_start(length, range, rpm) {
        return false;
    }
    let anchor = pulley_rules::anchor(pos, length);
    engine::prepare(host);
    let checked = {
        let Some(world) = HostWorld::new(&*host) else { return false };
        pulley_rules::assemble(&world, anchor, length).map(|found| {
            let mut states = std::collections::BTreeMap::new();
            for (p, state) in &found.blocks {
                states.insert(rules::sub(*p, anchor), state.clone());
            }
            let movement = if rpm > 0.0 { Dir::Down } else { Dir::Up };
            let first = create_kinetics::rules::contraption::relative(anchor, movement, 1);
            let blocked = rules::colliders(&states, movement, &engine::solid).iter().any(|c| {
                let cell = add(first, *c);
                // `isCollidingWithWorld`: the magnet above the anchor block is let through when
                // the structure moves up (a rope that takes the structure it set down back up).
                let magnet = movement == Dir::Up
                    && *c == [0, 0, 0]
                    && {
                        use create_kinetics::rules::contraption::World;
                        world.state(cell).is_some_and(|s| s.is(MAGNET))
                    };
                !magnet && rules::blocked_by(&world, cell)
            });
            (found, blocked)
        })
    };
    let (found, blocked) = match checked {
        Ok(done) => done,
        Err(fail) => {
            let changed = engine::store_exception(entity, Some(&fail));
            if manual {
                engine::tell_near(host, pos, &fail);
            }
            if changed {
                host.block_entity_sync(pos);
            }
            return false;
        }
    };
    // `!canAssembleStructure && getSpeed() > 0`: a rope can't be let down onto what is below.
    if blocked && rpm > 0.0 {
        return false;
    }
    remove_ropes(host, pos, length);
    let has_structure = !found.blocks.is_empty() && !blocked;
    if has_structure {
        let moving = engine::capture(host, anchor, &found.blocks, &[], &|p| rules::sub(p, anchor));
        engine::remove_from_world(host, &moving, &|local| add(local, anchor), &mut |_, _, _| false);
        engine::set_live(&dimension, pos, moving.clone());
        entity.nbt.insert(CONTRAPTION.into(), Tag::Compound(moving.to_nbt(false)));
    }
    entity.nbt.insert(NEEDS_CONTRAPTION.into(), flag(has_structure));
    entity.nbt.insert(RUNNING.into(), flag(true));
    entity.nbt.insert(INITIAL_OFFSET.into(), Tag::Int(length));
    util::set_float(&mut entity.nbt, OFFSET, length as f32);
    entity.nbt.insert(SYNC_TIME.into(), Tag::Long(now));
    engine::store_exception(entity, None);
    host.block_entity_sync(pos);
    true
}

/// `PulleyBlockEntity.disassemble`: the structure down where the rope is, the rope back as blocks.
fn set_down(host: &mut dyn Host, pos: Pos, nbt: &Compound, offset: f32, initial: i32) -> i32 {
    let dimension = host.dimension().to_owned();
    // `Contraption.stop`: the actors on the structure stop first.
    actor::stop(host, pos, nbt);
    // `AbstractContraptionEntity.disassemble`.
    util::create_sound_at(host, "contraption_disassemble", pos, 1.0, 1.0);
    let range = range_at(&*host, pos);
    let grid = pulley_rules::grid_offset(offset, range);
    make_ropes(host, pos, grid);
    if let Some(live) = engine::live(&dimension, pos, nbt) {
        let anchor = live.moving.anchor;
        let transform = Transform::translation([anchor[0], anchor[1] + initial - grid, anchor[2]]);
        engine::place_in_world(host, &live.moving, &transform, &mut |_, _, _| false);
    }
    engine::forget_live(&dimension, pos);
    grid
}

fn stop(entity: &mut ModBlockEntity, grid: i32, now: i64) {
    entity.nbt.insert(RUNNING.into(), flag(false));
    entity.nbt.remove(&psi_motion::STALLED.into());
    entity.nbt.remove(&CONTRAPTION.into());
    util::set_float(&mut entity.nbt, OFFSET, grid as f32);
    entity.nbt.insert(INITIAL_OFFSET.into(), Tag::Int(0));
    entity.nbt.insert(SYNC_TIME.into(), Tag::Long(now));
    put(entity, ASSEMBLE, flag(false));
    put(entity, DISASSEMBLE, flag(false));
}

impl Hooks for Pulley {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        for model in [COIL, ROPE_HALF, MAGNET_HALF] {
            event.models.push(model.to_owned());
        }
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == PULLEY).then(|| (PULLEY.to_owned(), new_entity()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == PULLEY {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != PULLEY {
            return;
        }
        let pos = event.pos;
        let Some(axis) = axis_of(&*host, pos) else {
            return;
        };
        let node = Node {
            impact: pulley_rules::IMPACT,
            ..Node::passive(Kind::shafts(&axis_ends(axis)))
        };
        let previous = kinetic::speed(event.entity);
        let Some((rpm, changed)) = kinetic::tick(host, pos, event.entity, node) else {
            return; // The network jammed and the block broke.
        };
        let now = util::game_time(&*host);
        let dimension = host.dimension().to_owned();
        let range = range_at(&*host, pos);
        let entity = &mut *event.entity;
        let mut state = saved(entity);
        let quiet = engine::quiet_ticks(&dimension, pos, rpm);
        let manual = util::int(&entity.nbt, ASSEMBLE) == i32::from(engine::ASKED);

        // `onSpeedChanged`: the rope follows the old speed up to now, then the new one.
        if changed {
            if state.running {
                // `LinearActuatorBlockEntity.onSpeedChanged`: a rope that turns round or stops
                // stops its actors (`Contraption.stop`).
                if speed_reversed(previous, rpm) {
                    actor::restart(host, pos, &entity.nbt);
                }
                let moved = psi_motion::moving_rpm(entity, previous);
                let offset = pulley_rules::offset_at(state.offset, state.time, moved, now as f64, range);
                util::set_float(&mut entity.nbt, OFFSET, offset);
                entity.nbt.insert(SYNC_TIME.into(), Tag::Long(now));
                state.offset = offset;
                state.time = now;
            }
            if !state.assemble {
                entity.nbt.insert(ASSEMBLE.into(), flag(true));
            }
            state.assemble = true;
            host.block_entity_sync(pos);
        }

        // A click of the Wrench on a running pulley.
        if state.running && util::boolean(&entity.nbt, DISASSEMBLE) {
            let moved = psi_motion::moving_rpm(entity, rpm);
            let offset = pulley_rules::offset_at(state.offset, state.time, moved, now as f64, range);
            let grid = set_down(host, pos, &entity.nbt.clone(), offset, state.initial);
            stop(entity, grid, now);
            host.block_entity_sync(pos);
            return;
        }

        // `assembleNextTick`.
        if state.assemble {
            if state.running {
                if rpm != 0.0 {
                    put(entity, ASSEMBLE, flag(false));
                    state.assemble = false;
                } else if quiet >= engine::QUIET_TICKS {
                    let moved = psi_motion::moving_rpm(entity, previous);
                    let offset = pulley_rules::offset_at(state.offset, state.time, moved, now as f64, range);
                    let grid = set_down(host, pos, &entity.nbt.clone(), offset, state.initial);
                    stop(entity, grid, now);
                    host.block_entity_sync(pos);
                    return;
                }
            } else {
                put(entity, ASSEMBLE, flag(false));
                state.assemble = false;
                if rpm != 0.0 && assemble(host, pos, entity, rpm, now, manual) {
                    state = saved(entity);
                }
            }
        }

        if !state.running || rpm == 0.0 {
            return;
        }
        // `LinearActuatorBlockEntity.tick`.
        let live = engine::live(&dimension, pos, &entity.nbt);
        if state.needs && live.is_none() {
            return; // A structure that can't be read: nothing to move.
        }
        // `PortableStorageInterfaceMovement` and `tickActors`: the interfaces and the actors
        // (drills, saws...) on the structure may hold it still (`Contraption.stalled`: the rope
        // doesn't pay out, `moveAndCollideContraption` returns early). The snapshot is moved to
        // now when that changes, and a held rope follows no speed.
        if let Some(live) = &live {
            let held = psi_motion::is_stalled(&entity.nbt);
            let spin = psi_motion::moving_rpm(entity, rpm);
            let (from, at, initial) = (state.offset, state.time, state.initial);
            let anchor = live.moving.anchor;
            // `toPosition`: the anchor, `initialOffset - offset` up.
            let origin = |t: f64| {
                let out = pulley_rules::offset_at(from, at, spin, t, range);
                [
                    f64::from(anchor[0]),
                    f64::from(anchor[1]) + f64::from(pulley_rules::rise(initial, out)),
                    f64::from(anchor[2]),
                ]
            };
            let pose = psi_motion::Pose::Translating { origin: &origin };
            let acted = actor::tick(host, pos, entity, &pose, now, Carrier::Controlled, Structure::Pulley);
            let want = psi_motion::tick(host, pos, &live.moving, &pose, now) | acted;
            if want != held {
                let at_now = pulley_rules::offset_at(from, at, spin, now as f64, range);
                util::set_float(&mut entity.nbt, OFFSET, at_now);
                entity.nbt.insert(SYNC_TIME.into(), Tag::Long(now));
                psi_motion::set_stalled(entity, want);
                state.offset = at_now;
                state.time = now;
                host.block_entity_sync(pos);
            }
        }
        let moved = psi_motion::moving_rpm(entity, rpm);
        let speed = pulley_rules::speed(moved);
        let offset = pulley_rules::offset_at(state.offset, state.time, moved, now as f64, range);
        let next = pulley_rules::offset_at(state.offset, state.time, moved, (now + 1) as f64, range);
        let mut settle_at = None;
        if let Some(live) = &live {
            if speed != 0.0 {
                let movement = if speed > 0.0 { Dir::Down } else { Dir::Up };
                let anchor = live.moving.anchor;
                let position = [
                    f64::from(anchor[0]),
                    f64::from(anchor[1]) + f64::from(pulley_rules::rise(state.initial, next)),
                    f64::from(anchor[2]),
                ];
                let cell = piston_rules::collision_anchor(position, movement);
                let colliders = live.colliders(movement);
                let blocked = HostWorld::new(&*host)
                    .is_some_and(|world| colliders.iter().any(|c| rules::blocked_by(&world, add(*c, cell))));
                if blocked {
                    settle_at = Some(offset);
                }
            }
        } else if speed > 0.0 && (next as i32) != (offset as i32) {
            // `visitNewPosition`: the magnet is about to meet something to pick up.
            let at = below(pos, next as i32 + 1);
            let pick = HostWorld::new(&*host).is_some_and(|world| {
                use create_kinetics::rules::contraption::World;
                world.state(at).is_some_and(|s| pulley_rules::picks_up(&world, &s, at))
            });
            if pick {
                let grid = set_down(host, pos, &entity.nbt.clone(), offset, state.initial);
                stop(entity, grid, now);
                entity.nbt.insert(ASSEMBLE.into(), flag(true));
                host.block_entity_sync(pos);
                return;
            }
        }
        if settle_at.is_none() && ((speed > 0.0 && next >= range as f32) || (speed < 0.0 && next <= 0.0)) {
            settle_at = Some(next);
        }
        if let Some(offset) = settle_at {
            let grid = set_down(host, pos, &entity.nbt.clone(), offset, state.initial);
            stop(entity, grid, now);
            host.block_entity_sync(pos);
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        let entity = event.entity;
        if entity.id != PULLEY {
            return None;
        }
        let state = saved(entity);
        let mut tag = kinetic::update_tag(entity);
        tag.insert(RUNNING.into(), flag(state.running));
        util::set_float(&mut tag, OFFSET, state.offset);
        tag.insert(SYNC_TIME.into(), Tag::Long(state.time));
        tag.insert(INITIAL_OFFSET.into(), Tag::Int(state.initial));
        let line = match util::get(&entity.nbt, LAST_EXCEPTION) {
            Some(Tag::String(s)) => s.to_string_lossy(),
            _ => String::new(),
        };
        tag.insert(LAST_EXCEPTION.into(), Tag::String(line.as_str().into()));
        tag.insert(CONTRAPTION.into(), engine::client_contraption(entity));
        tag.insert(psi_motion::STALLED.into(), psi_motion::tag_value(&entity.nbt));
        Some(tag)
    }

    /// `PulleyRenderer`: the shaft, the coil, the magnet and the rope below it, and the structure.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != PULLEY {
            return;
        }
        let Some(axis) = host.block_state_property(event.state, "axis").and_then(Axis::from_name) else {
            return;
        };
        let rpm = kinetic::speed(event.entity);
        let draw = &mut *event.draw;
        kinetic::draw_shaft(host, draw, axis, rpm, event.pos);
        draw.push();
        if axis == Axis::X {
            draw.rotate_about([0.0, 1.0, 0.0], 90.0, [0.5; 3]);
        }
        host.draw_model(draw, COIL, 0xFF_FFFF);
        draw.pop();
        let state = saved(event.entity);
        let range = pulley_rules::extension_range(event.pos[1], pulley_rules::min_build_height(host.dimension()));
        let now = draw.game_time as f64 + f64::from(draw.partial);
        // A held rope (an actor or an interface stalls the structure) follows no speed.
        let moved = psi_motion::moving_rpm(event.entity, rpm);
        // A Ponder scene's `movePulley` sets the offset; a virtual pulley counts as running
        // (`PulleyRenderer.isPulleyRunning`: `be.isVirtual()`).
        let scripted = util::float(&event.entity.nbt, crate::ponder::ANIMATED);
        let offset = if let Some(offset) = scripted {
            offset
        } else if state.running {
            pulley_rules::offset_at(state.offset, state.time, moved, now, range)
        } else {
            0.0
        };
        let plan = pulley_rules::plan(offset, state.running || scripted.is_some());
        if let Some((y, half)) = plan.magnet {
            draw.push();
            draw.translate([0.0, -y, 0.0]);
            if half {
                host.draw_model(draw, MAGNET_HALF, 0xFF_FFFF);
            } else {
                host.draw_block(draw, MAGNET, &[]);
            }
            draw.pop();
        }
        if let Some(y) = plan.half_rope {
            draw.push();
            draw.translate([0.0, -y, 0.0]);
            host.draw_model(draw, ROPE_HALF, 0xFF_FFFF);
            draw.pop();
        }
        for y in &plan.ropes {
            draw.push();
            draw.translate([0.0, -y, 0.0]);
            host.draw_block(draw, ROPE, &[]);
            draw.pop();
        }
        let Some(parsed) = engine::parsed(event.pos, event.entity) else {
            if !state.running && scripted.is_none() {
                draw.keep();
            }
            return;
        };
        let anchor = parsed.moving.anchor;
        let base = [
            (anchor[0] - event.pos[0]) as f32,
            (anchor[1] - event.pos[1]) as f32 + pulley_rules::rise(state.initial, offset),
            (anchor[2] - event.pos[2]) as f32,
        ];
        // How it moves, for the actors' heads.
        actor_draw::set_motion(if psi_motion::is_stalled(&event.entity.nbt) {
            actor_draw::Motion::Stalled
        } else {
            actor_draw::Motion::Slide(pulley_rules::speed(moved).abs())
        });
        engine::draw_blocks(host, draw, &parsed, base);
    }

    /// Where the rope and the structure draw: below the pulley, as far as the rope goes.
    fn block_entity_render_bounding_box(&self, _: &dyn Host, event: &mut BlockEntityRenderBoundingBoxEvent<'_>) {
        if event.block != PULLEY {
            return;
        }
        let state = saved(event.entity);
        if !state.running {
            return;
        }
        let reach = state.initial.max(state.offset.ceil() as i32) as f32 + 3.0;
        let extent = engine::parsed(event.pos, event.entity).map_or(0, |p| {
            let e = p.extent;
            (0..6).map(|i| e[i].abs()).max().unwrap_or(0)
        }) as f32;
        event.bounds[1] = event.bounds[1].min(-reach - extent - 1.0);
        event.bounds[4] = event.bounds[4].max(extent + 3.0);
        for i in [0, 2] {
            event.bounds[i] = event.bounds[i].min(-extent - 1.0);
            event.bounds[i + 3] = event.bounds[i + 3].max(extent + 2.0);
        }
    }

    /// `HorizontalAxisKineticBlock.getStateForPlacement` without the shaft beside it.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != PULLEY {
            return None;
        }
        let horizontal = create_kinetics::rules::saw::horizontal_direction(context.yaw);
        let axis = pulley_rules::placement_axis(horizontal);
        level.state_with_property(event.state, "axis", axis.name()).map(Some)
    }

    /// The Wrench turns the pulley across (`IWrenchable`): a running one is set down first.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != PULLEY {
            return Flow::Continue;
        }
        let pos = event.pos;
        util::wrench(host, pos, |host| {
            let mut running = false;
            host.change_mod_block_entity(pos, &mut |entity| {
                running = util::boolean(&entity.nbt, RUNNING);
                if running {
                    put(entity, DISASSEMBLE, flag(true));
                }
            });
            if running {
                return;
            }
            let Some(axis) = axis_of(host, pos) else { return };
            let next = if axis == Axis::X { Axis::Z } else { Axis::X };
            host.set_block_property(pos, "axis", next.name());
        })
    }

    /// `PulleyBlock.useItemOn`: an empty hand starts the assembly.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != PULLEY {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.sneaking || player.game_mode == util::SPECTATOR || player.game_mode == 2 {
            return Flow::Continue;
        }
        if host.held_stack(player.uuid).is_some_and(|stack| stack.count > 0) {
            return Flow::Continue;
        }
        if !util::first_click(&*host, player.uuid, pos) {
            return Flow::Cancel;
        }
        host.change_mod_block_entity(pos, &mut |entity| {
            entity.nbt.insert(ASSEMBLE.into(), Tag::Byte(engine::ASKED));
        });
        Flow::Cancel
    }

    /// `PulleyBlock.onRemove` and `RopeBlockBase.onRemove`: a pulley broken while it runs sets
    /// its structure down first and takes the rope below with it; a rope or magnet broken takes
    /// the rest of its rope, and the pulley forgets its length.
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        let pos = event.pos;
        let block = host.block(pos);
        if block == PULLEY {
            if let Some(entity) = host.mod_block_entity(pos) {
                let state = saved(&entity);
                if state.running {
                    let now = util::game_time(&*host);
                    let range = range_at(&*host, pos);
                    let rpm = psi_motion::moving_rpm(&entity, kinetic::speed(&entity));
                    let offset = pulley_rules::offset_at(state.offset, state.time, rpm, now as f64, range);
                    set_down(host, pos, &entity.nbt, offset, state.initial);
                }
            }
            break_rope(host, pos, Dir::Down);
            return Flow::Continue;
        }
        if block == ROPE || block == MAGNET {
            break_rope(host, pos, Dir::Up);
            break_rope(host, pos, Dir::Down);
            // The pulley at the top of the rope no longer has one.
            let mut top = pos;
            while matches!(host.block(relative_up(top)).as_str(), ROPE | MAGNET) {
                top = relative_up(top);
            }
            let pulley = relative_up(top);
            if host.block(pulley) == PULLEY {
                host.change_mod_block_entity(pulley, &mut |entity| {
                    util::set_float(&mut entity.nbt, OFFSET, 0.0);
                    entity.nbt.insert(INITIAL_OFFSET.into(), Tag::Int(0));
                });
                host.block_entity_sync(pulley);
            }
        }
        Flow::Continue
    }
}

fn relative_up(pos: Pos) -> Pos {
    [pos[0], pos[1] + 1, pos[2]]
}

fn axis_ends(axis: Axis) -> [Dir; 2] {
    match axis {
        Axis::X => [Dir::West, Dir::East],
        Axis::Y => [Dir::Down, Dir::Up],
        Axis::Z => [Dir::North, Dir::South],
    }
}

static BREAKING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Breaks the rope and magnet cells from the one next to `pos` on in `direction`, as
/// `destroyBlock` of each (they chain in Create): a flag keeps the breaks from running the hook
/// again for each.
fn break_rope(host: &mut dyn Host, pos: Pos, direction: Dir) {
    if BREAKING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let step = direction.normal();
    let mut current = add(pos, step);
    for _ in 0..pulley_rules::MAX_ROPE_LENGTH {
        if !matches!(host.block(current).as_str(), ROPE | MAGNET) {
            break;
        }
        host.destroy_block(current, None, true);
        current = add(current, step);
    }
    BREAKING.store(false, std::sync::atomic::Ordering::SeqCst);
}
