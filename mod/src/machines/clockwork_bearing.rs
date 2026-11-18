//! The Clockwork Bearing (`ClockworkBearingBlock`, `ClockworkBearingBlockEntity`,
//! `ClockworkContraption`, `BearingRenderer`) on the contraption engine (`contraption.rs`): two
//! structures, the hour hand from the block in front of it and the minute hand from the first cell
//! in line beyond the hour hand's line, turned about the bearing's axis so they show the day time.
//! The rules are `create_kinetics::rules::clockwork`.
//!
//! The angles follow the day time at most as fast as the bearing turns (`getHourArmSpeed`), so
//! they are worked out tick by tick from a snapshot the controller saves (`HourAngle`,
//! `MinuteAngle` at game time `SyncTime`, with the day time's offset to the game time) and re-saves
//! every five seconds and when the speed changes; the server and every client run the same
//! `Clock::angles_at` and agree without a packet a tick. The second structure is the controller's
//! `Minute` compound, cached under a position of its own (`minute_key`).
//!
//! Known differences from Create:
//! - The hands are never held still (`isStalled`): actors on a hand (drills, harvesters) aren't run.
//! - The scroll value (`ClockHands`) is a sneaking click with an empty hand that steps the mode and
//!   tells it on the action bar; it is saved as `ScrollValue`.
//! - The day time on a client is the game time plus the offset the last snapshot carried: with the
//!   daylight cycle off, or after `/time set`, the dial drifts until the next snapshot (at most 5 s).
//! - A dimension is natural when it is the Overworld (`dimensionType().natural()`); the Nether and the
//!   End run their dial 24 times faster.
//! - A structure whose controller is broken or wrenched sets both hands down unturned
//!   (`hourAngle = 0; minuteAngle = 0; applyRotations()`).
//! - Hands are drawn as their block models only; the block's top turns with the hour hand.

use create_kinetics::rules::bearing as bearing_rules;
use create_kinetics::rules::clockwork::{self as rules, Clock, Hands};
use create_kinetics::rules::contraption::{Transform, add};
use create_kinetics::rules::drill as drill_rules;
use create_kinetics::{Dir, Kind, Node, Pos, looking};
use crate::compat::TextExt;
use crate::compat::{
    BlockEntityEvent, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent, BlockEntityTickEvent,
    BlockEntityTicksEvent, BlockEntityUpdateTagEvent, BlockStateId, Compound, DestroyBlockEvent, Flow, Hooks,
    Host, LevelHost, ModBlockEntity, RightClickBlockEvent, StandaloneModelsEvent, StateForPlacementEvent, Tag,
    Text, UseItemOnEvent,
};

use super::contraption::{
    self as engine, ASSEMBLE, CONTRAPTION, HostWorld, LAST_EXCEPTION, Moving, RUNNING, SYNC_TIME,
};
use crate::{kinetic, util};

const ID: &str = rules::ID;
/// `AllPartialModels.BEARING_TOP`.
const TOP: &str = "create:block/bearing/top";
const CENTRE: [f32; 3] = [0.5; 3];

// Saved as `ClockworkBearingBlockEntity.write` names them, and what this port keeps besides.
const HOUR_ANGLE: &str = "HourAngle";
const MINUTE_ANGLE: &str = "MinuteAngle";
const MINUTE: &str = "Minute";
const OFFSET: &str = "Offset";
const MODE: &str = "ScrollValue";
const DAY_OFFSET: &str = "DayOffset";
const NATURAL: &str = "Natural";
const DISASSEMBLE: &str = "Disassemble";
/// How often the snapshot is renewed while the hands run, in ticks.
const RESYNC: i64 = 100;

struct Clockwork;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Clockwork)
}

fn flag(value: bool) -> Tag {
    Tag::Byte(i8::from(value))
}

fn put(entity: &mut ModBlockEntity, key: &str, value: Tag) {
    if util::differs(&entity.nbt, key, &value) {
        entity.nbt.insert(key.into(), value);
    }
}

/// The cache key of the minute hand: a position no block has.
fn minute_key(pos: Pos) -> Pos {
    [pos[0], pos[1] + 1_000_000, pos[2]]
}

fn clock_of(nbt: &Compound, facing: Dir, rpm: f32) -> Clock {
    Clock {
        rpm,
        hands: Hands::from_value(util::int(nbt, MODE)),
        step: facing.sign(),
        natural: util::boolean(nbt, NATURAL),
        day_offset: util::long(nbt, DAY_OFFSET),
    }
}

fn angles_of(nbt: &Compound) -> (f32, f32) {
    (util::float(nbt, HOUR_ANGLE).unwrap_or(0.0), util::float(nbt, MINUTE_ANGLE).unwrap_or(0.0))
}

fn store_snapshot(entity: &mut ModBlockEntity, angles: (f32, f32), now: i64) {
    util::set_float(&mut entity.nbt, HOUR_ANGLE, angles.0);
    util::set_float(&mut entity.nbt, MINUTE_ANGLE, angles.1);
    entity.nbt.insert(SYNC_TIME.into(), Tag::Long(now));
}

fn new_entity() -> Compound {
    let mut nbt = kinetic::new_entity();
    nbt.insert(RUNNING.into(), flag(false));
    nbt.insert(ASSEMBLE.into(), flag(false));
    nbt.insert(DISASSEMBLE.into(), flag(false));
    nbt.insert(SYNC_TIME.into(), Tag::Long(0));
    nbt.insert(MODE.into(), Tag::Int(0));
    nbt.insert(OFFSET.into(), Tag::Int(0));
    nbt.insert(LAST_EXCEPTION.into(), Tag::String("".into()));
    util::set_float(&mut nbt, HOUR_ANGLE, 0.0);
    util::set_float(&mut nbt, MINUTE_ANGLE, 0.0);
    nbt
}

/// `ClockworkBearingBlockEntity.assemble`. Returns whether the clock runs now.
fn assemble(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity, facing: Dir, now: i64, manual: bool) -> bool {
    let dimension = host.dimension().to_owned();
    engine::prepare(host);
    let arms = {
        let Some(world) = HostWorld::new(&*host) else { return false };
        rules::assemble(&world, pos, facing)
    };
    let arms = match arms {
        Ok(arms) => arms,
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
    if arms.hour.found.blocks.is_empty() {
        return false;
    }
    // Both hands are read before either leaves the world.
    let hour_anchor = arms.hour.anchor;
    let hour = engine::capture(host, hour_anchor, &arms.hour.found.blocks, &[], &|p| {
        [p[0] - hour_anchor[0], p[1] - hour_anchor[1], p[2] - hour_anchor[2]]
    });
    let minute = arms.minute.as_ref().map(|(arm, offset)| {
        let anchor = arm.anchor;
        let moving = engine::capture(host, anchor, &arm.found.blocks, &[], &|p| {
            [p[0] - anchor[0], p[1] - anchor[1], p[2] - anchor[2]]
        });
        (moving, *offset)
    });
    engine::remove_from_world(host, &hour, &|local| add(local, hour_anchor), &mut |_, _, _| false);
    engine::set_live(&dimension, pos, hour.clone());
    entity.nbt.insert(CONTRAPTION.into(), Tag::Compound(hour.to_nbt(false)));
    if let Some((moving, offset)) = &minute {
        let anchor = moving.anchor;
        engine::remove_from_world(host, moving, &|local| add(local, anchor), &mut |_, _, _| false);
        engine::set_live(&dimension, minute_key(pos), moving.clone());
        entity.nbt.insert(MINUTE.into(), Tag::Compound(moving.to_nbt(false)));
        entity.nbt.insert(OFFSET.into(), Tag::Int(*offset));
    } else {
        util::remove(&mut entity.nbt, MINUTE);
        entity.nbt.insert(OFFSET.into(), Tag::Int(0));
    }
    entity.nbt.insert(RUNNING.into(), flag(true));
    entity.nbt.insert(NATURAL.into(), flag(dimension == "minecraft:overworld"));
    entity.nbt.insert(DAY_OFFSET.into(), Tag::Long(host.day_time() - now));
    store_snapshot(entity, (0.0, 0.0), now);
    engine::store_exception(entity, None);
    host.block_entity_sync(pos);
    true
}

/// Sets both hands of the bearing at `pos` down, unturned (`disassemble` zeroes the angles first).
fn set_down(host: &mut dyn Host, pos: Pos, nbt: &Compound) {
    let dimension = host.dimension().to_owned();
    if let Some(live) = engine::live(&dimension, pos, nbt) {
        let transform = Transform::translation(live.moving.anchor);
        engine::place_in_world(host, &live.moving, &transform, &mut |_, _, _| false);
    }
    engine::forget_live(&dimension, pos);
    if let Some(live) = engine::live_with(&dimension, minute_key(pos), nbt, MINUTE) {
        let transform = Transform::translation(live.moving.anchor);
        engine::place_in_world(host, &live.moving, &transform, &mut |_, _, _| false);
    }
    engine::forget_live(&dimension, minute_key(pos));
}

fn stop(entity: &mut ModBlockEntity, now: i64) {
    entity.nbt.insert(RUNNING.into(), flag(false));
    entity.nbt.remove(&CONTRAPTION.into());
    entity.nbt.remove(&MINUTE.into());
    store_snapshot(entity, (0.0, 0.0), now);
    put(entity, ASSEMBLE, flag(false));
    put(entity, DISASSEMBLE, flag(false));
}

fn node(facing: Dir) -> Node {
    // `BearingBlock.hasShaftTowards`: the one shaft is out of the back.
    Node { impact: rules::IMPACT, ..Node::passive(Kind::shafts(&[bearing_rules::shaft_face(facing)])) }
}

/// The update tag's part for the minute hand (see `engine::client_contraption`).
fn client_minute(entity: &ModBlockEntity) -> Tag {
    match util::get(&entity.nbt, MINUTE) {
        Some(Tag::Compound(saved)) if util::boolean(&entity.nbt, RUNNING) => {
            Moving::from_nbt(saved).map_or_else(|| Tag::Compound(Compound::new()), |m| Tag::Compound(m.to_nbt(true)))
        }
        _ => Tag::Compound(Compound::new()),
    }
}

impl Hooks for Clockwork {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.push(TOP.to_owned());
    }

    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
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
        let Some(facing) = util::facing(host, pos, "facing") else { return };
        let previous = kinetic::speed(event.entity);
        let Some((rpm, changed)) = kinetic::tick(host, pos, event.entity, node(facing)) else {
            return; // The network jammed and the block broke.
        };
        let now = util::game_time(&*host);
        let dimension = host.dimension().to_owned();
        let entity = &mut *event.entity;
        let running = util::boolean(&entity.nbt, RUNNING);
        let quiet = engine::quiet_ticks(&dimension, pos, rpm);
        let manual = util::int(&entity.nbt, ASSEMBLE) == i32::from(engine::ASKED);

        // `onSpeedChanged`: the hands moved at the old speed up to now, and follow the new one from here.
        if changed {
            if running {
                let before = clock_of(&entity.nbt, facing, previous).angles_at(
                    angles_of(&entity.nbt),
                    util::long(&entity.nbt, SYNC_TIME),
                    now,
                );
                store_snapshot(entity, before, now);
            }
            if !util::boolean(&entity.nbt, ASSEMBLE) {
                entity.nbt.insert(ASSEMBLE.into(), flag(true));
            }
            host.block_entity_sync(pos);
        }

        // A click on a running bearing, or the Wrench, sets the hands down.
        if running && util::boolean(&entity.nbt, DISASSEMBLE) {
            set_down(host, pos, &entity.nbt.clone());
            stop(entity, now);
            host.block_entity_sync(pos);
            return;
        }

        // `assembleNextTick`.
        if util::boolean(&entity.nbt, ASSEMBLE) {
            if running {
                if rpm != 0.0 {
                    put(entity, ASSEMBLE, flag(false));
                } else if quiet >= engine::QUIET_TICKS {
                    set_down(host, pos, &entity.nbt.clone());
                    stop(entity, now);
                    host.block_entity_sync(pos);
                    return;
                }
            } else {
                put(entity, ASSEMBLE, flag(false));
                if rpm != 0.0 || manual {
                    assemble(host, pos, entity, facing, now, manual);
                }
            }
        }

        // The snapshot is renewed now and then, so a client works only a short way forward.
        if util::boolean(&entity.nbt, RUNNING) && now.rem_euclid(RESYNC) == 0 {
            let angles = clock_of(&entity.nbt, facing, rpm).angles_at(
                angles_of(&entity.nbt),
                util::long(&entity.nbt, SYNC_TIME),
                now,
            );
            store_snapshot(entity, angles, now);
            entity.nbt.insert(DAY_OFFSET.into(), Tag::Long(host.day_time() - now));
            host.block_entity_sync(pos);
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        let entity = event.entity;
        if entity.id != ID {
            return None;
        }
        let mut tag = kinetic::update_tag(entity);
        tag.insert(RUNNING.into(), flag(util::boolean(&entity.nbt, RUNNING)));
        let (hour, minute) = angles_of(&entity.nbt);
        util::set_float(&mut tag, HOUR_ANGLE, hour);
        util::set_float(&mut tag, MINUTE_ANGLE, minute);
        tag.insert(SYNC_TIME.into(), Tag::Long(util::long(&entity.nbt, SYNC_TIME)));
        tag.insert(DAY_OFFSET.into(), Tag::Long(util::long(&entity.nbt, DAY_OFFSET)));
        tag.insert(NATURAL.into(), flag(util::boolean(&entity.nbt, NATURAL)));
        tag.insert(MODE.into(), Tag::Int(util::int(&entity.nbt, MODE)));
        tag.insert(OFFSET.into(), Tag::Int(util::int(&entity.nbt, OFFSET)));
        let line = match util::get(&entity.nbt, LAST_EXCEPTION) {
            Some(Tag::String(s)) => s.to_string_lossy(),
            _ => String::new(),
        };
        tag.insert(LAST_EXCEPTION.into(), Tag::String(line.as_str().into()));
        tag.insert(CONTRAPTION.into(), engine::client_contraption(entity));
        tag.insert(MINUTE.into(), client_minute(entity));
        Some(tag)
    }

    /// `BearingRenderer`: half a shaft out of the back, the top turning with the hour hand, and the
    /// two hands, each about the axis through its own anchor.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let Some(facing) = host.block_state_property(event.state, "facing").and_then(Dir::from_name) else {
            return;
        };
        let rpm = kinetic::speed(event.entity);
        let draw = &mut *event.draw;
        kinetic::draw_shaft_half(host, draw, facing.opposite(), rpm);

        let running = util::boolean(&event.entity.nbt, RUNNING);
        let clock = clock_of(&event.entity.nbt, facing, rpm);
        let snapshot_time = util::long(&event.entity.nbt, SYNC_TIME);
        let frame = draw.game_time as f64 + f64::from(draw.partial);
        let whole = frame.floor() as i64;
        let here = if running { clock.angles_at(angles_of(&event.entity.nbt), snapshot_time, whole) } else { (0.0, 0.0) };
        // `Mth.lerp(partialTicks, hourAngle, hourAngle + getHourArmSpeed())`.
        let partial = (frame - whole as f64) as f32;
        let ahead = if running { clock.step_tick(here, whole + 1) } else { here };
        let lerp = |a: f32, b: f32| a + (b - a) * partial;
        let hour = lerp(here.0, here.0 + (ahead.0 - here.0 + 540.0).rem_euclid(360.0) - 180.0);
        let minute = lerp(here.1, here.1 + (ahead.1 - here.1 + 540.0).rem_euclid(360.0) - 180.0);

        let axis = facing.axis().unit();
        draw.push();
        if hour != 0.0 {
            draw.rotate_about(axis, hour, CENTRE);
        }
        draw.push();
        let (up_axis, degrees) = bearing_rules::up_to(facing);
        if degrees != 0.0 {
            draw.rotate_about(up_axis.unit(), degrees, CENTRE);
        }
        host.draw_model(draw, TOP, 0xFF_FFFF);
        draw.pop();
        draw.pop();

        let pos = event.pos;
        if let Some(parsed) = engine::parsed(pos, event.entity) {
            let anchor = parsed.moving.anchor;
            let n = facing.normal();
            let pivot = [0, 1, 2].map(|i| n[i] as f32 + 0.5);
            draw.push();
            draw.rotate_about(axis, hour, pivot);
            let base = [0, 1, 2].map(|i| (anchor[i] - pos[i]) as f32);
            engine::draw_blocks(host, draw, &parsed, base);
            draw.pop();
        }
        if let Some(parsed) = engine::parsed_with(minute_key(pos), event.entity, MINUTE) {
            let anchor = parsed.moving.anchor;
            let pivot = [0, 1, 2].map(|i| (anchor[i] - pos[i]) as f32 + 0.5);
            draw.push();
            draw.rotate_about(axis, minute, pivot);
            let base = [0, 1, 2].map(|i| (anchor[i] - pos[i]) as f32);
            engine::draw_blocks(host, draw, &parsed, base);
            draw.pop();
        }
        // The angles change every frame: nothing is kept.
    }

    /// Where the hands turn: cubes about both anchors as wide as their farthest block.
    fn block_entity_render_bounding_box(&self, _: &dyn Host, event: &mut BlockEntityRenderBoundingBoxEvent<'_>) {
        if event.block != ID {
            return;
        }
        let pos = event.pos;
        for parsed in [engine::parsed(pos, event.entity), engine::parsed_with(minute_key(pos), event.entity, MINUTE)]
            .into_iter()
            .flatten()
        {
            let e = parsed.extent;
            let reach = (0..6).map(|i| e[i].abs()).max().unwrap_or(0) + 2;
            for i in 0..3 {
                let rel = parsed.moving.anchor[i] - pos[i];
                event.bounds[i] = event.bounds[i].min((rel - reach) as f32);
                event.bounds[i + 3] = event.bounds[i + 3].max((rel + reach) as f32);
            }
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
        let facing = drill_rules::placement_facing(looking(context.yaw, context.pitch), context.sneaking);
        level.state_with_property(event.state, "facing", facing.name()).map(Some)
    }

    /// The Wrench turns the bearing and sets its hands down (`ClockworkBearingBlock.onWrenched`).
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != ID {
            return Flow::Continue;
        }
        let (pos, clicked) = (event.pos, Dir::from_normal(event.face));
        util::wrench(host, pos, |host| {
            let (Some(facing), Some(clicked)) = (util::facing(host, pos, "facing"), clicked) else {
                return;
            };
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

    /// `ClockworkBearingBlock.useItemOn`: an empty hand starts or ends the assembly; sneaking with
    /// an empty hand steps the clock hands setting.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else { return Flow::Continue };
        if player.game_mode == util::SPECTATOR || player.game_mode == 2 {
            return Flow::Continue;
        }
        if host.held_stack(player.uuid).is_some_and(|stack| stack.count > 0) {
            return Flow::Continue;
        }
        if !util::first_click(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        if player.sneaking {
            let mut next = Hands::HourFirst;
            host.change_mod_block_entity(pos, &mut |entity| {
                next = Hands::from_value(util::int(&entity.nbt, MODE)).next();
                entity.nbt.insert(MODE.into(), Tag::Int(next.value()));
            });
            host.block_entity_sync(pos);
            let (key, english) = next.label();
            let line = Text::translate_with_fallback("create.contraptions.clockwork.clock_hands", "Clock Hand Arrangement", &[])
                .append(Text::literal(": "))
                .append(Text::translate_with_fallback(key, english, &[]));
            host.action_bar(player.uuid, line);
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

    /// `ClockworkBearingBlockEntity.remove`: a bearing broken while it runs sets its hands down first.
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(entity) = host.mod_block_entity(pos) else { return Flow::Continue };
        if !util::boolean(&entity.nbt, RUNNING) {
            return Flow::Continue;
        }
        set_down(host, pos, &entity.nbt);
        Flow::Continue
    }
}

