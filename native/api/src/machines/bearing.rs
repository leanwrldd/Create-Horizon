//! The Mechanical Bearing (`MechanicalBearingBlock`, `MechanicalBearingBlockEntity`,
//! `BearingContraption`, `BearingRenderer`) on the contraption engine (`contraption.rs`, where the
//! design and the differences common to every contraption are written; the rules are
//! `create_kinetics::rules::bearing`).
//!
//! The bearing takes the block in front of it and what sticks to that (slime, honey, attached
//! blocks, Super Glue and chassis once those exist) when it starts turning or an empty hand
//! clicks it, turns it about its own axis at the speed of the shaft out of its back, and sets it
//! down turned to the nearest right angle when the rotation stops, a wrench turns the bearing or
//! an empty hand clicks it again.
//!
//! Known differences from Create:
//! - Always `RotationMode.ROTATE_PLACE`: the structure is set down wherever the bearing stops;
//!   the other two modes of the scroll value are not offered.
//! - A stopped bearing waits half a second of no rotation (`QUIET_TICKS`) before it sets the
//!   structure down, so that a reload or a network being solved again doesn't drop it.
//! - A bearing inside a structure is carried as a rigid block, not as a stabilised
//!   sub-contraption (`StabilizedBearingMovementBehaviour`).
//! - The windmill bearing, sails, and the Clockwork Bearing are not here yet.
//! - The assembly exception is shown on the action bar of the players near and in the goggle
//!   overlay (`goggles.rs`, `IDisplayAssemblyExceptions`), with or without goggles.

use create_kinetics::rules::bearing as bearing_rules;
use create_kinetics::rules::contraption::{Transform, add};
use create_kinetics::rules::drill as drill_rules;
use create_kinetics::{Dir, Kind, Node, Pos, looking};
use horizon_client::mod_api::{
    BlockEntityEvent, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent, BlockEntityTickEvent,
    BlockEntityTicksEvent, BlockEntityUpdateTagEvent, BlockStateId, Compound, DestroyBlockEvent, Flow,
    Hooks, Host, LevelHost, ModBlockEntity, RightClickBlockEvent, StandaloneModelsEvent,
    StateForPlacementEvent, Tag, UseItemOnEvent,
};

use super::contraption::{
    self as engine, ASSEMBLE, CONTRAPTION, HostWorld, LAST_EXCEPTION, RUNNING, SYNC_TIME,
};
use crate::{kinetic, util};

const ID: &str = "create:mechanical_bearing";
/// `AllPartialModels.BEARING_TOP`.
const TOP: &str = "create:block/bearing/top";
const CENTRE: [f32; 3] = [0.5; 3];

// Saved as `MechanicalBearingBlockEntity.write` names them.
const ANGLE: &str = "Angle";
/// A click on a running bearing: set the structure down at the next tick.
const DISASSEMBLE: &str = "Disassemble";

struct Bearing;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Bearing)
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
    angle: f32,
    time: i64,
    assemble: bool,
    disassemble: bool,
}

fn saved(entity: &ModBlockEntity) -> Saved {
    Saved {
        running: util::boolean(&entity.nbt, RUNNING),
        angle: util::float(&entity.nbt, ANGLE).unwrap_or(0.0),
        time: util::long(&entity.nbt, SYNC_TIME),
        assemble: util::boolean(&entity.nbt, ASSEMBLE),
        disassemble: util::boolean(&entity.nbt, DISASSEMBLE),
    }
}

/// The structure's angle at `now` (game time, with the frame's partial on a client) from the
/// snapshot, turning at `rpm`.
fn angle_at(angle: f32, time: i64, rpm: f32, now: f64) -> f32 {
    let turned = f64::from(bearing_rules::convert_to_angular(rpm)) * (now - time as f64);
    ((f64::from(angle) + turned) % 360.0) as f32
}

fn new_entity() -> Compound {
    let mut nbt = kinetic::new_entity();
    nbt.insert(RUNNING.into(), flag(false));
    nbt.insert(ASSEMBLE.into(), flag(false));
    nbt.insert(DISASSEMBLE.into(), flag(false));
    nbt.insert(SYNC_TIME.into(), Tag::Long(0));
    nbt.insert(LAST_EXCEPTION.into(), Tag::String("".into()));
    util::set_float(&mut nbt, ANGLE, 0.0);
    nbt
}

/// `MechanicalBearingBlockEntity.assemble`. Returns whether the bearing runs now.
fn assemble(
    host: &mut dyn Host,
    pos: Pos,
    entity: &mut ModBlockEntity,
    facing: Dir,
    now: i64,
    manual: bool,
) -> bool {
    let dimension = host.dimension().to_owned();
    let assembled = {
        let Some(world) = HostWorld::new(&*host) else {
            return false;
        };
        bearing_rules::assemble(&world, pos, facing)
    };
    let (anchor, found) = match assembled {
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
    if found.blocks.is_empty() {
        return false;
    }
    let moving = engine::capture(host, anchor, &found.blocks, &[], &|p| {
        [p[0] - anchor[0], p[1] - anchor[1], p[2] - anchor[2]]
    });
    engine::remove_from_world(host, &moving, &|local| add(local, anchor), &mut |_, _, _| false);
    engine::set_live(&dimension, pos, moving.clone());
    entity
        .nbt
        .insert(CONTRAPTION.into(), Tag::Compound(moving.to_nbt(false)));
    entity.nbt.insert(RUNNING.into(), flag(true));
    util::set_float(&mut entity.nbt, ANGLE, 0.0);
    entity.nbt.insert(SYNC_TIME.into(), Tag::Long(now));
    engine::store_exception(entity, None);
    host.block_entity_sync(pos);
    true
}

/// Sets the structure of the bearing at `pos` down turned by `angle`, as
/// `ControlledContraptionEntity.disassemble` does.
fn set_down(host: &mut dyn Host, pos: Pos, nbt: &Compound, facing: Dir, angle: f32) {
    let dimension = host.dimension().to_owned();
    if let Some(live) = engine::live(&dimension, pos, nbt) {
        let transform: Transform = bearing_rules::disassembly(live.moving.anchor, facing, angle);
        engine::place_in_world(host, &live.moving, &transform, &mut |_, _, _| false);
    }
    engine::forget_live(&dimension, pos);
}

fn stop(entity: &mut ModBlockEntity, now: i64) {
    entity.nbt.insert(RUNNING.into(), flag(false));
    entity.nbt.remove(&CONTRAPTION.into());
    util::set_float(&mut entity.nbt, ANGLE, 0.0);
    entity.nbt.insert(SYNC_TIME.into(), Tag::Long(now));
    put(entity, ASSEMBLE, flag(false));
    put(entity, DISASSEMBLE, flag(false));
}

impl Hooks for Bearing {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.push(TOP.to_owned());
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

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        let pos = event.pos;
        let Some(facing) = util::facing(host, pos, "facing") else {
            return;
        };
        // `BearingBlock.hasShaftTowards`: the one shaft is out of the back.
        let node = Node {
            impact: bearing_rules::IMPACT,
            ..Node::passive(Kind::shafts(&[bearing_rules::shaft_face(facing)]))
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

        // `onSpeedChanged`: the structure turned at the old speed up to now, and turns at the new
        // one from here.
        if changed {
            if state.running {
                let angle = angle_at(state.angle, state.time, previous, now as f64);
                util::set_float(&mut entity.nbt, ANGLE, angle);
                entity.nbt.insert(SYNC_TIME.into(), Tag::Long(now));
                state.angle = angle;
                state.time = now;
            }
            if !state.assemble {
                entity.nbt.insert(ASSEMBLE.into(), flag(true));
            }
            state.assemble = true;
            host.block_entity_sync(pos);
        }

        // A click on a running bearing sets it down (`MechanicalBearingBlock.useItemOn`).
        if state.running && state.disassemble {
            let angle = angle_at(state.angle, state.time, rpm, now as f64);
            set_down(host, pos, &entity.nbt.clone(), facing, angle);
            stop(entity, now);
            host.block_entity_sync(pos);
            return;
        }

        // `assembleNextTick`.
        if state.assemble {
            if state.running {
                if rpm != 0.0 {
                    put(entity, ASSEMBLE, flag(false));
                } else if quiet >= engine::QUIET_TICKS {
                    let angle = angle_at(state.angle, state.time, 0.0, now as f64);
                    set_down(host, pos, &entity.nbt.clone(), facing, angle);
                    stop(entity, now);
                    host.block_entity_sync(pos);
                }
            } else {
                put(entity, ASSEMBLE, flag(false));
                if rpm != 0.0 {
                    assemble(host, pos, entity, facing, now, manual);
                }
            }
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        let entity = event.entity;
        if entity.id != ID {
            return None;
        }
        let state = saved(entity);
        let mut tag = kinetic::update_tag(entity);
        tag.insert(RUNNING.into(), flag(state.running));
        util::set_float(&mut tag, ANGLE, state.angle);
        tag.insert(SYNC_TIME.into(), Tag::Long(state.time));
        let line = match util::get(&entity.nbt, LAST_EXCEPTION) {
            Some(Tag::String(s)) => s.to_string_lossy(),
            _ => String::new(),
        };
        tag.insert(LAST_EXCEPTION.into(), Tag::String(line.as_str().into()));
        tag.insert(CONTRAPTION.into(), engine::client_contraption(entity));
        Some(tag)
    }

    /// `BearingRenderer`: half a shaft out of the back, the top, and the structure, the last two
    /// turning about the facing's axis.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let Some(facing) = host
            .block_state_property(event.state, "facing")
            .and_then(Dir::from_name)
        else {
            return;
        };
        let rpm = kinetic::speed(event.entity);
        let state = saved(event.entity);
        let draw = &mut *event.draw;
        kinetic::draw_shaft_half(host, draw, facing.opposite(), rpm);

        let axis = facing.axis().unit();
        let parsed = engine::parsed(event.pos, event.entity);
        // The turn is the game's: `degrees * game_time + phase`, with the snapshot's angle
        // worked back to game time 0 (in f64, then reduced: an f32 phase would lose it).
        let rate = f64::from(bearing_rules::convert_to_angular(rpm));
        let turning = parsed.is_some() && rate != 0.0;
        let phase = if parsed.is_some() {
            (f64::from(state.angle) - rate * state.time as f64).rem_euclid(360.0) as f32
        } else {
            0.0
        };
        let still_angle = if parsed.is_some() { state.angle } else { 0.0 };
        if turning {
            draw.spin(axis, CENTRE, rate as f32, phase);
        } else if still_angle != 0.0 {
            draw.rotate_about(axis, still_angle, CENTRE);
        }
        draw.push();
        let (up_axis, degrees) = bearing_rules::up_to(facing);
        if degrees != 0.0 {
            draw.rotate_about(up_axis.unit(), degrees, CENTRE);
        }
        host.draw_model(draw, TOP, 0xFF_FFFF);
        draw.pop();
        draw.still();

        if let Some(parsed) = parsed {
            let n = facing.normal();
            let pivot = [0, 1, 2].map(|i| n[i] as f32 + 0.5);
            if turning {
                draw.spin(axis, pivot, rate as f32, phase);
            } else if still_angle != 0.0 {
                draw.rotate_about(axis, still_angle, pivot);
            }
            let anchor = parsed.moving.anchor;
            let base = [0, 1, 2].map(|i| (anchor[i] - event.pos[i]) as f32);
            engine::draw_blocks(host, draw, &parsed, base);
            draw.still();
        }
        // Geometry that turns by itself through `spin` is kept: a new speed arrives as a new
        // update tag.
        draw.keep();
    }

    /// Where the structure turns: a cube about the anchor as wide as its farthest block.
    fn block_entity_render_bounding_box(
        &self,
        _: &dyn Host,
        event: &mut BlockEntityRenderBoundingBoxEvent<'_>,
    ) {
        if event.block != ID {
            return;
        }
        let Some(parsed) = engine::parsed(event.pos, event.entity) else {
            return;
        };
        let e = parsed.extent;
        let reach = (0..6).map(|i| e[i].abs()).max().unwrap_or(0) + 2;
        for i in 0..3 {
            let rel = parsed.moving.anchor[i] - event.pos[i];
            event.bounds[i] = event.bounds[i].min((rel - reach) as f32);
            event.bounds[i + 3] = event.bounds[i + 3].max((rel + reach) as f32);
        }
    }

    /// `DirectionalKineticBlock.getStateForPlacement` without a preferred shaft.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != ID {
            return None;
        }
        let look = looking(context.yaw, context.pitch);
        let facing = drill_rules::placement_facing(look, context.sneaking);
        level
            .state_with_property(event.state, "facing", facing.name())
            .map(Some)
    }

    /// The Wrench turns the bearing and sets its structure down (`BearingBlock.onWrenched`).
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != ID {
            return Flow::Continue;
        }
        let (pos, clicked) = (event.pos, Dir::from_normal(event.face));
        util::wrench(host, pos, |host| {
            let (Some(facing), Some(clicked)) = (util::facing(host, pos, "facing"), clicked) else {
                return;
            };
            // A running bearing is first set down (`onWrenched` disassembles): the click does
            // only that, the next one turns it.
            let mut running = false;
            host.change_mod_block_entity(pos, &mut |entity| {
                running = util::boolean(&entity.nbt, RUNNING);
                if running {
                    put(entity, DISASSEMBLE, flag(true));
                }
            });
            let next = drill_rules::wrench_facing(facing, clicked);
            if !running && next != facing {
                host.set_block_property(pos, "facing", next.name());
            }
        })
    }

    /// `MechanicalBearingBlock.useItemOn`: an empty hand starts or ends the assembly.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
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
        // Once a click: the off hand's turn would ask for the opposite (a bearing just set down
        // is assembled again).
        if !util::first_click(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        host.change_mod_block_entity(pos, &mut |entity| {
            if util::boolean(&entity.nbt, RUNNING) {
                put(entity, DISASSEMBLE, flag(true));
            } else {
                entity.nbt.insert(ASSEMBLE.into(), Tag::Byte(engine::ASKED));
            }
        });
        Flow::Cancel
    }

    /// `MechanicalBearingBlockEntity.remove`: a bearing broken while it runs sets its structure
    /// down first.
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(entity) = host.mod_block_entity(pos) else {
            return Flow::Continue;
        };
        let state = saved(&entity);
        if !state.running {
            return Flow::Continue;
        }
        let Some(facing) = util::facing(host, pos, "facing") else {
            return Flow::Continue;
        };
        let now = util::game_time(host);
        let angle = angle_at(state.angle, state.time, kinetic::speed(&entity), now as f64);
        set_down(host, pos, &entity.nbt, facing, angle);
        Flow::Continue
    }
}
