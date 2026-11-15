//! The Steam Whistle and its extensions (`WhistleBlock`, `WhistleBlockEntity`,
//! `WhistleExtenderBlock`, `WhistleRenderer`, `WhistleSoundInstance`): a whistle on a Fluid Tank
//! (on top, or on a wall) that sounds while redstone powers it and its boiler is heated
//! (`steam_engine_boiler.rs`). Using a Steam Whistle item on one grows a column of extension
//! blocks above it, each lowering the note; the wrench cycles its octave. The rules (pitch, how
//! the column grows, the mouth's animation) are `create_kinetics::rules::steam_engine_whistle`.
//!
//! Known differences from Create (for the README):
//! - The sound is the whistle sample (`create:whistle`, `_low`, `_high`, three seconds that loop
//!   in Create's `WhistleSoundInstance`) played from the server again every 60 ticks while the
//!   whistle sounds, at its pitch (`2^(-pitch/12)`). It can't fade out: the sample being played
//!   finishes when the whistle stops, and the chiff at the start and the steam jet particles are
//!   not done. The content build imports Create's `sounds.json` and `.ogg` files from the jar.
//! - The mouth's rise is worked out on the client from the time the whistle started or stopped
//!   (`Since`, `From`, `On`), in closed form of Create's `LerpedFloat` chases, rather than
//!   chased tick by tick.
//! - The redstone signal, the pitch and the survival of the whistle and its extensions are polled
//!   by their block entities' ticks (an extension has an empty block entity for it), not heard
//!   from neighbour updates (`neighborChanged`, `updateShape`, `queuePitchUpdate`): a change takes
//!   up to 5 ticks to show in the pitch, and a broken whistle's extensions go within 4 ticks.
//! - The goggles' pitch readout and the Pipe Organ advancement are not done. The sneak-wrench on
//!   the upper half of a double extension (`onSneakWrenched`, turning it back to a single) just
//!   removes the block: the click point isn't known.
//! - `getCloneItemStack` of an extension (pick block giving a whistle) isn't done; neither is
//!   waterlogging.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};

use create_kinetics::rules::steam_engine::{from_yaw, horizontal_angle};
use create_kinetics::rules::steam_engine_whistle::{
    self as rules, Above, Below, Grow, Shape, Size,
};
use create_kinetics::{Axis, Dir};
use horizon_client::mod_api::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, BlockStateId, Compound, Flow, Hooks, Host, LevelHost,
    StandaloneModelsEvent, StateForPlacementEvent, Tag, UseItemOnEvent,
};

use super::steam_engine_boiler::{self as boiler, Attached};
use crate::util;

type Pos = [i32; 3];

pub const WHISTLE: &str = "create:steam_whistle";
pub const EXTENSION: &str = "create:steam_whistle_extension";
const CENTRE: [f32; 3] = [0.5; 3];

/// Ticks between two plays of the looping sample (three seconds).
const LOOP_TICKS: i64 = 60;
/// How often the pitch and the redstone are looked at, in ticks (the redstone every tick).
const PITCH_POLL: i64 = 5;
/// How often an extension checks what is under and over it.
const EXTENSION_POLL: i64 = 4;

// `WhistleBlockEntity.write`'s key, and the animation's (not Create's).
const PITCH: &str = "Pitch";
const ON: &str = "On";
const SINCE: &str = "Since";
const FROM: &str = "From";

static PLAYED: LazyLock<Mutex<HashMap<(String, Pos), i64>>> = LazyLock::new(Mutex::default);

struct Whistles;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Whistles)
}

fn add(a: Pos, b: Pos) -> Pos {
    util::add(a, b)
}

fn size_of(host: &dyn Host, pos: Pos) -> Size {
    host.block_property(pos, "size")
        .as_deref()
        .and_then(Size::from_name)
        .unwrap_or(Size::Medium)
}

fn facing_of(host: &dyn Host, pos: Pos) -> Dir {
    util::facing(host, pos, "facing").unwrap_or(Dir::South)
}

fn flag(host: &dyn Host, pos: Pos, key: &str) -> bool {
    host.block_property(pos, key).as_deref() == Some("true")
}

fn is_free(block: &str) -> bool {
    matches!(
        block,
        "minecraft:air"
            | "minecraft:cave_air"
            | "minecraft:void_air"
            | "minecraft:water"
            | "minecraft:lava"
            | "minecraft:short_grass"
            | "minecraft:tall_grass"
            | "minecraft:fern"
            | "minecraft:large_fern"
            | "minecraft:snow"
    )
}

/// What stands at `pos`, for a column above a whistle.
fn above_at(host: &dyn Host, pos: Pos) -> Above {
    let block = host.block(pos);
    if block == EXTENSION {
        let shape = host
            .block_property(pos, "shape")
            .as_deref()
            .and_then(Shape::from_name)
            .unwrap_or(Shape::Single);
        Above::Extension(shape)
    } else if is_free(&block) {
        Above::Free
    } else {
        Above::Blocked
    }
}

fn below_at(host: &dyn Host, pos: Pos) -> Below {
    let block = host.block(pos);
    if block == WHISTLE {
        Below::Whistle
    } else if block == EXTENSION {
        let shape = host
            .block_property(pos, "shape")
            .as_deref()
            .and_then(Shape::from_name)
            .unwrap_or(Shape::Single);
        Below::Extension(shape)
    } else {
        Below::Other
    }
}

/// `WhistleExtenderBlock.findRoot`: the whistle under a column of extensions.
fn find_root(host: &dyn Host, extension: Pos) -> Pos {
    let mut at = add(extension, [0, -1, 0]);
    while host.block(at) == EXTENSION {
        at = add(at, [0, -1, 0]);
    }
    at
}

fn chunk_loaded(host: &dyn Host, pos: Pos) -> bool {
    host.chunk_is_loaded(host.dimension(), [pos[0].div_euclid(16), pos[2].div_euclid(16)])
}

// --- the whistle's tick ---------------------------------------------------------------------------------------

fn tick_whistle(host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
    let pos = event.pos;
    let dimension = host.dimension();
    let now = util::game_time(host);
    let wall = flag(&*host, pos, "wall");
    let facing = facing_of(&*host, pos);
    let size = size_of(&*host, pos);
    let tank_pos = add(pos, rules::attached_direction(wall, facing).normal());

    // `canSurvive`: a tank on its attached side.
    let Some(tank) = boiler::tank_at(&*host, tank_pos) else {
        if chunk_loaded(&*host, tank_pos) && !boiler::is_tank(&host.block(tank_pos)) {
            host.destroy_block(pos, None, false);
        }
        return;
    };
    boiler::register(dimension, tank.controller, pos, Attached::Whistle, now);

    // `neighborChanged`: the block follows the redstone.
    let powered = host.neighbor_signal(dimension, pos) > 0;
    if powered != flag(&*host, pos, "powered") {
        host.set_block_property(pos, "powered", if powered { "true" } else { "false" });
    }

    // `updatePitch`, polled.
    let mut changed = false;
    let spread = i64::from(pos[0] ^ pos[2]) + i64::from(pos[1]);
    if (now + spread).rem_euclid(PITCH_POLL) == 0 {
        let pitch = rules::pitch(|height| above_at(&*host, add(pos, [0, 1 + height, 0])));
        if util::int(&event.entity.nbt, PITCH) != pitch {
            event.entity.nbt.insert(PITCH.into(), Tag::Int(pitch));
            changed = true;
        }
    }

    // The tank is heated and the whistle is powered: it sounds.
    let sounding = powered && tank.boiler.whistles_sound();
    let was = util::boolean(&event.entity.nbt, ON);
    if sounding != was {
        let since = util::long(&event.entity.nbt, SINCE);
        let from = util::float(&event.entity.nbt, FROM).unwrap_or(0.0);
        let value = rules::animation(was, from, (now - since) as f32);
        event.entity.nbt.insert(ON.into(), Tag::Byte(i8::from(sounding)));
        event.entity.nbt.insert(SINCE.into(), Tag::Long(now));
        util::set_float(&mut event.entity.nbt, FROM, value);
        changed = true;
    }
    if changed {
        host.block_entity_sync(pos);
    }

    // The sound, once the sample has ended.
    let key = (dimension.to_owned(), pos);
    if sounding {
        let due = PLAYED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
            .is_none_or(|at| now - *at >= LOOP_TICKS);
        if due {
            let pitch = rules::pitch_factor(util::int(&event.entity.nbt, PITCH));
            // `SoundSource.RECORDS` (2); the sample carries 64 blocks (`attenuationDistance`).
            host.play_sound(size.sound(), util::centre(pos), 2, 4.0, pitch);
            PLAYED.lock().unwrap_or_else(PoisonError::into_inner).insert(key, now);
        }
    } else {
        let mut played = PLAYED.lock().unwrap_or_else(PoisonError::into_inner);
        if played.remove(&key).is_some() || now.rem_euclid(600) == 0 {
            played.retain(|(d, _), at| d != dimension || now - *at < 1200);
        }
    }
}

// --- the extension's tick ------------------------------------------------------------------------------------

fn tick_extension(host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
    let pos = event.pos;
    let now = util::game_time(host);
    let spread = i64::from(pos[0] ^ pos[2]) + i64::from(pos[1]);
    if (now + spread).rem_euclid(EXTENSION_POLL) != 0 {
        return;
    }
    let below_pos = add(pos, [0, -1, 0]);
    if !chunk_loaded(&*host, below_pos) {
        return;
    }
    // `canSurvive`: a whistle or a double extension under it; else it goes, with no drops.
    if !rules::extension_survives(below_at(&*host, below_pos)) {
        host.destroy_block(pos, None, false);
        return;
    }
    // `updateShape` from below: it takes the size of what it stands on.
    let size = size_of(&*host, below_pos);
    if size_of(&*host, pos) != size {
        host.set_block_property(pos, "size", size.name());
    }
    // `updateShape` from above: a double joins the extension over it.
    let shape = host
        .block_property(pos, "shape")
        .as_deref()
        .and_then(Shape::from_name)
        .unwrap_or(Shape::Single);
    let above = host.block(add(pos, [0, 1, 0])) == EXTENSION;
    let next = rules::next_shape(shape, above);
    if next != shape {
        host.set_block_property(pos, "shape", next.name());
    }
}

// --- growing the column ---------------------------------------------------------------------------------------

/// `WhistleBlock.incrementSize` at the whistle at `root`.
fn increment_size(host: &mut dyn Host, root: Pos) {
    let size = size_of(&*host, root);
    let grow = rules::grow(|i| above_at(&*host, add(root, [0, i, 0])));
    let Some(pitch) = rules::grow_pitch(grow) else {
        return;
    };
    let (at, place) = match grow {
        Grow::Double(i) => (add(root, [0, i, 0]), false),
        Grow::Place(i) => (add(root, [0, i, 0]), true),
        Grow::Nothing => return,
    };
    if place {
        let dimension = host.dimension();
        let Some(state) = host.default_state(EXTENSION) else {
            return;
        };
        if !host.set_block(dimension, at, state, util::UPDATE_ALL) {
            return;
        }
        host.set_block_property(at, "size", size.name());
    } else {
        host.set_block_property(at, "shape", Shape::Double.name());
    }
    // `soundtype.getVolume()` of copper is 1: `(1 + 1) / 2`.
    let centre = util::centre(at);
    host.play_sound("minecraft:block.note_block.xylophone", centre, 4, 0.25, pitch);
    host.play_sound("minecraft:block.copper.hit", centre, 4, 1.0, pitch);
}

// --- drawing ------------------------------------------------------------------------------------------------------

/// `WhistleRenderer`: the mouth rises and wiggles while the whistle sounds.
fn draw_whistle(host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
    let (Some(facing), Some(size)) = (
        host.block_state_property(event.state, "facing").and_then(Dir::from_name),
        host.block_state_property(event.state, "size").and_then(Size::from_name),
    ) else {
        return;
    };
    let nbt = &event.entity.nbt;
    let on = util::boolean(nbt, ON);
    let from = util::float(nbt, FROM).unwrap_or(0.0);
    let since = util::long(nbt, SINCE);
    let draw = &mut *event.draw;
    let time = draw.game_time as f64 + f64::from(draw.partial);
    let value = rules::animation(on, from, (time - since as f64) as f32);
    let offset = rules::mouth_offset(value, on, time as f32, size);
    draw.rotate_about(Axis::Y.unit(), horizontal_angle(facing), CENTRE);
    draw.translate([0.0, offset, 0.0]);
    host.draw_model(draw, rules::mouth_model(size), 0xFF_FFFF);
    // The rise reads the game time: nothing here is kept.
}

// --- placement ------------------------------------------------------------------------------------------------------

fn is_tank_state(level: &dyn LevelHost, state: BlockStateId) -> bool {
    level.state_with_property(state, "shape", "window").is_some()
}

/// `WhistleBlock.getStateForPlacement`.
fn placement(
    level: &dyn LevelHost,
    event: &mut StateForPlacementEvent<'_>,
) -> Option<Option<BlockStateId>> {
    let context = event.context;
    let at = [context.pos.x, context.pos.y, context.pos.z];
    let (facing, wall) = rules::placement(util::dir(context.face), from_yaw(context.yaw));
    let tank = add(at, rules::attached_direction(wall, facing).normal());
    // `canSurvive`: no placement away from a tank.
    if !is_tank_state(level, level.block_state(tank)) {
        return Some(None);
    }
    let state = level.state_with_property(event.state, "facing", facing.name())?;
    let state = level.state_with_property(state, "wall", if wall { "true" } else { "false" })?;
    // The redstone is read by the first tick.
    level.state_with_property(state, "powered", "false").map(Some)
}

impl Hooks for Whistles {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        for size in Size::ALL {
            event.models.push(rules::mouth_model(size).to_owned());
        }
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        match event.block {
            WHISTLE => {
                let mut nbt = Compound::new();
                nbt.insert(PITCH.into(), Tag::Int(0));
                nbt.insert(ON.into(), Tag::Byte(0));
                nbt.insert(SINCE.into(), Tag::Long(0));
                util::set_float(&mut nbt, FROM, 0.0);
                Some((WHISTLE.to_owned(), nbt))
            }
            // Create's extension has none; this one only ticks, to keep itself standing.
            EXTENSION => Some((EXTENSION.to_owned(), Compound::new())),
            _ => None,
        }
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == WHISTLE || event.block == EXTENSION {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        match event.entity.id.as_str() {
            WHISTLE => tick_whistle(host, event),
            EXTENSION => tick_extension(host, event),
            _ => {}
        }
    }

    fn block_entity_update_tag(
        &self,
        event: &mut BlockEntityUpdateTagEvent<'_>,
    ) -> Option<Compound> {
        if event.entity.id != WHISTLE {
            return None;
        }
        let nbt = &event.entity.nbt;
        let mut tag = Compound::new();
        tag.insert(PITCH.into(), Tag::Int(util::int(nbt, PITCH)));
        tag.insert(ON.into(), Tag::Byte(i8::from(util::boolean(nbt, ON))));
        tag.insert(SINCE.into(), Tag::Long(util::long(nbt, SINCE)));
        util::set_float(&mut tag, FROM, util::float(nbt, FROM).unwrap_or(0.0));
        Some(tag)
    }

    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block == WHISTLE {
            draw_whistle(host, event);
        }
    }

    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        if &*event.context.item != WHISTLE {
            return None;
        }
        placement(level, event)
    }

    /// `WhistleBlock.useItemOn` (a Steam Whistle grows the column, whichever block of it is
    /// clicked) and the Wrench (`getRotatedBlockState` cycles the size; the extension asks its root).
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        let block = host.block(pos);
        if block != WHISTLE && block != EXTENSION {
            return Flow::Continue;
        }
        let item: &str = &event.stack.item;
        let root = if block == WHISTLE { pos } else { find_root(&*host, pos) };
        if item == WHISTLE {
            if host.block(root) == WHISTLE {
                increment_size(host, root);
                return Flow::Cancel;
            }
            return Flow::Continue;
        }
        if item != util::WRENCH {
            return Flow::Continue;
        }
        util::wrench(host, pos, |host| {
            if host.block(root) == WHISTLE {
                let next = size_of(&*host, root).next();
                host.set_block_property(root, "size", next.name());
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_free_block_is_replaceable_and_a_stone_is_not() {
        assert!(is_free("minecraft:air"));
        assert!(is_free("minecraft:water"));
        assert!(!is_free("minecraft:stone"));
        assert!(!is_free(EXTENSION));
    }

    #[test]
    fn the_player_faces_by_yaw() {
        assert_eq!(from_yaw(0.0), Dir::South);
        assert_eq!(from_yaw(-90.0), Dir::East);
    }
}
