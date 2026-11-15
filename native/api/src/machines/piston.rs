//! The Mechanical Piston and the Sticky Mechanical Piston (`MechanicalPistonBlock`,
//! `MechanicalPistonBlockEntity`, `LinearActuatorBlockEntity`, `PistonContraption`,
//! `MechanicalPistonRenderer`) on the contraption engine (`contraption.rs`, where the design and
//! the differences common to every contraption are written; the rules are
//! `create_kinetics::rules::piston`).
//!
//! A piston with Piston Extension Poles behind it takes the poles, a head and the blocks in a line
//! in front of it (and what sticks to those: slime, honey, attached blocks) when it starts
//! turning or an empty hand clicks it, and moves them along its facing by RPM / 512 blocks a
//! tick up to the length of its poles (`getExtensionRange`), until something is in the way or the
//! rotation stops. A Sticky Piston pulls the block in front of its head back as well.
//!
//! Known differences from Create:
//! - Always `MovementMode.MOVE_PLACE`: the structure is set down where the piston stops; the
//!   "move, never place" and "place only when returned" modes are not offered.
//! - A piston that is stopped is set down once the rotation has been absent for half a second
//!   (`QUIET_TICKS`, so that a reload or a network being solved again doesn't drop it), not at
//!   once.
//! - The structure collects as `PistonContraption` does, but nothing rides it or is pushed by it
//!   (see `contraption.rs`).
//! - Breaking a pole or the head doesn't break the rest of the piston (`playerWillDestroy`);
//!   breaking the piston while it runs sets the structure down first.
//! - The assembly exception is shown on the action bar of the players near, not in the goggles.
//! - The piston's own shape while extended or moving is Create's (`AllShapes.MECHANICAL_PISTON*`);
//!   waterlogging is left out, as elsewhere.

use create_kinetics::rules::contraption::{self as rules, State, Transform, add, relative};
use create_kinetics::rules::piston as piston_rules;
use create_kinetics::{Dir, Kind, Node, Pos, looking};
use horizon_client::mod_api::{
    BlockEntityEvent, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent, BlockEntityTickEvent,
    BlockEntityTicksEvent, BlockEntityUpdateTagEvent, BlockStateId, Compound, DestroyBlockEvent, Flow,
    Hooks, Host, LevelHost, ModBlockEntity, RightClickBlockEvent, StateForPlacementEvent, Tag,
    UseItemOnEvent,
};

use super::contraption::{
    self as engine, ASSEMBLE, CONTRAPTION, HostWorld, LAST_EXCEPTION, RUNNING, SYNC_TIME,
};
use crate::{kinetic, util};

const NORMAL: &str = "create:mechanical_piston";
const STICKY: &str = "create:sticky_mechanical_piston";

// Saved as `LinearActuatorBlockEntity.write` and `MechanicalPistonBlockEntity.write` name them.
const OFFSET: &str = "Offset";
const EXTENSION_LENGTH: &str = "ExtensionLength";

struct Piston;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Piston)
}

fn is_piston(id: &str) -> bool {
    id == NORMAL || id == STICKY
}

fn flag(value: bool) -> Tag {
    Tag::Byte(i8::from(value))
}

/// The block entity's state as the tick reads it.
struct Saved {
    running: bool,
    offset: f32,
    time: i64,
    range: i32,
    assemble: bool,
}

fn saved(entity: &ModBlockEntity) -> Saved {
    Saved {
        running: util::boolean(&entity.nbt, RUNNING),
        offset: util::float(&entity.nbt, OFFSET).unwrap_or(0.0),
        time: util::long(&entity.nbt, SYNC_TIME),
        range: util::int(&entity.nbt, EXTENSION_LENGTH),
        assemble: util::boolean(&entity.nbt, ASSEMBLE),
    }
}

/// Writes `value` under `key` when it differs: a write counts as a change whatever it holds.
fn put(entity: &mut ModBlockEntity, key: &str, value: Tag) {
    if util::differs(&entity.nbt, key, &value) {
        entity.nbt.insert(key.into(), value);
    }
}

fn state_of(host: &dyn Host, pos: Pos) -> Option<(Dir, bool)> {
    let facing = util::facing(host, pos, "facing")?;
    let along_first = host.block_property(pos, "axis_along_first").as_deref() == Some("true");
    Some((facing, along_first))
}

/// The state of the piston at `pos` as the rules want it.
fn block_state(host: &dyn Host, pos: Pos) -> Option<State> {
    let level = host.level(host.dimension())?;
    level.block_state(host, pos).map(|text| State::parse(&text))
}

/// `LinearActuatorBlockEntity.assemble` as `MechanicalPistonBlockEntity.assemble` does it.
/// Returns whether the piston is running now.
fn assemble(
    host: &mut dyn Host,
    pos: Pos,
    entity: &mut ModBlockEntity,
    facing: Dir,
    rpm: f32,
    now: i64,
    manual: bool,
) -> bool {
    let dimension = host.dimension().to_owned();
    let Some(piston) = block_state(host, pos) else {
        return false;
    };
    let velocity = piston_rules::velocity(rpm, facing);
    let retract = velocity < 0.0;
    let checked = {
        let Some(world) = HostWorld::new(&*host) else {
            return false;
        };
        piston_rules::assemble(&world, pos, facing, retract, &piston).map(|(collected, found)| {
            // Whether the first step is already blocked: `isCollidingWithWorld`. The structure's
            // blocks are keyed by position relative to the anchor, shifted back by the
            // extension the piston already has (`PistonContraption.toLocalPos`).
            let local = |p: Pos| relative(rules::sub(p, collected.anchor), facing, -collected.initial);
            let mut states = std::collections::BTreeMap::new();
            for (p, state) in collected.poles.iter().chain(found.blocks.iter()) {
                states.insert(local(*p), state.clone());
            }
            let movement = piston_rules::movement_direction(rpm, facing.axis());
            let anchor = relative(collected.anchor, facing, collected.initial);
            let first = relative(anchor, movement, 1);
            let blocked = rules::colliders(&states, movement, &engine::solid)
                .iter()
                .any(|c| rules::blocked_by(&world, add(first, *c)));
            (collected, found, blocked)
        })
    };
    let (collected, found, first_blocked) = match checked {
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
    if first_blocked {
        return false;
    }
    // Check if not at the limit already.
    let resulting = collected.initial as f32 + velocity.signum() * 0.5;
    if resulting <= 0.0 || resulting >= collected.extension_length as f32 {
        return false;
    }
    let moving = engine::capture(host, collected.anchor, &found.blocks, &collected.poles, &|p| {
        // `PistonContraption.toLocalPos`: global, minus the anchor, shifted back by the
        // extension the piston already has.
        relative(
            [p[0] - collected.anchor[0], p[1] - collected.anchor[1], p[2] - collected.anchor[2]],
            facing,
            -collected.initial,
        )
    });
    // `removeBlocksFromWorld(level, startPos)`: the blocks are at `local + anchor + k * facing`.
    let origin = relative(collected.anchor, facing, collected.initial);
    engine::remove_from_world(host, &moving, &|local| add(local, origin), &mut |host, at, _| {
        // `customBlockRemoval`: the piston's own cell keeps the piston, in its moving state.
        if at == pos {
            host.set_block_property(pos, "state", "moving");
            return true;
        }
        false
    });
    engine::set_live(&dimension, pos, moving.clone());
    entity
        .nbt
        .insert(CONTRAPTION.into(), Tag::Compound(moving.to_nbt(false)));
    entity.nbt.insert(RUNNING.into(), flag(true));
    entity
        .nbt
        .insert(EXTENSION_LENGTH.into(), Tag::Int(collected.extension_length));
    util::set_float(&mut entity.nbt, OFFSET, collected.initial as f32);
    entity.nbt.insert(SYNC_TIME.into(), Tag::Long(now));
    engine::store_exception(entity, None);
    host.block_entity_sync(pos);
    true
}

/// Sets the structure of the piston at `pos` down with its anchor `offset` blocks out along the
/// facing, as `ControlledContraptionEntity.disassemble` does. Returns the offset it settled at.
fn set_down(host: &mut dyn Host, pos: Pos, nbt: &Compound, facing: Dir, offset: f32, range: i32) -> Option<i32> {
    let dimension = host.dimension().to_owned();
    let live = engine::live(&dimension, pos, nbt)?;
    let moving = &live.moving;
    let settled = piston_rules::grid_offset(offset, range);
    // `MechanicalPistonBlockEntity.disassemble`: the piston reads as extended first.
    host.set_block_property(pos, "state", "extended");
    let transform = Transform::translation(relative(moving.anchor, facing, settled));
    engine::place_in_world(host, moving, &transform, &mut |host, at, state| {
        // `customBlockPlacement`: what lands on the piston's cell is never placed; a head there
        // means the piston is back in.
        if at == pos {
            if !state.is(engine::POLE) {
                host.set_block_property(pos, "state", "retracted");
            }
            return true;
        }
        false
    });
    engine::forget_live(&dimension, pos);
    Some(settled)
}

fn stop(entity: &mut ModBlockEntity, settled: i32, now: i64) {
    entity.nbt.insert(RUNNING.into(), flag(false));
    entity.nbt.remove(&CONTRAPTION.into());
    util::set_float(&mut entity.nbt, OFFSET, settled as f32);
    entity.nbt.insert(SYNC_TIME.into(), Tag::Long(now));
    put(entity, ASSEMBLE, flag(false));
}

fn new_entity() -> Compound {
    let mut nbt = kinetic::new_entity();
    nbt.insert(RUNNING.into(), flag(false));
    nbt.insert(ASSEMBLE.into(), flag(false));
    nbt.insert(EXTENSION_LENGTH.into(), Tag::Int(0));
    nbt.insert(SYNC_TIME.into(), Tag::Long(0));
    nbt.insert(LAST_EXCEPTION.into(), Tag::String("".into()));
    util::set_float(&mut nbt, OFFSET, 0.0);
    nbt
}

impl Hooks for Piston {
    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        is_piston(event.block).then(|| (event.block.to_owned(), new_entity()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if is_piston(event.block) {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if !is_piston(&event.entity.id) {
            return;
        }
        let pos = event.pos;
        let Some((facing, along_first)) = state_of(host, pos) else {
            return;
        };
        let node = Node {
            impact: piston_rules::IMPACT,
            ..Node::passive(Kind::shafts(&piston_rules::shaft_faces(facing, along_first)))
        };
        let previous = kinetic::speed(event.entity);
        let Some((rpm, changed)) = kinetic::tick(host, pos, event.entity, node) else {
            return; // The network jammed and the block broke.
        };
        let now = util::game_time(host);
        let dimension = host.dimension().to_owned();
        let entity = &mut *event.entity;
        let mut state = saved(entity);
        let quiet = engine::quiet_ticks(&dimension, pos, rpm);
        let manual = util::int(&entity.nbt, ASSEMBLE) == i32::from(engine::ASKED);

        // `onSpeedChanged`: the structure follows the old speed up to now, then the new one.
        if changed {
            if state.running {
                let velocity = piston_rules::velocity(previous, facing);
                let offset = piston_rules::offset_at(state.offset, state.time, velocity, now as f64, state.range);
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

        // `assembleNextTick`.
        if state.assemble {
            if state.running {
                if rpm != 0.0 {
                    put(entity, ASSEMBLE, flag(false));
                    state.assemble = false;
                } else if quiet >= engine::QUIET_TICKS {
                    // The rotation is gone: set the structure down where it is.
                    let velocity = piston_rules::velocity(previous, facing);
                    let offset = piston_rules::offset_at(state.offset, state.time, velocity, now as f64, state.range);
                    if let Some(settled) = set_down(host, pos, &entity.nbt.clone(), facing, offset, state.range) {
                        stop(entity, settled, now);
                        host.block_entity_sync(pos);
                    }
                    return;
                }
            } else {
                put(entity, ASSEMBLE, flag(false));
                state.assemble = false;
                if rpm != 0.0 {
                    if assemble(host, pos, entity, facing, rpm, now, manual) {
                        state = saved(entity);
                    }
                }
            }
        }

        if !state.running {
            return;
        }
        let Some(live) = engine::live(&dimension, pos, &entity.nbt) else {
            // A structure that can't be read: nothing to move.
            return;
        };
        if rpm == 0.0 {
            return;
        }
        // Where the structure is this tick, and whether it has reached an end or something in
        // its way (`LinearActuatorBlockEntity.tick`: the structure is moved by this tick's step
        // first, then looked at).
        let velocity = piston_rules::velocity(rpm, facing);
        let offset = piston_rules::offset_at(state.offset, state.time, velocity, now as f64, state.range);
        let next = piston_rules::offset_at(state.offset, state.time, velocity, (now + 1) as f64, state.range);
        let at_end = (velocity > 0.0 && next >= state.range as f32) || (velocity < 0.0 && next <= 0.0);
        let mut settle_at = None;
        if velocity != 0.0 {
            let movement = if velocity > 0.0 { facing } else { facing.opposite() };
            let position = piston_rules::position(live.moving.anchor, facing, next);
            let cell = piston_rules::collision_anchor(position, movement);
            let colliders = live.colliders(movement);
            let blocked = HostWorld::new(&*host).is_some_and(|world| {
                colliders.iter().any(|c| rules::blocked_by(&world, add(*c, cell)))
            });
            if blocked {
                // `collided`: back to the grid cell the structure left.
                settle_at = Some(offset);
            }
        }
        if settle_at.is_none() && at_end {
            settle_at = Some(next);
        }
        if let Some(offset) = settle_at {
            if let Some(settled) = set_down(host, pos, &entity.nbt.clone(), facing, offset, state.range) {
                stop(entity, settled, now);
                host.block_entity_sync(pos);
            }
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        let entity = event.entity;
        if !is_piston(&entity.id) {
            return None;
        }
        let state = saved(entity);
        let mut tag = kinetic::update_tag(entity);
        tag.insert(RUNNING.into(), flag(state.running));
        util::set_float(&mut tag, OFFSET, state.offset);
        tag.insert(SYNC_TIME.into(), Tag::Long(state.time));
        tag.insert(EXTENSION_LENGTH.into(), Tag::Int(state.range));
        // Present even when empty: a client keeps what a missing key had.
        let line = match util::get(&entity.nbt, LAST_EXCEPTION) {
            Some(Tag::String(s)) => s.to_string_lossy(),
            _ => String::new(),
        };
        tag.insert(LAST_EXCEPTION.into(), Tag::String(line.as_str().into()));
        tag.insert(CONTRAPTION.into(), engine::client_contraption(entity));
        Some(tag)
    }

    /// `MechanicalPistonRenderer`: the shaft through the casing, and the structure.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if !is_piston(event.block) {
            return;
        }
        let Some(facing) = host
            .block_state_property(event.state, "facing")
            .and_then(Dir::from_name)
        else {
            return;
        };
        let along_first = host.block_state_property(event.state, "axis_along_first") == Some("true");
        let rpm = kinetic::speed(event.entity);
        let draw = &mut *event.draw;
        kinetic::draw_shaft(host, draw, piston_rules::rotation_axis(facing, along_first), rpm);
        let state = saved(event.entity);
        let parsed = engine::parsed(event.pos, event.entity);
        let Some(parsed) = parsed else {
            draw.keep();
            return;
        };
        let now = draw.game_time as f64 + f64::from(draw.partial);
        let offset =
            piston_rules::offset_at(state.offset, state.time, piston_rules::velocity(rpm, facing), now, state.range);
        // The anchor is the cell in front of the piston; the structure's blocks are at
        // `local + anchor + offset * facing`.
        let n = facing.normal();
        let anchor = parsed.moving.anchor;
        let base = [0, 1, 2].map(|i| (anchor[i] - event.pos[i]) as f32 + n[i] as f32 * offset);
        engine::draw_blocks(host, draw, &parsed, base);
    }

    /// Where the structure draws: from the piston along the facing for the whole extension.
    fn block_entity_render_bounding_box(
        &self,
        host: &dyn Host,
        event: &mut BlockEntityRenderBoundingBoxEvent<'_>,
    ) {
        if !is_piston(event.block) {
            return;
        }
        let Some(parsed) = engine::parsed(event.pos, event.entity) else {
            return;
        };
        let Some(facing) = host
            .block_state_property(event.state, "facing")
            .and_then(Dir::from_name)
        else {
            return;
        };
        let range = util::int(&event.entity.nbt, EXTENSION_LENGTH).max(0);
        let (e, n) = (parsed.extent, facing.normal());
        for i in 0..3 {
            let rel = parsed.moving.anchor[i] - event.pos[i];
            let travel = n[i] * range;
            let (lo, hi) = (rel + e[i] + travel.min(0), rel + e[i + 3] + 1 + travel.max(0));
            event.bounds[i] = event.bounds[i].min(lo as f32 - 1.0);
            event.bounds[i + 3] = event.bounds[i + 3].max(hi as f32 + 1.0);
        }
    }

    /// `DirectionalAxisKineticBlock.getStateForPlacement` without the shaft beside it.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if !is_piston(&context.item) {
            return None;
        }
        let look = looking(context.yaw, context.pitch);
        let horizontal = create_kinetics::rules::saw::horizontal_direction(context.yaw);
        let (facing, along_first) = piston_rules::placement(look, horizontal, context.sneaking);
        let state = level.state_with_property(event.state, "facing", facing.name())?;
        level
            .state_with_property(
                state,
                "axis_along_first",
                if along_first { "true" } else { "false" },
            )
            .map(Some)
    }

    /// The Wrench turns a retracted piston (`MechanicalPistonBlock.onWrenched`); a slime ball
    /// on the head's face makes it sticky (`useItemOn`).
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let block = host.block(event.pos);
        if !is_piston(&block) {
            return Flow::Continue;
        }
        let pos = event.pos;
        if &*event.stack.item == util::WRENCH {
            let clicked = Dir::from_normal(event.face);
            return util::wrench(host, pos, |host| {
                if host.block_property(pos, "state").as_deref() != Some("retracted") {
                    return;
                }
                let (Some((facing, along_first)), Some(clicked)) = (state_of(host, pos), clicked) else {
                    return;
                };
                let (next_facing, next_along) = piston_rules::wrench(facing, along_first, clicked);
                if next_facing != facing {
                    host.set_block_property(pos, "facing", next_facing.name());
                }
                if next_along != along_first {
                    host.set_block_property(pos, "axis_along_first", if next_along { "true" } else { "false" });
                }
            });
        }
        if &*event.stack.item != "minecraft:slime_ball" || block != NORMAL {
            return Flow::Continue;
        }
        let Some((facing, along_first)) = state_of(host, pos) else {
            return Flow::Continue;
        };
        if host.block_property(pos, "state").as_deref() != Some("retracted")
            || Dir::from_normal(event.face) != Some(facing)
        {
            return Flow::Continue;
        }
        if let Some(player) = host.acting_player() {
            if player.sneaking || player.game_mode == util::SPECTATOR {
                return Flow::Continue;
            }
            if player.game_mode != util::CREATIVE {
                host.take_items("minecraft:slime_ball", 1);
            }
        }
        let sticky = State::new(STICKY)
            .with("facing", facing.name())
            .with("axis_along_first", if along_first { "true" } else { "false" })
            .with("state", "retracted");
        engine::place_state(host, pos, &sticky);
        Flow::Cancel
    }

    /// `MechanicalPistonBlock.useItemOn`: an empty hand starts the assembly.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if !is_piston(&host.block(pos)) {
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
        host.change_mod_block_entity(pos, &mut |entity| {
            entity.nbt.insert(ASSEMBLE.into(), Tag::Byte(engine::ASKED));
        });
        Flow::Cancel
    }

    /// `LinearActuatorBlockEntity.remove`: a piston broken while it runs sets its structure down
    /// first.
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        let pos = event.pos;
        if !is_piston(&host.block(pos)) {
            return Flow::Continue;
        }
        let Some(entity) = host.mod_block_entity(pos) else {
            return Flow::Continue;
        };
        let state = saved(&entity);
        if !state.running {
            return Flow::Continue;
        }
        let Some((facing, _)) = state_of(host, pos) else {
            return Flow::Continue;
        };
        let rpm = kinetic::speed(&entity);
        let now = util::game_time(host);
        let velocity = piston_rules::velocity(rpm, facing);
        let offset = piston_rules::offset_at(state.offset, state.time, velocity, now as f64, state.range);
        set_down(host, pos, &entity.nbt, facing, offset, state.range);
        Flow::Continue
    }
}
