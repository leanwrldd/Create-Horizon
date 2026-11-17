//! Contraptions: the engine that lifts a group of blocks out of the world, moves or turns them as
//! one and sets them down again, and the blocks that make it up (`Contraption`,
//! `AbstractContraptionEntity`, `ControlledContraptionEntity`, `BlockMovementChecks`,
//! `ContraptionCollider`, `StructureTransform`). The Mechanical Piston (`piston.rs`), the
//! Mechanical and Windmill Bearings (`bearing.rs`), the gantry (`gantry.rs`), the Rope Pulley
//! (`pulley.rs`) and the Elevator Pulley (`elevator.rs`) are built on it, and what gathers into
//! it comes from the chassis (`chassis.rs`) and Super Glue (`super_glue.rs`). This file holds the
//! shared glue and the blocks without a machine of their own (the Piston Extension Pole and the
//! Mechanical Piston Head); the rules, with no game types, are
//! `create_kinetics::rules::contraption`.
//!
//! # Design
//!
//! Horizon has no entity type a mod can give physics, rendering or a block list, no moving
//! collider and no way to draw a block model in the world without a block entity. What it does
//! have is the block entity: it ticks on the server, it is saved with its chunk, an update tag
//! reaches clients, and its draw hook can put any block model at any pose. So:
//!
//! - **Stored.** A moving structure belongs to its controller block entity (the piston, the
//!   bearing), in Create's own layout under `Contraption`: `Anchor` (an int array),
//!   `Blocks.Palette` (`{Name, Properties}`), `Blocks.BlockList` (`{Pos (BlockPos.asLong,
//!   relative to the anchor), State, Data}`), `Data` being the block's own block entity data as
//!   `block_entity_tag` reads it. The blocks are out of the world while it moves (`set_block`
//!   to air, as `removeBlocksFromWorld`), so the structure is saved with the chunk of its
//!   controller and a reload picks it up as it was. A block entity holds nothing in the world
//!   at the place it moves to, so the controller keeps no `ContraptionEntity`: the controller's
//!   own `Running`, `Offset` / `Angle`, `SyncTime` and the saved speed are the whole state.
//! - **Moved without per-tick writes.** The position is a function of time: the controller saves a
//!   snapshot (`Offset` or `Angle`, at game time `SyncTime`) whenever the speed changes, and
//!   both server and client work out where the structure is from the snapshot and the speed
//!   (`offset_at`, `angle_at`). No block entity data changes between syncs, and a reload resumes
//!   exactly. The server only looks at the structure each tick to check what is in its way
//!   and whether it has reached its end.
//! - **Drawn.** By the controller's `block_entity_submit`: every block of the structure through
//!   `draw_block` (its baked model with its state's properties) under a pose that moves it
//!   (a piston: translated by the offset, drawn afresh each frame) or turns it (a bearing: one
//!   `spin` about the axis, then `keep`: the game turns the geometry itself, no per-frame
//!   cost). Chests, ender chests and shulker boxes, which a block entity draws, are drawn as
//!   their item, as Create's contraption renderer does. The controller's render bounding box is
//!   widened to the structure. The light is the controller's. Horizon's update tag carries the
//!   block list (without the blocks' own data) at each sync and when a chunk is sent, so a
//!   client that joins midway sees it.
//! - **Collision: none yet.** A structure is not solid: players, mobs and items pass through it,
//!   and it carries nothing along. Horizon has no moving collider (`Host::collision_boxes` reads
//!   the world's blocks only), and its display entities are client-side per viewer and don't
//!   push. This is the biggest difference from Create (`ContraptionCollider`); everything else is
//!   ported. The structure itself does collide with the *world*: a piston stops against a block
//!   in its way (`isCollidingWithWorld`) and sets what it carries down at the grid cell behind.
//! - **Assembly.** `rules::contraption::search` is Create's `searchMovedStructure`: attached
//!   blocks (torches, ladders, signs, doors, beds...: `isBlockAttachedTowards`), slime and honey
//!   (`canStickTo`), double chests, the forced direction of a piston, the poles and the head of a
//!   piston, a bearing's front block, a gantry's carriage and shafts, up to 2048 blocks. Super
//!   Glue and chassis ranges are queried through `World::glued` and `World::chassis` and answered
//!   by `HostWorld` (`super_glue::glued` from a copy of the level's glue that `prepare` reads
//!   before each assembly, `chassis::range_at`). The block entity data of every block is read
//!   before it is removed.
//! - **Glue and sails travel with the structure.** `capture` takes the glue that holds the
//!   structure's blocks (`Moving.glue`, saved as `Superglue`) out of the level's attachment and
//!   `place_in_world` puts it back where the structure lands; `Moving.sails` counts a windmill's
//!   sail blocks (`Sails`).
//! - **Controllers that are not a block of their own.** A gantry's carriage moves with the
//!   structure, so its structure is kept by the gantry shaft it started on; the engine only needs
//!   a block entity that stays and ticks.
//! - **Disassembly.** `place_in_world` turns each position and state by the structure's
//!   `Transform` (`StructureTransform`: rotation about one axis, in quarter turns, then a move),
//!   breaks what stands in the way (with drops, as `destroyBlock`), places the state and
//!   hands the saved data back with `set_block_tag`. Brittle blocks (torches...) go last and
//!   first out, as Create orders them. A controller that is removed with the structure out
//!   (`destroy_block` hook) disassembles first; one that vanishes some other way (an explosion,
//!   `/setblock`) loses the structure: no hook says a block entity went away.
//! - **Rejected.** Block display entities (`client_entity_spawn`): per viewer, not saved or
//!   tracked, a spawn per block per player, and mod blocks drawn by block entities would not
//!   show. A falling block per moved block (`spawn_falling_block`): real entities but it lands
//!   and places itself, and can't turn. The world-level draw hook (`world_submit`): sprite quads
//!   only, no block models. Both would add a per-tick packet for every block; the block entity
//!   draw needs one update per speed change.
//!
//! # Known differences from Create
//!
//! - No collision with entities (above): nobody rides, stands on or is pushed by a structure.
//! - Blocks inside a structure are drawn as their block model only: what their block entity
//!   draws (a press head, a funnel's flap) is missing, kinetic parts don't turn, items on
//!   depots aren't drawn. Lit by the controller's light, not each block's.
//! - Actors (`MovementBehaviour`: drills, saws, harvesters, ploughs, deployers, funnels) act on the
//!   structures of pistons, bearings and windmills through one hook, `actor.rs` (`actor::tick`
//!   each tick the structure runs, `actor::stop` first thing in the controller's `set_down`); the
//!   gantry, the pulleys and the elevator don't run them. Seats don't carry riders, and movement
//!   interactions (doors, levers on a structure) are not clickable. Belts, fluid tanks and the other blocks whose block entity holds the position of
//!   another block can't be moved (`rules::contraption::movement_allowed`).
//! - A structure whose controller disappears without the `destroy_block` hook (an explosion, a
//!   command) is lost, and one in a chunk that unloads stops until it loads.
//! - Redstone contacts, buttons and plates are not reset when captured; sliding doors, sculk
//!   sensors and the other special cases of `Contraption.capture` and `addBlocksToWorld` aren't.
//! - Drops and break particles of blocks the structure lands in use `destroy_block`; a structure
//!   block that lands on an unbreakable block is dropped as an item.

// Shared with the piston and the bearing, which don't use every helper.
#![allow(dead_code, unused_imports)]

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, LazyLock, Mutex};

use create_kinetics::rules::contraption::{
    self as rules, Fail, State, Transform, World, pack, unpack,
};
use create_kinetics::rules::super_glue::GlueBox;
use create_kinetics::rules::drill::hardness;
use create_kinetics::{Dir, Pos};
use crate::compat::{
    BlockEntityDraw, BlockEntityEvent, BlockEntitySubmitEvent, BlockStateId, Compound, DisplayContext,
    Flow, Hooks, Host, ItemStack, LevelDimension, LevelHost, LevelRef, ModBlockEntity,
    StateForPlacementEvent, StandaloneModelsEvent, Tag, Text, UseItemOnEvent,
};

use crate::util;

pub const POLE: &str = "create:piston_extension_pole";
pub const HEAD: &str = "create:mechanical_piston_head";

// Saved as Create's `LinearActuatorBlockEntity` and `MechanicalBearingBlockEntity` write them.
pub const RUNNING: &str = "Running";
pub const CONTRAPTION: &str = "Contraption";
pub const LAST_EXCEPTION: &str = "LastException";
/// A request to assemble on the next tick, `assembleNextTick`: 1 when the speed changed, 2 when
/// a hand asked (then a failure is told to the players near). Saved, so it survives a reload.
pub const ASSEMBLE: &str = "Assemble";
/// The value of [`ASSEMBLE`] a click leaves.
pub const ASKED: i8 = 2;
/// The game time of the snapshot a client and the server follow the structure from.
pub const SYNC_TIME: &str = "SyncTime";

/// `Level.setBlock`'s flags for taking a block out of a structure and putting it in
/// (`UPDATE_NEIGHBORS | UPDATE_CLIENTS | UPDATE_KNOWN_SHAPE | UPDATE_SUPPRESS_DROPS |
/// UPDATE_MOVE_BY_PISTON`).
const MOVE_FLAGS: i32 = 1 | 2 | 16 | 32 | 64;

struct Contraptions;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Contraptions)
}

// --- the world as the rules see it ---------------------------------------------------------------

/// The server's level read through the host, for [`rules::search`] and friends.
pub struct HostWorld<'a> {
    host: &'a dyn Host,
    level: LevelRef,
    cache: RefCell<HashMap<Pos, Option<State>>>,
    tags: RefCell<HashMap<(String, String), bool>>,
}

impl<'a> HostWorld<'a> {
    pub fn new(host: &'a dyn Host) -> Option<Self> {
        let level = host.level(host.dimension())?;
        Some(HostWorld {
            host,
            level,
            cache: RefCell::default(),
            tags: RefCell::default(),
        })
    }
}

impl World for HostWorld<'_> {
    fn state(&self, pos: Pos) -> Option<State> {
        if let Some(known) = self.cache.borrow().get(&pos) {
            return known.clone();
        }
        let state = self
            .level
            .block_state(self.host, pos)
            .map(|text| State::parse(&text));
        self.cache.borrow_mut().insert(pos, state.clone());
        state
    }

    fn collision_empty(&self, pos: Pos) -> bool {
        self.host.block_collision(pos).is_empty()
    }

    fn in_tag(&self, block: &str, tag: &str) -> bool {
        let key = (block.to_owned(), tag.to_owned());
        if let Some(known) = self.tags.borrow().get(&key) {
            return *known;
        }
        let answer = self.host.block_name_in_tag(block, tag).unwrap_or(false);
        self.tags.borrow_mut().insert(key, answer);
        answer
    }

    fn running(&self, pos: Pos) -> bool {
        self.host
            .mod_block_entity(pos)
            .is_some_and(|entity| util::boolean(&entity.nbt, RUNNING))
    }

    fn glued(&self, pos: Pos, face: Dir) -> bool {
        super::super_glue::glued(self.host, pos, face)
    }

    fn chassis(&self, pos: Pos) -> Option<rules::ChassisInfo> {
        chassis_hook(self.host, pos)
    }
}

/// Reads what a search needs of the level that only the host's `&mut` can reach (the glue,
/// `super_glue.rs`): call it before `HostWorld::new` for an assembly.
pub fn prepare(host: &mut dyn Host) {
    super::super_glue::refresh(host);
}

/// A chassis at `pos` and its range (`ChassisBlockEntity.getRange`, kept by `chassis.rs`).
pub(crate) fn chassis_hook(host: &dyn Host, pos: Pos) -> Option<rules::ChassisInfo> {
    super::chassis::range_at(host, pos).map(|range| rules::ChassisInfo { range })
}

/// Whether a moved state has a collision shape (`getCollisionShape().isEmpty()` of the state;
/// the game can only be asked about a block in the world, so it is told by name).
pub fn solid(state: &State) -> bool {
    !state.is_air()
        && !rules::replaceable(state)
        && !rules::is_brittle_name(state)
        && !rules::movable_empty(state)
}

// --- a structure and its NBT ----------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct MovedBlock {
    /// Relative to the anchor (`StructureBlockInfo.pos`).
    pub local: Pos,
    pub state: State,
    /// The block entity's saved data, if it has one.
    pub data: Option<Compound>,
}

/// A moving structure: Create's `Contraption`'s `blocks` and `anchor`.
#[derive(Clone, Debug, Default)]
pub struct Moving {
    pub anchor: Pos,
    pub blocks: Vec<MovedBlock>,
    /// The Super Glue the structure carries (`Contraption.superglue`), in the same frame as the
    /// blocks: put back where it lands.
    pub glue: Vec<GlueBox>,
    /// The sail-like blocks in it (`BearingContraption.sailBlocks`, saved as `Sails`).
    pub sails: i32,
}

fn string(value: &str) -> Tag {
    Tag::String(value.into())
}

fn get_str(nbt: &Compound, key: &str) -> Option<String> {
    match util::get(nbt, key)? {
        Tag::String(s) => Some(s.to_string_lossy()),
        _ => None,
    }
}

impl Moving {
    /// `Contraption.writeNBT`: `for_client` leaves the blocks' own data out (`spawnPacket`).
    pub fn to_nbt(&self, for_client: bool) -> Compound {
        let mut palette: Vec<State> = Vec::new();
        let mut list = Vec::with_capacity(self.blocks.len());
        for block in &self.blocks {
            let id = palette
                .iter()
                .position(|s| *s == block.state)
                .unwrap_or_else(|| {
                    palette.push(block.state.clone());
                    palette.len() - 1
                });
            let mut entry = Compound::new();
            entry.insert("Pos".into(), Tag::Long(pack(block.local)));
            entry.insert("State".into(), Tag::Int(id as i32));
            if let (false, Some(data)) = (for_client, &block.data) {
                entry.insert("Data".into(), Tag::Compound(data.clone()));
            }
            list.push(Tag::Compound(entry));
        }
        let palette_tag = palette
            .iter()
            .map(|state| {
                let mut entry = Compound::new();
                entry.insert("Name".into(), string(&state.name));
                if !state.props.is_empty() {
                    let mut props = Compound::new();
                    for (key, value) in &state.props {
                        props.insert(key.as_str().into(), string(value));
                    }
                    entry.insert("Properties".into(), Tag::Compound(props));
                }
                Tag::Compound(entry)
            })
            .collect();
        let mut blocks = Compound::new();
        blocks.insert("Palette".into(), Tag::List(palette_tag));
        blocks.insert("BlockList".into(), Tag::List(list));
        let mut out = Compound::new();
        out.insert("Anchor".into(), Tag::IntArray(self.anchor.to_vec()));
        out.insert("Blocks".into(), Tag::Compound(blocks));
        // `Contraption.writeNBT`'s `Superglue`, as boxes of whole blocks.
        let glue = self
            .glue
            .iter()
            .map(|g| {
                let mut entry = Compound::new();
                entry.insert("From".into(), Tag::IntArray(g.min.to_vec()));
                entry.insert("To".into(), Tag::IntArray(g.max.to_vec()));
                Tag::Compound(entry)
            })
            .collect();
        out.insert("Superglue".into(), Tag::List(glue));
        out.insert("Sails".into(), Tag::Int(self.sails));
        out
    }

    /// `Contraption.readNBT`.
    pub fn from_nbt(nbt: &Compound) -> Option<Moving> {
        let Some(Tag::IntArray(anchor)) = util::get(nbt, "Anchor") else {
            return None;
        };
        let anchor: Pos = anchor.as_slice().try_into().ok()?;
        let Some(Tag::Compound(blocks)) = util::get(nbt, "Blocks") else {
            return None;
        };
        let Some(Tag::List(palette)) = util::get(blocks, "Palette") else {
            return None;
        };
        let states: Vec<State> = palette
            .iter()
            .map(|entry| {
                let mut state = State::new(&get_str(compound(entry)?, "Name")?);
                if let Some(Tag::Compound(props)) = util::get(compound(entry)?, "Properties") {
                    for (key, value) in props {
                        if let Tag::String(value) = value {
                            state.props.push((key.to_string_lossy(), value.to_string_lossy()));
                        }
                    }
                }
                Some(state)
            })
            .collect::<Option<_>>()?;
        let Some(Tag::List(list)) = util::get(blocks, "BlockList") else {
            return None;
        };
        let mut out = Vec::with_capacity(list.len());
        for entry in list {
            let entry = compound(entry)?;
            let id = usize::try_from(util::int(entry, "State")).ok()?;
            let data = match util::get(entry, "Data") {
                Some(Tag::Compound(data)) => Some(data.clone()),
                _ => None,
            };
            out.push(MovedBlock {
                local: unpack(util::long(entry, "Pos")),
                state: states.get(id)?.clone(),
                data,
            });
        }
        let mut glue = Vec::new();
        if let Some(Tag::List(list)) = util::get(nbt, "Superglue") {
            for entry in list {
                let corner = |key: &str| match util::get(compound(entry)?, key)? {
                    Tag::IntArray(v) => <Pos>::try_from(v.as_slice()).ok(),
                    _ => None,
                };
                if let (Some(min), Some(max)) = (corner("From"), corner("To")) {
                    glue.push(GlueBox { min, max });
                }
            }
        }
        Some(Moving { anchor, blocks: out, glue, sails: util::int(nbt, "Sails") })
    }

    /// The block states by position, for the collision check.
    pub fn states(&self) -> BTreeMap<Pos, State> {
        self.blocks
            .iter()
            .map(|b| (b.local, b.state.clone()))
            .collect()
    }

    /// How far out from the anchor the blocks reach along each axis, for the draw bounds.
    pub fn extent(&self) -> [i32; 6] {
        let mut e = [0, 0, 0, 0, 0, 0];
        for b in &self.blocks {
            for i in 0..3 {
                e[i] = e[i].min(b.local[i]);
                e[i + 3] = e[i + 3].max(b.local[i]);
            }
        }
        e
    }
}

fn compound(tag: &Tag) -> Option<&Compound> {
    match tag {
        Tag::Compound(c) => Some(c),
        _ => None,
    }
}

// --- capture, removal and placement (server) --------------------------------------------------------

/// Reads the blocks of a structure out of the world: states from `found` and `extra` (a piston's
/// poles), block entity data from the world. `local_of` gives a block's position relative to the
/// structure's anchor. Doesn't change the world.
pub fn capture(
    host: &mut dyn Host,
    anchor: Pos,
    found: &[(Pos, State)],
    extra: &[(Pos, State)],
    local_of: &dyn Fn(Pos) -> Pos,
) -> Moving {
    let dimension = host.dimension();
    let mut blocks = Vec::with_capacity(found.len() + extra.len());
    let in_structure: std::collections::HashSet<Pos> =
        found.iter().chain(extra.iter()).map(|(p, _)| *p).collect();
    for (pos, state) in extra {
        blocks.push(MovedBlock {
            local: local_of(*pos),
            state: state.clone(),
            data: None,
        });
    }
    for (pos, state) in found {
        let data = match host.block_entity_tag(dimension, *pos) {
            Some(Tag::Compound(mut data)) if !data.is_empty() => {
                localise_controller(&mut data, local_of, &|p| in_structure.contains(&p));
                Some(data)
            }
            _ => None,
        };
        blocks.push(MovedBlock {
            local: local_of(*pos),
            state: capture_state(state),
            data,
        });
    }
    // `removeBlocksFromWorld`: the glue that holds the structure leaves the level with it.
    let structure: Vec<Pos> = extra.iter().chain(found.iter()).map(|(p, _)| *p).collect();
    let found_cells: Vec<Pos> = found.iter().map(|(p, _)| *p).collect();
    let glue = super::super_glue::capture(host, &found_cells, &structure, [0; 3])
        .into_iter()
        .map(|g| GlueBox::span(local_of(g.min), local_of(g.max)))
        .collect();
    Moving { anchor, blocks, glue, sails: 0 }
}

/// `Contraption.moveBlock`'s handling of a multi-block's controller (a Fluid Tank's, an Item Vault's):
/// its saved `Controller` is kept relative to the structure's anchor. One that points outside
/// the structure (the multi-block is only partly in it) is dropped and the block stands alone.
pub fn localise_controller(data: &mut Compound, local_of: &dyn Fn(Pos) -> Pos, inside: &dyn Fn(Pos) -> bool) {
    let world: Pos = match util::get(data, "Controller") {
        Some(Tag::IntArray(c)) if c.len() == 3 => [c[0], c[1], c[2]],
        _ => return,
    };
    if inside(world) {
        data.insert("Controller".into(), Tag::IntArray(local_of(world).to_vec()));
        return;
    }
    util::remove(data, "Controller");
    for key in ["Size", "Height", "Length"] {
        if util::has(data, key) {
            data.insert(key.into(), Tag::Int(1));
        }
    }
}

/// The other way: a `Controller` relative to the structure goes where the structure lands
/// (`Contraption.addBlocksToWorld`'s `translateMultiblockControllers`).
pub fn globalise_controller(data: &mut Compound, transform: &Transform) {
    let local: Pos = match util::get(data, "Controller") {
        Some(Tag::IntArray(c)) if c.len() == 3 => [c[0], c[1], c[2]],
        _ => return,
    };
    data.insert("Controller".into(), Tag::IntArray(transform.apply(local).to_vec()));
}

/// What `Contraption.capture` changes in a state as it is taken: a button or pressure plate is
/// released.
fn capture_state(state: &State) -> State {
    let mut state = state.clone();
    let path = state.path();
    if state.name.starts_with("minecraft:")
        && (path.ends_with("_button") || path.ends_with("pressure_plate"))
        && state.has("powered")
    {
        state.set("powered", "false");
    }
    state
}

fn air(host: &dyn Host) -> Option<BlockStateId> {
    host.default_state("minecraft:air")
}

/// `Contraption.removeBlocksFromWorld`: the blocks leave the world, brittle ones first (so they
/// don't pop off as what they hang on goes), without drops, leaving water where a block was
/// waterlogged. `skip` is `customBlockRemoval`: a block it takes stays where it is.
pub fn remove_from_world(
    host: &mut dyn Host,
    moving: &Moving,
    world_of: &dyn Fn(Pos) -> Pos,
    skip: &mut dyn FnMut(&mut dyn Host, Pos, &State) -> bool,
) {
    let dimension = host.dimension();
    let Some(air_state) = air(host) else { return };
    let water = host.default_state("minecraft:water");
    for brittle_pass in [true, false] {
        for block in &moving.blocks {
            let brittle = rules::is_brittle_name(&block.state);
            if brittle != brittle_pass {
                continue;
            }
            let at = world_of(block.local);
            if skip(host, at, &block.state) {
                continue;
            }
            // `UPDATE_SUPPRESS_DROPS` does not stop a container spilling its items here, so the
            // items are taken out first: they travel in the structure's data.
            if let Some(data) = &block.data {
                if matches!(util::get(data, "Items"), Some(Tag::List(items)) if !items.is_empty()) {
                    let mut emptied = Compound::new();
                    emptied.insert("Items".into(), Tag::List(Vec::new()));
                    host.set_block_tag(dimension, at, Tag::Compound(emptied));
                }
            }
            let wet = host
                .level(dimension)
                .and_then(|level| level.block_property(host, at, "waterlogged"))
                .is_some_and(|v| v == "true");
            let target = if wet { water.unwrap_or(air_state) } else { air_state };
            host.set_block(dimension, at, target, MOVE_FLAGS);
        }
    }
}

/// Puts a state into the world at `pos` (`Level.setBlock` of a whole state). Where the game's
/// command can place it exactly, with every property at once; otherwise the default state and
/// its properties one by one.
pub fn place_state(host: &mut dyn Host, pos: Pos, state: &State) -> bool {
    let dimension = host.dimension();
    let command = format!(
        "execute in {dimension} run setblock {} {} {} {} replace",
        pos[0],
        pos[1],
        pos[2],
        state.text()
    );
    if host.run_command(&command) {
        return true;
    }
    // An unchanged block makes the command fail: that is a success too.
    if host.block(pos) == state.name {
        return true;
    }
    let Some(base) = host.default_state(&state.name) else {
        return false;
    };
    if !host.set_block(dimension, pos, base, MOVE_FLAGS) {
        return false;
    }
    for (key, value) in &state.props {
        host.set_block_property(pos, key, value);
    }
    true
}

/// `Contraption.addBlocksToWorld`: the structure is set down turned by `transform`, non-brittle
/// blocks first. `skip` is `customBlockPlacement`: a block it takes is not placed. What stands
/// in the way breaks and drops; a block that would land on an unbreakable one drops as an item.
/// Returns how many blocks were placed.
pub fn place_in_world(
    host: &mut dyn Host,
    moving: &Moving,
    transform: &Transform,
    skip: &mut dyn FnMut(&mut dyn Host, Pos, &State) -> bool,
) -> usize {
    let dimension = host.dimension();
    let mut placed = 0;
    for non_brittle_pass in [true, false] {
        for block in &moving.blocks {
            let brittle = rules::is_brittle_name(&block.state);
            if brittle == non_brittle_pass {
                continue;
            }
            let target = transform.apply(block.local);
            let mut state = transform.apply_state(&block.state);
            if skip(host, target, &state) {
                continue;
            }
            let existing = State::parse(&host.block(target));
            if hardness(&existing.name) < 0.0 {
                // `getDestroySpeed == -1`: the block is lost as an item (`dropResources`).
                if let Some(stack) = util::stack_of(&item_tag(&block.state.name)) {
                    util::spawn_item(host, util::centre(target), &stack, [0.0; 3], 10);
                }
                continue;
            }
            if state.has("waterlogged") {
                let wet = existing.is("minecraft:water");
                state.set("waterlogged", if wet { "true" } else { "false" });
            }
            if !existing.is_air() && existing.name != state.name {
                host.destroy_block(target, None, false);
            }
            if !place_state(host, target, &state) {
                continue;
            }
            placed += 1;
            if let Some(data) = &block.data {
                let mut data = data.clone();
                globalise_controller(&mut data, transform);
                host.set_block_tag(dimension, target, Tag::Compound(data));
                host.block_entity_sync(target);
            }
        }
    }
    // `addBlocksToWorld`: the glue comes back where it lands.
    super::super_glue::restore(host, &moving.glue, transform);
    placed
}

fn item_tag(id: &str) -> Tag {
    let mut tag = Compound::new();
    tag.insert("id".into(), string(id));
    tag.insert("count".into(), Tag::Int(1));
    Tag::Compound(tag)
}

// --- the structure while it runs (server) -----------------------------------------------------------

/// The parsed structure of one running controller, kept between ticks so the server doesn't read
/// the NBT each time.
pub struct Live {
    pub moving: Moving,
    states: BTreeMap<Pos, State>,
    colliders: Mutex<HashMap<Dir, Arc<Vec<Pos>>>>,
}

impl Live {
    pub fn new(moving: Moving) -> Self {
        let states = moving.states();
        Live {
            moving,
            states,
            colliders: Mutex::default(),
        }
    }

    /// `TranslatingContraption.getOrCreateColliders`: the blocks at the leading edge when moving
    /// along `direction`.
    pub fn colliders(&self, direction: Dir) -> Arc<Vec<Pos>> {
        let mut cache = self
            .colliders
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache
            .entry(direction)
            .or_insert_with(|| Arc::new(rules::colliders(&self.states, direction, &solid)))
            .clone()
    }
}

type Key = (String, Pos);

static LIVE: LazyLock<Mutex<HashMap<Key, Arc<Live>>>> = LazyLock::new(Mutex::default);
/// Ticks each controller has seen no speed (see [`quiet_ticks`]).
static QUIET: LazyLock<Mutex<HashMap<Key, u32>>> = LazyLock::new(Mutex::default);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The structure of the controller at `pos`, from the cache or read from its NBT.
pub fn live(dimension: &str, pos: Pos, nbt: &Compound) -> Option<Arc<Live>> {
    live_with(dimension, pos, nbt, CONTRAPTION)
}

/// [`live`] for a controller with a second structure (the Clockwork Bearing's minute hand): `pos`
/// is the key it is cached under (any position that is not another controller's) and `key` the
/// name the structure has in the NBT.
pub fn live_with(dimension: &str, pos: Pos, nbt: &Compound, key_in_nbt: &str) -> Option<Arc<Live>> {
    let key = (dimension.to_owned(), pos);
    if let Some(known) = lock(&LIVE).get(&key) {
        return Some(known.clone());
    }
    let Some(Tag::Compound(saved)) = util::get(nbt, key_in_nbt) else {
        return None;
    };
    let live = Arc::new(Live::new(Moving::from_nbt(saved)?));
    lock(&LIVE).insert(key, live.clone());
    Some(live)
}

pub fn set_live(dimension: &str, pos: Pos, moving: Moving) {
    lock(&LIVE).insert((dimension.to_owned(), pos), Arc::new(Live::new(moving)));
}

pub fn forget_live(dimension: &str, pos: Pos) {
    let key = (dimension.to_owned(), pos);
    lock(&LIVE).remove(&key);
    lock(&QUIET).remove(&key);
}

/// How many ticks in a row the controller has had no speed. A reload or a changed network leaves
/// a block with no speed for a tick or two before its network is solved again; a structure only
/// stops (and is set down) after this many.
pub const QUIET_TICKS: u32 = 10;

/// Counts a tick with (`rpm` nonzero: resets) or without speed; the count so far.
pub fn quiet_ticks(dimension: &str, pos: Pos, rpm: f32) -> u32 {
    let key = (dimension.to_owned(), pos);
    let mut quiet = lock(&QUIET);
    if rpm != 0.0 {
        quiet.remove(&key);
        return 0;
    }
    let count = quiet.entry(key).or_insert(0);
    *count = count.saturating_add(1);
    *count
}

/// Tells the players near `pos` why a structure did not assemble.
pub fn tell_near(host: &mut dyn Host, pos: Pos, fail: &Fail) {
    let dimension = host.dimension().to_owned();
    let line = fail.message();
    for player in host.server_players() {
        if player.dimension != dimension {
            continue;
        }
        let d: f64 = (0..3).map(|i| (player.position[i] - f64::from(pos[i])).powi(2)).sum();
        if d < 12.0 * 12.0 {
            host.action_bar(player.uuid, Text::literal(line.clone()));
        }
    }
}

/// Writes the exception into the block entity (`AssemblyException.write`) when it differs.
pub fn store_exception(entity: &mut ModBlockEntity, fail: Option<&Fail>) -> bool {
    let line = fail.map(Fail::message).unwrap_or_default();
    if get_str(&entity.nbt, LAST_EXCEPTION).unwrap_or_default() == line {
        return false;
    }
    entity.nbt.insert(LAST_EXCEPTION.into(), string(&line));
    true
}

/// The update tag's part for the structure: the `Contraption` compound without the blocks' data,
/// or an empty one when nothing runs (a client keeps what a missing key had).
pub fn client_contraption(entity: &ModBlockEntity) -> Tag {
    let Some(Tag::Compound(saved)) = util::get(&entity.nbt, CONTRAPTION) else {
        return Tag::Compound(Compound::new());
    };
    match Moving::from_nbt(saved) {
        Some(moving) if util::boolean(&entity.nbt, RUNNING) => Tag::Compound(moving.to_nbt(true)),
        _ => Tag::Compound(Compound::new()),
    }
}

// --- drawing (client) -----------------------------------------------------------------------------------

/// A structure as the client draws it each frame.
pub struct Parsed {
    pub moving: Moving,
    pub extent: [i32; 6],
}

static PARSED: LazyLock<Mutex<HashMap<Pos, (String, Option<Arc<Parsed>>)>>> =
    LazyLock::new(Mutex::default);

/// The structure in a controller's NBT, parsed once per version of the block entity.
pub fn parsed(pos: Pos, entity: &ModBlockEntity) -> Option<Arc<Parsed>> {
    parsed_with(pos, entity, CONTRAPTION)
}

/// [`parsed`] for a second structure of one controller (see [`live_with`]).
pub fn parsed_with(pos: Pos, entity: &ModBlockEntity, key_in_nbt: &str) -> Option<Arc<Parsed>> {
    if !util::boolean(&entity.nbt, RUNNING) {
        return None;
    }
    let version = format!("{:?}", entity.revision());
    let mut cache = lock(&PARSED);
    if let Some((seen, parsed)) = cache.get(&pos) {
        if *seen == version {
            return parsed.clone();
        }
    }
    let parsed = match util::get(&entity.nbt, key_in_nbt) {
        Some(Tag::Compound(saved)) => Moving::from_nbt(saved).map(|moving| {
            Arc::new(Parsed {
                extent: moving.extent(),
                moving,
            })
        }),
        _ => None,
    };
    if cache.len() > 512 {
        cache.clear();
    }
    cache.insert(pos, (version, parsed.clone()));
    parsed
}

/// Draws one block of a structure at the pose in force: its baked model with its state.
pub fn draw_state(host: &dyn Host, draw: &mut BlockEntityDraw, state: &State) {
    let path = state.path();
    if state.name.starts_with("minecraft:")
        && (matches!(path, "chest" | "trapped_chest" | "ender_chest") || path.ends_with("shulker_box"))
    {
        draw_entity_block(host, draw, state);
        return;
    }
    let props: Vec<(&str, &str)> = state
        .props
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    if !host.draw_block(draw, &state.name, &props) {
        host.draw_block(draw, &state.name, &[]);
    }
    // A Portable Interface's middle and top are drawn by its renderer, which doesn't run here.
    super::portable_interface_motion::draw_moving(host, draw, state);
}

/// A block a block entity draws all of (`ENTITYBLOCK_ANIMATED`): its item's model, turned to its
/// facing as `ChestRenderer` turns it (`BlockEntityRenderHelper.renderBlockEntities` does the same
/// for a riding one).
fn draw_entity_block(host: &dyn Host, draw: &mut BlockEntityDraw, state: &State) {
    let Some(stack) = util::stack_of(&item_tag(&state.name)) else {
        return;
    };
    draw.push();
    if state.path().ends_with("shulker_box") {
        let (axis, degrees) = match state.get("facing") {
            Some("down") => ([1.0, 0.0, 0.0], 180.0),
            Some("north") => ([1.0, 0.0, 0.0], -90.0),
            Some("south") => ([1.0, 0.0, 0.0], 90.0),
            Some("west") => ([0.0, 0.0, 1.0], 90.0),
            Some("east") => ([0.0, 0.0, 1.0], -90.0),
            _ => ([1.0, 0.0, 0.0], 0.0),
        };
        draw.rotate_about(axis, degrees, [0.5; 3]);
    } else {
        let y_rot = match state.get("facing") {
            Some("west") => 90.0,
            Some("north") => 180.0,
            Some("east") => 270.0,
            _ => 0.0,
        };
        draw.rotate_about([0.0, 1.0, 0.0], -y_rot, [0.5; 3]);
    }
    draw.translate([0.5; 3]);
    host.draw_item(draw, &stack, DisplayContext::None, None, 0);
    draw.pop();
}

/// Draws every block of a structure, each at `base` + its position (blocks, from the
/// controller's corner), under the pose in force. `skip` leaves a block out (a piston's pole at
/// the piston's own cell, which Create draws too: it's the shaft inside).
pub fn draw_blocks(host: &dyn Host, draw: &mut BlockEntityDraw, parsed: &Parsed, base: [f32; 3]) {
    // How the structure moves, for the actors' turning heads (`actor_draw.rs`).
    let motion = super::actor_draw::take_motion();
    for block in &parsed.moving.blocks {
        draw.push();
        draw.translate([
            base[0] + block.local[0] as f32,
            base[1] + block.local[1] as f32,
            base[2] + block.local[2] as f32,
        ]);
        draw_state(host, draw, &block.state);
        super::actor_draw::draw_actor(host, draw, &block.state, block.local, motion);
        draw.pop();
    }
}

// --- the hooks of the two blocks without a machine -----------------------------------------------------

impl Hooks for Contraptions {
    fn block_entity(
        &self,
        _: &mut dyn Host,
        _: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        None
    }

    /// `PistonExtensionPoleBlock.getStateForPlacement`: facing the face clicked, turned round.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != POLE {
            return None;
        }
        let clicked = util::dir(context.face);
        level
            .state_with_property(event.state, "facing", clicked.opposite().name())
            .map(Some)
    }

    /// The Wrench turns a pole (`WrenchableDirectionalBlock`) and picks it up.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != POLE {
            return Flow::Continue;
        }
        let (pos, clicked) = (event.pos, Dir::from_normal(event.face));
        util::wrench(host, pos, |host| {
            let (Some(facing), Some(clicked)) = (util::facing(host, pos, "facing"), clicked) else {
                return;
            };
            // `WrenchableDirectionalBlock.getRotatedBlockState`.
            let next = if facing.axis() == clicked.axis() {
                facing.opposite()
            } else {
                create_kinetics::rules::drill::clockwise(facing, clicked.axis())
            };
            if next != facing {
                host.set_block_property(pos, "facing", next.name());
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Moving {
        Moving {
            anchor: [10, 64, -5],
            blocks: vec![
                MovedBlock {
                    local: [0, 0, 0],
                    state: State::parse("minecraft:stone"),
                    data: None,
                },
                MovedBlock {
                    local: [-1, 2, -3],
                    state: State::parse("minecraft:oak_stairs[facing=east,half=top]"),
                    data: None,
                },
                MovedBlock {
                    local: [1, 0, 0],
                    state: State::parse("minecraft:chest[facing=north,type=single]"),
                    data: Some({
                        let mut data = Compound::new();
                        data.insert("Lock".into(), Tag::String("".into()));
                        data
                    }),
                },
                MovedBlock {
                    local: [2, 0, 0],
                    state: State::parse("minecraft:stone"),
                    data: None,
                },
            ],
            glue: vec![GlueBox { min: [0, 0, 0], max: [2, 0, 0] }],
            sails: 9,
        }
    }

    #[test]
    fn the_glue_a_structure_carries_survives_its_nbt() {
        let back = Moving::from_nbt(&sample().to_nbt(false)).expect("reads back");
        assert_eq!(back.glue, vec![GlueBox { min: [0, 0, 0], max: [2, 0, 0] }]);
        assert_eq!(back.sails, 9);
        // An older save without the key has none.
        let mut nbt = sample().to_nbt(false);
        nbt.remove(&"Superglue".into());
        assert!(Moving::from_nbt(&nbt).expect("reads").glue.is_empty());
    }

    #[test]
    fn a_structure_survives_its_nbt() {
        let moving = sample();
        let back = Moving::from_nbt(&moving.to_nbt(false)).expect("reads back");
        assert_eq!(back.anchor, moving.anchor);
        assert_eq!(back.blocks.len(), 4);
        for (a, b) in moving.blocks.iter().zip(&back.blocks) {
            assert_eq!(a.local, b.local);
            assert_eq!(a.state, b.state);
            assert_eq!(a.data, b.data);
        }
    }

    #[test]
    fn the_palette_holds_each_state_once() {
        let nbt = sample().to_nbt(false);
        let Some(Tag::Compound(blocks)) = util::get(&nbt, "Blocks") else {
            panic!("no blocks")
        };
        let Some(Tag::List(palette)) = util::get(blocks, "Palette") else {
            panic!("no palette")
        };
        assert_eq!(palette.len(), 3);
    }

    #[test]
    fn a_clients_copy_has_no_block_data() {
        let nbt = sample().to_nbt(true);
        let back = Moving::from_nbt(&nbt).expect("reads back");
        assert!(back.blocks.iter().all(|b| b.data.is_none()));
        assert_eq!(back.blocks.len(), 4);
    }

    #[test]
    fn a_multi_blocks_controller_is_saved_relative_and_goes_where_the_structure_lands() {
        let anchor = [10, 64, 10];
        let local_of = |p: Pos| [p[0] - anchor[0], p[1] - anchor[1], p[2] - anchor[2]];
        let mut data = Compound::new();
        data.insert("Controller".into(), Tag::IntArray(vec![10, 65, 11]));
        data.insert("Size".into(), Tag::Int(2));
        localise_controller(&mut data, &local_of, &|p| p == [10, 65, 11]);
        assert_eq!(util::get(&data, "Controller"), Some(&Tag::IntArray(vec![0, 1, 1])));
        assert_eq!(util::int(&data, "Size"), 2);
        // Landing three blocks east and a quarter turn about Y.
        let transform = Transform { offset: [13, 64, 10], axis: create_kinetics::Axis::Y, quarters: 0 };
        globalise_controller(&mut data, &transform);
        assert_eq!(util::get(&data, "Controller"), Some(&Tag::IntArray(vec![13, 65, 11])));
        // A controller outside the structure is dropped and the block stands alone.
        let mut outside = Compound::new();
        outside.insert("Controller".into(), Tag::IntArray(vec![0, 0, 0]));
        outside.insert("Size".into(), Tag::Int(3));
        outside.insert("Height".into(), Tag::Int(4));
        localise_controller(&mut outside, &local_of, &|_| false);
        assert!(!util::has(&outside, "Controller"));
        assert_eq!((util::int(&outside, "Size"), util::int(&outside, "Height")), (1, 1));
        // A block with no controller is left alone.
        let mut plain = Compound::new();
        localise_controller(&mut plain, &local_of, &|_| true);
        assert!(plain.is_empty());
    }

    #[test]
    fn the_extent_covers_every_block() {
        assert_eq!(sample().extent(), [-1, 0, -3, 2, 2, 0]);
    }

    #[test]
    fn colliders_of_a_live_structure() {
        let live = Live::new(sample());
        let east = live.colliders(Dir::East);
        assert!(east.contains(&[2, 0, 0]) && !east.contains(&[1, 0, 0]));
        // Cached: the same list again.
        assert!(Arc::ptr_eq(&east, &live.colliders(Dir::East)));
    }

    #[test]
    fn no_speed_has_to_last_before_a_structure_stops() {
        let (dimension, pos) = ("minecraft:overworld", [7, 7, 7]);
        for count in 1..=3 {
            assert_eq!(quiet_ticks(dimension, pos, 0.0), count);
        }
        assert_eq!(quiet_ticks(dimension, pos, 16.0), 0);
        assert_eq!(quiet_ticks(dimension, pos, 0.0), 1);
        forget_live(dimension, pos);
    }
}
