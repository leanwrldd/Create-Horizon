//! What the schematic modules do to and ask of the world, on the server: reading a box of blocks
//! into a structure (`SchematicExport.saveSchematic`), putting a state in with its data
//! (`BlockHelper.placeSchematicBlock`), and what stands at a position
//! (`SchematicPrinter.shouldPlaceBlock`'s `toReplace`). The rules are
//! `create_kinetics::rules::schematic_*`.
//!
//! Known differences from Create, for the README:
//! - A schematic holds blocks, not entities: the Schematic and Quill saves no item frames, armor
//!   stands or glue (`clampGlueBoxes`).
//! - A block's saved data is read with `Host::block_entity_nbt` (the `/data get block` text) and
//!   put back with `Host::set_block_tag`: a chest's items travel in the file, but a cannon prints only
//!   the data Create's `SAFE_NBT` allows (`rules::schematic_cannon::safe_block_entity_data`).

use create_kinetics::rules::contraption::State;
use create_kinetics::rules::schematic_nbt::Nbt;
use create_kinetics::rules::schematic_printer::{Existing, has_block_entity};
use create_kinetics::rules::schematic_structure::{Pos, Template, WorldBlock, fill_from_world};
use horizon_client::mod_api::{Hooks, Host, Tag, parse_snbt};

use super::schematic_files::{nbt_of, tag_of};
use crate::util;

/// A module of shared code: no hooks of its own.
struct Shared;

impl Hooks for Shared {}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

/// The most blocks a selection may hold: far past what a person selects, short of a mistake that
/// reads the world for minutes.
pub const MAX_VOLUME: i64 = 4_000_000;

/// The corners of a selection in order: `BoundingBox.fromCorners`.
pub fn corners(a: Pos, b: Pos) -> (Pos, Pos) {
    (
        [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])],
        [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])],
    )
}

pub fn size_of(a: Pos, b: Pos) -> Pos {
    let (lo, hi) = corners(a, b);
    [hi[0] - lo[0] + 1, hi[1] - lo[1] + 1, hi[2] - lo[2] + 1]
}

pub fn volume(size: Pos) -> i64 {
    i64::from(size[0]) * i64::from(size[1]) * i64::from(size[2])
}

/// A block's saved data as the rules hold it, with its `id` and without its position.
fn block_entity_data(host: &mut dyn Host, dimension: &str, pos: Pos, state: &State) -> Option<Nbt> {
    let own = host.mod_block_entity(pos).is_some();
    if !has_block_entity(state, own) {
        return None;
    }
    let text = host.block_entity_nbt(dimension, pos)?;
    let (tag, _) = parse_snbt(&text).ok()?;
    let mut nbt = nbt_of(&tag)?;
    for key in ["x", "y", "z"] {
        nbt.remove(key);
    }
    Some(nbt)
}

/// `SchematicExport.saveSchematic`'s reading: the box between two corners as a structure.
pub fn export(host: &mut dyn Host, dimension: &str, first: Pos, second: Pos) -> Result<Template, String> {
    let (low, _) = corners(first, second);
    let size = size_of(first, second);
    if volume(size) > MAX_VOLUME {
        return Err(format!("{}x{}x{} is too big a selection", size[0], size[1], size[2]));
    }
    let level = host.level(dimension).ok_or("the level is not open")?;
    let mut missing = 0;
    let template = {
        let mut read = |rel: Pos| -> Option<WorldBlock> {
            let at = [low[0] + rel[0], low[1] + rel[1], low[2] + rel[2]];
            let Some(text) = level.block_state(&*host, at) else {
                missing += 1;
                return None;
            };
            let state = State::parse(&text);
            Some(WorldBlock { state, nbt: None })
        };
        fill_from_world(size, &mut read)
    };
    if missing > 0 && missing == volume(size) as usize {
        return Err("none of the selection is loaded".to_owned());
    }
    // The block entities' data, for the blocks that have one.
    let mut template = template;
    for block in &mut template.blocks {
        let state = template.palette[block.state].clone();
        let at = [low[0] + block.pos[0], low[1] + block.pos[1], low[2] + block.pos[2]];
        block.nbt = block_entity_data(host, dimension, at, &state);
    }
    Ok(template)
}

/// What stands at `pos` now, for `shouldPlaceBlock`.
pub fn existing(host: &mut dyn Host, pos: Pos) -> Existing {
    let state = host
        .level(host.dimension())
        .and_then(|level| level.block_state(&*host, pos))
        .map_or_else(|| State::new("minecraft:air"), |text| State::parse(&text));
    let own = !state.name.starts_with("minecraft:") && host.mod_block_entity(pos).is_some();
    Existing { has_block_entity: has_block_entity(&state, own), state }
}

/// Whether the position is in a loaded chunk and inside the world border (`Level.isLoaded`,
/// `WorldBorder.isWithinBounds`).
pub fn loaded(host: &dyn Host, pos: Pos) -> bool {
    let dimension = host.dimension();
    if !host.chunk_is_loaded(dimension, [pos[0] >> 4, pos[2] >> 4]) {
        return false;
    }
    if let Some([min_x, min_z, max_x, max_z]) = host.world_border(dimension) {
        let (x, z) = (f64::from(pos[0]), f64::from(pos[2]));
        if x + 1.0 <= min_x || x >= max_x || z + 1.0 <= min_z || z >= max_z {
            return false;
        }
    }
    host.level(dimension)
        .is_some_and(|level| host.level_is_loaded(level, pos))
}

/// Puts a state into the world at `pos`, every property at once where the game's command can
/// (a block entity's tick), else the default state and its properties one by one
/// (`Level.setBlock(target, state, UPDATE_CLIENTS | UPDATE_KNOWN_SHAPE)`).
pub fn place_state(host: &mut dyn Host, pos: Pos, state: &State) -> bool {
    let dimension = host.dimension();
    let command =
        format!("execute in {dimension} run setblock {} {} {} {} replace", pos[0], pos[1], pos[2], state.text());
    if host.run_command(&command) {
        return true;
    }
    // An unchanged block makes the command fail: that is a success too.
    if let Some(level) = host.level(dimension)
        && level.block_state(&*host, pos).is_some_and(|text| create_kinetics::rules::schematic_printer::same_state(state, &State::parse(&text)))
    {
        return true;
    }
    let Some(base) = host.default_state(&state.name) else {
        return false;
    };
    if !host.set_block(dimension, pos, base, util::UPDATE_ALL) {
        return false;
    }
    for (key, value) in &state.props {
        host.set_block_property(pos, key, value);
    }
    true
}

/// `BlockHelper.placeSchematicBlock` for a state and its data: the state as filtered, the data
/// handed to the block entity it makes.
pub fn place_schematic_block(host: &mut dyn Host, pos: Pos, state: &State, data: Option<&Nbt>) -> bool {
    let state = create_kinetics::rules::schematic_printer::placement_state(state);
    // `Level.setBlock` replaces what is there: no drops, as in Create.
    if !place_state(host, pos, &state) {
        return false;
    }
    if let Some(data) = data
        && let Tag::Compound(mut tag) = tag_of(data)
    {
        for key in ["id", "x", "y", "z"] {
            tag.remove(&key.into());
        }
        let dimension = host.dimension();
        host.set_block_tag(dimension, pos, Tag::Compound(tag));
        host.block_entity_sync(pos);
    }
    true
}
