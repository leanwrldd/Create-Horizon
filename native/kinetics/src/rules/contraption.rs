//! The contraption engine's rules: which blocks a contraption takes along, and how a moved
//! structure is turned. Ported from Create (MIT, the Creators of Create), `mc1.21.1/dev` at
//! 4148fd8: `Contraption.searchMovedStructure` and `moveBlock`, `BlockMovementChecksImpl` (every
//! fallback check), `StructureTransform`, `BlockPos.asLong`, `ContraptionCollider.isCollidingWith
//! World` and `TranslatingContraption.createColliders`.
//!
//! No game types: the glue (`native/api/src/machines/contraption.rs`, where the design is written
//! down) answers the [`World`] questions from Horizon's `Host` and holds the NBT.
//!
//! Known differences from Create:
//! - A block is told by its name and the property strings of its state, never by its class:
//!   Create's `instanceof` checks are lists of names here (`is_brittle`, `attached_towards`,
//!   `not_supportive`), written for vanilla's blocks and this mod's.
//! - `BlockState.rotate` is not available, so [`rotate_state`] turns the properties that vanilla's
//!   and Create's blocks turn with (`facing`, `axis`, `rotation`, the four sides, rails,
//!   `axis_along_first`, `horizontal_axis`); another mod's block with a property of its own that
//!   needs turning isn't.
//! - No stabilised sub-contraptions (`canBeStabilized` is `stabiliseStableContraptions`, off by
//!   default, and the Mechanical Bearing's own check is not ported), seats, belts, bogeys, hose
//!   pulleys or cart assemblers: those blocks are not moved here. A gantry carriage and its shafts
//!   are (`rules::gantry::targets`); a running controller is not (`World::running`).

use std::collections::{BTreeMap, HashSet, VecDeque};

use crate::rules::contraption_facts::{BLOCKS_PUSH, PUSH_ONLY, REPLACEABLE};
use crate::rules::drill::{hardness, is_air};
use crate::{Axis, Dir, Pos};

/// `CKinetics.maxBlocksMoved`.
pub const MAX_BLOCKS_MOVED: usize = 2048;
/// `CKinetics.maxChassisRange`.
pub const MAX_CHASSIS_RANGE: i32 = 16;
/// `CKinetics.maxPistonPoles`.
pub const MAX_PISTON_POLES: i32 = 64;
/// `Contraption.searchMovedStructure`'s limit of steps before `structureTooLarge`.
const SEARCH_LIMIT: usize = 100_000;

// --- a block state as text -----------------------------------------------------------------------

/// A block state as `/setblock` writes it: `minecraft:oak_stairs[facing=east,half=bottom]`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct State {
    pub name: String,
    pub props: Vec<(String, String)>,
}

impl State {
    pub fn new(name: &str) -> Self {
        State {
            name: name.to_owned(),
            props: Vec::new(),
        }
    }

    pub fn with(mut self, key: &str, value: &str) -> Self {
        self.set(key, value);
        self
    }

    /// Reads `name[key=value,...]`; a name alone has no properties.
    pub fn parse(text: &str) -> Self {
        let text = text.trim();
        let Some((name, rest)) = text.split_once('[') else {
            return State::new(text);
        };
        let props = rest
            .trim_end_matches(']')
            .split(',')
            .filter_map(|pair| pair.split_once('='))
            .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
            .collect();
        State {
            name: name.to_owned(),
            props,
        }
    }

    pub fn text(&self) -> String {
        if self.props.is_empty() {
            return self.name.clone();
        }
        let props: Vec<String> = self.props.iter().map(|(k, v)| format!("{k}={v}")).collect();
        format!("{}[{}]", self.name, props.join(","))
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.props
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn set(&mut self, key: &str, value: &str) {
        match self.props.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) => *v = value.to_owned(),
            None => self.props.push((key.to_owned(), value.to_owned())),
        }
    }

    /// Whether the property is `true` (`BooleanProperty`).
    pub fn flag(&self, key: &str) -> bool {
        self.get(key) == Some("true")
    }

    pub fn dir(&self, key: &str) -> Option<Dir> {
        self.get(key).and_then(Dir::from_name)
    }

    pub fn is(&self, name: &str) -> bool {
        self.name == name
    }

    /// The block's id without its namespace.
    pub fn path(&self) -> &str {
        self.name.split_once(':').map_or(&self.name, |(_, path)| path)
    }

    pub fn is_air(&self) -> bool {
        is_air(&self.name)
    }
}

// --- BlockPos.asLong -----------------------------------------------------------------------------

/// `BlockPos.asLong`: x in 26 bits, z in 26 bits, y in 12 bits.
pub fn pack(pos: Pos) -> i64 {
    ((i64::from(pos[0]) & 0x3FF_FFFF) << 38)
        | ((i64::from(pos[2]) & 0x3FF_FFFF) << 12)
        | (i64::from(pos[1]) & 0xFFF)
}

/// `BlockPos.of(long)`, signed.
pub fn unpack(packed: i64) -> Pos {
    [
        (packed >> 38) as i32,
        ((packed << 52) >> 52) as i32,
        ((packed << 26) >> 38) as i32,
    ]
}

pub fn add(a: Pos, b: Pos) -> Pos {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn sub(a: Pos, b: Pos) -> Pos {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// `BlockPos.relative(direction, steps)`.
pub fn relative(pos: Pos, dir: Dir, steps: i32) -> Pos {
    let n = dir.normal();
    [pos[0] + n[0] * steps, pos[1] + n[1] * steps, pos[2] + n[2] * steps]
}

// --- what the search asks the world --------------------------------------------------------------

/// What the search reads of the world. The glue answers from Horizon's `Host`.
pub trait World {
    /// The state at `pos`; `None` where the chunk isn't loaded or the height is outside the world
    /// (`Level.isLoaded`, `isOutsideBuildHeight`).
    fn state(&self, pos: Pos) -> Option<State>;
    /// `state.getCollisionShape(level, pos).isEmpty()` of the block at `pos`.
    fn collision_empty(&self, pos: Pos) -> bool;
    /// Whether block `block` is in the block tag `tag` (`minecraft:doors`).
    fn in_tag(&self, _block: &str, _tag: &str) -> bool {
        false
    }
    /// `SuperGlueEntity.isGlued`: glue holds the block at `pos` to its neighbour on `face`.
    fn glued(&self, _pos: Pos, _face: Dir) -> bool {
        false
    }
    /// Whether the Mechanical Bearing or piston at `pos` is running (`isRunning`); a running
    /// controller is not moved.
    fn running(&self, _pos: Pos) -> bool {
        false
    }
    /// The chassis at `pos` (`ChassisBlockEntity`), if it is one.
    fn chassis(&self, _pos: Pos) -> Option<ChassisInfo> {
        None
    }
}

/// A chassis block as the search sees it (`ChassisBlockEntity`).
#[derive(Clone, Debug)]
pub struct ChassisInfo {
    /// `getRange`.
    pub range: i32,
}

/// Why a structure could not be put together (`AssemblyException`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fail {
    UnloadedChunk(Pos),
    /// `unmovableBlock(pos, state)`.
    UnmovableBlock(Pos, String),
    StructureTooLarge,
    TooManyPistonPoles,
    NoPistonPoles,
    NotEnoughSails(i32),
    /// `ElevatorContraption.assemble`: no contact, more than one, or another cabin in the column.
    NoContacts,
    TooManyContacts,
    ColumnConflict,
}

impl Fail {
    /// A short line for the player, in the words of `AssemblyException`'s messages.
    pub fn message(&self) -> String {
        match self {
            Fail::UnloadedChunk(p) => {
                format!("Cannot move blocks in an unloaded chunk ({}, {}, {})", p[0], p[1], p[2])
            }
            Fail::UnmovableBlock(p, name) => {
                format!("Cannot move the block at ({}, {}, {}): {name}", p[0], p[1], p[2])
            }
            Fail::StructureTooLarge => {
                format!("Structure too large (more than {MAX_BLOCKS_MOVED} blocks)")
            }
            Fail::TooManyPistonPoles => format!("Too many extension poles behind the piston (more than {MAX_PISTON_POLES})"),
            Fail::NoPistonPoles => "The piston has no extension poles behind it".to_owned(),
            Fail::NoContacts => "Elevators require a horizontal Contact attached to the structure".to_owned(),
            Fail::TooManyContacts => "Cannot attach more than one Contact to Elevators".to_owned(),
            Fail::ColumnConflict => {
                "Another Elevator is already targeting this column of contacts".to_owned()
            }
            Fail::NotEnoughSails(n) => format!(
                "Attached structure does not include enough sail-like blocks: {n}
A minimum of {} are required",
                crate::rules::windmill::MINIMUM_SAILS
            ),
        }
    }
}

// --- BlockMovementChecksImpl ---------------------------------------------------------------------

fn listed(list: &[&str], name: &str) -> bool {
    list.binary_search(&name).is_ok()
}

/// `state.canBeReplaced()`.
pub fn replaceable(state: &State) -> bool {
    listed(REPLACEABLE, &state.name)
}

/// `state.getPistonPushReaction() == PUSH_ONLY`.
pub fn push_only(state: &State) -> bool {
    listed(PUSH_ONLY, &state.name)
}

/// The vanilla blocks `BlockMovementChecksImpl.isBrittleFallback` and the `create:brittle` tag
/// list, by name; and Create's own.
pub fn is_brittle(world: &dyn World, state: &State) -> bool {
    is_brittle_name(state) || world.in_tag(&state.name, "create:brittle")
}

/// [`is_brittle`] without the tag the world may add.
pub fn is_brittle_name(state: &State) -> bool {
    let path = state.path();
    if state.has("hanging") {
        return true;
    }
    let vanilla = state.name.starts_with("minecraft:");
    if vanilla {
        // LadderBlock, BaseTorchBlock, SignBlock, BasePressurePlateBlock, BaseRailBlock,
        // DiodeBlock, RedStoneWireBlock, WoolCarpetBlock.
        if path == "ladder"
            || path.ends_with("torch")
            || path.ends_with("wall_torch")
            || path.ends_with("_sign")
            || path.ends_with("_wall_sign")
            || path.ends_with("hanging_sign")
            || path.ends_with("pressure_plate")
            || matches!(path, "rail" | "powered_rail" | "detector_rail" | "activator_rail")
            || matches!(path, "repeater" | "comparator" | "redstone_wire")
            || (path.ends_with("_carpet") && path != "moss_carpet" && path != "pale_moss_carpet")
        {
            return true;
        }
        // FaceAttachedHorizontalDirectionalBlock (levers and buttons) but not the grindstone.
        if path == "lever" || path.ends_with("_button") {
            return true;
        }
        // The `create:brittle` tag: bell, cocoa, flower pot, moss carpet, bamboo sapling, big
        // dripleaf, vines, cake, the corals, cave vines, banners, beds and doors.
        if matches!(
            path,
            "bell"
                | "cocoa"
                | "flower_pot"
                | "moss_carpet"
                | "bamboo_sapling"
                | "big_dripleaf"
                | "vine"
                | "twisting_vines"
                | "twisting_vines_plant"
                | "weeping_vines"
                | "weeping_vines_plant"
                | "cake"
                | "cave_vines"
                | "cave_vines_plant"
        ) || path.contains("coral")
            || path.ends_with("_banner")
            || path.ends_with("_bed")
            || path.ends_with("_door")
        {
            return true;
        }
        return false;
    }
    // Create's own: the whistle, its extension, belt funnels and the blocks tagged
    // `create:brittle` (`NozzleBlock`, `HandCrankBlock`).
    matches!(
        path,
        "steam_whistle"
            | "steam_whistle_extension"
            | "andesite_belt_funnel"
            | "brass_belt_funnel"
            | "nozzle"
            | "hand_crank"
            | "rope"
            | "pulley_magnet"
            | "redstone_link"
    )
}

/// `BlockMovementChecksImpl.isMovementNecessaryFallback`.
pub fn movement_necessary(world: &dyn World, state: &State, pos: Pos) -> bool {
    if is_brittle(world, state) {
        return true;
    }
    // `create:movable_empty_collider`: cobweb, powder snow, tripwire and its hook, vines, corals,
    // fence gates and banners move though they have no collision.
    if movable_empty(state) {
        return true;
    }
    if world.collision_empty(pos) {
        return false;
    }
    !replaceable(state)
}

pub fn movable_empty(state: &State) -> bool {
    let path = state.path();
    state.name.starts_with("minecraft:")
        && (matches!(
            path,
            "cobweb"
                | "powder_snow"
                | "tripwire"
                | "tripwire_hook"
                | "bamboo_sapling"
                | "vine"
                | "twisting_vines_plant"
                | "twisting_vines"
                | "weeping_vines_plant"
                | "weeping_vines"
                | "cave_vines"
                | "cave_vines_plant"
        ) || path.contains("coral")
            || path.ends_with("fence_gate")
            || path.ends_with("_banner"))
}

/// This mod's blocks whose block entity holds positions of other blocks (a tank's controller, an
/// engine's boiler, a belt's controller): moving them would leave their data pointing at the old
/// place, so they stay (`create:non_movable` here).
fn keeps_position_data(path: &str) -> bool {
    matches!(
        path,
        "belt"
            | "fluid_tank"
            | "creative_fluid_tank"
            | "steam_engine"
            | "crushing_wheel_controller"
            | "powered_shaft"
    )
}

/// `BlockMovementChecksImpl.isMovementAllowedFallback`.
pub fn movement_allowed(world: &dyn World, state: &State, pos: Pos) -> bool {
    let path = state.path();
    let create = state.name.starts_with("create:");
    // Chassis are always allowed.
    if create && matches!(path, "linear_chassis" | "secondary_linear_chassis" | "radial_chassis") {
        return true;
    }
    if hardness(&state.name) < 0.0 {
        return false;
    }
    if world.in_tag(&state.name, "minecraft:relocation_not_supported")
        || world.in_tag(&state.name, "create:non_movable")
    {
        return false;
    }
    if create && keeps_position_data(path) {
        return false;
    }
    // Controllers move only when they aren't moving.
    if create && matches!(path, "mechanical_piston" | "sticky_mechanical_piston") {
        return state.get("state") != Some("moving");
    }
    if create && matches!(path, "mechanical_bearing" | "windmill_bearing" | "gantry_shaft") {
        return !world.running(pos);
    }
    if state.is("minecraft:grindstone") {
        return true;
    }
    !listed(BLOCKS_PUSH, &state.name)
}

/// `BlockMovementChecksImpl.isBlockAttachedTowardsFallback`: whether this block hangs on the block
/// in `direction` (towards the block the search came from).
pub fn attached_towards(world: &dyn World, state: &State, direction: Dir) -> bool {
    let _ = world;
    let path = state.path();
    let vanilla = state.name.starts_with("minecraft:");
    let facing = state.dir("facing");
    if vanilla {
        if path == "ladder" || path == "wall_torch" || path == "soul_wall_torch"
            || path == "redstone_wall_torch" || path.ends_with("_wall_sign")
            || path.ends_with("_wall_banner")
        {
            return facing == Some(direction.opposite());
        }
        if path.ends_with("_sign") && !path.ends_with("hanging_sign") {
            return direction == Dir::Down;
        }
        if path.ends_with("pressure_plate") {
            return direction == Dir::Down;
        }
        if path.ends_with("_door") {
            if state.get("half") == Some("lower") && direction == Dir::Up {
                return true;
            }
            return direction == Dir::Down;
        }
        if path.ends_with("_bed") {
            let mut towards = facing.unwrap_or(Dir::North);
            if state.get("part") == Some("head") {
                towards = towards.opposite();
            }
            return direction == towards;
        }
        if path == "flower_pot" || path.starts_with("potted_") {
            return direction == Dir::Down;
        }
        if matches!(path, "repeater" | "comparator" | "redstone_wire") || path.ends_with("_carpet") {
            return direction == Dir::Down;
        }
        if path.ends_with("torch") {
            return direction == Dir::Down;
        }
        // Levers and buttons: `FaceAttachedHorizontalDirectionalBlock`.
        if path == "lever" || path.ends_with("_button") {
            return match state.get("face") {
                Some("ceiling") => direction == Dir::Up,
                Some("floor") => direction == Dir::Down,
                Some("wall") => Some(direction.opposite()) == facing,
                _ => false,
            };
        }
        if let Some(hanging) = state.get("hanging") {
            return direction == if hanging == "true" { Dir::Up } else { Dir::Down };
        }
        if matches!(path, "rail" | "powered_rail" | "detector_rail" | "activator_rail") {
            return direction == Dir::Down;
        }
        if path == "bell" {
            return match state.get("attachment") {
                Some("floor") => direction == Dir::Down,
                Some("ceiling") => direction == Dir::Up,
                _ => Some(direction) == facing,
            };
        }
        return false;
    }
    // Create's own blocks.
    match path {
        "hand_crank" | "nozzle" => facing == Some(direction.opposite()),
        "steam_whistle" => {
            direction
                == if state.flag("wall") {
                    facing.unwrap_or(Dir::Down)
                } else {
                    Dir::Down
                }
        }
        "steam_whistle_extension" => direction == Dir::Down,
        _ => false,
    }
}

/// `BlockMovementChecksImpl.isNotSupportiveFallback`: a block nothing can be attached to on the
/// face `facing`.
pub fn not_supportive(world: &dyn World, state: &State, facing: Dir) -> bool {
    let path = state.path();
    if state.name.starts_with("create:") {
        match path {
            "mechanical_drill" | "mechanical_bearing" | "mechanical_saw" => {
                return state.dir("facing") == Some(facing);
            }
            "piston_extension_pole" | "mechanical_piston_head" => {
                return state.dir("facing").map(Dir::axis) != Some(facing.axis());
            }
            _ => {}
        }
    } else if path.ends_with("_carpet") && path != "moss_carpet" {
        return facing == Dir::Up;
    }
    is_brittle(world, state)
}

/// `BlockState.canStickTo` of both ways: slime and honey stick to everything but each other.
pub fn can_stick_to(a: &State, b: &State) -> bool {
    let sticky = |s: &State| s.is("minecraft:slime_block") || s.is("minecraft:honey_block");
    if (a.is("minecraft:slime_block") && b.is("minecraft:honey_block"))
        || (a.is("minecraft:honey_block") && b.is("minecraft:slime_block"))
    {
        return false;
    }
    sticky(a) || sticky(b)
}

// --- the search ----------------------------------------------------------------------------------

/// What [`search`] hands back: every block to move, in the order found.
#[derive(Debug, Default)]
pub struct Found {
    pub blocks: Vec<(Pos, State)>,
}

/// What differs between the contraptions that use [`search`].
pub struct Config<'a> {
    /// The `forcedDirection`: a block next to the one searched in that direction is always taken
    /// (unless it is not supportive); a piston pushes along it.
    pub forced: Option<Dir>,
    /// `isAnchoringBlockAt`: cells that never join.
    pub anchoring: &'a dyn Fn(Pos) -> bool,
    /// The cells the frontier starts with (`searchMovedStructure`'s `frontier.add(pos)` and
    /// `addToInitialFrontier`).
    pub frontier: Vec<Pos>,
    /// Blocks already in the structure (a piston's poles), which count against the limit.
    pub preexisting: usize,
    /// Positions that need no search: they are already in the structure.
    pub known: HashSet<Pos>,
}

/// `Contraption.searchMovedStructure`. `Err` carries `AssemblyException`s.
pub fn search(world: &dyn World, config: &Config<'_>) -> Result<Found, Fail> {
    let mut frontier: VecDeque<Pos> = VecDeque::new();
    let mut queued: HashSet<Pos> = HashSet::new();
    let mut visited: HashSet<Pos> = HashSet::new();
    let mut found = Found::default();
    let mut taken: HashSet<Pos> = config.known.clone();
    let push = |frontier: &mut VecDeque<Pos>, queued: &mut HashSet<Pos>, pos: Pos| {
        // `UniqueLinkedList`: a position already waiting is not added again.
        if queued.insert(pos) {
            frontier.push_back(pos);
        }
    };
    for pos in &config.frontier {
        push(&mut frontier, &mut queued, *pos);
    }
    let mut steps = SEARCH_LIMIT;
    while let Some(pos) = frontier.pop_front() {
        queued.remove(&pos);
        steps -= 1;
        if steps == 0 {
            return Err(Fail::StructureTooLarge);
        }
        visited.insert(pos);
        let Some(state) = world.state(pos) else {
            // `isOutsideBuildHeight` is skipped, `!isLoaded` fails.
            if pos[1] < -2048 || pos[1] > 2048 {
                continue;
            }
            return Err(Fail::UnloadedChunk(pos));
        };
        if (config.anchoring)(pos) {
            continue;
        }
        if !movement_necessary(world, &state, pos) {
            continue;
        }
        if !movement_allowed(world, &state, pos) {
            return Err(Fail::UnmovableBlock(pos, state.name.clone()));
        }
        let add = |target: Pos, frontier: &mut VecDeque<Pos>, queued: &mut HashSet<Pos>| {
            if !visited.contains(&target) {
                push(frontier, queued, target);
            }
        };
        // Chassis: the range of blocks they hold on to, and the chassis next to them.
        if let Some(chassis) = world.chassis(pos) {
            for target in chassis_targets(world, &state, pos, chassis.range, config.forced) {
                add(target, &mut frontier, &mut queued);
            }
            for target in chassis_neighbours(world, &state, pos) {
                add(target, &mut frontier, &mut queued);
            }
        }
        // Double chest halves stick together.
        if state.has("type") && state.has("facing") && state.name.ends_with("chest") {
            let facing = state.dir("facing");
            match (state.get("type"), facing) {
                (Some("left"), Some(f)) => add(relative(pos, clockwise(f), 1), &mut frontier, &mut queued),
                (Some("right"), Some(f)) => {
                    add(relative(pos, clockwise(f).opposite(), 1), &mut frontier, &mut queued)
                }
                _ => {}
            }
        }
        // A bearing takes the block in front of it along (it is not stabilised here).
        if state.name == "create:mechanical_bearing" {
            if let Some(facing) = state.dir("facing") {
                add(relative(pos, facing, 1), &mut frontier, &mut queued);
            }
        }
        // A gantry carries its carriage and shafts (`moveGantryPinion`, `moveGantryShaft`).
        if state.is(crate::rules::gantry::CARRIAGE) || state.is(crate::rules::gantry::SHAFT) {
            for target in crate::rules::gantry::targets(world, pos, &state) {
                add(target, &mut frontier, &mut queued);
            }
        }
        // The Windmill Bearing attaches its structure when moved.
        if state.name == "create:windmill_bearing" {
            if let Some(facing) = state.dir("facing") {
                add(relative(pos, facing, 1), &mut frontier, &mut queued);
            }
        }
        // Pistons drag their attached poles and extension.
        if matches!(state.path(), "mechanical_piston" | "sticky_mechanical_piston")
            && state.name.starts_with("create:")
        {
            let Some(facing) = state.dir("facing") else {
                continue;
            };
            if state.get("state") == Some("moving") {
                return Err(Fail::UnmovableBlock(pos, state.name.clone()));
            }
            let behind = relative(pos, facing, -1);
            if is_pole_on(world, behind, facing.axis()) {
                add(behind, &mut frontier, &mut queued);
            }
            if state.get("state") == Some("extended") || state.is("create:sticky_mechanical_piston") {
                add(relative(pos, facing, 1), &mut frontier, &mut queued);
            }
        }
        if state.is("create:piston_extension_pole") {
            if let Some(facing) = state.dir("facing") {
                for d in [facing.axis_dirs()[0], facing.axis_dirs()[1]] {
                    let offset = relative(pos, d, 1);
                    if visited.contains(&offset) {
                        continue;
                    }
                    let Some(other) = world.state(offset) else { continue };
                    let on_axis = other.dir("facing").is_some_and(|f| f.axis() == d.axis());
                    if (other.is("create:piston_extension_pole") || other.is("create:mechanical_piston_head"))
                        && on_axis
                    {
                        add(offset, &mut frontier, &mut queued);
                    }
                    if other.name.starts_with("create:") && other.path().ends_with("mechanical_piston") {
                        let piston = other.dir("facing");
                        if piston == Some(d)
                            || (piston == Some(d.opposite()) && other.get("state") == Some("extended"))
                        {
                            add(offset, &mut frontier, &mut queued);
                        }
                    }
                }
            }
        }
        if state.is("create:mechanical_piston_head") {
            if let Some(direction) = state.dir("facing") {
                let offset = relative(pos, direction.opposite(), 1);
                if !visited.contains(&offset) {
                    if let Some(other) = world.state(offset) {
                        let pole = other.is("create:piston_extension_pole")
                            && other.dir("facing").is_some_and(|f| f.axis() == direction.axis());
                        let piston = other.name.starts_with("create:")
                            && other.path().ends_with("mechanical_piston")
                            && other.dir("facing") == Some(direction)
                            && other.get("state") == Some("extended");
                        if pole || piston {
                            add(offset, &mut frontier, &mut queued);
                        }
                    }
                }
                if state.get("type") == Some("sticky") {
                    add(relative(pos, direction, 1), &mut frontier, &mut queued);
                }
            }
        }

        // Slime blocks and super glue drag adjacent blocks if possible.
        for offset in Dir::ALL {
            let offset_pos = relative(pos, offset, 1);
            if (config.anchoring)(offset_pos) {
                continue;
            }
            let Some(neighbour) = world.state(offset_pos) else {
                continue;
            };
            if !movement_allowed(world, &neighbour, offset_pos) {
                if Some(offset) == config.forced {
                    return Err(Fail::UnmovableBlock(pos, state.name.clone()));
                }
                continue;
            }
            let was_visited = visited.contains(&offset_pos);
            let face_has_glue = world.glued(pos, offset);
            let attached = attached_towards(world, &neighbour, offset.opposite());
            let brittle = is_brittle(world, &neighbour);
            let mut can_stick = !brittle && can_stick_to(&state, &neighbour);
            if can_stick {
                if push_only(&state) || push_only(&neighbour) {
                    can_stick = false;
                }
                if not_supportive(world, &state, offset) {
                    can_stick = false;
                }
                if not_supportive(world, &neighbour, offset.opposite()) {
                    can_stick = false;
                }
            }
            if !was_visited
                && (can_stick
                    || attached
                    || face_has_glue
                    || (Some(offset) == config.forced
                        && !not_supportive(world, &state, config.forced.unwrap_or(offset))))
            {
                push(&mut frontier, &mut queued, offset_pos);
            }
        }

        if taken.insert(pos) {
            found.blocks.push((pos, state));
        }
        if found.blocks.len() + config.preexisting > MAX_BLOCKS_MOVED {
            return Err(Fail::StructureTooLarge);
        }
    }
    Ok(found)
}

fn is_pole_on(world: &dyn World, pos: Pos, axis: Axis) -> bool {
    world.state(pos).is_some_and(|s| {
        s.is("create:piston_extension_pole") && s.dir("facing").is_some_and(|f| f.axis() == axis)
    })
}

/// `Direction.getClockWise()` about Y: north to east.
pub fn clockwise(dir: Dir) -> Dir {
    match dir {
        Dir::North => Dir::East,
        Dir::East => Dir::South,
        Dir::South => Dir::West,
        Dir::West => Dir::North,
        other => other,
    }
}

/// `Direction.getClockWise(axis)` (`Dir::clockwise_about`): vanilla's tables, which are not one
/// handedness about every axis (X turns up to south).
pub fn clockwise_about(dir: Dir, axis: Axis) -> Dir {
    dir.clockwise_about(axis)
}

/// A direction turned `quarters` quarter turns right-handed about the positive end of `axis`
/// (positive: counterclockwise seen from there; `VecHelper.rotate` by 90 degrees each).
pub fn rotate_dir(dir: Dir, axis: Axis, quarters: i32) -> Dir {
    let n = rotate_pos(dir.normal(), axis, quarters);
    Dir::from_normal(n).unwrap_or(dir)
}

/// An integer vector turned `quarters` quarter turns right-handed about `axis` through the origin
/// (`VecHelper.rotate`: Y turns x to -z).
pub fn rotate_pos(pos: Pos, axis: Axis, quarters: i32) -> Pos {
    let [mut x, mut y, mut z] = pos;
    for _ in 0..quarters.rem_euclid(4) {
        let next = match axis {
            Axis::X => [x, -z, y],
            Axis::Y => [z, y, -x],
            Axis::Z => [-y, x, z],
        };
        [x, y, z] = next;
    }
    [x, y, z]
}

impl Dir {
    /// The two directions along this direction's axis (negative, positive).
    pub fn axis_dirs(self) -> [Dir; 2] {
        match self.axis() {
            Axis::X => [Dir::West, Dir::East],
            Axis::Y => [Dir::Down, Dir::Up],
            Axis::Z => [Dir::North, Dir::South],
        }
    }
}

// --- chassis (ChassisBlockEntity) -----------------------------------------------------------------

/// `AbstractChassisBlock.getGlueableSide` read from the state: whether `face` is sticky
/// (`rules::chassis`).
fn glueable(state: &State, face: Dir) -> bool {
    crate::rules::chassis::is_sticky(state, face)
}

fn axis_of(state: &State) -> Axis {
    state.get("axis").and_then(Axis::from_name).unwrap_or(Axis::Y)
}

/// `ChassisBlockEntity.getIncludedBlockPositions`.
pub fn chassis_targets(
    world: &dyn World,
    state: &State,
    pos: Pos,
    range: i32,
    forced: Option<Dir>,
) -> Vec<Pos> {
    let axis = axis_of(state);
    let mut found = Vec::new();
    if state.is("create:radial_chassis") {
        // `getIncludedBlockPositionsRadial`.
        for facing in Dir::ALL {
            if facing.axis() == axis || !glueable(state, facing) {
                continue;
            }
            let mut local: VecDeque<Pos> = VecDeque::from([relative(pos, facing, 1)]);
            let mut seen: HashSet<Pos> = HashSet::new();
            while let Some(search) = local.pop_front() {
                if seen.contains(&search) {
                    continue;
                }
                // `closerThan(pos, range + .5)`: distance between block centres.
                let d: f64 = (0..3).map(|i| f64::from(search[i] - pos[i]).powi(2)).sum();
                if d >= (f64::from(range) + 0.5).powi(2) {
                    continue;
                }
                let Some(searched) = world.state(search) else { continue };
                if !movement_necessary(world, &searched, search) || is_brittle(world, &searched) {
                    continue;
                }
                seen.insert(search);
                if search != pos {
                    found.push(search);
                }
                for offset in Dir::ALL {
                    if offset.axis() == axis {
                        continue;
                    }
                    if search == pos && offset != facing {
                        continue;
                    }
                    if not_supportive(world, &searched, offset) {
                        continue;
                    }
                    local.push_back(relative(search, offset, 1));
                }
            }
        }
        return found;
    }
    // `getIncludedBlockPositionsLinear`.
    for facing in [axis_positive(axis), axis_positive(axis).opposite()] {
        let sticky = glueable(state, facing);
        for i in 1..=range {
            let current = relative(pos, facing, i);
            let Some(current_state) = world.state(current) else { break };
            if forced != Some(facing) && !sticky {
                break;
            }
            if !movement_necessary(world, &current_state, current) {
                break;
            }
            if is_brittle(world, &current_state) {
                break;
            }
            found.push(current);
            if not_supportive(world, &current_state, facing) {
                break;
            }
        }
    }
    found
}

fn axis_positive(axis: Axis) -> Dir {
    match axis {
        Axis::X => Dir::East,
        Axis::Y => Dir::Up,
        Axis::Z => Dir::South,
    }
}

/// `ChassisBlockEntity.addAttachedChasses`: the chassis that join this one.
pub fn chassis_neighbours(world: &dyn World, state: &State, pos: Pos) -> Vec<Pos> {
    let axis = axis_of(state);
    let mut out = Vec::new();
    if state.is("create:radial_chassis") {
        for offset in [-1, 1] {
            let current = relative(pos, axis_positive(axis), offset);
            if let Some(neighbour) = world.state(current) {
                if neighbour.is("create:radial_chassis") && axis_of(&neighbour) == axis {
                    out.push(current);
                }
            }
        }
        return out;
    }
    for offset in Dir::ALL {
        let current = relative(pos, offset, 1);
        let Some(neighbour) = world.state(current) else { continue };
        let linear = neighbour.is("create:linear_chassis") || neighbour.is("create:secondary_linear_chassis");
        // `LinearChassisBlock.sameKind`.
        if linear && neighbour.name == state.name && axis_of(&neighbour) == axis {
            out.push(current);
        }
    }
    out
}

// --- colliders (TranslatingContraption) -----------------------------------------------------------

/// A moved block as the collision check needs it.
pub struct Part<'a> {
    pub pos: Pos,
    pub state: &'a State,
}

/// `TranslatingContraption.createColliders`: the blocks at the leading edge of a structure
/// moving along `direction`: they have a collision shape, and the block ahead of them in the
/// structure has none. `solid` says whether a moved state has any collision.
pub fn colliders(
    blocks: &BTreeMap<Pos, State>,
    direction: Dir,
    solid: &dyn Fn(&State) -> bool,
) -> Vec<Pos> {
    blocks
        .iter()
        .filter(|(pos, state)| {
            if !solid(state) {
                return false;
            }
            let ahead = relative(**pos, direction, 1);
            !blocks.get(&ahead).is_some_and(|s| solid(s))
        })
        .map(|(pos, _)| *pos)
        .collect()
}

/// `ContraptionCollider.isCollidingWithWorld` for one collider cell: the world block the moved
/// block would overlap is neither replaceable nor without collision.
pub fn blocked_by(world: &dyn World, cell: Pos) -> bool {
    let Some(state) = world.state(cell) else {
        return true; // `!world.isLoaded`
    };
    if state.path() == "cocoa" && state.name.starts_with("minecraft:") {
        return false;
    }
    !replaceable(&state) && !world.collision_empty(cell)
}

// --- StructureTransform ---------------------------------------------------------------------------

/// `StructureTransform` for a rotation about one axis and a move: positions turn about the origin
/// cell's centre, then `offset` is added.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transform {
    pub offset: Pos,
    pub axis: Axis,
    /// Quarter turns right-handed about the positive end of `axis` (0 to 3).
    pub quarters: i32,
}

impl Transform {
    /// `new StructureTransform(offset, xRot, yRot, zRot)`: the angle is rounded to the nearest
    /// 90 degrees (`Math.round(angle / 90) * 90`).
    pub fn from_angle(offset: Pos, axis: Axis, angle: f32) -> Self {
        let quarters = ((angle / 90.0).round() as i32).rem_euclid(4);
        Transform {
            offset,
            axis,
            quarters,
        }
    }

    pub fn translation(offset: Pos) -> Self {
        Transform {
            offset,
            axis: Axis::Y,
            quarters: 0,
        }
    }

    /// `apply(BlockPos)`.
    pub fn apply(&self, local: Pos) -> Pos {
        add(rotate_pos(local, self.axis, self.quarters), self.offset)
    }

    /// `apply(BlockState)`: the state turned as the structure was.
    pub fn apply_state(&self, state: &State) -> State {
        rotate_state(state, self.axis, self.quarters)
    }
}

/// `StructureTransform.apply(BlockState)` with `BlockState.rotate`: about Y vanilla's rotation
/// (`facing`, `axis`, `rotation`, the sides, rails), about X and Z what Create special-cases
/// (`facing`, `axis`, stairs, face-attached blocks, half turns).
pub fn rotate_state(state: &State, axis: Axis, quarters: i32) -> State {
    let quarters = quarters.rem_euclid(4);
    if quarters == 0 {
        return state.clone();
    }
    // `AbstractChassisBlock.transform`: the sticky sides turn with the block.
    if crate::rules::chassis::is_chassis(&state.name) {
        return crate::rules::chassis::rotate(state, axis, quarters);
    }
    let mut out = state.clone();
    // Create's own `TransformableBlock`s: the directional axis blocks flip `axis_along_first` on
    // a quarter turn (`DirectionalAxisKineticBlock.rotate`).
    if axis == Axis::Y {
        rotate_y(&mut out, quarters);
        return out;
    }
    let name = state.name.as_str();
    let path = state.path();
    // `FaceAttachedHorizontalDirectionalBlock` (levers, buttons, grindstone).
    if state.has("face") && state.has("facing") && matches!(state.get("face"), Some("floor" | "wall" | "ceiling")) {
        return rotate_face_attached(state, axis, quarters);
    }
    if path.ends_with("_stairs") && name.starts_with("minecraft:") {
        return rotate_stairs(state, axis, quarters);
    }
    if let Some(facing) = state.dir("facing") {
        // `state.hasProperty(FACING)`: six-way facings turn; a horizontal-only block turns
        // only on half turns (below).
        if has_vertical_facing(state) {
            out.set("facing", rotate_dir(facing, axis, quarters).name());
            return out;
        }
    }
    if let Some(a) = state.get("axis").and_then(Axis::from_name) {
        let positive = rotate_dir(axis_positive(a), axis, quarters);
        out.set("axis", positive.axis().name());
        return out;
    }
    if quarters == 2 {
        if let Some(facing) = state.dir("facing") {
            if facing.axis() == axis {
                return out;
            }
        }
        rotate_y(&mut out, 2);
        // Slabs flip.
        match out.get("type") {
            Some("bottom") if path.ends_with("_slab") => out.set("type", "top"),
            Some("top") if path.ends_with("_slab") => out.set("type", "bottom"),
            _ => {}
        }
    }
    out
}

/// Whether the block's `facing` can point up and down (a `DirectionalBlock`'s) rather than only
/// sideways: told by the property of this mod's blocks and vanilla's six-way ones.
fn has_vertical_facing(state: &State) -> bool {
    if state.name.starts_with("create:") {
        return !matches!(
            state.path(),
            "belt" | "andesite_funnel" | "brass_funnel" | "chute" | "smart_chute" | "fluid_tank"
        ) || state.dir("facing").is_some_and(|f| f.axis() == Axis::Y);
    }
    matches!(
        state.path(),
        "piston" | "sticky_piston" | "dispenser" | "dropper" | "observer" | "piston_head"
            | "end_rod" | "lightning_rod" | "barrel" | "hopper" | "command_block"
            | "chain_command_block" | "repeating_command_block" | "amethyst_cluster"
            | "large_amethyst_bud" | "medium_amethyst_bud" | "small_amethyst_bud"
    ) || state.get("facing").is_some_and(|f| f == "up" || f == "down")
}

/// `BlockState.rotate(Rotation)` about the Y axis, `quarters` counterclockwise quarter turns seen
/// from above: facing, axis, `rotation`, the four sides, rails and Create's `axis_along_first`.
fn rotate_y(state: &mut State, quarters: i32) {
    // Counterclockwise from above: north -> west -> south -> east.
    let turn = |dir: Dir| rotate_dir(dir, Axis::Y, quarters);
    if let Some(facing) = state.dir("facing") {
        if facing.axis() != Axis::Y {
            state.set("facing", turn(facing).name());
        }
    }
    if let Some(a) = state.get("axis").and_then(Axis::from_name) {
        if a != Axis::Y && quarters % 2 == 1 {
            state.set("axis", if a == Axis::X { "z" } else { "x" });
        }
    }
    if let Some(a) = state.get("horizontal_axis").and_then(Axis::from_name) {
        if quarters % 2 == 1 {
            state.set("horizontal_axis", if a == Axis::X { "z" } else { "x" });
        }
    }
    // A Create block with `axis_along_first` swaps it on odd turns.
    if state.has("axis_along_first") && quarters % 2 == 1 {
        let flipped = !state.flag("axis_along_first");
        state.set("axis_along_first", if flipped { "true" } else { "false" });
    }
    // Signs and banners: `rotation` 0 to 15, 4 per quarter. Counterclockwise from above is
    // -4 on the sign's clockwise count (`Rotation.COUNTERCLOCKWISE_90.rotate(rot, 16)`).
    if let Some(rotation) = state.get("rotation").and_then(|r| r.parse::<i32>().ok()) {
        state.set("rotation", &(rotation - 4 * quarters).rem_euclid(16).to_string());
    }
    // Fences, walls, panes, redstone wire, vines and multifaces: the four sides move.
    if state.has("north") || state.has("east") || state.has("south") || state.has("west") {
        let old: Vec<(Dir, Option<String>)> = [Dir::North, Dir::East, Dir::South, Dir::West]
            .into_iter()
            .map(|d| (d, state.get(d.name()).map(str::to_owned)))
            .collect();
        for (dir, value) in old {
            if let Some(value) = value {
                state.set(turn(dir).name(), &value);
            }
        }
    }
    if let Some(shape) = state.get("shape").map(str::to_owned) {
        if let Some(rotated) = rotate_rail_shape(&shape, quarters) {
            state.set("shape", rotated);
        }
    }
}

/// `BaseRailBlock` shapes under `Rotation.rotate`.
fn rotate_rail_shape(shape: &str, quarters: i32) -> Option<&'static str> {
    const ORDER: [&str; 4] = ["north", "west", "south", "east"];
    let turn = |name: &str| {
        let i = ORDER.iter().position(|n| *n == name)?;
        Some(ORDER[(i + quarters as usize) % 4])
    };
    Some(match shape {
        "north_south" => {
            if quarters % 2 == 1 { "east_west" } else { "north_south" }
        }
        "east_west" => {
            if quarters % 2 == 1 { "north_south" } else { "east_west" }
        }
        s if s.starts_with("ascending_") => match turn(s.trim_start_matches("ascending_"))? {
            "north" => "ascending_north",
            "south" => "ascending_south",
            "east" => "ascending_east",
            _ => "ascending_west",
        },
        "south_east" | "south_west" | "north_west" | "north_east" => {
            // The corners, counterclockwise: south_east -> north_east -> north_west -> south_west.
            const CORNERS: [&str; 4] = ["south_east", "north_east", "north_west", "south_west"];
            let i = CORNERS.iter().position(|c| *c == shape)?;
            CORNERS[(i + quarters as usize) % 4]
        }
        _ => return None,
    })
}

/// `StructureTransform.transformStairs`.
fn rotate_stairs(state: &State, axis: Axis, quarters: i32) -> State {
    let mut out = state.clone();
    let half_turn = quarters == 2;
    let Some(facing) = state.dir("facing") else { return out };
    if facing.axis() != axis {
        for _ in 0..quarters {
            let direction = out.dir("facing").unwrap_or(facing);
            let bottom = out.get("half") == Some("bottom");
            if (direction.sign() > 0) ^ bottom ^ (direction.axis() == Axis::Z) {
                out.set("half", if bottom { "top" } else { "bottom" });
            } else {
                out.set("facing", direction.opposite().name());
            }
        }
    } else if half_turn {
        let bottom = out.get("half") == Some("bottom");
        out.set("half", if bottom { "top" } else { "bottom" });
    }
    out
}

/// `StructureTransform`'s face-attached branch (`FaceAttachedHorizontalDirectionalBlock`).
fn rotate_face_attached(state: &State, axis: Axis, quarters: i32) -> State {
    let mut out = state.clone();
    let z = axis == Axis::Z;
    let forced = if z { Dir::West } else { Dir::South };
    let facing = out.dir("facing").unwrap_or(Dir::North);
    if facing.axis() == axis && out.get("face") == Some("wall") {
        return out;
    }
    // `Rotation.ordinal()`: clockwise turns, so a right-handed quarter is three of them.
    let steps = (4 - quarters).rem_euclid(4);
    for _ in 0..steps {
        let face = out.get("face").unwrap_or("wall").to_owned();
        let facing = out.dir("facing").unwrap_or(Dir::North);
        let ceiling = face == "ceiling";
        out.set("facing", if ceiling { forced } else { forced.opposite() }.name());
        if face != "wall" {
            out.set("face", "wall");
            continue;
        }
        let negative = if z { facing.sign() < 0 } else { facing.sign() > 0 };
        out.set("face", if negative { "floor" } else { "ceiling" });
    }
    out
}

// --- tests -----------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Grid {
        blocks: HashMap<Pos, State>,
        glue: HashSet<(Pos, Dir)>,
    }

    impl Grid {
        fn new() -> Self {
            Grid {
                blocks: HashMap::new(),
                glue: HashSet::new(),
            }
        }

        fn set(&mut self, pos: Pos, text: &str) {
            self.blocks.insert(pos, State::parse(text));
        }

        fn fill(&mut self, from: Pos, to: Pos, text: &str) {
            for x in from[0]..=to[0] {
                for y in from[1]..=to[1] {
                    for z in from[2]..=to[2] {
                        self.set([x, y, z], text);
                    }
                }
            }
        }
    }

    impl World for Grid {
        fn state(&self, pos: Pos) -> Option<State> {
            if pos[1] < -64 || pos[1] > 320 {
                return None;
            }
            Some(self.blocks.get(&pos).cloned().unwrap_or_else(|| State::new("minecraft:air")))
        }
        fn collision_empty(&self, pos: Pos) -> bool {
            self.blocks.get(&pos).is_none_or(|s| s.is_air() || is_brittle(self, s))
        }
        fn glued(&self, pos: Pos, face: Dir) -> bool {
            self.glue.contains(&(pos, face))
                || self.glue.contains(&(relative(pos, face, 1), face.opposite()))
        }
    }

    fn config(forced: Option<Dir>, anchor: Pos) -> Config<'static> {
        Config {
            forced,
            anchoring: Box::leak(Box::new(move |p: Pos| p == [anchor[0], anchor[1] + 1000, anchor[2]])),
            frontier: vec![anchor],
            preexisting: 0,
            known: HashSet::new(),
        }
    }

    #[test]
    fn state_text_round_trips() {
        let s = State::parse("minecraft:oak_stairs[facing=east,half=bottom,shape=straight]");
        assert_eq!(s.name, "minecraft:oak_stairs");
        assert_eq!(s.get("half"), Some("bottom"));
        assert_eq!(s.text(), "minecraft:oak_stairs[facing=east,half=bottom,shape=straight]");
        assert_eq!(State::parse("minecraft:stone").props.len(), 0);
        assert_eq!(State::parse("minecraft:stone").text(), "minecraft:stone");
    }

    #[test]
    fn positions_pack_like_block_pos() {
        for pos in [[0, 0, 0], [1, 2, 3], [-1, -1, -1], [-5, 64, 300], [30_000_000, -64, -30_000_000]] {
            assert_eq!(unpack(pack(pos)), pos);
        }
        // `BlockPos.asLong(1, 2, 3)`.
        assert_eq!(pack([1, 2, 3]), (1 << 38) | (3 << 12) | 2);
    }

    #[test]
    fn a_column_of_stone_is_one_structure_with_glue_or_without_when_forced() {
        // Nothing sticks to stone by itself: a plain piston pushes the line in front of it.
        let mut grid = Grid::new();
        grid.fill([0, 0, 0], [0, 0, 3], "minecraft:stone");
        grid.set([1, 0, 0], "minecraft:stone");
        let found = search(&grid, &config(Some(Dir::South), [0, 0, 0])).expect("found");
        let positions: HashSet<Pos> = found.blocks.iter().map(|(p, _)| *p).collect();
        // The column ahead in the forced direction, not the block beside it.
        assert!(positions.contains(&[0, 0, 0]) && positions.contains(&[0, 0, 3]));
        assert!(!positions.contains(&[1, 0, 0]));
        assert_eq!(positions.len(), 4);
    }

    #[test]
    fn glue_and_slime_pull_in_neighbours() {
        let mut grid = Grid::new();
        grid.set([0, 0, 0], "minecraft:stone");
        grid.set([1, 0, 0], "minecraft:stone");
        grid.set([0, 1, 0], "minecraft:dirt");
        grid.glue.insert(([0, 0, 0], Dir::East));
        let found = search(&grid, &config(None, [0, 0, 0])).expect("found");
        let positions: HashSet<Pos> = found.blocks.iter().map(|(p, _)| *p).collect();
        assert!(positions.contains(&[1, 0, 0]));
        assert!(!positions.contains(&[0, 1, 0]));

        grid.set([0, 1, 0], "minecraft:slime_block");
        grid.set([0, 2, 0], "minecraft:dirt");
        let found = search(&grid, &config(None, [0, 0, 0])).expect("found");
        let positions: HashSet<Pos> = found.blocks.iter().map(|(p, _)| *p).collect();
        assert!(positions.contains(&[0, 1, 0]) && positions.contains(&[0, 2, 0]));
    }

    #[test]
    fn brittle_blocks_follow_what_they_hang_on() {
        let mut grid = Grid::new();
        grid.set([0, 0, 0], "minecraft:stone");
        grid.set([0, 1, 0], "minecraft:torch");
        grid.set([1, 0, 0], "minecraft:wall_torch[facing=east]");
        grid.set([-1, 0, 0], "minecraft:wall_torch[facing=east]");
        grid.set([0, 0, 1], "minecraft:oak_pressure_plate");
        let found = search(&grid, &config(None, [0, 0, 0])).expect("found");
        let positions: HashSet<Pos> = found.blocks.iter().map(|(p, _)| *p).collect();
        assert!(positions.contains(&[0, 1, 0]), "a torch stands on the block");
        assert!(positions.contains(&[1, 0, 0]), "a wall torch facing away from it hangs on it");
        assert!(!positions.contains(&[-1, 0, 0]), "one facing the block isn't attached to it");
        assert!(!positions.contains(&[0, 0, 1]), "a plate beside the block isn't");
    }

    #[test]
    fn unmovable_blocks_stop_the_search_in_the_forced_direction_only() {
        let mut grid = Grid::new();
        grid.set([0, 0, 0], "minecraft:stone");
        grid.set([0, 0, 1], "minecraft:obsidian");
        assert_eq!(
            search(&grid, &config(Some(Dir::South), [0, 0, 0])).unwrap_err(),
            Fail::UnmovableBlock([0, 0, 0], "minecraft:stone".to_owned())
        );
        // Beside it, glue or no glue, it's left alone.
        let found = search(&grid, &config(None, [0, 0, 0])).expect("found");
        assert_eq!(found.blocks.len(), 1);
        grid.set([0, 0, 1], "minecraft:bedrock");
        assert!(search(&grid, &config(Some(Dir::South), [0, 0, 0])).is_err());
    }

    #[test]
    fn the_search_stops_at_the_limit() {
        let mut grid = Grid::new();
        grid.fill([0, 0, 0], [15, 15, 15], "minecraft:stone");
        grid.fill([0, 16, 0], [15, 16, 15], "minecraft:stone");
        // Slime everywhere makes it one 4352-block structure.
        for pos in grid.blocks.clone().keys() {
            grid.set(*pos, "minecraft:slime_block");
        }
        assert_eq!(search(&grid, &config(None, [0, 0, 0])).unwrap_err(), Fail::StructureTooLarge);
    }

    #[test]
    fn unloaded_cells_fail_the_search() {
        let mut grid = Grid::new();
        grid.set([0, 64, 0], "minecraft:slime_block");
        grid.set([0, 321, 0], "minecraft:slime_block");
        // Above the world, `state` is `None` and the cell is skipped as outside build height...
        assert!(search(&grid, &config(None, [0, 64, 0])).is_ok());
    }

    #[test]
    fn double_chests_move_together() {
        let mut grid = Grid::new();
        grid.set([0, 0, 0], "minecraft:chest[facing=north,type=left]");
        grid.set([1, 0, 0], "minecraft:chest[facing=north,type=right]");
        let found = search(&grid, &config(None, [0, 0, 0])).expect("found");
        assert_eq!(found.blocks.len(), 2);
        // A chest facing north: its left half is on the east side, so its partner is at +x.
        assert_eq!(clockwise(Dir::North), Dir::East);
    }

    #[test]
    fn rotations_turn_positions_and_directions() {
        // `VecHelper.rotate` about Y by 90 degrees takes +x to -z.
        assert_eq!(rotate_pos([1, 0, 0], Axis::Y, 1), [0, 0, -1]);
        assert_eq!(rotate_pos([0, 0, -1], Axis::Y, 1), [-1, 0, 0]);
        assert_eq!(rotate_pos([0, 0, 1], Axis::X, 1), [0, -1, 0]);
        assert_eq!(rotate_pos([1, 0, 0], Axis::Z, 1), [0, 1, 0]);
        for axis in Axis::ALL {
            let p = [3, -2, 5];
            assert_eq!(rotate_pos(rotate_pos(p, axis, 1), axis, 3), p);
            assert_eq!(rotate_pos(p, axis, 2), rotate_pos(rotate_pos(p, axis, 1), axis, 1));
        }
        // North turned counterclockwise from above is west; `getClockWise` is the opposite.
        assert_eq!(rotate_dir(Dir::North, Axis::Y, 1), Dir::West);
        assert_eq!(clockwise_about(Dir::North, Axis::Y), Dir::East);
    }

    #[test]
    fn angles_snap_to_quarter_turns() {
        let t = Transform::from_angle([10, 0, 10], Axis::Y, 91.0);
        assert_eq!(t.quarters, 1);
        assert_eq!(t.apply([1, 0, 0]), [10, 0, 9]);
        assert_eq!(Transform::from_angle([0; 3], Axis::Y, -89.0).quarters, 3);
        assert_eq!(Transform::from_angle([0; 3], Axis::Y, 44.0).quarters, 0);
        assert_eq!(Transform::from_angle([0; 3], Axis::Y, 135.0).quarters, 2);
        assert_eq!(Transform::from_angle([0; 3], Axis::Y, 359.0).quarters, 0);
    }

    #[test]
    fn states_turn_with_the_structure() {
        let stairs = State::parse("minecraft:oak_stairs[facing=north,half=bottom,shape=straight]");
        assert_eq!(rotate_state(&stairs, Axis::Y, 1).get("facing"), Some("west"));
        assert_eq!(rotate_state(&stairs, Axis::Y, 2).get("facing"), Some("south"));
        let log = State::parse("minecraft:oak_log[axis=x]");
        assert_eq!(rotate_state(&log, Axis::Y, 1).get("axis"), Some("z"));
        assert_eq!(rotate_state(&log, Axis::Z, 1).get("axis"), Some("y"));
        let shaft = State::parse("create:shaft[axis=y]");
        assert_eq!(rotate_state(&shaft, Axis::Y, 1).get("axis"), Some("y"));
        assert_eq!(rotate_state(&shaft, Axis::X, 1).get("axis"), Some("z"));
        let fence = State::parse("minecraft:oak_fence[east=true,north=false,south=false,west=false]");
        let turned = rotate_state(&fence, Axis::Y, 1);
        // East turned counterclockwise from above is north.
        assert_eq!(turned.get("north"), Some("true"));
        assert_eq!(turned.get("east"), Some("false"));
        let sign = State::parse("minecraft:oak_sign[rotation=4]");
        assert_eq!(rotate_state(&sign, Axis::Y, 1).get("rotation"), Some("0"));
        let piston = State::parse("create:mechanical_piston[axis_along_first=false,facing=east,state=retracted]");
        let turned = rotate_state(&piston, Axis::Y, 1);
        assert_eq!(turned.get("facing"), Some("north"));
        assert_eq!(turned.get("axis_along_first"), Some("true"));
        // Up and down stay when turning about Y; a six-way facing turns about X.
        let up = State::parse("create:mechanical_drill[facing=up]");
        assert_eq!(rotate_state(&up, Axis::Y, 1).get("facing"), Some("up"));
        assert_eq!(rotate_state(&up, Axis::X, 1).get("facing"), Some("south"));
        // Stone is stone.
        assert_eq!(rotate_state(&State::new("minecraft:stone"), Axis::Y, 1).text(), "minecraft:stone");
    }

    #[test]
    fn stairs_turned_over_a_horizontal_axis_flip() {
        let stairs = State::parse("minecraft:oak_stairs[facing=east,half=bottom,shape=straight]");
        // About X, a half turn flips an east stair upside down.
        let turned = rotate_state(&stairs, Axis::X, 2);
        assert_eq!(turned.get("half"), Some("top"));
        assert_eq!(turned.get("facing"), Some("east"));
    }

    #[test]
    fn colliders_are_the_leading_edge() {
        let mut blocks = BTreeMap::new();
        for z in 0..3 {
            blocks.insert([0, 0, z], State::new("minecraft:stone"));
        }
        let solid = |s: &State| !s.is_air();
        assert_eq!(colliders(&blocks, Dir::South, &solid), vec![[0, 0, 2]]);
        assert_eq!(colliders(&blocks, Dir::North, &solid), vec![[0, 0, 0]]);
        // Moving sideways, every block leads.
        assert_eq!(colliders(&blocks, Dir::East, &solid).len(), 3);
    }

    #[test]
    fn replaceable_and_blocking_come_from_the_game_data() {
        assert!(replaceable(&State::new("minecraft:short_grass")));
        assert!(!replaceable(&State::new("minecraft:stone")));
        assert!(listed(BLOCKS_PUSH, "minecraft:obsidian"));
        assert!(push_only(&State::new("minecraft:red_glazed_terracotta")));
    }

    #[test]
    fn a_linear_chassis_takes_its_range() {
        struct Chassis(Grid);
        impl World for Chassis {
            fn state(&self, pos: Pos) -> Option<State> {
                self.0.state(pos)
            }
            fn collision_empty(&self, pos: Pos) -> bool {
                self.0.collision_empty(pos)
            }
            fn chassis(&self, pos: Pos) -> Option<ChassisInfo> {
                self.0
                    .blocks
                    .get(&pos)
                    .filter(|s| s.name.ends_with("chassis"))
                    .map(|_| ChassisInfo { range: 3 })
            }
        }
        let mut grid = Grid::new();
        grid.set([0, 0, 0], "create:linear_chassis[axis=x,sticky_top=true,sticky_bottom=false]");
        grid.fill([1, 0, 0], [5, 0, 0], "minecraft:stone");
        grid.set([-1, 0, 0], "minecraft:stone");
        let world = Chassis(grid);
        let found = search(&world, &config(None, [0, 0, 0])).expect("found");
        let positions: HashSet<Pos> = found.blocks.iter().map(|(p, _)| *p).collect();
        // Axis x: top is +x. Three blocks (the range), none on the unsticky side.
        assert_eq!(positions.len(), 4);
        assert!(positions.contains(&[3, 0, 0]) && !positions.contains(&[4, 0, 0]));
        assert!(!positions.contains(&[-1, 0, 0]));
    }

    #[test]
    fn a_radial_chassis_takes_a_ball_of_blocks_through_its_sticky_side() {
        struct Radial(Grid);
        impl World for Radial {
            fn state(&self, pos: Pos) -> Option<State> {
                self.0.state(pos)
            }
            fn collision_empty(&self, pos: Pos) -> bool {
                self.0.collision_empty(pos)
            }
            fn chassis(&self, pos: Pos) -> Option<ChassisInfo> {
                self.0
                    .blocks
                    .get(&pos)
                    .filter(|s| s.name == "create:radial_chassis")
                    .map(|_| ChassisInfo { range: 2 })
            }
        }
        let mut grid = Grid::new();
        grid.set(
            [0, 0, 0],
            "create:radial_chassis[axis=y,sticky_east=true,sticky_north=false,sticky_south=false,sticky_west=false]",
        );
        grid.fill([1, 0, -1], [2, 0, 2], "minecraft:stone");
        grid.set([-1, 0, 0], "minecraft:stone");
        grid.set([0, 1, 0], "minecraft:stone");
        grid.set([4, 0, 0], "minecraft:stone");
        let found = search(&Radial(grid), &config(None, [0, 0, 0])).expect("found");
        let positions: HashSet<Pos> = found.blocks.iter().map(|(p, _)| *p).collect();
        // The side facing east is sticky: the stone out that way within 2.5 blocks; not the
        // other sides, the ends (along the axis) or anything farther.
        assert!(positions.contains(&[1, 0, 0]) && positions.contains(&[2, 0, 0]));
        assert!(positions.contains(&[2, 0, 1]) && !positions.contains(&[2, 0, 2]));
        assert!(!positions.contains(&[-1, 0, 0]) && !positions.contains(&[0, 1, 0]));
        assert!(!positions.contains(&[4, 0, 0]));
    }

    #[test]
    fn a_turned_chassis_keeps_its_glue_on_the_same_blocks() {
        let state = State::parse("create:linear_chassis[axis=y,sticky_bottom=false,sticky_top=true]");
        let turned = rotate_state(&state, Axis::Z, 1);
        assert_eq!(turned.get("axis"), Some("x"));
        // The top end turned from up to a horizontal end.
        assert!(turned.flag("sticky_top") ^ turned.flag("sticky_bottom"));
    }
}
