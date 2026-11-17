//! The Cuckoo Clock and the Mysterious Cuckoo Clock (`CuckooClockBlock`, `CuckooClockBlockEntity`,
//! `CuckooClockRenderer`): a clock on a shaft out of its back that shows the world's time on its
//! hands and brings a pig out at noon and a creeper at dusk; the mysterious one brings out a creeper
//! that blows the clock up. The rules and numbers are `create_kinetics::rules::cuckoo_clock`.
//!
//! Known differences from Create, for the README:
//! - Create plays the clock's sounds and smoke on each client (`playLocalSound`, `addParticle`)
//!   from a progress counter it keeps there. A Horizon block entity ticks on the server only, so the
//!   server plays them (`play_sound`, `send_particles`), from the same timeline counted from the
//!   tick the animation started; a player far away hears nothing, as for any block sound.
//! - The hands show the exact time each frame, not eased towards it (`LerpedFloat.chase`), and a
//!   clock that isn't turning draws them at twelve rather than where they stopped.
//! - The animation is saved as `Animation` (Create's name) and `AnimationStart`, the game time it
//!   began, where Create saves a counter it increments each tick; the client derives its progress
//!   from the game time.
//! - The mysterious clock's explosion is a primed TNT with no fuse and an `explosion_power` of 3
//!   (Create's `level.explode(.., 3, false, BLOCK)`), so it blows up as TNT does under `tntExplodes`
//!   and has TNT's damage source, not `create:cuckoo_surprise`. Horizon gives a mod no call to
//!   explode directly, and this was not tried in game.
//! - "natural" dimensions are the Overworld; a custom dimension type is not read.
//! - No shaft preference on placement (`getPreferredHorizontalFacing`), and no "Is it Time?"
//!   advancement or time-of-day display source.

use create_kinetics::rules::cuckoo_clock::{self as rules, Animation};
use create_kinetics::{Dir, Kind, Node};
use crate::compat::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, BlockStateId, Compound, Flow, Hooks, Host, LevelHost, ParticleOptions,
    ParticleType, StandaloneModelsEvent, StateForPlacementEvent, Tag, UseItemOnEvent,
};

use crate::{kinetic, util};

const CLOCK: &str = "create:cuckoo_clock";
const MYSTERIOUS: &str = "create:mysterious_cuckoo_clock";
const ANIMATION: &str = "Animation";
const START: &str = "AnimationStart";
const HOUR_HAND: &str = "create:block/cuckoo_clock/hour_hand";
const MINUTE_HAND: &str = "create:block/cuckoo_clock/minute_hand";
const LEFT_DOOR: &str = "create:block/cuckoo_clock/left_door";
const RIGHT_DOOR: &str = "create:block/cuckoo_clock/right_door";
const PIG: &str = "create:block/cuckoo_clock/pig";
const CREEPER: &str = "create:block/cuckoo_clock/creeper";
const CENTRE: [f32; 3] = [0.5; 3];
const OVERWORLD: &str = "minecraft:overworld";
/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;

struct Clock;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Clock)
}

fn is_clock(block: &str) -> bool {
    block == CLOCK || block == MYSTERIOUS
}

fn animation(entity: &crate::compat::ModBlockEntity) -> Animation {
    match util::get(&entity.nbt, ANIMATION) {
        Some(Tag::String(name)) => Animation::from_name(&name.to_string_lossy()).unwrap_or(Animation::None),
        _ => Animation::None,
    }
}

/// `Direction.getClockWise()` for a horizontal direction.
fn clockwise(facing: Dir) -> Dir {
    match facing {
        Dir::North => Dir::East,
        Dir::East => Dir::South,
        Dir::South => Dir::West,
        Dir::West => Dir::North,
        other => other,
    }
}

impl Hooks for Clock {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event
            .models
            .extend([HOUR_HAND, MINUTE_HAND, LEFT_DOOR, RIGHT_DOOR, PIG, CREEPER].map(str::to_owned));
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        is_clock(event.block).then(|| {
            let mut nbt = kinetic::new_entity();
            nbt.insert(ANIMATION.into(), Tag::String(Animation::None.name().into()));
            (event.block.to_owned(), nbt)
        })
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if is_clock(event.block) {
            event.ticks = true;
        }
    }

    /// `CuckooClockBlockEntity.tick`, the server's half and the sounds of the client's.
    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if !is_clock(&event.entity.id) {
            return;
        }
        let pos = event.pos;
        let mysterious = event.entity.id == MYSTERIOUS;
        let Some(facing) = util::facing(host, pos, "facing") else {
            return;
        };
        // `hasShaftTowards`: the shaft comes out of the back.
        let node = Node {
            impact: rules::IMPACT,
            ..Node::passive(Kind::shafts(&[facing.opposite()]))
        };
        let Some((rpm, changed)) = kinetic::tick(host, pos, event.entity, node) else {
            return; // The network jammed and the block broke.
        };
        if changed {
            host.block_entity_sync(pos);
        }
        if rpm == 0.0 {
            return;
        }

        let natural = host.dimension() == OVERWORLD;
        let (hours, minutes) = rules::clock_time(host.day_time(), natural);
        let now = util::game_time(host);
        let at = util::centre(pos);
        let mut animation = animation(event.entity);

        if !natural {
            // The clock only ticks in a world with no natural time.
            if let Some((sound, volume, pitch)) = rules::idle_tick(now, false) {
                host.play_sound(sound, at, BLOCKS, volume, pitch);
            }
            return;
        }

        if animation == Animation::None {
            if let Some(due) = rules::due(hours, minutes) {
                // `startAnimation`.
                let kind = rules::started(due, mysterious);
                event.entity.nbt.insert(ANIMATION.into(), Tag::String(kind.name().into()));
                event.entity.nbt.insert(START.into(), Tag::Long(now));
                host.block_entity_sync(pos);
                return;
            }
            if let Some((sound, volume, pitch)) = rules::idle_tick(now, true) {
                host.play_sound(sound, at, BLOCKS, volume, pitch);
            }
            return;
        }

        // The tick after the start has `value` 0 and counts up from there.
        let value = (now - util::long(&event.entity.nbt, START) - 1).max(0) as f32;
        if value > rules::LAST_PROGRESS {
            animation = Animation::None;
            event.entity.nbt.insert(ANIMATION.into(), Tag::String(animation.name().into()));
            host.block_entity_sync(pos);
            return;
        }
        if rules::explodes(animation, value) {
            let dimension = host.dimension();
            // `level.destroyBlock(worldPosition, false)`, then the explosion of radius 3.
            host.destroy_block(pos, None, false);
            host.run_command(&format!(
                "execute in {dimension} run summon minecraft:tnt {} {} {} {{fuse:0s,explosion_power:3f}}",
                at[0], at[1], at[2]
            ));
            return;
        }
        for (sound, volume, pitch) in rules::cues(animation, value as i32) {
            host.play_sound(sound, at, BLOCKS, volume, pitch);
        }
        if rules::smokes(animation, value as i32) {
            // `VecHelper.offsetRandomly(centre, random, .5f)`: a puff near the clock.
            host.send_particles(
                &ParticleOptions::simple(ParticleType::LargeSmoke),
                None,
                at,
                [0.25, 0.25, 0.25],
                0.0,
                1,
            );
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        is_clock(&event.entity.id).then(|| {
            let mut tag = kinetic::update_tag(event.entity);
            tag.insert(ANIMATION.into(), Tag::String(animation(event.entity).name().into()));
            tag.insert(START.into(), Tag::Long(util::long(&event.entity.nbt, START)));
            tag
        })
    }

    /// `CuckooClockRenderer.renderSafe`: the shaft, the hands, the doors and the bird.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if !is_clock(event.block) {
            return;
        }
        let Some(facing) = host.block_state_property(event.state, "facing").and_then(Dir::from_name) else {
            return;
        };
        let rpm = kinetic::speed(event.entity);
        let draw = &mut *event.draw;
        kinetic::draw_shaft_half(host, draw, facing.opposite(), rpm);

        let natural = host.dimension() == OVERWORLD;
        let (hours, minutes) = if rpm != 0.0 { rules::clock_time(host.day_time(), natural) } else { (0, 0) };
        let (hour, minute) = rules::hand_angles(hours, minutes);
        let turn = rules::model_angle(facing.name());
        let y = [0.0, 1.0, 0.0];

        // `rotateHand`: about the clock's centre, then the hand about its pivot.
        for (model, angle) in [(HOUR_HAND, hour), (MINUTE_HAND, minute)] {
            draw.push();
            draw.rotate_about(y, turn, CENTRE);
            draw.rotate_about([1.0, 0.0, 0.0], angle, [2.0 / 16.0, 6.0 / 16.0, 8.0 / 16.0]);
            host.draw_model(draw, model, 0xFF_FFFF);
            draw.pop();
        }

        // The doors open at each phase of the animation, and the bird comes out between them.
        let kind = match util::get(&event.entity.nbt, ANIMATION) {
            Some(Tag::String(name)) => Animation::from_name(&name.to_string_lossy()).unwrap_or(Animation::None),
            _ => Animation::None,
        };
        let progress = (draw.game_time - util::long(&event.entity.nbt, START)) as f32 + draw.partial - 1.0;
        let live = kind != Animation::None && (0.0..=rules::LAST_PROGRESS + 1.0).contains(&progress);
        let door = if live { rules::door_angle(kind, progress.max(0.0)) } else { 0.0 };
        for (model, left, z) in [(LEFT_DOOR, true, 6.0), (RIGHT_DOOR, false, 10.0)] {
            draw.push();
            draw.rotate_about(y, turn, CENTRE);
            draw.rotate_about(y, door * if left { -1.0 } else { 1.0 }, [2.0 / 16.0, 0.0, z / 16.0]);
            host.draw_model(draw, model, 0xFF_FFFF);
            draw.pop();
        }
        if live {
            draw.push();
            draw.rotate_about(y, turn, CENTRE);
            draw.translate([rules::figure_offset(door), 0.0, 0.0]);
            host.draw_model(draw, if kind == Animation::Pig { PIG } else { CREEPER }, 0xFF_FFFF);
            draw.pop();
        }
        // Not kept: the hands read the time of day and the doors the game time.
    }

    /// `HorizontalKineticBlock.getStateForPlacement`: the clock faces the player (without lining up
    /// with a shaft beside it, which a placement hook can't read).
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        if !is_clock(&event.context.item) {
            return None;
        }
        let facing = create_kinetics::looking(event.context.yaw, 0.0).opposite();
        level.state_with_property(event.state, "facing", facing.name()).map(Some)
    }

    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || !is_clock(&host.block(event.pos)) {
            return Flow::Continue;
        }
        let (pos, clicked) = (event.pos, Dir::from_normal(event.face));
        util::wrench(host, pos, |host| {
            // `IWrenchable.getRotatedBlockState`: a click on the top or bottom turns it clockwise.
            if let (Some(facing), Some(clicked)) = (util::facing(host, pos, "facing"), clicked)
                && clicked.axis() == create_kinetics::Axis::Y
            {
                host.set_block_property(pos, "facing", clockwise(facing).name());
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrench_turns_the_clock_clockwise_round_the_vertical() {
        assert_eq!(clockwise(Dir::North), Dir::East);
        assert_eq!(clockwise(Dir::West), Dir::North);
        let mut d = Dir::South;
        for _ in 0..4 {
            d = clockwise(d);
        }
        assert_eq!(d, Dir::South);
    }

    #[test]
    fn both_clocks_are_clocks() {
        assert!(is_clock(CLOCK) && is_clock(MYSTERIOUS));
        assert!(!is_clock("create:turntable"));
    }
}
